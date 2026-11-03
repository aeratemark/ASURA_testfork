//! The world server's gameplay connection.
//!
//! A world server is where a client plays. The auth service tells a travelling client *where* to
//! connect (`AUTH_SMSG_GAME_SERVER_INFO`); this module is what answers when it does.
//!
//! The order, and it is the reference's order (`docs/03-protocol.md` section 11):
//!
//! ```text
//! 1. classify          the first four bytes name the connection kind; anything but a game
//!                      connection is refused here, not guessed at
//! 2. GAME_CMSG_VERSION 64 bytes: the world id, map, player token, account and character
//! 3. claim             the token pair must match a transfer this world was told about, or the
//!                      connection is closed (the reference closes it with no message at all)
//! 4. key exchange      the client's seed, the server's seed; everything after is scrambled
//! 5. the load block    four messages, sent without being asked: INSTANCE_LOAD_HEAD,
//!                      PLAYER_DATA_START, LOAD_PLAYER_NAME, LOAD_INFO
//! 6. the three asks    the client asks for items, for a spawn point, and for the players; each is
//!                      answered, and the last answer ends with INSTANCE_LOAD_FINISH
//! 7. keep-alive        a world tick every 500 ms and a ping every 5 s, forever
//! ```
//!
//! What exists today, and what does not -- `docs/07-status.md` holds the hand-kept answer; its
//! generator was retired when parity closed (`docs/16-implementation-plan.md`):
//!
//! * the character the transfer named, standing where the map's spawn says - or where it stood when it
//!   last left, which is saved and read back (`asura_db::game::record_position`);
//! * the other players in the same instance: each newcomer is introduced to the ones already there
//!   and told when one leaves (`instance.rs`, `PeerView` and `introduction`);
//! * the character's inventory, read from rows and built into the reference's messages, including the
//!   nine items `UPDATE_AGENT_VISUAL_EQUIPMENT` puts on the model (`asura_db::inventory`);
//! * `GAME_CMSG_DROP_ITEM`, which deducts the stack and sends nothing: the reference's own handler is
//!   unfinished and the ground agent crashed a live client (`item.rs:243-249`);
//! * no NPCs, no combat, no quests, no picking anything up.
//!
//! The load messages go out even for the parts that are not simulated, because a client that does not
//! receive them never leaves the loading screen.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

// The crate's externals, re-exported at crate visibility so a domain file's `use crate::*;`
// reaches them the same way it reaches the domains themselves.
pub use asura_db::game as db_game;
pub use asura_db::inventory::{self as db_inventory, Inventory};
pub use asura_db::{placement, PgPool, Uuid};
pub use asura_protocol::client::game::GameVersion;
pub use asura_protocol::client::handshake;
pub use asura_protocol::client::handshake::{Established, CLIENT_SEED_LEN, MASTER_SECRET_LEN};
pub use asura_protocol::client::version;
pub use asura_protocol::client::version::GAME_CMSG_VERSION_LEN;
pub use asura_protocol::client::{Frame, FrameError};
pub use asura_protocol::game;
pub use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub use tokio::net::{TcpListener, TcpStream};

// -------------------------------------------------------------------------------------------
// The domains. One file per noun; `lib.rs` is the
// facade that re-exports them, so `asura_world::MapEntry` and `asura_world::map::MapEntry` are
// the same type. A domain's file is where to look: `item.rs` for items, `chat.rs` for chat.
//
// A noun gets its file when it first has code - no empty `party.rs`, `skill.rs` or `effects.rs`
// standing in for something that does not exist yet. When the first party rule lands, it lands
// in `party.rs`, and the dispatcher calls it; until then the noun is a name in a comment, not a
// module.
// -------------------------------------------------------------------------------------------

pub mod chat;
pub mod connection;
pub mod creation;
pub mod dispatch;
pub mod instance;
pub mod item;
pub mod map;
pub mod player;
pub mod travel;
pub mod world;

pub use connection::*;
pub use creation::*;
pub use instance::*;
pub use item::*;
pub use map::*;
pub use player::*;
pub use travel::*;
pub use world::*;

pub use agent::*;
pub use cartography::*;
pub use chat::*;
pub use dispatch::*;

/// The exploration grid the client's map surfaces draw from: the `0x008B` + `0x008A` pair, which
/// this server sent nothing of until a live client asserted its map view without it.
pub mod cartography;

/// One agent's position, and the reference's rules for believing a client about it.
pub mod agent;

/// The path-graph dump: `PathStaticData` written once and loaded without a `Gw.dat`, so a server
/// machine needs a few megabytes instead of the client's 3.6 GB archive (`asura-world
/// --dump-path-graphs` / `--path-graphs`).
pub mod graph_dump;

/// A server-side packet capture: every game message both ways, plaintext JSONL, for diagnosing a
/// live session without an injected client (`--capture` / `ASURA_CAPTURE`).
pub mod capture;

/// Replaying the reference's captured stream, for bisection (ASURA_WORLD_REPLAY).
pub mod replay;

/// The `Gw.dat` archive's index: the reference's `FaArchive.c`, ported far enough to find a file id and
/// say where its bytes are. The decompressor is the other half and is not here yet (docs/10-parity-queue.md,
/// item D).
pub mod gwdat;

/// The reference's decompressor (`FaCompress.c`), ported: a `Gw.dat` entry is compressed with an LZ +
/// Huffman scheme and this is the only thing that turns it into bytes.
pub mod fa_compress;

/// The decompressor's own tables, generated from the reference by `tools/gen-fa-compress-tables.py`.
pub mod fa_compress_tables;

/// A map blob: the reference's container (`GmMapDataImport.c`) and the path graph inside it, which is what
/// answers "which trapezoid is this position in" -- the first half of the reference's path finder.
pub mod map_data;

/// The reference's path finder (`GmPaths.c` + `GmPathHeap.c`): A* over the trapezoid graph, and the
/// waypoint reduction that turns the chosen chain into a walk. `tools/parity-path.py --check` compares it
/// against the reference's own code, case by case and bit for bit.
pub mod pathfind;
/// The travel doorways of each instance, measured from the client's own map data.
pub mod portals;

/// The path finder's fixed-point lookup tables, generated from `GmPaths.c` by `tools/gen-path-tables.py`.
pub mod path_tables;

/// The reference's `/tp` alias table (`s_teleport_targets`), generated from `GmCommands.c` by
/// `tools/gen-map-config.py`. Every alias it accepts, with the map it names -- which is not the same set as
/// "the labels of the maps this world hosts": `/tp lakeside` is an alias, "Lakeside County" is the label.
pub mod travel_targets;

/// What a new character wears, per campaign and profession, generated by `tools/gen-default-equipment.py`
/// from the reference's `GmDefaultArmors.c`. `GameSrv_HandleCharCreationChangeProf` answers a profession
/// change out of that same table.
pub mod default_equipment;

/// Why a connection ended.
#[derive(Debug)]
pub enum GameError {
    Io(io::Error),
    /// The first four bytes are not a game connection's.
    NotAGameConnection(version::VersionError),
    /// The version message was malformed or never finished arriving.
    BadVersion(version::GameVersionError),
    /// The client presented a token pair this world was not told about, or one already claimed, or a
    /// character that is not the one the transfer was issued for.
    UnknownTransfer {
        map_token: u32,
        player_token: u32,
    },
    /// The transfer is real but names a map this world does not host.
    UnknownMap(i32),
    /// The transfer is real but the character it names cannot be read.
    UnknownCharacter,
    /// A client message arrived that this world cannot frame, so the stream cannot be trusted.
    Framing(FrameError),
    /// The key exchange failed.
    Handshake(handshake::HandshakeError),
    /// The database failed.
    Database(asura_db::DatabaseError),
}

impl core::fmt::Display for GameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GameError::Io(err) => write!(f, "socket: {err}"),
            GameError::NotAGameConnection(err) => write!(f, "not a game connection: {err}"),
            GameError::BadVersion(err) => write!(f, "version: {err}"),
            GameError::UnknownTransfer {
                map_token,
                player_token,
            } => write!(
                f,
                "no pending transfer for world {map_token} with player token {player_token:#010X}"
            ),
            GameError::UnknownMap(map_id) => write!(f, "this world does not host map {map_id}"),
            GameError::UnknownCharacter => write!(f, "the character named by the transfer is gone"),
            GameError::Framing(err) => write!(f, "could not frame a client message: {err}"),
            GameError::Handshake(err) => write!(f, "key exchange: {err}"),
            GameError::Database(err) => write!(f, "database: {err}"),
        }
    }
}

impl std::error::Error for GameError {}

impl From<io::Error> for GameError {
    fn from(err: io::Error) -> Self {
        GameError::Io(err)
    }
}

/// One row as the wire's field widths. Every value here comes from a row this schema wrote, so a
/// narrowing that cannot fail is the only kind that can occur; a value that somehow does is clamped
/// rather than panicking a live connection.
fn wire_u8(value: i32) -> u8 {
    u8::try_from(value).unwrap_or(0)
}

fn wire_u16(value: i32) -> u16 {
    u16::try_from(value).unwrap_or(0)
}

fn wire_u32(value: i32) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

/// A fresh `u32` for a player token. The auth service mints the same kind
/// (`AuthSrv_HandleRequestGameInstance`; `crates/asura-auth/src/travel.rs:178` sends the field.
/// Milliseconds since this process started, for a log line that has to be placed in time.
///
/// The world's log carries no clock, and a delay measured by eye is not a measurement: the load block's own
/// line now carries this stamp, so the gap between two runs, or between two loads, is arithmetic.
pub fn mark_redirect(player_token: u32) {
    if let Ok(mut slot) = last_redirect().lock() {
        *slot = Some((player_token, std::time::Instant::now()));
    }
}

fn last_redirect() -> &'static std::sync::Mutex<Option<(u32, std::time::Instant)>> {
    static LAST: std::sync::OnceLock<std::sync::Mutex<Option<(u32, std::time::Instant)>>> =
        std::sync::OnceLock::new();
    LAST.get_or_init(|| std::sync::Mutex::new(None))
}

pub fn stamp_ms() -> u128 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
}
fn random_u32() -> Result<u32, GameError> {
    let mut bytes = [0u8; 4];
    getrandom::getrandom(&mut bytes)
        .map_err(|err| GameError::Io(io::Error::other(err.to_string())))?;
    Ok(u32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lakeside County, with the reference's verified blob and spawn.
    fn test_map() -> MapEntry {
        MapEntry {
            map_id: 146,
            map_file_id: 0x1B97D,
            map_type: 0,
            spawn: (6602.0, 4485.0),
            spawn_plane: 0,
            name: "Lakeside County".to_string(),
            capacity: 8,
        }
    }

    #[test]
    fn the_tick_and_ping_intervals_are_the_references() {
        // The ping is the reference's own constant; the tick cadence is the measured 100 ms (the frame is
        // the reference's 6-byte form either way).
        assert_eq!(TICK_INTERVAL, Duration::from_millis(100));
        assert_eq!(PING_INTERVAL, Duration::from_secs(5));
        assert_eq!(PING_VALUE, 20);
    }

    #[test]
    fn the_map_lookup_is_by_map_id() {
        let maps = [test_map()];
        let find = |map_id: i32| maps.iter().find(|map| map.map_id == map_id);
        assert!(find(146).is_some());
        assert!(find(15).is_none(), "a map this world does not host");
    }

    #[test]
    fn a_player_message_is_framed_by_the_generated_game_table() {
        // The three load requests, at the sizes the reference's own table gives them. A bare-header
        // message is two bytes; the players request needs all nineteen.
        let (frame, used) = Frame::decode_game_message(&[0x88, 0x80], true).expect("frames");
        assert_eq!((frame.header(), used), (0x8088, 2));

        assert!(matches!(
            Frame::decode_game_message(&[0x90, 0x80], true),
            Err(FrameError::Incomplete { .. })
        ));
        let mut full = vec![0x90, 0x80];
        full.resize(19, 0);
        let (frame, used) = Frame::decode_game_message(&full, true).expect("frames");
        assert_eq!((frame.header(), used), (0x8090, 19));

        let mut items = vec![0x91, 0x80];
        items.resize(4, 0);
        let (_, used) = Frame::decode_game_message(&items, true).expect("frames");
        assert_eq!(used, 4, "the item request is four bytes");
    }

    #[test]
    fn an_unknown_header_is_an_error_rather_than_a_guess() {
        // 0x8004 is a game header that exists; 0x7FFF is not. Framing either wrongly would consume
        // bytes that belong to the next message, so an unknown header ends the session instead.
        assert!(matches!(
            Frame::decode_game_message(&[0xFF, 0x7F], true),
            Err(FrameError::UnknownHeader { .. })
        ));
    }

    #[test]
    fn the_two_families_disagree_about_the_same_header_value() {
        // 0x8000 is AUTH_CMSG_HEARTBEAT on the auth connection and GAME_CMSG_TRADE_ACKNOWLEDGE here,
        // and the game message is longer than its header. That collision is why the world frames with
        // the game table and never with the auth one.
        let mut message = vec![0x00, 0x80];
        message.resize(6, 0); // whatever the trade acknowledge body is; the point is the header
        let (frame, used) = Frame::decode_game_message(&message, true).expect("a game header");
        assert_eq!(frame.header(), 0x8000);
        assert!(
            used > 2,
            "a game message with a body, not the auth heartbeat"
        );
        assert_eq!(
            asura_protocol::client::tables::entry(0x8000).map(|entry| entry.header),
            Some(0x8000),
            "and the auth table knows the same value for a different message"
        );
    }

    #[test]
    fn every_profession_starts_with_the_references_own_equipment() {
        use crate::default_equipment::{default_equipment, EQUIPMENT};

        // Four campaigns x eight professions, which is what `GetDefaultEquipments`'s switch covers.
        assert_eq!(EQUIPMENT.len(), 32, "four campaigns by eight professions");
        for (campaign, profession, items) in EQUIPMENT {
            assert!(
                !items.is_empty(),
                "campaign {campaign} profession {profession} has no items"
            );
        }

        // The first item of a Prophecies Warrior, verbatim from `g_DefWarriorPropheciesPveEquipments`:
        // the chest piece, grey, model 823, a four-unit name and three modifiers.
        let warrior = default_equipment(1, 1);
        assert_eq!(
            warrior.len(),
            6,
            "a Warrior's starting set: five armour pieces and a weapon"
        );
        let chest = warrior[0];
        assert_eq!(chest.file_id, 0x8000_005B);
        assert_eq!(chest.item_type, 7, "ItemType_Body");
        assert_eq!(chest.dye_colors, 11, "DyeColor_Gray");
        assert_eq!(chest.model_id, 823);
        assert_eq!(chest.name, &[0x22A3, 0xFB75, 0xA867, 0x466D]);
        assert_eq!(chest.modifiers, &[0xA3C8_1900, 0x8040_0000, 0xA0F8_1400]);

        // A pair the reference has no table for answers with nothing, rather than a guess.
        assert!(default_equipment(9, 1).is_empty());
    }

    #[test]
    fn the_block_sends_the_characters_own_appearance_not_the_default() {
        // The reference's load block carries whatever `HandleCharCreationConfirm` stored
        // (`GameSrv.c:1869-1886`). Before the traits existed this server sent
        // `Appearance::REFERENCE_DEFAULT` to everyone, so every character looked the same.
        let map = test_map();
        let standing = Standing::from_map(&map);
        let inventory = db_inventory::default_inventory();
        let traits = db_game::Traits {
            appearance: 0x1234_5678,
            profession: 9,
            unlocked_professions: 0x40,
            gold: 0,
            campaign: 1,
            secondary_profession: 0,
            level: 1,
        };
        let frames = player_block(
            "Asura",
            standing,
            &inventory,
            Instant::now(),
            PLAYER_ID,
            AGENT_ID,
            traits,
            crate::map::UNLOCKED_MAP_ZEROS,
        );
        let created = frames
            .iter()
            .find(|frame| frame.header() == game::PLAYER_CREATE)
            .expect("a player create");
        assert_eq!(
            u32::from_le_bytes([
                created.body()[8],
                created.body()[9],
                created.body()[10],
                created.body()[11]
            ]),
            traits.appearance,
            "the appearance the character was created with"
        );

        // A character with no bits recorded still gets the reference's default, because all-zero bits say
        // `Profession_None` and the client cannot draw that.
        let fallback = player_block(
            "Asura",
            standing,
            &inventory,
            Instant::now(),
            PLAYER_ID,
            AGENT_ID,
            db_game::Traits::default(),
            crate::map::UNLOCKED_MAP_ZEROS,
        );
        let created = fallback
            .iter()
            .find(|frame| frame.header() == game::PLAYER_CREATE)
            .expect("a player create");
        assert_eq!(
            u32::from_le_bytes([
                created.body()[8],
                created.body()[9],
                created.body()[10],
                created.body()[11]
            ]),
            game::Appearance::REFERENCE_DEFAULT.pack(),
            "an unrecorded appearance falls back to the reference's default"
        );
    }

    #[test]
    fn the_block_ends_with_the_message_that_leaves_the_loading_screen() {
        let frames = player_block(
            "Asura",
            Standing::from_map(&test_map()),
            &db_inventory::default_inventory(),
            Instant::now(),
            PLAYER_ID,
            AGENT_ID,
            db_game::Traits::default(),
            crate::map::UNLOCKED_MAP_ZEROS,
        );
        assert_eq!(
            frames.last().expect("a block").header(),
            game::INSTANCE_LOAD_FINISH
        );
        assert!(
            frames
                .iter()
                .all(|frame| frame.header() != game::MAP_INIT_START),
            "the exploration grid belongs to the load block, not here"
        );
        assert!(
            frames.len() > 20,
            "the block is the reference's twenty-odd messages"
        );
    }

    #[test]
    fn the_load_block_carries_the_exploration_grid_after_the_instance_info() {
        let frames = load_block(&test_map(), "Asura", AGENT_ID);
        let info = frames
            .iter()
            .position(|frame| frame.header() == game::INSTANCE_LOAD_INFO)
            .expect("the instance's identity");
        let announced = frames
            .iter()
            .position(|frame| frame.header() == game::MAP_INIT_START)
            .expect("the grid's dimensions are announced");
        let streamed = frames
            .iter()
            .position(|frame| frame.header() == game::CARTOGRAPHY_DATA)
            .expect("the grid is streamed");
        assert_eq!(
            info + 1,
            announced,
            "a live server sends the pair right after INSTANCE_LOAD_INFO, 0.58-0.73 s into the load"
        );
        assert!(
            announced < streamed,
            "the announcement comes before the data it sizes"
        );
        assert_eq!(
            frames[announced].len(),
            14,
            "three dwords after the header: the two dimensions and the byte count"
        );
    }

    #[test]
    fn the_manifest_has_six_messages_and_ends_on_the_map_itself() {
        let frames = manifest(146);
        assert_eq!(frames.len(), 6);
        assert_eq!(frames[0].header(), game::INSTANCE_MANIFEST_PHASE);
        assert_eq!(frames[2].header(), game::INSTANCE_MANIFEST_DONE);
        let last = frames[5].encode();
        assert_eq!(last[0..2], [0x98, 0x01], "the last is a manifest DONE");
        assert_eq!(
            last[2], PHASE_1,
            "the second DONE carries phase 1, as the reference sends it"
        );
        assert_eq!(last[3..5], 146u16.to_le_bytes(), "and it names the map");
    }

    /// The instance bitmap is addressed the way the client addresses it: instance `n` is bit `n % 32` of
    /// word `n / 32` (read from the client's world context, `docs/22-world-model.md`). A test rather than a
    /// comment, because the whole world map depends on the address: an off-by-one here reads as either
    /// "everywhere is locked" or "the wrong instance is open".
    #[test]
    fn the_unlock_bitmap_puts_each_instance_at_the_bit_the_client_reads() {
        let bitmap = map::unlock_bitmap(&[148, 145, 0, 991, 992, -1]);

        assert_eq!(
            bitmap.len(),
            map::UNLOCKED_MAP_ZEROS.len(),
            "31 words: the width the reference sends, and instances 0 to 991"
        );
        // 145 and 148 share word 4, so each bit is asked for on its own rather than the whole word.
        assert_eq!(
            bitmap[148 / 32] & (1 << (148 % 32)),
            1 << (148 % 32),
            "instance 148 is bit 20 of word 4"
        );
        assert_eq!(
            bitmap[145 / 32] & (1 << (145 % 32)),
            1 << (145 % 32),
            "instance 145 is bit 17 of the same word"
        );
        assert_eq!(bitmap[0], 1, "instance 0 is bit 0 of word 0");
        assert_eq!(
            bitmap[991 / 32] & (1 << (991 % 32)),
            1 << (991 % 32),
            "991 is the last addressable instance"
        );
        let set: u32 = bitmap.iter().map(|word| word.count_ones()).sum();
        assert_eq!(
            set, 4,
            "992 is past the last word and -1 is not an instance"
        );
    }

    /// The instance table the client is given: the batches a live load carried, each framed at the row's
    /// own layout (a `u8` count then `{u16 instance, u32 value}` pairs), and the same walker that frames
    /// a client message consumes exactly the bytes the builder wrote.
    #[test]
    fn the_instance_table_carries_what_the_capture_holds_and_frames_as_its_row_says() {
        let batches: usize = game::instance_table::INSTANCE_TABLE.len();
        assert_eq!(batches, 10, "the live load carried ten of these");

        let pairs: usize = game::instance_table::INSTANCE_TABLE
            .iter()
            .map(|batch| batch.len())
            .sum();
        assert_eq!(
            pairs, 137,
            "137 of the live table's 147 pairs: every recorded message was four bytes short, and the \
             ten values that were cut are absent rather than invented"
        );
        assert_eq!(
            game::instance_table::INSTANCE_TABLE[0][0],
            (148, 0xBB14_F539),
            "the first pair is the instance the capture was taken in"
        );

        for batch in game::instance_table::INSTANCE_TABLE {
            let frame = game::instance_table(batch);
            let bytes = frame.encode();
            assert_eq!(
                bytes.len(),
                2 + 1 + batch.len() * 6,
                "a u8 count then {} pairs",
                batch.len()
            );
            assert_eq!(
                bytes[2] as usize,
                batch.len(),
                "the count is the number of pairs"
            );
            let (_, used) =
                Frame::decode_game_message(&bytes, false).expect("the walker frames the table");
            assert_eq!(
                used,
                bytes.len(),
                "the row's layout and the builder agree byte for byte"
            );
        }
    }

    #[test]
    fn the_player_block_carries_the_name_in_the_two_messages_that_name_a_character() {
        let frames = player_block(
            "Asura",
            Standing::from_map(&test_map()),
            &db_inventory::default_inventory(),
            Instant::now(),
            PLAYER_ID,
            AGENT_ID,
            db_game::Traits::default(),
            crate::map::UNLOCKED_MAP_ZEROS,
        );
        let named: Vec<u16> = frames
            .iter()
            .filter(|frame| {
                frame.header() == game::PLAYER_CREATE
                    || frame.header() == game::INSTANCE_LOAD_PLAYER_NAME
            })
            .map(|frame| frame.header())
            .collect();
        assert_eq!(
            named,
            vec![game::PLAYER_CREATE],
            "only PLAYER_CREATE is in this block; the load name went out earlier"
        );
    }

    /// The inventory the reference's capture describes, as rows.
    fn reference_inventory() -> Inventory {
        db_inventory::default_inventory()
    }

    #[test]
    fn the_item_block_opens_with_the_stream_and_carries_the_capture() {
        let frames = items_block(&test_map(), &reference_inventory(), 0);
        assert_eq!(frames[0].header(), game::ITEM_STREAM_CREATE);
        assert_eq!(frames[1].header(), game::ITEM_SET_ACTIVE_WEAPON_SET);
        assert_eq!(
            frames.last().expect("a block").header(),
            game::INSTANCE_MANIFEST_DONE,
            "the answer ends with the download manifest, as the reference's does"
        );
        assert!(
            frames
                .iter()
                .any(|frame| frame.header() == game::CREATE_NAMED_ITEM),
            "the items are in it"
        );
        assert!(
            frames
                .iter()
                .any(|frame| frame.header() == game::ITEM_MOVE_TO_LOCATION),
            "and where they sit"
        );
    }

    #[test]
    fn the_bags_go_out_in_model_order_not_id_order() {
        // The trap the rows are read in the wrong order by: the capture's bags are ids 1, 2, 6, 3, 4, 5,
        // because the reference walks its bag *array*. Sorting by id sends a different stream, and a
        // client that reads the equipped-items bag before the backpack has nowhere to put the armour.
        let frames = inventory_block(&reference_inventory());
        let bags: Vec<u16> = frames
            .iter()
            .filter(|frame| frame.header() == game::INVENTORY_CREATE_BAG)
            .map(|frame| u16::from_le_bytes([frame.body()[4], frame.body()[5]]))
            .collect();
        assert_eq!(bags, vec![1, 2, 6, 3, 4, 5]);
    }

    #[test]
    fn an_agent_with_no_inventory_still_gets_a_well_formed_block() {
        // A character whose bags have not been written yet: the client must not be told about items that
        // do not exist, and the block must still end where the loading screen ends.
        let frames = items_block(&test_map(), &Inventory::default(), 0);
        assert_eq!(frames[0].header(), game::ITEM_STREAM_CREATE);
        assert!(!frames
            .iter()
            .any(|frame| frame.header() == game::CREATE_NAMED_ITEM));
        assert_eq!(
            frames.last().expect("a block").header(),
            game::INSTANCE_MANIFEST_DONE
        );
    }

    #[test]
    fn a_character_stands_where_it_left_and_nowhere_else() {
        let map = test_map();
        // Never saved: the map's own spawn.
        assert_eq!(Standing::resume(&map, None), Standing::from_map(&map));

        // Saved here: that position, on that plane.
        let saved = db_game::Position {
            map_id: map.map_id,
            x: 6700.0,
            y: 4400.0,
            plane: 3,
        };
        assert_eq!(
            Standing::resume(&map, Some(saved)),
            Standing {
                position: (6700.0, 4400.0),
                plane: 3
            }
        );

        // Saved in another map: this one's spawn, because that coordinate is not in this map.
        let elsewhere = db_game::Position {
            map_id: map.map_id + 1,
            x: 6700.0,
            y: 4400.0,
            plane: 3,
        };
        assert_eq!(
            Standing::resume(&map, Some(elsewhere)),
            Standing::from_map(&map),
            "a position is only usable on the map it was taken in"
        );
    }

    #[test]
    fn the_spawn_point_is_the_standing_position() {
        let map = test_map();
        let standing = Standing {
            position: (6700.0, 4400.0),
            plane: 2,
        };
        let frames = spawn_block(&map, standing);
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].header(),
            game::INSTANCE_LOAD_SPAWN_POINT,
            "the client is told the same coordinate the load info and the agent were built from"
        );
        assert_eq!(
            f32::from_le_bytes([
                frames[0].body()[4],
                frames[0].body()[5],
                frames[0].body()[6],
                frames[0].body()[7]
            ]),
            6700.0
        );
    }

    #[test]
    fn a_drop_sends_nothing_because_both_candidates_crashed_a_client() {
        // Not a placeholder: a tripwire. `ITEM_REMOVE` (`0x014D`) and an item agent (`0x0020`, type 4)
        // each crashed a live client, and neither appears in any capture. If a message is ever added to
        // this path, it has to arrive with a capture that shows it working -- so this test failing is the
        // reminder, and the reason is in `drop_messages`' documentation.
        assert!(
            drop_messages().is_empty(),
            "a drop must not put an unmeasured message on the wire"
        );
    }

    #[test]
    fn the_chat_messages_a_line_produces_are_the_references_pair() {
        // A spoken line is the text (a coded string, `game::player_chat_line`) and then the speaker.
        // Plain text crashed the client on its coded-string assert; the coded body is the shape the
        // Rurik project saw render on screen.
        let frames = game::player_chat_line(PLAYER_ID, game::CHANNEL_ALL, "hello");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].header(), game::CHAT_MESSAGE_CORE);
        assert_eq!(
            frames[0].len(),
            2 + 2 + 2 * 8,
            "wrapper, literal start, five units of text, literal end"
        );
        assert_eq!(frames[1].len(), 5, "a word for the player and a byte for the channel");
        assert_eq!(u32::from(frames[1].body()[2]), game::CHANNEL_ALL);
        assert_eq!(game::CHANNEL_GROUP, 11, "Channel_Group");
        assert_eq!(game::CHANNEL_TRADE, 12, "Channel_Trade");
    }

    #[test]
    fn maps_prints_the_references_table_and_nothing_else() {
        // `GmCommand_ListTargets` prints two things, and they are *not* the same list:
        //
        //   the chat half     one alias per map -- the first in the table, the one to type -- comma-separated
        //                     and wrapped, then the reference's fixed "Use: /tp <name>, e.g. /tp lakeside
        //                     county";
        //   the log table     **every alias**, one row each, header counting aliases. `%-15s` for the alias
        //                     and `%3d` for the map id, because a format that drifts is a format nobody can
        //                     compare.
        //
        // Collapsing the table to one row per map (a mistake this test used to pin) is exactly what made
        // `/maps` stop naming the aliases a tester has to type: map 146 has two, and both are answers.
        let answer = maps_answer(&[test_map()]);
        assert_eq!(
            answer.table,
            vec![
                "Teleport targets (2 aliases, every one with a map config):".to_string(),
                "  /tp lakeside        -> map 146  Lakeside County".to_string(),
                "  /tp lakeside county -> map 146  Lakeside County".to_string(),
            ],
            "the reference's header (counting aliases) and one row per alias"
        );
        assert_eq!(
            answer.chat,
            vec![
                "Maps: lakeside,".to_string(),
                "Use: /tp <name>, e.g. /tp lakeside county".to_string(),
            ],
            "and its chat half: the first alias of each map, then the reference's own sentence"
        );

        // Two maps: every alias of both, in the reference's table order (148 comes first in its table).
        let second = MapEntry {
            map_id: 148,
            name: "Ascalon City".to_string(),
            ..test_map()
        };
        let both = maps_answer(&[test_map(), second]);
        assert_eq!(
            both.table,
            vec![
                "Teleport targets (4 aliases, every one with a map config):".to_string(),
                "  /tp ascalon         -> map 148  Ascalon City (pre-Searing)".to_string(),
                "  /tp ascalon city    -> map 148  Ascalon City (pre-Searing)".to_string(),
                "  /tp lakeside        -> map 146  Lakeside County".to_string(),
                "  /tp lakeside county -> map 146  Lakeside County".to_string(),
            ],
            "table order, and the alias column is 15 wide"
        );
        assert_eq!(
            both.chat[0], "Maps: ascalon, lakeside,",
            "one chat line, one alias per map, comma-separated"
        );

        // A map with no name and no row in the reference's table has no alias to type; it prints its id
        // instead of inventing one.
        let unnamed = MapEntry {
            map_id: 50_146,
            name: String::new(),
            ..test_map()
        };
        let odd = maps_answer(&[unnamed]);
        assert_eq!(
            odd.table,
            vec![
                "Teleport targets (1 aliases, every one with a map config):".to_string(),
                "  /tp map 50146       -> map 50146  ".to_string(),
            ],
            "an unnamed, uncatalogued map is named by its id"
        );
    }

    #[test]
    fn every_alias_the_reference_accepts_resolves_to_its_map() {
        // The command layer's half of the map catalogue: `s_teleport_targets` has twenty aliases over
        // twelve maps, and `/tp` has to accept all twenty. Matching on the map's *label* accepted eight
        // (`lakeside county`, `the catacombs`, `ashford abbey`, `foibles fair`, `fort ranik`, ...) and
        // answered "unknown map name" for the reference's own short forms.
        assert_eq!(
            travel_targets::TRAVEL_TARGETS.len(),
            20,
            "the reference's table, generated"
        );

        for alias in [
            "lakeside",
            "lakeside county",
            "ascalon",
            "ascalon city",
            "ashford",
            "ashford abbey",
            "catacombs",
            "the catacombs",
            "northlands",
            "the northlands",
            "green hills",
            "barradin",
            "barradin estate",
            "wizards folly",
            "foibles",
            "foibles fair",
            "regent",
            "regent valley",
            "fort ranik",
            "kamadan",
        ] {
            let target = travel_target(alias).unwrap_or_else(|| panic!("/tp {alias} is an alias"));
            assert!(target.hosted, "{alias} names a map this server hosts");
        }

        // The rule is the reference's (`GmCommand_ToAscii`): case-insensitive, runs of spaces collapsed.
        assert_eq!(
            travel_target("  Lakeside   County ").map(|target| target.map_id),
            Some(146)
        );
        // And nothing else resolves. The reference's own comment claims "/tp LakesideCounty both work";
        // its code lower-cases and collapses spaces without splitting camel case, so it does not, and
        // neither does this.
        assert!(travel_target("LakesideCounty").is_none());
        assert!(travel_target("lakeside countyy").is_none());
        assert!(travel_target("").is_none());
    }

    #[test]
    fn stopping_and_stucking_are_different_messages() {
        // Two paths that look alike in a client and are not alike in the reference:
        //   `GAME_CMSG_CANCEL_MOVEMENT` -> `GameSrv_BroadcastAgentStopMoving`  (0x0028)
        //   `/stuck`                    -> `GameSrv_BroadcastAgentPosition`    (0x002C)
        // Sending the wrong one is the sort of parity bug that looks fine until someone compares traces.
        let stop = game::agent_stop_moving(1);
        assert_eq!(stop.header(), game::AGENT_STOP_MOVING);
        assert_eq!(stop.len(), 6, "header, then the agent id");

        let position = game::agent_update_position(1, (6602.0, 4485.0), 0);
        assert_eq!(position.header(), game::AGENT_UPDATE_POSITION);
        assert_eq!(position.len(), 16, "header, agent id, two floats, plane");
        assert_eq!(
            f32::from_le_bytes([
                position.body()[4],
                position.body()[5],
                position.body()[6],
                position.body()[7]
            ]),
            6602.0,
            "the x coordinate follows the agent id"
        );
        assert_eq!(
            u16::from_le_bytes([position.body()[12], position.body()[13]]),
            0,
            "and the plane is the last field"
        );
    }

    #[test]
    fn agent_ids_do_not_collide() {
        // A player's agent and a dropped item's are told apart: two agents sharing an id is one agent the
        // client draws and one it does not. The instance hands both out.
        let map = test_map();
        let instance = Instance::new(map.clone());
        let (outbox, _inbox) = tokio::sync::mpsc::unbounded_channel();
        let (_, player_agent) =
            instance.join("test".to_string(), Standing::from_map(&map), [0; 9], outbox);
        let first = instance.allocate_agent_id();
        let second = instance.allocate_agent_id();
        assert_eq!(first, player_agent + 1, "the player's id is taken");
        assert_ne!(first, second);
        assert_ne!(first, player_agent);
    }
}
