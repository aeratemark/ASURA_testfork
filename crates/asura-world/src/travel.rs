//! Travel: leaving one instance for another.
//!
//! The reference's request/redirect cycle (`docs/02-architecture.md`, "Travel, end to end"):
//! reserve a slot, write a transfer row, send `TRANSFER_GAME_SERVER_INFO`, then
//! `MAP_UPDATE_CURRENT`. [`teleport`] is the `/tp` half; [`load_map_on_demand`] is the
//! documented fallback that loads a map this world was not started with (`docs/03-protocol.md`
//! #5c, measured 0-2 ms per map).
//!
//! The client-driven half of travel - `AUTH_CMSG_REQUEST_GAME_INSTANCE` and the front door - is
//! in `asura-auth`; a world only decides where a player *already here* goes next.

use crate::*;

/// The district number the reference always answers with (`AuthSrv_HandleRequestGameInstance`).
pub const DISTRICT_NUMBER: u32 = 1;

/// The effect word a live server sets on the travelling agent just before a redirect
/// (`captures/zone-ref2-012225.jsonl` seq 971: `f1 00 00 00 | 43 00 00 00 | 00 01 00 00`).
pub const TRAVEL_EFFECT: u32 = 0x100;

/// How a redirect's `is_explorable` is decided, and why the default is not the destination's kind.
///
/// A live server sends 1 for an explorable and 0 for an outpost (`doors-20261031-095754.jsonl`: 146 and
/// 147 -> 1, 148 -> 0), and this world sent that too until 2026-10-31. At 1 an explorable's load against
/// this world waits: 5138 ms from the redirect to the players block against 849 ms at 0, over the same
/// walk through the same portal (`docs/23-initialization-protocol.md`, "The explorable"). The cause of
/// that wait is open - the exploration grid and the instance table were ruled out on 2026-10-31 - so the
/// default is the value that loads, and the faithful value is one variable away.
///
/// `ASURA_TRAVEL_EXPLORABLE`: unset or `0` sends 0 for every travel (the default); `1` sends 1; `auto`
/// sends the destination's own kind, which is what a live server does and what the default becomes once
/// an explorable carries the initialisation `docs/23` says it is missing.
pub fn explorable_flag_for(map_type: i32) -> u32 {
    match flag_mode() {
        FlagMode::Zero => 0,
        FlagMode::One => 1,
        FlagMode::Auto => match map_type {
            1 | 4 | 6 => 1,
            _ => 0,
        },
    }
}

/// The switch's mode in words, for the startup line: a log always says which value travels.
pub fn explorable_flag_mode() -> &'static str {
    match flag_mode() {
        FlagMode::Zero => "0 for every travel (default: an explorable is announced as an outpost)",
        FlagMode::One => "1 for every travel",
        FlagMode::Auto => "auto: the destination's own kind, which is what a live server sends",
    }
}

#[derive(Clone, Copy)]
enum FlagMode {
    Zero,
    One,
    Auto,
}

fn flag_mode() -> FlagMode {
    static MODE: std::sync::OnceLock<FlagMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("ASURA_TRAVEL_EXPLORABLE").ok().as_deref() {
        Some("auto") => FlagMode::Auto,
        Some("1") => FlagMode::One,
        _ => FlagMode::Zero,
    })
}

/// Load a catalogue map this world was **not** started with - the `--on-demand` experiment.
///
/// Off unless `--on-demand` (`World::load_on_demand`): the reference pre-spawns one server per
/// configured map so that a travel never constructs anything, and this server preloads every hosted
/// map for the same reason. This switch answers the open question of whether that readiness is a hard
/// requirement for ASURA or only a property of OpenTyria's design (`docs/03-protocol.md`).
///
/// The three steps are done here, in this order, because they are what a preload would have done at
/// startup: read the `map_config` row, publish the seat pool (what `reserve` counts and what the auth
/// front door's world lookup sees), and import the path graph out of `Gw.dat`. Each step's cost is
/// logged; a client load that hangs while one of these runs is the measurement the switch exists for.
///
/// `None` means "no usable config": no `map_file_id`, or a map type this world does not serve. That is
/// the same refusal a preloaded map with bad config would get, so the client sees one refusal shape.
pub async fn load_map_on_demand(
    world: &World,
    map_id: i32,
) -> Result<Option<MapEntry>, asura_db::DatabaseError> {
    let started = Instant::now();
    let Some(configured) = db_game::spawn_map(&world.pool, map_id).await? else {
        return Ok(None);
    };
    let map_file_id = u32::try_from(configured.map_file_id).unwrap_or(0);
    if map_file_id == 0 {
        return Ok(None);
    }
    if !matches!(
        configured.map_type,
        MAP_TYPE_MAIN_TOWN | MAP_TYPE_MAIN_EXPLORABLE | MAP_TYPE_CHARACTER_CREATION
    ) {
        return Ok(None);
    }
    let entry = MapEntry {
        map_id,
        map_file_id,
        map_type: configured.map_type,
        spawn: (configured.spawn_x, configured.spawn_y),
        spawn_plane: configured.spawn_plane as u16,
        name: configured.name.clone(),
        capacity: world.capacity,
    };
    let row_done = started.elapsed();

    let pool = Instant::now();
    placement::publish_pool(&world.pool, world.world_id, map_id, world.capacity).await?;
    let pool_done = pool.elapsed();

    let graph = Instant::now();
    if let Err(err) = world.graphs.import(&entry, &world.archive) {
        eprintln!(
            "world: map {map_id} '{}' loaded on demand without a path graph: {err}",
            entry.name
        );
    }
    let graph_done = graph.elapsed();
    world.insert_lazy_map(entry.clone());

    eprintln!(
        "world: map {} '{}' loaded on demand in {} ms (row {} ms, pool {} ms, path graph {} ms); \
         the travel goes on with this destination",
        map_id,
        entry.name,
        started.elapsed().as_millis(),
        row_done.as_millis(),
        pool_done.as_millis(),
        graph_done.as_millis()
    );
    Ok(Some(entry))
}

/// A travel the **client** announced: `GAME_CMSG_TRAVEL_INIT` (`0x80C1`), the portal touch.
///
/// Ten bytes on the wire, two agent ids and nothing else (the row's fields; the capture in
/// `recordings/client/feature/movement/movement-sniff.log` seq 1651 shows the same message with a
/// four-byte header, `3a 00 00 00 | 00 00 00 00` after it, and seq 1652 shows the client sending
/// `INTERACT_LIVING` for that same agent 0x3A in the same millisecond - so the first id is the travel
/// object the player used, and the second was zero).
///
/// **It names an agent, not a map**, so the destination is this server's knowledge, not the client's:
/// the object it names leads somewhere only because a row says so. This world has no portal table yet
/// and no non-player agent to touch (it sends exactly one agent, the player), so this arm names the
/// touch and starts no transfer - it is the measurement that has to come first. The line below is
/// what the next live touch writes: which agent, in which map, for which player. A portal row keyed
/// by that map and agent is what turns this into the same redirect `/tp` already sends
/// (`docs/03-protocol.md` 5e).
pub fn handle_travel_init(frame: &Frame, player: &Player, _trace: bool) -> Result<bool, GameError> {
    if frame.body().len() < 8 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 10 }));
    }
    let agent = u32::from_le_bytes([frame.body()[0], frame.body()[1], frame.body()[2], frame.body()[3]]);
    let other = u32::from_le_bytes([frame.body()[4], frame.body()[5], frame.body()[6], frame.body()[7]]);
    eprintln!(
        "world: {:?} touched a travel object in map {} '{}': TRAVEL_INIT agent {agent:#010X}, \
         second {other:#010X}; checking the doorways at the character's position",
        player.name, player.instance.map.map_id, player.instance.map.name,
    );
    Ok(true)
}

/// `/tp <alias>` and `/teleport <alias>`: `GameSrv_HandleTeleportCommand` (`GmCommands.c:155-216`).
///
/// A teleport is not a position change - it is a **server transfer**, the same cycle a portal starts.
/// The reference asks its auth service for a game server on the target district; the auth service mints
/// a player token and records the identity the client's *next* connection must present; the old server
/// sends `TRANSFER_GAME_SERVER_INFO` and the client reconnects to be handed on
/// (`PreSearing/docs/08-gm-commands.md`, "What a teleport actually does").
///
/// This is that cycle with this server's pieces: the seat is reserved for the destination (giving up the
/// current one in the same transaction, `placement::reserve`), the transfer row is written *before* the
/// redirect so a client that reconnects the instant it is told finds it, and the two messages are the
/// five fields the capture shows. The limit is stated rather than hidden: this server never constructs
/// another world, so the alias has to name a map it already hosts.
pub async fn teleport(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    argument: &str,
    trace: bool,
) -> Result<bool, GameError> {
    let wanted = alias_key(argument);
    if wanted.is_empty() {
        // `GmCommand_ToAscii` refuses an empty argument, and the answer is the invalid-command line.
        eprintln!("world: /tp without a map name");
        return Ok(true);
    }
    // The alias table, not the map names: the reference accepts `lakeside` as well as `lakeside county`
    // (`s_teleport_targets` lists both), so matching against `MapEntry::alias()` answered "unknown map
    // name" for half the aliases the reference's own `/maps` prints.
    let Some(target) = travel_target(argument) else {
        // `GmCommands.c:172-177`: "Unknown map name. Type /maps." through the gated chat sender, which
        // with the gate off is a log line - the same shape as every other GM answer here.
        eprintln!("world: Unknown map name. Type /maps.");
        return Ok(true);
    };
    if !target.hosted {
        // `GmCommands.c:180-190`: the reference refuses a target with no `GmMapConfig` rather than letting
        // `GameSrv_LoadMap` fall back to Kamadan. Here the equivalent is a map this world has no row for.
        eprintln!(
            "world: That map has no config; the server would fall back, so the teleport is refused ({}).",
            target.label
        );
        return Ok(true);
    }
    let target = match world.map(target.map_id) {
        Some(map) => map,
        None if world.load_on_demand => {
            // `--on-demand`: the destination was not preloaded. Load it here, and measure the cost -
            // this is the experiment the switch exists for (docs/03-protocol.md, "precache").
            match load_map_on_demand(world, target.map_id)
                .await
                .map_err(GameError::Database)?
            {
                Some(loaded) => loaded,
                None => {
                    eprintln!(
                        "world: {} ({}) has no usable config (no map file id, or a map type this \
                         world does not serve); the teleport is refused",
                        target.label, target.map_id
                    );
                    return Ok(true);
                }
            }
        }
        None => {
            // The table knows the map; this world was not started with `--maps` for it. The
            // reference pre-spawns one server per map, so its answer would be a transfer; this one
            // can only say so - unless `--on-demand` loaded it (handled above).
            eprintln!(
                "world: {} ({}) is not hosted by this world; start it with --maps {} (or --on-demand \
                 to load destinations at travel time)",
                target.label, target.map_id, target.map_id
            );
            return Ok(true);
        }
    };
    if target.map_id == player.instance.map.map_id {
        // `GmCommands.c:198-202`. **The command's own refusal, not a travel policy**: the client-driven
        // requests (`0x80B1`, `0x80C1`) are carried out without this check, because there the client has
        // already decided and a second opinion is a gate (docs/22-world-model.md, "No server-side travel
        // policy"). A typed alias that names the instance the character stands in is the reference's
        // "You are already here." and nothing more.
        eprintln!("world: You are already here.");
        return Ok(true);
    }
    begin_transfer(stream, session, world, player, target, trace).await
}

/// The client's own request, from the world map: `GAME_CMSG_PARTY_TRAVEL` (`0x80B1`).
///
/// **The client decides where it may go; this server carries the decision out.** That is the whole
/// policy (`docs/22-world-model.md`): the unlocked-instance bitmap is what a client uses to offer a
/// destination, and a server that second-guesses it blocks legitimate travel - an outpost and the
/// explorable it exits to can share one map blob, so a rule about "which kinds of instance you may
/// travel to" refuses travel the game itself allows.
///
/// The wire, measured from this client's own sends (`logs/world-8001.log`, `world <- 0x80B1 (9 bytes)`):
///
/// ```text
/// B1 80 | 94 00 | 02 | 00 00 | 00 | 01      destination 148
/// B1 80 | A6 00 | 02 | 00 00 | 00 | 01      destination 166
/// ```
///
/// so the destination instance is a `u16` first, then three bytes this server does not interpret. The
/// retail client sends the same request with a 32-bit instance id
/// (`captures/zone-ref2-012225.jsonl`: `b1 00 00 00 | a4 00 00 00 | ... | 01 00 00 00`), which is the
/// other build of the protocol (`docs/03-protocol.md` 5e) - the field is the same one either way.
pub async fn handle_party_travel(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    if frame.body().len() < 2 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 9 }));
    }
    let map_id = i32::from(u16::from_le_bytes([frame.body()[0], frame.body()[1]]));
    eprintln!(
        "world: {:?} asked for instance {map_id} from the world map in map {}",
        player.name, player.instance.map.map_id
    );
    let Some(destination) = resolve_instance(world, map_id).await? else {
        // A statement about this server, not about the client's rights: it has no row, no blob, or a
        // type this world does not host.
        eprintln!("world: instance {map_id} cannot be served by this world; the travel is refused");
        return Ok(true);
    };
    begin_transfer(stream, session, world, player, destination, trace).await
}

/// The instance a travel names, demand-loaded when this world hosts it only on demand.
///
/// `None` means this server cannot serve it at all.
async fn resolve_instance(world: &World, map_id: i32) -> Result<Option<MapEntry>, GameError> {
    if let Some(map) = world.map(map_id) {
        return Ok(Some(map));
    }
    if !world.load_on_demand {
        return Ok(None);
    }
    load_map_on_demand(world, map_id)
        .await
        .map_err(GameError::Database)
}

/// A doorway was reached: the third trigger of the same cycle.
///
/// `/tp`, the world map's request (`0x80B1`) and this one all end in [`begin_transfer`]. This is the trigger
/// a live server works out for itself: a real crossing contains **no client message at all**
/// (`docs/03-protocol.md` 5f, `login-to-portal-20261031-003720.jsonl`), so the world compares the position
/// it is already being sent with the doorways of the instance it is hosting (`crate::portals`).
///
/// A doorway whose destination is not recorded fires nothing and says so, naming the coordinates. That line
/// is the measurement which fills the next row - and it is why an unmeasured doorway is safer than a guess.
pub async fn portal_reached(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let (x, y) = player.standing().position;
    portal_at_position(stream, session, world, player, x, y, trace).await
}

/// A doorway reached by a **walk that is only being asked for**: `0x803E` `MOVE_TO_COORD` names the
/// coordinate the click sends the character toward (`crates/asura-world/src/agent.rs`), and it arrives
/// before the walk does. Without this, a click that lands the character inside a doorway can end with its
/// last position report outside `portals::REACH`, and no crossing fires until the player moves again - the
/// way back that only worked once the portal was touched a second time (measured 2026-10-31).
pub async fn portal_target_reached(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    if frame.body().len() < 8 {
        return Ok(true);
    }
    let x = f32::from_le_bytes([frame.body()[0], frame.body()[1], frame.body()[2], frame.body()[3]]);
    let y = f32::from_le_bytes([frame.body()[4], frame.body()[5], frame.body()[6], frame.body()[7]]);
    portal_at_position(stream, session, world, player, x, y, trace).await
}

/// A crossing decided from the player's **own step**: where the character stood before this report and where
/// it stands after it.
///
/// This is the rule a doorway wants, and the only one: the segment between two consecutive positions is the
/// movement the client actually performed, so a step taken through a door crosses it, a click that names a
/// place beyond a door does not fire early, and nothing depends on where a report happened to stop. The
/// doorway still carries the destination; a door without one does nothing and says so.
pub async fn portal_crossed(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    from: (f32, f32),
    to: (f32, f32),
    trace: bool,
) -> Result<bool, GameError> {
    if player.moved {
        return Ok(true);
    }
    let instance = player.instance.map.map_id;
    match crate::portals::portal_along(instance, from, to) {
        Some(portal) => {
            portal_at_position(stream, session, world, player, portal.x, portal.y, trace).await
        }
        None => Ok(true),
    }
}
/// The crossing, for a doorway reached at `(x, y)`.
async fn portal_at_position(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    x: f32,
    y: f32,
    trace: bool,
) -> Result<bool, GameError> {
    // A crossing already in flight owns this connection: its seat belongs to the destination and the
    // client is about to reconnect, so a further position report from inside the doorway must not start a
    // second one. Without this a walk into a door can send two redirects, and a client given two
    // reconnects takes its time to settle.
    if player.moved {
        return Ok(true);
    }
    let instance = player.instance.map.map_id;
    let Some(portal) = crate::portals::portal_at(instance, x, y) else {
        return Ok(true);
    };
    // The landing point recorded for this door - per door, not per instance. It is written as the
    // character's position in the destination before the redirect, so the connection the client opens
    // stands where the live server put it, instead of on the instance's configured spawn.
    if let (Some(to_instance), Some((ax, ay))) = (portal.to_instance, portal.arrive) {
        if let Err(err) =
            db_game::record_position(&world.pool, player.character, to_instance, ax, ay, 0).await
        {
            eprintln!("world: could not write the landing point for instance {to_instance}: {err}");
        }
    }
    let Some(to_instance) = portal.to_instance else {
        eprintln!(
            "world: {:?} is standing in a travel portal of instance {instance} at ({:.0}, {:.0}); \
             no destination is recorded for that doorway",
            player.name, portal.x, portal.y
        );
        return Ok(true);
    };
    eprintln!(
        "world: {:?} reached the portal at ({:.0}, {:.0}) in instance {instance}: crossing to {to_instance}",
        player.name, portal.x, portal.y
    );
    let Some(destination) = resolve_instance(world, to_instance).await? else {
        eprintln!(
            "world: instance {to_instance} cannot be served by this world; the crossing is refused"
        );
        return Ok(true);
    };
    begin_transfer(stream, session, world, player, destination, trace).await
}

/// The cycle itself: the seat on the destination, the transfer row, and the pair that sends the client
/// on its way.
///
/// Every trigger ends here - `/tp`, the world map's request (`0x80B1`) and, once this world has a portal
/// table, a crossing (`0x80C1`) - because the reference has one cycle and not one per trigger.
async fn begin_transfer(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    destination: MapEntry,
    trace: bool,
) -> Result<bool, GameError> {
    let to_map = destination.map_id;

    // The seat on the destination. `reserve` gives up this session's current lease in the same
    // transaction, which is exactly a transfer's accounting: one seat, moved.
    let Some(world_id) = placement::reserve(&world.pool, world.world_id, to_map, player.session_id)
        .await
        .map_err(GameError::Database)?
    else {
        eprintln!(
            "world: {} ({to_map}) has no free seat; the travel is refused",
            destination.label()
        );
        return Ok(true);
    };

    let player_token = random_u32()?;
    placement::begin_transfer(
        &world.pool,
        player_token,
        world_id,
        to_map,
        player.session_id,
        player.account_id,
        player.character,
    )
    .await
    .map_err(GameError::Database)?;

    let std::net::SocketAddr::V4(public) = world.public else {
        eprintln!("world: a travel needs an IPv4 advertised address; refused");
        return Ok(true);
    };
    // The flag is 0 by default, not the destination's kind, and `explorable_flag_for` says why: at 1 an
    // explorable's load waits 4.3 s here. `ASURA_TRAVEL_EXPLORABLE=auto` restores live's value.
    let is_explorable = explorable_flag_for(destination.map_type);
    crate::mark_redirect(player_token);
    eprintln!(
        "world: [+{} ms] redirect for map {to_map} (player token {player_token:#010X}, \
         is_explorable {is_explorable})",
        crate::stamp_ms()
    );
    let frames = vec![
        // Four messages before the redirect and four after it, in the order and at the sizes a live
        // server sends them (`captures/zone-ref2-012225.jsonl`, one travel, seq 971 to 982):
        //
        //   0x00F1 agent effect   `f1 00 00 00 | 43 00 00 00 | 00 01 00 00`   the player's agent, word 0x100
        //   0x0028 stop moving    `28 00 00 00 | 43 00 00 00`                  the walk is over
        //   0x01A6 redirect       host[24], then the five fields
        //   0x0099 map update     the same millisecond, the twelve-byte form
        //   0x00B0 party size     `b0 00 00 00 | 00 00 00 00 | 01 00 00 00`
        //   0x00B1 party          `b1 00 00 00 | 0b 00 00 00 | 00 00 00 00`   player 0x0B, leader 0
        //   0x005A player destroy `5a 00 00 00 | 0b 00 00 00`
        //   0x0021 agent remove   `21 00 00 00 | 3b 00 00 00`
        //
        // The last four withdraw the travelling body from the instance it is leaving, which is what the
        // capture shows happening in the same millisecond as the redirect. The capture's own numbers are
        // not ours to reuse, so they are this connection's: its player id and its agent id. Its agent id
        // differs from the one its own position updates name, so that instance had a second body; this
        // world has one, and withdraws that one.
        //
        // **All eight are sent, and that is the point of this list.** From 2026-10-31 this vec carried
        // only the first four, while the comment above still described eight: a travel withdrew nothing
        // from the map it left, `docs/03-protocol.md:461-475` said it should, and the test that pins the
        // eight (`crates/asura-world/tests/load.rs:1775`) had been red since the experiment that cut them
        // was never reverted. Restored 2026-11-01 against that test and that table.
        game::agent_update_effects(player.agent_id, TRAVEL_EFFECT),
        game::agent_stop_moving(player.agent_id),
        game::transfer_game_server_info(
            public.ip().octets(),
            public.port(),
            world_id as u32,
            0, // region: the reference sends its district's region, and this server has no regions
            to_map as u32,
            is_explorable,
            player_token,
        ),
        game::map_update_current_redirect(to_map as u32),
        game::player_update_party_size(player.player_id, 1),
        game::player_update_party(player.player_id, 0),
        game::player_destroy(player.player_id),
        game::world_remove_agent(player.agent_id),
    ];
    send(stream, session, frames, trace).await?;
    // From here the session's seat belongs to the connection the client is about to open, so its own
    // teardown must not hand it back (see the release at the end of `handle_connection`).
    player.moved = true;
    eprintln!(
        "world: {:?} travelling: map {} -> map {} '{}', player token {player_token:#010X}, \
         redirect to {public}",
        player.name,
        player.instance.map.map_id,
        to_map,
        destination.label()
    );

    Ok(true)
}
