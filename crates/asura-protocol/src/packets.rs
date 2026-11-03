//! **Every packet, in one table.**
//!
//! One row per header of both protocol families: the auth family (the login connection) and the
//! game family (a world connection). `782` rows: auth 60 client-to-server + 40 server-to-client; game 194 + 488.
//!
//! The objective of this project is to reproduce the packet stream of the live server and client
//! (`docs/HANDOFF.md`, "The objective"), so a packet's identity is a copied fact, not a design:
//! each row carries the header, the reference's index, its size, its name and its field layout,
//! and those columns are never edited here. What *is* maintained here is what a session learns
//! about a packet: its [`Status`] and its evidence.
//!
//! ## The two columns that are facts
//!
//! * `size` is, for the auth family, the fixed on-wire size; for the game family, the reference's
//!   `max_size`, which is a maximum and never the length of a message whose layout has a counted
//!   field (`client::frame` walks those structurally instead).
//! * `fields` is the layout a decoder walks, in wire order.
//!
//! ## The two columns a session edits
//!
//! * [`Status`] says how far a packet is known: unnamed, named, observed, emitted, handled, or
//!   inferred. It is the project's progress number, and [`progress`] counts it.
//! * `evidence` names where the claim comes from: a capture under `recordings/`, a section of
//!   `docs/03-protocol.md`, a `file:line`. A name or a function that is a guess says so, and the
//!   evidence is the thing that makes it a fact.
//!
//! ## Where this file came from
//!
//! It was produced once, mechanically, by `tools/merge-packet-tables.py`, which merged the two
//! generated tables (`client::tables` and `client::game_tables`) and proved the result identical
//! to them, row for row and constant for constant. That script is the record of the merge; this
//! file is the implementation from now on, and nothing regenerates it.
//!
//! Status seed, as of the merge: the dispatchers in `asura-world` and `asura-auth` mark what this
//! tree actually handles, and the builders in [`crate::game`] mark what it sends. Everything else
//! named is `Known`, and everything the reference does not name is `Unnamed`.

/// Which connection a message belongs to. The two families share header values (`0x8000` is a
/// different message in each), so the connection is part of every key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Conn {
    /// The login connection: key exchange, session, character list, travel.
    Auth = 0,
    /// A world connection: the map, agents, items, movement.
    Game = 1,
}

impl Conn {
    /// The family's name, for a log line or an error.
    pub fn label(self) -> &'static str {
        match self {
            Conn::Auth => "auth",
            Conn::Game => "game",
        }
    }
}

/// How far a packet is known. Ordered from least to most proven, so a table can be sorted or a
/// progress line can be read as a bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    /// The reference does not name this header (342 of the game rows). It is still framed, so an
    /// unexpected-but-real message is recognised rather than mistaken for a malformed one.
    Unnamed = 0,
    /// The reference names it, and nothing else is proven yet.
    Known = 1,
    /// Seen in a capture. Nothing acts on it.
    Observed = 2,
    /// This server sends it (`crate::game` builders, `client::messages` reply builders).
    Emitted = 3,
    /// This server acts on the client's message (`asura-world`'s `answer`, `asura-auth`'s dispatch).
    Handled = 4,
    /// The name or the function was inferred rather than read: the evidence column must say from
    /// what.
    Inferred = 5,
}

impl Status {
    /// A short word a progress line or a log can print.
    pub fn label(self) -> &'static str {
        match self {
            Status::Unnamed => "unnamed",
            Status::Known => "known",
            Status::Observed => "observed",
            Status::Emitted => "emitted",
            Status::Handled => "handled",
            Status::Inferred => "inferred",
        }
    }
}

/// A field kind, as `msgdefs.c` names it (`FieldType` in `msgdefs.h`). One enum for both
/// families: the reference uses one field vocabulary for both, and the counted kinds are the same
/// on the wire -- a `u16` count and that many units, with no padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldKind {
    /// `TYPE_MSG_HEADER`: the u16 header, two bytes on the wire.
    MsgHeader,
    /// `TYPE_MSG_HEADER_GAME`: the header plus a zero u16 -- four bytes on the wire. Defined,
    /// packed and unpacked in the reference, but no row of either family uses it; kept so a
    /// capture that shows one can be named.
    MsgHeaderGame,
    /// `TYPE_AGENT_ID`.
    AgentId,
    /// `TYPE_FLOAT`.
    Float,
    /// `TYPE_VECT2`: two floats.
    Vect2,
    /// `TYPE_VECT3`: three floats.
    Vect3,
    /// `TYPE_BYTE`.
    Byte,
    /// `TYPE_WORD`.
    Word,
    /// `TYPE_DWORD`.
    Dword,
    /// `TYPE_BLOB`: `length` opaque bytes, no count prefix.
    Blob,
    /// `TYPE_STRING_16`: on the wire a `u16` count then that many u16 units; `length` is the
    /// capacity, which bounds the count and never pads the message.
    String16,
    /// `TYPE_ARRAY_8`: a `u16` count then that many u8 units.
    Array8,
    /// `TYPE_ARRAY_16`: a `u16` count then that many u16 units.
    Array16,
    /// `TYPE_ARRAY_32`: a `u16` count then that many u32 units.
    Array32,
    /// `TYPE_NESTED_STRUCT`: a `u8` count then that many repeats of the fields that follow it.
    NestedStruct,
}

/// One field: its kind and, for the kinds that carry a count or a length, that number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Field {
    pub kind: FieldKind,
    /// `field.length` in the reference: the unit capacity, the blob size, or 0 when unused.
    pub length: u16,
}

impl Field {
    /// Bytes this field occupies when a counted field is written at full capacity, or `None` for
    /// [`FieldKind::NestedStruct`], whose size depends on the repeats it carries.
    ///
    /// This is the arithmetic a declared size comes from (the reference's `max_size`): the count
    /// field is four bytes wide in the reference's struct, and the units fill the capacity. It is
    /// *not* the wire size -- see [`Field::wire_size_for`].
    pub fn wire_size(&self) -> Option<usize> {
        let n = self.length as usize;
        match self.kind {
            FieldKind::MsgHeader => Some(2),
            FieldKind::MsgHeaderGame => Some(4),
            FieldKind::AgentId | FieldKind::Float | FieldKind::Dword => Some(4),
            FieldKind::Vect2 => Some(8),
            FieldKind::Vect3 => Some(12),
            FieldKind::Byte => Some(1),
            FieldKind::Word => Some(2),
            FieldKind::Blob => Some(n),
            FieldKind::String16 | FieldKind::Array16 => Some(4 + n * 2),
            FieldKind::Array8 => Some(4 + n),
            FieldKind::Array32 => Some(4 + n * 4),
            FieldKind::NestedStruct => None,
        }
    }

    /// Bytes this field occupies on the wire when a counted field carries `count` units, or `None`
    /// for [`FieldKind::NestedStruct`].
    ///
    /// A counted field is a two-byte count then `count` units and nothing else, so even at capacity
    /// it is two bytes smaller than the field's share of a declared size: the declared maximum
    /// bounds the wire, it is not the wire.
    pub fn wire_size_for(&self, count: u16) -> Option<usize> {
        let n = self.length as usize;
        let count = count as usize;
        match self.kind {
            FieldKind::MsgHeader => Some(2),
            FieldKind::MsgHeaderGame => Some(4),
            FieldKind::AgentId | FieldKind::Float | FieldKind::Dword => Some(4),
            FieldKind::Vect2 => Some(8),
            FieldKind::Vect3 => Some(12),
            FieldKind::Byte => Some(1),
            FieldKind::Word => Some(2),
            FieldKind::Blob => Some(n),
            // The capacity is only a maximum; the count the body carries decides the wire, and the
            // reference rejects a count above the capacity (`pack_helper`).
            FieldKind::String16 | FieldKind::Array16 => Some(2 + count * 2),
            FieldKind::Array8 => Some(2 + count),
            FieldKind::Array32 => Some(2 + count * 4),
            FieldKind::NestedStruct => None,
        }
    }
}

/// One packet: a header of one family, one direction. The single source for what a message is.
#[derive(Debug, Clone, Copy)]
pub struct Packet {
    /// The family this header belongs to.
    pub conn: Conn,
    /// The wire value, mask included: `0x8000 | index` or `0x0000 | index`.
    pub header: u16,
    /// The index GWPS uses to address its format array (`header & 0x7FFF`).
    pub index: u16,
    /// The auth family's fixed on-wire size, or the game family's declared maximum. Never the
    /// length of a game message that carries a counted field.
    pub size: usize,
    /// The name `opcodes.h` gives this index; `None` for the indices it does not name.
    pub name: Option<&'static str>,
    /// Whether the header is client-to-server (`0x8000` set).
    pub from_client: bool,
    /// How far this packet is known. A session edits this column.
    pub status: Status,
    /// Where the status, name or function comes from: a capture, a protocol document section, a
    /// `file:fn`. A session edits this column, and a guess says it is a guess.
    pub evidence: &'static [&'static str],
    /// The fields the message is made of, in wire order.
    pub fields: &'static [Field],
}

impl Packet {
    /// The name for a log line: the reference's name, or the header when it has none.
    pub fn label(&self) -> String {
        match self.name {
            Some(name) => String::from(name),
            None => format!("{:#06X} {}", self.header, self.conn.label()),
        }
    }
}

/// The one table. `782` rows, sorted by `(conn, client-to-server first, header)` so
/// [`lookup`] can binary-search it. One row per packet; no other list of headers exists.
#[rustfmt::skip]
pub static PACKETS: &[Packet] = &[
    Packet { conn: Conn::Auth, header: 0x8000, index:   0, size:     6, name: Some("AUTH_CMSG_HEARTBEAT"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8001, index:   1, size:   138, name: Some("AUTH_CMSG_SEND_COMPUTER_INFO"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 1 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Auth, header: 0x8002, index:   2, size:    22, name: Some("AUTH_CMSG_SEND_COMPUTER_HASH"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 2 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }] },
    Packet { conn: Conn::Auth, header: 0x8003, index:   3, size:   214, name: Some("AUTH_CMSG_ACCOUNT_CREATE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 3 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 64 }, Field { kind: FieldKind::Blob, length: 20 }, Field { kind: FieldKind::String16, length: 26 }] },
    Packet { conn: Conn::Auth, header: 0x8004, index:   4, size:     6, name: Some("AUTH_CMSG_ACCOUNT_LOGIN"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 4 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8005, index:   5, size:   182, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 5 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 20 }, Field { kind: FieldKind::Blob, length: 20 }, Field { kind: FieldKind::String16, length: 64 }] },
    Packet { conn: Conn::Auth, header: 0x8006, index:   6, size:    10, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 6 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8007, index:   7, size:    50, name: Some("AUTH_CMSG_DELETE_CHARACTER"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 7 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x8008, index:   8, size:    50, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 8 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x8009, index:   9, size:   118, name: Some("AUTH_CMSG_UPDATE_CHARACTER_SETTINGS"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 9 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::Array8, length: 64 }] },
    Packet { conn: Conn::Auth, header: 0x800A, index:  10, size:    50, name: Some("AUTH_CMSG_CHANGE_PLAY_CHARACTER"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 10 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x800B, index:  11, size:    18, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 11 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x800C, index:  12, size:    26, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 12 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }] },
    Packet { conn: Conn::Auth, header: 0x800D, index:  13, size:     6, name: Some("AUTH_CMSG_DISCONNECT"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 13 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x800E, index:  14, size:     6, name: Some("AUTH_CMSG_SET_PLAYER_STATUS"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 14 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x800F, index:  15, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 15 }] },
    Packet { conn: Conn::Auth, header: 0x8010, index:  16, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 16 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8011, index:  17, size:   518, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 17 }, Field { kind: FieldKind::String16, length: 256 }] },
    Packet { conn: Conn::Auth, header: 0x8012, index:  18, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 18 }] },
    Packet { conn: Conn::Auth, header: 0x8013, index:  19, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 19 }] },
    Packet { conn: Conn::Auth, header: 0x8014, index:  20, size:   790, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 20 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 64 }, Field { kind: FieldKind::String16, length: 64 }, Field { kind: FieldKind::String16, length: 256 }] },
    Packet { conn: Conn::Auth, header: 0x8015, index:  21, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 21 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8016, index:  22, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 22 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8017, index:  23, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 23 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8018, index:  24, size:   138, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 24 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 64 }] },
    Packet { conn: Conn::Auth, header: 0x8019, index:  25, size:    50, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 25 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 20 }, Field { kind: FieldKind::Blob, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x801A, index:  26, size:    98, name: Some("AUTH_CMSG_FRIEND_ADD"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 26 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x801B, index:  27, size:    66, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 27 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x801C, index:  28, size:    62, name: Some("AUTH_CMSG_ADD_ACCESS_KEY"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 28 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 26 }] },
    Packet { conn: Conn::Auth, header: 0x801D, index:  29, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 29 }] },
    Packet { conn: Conn::Auth, header: 0x801E, index:  30, size:    66, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 30 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 26 }] },
    Packet { conn: Conn::Auth, header: 0x801F, index:  31, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 31 }] },
    Packet { conn: Conn::Auth, header: 0x8020, index:  32, size:   522, name: Some("AUTH_CMSG_SETTING_UPDATE_CONTENT"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 32 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array8, length: 512 }] },
    Packet { conn: Conn::Auth, header: 0x8021, index:  33, size:    10, name: Some("AUTH_CMSG_SETTING_UPDATE_SIZE"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 33 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8022, index:  34, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 34 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8023, index:  35, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 35 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8024, index:  36, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 36 }] },
    Packet { conn: Conn::Auth, header: 0x8025, index:  37, size:    38, name: Some("AUTH_CMSG_REQUEST_GUILD_HALL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 37 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }] },
    Packet { conn: Conn::Auth, header: 0x8026, index:  38, size:     3, name: Some("AUTH_CMSG_ACCEPT_EULA"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 38 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8027, index:  39, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 39 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8028, index:  40, size:   410, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 40 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 28 }, Field { kind: FieldKind::String16, length: 28 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::String16, length: 12 }, Field { kind: FieldKind::String16, length: 28 }, Field { kind: FieldKind::String16, length: 28 }] },
    Packet { conn: Conn::Auth, header: 0x8029, index:  41, size:    26, name: Some("AUTH_CMSG_REQUEST_GAME_INSTANCE"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 41 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x802A, index:  42, size:    94, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 42 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x802B, index:  43, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 43 }] },
    Packet { conn: Conn::Auth, header: 0x802C, index:  44, size:  1050, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 44 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 128 }, Field { kind: FieldKind::Array32, length: 128 }] },
    Packet { conn: Conn::Auth, header: 0x802D, index:  45, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 45 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x802E, index:  46, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 46 }] },
    Packet { conn: Conn::Auth, header: 0x802F, index:  47, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 47 }] },
    Packet { conn: Conn::Auth, header: 0x8030, index:  48, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 48 }] },
    Packet { conn: Conn::Auth, header: 0x8031, index:  49, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 49 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8032, index:  50, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 50 }] },
    Packet { conn: Conn::Auth, header: 0x8033, index:  51, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 51 }] },
    Packet { conn: Conn::Auth, header: 0x8034, index:  52, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 52 }] },
    Packet { conn: Conn::Auth, header: 0x8035, index:  53, size:     6, name: Some("AUTH_CMSG_ASK_SERVER_RESPONSE"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 53 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x8036, index:  54, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 54 }] },
    Packet { conn: Conn::Auth, header: 0x8037, index:  55, size:    94, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 55 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x8038, index:  56, size:   150, name: Some("AUTH_CMSG_PORTAL_ACCOUNT_LOGIN"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 56 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Auth, header: 0x8039, index:  57, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 57 }] },
    Packet { conn: Conn::Auth, header: 0x803A, index:  58, size:   526, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 58 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array8, length: 512 }] },
    Packet { conn: Conn::Auth, header: 0x803B, index:  59, size:   522, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 59 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array8, length: 512 }] },
    Packet { conn: Conn::Auth, header: 0x0000, index:   0, size:     6, name: Some("AUTH_SMSG_HEARTBEAT"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x0001, index:   1, size:    10, name: Some("AUTH_SMSG_SESSION_INFO"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 1 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x0002, index:   2, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 2 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x0003, index:   3, size:    10, name: Some("AUTH_SMSG_REQUEST_RESPONSE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 3 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x0004, index:   4, size:  4106, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 4 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 2048 }] },
    Packet { conn: Conn::Auth, header: 0x0005, index:   5, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 5 }] },
    Packet { conn: Conn::Auth, header: 0x0006, index:   6, size:    42, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 6 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x0007, index:   7, size:   138, name: Some("AUTH_SMSG_CHARACTER_INFO"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 7 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::Array8, length: 64 }] },
    Packet { conn: Conn::Auth, header: 0x0008, index:   8, size:    38, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 8 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 24 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x0009, index:   9, size:    42, name: Some("AUTH_SMSG_GAME_SERVER_INFO"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 9 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 24 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x000A, index:  10, size:    70, name: Some("AUTH_SMSG_FRIEND_UPDATE_INFO"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 10 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x000B, index:  11, size:   110, name: Some("AUTH_SMSG_FRIEND_UPDATE_STATUS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 11 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x000C, index:  12, size:   566, name: Some("AUTH_SMSG_WHISPER_RECEIVED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 12 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 256 }] },
    Packet { conn: Conn::Auth, header: 0x000D, index:  13, size:  1042, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 13 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 256 }, Field { kind: FieldKind::String16, length: 256 }] },
    Packet { conn: Conn::Auth, header: 0x000E, index:  14, size:    46, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 14 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array8, length: 32 }] },
    Packet { conn: Conn::Auth, header: 0x000F, index:  15, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 15 }] },
    Packet { conn: Conn::Auth, header: 0x0010, index:  16, size:   630, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 16 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 256 }] },
    Packet { conn: Conn::Auth, header: 0x0011, index:  17, size:   275, name: Some("AUTH_SMSG_ACCOUNT_INFO"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 17 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array8, length: 200 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x0012, index:  18, size:   630, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 18 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 256 }] },
    Packet { conn: Conn::Auth, header: 0x0013, index:  19, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 19 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x0014, index:  20, size:    10, name: Some("AUTH_SMSG_FRIEND_STREAM_END"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 20 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x0015, index:  21, size:    34, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 21 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x0016, index:  22, size:  2058, name: Some("AUTH_SMSG_ACCOUNT_SETTINGS"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 22 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array8, length: 2048 }] },
    Packet { conn: Conn::Auth, header: 0x0017, index:  23, size:  4102, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 23 }, Field { kind: FieldKind::Array8, length: 4096 }] },
    Packet { conn: Conn::Auth, header: 0x0018, index:  24, size:   226, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 24 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Array8, length: 200 }] },
    Packet { conn: Conn::Auth, header: 0x0019, index:  25, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 25 }] },
    Packet { conn: Conn::Auth, header: 0x001A, index:  26, size:    32, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 26 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::NestedStruct, length: 11 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x001B, index:  27, size:    18, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 27 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x001C, index:  28, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 28 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x001D, index:  29, size:    14, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 29 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x001E, index:  30, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 30 }] },
    Packet { conn: Conn::Auth, header: 0x001F, index:  31, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 31 }] },
    Packet { conn: Conn::Auth, header: 0x0020, index:  32, size:    54, name: Some("AUTH_SMSG_FRIEND_UPDATE_LOCATION"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 32 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x0021, index:  33, size:    54, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 33 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Auth, header: 0x0022, index:  34, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 34 }] },
    Packet { conn: Conn::Auth, header: 0x0023, index:  35, size:    26, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 35 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }] },
    Packet { conn: Conn::Auth, header: 0x0024, index:  36, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 36 }] },
    Packet { conn: Conn::Auth, header: 0x0025, index:  37, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 37 }] },
    Packet { conn: Conn::Auth, header: 0x0026, index:  38, size:    10, name: Some("AUTH_SMSG_SERVER_RESPONSE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 38 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Auth, header: 0x0027, index:  39, size:  4106, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 39 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array8, length: 4096 }] },
    Packet { conn: Conn::Game, header: 0x8000, index:   0, size:     6, name: Some("GAME_CMSG_TRADE_ACKNOWLEDGE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8001, index:   1, size:     2, name: Some("GAME_CMSG_TRADE_CANCEL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 1 }] },
    Packet { conn: Conn::Game, header: 0x8002, index:   2, size:     7, name: Some("GAME_CMSG_TRADE_ADD_ITEM"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 2 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8003, index:   3, size:     6, name: Some("GAME_CMSG_TRADE_SEND_OFFER"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 3 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8004, index:   4, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 4 }] },
    Packet { conn: Conn::Game, header: 0x8005, index:   5, size:     7, name: Some("GAME_CMSG_TRADE_REMOVE_ITEM"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 5 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8006, index:   6, size:     2, name: Some("GAME_CMSG_TRADE_CANCEL_OFFER"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 6 }] },
    Packet { conn: Conn::Game, header: 0x8007, index:   7, size:     2, name: Some("GAME_CMSG_TRADE_ACCEPT"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 7 }] },
    Packet { conn: Conn::Game, header: 0x8008, index:   8, size:     2, name: Some("GAME_CMSG_DISCONNECT"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x8009, index:   9, size:    10, name: Some("GAME_CMSG_PING_REPLY"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 9 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x800A, index:  10, size:   246, name: Some("GAME_CMSG_COMPUTER_INFO"), from_client: true, status: Status::Known, evidence: &["the pair a real client opens its game connection with; the auth connection opens with the same two under SEND_COMPUTER_INFO / SEND_COMPUTER_HASH (docs/09-parity-audit.md row 1), and the reference stores their contents and sends nothing. Both carry counted fields, so the row keeps the declared maximum and the frame is walked structurally: this client sends 140 and 88 bytes at the socket (recordings/asura/world/live-client-session-20261027-223528.jsonl), and the 240 and 324 a sniffer capture shows are the client's message struct, not the wire (docs/03-protocol.md 5e)"], fields: &[Field { kind: FieldKind::MsgHeader, length: 10 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 12 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 64 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x800B, index:  11, size:   330, name: Some("GAME_CMSG_COMPUTER_HASH"), from_client: true, status: Status::Known, evidence: &["the pair a real client opens its game connection with; the auth connection opens with the same two under SEND_COMPUTER_INFO / SEND_COMPUTER_HASH (docs/09-parity-audit.md row 1), and the reference stores their contents and sends nothing. Both carry counted fields, so the row keeps the declared maximum and the frame is walked structurally: this client sends 140 and 88 bytes at the socket (recordings/asura/world/live-client-session-20261027-223528.jsonl), and the 240 and 324 a sniffer capture shows are the client's message struct, not the wire (docs/03-protocol.md 5e)"], fields: &[Field { kind: FieldKind::MsgHeader, length: 11 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 16 }, Field { kind: FieldKind::String16, length: 128 }] },
    Packet { conn: Conn::Game, header: 0x800C, index:  12, size:     2, name: Some("GAME_CMSG_HEARTBEAT"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 12 }] },
    Packet { conn: Conn::Game, header: 0x800D, index:  13, size:     2, name: Some("GAME_CMSG_PING_REQUEST"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 13 }] },
    Packet { conn: Conn::Game, header: 0x800E, index:  14, size:    14, name: Some("GAME_CMSG_ATTRIBUTE_DECREASE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 14 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x800F, index:  15, size:    14, name: Some("GAME_CMSG_ATTRIBUTE_INCREASE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 15 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8010, index:  16, size:   142, name: Some("GAME_CMSG_ATTRIBUTE_LOAD"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 16 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Array32, length: 16 }, Field { kind: FieldKind::Array32, length: 16 }] },
    Packet { conn: Conn::Game, header: 0x8011, index:  17, size:     6, name: Some("GAME_CMSG_QUEST_ABANDON"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 17 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8012, index:  18, size:     6, name: Some("GAME_CMSG_QUEST_REQUEST_INFOS"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 18 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8013, index:  19, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 19 }] },
    Packet { conn: Conn::Game, header: 0x8014, index:  20, size:     7, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 20 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8015, index:  21, size:    10, name: Some("GAME_CMSG_HERO_BEHAVIOR"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 21 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8016, index:  22, size:    10, name: Some("GAME_CMSG_HERO_LOCK_TARGET"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 22 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8017, index:  23, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 23 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8018, index:  24, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 24 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8019, index:  25, size:    10, name: Some("GAME_CMSG_HERO_SKILL_TOGGLE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 25 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x801A, index:  26, size:    18, name: Some("GAME_CMSG_HERO_FLAG_SINGLE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 26 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x801B, index:  27, size:    14, name: Some("GAME_CMSG_HERO_FLAG_ALL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 27 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x801C, index:  28, size:    18, name: Some("GAME_CMSG_HERO_USE_SKILL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 28 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x801D, index:  29, size:    14, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 29 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x801E, index:  30, size:     4, name: Some("GAME_CMSG_HERO_ADD"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 30 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x801F, index:  31, size:     4, name: Some("GAME_CMSG_HERO_KICK"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 31 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8020, index:  32, size:     3, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 32 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8021, index:  33, size:    10, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 33 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8022, index:  34, size:    10, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 34 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8023, index:  35, size:     7, name: Some("GAME_CMSG_TARGET_CALL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 35 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8024, index:  36, size:    10, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 36 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8025, index:  37, size:    14, name: Some("GAME_CMSG_PING_WEAPON_SET"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 37 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8026, index:  38, size:     7, name: Some("GAME_CMSG_ATTACK_AGENT"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 38 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8027, index:  39, size:    15, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 39 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8028, index:  40, size:     2, name: Some("GAME_CMSG_CANCEL_MOVEMENT"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 40 }] },
    Packet { conn: Conn::Game, header: 0x8029, index:  41, size:     6, name: Some("GAME_CMSG_DROP_BUFF"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 41 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x802A, index:  42, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 42 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x802B, index:  43, size:    74, name: Some("GAME_CMSG_DRAW_MAP"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 43 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 16 }] },
    Packet { conn: Conn::Game, header: 0x802C, index:  44, size:     7, name: Some("GAME_CMSG_DROP_ITEM"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 44 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x802D, index:  45, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 45 }] },
    Packet { conn: Conn::Game, header: 0x802E, index:  46, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 46 }] },
    Packet { conn: Conn::Game, header: 0x802F, index:  47, size:     6, name: Some("GAME_CMSG_DROP_GOLD"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 47 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8030, index:  48, size:     6, name: Some("GAME_CMSG_EQUIP_ITEM"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 48 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8031, index:  49, size:    10, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 49 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8032, index:  50, size:     3, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 50 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8033, index:  51, size:     6, name: Some("GAME_CMSG_INTERACT_PLAYER"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 51 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8034, index:  52, size:    27, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 52 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Array32, length: 4 }] },
    Packet { conn: Conn::Game, header: 0x8035, index:  53, size:    11, name: Some("GAME_CMSG_DEPOSIT_FACTION"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 53 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8036, index:  54, size:    90, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 54 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::String16, length: 6 }] },
    Packet { conn: Conn::Game, header: 0x8037, index:  55, size:    26, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 55 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 4 }] },
    Packet { conn: Conn::Game, header: 0x8038, index:  56, size:    14, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 56 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8039, index:  57, size:     7, name: Some("GAME_CMSG_INTERACT_LIVING"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 57 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x803A, index:  58, size:    26, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 58 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 4 }] },
    Packet { conn: Conn::Game, header: 0x803B, index:  59, size:     6, name: Some("GAME_CMSG_SEND_DIALOG"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 59 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x803C, index:  60, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 60 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x803D, index:  61, size:    26, name: Some("GAME_CMSG_POSITION_UPDATE"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 61 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x803E, index:  62, size:    14, name: Some("GAME_CMSG_MOVE_TO_COORD"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 62 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x803F, index:  63, size:     7, name: Some("GAME_CMSG_INTERACT_ITEM"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 63 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8040, index:  64, size:    10, name: Some("GAME_CMSG_ROTATE_PLAYER"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 64 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8041, index:  65, size:     7, name: Some("GAME_CMSG_CHANGE_SECOND_PROFESSION"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 65 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8042, index:  66, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 66 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8043, index:  67, size:     3, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 67 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8044, index:  68, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 68 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8045, index:  69, size:   262, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 69 }, Field { kind: FieldKind::Array8, length: 256 }] },
    Packet { conn: Conn::Game, header: 0x8046, index:  70, size:    15, name: Some("GAME_CMSG_USE_SKILL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 70 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8047, index:  71, size:    14, name: Some("GAME_CMSG_LAST_POS_BEFORE_MOVE_CANCELED"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 71 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8048, index:  72, size:    14, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 72 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8049, index:  73, size:     6, name: Some("GAME_CMSG_TRADE_INITIATE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 73 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x804A, index:  74, size:   147, name: Some("GAME_CMSG_BUY_MATERIALS"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 74 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 16 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 16 }] },
    Packet { conn: Conn::Game, header: 0x804B, index:  75, size:     3, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 75 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x804C, index:  76, size:   148, name: Some("GAME_CMSG_REQUEST_QUOTE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 76 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 16 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 16 }] },
    Packet { conn: Conn::Game, header: 0x804D, index:  77, size:   187, name: Some("GAME_CMSG_TRANSACT_ITEMS"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 77 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 16 }, Field { kind: FieldKind::Array8, length: 16 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 16 }, Field { kind: FieldKind::Array8, length: 16 }] },
    Packet { conn: Conn::Game, header: 0x804E, index:  78, size:   251, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 78 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x804F, index:  79, size:     6, name: Some("GAME_CMSG_UNEQUIP_ITEM"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 79 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8050, index:  80, size:    10, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 80 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8051, index:  81, size:     7, name: Some("GAME_CMSG_INTERACT_GADGET"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 81 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8052, index:  82, size:    10, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 82 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8053, index:  83, size:     6, name: Some("GAME_CMSG_OPEN_CHEST"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 83 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8054, index:  84, size:    15, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 84 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8055, index:  85, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 85 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8056, index:  86, size:     8, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 86 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8057, index:  87, size:    10, name: Some("GAME_CMSG_EQUIP_VISIBILITY"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 87 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8058, index:  88, size:     6, name: Some("GAME_CMSG_TITLE_DISPLAY"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 88 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8059, index:  89, size:     2, name: Some("GAME_CMSG_TITLE_HIDE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 89 }] },
    Packet { conn: Conn::Game, header: 0x805A, index:  90, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 90 }] },
    Packet { conn: Conn::Game, header: 0x805B, index:  91, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 91 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x805C, index:  92, size:    18, name: Some("GAME_CMSG_SKILLBAR_SKILL_SET"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 92 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x805D, index:  93, size:    42, name: Some("GAME_CMSG_SKILLBAR_LOAD"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 93 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Array32, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x805E, index:  94, size:    22, name: Some("GAME_CMSG_SKILLBAR_SKILL_REPLACE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 94 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x805F, index:  95, size:    18, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 95 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8060, index:  96, size:     4, name: Some("GAME_CMSG_CHAR_CREATION_CHANGE_PROF"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 96 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8061, index:  97, size:    70, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 97 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x8062, index:  98, size:    71, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 98 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8063, index:  99, size:     2, name: Some("GAME_CMSG_SKIP_CINEMATIC"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 99 }] },
    Packet { conn: Conn::Game, header: 0x8064, index: 100, size:   286, name: Some("GAME_CMSG_SEND_CHAT_MESSAGE"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 100 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::String16, length: 138 }] },
    Packet { conn: Conn::Game, header: 0x8065, index: 101, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 101 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8066, index: 102, size:    11, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 102 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8067, index: 103, size:    10, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 103 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8068, index: 104, size:     4, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 104 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8069, index: 105, size:     6, name: Some("GAME_CMSG_ITEM_DESTROY"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 105 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x806A, index: 106, size:    26, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 106 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 4 }] },
    Packet { conn: Conn::Game, header: 0x806B, index: 107, size:     7, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 107 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x806C, index: 108, size:    10, name: Some("GAME_CMSG_ITEM_IDENTIFY"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 108 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x806D, index: 109, size:    10, name: Some("GAME_CMSG_TOME_UNLOCK_SKILL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 109 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x806E, index: 110, size:    10, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 110 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x806F, index: 111, size:    10, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 111 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8070, index: 112, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 112 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8071, index: 113, size:    37, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 113 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::NestedStruct, length: 6 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8072, index: 114, size:     9, name: Some("GAME_CMSG_ITEM_MOVE"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 114 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8073, index: 115, size:     4, name: Some("GAME_CMSG_ITEM_ACCEPT_ALL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 115 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8074, index: 116, size:     4, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 116 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8075, index: 117, size:    13, name: Some("GAME_CMSG_ITEM_SPLIT_STACK"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 117 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8076, index: 118, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 118 }] },
    Packet { conn: Conn::Game, header: 0x8077, index: 119, size:    12, name: Some("GAME_CMSG_ITEM_SALVAGE_SESSION_OPEN"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 119 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8078, index: 120, size:     2, name: Some("GAME_CMSG_ITEM_SALVAGE_SESSION_CANCEL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 120 }] },
    Packet { conn: Conn::Game, header: 0x8079, index: 121, size:     2, name: Some("GAME_CMSG_ITEM_SALVAGE_SESSION_DONE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 121 }] },
    Packet { conn: Conn::Game, header: 0x807A, index: 122, size:     2, name: Some("GAME_CMSG_ITEM_SALVAGE_MATERIALS"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x807B, index: 123, size:     3, name: Some("GAME_CMSG_ITEM_SALVAGE_UPGRADE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 123 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x807C, index: 124, size:    10, name: Some("GAME_CMSG_ITEM_CHANGE_GOLD"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 124 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x807D, index: 125, size:     9, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 125 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x807E, index: 126, size:     6, name: Some("GAME_CMSG_ITEM_USE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 126 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x807F, index: 127, size:     7, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 127 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8080, index: 128, size:     8, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 128 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8081, index: 129, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 129 }] },
    Packet { conn: Conn::Game, header: 0x8082, index: 130, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 130 }] },
    Packet { conn: Conn::Game, header: 0x8083, index: 131, size:     3, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 131 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8084, index: 132, size:     4, name: Some("GAME_CMSG_ITEM_UPDATE_EQUIPPED_COLOR"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 132 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8085, index: 133, size:     4, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 133 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8086, index: 134, size:    21, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 134 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Array16, length: 4 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8087, index: 135, size:     5, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 135 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8088, index: 136, size:     2, name: Some("GAME_CMSG_INSTANCE_LOAD_REQUEST_SPAWN"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 136 }] },
    Packet { conn: Conn::Game, header: 0x8089, index: 137, size:     2, name: Some("GAME_CMSG_CHAR_CREATION_REQUEST_PLAYER"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 137 }] },
    Packet { conn: Conn::Game, header: 0x808A, index: 138, size:     2, name: Some("GAME_CMSG_CHAR_CREATION_REQUEST_ARMORS"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 138 }] },
    Packet { conn: Conn::Game, header: 0x808B, index: 139, size:    58, name: Some("GAME_CMSG_CHAR_CREATION_CONFIRM"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 139 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::Blob, length: 8 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x808C, index: 140, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 140 }] },
    Packet { conn: Conn::Game, header: 0x808D, index: 141, size:     5, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 141 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x808E, index: 142, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 142 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x808F, index: 143, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 143 }] },
    Packet { conn: Conn::Game, header: 0x8090, index: 144, size:    19, name: Some("GAME_CMSG_INSTANCE_LOAD_REQUEST_PLAYERS"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 144 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8091, index: 145, size:     4, name: Some("GAME_CMSG_INSTANCE_LOAD_REQUEST_ITEMS"), from_client: true, status: Status::Handled, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 145 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8092, index: 146, size:   122, name: None, from_client: true, status: Status::Unnamed, evidence: &["72 sightings from this client, 120 wire bytes: a u16 near the instance id then a run of DAT-shaped ids (recordings/asura/feature/travel/first-crossing-20261031.jsonl and slow-crossing.jsonl), sent while an instance is being chosen or crossed with 0x8093. It is one of the two client questions no live capture carries (docs/23-initialization-protocol.md:73), so only a provoked world-map click can name it"], fields: &[Field { kind: FieldKind::MsgHeader, length: 146 }, Field { kind: FieldKind::Array8, length: 116 }] },
    Packet { conn: Conn::Game, header: 0x8093, index: 147, size:     4, name: Some("GAME_CMSG_INSTANCE_QUERY"), from_client: true, status: Status::Observed, evidence: &["eight sightings in one and a half seconds of one run, all while the world map opened, one per instance: 166, 145, 146, 148, 160, 161, 162, 164 (capture of 2026-10-31 19:48, local raw). The payload is the instance id alone, so the client asks the server about each instance the map can show. No capture carries an answer to it"], fields: &[Field { kind: FieldKind::MsgHeader, length: 147 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8094, index: 148, size:     3, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 148 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8095, index: 149, size:     3, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 149 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8096, index: 150, size:     9, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 150 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x8097, index: 151, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 151 }] },
    Packet { conn: Conn::Game, header: 0x8098, index: 152, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 152 }] },
    Packet { conn: Conn::Game, header: 0x8099, index: 153, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 153 }] },
    Packet { conn: Conn::Game, header: 0x809A, index: 154, size:     2, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 154 }] },
    Packet { conn: Conn::Game, header: 0x809B, index: 155, size:     3, name: Some("GAME_CMSG_PARTY_SET_DIFFICULTY"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 155 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x809C, index: 156, size:     4, name: Some("GAME_CMSG_PARTY_ACCEPT_INVITE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 156 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x809D, index: 157, size:     4, name: Some("GAME_CMSG_PARTY_ACCEPT_CANCEL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 157 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x809E, index: 158, size:     4, name: Some("GAME_CMSG_PARTY_ACCEPT_REFUSE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 158 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x809F, index: 159, size:     4, name: Some("GAME_CMSG_PARTY_INVITE_NPC"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 159 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80A0, index: 160, size:     4, name: Some("GAME_CMSG_PARTY_INVITE_PLAYER"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 160 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80A1, index: 161, size:    46, name: Some("GAME_CMSG_PARTY_INVITE_PLAYER_NAME"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 161 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x80A2, index: 162, size:     2, name: Some("GAME_CMSG_PARTY_LEAVE_GROUP"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 162 }] },
    Packet { conn: Conn::Game, header: 0x80A3, index: 163, size:     2, name: Some("GAME_CMSG_PARTY_CANCEL_ENTER_CHALLENGE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 163 }] },
    Packet { conn: Conn::Game, header: 0x80A4, index: 164, size:   135, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 164 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 64 }] },
    Packet { conn: Conn::Game, header: 0x80A5, index: 165, size:     3, name: Some("GAME_CMSG_PARTY_ENTER_CHALLENGE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 165 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80A6, index: 166, size:     9, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 166 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80A7, index: 167, size:     2, name: Some("GAME_CMSG_PARTY_RETURN_TO_OUTPOST"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 167 }] },
    Packet { conn: Conn::Game, header: 0x80A8, index: 168, size:     4, name: Some("GAME_CMSG_PARTY_KICK_NPC"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 168 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80A9, index: 169, size:     4, name: Some("GAME_CMSG_PARTY_KICK_PLAYER"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 169 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80AA, index: 170, size:    73, name: Some("GAME_CMSG_PARTY_SEARCH_SEEK"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 170 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80AB, index: 171, size:     2, name: Some("GAME_CMSG_PARTY_SEARCH_CANCEL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 171 }] },
    Packet { conn: Conn::Game, header: 0x80AC, index: 172, size:     4, name: Some("GAME_CMSG_PARTY_SEARCH_REQUEST_JOIN"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 172 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80AD, index: 173, size:     4, name: Some("GAME_CMSG_PARTY_SEARCH_REQUEST_REPLY"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 173 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80AE, index: 174, size:     3, name: Some("GAME_CMSG_PARTY_SEARCH_TYPE"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 174 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80AF, index: 175, size:     3, name: Some("GAME_CMSG_PARTY_READY_STATUS"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 175 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80B0, index: 176, size:    19, name: Some("GAME_CMSG_PARTY_ENTER_GUILD_HALL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 176 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80B1, index: 177, size:     9, name: Some("GAME_CMSG_PARTY_TRAVEL"), from_client: true, status: Status::Handled, evidence: &["logs/world-8001.log: a real client sending B1 80 | 94 00 | 02 | 00 00 | 00 | 01 (instance 148) and B1 80 | A6 00 | 02 | 00 00 | 00 | 01 (instance 166) - the destination instance leads the message", "captures/zone-ref2-012225.jsonl seq 562: the same request from the retail build with a 32-bit instance id first, answered there with 0x01A6 and 0x0099"], fields: &[Field { kind: FieldKind::MsgHeader, length: 177 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80B2, index: 178, size:     3, name: Some("GAME_CMSG_PARTY_LEAVE_GUILD_HALL"), from_client: true, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 178 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80B3, index: 179, size:    50, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 179 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x80B4, index: 180, size:    50, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 180 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x80B5, index: 181, size:    75, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 181 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80B6, index: 182, size:    23, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 182 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80B7, index: 183, size:    23, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 183 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80B8, index: 184, size:    50, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 184 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x80B9, index: 185, size:     8, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 185 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80BA, index: 186, size:    50, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 186 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x80BB, index: 187, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 187 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80BC, index: 188, size:     6, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 188 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80BD, index: 189, size:    50, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 189 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x80BE, index: 190, size:   522, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 190 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 256 }] },
    Packet { conn: Conn::Game, header: 0x80BF, index: 191, size:    75, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 191 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x80C0, index: 192, size:    71, name: None, from_client: true, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 192 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x80C1, index: 193, size:    10, name: Some("GAME_CMSG_TRAVEL_INIT"), from_client: true, status: Status::Observed, evidence: &["recordings/client/feature/movement/movement-sniff.log seq 1651: a real client sends TRAVEL_INIT as c1 00 00 00 | 3a 00 00 00 | 00 00 00 00, the sniffer's four-byte-header form of this row's two agent ids", "recordings/client/feature/movement/movement-sniff.log seq 1652: the same client sends INTERACT_LIVING for the same agent 0x3A in the same millisecond, so the first id is the object the player used", "docs/09-parity-audit.md section 4: a client-announced travel begins with TRAVEL_INIT, which the reference's /tp skips"], fields: &[Field { kind: FieldKind::MsgHeader, length: 193 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0000, index:   0, size:     6, name: Some("GAME_SMSG_TRADE_REQUEST"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0001, index:   1, size:     3, name: Some("GAME_SMSG_TRADE_TERMINATE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 1 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0002, index:   2, size:     7, name: Some("GAME_SMSG_TRADE_CHANGE_OFFER"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 2 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0003, index:   3, size:     6, name: Some("GAME_SMSG_TRADE_RECEIVE_OFFER"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 3 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0004, index:   4, size:     7, name: Some("GAME_SMSG_TRADE_ADD_ITEM"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 4 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0005, index:   5, size:     6, name: Some("GAME_SMSG_TRADE_ACKNOWLEDGE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 5 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0006, index:   6, size:     2, name: Some("GAME_SMSG_TRADE_ACCEPT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 6 }] },
    Packet { conn: Conn::Game, header: 0x0007, index:   7, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 7 }] },
    Packet { conn: Conn::Game, header: 0x0008, index:   8, size:     2, name: Some("GAME_SMSG_TRADE_OFFERED_COUNT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x0009, index:   9, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 9 }] },
    Packet { conn: Conn::Game, header: 0x000A, index:  10, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 10 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x000B, index:  11, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 11 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x000C, index:  12, size:     2, name: Some("GAME_SMSG_PING_REQUEST"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 12 }] },
    Packet { conn: Conn::Game, header: 0x000D, index:  13, size:     6, name: Some("GAME_SMSG_PING_REPLY"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 13 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x000E, index:  14, size:  1042, name: Some("GAME_SMSG_FRIENDLIST_MESSAGE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 14 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 256 }, Field { kind: FieldKind::String16, length: 256 }] },
    Packet { conn: Conn::Game, header: 0x000F, index:  15, size:     8, name: Some("GAME_SMSG_ACCOUNT_FEATURE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 15 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0010, index:  16, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 16 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0011, index:  17, size:     4, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 17 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0012, index:  18, size:     4, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 18 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0013, index:  19, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 19 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0014, index:  20, size:    11, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 20 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0015, index:  21, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 21 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0016, index:  22, size:   250, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 22 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x0017, index:  23, size:    13, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 23 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0018, index:  24, size:    38, name: Some("GAME_SMSG_PVP_UPDATE_UNLOCKED_HEROES"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 24 }, Field { kind: FieldKind::Array32, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x0019, index:  25, size:   265, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 25 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::NestedStruct, length: 64 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x001A, index:  26, size:   264, name: Some("GAME_SMSG_PVP_ITEM_STREAM_ADD"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 26 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::NestedStruct, length: 64 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x001B, index:  27, size:     2, name: Some("GAME_SMSG_PVP_ITEM_STREAM_END"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 27 }] },
    Packet { conn: Conn::Game, header: 0x001C, index:  28, size:     5, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 28 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x001D, index:  29, size:   518, name: Some("GAME_SMSG_PVP_UPDATE_UNLOCKED_SKILLS"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 29 }, Field { kind: FieldKind::Array32, length: 128 }] },
    Packet { conn: Conn::Game, header: 0x001E, index:  30, size:     6, name: Some("GAME_SMSG_WORLD_SIMULATION_TICK"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 30 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x001F, index:  31, size:     6, name: Some("GAME_SMSG_WORLD_UPDATE_LOAD_TIME"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 31 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0020, index:  32, size:    99, name: Some("GAME_SMSG_WORLD_CREATE_AGENT"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 32 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Float, length: 0 }, Field { kind: FieldKind::Float, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0021, index:  33, size:     8, name: Some("GAME_SMSG_WORLD_REMOVE_AGENT"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeaderGame, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0022, index:  34, size:    10, name: Some("GAME_SMSG_WORLD_UPDATE_CONTROLLED_AGENT"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 34 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0023, index:  35, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 35 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0024, index:  36, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 36 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0025, index:  37, size:    15, name: Some("GAME_SMSG_AGENT_UPDATE_DIRECTION"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 37 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0026, index:  38, size:     7, name: Some("GAME_SMSG_AGENT_UPDATE_FLAGS"), from_client: false, status: Status::Observed, evidence: &["7 sightings, two dwords: an agent id then a flags word (8 or 9) - recordings/client/feature/travel/doors-20261031-095754.jsonl seq 410, 1079, 2988, 8998. The client own handler writes the dword at agent+0x20 under AgAgent.cpp:2301 (m_flags & INTERNAL_FLAG_IN_WORLD), keeping the old bits inside mask 0x3f0000 and taking the new ones elsewhere (Rurik, studies/smsg/FINDINGS.md:436-452, build 38797). The reference table leaves this header unnamed"], fields: &[Field { kind: FieldKind::MsgHeader, length: 38 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0027, index:  39, size:    10, name: Some("GAME_SMSG_AGENT_UPDATE_SPEED_BASE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 39 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Float, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0028, index:  40, size:     6, name: Some("GAME_SMSG_AGENT_STOP_MOVING"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 40 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0029, index:  41, size:    18, name: Some("GAME_SMSG_AGENT_MOVE_TO_POINT"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 41 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x002A, index:  42, size:    22, name: Some("GAME_SMSG_AGENT_UPDATE_DESTINATION"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 42 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x002B, index:  43, size:    11, name: Some("GAME_SMSG_AGENT_UPDATE_SPEED"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 43 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Float, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x002C, index:  44, size:    16, name: Some("GAME_SMSG_AGENT_UPDATE_POSITION"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 44 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x002D, index:  45, size:     6, name: Some("GAME_SMSG_AGENT_PLAYER_DIE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 45 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x002E, index:  46, size:    14, name: Some("GAME_SMSG_AGENT_UPDATE_ROTATION"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 46 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x002F, index:  47, size:    10, name: Some("GAME_SMSG_AGENT_UPDATE_ALLEGIANCE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 47 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0030, index:  48, size:    94, name: Some("GAME_SMSG_CHARACTER_UPDATE_INFO"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 48 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0031, index:  49, size:    70, name: Some("GAME_SMSG_CHARACTER_UPDATE_NAME"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 49 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x0032, index:  50, size:    26, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 50 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0033, index:  51, size:   134, name: Some("GAME_SMSG_MESSAGE_OF_THE_DAY"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 51 }, Field { kind: FieldKind::String16, length: 64 }] },
    Packet { conn: Conn::Game, header: 0x0034, index:  52, size:     8, name: Some("GAME_SMSG_AGENT_PINGED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 52 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0035, index:  53, size:    14, name: None, from_client: false, status: Status::Unnamed, evidence: &["4 sightings, 14 bytes: an agent id then two floats whose second is 1.0 (0x28 / 2.472 / 1.0, 0x14 / 1.75 / 1.0 in recordings/client/feature/travel/doors-20261031-095754.jsonl seq 2969, 10453). It arrives inside an agent block, right after UPDATE_AGENT_VISUAL_EQUIPMENT or NPC_UPDATE_WEAPONS (0x006D) and before a property update, which is where a weapon interval would be declared. No catalogue of this lineage names it; a provoked test is a weapon or henchman gear change with the header watched"], fields: &[Field { kind: FieldKind::MsgHeader, length: 53 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0036, index:  54, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 54 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0037, index:  55, size:     8, name: Some("GAME_SMSG_AGENT_UPDATE_ATTRIBUTE_POINTS"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 55 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0038, index:  56, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 56 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0039, index:  57, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 57 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x003A, index:  58, size:   202, name: Some("GAME_SMSG_AGENT_UPDATE_ATTRIBUTES"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 58 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Array32, length: 48 }] },
    Packet { conn: Conn::Game, header: 0x003B, index:  59, size:     9, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 59 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x003C, index:  60, size:    12, name: Some("GAME_SMSG_PLAYER_UPDATE_FLAGS"), from_client: false, status: Status::Emitted, evidence: &["115 sightings; the fields are a u16 player id, a u32 value and a u32 mask, and the mask is 7 in all 29 samples of recordings/client/feature/maps/pre-searing-load-3.jsonl (seq 219, 306, 336, 352). It arrives once per agent in a load, between AGENT_PROPERTY_UPDATE_INT and AGENT_INITIAL_EFFECTS. The client handler does (old & ~mask) | value on the flags word at player_record+0x34 and posts UI event 0x10000067 to the world view and the attribute panel (Rurik, studies/smsgnames/FINDINGS.md:210-224, earned there 2026-08-22). Which of the three bits carries what is unsettled. This tree now sends it: crates/asura-world/src/player.rs, once per load right after GAME_SMSG_CHARACTER_UPDATE_FACTIONS, value 0 under mask 7, pinned by the_two_player_state_messages_are_the_captured_bytes"], fields: &[Field { kind: FieldKind::MsgHeader, length: 60 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x003D, index:  61, size:   287, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 61 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::String16, length: 128 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x003E, index:  62, size:     6, name: Some("GAME_SMSG_AGENT_ALLY_DESTROY"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 62 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x003F, index:  63, size:    20, name: Some("GAME_SMSG_EFFECT_UPKEEP_ADDED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 63 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0040, index:  64, size:    10, name: Some("GAME_SMSG_EFFECT_UPKEEP_REMOVED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 64 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0041, index:  65, size:    20, name: Some("GAME_SMSG_EFFECT_UPKEEP_APPLIED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 65 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0042, index:  66, size:    20, name: Some("GAME_SMSG_EFFECT_APPLIED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 66 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0043, index:  67, size:    18, name: Some("GAME_SMSG_EFFECT_RENEWED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 67 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0044, index:  68, size:    10, name: Some("GAME_SMSG_EFFECT_REMOVED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 68 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0045, index:  69, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 69 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0046, index:  70, size:    14, name: Some("GAME_SMSG_SCREEN_SHAKE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 70 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0047, index:  71, size:    20, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 71 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0048, index:  72, size:     7, name: Some("GAME_SMSG_AGENT_DISPLAY_CAPE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 72 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0049, index:  73, size:    84, name: Some("GAME_SMSG_QUEST_ADD"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 73 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x004A, index:  74, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 74 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x004B, index:  75, size:   262, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 75 }, Field { kind: FieldKind::Array32, length: 64 }] },
    Packet { conn: Conn::Game, header: 0x004C, index:  76, size:   526, name: Some("GAME_SMSG_QUEST_DESCRIPTION"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 76 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 128 }, Field { kind: FieldKind::String16, length: 128 }] },
    Packet { conn: Conn::Game, header: 0x004D, index:  77, size:    18, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 77 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x004E, index:  78, size:    14, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 78 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x004F, index:  79, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 79 }] },
    Packet { conn: Conn::Game, header: 0x0050, index:  80, size:    72, name: Some("GAME_SMSG_QUEST_GENERAL_INFO"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 80 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0051, index:  81, size:    18, name: Some("GAME_SMSG_QUEST_UPDATE_MARKER"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 81 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0052, index:  82, size:     6, name: Some("GAME_SMSG_QUEST_REMOVE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 82 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0053, index:  83, size:    18, name: Some("GAME_SMSG_QUEST_ADD_MARKER"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 83 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0054, index:  84, size:   266, name: Some("GAME_SMSG_QUEST_UPDATE_NAME"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 84 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 128 }] },
    Packet { conn: Conn::Game, header: 0x0055, index:  85, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 85 }] },
    Packet { conn: Conn::Game, header: 0x0056, index:  86, size:    48, name: Some("GAME_SMSG_NPC_UPDATE_PROPERTIES"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 86 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x0057, index:  87, size:    42, name: Some("GAME_SMSG_NPC_UPDATE_MODEL"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 87 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x0058, index:  88, size:    74, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 88 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x0059, index:  89, size:    91, name: Some("GAME_SMSG_PLAYER_CREATE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 89 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x005A, index:  90, size:     8, name: Some("GAME_SMSG_PLAYER_DESTROY"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeaderGame, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x005B, index:  91, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 91 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x005C, index:  92, size:    14, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 92 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x005D, index:  93, size:   250, name: Some("GAME_SMSG_CHAT_MESSAGE_CORE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 93 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x005E, index:  94, size:     5, name: Some("GAME_SMSG_CHAT_MESSAGE_SERVER"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 94 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x005F, index:  95, size:    28, name: Some("GAME_SMSG_CHAT_MESSAGE_NPC"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 95 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x0060, index:  96, size:    86, name: Some("GAME_SMSG_CHAT_MESSAGE_GLOBAL"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 96 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::String16, length: 6 }] },
    Packet { conn: Conn::Game, header: 0x0061, index:  97, size:     5, name: Some("GAME_SMSG_CHAT_MESSAGE_LOCAL"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 97 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0062, index:  98, size:    10, name: Some("GAME_SMSG_HERO_BEHAVIOR"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 98 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0063, index:  99, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 99 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0064, index: 100, size:     8, name: Some("GAME_SMSG_HERO_SKILL_STATUS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 100 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0065, index: 101, size:     7, name: Some("GAME_SMSG_HERO_SKILL_STATUS_BITMAP"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 101 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0066, index: 102, size:    16, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 102 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0067, index: 103, size:    12, name: None, from_client: false, status: Status::Unnamed, evidence: &["One sighting, 12 bytes: a vec2 of two FLT_MAX (0000807f 0000807f) then a zero word, immediately before CINEMATIC_START (0x0100) - recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl seq 800. A position that is not set yet, handed to a cinematic. Unnamed in every catalogue; a provoked test is any cinematic"], fields: &[Field { kind: FieldKind::MsgHeader, length: 103 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0068, index: 104, size:    12, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 104 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0069, index: 105, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 105 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x006A, index: 106, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 106 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x006B, index: 107, size:     7, name: Some("GAME_SMSG_POST_PROCESS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 107 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x006C, index: 108, size:    14, name: Some("GAME_SMSG_DUNGEON_REWARD"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 108 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x006D, index: 109, size:    14, name: Some("GAME_SMSG_NPC_UPDATE_WEAPONS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 109 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x006E, index: 110, size:    42, name: Some("GAME_SMSG_UPDATE_AGENT_VISUAL_EQUIPMENT"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 110 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x006F, index: 111, size:    14, name: Some("GAME_SMSG_UPDATE_AGENT_VISUAL_EQUIPMENT_SLOT"), from_client: false, status: Status::Observed, evidence: &["One sighting, 14 bytes: agent, slot, item id (recordings/client/feature/travel/doors-20261031-095754.jsonl seq 2986, agent 0x27 slot 1 item 0). The source reading is the single-slot form of 0x006E: one item into one of the same nine positions the bulk UPDATE_AGENT_VISUAL_EQUIPMENT carries, and retail wire pairs and each one with a 0x015E declaring that item, whose type matches the slot (2 body, 3 boots, 4 legs, 5 gloves, 6 head) - Rurik, studies/newopcodes/FINDINGS.md:1710-1750, where no catalogue names it"], fields: &[Field { kind: FieldKind::MsgHeader, length: 111 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0070, index: 112, size:     3, name: Some("GAME_SMSG_HARD_MODE_UNLOCKED"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 112 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0071, index: 113, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 113 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0072, index: 114, size:    16, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 114 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0073, index: 115, size:    53, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 115 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 8 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0074, index: 116, size:   129, name: Some("GAME_SMSG_MERCENARY_INFO"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 116 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x0075, index: 117, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 117 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0076, index: 118, size:     4, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 118 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0077, index: 119, size:     4, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 119 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0078, index: 120, size:   391, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 120 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::NestedStruct, length: 32 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0079, index: 121, size:     8, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 121 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x007A, index: 122, size:    86, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 122 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x007B, index: 123, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 123 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x007C, index: 124, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &["32 sightings, one dword that is 1 in 31 of them and 2 in one, in the per-player refresh block: after INSTANCE_LOADED (0x00F2) and before 0x007D - recordings/client/feature/travel/doors-20261031-095754.jsonl seq 249, 617, 2453. Nothing names it in this lineage"], fields: &[Field { kind: FieldKind::MsgHeader, length: 124 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x007D, index: 125, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &["33 sightings, one dword that reads as a player or agent id (2, 0x3d, 0x1f, 0x20 in recordings/client/feature/travel/doors-20261031-095754.jsonl seq 250, 618, 972), always immediately after 0x007C. The pair looks like a per-player declaration; unnamed in every catalogue"], fields: &[Field { kind: FieldKind::MsgHeader, length: 125 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x007E, index: 126, size:   271, name: Some("GAME_SMSG_DIALOG_BUTTON"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 126 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 128 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x007F, index: 127, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 127 }] },
    Packet { conn: Conn::Game, header: 0x0080, index: 128, size:   250, name: Some("GAME_SMSG_DIALOG_BODY"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 128 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x0081, index: 129, size:     6, name: Some("GAME_SMSG_DIALOG_SENDER"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 129 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0082, index: 130, size:     8, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 130 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0083, index: 131, size:    11, name: Some("GAME_SMSG_WINDOW_OPEN"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 131 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0084, index: 132, size:    70, name: Some("GAME_SMSG_WINDOW_ADD_ITEMS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 132 }, Field { kind: FieldKind::Array32, length: 16 }] },
    Packet { conn: Conn::Game, header: 0x0085, index: 133, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 133 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0086, index: 134, size:     3, name: Some("GAME_SMSG_WINDOW_ITEM_STREAM_END"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 134 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0087, index: 135, size:    46, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 135 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0088, index: 136, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 136 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0089, index: 137, size:    23, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 137 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Vect3, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x008A, index: 138, size:   262, name: Some("GAME_SMSG_CARTOGRAPHY_DATA"), from_client: false, status: Status::Emitted, evidence: &["132 struct bytes on every load of one run except the session first (capture of 2026-10-31 19:48), which is a counted array and not the 4096 or 256 areas the client holds: worldcontext.cartographed_areas read 4096 in one map and 256 in another while this message stayed the same size, so what it carries is not the whole grid"], fields: &[Field { kind: FieldKind::MsgHeader, length: 138 }, Field { kind: FieldKind::Array32, length: 64 }] },
    Packet { conn: Conn::Game, header: 0x008B, index: 139, size:    14, name: Some("GAME_SMSG_MAP_INIT_START"), from_client: false, status: Status::Emitted, evidence: &["docs/03-protocol.md, the exploration grid: three dwords after the header - the shape this client frames to"], fields: &[Field { kind: FieldKind::MsgHeader, length: 139 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x008C, index: 140, size:    11, name: None, from_client: false, status: Status::Unnamed, evidence: &["54 sightings, 11 bytes: an id (0x20, 0x21, 32, 33), a second small number (20 to 22) and a byte that is 1 - 36 in a town capture and 15 in a movement capture, after MOVE_TO_POINT (0x0029) or between ticks (movement-sniff.log seq 10, 90, 259). Unnamed in every catalogue"], fields: &[Field { kind: FieldKind::MsgHeader, length: 140 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x008D, index: 141, size:    45, name: Some("GAME_SMSG_MAP_MARKER"), from_client: false, status: Status::Inferred, evidence: &["One sighting, 48 struct bytes: recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl seq 1100, layout vec2, word, byte, dword, dword, string16[8] - the indexed marker record keyed into the marker table at ctx+0x2c +0x7ec (Rurik, studies/smsgnames/FINDINGS.md section 4, candidate MAP_MARKER, kept PARTIAL there). The name is the function that reading gives it; this tree has not exercised it"], fields: &[Field { kind: FieldKind::MsgHeader, length: 141 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x008E, index: 142, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 142 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x008F, index: 143, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 143 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0090, index: 144, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 144 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0091, index: 145, size:    76, name: Some("GAME_SMSG_COMPASS_DRAWING"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 145 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 16 }] },
    Packet { conn: Conn::Game, header: 0x0092, index: 146, size:    12, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 146 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0093, index: 147, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 147 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0094, index: 148, size:   662, name: Some("GAME_SMSG_MAP_UPDATE_UNLOCKED_LIST"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 148 }, Field { kind: FieldKind::Array32, length: 32 }, Field { kind: FieldKind::Array32, length: 32 }, Field { kind: FieldKind::Array32, length: 32 }, Field { kind: FieldKind::Array32, length: 32 }, Field { kind: FieldKind::Array32, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x0095, index: 149, size:     8, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 149 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0096, index: 150, size:    22, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 150 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0097, index: 151, size:   263, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 151 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 128 }] },
    Packet { conn: Conn::Game, header: 0x0098, index: 152, size:    15, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 152 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0099, index: 153, size:     5, name: Some("GAME_SMSG_MAP_UPDATE_CURRENT"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 153 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x009A, index: 154, size:    10, name: Some("GAME_SMSG_AGENT_UPDATE_SCALE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 154 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x009B, index: 155, size:    74, name: Some("GAME_SMSG_AGENT_UPDATE_NPC_NAME"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 155 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x009C, index: 156, size:    10, name: Some("GAME_SMSG_AGENT_UPDATE_MORALE"), from_client: false, status: Status::Emitted, evidence: &["33 sightings, every one two dwords: an agent id then 100 (0x64) - recordings/client/feature/travel/doors-20261031-095754.jsonl seq 257, 625, 979 - in the per-player refresh block, after SKILLBAR_UPDATE and before CHARACTER_UPDATE_FACTIONS. The source reading is the per-agent morale percentage (Rurik, studies/morale/FINDINGS.md:480). A morale change is the provoked test this name waits on. This tree now sends it: crates/asura-world/src/player.rs, once per agent it creates, with 100 - the value a live load puts in every body it describes"], fields: &[Field { kind: FieldKind::MsgHeader, length: 156 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x009D, index: 157, size:    14, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 157 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x009E, index: 158, size:   323, name: Some("GAME_SMSG_AGENT_DISPLAY_DIALOG"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 158 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x009F, index: 159, size:    14, name: Some("GAME_SMSG_AGENT_PROPERTY_UPDATE_INT"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 159 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00A0, index: 160, size:    18, name: Some("GAME_SMSG_AGENT_PROPERTY_UPDATE_INT_TARGET"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 160 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00A1, index: 161, size:    20, name: Some("GAME_SMSG_AGENT_PROPERTY_PLAY_EFFECT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 161 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00A2, index: 162, size:    14, name: Some("GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 162 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00A3, index: 163, size:    18, name: Some("GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT_TARGET"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 163 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00A4, index: 164, size:    29, name: Some("GAME_SMSG_AGENT_PROJECTILE_LAUNCHED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 164 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00A5, index: 165, size:   254, name: Some("GAME_SMSG_AGENT_UPDATE_SPEECH_BUBBLE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 165 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x00A6, index: 166, size:     8, name: Some("GAME_SMSG_AGENT_UPDATE_PROFESSION"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 166 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00A7, index: 167, size:    14, name: None, from_client: false, status: Status::Unnamed, evidence: &["394 sightings, three dwords: a first value between 9 and 38, a second between 1 and 12, a third between 1 and 11 - 354 of them in movement-sniff.log (seq 282, 284, 299) and only 2 in a town capture, at roughly three per second while walking. Unnamed in every catalogue; the provoked test separates walking, standing and fighting"], fields: &[Field { kind: FieldKind::MsgHeader, length: 167 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00A8, index: 168, size:    16, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 168 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00A9, index: 169, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 169 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00AA, index: 170, size:    14, name: Some("GAME_SMSG_AGENT_CREATE_NPC"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 170 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00AB, index: 171, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 171 }] },
    Packet { conn: Conn::Game, header: 0x00AC, index: 172, size:     7, name: Some("GAME_SMSG_AGENT_UPDATE_OBSERVED_TARGET"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 172 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00AD, index: 173, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 173 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00AE, index: 174, size:    10, name: Some("GAME_SMSG_AGENT_UPDATE_MODEL"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 174 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00AF, index: 175, size:    70, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 175 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x00B0, index: 176, size:    12, name: Some("GAME_SMSG_PLAYER_UPDATE_PARTY_SIZE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeaderGame, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00B1, index: 177, size:    12, name: Some("GAME_SMSG_PLAYER_UPDATE_PARTY"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeaderGame, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00B2, index: 178, size:    90, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 178 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00B3, index: 179, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 179 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00B4, index: 180, size:    74, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 180 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x00B5, index: 181, size:   262, name: Some("GAME_SMSG_PREFERENCES_BLOB"), from_client: false, status: Status::Inferred, evidence: &["28 sightings, the payload an empty counted array (4 struct bytes) - recordings/client/feature/travel/doors-20261031-095754.jsonl seq 260, 628, 982 - while a live corpus carries the same header with a fixed 50-byte array instead. The source reading follows the handler into the client own Preferences module: the assert path P:\\Code\\Gw\\Pref\\PrApi.cpp:1431 (bytes || !data), with PREF_FLAGS, PREF_STRINGS, PREF_VALUES and PREF_WINDOWS in its neighbours, so a server pushes a session-stable options blob at area load (Rurik, studies/newopcodes/FINDINGS.md:1846-1880). What the blob holds is unverified"], fields: &[Field { kind: FieldKind::MsgHeader, length: 181 }, Field { kind: FieldKind::Array8, length: 256 }] },
    Packet { conn: Conn::Game, header: 0x00B6, index: 182, size:    10, name: Some("GAME_SMSG_PLAYER_UPDATE_UNLOCKED_PROFESSIONS"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 182 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00B7, index: 183, size:     9, name: Some("GAME_SMSG_PLAYER_UPDATE_PROFESSION"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 183 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00B8, index: 184, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 184 }] },
    Packet { conn: Conn::Game, header: 0x00B9, index: 185, size:   266, name: Some("GAME_SMSG_MISSION_INFOBOX_ADD"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 185 }, Field { kind: FieldKind::String16, length: 128 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00BA, index: 186, size:     2, name: Some("GAME_SMSG_MISSION_STREAM_START"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 186 }] },
    Packet { conn: Conn::Game, header: 0x00BB, index: 187, size:   270, name: Some("GAME_SMSG_MISSION_OBJECTIVE_ADD"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 187 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 128 }] },
    Packet { conn: Conn::Game, header: 0x00BC, index: 188, size:     6, name: Some("GAME_SMSG_MISSION_OBJECTIVE_COMPLETE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 188 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00BD, index: 189, size:   266, name: Some("GAME_SMSG_MISSION_OBJECTIVE_UPDATE_STRING"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 189 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 128 }] },
    Packet { conn: Conn::Game, header: 0x00BE, index: 190, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 190 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00BF, index: 191, size:   262, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 191 }, Field { kind: FieldKind::Array32, length: 64 }] },
    Packet { conn: Conn::Game, header: 0x00C0, index: 192, size:   534, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 192 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 128 }, Field { kind: FieldKind::String16, length: 128 }] },
    Packet { conn: Conn::Game, header: 0x00C1, index: 193, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 193 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00C2, index: 194, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 194 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00C3, index: 195, size:     7, name: Some("GAME_SMSG_MERCHANT_WINDOW_OPEN"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 195 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00C4, index: 196, size:     6, name: Some("GAME_SMSG_MERCHANT_WINDOW_UPDATE_OWNER"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 196 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00C5, index: 197, size:   251, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 197 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x00C6, index: 198, size:     7, name: Some("GAME_SMSG_TRADER_TRANSACTION_REJECT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 198 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00C7, index: 199, size:    75, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 199 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 16 }] },
    Packet { conn: Conn::Game, header: 0x00C8, index: 200, size:    11, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 200 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00C9, index: 201, size:     4, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 201 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00CA, index: 202, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 202 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00CB, index: 203, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 203 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00CC, index: 204, size:     3, name: Some("GAME_SMSG_TRADER_TRANSACTION_DONE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 204 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00CD, index: 205, size:     9, name: Some("GAME_SMSG_TRADER_WINDOW_OPEN"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 205 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00CE, index: 206, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 206 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00CF, index: 207, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &["9 sightings, two dwords: 10 then 25, 1 or 0 (doors-20261031-095754.jsonl seq 10272, 10358, 10461), in a town, after AGENT_PROPERTY_UPDATE_INT and before 0x009B or AGENT_PROPERTY_UPDATE_FLOAT_TARGET. The source reading of the 0x00CF / 0x00D0 / 0x00D2 trio is that they ride adrenaline spends (Rurik, studies/deskwork/PLAN.md:239 and studies/animref/FINDINGS.md:217); no name is proposed there or here. Test: play an adrenal skill"], fields: &[Field { kind: FieldKind::MsgHeader, length: 207 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00D0, index: 208, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &["One sighting, one dword 10 (doors-20261031-095754.jsonl seq 10931), after a direction update and a move, before ticks. Sits with 0x00CF, which the source reading ties to adrenaline spends; unnamed in every catalogue"], fields: &[Field { kind: FieldKind::MsgHeader, length: 208 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00D1, index: 209, size:    16, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 209 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00D2, index: 210, size:    12, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 210 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00D3, index: 211, size:   518, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 211 }, Field { kind: FieldKind::Array32, length: 128 }] },
    Packet { conn: Conn::Game, header: 0x00D4, index: 212, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 212 }] },
    Packet { conn: Conn::Game, header: 0x00D5, index: 213, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 213 }] },
    Packet { conn: Conn::Game, header: 0x00D6, index: 214, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 214 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00D7, index: 215, size:    38, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 215 }, Field { kind: FieldKind::Array32, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x00D8, index: 216, size:    22, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 216 }, Field { kind: FieldKind::Array32, length: 4 }] },
    Packet { conn: Conn::Game, header: 0x00D9, index: 217, size:    13, name: Some("GAME_SMSG_SKILLBAR_UPDATE_SKILL"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 217 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00DA, index: 218, size:    79, name: Some("GAME_SMSG_SKILLBAR_UPDATE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 218 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Array32, length: 8 }, Field { kind: FieldKind::Array32, length: 8 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00DB, index: 219, size:   518, name: Some("GAME_SMSG_UPDATE_UNLOCKED_SKILLS"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 219 }, Field { kind: FieldKind::Array32, length: 128 }] },
    Packet { conn: Conn::Game, header: 0x00DC, index: 220, size:     8, name: Some("GAME_SMSG_SKILL_WINDOW_COUNT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 220 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00DD, index: 221, size:     8, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 221 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00DE, index: 222, size:     4, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 222 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00DF, index: 223, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 223 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00E0, index: 224, size:    71, name: Some("GAME_SMSG_SKILL_WINDOW_DATA"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 224 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Array32, length: 16 }] },
    Packet { conn: Conn::Game, header: 0x00E1, index: 225, size:     3, name: Some("GAME_SMSG_SKILL_WINDOW_END"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 225 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00E2, index: 226, size:    12, name: Some("GAME_SMSG_SKILL_INTERUPTED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 226 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00E3, index: 227, size:    12, name: Some("GAME_SMSG_SKILL_CANCEL"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 227 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00E4, index: 228, size:    12, name: Some("GAME_SMSG_SKILL_ACTIVATE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 228 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00E5, index: 229, size:    16, name: Some("GAME_SMSG_SKILL_RECHARGE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 229 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00E6, index: 230, size:    12, name: Some("GAME_SMSG_SKILL_RECHARGED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 230 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00E7, index: 231, size:    12, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 231 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00E8, index: 232, size:    20, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 232 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00E9, index: 233, size:    62, name: Some("GAME_SMSG_CHARACTER_UPDATE_FACTIONS"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 233 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00EA, index: 234, size:     6, name: Some("GAME_SMSG_CHARACTER_FACTION_MAX_KURZICK"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 234 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00EB, index: 235, size:     6, name: Some("GAME_SMSG_CHARACTER_FACTION_MAX_LUXON"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 235 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00EC, index: 236, size:     6, name: Some("GAME_SMSG_CHARACTER_FACTION_MAX_BALTHAZAR"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 236 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00ED, index: 237, size:     6, name: Some("GAME_SMSG_CHARACTER_FACTION_MAX_IMPERIAL"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 237 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00EE, index: 238, size:    10, name: Some("GAME_SMSG_CHARACTER_FACTION_UPDATE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 238 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00EF, index: 239, size:    10, name: Some("GAME_SMSG_CHAR_VISIBILITY_FLAGS"), from_client: false, status: Status::Emitted, evidence: &["33 sightings, always the two dwords 0xFFFFFFFF and 0xFFFFFFFF, once per character refresh immediately after 0x00E9 CHARACTER_UPDATE_FACTIONS (recordings/client/feature/travel/doors-20261031-095754.jsonl seq 262 then 263). The source reading is s2c 0x00EF CHAR_VISIBILITY_FLAGS, [u32 value, u32 mask], the inventory panel per-slot display mode, answered client-to-server by 0x0057; that study also records a server that never sent it leaving the drop-down empty (Rurik, studies/deskwork/FINDINGS.md:1683-1725). This tree now sends it: crates/asura-world/src/player.rs, FFFFFFFF then FFFFFFFF right after the faction block, pinned by the_two_player_state_messages_are_the_captured_bytes"], fields: &[Field { kind: FieldKind::MsgHeader, length: 239 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00F0, index: 240, size:    10, name: Some("GAME_SMSG_AGENT_INITIAL_EFFECTS"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 240 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00F1, index: 241, size:    10, name: Some("GAME_SMSG_AGENT_UPDATE_EFFECTS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 241 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00F2, index: 242, size:     6, name: Some("GAME_SMSG_INSTANCE_LOADED"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 242 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00F3, index: 243, size:    34, name: Some("GAME_SMSG_TITLE_RANK_DATA"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 243 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x00F4, index: 244, size:     8, name: Some("GAME_SMSG_TITLE_RANK_DISPLAY"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 244 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00F5, index: 245, size:    10, name: Some("GAME_SMSG_TITLE_UPDATE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 245 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00F6, index: 246, size:    82, name: Some("GAME_SMSG_TITLE_TRACK_INFO"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 246 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::String16, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x00F7, index: 247, size:    10, name: Some("GAME_SMSG_ITEM_PRICE_QUOTE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 247 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00F8, index: 248, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 248 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00F9, index: 249, size:    70, name: Some("GAME_SMSG_ITEM_PRICES"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 249 }, Field { kind: FieldKind::Array32, length: 16 }] },
    Packet { conn: Conn::Game, header: 0x00FA, index: 250, size:   134, name: Some("GAME_SMSG_VANQUISH_PROGRESS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 250 }, Field { kind: FieldKind::Array32, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x00FB, index: 251, size:    12, name: Some("GAME_SMSG_VANQUISH_COMPLETE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 251 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00FC, index: 252, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 252 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00FD, index: 253, size:    71, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 253 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x00FE, index: 254, size:     2, name: Some("GAME_SMSG_CINEMATIC_SKIP_EVERYONE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 254 }] },
    Packet { conn: Conn::Game, header: 0x00FF, index: 255, size:     4, name: Some("GAME_SMSG_CINEMATIC_SKIP_COUNT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 255 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0100, index: 256, size:     3, name: Some("GAME_SMSG_CINEMATIC_START"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 256 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0101, index: 257, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 257 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0102, index: 258, size:   172, name: Some("GAME_SMSG_CINEMATIC_TEXT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 258 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::String16, length: 80 }] },
    Packet { conn: Conn::Game, header: 0x0103, index: 259, size:     2, name: Some("GAME_SMSG_CINEMATIC_DATA_END"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 259 }] },
    Packet { conn: Conn::Game, header: 0x0104, index: 260, size:  1030, name: Some("GAME_SMSG_CINEMATIC_DATA"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 260 }, Field { kind: FieldKind::Array8, length: 1024 }] },
    Packet { conn: Conn::Game, header: 0x0105, index: 261, size:     2, name: Some("GAME_SMSG_CINEMATIC_END"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 261 }] },
    Packet { conn: Conn::Game, header: 0x0106, index: 262, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 262 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0107, index: 263, size:    83, name: None, from_client: false, status: Status::Unnamed, evidence: &["83 bytes, and its identifier is carried into the create that follows: in every one of the 20 pairs the dword at +8 here is the dword at +16 of the next 0x0020, same instant, zero mismatches (capture of 2026-10-31 19:48). It is not sent for every create - 5 of 79 creates in one load, 3 of 82 in another, 0 of 28 in a third, 0 of 23 in a fourth - and the five it covered in one load were agents 8, 11, 13, 22, 23, not the players whose rows carry ids 271 to 425. What selects those bodies is the open question"], fields: &[Field { kind: FieldKind::MsgHeader, length: 263 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x0108, index: 264, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 264 }, Field { kind: FieldKind::AgentId, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0109, index: 265, size:   254, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 265 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x010A, index: 266, size:   257, name: Some("GAME_SMSG_SIGNPOST_BUTTON"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 266 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 122 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x010B, index: 267, size:   250, name: Some("GAME_SMSG_SIGNPOST_BODY"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 267 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x010C, index: 268, size:     6, name: Some("GAME_SMSG_SIGNPOST_SENDER"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 268 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x010D, index: 269, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 269 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x010E, index: 270, size:     9, name: Some("GAME_SMSG_MANIPULATE_MAP_OBJECT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 270 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x010F, index: 271, size:     5, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 271 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0110, index: 272, size:     8, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 272 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0111, index: 273, size:    12, name: Some("GAME_SMSG_MANIPULATE_MAP_OBJECT2"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 273 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0112, index: 274, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 274 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0113, index: 275, size:    74, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 275 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x0114, index: 276, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 276 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0115, index: 277, size:    10, name: Some("GAME_SMSG_GADGET_SET_FLAGS"), from_client: false, status: Status::Inferred, evidence: &["55 sightings in one run, always a dword agent id then a flags word, and in one load it follows the create of that same agent (agent 8 created, then 0x0115 for agent 8 with flags 0). The source reading resolves field 1 through the gadget lookup and the GW_AGENTDEF_GADGET test (Rurik, studies/smsg/FINDINGS.md:1071-1085, proposed GADGET_SET_FLAGS, held PARTIAL there). Whether every created agent gets one, or only gadgets, is what the next load will show"], fields: &[Field { kind: FieldKind::MsgHeader, length: 277 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0116, index: 278, size:    25, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 278 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0117, index: 279, size:    18, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 279 }, Field { kind: FieldKind::Blob, length: 16 }] },
    Packet { conn: Conn::Game, header: 0x0118, index: 280, size:    21, name: Some("GAME_SMSG_GUILD_PLAYER_ROLE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 280 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0119, index: 281, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 281 }] },
    Packet { conn: Conn::Game, header: 0x011A, index: 282, size:   102, name: Some("GAME_SMSG_TOWN_ALLIANCE_OBJECT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 282 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::String16, length: 6 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x011B, index: 283, size:     7, name: Some("GAME_SMSG_TOWN_ALLIANCE_COMMIT"), from_client: false, status: Status::Inferred, evidence: &["One sighting, fields a u32 and a byte: recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl seq 689. The source reading is the commit of the 0x011A town-alliance list - same container, field1 at +0x20, field2 at +0x18, a loaded flag at +0x1c, then an in-place median-of-three sort - placed after the 18th 0x011A row and before 0x011C (Rurik, studies/newopcodes/FINDINGS.md:280-300). Both field meanings are not found there"], fields: &[Field { kind: FieldKind::MsgHeader, length: 283 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x011C, index: 284, size:     2, name: Some("GAME_SMSG_TOWN_ALLIANCE_END"), from_client: false, status: Status::Inferred, evidence: &["9 sightings, 2 wire bytes, arriving right after GUILD_PLAYER_ROLE (0x0118) and 0x012E and before the party block - recordings/client/feature/travel/doors-20261031-095754.jsonl seq 314 to 316 - and 2 more in another capture, with no 0x011A row anywhere in that file. The source reading is the last step of the 0x011A block: it sets the loaded flag at container+0x5c and posts UI message 0x100000ce, after the 0x011B commit sorts the rows (Rurik, studies/newopcodes/FINDINGS.md:280-300). Which container it closes is not measured here"], fields: &[Field { kind: FieldKind::MsgHeader, length: 284 }] },
    Packet { conn: Conn::Game, header: 0x011D, index: 285, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 285 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x011E, index: 286, size:     8, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 286 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x011F, index: 287, size:   138, name: None, from_client: false, status: Status::Unnamed, evidence: &["60 sightings, 20 rows once per load, 136 bytes: mostly zero dwords, a masked name run, then 0x28, 0xF0, 0x3F, 0x3F, 2, 0x16, 0x5F (character_creation-live-necro_factions.jsonl seq 694 to 696, pre-searing-load-3.jsonl). It arrives in the guild block after GUILD_PLAYER_ROLE (0x0118) and 0x012E, before its own tail 0x012F. Unnamed in every catalogue"], fields: &[Field { kind: FieldKind::MsgHeader, length: 287 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 64 }] },
    Packet { conn: Conn::Game, header: 0x0120, index: 288, size:   119, name: Some("GAME_SMSG_GUILD_ALLIANCE_INFO"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 288 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::String16, length: 6 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0121, index: 289, size:   132, name: Some("GAME_SMSG_GUILD_GENERAL_INFO"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 289 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::String16, length: 6 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0122, index: 290, size:     9, name: Some("GAME_SMSG_GUILD_CHANGE_FACTION"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 290 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0123, index: 291, size:   155, name: Some("GAME_SMSG_GUILD_INVITE_RECEIVED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 291 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::String16, length: 6 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0124, index: 292, size:    19, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 292 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0125, index: 293, size:     8, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 293 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0126, index: 294, size:     5, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 294 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0127, index: 295, size:   283, name: Some("GAME_SMSG_GUILD_PLAYER_INFO"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 295 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::String16, length: 64 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0128, index: 296, size:    46, name: Some("GAME_SMSG_GUILD_PLAYER_REMOVE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 296 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x0129, index: 297, size:    46, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 297 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x012A, index: 298, size:    46, name: Some("GAME_SMSG_GUILD_PLAYER_CHANGE_COMPLETE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 298 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x012B, index: 299, size:   138, name: Some("GAME_SMSG_GUILD_CHANGE_PLAYER_CONTEXT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 299 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 64 }] },
    Packet { conn: Conn::Game, header: 0x012C, index: 300, size:    10, name: Some("GAME_SMSG_GUILD_CHANGE_PLAYER_STATUS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 300 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x012D, index: 301, size:     3, name: Some("GAME_SMSG_GUILD_CHANGE_PLAYER_TYPE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 301 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x012E, index: 302, size:     4, name: Some("GAME_SMSG_LOCATION_TAG"), from_client: false, status: Status::Inferred, evidence: &["12 sightings, 4 bytes: header plus one word. The source reading groups 0x012E with the map location tags drawn by GmMapCtlLocationTag.cpp and GmMapWorld.cpp, beside 0x0118, 0x0119, 0x011B and 0x01BE (Rurik, studies/minimap/FINDINGS.md:265). What the word carries is not read there or here"], fields: &[Field { kind: FieldKind::MsgHeader, length: 302 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x012F, index: 303, size:   562, name: None, from_client: false, status: Status::Unnamed, evidence: &["3 sightings, one per load, 556 bytes: zeros, a single 0xF1, then a counted string run (character_creation-live-necro_factions.jsonl seq 714), always after the twenty 0x011F rows and before GUILD_PLAYER_INFO (0x0127). The tail of the same block. Unnamed in every catalogue"], fields: &[Field { kind: FieldKind::MsgHeader, length: 303 }, Field { kind: FieldKind::String16, length: 256 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x0130, index: 304, size:     8, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 304 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0131, index: 305, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 305 }] },
    Packet { conn: Conn::Game, header: 0x0132, index: 306, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 306 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0133, index: 307, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 307 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0134, index: 308, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 308 }] },
    Packet { conn: Conn::Game, header: 0x0135, index: 309, size:    12, name: Some("GAME_SMSG_ITEM_UPDATE_OWNER"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 309 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0136, index: 310, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 310 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0137, index: 311, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 311 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0138, index: 312, size:    87, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 312 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::String16, length: 8 }, Field { kind: FieldKind::String16, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x0139, index: 313, size:    10, name: Some("GAME_SMSG_ITEM_UPDATE_QUANTITY"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 313 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x013A, index: 314, size:    74, name: Some("GAME_SMSG_ITEM_UPDATE_CUSTOMIZED_NAME"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 314 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 32 }] },
    Packet { conn: Conn::Game, header: 0x013B, index: 315, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 315 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x013C, index: 316, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 316 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x013D, index: 317, size:     9, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 317 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x013E, index: 318, size:    11, name: Some("GAME_SMSG_ITEM_MOVED_TO_LOCATION"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 318 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x013F, index: 319, size:    13, name: Some("GAME_SMSG_INVENTORY_CREATE_BAG"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 319 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0140, index: 320, size:     8, name: Some("GAME_SMSG_UPDATE_GOLD_CHARACTER"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 320 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0141, index: 321, size:     8, name: Some("GAME_SMSG_UPDATE_GOLD_STORAGE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 321 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0142, index: 322, size:     4, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 322 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0143, index: 323, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 323 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0144, index: 324, size:     5, name: Some("GAME_SMSG_ITEM_STREAM_CREATE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 324 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0145, index: 325, size:     4, name: Some("GAME_SMSG_ITEM_STREAM_DESTROY"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 325 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0146, index: 326, size:    12, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 326 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0147, index: 327, size:    13, name: Some("GAME_SMSG_ITEM_WEAPON_SET"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 327 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0148, index: 328, size:     5, name: Some("GAME_SMSG_ITEM_SET_ACTIVE_WEAPON_SET"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 328 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0149, index: 329, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 329 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x014A, index: 330, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 330 }] },
    Packet { conn: Conn::Game, header: 0x014B, index: 331, size:    11, name: Some("GAME_SMSG_ITEM_CHANGE_LOCATION"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 331 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x014C, index: 332, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 332 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x014D, index: 333, size:     8, name: Some("GAME_SMSG_ITEM_REMOVE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 333 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x014E, index: 334, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 334 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x014F, index: 335, size:     8, name: Some("GAME_SMSG_GOLD_CHARACTER_REMOVE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 335 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0150, index: 336, size:     8, name: Some("GAME_SMSG_GOLD_STORAGE_REMOVE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 336 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0151, index: 337, size:     8, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 337 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0152, index: 338, size:    12, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 338 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0153, index: 339, size:    11, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 339 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0154, index: 340, size:     8, name: Some("GAME_SMSG_TOME_SHOW_SKILLS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 340 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0155, index: 341, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 341 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0156, index: 342, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 342 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0157, index: 343, size:    18, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 343 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0158, index: 344, size:     4, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 344 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0159, index: 345, size:     8, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 345 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x015A, index: 346, size:     7, name: Some("GAME_SMSG_ITEM_SET_PROFESSION"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 346 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x015B, index: 347, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &["25 sightings, one per item, two identical dwords that are an item id (0x46d, 0x0d, 0xe3 in character_creation-live-necro_factions.jsonl seq 293, 296, 299), interleaved with the item high detail (0x0161) and 0x015A. It appears only on the character-creation screen - the group the source reading calls char-select-only (Rurik, studies/reconstruction/FINDINGS.md:2298). Unnamed in every catalogue"], fields: &[Field { kind: FieldKind::MsgHeader, length: 347 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x015C, index: 348, size:    92, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 348 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::NestedStruct, length: 7 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x015D, index: 349, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 349 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x015E, index: 350, size:    25, name: Some("GAME_SMSG_ITEM_LOW_DETAIL"), from_client: false, status: Status::Observed, evidence: &["279 sightings in one load, one per item stream row, the first field a file id and then type, flags and value words (recordings/client/feature/maps/pre-searing-load-3.jsonl seq 203-206). The client own split is the name: the shared item builder installs fields 1-9 as a low-detail record and 0x0161 installs the high-detail half on top (Rurik, studies/smsgnames/FINDINGS.md:67 and section 9, ITEM_LOW_DETAIL earned 2026-08-22)"], fields: &[Field { kind: FieldKind::MsgHeader, length: 350 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x015F, index: 351, size:    25, name: Some("GAME_SMSG_CREATE_UNNAMED_ITEM"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 351 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0160, index: 352, size:   406, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 352 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 64 }, Field { kind: FieldKind::NestedStruct, length: 64 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0161, index: 353, size:   425, name: Some("GAME_SMSG_CREATE_NAMED_ITEM"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 353 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 64 }, Field { kind: FieldKind::NestedStruct, length: 64 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0162, index: 354, size:   425, name: Some("GAME_SMSG_ITEM_UPDATE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 354 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 64 }, Field { kind: FieldKind::NestedStruct, length: 64 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0163, index: 355, size:    24, name: Some("GAME_SMSG_ITEM_SALVAGE_SESSION_START"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 355 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 3 }] },
    Packet { conn: Conn::Game, header: 0x0164, index: 356, size:     2, name: Some("GAME_SMSG_ITEM_SALVAGE_SESSION_CANCEL"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 356 }] },
    Packet { conn: Conn::Game, header: 0x0165, index: 357, size:     2, name: Some("GAME_SMSG_ITEM_SALVAGE_SESSION_DONE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 357 }] },
    Packet { conn: Conn::Game, header: 0x0166, index: 358, size:     2, name: Some("GAME_SMSG_ITEM_SALVAGE_SESSION_SUCCESS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 358 }] },
    Packet { conn: Conn::Game, header: 0x0167, index: 359, size:     3, name: Some("GAME_SMSG_ITEM_SALVAGE_SESSION_ITEM_KEPT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 359 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0168, index: 360, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &["6 sightings, two words (0x31 / 0x81, 0x26 / 0x106 in doors-20261031-095754.jsonl seq 1328, 2854), always right after an item (0x0161) and before an agent create (0x0020). The source reading puts it in the death-drop burst with 0x0135 and an item agent (Rurik, studies/presearing/CONFIRM-2026-09-30.md:218). Unnamed in every catalogue; the test is a kill whose drop the player sees"], fields: &[Field { kind: FieldKind::MsgHeader, length: 360 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0169, index: 361, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 361 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x016A, index: 362, size:    24, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 362 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Array32, length: 3 }] },
    Packet { conn: Conn::Game, header: 0x016B, index: 363, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 363 }] },
    Packet { conn: Conn::Game, header: 0x016C, index: 364, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 364 }] },
    Packet { conn: Conn::Game, header: 0x016D, index: 365, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 365 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x016E, index: 366, size:     3, name: Some("GAME_SMSG_LATENCY_ENABLE"), from_client: false, status: Status::Inferred, evidence: &["11 sightings, 3 bytes, every one a zero byte (recordings/client/feature/travel/doors-20261031-095754.jsonl seq 326, 764, 2839). The source reading is that the latency readout object is built only when this message has arrived, and a server that sent it with byte 0x04 was accepted silently and stayed healthy (Rurik, studies/smsg/FINDINGS.md:645-655). This tree sends nothing here"], fields: &[Field { kind: FieldKind::MsgHeader, length: 366 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x016F, index: 367, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 367 }] },
    Packet { conn: Conn::Game, header: 0x0170, index: 368, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 368 }] },
    Packet { conn: Conn::Game, header: 0x0171, index: 369, size:    57, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 369 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0172, index: 370, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 370 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0173, index: 371, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 371 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0174, index: 372, size:    57, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 372 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0175, index: 373, size:   141, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 373 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 64 }] },
    Packet { conn: Conn::Game, header: 0x0176, index: 374, size:     8, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 374 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0177, index: 375, size:   141, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 375 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 64 }] },
    Packet { conn: Conn::Game, header: 0x0178, index: 376, size:   183, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 376 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::String16, length: 64 }, Field { kind: FieldKind::String16, length: 6 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0179, index: 377, size:     9, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 377 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x017A, index: 378, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 378 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x017B, index: 379, size:     2, name: Some("GAME_SMSG_INSTANCE_SHOW_WIN"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 379 }] },
    Packet { conn: Conn::Game, header: 0x017C, index: 380, size:     6, name: Some("GAME_SMSG_INSTANCE_LOAD_HEAD"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 380 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x017D, index: 381, size:    46, name: Some("GAME_SMSG_INSTANCE_LOAD_PLAYER_NAME"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 381 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x017E, index: 382, size:     2, name: Some("GAME_SMSG_INSTANCE_COUNTDOWN_STOP"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 382 }] },
    Packet { conn: Conn::Game, header: 0x017F, index: 383, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 383 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0180, index: 384, size:   259, name: Some("GAME_SMSG_INSTANCE_COUNTDOWN"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 384 }, Field { kind: FieldKind::String16, length: 122 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0181, index: 385, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 385 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0182, index: 386, size:   251, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 386 }, Field { kind: FieldKind::String16, length: 122 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0183, index: 387, size:   266, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 387 }, Field { kind: FieldKind::String16, length: 128 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0184, index: 388, size:   778, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 388 }, Field { kind: FieldKind::String16, length: 128 }, Field { kind: FieldKind::String16, length: 256 }] },
    Packet { conn: Conn::Game, header: 0x0185, index: 389, size:   266, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 389 }, Field { kind: FieldKind::String16, length: 128 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0186, index: 390, size:     2, name: Some("GAME_SMSG_INSTANCE_PLAYER_DATA_START"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 390 }] },
    Packet { conn: Conn::Game, header: 0x0187, index: 391, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 391 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0188, index: 392, size:  1092, name: Some("GAME_SMSG_CHAR_CREATION_SUCCESS"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 392 }, Field { kind: FieldKind::Blob, length: 16 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Array8, length: 1024 }] },
    Packet { conn: Conn::Game, header: 0x0189, index: 393, size:     2, name: Some("GAME_SMSG_CHAR_CREATION_START"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 393 }] },
    Packet { conn: Conn::Game, header: 0x018A, index: 394, size:     2, name: Some("GAME_SMSG_INSTANCE_PLAYER_DATA_DONE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 394 }] },
    Packet { conn: Conn::Game, header: 0x018B, index: 395, size:     6, name: Some("GAME_SMSG_CHAR_CREATION_ERROR"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 395 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x018C, index: 396, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 396 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x018D, index: 397, size:     4, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 397 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x018E, index: 398, size:     2, name: Some("GAME_SMSG_INSTANCE_LOAD_FINISH"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 398 }] },
    Packet { conn: Conn::Game, header: 0x018F, index: 399, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 399 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0190, index: 400, size:     7, name: Some("GAME_SMSG_JUMBO_MESSAGE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 400 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0191, index: 401, size:     4, name: Some("GAME_SMSG_INSTANCE_REDIRECT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 401 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0192, index: 402, size:   499, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 402 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 64 }, Field { kind: FieldKind::NestedStruct, length: 8 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x0193, index: 403, size:     5, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 403 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0194, index: 404, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &["11 sightings, one dword that is always 0, right after the latency enable (0x016E) and INSTANCE_LOAD_FINISH (0x018E) - doors-20261031-095754.jsonl seq 328, 766, 2841. The load-finish group. Unnamed in every catalogue"], fields: &[Field { kind: FieldKind::MsgHeader, length: 404 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0195, index: 405, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 405 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0196, index: 406, size:    26, name: Some("GAME_SMSG_INSTANCE_LOAD_SPAWN_POINT"), from_client: false, status: Status::Emitted, evidence: &["the first dword is constant across the loads of one session (0x1bb1d) and the vec2 after it is the position the character is placed at, different per instance: 166 at (0x46ac6966, 0x46321b26) and 162 at (0x46b0ac00, 0x45d60800) in the capture of 2026-10-31 19:48. This world sends one fixed spawn per instance and has neither the random entry points nor the arriving-from-an-explorable set"], fields: &[Field { kind: FieldKind::MsgHeader, length: 406 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Blob, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x0197, index: 407, size:  1030, name: Some("GAME_SMSG_INSTANCE_MANIFEST_DATA"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 407 }, Field { kind: FieldKind::Array8, length: 1024 }] },
    Packet { conn: Conn::Game, header: 0x0198, index: 408, size:     9, name: Some("GAME_SMSG_INSTANCE_MANIFEST_DONE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 408 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x0199, index: 409, size:     3, name: Some("GAME_SMSG_INSTANCE_MANIFEST_PHASE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 409 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x019A, index: 410, size:    16, name: Some("GAME_SMSG_INSTANCE_LOAD_INFO"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 410 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x019B, index: 411, size:    10, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 411 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x019C, index: 412, size:     4, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 412 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x019D, index: 413, size:   250, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 413 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x019E, index: 414, size:   237, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 414 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::NestedStruct, length: 2 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 48 }] },
    Packet { conn: Conn::Game, header: 0x019F, index: 415, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 415 }] },
    Packet { conn: Conn::Game, header: 0x01A0, index: 416, size:   102, name: Some("GAME_SMSG_INSTANCE_TABLE"), from_client: false, status: Status::Emitted, evidence: &["two batches in the first load of one session and none in the four loads after it (capture of 2026-10-31 19:48), the opposite of this tree, which sends ten batches on every load. Ten counted batches of u16 instance and u32 value pairs in the load block (docs/21-surrounding-packets.md, The map table; crates/asura-protocol/src/game/instance_table.rs; docs/22-world-model.md:230), and a client accepts them"], fields: &[Field { kind: FieldKind::MsgHeader, length: 416 }, Field { kind: FieldKind::NestedStruct, length: 16 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01A1, index: 417, size:   516, name: Some("GAME_SMSG_CREATE_MISSION_PROGRESS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 417 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 122 }, Field { kind: FieldKind::String16, length: 122 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01A2, index: 418, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 418 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01A3, index: 419, size:    20, name: Some("GAME_SMSG_UPDATE_MISSION_PROGRESS"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 419 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01A4, index: 420, size:   499, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 420 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 122 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x01A5, index: 421, size:    83, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 421 }, Field { kind: FieldKind::AgentId, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Vect2, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::Blob, length: 8 }] },
    Packet { conn: Conn::Game, header: 0x01A6, index: 422, size:    46, name: Some("GAME_SMSG_TRANSFER_GAME_SERVER_INFO"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 422 }, Field { kind: FieldKind::Blob, length: 24 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01A7, index: 423, size:   514, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 423 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 122 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 122 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01A8, index: 424, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 424 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01A9, index: 425, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 425 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01AA, index: 426, size:     7, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 426 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01AB, index: 427, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 427 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01AC, index: 428, size:     6, name: Some("GAME_SMSG_READY_FOR_MAP_SPAWN"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 428 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01AD, index: 429, size:    56, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 429 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x01AE, index: 430, size:    55, name: None, from_client: false, status: Status::Unnamed, evidence: &["21 sightings, 55 bytes: a leading dword 1 then a masked player name, right after the client asks for players (0x8090) - doors-20261031-095754.jsonl seq 242, character_creation-live-necro_factions.jsonl seq 601, 1039. It answers that request with one row, so it is a player or party roster entry; unnamed in every catalogue"], fields: &[Field { kind: FieldKind::MsgHeader, length: 430 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 20 }] },
    Packet { conn: Conn::Game, header: 0x01AF, index: 431, size:   104, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 431 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::String16, length: 6 }] },
    Packet { conn: Conn::Game, header: 0x01B0, index: 432, size:   250, name: Some("GAME_SMSG_DOA_COMPLETE_ZONE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 432 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x01B1, index: 433, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 433 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01B2, index: 434, size:     9, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 434 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01B3, index: 435, size:     5, name: Some("GAME_SMSG_CHARACTER_UDPATE_PARTY"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 435 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01B4, index: 436, size:    81, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 436 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01B5, index: 437, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 437 }] },
    Packet { conn: Conn::Game, header: 0x01B6, index: 438, size:    81, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 438 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01B7, index: 439, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 439 }] },
    Packet { conn: Conn::Game, header: 0x01B8, index: 440, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 440 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01B9, index: 441, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 441 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01BA, index: 442, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 442 }] },
    Packet { conn: Conn::Game, header: 0x01BB, index: 443, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 443 }] },
    Packet { conn: Conn::Game, header: 0x01BC, index: 444, size:   253, name: Some("GAME_SMSG_INSTANCE_TRAVEL_TIMER"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 444 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x01BD, index: 445, size:     3, name: Some("GAME_SMSG_INSTANCE_TRAVEL_FAILURE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 445 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01BE, index: 446, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &["11 sightings, one dword that is always 0, after CHARACTER_UPDATE_PARTY (0x01B3) and the 0x01BF party message and before MISSION_STREAM_START (0x00BA) - doors-20261031-095754.jsonl seq 324, 762, 2837. The source table numbers a henchman add at 0x01BF, which is one above this header, so the family here is the party roster and the numbering between the two tables is not settled. Unnamed in every catalogue"], fields: &[Field { kind: FieldKind::MsgHeader, length: 446 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01BF, index: 447, size:     3, name: Some("GAME_SMSG_PARTY_SET_DIFFICULTY"), from_client: false, status: Status::Observed, evidence: &["the value is the difficulty and it is one dword: 1 at t39.3, 0 at t41.2, 1 at t42.0, 0 at t42.9 while the party panel toggled, then 0 re-sent at every load of the session (t48.9, 123.6, 167.2, 362.7). worldcontext.is_hard_mode_unlocked follows it in the dumps."], fields: &[Field { kind: FieldKind::MsgHeader, length: 447 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01C0, index: 448, size:    52, name: Some("GAME_SMSG_PARTY_HENCHMAN_ADD"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 448 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01C1, index: 449, size:     6, name: Some("GAME_SMSG_PARTY_HENCHMAN_REMOVE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 449 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01C2, index: 450, size:     4, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 450 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01C3, index: 451, size:    10, name: Some("GAME_SMSG_PARTY_HERO_ADD"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 451 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01C4, index: 452, size:     8, name: Some("GAME_SMSG_PARTY_HERO_REMOVE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 452 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01C5, index: 453, size:     4, name: Some("GAME_SMSG_PARTY_INVITE_ADD"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 453 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01C6, index: 454, size:     4, name: Some("GAME_SMSG_PARTY_JOIN_REQUEST"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 454 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01C7, index: 455, size:     4, name: Some("GAME_SMSG_PARTY_INVITE_CANCEL"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 455 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01C8, index: 456, size:     4, name: Some("GAME_SMSG_PARTY_REQUEST_CANCEL"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 456 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01C9, index: 457, size:     4, name: Some("GAME_SMSG_PARTY_REQUEST_RESPONSE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 457 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01CA, index: 458, size:     4, name: Some("GAME_SMSG_PARTY_INVITE_RESPONSE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 458 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01CB, index: 459, size:     3, name: Some("GAME_SMSG_PARTY_YOU_ARE_LEADER"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 459 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01CC, index: 460, size:     7, name: Some("GAME_SMSG_PARTY_PLAYER_ADD"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 460 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01CD, index: 461, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 461 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01CE, index: 462, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 462 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01CF, index: 463, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 463 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01D0, index: 464, size:     6, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 464 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01D1, index: 465, size:     6, name: Some("GAME_SMSG_PARTY_PLAYER_REMOVE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 465 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01D2, index: 466, size:     7, name: Some("GAME_SMSG_PARTY_PLAYER_READY"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 466 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01D3, index: 467, size:     4, name: Some("GAME_SMSG_PARTY_CREATE"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 467 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01D4, index: 468, size:     4, name: Some("GAME_SMSG_PARTY_MEMBER_STREAM_END"), from_client: false, status: Status::Emitted, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 468 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01D5, index: 469, size:   250, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 469 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x01D6, index: 470, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 470 }] },
    Packet { conn: Conn::Game, header: 0x01D7, index: 471, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 471 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01D8, index: 472, size:   234, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 472 }, Field { kind: FieldKind::Array16, length: 16 }, Field { kind: FieldKind::NestedStruct, length: 16 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01D9, index: 473, size:     2, name: Some("GAME_SMSG_PARTY_DEFEATED"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 473 }] },
    Packet { conn: Conn::Game, header: 0x01DA, index: 474, size:   252, name: Some("GAME_SMSG_PARTY_LOCK"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 474 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x01DB, index: 475, size:     2, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 475 }] },
    Packet { conn: Conn::Game, header: 0x01DC, index: 476, size:     5, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 476 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01DD, index: 477, size:     6, name: Some("GAME_SMSG_PARTY_SEARCH_REQUEST_JOIN"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 477 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01DE, index: 478, size:     6, name: Some("GAME_SMSG_PARTY_SEARCH_REQUEST_DONE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 478 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01DF, index: 479, size:   130, name: Some("GAME_SMSG_PARTY_SEARCH_ADVERTISEMENT"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 479 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::String16, length: 32 }, Field { kind: FieldKind::String16, length: 20 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Dword, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01E0, index: 480, size:     4, name: Some("GAME_SMSG_PARTY_SEARCH_SEEK"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 480 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01E1, index: 481, size:     4, name: Some("GAME_SMSG_PARTY_SEARCH_REMOVE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 481 }, Field { kind: FieldKind::Word, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01E2, index: 482, size:     6, name: Some("GAME_SMSG_PARTY_SEARCH_SIZE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 482 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01E3, index: 483, size:     6, name: Some("GAME_SMSG_PARTY_SEARCH_TYPE"), from_client: false, status: Status::Known, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 483 }, Field { kind: FieldKind::Word, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01E4, index: 484, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 484 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01E5, index: 485, size:   250, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 485 }, Field { kind: FieldKind::String16, length: 122 }] },
    Packet { conn: Conn::Game, header: 0x01E6, index: 486, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 486 }, Field { kind: FieldKind::Byte, length: 0 }] },
    Packet { conn: Conn::Game, header: 0x01E7, index: 487, size:     3, name: None, from_client: false, status: Status::Unnamed, evidence: &[], fields: &[Field { kind: FieldKind::MsgHeader, length: 487 }, Field { kind: FieldKind::Byte, length: 0 }] },
];

/// The row for one header in one direction of one family, or `None` if that family has no such
/// message. A binary search over [`PACKETS`]: framing asks this for every message.
pub fn lookup(conn: Conn, header: u16, from_client: bool) -> Option<&'static Packet> {
    let key = key_of(conn, header, from_client);
    PACKETS
        .binary_search_by(|packet| key_of(packet.conn, packet.header, packet.from_client).cmp(&key))
        .ok()
        .map(|index| &PACKETS[index])
}

/// The binary-search key: family, then client-to-server before server-to-client, then header.
fn key_of(conn: Conn, header: u16, from_client: bool) -> (u8, u8, u16) {
    (conn as u8, u8::from(!from_client), header)
}

/// Every row of one family, in table order.
pub fn rows(conn: Conn) -> impl Iterator<Item = &'static Packet> {
    PACKETS.iter().filter(move |packet| packet.conn == conn)
}

/// How many rows of one family are in each status. The progress number: "how much of the packet
/// stream is decoded", counted rather than narrated.
pub fn progress(conn: Conn) -> [(Status, usize); 6] {
    let mut counts = [
        (Status::Unnamed, 0usize),
        (Status::Known, 0),
        (Status::Observed, 0),
        (Status::Emitted, 0),
        (Status::Handled, 0),
        (Status::Inferred, 0),
    ];
    for packet in rows(conn) {
        if let Some(slot) = counts
            .iter_mut()
            .find(|(status, _)| *status == packet.status)
        {
            slot.1 += 1;
        }
    }
    counts
}

/// The reference's `max_size` arithmetic: the field sum with every counted field at full capacity
/// behind a four-byte count, and a nested struct at `length` repeats.
///
/// Not a wire size: this is what a declared maximum is computed from, and it exists so a row's
/// `size` can be checked against its own layout instead of trusted.
pub fn declared_max_size(fields: &[Field]) -> usize {
    let mut total = 0;
    for (position, field) in fields.iter().enumerate() {
        let n = field.length as usize;
        total += match field.kind {
            FieldKind::MsgHeader => 2,
            FieldKind::MsgHeaderGame => 4,
            FieldKind::AgentId | FieldKind::Float | FieldKind::Dword => 4,
            FieldKind::Vect2 => 8,
            FieldKind::Vect3 => 12,
            FieldKind::Byte => 1,
            FieldKind::Word => 2,
            FieldKind::Blob => n,
            FieldKind::String16 | FieldKind::Array16 => 4 + n * 2,
            FieldKind::Array8 => 4 + n,
            FieldKind::Array32 => 4 + n * 4,
            FieldKind::NestedStruct => 4 + n * declared_max_size(&fields[position + 1..]),
        };
        if field.kind == FieldKind::NestedStruct {
            break;
        }
    }
    total
}

/// The auth family's accessor surface: the masks, the counts, the named headers, and the two
/// lookups `client::frame` and `client::messages` use. Everything reads [`PACKETS`].
pub mod auth {
    use super::{lookup, Conn};

    pub use super::Packet as Entry;
    pub use super::{Field, FieldKind};

    /// `AUTH_CMSG_MASK`: set on client-to-server headers, and therefore also the direction bit.
    pub const AUTH_CMSG_MASK: u16 = 0x8000;
    /// `AUTH_SMSG_MASK`: server-to-client headers carry no direction bit.
    pub const AUTH_SMSG_MASK: u16 = 0x0000;

    /// Number of client-to-server auth headers (`AUTH_CMSG_COUNT`).
    pub const AUTH_CMSG_COUNT: usize = 60;
    /// Number of server-to-client auth headers (`AUTH_SMSG_COUNT`).
    pub const AUTH_SMSG_COUNT: usize = 40;

    // ---------------------------------------------------------------- named headers
    // The names are GWPS's own (`opcodes.h`), kept verbatim so a Rust message can be diffed
    // against a C one by name rather than by number.
    //
    // A constant is an alias of a row's `header`, not a second declaration of it: the value is
    // copied here once because `match` needs a constant, and `every_header_agrees_with_its_index`
    // checks that the two columns cannot drift.

    /// `AUTH_CMSG_HEARTBEAT` (6 bytes on the wire).
    pub const AUTH_CMSG_HEARTBEAT: u16 = 0x8000;

    /// `AUTH_CMSG_SEND_COMPUTER_INFO` (138 bytes on the wire).
    pub const AUTH_CMSG_SEND_COMPUTER_INFO: u16 = 0x8001;

    /// `AUTH_CMSG_SEND_COMPUTER_HASH` (22 bytes on the wire).
    pub const AUTH_CMSG_SEND_COMPUTER_HASH: u16 = 0x8002;

    /// `AUTH_CMSG_ACCOUNT_CREATE` (214 bytes on the wire).
    pub const AUTH_CMSG_ACCOUNT_CREATE: u16 = 0x8003;

    /// `AUTH_CMSG_ACCOUNT_LOGIN` (6 bytes on the wire).
    pub const AUTH_CMSG_ACCOUNT_LOGIN: u16 = 0x8004;

    /// `AUTH_CMSG_DELETE_CHARACTER` (50 bytes on the wire).
    pub const AUTH_CMSG_DELETE_CHARACTER: u16 = 0x8007;

    /// `AUTH_CMSG_UPDATE_CHARACTER_SETTINGS` (118 bytes on the wire).
    pub const AUTH_CMSG_UPDATE_CHARACTER_SETTINGS: u16 = 0x8009;

    /// `AUTH_CMSG_CHANGE_PLAY_CHARACTER` (50 bytes on the wire).
    pub const AUTH_CMSG_CHANGE_PLAY_CHARACTER: u16 = 0x800A;

    /// `AUTH_CMSG_DISCONNECT` (6 bytes on the wire).
    pub const AUTH_CMSG_DISCONNECT: u16 = 0x800D;

    /// `AUTH_CMSG_SET_PLAYER_STATUS` (6 bytes on the wire).
    pub const AUTH_CMSG_SET_PLAYER_STATUS: u16 = 0x800E;

    /// `AUTH_CMSG_FRIEND_ADD` (98 bytes on the wire).
    pub const AUTH_CMSG_FRIEND_ADD: u16 = 0x801A;

    /// `AUTH_CMSG_ADD_ACCESS_KEY` (62 bytes on the wire).
    pub const AUTH_CMSG_ADD_ACCESS_KEY: u16 = 0x801C;

    /// `AUTH_CMSG_SETTING_UPDATE_CONTENT` (522 bytes on the wire).
    pub const AUTH_CMSG_SETTING_UPDATE_CONTENT: u16 = 0x8020;

    /// `AUTH_CMSG_SETTING_UPDATE_SIZE` (10 bytes on the wire).
    pub const AUTH_CMSG_SETTING_UPDATE_SIZE: u16 = 0x8021;

    /// `AUTH_CMSG_REQUEST_GUILD_HALL` (38 bytes on the wire).
    pub const AUTH_CMSG_REQUEST_GUILD_HALL: u16 = 0x8025;

    /// `AUTH_CMSG_ACCEPT_EULA` (3 bytes on the wire).
    pub const AUTH_CMSG_ACCEPT_EULA: u16 = 0x8026;

    /// `AUTH_CMSG_REQUEST_GAME_INSTANCE` (26 bytes on the wire).
    pub const AUTH_CMSG_REQUEST_GAME_INSTANCE: u16 = 0x8029;

    /// `AUTH_CMSG_ASK_SERVER_RESPONSE` (6 bytes on the wire).
    pub const AUTH_CMSG_ASK_SERVER_RESPONSE: u16 = 0x8035;

    /// `AUTH_CMSG_PORTAL_ACCOUNT_LOGIN` (150 bytes on the wire).
    pub const AUTH_CMSG_PORTAL_ACCOUNT_LOGIN: u16 = 0x8038;

    /// `AUTH_SMSG_HEARTBEAT` (6 bytes on the wire).
    pub const AUTH_SMSG_HEARTBEAT: u16 = 0x0000;

    /// `AUTH_SMSG_SESSION_INFO` (10 bytes on the wire).
    pub const AUTH_SMSG_SESSION_INFO: u16 = 0x0001;

    /// `AUTH_SMSG_REQUEST_RESPONSE` (10 bytes on the wire).
    pub const AUTH_SMSG_REQUEST_RESPONSE: u16 = 0x0003;

    /// `AUTH_SMSG_CHARACTER_INFO` (138 bytes on the wire).
    pub const AUTH_SMSG_CHARACTER_INFO: u16 = 0x0007;

    /// `AUTH_SMSG_GAME_SERVER_INFO` (42 bytes on the wire).
    pub const AUTH_SMSG_GAME_SERVER_INFO: u16 = 0x0009;

    /// `AUTH_SMSG_FRIEND_UPDATE_INFO` (70 bytes on the wire).
    pub const AUTH_SMSG_FRIEND_UPDATE_INFO: u16 = 0x000A;

    /// `AUTH_SMSG_FRIEND_UPDATE_STATUS` (110 bytes on the wire).
    pub const AUTH_SMSG_FRIEND_UPDATE_STATUS: u16 = 0x000B;

    /// `AUTH_SMSG_WHISPER_RECEIVED` (566 bytes on the wire).
    pub const AUTH_SMSG_WHISPER_RECEIVED: u16 = 0x000C;

    /// `AUTH_SMSG_ACCOUNT_INFO` (275 bytes on the wire).
    pub const AUTH_SMSG_ACCOUNT_INFO: u16 = 0x0011;

    /// `AUTH_SMSG_FRIEND_STREAM_END` (10 bytes on the wire).
    pub const AUTH_SMSG_FRIEND_STREAM_END: u16 = 0x0014;

    /// `AUTH_SMSG_ACCOUNT_SETTINGS` (2058 bytes on the wire).
    pub const AUTH_SMSG_ACCOUNT_SETTINGS: u16 = 0x0016;

    /// `AUTH_SMSG_FRIEND_UPDATE_LOCATION` (54 bytes on the wire).
    pub const AUTH_SMSG_FRIEND_UPDATE_LOCATION: u16 = 0x0020;

    /// `AUTH_SMSG_SERVER_RESPONSE` (10 bytes on the wire).
    pub const AUTH_SMSG_SERVER_RESPONSE: u16 = 0x0026;

    /// Headers whose declared size cannot be recomputed from their field layout, because a
    /// `TYPE_NESTED_STRUCT` makes the length depend on a count carried on the wire.
    ///
    /// One of them, `0x001A`, declares 32 bytes where its own layout can only occupy 31: the
    /// reference's maximum is not exactly its packer's output. It is recorded rather than guessed
    /// at (`docs/03-protocol.md`, open questions).
    pub const VARIABLE_LENGTH_HEADERS: &[u16] = &[0x001A];

    /// Every auth row, in table order.
    pub fn rows() -> impl Iterator<Item = &'static Entry> {
        super::rows(Conn::Auth)
    }

    /// The row for an auth header, whichever direction. The family's two directions use disjoint
    /// header ranges, so the header alone identifies it.
    pub fn entry(header: u16) -> Option<&'static Entry> {
        lookup(Conn::Auth, header, header & AUTH_CMSG_MASK != 0)
    }

    /// The fixed on-wire size for a header, or `None` if it is not an auth header at all.
    pub fn expected_size(header: u16) -> Option<usize> {
        entry(header).map(|entry| entry.size)
    }
}

/// The game family's accessor surface: the masks, the counts, the named headers, and the lookups a
/// world server's framing and dispatch use. Everything reads [`PACKETS`].
pub mod game {
    use super::{lookup as packet_lookup, Conn};

    pub use super::Packet as Entry;
    pub use super::{declared_max_size, Field, FieldKind};

    /// `GAME_CMSG_MASK`: set on client-to-server headers, and therefore also the direction bit.
    pub const GAME_CMSG_MASK: u16 = 0x8000;
    /// `GAME_SMSG_MASK`: server-to-client headers carry no direction bit.
    pub const GAME_SMSG_MASK: u16 = 0x0000;

    /// Number of client-to-server game headers (`GAME_CMSG_COUNT`).
    pub const GAME_CMSG_COUNT: usize = 194;
    /// Number of server-to-client game headers (`GAME_SMSG_COUNT`).
    pub const GAME_SMSG_COUNT: usize = 488;

    /// Headers `opcodes.h` gives more than one name: `(header, from_client, the name this table
    /// uses, the other name)`.
    ///
    /// A collision in the reference, not in this table: one header value, two message names.
    /// Nothing distinguishes them on the wire, so the row keeps the first name, the alias
    /// constant keeps the second, and a decoder has to decide from the body which message it has.
    pub const AMBIGUOUS_NAME_HEADERS: &[(u16, bool, &str, &str)] = &[(
        0x00E3,
        false,
        "GAME_SMSG_SKILL_CANCEL",
        "GAME_SMSG_SKILL_ACTIVATED",
    )];

    /// Rows whose `size` does not follow from their own field layout: 2 of 682.
    ///
    /// The reference wrote these two rows with a two-byte array count where all 680 other rows use
    /// a four-byte one, so each declares two bytes less per counted field than
    /// [`declared_max_size`] recomputes. Both numbers are carried through as declared -- the
    /// reference is the authority for a message's declared maximum, and `max_size` is never the
    /// wire size in any case, so the difference is recorded rather than guessed at.
    pub const SIZE_NOT_RECOMPUTABLE_FROM_LAYOUT: &[(u16, bool)] =
        &[(0x005F, false), (0x0060, false)];

    // ---------------------------------------------------------------- named headers
    // The names are GWPS's own (`opcodes.h`), kept verbatim so a Rust message can be diffed
    // against a C one by name rather than by number. 340 of the 682 headers have one.
    //
    // A constant is an alias of a row's `header`, not a second declaration of it; see
    // `every_header_agrees_with_its_index`.

    /// `GAME_CMSG_TRADE_ACKNOWLEDGE` (declared maximum 6 bytes; header `0x8000`).
    pub const GAME_CMSG_TRADE_ACKNOWLEDGE: u16 = 0x8000;

    /// `GAME_CMSG_TRADE_CANCEL` (declared maximum 2 bytes; header `0x8001`).
    pub const GAME_CMSG_TRADE_CANCEL: u16 = 0x8001;

    /// `GAME_CMSG_TRADE_ADD_ITEM` (declared maximum 7 bytes; header `0x8002`).
    pub const GAME_CMSG_TRADE_ADD_ITEM: u16 = 0x8002;

    /// `GAME_CMSG_TRADE_SEND_OFFER` (declared maximum 6 bytes; header `0x8003`).
    pub const GAME_CMSG_TRADE_SEND_OFFER: u16 = 0x8003;

    /// `GAME_CMSG_TRADE_REMOVE_ITEM` (declared maximum 7 bytes; header `0x8005`).
    pub const GAME_CMSG_TRADE_REMOVE_ITEM: u16 = 0x8005;

    /// `GAME_CMSG_TRADE_CANCEL_OFFER` (declared maximum 2 bytes; header `0x8006`).
    pub const GAME_CMSG_TRADE_CANCEL_OFFER: u16 = 0x8006;

    /// `GAME_CMSG_TRADE_ACCEPT` (declared maximum 2 bytes; header `0x8007`).
    pub const GAME_CMSG_TRADE_ACCEPT: u16 = 0x8007;

    /// `GAME_CMSG_DISCONNECT` (declared maximum 2 bytes; header `0x8008`).
    pub const GAME_CMSG_DISCONNECT: u16 = 0x8008;

    /// `GAME_CMSG_PING_REPLY` (declared maximum 10 bytes; header `0x8009`).
    pub const GAME_CMSG_PING_REPLY: u16 = 0x8009;

    /// `GAME_CMSG_HEARTBEAT` (declared maximum 2 bytes; header `0x800C`).
    pub const GAME_CMSG_HEARTBEAT: u16 = 0x800C;

    /// `GAME_CMSG_PING_REQUEST` (declared maximum 2 bytes; header `0x800D`).
    pub const GAME_CMSG_PING_REQUEST: u16 = 0x800D;

    /// `GAME_CMSG_ATTRIBUTE_DECREASE` (declared maximum 14 bytes; header `0x800E`).
    pub const GAME_CMSG_ATTRIBUTE_DECREASE: u16 = 0x800E;

    /// `GAME_CMSG_ATTRIBUTE_INCREASE` (declared maximum 14 bytes; header `0x800F`).
    pub const GAME_CMSG_ATTRIBUTE_INCREASE: u16 = 0x800F;

    /// `GAME_CMSG_ATTRIBUTE_LOAD` (declared maximum 142 bytes; header `0x8010`).
    pub const GAME_CMSG_ATTRIBUTE_LOAD: u16 = 0x8010;

    /// `GAME_CMSG_QUEST_ABANDON` (declared maximum 6 bytes; header `0x8011`).
    pub const GAME_CMSG_QUEST_ABANDON: u16 = 0x8011;

    /// `GAME_CMSG_QUEST_REQUEST_INFOS` (declared maximum 6 bytes; header `0x8012`).
    pub const GAME_CMSG_QUEST_REQUEST_INFOS: u16 = 0x8012;

    /// `GAME_CMSG_HERO_BEHAVIOR` (declared maximum 10 bytes; header `0x8015`).
    pub const GAME_CMSG_HERO_BEHAVIOR: u16 = 0x8015;

    /// `GAME_CMSG_HERO_LOCK_TARGET` (declared maximum 10 bytes; header `0x8016`).
    pub const GAME_CMSG_HERO_LOCK_TARGET: u16 = 0x8016;

    /// `GAME_CMSG_HERO_SKILL_TOGGLE` (declared maximum 10 bytes; header `0x8019`).
    pub const GAME_CMSG_HERO_SKILL_TOGGLE: u16 = 0x8019;

    /// `GAME_CMSG_HERO_FLAG_SINGLE` (declared maximum 18 bytes; header `0x801A`).
    pub const GAME_CMSG_HERO_FLAG_SINGLE: u16 = 0x801A;

    /// `GAME_CMSG_HERO_FLAG_ALL` (declared maximum 14 bytes; header `0x801B`).
    pub const GAME_CMSG_HERO_FLAG_ALL: u16 = 0x801B;

    /// `GAME_CMSG_HERO_USE_SKILL` (declared maximum 18 bytes; header `0x801C`).
    pub const GAME_CMSG_HERO_USE_SKILL: u16 = 0x801C;

    /// `GAME_CMSG_HERO_ADD` (declared maximum 4 bytes; header `0x801E`).
    pub const GAME_CMSG_HERO_ADD: u16 = 0x801E;

    /// `GAME_CMSG_HERO_KICK` (declared maximum 4 bytes; header `0x801F`).
    pub const GAME_CMSG_HERO_KICK: u16 = 0x801F;

    /// `GAME_CMSG_TARGET_CALL` (declared maximum 7 bytes; header `0x8023`).
    pub const GAME_CMSG_TARGET_CALL: u16 = 0x8023;

    /// `GAME_CMSG_PING_WEAPON_SET` (declared maximum 14 bytes; header `0x8025`).
    pub const GAME_CMSG_PING_WEAPON_SET: u16 = 0x8025;

    /// `GAME_CMSG_ATTACK_AGENT` (declared maximum 7 bytes; header `0x8026`).
    pub const GAME_CMSG_ATTACK_AGENT: u16 = 0x8026;

    /// `GAME_CMSG_CANCEL_MOVEMENT` (declared maximum 2 bytes; header `0x8028`).
    pub const GAME_CMSG_CANCEL_MOVEMENT: u16 = 0x8028;

    /// `GAME_CMSG_DROP_BUFF` (declared maximum 6 bytes; header `0x8029`).
    pub const GAME_CMSG_DROP_BUFF: u16 = 0x8029;

    /// `GAME_CMSG_DRAW_MAP` (declared maximum 74 bytes; header `0x802B`).
    pub const GAME_CMSG_DRAW_MAP: u16 = 0x802B;

    /// `GAME_CMSG_DROP_ITEM` (declared maximum 7 bytes; header `0x802C`).
    pub const GAME_CMSG_DROP_ITEM: u16 = 0x802C;

    /// `GAME_CMSG_DROP_GOLD` (declared maximum 6 bytes; header `0x802F`).
    pub const GAME_CMSG_DROP_GOLD: u16 = 0x802F;

    /// `GAME_CMSG_EQUIP_ITEM` (declared maximum 6 bytes; header `0x8030`).
    pub const GAME_CMSG_EQUIP_ITEM: u16 = 0x8030;

    /// `GAME_CMSG_INTERACT_PLAYER` (declared maximum 6 bytes; header `0x8033`).
    pub const GAME_CMSG_INTERACT_PLAYER: u16 = 0x8033;

    /// `GAME_CMSG_DEPOSIT_FACTION` (declared maximum 11 bytes; header `0x8035`).
    pub const GAME_CMSG_DEPOSIT_FACTION: u16 = 0x8035;

    /// `GAME_CMSG_INTERACT_LIVING` (declared maximum 7 bytes; header `0x8039`).
    pub const GAME_CMSG_INTERACT_LIVING: u16 = 0x8039;

    /// `GAME_CMSG_SEND_DIALOG` (declared maximum 6 bytes; header `0x803B`).
    pub const GAME_CMSG_SEND_DIALOG: u16 = 0x803B;

    /// `GAME_CMSG_POSITION_UPDATE` (declared maximum 26 bytes; header `0x803D`).
    pub const GAME_CMSG_POSITION_UPDATE: u16 = 0x803D;

    /// `GAME_CMSG_MOVE_TO_COORD` (declared maximum 14 bytes; header `0x803E`).
    pub const GAME_CMSG_MOVE_TO_COORD: u16 = 0x803E;

    /// `GAME_CMSG_INTERACT_ITEM` (declared maximum 7 bytes; header `0x803F`).
    pub const GAME_CMSG_INTERACT_ITEM: u16 = 0x803F;

    /// `GAME_CMSG_ROTATE_PLAYER` (declared maximum 10 bytes; header `0x8040`).
    pub const GAME_CMSG_ROTATE_PLAYER: u16 = 0x8040;

    /// `GAME_CMSG_CHANGE_SECOND_PROFESSION` (declared maximum 7 bytes; header `0x8041`).
    pub const GAME_CMSG_CHANGE_SECOND_PROFESSION: u16 = 0x8041;

    /// `GAME_CMSG_USE_SKILL` (declared maximum 15 bytes; header `0x8046`).
    pub const GAME_CMSG_USE_SKILL: u16 = 0x8046;

    /// `GAME_CMSG_LAST_POS_BEFORE_MOVE_CANCELED` (declared maximum 14 bytes; header `0x8047`).
    pub const GAME_CMSG_LAST_POS_BEFORE_MOVE_CANCELED: u16 = 0x8047;

    /// `GAME_CMSG_TRADE_INITIATE` (declared maximum 6 bytes; header `0x8049`).
    pub const GAME_CMSG_TRADE_INITIATE: u16 = 0x8049;

    /// `GAME_CMSG_BUY_MATERIALS` (declared maximum 147 bytes; header `0x804A`).
    pub const GAME_CMSG_BUY_MATERIALS: u16 = 0x804A;

    /// `GAME_CMSG_REQUEST_QUOTE` (declared maximum 148 bytes; header `0x804C`).
    pub const GAME_CMSG_REQUEST_QUOTE: u16 = 0x804C;

    /// `GAME_CMSG_TRANSACT_ITEMS` (declared maximum 187 bytes; header `0x804D`).
    pub const GAME_CMSG_TRANSACT_ITEMS: u16 = 0x804D;

    /// `GAME_CMSG_UNEQUIP_ITEM` (declared maximum 6 bytes; header `0x804F`).
    pub const GAME_CMSG_UNEQUIP_ITEM: u16 = 0x804F;

    /// `GAME_CMSG_INTERACT_GADGET` (declared maximum 7 bytes; header `0x8051`).
    pub const GAME_CMSG_INTERACT_GADGET: u16 = 0x8051;

    /// `GAME_CMSG_OPEN_CHEST` (declared maximum 6 bytes; header `0x8053`).
    pub const GAME_CMSG_OPEN_CHEST: u16 = 0x8053;

    /// `GAME_CMSG_EQUIP_VISIBILITY` (declared maximum 10 bytes; header `0x8057`).
    pub const GAME_CMSG_EQUIP_VISIBILITY: u16 = 0x8057;

    /// `GAME_CMSG_TITLE_DISPLAY` (declared maximum 6 bytes; header `0x8058`).
    pub const GAME_CMSG_TITLE_DISPLAY: u16 = 0x8058;

    /// `GAME_CMSG_TITLE_HIDE` (declared maximum 2 bytes; header `0x8059`).
    pub const GAME_CMSG_TITLE_HIDE: u16 = 0x8059;

    /// `GAME_CMSG_SKILLBAR_SKILL_SET` (declared maximum 18 bytes; header `0x805C`).
    pub const GAME_CMSG_SKILLBAR_SKILL_SET: u16 = 0x805C;

    /// `GAME_CMSG_SKILLBAR_LOAD` (declared maximum 42 bytes; header `0x805D`).
    pub const GAME_CMSG_SKILLBAR_LOAD: u16 = 0x805D;

    /// `GAME_CMSG_SKILLBAR_SKILL_REPLACE` (declared maximum 22 bytes; header `0x805E`).
    pub const GAME_CMSG_SKILLBAR_SKILL_REPLACE: u16 = 0x805E;

    /// `GAME_CMSG_CHAR_CREATION_CHANGE_PROF` (declared maximum 4 bytes; header `0x8060`).
    pub const GAME_CMSG_CHAR_CREATION_CHANGE_PROF: u16 = 0x8060;

    /// `GAME_CMSG_SKIP_CINEMATIC` (declared maximum 2 bytes; header `0x8063`).
    pub const GAME_CMSG_SKIP_CINEMATIC: u16 = 0x8063;

    /// `GAME_CMSG_SEND_CHAT_MESSAGE` (declared maximum 286 bytes; header `0x8064`).
    pub const GAME_CMSG_SEND_CHAT_MESSAGE: u16 = 0x8064;

    /// `GAME_CMSG_ITEM_DESTROY` (declared maximum 6 bytes; header `0x8069`).
    pub const GAME_CMSG_ITEM_DESTROY: u16 = 0x8069;

    /// `GAME_CMSG_ITEM_IDENTIFY` (declared maximum 10 bytes; header `0x806C`).
    pub const GAME_CMSG_ITEM_IDENTIFY: u16 = 0x806C;

    /// `GAME_CMSG_TOME_UNLOCK_SKILL` (declared maximum 10 bytes; header `0x806D`).
    pub const GAME_CMSG_TOME_UNLOCK_SKILL: u16 = 0x806D;

    /// `GAME_CMSG_ITEM_MOVE` (declared maximum 9 bytes; header `0x8072`).
    pub const GAME_CMSG_ITEM_MOVE: u16 = 0x8072;

    /// `GAME_CMSG_ITEM_ACCEPT_ALL` (declared maximum 4 bytes; header `0x8073`).
    pub const GAME_CMSG_ITEM_ACCEPT_ALL: u16 = 0x8073;

    /// `GAME_CMSG_ITEM_SPLIT_STACK` (declared maximum 13 bytes; header `0x8075`).
    pub const GAME_CMSG_ITEM_SPLIT_STACK: u16 = 0x8075;

    /// `GAME_CMSG_ITEM_SALVAGE_SESSION_OPEN` (declared maximum 12 bytes; header `0x8077`).
    pub const GAME_CMSG_ITEM_SALVAGE_SESSION_OPEN: u16 = 0x8077;

    /// `GAME_CMSG_ITEM_SALVAGE_SESSION_CANCEL` (declared maximum 2 bytes; header `0x8078`).
    pub const GAME_CMSG_ITEM_SALVAGE_SESSION_CANCEL: u16 = 0x8078;

    /// `GAME_CMSG_ITEM_SALVAGE_SESSION_DONE` (declared maximum 2 bytes; header `0x8079`).
    pub const GAME_CMSG_ITEM_SALVAGE_SESSION_DONE: u16 = 0x8079;

    /// `GAME_CMSG_ITEM_SALVAGE_MATERIALS` (declared maximum 2 bytes; header `0x807A`).
    pub const GAME_CMSG_ITEM_SALVAGE_MATERIALS: u16 = 0x807A;

    /// `GAME_CMSG_ITEM_SALVAGE_UPGRADE` (declared maximum 3 bytes; header `0x807B`).
    pub const GAME_CMSG_ITEM_SALVAGE_UPGRADE: u16 = 0x807B;

    /// `GAME_CMSG_ITEM_CHANGE_GOLD` (declared maximum 10 bytes; header `0x807C`).
    pub const GAME_CMSG_ITEM_CHANGE_GOLD: u16 = 0x807C;

    /// `GAME_CMSG_ITEM_USE` (declared maximum 6 bytes; header `0x807E`).
    pub const GAME_CMSG_ITEM_USE: u16 = 0x807E;

    /// `GAME_CMSG_ITEM_UPDATE_EQUIPPED_COLOR` (declared maximum 4 bytes; header `0x8084`).
    pub const GAME_CMSG_ITEM_UPDATE_EQUIPPED_COLOR: u16 = 0x8084;

    /// `GAME_CMSG_INSTANCE_LOAD_REQUEST_SPAWN` (declared maximum 2 bytes; header `0x8088`).
    pub const GAME_CMSG_INSTANCE_LOAD_REQUEST_SPAWN: u16 = 0x8088;

    /// `GAME_CMSG_CHAR_CREATION_REQUEST_PLAYER` (declared maximum 2 bytes; header `0x8089`).
    pub const GAME_CMSG_CHAR_CREATION_REQUEST_PLAYER: u16 = 0x8089;

    /// `GAME_CMSG_CHAR_CREATION_REQUEST_ARMORS` (declared maximum 2 bytes; header `0x808A`).
    pub const GAME_CMSG_CHAR_CREATION_REQUEST_ARMORS: u16 = 0x808A;

    /// `GAME_CMSG_CHAR_CREATION_CONFIRM` (declared maximum 58 bytes; header `0x808B`).
    pub const GAME_CMSG_CHAR_CREATION_CONFIRM: u16 = 0x808B;

    /// `GAME_CMSG_INSTANCE_LOAD_REQUEST_PLAYERS` (declared maximum 19 bytes; header `0x8090`).
    pub const GAME_CMSG_INSTANCE_LOAD_REQUEST_PLAYERS: u16 = 0x8090;

    /// `GAME_CMSG_INSTANCE_LOAD_REQUEST_ITEMS` (declared maximum 4 bytes; header `0x8091`).
    pub const GAME_CMSG_INSTANCE_LOAD_REQUEST_ITEMS: u16 = 0x8091;

    /// `GAME_CMSG_PARTY_SET_DIFFICULTY` (declared maximum 3 bytes; header `0x809B`).
    pub const GAME_CMSG_PARTY_SET_DIFFICULTY: u16 = 0x809B;

    /// `GAME_CMSG_PARTY_ACCEPT_INVITE` (declared maximum 4 bytes; header `0x809C`).
    pub const GAME_CMSG_PARTY_ACCEPT_INVITE: u16 = 0x809C;

    /// `GAME_CMSG_PARTY_ACCEPT_CANCEL` (declared maximum 4 bytes; header `0x809D`).
    pub const GAME_CMSG_PARTY_ACCEPT_CANCEL: u16 = 0x809D;

    /// `GAME_CMSG_PARTY_ACCEPT_REFUSE` (declared maximum 4 bytes; header `0x809E`).
    pub const GAME_CMSG_PARTY_ACCEPT_REFUSE: u16 = 0x809E;

    /// `GAME_CMSG_PARTY_INVITE_NPC` (declared maximum 4 bytes; header `0x809F`).
    pub const GAME_CMSG_PARTY_INVITE_NPC: u16 = 0x809F;

    /// `GAME_CMSG_PARTY_INVITE_PLAYER` (declared maximum 4 bytes; header `0x80A0`).
    pub const GAME_CMSG_PARTY_INVITE_PLAYER: u16 = 0x80A0;

    /// `GAME_CMSG_PARTY_INVITE_PLAYER_NAME` (declared maximum 46 bytes; header `0x80A1`).
    pub const GAME_CMSG_PARTY_INVITE_PLAYER_NAME: u16 = 0x80A1;

    /// `GAME_CMSG_PARTY_LEAVE_GROUP` (declared maximum 2 bytes; header `0x80A2`).
    pub const GAME_CMSG_PARTY_LEAVE_GROUP: u16 = 0x80A2;

    /// `GAME_CMSG_PARTY_CANCEL_ENTER_CHALLENGE` (declared maximum 2 bytes; header `0x80A3`).
    pub const GAME_CMSG_PARTY_CANCEL_ENTER_CHALLENGE: u16 = 0x80A3;

    /// `GAME_CMSG_PARTY_ENTER_CHALLENGE` (declared maximum 3 bytes; header `0x80A5`).
    pub const GAME_CMSG_PARTY_ENTER_CHALLENGE: u16 = 0x80A5;

    /// `GAME_CMSG_PARTY_RETURN_TO_OUTPOST` (declared maximum 2 bytes; header `0x80A7`).
    pub const GAME_CMSG_PARTY_RETURN_TO_OUTPOST: u16 = 0x80A7;

    /// `GAME_CMSG_PARTY_KICK_NPC` (declared maximum 4 bytes; header `0x80A8`).
    pub const GAME_CMSG_PARTY_KICK_NPC: u16 = 0x80A8;

    /// `GAME_CMSG_PARTY_KICK_PLAYER` (declared maximum 4 bytes; header `0x80A9`).
    pub const GAME_CMSG_PARTY_KICK_PLAYER: u16 = 0x80A9;

    /// `GAME_CMSG_PARTY_SEARCH_SEEK` (declared maximum 73 bytes; header `0x80AA`).
    pub const GAME_CMSG_PARTY_SEARCH_SEEK: u16 = 0x80AA;

    /// `GAME_CMSG_PARTY_SEARCH_CANCEL` (declared maximum 2 bytes; header `0x80AB`).
    pub const GAME_CMSG_PARTY_SEARCH_CANCEL: u16 = 0x80AB;

    /// `GAME_CMSG_PARTY_SEARCH_REQUEST_JOIN` (declared maximum 4 bytes; header `0x80AC`).
    pub const GAME_CMSG_PARTY_SEARCH_REQUEST_JOIN: u16 = 0x80AC;

    /// `GAME_CMSG_PARTY_SEARCH_REQUEST_REPLY` (declared maximum 4 bytes; header `0x80AD`).
    pub const GAME_CMSG_PARTY_SEARCH_REQUEST_REPLY: u16 = 0x80AD;

    /// `GAME_CMSG_PARTY_SEARCH_TYPE` (declared maximum 3 bytes; header `0x80AE`).
    pub const GAME_CMSG_PARTY_SEARCH_TYPE: u16 = 0x80AE;

    /// `GAME_CMSG_PARTY_READY_STATUS` (declared maximum 3 bytes; header `0x80AF`).
    pub const GAME_CMSG_PARTY_READY_STATUS: u16 = 0x80AF;

    /// `GAME_CMSG_PARTY_ENTER_GUILD_HALL` (declared maximum 19 bytes; header `0x80B0`).
    pub const GAME_CMSG_PARTY_ENTER_GUILD_HALL: u16 = 0x80B0;

    /// `GAME_CMSG_PARTY_TRAVEL` (declared maximum 9 bytes; header `0x80B1`).
    pub const GAME_CMSG_PARTY_TRAVEL: u16 = 0x80B1;

    /// `GAME_CMSG_PARTY_LEAVE_GUILD_HALL` (declared maximum 3 bytes; header `0x80B2`).
    pub const GAME_CMSG_PARTY_LEAVE_GUILD_HALL: u16 = 0x80B2;

    /// `GAME_CMSG_TRAVEL_INIT` (declared maximum 10 bytes; header `0x80C1`).
    ///
    /// The client's own announcement that a travel has begun: the player touched a travel object,
    /// and the two agent ids name it. Named from `recordings/client/feature/movement/movement-sniff.log`
    /// (seq 1651/1652), where a real client sends it beside `INTERACT_LIVING` for the same agent.
    pub const GAME_CMSG_TRAVEL_INIT: u16 = 0x80C1;

    /// `GAME_SMSG_TRADE_REQUEST` (declared maximum 6 bytes; header `0x0000`).
    pub const GAME_SMSG_TRADE_REQUEST: u16 = 0x0000;

    /// `GAME_SMSG_TRADE_TERMINATE` (declared maximum 3 bytes; header `0x0001`).
    pub const GAME_SMSG_TRADE_TERMINATE: u16 = 0x0001;

    /// `GAME_SMSG_TRADE_CHANGE_OFFER` (declared maximum 7 bytes; header `0x0002`).
    pub const GAME_SMSG_TRADE_CHANGE_OFFER: u16 = 0x0002;

    /// `GAME_SMSG_TRADE_RECEIVE_OFFER` (declared maximum 6 bytes; header `0x0003`).
    pub const GAME_SMSG_TRADE_RECEIVE_OFFER: u16 = 0x0003;

    /// `GAME_SMSG_TRADE_ADD_ITEM` (declared maximum 7 bytes; header `0x0004`).
    pub const GAME_SMSG_TRADE_ADD_ITEM: u16 = 0x0004;

    /// `GAME_SMSG_TRADE_ACKNOWLEDGE` (declared maximum 6 bytes; header `0x0005`).
    pub const GAME_SMSG_TRADE_ACKNOWLEDGE: u16 = 0x0005;

    /// `GAME_SMSG_TRADE_ACCEPT` (declared maximum 2 bytes; header `0x0006`).
    pub const GAME_SMSG_TRADE_ACCEPT: u16 = 0x0006;

    /// `GAME_SMSG_TRADE_OFFERED_COUNT` (declared maximum 2 bytes; header `0x0008`).
    pub const GAME_SMSG_TRADE_OFFERED_COUNT: u16 = 0x0008;

    /// `GAME_SMSG_PING_REQUEST` (declared maximum 2 bytes; header `0x000C`).
    pub const GAME_SMSG_PING_REQUEST: u16 = 0x000C;

    /// `GAME_SMSG_PING_REPLY` (declared maximum 6 bytes; header `0x000D`).
    pub const GAME_SMSG_PING_REPLY: u16 = 0x000D;

    /// `GAME_SMSG_FRIENDLIST_MESSAGE` (declared maximum 1042 bytes; header `0x000E`).
    pub const GAME_SMSG_FRIENDLIST_MESSAGE: u16 = 0x000E;

    /// `GAME_SMSG_ACCOUNT_FEATURE` (declared maximum 8 bytes; header `0x000F`).
    pub const GAME_SMSG_ACCOUNT_FEATURE: u16 = 0x000F;

    /// `GAME_SMSG_PVP_UPDATE_UNLOCKED_HEROES` (declared maximum 38 bytes; header `0x0018`).
    pub const GAME_SMSG_PVP_UPDATE_UNLOCKED_HEROES: u16 = 0x0018;

    /// `GAME_SMSG_PVP_ITEM_STREAM_ADD` (declared maximum 264 bytes; header `0x001A`).
    pub const GAME_SMSG_PVP_ITEM_STREAM_ADD: u16 = 0x001A;

    /// `GAME_SMSG_PVP_ITEM_STREAM_END` (declared maximum 2 bytes; header `0x001B`).
    pub const GAME_SMSG_PVP_ITEM_STREAM_END: u16 = 0x001B;

    /// `GAME_SMSG_PVP_UPDATE_UNLOCKED_SKILLS` (declared maximum 518 bytes; header `0x001D`).
    pub const GAME_SMSG_PVP_UPDATE_UNLOCKED_SKILLS: u16 = 0x001D;

    /// `GAME_SMSG_WORLD_SIMULATION_TICK` (declared maximum 6 bytes; header `0x001E`).
    pub const GAME_SMSG_WORLD_SIMULATION_TICK: u16 = 0x001E;

    /// `GAME_SMSG_WORLD_UPDATE_LOAD_TIME` (declared maximum 6 bytes; header `0x001F`).
    pub const GAME_SMSG_WORLD_UPDATE_LOAD_TIME: u16 = 0x001F;

    /// `GAME_SMSG_WORLD_CREATE_AGENT` (declared maximum 99 bytes; header `0x0020`).
    pub const GAME_SMSG_WORLD_CREATE_AGENT: u16 = 0x0020;

    /// `GAME_SMSG_WORLD_REMOVE_AGENT` (declared maximum 8 bytes; header `0x0021`).
    pub const GAME_SMSG_WORLD_REMOVE_AGENT: u16 = 0x0021;

    /// `GAME_SMSG_WORLD_UPDATE_CONTROLLED_AGENT` (declared maximum 10 bytes; header `0x0022`).
    pub const GAME_SMSG_WORLD_UPDATE_CONTROLLED_AGENT: u16 = 0x0022;

    /// `GAME_SMSG_AGENT_UPDATE_DIRECTION` (declared maximum 15 bytes; header `0x0025`).
    pub const GAME_SMSG_AGENT_UPDATE_DIRECTION: u16 = 0x0025;

    /// `GAME_SMSG_AGENT_UPDATE_SPEED_BASE` (declared maximum 10 bytes; header `0x0027`).
    pub const GAME_SMSG_AGENT_UPDATE_SPEED_BASE: u16 = 0x0027;

    /// `GAME_SMSG_AGENT_STOP_MOVING` (declared maximum 6 bytes; header `0x0028`).
    pub const GAME_SMSG_AGENT_STOP_MOVING: u16 = 0x0028;

    /// `GAME_SMSG_AGENT_MOVE_TO_POINT` (declared maximum 18 bytes; header `0x0029`).
    pub const GAME_SMSG_AGENT_MOVE_TO_POINT: u16 = 0x0029;

    /// `GAME_SMSG_AGENT_UPDATE_DESTINATION` (declared maximum 22 bytes; header `0x002A`).
    pub const GAME_SMSG_AGENT_UPDATE_DESTINATION: u16 = 0x002A;

    /// `GAME_SMSG_AGENT_UPDATE_SPEED` (declared maximum 11 bytes; header `0x002B`).
    pub const GAME_SMSG_AGENT_UPDATE_SPEED: u16 = 0x002B;

    /// `GAME_SMSG_AGENT_UPDATE_POSITION` (declared maximum 16 bytes; header `0x002C`).
    pub const GAME_SMSG_AGENT_UPDATE_POSITION: u16 = 0x002C;

    /// `GAME_SMSG_AGENT_PLAYER_DIE` (declared maximum 6 bytes; header `0x002D`).
    pub const GAME_SMSG_AGENT_PLAYER_DIE: u16 = 0x002D;

    /// `GAME_SMSG_AGENT_UPDATE_ROTATION` (declared maximum 14 bytes; header `0x002E`).
    pub const GAME_SMSG_AGENT_UPDATE_ROTATION: u16 = 0x002E;

    /// `GAME_SMSG_AGENT_UPDATE_ALLEGIANCE` (declared maximum 10 bytes; header `0x002F`).
    pub const GAME_SMSG_AGENT_UPDATE_ALLEGIANCE: u16 = 0x002F;

    /// `GAME_SMSG_CHARACTER_UPDATE_INFO` (declared maximum 94 bytes; header `0x0030`).
    pub const GAME_SMSG_CHARACTER_UPDATE_INFO: u16 = 0x0030;

    /// `GAME_SMSG_CHARACTER_UPDATE_NAME` (declared maximum 70 bytes; header `0x0031`).
    pub const GAME_SMSG_CHARACTER_UPDATE_NAME: u16 = 0x0031;

    /// `GAME_SMSG_MESSAGE_OF_THE_DAY` (declared maximum 134 bytes; header `0x0033`).
    pub const GAME_SMSG_MESSAGE_OF_THE_DAY: u16 = 0x0033;

    /// `GAME_SMSG_AGENT_PINGED` (declared maximum 8 bytes; header `0x0034`).
    pub const GAME_SMSG_AGENT_PINGED: u16 = 0x0034;

    /// `GAME_SMSG_AGENT_UPDATE_ATTRIBUTE_POINTS` (declared maximum 8 bytes; header `0x0037`).
    pub const GAME_SMSG_AGENT_UPDATE_ATTRIBUTE_POINTS: u16 = 0x0037;

    /// `GAME_SMSG_AGENT_UPDATE_ATTRIBUTES` (declared maximum 202 bytes; header `0x003A`).
    pub const GAME_SMSG_AGENT_UPDATE_ATTRIBUTES: u16 = 0x003A;

    /// `GAME_SMSG_AGENT_ALLY_DESTROY` (declared maximum 6 bytes; header `0x003E`).
    pub const GAME_SMSG_AGENT_ALLY_DESTROY: u16 = 0x003E;

    /// `GAME_SMSG_EFFECT_UPKEEP_ADDED` (declared maximum 20 bytes; header `0x003F`).
    pub const GAME_SMSG_EFFECT_UPKEEP_ADDED: u16 = 0x003F;

    /// `GAME_SMSG_EFFECT_UPKEEP_REMOVED` (declared maximum 10 bytes; header `0x0040`).
    pub const GAME_SMSG_EFFECT_UPKEEP_REMOVED: u16 = 0x0040;

    /// `GAME_SMSG_EFFECT_UPKEEP_APPLIED` (declared maximum 20 bytes; header `0x0041`).
    pub const GAME_SMSG_EFFECT_UPKEEP_APPLIED: u16 = 0x0041;

    /// `GAME_SMSG_EFFECT_APPLIED` (declared maximum 20 bytes; header `0x0042`).
    pub const GAME_SMSG_EFFECT_APPLIED: u16 = 0x0042;

    /// `GAME_SMSG_EFFECT_RENEWED` (declared maximum 18 bytes; header `0x0043`).
    pub const GAME_SMSG_EFFECT_RENEWED: u16 = 0x0043;

    /// `GAME_SMSG_EFFECT_REMOVED` (declared maximum 10 bytes; header `0x0044`).
    pub const GAME_SMSG_EFFECT_REMOVED: u16 = 0x0044;

    /// `GAME_SMSG_SCREEN_SHAKE` (declared maximum 14 bytes; header `0x0046`).
    pub const GAME_SMSG_SCREEN_SHAKE: u16 = 0x0046;

    /// `GAME_SMSG_AGENT_DISPLAY_CAPE` (declared maximum 7 bytes; header `0x0048`).
    pub const GAME_SMSG_AGENT_DISPLAY_CAPE: u16 = 0x0048;

    /// `GAME_SMSG_QUEST_ADD` (declared maximum 84 bytes; header `0x0049`).
    pub const GAME_SMSG_QUEST_ADD: u16 = 0x0049;

    /// `GAME_SMSG_QUEST_DESCRIPTION` (declared maximum 526 bytes; header `0x004C`).
    pub const GAME_SMSG_QUEST_DESCRIPTION: u16 = 0x004C;

    /// `GAME_SMSG_QUEST_GENERAL_INFO` (declared maximum 72 bytes; header `0x0050`).
    pub const GAME_SMSG_QUEST_GENERAL_INFO: u16 = 0x0050;

    /// `GAME_SMSG_QUEST_UPDATE_MARKER` (declared maximum 18 bytes; header `0x0051`).
    pub const GAME_SMSG_QUEST_UPDATE_MARKER: u16 = 0x0051;

    /// `GAME_SMSG_QUEST_REMOVE` (declared maximum 6 bytes; header `0x0052`).
    pub const GAME_SMSG_QUEST_REMOVE: u16 = 0x0052;

    /// `GAME_SMSG_QUEST_ADD_MARKER` (declared maximum 18 bytes; header `0x0053`).
    pub const GAME_SMSG_QUEST_ADD_MARKER: u16 = 0x0053;

    /// `GAME_SMSG_QUEST_UPDATE_NAME` (declared maximum 266 bytes; header `0x0054`).
    pub const GAME_SMSG_QUEST_UPDATE_NAME: u16 = 0x0054;

    /// `GAME_SMSG_NPC_UPDATE_PROPERTIES` (declared maximum 48 bytes; header `0x0056`).
    pub const GAME_SMSG_NPC_UPDATE_PROPERTIES: u16 = 0x0056;

    /// `GAME_SMSG_NPC_UPDATE_MODEL` (declared maximum 42 bytes; header `0x0057`).
    pub const GAME_SMSG_NPC_UPDATE_MODEL: u16 = 0x0057;

    /// `GAME_SMSG_PLAYER_CREATE` (declared maximum 91 bytes; header `0x0059`).
    pub const GAME_SMSG_PLAYER_CREATE: u16 = 0x0059;

    /// `GAME_SMSG_PLAYER_DESTROY` (declared maximum 8 bytes; header `0x005A`).
    pub const GAME_SMSG_PLAYER_DESTROY: u16 = 0x005A;

    /// `GAME_SMSG_CHAT_MESSAGE_CORE` (declared maximum 250 bytes; header `0x005D`).
    pub const GAME_SMSG_CHAT_MESSAGE_CORE: u16 = 0x005D;

    /// `GAME_SMSG_CHAT_MESSAGE_SERVER` (5 bytes on the live wire; header `0x005E`).
    pub const GAME_SMSG_CHAT_MESSAGE_SERVER: u16 = 0x005E;

    /// `GAME_SMSG_CHAT_MESSAGE_NPC` (declared maximum 28 bytes; header `0x005F`).
    pub const GAME_SMSG_CHAT_MESSAGE_NPC: u16 = 0x005F;

    /// `GAME_SMSG_CHAT_MESSAGE_GLOBAL` (declared maximum 86 bytes; header `0x0060`).
    pub const GAME_SMSG_CHAT_MESSAGE_GLOBAL: u16 = 0x0060;

    /// `GAME_SMSG_CHAT_MESSAGE_LOCAL` (5 bytes on the live wire; header `0x0061`).
    pub const GAME_SMSG_CHAT_MESSAGE_LOCAL: u16 = 0x0061;

    /// `GAME_SMSG_HERO_BEHAVIOR` (declared maximum 10 bytes; header `0x0062`).
    pub const GAME_SMSG_HERO_BEHAVIOR: u16 = 0x0062;

    /// `GAME_SMSG_HERO_SKILL_STATUS` (declared maximum 8 bytes; header `0x0064`).
    pub const GAME_SMSG_HERO_SKILL_STATUS: u16 = 0x0064;

    /// `GAME_SMSG_HERO_SKILL_STATUS_BITMAP` (declared maximum 7 bytes; header `0x0065`).
    pub const GAME_SMSG_HERO_SKILL_STATUS_BITMAP: u16 = 0x0065;

    /// `GAME_SMSG_POST_PROCESS` (declared maximum 7 bytes; header `0x006B`).
    pub const GAME_SMSG_POST_PROCESS: u16 = 0x006B;

    /// `GAME_SMSG_DUNGEON_REWARD` (declared maximum 14 bytes; header `0x006C`).
    pub const GAME_SMSG_DUNGEON_REWARD: u16 = 0x006C;

    /// `GAME_SMSG_NPC_UPDATE_WEAPONS` (declared maximum 14 bytes; header `0x006D`).
    pub const GAME_SMSG_NPC_UPDATE_WEAPONS: u16 = 0x006D;

    /// `GAME_SMSG_UPDATE_AGENT_VISUAL_EQUIPMENT` (declared maximum 42 bytes; header `0x006E`).
    pub const GAME_SMSG_UPDATE_AGENT_VISUAL_EQUIPMENT: u16 = 0x006E;

    /// `GAME_SMSG_HARD_MODE_UNLOCKED` (declared maximum 3 bytes; header `0x0070`).
    pub const GAME_SMSG_HARD_MODE_UNLOCKED: u16 = 0x0070;

    /// `GAME_SMSG_MERCENARY_INFO` (declared maximum 129 bytes; header `0x0074`).
    pub const GAME_SMSG_MERCENARY_INFO: u16 = 0x0074;

    /// `GAME_SMSG_DIALOG_BUTTON` (declared maximum 271 bytes; header `0x007E`).
    pub const GAME_SMSG_DIALOG_BUTTON: u16 = 0x007E;

    /// `GAME_SMSG_DIALOG_BODY` (declared maximum 250 bytes; header `0x0080`).
    pub const GAME_SMSG_DIALOG_BODY: u16 = 0x0080;

    /// `GAME_SMSG_DIALOG_SENDER` (declared maximum 6 bytes; header `0x0081`).
    pub const GAME_SMSG_DIALOG_SENDER: u16 = 0x0081;

    /// `GAME_SMSG_WINDOW_OPEN` (declared maximum 11 bytes; header `0x0083`).
    pub const GAME_SMSG_WINDOW_OPEN: u16 = 0x0083;

    /// `GAME_SMSG_WINDOW_ADD_ITEMS` (declared maximum 70 bytes; header `0x0084`).
    pub const GAME_SMSG_WINDOW_ADD_ITEMS: u16 = 0x0084;

    /// `GAME_SMSG_WINDOW_ITEM_STREAM_END` (declared maximum 3 bytes; header `0x0086`).
    pub const GAME_SMSG_WINDOW_ITEM_STREAM_END: u16 = 0x0086;

    /// `GAME_SMSG_CARTOGRAPHY_DATA` (declared maximum 262 bytes; header `0x008A`).
    pub const GAME_SMSG_CARTOGRAPHY_DATA: u16 = 0x008A;

    /// `GAME_SMSG_COMPASS_DRAWING` (declared maximum 76 bytes; header `0x0091`).
    pub const GAME_SMSG_COMPASS_DRAWING: u16 = 0x0091;

    /// `GAME_SMSG_MAP_UPDATE_UNLOCKED_LIST` (declared maximum 662 bytes; header `0x0094`).
    pub const GAME_SMSG_MAP_UPDATE_UNLOCKED_LIST: u16 = 0x0094;

    /// `GAME_SMSG_MAP_UPDATE_CURRENT` (declared maximum 5 bytes; header `0x0099`).
    pub const GAME_SMSG_MAP_UPDATE_CURRENT: u16 = 0x0099;

    /// `GAME_SMSG_AGENT_UPDATE_SCALE` (declared maximum 10 bytes; header `0x009A`).
    pub const GAME_SMSG_AGENT_UPDATE_SCALE: u16 = 0x009A;

    /// `GAME_SMSG_AGENT_UPDATE_NPC_NAME` (declared maximum 74 bytes; header `0x009B`).
    pub const GAME_SMSG_AGENT_UPDATE_NPC_NAME: u16 = 0x009B;

    /// `GAME_SMSG_AGENT_DISPLAY_DIALOG` (declared maximum 323 bytes; header `0x009E`).
    pub const GAME_SMSG_AGENT_DISPLAY_DIALOG: u16 = 0x009E;

    /// `GAME_SMSG_AGENT_PROPERTY_UPDATE_INT` (declared maximum 14 bytes; header `0x009F`).
    pub const GAME_SMSG_AGENT_PROPERTY_UPDATE_INT: u16 = 0x009F;

    /// `GAME_SMSG_AGENT_PROPERTY_UPDATE_INT_TARGET` (declared maximum 18 bytes; header `0x00A0`).
    pub const GAME_SMSG_AGENT_PROPERTY_UPDATE_INT_TARGET: u16 = 0x00A0;

    /// `GAME_SMSG_AGENT_PROPERTY_PLAY_EFFECT` (declared maximum 20 bytes; header `0x00A1`).
    pub const GAME_SMSG_AGENT_PROPERTY_PLAY_EFFECT: u16 = 0x00A1;

    /// `GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT` (declared maximum 14 bytes; header `0x00A2`).
    pub const GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT: u16 = 0x00A2;

    /// `GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT_TARGET` (declared maximum 18 bytes; header `0x00A3`).
    pub const GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT_TARGET: u16 = 0x00A3;

    /// `GAME_SMSG_AGENT_PROJECTILE_LAUNCHED` (declared maximum 29 bytes; header `0x00A4`).
    pub const GAME_SMSG_AGENT_PROJECTILE_LAUNCHED: u16 = 0x00A4;

    /// `GAME_SMSG_AGENT_UPDATE_SPEECH_BUBBLE` (declared maximum 254 bytes; header `0x00A5`).
    pub const GAME_SMSG_AGENT_UPDATE_SPEECH_BUBBLE: u16 = 0x00A5;

    /// `GAME_SMSG_AGENT_UPDATE_PROFESSION` (declared maximum 8 bytes; header `0x00A6`).
    pub const GAME_SMSG_AGENT_UPDATE_PROFESSION: u16 = 0x00A6;

    /// `GAME_SMSG_AGENT_CREATE_NPC` (declared maximum 14 bytes; header `0x00AA`).
    pub const GAME_SMSG_AGENT_CREATE_NPC: u16 = 0x00AA;

    /// `GAME_SMSG_AGENT_UPDATE_OBSERVED_TARGET` (declared maximum 7 bytes; header `0x00AC`).
    pub const GAME_SMSG_AGENT_UPDATE_OBSERVED_TARGET: u16 = 0x00AC;

    /// `GAME_SMSG_AGENT_UPDATE_MODEL` (declared maximum 10 bytes; header `0x00AE`).
    pub const GAME_SMSG_AGENT_UPDATE_MODEL: u16 = 0x00AE;

    /// `GAME_SMSG_PLAYER_UPDATE_PARTY_SIZE` (declared maximum 12 bytes; header `0x00B0`).
    pub const GAME_SMSG_PLAYER_UPDATE_PARTY_SIZE: u16 = 0x00B0;

    /// `GAME_SMSG_PLAYER_UPDATE_PARTY` (declared maximum 12 bytes; header `0x00B1`).
    pub const GAME_SMSG_PLAYER_UPDATE_PARTY: u16 = 0x00B1;

    /// `GAME_SMSG_PLAYER_UPDATE_UNLOCKED_PROFESSIONS` (declared maximum 10 bytes; header `0x00B6`).
    pub const GAME_SMSG_PLAYER_UPDATE_UNLOCKED_PROFESSIONS: u16 = 0x00B6;

    /// `GAME_SMSG_PLAYER_UPDATE_PROFESSION` (declared maximum 9 bytes; header `0x00B7`).
    pub const GAME_SMSG_PLAYER_UPDATE_PROFESSION: u16 = 0x00B7;

    /// `GAME_SMSG_MISSION_INFOBOX_ADD` (declared maximum 266 bytes; header `0x00B9`).
    pub const GAME_SMSG_MISSION_INFOBOX_ADD: u16 = 0x00B9;

    /// `GAME_SMSG_MISSION_STREAM_START` (declared maximum 2 bytes; header `0x00BA`).
    pub const GAME_SMSG_MISSION_STREAM_START: u16 = 0x00BA;

    /// `GAME_SMSG_MISSION_OBJECTIVE_ADD` (declared maximum 270 bytes; header `0x00BB`).
    pub const GAME_SMSG_MISSION_OBJECTIVE_ADD: u16 = 0x00BB;

    /// `GAME_SMSG_MISSION_OBJECTIVE_COMPLETE` (declared maximum 6 bytes; header `0x00BC`).
    pub const GAME_SMSG_MISSION_OBJECTIVE_COMPLETE: u16 = 0x00BC;

    /// `GAME_SMSG_MISSION_OBJECTIVE_UPDATE_STRING` (declared maximum 266 bytes; header `0x00BD`).
    pub const GAME_SMSG_MISSION_OBJECTIVE_UPDATE_STRING: u16 = 0x00BD;

    /// `GAME_SMSG_MERCHANT_WINDOW_OPEN` (declared maximum 7 bytes; header `0x00C3`).
    pub const GAME_SMSG_MERCHANT_WINDOW_OPEN: u16 = 0x00C3;

    /// `GAME_SMSG_MERCHANT_WINDOW_UPDATE_OWNER` (declared maximum 6 bytes; header `0x00C4`).
    pub const GAME_SMSG_MERCHANT_WINDOW_UPDATE_OWNER: u16 = 0x00C4;

    /// `GAME_SMSG_TRADER_TRANSACTION_REJECT` (declared maximum 7 bytes; header `0x00C6`).
    pub const GAME_SMSG_TRADER_TRANSACTION_REJECT: u16 = 0x00C6;

    /// `GAME_SMSG_TRADER_TRANSACTION_DONE` (declared maximum 3 bytes; header `0x00CC`).
    pub const GAME_SMSG_TRADER_TRANSACTION_DONE: u16 = 0x00CC;

    /// `GAME_SMSG_TRADER_WINDOW_OPEN` (declared maximum 9 bytes; header `0x00CD`).
    pub const GAME_SMSG_TRADER_WINDOW_OPEN: u16 = 0x00CD;

    /// `GAME_SMSG_SKILLBAR_UPDATE_SKILL` (declared maximum 13 bytes; header `0x00D9`).
    pub const GAME_SMSG_SKILLBAR_UPDATE_SKILL: u16 = 0x00D9;

    /// `GAME_SMSG_SKILLBAR_UPDATE` (declared maximum 79 bytes; header `0x00DA`).
    pub const GAME_SMSG_SKILLBAR_UPDATE: u16 = 0x00DA;

    /// `GAME_SMSG_UPDATE_UNLOCKED_SKILLS` (declared maximum 518 bytes; header `0x00DB`).
    pub const GAME_SMSG_UPDATE_UNLOCKED_SKILLS: u16 = 0x00DB;

    /// `GAME_SMSG_SKILL_WINDOW_COUNT` (declared maximum 8 bytes; header `0x00DC`).
    pub const GAME_SMSG_SKILL_WINDOW_COUNT: u16 = 0x00DC;

    /// `GAME_SMSG_SKILL_WINDOW_DATA` (declared maximum 71 bytes; header `0x00E0`).
    pub const GAME_SMSG_SKILL_WINDOW_DATA: u16 = 0x00E0;

    /// `GAME_SMSG_SKILL_WINDOW_END` (declared maximum 3 bytes; header `0x00E1`).
    pub const GAME_SMSG_SKILL_WINDOW_END: u16 = 0x00E1;

    /// `GAME_SMSG_SKILL_INTERUPTED` (declared maximum 12 bytes; header `0x00E2`).
    pub const GAME_SMSG_SKILL_INTERUPTED: u16 = 0x00E2;

    /// `GAME_SMSG_SKILL_CANCEL` (declared maximum 12 bytes; header `0x00E3`).
    ///
    /// `opcodes.h` also gives this header the name `GAME_SMSG_SKILL_ACTIVATED`.
    pub const GAME_SMSG_SKILL_CANCEL: u16 = 0x00E3;

    /// `GAME_SMSG_SKILL_ACTIVATED`: a second name for `GAME_SMSG_SKILL_CANCEL`, the same header `0x00E3`.
    pub const GAME_SMSG_SKILL_ACTIVATED: u16 = 0x00E3;

    /// `GAME_SMSG_SKILL_ACTIVATE` (declared maximum 12 bytes; header `0x00E4`).
    pub const GAME_SMSG_SKILL_ACTIVATE: u16 = 0x00E4;

    /// `GAME_SMSG_SKILL_RECHARGE` (declared maximum 16 bytes; header `0x00E5`).
    pub const GAME_SMSG_SKILL_RECHARGE: u16 = 0x00E5;

    /// `GAME_SMSG_SKILL_RECHARGED` (declared maximum 12 bytes; header `0x00E6`).
    pub const GAME_SMSG_SKILL_RECHARGED: u16 = 0x00E6;

    /// `GAME_SMSG_CHARACTER_UPDATE_FACTIONS` (declared maximum 62 bytes; header `0x00E9`).
    pub const GAME_SMSG_CHARACTER_UPDATE_FACTIONS: u16 = 0x00E9;

    /// `GAME_SMSG_CHARACTER_FACTION_MAX_KURZICK` (declared maximum 6 bytes; header `0x00EA`).
    pub const GAME_SMSG_CHARACTER_FACTION_MAX_KURZICK: u16 = 0x00EA;

    /// `GAME_SMSG_CHARACTER_FACTION_MAX_LUXON` (declared maximum 6 bytes; header `0x00EB`).
    pub const GAME_SMSG_CHARACTER_FACTION_MAX_LUXON: u16 = 0x00EB;

    /// `GAME_SMSG_CHARACTER_FACTION_MAX_BALTHAZAR` (declared maximum 6 bytes; header `0x00EC`).
    pub const GAME_SMSG_CHARACTER_FACTION_MAX_BALTHAZAR: u16 = 0x00EC;

    /// `GAME_SMSG_CHARACTER_FACTION_MAX_IMPERIAL` (declared maximum 6 bytes; header `0x00ED`).
    pub const GAME_SMSG_CHARACTER_FACTION_MAX_IMPERIAL: u16 = 0x00ED;

    /// `GAME_SMSG_CHARACTER_FACTION_UPDATE` (declared maximum 10 bytes; header `0x00EE`).
    pub const GAME_SMSG_CHARACTER_FACTION_UPDATE: u16 = 0x00EE;

    /// `GAME_SMSG_AGENT_INITIAL_EFFECTS` (declared maximum 10 bytes; header `0x00F0`).
    pub const GAME_SMSG_AGENT_INITIAL_EFFECTS: u16 = 0x00F0;

    /// `GAME_SMSG_AGENT_UPDATE_EFFECTS` (declared maximum 10 bytes; header `0x00F1`).
    pub const GAME_SMSG_AGENT_UPDATE_EFFECTS: u16 = 0x00F1;

    /// `GAME_SMSG_INSTANCE_LOADED` (declared maximum 6 bytes; header `0x00F2`).
    pub const GAME_SMSG_INSTANCE_LOADED: u16 = 0x00F2;

    /// `GAME_SMSG_TITLE_RANK_DATA` (declared maximum 34 bytes; header `0x00F3`).
    pub const GAME_SMSG_TITLE_RANK_DATA: u16 = 0x00F3;

    /// `GAME_SMSG_TITLE_RANK_DISPLAY` (declared maximum 8 bytes; header `0x00F4`).
    pub const GAME_SMSG_TITLE_RANK_DISPLAY: u16 = 0x00F4;

    /// `GAME_SMSG_TITLE_UPDATE` (declared maximum 10 bytes; header `0x00F5`).
    pub const GAME_SMSG_TITLE_UPDATE: u16 = 0x00F5;

    /// `GAME_SMSG_TITLE_TRACK_INFO` (declared maximum 82 bytes; header `0x00F6`).
    pub const GAME_SMSG_TITLE_TRACK_INFO: u16 = 0x00F6;

    /// `GAME_SMSG_ITEM_PRICE_QUOTE` (declared maximum 10 bytes; header `0x00F7`).
    pub const GAME_SMSG_ITEM_PRICE_QUOTE: u16 = 0x00F7;

    /// `GAME_SMSG_ITEM_PRICES` (declared maximum 70 bytes; header `0x00F9`).
    pub const GAME_SMSG_ITEM_PRICES: u16 = 0x00F9;

    /// `GAME_SMSG_VANQUISH_PROGRESS` (declared maximum 134 bytes; header `0x00FA`).
    pub const GAME_SMSG_VANQUISH_PROGRESS: u16 = 0x00FA;

    /// `GAME_SMSG_VANQUISH_COMPLETE` (declared maximum 12 bytes; header `0x00FB`).
    pub const GAME_SMSG_VANQUISH_COMPLETE: u16 = 0x00FB;

    /// `GAME_SMSG_CINEMATIC_SKIP_EVERYONE` (declared maximum 2 bytes; header `0x00FE`).
    pub const GAME_SMSG_CINEMATIC_SKIP_EVERYONE: u16 = 0x00FE;

    /// `GAME_SMSG_CINEMATIC_SKIP_COUNT` (declared maximum 4 bytes; header `0x00FF`).
    pub const GAME_SMSG_CINEMATIC_SKIP_COUNT: u16 = 0x00FF;

    /// `GAME_SMSG_CINEMATIC_START` (declared maximum 3 bytes; header `0x0100`).
    pub const GAME_SMSG_CINEMATIC_START: u16 = 0x0100;

    /// `GAME_SMSG_CINEMATIC_TEXT` (declared maximum 172 bytes; header `0x0102`).
    pub const GAME_SMSG_CINEMATIC_TEXT: u16 = 0x0102;

    /// `GAME_SMSG_CINEMATIC_DATA_END` (declared maximum 2 bytes; header `0x0103`).
    pub const GAME_SMSG_CINEMATIC_DATA_END: u16 = 0x0103;

    /// `GAME_SMSG_CINEMATIC_DATA` (declared maximum 1030 bytes; header `0x0104`).
    pub const GAME_SMSG_CINEMATIC_DATA: u16 = 0x0104;

    /// `GAME_SMSG_CINEMATIC_END` (declared maximum 2 bytes; header `0x0105`).
    pub const GAME_SMSG_CINEMATIC_END: u16 = 0x0105;

    /// `GAME_SMSG_SIGNPOST_BUTTON` (declared maximum 257 bytes; header `0x010A`).
    pub const GAME_SMSG_SIGNPOST_BUTTON: u16 = 0x010A;

    /// `GAME_SMSG_SIGNPOST_BODY` (declared maximum 250 bytes; header `0x010B`).
    pub const GAME_SMSG_SIGNPOST_BODY: u16 = 0x010B;

    /// `GAME_SMSG_SIGNPOST_SENDER` (declared maximum 6 bytes; header `0x010C`).
    pub const GAME_SMSG_SIGNPOST_SENDER: u16 = 0x010C;

    /// `GAME_SMSG_MANIPULATE_MAP_OBJECT` (declared maximum 9 bytes; header `0x010E`).
    pub const GAME_SMSG_MANIPULATE_MAP_OBJECT: u16 = 0x010E;

    /// `GAME_SMSG_MANIPULATE_MAP_OBJECT2` (declared maximum 12 bytes; header `0x0111`).
    pub const GAME_SMSG_MANIPULATE_MAP_OBJECT2: u16 = 0x0111;

    /// `GAME_SMSG_GUILD_PLAYER_ROLE` (declared maximum 21 bytes; header `0x0118`).
    pub const GAME_SMSG_GUILD_PLAYER_ROLE: u16 = 0x0118;

    /// `GAME_SMSG_TOWN_ALLIANCE_OBJECT` (declared maximum 102 bytes; header `0x011A`).
    pub const GAME_SMSG_TOWN_ALLIANCE_OBJECT: u16 = 0x011A;

    /// `GAME_SMSG_GUILD_ALLIANCE_INFO` (declared maximum 119 bytes; header `0x0120`).
    pub const GAME_SMSG_GUILD_ALLIANCE_INFO: u16 = 0x0120;

    /// `GAME_SMSG_GUILD_GENERAL_INFO` (declared maximum 132 bytes; header `0x0121`).
    pub const GAME_SMSG_GUILD_GENERAL_INFO: u16 = 0x0121;

    /// `GAME_SMSG_GUILD_CHANGE_FACTION` (declared maximum 9 bytes; header `0x0122`).
    pub const GAME_SMSG_GUILD_CHANGE_FACTION: u16 = 0x0122;

    /// `GAME_SMSG_GUILD_INVITE_RECEIVED` (declared maximum 155 bytes; header `0x0123`).
    pub const GAME_SMSG_GUILD_INVITE_RECEIVED: u16 = 0x0123;

    /// `GAME_SMSG_GUILD_PLAYER_INFO` (declared maximum 283 bytes; header `0x0127`).
    pub const GAME_SMSG_GUILD_PLAYER_INFO: u16 = 0x0127;

    /// `GAME_SMSG_GUILD_PLAYER_REMOVE` (declared maximum 46 bytes; header `0x0128`).
    pub const GAME_SMSG_GUILD_PLAYER_REMOVE: u16 = 0x0128;

    /// `GAME_SMSG_GUILD_PLAYER_CHANGE_COMPLETE` (declared maximum 46 bytes; header `0x012A`).
    pub const GAME_SMSG_GUILD_PLAYER_CHANGE_COMPLETE: u16 = 0x012A;

    /// `GAME_SMSG_GUILD_CHANGE_PLAYER_CONTEXT` (declared maximum 138 bytes; header `0x012B`).
    pub const GAME_SMSG_GUILD_CHANGE_PLAYER_CONTEXT: u16 = 0x012B;

    /// `GAME_SMSG_GUILD_CHANGE_PLAYER_STATUS` (declared maximum 10 bytes; header `0x012C`).
    pub const GAME_SMSG_GUILD_CHANGE_PLAYER_STATUS: u16 = 0x012C;

    /// `GAME_SMSG_GUILD_CHANGE_PLAYER_TYPE` (declared maximum 3 bytes; header `0x012D`).
    pub const GAME_SMSG_GUILD_CHANGE_PLAYER_TYPE: u16 = 0x012D;

    /// `GAME_SMSG_ITEM_UPDATE_OWNER` (declared maximum 12 bytes; header `0x0135`).
    pub const GAME_SMSG_ITEM_UPDATE_OWNER: u16 = 0x0135;

    /// `GAME_SMSG_ITEM_UPDATE_QUANTITY` (declared maximum 10 bytes; header `0x0139`).
    pub const GAME_SMSG_ITEM_UPDATE_QUANTITY: u16 = 0x0139;

    /// `GAME_SMSG_ITEM_UPDATE_CUSTOMIZED_NAME` (declared maximum 74 bytes; header `0x013A`).
    pub const GAME_SMSG_ITEM_UPDATE_CUSTOMIZED_NAME: u16 = 0x013A;

    /// `GAME_SMSG_ITEM_MOVED_TO_LOCATION` (declared maximum 11 bytes; header `0x013E`).
    pub const GAME_SMSG_ITEM_MOVED_TO_LOCATION: u16 = 0x013E;

    /// `GAME_SMSG_INVENTORY_CREATE_BAG` (declared maximum 13 bytes; header `0x013F`).
    pub const GAME_SMSG_INVENTORY_CREATE_BAG: u16 = 0x013F;

    /// `GAME_SMSG_UPDATE_GOLD_CHARACTER` (declared maximum 8 bytes; header `0x0140`).
    pub const GAME_SMSG_UPDATE_GOLD_CHARACTER: u16 = 0x0140;

    /// `GAME_SMSG_UPDATE_GOLD_STORAGE` (declared maximum 8 bytes; header `0x0141`).
    pub const GAME_SMSG_UPDATE_GOLD_STORAGE: u16 = 0x0141;

    /// `GAME_SMSG_ITEM_STREAM_CREATE` (declared maximum 5 bytes; header `0x0144`).
    pub const GAME_SMSG_ITEM_STREAM_CREATE: u16 = 0x0144;

    /// `GAME_SMSG_ITEM_STREAM_DESTROY` (declared maximum 4 bytes; header `0x0145`).
    pub const GAME_SMSG_ITEM_STREAM_DESTROY: u16 = 0x0145;

    /// `GAME_SMSG_ITEM_WEAPON_SET` (declared maximum 13 bytes; header `0x0147`).
    pub const GAME_SMSG_ITEM_WEAPON_SET: u16 = 0x0147;

    /// `GAME_SMSG_ITEM_SET_ACTIVE_WEAPON_SET` (declared maximum 5 bytes; header `0x0148`).
    pub const GAME_SMSG_ITEM_SET_ACTIVE_WEAPON_SET: u16 = 0x0148;

    /// `GAME_SMSG_ITEM_CHANGE_LOCATION` (declared maximum 11 bytes; header `0x014B`).
    pub const GAME_SMSG_ITEM_CHANGE_LOCATION: u16 = 0x014B;

    /// `GAME_SMSG_ITEM_REMOVE` (declared maximum 8 bytes; header `0x014D`).
    pub const GAME_SMSG_ITEM_REMOVE: u16 = 0x014D;

    /// `GAME_SMSG_GOLD_CHARACTER_REMOVE` (declared maximum 8 bytes; header `0x014F`).
    pub const GAME_SMSG_GOLD_CHARACTER_REMOVE: u16 = 0x014F;

    /// `GAME_SMSG_GOLD_STORAGE_REMOVE` (declared maximum 8 bytes; header `0x0150`).
    pub const GAME_SMSG_GOLD_STORAGE_REMOVE: u16 = 0x0150;

    /// `GAME_SMSG_TOME_SHOW_SKILLS` (declared maximum 8 bytes; header `0x0154`).
    pub const GAME_SMSG_TOME_SHOW_SKILLS: u16 = 0x0154;

    /// `GAME_SMSG_ITEM_SET_PROFESSION` (declared maximum 7 bytes; header `0x015A`).
    pub const GAME_SMSG_ITEM_SET_PROFESSION: u16 = 0x015A;

    /// `GAME_SMSG_CREATE_UNNAMED_ITEM` (declared maximum 25 bytes; header `0x015F`).
    pub const GAME_SMSG_CREATE_UNNAMED_ITEM: u16 = 0x015F;

    /// `GAME_SMSG_CREATE_NAMED_ITEM` (declared maximum 425 bytes; header `0x0161`).
    pub const GAME_SMSG_CREATE_NAMED_ITEM: u16 = 0x0161;

    /// `GAME_SMSG_ITEM_UPDATE` (declared maximum 425 bytes; header `0x0162`).
    pub const GAME_SMSG_ITEM_UPDATE: u16 = 0x0162;

    /// `GAME_SMSG_ITEM_SALVAGE_SESSION_START` (declared maximum 24 bytes; header `0x0163`).
    pub const GAME_SMSG_ITEM_SALVAGE_SESSION_START: u16 = 0x0163;

    /// `GAME_SMSG_ITEM_SALVAGE_SESSION_CANCEL` (declared maximum 2 bytes; header `0x0164`).
    pub const GAME_SMSG_ITEM_SALVAGE_SESSION_CANCEL: u16 = 0x0164;

    /// `GAME_SMSG_ITEM_SALVAGE_SESSION_DONE` (declared maximum 2 bytes; header `0x0165`).
    pub const GAME_SMSG_ITEM_SALVAGE_SESSION_DONE: u16 = 0x0165;

    /// `GAME_SMSG_ITEM_SALVAGE_SESSION_SUCCESS` (declared maximum 2 bytes; header `0x0166`).
    pub const GAME_SMSG_ITEM_SALVAGE_SESSION_SUCCESS: u16 = 0x0166;

    /// `GAME_SMSG_ITEM_SALVAGE_SESSION_ITEM_KEPT` (declared maximum 3 bytes; header `0x0167`).
    pub const GAME_SMSG_ITEM_SALVAGE_SESSION_ITEM_KEPT: u16 = 0x0167;

    /// `GAME_SMSG_INSTANCE_SHOW_WIN` (declared maximum 2 bytes; header `0x017B`).
    pub const GAME_SMSG_INSTANCE_SHOW_WIN: u16 = 0x017B;

    /// `GAME_SMSG_INSTANCE_LOAD_HEAD` (declared maximum 6 bytes; header `0x017C`).
    pub const GAME_SMSG_INSTANCE_LOAD_HEAD: u16 = 0x017C;

    /// `GAME_SMSG_INSTANCE_LOAD_PLAYER_NAME` (declared maximum 46 bytes; header `0x017D`).
    pub const GAME_SMSG_INSTANCE_LOAD_PLAYER_NAME: u16 = 0x017D;

    /// `GAME_SMSG_INSTANCE_COUNTDOWN_STOP` (declared maximum 2 bytes; header `0x017E`).
    pub const GAME_SMSG_INSTANCE_COUNTDOWN_STOP: u16 = 0x017E;

    /// `GAME_SMSG_INSTANCE_COUNTDOWN` (declared maximum 259 bytes; header `0x0180`).
    pub const GAME_SMSG_INSTANCE_COUNTDOWN: u16 = 0x0180;

    /// `GAME_SMSG_INSTANCE_PLAYER_DATA_START` (declared maximum 2 bytes; header `0x0186`).
    pub const GAME_SMSG_INSTANCE_PLAYER_DATA_START: u16 = 0x0186;

    /// `GAME_SMSG_CHAR_CREATION_SUCCESS` (declared maximum 1092 bytes; header `0x0188`).
    pub const GAME_SMSG_CHAR_CREATION_SUCCESS: u16 = 0x0188;

    /// `GAME_SMSG_CHAR_CREATION_START` (declared maximum 2 bytes; header `0x0189`).
    pub const GAME_SMSG_CHAR_CREATION_START: u16 = 0x0189;

    /// `GAME_SMSG_INSTANCE_PLAYER_DATA_DONE` (declared maximum 2 bytes; header `0x018A`).
    pub const GAME_SMSG_INSTANCE_PLAYER_DATA_DONE: u16 = 0x018A;

    /// `GAME_SMSG_CHAR_CREATION_ERROR` (declared maximum 6 bytes; header `0x018B`).
    pub const GAME_SMSG_CHAR_CREATION_ERROR: u16 = 0x018B;

    /// `GAME_SMSG_INSTANCE_LOAD_FINISH` (declared maximum 2 bytes; header `0x018E`).
    pub const GAME_SMSG_INSTANCE_LOAD_FINISH: u16 = 0x018E;

    /// `GAME_SMSG_JUMBO_MESSAGE` (declared maximum 7 bytes; header `0x0190`).
    pub const GAME_SMSG_JUMBO_MESSAGE: u16 = 0x0190;

    /// `GAME_SMSG_INSTANCE_REDIRECT` (declared maximum 4 bytes; header `0x0191`).
    pub const GAME_SMSG_INSTANCE_REDIRECT: u16 = 0x0191;

    /// `GAME_SMSG_INSTANCE_LOAD_SPAWN_POINT` (declared maximum 26 bytes; header `0x0196`).
    pub const GAME_SMSG_INSTANCE_LOAD_SPAWN_POINT: u16 = 0x0196;

    /// `GAME_SMSG_INSTANCE_MANIFEST_DATA` (declared maximum 1030 bytes; header `0x0197`).
    pub const GAME_SMSG_INSTANCE_MANIFEST_DATA: u16 = 0x0197;

    /// `GAME_SMSG_INSTANCE_MANIFEST_DONE` (declared maximum 9 bytes; header `0x0198`).
    pub const GAME_SMSG_INSTANCE_MANIFEST_DONE: u16 = 0x0198;

    /// `GAME_SMSG_INSTANCE_MANIFEST_PHASE` (declared maximum 3 bytes; header `0x0199`).
    pub const GAME_SMSG_INSTANCE_MANIFEST_PHASE: u16 = 0x0199;

    /// `GAME_SMSG_INSTANCE_LOAD_INFO` (declared maximum 16 bytes; header `0x019A`).
    pub const GAME_SMSG_INSTANCE_LOAD_INFO: u16 = 0x019A;

    /// `GAME_SMSG_CREATE_MISSION_PROGRESS` (declared maximum 516 bytes; header `0x01A1`).
    pub const GAME_SMSG_CREATE_MISSION_PROGRESS: u16 = 0x01A1;

    /// `GAME_SMSG_UPDATE_MISSION_PROGRESS` (declared maximum 20 bytes; header `0x01A3`).
    pub const GAME_SMSG_UPDATE_MISSION_PROGRESS: u16 = 0x01A3;

    /// `GAME_SMSG_TRANSFER_GAME_SERVER_INFO` (declared maximum 46 bytes; header `0x01A6`).
    pub const GAME_SMSG_TRANSFER_GAME_SERVER_INFO: u16 = 0x01A6;

    /// `GAME_SMSG_READY_FOR_MAP_SPAWN` (declared maximum 6 bytes; header `0x01AC`).
    pub const GAME_SMSG_READY_FOR_MAP_SPAWN: u16 = 0x01AC;

    /// `GAME_SMSG_DOA_COMPLETE_ZONE` (declared maximum 250 bytes; header `0x01B0`).
    pub const GAME_SMSG_DOA_COMPLETE_ZONE: u16 = 0x01B0;

    /// `GAME_SMSG_CHARACTER_UDPATE_PARTY` (declared maximum 5 bytes; header `0x01B3`).
    pub const GAME_SMSG_CHARACTER_UDPATE_PARTY: u16 = 0x01B3;

    /// `GAME_SMSG_INSTANCE_TRAVEL_TIMER` (declared maximum 253 bytes; header `0x01BC`).
    pub const GAME_SMSG_INSTANCE_TRAVEL_TIMER: u16 = 0x01BC;

    /// `GAME_SMSG_INSTANCE_TRAVEL_FAILURE` (declared maximum 3 bytes; header `0x01BD`).
    pub const GAME_SMSG_INSTANCE_TRAVEL_FAILURE: u16 = 0x01BD;

    /// `GAME_SMSG_PARTY_SET_DIFFICULTY` (declared maximum 3 bytes; header `0x01BF`).
    pub const GAME_SMSG_PARTY_SET_DIFFICULTY: u16 = 0x01BF;

    /// `GAME_SMSG_PARTY_HENCHMAN_ADD` (declared maximum 52 bytes; header `0x01C0`).
    pub const GAME_SMSG_PARTY_HENCHMAN_ADD: u16 = 0x01C0;

    /// `GAME_SMSG_PARTY_HENCHMAN_REMOVE` (declared maximum 6 bytes; header `0x01C1`).
    pub const GAME_SMSG_PARTY_HENCHMAN_REMOVE: u16 = 0x01C1;

    /// `GAME_SMSG_PARTY_HERO_ADD` (declared maximum 10 bytes; header `0x01C3`).
    pub const GAME_SMSG_PARTY_HERO_ADD: u16 = 0x01C3;

    /// `GAME_SMSG_PARTY_HERO_REMOVE` (declared maximum 8 bytes; header `0x01C4`).
    pub const GAME_SMSG_PARTY_HERO_REMOVE: u16 = 0x01C4;

    /// `GAME_SMSG_PARTY_INVITE_ADD` (declared maximum 4 bytes; header `0x01C5`).
    pub const GAME_SMSG_PARTY_INVITE_ADD: u16 = 0x01C5;

    /// `GAME_SMSG_PARTY_JOIN_REQUEST` (declared maximum 4 bytes; header `0x01C6`).
    pub const GAME_SMSG_PARTY_JOIN_REQUEST: u16 = 0x01C6;

    /// `GAME_SMSG_PARTY_INVITE_CANCEL` (declared maximum 4 bytes; header `0x01C7`).
    pub const GAME_SMSG_PARTY_INVITE_CANCEL: u16 = 0x01C7;

    /// `GAME_SMSG_PARTY_REQUEST_CANCEL` (declared maximum 4 bytes; header `0x01C8`).
    pub const GAME_SMSG_PARTY_REQUEST_CANCEL: u16 = 0x01C8;

    /// `GAME_SMSG_PARTY_REQUEST_RESPONSE` (declared maximum 4 bytes; header `0x01C9`).
    pub const GAME_SMSG_PARTY_REQUEST_RESPONSE: u16 = 0x01C9;

    /// `GAME_SMSG_PARTY_INVITE_RESPONSE` (declared maximum 4 bytes; header `0x01CA`).
    pub const GAME_SMSG_PARTY_INVITE_RESPONSE: u16 = 0x01CA;

    /// `GAME_SMSG_PARTY_YOU_ARE_LEADER` (declared maximum 3 bytes; header `0x01CB`).
    pub const GAME_SMSG_PARTY_YOU_ARE_LEADER: u16 = 0x01CB;

    /// `GAME_SMSG_PARTY_PLAYER_ADD` (declared maximum 7 bytes; header `0x01CC`).
    pub const GAME_SMSG_PARTY_PLAYER_ADD: u16 = 0x01CC;

    /// `GAME_SMSG_PARTY_PLAYER_REMOVE` (declared maximum 6 bytes; header `0x01D1`).
    pub const GAME_SMSG_PARTY_PLAYER_REMOVE: u16 = 0x01D1;

    /// `GAME_SMSG_PARTY_PLAYER_READY` (declared maximum 7 bytes; header `0x01D2`).
    pub const GAME_SMSG_PARTY_PLAYER_READY: u16 = 0x01D2;

    /// `GAME_SMSG_PARTY_CREATE` (declared maximum 4 bytes; header `0x01D3`).
    pub const GAME_SMSG_PARTY_CREATE: u16 = 0x01D3;

    /// `GAME_SMSG_PARTY_MEMBER_STREAM_END` (declared maximum 4 bytes; header `0x01D4`).
    pub const GAME_SMSG_PARTY_MEMBER_STREAM_END: u16 = 0x01D4;

    /// `GAME_SMSG_PARTY_DEFEATED` (declared maximum 2 bytes; header `0x01D9`).
    pub const GAME_SMSG_PARTY_DEFEATED: u16 = 0x01D9;

    /// `GAME_SMSG_PARTY_LOCK` (declared maximum 252 bytes; header `0x01DA`).
    pub const GAME_SMSG_PARTY_LOCK: u16 = 0x01DA;

    /// `GAME_SMSG_PARTY_SEARCH_REQUEST_JOIN` (declared maximum 6 bytes; header `0x01DD`).
    pub const GAME_SMSG_PARTY_SEARCH_REQUEST_JOIN: u16 = 0x01DD;

    /// `GAME_SMSG_PARTY_SEARCH_REQUEST_DONE` (declared maximum 6 bytes; header `0x01DE`).
    pub const GAME_SMSG_PARTY_SEARCH_REQUEST_DONE: u16 = 0x01DE;

    /// `GAME_SMSG_PARTY_SEARCH_ADVERTISEMENT` (declared maximum 130 bytes; header `0x01DF`).
    pub const GAME_SMSG_PARTY_SEARCH_ADVERTISEMENT: u16 = 0x01DF;

    /// `GAME_SMSG_PARTY_SEARCH_SEEK` (declared maximum 4 bytes; header `0x01E0`).
    pub const GAME_SMSG_PARTY_SEARCH_SEEK: u16 = 0x01E0;

    /// `GAME_SMSG_PARTY_SEARCH_REMOVE` (declared maximum 4 bytes; header `0x01E1`).
    pub const GAME_SMSG_PARTY_SEARCH_REMOVE: u16 = 0x01E1;

    /// `GAME_SMSG_PARTY_SEARCH_SIZE` (declared maximum 6 bytes; header `0x01E2`).
    pub const GAME_SMSG_PARTY_SEARCH_SIZE: u16 = 0x01E2;

    /// `GAME_SMSG_PARTY_SEARCH_TYPE` (declared maximum 6 bytes; header `0x01E3`).
    pub const GAME_SMSG_PARTY_SEARCH_TYPE: u16 = 0x01E3;

    /// Every game row, in table order.
    pub fn rows() -> impl Iterator<Item = &'static Entry> {
        super::rows(Conn::Game)
    }

    /// The row for a header in one direction, or `None` if the game family has no such message.
    ///
    /// The direction is part of the key because the auth family has headers with the same value:
    /// `0x8004` is `AUTH_CMSG_ACCOUNT_LOGIN` on the auth connection and a different message on
    /// the world connection, with a different size.
    pub fn lookup(header: u16, from_client: bool) -> Option<&'static Entry> {
        packet_lookup(Conn::Game, header, from_client)
    }

    /// The declared maximum size for a header in one direction, or `None` if it is not a game
    /// header at all.
    ///
    /// A maximum, not the wire size: never read this many bytes to consume a message whose layout
    /// has a counted field. Framing walks that structurally (`client::frame`).
    pub fn expected_size(header: u16, from_client: bool) -> Option<usize> {
        lookup(header, from_client).map(|entry| entry.size)
    }

    /// The field layout for a header in one direction, or `None` if it is not a game header.
    ///
    /// The layout is what a decoder walks: the counted lengths inside the body decide where the
    /// next field starts, and the header alone does not.
    pub fn fields(header: u16, from_client: bool) -> Option<&'static [Field]> {
        lookup(header, from_client).map(|entry| entry.fields)
    }

    // The names below were given to their rows during the identification passes of 2026-11-01; the row
    // carries the evidence, this block only saves a reader from typing the header.

    /// `GAME_SMSG_AGENT_UPDATE_FLAGS` (`0x0026`).
    pub const GAME_SMSG_AGENT_UPDATE_FLAGS: u16 = 0x0026;
    /// `GAME_SMSG_UPDATE_AGENT_VISUAL_EQUIPMENT_SLOT` (`0x006F`).
    pub const GAME_SMSG_UPDATE_AGENT_VISUAL_EQUIPMENT_SLOT: u16 = 0x006F;
    /// `GAME_SMSG_MAP_INIT_START` (`0x008B`).
    pub const GAME_SMSG_MAP_INIT_START: u16 = 0x008B;
    /// `GAME_SMSG_MAP_MARKER` (`0x008D`).
    pub const GAME_SMSG_MAP_MARKER: u16 = 0x008D;
    /// `GAME_SMSG_AGENT_UPDATE_MORALE` (`0x009C`).
    pub const GAME_SMSG_AGENT_UPDATE_MORALE: u16 = 0x009C;
    /// `GAME_SMSG_PREFERENCES_BLOB` (`0x00B5`).
    pub const GAME_SMSG_PREFERENCES_BLOB: u16 = 0x00B5;
    /// `GAME_SMSG_CHAR_VISIBILITY_FLAGS` (`0x00EF`).
    pub const GAME_SMSG_CHAR_VISIBILITY_FLAGS: u16 = 0x00EF;
    /// `GAME_SMSG_GADGET_SET_FLAGS` (`0x0115`).
    pub const GAME_SMSG_GADGET_SET_FLAGS: u16 = 0x0115;
    /// `GAME_SMSG_TOWN_ALLIANCE_COMMIT` (`0x011B`).
    pub const GAME_SMSG_TOWN_ALLIANCE_COMMIT: u16 = 0x011B;
    /// `GAME_SMSG_TOWN_ALLIANCE_END` (`0x011C`).
    pub const GAME_SMSG_TOWN_ALLIANCE_END: u16 = 0x011C;
    /// `GAME_SMSG_LOCATION_TAG` (`0x012E`).
    pub const GAME_SMSG_LOCATION_TAG: u16 = 0x012E;
    /// `GAME_SMSG_ITEM_LOW_DETAIL` (`0x015E`).
    pub const GAME_SMSG_ITEM_LOW_DETAIL: u16 = 0x015E;
    /// `GAME_SMSG_LATENCY_ENABLE` (`0x016E`).
    pub const GAME_SMSG_LATENCY_ENABLE: u16 = 0x016E;
    /// `GAME_SMSG_INSTANCE_TABLE` (`0x01A0`).
    pub const GAME_SMSG_INSTANCE_TABLE: u16 = 0x01A0;
    /// `GAME_CMSG_COMPUTER_INFO` (`0x800A`).
    pub const GAME_CMSG_COMPUTER_INFO: u16 = 0x800A;
    /// `GAME_CMSG_COMPUTER_HASH` (`0x800B`).
    pub const GAME_CMSG_COMPUTER_HASH: u16 = 0x800B;
    /// `GAME_CMSG_INSTANCE_QUERY` (`0x8093`).
    /// `GAME_SMSG_PLAYER_UPDATE_FLAGS` (`0x003C`).
    pub const GAME_SMSG_PLAYER_UPDATE_FLAGS: u16 = 0x003C;
    pub const GAME_CMSG_INSTANCE_QUERY: u16 = 0x8093;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_auth_row_recomputes_to_its_recorded_size() {
        for packet in rows(Conn::Auth) {
            let variable = auth::VARIABLE_LENGTH_HEADERS.contains(&packet.header);
            let mut known = 0usize;
            let mut has_variable = false;
            for field in packet.fields {
                match field.wire_size() {
                    Some(width) => known += width,
                    None => has_variable = true,
                }
            }
            assert_eq!(
                has_variable, variable,
                "{:#06X}: the table and the layout disagree about variability",
                packet.header
            );
            if !variable {
                assert_eq!(
                    packet.size,
                    known,
                    "{:#06X} ({}): declared size is not the sum of its fields",
                    packet.header,
                    packet.name.unwrap_or("unnamed")
                );
            }
        }
    }

    #[test]
    fn every_game_row_recomputes_to_its_declared_maximum() {
        for packet in rows(Conn::Game) {
            let exempt = game::SIZE_NOT_RECOMPUTABLE_FROM_LAYOUT
                .contains(&(packet.header, packet.from_client));
            if exempt {
                continue;
            }
            assert_eq!(
                declared_max_size(packet.fields),
                packet.size,
                "{:#06X} ({}): declared maximum is not what its layout computes",
                packet.header,
                packet.label()
            );
        }
    }

    #[test]
    fn the_table_is_sorted_and_the_keys_are_unique() {
        let keys: Vec<_> = PACKETS
            .iter()
            .map(|packet| key_of(packet.conn, packet.header, packet.from_client))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(
            keys, sorted,
            "the table is not sorted for the binary search"
        );
        let mut unique = sorted.clone();
        unique.dedup();
        assert_eq!(
            unique.len(),
            sorted.len(),
            "a (conn, direction, header) is in the table twice"
        );
    }

    #[test]
    fn every_header_agrees_with_its_index() {
        for packet in PACKETS {
            let mask = if packet.from_client { 0x8000 } else { 0x0000 };
            assert_eq!(
                packet.header,
                mask | packet.index,
                "{:#06X}: the header and the index disagree",
                packet.header
            );
        }
    }

    #[test]
    fn the_named_headers_are_unique_within_a_family() {
        // Two names on one header exist once in the reference (`game::AMBIGUOUS_NAME_HEADERS`),
        // and the alias constant is the second. Two *rows* with one name would be a real defect.
        for conn in [Conn::Auth, Conn::Game] {
            let mut names: Vec<_> = rows(conn).filter_map(|packet| packet.name).collect();
            names.sort_unstable();
            let before = names.len();
            names.dedup();
            assert_eq!(names.len(), before, "{} rows share a name", conn.label());
        }
    }

    #[test]
    fn lookup_finds_every_row_and_nothing_else() {
        for packet in PACKETS {
            let found =
                lookup(packet.conn, packet.header, packet.from_client).expect("its own row");
            assert_eq!(found.header, packet.header);
            assert_eq!(found.size, packet.size);
        }
        assert!(lookup(Conn::Auth, 0x1234, true).is_none());
        assert!(lookup(Conn::Game, 0x1234, true).is_none());
    }

    /// The portal touch: the row that makes a client's travel announcement frame instead of ending
    /// the session. Pinned because the *direction* is the whole mechanism - a server-to-client row
    /// would leave the client's message unnameable, and `Frame::decode_game_message(.., true)` would
    /// refuse it as an unknown header.
    ///
    /// Evidence: `recordings/client/feature/movement/movement-sniff.log` seq 1651, a real client
    /// sending `c1 00 00 00 | 3a 00 00 00 | 00 00 00 00`.
    #[test]
    fn the_portal_touch_is_a_client_message_and_is_named() {
        let packet =
            lookup(Conn::Game, game::GAME_CMSG_TRAVEL_INIT, true).expect("client-to-server");
        assert_eq!(packet.name, Some("GAME_CMSG_TRAVEL_INIT"));
        assert_eq!(packet.size, 10, "two agent ids after a two-byte header");
        assert!(packet.from_client);
    }

    #[test]
    fn the_progress_numbers_add_up() {
        for conn in [Conn::Auth, Conn::Game] {
            let total: usize = progress(conn).iter().map(|(_, count)| count).sum();
            assert_eq!(
                total,
                rows(conn).count(),
                "{} rows are not all counted",
                conn.label()
            );
        }
    }
}
