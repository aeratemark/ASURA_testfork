//! The socket layer: one listener, one decision per connection.
//!
//! A real client opens two connections to the same address: its login connection and, after being told
//! where to play, its gameplay connection (`docs/38-layer-decisions.md`, D26). Both arrive here. This
//! crate reads the four bytes that name the connection kind and nothing else about the protocol:
//!
//! * **auth** - hand the socket to `asura_auth::converse`, which speaks the login conversation;
//! * **game** - find the world the client named in the registry and copy bytes between the two
//!   ([`relay`]);
//! * **anything else** - close it, naming the kind, exactly as the reference does.
//!
//! What this crate must never grow: a game rule, a character lookup, a message decoder beyond the four
//! classification bytes. Those belong to `asura-rules`, `asura-db` and `asura-protocol`. Its one
//! authority is the registry lookup that decides *which* world a gameplay socket reaches.

use std::net::SocketAddr;

use asura_auth::ConnectionError;
use asura_db::PgPool;
use asura_protocol::client::filecheck;
use asura_protocol::client::version::{self, VersionError};
use asura_protocol::Trace;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub mod relay;

/// How much to read before classifying: the four bytes that name a connection kind.
const CLASSIFY_LEN: usize = 4;

/// Why a connection ended.
#[derive(Debug)]
pub enum Error {
    /// The socket failed.
    Io(std::io::Error),
    /// The first four bytes name no connection kind this build knows.
    UnknownConnection(VersionError),
    /// A well-formed connection that belongs to a different program: a world server, not this one.
    NotAnAuthConnection {
        /// The connection kind the first four bytes named.
        kind: version::ConnectionKind,
    },
    /// The auth conversation failed, after the connection was handed over.
    Auth(ConnectionError),
    /// A gameplay connection could not be routed to the world the client named. The client's socket
    /// is closed without a message, which is what the reference does when it cannot place one.
    GameRoute(String),
    /// The file check's bytes could not be read from `config/Server/file_check.json`.
    FileCheck(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(err) => write!(f, "socket: {err}"),
            Error::UnknownConnection(err) => write!(f, "unknown connection: {err}"),
            Error::NotAnAuthConnection { kind } => {
                write!(f, "a {kind:?} connection belongs to another program")
            }
            Error::Auth(err) => write!(f, "{err}"),
            Error::GameRoute(reason) => write!(f, "game connection not routed: {reason}"),
            Error::FileCheck(reason) => write!(f, "the file check is not configured: {reason}"),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<ConnectionError> for Error {
    fn from(err: ConnectionError) -> Self {
        match err {
            ConnectionError::Io(err) => Error::Io(err),
            other => Error::Auth(other),
        }
    }
}

/// Accept connections until the listener fails.
pub async fn serve(listener: TcpListener, pool: PgPool, trace: Trace) -> std::io::Result<()> {
    let address = listener.local_addr()?;
    // What a client is told to dial for its game connection. It is this service's own address,
    // because that is where a real client dials - measured, not assumed: a client answered
    // `GAME_SERVER_INFO` naming port 6113 by connecting to this port anyway.
    let advertised = advertised_address(address);
    eprintln!("asura-gateway: listening on {address}");
    if advertised != address {
        eprintln!("asura-gateway: advertising {advertised} to clients for game connections");
    }
    if trace.on() {
        eprintln!("asura-gateway: per-message trace on (ASURA_TRACE)");
    }

    loop {
        let (stream, peer) = listener.accept().await?;
        let pool = pool.clone();
        tokio::spawn(async move {
            match handle_connection(stream, &pool, trace, advertised).await {
                Ok(()) => {
                    if trace.on() {
                        eprintln!("gateway: {peer} closed");
                    }
                }
                Err(err) => eprintln!("gateway: {peer}: {err}"),
            }
        });
    }
}

/// The address to hand a client for its game connection: `ASURA_AUTH_PUBLIC` when it is set,
/// otherwise the address this service is bound to.
///
/// A server behind a NAT is the reason this is configurable: it listens on a private address and is
/// reached at a public one, and the client needs the second.
fn advertised_address(bind: SocketAddr) -> SocketAddr {
    match std::env::var("ASURA_AUTH_PUBLIC") {
        Ok(value) if !value.trim().is_empty() => match value.trim().parse() {
            Ok(address) => address,
            Err(err) => {
                eprintln!(
                    "gateway: ASURA_AUTH_PUBLIC {value:?} is not an address ({err}); using {bind}"
                );
                bind
            }
        },
        _ => bind,
    }
}

/// Read the four classification bytes, then hand the socket to the service that owns the conversation.
///
/// The four bytes are the beginning of the version message, not a message of their own, so they are
/// passed on rather than discarded.
pub async fn handle_connection(
    mut stream: TcpStream,
    pool: &PgPool,
    trace: Trace,
    advertised: SocketAddr,
) -> Result<(), Error> {
    let mut buffer = Vec::with_capacity(CLASSIFY_LEN * 4);

    // ---- 0. the file check ------------------------------------------------------------------
    // The client asks which game files it should have: a 21-byte request, answered with 32 bytes. Two
    // measurements of 2026-10-25 say what this block is for.
    //
    // On the wire, with `pktmon` filtered to port 6112 and the run's own pcap parser: one request and one
    // reply, and the request is **first** on that socket, before any handshake. Its failure is fatal to
    // the login - with those addresses made unreachable the client refused with "Guild Wars is unable to
    // connect to the ArenaNet network" (`docs/03-protocol.md` 7).
    //
    // Against a server of ours, with hosts entries for the six file names: one check arrived on loopback
    // and was answered - "File check on 0114: answered 32 of 32 bytes" in that run's log.
    //
    // Two consequences this block lives with. The address is the client's own choice: it builds and
    // resolves the file-server names itself, and some of those lookups ignore the hosts file, so a check
    // reaches this port only when the machine's name resolution points the client here. In the default
    // open mode it never does, and this handler is idle rather than wrong. And the client uses several
    // file servers, so one socket may ask more than once, which is why this is a loop.
    //
    // The client also sends a 4-byte follow-up after the check (`F1 0F 04 00`, header `0x40FF1`) that
    // the reference does not answer and the client tolerates unanswered (same capture). It belongs to
    // the file-server socket, not to the handshake that follows here; if it does arrive, this layer
    // classifies it as an unknown connection and closes, which the measurement says is harmless.
    // The bytes are configuration (`config/Server/file_check.json`): the question the client compares
    // whole and the answer it parses. A file that is missing or malformed is a start-up fault - this
    // layer refuses to run without it rather than closing a socket the client is waiting on - so the
    // read here is the same one `main` performed.
    let check = match asura_config::file_check() {
        Ok(check) => check,
        Err(err) => return Err(Error::FileCheck(err)),
    };
    let request = check.request_bytes().map_err(Error::FileCheck)?;
    let response = check.response_bytes().map_err(Error::FileCheck)?;
    read_until(&mut stream, &mut buffer, 1).await?;
    while filecheck::starts_with_request(&buffer, &request) {
        buffer.drain(..request.len());
        stream.write_all(&response).await?;
        if trace.on() {
            eprintln!("gateway <- file check, answered {} bytes", response.len());
        }
        if buffer.is_empty() {
            read_until(&mut stream, &mut buffer, 1).await?;
        }
    }

    // ---- 1. which service owns this connection? ---------------------------------------------
    // Four bytes arrive first and are *kept*: they are the beginning of the version message, not a
    // message of their own (`docs/03-protocol.md` 1).
    read_until(&mut stream, &mut buffer, CLASSIFY_LEN).await?;
    let kind = version::classify(&buffer).map_err(Error::UnknownConnection)?;
    match kind {
        version::ConnectionKind::Game => relay::route(stream, buffer, pool, trace.on())
            .await
            .map_err(Error::GameRoute),
        version::ConnectionKind::Auth => {
            asura_auth::converse(stream, buffer, pool, trace, advertised)
                .await
                .map_err(Error::from)
        }
        other => Err(Error::NotAnAuthConnection { kind: other }),
    }
}

/// Read until `buffer` holds at least `needed` bytes, keeping what has already arrived.
async fn read_until(
    stream: &mut TcpStream,
    buffer: &mut Vec<u8>,
    needed: usize,
) -> std::io::Result<()> {
    while buffer.len() < needed {
        let mut chunk = [0u8; 512];
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed while a message was still arriving",
            ));
        }
        buffer.extend_from_slice(&chunk[..count]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_first_word_is_named_rather_than_guessed() {
        let kind = version::classify(&[0, 0, 0, 0]);
        assert!(kind.is_err(), "four zeroes name no connection");
    }

    #[test]
    fn the_advertised_address_falls_back_to_the_bind_address() {
        let bind: SocketAddr = "127.0.0.1:6112".parse().expect("an address");
        // With no ASURA_AUTH_PUBLIC set in this process, the answer is the bind address itself.
        assert_eq!(advertised_address(bind), bind);
    }
}
