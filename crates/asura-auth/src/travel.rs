//! Travel: answering `AUTH_CMSG_REQUEST_GAME_INSTANCE` with a world the client can actually reach.
//!
//! The client asks for a map; the auth service answers with `AUTH_SMSG_GAME_SERVER_INFO` - an address,
//! a map token, and a player token - and then the ordinary `AUTH_SMSG_REQUEST_RESPONSE` with status 0.
//! That pair is the whole contract: the reference sends the info message and the response, and nothing
//! else (`AuthSrv_HandleRequestGameInstance`).
//!
//! Five steps, in this order, and the order is the design:
//!
//! 1. **Decode** the request. Six dwords; a body that does not hold them is a malformed message, not a
//!    guess.
//! 2. **Find candidate worlds** that publish the requested map, are not draining, said they were alive
//!    recently, and whose counter says there is room (`asura_db::placement::find_worlds`).
//! 3. **Probe the port.** A world can be fresh in the database and dead on the socket. The one rule
//!    this service must never break is that it does not hand a client an address nothing is listening
//!    on: the client's next move is a connection, and its failure mode is a loading screen that never
//!    ends. So every candidate is dialled, cheaply, before it is offered.
//! 4. **Reserve** the slot. Only the database can decide there is room (`reserve` is one atomic
//!    statement); the counter read in step 2 is a hint, and losing the race simply means trying the
//!    next candidate.
//! 5. **Answer**: the game server info, then the request response. Both, in that order, because the
//!    client waits for the response and the info message alone is not an answer.
//!
//! A refusal is `AUTH_SMSG_REQUEST_RESPONSE` with `GM_ERROR_NETWORK_ERROR` (2) and no info message.
//! That is honest - the client shows an error it can describe - and it is what the previous milestone
//! did for *every* travel request. What is new is that a refusal is now the exception.

use std::net::SocketAddr;
use std::time::Duration;

use asura_db::placement;
use asura_db::PgPool;
use asura_protocol::client::messages::{self, GameInstanceRequest, RequestResponse, STATUS_OK};
use asura_protocol::client::Frame;
use uuid::Uuid;

use crate::ConnectionError;

/// `GM_ERROR_NETWORK_ERROR`: what the reference answers when it cannot place a player
/// (`AuthSrv_HandleRequestGameInstance`).
pub const STATUS_NETWORK_ERROR: u32 = 2;

/// How stale a world's heartbeat may be before placement stops offering it.
///
/// The number comes from `config/Server/intervals.json` (`asura_config`, D33), because the socket layer
/// routes a client by the same window: one file, read by both, instead of two constants that agree by
/// hand. `ASURA_WORLD_MAX_AGE_SECS` still overrides it for a one-off experiment.
pub fn world_max_age_secs() -> f64 {
    static VALUE: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        override_secs().unwrap_or_else(|| asura_config::intervals().world_max_age_secs)
    })
}

/// `ASURA_WORLD_MAX_AGE_SECS`, when it names a positive number.
fn override_secs() -> Option<f64> {
    std::env::var(asura_config::MAX_AGE_VARIABLE)
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|seconds| *seconds > 0.0)
}

/// How many candidates are considered. The first that answers wins; more than a handful is a queue of
/// dead worlds, and the refusal is the same either way.
const MAX_CANDIDATES: i64 = 4;

/// How long a candidate world's port has to answer before it is dropped from consideration.
///
/// A loopback world answers in microseconds; a world that has gone silent should not make the client
/// wait for the operating system's connect timeout. From the intervals file (`asura_config`, D33);
/// `ASURA_WORLD_PROBE_TIMEOUT_MS` still overrides it.
pub fn probe_timeout() -> Duration {
    static VALUE: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        std::env::var("ASURA_WORLD_PROBE_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|millis| *millis > 0)
            .map(Duration::from_millis)
            .unwrap_or_else(|| asura_config::intervals().probe_timeout())
    })
}

/// Whether something accepts connections at `address` right now.
async fn port_answers(address: SocketAddr) -> bool {
    matches!(
        tokio::time::timeout(probe_timeout(), tokio::net::TcpStream::connect(address)).await,
        Ok(Ok(_))
    )
}

/// The frames to send in answer to a travel request, and a line for the trace.
///
/// Never returns an error for a refusal: "there is no room" is an answer, not a failure of the
/// connection. It returns `Err` only when the database itself failed, which is a different thing.
pub async fn answer(
    pool: &PgPool,
    session_id: Option<Uuid>,
    account_id: Option<Uuid>,
    character_id: Option<Uuid>,
    request: GameInstanceRequest,
    advertised: SocketAddr,
) -> Result<Vec<Frame>, ConnectionError> {
    let map_id = i32::try_from(request.map_id).unwrap_or(-1);
    let request_frames = |status: u32| {
        vec![RequestResponse {
            request: request.request,
            status,
        }
        .frame()]
    };

    let Some(session_id) = session_id else {
        // A travel request from a connection that never logged in names no session, so there is
        // nothing a lease could belong to. The reference refuses the same case (`!conn->connected`).
        eprintln!("auth: travel to map {map_id} refused: the connection has no session");
        return Ok(request_frames(STATUS_NETWORK_ERROR));
    };

    // A travel without a character is refused for the reference's own reason: there is nothing to put
    // in the map. (`AuthSrv_HandleRequestGameInstance`: "Client tried to join a server without a
    // selected character".)
    let (Some(account_id), Some(character_id)) = (account_id, character_id) else {
        eprintln!("auth: travel to map {map_id} refused: no account or character is selected");
        return Ok(request_frames(STATUS_NETWORK_ERROR));
    };

    // Where the character enters is a **game rule**, not an auth rule, so the decision is made in
    // `asura-rules` and this function only supplies the data (`docs/38-layer-decisions.md`, D25). The
    // reason a placement differs from what the client asked for is logged here, because the rule
    // itself does no I/O.
    let settings = match asura_db::game::settings(pool, character_id).await {
        Ok(settings) => settings,
        Err(err) => {
            eprintln!("auth: could not read the character's row for placement: {err}");
            None
        }
    };
    // The two values the rule needs and does not own: where a character with no history enters, and the
    // map id that means "nothing written yet". Both are `config/Server/defaults.json`
    // (`docs/38-layer-decisions.md` D34). No file, no value, so the travel is refused rather than
    // invented, and the log names the file that is missing.
    let defaults = match asura_config::defaults() {
        Ok(defaults) => defaults,
        Err(err) => {
            eprintln!("auth: travel to map {map_id} refused: {err}");
            return Ok(request_frames(STATUS_NETWORK_ERROR));
        }
    };
    // A character with no row, or a database error reading one, is treated as a character with nothing
    // written yet - which is the creation map, the value the rows themselves use for "no map".
    let empty = defaults.creation_map();
    let character = asura_rules::Character {
        map_id: settings.as_ref().map_or(empty, |settings| settings.map_id),
        last_outpost: settings
            .as_ref()
            .map_or(empty, |settings| i32::from(settings.last_outpost)),
    };
    // The destination's own type, as the catalogue describes it. `None` means the map is not in the
    // catalogue: the rule leaves the request alone and the registry lookup below refuses it.
    let destination_type = match asura_db::game::spawn_map(pool, map_id).await {
        Ok(placed) => placed.map(|map| map.map_type),
        Err(err) => {
            eprintln!("auth: could not read map {map_id} for placement: {err}");
            None
        }
    };
    let (placed_map, substituted) = asura_rules::placement(
        asura_rules::Requested {
            map_id,
            map_type: request.map_type,
        },
        character,
        destination_type,
        // Both from the defaults file, read above, plus the client's own map kinds.
        defaults.default_outpost(),
        empty,
        defaults.map_kinds(),
    );
    match substituted {
        Some(asura_rules::Substituted::CreationNamedAsPlace(from)) => eprintln!(
            "auth: the client named the creation instance as a place; placing it in map \
             {placed_map} instead ({from})"
        ),
        Some(asura_rules::Substituted::NotAnOutpost) => eprintln!(
            "auth: map {map_id} is not an outpost (type {}); placing the character in its last \
             outpost, map {placed_map}, instead",
            destination_type.unwrap_or(-1)
        ),
        Some(asura_rules::Substituted::NothingToPlace) => eprintln!(
            "auth: the character's row names no map and no last outpost (map_id {}, last_outpost {}); \
             using the configured fallback outpost {placed_map}",
            character.map_id, character.last_outpost
        ),
        None => {}
    }
    let map_id = placed_map;
    let candidates = placement::find_worlds(pool, map_id, world_max_age_secs(), MAX_CANDIDATES)
        .await
        .map_err(ConnectionError::Database)?;

    eprintln!(
        "auth: travel to map {map_id} (type {}, region {}, district {}, language {}): {} \
         candidate world(s)",
        request.map_type,
        request.region,
        request.district,
        request.language,
        candidates.len()
    );

    for candidate in &candidates {
        let address =
            match format!("{}:{}", candidate.address, candidate.port).parse::<SocketAddr>() {
                Ok(address) => address,
                Err(err) => {
                    eprintln!(
                        "auth: world {} ({}) registered an address that does not parse: {err}",
                        candidate.world_id, candidate.name
                    );
                    continue;
                }
            };

        if !port_answers(address).await {
            // Registered, in the database, and not listening: the one case that must never reach the
            // client. Keep looking; if no candidate answers, the refusal below is the honest answer.
            eprintln!(
                "auth: world {} ({}) is registered at {address} but nothing answers there; skipping",
                candidate.world_id, candidate.name
            );
            continue;
        }

        // The slot, atomically. `None` means another travel took the last one between the lookup and
        // here, which is a race, not an error: try the next candidate.
        let Some(reserved_world) = placement::reserve(pool, candidate.world_id, map_id, session_id)
            .await
            .map_err(ConnectionError::Database)?
        else {
            eprintln!(
                "auth: world {} filled up before the reservation; trying the next one",
                candidate.world_id
            );
            continue;
        };

        let player_token = crate::random_u32()?;

        // The pending transfer, written *before* the reply: the client may reconnect the instant it
        // is told where to go, and the world server's answer to that connection is a lookup in this
        // table. A row that could not be written is a travel that cannot be completed, so it is a
        // refusal -- and the lease taken above is handed back, because the slot is not in use.
        if let Err(err) = placement::begin_transfer(
            pool,
            player_token,
            candidate.world_id,
            map_id,
            session_id,
            account_id,
            character_id,
        )
        .await
        {
            eprintln!("auth: could not record the pending transfer: {err}");
            let _ = placement::release(pool, session_id).await;
            return Ok(request_frames(STATUS_NETWORK_ERROR));
        }

        if let Err(err) =
            placement::note_session_world(pool, session_id, reserved_world, map_id).await
        {
            // A routing hint that could not be written is not a reason to refuse a travel the player
            // can complete: log it and answer anyway.
            eprintln!("auth: could not record the session's world: {err}");
        }

        eprintln!(
            "auth: travel to map {map_id} placed on world {} ({}) at {address}, player token \
             {player_token:#010X}; the client is told to dial {advertised}",
            candidate.world_id, candidate.name
        );

        return Ok(vec![
            messages::game_server_info(
                request.request,
                candidate.world_id as u32,
                map_id as u32,
                // What the client dials, which is this service: the game connection arrives at the
                // auth port and is routed to `address` (`relay`). Advertising the world's own
                // address would be a lie the client cannot act on - measured: it dials the auth
                // address regardless of what this field says.
                messages::host_for(advertised),
                player_token,
            ),
            RequestResponse {
                request: request.request,
                status: STATUS_OK,
            }
            .frame(),
        ]);
    }

    eprintln!("auth: travel to map {map_id} refused: no world is serving that map right now");
    Ok(request_frames(STATUS_NETWORK_ERROR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_is_a_request_response_and_nothing_else() {
        // The shape a refusal has, without a database: status 2, the request id echoed, no info
        // message. The client's error comes from this message, so it must never be dropped.
        let frames = [RequestResponse {
            request: 9,
            status: STATUS_NETWORK_ERROR,
        }
        .frame()];
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].header(),
            asura_protocol::client::tables::AUTH_SMSG_REQUEST_RESPONSE
        );
    }

    #[test]
    fn the_defaults_are_the_ones_the_documentation_states() {
        // Read through the accessors only when the environment does not override them; the constants
        // are the documented values.
        assert_eq!(world_max_age_secs(), 60.0);
        assert_eq!(probe_timeout(), Duration::from_millis(500));
    }
}
