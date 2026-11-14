//! Chat: spoken lines, and the command table they arrive through.
//!
//! The reference has no chat handler; it prints and answers into its own log
//! (`docs/HANDOFF.md`). This module is that half, plus the `!`/`#`/`$` channel rule, the `"` whisper
//! and the `/maps`, `/stuck` and `/tp` command table ([`Command`], [`command`]) a player types into a
//! chat line. The travel `/tp` starts lives in `travel.rs`; this module only decides which
//! command a line names. The party channel, the whisper, `/roll` and `/age` follow preforged
//! (`docs/51-compass-and-chat.md`).

use crate::*;

/// The file inside a pack that holds the chat rules.
pub const CHAT_FILE: &str = "chat.json";

/// The values the chat commands decide by: the pack's `chat.json`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct ChatRules {
    /// Free text for whoever opens the file.
    #[serde(default)]
    pub note: Option<String>,
    /// The fewest sides `/roll` accepts.
    pub roll_min_sides: crate::character_defaults::Value<u32>,
    /// The most sides `/roll` accepts.
    pub roll_max_sides: crate::character_defaults::Value<u32>,
    /// The most characters a whisper's recipient name may have.
    pub whisper_name_chars: crate::character_defaults::Value<usize>,
}

/// Read `<pack>/chat.json`.
pub fn load_chat_rules(pack: &std::path::Path) -> Result<ChatRules, String> {
    let path = pack.join(CHAT_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("{} is not readable: {err}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|err| format!("{} is not a chat rules file: {err}", path.display()))
}

/// The chat rules in force, read once per process. A missing or broken file is logged once and answers
/// `None`: `/roll` and a whisper are then refused with a logged reason.
pub fn chat_rules() -> Option<&'static ChatRules> {
    static IN_FORCE: std::sync::OnceLock<Option<ChatRules>> = std::sync::OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let pack = crate::spawns::pack();
            match load_chat_rules(&pack) {
                Ok(rules) => {
                    eprintln!("asura: chat rules from {}", pack.display());
                    Some(rules)
                }
                Err(err) => {
                    eprintln!("asura: {err}; /roll and whispers are refused");
                    None
                }
            }
        })
        .as_ref()
}

/// A whisper line's two halves: the client sends `<name>,<message>` after the `"` channel mark. `None`
/// when either half is empty or the name is longer than `name_chars`.
pub fn whisper_parts(line: &str, name_chars: usize) -> Option<(&str, &str)> {
    let (name, message) = line.split_once(',')?;
    let (name, message) = (name.trim(), message.trim());
    let fits = !name.is_empty() && name.chars().count() <= name_chars;
    (fits && !message.is_empty()).then_some((name, message))
}

/// `/roll`'s argument as a number of sides within `rules`, or `None`.
pub fn roll_sides(argument: &str, rules: &ChatRules) -> Option<u32> {
    let sides: u32 = argument.parse().ok()?;
    (rules.roll_min_sides.value..=rules.roll_max_sides.value)
        .contains(&sides)
        .then_some(sides)
}

/// A die's face for 32 random bits: 1 to `sides`. `sides` is never 0 (`roll_sides`).
pub fn roll_face(random: u32, sides: u32) -> u32 {
    1 + random % sides
}

/// One line of the server's own, in the player's chat log and in the console.
pub(crate) fn tell(player: &Player, line: &str) -> Vec<Frame> {
    eprintln!("world: chat: to {:?}: {line}", player.name);
    game::server_chat_line(player.player_id, game::CHANNEL_GLOBAL, line)
}

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
        names: &["item"],
        reference: "none -- an ASURA test command, not in the reference",
        takes_argument: true,
        answer: "gives one live-verified catalogue definition (0x0161 then 0x014B) in the next free carrying slot; `/item clear` empties the carrying bags (0x014D per item)",
    },
    Command {
        names: &["resign"],
        reference: "the client's own /resign chat line; the live server's batch is `multiplay-live-20261106-200638.jsonl`",
        takes_argument: false,
        answer: "the member resigns, then dies; when every member has resigned the party is defeated (0x01D9) and the leader's 0x80A7 returns it to its last outpost",
    },
    Command {
        names: &["roll"],
        reference: "none -- preforged's /roll (gw_chat_commands.h), not in the reference",
        takes_argument: true,
        answer: "one line on the emote channel to the whole instance: `<name> rolls <face> on a <sides>-sided die.`; the bounds are the pack's chat.json",
    },
    Command {
        names: &["age"],
        reference: "none -- preforged's /age (gw_chat_commands.h), not in the reference",
        takes_argument: false,
        answer: "a server line to the asker: minutes this connection has been in this map",
    },
    Command {
        names: &["skill"],
        reference: "none -- an ASURA test command (owner, 2026-11-12), standing in for the quests and trainers that teach skills",
        takes_argument: true,
        answer: "`/skill list`, `/skill learn <id|all>`, `/skill forget <id>`: the learned set (0x00DB) with 0x00DC or 0x00DD per skill, and a server line",
    },
    Command {
        names: &["npc"],
        reference: "none -- an ASURA test command, not in the reference",
        takes_argument: true,
        answer: "`/npc list` prints each placement's body, route and held items; `/npc patrol <placement|all>` starts an authored route; `/npc stop <placement|all>` halts a body (0x0028 on the next tick)",
    },
    Command {
        names: &["xunlai"],
        reference: "none -- an ASURA test command, standing in for a Xunlai NPC on maps that have none (docs/53-npc-services.md)",
        takes_argument: false,
        answer: "opens the Xunlai storage window on the player's own agent (0x0083 {agent, 0, 3}); storage moves and gold moves are then accepted",
    },
    Command {
        names: &[".exp"],
        reference: "preforged's /.exp (gw_development_commands.h), an ASURA test command",
        takes_argument: true,
        answer: "sets total experience; the level follows the pack's curve, and the progress block (0x00E9), public level (0x009F 36) and attribute points (0x0037) are re-sent",
    },
    Command {
        names: &["xp"],
        reference: "preforged's /xp (gw_chat_commands.h), an ASURA test command",
        takes_argument: true,
        answer: "with a total, sets experience like /.exp; without one, a server line with the total and level",
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
            // instance hears `!` and `$`; `#` is the party channel, heard by the speaker's party
            // members standing in the instance.
            let line = game::player_chat_line(player.player_id, channel, &spoken);
            if speaker == '#' {
                crate::compass::party_send(&player.instance, player.player_id, &line, false);
            } else {
                player.instance.broadcast(player.player_id, &line);
            }
            send(stream, session, line, trace).await?;
            Ok(true)
        }
        '"' => {
            // A whisper: `<name>,<message>`. Delivered on the whisper channel to a player standing
            // in this instance, as preforged delivers it; `CHAT_MESSAGE_LOCAL` names the speaker by
            // an id that only means something inside one instance, so a recipient anywhere else is
            // refused with a line of the server's own rather than guessed at.
            let answer = match chat_rules()
                .and_then(|rules| whisper_parts(&spoken, rules.whisper_name_chars.value))
            {
                None => tell(player, "Usage: /whisper <character name>, <message>"),
                Some((name, message)) => match player.instance.player_named(name) {
                    None => tell(
                        player,
                        &format!(
                            "{name} is not in this map; a whisper only reaches a player here."
                        ),
                    ),
                    Some(recipient) => {
                        if trace {
                            eprintln!("world: {:?} whispers to player {recipient}", player.name);
                        }
                        player.instance.send_to(
                            recipient,
                            &game::player_chat_line(
                                player.player_id,
                                game::CHANNEL_WHISPER,
                                message,
                            ),
                        );
                        tell(player, &format!("Whisper sent to {name}."))
                    }
                },
            };
            send(stream, session, answer, trace).await?;
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
                    let mut frames = Vec::new();
                    for line in &answer.table {
                        eprintln!("world: {line}");
                        frames.extend(game::server_chat_line(
                            player.player_id,
                            game::CHANNEL_GLOBAL,
                            line,
                        ));
                    }
                    if trace {
                        for line in &answer.chat {
                            eprintln!("world: /maps (chat, not sent): {line}");
                        }
                    }
                    send(stream, session, frames, trace).await?;
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
                Some(named) if named.names.contains(&"item") && argument == "clear" => {
                    // A test helper: empty the carrying bags (the measured trash answer per item),
                    // so a catalogue sweep does not run out of slots.
                    crate::item::clear_carrying(stream, session, world, player, trace).await?;
                    Ok(true)
                }
                Some(named) if named.names.contains(&"item") && !argument.is_empty() => {
                    // Verified definitions only: the command was never the crash, the
                    // unverified item ids were. Send the measured create/move pair.
                    crate::item::grant_definition(stream, session, world, player, argument, trace)
                        .await?;
                    Ok(true)
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
                Some(named) if named.names.contains(&"roll") => {
                    // Preforged's `/roll`: one line on the emote channel, heard by the whole
                    // instance, with the face chosen here. The bounds are the pack's.
                    let Some(sides) = chat_rules().and_then(|rules| roll_sides(argument, rules))
                    else {
                        let usage = match chat_rules() {
                            Some(rules) => format!(
                                "Usage: /roll <sides>, from {} to {}.",
                                rules.roll_min_sides.value, rules.roll_max_sides.value
                            ),
                            None => {
                                "Rolls are unavailable: the pack has no chat rules.".to_string()
                            }
                        };
                        send(stream, session, tell(player, &usage), trace).await?;
                        return Ok(true);
                    };
                    let face = roll_face(crate::random_u32()?, sides);
                    let line = game::player_chat_line(
                        player.player_id,
                        game::CHANNEL_EMOTE,
                        &format!("{} rolls {face} on a {sides}-sided die.", player.name),
                    );
                    player.instance.broadcast(player.player_id, &line);
                    send(stream, session, line, trace).await?;
                    Ok(true)
                }
                Some(named) if named.names.contains(&"age") && argument.is_empty() => {
                    // Preforged's `/age`, the part this server can answer: how long this connection
                    // has been in this map. Character and account age are not kept.
                    let minutes = player.started.elapsed().as_secs() / 60;
                    let line = format!("You have been in this map for {minutes} minute(s).");
                    send(stream, session, tell(player, &line), trace).await?;
                    Ok(true)
                }
                Some(named) if named.names.contains(&"skill") => {
                    // Not the reference's: the owner's test command for skills (`crate::skill`).
                    crate::skill::character::handle_skill_command(
                        stream, session, world, player, argument, trace,
                    )
                    .await
                }
                Some(named) if named.names.contains(&"xunlai") && argument.is_empty() => {
                    // Not the reference's: the owner's test command for storage (`crate::vendor`).
                    crate::vendor::open_own_storage(stream, session, player, trace).await
                }
                Some(named) if named.names.contains(&".exp") && !argument.is_empty() => {
                    // Preforged's /.exp: set the total, and let the curve answer the level.
                    let Ok(total) = argument.parse::<u32>() else {
                        eprintln!("world: /.exp: {argument:?} is not a total experience");
                        let line = "Usage: /.exp <total experience>, e.g. /.exp 2000";
                        send(stream, session, tell(player, line), trace).await?;
                        return Ok(true);
                    };
                    crate::player::set_experience(stream, session, world, player, total, trace)
                        .await
                }
                Some(named) if named.names.contains(&"xp") => {
                    // Preforged's /xp is the reading half; with an argument it sets, like /.exp.
                    if argument.is_empty() {
                        let line = format!(
                            "Experience {}; level {}.",
                            player.traits.experience, player.traits.level
                        );
                        send(stream, session, tell(player, &line), trace).await?;
                        Ok(true)
                    } else {
                        let Ok(total) = argument.parse::<u32>() else {
                            eprintln!("world: /xp: {argument:?} is not a total experience");
                            let line = "Usage: /xp <total experience>, e.g. /xp 2000";
                            send(stream, session, tell(player, line), trace).await?;
                            return Ok(true);
                        };
                        crate::player::set_experience(
                            stream, session, world, player, total, trace,
                        )
                        .await
                    }
                }
                Some(named) if named.names.contains(&"npc") && !argument.is_empty() => {
                    // Not the reference's: the owner's test command for the NPC table. The answer is
                    // printed to the console and also committed to the client's chat log as a
                    // server-composed line, so the tester sees it without leaving the game.
                    let lines = player.instance.npc_command(argument);
                    let mut frames = Vec::new();
                    for line in &lines {
                        eprintln!("world: /npc {line}");
                        frames.extend(game::server_chat_line(
                            player.player_id,
                            game::CHANNEL_GLOBAL,
                            line,
                        ));
                    }
                    send(stream, session, frames, trace).await?;
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
            // `@` and `%` are ignored by the reference; anything else it refuses.
            if matches!(other, '@' | '%') {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> ChatRules {
        load_chat_rules(&crate::spawns::pack()).expect("the pack's chat rules load")
    }

    #[test]
    fn a_whisper_is_a_name_a_comma_and_a_message() {
        assert_eq!(
            whisper_parts("Gwen Thackeray, hello, you", 19),
            Some(("Gwen Thackeray", "hello, you"))
        );
        assert_eq!(whisper_parts("Gwen Thackeray", 19), None, "no comma");
        assert_eq!(whisper_parts(", hello", 19), None, "no name");
        assert_eq!(whisper_parts("Gwen,  ", 19), None, "no message");
        assert_eq!(whisper_parts("Twenty Letters Long X, hi", 19), None);
    }

    #[test]
    fn a_roll_stays_inside_the_packs_bounds() {
        let rules = rules();
        let (low, high) = (rules.roll_min_sides.value, rules.roll_max_sides.value);
        assert_eq!(roll_sides(&low.to_string(), &rules), Some(low));
        assert_eq!(roll_sides(&high.to_string(), &rules), Some(high));
        assert_eq!(roll_sides(&(high + 1).to_string(), &rules), None);
        assert_eq!(roll_sides(&(low - 1).to_string(), &rules), None);
        assert_eq!(roll_sides("six", &rules), None);
        assert_eq!(roll_sides("", &rules), None);
        for random in [0, 1, 5, 6, u32::MAX] {
            assert!((1..=6).contains(&roll_face(random, 6)));
        }
        assert_eq!((roll_face(0, 6), roll_face(5, 6)), (1, 6));
    }

    #[test]
    fn the_new_commands_are_in_the_table() {
        assert!(command("roll").is_some_and(|named| named.takes_argument));
        assert!(command("age").is_some_and(|named| !named.takes_argument));
    }
}
