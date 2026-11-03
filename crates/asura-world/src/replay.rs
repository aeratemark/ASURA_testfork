//! Replaying the reference's own captured stream, for bisection.
//!
//! When a client dies against this server and not against GWPS, the load stream differs somewhere, and
//! the fastest way to find out *whether* it is the stream at all is to serve the reference's own bytes
//! verbatim. If a client survives a replay and dies against the built blocks, the difference is content.
//! If it dies against the replay too, the problem is somewhere else (routing, timing, a message the
//! client wants afterwards) and there is no point editing payloads.
//!
//! The capture is the one `tools/patch-gwps-full-trace.py` makes GWPS write (`GWPS_TRACE=1`): one
//! `SEND`/`RECV 0xHHHH  N bytes : <hex>` line per message, in order. The `RECV` lines are what delimits
//! the blocks, because they are the client's requests:
//!
//! ```text
//! SEND 0x017C ...                 the load block, sent unasked
//! RECV 0x8091                     the client asks for items
//! SEND 0x0144 ...                 -> the item block
//! RECV 0x8088                     the client asks where to stand
//! SEND 0x0196                     -> the spawn point
//! RECV 0x8090                     the client asks who is here
//! SEND 0x001D ... 0x018E          -> the player block
//! ```
//!
//! The world server's own ticks and pings are interleaved in the capture and are filtered out: a
//! replayed block is the answer to a request, not a timeline.

use std::path::Path;

use asura_protocol::client::Frame;
use asura_protocol::packets::game as msg;

/// A block of messages the reference sent, ready to be sent again.
pub type Block = Vec<Frame>;

/// Everything a replay needs: the four unasked messages and the three answers.
#[derive(Debug, Clone)]
pub struct Replay {
    pub load: Block,
    pub items: Block,
    pub spawn: Block,
    pub players: Block,
    /// Which blocks to actually replace with the capture's.
    pub pick: Pick,
}

/// Which blocks a replay should replace, from `ASURA_WORLD_REPLAY_PICK`.
///
/// Bisection needs to mix: replacing *only* the item block and leaving this server's own player block
/// in place answers "is it the items or the players" in one run, which is the difference between one
/// question to a human and two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pick {
    pub load: bool,
    pub items: bool,
    pub spawn: bool,
    pub players: bool,
}

/// One of the four load blocks, named so a [`Pick`] can be asked about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockName {
    Load,
    Items,
    Spawn,
    Players,
}

impl Pick {
    /// Everything, which is what a bare `ASURA_WORLD_REPLAY` means.
    pub const ALL: Pick = Pick {
        load: true,
        items: true,
        spawn: true,
        players: true,
    };

    /// Read `ASURA_WORLD_REPLAY_PICK`: a comma-separated list of `load`, `items`, `spawn` and
    /// `players`. Unset or empty is [`Pick::ALL`]; an unknown name is an error, because a bisection
    /// that silently ignores what it was told is a bisection that tests nothing.
    pub fn from_environment() -> Result<Pick, String> {
        let Ok(value) = std::env::var("ASURA_WORLD_REPLAY_PICK") else {
            return Ok(Pick::ALL);
        };
        if value.trim().is_empty() {
            return Ok(Pick::ALL);
        }
        let mut pick = Pick {
            load: false,
            items: false,
            spawn: false,
            players: false,
        };
        for part in value.split(',') {
            match part.trim() {
                "load" => pick.load = true,
                "items" => pick.items = true,
                "spawn" => pick.spawn = true,
                "players" => pick.players = true,
                other => {
                    return Err(format!(
                    "ASURA_WORLD_REPLAY_PICK: {other:?} is not one of load, items, spawn, players"
                ))
                }
            }
        }
        Ok(pick)
    }

    /// Whether this block should be replayed.
    pub fn wants(self, block: BlockName) -> bool {
        match block {
            BlockName::Load => self.load,
            BlockName::Items => self.items,
            BlockName::Spawn => self.spawn,
            BlockName::Players => self.players,
        }
    }
}

/// Headers that belong to the world's own clock rather than to an answer. They appear inside the
/// capture's answer windows because the reference keeps its clock running while it answers.
const CLOCK_HEADERS: [u16; 3] = [0x001E, 0x000C, 0x000D];

impl Replay {
    /// Read a capture written by the reference's full trace.
    pub fn from_capture(path: &Path) -> Result<Replay, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("cannot read {}: {err}", path.display()))?;

        let mut load: Block = Vec::new();
        let mut items: Block = Vec::new();
        let mut spawn: Block = Vec::new();
        let mut players: Block = Vec::new();

        // Which block the next SEND lines belong to. `None` is "before the client asked for anything",
        // which is the load block.
        #[derive(PartialEq, Clone, Copy)]
        enum Target {
            Load,
            Items,
            Spawn,
            Players,
            Ignore,
        }
        let mut target = Target::Load;
        let mut saw_load_head = false;

        for line in text.lines() {
            if let Some((header, frame)) = parse_send(line) {
                if header == 0x017C {
                    saw_load_head = true;
                }
                if !saw_load_head || CLOCK_HEADERS.contains(&header) {
                    continue;
                }
                match target {
                    Target::Load => load.push(frame),
                    Target::Items => items.push(frame),
                    Target::Spawn => spawn.push(frame),
                    Target::Players => players.push(frame),
                    Target::Ignore => {}
                }
                if header == 0x018E {
                    target = Target::Ignore;
                }
                continue;
            }
            if let Some(header) = parse_recv(line) {
                target = match header {
                    msg::GAME_CMSG_INSTANCE_LOAD_REQUEST_ITEMS => Target::Items,
                    msg::GAME_CMSG_INSTANCE_LOAD_REQUEST_SPAWN => Target::Spawn,
                    msg::GAME_CMSG_INSTANCE_LOAD_REQUEST_PLAYERS => Target::Players,
                    _ => target,
                };
            }
        }

        for (name, block) in [
            ("load", &load),
            ("items", &items),
            ("spawn", &spawn),
            ("players", &players),
        ] {
            if block.is_empty() {
                return Err(format!(
                    "the capture has no {name} block; is it a full trace (GWPS_TRACE=1)?"
                ));
            }
        }

        Ok(Replay {
            load,
            items,
            spawn,
            players,
            pick: Pick::from_environment()?,
        })
    }

    /// The replay a service should use, from `ASURA_WORLD_REPLAY` when it is set.
    ///
    /// A file that cannot be read is fatal rather than silently ignored: a replay that quietly does not
    /// replay is a bisection that silently tests the wrong thing.
    pub fn from_environment() -> Result<Option<Replay>, String> {
        match std::env::var("ASURA_WORLD_REPLAY") {
            Ok(value) if !value.trim().is_empty() => {
                Ok(Some(Replay::from_capture(Path::new(value.trim()))?))
            }
            _ => Ok(None),
        }
    }
}

/// `SEND 0x017C   6 bytes : 7C 01 3F 3F 00 00`, with or without the log's timestamp prefix
/// (`[2026-10-26T04:04:25.541]  info: SEND ...`, which is what the file actually contains).
fn parse_send(line: &str) -> Option<(u16, Frame)> {
    let rest = line
        .trim()
        .get(line.trim().find("SEND ")? + "SEND ".len()..)?;
    parse_message(rest)
}

fn parse_recv(line: &str) -> Option<u16> {
    let trimmed = line.trim();
    let rest = trimmed.get(trimmed.find("RECV ")? + "RECV ".len()..)?;
    let header = rest.split_whitespace().next()?;
    u16::from_str_radix(header.strip_prefix("0x")?, 16).ok()
}

fn parse_message(rest: &str) -> Option<(u16, Frame)> {
    // `0x017C   6 bytes : 7C 01 ...`
    let (header, remainder) = rest.split_once(char::is_whitespace)?;
    let header = u16::from_str_radix(header.strip_prefix("0x")?, 16).ok()?;
    let (_, hex) = remainder.split_once(':')?;
    let mut bytes = Vec::new();
    for part in hex.split_whitespace() {
        bytes.push(u8::from_str_radix(part, 16).ok()?);
    }
    if bytes.len() < 2 {
        return None;
    }
    let body = bytes[2..].to_vec();
    Some((header, Frame::new(header, body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pick_says_which_blocks_to_replace() {
        // No environment variable is `ALL`, and a bisection can name any subset.
        let pick = Pick {
            load: true,
            items: false,
            spawn: true,
            players: false,
        };
        assert!(pick.wants(BlockName::Load));
        assert!(!pick.wants(BlockName::Items));
        assert!(pick.wants(BlockName::Spawn));
        assert!(!pick.wants(BlockName::Players));
        assert!(Pick::ALL.wants(BlockName::Players));
    }

    #[test]
    fn a_captured_block_parses_into_frames() {
        let line = "SEND 0x017C   6 bytes : 7C 01 3F 3F 00 00";
        let (header, frame) = parse_send(line).expect("parses");
        assert_eq!(header, 0x017C);
        assert_eq!(frame.encode(), vec![0x7C, 0x01, 0x3F, 0x3F, 0x00, 0x00]);
    }

    #[test]
    fn the_log_prefix_the_capture_actually_has_is_tolerated() {
        // The file's own shape, timestamp and level included: a parser that only accepts a bare `SEND`
        // finds no block in a real capture.
        let line = "[2026-10-26T04:04:25.541]  info: SEND 0x017C   6 bytes : 7C 01 3F 3F 00 00";
        let (header, _frame) = parse_send(line).expect("parses with its prefix");
        assert_eq!(header, 0x017C);
        assert_eq!(
            parse_recv("[2026-10-26T04:04:25.541]  info: RECV 0x8091   4 bytes : 91 80 00 00"),
            Some(0x8091)
        );
    }

    #[test]
    fn a_receive_line_names_the_request_it_answers() {
        assert_eq!(
            parse_recv("RECV 0x8091   4 bytes : 91 80 00 00"),
            Some(0x8091)
        );
        assert_eq!(parse_recv("RECV 0x8090  19 bytes : 90 80"), Some(0x8090));
        assert_eq!(parse_recv("SEND 0x017C   6 bytes : 7C 01"), None);
    }

    #[test]
    fn something_that_is_not_a_trace_line_is_not_guessed_at() {
        assert!(parse_send("garbage").is_none());
        assert!(parse_send("SEND 0xZZZZ 6 bytes : 00").is_none());
        assert!(parse_send("SEND 0x017C no colon here").is_none());
    }
}
