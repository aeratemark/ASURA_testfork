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
//!
//! **A party travels as one** (`docs/45-party-travel.md`, experimental): any member crossing a doorway
//! moves the whole party at once, a leader's world-map travel in an outpost moves it after a countdown,
//! and anyone else's world-map travel leaves the party first (`asura_rules::party::travel_request`).
//! A move writes the party ledger and hands every traveller's own connection an [`Order`], so each
//! member runs the same cycle a lone travel runs, on its own socket.

use crate::*;

/// Something a connection is told to do by another player's connection: the channel each slot holds
/// beside its outbox (`Instance::order`).
#[derive(Debug, Clone)]
pub enum Order {
    /// Travel with the party: the destination the move resolved once for everyone, the arrival point a
    /// doorway stated (or none), and every traveller's agent id, which each traveller's redirect batch
    /// marks with the travel effect.
    Travel {
        /// Where the party goes.
        destination: MapEntry,
        /// Where it stands on arrival, when a doorway said so.
        landing: Option<(f32, f32)>,
        /// The agents of every traveller, this one's own included.
        companions: Vec<u32>,
    },
}

/// Carry out an order on this connection's own socket.
pub async fn carry_out(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    order: Order,
    trace: bool,
) -> Result<bool, GameError> {
    match order {
        Order::Travel {
            destination,
            landing,
            companions,
        } => {
            if player.moved {
                return Ok(true);
            }
            let others: Vec<u32> = companions
                .into_iter()
                .filter(|agent| *agent != player.agent_id)
                .collect();
            let answer = begin_transfer(
                stream,
                session,
                world,
                player,
                destination,
                landing,
                &others,
                trace,
            )
            .await;
            if !player.moved {
                // Refused (no seat, no address): this member stays behind, alone.
                crate::party::stays_behind(&player.instance, player.player_id, trace);
            }
            answer
        }
    }
}

/// Move a party that [`crate::party::begin_move`] has started: write the ledger the destination builds
/// the party from (`asura_db::parties::depart`), then order every traveller's connection to go. A ledger
/// that could not be written is logged and the travellers still go; they arrive as separate players.
pub async fn move_party(
    world: &World,
    instance: &Instance,
    mv: crate::party::Move,
    destination: MapEntry,
    landing: Option<(f32, f32)>,
    trace: bool,
) {
    let characters: Vec<Uuid> = mv.travellers.iter().map(|(_, _, c)| *c).collect();
    match asura_db::parties::depart(
        &world.pool,
        mv.leader,
        world.world_id,
        destination.map_id,
        &characters,
    )
    .await
    {
        Ok(ledger) => eprintln!(
            "world: party {} leaves map {} for map {} '{}' with {} traveller(s) (ledger {ledger})",
            mv.party_id,
            instance.map.map_id,
            destination.map_id,
            destination.label(),
            mv.travellers.len()
        ),
        Err(err) => eprintln!(
            "world: party {} could not write its ledger ({err}); its {} traveller(s) go and arrive \
             as separate players",
            mv.party_id,
            mv.travellers.len()
        ),
    }
    let companions: Vec<u32> = mv.travellers.iter().map(|(_, agent, _)| *agent).collect();
    for (member, _, _) in &mv.travellers {
        let order = Order::Travel {
            destination: destination.clone(),
            landing,
            companions: companions.clone(),
        };
        if !instance.order(*member, order) && trace {
            eprintln!("world: party: player {member} is gone before its travel order reached it");
        }
    }
}

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
    *MODE.get_or_init(
        || match std::env::var("ASURA_TRAVEL_EXPLORABLE").ok().as_deref() {
            Some("auto") => FlagMode::Auto,
            Some("1") => FlagMode::One,
            _ => FlagMode::Zero,
        },
    )
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
    let pack_file_id = asura_config::world::catalogue()
        .map_err(|err| {
            asura_db::DatabaseError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, err))
        })?
        .get(&map_id)
        .and_then(|map| map.map_file_id);
    let map_file_id =
        pack_file_id.unwrap_or_else(|| u32::try_from(configured.map_file_id).unwrap_or(0));
    if map_file_id == 0 {
        return Ok(None);
    }
    let kinds = map_kinds();
    if configured.map_type != kinds.main_town
        && configured.map_type != kinds.main_explorable
        && configured.map_type != kinds.character_creation
    {
        return Ok(None);
    }
    let entry = MapEntry {
        map_id,
        map_file_id,
        map_type: configured.map_type,
        spawn: (configured.spawn_x, configured.spawn_y),
        spawn_plane: configured.spawn_plane as u16,
        spawns: crate::map_spawns::for_map(map_id)
            .map(<[_]>::to_vec)
            .unwrap_or_default(),
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
    let agent = u32::from_le_bytes([
        frame.body()[0],
        frame.body()[1],
        frame.body()[2],
        frame.body()[3],
    ]);
    let other = u32::from_le_bytes([
        frame.body()[4],
        frame.body()[5],
        frame.body()[6],
        frame.body()[7],
    ]);
    eprintln!(
        "world: {:?} touched a travel object in map {} '{}': TRAVEL_INIT agent {agent:#010X}, \
         second {other:#010X}; no portal row names that agent, so no travel starts",
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
    // A teleport is one character's: it leaves its party first, as a world-map travel by a member does.
    crate::party::leave_to_travel(player, trace);
    begin_transfer(stream, session, world, player, target, None, &[], trace).await
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
    let Some(request) = game::read_party_travel(frame.body()) else {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 9 }));
    };
    let map_id = i32::from(request.map_id);
    eprintln!(
        "world: {:?} asked for instance {map_id} (district {}, language {}, flag {}) from the world \
         map in map {}",
        player.name,
        request.district,
        request.language,
        request.flag,
        player.instance.map.map_id
    );
    // What the request does to the player's party (the owner's notes, 2026-11-06).
    let (leader, size) = player.instance.parties.leader_and_size(player.player_id);
    let in_outpost = player.instance.map.map_type == map_kinds().main_town;
    let decision =
        asura_rules::party::travel_request(in_outpost, leader == player.player_id, size as usize);
    // A live server does not travel a party member's request at once: it answers `0x01E7 {4}`,
    // the client asks "You will leave your party if you travel there...", and only a confirmed
    // request (flag 0) comes back. `multiplay-live-20261106-200444.jsonl` seq 768-769 and 891-892
    // are the two unconfirmed attempts; seq 924 is the confirmed one that travels.
    if decision == asura_rules::party::TravelRequest::LeavesParty && request.flag == 1 {
        eprintln!("world: party: the travel would leave the party; the client is asked first");
        let answer = game::party_travel_confirmation(4);
        send(stream, session, vec![answer], trace).await?;
        return Ok(true);
    }
    let Some(destination) = resolve_instance(world, map_id).await? else {
        // A statement about this server, not about the client's rights: it has no row, no blob, or a
        // type this world does not host.
        eprintln!("world: instance {map_id} cannot be served by this world; the travel is refused");
        return Ok(true);
    };
    // A leader's world-map travel can leave behind members who have not unlocked the destination.
    // The live client asks first with `0x01E7 {5}` ("your party has members who cannot travel
    // there. Do you wish to leave them behind?"), and the confirmed request (flag 0) moves at once,
    // with no 10-second timer (`multiplay-live-20261106-225615.jsonl` seq 176-177, then seq 181
    // and the lock at seq 190). A normal party keeps the countdown.
    let left_out = if decision == asura_rules::party::TravelRequest::PartyAfterCountdown {
        members_who_cannot_travel(world, &player.instance, player.player_id, destination.map_id)
            .await
    } else {
        Vec::new()
    };
    if !left_out.is_empty() {
        if request.flag == 1 {
            eprintln!(
                "world: party: {} member(s) cannot travel to map {}; the leader is asked first",
                left_out.len(),
                destination.map_id
            );
            let answer = game::party_travel_confirmation(5);
            send(stream, session, vec![answer], trace).await?;
            return Ok(true);
        }
        // The leader confirmed: leave those members behind and move the rest now, without a
        // countdown. `begin_move` sends the party split before the lock, as the capture does.
        let size = crate::party::max_size(destination.map_id);
        match crate::party::begin_move(&player.instance, player.player_id, size, &left_out, trace) {
            Ok(mv) => {
                let instance = player.instance.clone();
                move_party(world, &instance, mv, destination, None, trace).await;
            }
            Err(why) => {
                eprintln!(
                    "world: party: the leader's party does not move as one ({why:?}); the leader \
                     travels alone"
                );
                player.instance.order(
                    player.player_id,
                    Order::Travel {
                        destination,
                        landing: None,
                        companions: Vec::new(),
                    },
                );
            }
        }
        return Ok(true);
    }
    match decision {
        asura_rules::party::TravelRequest::Alone => {
            // A live solo travel is preceded by a six-byte `0x01DA {2, 1, ""}` in the same tick as the
            // redirect (`multiplay-live-20261106-195109.jsonl` seq 1196); the move works without it,
            // so it is not sent here yet.
            begin_transfer(
                stream,
                session,
                world,
                player,
                destination,
                None,
                &[],
                trace,
            )
            .await
        }
        asura_rules::party::TravelRequest::LeavesParty => {
            crate::party::leave_to_travel_grey(player, trace);
            begin_transfer(
                stream,
                session,
                world,
                player,
                destination,
                None,
                &[],
                trace,
            )
            .await
        }
        asura_rules::party::TravelRequest::PartyAfterCountdown => {
            let Some((ticket, seconds)) =
                crate::party::start_countdown(player, destination.map_id, trace)
            else {
                return Ok(true);
            };
            let world = world.clone();
            let instance = player.instance.clone();
            let leader = player.player_id;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(u64::from(seconds))).await;
                if !crate::party::countdown_stands(&instance, leader, ticket) {
                    if trace {
                        eprintln!("world: party: the countdown of ticket {ticket} was voided");
                    }
                    return;
                }
                // The unlock check was made when the leader asked; every member present can follow.
                let size = crate::party::max_size(destination.map_id);
                match crate::party::begin_move(&instance, leader, size, &[], trace) {
                    Ok(mv) => move_party(&world, &instance, mv, destination, None, trace).await,
                    Err(why) => {
                        // Nobody left to take along, or no size for the destination: the leader goes
                        // alone, which is what its own request asked for.
                        eprintln!(
                            "world: party: the leader's party does not move as one ({why:?}); the \
                             leader travels alone"
                        );
                        let companions = Vec::new();
                        instance.order(
                            leader,
                            Order::Travel {
                                destination,
                                landing: None,
                                companions,
                            },
                        );
                    }
                }
            });
            Ok(true)
        }
    }
}

/// The present members of a leader's party who have not unlocked `destination`, in party order.
///
/// The leader's own client offered the destination, so only the others are read. An unreadable
/// unlock list is treated as "not unlocked", which leaves the member behind rather than moving a
/// character the server cannot prove may enter.
async fn members_who_cannot_travel(
    world: &World,
    instance: &Instance,
    leader: u32,
    destination: i32,
) -> Vec<u32> {
    let mut left_out = Vec::new();
    for (member, character) in crate::party::present_characters(instance, leader) {
        if member == leader {
            continue;
        }
        let unlocked = match db_game::unlocked_instances(&world.pool, character).await {
            Ok(instances) => instances.contains(&destination),
            Err(err) => {
                eprintln!("world: party: could not read player {member}'s unlocks: {err}");
                false
            }
        };
        if !asura_rules::party::follows_to_outpost(unlocked) {
            eprintln!(
                "world: party: player {member} has not unlocked map {destination} and stays behind"
            );
            left_out.push(member);
        }
    }
    left_out
}

/// `GAME_CMSG_PARTY_RETURN_TO_OUTPOST` (`0x80A7`): the defeated leader's confirmation. A resigned
/// party is told it is defeated (`0x01D9`); the leader's client then offers the return, and this is
/// the request that follows it (`multiplay-live-20261106-200638.jsonl`: `0x01D9` at t=16.888, then
/// `0x01DA` and the redirect at t=21.500-21.550, with no other client message between them on the
/// captured member's side). The destination is the leader's `last_outpost`, the outpost the character
/// last stood in (`db_game::settings`), so the whole party goes back the way it came.
pub async fn handle_return_to_outpost(
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(party) = crate::party::defeated_party(&player.instance, player.player_id) else {
        eprintln!(
            "world: {:?} asked its party to return, but it leads no defeated party",
            player.name
        );
        return Ok(true);
    };
    let Some(settings) = db_game::settings(&world.pool, party.leader)
        .await
        .map_err(GameError::Database)?
    else {
        eprintln!("world: party: the leader's character row is gone; the return is refused");
        return Ok(true);
    };
    let map_id = i32::from(settings.last_outpost);
    if map_id == 0 {
        eprintln!("world: party: the leader has no last outpost; the return is refused");
        return Ok(true);
    }
    let Some(destination) = resolve_instance(world, map_id).await? else {
        eprintln!("world: party: last outpost {map_id} cannot be served; the return is refused");
        return Ok(true);
    };
    if trace {
        eprintln!(
            "world: party: leader {:?} returns defeated party {} to outpost {map_id}",
            player.name, party.party_id
        );
    }
    // A live return is preceded by `0x01DA {2, 0, ""}` on the member's side
    // (`multiplay-live-20261106-200638.jsonl` seq 84); the move works without it, so it is not sent yet.
    let size = crate::party::max_size(map_id);
    match crate::party::begin_move(&player.instance, player.player_id, size, &[], trace) {
        Ok(mv) => {
            move_party(
                world,
                &player.instance,
                mv,
                destination.clone(),
                None,
                trace,
            )
            .await;
        }
        Err(crate::party::NoMove::Alone) => {
            player.instance.order(
                player.player_id,
                Order::Travel {
                    destination,
                    landing: None,
                    companions: Vec::new(),
                },
            );
        }
        Err(crate::party::NoMove::AlreadyMoving) => {}
        Err(crate::party::NoMove::NoSizeForDestination) => {
            eprintln!(
                "world: party: map {map_id} has no party size; the defeated leader returns alone"
            );
            player.instance.order(
                player.player_id,
                Order::Travel {
                    destination,
                    landing: None,
                    companions: Vec::new(),
                },
            );
        }
    }
    Ok(true)
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

/// A crossing decided from the player's **own step**: where the character stood before this report and
/// where it stands after it.
///
/// This is the rule a doorway wants, and the only one. The world hosts the map's doorways from the pack
/// (`config/World/<pack>/.../portals.json`, [`crate::portals`]), where an arch is the **wall** a character
/// crosses - the owner's rule, 2026-11-05: "the portal is an invisible wall we cross ... should be a
/// plane". So the test is whether the step from the last believed position to this one passes through
/// the wall; touching it, or walking beside it, is not crossing it. A doorway still records its
/// destination, and a wall that names none does nothing and says so.
///
/// The **arrival** belongs to the receiving map: a crossing hands over the destination's own arrival for
/// the map it came from (`portals::arrival`), which `begin_transfer` writes as the character's position
/// before the redirect, and which the arrival connection reads back to stand on - and face along.
pub async fn portal_crossed(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    from: (f32, f32),
    to: (f32, f32),
    trace: bool,
) -> Result<bool, GameError> {
    // A crossing already in flight owns this connection: its seat belongs to the destination and the
    // client is about to reconnect, so a further step must not start a second one. Without this a walk
    // into a door can send two redirects, and a client given two reconnects takes its time to settle.
    if player.moved {
        return Ok(true);
    }
    let instance = player.instance.map.map_id;
    let Some(crossed) = crate::portals::crossed(instance, from, to) else {
        return Ok(true);
    };
    let Some(to_instance) = crossed.to_instance else {
        eprintln!(
            "world: {:?} crossed a doorway of instance {instance} from ({:.0}, {:.0}) to \
             ({:.0}, {:.0}); no destination is recorded for that doorway",
            player.name, from.0, from.1, to.0, to.1
        );
        return Ok(true);
    };
    // The receiving map decides where the traveller stands. An arrival it states for this instance is a
    // position and a facing; without one the destination falls back to its own spawn point.
    let arrival = crate::portals::arrival(to_instance, instance);
    let landing = arrival.map(|arrival| (arrival.x, arrival.y));
    eprintln!(
        "world: {:?} crossed the doorway of instance {instance} at ({:.0}, {:.0}) -> ({:.0}, {:.0}): \
         crossing to {to_instance}{}",
        player.name,
        from.0,
        from.1,
        to.0,
        to.1,
        match arrival {
            Some(arrival) => format!(
                " (arrival at ({:.1}, {:.1}) facing {})",
                arrival.x,
                arrival.y,
                arrival.angle.unwrap_or(0.0)
            ),
            None => " (no arrival stated; the destination's spawn point answers)".to_string(),
        }
    );
    let Some(destination) = resolve_instance(world, to_instance).await? else {
        eprintln!(
            "world: instance {to_instance} cannot be served by this world; the crossing is refused"
        );
        return Ok(true);
    };
    // Any member crossing brings the whole party through (ledger R9): the destination is resolved once,
    // here, and every member's own connection is ordered to go.
    let size = crate::party::max_size(destination.map_id);
    match crate::party::begin_move(&player.instance, player.player_id, size, &[], trace) {
        Ok(mv) => {
            let instance = player.instance.clone();
            move_party(world, &instance, mv, destination, landing, trace).await;
            Ok(true)
        }
        Err(crate::party::NoMove::AlreadyMoving) => Ok(true),
        Err(crate::party::NoMove::Alone) => {
            begin_transfer(
                stream,
                session,
                world,
                player,
                destination,
                landing,
                &[],
                trace,
            )
            .await
        }
        Err(crate::party::NoMove::NoSizeForDestination) => {
            eprintln!(
                "world: party: map {to_instance} has no party size in the pack; {:?} crosses alone",
                player.name
            );
            crate::party::leave_to_travel(player, trace);
            begin_transfer(
                stream,
                session,
                world,
                player,
                destination,
                landing,
                &[],
                trace,
            )
            .await
        }
    }
}

/// The cycle itself: the arriving position, the seat on the destination, the transfer row, and the
/// pair that sends the client on its way.
///
/// Every trigger ends here - `/tp`, the world map's request (`0x80B1`) and a crossing (`0x80C1`) -
/// because the reference has one cycle and not one per trigger.
///
/// `landing` is where the destination puts the arriving character, when the trigger knows: a portal
/// crossing hands over that doorway's landing point (the entrance spawn). Every other travel passes
/// `None`, which **clears** the saved position: a travel does not preserve where a character was, only
/// a disconnect does, and an outpost arrival picks its own spawn point at random regardless
/// (`Standing::arrival`).
///
/// `companions` are the agents of the other members travelling with this one: a live party crossing marks
/// each traveller's agent with the travel effect in every traveller's batch, its own first
/// (`multiplay/party.jsonl` seq 2512-2515: `0x00F1` for agents `0xF1` and `0x4C`, then `0x0028` for its
/// own).
#[allow(clippy::too_many_arguments)]
async fn begin_transfer(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    destination: MapEntry,
    landing: Option<(f32, f32)>,
    companions: &[u32],
    trace: bool,
) -> Result<bool, GameError> {
    let to_map = destination.map_id;

    // A travel the client is told to make needs an address the client can dial; this is a property of
    // the world, so it is checked before anything is written.
    let std::net::SocketAddr::V4(public) = world.public else {
        eprintln!("world: a travel needs an IPv4 advertised address; refused");
        return Ok(true);
    };

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

    // The arriving position is the destination's to decide, and it is written once the travel is
    // committed (a refused travel above writes nothing, so the character keeps the position it is
    // standing in) and before the redirect, so the connection the client is about to open reads it.
    // Best-effort: a failed write must not refuse a travel the player asked for, and a spawn point is
    // the worst case if it does.
    match landing {
        Some((x, y)) => {
            placement::set_transfer_arrival_origin(
                &world.pool,
                player_token,
                player.instance.map.map_id,
            )
            .await
            .map_err(GameError::Database)?;
            if let Err(err) =
                db_game::record_position(&world.pool, player.character, to_map, x, y, 0).await
            {
                eprintln!("world: could not write the landing point for instance {to_map}: {err}");
            }
        }
        None => {
            if let Err(err) = db_game::forget_position(&world.pool, player.character).await {
                eprintln!("world: could not clear the saved position for the travel: {err}");
            }
        }
    }

    // The flag is 0 by default, not the destination's kind, and `explorable_flag_for` says why: at 1 an
    // explorable's load waits 4.3 s here. `ASURA_TRAVEL_EXPLORABLE=auto` restores live's value.
    let is_explorable = explorable_flag_for(destination.map_type);
    crate::mark_redirect(player_token);
    eprintln!(
        "world: [+{} ms] redirect for map {to_map} (player token {player_token:#010X}, \
         is_explorable {is_explorable})",
        crate::stamp_ms()
    );
    let mut frames = vec![
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
    // The party's other travellers, each with the same effect, right after this one's own.
    frames.splice(
        1..1,
        companions
            .iter()
            .map(|agent| game::agent_update_effects(*agent, TRAVEL_EFFECT)),
    );
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
