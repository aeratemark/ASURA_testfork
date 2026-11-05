//! Log in to any server that speaks this protocol, and print every message it sends.
//!
//! Built to answer one question with evidence: *what does the reference server actually send* for a
//! login? GWPS does not log its auth traffic (its trace covers game messages), so the cheapest honest
//! way to see it is to be the client - this one speaks the same wire format, including the
//! variable-length client messages a real client sends.
//!
//! ```text
//! cargo run -p asura-auth --example probe -- 127.0.0.1:6112 <portal-user> <session-id>
//! ```
//!
//! It prints each frame as `header name size` followed by the body in hex, so two servers can be
//! compared byte for byte.

use std::process::ExitCode;

use asura_protocol::client::handshake;
use asura_protocol::client::messages::PortalAccountLogin;
use asura_protocol::client::reference_client::ReferenceClient;
use asura_protocol::client::tables;
use asura_protocol::client::Frame;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use uuid::Uuid;

const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let address = args.first().map(String::as_str).unwrap_or("127.0.0.1:6112");
    let (Some(user), Some(session)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: probe [ADDRESS] <portal-user-uuid> <session-uuid>");
        return ExitCode::from(2);
    };
    let Ok(user) = Uuid::parse_str(user) else {
        eprintln!("probe: not a UUID: {user}");
        return ExitCode::from(2);
    };
    let Ok(session) = Uuid::parse_str(session) else {
        eprintln!("probe: not a UUID: {session}");
        return ExitCode::from(2);
    };

    let mut stream = match TcpStream::connect(address).await {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("probe: cannot connect to {address}: {err}");
            return ExitCode::FAILURE;
        }
    };
    println!("# connected to {address}");

    let mut client = ReferenceClient::with_seed(0xA11CE);
    stream
        .write_all(&client.version_message(39112))
        .await
        .expect("write the version message");
    stream
        .write_all(&client.client_seed_message())
        .await
        .expect("write the client seed");

    let mut server_seed = vec![0u8; handshake::SERVER_SEED_LEN];
    if let Err(err) = stream.read_exact(&mut server_seed).await {
        eprintln!("probe: no server seed: {err}");
        return ExitCode::FAILURE;
    }
    if let Err(err) = client.accept_server_seed(&server_seed) {
        eprintln!("probe: bad server seed: {err}");
        return ExitCode::FAILURE;
    }
    println!("# key exchange done");

    // The preamble a real client sends, then the login. Both in the client's variable-length form.
    let mut preamble = client.computer_info_preamble("Apo", "DESKTOP-DURJ7N0", 39112);
    client.seal_raw(&mut preamble);
    stream.write_all(&preamble).await.expect("write preamble");

    let login = PortalAccountLogin {
        request: 1,
        user_id: user.to_bytes_le(),
        session_id: session.to_bytes_le(),
        character_name: String::new(),
        kind: "Portal".to_string(),
    }
    .try_to_frame()
    .expect("builds");
    let mut login_bytes = login.encode();
    client.seal_raw(&mut login_bytes);

    let mut buffer = vec![0u8; 16384];
    let mut login_sent = false;
    let mut printed = 0usize;
    let mut raw = Vec::new();

    loop {
        let read = match tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buffer)).await {
            Err(_) => {
                println!("# no more data for {}s", READ_TIMEOUT.as_secs());
                break;
            }
            Ok(Ok(0)) => {
                println!("# server closed the connection");
                break;
            }
            Ok(Ok(read)) => read,
            Ok(Err(err)) => {
                println!("# read error: {err}");
                break;
            }
        };

        // The stream itself, decrypted but not framed: the ground truth when two servers disagree
        // about where a message ends.
        let mut chunk = buffer[..read].to_vec();
        client.decrypt(&mut chunk);
        raw.extend_from_slice(&chunk);

        for frame in frame_stream(&raw.clone()) {
            let name = tables::entry(frame.header())
                .and_then(|entry| entry.name)
                .unwrap_or("unnamed");
            println!(
                "{:#06X} {:<32} {} bytes  {}",
                frame.header(),
                name,
                frame.len(),
                hex(frame.body())
            );
            printed += 1;
        }

        if !login_sent && printed > 0 {
            stream.write_all(&login_bytes).await.expect("write login");
            println!("# login sent");
            login_sent = true;
        }

        // A server that keeps a connection alive with heartbeats never goes quiet, so waiting for
        // silence would wait forever: stop two messages after the login is answered, which is enough
        // to see whether anything follows it.
        if login_sent {
            if let Some(response_at) = frame_stream(&raw.clone())
                .iter()
                .position(|frame| frame.header() == tables::AUTH_SMSG_REQUEST_RESPONSE)
            {
                if frame_stream(&raw.clone()).len() > response_at + 2 {
                    println!("# stopping: the login was answered");
                    break;
                }
            }
        }
    }

    // The stream itself, saved, so a fixture can be taken from it rather than typed.
    if let Ok(path) = std::env::var("PROBE_RAW_OUT") {
        if let Err(err) = std::fs::write(&path, &raw) {
            println!("# could not write {path}: {err}");
        } else {
            println!("# raw stream written to {path}");
        }
    }

    println!("# {printed} message(s)");
    println!("# raw decrypted stream ({} bytes)", raw.len());
    for (index, line) in raw.chunks(32).enumerate() {
        let hexed: Vec<String> = line.iter().map(|byte| format!("{byte:02x}")).collect();
        println!("{:06X}  {}", index * 32, hexed.join(" "));
    }
    ExitCode::SUCCESS
}

/// Frame as much of the stream as possible, guessing nothing: stops at the first boundary it cannot
/// resolve and says so.
fn frame_stream(bytes: &[u8]) -> Vec<Frame> {
    let mut frames = Vec::new();
    let mut position = 0usize;
    while position + 2 <= bytes.len() {
        match Frame::decode_client_message(&bytes[position..]) {
            Ok((frame, used)) => {
                position += used;
                frames.push(frame);
            }
            Err(_) => break,
        }
    }
    if position != bytes.len() && !bytes.is_empty() {
        println!("# stalled at offset {position} of {}", bytes.len());
    }
    frames
}

fn hex(bytes: &[u8]) -> String {
    let shown: Vec<String> = bytes
        .iter()
        .take(80)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if bytes.len() > 80 {
        format!("{} ...", shown.join(" "))
    } else {
        shown.join(" ")
    }
}
