//! The title family: four server-to-client messages and two client-to-server ones.
//!
//! What each message is, field by field, with the evidence beside it, is `docs/40-titles-system-research.md`
//! (sections 3 and 6). This module only writes and reads the bytes; which rank a track is on, which records a
//! load sends and in what order are decisions, and live in `asura-rules` (`titles`).
//!
//! The numbers are build 39112's. The family keeps its order between client builds and moves as a block
//! (`0x00E0`.. in 2008, `0x00F2`.. in a 2025 dump, `0x00F3`.. here), so the headers come from the generated
//! table, never from a literal in this file.
//!
//! Three wire rules that come from measurements, not from a struct:
//!
//! * **A rank must exist before anything names it.** A track that names an unseeded rank is accepted on
//!   receive and kills the client when the Hero panel draws it (`Array.h(587)`, Rurik
//!   `studies/character/RUNS.md:177-186`). That ordering is the caller's to keep; nothing here can see it.
//! * **A coded string here carries at most seven units.** The field is `string16(8)`, and a string that
//!   fills it exactly is an instant `Code=007` hangup (Rurik `studies/character/RUNS.md:165-176`). The
//!   builders refuse a longer one rather than cut it: a cut coded string is a different string.
//! * **A counted field is a `u16` count and exactly that many units**, the rule every builder in `super`
//!   follows. No packed `0x00F3`/`0x00F6` frame has been captured yet (doc 40, section 11); the shape below
//!   is the same one the rest of the game connection is measured to use.

use super::Builder;

/// `GAME_SMSG_TITLE_RANK_DATA` (`0x00F3`): one rank record - `rank_id, flags, value, coded name`.
pub const TITLE_RANK_DATA: u16 = crate::packets::game::GAME_SMSG_TITLE_RANK_DATA;
/// `GAME_SMSG_TITLE_RANK_DISPLAY` (`0x00F4`): the rank a player shows under its name - a word and a dword.
pub const TITLE_RANK_DISPLAY: u16 = crate::packets::game::GAME_SMSG_TITLE_RANK_DISPLAY;
/// `GAME_SMSG_TITLE_UPDATE` (`0x00F5`): a track's new point count - two dwords.
pub const TITLE_UPDATE: u16 = crate::packets::game::GAME_SMSG_TITLE_UPDATE;
/// `GAME_SMSG_TITLE_TRACK_INFO` (`0x00F6`): one whole track - ten dwords and two coded strings.
pub const TITLE_TRACK_INFO: u16 = crate::packets::game::GAME_SMSG_TITLE_TRACK_INFO;
/// `GAME_CMSG_TITLE_DISPLAY` (`0x8058`): the client asks to show a title - one dword, the title id.
pub const TITLE_DISPLAY_REQUEST: u16 = crate::packets::game::GAME_CMSG_TITLE_DISPLAY;
/// `GAME_CMSG_TITLE_HIDE` (`0x8059`): the client asks to show none - a bare header.
pub const TITLE_HIDE_REQUEST: u16 = crate::packets::game::GAME_CMSG_TITLE_HIDE;

/// The most units a title string may carry: the `string16(8)` field's eight, minus the one a string that
/// fills the field exactly costs (measured as a `Code=007` hangup, doc 40 section 6 rule 2).
pub const TITLE_STRING_UNITS: usize = 7;

/// The rows of the client's own title table (`s_titleClientData`): a title id is `0..TITLE_ID_COUNT`.
/// The client asserts on anything else (`ConstTitle:81`, doc 40 section 4).
pub const TITLE_ID_COUNT: u32 = 48;

/// "There is no next rank": the value a maxed track's `next_minimum` carries, and the value of the sentinel
/// rank record it points at (Kurzick, Treasure Hunter and Wisdom in build 39112's load, doc 40 section 5.1).
pub const TITLE_NO_NEXT: u32 = 0xFFFF_FFFF;

/// A title string longer than the field admits. Refused rather than cut: a coded string is a token and its
/// key, and a shorter one is a different string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleStringTooLong {
    /// The message the string was for.
    pub header: u16,
    /// How many units it had.
    pub units: usize,
}

impl std::fmt::Display for TitleStringTooLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a title string of {} units for {:#06X}; the field admits {}",
            self.units, self.header, TITLE_STRING_UNITS
        )
    }
}

impl std::error::Error for TitleStringTooLong {}

fn checked(header: u16, units: &[u16]) -> Result<&[u16], TitleStringTooLong> {
    if units.len() > TITLE_STRING_UNITS {
        Err(TitleStringTooLong {
            header,
            units: units.len(),
        })
    } else {
        Ok(units)
    }
}

/// `GAME_SMSG_TITLE_RANK_DATA`: define one rank.
///
/// `flags` bit 0 says `value` is a per-mille threshold (a Cartographer rank, drawn as `60.0`); clear, `value`
/// is the rank number drawn in parentheses. `name` is the coded string the Hero panel draws as the row's name.
pub fn title_rank_data(
    rank_id: u32,
    flags: u32,
    value: u32,
    name: &[u16],
) -> Result<crate::client::Frame, TitleStringTooLong> {
    let name = checked(TITLE_RANK_DATA, name)?;
    Ok(Builder::new(TITLE_RANK_DATA)
        .u32(rank_id)
        .u32(flags)
        .u32(value)
        .units(name, TITLE_STRING_UNITS)
        .frame())
}

/// `GAME_SMSG_TITLE_RANK_DISPLAY`: the rank one player shows under its name. The player id is a word on the
/// wire (the client widens it to a dword in its own struct, which is why a sniff shows twelve bytes).
pub fn title_rank_display(player_id: u16, rank_id: u32) -> crate::client::Frame {
    Builder::new(TITLE_RANK_DISPLAY)
        .u16(player_id)
        .u32(rank_id)
        .frame()
}

/// `GAME_SMSG_TITLE_UPDATE`: patch the points of a track this connection has already been sent.
///
/// Inert on a track no `0x00F6` created (Rurik `studies/newopcodes/FINDINGS.md:1433-1437`): a patch, not a
/// create.
pub fn title_update(title_id: u32, points: u32) -> crate::client::Frame {
    Builder::new(TITLE_UPDATE).u32(title_id).u32(points).frame()
}

/// `GAME_SMSG_TITLE_TRACK_INFO`'s fields, in wire order. A struct because ten adjacent dwords passed in
/// order are a swap the compiler cannot see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleTrackInfo<'a> {
    /// Field 1: the title, `0..TITLE_ID_COUNT`.
    pub title_id: u32,
    /// Field 2: the track's flag word. Bit 0 a percentage track, `(flags & 3) == 2` an "x of y" count;
    /// build 39112 sends 0, 1, 2 and 4. Unknown bits are carried as configured, never normalised.
    pub flags: u32,
    /// Field 3: the points on the track - the one field `0x00F5` patches.
    pub points: u32,
    /// Field 4: the rank the track is on. Must already be seeded.
    pub current_rank_id: u32,
    /// Field 5: the points at which the current rank was reached.
    pub current_minimum: u32,
    /// Field 6: `title_mode_flags` - 0 none, 1 Dhuum's Covenant, 2 Reforged Mode. 0 in every captured track.
    pub mode_flags: u32,
    /// Field 7: the rank after this one. Must already be seeded.
    pub next_rank_id: u32,
    /// Field 8: the points the next rank needs, or [`TITLE_NO_NEXT`] on a maxed track.
    pub next_minimum: u32,
    /// Field 9: the highest rank number on the track.
    pub max_rank: u32,
    /// Field 10: the rank-1 record of the title's block in 14 of 15 captured tracks (doc 40 correction 2).
    pub base_rank_id: u32,
    /// Field 11: the progress template, a coded string.
    pub progress_template: &'a [u16],
    /// Field 12: the description template, a coded string.
    pub description_template: &'a [u16],
}

/// `GAME_SMSG_TITLE_TRACK_INFO`: one whole track, the ten-dword layout this client build reads.
pub fn title_track_info(
    track: &TitleTrackInfo<'_>,
) -> Result<crate::client::Frame, TitleStringTooLong> {
    let progress = checked(TITLE_TRACK_INFO, track.progress_template)?;
    let description = checked(TITLE_TRACK_INFO, track.description_template)?;
    Ok(Builder::new(TITLE_TRACK_INFO)
        .u32(track.title_id)
        .u32(track.flags)
        .u32(track.points)
        .u32(track.current_rank_id)
        .u32(track.current_minimum)
        .u32(track.mode_flags)
        .u32(track.next_rank_id)
        .u32(track.next_minimum)
        .u32(track.max_rank)
        .u32(track.base_rank_id)
        .units(progress, TITLE_STRING_UNITS)
        .units(description, TITLE_STRING_UNITS)
        .frame())
}

/// The title id a `GAME_CMSG_TITLE_DISPLAY` body asks for: its one dword. `None` for a body too short to
/// hold it, which the generated table's framing should already have refused.
pub fn read_title_display_request(body: &[u8]) -> Option<u32> {
    let bytes: [u8; 4] = body.get(..4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hero's rank 6 as build 39112 sends it: rank 6, flags 0, value 6, and the four-unit coded name
    /// read out of `recordings/client/feature/travel/portal-battery-20261031-013856.jsonl` seq 709
    /// (`2b41 7f84 a0e3 fe3e`, then the client's own terminator).
    const HERO_RANK_6_NAME: [u16; 4] = [0x412B, 0x847F, 0xE3A0, 0x3EFE];

    #[test]
    fn the_headers_are_the_tables() {
        assert_eq!(TITLE_RANK_DATA, 0x00F3);
        assert_eq!(TITLE_RANK_DISPLAY, 0x00F4);
        assert_eq!(TITLE_UPDATE, 0x00F5);
        assert_eq!(TITLE_TRACK_INFO, 0x00F6);
        assert_eq!(TITLE_DISPLAY_REQUEST, 0x8058);
        assert_eq!(TITLE_HIDE_REQUEST, 0x8059);
    }

    #[test]
    fn a_rank_record_is_three_dwords_and_a_counted_name() {
        let frame = title_rank_data(6, 0, 6, &HERO_RANK_6_NAME).expect("four units fit");
        let mut expected = vec![0xF3, 0x00];
        expected.extend_from_slice(&6u32.to_le_bytes());
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&6u32.to_le_bytes());
        expected.extend_from_slice(&4u16.to_le_bytes());
        for unit in HERO_RANK_6_NAME {
            expected.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(frame.encode(), expected);
        assert_eq!(frame.encode().len(), 24);
    }

    #[test]
    fn the_fixed_messages_are_their_declared_sizes() {
        // 0x00F4: 2 + word + dword; 0x00F5: 2 + two dwords. Both fixed in the generated table.
        let declared = |header: u16| {
            crate::packets::lookup(crate::packets::Conn::Game, header, false).map(|row| row.size)
        };
        let display = title_rank_display(0x35, 0x98);
        assert_eq!(
            display.encode(),
            vec![0xF4, 0x00, 0x35, 0x00, 0x98, 0x00, 0x00, 0x00]
        );
        assert_eq!(Some(display.encode().len()), declared(TITLE_RANK_DISPLAY));
        assert_eq!(
            Some(title_update(21, 14).encode().len()),
            declared(TITLE_UPDATE)
        );
        // The two counted messages stay within their declared maxima with a full seven-unit string.
        let seven = [0x0101u16; 7];
        let rank = title_rank_data(1, 0, 1, &seven).expect("seven units fit");
        assert!(Some(rank.encode().len()) <= declared(TITLE_RANK_DATA));
    }

    #[test]
    fn a_track_is_ten_dwords_in_the_captured_order() {
        // Hero (title 0) from the same capture, seq 711: every dword as build 39112 carries it.
        let progress = [0x4124, 0xD05E, 0xCBC0, 0x5680];
        let description = [0x4125, 0xA1DB, 0xFCBA, 0x6C2C];
        let frame = title_track_info(&TitleTrackInfo {
            title_id: 0,
            flags: 4,
            points: 1448,
            current_rank_id: 6,
            current_minimum: 1000,
            mode_flags: 0,
            next_rank_id: 7,
            next_minimum: 1680,
            max_rank: 15,
            base_rank_id: 3,
            progress_template: &progress,
            description_template: &description,
        })
        .expect("four units fit");
        let bytes = frame.encode();
        let dwords: Vec<u32> = bytes[2..42]
            .chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(dwords, vec![0, 4, 1448, 6, 1000, 0, 7, 1680, 15, 3]);
        // Two counted strings of four units each.
        assert_eq!(bytes.len(), 2 + 40 + (2 + 8) * 2);
        assert_eq!(&bytes[42..44], &4u16.to_le_bytes());
        assert_eq!(&bytes[52..54], &4u16.to_le_bytes());
    }

    #[test]
    fn a_string_that_fills_the_field_is_refused_not_cut() {
        let seven = [0x0101u16; 7];
        let eight = [0x0101u16; 8];
        assert!(title_rank_data(1, 0, 1, &seven).is_ok());
        assert_eq!(
            title_rank_data(1, 0, 1, &eight),
            Err(TitleStringTooLong {
                header: TITLE_RANK_DATA,
                units: 8
            })
        );
        let track = TitleTrackInfo {
            title_id: 21,
            flags: 2,
            points: 13,
            current_rank_id: 176,
            current_minimum: 0,
            mode_flags: 0,
            next_rank_id: 177,
            next_minimum: 20,
            max_rank: 1,
            base_rank_id: 177,
            progress_template: &seven,
            description_template: &eight,
        };
        assert!(title_track_info(&track).is_err());
    }

    #[test]
    fn a_display_request_carries_the_title_id() {
        assert_eq!(read_title_display_request(&[21, 0, 0, 0]), Some(21));
        assert_eq!(read_title_display_request(&[21, 0]), None);
    }
}
