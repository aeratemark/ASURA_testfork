//! The quest log's messages: an entry, its details, its marker, its progress and its removal.
//!
//! The field order and the bounds are preforged's `gw_quest_wire.h`, which was written against the
//! client's own constructors: `add_2022` (2022 opcode 72), `details_2022` (75), `marker_2022` (76),
//! `update_2022` (79), `remove_2022` (81) and `objectives_2022` (83). This client is one build later,
//! and the live capture confirms each opcode a fixture covers: add `0x0049` (73), details `0x004C`
//! (76), marker `0x004D` (77), remove `0x0052` (82) and objectives `0x0054` (84). The packet table's
//! names for 77, 81 and 83 are the reference's; the live capture uses 77 for the turn-in marker and 81
//! for a progress marker, and 80 and 83 to put an active quest back after a map travel
//! ([`quest_general_info`], [`quest_add_marker`]). Preforged's native work names 84's
//! operation: it replaces the objectives text and raises `QuestUpdated`, not `QuestAdded`
//! (`quest-evidence.txt`, 2026-10-13), so [`quest_objectives`] is the name this module uses for it.
//!
//! The strings are **encoded units**, the client's own string ids and control codes, never text this
//! server composed. The three name fields decode into 8-unit buffers and the two detail fields into
//! 128-unit buffers, each with a terminator reserved: 7 and 127 units are the most a capture may
//! carry (`gw_quest_wire.h`, `encoded_name` and `details_2022`). The pack loader
//! (`crate::quest`) enforces those bounds; the builders below walk them.
//!
//! A marker is either finite or the pair `+infinity, +infinity`, the client's own "no marker"
//! sentinel (`gw_quest_wire.h`, `valid_marker`, from the native `790144` comparison).
//!
//! The floating marker above an NPC is a different message: `0x009F` agent property
//! [`AGENT_PROPERTY_QUEST_MARKER`], sent with an agent id and a value. The live capture sets the Town
//! Crier to 5 when it has a quest to offer (tick 169041703) and Sir Tydus to 4 once quest 80 is
//! accepted (tick 169042640). The values are configuration, not code: `config/Server/quests.json` holds
//! `marker_offer`, `marker_turn_in` and `marker_none`, each marked `immutable` because they are the
//! client's own marker states.

use super::Builder;
use crate::packets::game as msg;

/// The agent property that draws the quest marker above an NPC: `0x009F {11, agent, value}`. The
/// property id is the client's field vocabulary, like a message header; the values are settings
/// (`config/Server/quests.json`).
///
/// Measured on the live server: `side-quests-2026-10-12` packets, ticks 169039203 (`{11, 42, 5}`),
/// 169041703 (`{11, 75, 5}`), 169042640 (`{11, 42, 4}`), 169069953 and 169070859.
pub const AGENT_PROPERTY_QUEST_MARKER: u32 = 11;

/// The units one name field decodes into: the `String16` capacity of `GAME_SMSG_QUEST_ADD`'s three
/// strings (`gw_quest_wire.h`: an 8-unit native buffer).
pub const QUEST_NAME_UNITS: usize = 8;
/// The units a details field decodes into: the `String16` capacity of `0x004C` and `0x0054`.
pub const QUEST_DETAIL_UNITS: usize = 128;
/// The most units a name may carry: the buffer minus the terminator (`gw_quest_wire.h`, `encoded_name`).
pub const QUEST_NAME_LIMIT: usize = QUEST_NAME_UNITS - 1;
/// The most units a details field may carry: the buffer minus the terminator (`details_2022`).
pub const QUEST_DETAIL_LIMIT: usize = QUEST_DETAIL_UNITS - 1;
/// The client's "no marker" value, as the client's own constant (`0x7f800000`).
pub const QUEST_NO_MARKER: f32 = f32::INFINITY;

/// One quest-log entry, as `GAME_SMSG_QUEST_ADD` carries it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuestEntry<'a> {
    /// The client's quest id.
    pub id: u32,
    /// The log state. Bit 1 is the details-loaded flag and bit 2 the completed flag; both are owned by
    /// the details/objectives messages, and a restore masks them before sending (`gw_quest_restore.h`).
    pub state: u32,
    /// The marker, in map coordinates. Both `+infinity` means no marker.
    pub x: f32,
    /// See [`QuestEntry::x`].
    pub y: f32,
    /// The marker's plane.
    pub plane: u16,
    /// The map the marker points at.
    pub map_to: u16,
    /// The map the quest was taken on.
    pub map_from: u16,
    /// The log category, encoded units.
    pub location: &'a [u16],
    /// The quest's name, encoded units.
    pub name: &'a [u16],
    /// The quest giver's name, encoded units.
    pub npc: &'a [u16],
}

/// `GAME_SMSG_QUEST_ADD` (`0x0049`): one entry in the log.
///
/// The order is preforged's `add_2022` and the native constructor it was checked against: id, x, y,
/// plane, destination map, state, location, name, NPC, source map.
pub fn quest_add(entry: &QuestEntry<'_>) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_QUEST_ADD)
        .u32(entry.id)
        .f32(entry.x)
        .f32(entry.y)
        .u16(entry.plane)
        .u16(entry.map_to)
        .u32(entry.state)
        .units(entry.location, QUEST_NAME_UNITS)
        .units(entry.name, QUEST_NAME_UNITS)
        .units(entry.npc, QUEST_NAME_UNITS)
        .u16(entry.map_from)
        .frame()
}

/// `GAME_SMSG_QUEST_DESCRIPTION` (`0x004C`): an entry's details, two encoded strings.
///
/// The quest must already exist in the client's log; the native callback asserts on a missing entry
/// (`gw_quest_wire.h`, `details_2022`). Publication order is add, then this.
pub fn quest_description(id: u32, description: &[u16], objectives: &[u16]) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_QUEST_DESCRIPTION)
        .u32(id)
        .units(description, QUEST_DETAIL_UNITS)
        .units(objectives, QUEST_DETAIL_UNITS)
        .frame()
}

/// The quest marker's header on this build. The row is unnamed in the packet table, and the live
/// capture writes it before a turn-in's details and removal (`side-quests-2026-10-12` packets, tick
/// 169070859, quest 80: `4d 00 00 00` then id, x, y, plane, destination). It is the 2022 fixture's
/// marker opcode 76 one build later (`gw_quest_wire.h`, `marker_2022`). The table's
/// `GAME_SMSG_QUEST_UPDATE_MARKER` (`0x0051`) is a progress marker; `GAME_SMSG_QUEST_ADD_MARKER`
/// (`0x0053`) is the marker a map travel re-sends ([`quest_add_marker`]).
pub const GAME_SMSG_QUEST_MARKER: u16 = 0x004D;

/// A quest's marker: the id, the position, the plane and the destination map.
pub fn quest_marker(id: u32, x: f32, y: f32, plane: u16, map_to: u16) -> crate::client::Frame {
    Builder::new(GAME_SMSG_QUEST_MARKER)
        .u32(id)
        .f32(x)
        .f32(y)
        .u16(plane)
        .u16(map_to)
        .frame()
}

/// `GAME_SMSG_QUEST_GENERAL_INFO` (`0x0050`): an entry without its marker - the id, the state, the
/// three names and the source map.
///
/// What the live server sends for every active quest after a map travel, followed by
/// [`quest_add_marker`] and the details: four-professions-2026-10-11 packets, quest 222 accepted in
/// map 148 (tick 81506890, `0x0049`) and re-sent in map 146 (81524875 `0x0050`, 81525390 `0x0053`,
/// 81525500 `0x004C`); quest 49 the same at 82591078 and 83183968. The captured fields are the
/// add's own values in the add's order, less the marker: `de000000`, state `20000000`, the location,
/// name and NPC units, source map `9400`.
pub fn quest_general_info(entry: &QuestEntry<'_>) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_QUEST_GENERAL_INFO)
        .u32(entry.id)
        .u32(entry.state)
        .units(entry.location, QUEST_NAME_UNITS)
        .units(entry.name, QUEST_NAME_UNITS)
        .units(entry.npc, QUEST_NAME_UNITS)
        .u16(entry.map_from)
        .frame()
}

/// `GAME_SMSG_QUEST_ADD_MARKER` (`0x0053`): an entry's marker after a travel - the id, the position,
/// the plane and the destination map, the layout of [`quest_marker`]. The live server sends it after
/// [`quest_general_info`] (quest 222 in map 146: `de000000`, x `0030be45`, y `00088245`, plane 0,
/// destination `9200`).
pub fn quest_add_marker(id: u32, x: f32, y: f32, plane: u16, map_to: u16) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_QUEST_ADD_MARKER)
        .u32(id)
        .f32(x)
        .f32(y)
        .u16(plane)
        .u16(map_to)
        .frame()
}

/// `GAME_SMSG_QUEST_UPDATE_NAME` (`0x0054`): the objectives text.
///
/// Preforged's native work established the operation: the native handler replaces the objectives and
/// raises `QuestUpdated` (`gw_quest_wire.h`, `objectives_2022`; `quest-evidence.txt`, 2026-10-13). The
/// table's name is the reference's own and is kept in the packet catalogue; this builder is named for
/// what the message does.
pub fn quest_objectives(id: u32, objectives: &[u16]) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_QUEST_UPDATE_NAME)
        .u32(id)
        .units(objectives, QUEST_DETAIL_UNITS)
        .frame()
}

/// `GAME_SMSG_QUEST_REMOVE` (`0x0052`): take the entry out of the log, after a reward or an abandon.
pub fn quest_remove(id: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_QUEST_REMOVE).u32(id).frame()
}

/// Read `GAME_CMSG_QUEST_ABANDON` (`0x8011`): the quest id, four bytes and nothing else.
pub fn read_quest_abandon(body: &[u8]) -> Option<u32> {
    body.try_into().ok().map(u32::from_le_bytes)
}

/// Read `GAME_CMSG_QUEST_REQUEST_INFOS` (`0x8012`): the quest id whose details the client asks for.
///
/// Preforged saw the same request shape on the 2022 build (`gw_quest_request.h`: a flagged or
/// unflagged quest id); the table's 6-byte row is this dword.
pub fn read_quest_info_request(body: &[u8]) -> Option<u32> {
    body.try_into().ok().map(u32::from_le_bytes)
}

/// Whether a marker is one the client accepts: both finite, or exactly the `+infinity` pair.
///
/// `NaN` is not the sentinel (`gw_quest_wire.h`: the native comparison tests both coordinates against
/// the `+infinity` constant `0x7f800000`).
pub fn marker_is_valid(x: f32, y: f32) -> bool {
    (x.is_finite() && y.is_finite()) || (x == QUEST_NO_MARKER && y == QUEST_NO_MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 2022 fixture's entry (`tests/fixtures/quest-wire-2022.txt`): id 101, (12.5, -4.25), plane 2,
    /// destination 85, state 87, "UW", "Chamber", "Soul", source 72.
    fn entry() -> QuestEntry<'static> {
        const LOCATION: [u16; 2] = [0x0055, 0x0057];
        const NAME: [u16; 7] = [0x0043, 0x0068, 0x0061, 0x006D, 0x0062, 0x0065, 0x0072];
        const NPC: [u16; 4] = [0x0053, 0x006F, 0x0075, 0x006C];
        QuestEntry {
            id: 101,
            state: 87,
            x: f32::from_bits(0x4148_0000),
            y: f32::from_bits(0xC088_0000),
            plane: 2,
            map_to: 85,
            map_from: 72,
            location: &LOCATION,
            name: &NAME,
            npc: &NPC,
        }
    }

    #[test]
    fn general_info_is_the_add_without_its_marker() {
        // The add is header, id, x, y, plane, destination, state, then the names and the source; the
        // general info is header, id, state, then the same names and source.
        let add = quest_add(&entry()).encode();
        let info = quest_general_info(&entry()).encode();
        assert_eq!(&info[0..2], &[0x50, 0x00]);
        assert_eq!(&info[2..6], &add[2..6]);
        assert_eq!(&info[6..10], &add[18..22]);
        assert_eq!(&info[10..], &add[22..]);
        let marker = quest_add_marker(101, 12.5, -4.25, 2, 72).encode();
        assert_eq!(&marker[0..2], &[0x53, 0x00]);
        assert_eq!(
            &marker[2..],
            &quest_marker(101, 12.5, -4.25, 2, 72).encode()[2..],
            "the layout of 0x004D"
        );
    }

    #[test]
    fn an_entry_is_the_2022_fixture_with_this_builds_header() {
        // The fixture begins `4880 65000000 ...`; 0x48 is the 2022 opcode and 0x49 this build's.
        let expected: Vec<u8> = vec![
            0x49, 0x00, 0x65, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x41, 0x00, 0x00, 0x88, 0xC0,
            0x02, 0x00, 0x55, 0x00, 0x57, 0x00, 0x00, 0x00, 0x02, 0x00, 0x55, 0x00, 0x57, 0x00,
            0x07, 0x00, 0x43, 0x00, 0x68, 0x00, 0x61, 0x00, 0x6D, 0x00, 0x62, 0x00, 0x65, 0x00,
            0x72, 0x00, 0x04, 0x00, 0x53, 0x00, 0x6F, 0x00, 0x75, 0x00, 0x6C, 0x00, 0x48, 0x00,
        ];
        assert_eq!(quest_add(&entry()).encode(), expected);
    }

    #[test]
    fn details_are_the_two_captured_strings() {
        let description: Vec<u16> = "Reward".encode_utf16().collect();
        let objectives: Vec<u16> = "Kill foes".encode_utf16().collect();
        let frame = quest_description(101, &description, &objectives);
        let mut expected = vec![0x4C, 0x00, 0x65, 0x00, 0x00, 0x00];
        expected.extend_from_slice(&[6, 0]);
        for unit in &description {
            expected.extend_from_slice(&unit.to_le_bytes());
        }
        expected.extend_from_slice(&[9, 0]);
        for unit in &objectives {
            expected.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(frame.encode(), expected);
    }

    #[test]
    fn objectives_are_one_counted_string() {
        let objectives: Vec<u16> = "Return for reward".encode_utf16().collect();
        let frame = quest_objectives(101, &objectives);
        let mut expected = vec![0x54, 0x00, 0x65, 0x00, 0x00, 0x00, 17, 0];
        for unit in &objectives {
            expected.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(frame.encode(), expected);
    }

    #[test]
    fn a_marker_is_id_position_plane_and_destination() {
        let frame = quest_marker(101, 12.5, -4.25, 2, 72);
        assert_eq!(
            frame.encode(),
            vec![
                0x4D, 0x00, 0x65, 0, 0, 0, 0x00, 0x00, 0x48, 0x41, 0x00, 0x00, 0x88, 0xC0, 0x02,
                0x00, 0x48, 0x00
            ]
        );
    }

    #[test]
    fn a_removal_names_one_quest() {
        assert_eq!(quest_remove(101).encode(), vec![0x52, 0x00, 0x65, 0, 0, 0]);
    }

    #[test]
    fn the_client_requests_are_one_dword() {
        assert_eq!(read_quest_abandon(&[0x65, 0, 0, 0]), Some(101));
        assert_eq!(read_quest_info_request(&[0x65, 0, 0, 0]), Some(101));
        assert_eq!(read_quest_abandon(&[0x65, 0, 0]), None);
        assert_eq!(read_quest_info_request(&[]), None);
    }

    #[test]
    fn a_marker_is_finite_or_the_positive_infinity_pair() {
        assert!(marker_is_valid(0.0, 0.0));
        assert!(marker_is_valid(-1234.5, 9876.25));
        assert!(marker_is_valid(QUEST_NO_MARKER, QUEST_NO_MARKER));
        assert!(!marker_is_valid(QUEST_NO_MARKER, 0.0));
        assert!(!marker_is_valid(f32::NAN, f32::NAN));
        assert!(!marker_is_valid(f32::NEG_INFINITY, f32::NEG_INFINITY));
    }
}
