//! One gameplay connection: hello, load, gameplay, goodbye.
//!
//! The server's `ConnectedClient`, and [`run`], which owns it. A socket is
//! classified, its key exchange is completed, the token pair is claimed against a transfer, the
//! load block goes out unasked, and then [`run`] owns the socket: the world tick, the ping, the
//! client's messages (through `dispatch.rs`) and the broadcasts other players push at it.
//!
//! The player record (`player.rs`) is what a connection drives, and it is where the character, the
//! inventory, the agents and the instance they are in currently live. That is the shape the
//! `Instance` object would replace: the map state belongs to the map, not to the socket. Recorded,
//! not fixed here.

use crate::*;

/// The cadence this world runs at: the tick, the ping and the value a ping is answered with.
///
/// They are read from `config/Server/intervals.json` once per process (`asura_config`, D33), because they
/// were constants in three crates and nothing listed them: the tick is measured from the live client
/// (`recordings/client/feature/movement/movement-sniff.log`: a `0x001E` every ~100 ms, median 81 ms,
/// where the reference's floor is 500 ms), the ping interval is the reference's
/// `GAME_SRV_TIME_BETWEEN_PING_MS`, and the ping value is GWPS's 20, which our client has never been
/// asked about.
pub fn intervals() -> &'static asura_config::Intervals {
    asura_config::intervals()
}

/// How often the world's clock is sent, as this machine's file states it.
pub fn tick_interval() -> Duration {
    intervals().tick()
}

/// How often the server asks the client whether it is still there.
pub fn ping_interval() -> Duration {
    intervals().ping()
}

/// The ping value the server answers with.
pub fn ping_value() -> u32 {
    intervals().ping_value
}

/// Accept gameplay connections until the listener fails.
pub async fn serve(listener: TcpListener, world: Arc<World>, trace: bool) -> io::Result<()> {
    let address = listener.local_addr()?;
    eprintln!(
        "asura-world: listening on {address} for world {} ({} map(s))",
        world.world_id,
        world.maps.len()
    );
    loop {
        let (stream, peer) = listener.accept().await?;
        let world = Arc::clone(&world);
        let connection =
            tokio::spawn(async move { handle_connection(stream, &world, trace).await });
        // The connection is reaped rather than detached. A task that dies of a *panic* leaves this
        // process alive, so from outside "the server crashed" and "one connection panicked" look the
        // same - and the panic's own message is the only thing that says which it was.
        tokio::spawn(async move {
            match connection.await {
                Ok(Ok(())) => {
                    if trace {
                        eprintln!("world: {peer} left");
                    }
                }
                Ok(Err(err)) => eprintln!("world: {peer}: {err}"),
                Err(err) => {
                    if err.is_panic() {
                        eprintln!(
                            "world: {peer}: connection task PANICKED: {}",
                            panic_message(err)
                        );
                    } else {
                        eprintln!("world: {peer}: connection task ended: {err}");
                    }
                }
            }
        });
    }
}

/// The message a panicking task carried, if it carried one a human can read.
pub fn panic_message(err: tokio::task::JoinError) -> String {
    let payload = err.into_panic();
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".to_string())
}

/// One connection's cipher state and the decrypted bytes not yet framed.
pub struct Session {
    pub established: Established,
    pub plaintext: Vec<u8>,
}

impl Session {
    pub fn new(established: Established) -> Self {
        Session {
            established,
            plaintext: Vec::new(),
        }
    }

    /// Take `count` bytes, decrypt them in place, and append them to the plaintext buffer.
    pub fn absorb(&mut self, bytes: &[u8]) {
        let start = self.plaintext.len();
        self.plaintext.extend_from_slice(bytes);
        self.established
            .incoming
            .apply(&mut self.plaintext[start..]);
    }
}

/// Drive one gameplay connection from the first byte to the close.
pub async fn handle_connection(
    mut stream: TcpStream,
    world: &World,
    trace: bool,
) -> Result<(), GameError> {
    // ---- 1. what kind of connection is this? ------------------------------------------------
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await?;
    match version::classify(&header) {
        Ok(version::ConnectionKind::Game) => {}
        Ok(other) => {
            // An auth or control connection belongs to another program. Refusing it by name is the
            // point of classifying before anything else.
            eprintln!("world: a {other:?} connection arrived on the game port; closing it");
            return Err(GameError::NotAGameConnection(
                version::VersionError::Unknown {
                    header: u32::from_le_bytes(header),
                },
            ));
        }
        Err(err) => return Err(GameError::NotAGameConnection(err)),
    }

    // ---- 2. the version message -------------------------------------------------------------
    let mut message = vec![0u8; GAME_CMSG_VERSION_LEN];
    message[..4].copy_from_slice(&header);
    stream.read_exact(&mut message[4..]).await?;
    let (game_version, _) = GameVersion::decode(&message).map_err(GameError::BadVersion)?;
    if trace {
        eprintln!(
            "world <- game version build={} world={} map={} player token={:#010X}",
            game_version.version,
            game_version.map_token,
            game_version.map_id,
            game_version.player_token
        );
    }

    // ---- 3. the token pair is the credential ------------------------------------------------
    let transfer =
        placement::claim_transfer(&world.pool, world.world_id, game_version.player_token)
            .await
            .map_err(GameError::Database)?
            .ok_or(GameError::UnknownTransfer {
                map_token: game_version.map_token,
                player_token: game_version.player_token,
            })?;

    // The character the client presents is the one it is entering with, and the transfer row names the
    // one the session last selected. Those are not always the same: the client chooses its character
    // after login, in a 6-byte message (`0x8023`) this server does not yet read, so the session keeps
    // whatever it had - measured 2026-10-28, the row named Asura Three while the client was entering
    // with Asura One, and every login of Asura One died here as "no pending transfer".
    //
    // What must still hold is the reference's own rule (`AuthSrv_DoPostHandshake`): the character must
    // belong to the account the transfer was issued for. Anything else is refused, and the refusal
    // names both ids so a mismatch is readable instead of inferred.
    // A character-creation connection presents **no** character of its own - the character is what is
    // about to be made - and the client's idea of it can be a character that has just been deleted.
    // Measured 2026-10-28: the version message carried 00000000-... after one delete and the deleted
    // character's own uuid after another, and both were refused here.
    //
    // What must hold is the reference's own rule (`AuthSrv_DoPostHandshake`): the character played must
    // belong to the account the transfer was issued for. So a presented character is used only if it is
    // on that account; anything else - nil, deleted, or someone else's - falls back to the character the
    // transfer itself names, which is on the account by construction. That is not a widening: a client
    // cannot reach a character this way that it could not reach before, it can only be served the one
    // its own token was issued for.
    let presented = Uuid::from_bytes_le(game_version.character);
    let mut character = if presented.is_nil() {
        transfer.character_id
    } else {
        presented
    };
    if character != transfer.character_id {
        let owned = asura_db::login::characters(&world.pool, transfer.account_id)
            .await
            .map_err(GameError::Database)?
            .iter()
            .any(|candidate| candidate.id == character);
        if !owned {
            eprintln!(
                "world: token {:#010X} is for account {}, and character {} is not on it; using the \
                 one the transfer names ({})",
                game_version.player_token, transfer.account_id, character, transfer.character_id
            );
            character = transfer.character_id;
        } else {
            eprintln!(
                "world: token {:#010X} named character {} but the client entered with {}; using the \
                 client's, which is on the same account",
                game_version.player_token, transfer.character_id, character
            );
        }
    }

    let map_id = transfer.map_id;
    // A client whose character stands in a map this world was **not** started with: with `--on-demand`
    // the map is loaded here and the connection goes on, instead of the client being kicked with
    // "this world does not host map X". That is the whole point of the switch - a login into an
    // unhosted map and a /tp into one are the same refusal when it is off, and the same demand-load
    // when it is on.
    let map = match world.map(map_id) {
        Some(map) => map,
        None if world.load_on_demand => {
            match load_map_on_demand(world, map_id)
                .await
                .map_err(GameError::Database)?
            {
                Some(map) => map,
                None => return Err(GameError::UnknownMap(map_id)),
            }
        }
        None => return Err(GameError::UnknownMap(map_id)),
    };

    let Some(name) = db_game::character_name(&world.pool, character)
        .await
        .map_err(GameError::Database)?
    else {
        return Err(GameError::UnknownCharacter);
    };

    // ---- the character's inventory ------------------------------------------------------------
    // A character with no bags has never been played, and the reference gives it the default set before
    // it sends anything (`GameSrv_CreateDefaultBags`). A client with no inventory model dies when it draws
    // the character it was just given, so this happens before the load block and not lazily.
    let mut inventory = db_inventory::load(&world.pool, character)
        .await
        .map_err(GameError::Database)?;
    if inventory.bags.is_empty() {
        let seeded = db_inventory::seed_default(&world.pool, character)
            .await
            .map_err(GameError::Database)?;
        if seeded {
            inventory = db_inventory::load(&world.pool, character)
                .await
                .map_err(GameError::Database)?;
            eprintln!(
                "world: seeded the default inventory for {name:?} ({} bags, {} items)",
                inventory.bags.len(),
                inventory.items.len()
            );
        }
    }

    // Where the character stands: a random point of an outpost's own spawn list, or - for every
    // other map - where it left, if it left from this map (`Standing::arrival`).
    let saved = db_game::position(&world.pool, character)
        .await
        .map_err(GameError::Database)?;
    let standing = Standing::arrival(&map, saved)?;

    eprintln!(
        "world: {name:?} joined map {} (file id {}) on world {}, standing at ({:.1}, {:.1}) plane \
         {} facing ({:.2}, {:.2}) with {} bag(s) and {} item(s)",
        map.map_id,
        map.map_file_id,
        world.world_id,
        standing.position.0,
        standing.position.1,
        standing.plane,
        standing.direction.0,
        standing.direction.1,
        inventory.bags.len(),
        inventory.items.len()
    );

    // ---- 4. the key exchange ----------------------------------------------------------------
    let mut client_seed = vec![0u8; CLIENT_SEED_LEN];
    stream.read_exact(&mut client_seed).await?;
    let mut master_secret = [0u8; MASTER_SECRET_LEN];
    getrandom::getrandom(&mut master_secret).map_err(|err| io::Error::other(err.to_string()))?;
    let established =
        handshake::accept(&client_seed, &master_secret).map_err(GameError::Handshake)?;
    stream.write_all(&established.reply).await?;
    if trace {
        eprintln!("world -> server seed, scrambled from here on");
    }

    let mut session = Session::new(established);
    // What this connection knows about itself: whose character it is, where it stands, and the agents it
    // has created. The ids are allocated here rather than being the constants they used to be: two
    // players in one map need two of each, and everything the reference sends a player about itself
    // carries them.
    let equipped = inventory.equipped_item_ids();
    let (outbox, mut inbox) = tokio::sync::mpsc::unbounded_channel();
    // The map as a running object, not a row: the instance keeps who is in it, owns each player's
    // agent, and starts its clock the first time somebody joins (`Instance::run`).
    let instance = world.instances.for_map(&map);
    let marks = cartography::spawn_marker(outbox.clone());
    let (player_id, agent_id) = instance.join(name.clone(), standing, equipped, outbox);
    let map_type = map.map_type;
    // What this character has explored of the continent it is arriving on. A row whose dims no
    // longer match the grid this server would send is treated as absent rather than reshaped:
    // drawing it would reveal the map in the wrong places with nothing to say so (migration 0027).
    let grid = cartography::grid_for(map.map_id);
    let stored = db_cartography::explored(
        &world.pool,
        character,
        cartography::continent_for(map.map_id),
    )
    .await
    .map_err(GameError::Database)?;
    let explored = stored
        .filter(|row| row.is_self_consistent())
        .and_then(|row| {
            cartography::Grid::from_packed((row.dims_x as u32, row.dims_y as u32), &row.blocks)
        })
        .filter(|saved| saved.dims() == grid)
        .unwrap_or_else(|| cartography::Grid::empty(grid));
    let mut player = Player {
        character,
        player_id,
        agent_id,
        instance,
        name,
        inventory,
        session_id: transfer.session_id,
        account_id: transfer.account_id,
        moved: false,
        last_step: None,
        // A creation instance is its own map type (`MapType_CharacterCreation`, 11): a connection that
        // arrived in one is there to make a character, and one that arrived in anything else cannot.
        creation: if map_type == map_kinds().character_creation {
            Some(Creation::start())
        } else {
            None
        },
        traits: db_game::traits(&world.pool, character)
            .await
            .map_err(GameError::Database)?
            .unwrap_or_default(),
        explored,
        walker: cartography::Walker::new(Some(marks)),
        started: Instant::now(),
    };
    if move_trace_on() {
        eprintln!(
            "move[spawn] agent {} starts on map {} at (x={:.1}, y={:.1}, plane={}) with speed {} \
             (the client will walk from here)",
            player.GetAgentID(),
            player.instance.map.map_id,
            player.GetXY().0,
            player.GetXY().1,
            player.standing().plane,
            game::PLAYER_SPEED_BASE
        );
    }

    // ---- 5. the load block ------------------------------------------------------------------
    // The reference sends these four the moment it has the connection, without waiting to be asked.
    //
    // A creation instance is not loaded, it is a conversation: measured from a live creation
    // (`recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`),
    // the server sends `INSTANCE_LOAD_HEAD` and `CHAR_CREATION_START` and then nothing until the
    // client asks for the player block (`0x8089`, answered in `creation.rs`). The map-load head, the
    // player name and the load info belong to the map path and are not sent here.
    // The saved cells travel in the init on the ordinary path only when
    // `config/Server/cartography.json` says so. The working default is the empty init plus the paced
    // mark replay, because every sparse stored init measured on 2026-11-05 was refused with `007`
    // before the client asked to spawn. A load that is sent the saved grid and never reaches the
    // spawn request is remembered as a refusal, so the next load uses the empty grid.
    let carried = player.creation.is_none()
        && world.replay.is_none()
        && cartography::init_carries_stored()
        && !cartography::init_refused(player.character.as_u128());
    player.walker.init_carried = carried;
    let init_grid = if carried {
        player.explored.clone()
    } else {
        cartography::Grid::empty(player.explored.dims())
    };
    let block = match &world.replay {
        Some(replay) if replay.pick.wants(crate::replay::BlockName::Load) => replay.load.clone(),
        _ if player.creation.is_some() => vec![
            game::instance_load_head_default(),
            game::char_creation_start(),
        ],
        _ => load_block(
            &player.instance.map,
            player.GetName(),
            player.GetAgentID(),
            &init_grid,
        ),
    };
    send(&mut stream, &mut session, block, trace).await?;

    // ---- 6/7. the asks, then the session ----------------------------------------------------
    // Run to its end however it ends. A session that dies of a framing error is still a player that left,
    // and both things that happen on the way out - the position and the pool slot - belong to the leaving
    // and not to the ending: skipping them for the untidy exits would leak a slot until the reaper and
    // lose where the character stood.
    let outcome = run(
        &mut stream,
        &mut session,
        world,
        &mut player,
        &mut inbox,
        trace,
    )
    .await;

    // Where the player was when it left is what it comes back to, read from the instance before the
    // player leaves it. Written *before* the lease goes back, and best-effort: a failed save must not
    // turn an ordinary logout into an error, and losing a position is cheaper than refusing a player
    // its way out.
    //
    // **Unless it travelled.** A travel does not preserve a position - only a disconnect does - so a
    // redirected connection writes nothing: the destination's arrival must not read a coordinate
    // from the map the client is leaving, and a portal crossing has already written the landing it
    // needs when the crossing started (`travel::begin_transfer`).
    let stood = player.standing();
    let left = stood.position;
    let left_plane = stood.plane;
    let stats = player
        .instance
        .move_stats(player.player_id)
        .unwrap_or_default();

    // The player left: the instance loses the agent before the slot goes back, so a client that is
    // being told about the departure never sees an agent whose seat is already free.
    if player.instance.leave(player.player_id) {
        player
            .instance
            .broadcast_all(&farewell(player.GetPlayerNumber(), player.GetAgentID()));
    }

    // A creation instance is not a place, so it is not a position: a character made there has been
    // written with the outpost it starts in (`creation::handle_confirm`), and a connection that
    // arrives in the creation map without making a character is one whose client still believes it is
    // there. Saving this map would overwrite the outpost with the instance and strand the character -
    // measured 2026-10-28: every character that entered map 0 came back to map 0 on the next login.
    if !player.moved && player.instance.map.map_type != map_kinds().character_creation {
        if let Err(err) = db_game::record_position(
            &world.pool,
            player.character,
            player.instance.map.map_id,
            left.0,
            left.1,
            left_plane as i16,
        )
        .await
        {
            eprintln!("world: could not save the position of the leaving player: {err}");
        }
    }

    // A load that was sent the saved grid and closed before the client asked to spawn is what a
    // refused init stream looks like from here. Remember it, so this character's next load is sent
    // the empty grid: one `007`, then the working path, never a lockout.
    if player.walker.init_carried && !player.walker.loaded {
        cartography::note_init_refused(player.character.as_u128());
        eprintln!(
            "world: cartography: {}'s load ended before it spawned; its next load gets the empty grid",
            player.GetName()
        );
    }
    // The exploration grid, beside the position and for the same reason: it is this character's
    // progress, and this session is where it changed. Saved even when the player was moved, because
    // exploration is not a place. A creation instance is excluded for the same reason the position is.
    if player.instance.map.map_type != map_kinds().character_creation {
        player::save_explored(world, &player).await;
    }

    // The player left. The slot goes back to the pool now rather than when a reaper notices: the
    // player is gone, and a pool that only empties on a reaper would refuse a legitimate travel after
    // an ordinary logout.
    //
    // **Unless it was moved.** `/tp` reserves the *destination*'s seat for this session in the same
    // transaction that gives up this one, so the only lease the session holds by the time this runs is
    // the one the client is about to arrive on. Releasing it would free a seat a player is standing in -
    // the reference's own mirror of this race is recorded in `PreSearing/docs/08-gm-commands.md`.
    if player.moved {
        eprintln!(
            "world: {:?} was moved to map {}; the seat it holds is the destination's",
            player.name, player.instance.map.map_id
        );
    } else if let Err(err) = placement::release(&world.pool, transfer.session_id).await {
        eprintln!("world: could not release the lease of the leaving player: {err}");
    }
    eprintln!(
        "world: {:?} left map {} from ({:.1}, {:.1}) plane {left_plane}, {}",
        player.name,
        player.instance.map.map_id,
        left.0,
        left.1,
        if player.moved {
            "not saved (the connection was redirected to another map)"
        } else {
            "saved"
        }
    );
    if move_trace_on() {
        eprintln!(
            "move[summary] {} walk request(s) (pathfinding), {} position report(s) (manual walk): \
             {} accepted, {} clamped, {} refused, {} plane change(s); {} tick(s) walked, \
             largest client/server gap {:.1} units",
            stats.walk_requests,
            stats.reports,
            stats.accepted,
            stats.clamped,
            stats.refused,
            stats.plane_changes,
            stats.ticks,
            stats.max_drift
        );
    }
    outcome
}

/// The conversation itself: answer what arrives until the client goes away, a message cannot be framed,
/// or the socket fails.
pub async fn run(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    inbox: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<Frame>>,
    trace: bool,
) -> Result<(), GameError> {
    // The instance's clock is **not** released here. It is released by the players block, whose last
    // message is `INSTANCE_LOAD_FINISH` (`world::handle_request_players`), and the wire is why: in 36
    // loads of five captures against live servers, not one `0x001E` reached a client between its spawn
    // request and its players request, while this world sent 49 of them into exactly that window. The
    // client is inside its loading screen there and does not read, so every tick written at it is work
    // queued for the moment it comes back - a clock inside a load is a clock it was not built to take.
    let mut ping = tokio::time::interval(ping_interval());
    // The walking reveal, on the world-tick cadence. A server-side walk is advanced by the instance
    // tick and sends no position reports until it ends, so the reports alone would reveal nothing
    // until the arrival. This polls the server's own copy of the position and does nothing else:
    // the doorway test keeps its own trigger in the movement arms.
    let mut walk = tokio::time::interval(tick_interval());
    let mut walked_from = player.standing().position;
    let mut buffer = [0u8; 8192];

    'session: loop {
        tokio::select! {
            biased;

            // Frames another player's connection pushed at us. This is the whole of the broadcast
            // mechanism: the reference writes into every other connection's buffer, and here every
            // other connection's task writes into this channel.
            Some(frames) = inbox.recv() => {
                send(stream, session, frames, trace).await?;
            }

            read = stream.read(&mut buffer) => {
                let read = read?;
                if read == 0 {
                    break; // the client closed its side
                }
                session.absorb(&buffer[..read]);

                loop {
                    // Framed by the generated game table, field by field: a counted field carries its
                    // own length, so a message is exactly as long as it says. What arrives is framed
                    // before it is answered, and a header the table does not know ends the session
                    // rather than consuming the wrong number of bytes.
                    match Frame::decode_game_message(&session.plaintext, true) {
                        Ok((frame, used)) => {
                            session.plaintext.drain(..used);
                            crate::capture::record("CToS", frame.header(), &frame.encode());
                            if trace {
                                eprintln!(
                                    "world <- {:#06X} ({used} bytes): {}",
                                    frame.header(),
                                    crate::capture::hex_preview(&frame.encode())
                                );
                            }
                            if !answer(stream, session, frame, world, player, trace).await? {
                                break 'session;
                            }
                        }
                        Err(FrameError::Incomplete { .. }) => break,
                        Err(err) => return Err(GameError::Framing(err)),
                    }
                }
            }

            _ = ping.tick() => {
                send(stream, session, vec![game::ping_request()], trace).await?;
            }

            _ = walk.tick() => {
                let here = player.standing().position;
                if here != walked_from {
                    walked_from = here;
                    player::walked(world, player, trace).await;
                }
            }
        }
    }
    Ok(())
}

/// Scramble and write frames.
pub async fn send(
    stream: &mut TcpStream,
    session: &mut Session,
    frames: Vec<Frame>,
    trace: bool,
) -> Result<(), GameError> {
    for frame in frames {
        let mut bytes = frame.encode();
        crate::capture::record("StoC", frame.header(), &bytes);
        if trace {
            eprintln!("world -> {:#06X} ({} bytes)", frame.header(), bytes.len());
        }
        session.established.outgoing.apply(&mut bytes);
        stream.write_all(&bytes).await?;
    }
    Ok(())
}

/// An item and where it is being put: the three fields `GAME_CMSG_ITEM_MOVE` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub item_id: i32,
    pub bag_id: i32,
    pub slot: i32,
}

/// The client asks to be pinged back.
pub async fn handle_ping_request(
    stream: &mut TcpStream,
    session: &mut Session,
    trace: bool,
) -> Result<bool, GameError> {
    send(stream, session, vec![game::ping_reply(ping_value())], trace).await?;
    Ok(true)
}
