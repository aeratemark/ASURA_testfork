//! The game connection, as the **auth** service sees it: the client's second connection arrives here.
//!
//! This was measured, not assumed. A real client (build 39112) answered `AUTH_SMSG_GAME_SERVER_INFO`
//! by opening its game connection to the address it was launched with - the auth service - even though
//! the message named another port. The reference never noticed, because its info message names the auth
//! address too: its auth port receives *both* kinds of connection and hands the gameplay one to a game
//! server over its control channel (`AuthSrv_DoPostHandshake`, `CtrlMsgId_TransferConnection`).
//!
//! ASURA keeps that front door and changes only the last step. A socket handle cannot cross a process
//! boundary cleanly, so instead of passing the handle the auth service **opens a second connection** to
//! the world the client named and copies bytes between the two. Everything the client sees is the
//! reference's sequence; everything the world sees is a client that connected to it directly, so the
//! world needs no knowledge of the front door at all.
//!
//! What the copy does *not* do is trust anything: the transfer is still claimed by the world server,
//! which closes the connection if the token pair or the character is not the one it was told about. The
//! front door is a router, not an authority.
//!
//! Three things are worth stating because they are easy to get wrong:
//!
//! * the 64-byte version message is read here only to learn **which world** the client named, and then
//!   forwarded byte for byte - the world decodes it again, and that decode is the authoritative one;
//! * the router dials the world's **registered** address, which is the address the world publishes for
//!   clients, not the address it binds (a server behind a NAT listens on one and is reachable at the
//!   other, exactly as a realmlist works);
//! * a world that cannot be reached closes the client's connection with a log line and no message, which
//!   is what the reference does when it cannot find a game server for a connection.

use std::net::SocketAddr;

use asura_db::placement;
use asura_db::PgPool;
use asura_protocol::client::game::GameVersion;
use asura_protocol::client::version::GAME_CMSG_VERSION_LEN;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::ConnectionError;

/// How long to wait for the world's port before giving up on a client's game connection.
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Route a game connection to the world it named. Returns when either side closes.
///
/// `first_bytes` is what the caller has already read: the four bytes it classified the connection
/// with. They are part of the version message, not separate from it, so they are forwarded rather than
/// dropped.
pub async fn route(
    mut client: TcpStream,
    first_bytes: Vec<u8>,
    pool: &PgPool,
    trace: bool,
) -> Result<(), ConnectionError> {
    // ---- which world did the client name? ----------------------------------------------------
    //
    // `head` is every byte already taken off the client, and it is forwarded **whole**: a client that
    // sends its version message and its key-exchange seed in one write puts both in the same read, and
    // dropping the tail would leave the world waiting for a seed that never comes.
    let mut head = first_bytes;
    if head.len() < GAME_CMSG_VERSION_LEN {
        let need = GAME_CMSG_VERSION_LEN - head.len();
        let mut more = vec![0u8; need];
        client
            .read_exact(&mut more)
            .await
            .map_err(ConnectionError::Io)?;
        head.extend_from_slice(&more);
    }
    let (version, _) = GameVersion::decode(&head[..GAME_CMSG_VERSION_LEN]).map_err(|err| {
        // A game connection that is not a game connection: the reference closes it too.
        eprintln!("auth: a game connection's version message is malformed ({err}); closing");
        ConnectionError::GameRoute(err.to_string())
    })?;

    if trace {
        eprintln!(
            "auth <- game version build={} world={} map={} player token={:#010X}",
            version.version, version.map_token, version.map_id, version.player_token
        );
    }

    // ---- where does that world live? ---------------------------------------------------------
    let Some(endpoint) =
        placement::world_endpoint(pool, version.map_token as i32, world_freshness())
            .await
            .map_err(ConnectionError::Database)?
    else {
        eprintln!(
            "auth: a game connection named world {} (map {}), which is not registered; closing",
            version.map_token, version.map_id
        );
        return Err(ConnectionError::GameRoute(format!(
            "world {} is not registered",
            version.map_token
        )));
    };

    let Ok(address) = format!("{}:{}", endpoint.address, endpoint.port).parse::<SocketAddr>()
    else {
        eprintln!(
            "auth: world {} registered an address that does not parse; closing the client",
            endpoint.world_id
        );
        return Err(ConnectionError::GameRoute(format!(
            "world {} has no usable address",
            endpoint.world_id
        )));
    };

    // ---- hand over ----------------------------------------------------------------------------
    let mut world = match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(err)) => {
            eprintln!(
                "auth: world {} at {address} is not answering ({err}); closing the client",
                endpoint.world_id
            );
            return Err(ConnectionError::GameRoute(err.to_string()));
        }
        Err(_) => {
            eprintln!(
                "auth: world {} at {address} did not accept within {CONNECT_TIMEOUT:?}",
                endpoint.world_id
            );
            return Err(ConnectionError::GameRoute("connect timed out".to_string()));
        }
    };

    if trace {
        eprintln!(
            "auth: routing the game connection to world {} ({}) at {address}",
            endpoint.world_id, endpoint.name
        );
    }

    world.write_all(&head).await.map_err(ConnectionError::Io)?;

    // From here the two sockets are one pipe. Nothing in this loop interprets a byte: the world does
    // the key exchange and the load, and the client sees the reference's sequence.
    match tokio::io::copy_bidirectional(&mut client, &mut world).await {
        Ok((to_world, from_world)) => {
            if trace {
                eprintln!(
                    "auth: world {} connection ended after {to_world} bytes up and {from_world} down",
                    endpoint.world_id
                );
            }
            Ok(())
        }
        Err(err) => Err(ConnectionError::Io(err)),
    }
}

/// The freshness window for routing, from `ASURA_WORLD_MAX_AGE_SECS` when it is set.
///
/// The same variable the placement decision reads, and the same default: routing a client to a world
/// that placement already considers dead would only reproduce the failure it exists to prevent.
fn world_freshness() -> f64 {
    std::env::var("ASURA_WORLD_MAX_AGE_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|seconds| *seconds > 0.0)
        .unwrap_or(crate::travel::DEFAULT_WORLD_MAX_AGE_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_message_is_64_bytes_and_the_front_door_needs_all_of_them() {
        assert_eq!(GAME_CMSG_VERSION_LEN, 64);
    }

    #[test]
    fn a_malformed_version_message_is_refused_before_any_world_is_dialled() {
        let mut bytes = [0u8; GAME_CMSG_VERSION_LEN];
        bytes[0..4].copy_from_slice(&0x0000_0C04u32.to_le_bytes()); // an auth header, not a game one
        assert!(GameVersion::decode(&bytes).is_err());
    }
}
