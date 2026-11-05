//! The game protocol: what a world server sends to a client that has just been placed in a map.
//!
//! `docs/03-protocol.md` section 11 records the order. This module owns the **messages**, one builder
//! per message, each written to the byte layout of the reference's packed struct (`GameMsg.h`) and
//! each tested against the size the reference's own trace records for it.
//!
//! Two rules that come from the reference and are not negotiable:
//!
//! * **The header is two bytes, and the direction is the top bit.** Client-to-server is `0x8000 | n`,
//!   server-to-client is `n` (GWPS `opcodes.h`). A world server that swaps them sends a client a
//!   message it reads as a different one.
//! * **A counted field is a `u16` count followed by exactly that many units**, with no padding -- the
//!   same rule as the auth path (`client::frame`). The reference's structs declare wider counters
//!   (`uint32_t n_name`), which is why `sizeof(struct)` is not the wire size and why these builders
//!   write the count themselves instead of mirroring a struct.
//!
//! Everything here is server-to-client. The client-to-server messages a world server must *read* are
//! framed by the generated game table (`super::client::game_tables`) through
//! `super::client::Frame::decode_game_message`.

/// The unlockable PvP item table, generated from the reference: 387 `(id, params)` pairs the client
/// builds its item table from. Data rather than protocol, which is why it is generated and not typed.
/// The client's `MapType` values, as configuration holds them.
///
/// Three numbers that mean "a town", "an explorable area" and "the creation instance" to the client. They
/// live here because they are the client's vocabulary, and they are *values*, so a server reads them from
/// `config/Server/defaults.json` rather than compiling them (`docs/38-layer-decisions.md`, D34): a file
/// states them, a rule is handed them, and this type is what travels between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapKinds {
    /// A town or outpost: a destination the outpost rule never overrides.
    pub main_town: i32,
    /// An explorable area: a destination placement replaces with the last outpost.
    pub main_explorable: i32,
    /// The creation instance: the one kind that is a conversation rather than a place.
    pub character_creation: i32,
}

pub mod appearance;
/// Re-exported so `Appearance::...` reads the same as it did when the type lived here.
pub use appearance::Appearance;

/// The title family's six messages (`0x00F3`-`0x00F6`, `0x8058`, `0x8059`): builders and the one reader.
pub mod title;

/// `GAME_SMSG_INSTANCE_LOAD_HEAD` (`0x017C`): 6 bytes. The first message a client receives on the
/// game connection.
pub const INSTANCE_LOAD_HEAD: u16 = crate::packets::game::GAME_SMSG_INSTANCE_LOAD_HEAD;
/// `GAME_SMSG_INSTANCE_PLAYER_DATA_START` (`0x0186`): 2 bytes.
pub const INSTANCE_PLAYER_DATA_START: u16 =
    crate::packets::game::GAME_SMSG_INSTANCE_PLAYER_DATA_START;
/// `GAME_SMSG_INSTANCE_PLAYER_DATA_DONE` (`0x018A`): 2 bytes.
pub const INSTANCE_PLAYER_DATA_DONE: u16 =
    crate::packets::game::GAME_SMSG_INSTANCE_PLAYER_DATA_DONE;
/// `GAME_SMSG_CHAR_CREATION_SUCCESS` (`0x0188`): 16-byte uuid, counted name, `0x2211`, counted settings.
pub const CHAR_CREATION_SUCCESS: u16 = crate::packets::game::GAME_SMSG_CHAR_CREATION_SUCCESS;
/// `GAME_SMSG_CHAR_CREATION_START` (`0x0189`): 2 bytes.
pub const CHAR_CREATION_START: u16 = crate::packets::game::GAME_SMSG_CHAR_CREATION_START;
/// `GAME_SMSG_CHAR_CREATION_ERROR` (`0x018B`): 6 bytes, header and an error code.
pub const CHAR_CREATION_ERROR: u16 = crate::packets::game::GAME_SMSG_CHAR_CREATION_ERROR;
/// `GAME_SMSG_INSTANCE_LOAD_PLAYER_NAME` (`0x017D`): `2 + 2 + 2n`.
pub const INSTANCE_LOAD_PLAYER_NAME: u16 =
    crate::packets::game::GAME_SMSG_INSTANCE_LOAD_PLAYER_NAME;
/// `GAME_SMSG_INSTANCE_LOAD_INFO` (`0x019A`): 16 bytes.
pub const INSTANCE_LOAD_INFO: u16 = crate::packets::game::GAME_SMSG_INSTANCE_LOAD_INFO;
/// `GAME_SMSG_INSTANCE_LOAD_SPAWN_POINT` (`0x0196`): 26 bytes.
pub const INSTANCE_LOAD_SPAWN_POINT: u16 =
    crate::packets::game::GAME_SMSG_INSTANCE_LOAD_SPAWN_POINT;
/// `GAME_SMSG_INSTANCE_LOADED` (`0x00F2`): 6 bytes.
pub const INSTANCE_LOADED: u16 = crate::packets::game::GAME_SMSG_INSTANCE_LOADED;
/// `GAME_SMSG_INSTANCE_LOAD_FINISH` (`0x018E`): 2 bytes. The client leaves the loading screen on it.
pub const INSTANCE_LOAD_FINISH: u16 = crate::packets::game::GAME_SMSG_INSTANCE_LOAD_FINISH;
/// `GAME_SMSG_INSTANCE_MANIFEST_PHASE` (`0x0199`): 3 bytes.
pub const INSTANCE_MANIFEST_PHASE: u16 = crate::packets::game::GAME_SMSG_INSTANCE_MANIFEST_PHASE;
/// `GAME_SMSG_INSTANCE_MANIFEST_DONE` (`0x0198`): 9 bytes.
pub const INSTANCE_MANIFEST_DONE: u16 = crate::packets::game::GAME_SMSG_INSTANCE_MANIFEST_DONE;

/// `GAME_SMSG_MAP_UPDATE_CURRENT` (`0x0099`), the **login** shape: 5 bytes, `map_id` is a `u16`.
///
/// The redirect shape of the same opcode is 12 bytes with a `u32` map id. Both exist in the reference
/// (`GameMsg.h`, `MapUpdateCurrent` comment) and getting them the wrong way round is the trap that
/// costs a client error 007.
pub const MAP_UPDATE_CURRENT: u16 = crate::packets::game::GAME_SMSG_MAP_UPDATE_CURRENT;
/// `GAME_SMSG_TRANSFER_GAME_SERVER_INFO` (`0x01A6`): the redirect a travel ends with. Re-exported from
/// [`crate::client`], where the auth service's copy of the same message already lives - one constant, so
/// the two connections cannot drift about which header a redirect uses.
pub const TRANSFER_GAME_SERVER_INFO: u16 = crate::client::TRANSFER_GAME_SERVER_INFO;
/// `GAME_SMSG_READY_FOR_MAP_SPAWN` (`0x01AC`): 6 bytes.
pub const READY_FOR_MAP_SPAWN: u16 = crate::packets::game::GAME_SMSG_READY_FOR_MAP_SPAWN;
/// `GAME_SMSG_MAP_UPDATE_UNLOCKED_LIST` (`0x0094`): five counted lists of dwords.
pub const MAP_UPDATE_UNLOCKED_LIST: u16 = crate::packets::game::GAME_SMSG_MAP_UPDATE_UNLOCKED_LIST;

/// `GAME_SMSG_MAP_INIT_START` (`0x008B`, 14 bytes): the exploration grid's dimensions and how many
/// compressed bytes the `0x008A` messages that follow carry.
///
/// Unnamed in the generated table (three dwords is all it records); the name is the client's own
/// handler, `OnMsgSrvMapInitStart`, and the shape is measured - `{dims.x, dims.y, compressed
/// byte count}`, three dwords. See [`map_init_start`].
pub const MAP_INIT_START: u16 = crate::packets::game::GAME_SMSG_MAP_INIT_START;
/// `GAME_SMSG_CARTOGRAPHY_DATA` (`0x008A`, declared maximum 262 bytes): one slice of the exploration
/// grid.
pub const CARTOGRAPHY_DATA: u16 = crate::packets::game::GAME_SMSG_CARTOGRAPHY_DATA;
/// `GAME_SMSG_MAP_EXPLORATION_MARK` (`0x008C`, 11 bytes): reveal a `(2 * half_span + 1)` square.
pub const MAP_EXPLORATION_MARK: u16 = 0x008C;
/// The dwords the fixed frame carries after its `u32` count: 63 dwords, then the two bytes that make
/// the client's 254-byte payload.
///
/// The packet table declares 64 dwords because that is the client's own array capacity; the frame
/// this client frames to is the 63-dword shape below. A `u16` count shifts the stream by two bytes,
/// and a 262-byte frame is read as the next message's header - both measured against this client on
/// 2026-11-04. The count stays four bytes wide, so the stream begins at byte 6.
pub const CARTOGRAPHY_DATA_SLOTS: usize = 63;
/// The blob bytes one [`CARTOGRAPHY_DATA`] message carries.
pub const CARTOGRAPHY_DATA_BYTES: usize = CARTOGRAPHY_DATA_SLOTS * 4;

/// `GAME_SMSG_ITEM_STREAM_CREATE` (`0x0144`): 5 bytes.
pub const ITEM_STREAM_CREATE: u16 = crate::packets::game::GAME_SMSG_ITEM_STREAM_CREATE;
/// `GAME_SMSG_INVENTORY_CREATE_BAG` (`0x013F`): 13 bytes.
pub const INVENTORY_CREATE_BAG: u16 = crate::packets::game::GAME_SMSG_INVENTORY_CREATE_BAG;
/// `GAME_SMSG_CREATE_NAMED_ITEM` (`0x0161`): `34 + 2n + 4m`, and no fixed size - see the builder.
pub const CREATE_NAMED_ITEM: u16 = crate::packets::game::GAME_SMSG_CREATE_NAMED_ITEM;
/// `GAME_SMSG_CREATE_UNNAMED_ITEM` (`0x015F`): 25 bytes, the same item without its name. The reference
/// sends this one for bag slots and equipped items (`GmItem.c:111-160`) and the named one for the
/// inventory stream (`GameSrv_SendItemById`).
pub const CREATE_UNNAMED_ITEM: u16 = crate::packets::game::GAME_SMSG_CREATE_UNNAMED_ITEM;
/// `GAME_SMSG_ITEM_MOVE_TO_LOCATION` (`0x013E`): 11 bytes.
pub const ITEM_MOVE_TO_LOCATION: u16 = crate::packets::game::GAME_SMSG_ITEM_MOVED_TO_LOCATION;
/// `GAME_SMSG_ITEM_REMOVE` (`0x014D`): 8 bytes. Takes an item out of the client's inventory model.
///
/// The reference has this message and uses it when it frees a bag (`GameSrv_FreeBagItems`); it is not
/// used anywhere near a drop, because its drop handler never removes the item. A port that does remove
/// one has to tell the client, and this is the message that does it.
pub const ITEM_REMOVE: u16 = crate::packets::game::GAME_SMSG_ITEM_REMOVE;
/// `GAME_SMSG_ITEM_SET_PROFESSION` (`0x015A`): 7 bytes.
pub const ITEM_SET_PROFESSION: u16 = crate::packets::game::GAME_SMSG_ITEM_SET_PROFESSION;
/// `GAME_SMSG_ITEM_SET_ACTIVE_WEAPON_SET` (`0x0148`): 5 bytes.
pub const ITEM_SET_ACTIVE_WEAPON_SET: u16 =
    crate::packets::game::GAME_SMSG_ITEM_SET_ACTIVE_WEAPON_SET;
/// `GAME_SMSG_ITEM_WEAPON_SET` (`0x0147`): 13 bytes, sent once per weapon slot.
pub const ITEM_WEAPON_SET: u16 = crate::packets::game::GAME_SMSG_ITEM_WEAPON_SET;
/// `GAME_SMSG_UPDATE_GOLD_STORAGE` (`0x0141`): 8 bytes.
pub const UPDATE_GOLD_STORAGE: u16 = crate::packets::game::GAME_SMSG_UPDATE_GOLD_STORAGE;
/// `GAME_SMSG_CHARACTER_FACTION_MAX_KURZICK` (`0x00EA`): 6 bytes.
pub const FACTION_MAX_KURZICK: u16 = crate::packets::game::GAME_SMSG_CHARACTER_FACTION_MAX_KURZICK;
/// `GAME_SMSG_CHARACTER_FACTION_MAX_LUXON` (`0x00EB`): 6 bytes.
pub const FACTION_MAX_LUXON: u16 = crate::packets::game::GAME_SMSG_CHARACTER_FACTION_MAX_LUXON;
/// `GAME_SMSG_CHARACTER_FACTION_MAX_BALTHAZAR` (`0x00EC`): 6 bytes.
pub const FACTION_MAX_BALTHAZAR: u16 =
    crate::packets::game::GAME_SMSG_CHARACTER_FACTION_MAX_BALTHAZAR;
/// `GAME_SMSG_CHARACTER_FACTION_MAX_IMPERIAL` (`0x00ED`): 6 bytes.
pub const FACTION_MAX_IMPERIAL: u16 =
    crate::packets::game::GAME_SMSG_CHARACTER_FACTION_MAX_IMPERIAL;
/// `GAME_SMSG_CHARACTER_UPDATE_INFO` (`0x0030`): `28 + 2n`.
pub const CHARACTER_UPDATE_INFO: u16 = crate::packets::game::GAME_SMSG_CHARACTER_UPDATE_INFO;
/// `GAME_SMSG_HARD_MODE_UNLOCKED` (`0x0070`): 3 bytes.
pub const HARD_MODE_UNLOCKED: u16 = crate::packets::game::GAME_SMSG_HARD_MODE_UNLOCKED;
/// `GAME_SMSG_CHARACTER_UPDATE_FACTIONS` (`0x00E9`): 62 bytes.
pub const CHARACTER_UPDATE_FACTIONS: u16 =
    crate::packets::game::GAME_SMSG_CHARACTER_UPDATE_FACTIONS;
/// `GAME_SMSG_CHAR_VISIBILITY_FLAGS` (`0x00EF`): 10 bytes - the inventory panel's per-slot display mode.
pub const CHAR_VISIBILITY_FLAGS: u16 = crate::packets::game::GAME_SMSG_CHAR_VISIBILITY_FLAGS;
/// `GAME_SMSG_PLAYER_UPDATE_FLAGS` (`0x003C`): 12 bytes - a player's flags word, written under a mask.
pub const PLAYER_UPDATE_FLAGS: u16 = crate::packets::game::GAME_SMSG_PLAYER_UPDATE_FLAGS;

/// `GAME_SMSG_PVP_UPDATE_UNLOCKED_SKILLS` (`0x001D`): `4 + 4n`.
pub const PVP_UPDATE_UNLOCKED_SKILLS: u16 =
    crate::packets::game::GAME_SMSG_PVP_UPDATE_UNLOCKED_SKILLS;
/// `GAME_SMSG_PVP_UPDATE_UNLOCKED_HEROES` (`0x0018`): `4 + 4n`.
pub const PVP_UPDATE_UNLOCKED_HEROES: u16 =
    crate::packets::game::GAME_SMSG_PVP_UPDATE_UNLOCKED_HEROES;
/// `GAME_SMSG_PVP_ITEM_STREAM_ADD` (`0x001A`): `5 + 4n`.
pub const PVP_ITEM_STREAM_ADD: u16 = crate::packets::game::GAME_SMSG_PVP_ITEM_STREAM_ADD;
/// `GAME_SMSG_PVP_ITEM_STREAM_END` (`0x001B`): 2 bytes.
pub const PVP_ITEM_STREAM_END: u16 = crate::packets::game::GAME_SMSG_PVP_ITEM_STREAM_END;
/// `GAME_SMSG_ACCOUNT_FEATURE` (`0x000F`): 8 bytes.
pub const ACCOUNT_FEATURE: u16 = crate::packets::game::GAME_SMSG_ACCOUNT_FEATURE;

/// `GAME_SMSG_AGENT_UPDATE_ATTRIBUTE_POINTS` (`0x0037`): 8 bytes.
pub const AGENT_UPDATE_ATTRIBUTE_POINTS: u16 =
    crate::packets::game::GAME_SMSG_AGENT_UPDATE_ATTRIBUTE_POINTS;
/// `GAME_SMSG_PLAYER_UPDATE_PROFESSION` (`0x00B7`): 9 bytes.
pub const PLAYER_UPDATE_PROFESSION: u16 = crate::packets::game::GAME_SMSG_PLAYER_UPDATE_PROFESSION;
/// `GAME_SMSG_PLAYER_UPDATE_UNLOCKED_PROFESSIONS` (`0x00B6`): 10 bytes.
pub const PLAYER_UPDATE_UNLOCKED_PROFESSIONS: u16 =
    crate::packets::game::GAME_SMSG_PLAYER_UPDATE_UNLOCKED_PROFESSIONS;
/// `GAME_SMSG_SKILLBAR_UPDATE` (`0x00DA`): `11 + 4n_skills + 4n_masks + 1`.
pub const SKILLBAR_UPDATE: u16 = crate::packets::game::GAME_SMSG_SKILLBAR_UPDATE;
/// `GAME_SMSG_UPDATE_UNLOCKED_SKILLS` (`0x00DB`): `4 + 4n`.
pub const UPDATE_UNLOCKED_SKILLS: u16 = crate::packets::game::GAME_SMSG_UPDATE_UNLOCKED_SKILLS;
/// `GAME_SMSG_AGENT_UPDATE_ATTRIBUTES` (`0x003A`): `8 + 4n`.
pub const AGENT_UPDATE_ATTRIBUTES: u16 = crate::packets::game::GAME_SMSG_AGENT_UPDATE_ATTRIBUTES;
/// `GAME_SMSG_WORLD_UPDATE_LOAD_TIME` (`0x001F`): 6 bytes.
pub const WORLD_UPDATE_LOAD_TIME: u16 = crate::packets::game::GAME_SMSG_WORLD_UPDATE_LOAD_TIME;

/// `GAME_SMSG_PLAYER_CREATE` (`0x0059`): `25 + 2n`.
pub const PLAYER_CREATE: u16 = crate::packets::game::GAME_SMSG_PLAYER_CREATE;
/// `GAME_SMSG_WORLD_CREATE_AGENT` (`0x0020`): 99 bytes, fixed.
pub const WORLD_CREATE_AGENT: u16 = crate::packets::game::GAME_SMSG_WORLD_CREATE_AGENT;
/// `GAME_SMSG_WORLD_REMOVE_AGENT` (`0x0021`): 6 bytes. An agent left the map (`GmAgent.c:60`).
pub const WORLD_REMOVE_AGENT: u16 = crate::packets::game::GAME_SMSG_WORLD_REMOVE_AGENT;
/// `GAME_SMSG_PLAYER_DESTROY` (`0x005A`): 6 bytes. A player left the map (`GmPlayer.c:314`).
pub const PLAYER_DESTROY: u16 = crate::packets::game::GAME_SMSG_PLAYER_DESTROY;
/// `GAME_SMSG_AGENT_UPDATE_MORALE` (`0x009C`): 10 bytes - an agent id and its morale percentage.
pub const AGENT_UPDATE_MORALE: u16 = crate::packets::game::GAME_SMSG_AGENT_UPDATE_MORALE;
/// `GAME_SMSG_AGENT_PROPERTY_UPDATE_INT` (`0x009F`): 14 bytes.
pub const AGENT_PROPERTY_UPDATE_INT: u16 =
    crate::packets::game::GAME_SMSG_AGENT_PROPERTY_UPDATE_INT;
/// `GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT` (`0x00A2`): 14 bytes.
pub const AGENT_PROPERTY_UPDATE_FLOAT: u16 =
    crate::packets::game::GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT;
/// `GAME_SMSG_AGENT_INITIAL_EFFECTS` (`0x00F0`): 10 bytes.
pub const AGENT_INITIAL_EFFECTS: u16 = crate::packets::game::GAME_SMSG_AGENT_INITIAL_EFFECTS;
/// `GAME_SMSG_AGENT_UPDATE_EFFECTS` (`0x00F1`): 10 bytes.
pub const AGENT_UPDATE_EFFECTS: u16 = crate::packets::game::GAME_SMSG_AGENT_UPDATE_EFFECTS;
/// `GAME_SMSG_UPDATE_AGENT_VISUAL_EQUIPMENT` (`0x006E`): 42 bytes.
pub const UPDATE_AGENT_VISUAL_EQUIPMENT: u16 =
    crate::packets::game::GAME_SMSG_UPDATE_AGENT_VISUAL_EQUIPMENT;
/// `GAME_SMSG_AGENT_DISPLAY_CAPE` (`0x0048`): 7 bytes.
pub const AGENT_DISPLAY_CAPE: u16 = crate::packets::game::GAME_SMSG_AGENT_DISPLAY_CAPE;
/// `GAME_SMSG_WORLD_UPDATE_CONTROLLED_AGENT` (`0x0022`): 10 bytes.
pub const WORLD_UPDATE_CONTROLLED_AGENT: u16 =
    crate::packets::game::GAME_SMSG_WORLD_UPDATE_CONTROLLED_AGENT;

/// `GAME_SMSG_PARTY_CREATE` (`0x01D3`): 4 bytes.
pub const PARTY_CREATE: u16 = crate::packets::game::GAME_SMSG_PARTY_CREATE;
/// `GAME_SMSG_PARTY_PLAYER_ADD` (`0x01CC`): 7 bytes.
pub const PARTY_PLAYER_ADD: u16 = crate::packets::game::GAME_SMSG_PARTY_PLAYER_ADD;
/// `GAME_SMSG_PARTY_MEMBER_STREAM_END` (`0x01D4`): 4 bytes.
pub const PARTY_MEMBER_STREAM_END: u16 = crate::packets::game::GAME_SMSG_PARTY_MEMBER_STREAM_END;
/// `GAME_SMSG_CHARACTER_UDPATE_PARTY` (`0x01B3`, the reference's own spelling): 5 bytes.
pub const CHARACTER_UPDATE_PARTY: u16 = crate::packets::game::GAME_SMSG_CHARACTER_UDPATE_PARTY;
/// `GAME_SMSG_PLAYER_UPDATE_PARTY_SIZE` (`0x00B0`): 12 bytes on the wire (the table says 5 - the
/// capture wins; see the builder).
pub const PLAYER_UPDATE_PARTY_SIZE: u16 = crate::packets::game::GAME_SMSG_PLAYER_UPDATE_PARTY_SIZE;
/// `GAME_SMSG_PLAYER_UPDATE_PARTY` (`0x00B1`): 12 bytes on the wire (the table says 6 - same rule).
pub const PLAYER_UPDATE_PARTY: u16 = crate::packets::game::GAME_SMSG_PLAYER_UPDATE_PARTY;

/// `GAME_SMSG_CHAT_MESSAGE_CORE` (`0x005D`): the text of a chat message, `2 + 2 + 2n`.
pub const CHAT_MESSAGE_CORE: u16 = crate::packets::game::GAME_SMSG_CHAT_MESSAGE_CORE;
/// `GAME_SMSG_CHAT_MESSAGE_SERVER` (`0x005E`): 5 bytes. The server speaking on a channel - what the
/// reference answers a command with, because a core message on its own renders nothing.
pub const CHAT_MESSAGE_SERVER: u16 = crate::packets::game::GAME_SMSG_CHAT_MESSAGE_SERVER;
/// `GAME_SMSG_CHAT_MESSAGE_LOCAL` (`0x0061`): which player spoke, on which channel. 5 bytes.
pub const CHAT_MESSAGE_LOCAL: u16 = crate::packets::game::GAME_SMSG_CHAT_MESSAGE_LOCAL;
/// `Channel_All` in the client's own enum (`GmChat.h`): the channel `!` speaks on.
pub const CHANNEL_ALL: u32 = 3;
/// `Channel_Group`: the channel `#` speaks on.
pub const CHANNEL_GROUP: u32 = 11;
/// `Channel_Trade`: the channel `$` speaks on.
pub const CHANNEL_TRADE: u32 = 12;
/// `Channel_Warning`: the middle-of-the-screen channel the reference answers a command on.
pub const CHANNEL_WARNING: u32 = 7;

/// `GAME_SMSG_CHAT_MESSAGE_CORE`: one fragment of the coded-string body.
pub fn chat_message_core(text: &str) -> crate::client::Frame {
    Builder::new(CHAT_MESSAGE_CORE).text(text, 122).frame()
}

/// The leading word of a player's All-channel chat body: string id 8, the All-chat body wrapper.
pub const CHAT_BODY_WRAPPER: u16 = 0x0108;
/// The marker that opens a literal run inside a coded string (string id 7).
pub const CODED_LITERAL_START: u16 = 0x0107;
/// The terminator that closes a literal run (a marker word, below `0x100`).
pub const CODED_LITERAL_END: u16 = 0x0001;
/// Units per `CHAT_MESSAGE_CORE` fragment: the declared 122 minus one.
pub const CHAT_FRAGMENT_UNITS: usize = 121;

/// A typed chat line as the coded string the client renders: wrapper, literal start, the text,
/// literal end.
///
/// `CHAT_MESSAGE_CORE` carries a *coded* string: a word below `0x100` is a control marker and a word
/// at or above it is a string id. Plain ASCII is all below `0x100`, so bare text opens with a marker
/// the client does not expect and hits its coded-string assert. Units below `0x20` are dropped so a
/// stray control code cannot end the literal early.
pub fn chat_body_units(text: &str) -> Vec<u16> {
    let mut units = vec![CHAT_BODY_WRAPPER, CODED_LITERAL_START];
    units.extend(text.encode_utf16().filter(|unit| *unit >= 0x20));
    units.push(CODED_LITERAL_END);
    units
}

/// One player's chat line, ready to send: the coded body in `CHAT_MESSAGE_CORE` fragments of at most
/// [`CHAT_FRAGMENT_UNITS`] units, then the `CHAT_MESSAGE_LOCAL` tag that commits the line to
/// `channel` under `player_id`.
pub fn player_chat_line(player_id: u32, channel: u32, text: &str) -> Vec<crate::client::Frame> {
    let body = chat_body_units(text);
    let mut frames: Vec<crate::client::Frame> = body
        .chunks(CHAT_FRAGMENT_UNITS)
        .map(|part| {
            Builder::new(CHAT_MESSAGE_CORE)
                .units(part, CHAT_FRAGMENT_UNITS)
                .frame()
        })
        .collect();
    frames.push(chat_message_local(player_id, channel));
    frames
}

/// `GAME_SMSG_CHAT_MESSAGE_LOCAL`: who is speaking, and on which channel - a word and a byte, 5 bytes.
///
/// The generated table's two-dword form was read as 5 bytes plus the start of another message, and
/// the client dropped the connection right after the line rendered.
pub fn chat_message_local(player_id: u32, channel: u32) -> crate::client::Frame {
    Builder::new(CHAT_MESSAGE_LOCAL)
        .u16(player_id as u16)
        .u8(channel as u8)
        .frame()
}

/// `GAME_SMSG_CHAT_MESSAGE_SERVER`: a server-composed line, committed under a subject player on a
/// channel - a word and a byte, 5 bytes, the same shape as [`chat_message_local`].
pub fn chat_message_server(subject_player_id: u32, channel: u32) -> crate::client::Frame {
    Builder::new(CHAT_MESSAGE_SERVER)
        .u16(subject_player_id as u16)
        .u8(channel as u8)
        .frame()
}

/// `EquippedItemSlot` (`GmInventory.h`): the equipped-items bag slot an item of this `ItemType`
/// (`GmItem.h`) is worn in.
///
/// Six of these are measured - the reference's default character carries a hammer (`15` -> weapon), a
/// body, legs, head, boots and gloves, and the capture places each of them in the slot below. The rest
/// follow the client's own enum names rather than a measurement, which is worth saying: a weapon is a
/// weapon, but whether a `Present` or a `Minipet` is equippable at all is a guess answered by *no* here
/// rather than by a slot.
///
/// `None` means "this is not something you wear"; the caller refuses the equip rather than inventing a
/// slot for it.
pub fn equipped_slot(item_type: u8) -> Option<u8> {
    /// `EquippedItemSlot_Weapon` and friends, in the client's order.
    const WEAPON: u8 = 0;
    const OFF_HAND: u8 = 1;
    const BODY: u8 = 2;
    const LEGS: u8 = 3;
    const HEAD: u8 = 4;
    const BOOTS: u8 = 5;
    const GLOVES: u8 = 6;
    const COSTUME_BODY: u8 = 7;
    const COSTUME_HEAD: u8 = 8;

    let slot = match item_type {
        // `ItemType_Leadhand`, `Axe`, `Bow`, `Hammer`, `Wand`, `Staff`, `Sword`, `Daggers`, `Scythe`,
        // `Spear`. A leadhand is the focus/hand pair's weapon half, and the reference's own default
        // character wears the hammer in slot 0.
        1 | 2 | 5 | 15 | 22 | 26 | 27 | 32 | 35 | 36 => WEAPON,
        // `ItemType_Focus`, `Shield`.
        12 | 24 => OFF_HAND,
        7 => BODY,
        19 => LEGS,
        16 => HEAD,
        4 => BOOTS,
        13 => GLOVES,
        44 => COSTUME_BODY,
        45 => COSTUME_HEAD,
        _ => return None,
    };
    Some(slot)
}

/// `GAME_SMSG_WORLD_SIMULATION_TICK` (`0x001E`): 6 bytes.
pub const WORLD_SIMULATION_TICK: u16 = crate::packets::game::GAME_SMSG_WORLD_SIMULATION_TICK;
/// The mode [`agent_update_speed_live`] carries when a walk **starts**. Measured: every captured start
/// carries 1.
pub const AGENT_SPEED_MODE_START: u32 = 1;
/// The mode [`agent_update_speed_live`] carries when a walk is **cancelled**. Measured: every captured
/// cancel carries 9.
pub const AGENT_SPEED_MODE_STOP: u32 = 9;

/// `GAME_SMSG_AGENT_MOVE_TO_POINT` (`0x0029`): 24 bytes as the live client reads it. Tells the client
/// where an agent is walking to.
pub const AGENT_MOVE_TO_POINT: u16 = crate::packets::game::GAME_SMSG_AGENT_MOVE_TO_POINT;
/// `GAME_SMSG_AGENT_UPDATE_DIRECTION` (`0x0025`): 20 bytes. The direction an agent is walking in.
pub const AGENT_UPDATE_DIRECTION: u16 = crate::packets::game::GAME_SMSG_AGENT_UPDATE_DIRECTION;
/// `GAME_SMSG_AGENT_UPDATE_SPEED` (`0x002B`): 16 bytes. An agent's speed and whether it is walking.
pub const AGENT_UPDATE_SPEED: u16 = crate::packets::game::GAME_SMSG_AGENT_UPDATE_SPEED;
/// `GAME_SMSG_AGENT_STOP_MOVING` (`0x0028`): 6 bytes. Only the two-byte-header dialect has it.
pub const AGENT_STOP_MOVING: u16 = crate::packets::game::GAME_SMSG_AGENT_STOP_MOVING;
/// `GAME_SMSG_AGENT_UPDATE_POSITION` (`0x002C`): 16 bytes.
pub const AGENT_UPDATE_POSITION: u16 = crate::packets::game::GAME_SMSG_AGENT_UPDATE_POSITION;
/// `GAME_SMSG_PING_REQUEST` (`0x000C`): 2 bytes.
pub const PING_REQUEST: u16 = crate::packets::game::GAME_SMSG_PING_REQUEST;
/// `GAME_SMSG_PING_REPLY` (`0x000D`): 6 bytes.
pub const PING_REPLY: u16 = crate::packets::game::GAME_SMSG_PING_REPLY;

/// The agent/character property ids this module needs (`GmAgentProperties.h`).
pub const AGENT_PROPERTY_PUBLIC_LEVEL: u32 = 36;
/// See [`AGENT_PROPERTY_PUBLIC_LEVEL`].
pub const AGENT_PROPERTY_ENERGY: u32 = 41;
/// See [`AGENT_PROPERTY_PUBLIC_LEVEL`].
pub const AGENT_PROPERTY_HEALTH: u32 = 42;
/// See [`AGENT_PROPERTY_PUBLIC_LEVEL`].
pub const AGENT_PROPERTY_ENERGY_REGEN: u32 = 43;

/// `GameSrv_CreatePlayerAgent`'s defaults: a player agent is a living agent that moves at 288 units a
/// second, with 400 health and 25 energy. The reference hardcodes them and so does this.
pub const PLAYER_SPEED_BASE: f32 = 288.0;
/// See [`PLAYER_SPEED_BASE`].
pub const PLAYER_HEALTH_MAX: u32 = 400;
/// See [`PLAYER_SPEED_BASE`].
pub const PLAYER_ENERGY_MAX: u32 = 25;
/// See [`PLAYER_SPEED_BASE`]; `energy_per_sec`.
pub const PLAYER_ENERGY_REGEN: f32 = 0.033;

/// `CHAR_CLASS_PLAYER_BASE`: the top nibble of a player agent's model id. A player agent's model id
/// is this bit-or'd with the player id (`GameSrv_CreatePlayerAgent`).
pub const CHAR_CLASS_PLAYER_BASE: u32 = 0x3000_0000;
/// `AgentType_Living`: the agent type of a player (`GmAgent.h`).
pub const AGENT_TYPE_LIVING: u8 = 1;
/// `AgentType_Item`: the agent type of an item on the ground (`GmAgent.h`). What a drop creates.
pub const AGENT_TYPE_ITEM: u8 = 4;

/// A message body being built, with the header first.
struct Builder {
    bytes: Vec<u8>,
}

impl Builder {
    fn new(header: u16) -> Self {
        let mut bytes = Vec::with_capacity(128);
        bytes.extend_from_slice(&header.to_le_bytes());
        Builder { bytes }
    }

    fn u8(mut self, value: u8) -> Self {
        self.bytes.push(value);
        self
    }

    fn u16(mut self, value: u16) -> Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn u32(mut self, value: u32) -> Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn f32(mut self, value: f32) -> Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    /// A `u16` count and the UTF-16 units of `text`, with no padding.
    ///
    /// The reference's structs declare a `uint32_t` count and a fixed array; the wire carries the
    /// count as two bytes and only the units actually used. The trace settles it: a five-character
    /// name is `2 + 2 + 10`.
    fn text(self, text: &str, capacity: usize) -> Self {
        let units: Vec<u16> = text.encode_utf16().take(capacity).collect();
        self.units(&units, capacity)
    }

    /// A `u16` count and that many UTF-16 code units.
    ///
    /// An item's name arrives as units rather than as a string: some of them are not characters at all
    /// (the backpack's name is a `Gw.dat` template id, and `0xD157` is a lone surrogate), so an item name
    /// cannot go through a `&str` without being mangled.
    fn units(mut self, units: &[u16], capacity: usize) -> Self {
        let units = &units[..units.len().min(capacity)];
        self.bytes
            .extend_from_slice(&(units.len() as u16).to_le_bytes());
        for unit in units {
            self.bytes.extend_from_slice(&unit.to_le_bytes());
        }
        self
    }

    /// A `u16` count followed by that many dwords.
    fn dwords(mut self, values: &[u32]) -> Self {
        self.bytes
            .extend_from_slice(&(values.len() as u16).to_le_bytes());
        for value in values {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }
        self
    }

    /// A **one-byte** count followed by that many dwords: `TYPE_NESTED_STRUCT`'s shape (`msgpack.c`),
    /// which is the only counted field in either family that counts one byte instead of two.
    fn nested_dwords(mut self, values: &[u32]) -> Self {
        self.bytes
            .push(values.len().min(usize::from(u8::MAX)) as u8);
        for value in values.iter().take(usize::from(u8::MAX)) {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }
        self
    }

    /// Bytes that are not one of the format's fields: the `sockaddr_in` a redirect carries, whose port
    /// is big endian and whose tail is padding, and the upper half of a four-byte game header.
    fn raw(mut self, bytes: &[u8]) -> Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    fn frame(self) -> crate::client::Frame {
        let header = u16::from_le_bytes([self.bytes[0], self.bytes[1]]);
        crate::client::Frame::new(header, self.bytes[2..].to_vec())
    }
}

/// `GAME_SMSG_INSTANCE_LOAD_HEAD`: which campaigns this account may enter.
///
/// The reference sends `0x31 | Prophecies | Factions | Nightfall` for both the PvE and the PvP mask,
/// i.e. `0x3F`. The two trailing bytes are zero.
pub fn instance_load_head(pve_unlocked: u8, pvp_unlocked: u8) -> crate::client::Frame {
    Builder::new(INSTANCE_LOAD_HEAD)
        .u8(pve_unlocked)
        .u8(pvp_unlocked)
        .u8(0)
        .u8(0)
        .frame()
}

/// `GAME_SMSG_INSTANCE_LOAD_HEAD` with the reference's own flags.
pub fn instance_load_head_default() -> crate::client::Frame {
    instance_load_head(0x3F, 0x3F)
}

/// `GAME_SMSG_INSTANCE_PLAYER_DATA_START`: a bare header; it opens the player-data stream.
pub fn instance_player_data_start() -> crate::client::Frame {
    Builder::new(INSTANCE_PLAYER_DATA_START).frame()
}

/// `GAME_SMSG_INSTANCE_PLAYER_DATA_DONE` (`0x018A`): the player-data block is over. Two bytes - the
/// reference sends `sizeof(buffer->header)` (`GameSrv.c:311`), and its only caller is the
/// character-creation flow, which is why the map-load block never carries it (`docs/10-parity-queue.md` B).
pub fn instance_player_data_done() -> crate::client::Frame {
    Builder::new(INSTANCE_PLAYER_DATA_DONE).frame()
}

/// `GAME_SMSG_CHAR_CREATION_START` (`0x0189`): the creation instance may show its screen. Two bytes,
/// header only (`GameSrv.c:305`).
pub fn char_creation_start() -> crate::client::Frame {
    Builder::new(CHAR_CREATION_START).frame()
}

/// `GAME_SMSG_CHAR_CREATION_ERROR` (`0x018B`): six bytes, header and an error code. The reference's only
/// code so far is `GM_ERROR_CHARACTER_NAME_ALREADY_EXIST` (`GameSrv.c:1939`).
pub fn char_creation_error(error_code: u32) -> crate::client::Frame {
    Builder::new(CHAR_CREATION_ERROR).u32(error_code).frame()
}

/// `GAME_SMSG_CHAR_CREATION_SUCCESS` (`0x0188`): the character was written. The reference's own shape
/// (`GameSrv.c:1951`): a 16-byte little-endian uuid, the name as a counted string of at most 20 units,
/// the constant `0x2211` its source asks "what is that?" about, and the settings blob as a counted array
/// of at most 1024 bytes.
///
/// `n_settings` counts **bytes** here, not units: the table's field is `Array8`.
pub fn char_creation_success(
    char_id: [u8; 16],
    name: &str,
    settings: &[u8],
) -> crate::client::Frame {
    let units: Vec<u16> = name.encode_utf16().take(20).collect();
    Builder::new(CHAR_CREATION_SUCCESS)
        .raw(&char_id)
        .u16(units.len() as u16)
        .raw(
            &units
                .iter()
                .flat_map(|unit| unit.to_le_bytes())
                .collect::<Vec<u8>>(),
        )
        .u16(0x2211) // the reference's own constant, unexplained there too
        .u16(settings.len().min(1024) as u16)
        .raw(&settings[..settings.len().min(1024)])
        .frame()
}

/// `GAME_SMSG_INSTANCE_LOAD_PLAYER_NAME`: the character's name, as a counted UTF-16 string.
pub fn instance_load_player_name(name: &str) -> crate::client::Frame {
    Builder::new(INSTANCE_LOAD_PLAYER_NAME)
        .text(name, 20)
        .frame()
}

/// `GAME_SMSG_INSTANCE_LOAD_INFO`: which agent the client controls, in which map.
///
/// `is_explorable` is hardcoded to 1 in the reference with the comment `// what to put here?`, and the
/// reference's behaviour is the floor this project builds on, so it is a parameter here and the
/// caller passes 1 for now.
pub fn instance_load_info(
    agent: u32,
    map_id: u16,
    is_explorable: u8,
    district: u32,
    language: u8,
    is_observer: u8,
) -> crate::client::Frame {
    Builder::new(INSTANCE_LOAD_INFO)
        .u32(agent)
        .u16(map_id)
        .u8(is_explorable)
        .u32(district)
        .u8(language)
        .u8(is_observer)
        .u8(0)
        .frame()
}

/// `GAME_SMSG_INSTANCE_LOAD_SPAWN_POINT`: where to put the character, and which map blob to load.
pub fn instance_load_spawn_point(
    map_file_id: u32,
    x: f32,
    y: f32,
    plane: u16,
) -> crate::client::Frame {
    let mut builder = Builder::new(INSTANCE_LOAD_SPAWN_POINT)
        .u32(map_file_id)
        .f32(x)
        .f32(y)
        .u16(plane)
        .u8(0) // unk0
        .u8(0); // is_cinematic
    builder.bytes.extend_from_slice(&[0u8; 8]);
    builder.frame()
}

/// `GAME_SMSG_INSTANCE_LOADED`: the player's team token, which the client keeps for the session.
pub fn instance_loaded(player_team_token: u32) -> crate::client::Frame {
    Builder::new(INSTANCE_LOADED).u32(player_team_token).frame()
}

/// `GAME_SMSG_INSTANCE_LOAD_FINISH`: the last message of the load; the client stops loading on it.
pub fn instance_load_finish() -> crate::client::Frame {
    Builder::new(INSTANCE_LOAD_FINISH).frame()
}

/// `GAME_SMSG_INSTANCE_MANIFEST_PHASE`: one stage of the download manifest.
pub fn instance_manifest_phase(phase: u8) -> crate::client::Frame {
    Builder::new(INSTANCE_MANIFEST_PHASE).u8(phase).frame()
}

/// `GAME_SMSG_INSTANCE_MANIFEST_DONE`: a manifest stage finished, naming the map it covered.
pub fn instance_manifest_done(phase: u8, map_id: u16) -> crate::client::Frame {
    Builder::new(INSTANCE_MANIFEST_DONE)
        .u8(phase)
        .u16(map_id)
        .u32(0)
        .frame()
}

/// `GAME_SMSG_ITEM_STREAM_CREATE`: opens the item stream on which the inventory follows.
pub fn item_stream_create(stream: u16, is_hero: u8) -> crate::client::Frame {
    Builder::new(ITEM_STREAM_CREATE)
        .u16(stream)
        .u8(is_hero)
        .frame()
}

/// `GAME_SMSG_ITEM_SET_ACTIVE_WEAPON_SET`: which weapon set is in hand.
pub fn item_set_active_weapon_set(stream: u16, slot: u8) -> crate::client::Frame {
    Builder::new(ITEM_SET_ACTIVE_WEAPON_SET)
        .u16(stream)
        .u8(slot)
        .frame()
}

/// `GAME_SMSG_INVENTORY_CREATE_BAG`: one bag of the inventory.
///
/// A client that gets no bag at all has no inventory model to put the weapon sets and the equipped
/// items into; the reference always creates six (`GameSrv_CreateDefaultBags`), and this builder exists
/// so that "no bags" is never sent by accident.
pub fn inventory_create_bag(
    stream: u16,
    bag_type: u8,
    bag_model_id: u8,
    bag_id: u16,
    slot_count: u8,
    assoc_item_id: u32,
) -> crate::client::Frame {
    Builder::new(INVENTORY_CREATE_BAG)
        .u16(stream)
        .u8(bag_type)
        .u8(bag_model_id)
        .u16(bag_id)
        .u8(slot_count)
        .u32(assoc_item_id)
        .frame()
}

/// One item, in the field order and the field widths `GAME_SMSG_CREATE_NAMED_ITEM` writes them.
///
/// `name` and `modifiers` are raw wire values rather than a string and a struct: an item's name is
/// UTF-16 code units that are not necessarily characters, and its modifiers are the client's own dwords.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamedItem {
    /// The item's own id in the item table.
    pub id: u32,
    /// The `Gw.dat` file id the client draws the model from.
    pub file_id: u32,
    /// The item's kind code (`GmItem.h`); the client groups bags and dyes by it.
    pub item_type: u8,
    /// Which dye slot mask applies; zero for an undyed item.
    pub dye_tint: u8,
    /// The four packed dye colours, four bits each.
    pub dye_colors: u16,
    /// Crafting materials the item carries or needs, as the reference packs them.
    pub materials: u16,
    /// The reference's own unverified byte; copied, not reasoned about.
    pub unk1: u8,
    /// Item flags: identified, customized, equipped and the rest.
    pub flags: u32,
    /// The item's value in gold, as the client shows it.
    pub value: u32,
    /// The model id the client draws, which for some item kinds is not `file_id`.
    pub model_id: u32,
    /// How many are in the stack; 1 for a non-stackable item.
    pub quantity: u32,
    /// The name, as UTF-16 code units, at most 64.
    pub name: Vec<u16>,
    /// The modifier list, at most 64 dwords.
    pub modifiers: Vec<u32>,
}

/// `GAME_SMSG_CREATE_UNNAMED_ITEM` (`0x015F`): an item **without** its name, 25 bytes.
///
/// The reference has two item creators and uses both: `GameSrv_SendItemById` sends the *named* one (the
/// inventory stream, `0x0161`), while `GameSrv_SendItemsInBag` (`GmItem.c:111-127` - which the creation
/// screen's profession change calls) and `GameSrv_BroadcastPlayerEquippedItems` (`GmItem.c:129-160`) send
/// this one. The difference is the name: an item sitting in a bag slot is drawn from its model, so the
/// client is not told a name it can already resolve.
pub fn create_unnamed_item(item: &NamedItem) -> crate::client::Frame {
    Builder::new(CREATE_UNNAMED_ITEM)
        .u32(item.id)
        .u32(item.file_id)
        .u8(item.item_type)
        .u8(item.dye_tint)
        .u16(item.dye_colors)
        .u16(item.materials)
        .u8(item.unk1)
        .u32(item.flags)
        .u32(item.value)
        .frame()
}

/// `GAME_SMSG_CREATE_NAMED_ITEM`: one item, with everything a client needs to draw it.
///
/// **It has no fixed size**, and its two counted fields count differently: the name is a `u16` count of
/// code units and the modifier list is a **one-byte** count of dwords. That second one is the trap:
/// the packed message reaches a `TYPE_NESTED_STRUCT` field and writes a `u8` there while every other
/// counted field is a `u16`, which is why the fixed struct is 425 bytes and the message that carries
/// a cupcake is 36. Measured against a real client capture.
///
/// The fixed part is 33 bytes after the header: 4 + 4 + 1 + 1 + 2 + 2 + 1 + 4 + 4 + 4 + 4 + 2, then
/// `2 * name.len()`, then 1, then `4 * modifiers.len()`.
pub fn create_named_item(item: &NamedItem) -> crate::client::Frame {
    Builder::new(CREATE_NAMED_ITEM)
        .u32(item.id)
        .u32(item.file_id)
        .u8(item.item_type)
        .u8(item.dye_tint)
        .u16(item.dye_colors)
        .u16(item.materials)
        .u8(item.unk1)
        .u32(item.flags)
        .u32(item.value)
        .u32(item.model_id)
        .u32(item.quantity)
        .units(&item.name, 64)
        .nested_dwords(&item.modifiers)
        .frame()
}

/// `GAME_SMSG_ITEM_MOVE_TO_LOCATION`: where an item sits, which is what puts it in a bag slot.
pub fn item_move_to_location(
    stream: u16,
    item_id: u32,
    bag_id: u16,
    slot: u8,
) -> crate::client::Frame {
    Builder::new(ITEM_MOVE_TO_LOCATION)
        .u16(stream)
        .u32(item_id)
        .u16(bag_id)
        .u8(slot)
        .frame()
}

/// `GAME_SMSG_ITEM_SET_PROFESSION`: the profession an item was made for. The reference only sends it
/// when the item's profession is not `Profession_None`.
pub fn item_set_profession(item_id: u32, profession: u8) -> crate::client::Frame {
    Builder::new(ITEM_SET_PROFESSION)
        .u32(item_id)
        .u8(profession)
        .frame()
}

/// `GAME_SMSG_ITEM_REMOVE`: the client drops an item out of its inventory model.
///
/// There is no "this item's quantity changed" message in either family, so changing a stack is this
/// message followed by the item re-created at its new quantity and moved back into its slot - which is
/// exactly what the reference does when it (re)sends a bag's items.
pub fn item_remove(stream: u16, item_id: u32) -> crate::client::Frame {
    Builder::new(ITEM_REMOVE).u16(stream).u32(item_id).frame()
}

/// `GAME_SMSG_ITEM_WEAPON_SET`: the items in one weapon slot. The reference sends four of these, one
/// per slot, all empty.
pub fn item_weapon_set(stream: u16, slot: u8, leadhand: u32, offhand: u32) -> crate::client::Frame {
    Builder::new(ITEM_WEAPON_SET)
        .u16(stream)
        .u8(slot)
        .u32(leadhand)
        .u32(offhand)
        .frame()
}

/// `GAME_SMSG_UPDATE_GOLD_STORAGE`: how much gold is in storage.
pub fn update_gold_storage(stream: u16, gold: u32) -> crate::client::Frame {
    Builder::new(UPDATE_GOLD_STORAGE)
        .u16(stream)
        .u32(gold)
        .frame()
}

/// A `GAME_SMSG_CHARACTER_FACTION_MAX_*` message: the cap on one faction.
pub fn faction_max(header: u16, max_faction: u32) -> crate::client::Frame {
    Builder::new(header).u32(max_faction).frame()
}

/// `GAME_SMSG_CHARACTER_UPDATE_INFO`: the character's name and six numbers the reference sets to
/// `{0, 0, 1000, 0, 0, 0}` with an **empty** name. The name is passed in so a caller may fill it, and
/// the reference's own choice (empty) is what the world server uses today.
pub fn character_update_info(name: &str) -> crate::client::Frame {
    Builder::new(CHARACTER_UPDATE_INFO)
        .text(name, 32)
        .u32(0)
        .u32(0)
        .u32(1000)
        .u32(0)
        .u32(0)
        .u32(0)
        .frame()
}

/// `GAME_SMSG_MAP_UPDATE_CURRENT`, login shape: the map the client is now in, `u16` and a zero byte.
pub fn map_update_current(map_id: u16) -> crate::client::Frame {
    Builder::new(MAP_UPDATE_CURRENT).u16(map_id).u8(0).frame()
}

/// `GAME_SMSG_READY_FOR_MAP_SPAWN`: the client may place the agents.
pub fn ready_for_map_spawn() -> crate::client::Frame {
    Builder::new(READY_FOR_MAP_SPAWN).u32(0).frame()
}

/// `GAME_SMSG_MAP_UPDATE_CURRENT` in the form that **follows a redirect**: twelve bytes, where the
/// login batch's copy is five.
///
/// Captured in the same millisecond as every `TRANSFER_GAME_SERVER_INFO`, both times a live zone
/// transition was recorded (`captures/zone-ref-005829.jsonl` seq 43 and 425):
/// `99 00 00 00 | a1 00 00 00 | 00 00 00 00` - a four-byte header, the destination map, then zero. The
/// reference documents the pair as deliberate (`GameSrv.c:1429-1432`: "in 10 of 10 captured transitions
/// a 52-byte 0x01A6 is followed immediately ... by a 12-byte 0x0099").
pub fn map_update_current_redirect(map_id: u32) -> crate::client::Frame {
    Builder::new(MAP_UPDATE_CURRENT)
        .u16(0) // the upper half of the four-byte game header
        .u32(map_id)
        .u32(0)
        .frame()
}

/// `GAME_SMSG_TRANSFER_GAME_SERVER_INFO` (`0x01A6`): where the client should reconnect, and with what.
///
/// **A faithful port of GWPS's own struct** (`GameMsg.h:659`, sent as `sizeof(*msg2)`):
///
/// ```c
/// typedef struct GameSrv_TransferGameServerInfo {
///     uint16_t header;      // two-byte header
///     uint8_t  host[24];
///     uint32_t map_token;   // token1: the world/server id
///     uint32_t region;
///     uint32_t player_token; // token2
///     uint32_t is_explorable;
///     uint32_t map_id;
/// } // 46 bytes
/// ```
///
/// **The field order is GWPS's, not the live capture's**: `player_token` comes before
/// `is_explorable`/`map_id`. Sending the captured order in a 46-byte frame made the client read the
/// map id out of the token slot (`007`); sending a 52-byte frame with a four-byte header made it read
/// `00 00` as the address family (`005`). GWPS's exact bytes are the port target.
///
/// `host` is GWPS's shape (`GameSrv.c:1406-1420`): family little endian, port **big** endian, four
/// address bytes, then zero.
pub fn transfer_game_server_info(
    host: [u8; 4],
    port: u16,
    map_token: u32,
    region: u32,
    map_id: u32,
    is_explorable: u32,
    player_token: u32,
) -> crate::client::Frame {
    Builder::new(TRANSFER_GAME_SERVER_INFO)
        .u16(2) // AF_INET, little endian
        .raw(&port.to_be_bytes())
        .raw(&host)
        .raw(&[0u8; 16]) // the rest of the sockaddr_in
        .u32(map_token)
        .u32(region)
        .u32(player_token) // GWPS: token2 before is_explorable
        .u32(is_explorable)
        .u32(map_id) // GWPS: map_id last
        .frame()
}

/// `GAME_SMSG_0x01A0`, the client's instance table: which instances exist, and one value each.
///
/// Unnamed in the reference's own table, which is why it is addressed by its header here. A live server
/// sends it in batches during a load - ten of them for the load of instance 148
/// (`recordings/client/feature/maps/pre-searing-load-3.jsonl`) - and the data is `game::instance_table`.
pub const INSTANCE_TABLE: u16 = crate::packets::game::GAME_SMSG_INSTANCE_TABLE;

/// One batch of [`INSTANCE_TABLE`]: a `u8` count and that many `{u16 instance, u32 value}` pairs.
///
/// The row's own layout is a counted nested list with a capacity of 16 per message, which is the batch
/// size a live server sends; fifteen pairs is 2 + 1 + 90 bytes on the wire.
pub fn instance_table(batch: &[(u16, u32)]) -> crate::client::Frame {
    let mut builder = Builder::new(INSTANCE_TABLE).u8(batch.len() as u8);
    for (instance, value) in batch {
        builder = builder.u16(*instance).u32(*value);
    }
    builder.frame()
}

/// `GAME_SMSG_MAP_UPDATE_UNLOCKED_LIST`: five counted lists - completed missions and bonuses on
/// normal and hard mode, and the unlocked maps.
pub fn map_update_unlocked_list(lists: [&[u32]; 5]) -> crate::client::Frame {
    let mut builder = Builder::new(MAP_UPDATE_UNLOCKED_LIST);
    for list in lists {
        builder = builder.dwords(list);
    }
    builder.frame()
}

/// `GAME_SMSG_MAP_INIT_START`: the exploration grid's dimensions, and how many compressed bytes the
/// [`cartography_data`] messages that follow will carry.
///
/// **The client has no other source for these dimensions.** It stores them, replaces the whole
/// per-continent grid with what arrives, and asserts `dims.x % 32 == 0` when it decompresses. A
/// server that never sends this leaves the dimensions at zero, and the map surfaces then fail their
/// own consistency check the first time a player opens a map: client build 39112,
/// `Assertion: worldMapDims.x == mapDims.x * DXT_BLOCK_SIZE`, `GmMapView.cpp(1731)`, measured
/// 2026-10-29 against this server.
///
/// The reference's shape is the one this client frames to: a two-byte header, then three dwords, 14
/// bytes. A live capture reports 16 (a four-byte header), and sending 16 desynchronises this client -
/// `007` at map load, measured 2026-10-30 - so the capture and the framing disagree and the client
/// settles it.
pub fn map_init_start(dims_x: u32, dims_y: u32, byte_count: u32) -> crate::client::Frame {
    Builder::new(MAP_INIT_START)
        .u32(dims_x)
        .u32(dims_y)
        .u32(byte_count)
        .frame()
}

/// `GAME_SMSG_CARTOGRAPHY_DATA`: a slice of the compressed grid - a `u32` count at byte 2, the stream
/// at byte 6, 63 dwords and two pad bytes, 260 bytes.
///
/// The count is four bytes wide, so the stream begins at byte 6. A `u16` count loads but shifts the
/// stream two bytes: band 1 lands in band 0. A 262-byte frame is read as a header in the middle of
/// the message. Both were measured against build 39112 on 2026-11-04.
pub fn cartography_data(blob: &[u8]) -> crate::client::Frame {
    let bytes = &blob[..blob.len().min(CARTOGRAPHY_DATA_BYTES)];
    let dwords = bytes.len().div_ceil(4);
    let mut builder = Builder::new(CARTOGRAPHY_DATA).u32(dwords as u32);
    for slot in 0..CARTOGRAPHY_DATA_SLOTS {
        let at = slot * 4;
        let mut word = [0u8; 4];
        for (index, byte) in word.iter_mut().enumerate() {
            if let Some(source) = bytes.get(at + index) {
                *byte = *source;
            }
        }
        builder = builder.u32(u32::from_le_bytes(word));
    }
    builder.u16(0).frame()
}

/// `GAME_SMSG_MAP_EXPLORATION_MARK`: reveal a `(2 * half_span + 1)` square of continent cells.
pub fn map_exploration_mark(x: u32, y: u32, half_span: u8) -> crate::client::Frame {
    Builder::new(MAP_EXPLORATION_MARK)
        .u32(x)
        .u32(y)
        .u8(half_span)
        .frame()
}

/// `GAME_SMSG_HARD_MODE_UNLOCKED`.
pub fn hard_mode_unlocked(unlocked: u8) -> crate::client::Frame {
    Builder::new(HARD_MODE_UNLOCKED).u8(unlocked).frame()
}

/// The character and account progress `GAME_SMSG_CHARACTER_UPDATE_FACTIONS` reports, one field per
/// reference argument (`GameSrv_UpdatePlayerFactions`).
///
/// A struct rather than thirteen arguments: the dwords are positional on the wire, so a caller that
/// passes them in order reads as a wall of zeros and the compiler cannot catch a swap. Two dwords of
/// the body (`unk_faction4`, `unk_faction5`) are not modelled here because nothing measured sets them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CharacterProgress {
    /// Total experience; the client shows it on the character sheet.
    pub experience: u32,
    /// Kurzick faction held now.
    pub current_kurzick: u32,
    /// Kurzick faction earned in total.
    pub total_earned_kurzick: u32,
    /// Luxon faction held now.
    pub current_luxon: u32,
    /// Luxon faction earned in total.
    pub total_earned_luxon: u32,
    /// Imperial faction held now.
    pub current_imperial: u32,
    /// Imperial faction earned in total.
    pub total_earned_imperial: u32,
    /// The character's level, from `characters.level`.
    pub level: u32,
    /// Morale percentage: 100 is the unmodified value a live load carries.
    pub morale: u32,
    /// Balthazar faction held now.
    pub current_balthazar: u32,
    /// Balthazar faction earned in total.
    pub total_earned_balthazar: u32,
    /// Skill points held now.
    pub current_skill_points: u32,
    /// Skill points earned in total.
    pub total_earned_skill_points: u32,
}

/// `GAME_SMSG_CHARACTER_UPDATE_FACTIONS`: fifteen dwords of character and account progress, in the
/// reference's order (`GameSrv_UpdatePlayerFactions`).
pub fn character_update_factions(progress: &CharacterProgress) -> crate::client::Frame {
    Builder::new(CHARACTER_UPDATE_FACTIONS)
        .u32(progress.experience)
        .u32(progress.current_kurzick)
        .u32(progress.total_earned_kurzick)
        .u32(progress.current_luxon)
        .u32(progress.total_earned_luxon)
        .u32(progress.current_imperial)
        .u32(progress.total_earned_imperial)
        .u32(0) // unk_faction4
        .u32(0) // unk_faction5
        .u32(progress.level)
        .u32(progress.morale)
        .u32(progress.current_balthazar)
        .u32(progress.total_earned_balthazar)
        .u32(progress.current_skill_points)
        .u32(progress.total_earned_skill_points)
        .frame()
}

/// `GAME_SMSG_CHAR_VISIBILITY_FLAGS`: two dwords, a value and the mask it applies under. A live load
/// sends `FFFFFFFF, FFFFFFFF` once, immediately after `GAME_SMSG_CHARACTER_UPDATE_FACTIONS` (header
/// `00EF` in `recordings/asura/world/load-layers-20261031.jsonl`, captured in `ef000000 | ffffffff |
/// ffffffff`). Uninitialized, the inventory panel's per-slot display mode stays blank for every slot.
pub fn char_visibility_flags(value: u32, mask: u32) -> crate::client::Frame {
    Builder::new(CHAR_VISIBILITY_FLAGS)
        .u32(value)
        .u32(mask)
        .frame()
}

/// `GAME_SMSG_PLAYER_UPDATE_FLAGS`: one player's flags word as a masked read-modify-write - a u16
/// player id, the value, and the mask. A live load sends it once per player with mask 7, right after
/// the visibility flags: `3c000000 | 01000000 | 00000000 | 07000000` (header `003C` in the same
/// capture). The player record's flags are what the world view and the attribute panel read.
pub fn player_update_flags(player_id: u16, value: u32, mask: u32) -> crate::client::Frame {
    Builder::new(PLAYER_UPDATE_FLAGS)
        .u16(player_id)
        .u32(value)
        .u32(mask)
        .frame()
}

/// `GAME_SMSG_PVP_UPDATE_UNLOCKED_SKILLS`: the skills unlocked for PvP.
pub fn pvp_update_unlocked_skills(skills: &[u32]) -> crate::client::Frame {
    Builder::new(PVP_UPDATE_UNLOCKED_SKILLS)
        .dwords(skills)
        .frame()
}

/// `GAME_SMSG_PVP_UPDATE_UNLOCKED_HEROES`: a bitmap of unlocked PvP heroes. The reference sends eight
/// words of `0xFFFFFFFF`.
pub fn pvp_update_unlocked_heroes(bitmap: &[u32]) -> crate::client::Frame {
    Builder::new(PVP_UPDATE_UNLOCKED_HEROES)
        .dwords(bitmap)
        .frame()
}

/// `GAME_SMSG_PVP_ITEM_STREAM_ADD`: one unlockable PvP item and its parameters.
pub fn pvp_item_stream_add(item_id: u16, params: &[u32]) -> crate::client::Frame {
    // The parameter count is a **u8**, not the u16 every other counted field uses: the reference's
    // layout declares this one as a nested struct, and its own log shows an entry with three
    // parameters at 17 bytes (`2 + 2 + 1 + 3 * 4`).
    let mut builder = Builder::new(PVP_ITEM_STREAM_ADD)
        .u16(item_id)
        .u8(params.len() as u8);
    for param in params {
        builder = builder.u32(*param);
    }
    builder.frame()
}

/// `GAME_SMSG_PVP_ITEM_STREAM_END`.
pub fn pvp_item_stream_end() -> crate::client::Frame {
    Builder::new(PVP_ITEM_STREAM_END).frame()
}

/// `GAME_SMSG_ACCOUNT_FEATURE`: one account entitlement, `feature_id` and two parameters.
pub fn account_feature(feature_id: u16, param1: u16, param2: u16) -> crate::client::Frame {
    Builder::new(ACCOUNT_FEATURE)
        .u16(feature_id)
        .u16(param1)
        .u16(param2)
        .frame()
}

/// `GAME_SMSG_AGENT_UPDATE_ATTRIBUTE_POINTS`: unspent and spent attribute points.
pub fn agent_update_attribute_points(
    agent: u32,
    unused_points: u8,
    used_points: u8,
) -> crate::client::Frame {
    Builder::new(AGENT_UPDATE_ATTRIBUTE_POINTS)
        .u32(agent)
        .u8(unused_points)
        .u8(used_points)
        .frame()
}

/// `GAME_SMSG_PLAYER_UPDATE_PROFESSION`: the agent's primary and secondary profession.
pub fn player_update_profession(
    agent: u32,
    primary: u8,
    secondary: u8,
    is_pvp: u8,
) -> crate::client::Frame {
    Builder::new(PLAYER_UPDATE_PROFESSION)
        .u32(agent)
        .u8(primary)
        .u8(secondary)
        .u8(is_pvp)
        .frame()
}

/// `GAME_SMSG_PLAYER_UPDATE_UNLOCKED_PROFESSIONS`: a bitmap of professions the character may change to.
pub fn player_update_unlocked_professions(agent: u32, unlocked: u32) -> crate::client::Frame {
    Builder::new(PLAYER_UPDATE_UNLOCKED_PROFESSIONS)
        .u32(agent)
        .u32(unlocked)
        .frame()
}

/// `GAME_SMSG_SKILLBAR_UPDATE`: the eight skills on the bar and their PvP masks.
pub fn skillbar_update(
    agent: u32,
    skills: &[u32],
    pvp_masks: &[u32],
    unk: u8,
) -> crate::client::Frame {
    let mut builder = Builder::new(SKILLBAR_UPDATE).u32(agent).dwords(skills);
    builder = builder.dwords(pvp_masks).u8(unk);
    builder.frame()
}

/// `GAME_SMSG_UPDATE_UNLOCKED_SKILLS`: the skills unlocked for PvE.
pub fn update_unlocked_skills(skills: &[u32]) -> crate::client::Frame {
    Builder::new(UPDATE_UNLOCKED_SKILLS).dwords(skills).frame()
}

/// `GAME_SMSG_AGENT_UPDATE_ATTRIBUTES`: the agent's attribute ranks, one dword each.
pub fn agent_update_attributes(agent: u32, attributes: &[u32]) -> crate::client::Frame {
    Builder::new(AGENT_UPDATE_ATTRIBUTES)
        .u32(agent)
        .dwords(attributes)
        .frame()
}

/// `GAME_SMSG_WORLD_UPDATE_LOAD_TIME`: how long this instance took to load, in milliseconds.
pub fn world_update_load_time(load_time: u32) -> crate::client::Frame {
    Builder::new(WORLD_UPDATE_LOAD_TIME).u32(load_time).frame()
}

/// `GAME_SMSG_PLAYER_CREATE`: a player exists, with its id, agent, appearance and name.
pub fn player_create(
    player_id: u32,
    agent_id: u32,
    appearance: u32,
    name: &str,
) -> crate::client::Frame {
    Builder::new(PLAYER_CREATE)
        .u32(player_id)
        .u32(agent_id)
        .u32(appearance)
        .u8(0) // unk0
        .u32(0) // unk1
        .u32(0) // unk2
        .text(name, 32)
        .frame()
}

/// One agent, in the field order `GAME_SMSG_WORLD_CREATE_AGENT` writes it.
///
/// `agent_type` is the only field that differs between a player (`AgentType_Living`) and an item on the
/// ground (`AgentType_Item`); everything else is what the reference fills from its `GmAgent`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CreateAgent {
    /// The id the rest of the stream refers to this body by.
    pub agent_id: u32,
    /// What the client draws: a character class word for a player, an item model for a drop.
    pub model_id: u32,
    /// `AgentType_Living` (1) for a player, `AgentType_Item` (4) for a dropped item.
    pub agent_type: u8,
    /// Where the body stands, in the map's own units.
    pub position: (f32, f32),
    /// The pathing plane, not a height.
    pub plane: u16,
    /// Which way it faces, as a unit vector.
    pub direction: (f32, f32),
    /// Walking speed the client uses until a `0x002B` says otherwise.
    pub speed_base: f32,
    /// The team the body belongs to; 0 for a neutral body.
    pub player_team_token: u32,
}

/// The 99-byte `GAME_SMSG_WORLD_CREATE_AGENT`, field for field as the reference builds it
/// (`GameSrv_BuildCreateAgentMsg`).
///
/// `h001E = 1`, `h0023 = 1.0` and `h0027 = 0x41400000` are the reference's own unverified
/// constants; the two `HUGE_VALF` pairs are what it sends for the unmodelled vectors. They are copied
/// rather than reasoned about, because a client that reads them differently will still read them the
/// same way as it does from GWPS.
pub fn world_create_agent(agent: &CreateAgent) -> crate::client::Frame {
    let mut builder = Builder::new(WORLD_CREATE_AGENT)
        .u32(agent.agent_id)
        .u32(agent.model_id)
        .u8(agent.agent_type)
        .u8(5) // h000B
        .f32(agent.position.0)
        .f32(agent.position.1)
        .u16(agent.plane)
        .f32(agent.direction.0)
        .f32(agent.direction.1)
        .u8(1) // h001E
        .f32(agent.speed_base)
        .f32(1.0) // h0023
        .u32(0x4140_0000) // h0027: 12.0 reinterpreted, exactly as the reference writes it
        .u32(agent.player_team_token)
        .u32(0) // h002F
        .u32(0) // h0033
        .u32(0) // h0037
        .u32(0) // h003B
        .u32(0) // h003F
        .f32(0.0) // h0043.x
        .f32(0.0) // h0043.y
        .f32(f32::INFINITY) // h004B.x
        .f32(f32::INFINITY) // h004B.y
        .u16(0) // h0053
        .u32(0) // h0055
        .f32(f32::INFINITY) // h0059.x
        .f32(f32::INFINITY) // h0059.y
        .u16(0); // h0061
    builder.bytes.reserve(0);
    builder.frame()
}

/// `GAME_SMSG_AGENT_UPDATE_MORALE`: one agent's morale as a percentage, 100 being the unmodified value.
/// A live server sends one per body at load - `9c000000 | 1f000000 | 64000000` for agent 0x1F
/// (`recordings/client/feature/travel/doors-20261031-095754.jsonl`, header `009C`) - and the party
/// panel draws its morale icon from this record rather than from the character's own morale field.
pub fn agent_update_morale(agent_id: u32, morale: u32) -> crate::client::Frame {
    Builder::new(AGENT_UPDATE_MORALE)
        .u32(agent_id)
        .u32(morale)
        .frame()
}

/// `GAME_SMSG_AGENT_PROPERTY_UPDATE_INT`: one integer property of an agent.
pub fn agent_property_update_int(prop_id: u32, agent_id: u32, value: u32) -> crate::client::Frame {
    Builder::new(AGENT_PROPERTY_UPDATE_INT)
        .u32(prop_id)
        .u32(agent_id)
        .u32(value)
        .frame()
}

/// `GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT`: one float property of an agent.
pub fn agent_property_update_float(
    prop_id: u32,
    agent_id: u32,
    value: f32,
) -> crate::client::Frame {
    Builder::new(AGENT_PROPERTY_UPDATE_FLOAT)
        .u32(prop_id)
        .u32(agent_id)
        .f32(value)
        .frame()
}

/// `GAME_SMSG_AGENT_INITIAL_EFFECTS`: the effect bitmask an agent starts with.
pub fn agent_initial_effects(agent_id: u32, effects: u32) -> crate::client::Frame {
    Builder::new(AGENT_INITIAL_EFFECTS)
        .u32(agent_id)
        .u32(effects)
        .frame()
}

/// `GAME_SMSG_AGENT_UPDATE_EFFECTS` (`0x00F1`): the effect word an agent now carries.
///
/// A live server sends it for the travelling player immediately before a redirect
/// (`captures/zone-ref2-012225.jsonl` seq 971: `f1 00 00 00 | 43 00 00 00 | 00 01 00 00`, the player's own
/// agent and the word `0x100`), and this world sends the same pair of messages in the same order.
pub fn agent_update_effects(agent_id: u32, effects: u32) -> crate::client::Frame {
    Builder::new(AGENT_UPDATE_EFFECTS)
        .u32(agent_id)
        .u32(effects)
        .frame()
}

/// `GAME_SMSG_UPDATE_AGENT_VISUAL_EQUIPMENT`: the nine item ids an agent wears.
pub fn update_agent_visual_equipment(agent_id: u32, items: [u32; 9]) -> crate::client::Frame {
    let mut builder = Builder::new(UPDATE_AGENT_VISUAL_EQUIPMENT).u32(agent_id);
    for item in items {
        builder = builder.u32(item);
    }
    builder.frame()
}

/// `GAME_SMSG_AGENT_DISPLAY_CAPE`.
pub fn agent_display_cape(agent_id: u32, status: u8) -> crate::client::Frame {
    Builder::new(AGENT_DISPLAY_CAPE)
        .u32(agent_id)
        .u8(status)
        .frame()
}

/// `GAME_SMSG_WORLD_UPDATE_CONTROLLED_AGENT`: which agent this client drives. The reference sends 3 as
/// the second field and nothing reads it back.
pub fn world_update_controlled_agent(agent_id: u32, unk: u32) -> crate::client::Frame {
    Builder::new(WORLD_UPDATE_CONTROLLED_AGENT)
        .u32(agent_id)
        .u32(unk)
        .frame()
}

/// `GAME_SMSG_PARTY_CREATE`.
pub fn party_create(party_id: u16) -> crate::client::Frame {
    Builder::new(PARTY_CREATE).u16(party_id).frame()
}

/// `GAME_SMSG_PARTY_PLAYER_ADD`: one member of the party.
pub fn party_player_add(party_id: u16, player_id: u16, is_loaded: u8) -> crate::client::Frame {
    Builder::new(PARTY_PLAYER_ADD)
        .u16(party_id)
        .u16(player_id)
        .u8(is_loaded)
        .frame()
}

/// `GAME_SMSG_PARTY_MEMBER_STREAM_END`.
pub fn party_member_stream_end(party_id: u16) -> crate::client::Frame {
    Builder::new(PARTY_MEMBER_STREAM_END).u16(party_id).frame()
}

/// `GAME_SMSG_CHARACTER_UDPATE_PARTY`: which party the character is in.
pub fn character_update_party(party_id: u16, unknown: u8) -> crate::client::Frame {
    Builder::new(CHARACTER_UPDATE_PARTY)
        .u16(party_id)
        .u8(unknown)
        .frame()
}

/// `GAME_SMSG_PLAYER_UPDATE_PARTY_SIZE`: how many members a player's party has.
///
/// **Twelve bytes, and the generated table disagrees twice.** The client's table says five
/// (`MsgHeader`, `Word`, `Byte`, from its `stoc.h` struct) with a two-byte header; a live capture
/// carries twelve every time a map loads - `b0 00 00 00 | 01 00 00 00 | 01 00 00 00`, 48 of 48
/// occurrences. The fourth byte is the upper half of a **four-byte game header**, which the reference
/// calls `TYPE_MSG_HEADER_GAME` (`msgdefs.h:19-23`) and applies per message on evidence. Its own
/// precedence rule is "a capture beats every table" (`PreSearing/docs/07-packet-scope.md`), and the
/// same rule settled `TRANSFER_GAME_SERVER_INFO`.
///
/// So the two leading zero bytes are written as body, not header: `Frame` always encodes a two-byte
/// header, and the bytes on the wire are then identical to the capture.
pub fn player_update_party_size(player_id: u32, party_size: u32) -> crate::client::Frame {
    Builder::new(PLAYER_UPDATE_PARTY_SIZE)
        .u16(0) // the upper half of the four-byte game header
        .u32(player_id)
        .u32(party_size)
        .frame()
}

/// `GAME_SMSG_PLAYER_UPDATE_PARTY`: which player leads the party this player is in.
///
/// Twelve bytes for the same reason as the size above, and captured in the same breath
/// (`b1 00 00 00 | 07 00 00 00 | 07 00 00 00`: player 7 leading player 7's own party). The reference's
/// builder (`GmPlayer.c:344-354`) has no caller - GWPS sends only the size message, and only when
/// another player is visible - so these values come from the capture, not from GWPS.
pub fn player_update_party(player_id: u32, party_leader_player_id: u32) -> crate::client::Frame {
    Builder::new(PLAYER_UPDATE_PARTY)
        .u16(0)
        .u32(player_id)
        .u32(party_leader_player_id)
        .frame()
}

/// `GAME_SMSG_PLAYER_DESTROY`: a player has left the map (`GameSrv_BroadcastPlayerDestroy`,
/// `GmPlayer.c:314`). Eight bytes: a four-byte header, then the player id - the client's own sniffer
/// calls `0x5A` `AGENT_DESTROY_PLAYER`, and the capture carries `5a 00 00 00 | 0b 00 00 00`.
pub fn player_destroy(player_id: u32) -> crate::client::Frame {
    Builder::new(PLAYER_DESTROY).u16(0).u32(player_id).frame()
}

/// `GAME_SMSG_WORLD_REMOVE_AGENT`: an agent has left the map (`GameSrv_BroadcastAgentRemove`,
/// `GmAgent.c:60`). Eight bytes, captured as `21 00 00 00 | <agent id>` eight times over.
pub fn world_remove_agent(agent_id: u32) -> crate::client::Frame {
    Builder::new(WORLD_REMOVE_AGENT)
        .u16(0)
        .u32(agent_id)
        .frame()
}

/// `GAME_SMSG_WORLD_SIMULATION_TICK`: the world's clock. The client expects one at least every 500 ms.
pub fn world_simulation_tick(delta_ms: u32) -> crate::client::Frame {
    Builder::new(WORLD_SIMULATION_TICK).u32(delta_ms).frame()
}

/// `GAME_SMSG_AGENT_MOVE_TO_POINT` in the two-byte-header form: an agent is walking to `destination` on
/// `plane`, from `current_plane` (`GameSrv_MoveAgentToPoint`), 18 bytes.
///
/// The sniffer reads the unpacked client struct; the wire uses this packed layout
/// (`docs/03-protocol.md` 5e).
pub fn agent_move_to_point(
    agent_id: u32,
    destination: (f32, f32),
    plane: u16,
    current_plane: u16,
) -> crate::client::Frame {
    Builder::new(AGENT_MOVE_TO_POINT)
        .u32(agent_id)
        .f32(destination.0)
        .f32(destination.1)
        .u16(plane)
        .u16(current_plane)
        .frame()
}

/// Packed direction reply: agent, direction, facing (`packets.rs`, 0x0025).
pub fn agent_update_direction(
    agent_id: u32,
    direction: (f32, f32),
    facing: u8,
) -> crate::client::Frame {
    Builder::new(AGENT_UPDATE_DIRECTION)
        .u32(agent_id)
        .f32(direction.0)
        .f32(direction.1)
        .u8(facing)
        .frame()
}

/// Packed move-type reply: agent, speed factor, facing (`packets.rs`, 0x002B).
pub fn agent_update_speed(agent_id: u32, speed: f32, facing: u8) -> crate::client::Frame {
    Builder::new(AGENT_UPDATE_SPEED)
        .u32(agent_id)
        .f32(speed)
        .u8(facing)
        .frame()
}

/// `GAME_SMSG_AGENT_MOVE_TO_POINT`, the live form: 24 bytes,
/// `29 00 00 00 | agent | x | y | trap | plane`.
///
/// Measured from the live capture (`captures/live-move.jsonl`). The upper two bytes of the four-byte
/// game header are zero, as they are in every live game message. After those four bytes: the agent, the
/// position, the **trap id** of the destination waypoint (zero when the point is not on a trap), and the
/// plane as a dword. The reference's two-byte-header form packs the plane and current plane into those
/// last four bytes instead; live widened both to dwords and carries the trap the reference drops
/// (`GmPaths.h`'s `Waypoint { GmPos pos; uint32_t trap_id; }`).
pub fn agent_move_to_point_live(
    agent_id: u32,
    destination: (f32, f32),
    trap_id: u32,
    plane: u32,
) -> crate::client::Frame {
    Builder::new(AGENT_MOVE_TO_POINT)
        .u16(0) // the upper half of the four-byte game header
        .u32(agent_id)
        .f32(destination.0)
        .f32(destination.1)
        .u32(trap_id)
        .u32(plane)
        .frame()
}

/// `GAME_SMSG_AGENT_UPDATE_DIRECTION`, the live form: 20 bytes,
/// `25 00 00 00 | agent | dx | dy | 1`. The direction is a unit vector; the trailing dword was 1 in
/// every captured instance.
pub fn agent_update_direction_live(agent_id: u32, direction: (f32, f32)) -> crate::client::Frame {
    Builder::new(AGENT_UPDATE_DIRECTION)
        .u16(0)
        .u32(agent_id)
        .f32(direction.0)
        .f32(direction.1)
        .u32(1)
        .frame()
}

/// `GAME_SMSG_AGENT_UPDATE_SPEED`, the live form: 16 bytes, `2b 00 00 00 | agent | 1.0 | mode`.
///
/// The capture shows 1.0 in every instance, mode 1 when a walk starts and mode 9 when it is cancelled.
pub fn agent_update_speed_live(agent_id: u32, speed: f32, mode: u32) -> crate::client::Frame {
    Builder::new(AGENT_UPDATE_SPEED)
        .u16(0)
        .u32(agent_id)
        .f32(speed)
        .u32(mode)
        .frame()
}

/// `GAME_SMSG_WORLD_SIMULATION_TICK`, the live form: 8 bytes, `1e 00 00 00 | elapsed ms`.
///
/// The live server sends one every ~100 ms with the real elapsed time since the previous tick (median
/// 81 ms, minimum 1, maximum 519 over the measured capture).
pub fn world_simulation_tick_live(elapsed_ms: u32) -> crate::client::Frame {
    Builder::new(WORLD_SIMULATION_TICK)
        .u16(0)
        .u32(elapsed_ms)
        .frame()
}

/// `GAME_SMSG_AGENT_STOP_MOVING`: the agent is no longer walking (`GameSrv_AgentStopMoving`).
pub fn agent_stop_moving(agent_id: u32) -> crate::client::Frame {
    Builder::new(AGENT_STOP_MOVING).u32(agent_id).frame()
}

/// `GAME_SMSG_AGENT_UPDATE_POSITION`: where an agent is now. Sent to everyone *else* - the client that
/// reported the position already knows it (`GameSrv_BroadcastAgentPosition`).
pub fn agent_update_position(
    agent_id: u32,
    position: (f32, f32),
    plane: u16,
) -> crate::client::Frame {
    Builder::new(AGENT_UPDATE_POSITION)
        .u32(agent_id)
        .f32(position.0)
        .f32(position.1)
        .u16(plane)
        .frame()
}

/// `GAME_SMSG_PING_REQUEST`: asks the client to answer with a ping reply.
pub fn ping_request() -> crate::client::Frame {
    Builder::new(PING_REQUEST).frame()
}

/// `GAME_SMSG_PING_REPLY`: the answer to a client's ping request.
pub fn ping_reply(ping: u32) -> crate::client::Frame {
    Builder::new(PING_REPLY).u32(ping).frame()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire size of a built frame, header included.
    fn wire(frame: &crate::client::Frame) -> usize {
        frame.encode().len()
    }

    #[test]
    fn the_fixed_size_messages_match_the_size_the_reference_sends() {
        // Every number here is a size measured from the reference's own send log
        // (docs/03-protocol.md section 11), not from a struct: `sizeof` is a maximum, the wire is not.
        assert_eq!(wire(&instance_load_head_default()), 6);
        assert_eq!(wire(&instance_player_data_start()), 2);
        // The character-creation block: header-only messages, a six-byte error, and the success message
        // whose two counted fields make it variable (16 + 2 + 2n + 2 + 2 + m).
        assert_eq!(wire(&instance_player_data_done()), 2);
        assert_eq!(wire(&char_creation_start()), 2);
        assert_eq!(wire(&char_creation_error(0x0106)), 6);
        assert_eq!(wire(&char_creation_success([0; 16], "Asura", &[])), 34);
        assert_eq!(
            wire(&char_creation_success([0; 16], "Asura", &[1, 2, 3])),
            37
        );
        assert_eq!(wire(&instance_load_info(1, 146, 1, 1, 0, 0)), 16);
        assert_eq!(wire(&instance_load_spawn_point(1, 0.0, 0.0, 0)), 26);
        assert_eq!(wire(&instance_loaded(0xBAAD_F00D)), 6);
        assert_eq!(wire(&instance_load_finish()), 2);
        assert_eq!(wire(&instance_manifest_phase(1)), 3);
        assert_eq!(wire(&instance_manifest_done(2, 0xFFFF)), 9);
        assert_eq!(wire(&item_stream_create(1, 0)), 5);
        assert_eq!(wire(&item_set_active_weapon_set(1, 0)), 5);
        assert_eq!(wire(&item_weapon_set(1, 0, 0, 0)), 13);
        assert_eq!(wire(&inventory_create_bag(1, 1, 0, 1, 20, 0)), 13);
        assert_eq!(wire(&item_move_to_location(1, 2, 1, 0)), 11);
        assert_eq!(wire(&item_remove(1, 2)), 8);
        // The unnamed form is fixed at 25 bytes; the named one has no fixed size at all.
        assert_eq!(wire(&create_unnamed_item(&NamedItem::default())), 25);
        assert_eq!(wire(&item_set_profession(3, 1)), 7);
        assert_eq!(wire(&chat_message_local(1, CHANNEL_ALL)), 5);
        assert_eq!(wire(&chat_message_server(1, CHANNEL_WARNING)), 5);
        assert_eq!(wire(&update_gold_storage(1, 0)), 8);
        assert_eq!(wire(&faction_max(FACTION_MAX_KURZICK, 0)), 6);
        assert_eq!(wire(&hard_mode_unlocked(1)), 3);
        assert_eq!(wire(&map_update_current(146)), 5);
        assert_eq!(wire(&ready_for_map_spawn()), 6);
        // The redirect pair, both measured from GWPS's own build: 46 and 12 bytes.
        assert_eq!(
            wire(&transfer_game_server_info(
                [127, 0, 0, 1],
                6112,
                0x1111_1111,
                0,
                0x0000_0094,
                0,
                0x2222_2222
            )),
            46
        );
        // And byte for byte, GWPS's struct field order (`GameMsg.h:659`): header, host[24],
        // map_token, region, player_token, is_explorable, map_id.
        let mut expected = vec![0xA6, 0x01, 0x02, 0x00, 0x17, 0xE0, 0x7F, 0x00, 0x00, 0x01];
        expected.extend_from_slice(&[0u8; 16]);
        expected.extend_from_slice(&0x1111_1111u32.to_le_bytes()); // map_token
        expected.extend_from_slice(&0u32.to_le_bytes()); // region
        expected.extend_from_slice(&0x2222_2222u32.to_le_bytes()); // player_token
        expected.extend_from_slice(&0u32.to_le_bytes()); // is_explorable
        expected.extend_from_slice(&0x0000_0094u32.to_le_bytes()); // map_id
        assert_eq!(
            transfer_game_server_info(
                [127, 0, 0, 1],
                6112,
                0x1111_1111,
                0,
                0x0000_0094,
                0,
                0x2222_2222
            )
            .encode(),
            expected
        );
        assert_eq!(wire(&map_update_current_redirect(146)), 12);
        assert_eq!(wire(&pvp_item_stream_end()), 2);
        // The reference's own sizes: 17 bytes for three parameters, 25 for five.
        assert_eq!(
            wire(&pvp_item_stream_add(
                1,
                &[0x2408_0081, 0x2530_0102, 0x24B8_0300]
            )),
            17
        );
        assert_eq!(wire(&pvp_item_stream_add(48, &[0; 5])), 25);
        assert_eq!(wire(&account_feature(1, 10, 0)), 8);
        assert_eq!(wire(&agent_update_attribute_points(1, 50, 50)), 8);
        assert_eq!(wire(&player_update_profession(1, 0, 0, 0)), 9);
        assert_eq!(wire(&player_update_unlocked_professions(1, 0)), 10);
        assert_eq!(wire(&world_update_load_time(0)), 6);
        // Four-byte game headers: the table says 6 for the first two and 5/6 for the party pair, and
        // the capture says 8 and 12. The capture wins (see the builders).
        assert_eq!(wire(&player_destroy(7)), 8);
        assert_eq!(wire(&world_remove_agent(7)), 8);
        assert_eq!(wire(&player_update_party_size(1, 1)), 12);
        assert_eq!(wire(&player_update_party(1, 1)), 12);
        assert_eq!(
            wire(&world_create_agent(&CreateAgent {
                agent_id: 1,
                model_id: 0x3000_0001,
                agent_type: AGENT_TYPE_LIVING,
                position: (0.0, 0.0),
                plane: 0,
                direction: (1.0, 0.0),
                speed_base: 288.0,
                player_team_token: 1,
            })),
            99
        );
        assert_eq!(
            wire(&agent_property_update_int(AGENT_PROPERTY_ENERGY, 1, 25)),
            14
        );
        assert_eq!(
            wire(&agent_property_update_float(
                AGENT_PROPERTY_ENERGY_REGEN,
                1,
                0.033
            )),
            14
        );
        assert_eq!(wire(&agent_initial_effects(1, 0)), 10);
        assert_eq!(wire(&update_agent_visual_equipment(1, [0; 9])), 42);
        assert_eq!(wire(&agent_display_cape(1, 1)), 7);
        assert_eq!(wire(&world_update_controlled_agent(1, 3)), 10);
        assert_eq!(wire(&party_create(1)), 4);
        assert_eq!(wire(&party_player_add(1, 1, 1)), 7);
        assert_eq!(wire(&party_member_stream_end(1)), 4);
        assert_eq!(wire(&character_update_party(1, 1)), 5);
        assert_eq!(wire(&world_simulation_tick(16)), 6);
        assert_eq!(wire(&ping_request()), 2);
        assert_eq!(wire(&ping_reply(20)), 6);
    }

    #[test]
    fn the_live_movement_messages_are_the_bytes_the_live_server_sends() {
        // Verbatim from `captures/live-move.jsonl`, a live client walking under the live servers. The
        // header is four bytes wide, so the upper two bytes of the message index are zero.
        assert_eq!(
            agent_move_to_point_live(
                0x45,
                (f32::from_bits(0x45FB_FD23), f32::from_bits(0x45AB_0356)),
                0,
                0
            )
            .encode(),
            vec![
                0x29, 0, 0, 0, 0x45, 0, 0, 0, 0x23, 0xfd, 0xfb, 0x45, 0x56, 0x03, 0xab, 0x45, 0, 0,
                0, 0, 0, 0, 0, 0
            ]
        );
        // A waypoint on a trap: the trap id is the fourth dword and the plane the fifth. Verbatim from
        // the capture at t=22.414 (10860, 4532, trap 18, plane 0) and t=22.769 (10860, 5579, trap 0,
        // plane 18) -- the second is where the trap in the first one leads.
        assert_eq!(
            agent_move_to_point_live(0x45, (10860.0, 4532.0), 18, 0).encode(),
            vec![
                0x29, 0, 0, 0, 0x45, 0, 0, 0, 0x00, 0xb0, 0x29, 0x46, 0x00, 0xa0, 0x8d, 0x45, 0x12,
                0, 0, 0, 0, 0, 0, 0
            ]
        );
        assert_eq!(
            agent_move_to_point_live(0x45, (10860.0, 5579.0), 0, 18).encode(),
            vec![
                0x29, 0, 0, 0, 0x45, 0, 0, 0, 0x00, 0xb0, 0x29, 0x46, 0x00, 0x58, 0xae, 0x45, 0, 0,
                0, 0, 0x12, 0, 0, 0
            ]
        );
        assert_eq!(
            agent_update_direction_live(
                0x45,
                (f32::from_bits(0xBE23_B096), f32::from_bits(0xBF7C_6C17))
            )
            .encode(),
            vec![
                0x25, 0, 0, 0, 0x45, 0, 0, 0, 0x96, 0xb0, 0x23, 0xbe, 0x17, 0x6c, 0x7c, 0xbf, 1, 0,
                0, 0
            ]
        );
        // Mode 9 stops a walk, mode 1 starts one; both carry 1.0 as the speed.
        assert_eq!(
            agent_update_speed_live(0x45, 1.0, 9).encode(),
            vec![0x2b, 0, 0, 0, 0x45, 0, 0, 0, 0, 0, 0x80, 0x3f, 9, 0, 0, 0]
        );
        assert_eq!(
            agent_update_speed_live(0x45, 1.0, 1).encode(),
            vec![0x2b, 0, 0, 0, 0x45, 0, 0, 0, 0, 0, 0x80, 0x3f, 1, 0, 0, 0]
        );
        // And the tick is eight bytes with the real elapsed time (here 503 ms).
        assert_eq!(
            world_simulation_tick_live(0x1F7).encode(),
            vec![0x1e, 0, 0, 0, 0xf7, 0x01, 0, 0]
        );
    }

    #[test]
    fn the_party_messages_are_the_bytes_a_live_server_sends() {
        // Verbatim from `PreSearing/captures/zone-ref-005829.jsonl`, the only evidence there is: the
        // client's own table says 5 and 6 bytes for these two, and live sends 12, and the client is
        // happy with live. See the builders for the precedence rule this follows.
        assert_eq!(
            player_update_party_size(1, 1).encode(),
            vec![0xb0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]
        );
        assert_eq!(
            player_update_party(7, 7).encode(),
            vec![0xb1, 0, 0, 0, 7, 0, 0, 0, 7, 0, 0, 0]
        );
        assert_eq!(player_destroy(7).encode(), vec![0x5a, 0, 0, 0, 7, 0, 0, 0]);
        assert_eq!(
            world_remove_agent(7).encode(),
            vec![0x21, 0, 0, 0, 7, 0, 0, 0]
        );
    }

    #[test]
    fn the_counted_messages_are_exactly_as_long_as_their_counts_say() {
        assert_eq!(
            wire(&instance_load_player_name("")),
            4,
            "an empty name is 2 + 2"
        );
        assert_eq!(
            wire(&instance_load_player_name("Asura")),
            14,
            "5 units: 2 + 2 + 10"
        );
        assert_eq!(
            wire(&character_update_info("")),
            28,
            "the reference's empty name"
        );
        assert_eq!(wire(&character_update_info("Asura")), 38);
        assert_eq!(wire(&player_create(1, 2, 0, "Asura")), 35, "25 + 2 + 10");
        assert_eq!(wire(&pvp_update_unlocked_skills(&[])), 4);
        assert_eq!(wire(&pvp_update_unlocked_skills(&[1, 2, 3])), 16);
        assert_eq!(wire(&pvp_update_unlocked_heroes(&[0xFFFF_FFFF; 8])), 36);
        assert_eq!(wire(&update_unlocked_skills(&[])), 4);
        assert_eq!(wire(&agent_update_attributes(1, &[0; 42])), 176);
        assert_eq!(wire(&skillbar_update(1, &[0; 8], &[], 0)), 43);
        assert_eq!(
            wire(&skillbar_update(1, &[0; 8], &[0; 8], 1)),
            75,
            "the reference sends eight skills and eight PvP masks"
        );
        assert_eq!(
            wire(&map_update_unlocked_list([&[][..], &[], &[], &[], &[]])),
            12,
            "five empty lists are five counts"
        );
        assert_eq!(
            wire(&map_update_unlocked_list([&[7][..], &[8], &[], &[], &[9]])),
            24,
            "each list is a count plus its dwords"
        );
    }

    #[test]
    fn an_items_modifier_count_is_one_byte() {
        // `TYPE_NESTED_STRUCT` is the one counted field the reference's packer counts with a `u8`
        // (`msgpack.c`). Writing two bytes shifts every modifier and the rest of the message with it.
        let bytes = create_named_item(&NamedItem {
            modifiers: vec![0xAA_BB_CC_DD],
            ..NamedItem::default()
        })
        .encode();
        // 2 header + 31 fixed + 2 for an empty name = 35, and then the count.
        assert_eq!(bytes.len(), 2 + 31 + 2 + 1 + 4);
        assert_eq!(bytes[35], 1, "one byte of count, not two");
        assert_eq!(&bytes[36..40], &[0xDD, 0xCC, 0xBB, 0xAA]);
    }

    #[test]
    fn an_item_with_no_name_and_no_modifiers_is_thirty_six_bytes() {
        // The cupcake, which is most of what the reference sends: a client that dies in its inventory
        // is a client given one of these at the wrong size.
        assert_eq!(wire(&create_named_item(&NamedItem::default())), 36);
    }

    #[test]
    fn an_items_type_says_which_equipped_slot_it_belongs_in() {
        // The six the capture carries: the reference's default character wears a hammer, a body, legs,
        // a head, boots and gloves, and `ITEM_MOVED_TO_LOCATION` puts each in the slot below.
        assert_eq!(equipped_slot(15), Some(0), "ItemType_Hammer -> weapon");
        assert_eq!(equipped_slot(7), Some(2), "ItemType_Body -> body");
        assert_eq!(equipped_slot(19), Some(3), "ItemType_Legs -> legs");
        assert_eq!(equipped_slot(16), Some(4), "ItemType_Head -> head");
        assert_eq!(equipped_slot(4), Some(5), "ItemType_Boots -> boots");
        assert_eq!(equipped_slot(13), Some(6), "ItemType_Gloves -> gloves");

        // The rest are the client's own enum names, not a measurement.
        assert_eq!(equipped_slot(27), Some(0), "Sword");
        assert_eq!(equipped_slot(24), Some(1), "Shield is the off hand");
        assert_eq!(equipped_slot(12), Some(1), "Focus is the off hand");
        assert_eq!(equipped_slot(44), Some(7), "CostumeBody");
        assert_eq!(equipped_slot(45), Some(8), "CostumeHead");

        // A cupcake is not something a character wears, and a slot is not invented for it.
        assert_eq!(equipped_slot(9), None, "ItemType_Consumable");
        assert_eq!(equipped_slot(3), None, "ItemType_Bag");
        assert_eq!(equipped_slot(20), None, "ItemType_Coins");
    }

    #[test]
    fn character_update_factions_is_fifteen_dwords_of_body() {
        let frame = character_update_factions(&CharacterProgress::default());
        assert_eq!(wire(&frame), 62);
        assert_eq!(frame.body().len(), 60, "fifteen dwords");
    }

    #[test]
    fn the_two_player_state_messages_are_the_captured_bytes() {
        // Copied from a live load, not designed: the visibility flags as `ef000000 | ffffffff |
        // ffffffff` and the player flags as `3c000000 | 01000000 | 00000000 | 07000000`
        // (recordings/asura/world/load-layers-20261031.jsonl, headers 00EF and 003C).
        let visibility = char_visibility_flags(0xFFFF_FFFF, 0xFFFF_FFFF);
        assert_eq!(
            visibility.bytes(),
            &[0xEF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
        );

        let flags = player_update_flags(1, 0, 7);
        assert_eq!(
            flags.bytes(),
            &[0x3C, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00]
        );
    }

    /// The word the reference's own `PLAYER_CREATE` carries for its character: profession 1 (Warrior) and
    /// hair style 3, everything else zero. Written out here because this crate packs and unpacks a word;
    /// the *default* a server sends for a character with no bits of its own is a value, and it lives in
    /// `config/World/Vanilla/character_defaults.json`.
    const REFERENCE_WORD: u32 = 0x0310_0000;

    #[test]
    fn the_appearance_bitfield_matches_the_clients_own_word() {
        // If this packing is wrong the client builds a model out of nonsense, so the number is asserted
        // from the wire.
        let unpacked = Appearance::unpack(REFERENCE_WORD);
        assert_eq!(unpacked.primary_profession, 1, "Warrior");
        assert_eq!(unpacked.hair_style, 3);
        assert_eq!(unpacked.race, 0, "Prophecies");
        assert_eq!(unpacked.sex, 0);
        assert_eq!(unpacked.height, 0);
        assert_eq!(
            unpacked.pack(),
            REFERENCE_WORD,
            "packing what was unpacked returns the same word"
        );
    }

    #[test]
    fn a_zero_appearance_is_not_the_references_word() {
        // The bug this exists to prevent: 0 packs to profession 0 (Profession_None) and hair 0, which
        // is a character model the client has no data for.
        assert_ne!(Appearance::default().pack(), REFERENCE_WORD);
        assert_eq!(Appearance::unpack(0).primary_profession, 0);
    }

    #[test]
    fn the_cartography_payload_is_the_arrays_the_table_declares() {
        // The row is `FieldKind::Array32` with a length of 64, the client's own array capacity. The
        // frame this client frames to carries 63 dwords and two pad bytes after a four-byte count:
        // 2 header + 4 count + 252 + 2 = 260. This test is the one place the two numbers meet.
        let row = crate::packets::PACKETS
            .iter()
            .find(|row| row.header == CARTOGRAPHY_DATA)
            .expect("the row exists");
        assert_eq!(row.size, 262, "the table's declared maximum");
        assert_eq!(
            2 + 4 + CARTOGRAPHY_DATA_BYTES + 2,
            260,
            "the frame this tree sends"
        );
        assert_eq!(
            CARTOGRAPHY_DATA_SLOTS, 63,
            "the frame's slots, not the client's array capacity"
        );
        let array = row
            .fields
            .iter()
            .find(|field| field.kind == crate::packets::FieldKind::Array32)
            .expect("the array field");
        assert_eq!(usize::from(array.length), 64, "the client's array capacity");
    }

    #[test]
    fn the_header_is_the_servers_and_the_body_follows_it() {
        let frame = map_update_current(146);
        let bytes = frame.encode();
        assert_eq!(&bytes[0..2], &0x0099u16.to_le_bytes());
        assert_eq!(&bytes[2..4], &146u16.to_le_bytes());
        assert_eq!(bytes[4], 0);
    }
}
