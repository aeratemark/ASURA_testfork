//! Talk to a running auth service the way the game does, and print what comes back.
//!
//! This is the "does it answer?" check that a person can run before pointing the real client at the
//! service. It uses the reference client in `asura_protocol::client::peer` - the same one the
//! end-to-end tests use - so it needs no game, no patch and no captures:
//!
//! ```text
//! cargo run -p asura-auth --example login -- 127.0.0.1:6112 <portal-user-uuid> <session-uuid>
//! ```
//!
//! The pair to pass is the one `tools/seed-dev.ps1` writes, which is also the pair the rig's portal
//! DLL hands the client:
//!
//! ```text
//! user1@example.com  fa520ee2-4419-4eb4-ae49-6e9abe6ef24f  d8b9bf5d-90b1-4cbd-9b76-88da7be763b6
//! user2@example.com  5fb222bc-9d19-4308-ac3e-3c62685bc6ae  224aee30-d8a5-4f18-ad8e-b5fcbf121c4e
//! user3@example.com  9957c79c-b93c-4b60-9d84-9f38a1f62be3  ccdb4b8d-1c88-48c0-bda0-0a5e8d87f4da
//! ```
//!
//! It is deliberately independent of the database: it knows nothing about accounts and only speaks
//! the wire, so it fails for the right reason when the service is wrong.

use std::process::ExitCode;

use asura_protocol::client::handshake;
use asura_protocol::client::messages::{self, RequestResponse};
use asura_protocol::client::peer::ReferenceClient;
use asura_protocol::client::tables;
use asura_protocol::client::version;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use uuid::Uuid;

const REQUEST_ID: u32 = 0xA5A5;
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let address = args.first().map(String::as_str).unwrap_or("127.0.0.1:6112");

    let (user_id, session_id) = match (args.get(1), args.get(2)) {
        (Some(user), Some(session)) => {
            let user = match Uuid::parse_str(user) {
                Ok(uuid) => uuid,
                Err(err) => {
                    eprintln!("login: {user:?} is not a UUID: {err}");
                    return ExitCode::from(2);
                }
            };
            let session = match Uuid::parse_str(session) {
                Ok(uuid) => uuid,
                Err(err) => {
                    eprintln!("login: {session:?} is not a UUID: {err}");
                    return ExitCode::from(2);
                }
            };
            (user, session)
        }
        _ => {
            eprintln!(
                "usage: cargo run -p asura-auth --example login -- [ADDRESS] <portal-user> <session>"
            );
            eprintln!();
            eprintln!("The pairs, from tools/seed-dev.ps1:");
            eprintln!(
                "  fa520ee2-4419-4eb4-ae49-6e9abe6ef24f  d8b9bf5d-90b1-4cbd-9b76-88da7be763b6"
            );
            eprintln!(
                "  5fb222bc-9d19-4308-ac3e-3c62685bc6ae  224aee30-d8a5-4f18-ad8e-b5fcbf121c4e"
            );
            eprintln!(
                "  9957c79c-b93c-4b60-9d84-9f38a1f62be3  ccdb4b8d-1c88-48c0-bda0-0a5e8d87f4da"
            );
            return ExitCode::from(2);
        }
    };

    eprintln!("login: connecting to {address}");
    let mut stream = match TcpStream::connect(address).await {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("login: cannot connect: {err}");
            return ExitCode::FAILURE;
        }
    };

    let mut client = ReferenceClient::with_seed(0x1234_5678);
    if let Err(err) = stream.write_all(&client.version_message(34591)).await {
        eprintln!("login: writing the version message failed: {err}");
        return ExitCode::FAILURE;
    }
    eprintln!("login -> version {}", version::AUTH_CMSG_VERSION_HEADER);

    if let Err(err) = stream.write_all(&client.client_seed_message()).await {
        eprintln!("login: writing the client seed failed: {err}");
        return ExitCode::FAILURE;
    }

    let mut server_seed = vec![0u8; handshake::SERVER_SEED_LEN];
    if let Err(err) = stream.read_exact(&mut server_seed).await {
        eprintln!("login: no server seed came back: {err}");
        return ExitCode::FAILURE;
    }
    match client.accept_server_seed(&server_seed) {
        Ok(()) => eprintln!("login <- server seed, scrambled from here on"),
        Err(err) => {
            eprintln!("login: the server seed was rejected: {err}");
            return ExitCode::FAILURE;
        }
    }

    let frame =
        match client.login_message(REQUEST_ID, user_id.to_bytes_le(), session_id.to_bytes_le()) {
            Ok(frame) => frame,
            Err(err) => {
                eprintln!("login: could not build the login message: {err}");
                return ExitCode::FAILURE;
            }
        };
    // A client message is not the table's size, so it is sealed as raw bytes rather than validated.
    let mut sealed = frame.encode();
    client.seal_raw(&mut sealed);
    if let Err(err) = stream.write_all(&sealed).await {
        eprintln!("login: writing the login failed: {err}");
        return ExitCode::FAILURE;
    }
    eprintln!("login -> AUTH_CMSG_PORTAL_ACCOUNT_LOGIN {user_id} / {session_id}");

    // Collect until the request is answered, printing every message on the way.
    let mut status = None;
    let mut characters = 0usize;
    let mut buffer = vec![0u8; 8192];
    while status.is_none() {
        let read = match tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buffer)).await {
            Err(_) => {
                eprintln!("login: the server stopped answering");
                return ExitCode::FAILURE;
            }
            Ok(Ok(0)) => {
                eprintln!("login: the server closed the connection");
                return ExitCode::FAILURE;
            }
            Ok(Ok(read)) => read,
            Ok(Err(err)) => {
                eprintln!("login: read failed: {err}");
                return ExitCode::FAILURE;
            }
        };

        let frames = match client.receive(&buffer[..read]) {
            Ok(frames) => frames,
            Err(err) => {
                eprintln!("login: {err}");
                return ExitCode::FAILURE;
            }
        };

        for frame in frames {
            let name = tables::entry(frame.header())
                .and_then(|entry| entry.name)
                .unwrap_or("unnamed");
            eprintln!(
                "login <- {:#06X} {name} ({} bytes)",
                frame.header(),
                frame.len()
            );

            match frame.header() {
                tables::AUTH_SMSG_CHARACTER_INFO => {
                    characters += 1;
                    eprintln!("           character: {}", read_character_name(&frame.body()));
                }
                tables::AUTH_SMSG_ACCOUNT_SETTINGS => {
                    let data = u32::from_le_bytes([
                        frame.body()[4],
                        frame.body()[5],
                        frame.body()[6],
                        frame.body()[7],
                    ]);
                    eprintln!("           stored settings: {data} bytes");
                }
                tables::AUTH_SMSG_ACCOUNT_INFO => {
                    let account =
                        Uuid::from_bytes_le(frame.body()[28..44].try_into().expect("16 bytes"));
                    eprintln!("           account: {account}");
                }
                tables::AUTH_SMSG_REQUEST_RESPONSE => {
                    let reply = RequestResponse::decode(&frame).expect("a request response");
                    status = Some(reply.status);
                }
                _ => {}
            }
        }
    }

    match status {
        Some(messages::STATUS_OK) => {
            println!("OK: login accepted, {characters} character(s) on the list");
            ExitCode::SUCCESS
        }
        Some(status) => {
            let meaning = if status == messages::STATUS_AUTH_ERROR {
                "AUTH_ERROR: the identity pair did not resolve"
            } else {
                "refused"
            };
            println!("REFUSED: {meaning} (status {status})");
            ExitCode::FAILURE
        }
        None => ExitCode::FAILURE,
    }
}

/// Pull the character name out of a `AUTH_SMSG_CHARACTER_INFO` body: a `u32` count, then that many
/// UTF-16 units, at fixed offsets (`AuthSrv_CharacterInfo`).
fn read_character_name(body: &[u8]) -> String {
    let count = u32::from_le_bytes([body[24], body[25], body[26], body[27]]) as usize;
    let units: Vec<u16> = (0..count.min(20))
        .map(|index| {
            let at = 28 + index * 2;
            u16::from_le_bytes([body[at], body[at + 1]])
        })
        .collect();
    String::from_utf16(&units).unwrap_or_else(|_| "<not utf-16>".to_string())
}
