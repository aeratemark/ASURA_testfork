//! Chat: spoken lines, and the command table they arrive through.
//!
//! The reference has no chat handler; it prints and answers into its own log
//! (`docs/HANDOFF.md`). This module is that half, plus the `!`/`#`/`$` channel rule and the
//! `/maps`, `/stuck` and `/tp` command table ([`Command`], [`command`]) a player types into a
//! chat line. The travel `/tp` starts lives in `travel.rs`; this module only decides which
//! command a line names.

use crate::*;

/// The reference's `/maps`, line for line (`GmCommand_ListTargets`).
///
/// It prints two things and they are **not the same text**, which is the part I had wrong until a client
/// tester pointed out that the list no longer showed every map:
///
/// * the **chat** side is one alias per map -- the first in the table, because that is the one to type --
///   comma-separated and wrapped inside one chat fragment, then the reference's fixed
///   "`Use: /tp <name>, e.g. /tp lakeside county`";
/// * the **log** side is **every alias**, one row each, with the map it names, in the reference's own column
///   widths and in its table order:
///
/// ```text
/// Teleport targets (20 aliases, every one with a map config):
///   /tp ascalon          -> map 148  Ascalon City (pre-Searing)
///   /tp ascalon city     -> map 148  Ascalon City (pre-Searing)
/// ```
///
/// The header's count is the **alias** count, not the map count, and the loop below prints a row per alias:
/// collapsing it to one row per map (which is what the chat half does) is how `/maps` stopped naming the
/// aliases a tester has to type.
///
/// The table is what a person reads, and it is the half that has always worked: no chat text has ever
/// reached a client's screen, in the reference or here, and GWPS's own chat gate leaves the chat half in its
/// log with the rest. So this returns the log lines, in that format, and `/maps` prints them. The chat
/// fragments are built too, because they are the answer's other half and a caller may one day have a channel
/// that renders them -- but `/maps` does not send them: sending chat crashed a live client here.
///
/// Only the aliases of maps **this world hosts** are listed, so a narrowed `--maps` does not advertise a
/// target `/tp` would refuse. With the default (the whole catalogue) that is the reference's whole table.
/// A map the reference's table does not name gets one row of its own, named by its id, rather than vanishing.
pub fn maps_answer(maps: &[MapEntry]) -> MappedMaps {
    let hosted: Vec<i32> = maps.iter().map(|map| map.map_id).collect();

    // The rows, in the reference's table order: one per alias.
    let mut rows: Vec<(String, i32, String)> = Vec::new();
    let mut chat: Vec<String> = Vec::new();
    let mut seen: Vec<i32> = Vec::new();
    for target in travel_targets::TRAVEL_TARGETS {
        if !hosted.contains(&target.map_id) {
            continue;
        }
        rows.push((
            target.alias.to_string(),
            target.map_id,
            target.label.to_string(),
        ));
        if !seen.contains(&target.map_id) {
            seen.push(target.map_id);
            chat.push(target.alias.to_string());
        }
    }
    for map in maps {
        // A creation instance is not a place: it is where a character is made, it has no alias, and
        // nobody travels to it. It stays out of the teleport table.
        if map.map_type == crate::map_kinds().character_creation {
            continue;
        }
        if seen.contains(&map.map_id) {
            continue;
        }
        seen.push(map.map_id);
        let alias = map.alias();
        chat.push(alias.clone());
        rows.push((alias, map.map_id, map.name.clone()));
    }

    // The chat line, split the way the reference splits it: `100 <= used + needed` starts a continuation
    // line indented by six spaces, each entry followed by a comma.
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::from("Maps:");
    for alias in &chat {
        if 100 <= current.len() + alias.len() + 3 {
            lines.push(current);
            current = String::from("      ");
        }
        current.push_str(&format!(" {alias},"));
    }
    lines.push(current);
    // The reference's own sentence, literal: it names `lakeside county` whatever is in the table.
    lines.push("Use: /tp <name>, e.g. /tp lakeside county".to_string());

    // The table the log gets: the alias count in the header, then every alias.
    let mut table = vec![format!(
        "Teleport targets ({} aliases, every one with a map config):",
        rows.len()
    )];
    for (alias, map_id, label) in &rows {
        table.push(format!("  /tp {alias:<15} -> map {map_id:>3}  {label}"));
    }

    MappedMaps { chat: lines, table }
}

/// What `/maps` produces: the lines the reference would put on the chat channel, and the table it puts in
/// its log. Two shapes, because the reference has two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedMaps {
    pub chat: Vec<String>,
    pub table: Vec<String>,
}

/// `UPDATE_AGENT_VISUAL_EQUIPMENT`: what the agent is wearing, read out of the equipped-items bag.
/// How a map alias is compared: lower case, runs of spaces collapsed. The reference's own rule for
/// `/tp` (`GmCommand_ToAscii`).
pub fn alias_key(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// One chat command this world answers.
///
/// The table is the single place the command surface is written down, and the handler dispatches through
/// it, so `tools/parity-commands.py` comparing it with the reference cannot drift from what the code does.
/// The reference's own surface is two functions: `GameSrv_TryHandleGmCommand` (`GmCommands.c`, which owns
/// `/tp`, `/teleport` and `/maps`) and `GameSrv_HandleChatCommand` (`GmChat.c`, which owns `/stuck`).
#[derive(Debug, Clone, Copy)]
pub struct Command {
    /// What a player types, without the slash. `GmCommands.c` accepts two names for one command.
    pub names: &'static [&'static str],
    /// Where the reference's own dispatch handles it, for a reader who wants to check.
    pub reference: &'static str,
    /// Whether the command takes an argument (`GameSrv_TryHandleGmCommand` splits on the first space).
    pub takes_argument: bool,
    /// What answering it does.
    pub answer: &'static str,
}

/// Every chat command this world answers, in the reference's order.
pub const COMMANDS: &[Command] = &[
    Command {
        names: &["tp", "teleport"],
        reference: "GmCommands.c:184 GameSrv_TryHandleGmCommand -> GameSrv_HandleTeleportCommand",
        takes_argument: true,
        answer: "a server transfer, or one of the reference's three refusals (unknown map, no config, already here)",
    },
    Command {
        names: &["maps"],
        reference: "GmCommands.c:206 GameSrv_TryHandleGmCommand -> GmCommand_ListTargets",
        takes_argument: false,
        answer: "the alias table, printed: the reference's chat half is gated off, so nothing goes on the wire",
    },
    Command {
        names: &["stuck"],
        reference: "GmChat.c:243 GameSrv_HandleChatCommand",
        takes_argument: false,
        answer: "AGENT_UPDATE_POSITION for the agent -- a position, not AGENT_STOP_MOVING",
    },
    Command {
        names: &["title"],
        reference: "none -- an ASURA test command (owner, 2026-11-05), not in the reference",
        takes_argument: true,
        answer: "sets a title's points: 0x00F5, or 0x00F3 x2 then 0x00F6, as a point change sends them",
    },
    Command {
        names: &["guild"],
        reference: "none -- an ASURA command (owner, 2026-11-06), standing in for the Guild Registrar",
        takes_argument: true,
        answer: "`/guild create` opens the Guild Registration window (0x00C4, 0x0084, 0x00CA, 0x00C3 {8, 0})",
    },
    Command {
        names: &["resign"],
        reference: "the client's own /resign chat line; the live server's batch is `multiplay-live-20261106-200638.jsonl`",
        takes_argument: false,
        answer: "the member resigns, then dies; when every member has resigned the party is defeated (0x01D9) and the leader's 0x80A7 returns it to its last outpost",
    },
];

/// The command a player typed, if this world answers it.
pub fn command(name: &str) -> Option<&'static Command> {
    COMMANDS
        .iter()
        .find(|candidate| candidate.names.contains(&name))
}

/// The reference's `/tp` lookup: normalise the argument, find it in its table.
///
/// `GmCommand_ToAscii` refuses anything that is not printable ASCII, which is why a non-ASCII argument is
/// an unknown command rather than a fuzzy match; and the table is exact, so `lakesidecounty` is not
/// `lakeside county` even though the reference's own comment says otherwise.
pub fn travel_target(argument: &str) -> Option<&'static travel_targets::TravelTarget> {
    let wanted = alias_key(argument);
    if wanted.is_empty() {
        return None;
    }
    travel_targets::TRAVEL_TARGETS
        .iter()
        .find(|target| target.alias == wanted)
}

/// The client speaks (`GameSrv_HandleChatMessage`). The first unit of the message is the channel:
/// `!` is local/general, `#` group, `$` trade, `/` a command, and the reference answers anything
/// else with bad user data.
///
/// **A player's message is sent ungated**, exactly as the reference sends it: its
/// `GameSrv_ChatSendingEnabled` gate covers the text the *server* generates (GM answers, "unknown
/// command"), not a line a player typed. This port once had a switch on everything, which is why
/// a `!` line went nowhere; the switch is gone (`docs/03` #11), so nothing here is gated and only
/// the messages the reference never sends are withheld.
pub async fn handle_chat_message(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let body = &frame.body();
    if body.len() < 6 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 8 }));
    }
    let units = u16::from_le_bytes([body[4], body[5]]) as usize;
    let text: Vec<u16> = (0..units)
        .map(|index| {
            let at = 6 + index * 2;
            u16::from_le_bytes([
                body.get(at).copied().unwrap_or(0),
                body.get(at + 1).copied().unwrap_or(0),
            ])
        })
        .collect();
    let Some(channel) = text.first().copied() else {
        return Ok(true); // an empty message, which the reference drops
    };
    let spoken: String = char::decode_utf16(text[1.min(text.len())..].iter().copied())
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();

    match u8::try_from(channel).unwrap_or(0) as char {
        // `Channel_All`, `Channel_Group`, `Channel_Trade`: the three channels the reference
        // speaks on. Sent ungated, as the reference sends them. None of the three chat messages
        // appears in any capture, and no chat text has ever rendered on screen in either server
        // -- so what this path matches is the reference's *bytes*, not its visible result.
        speaker @ ('!' | '#' | '$') => {
            let channel = match speaker {
                '#' => game::CHANNEL_GROUP,
                '$' => game::CHANNEL_TRADE,
                _ => game::CHANNEL_ALL,
            };
            if trace {
                eprintln!("world: {speaker} {spoken:?} on channel {channel}");
            }
            // The body is a coded string (`game::player_chat_line`): plain text hit the client's
            // coded-string assert, which is why no chat line had ever rendered here. Everyone in the
            // instance hears `!` and `$`; `#` is the party channel, and a party here is the speaker
            // alone, so it goes back to the speaker only.
            let line = game::player_chat_line(player.player_id, channel, &spoken);
            if speaker != '#' {
                player.instance.broadcast(player.player_id, &line);
            }
            send(stream, session, line, trace).await?;
            Ok(true)
        }
        '/' => {
            // The command surface, from the reference's own two places: `GmCommands.c`'s
            // `GameSrv_TryHandleGmCommand` (`/tp`, `/teleport`, `/maps`) and `GmChat.c`'s
            // `GameSrv_HandleChatCommand` (`/stuck`). `COMMANDS` names them once, and
            // `tools/parity-commands.py` compares that list with the reference's so the surface
            // cannot drift without a check failing.
            let spoken = spoken.trim();
            let (name, argument) = match spoken.split_once(' ') {
                Some((name, rest)) => (name, rest.trim_start()),
                None => (spoken, ""),
            };
            match command(name) {
                Some(named) if named.names.contains(&"stuck") && argument.is_empty() => {
                    // `GameSrv_HandleChatCommand`'s `stuck`: cancel the movement and broadcast where
                    // the agent is now. **Not** `AGENT_STOP_MOVING` -- that is what
                    // `HandleCancelMovement` (`0x8028`) answers, and the reference sends a position
                    // update here instead. It also broadcasts a world tick first; the 500 ms ticker
                    // covers that, so no second clock is invented.
                    let Some(agent) = player.GetAgent() else {
                        return Ok(true);
                    };
                    if trace {
                        eprintln!(
                            "world: /stuck -- agent {} stands at ({:.1}, {:.1}) plane {}",
                            agent.agent_id, agent.position.0, agent.position.1, agent.plane
                        );
                    }
                    send(
                        stream,
                        session,
                        vec![game::agent_update_position(
                            agent.agent_id,
                            agent.position,
                            agent.plane,
                        )],
                        trace,
                    )
                    .await?;
                    Ok(true)
                }
                Some(named) if named.names.contains(&"maps") && argument.is_empty() => {
                    // `GmCommands_ListTargets`, as it behaves with the reference's own defaults: the
                    // table goes to the log, and nothing goes on the wire. That is parity, not a
                    // gate -- GWPS's chat gate is what leaves its chat half in its log too, and no
                    // chat text has ever reached a screen in either server. Sending it here crashed
                    // a live client, which is why the answer is printed and not sent.
                    let mut listed = world.all_maps();
                    if world.load_on_demand {
                        // On-demand: every catalogue map is one this world will serve
                        // (it loads it on first use), so /maps answers the reference's
                        // table even before a map has been used. One query per map that
                        // has not been loaded yet; /maps is a console command, not a hot
                        // path.
                        if let Ok(ids) = db_game::configured_map_ids(&world.pool).await {
                            for map_id in ids {
                                if listed.iter().any(|map| map.map_id == map_id) {
                                    continue;
                                }
                                if let Ok(Some(configured)) =
                                    db_game::spawn_map(&world.pool, map_id).await
                                {
                                    listed.push(MapEntry {
                                        map_id,
                                        map_file_id: u32::try_from(configured.map_file_id)
                                            .unwrap_or(0),
                                        map_type: configured.map_type,
                                        spawn: (configured.spawn_x, configured.spawn_y),
                                        spawn_plane: configured.spawn_plane as u16,
                                        spawns: crate::map_spawns::for_map(map_id)
                                            .map(<[_]>::to_vec)
                                            .unwrap_or_default(),
                                        name: configured.name.clone(),
                                        capacity: world.capacity,
                                    });
                                }
                            }
                        }
                    }
                    let answer = maps_answer(&listed);
                    for line in &answer.table {
                        eprintln!("world: {line}");
                    }
                    if trace {
                        for line in &answer.chat {
                            eprintln!("world: /maps (chat, not sent): {line}");
                        }
                    }
                    Ok(true)
                }
                Some(named) if named.names.contains(&"tp") && !argument.is_empty() => {
                    // `GameSrv_HandleTeleportCommand`: a *server transfer*, which is what the game
                    // itself does when a portal is walked through. Every byte of the redirect is
                    // measured (`docs/03-protocol.md` #5) and the reference's refusals are here
                    // verbatim, so this is a port rather than an invention.
                    teleport(stream, session, world, player, argument, trace).await
                }
                Some(named) if named.names.contains(&"title") && !argument.is_empty() => {
                    // Not the reference's: the owner's test command for titles (`crate::titles`).
                    crate::titles::handle_title_command(
                        stream, session, world, player, argument, trace,
                    )
                    .await
                }
                Some(named) if named.names.contains(&"guild") && !argument.is_empty() => {
                    // Not the reference's: the owner's stand-in for the Guild Registrar (`crate::guild`).
                    crate::guild::handle_guild_command(stream, session, player, argument, trace)
                        .await
                }
                Some(named) if named.names.contains(&"resign") && argument.is_empty() => {
                    // `/resign` is a chat line in the client, not a GAME_CMSG. Every member types
                    // its own; each dies after the pack's delay, and the last death is what sends
                    // `0x01D9` to the party (`docs/45-party-travel.md`).
                    crate::party::resign(
                        player.instance.clone(),
                        player.player_id,
                        player.agent_id,
                        trace,
                    );
                    Ok(true)
                }
                _ => {
                    // `GameSrv_SendInvalidCommand`: the reference's text is "Unknown command". That
                    // covers `/tp` with no map name (`GmCommand_ToAscii` refuses the empty
                    // argument) and every name that is not in the table; with its chat gate off
                    // it is a log line and nothing else, and so it is here.
                    eprintln!("world: Unknown command");
                    Ok(true)
                }
            }
        }
        other => {
            // `@`, `%` and `"` are ignored by the reference; anything else it refuses.
            if matches!(other, '@' | '%' | '"') {
                if trace {
                    eprintln!("world: channel {other:?} ignored, as the reference ignores it");
                }
                return Ok(true);
            }
            eprintln!("world: chat on channel {other:?} refused, as the reference refuses it");
            Ok(true)
        }
    }
}
