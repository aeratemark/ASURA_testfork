#!/usr/bin/env python3
"""One-off migration: merge the two generated message tables into one static table.

**Do not run this again.** It is the record of how `packets.rs` was produced, and the two input
files it reads no longer exist. From now on `crates/asura-protocol/src/packets.rs` is the
implementation: the fact columns are copied from the reference and edited only against a
measurement, and the `status`/`evidence` columns are edited by hand as sessions learn.

Inputs (both generated, both being deleted by this migration):

    crates/asura-protocol/src/client/tables.rs        the auth family, 100 rows
    crates/asura-protocol/src/client/game_tables.rs   the game family, 682 rows

Output:

    crates/asura-protocol/src/packets.rs   one table, one row per packet, both families

The script does not retype a single number. It parses the two files, emits the merged
module, then parses its own output back and asserts that every (conn, header, index,
size, name, from_client, fields) tuple and every named-header constant is identical to
what the inputs declared. A mismatch aborts with a non-zero exit.

Status column seed, all derived from the tree, none invented:

    Unnamed   the reference (`opcodes.h`) has no name for the header
    Handled   a header the service actually dispatches: the `answer` arms in
              `crates/asura-world/src/lib.rs` (game, client-to-server) and the
              `tables::AUTH_CMSG_*` arms in `crates/asura-auth/src/lib.rs` (auth)
    Emitted   a server-to-client header the service sends: the builder constants in
              `crates/asura-protocol/src/game/mod.rs` (game), and `AUTH_SMSG_*` references
              in `client/messages.rs` (auth)
    Known     named by the reference, nothing proven about it yet

`evidence` starts empty on every row: no capture has been attached to a row yet. That
column is the one to fill as each packet is decoded, and it is the only column a session
should edit by hand.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
AUTH_PATH = ROOT / "crates/asura-protocol/src/client/tables.rs"
GAME_PATH = ROOT / "crates/asura-protocol/src/client/game_tables.rs"
OUT_PATH = ROOT / "crates/asura-protocol/src/packets.rs"

ROW_RE = re.compile(
    r"^    Entry \{ header: (0x[0-9A-Fa-f]{4}), index:\s*(\d+), size:\s*(\d+), "
    r"name: (Some\(\"([A-Za-z0-9_]+)\"\)|None), from_client: (true|false), "
    r"fields: &\[(.*)\] \},$"
)
FIELD_RE = re.compile(r"Field \{ kind: FieldKind::(\w+), length: (\d+) \}")
CONST_RE = re.compile(r"^pub const ([A-Za-z0-9_]+): u16 = (0x[0-9A-Fa-f]{4});$")
DOC_RE = re.compile(r"^/// ?(.*)$")
LIST_CONST_RE = re.compile(r"^pub const (VARIABLE_LENGTH_HEADERS|SIZE_NOT_RECOMPUTABLE_FROM_LAYOUT|AMBIGUOUS_NAME_HEADERS): ([^=]+)= (.*);$")

OUT_ROW_RE = re.compile(
    r"^    Packet \{ conn: Conn::(\w+), header: (0x[0-9A-Fa-f]{4}), index:\s*(\d+), "
    r"size:\s*(\d+), name: (Some\(\"([A-Za-z0-9_]+)\"\)|None), from_client: (true|false), "
    r"status: Status::(\w+), evidence: &\[(.*)\], fields: &\[(.*)\] \},$"
)

MASKS = {"AUTH_CMSG_MASK", "AUTH_SMSG_MASK", "GAME_CMSG_MASK", "GAME_SMSG_MASK"}
KINDS = [
    "MsgHeader",
    "MsgHeaderGame",
    "AgentId",
    "Float",
    "Vect2",
    "Vect3",
    "Byte",
    "Word",
    "Dword",
    "Blob",
    "String16",
    "Array8",
    "Array16",
    "Array32",
    "NestedStruct",
]


def die(message):
    print(f"merge-packet-tables: {message}", file=sys.stderr)
    raise SystemExit(1)


def parse(path):
    """Rows in source order, named-header constants in source order, and the special lists."""
    rows, consts, lists = [], [], {}
    doc = []
    for line in path.read_text(encoding="utf-8").splitlines():
        match = ROW_RE.match(line)
        if match:
            fields = [
                {"kind": kind, "length": int(length)}
                for kind, length in FIELD_RE.findall(match.group(7))
            ]
            rows.append(
                {
                    "header": int(match.group(1), 16),
                    "index": int(match.group(2)),
                    "size": int(match.group(3)),
                    "name": match.group(5) if match.group(4) != "None" else None,
                    "from_client": match.group(6) == "true",
                    "fields": fields,
                }
            )
            doc = []
            continue
        match = CONST_RE.match(line)
        if match and match.group(1) not in MASKS:
            consts.append({"doc": doc, "name": match.group(1), "value": int(match.group(2), 16)})
            doc = []
            continue
        match = LIST_CONST_RE.match(line)
        if match:
            lists[match.group(1)] = {"type": match.group(2).rstrip(), "value": match.group(3)}
            doc = []
            continue
        match = DOC_RE.match(line)
        if match:
            doc.append(match.group(1))
        elif not line.strip():
            doc = []
    return rows, consts, lists


def scan(path, pattern):
    if not path.exists():
        return set()
    return set(match.group(1) for match in re.finditer(pattern, path.read_text(encoding="utf-8"), re.MULTILINE))


def main():
    auth_rows, auth_consts, auth_lists = parse(AUTH_PATH)
    game_rows, game_consts, game_lists = parse(GAME_PATH)

    if len(auth_rows) != 100 or len(game_rows) != 682:
        die(f"expected 100 auth rows and 682 game rows, read {len(auth_rows)} and {len(game_rows)}")

    # One constant per named row, plus the one alias `opcodes.h` gives a second name to
    # (`GAME_SMSG_SKILL_ACTIVATED` beside `GAME_SMSG_SKILL_CANCEL`, both `0x00E3`).
    auth_named = sum(1 for row in auth_rows if row["name"])
    game_named = sum(1 for row in game_rows if row["name"])
    if len(auth_consts) != auth_named:
        die(f"expected {auth_named} auth constants, read {len(auth_consts)}")
    if len(game_consts) != game_named + 1:
        die(f"expected {game_named + 1} game constants ({game_named} names + 1 alias), read {len(game_consts)}")

    # ---- the status seed, derived from the tree ------------------------------------------
    handled_game = {
        int(value, 16)
        for value in scan(ROOT / "crates/asura-world/src/lib.rs", r"0x([0-9A-Fa-f]{4})\s*=>")
    }
    handled_auth = {
        name
        for name in scan(ROOT / "crates/asura-auth/src/lib.rs", r"tables::(AUTH_CMSG_[A-Z0-9_]+)")
    }
    emitted_game = {
        int(value, 16)
        for value in scan(ROOT / "crates/asura-protocol/src/game/mod.rs", r"^pub const [A-Z_0-9]+: u16 = (0x[0-9A-Fa-f]{4});")
    }
    # These two are emitted by the world through `client::TRANSFER_GAME_SERVER_INFO` and
    # `client::MAP_UPDATE_CURRENT`, which are not literals in `game/mod.rs`.
    emitted_game |= {0x01A6, 0x0099}
    emitted_auth = scan(ROOT / "crates/asura-protocol/src/client/messages.rs", r"(AUTH_SMSG_[A-Z0-9_]+)")

    auth_headers = {row["header"] for row in auth_rows}
    game_inbound = {row["header"] for row in game_rows if row["from_client"]}
    game_outbound = {row["header"] for row in game_rows if not row["from_client"]}

    unknown = handled_game - game_inbound
    if unknown:
        die(f"client-to-server dispatch names headers the game table does not have: {sorted(hex(h) for h in unknown)}")
    auth_names = {row["name"] for row in auth_rows if row["from_client"] and row["name"]}
    missing = handled_auth - auth_names
    if missing:
        die(f"auth dispatch names headers the auth table does not have: {sorted(missing)}")
    if emitted_game - game_outbound:
        die(f"emitted game headers missing from the table: {sorted(hex(h) for h in emitted_game - game_outbound)}")

    def status(conn, row):
        if row["name"] is None:
            return "Unnamed"
        if conn == "Game" and row["from_client"] and row["header"] in handled_game:
            return "Handled"
        if conn == "Auth" and row["from_client"] and row["name"] in handled_auth:
            return "Handled"
        if conn == "Game" and not row["from_client"] and row["header"] in emitted_game:
            return "Emitted"
        if conn == "Auth" and not row["from_client"] and row["name"] in emitted_auth:
            return "Emitted"
        return "Known"

    def sort_key(conn, row):
        return (0 if conn == "Auth" else 1, 0 if row["from_client"] else 1, row["header"])

    rows = [(conn, row) for conn, source in (("Auth", auth_rows), ("Game", game_rows)) for row in source]
    rows.sort(key=lambda item: sort_key(*item))

    for (conn, row) in rows:
        assert row["header"] in (auth_headers if conn == "Auth" else game_inbound | game_outbound)

    def format_row(conn, row):
        fields = ", ".join(
            f"Field {{ kind: FieldKind::{field['kind']}, length: {field['length']} }}"
            for field in row["fields"]
        )
        name = f'Some("{row["name"]}")' if row["name"] else "None"
        return (
            f"    Packet {{ conn: Conn::{conn}, header: 0x{row['header']:04X}, "
            f"index: {row['index']:>3}, size: {row['size']:>5}, name: {name}, "
            f"from_client: {'true' if row['from_client'] else 'false'}, "
            f"status: Status::{status(conn, row)}, evidence: &[], fields: &[{fields}] }},"
        )

    counts = {"Auth": {"ClientToServer": 0, "ServerToClient": 0}, "Game": {"ClientToServer": 0, "ServerToClient": 0}}
    by_status = {name: 0 for name in ("Unnamed", "Known", "Observed", "Emitted", "Handled", "Inferred")}
    for conn, row in rows:
        counts[conn]["ClientToServer" if row["from_client"] else "ServerToClient"] += 1
        by_status[status(conn, row)] += 1

    def format_consts(consts):
        out = []
        for const in consts:
            for line in const["doc"]:
                out.append(f"/// {line}".rstrip())
            out.append(f"pub const {const['name']}: u16 = 0x{const['value']:04X};")
            out.append("")
        return "\n".join(out).rstrip()

    table = "\n".join(format_row(conn, row) for conn, row in rows)

    module = MODULE.replace("__ROWS__", table)
    module = module.replace("__AUTH_CONSTS__", format_consts(auth_consts))
    module = module.replace("__GAME_CONSTS__", format_consts(game_consts))
    module = module.replace(
        "__AUTH_VARIABLE__",
        f"pub const VARIABLE_LENGTH_HEADERS: &[u16] = {auth_lists['VARIABLE_LENGTH_HEADERS']['value']};",
    )
    module = module.replace(
        "__GAME_AMBIGUOUS__",
        "pub const AMBIGUOUS_NAME_HEADERS: &[(u16, bool, &str, &str)] = "
        + game_lists["AMBIGUOUS_NAME_HEADERS"]["value"]
        + ";",
    )
    module = module.replace(
        "__GAME_NOT_RECOMPUTABLE__",
        "pub const SIZE_NOT_RECOMPUTABLE_FROM_LAYOUT: &[(u16, bool)] = "
        + game_lists["SIZE_NOT_RECOMPUTABLE_FROM_LAYOUT"]["value"]
        + ";",
    )
    module = module.replace("__TOTAL__", str(len(rows)))
    module = module.replace("__AUTH_TOTAL__", str(len(auth_rows)))
    module = module.replace("__GAME_TOTAL__", str(len(game_rows)))
    module = module.replace(
        "__COUNTS__",
        f"auth {counts['Auth']['ClientToServer']} client-to-server + {counts['Auth']['ServerToClient']} server-to-client; "
        f"game {counts['Game']['ClientToServer']} + {counts['Game']['ServerToClient']}",
    )

    # ---- write, then verify the output against the inputs ---------------------------------
    OUT_PATH.write_text(module, encoding="utf-8")
    verify(rows, auth_consts + game_consts, module)

    print(f"wrote {OUT_PATH.relative_to(ROOT)}")
    print(f"  rows: {len(rows)} (auth {len(auth_rows)}, game {len(game_rows)})")
    print(f"  constants: {len(auth_consts)} auth + {len(game_consts)} game")
    print("  status seed: " + ", ".join(f"{name} {count}" for name, count in by_status.items() if count))
    print("  handled game (client-to-server): " + ", ".join(sorted(hex(h) for h in handled_game)))
    print("  handled auth (client-to-server): " + ", ".join(sorted(n[len('AUTH_CMSG_'):] for n in handled_auth)))


def verify(rows, consts, module):
    """Re-parse the emitted module and compare it with the rows it was built from."""
    got_rows = []
    for line in module.splitlines():
        match = OUT_ROW_RE.match(line)
        if not match:
            continue
        fields = [
            {"kind": kind, "length": int(length)}
            for kind, length in FIELD_RE.findall(match.group(10))
        ]
        got_rows.append(
            (
                match.group(1),
                int(match.group(2), 16),
                int(match.group(3)),
                int(match.group(4)),
                match.group(6) if match.group(5) != "None" else None,
                match.group(7) == "true",
                tuple((field["kind"], field["length"]) for field in fields),
            )
        )

    want_rows = [
        (
            conn,
            row["header"],
            row["index"],
            row["size"],
            row["name"],
            row["from_client"],
            tuple((field["kind"], field["length"]) for field in row["fields"]),
        )
        for conn, row in rows
    ]
    if got_rows != want_rows:
        for position, (got, want) in enumerate(zip(got_rows, want_rows)):
            if got != want:
                die(f"row {position} differs after the round trip:\n  got  {got}\n  want {want}")
        die(f"row count differs: wrote {len(got_rows)}, expected {len(want_rows)}")

    got_consts = [
        (match.group(1), int(match.group(2), 16))
        for match in (
            CONST_RE.match(line) for line in module.splitlines()
        )
        if match and match.group(1) not in MASKS
    ]
    want_consts = [(const["name"], const["value"]) for const in consts]
    if got_consts != want_consts:
        missing = [c for c in want_consts if c not in got_consts]
        extra = [c for c in got_consts if c not in want_consts]
        die(f"constants differ after the round trip: missing {missing[:5]}, extra {extra[:5]}")
    if len({const["name"] for const in consts}) != len(consts):
        die("a header name is declared twice")

    print(f"  verified: {len(got_rows)} rows and {len(got_consts)} constants round-trip identically")


MODULE = r'''//! **Every packet, in one table.**
//!
//! One row per header of both protocol families: the auth family (the login connection) and the
//! game family (a world connection). `__TOTAL__` rows: __COUNTS__.
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

/// The one table. `__TOTAL__` rows, sorted by `(conn, client-to-server first, header)` so
/// [`lookup`] can binary-search it. One row per packet; no other list of headers exists.
#[rustfmt::skip]
pub static PACKETS: &[Packet] = &[
__ROWS__
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
        if let Some(slot) = counts.iter_mut().find(|(status, _)| *status == packet.status) {
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

    __AUTH_CONSTS__

    /// Headers whose declared size cannot be recomputed from their field layout, because a
    /// `TYPE_NESTED_STRUCT` makes the length depend on a count carried on the wire.
    ///
    /// One of them, `0x001A`, declares 32 bytes where its own layout can only occupy 31: the
    /// reference's maximum is not exactly its packer's output. It is recorded rather than guessed
    /// at (`docs/03-protocol.md`, open questions).
    __AUTH_VARIABLE__

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
    __GAME_AMBIGUOUS__

    /// Rows whose `size` does not follow from their own field layout: 2 of 682.
    ///
    /// The reference wrote these two rows with a two-byte array count where all 680 other rows use
    /// a four-byte one, so each declares two bytes less per counted field than
    /// [`declared_max_size`] recomputes. Both numbers are carried through as declared -- the
    /// reference is the authority for a message's declared maximum, and `max_size` is never the
    /// wire size in any case, so the difference is recorded rather than guessed at.
    __GAME_NOT_RECOMPUTABLE__

    // ---------------------------------------------------------------- named headers
    // The names are GWPS's own (`opcodes.h`), kept verbatim so a Rust message can be diffed
    // against a C one by name rather than by number. 340 of the 682 headers have one.
    //
    // A constant is an alias of a row's `header`, not a second declaration of it; see
    // `every_named_header_is_a_row`.

    __GAME_CONSTS__

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
                    packet.size, known,
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
        assert_eq!(keys, sorted, "the table is not sorted for the binary search");
        let mut unique = sorted.clone();
        unique.dedup();
        assert_eq!(unique.len(), sorted.len(), "a (conn, direction, header) is in the table twice");
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
            let found = lookup(packet.conn, packet.header, packet.from_client).expect("its own row");
            assert_eq!(found.header, packet.header);
            assert_eq!(found.size, packet.size);
        }
        assert!(lookup(Conn::Auth, 0x1234, true).is_none());
        assert!(lookup(Conn::Game, 0x1234, true).is_none());
    }

    #[test]
    fn the_progress_numbers_add_up() {
        for conn in [Conn::Auth, Conn::Game] {
            let total: usize = progress(conn).iter().map(|(_, count)| count).sum();
            assert_eq!(total, rows(conn).count(), "{} rows are not all counted", conn.label());
        }
    }
}
'''


if __name__ == "__main__":
    main()
