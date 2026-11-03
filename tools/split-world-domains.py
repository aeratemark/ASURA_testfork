#!/usr/bin/env python3
"""One-off: split the world server's monolith into domain modules, one file per noun.

**Do not run this again.** It reads `lib.rs` as its input, and `lib.rs` is now the facade it
wrote. It ran once, on the pre-split file; the domain modules are the implementation from here
on. `tools/widen-world-visibility.py` is the second half of the move (the items need a
visibility once they live in different modules), and `tools/check-world-split.py` proves no
item was lost.

`crates/asura-world/src/lib.rs` held every domain at once. This moves each top-level
item to the file named after the game noun it belongs to, and leaves `lib.rs` as the
facade: module list, re-exports, the crate error, and the tests.

No line of code is rewritten: every item is moved as text, with the doc comments and
attributes that sit above it. The compiler and the test suite are what prove the move.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LIB = ROOT / "crates/asura-world/src/lib.rs"
SRC = ROOT / "crates/asura-world/src"

# item -> destination module. Keys are the declaration's own name as the scanner sees it.
TARGETS = {
    # ---- agent: the agent record, standing, walk, movement trace -----------------------
    "struct Standing": "agent",
    "impl Standing": "agent",
    "enum Walk": "agent",
    "struct MoveStats": "agent",
    "fn move_trace_on": "agent",
    "const LIVE_CADENCE_STEP": "agent",
    "fn live_cadence_step": "agent",
    "fn densify": "agent",
    "fn waypoints_preview": "agent",
    "const AGENT_PROPERTY_VALUE64": "agent",
    # ---- player: the placed character and the blocks that describe it -------------------
    "fn player_block": "player",
    "const PLAYER_ID": "player",
    "const AGENT_ID": "player",
    "const PARTY_ID": "player",
    "const PLAYER_TEAM_TOKEN": "player",
    "const DEFAULT_PRIMARY_PROFESSION": "player",
    "const DEFAULT_UNLOCKED_PROFESSIONS": "player",
    # ---- map: the catalogue entry, the path graphs, the map-side blocks ----------------
    "struct MapEntry": "map",
    "impl MapEntry": "map",
    "struct PathGraphs": "map",
    "struct PathGraphsInner": "map",
    "impl PathGraphs": "map",
    "fn import_graph": "map",
    "fn find_path_blob": "map",
    "fn spawn_block": "map",
    "fn load_block": "map",
    "fn manifest": "map",
    "const MAP_TYPE_CHARACTER_CREATION": "map",
    "const MAP_TYPE_MAIN_TOWN": "map",
    "const MAP_TYPE_MAIN_EXPLORABLE": "map",
    "const MAP_ID_COUNT": "map",
    "const UNLOCKED_MAP_ZEROS": "map",
    "const PHASE_1": "map",
    "const PHASE_2": "map",
    "const PHASE_DONE": "map",
    # ---- party: everyone in the world, and how they are introduced ---------------------
    "struct World": "world",
    "struct Peer": "world",
    "struct Peers": "world",
    "impl Peers": "world",
    "struct PeerView": "world",
    "impl From<&Peer> for PeerView": "world",
    "fn introduction": "world",
    "fn farewell": "world",
    "impl World": "world",
    # ---- item: bags, items, and what a client may do with them -------------------------
    "fn items_block": "item",
    "fn inventory_block": "item",
    "fn named_item": "item",
    "fn drop_item": "item",
    "fn drop_messages": "item",
    "fn move_item": "item",
    "fn equip_item": "item",
    "fn equipment_frame": "item",
    "const ITEM_STREAM": "item",
    "const DROP_OFFSET": "item",
    # ---- chat: spoken lines, and the command table they arrive through -----------------
    "fn maps_answer": "chat",
    "struct MappedMaps": "chat",
    "fn alias_key": "chat",
    "struct Command": "chat",
    "const COMMANDS": "chat",
    "fn command": "chat",
    "fn travel_target": "chat",
    # ---- travel: leaving one instance for another -------------------------------------
    "fn teleport": "travel",
    "fn load_map_on_demand": "travel",
    "const DISTRICT_NUMBER": "travel",
    # ---- creation: the character-creation instance ------------------------------------
    "struct Creation": "creation",
    "impl Creation": "creation",
    "fn creation_inventory": "creation",
    "fn creation_equipment_rows": "creation",
    "fn reload": "creation",
    "const CREATION_ENERGY": "creation",
    "const CREATION_HEALTH": "creation",
    "const GM_ERROR_CHARACTER_NAME_ALREADY_EXIST": "creation",
    # ---- connection: one gameplay socket, from hello to goodbye -----------------------
    "const TICK_INTERVAL": "connection",
    "const PING_INTERVAL": "connection",
    "const PING_VALUE": "connection",
    "fn serve": "connection",
    "fn panic_message": "connection",
    "struct Session": "connection",
    "impl Session": "connection",
    "fn handle_connection": "connection",
    "fn run": "connection",
    "fn send": "connection",
    "struct Play": "connection",
    "impl Play": "connection",
    "struct Placement": "connection",
    "fn hex_preview": "connection",
    # ---- dispatch: one table from header to the domain that handles it ----------------
    "fn answer": "dispatch",
    # ---- stays at the root -------------------------------------------------------------
    "enum GameError": "lib",
    "impl core::fmt::Display for GameError": "lib",
    "impl std::error::Error for GameError": "lib",
    "impl From<io::Error> for GameError": "lib",
    "fn random_u32": "lib",
    "fn wire_u8": "lib",
    "fn wire_u16": "lib",
    "fn wire_u32": "lib",
    "mod tests": "lib",
    # the modules the crate already had: they stay declared at the root
    "mod agent": "lib",
    "mod graph_dump": "lib",
    "mod capture": "lib",
    "mod replay": "lib",
    "mod gwdat": "lib",
    "mod fa_compress": "lib",
    "mod fa_compress_tables": "lib",
    "mod map_data": "lib",
    "mod pathfind": "lib",
    "mod path_tables": "lib",
    "mod travel_targets": "lib",
    "mod default_equipment": "lib",
}

DECL = re.compile(
    r"^(pub |pub\(crate\) )?(async )?(fn|struct|enum|impl|mod|trait|const|static|type) (.*)$"
)


def signature(line):
    match = DECL.match(line)
    if not match:
        return None
    kind, rest = match.group(3), match.group(4)
    if kind == "impl":
        return "impl " + rest.split("{")[0].strip()
    if kind == "mod":
        name = re.match(r"([A-Za-z0-9_]+)", rest)
        return "mod " + name.group(1) if name else None
    name = re.match(r"([A-Za-z0-9_]+)", rest)
    if not name:
        return None
    return f"{kind} {name.group(1)}"


def main():
    lines = LIB.read_text(encoding="utf-8").splitlines()
    starts = []
    for index, line in enumerate(lines):
        sig = signature(line)
        if sig is not None:
            starts.append((index, sig))

    # a chunk begins at the doc/attribute block above its declaration
    begins = []
    for index, _sig in starts:
        begin = index
        while begin > 0 and re.match(r"^(///|//!|#\[|// )", lines[begin - 1]):
            begin -= 1
        begins.append(begin)

    chunks = []
    for position, (index, sig) in enumerate(starts):
        begin = begins[position]
        # the chunk ends where the *next* item's doc block starts, so no doc comment is
        # ever left dangling at the end of a file
        end = begins[position + 1] if position + 1 < len(begins) else len(lines)
        while end > begin and lines[end - 1].strip() == "":
            end -= 1
        chunks.append({"sig": sig, "begin": begin, "end": end, "decl": index})

    # every chunk must have a destination, or the split is incomplete
    missing = [chunk["sig"] for chunk in chunks if chunk["sig"] not in TARGETS]
    if missing:
        print("unassigned items:", file=sys.stderr)
        for sig in missing:
            print("   ", sig, file=sys.stderr)
        raise SystemExit(1)

    # the root keeps what it needs and hands the rest to its module
    head_end = chunks[0]["begin"]
    head = lines[:head_end]
    while head and head[-1].strip() == "":
        head.pop()

    buckets = {}
    seen = set()
    for chunk in chunks:
        if chunk["sig"] in seen:
            # an overload (two `impl` blocks on one type) is fine, but a duplicate
            # signature means the mapping table is ambiguous -- refuse to guess
            raise SystemExit(f"ambiguous signature moving twice: {chunk['sig']}")
        seen.add(chunk["sig"])
        target = TARGETS[chunk["sig"]]
        text = "\n".join(lines[chunk["begin"]:chunk["end"]])
        buckets.setdefault(target, []).append(text)

    for module, texts in buckets.items():
        if module == "lib":
            continue
        body = "\n\n".join(texts)
        path = SRC / f"{module}.rs"
        if path.exists():
            # merge into a module that already exists (agent.rs): its items stay, the moved
            # items go in before its test module, and the tests move to the bottom.
            existing = path.read_text(encoding="utf-8")
            split = existing.find("#[cfg(test)]")
            if split >= 0:
                module_body = existing[:split].rstrip()
                tests = existing[split:].rstrip()
            else:
                module_body, tests = existing.rstrip(), ""
            merged = module_body + "\n\nuse crate::*;\n\n" + body + "\n\n" + tests + "\n"
            path.write_text(merged, encoding="utf-8")
            print(f"{module}.rs: merged {len(texts)} items into the existing module")
        else:
            header = HEADERS[module]
            (path).write_text(header + "\n\nuse crate::*;\n\n" + body + "\n", encoding="utf-8")
            print(f"{module}.rs: {len(texts)} items, {body.count(chr(10)) + 1} lines")

    # ---- the facade -------------------------------------------------------------------
    module_list = "\n".join(
        f"pub use {module}::*;" for module in ("connection", "creation", "item", "map", "player", "travel", "world")
    )
    private_reexport = "\n".join(
        f"pub use {module}::*;" for module in ("agent", "chat", "dispatch")
    )
    root_items = "\n\n".join(buckets.get("lib", []))
    facade = FACADE.replace("__HEAD__", "\n".join(head))
    facade = facade.replace("__MODULES__", MODULE_DECLS)
    facade = facade.replace("__REEXPORTS__", module_list)
    facade = facade.replace("__PRIVATE__", private_reexport)
    facade = facade.replace("__ROOT_ITEMS__", root_items)
    LIB.write_text(facade + "\n", encoding="utf-8")
    print(f"lib.rs: facade, {facade.count(chr(10)) + 1} lines")


HEADERS = {
    "agent": '''//! The agent: one thing standing in a map, and where it is.
//!
//! One noun, one set of questions, answered against the instance that owns the agents.
//! The record is [`AgentState`] and [`Agents`] (below, moved here unchanged); a
//! player's placement is [`Standing`]; a walk is [`Walk`] and [`Motion`]; the movement trace
//! that a drift is diagnosed from lives at the bottom.
//!
//! What a client sends about its own agent, and what the server believes, is the reference's
//! rule set (`GmAgent.c`, `GameSrv_HandlePositionUpdate`): 50 units of slack, a 2-second
//! budget, a clamp instead of a drop. The doc comments on the functions say which.''' ,
    "player": '''//! The player: the character one connection drives, and the block that describes it.
//!
//! [`Player`] is the record; [`player_block`] is the character's own
//! `GAME_SMSG_INSTANCE_PLAYER_DATA_*` run; the constants below are the
//! reference's defaults for it (`PLAYER_ID`, `AGENT_ID`, `PARTY_ID`) and the profession it
//! reports when a character row carries none.''' ,
    "map": '''//! The map: a catalogue entry, its path graph, and the blocks that carry it to a client.
//!
//! [`Map`] is the noun; [`MapEntry`] is what a world publishes and what `/maps` lists;
//! [`PathGraphs`] is the imported `Gw.dat` geometry a `MOVE_TO_COORD` is answered from (the
//! reference's `srv->map_context`); [`spawn_block`], [`load_block`] and [`manifest`] are the
//! map's side of the instance load.
//!
//! A live instance is **not** here yet: today the map a connection is in is a `MapEntry` on its
//! `Play` record (`connection.rs`). The instance object is the missing noun, recorded in
//! `docs/HANDOFF.md` and deferred with the simulation (`docs/adr/0003-simulation-in-scope.md`).''' ,
    "party": '''//! The party.
//!
//! The noun this file is for. Nothing in it yet: the wire carries `PARTY_CREATE` /
//! `PARTY_PLAYER_ADD` (built in `asura_protocol::game`), and membership is implied by the peers
//! standing in the same world (`world.rs`). The file exists so the noun has a home before the
//! first party rule does.''' ,
    "item": '''//! The item and the inventory: what a character carries, and what a client may do with it.
//!
//! [`Item`] and [`Inventory`] are the two nouns. [`items_block`] and [`inventory_block`] are the
//! load-time blocks; the four
//! client operations below are `DROP_ITEM`, `ITEM_MOVE`, `EQUIP_ITEM` and the refusal path, and
//! the row-level work is `asura_db::inventory`.
//!
//! [`move_item`] and [`equip_item`] are messages the reference does not dispatch at all - their
//! shapes are measured, not ported (`docs/HANDOFF.md`, M5).''' ,
    "chat": '''//! Chat: spoken lines, and the command table they arrive through.
//!
//! The reference has no chat handler; it prints and answers into its own log
//! (`docs/HANDOFF.md`). This module is that half, plus the `!`/`#`/`$` channel rule and the
//! `/maps`, `/stuck` and `/tp` command table ([`Command`], [`command`]) a player types into a
//! chat line. The travel `/tp` starts lives in `travel.rs`; this module only decides which
//! command a line names.''' ,
    "travel": '''//! Travel: leaving one instance for another.
//!
//! The reference's request/redirect cycle (`docs/02-architecture.md`, "Travel, end to end"):
//! reserve a slot, write a transfer row, send `TRANSFER_GAME_SERVER_INFO`, then
//! `MAP_UPDATE_CURRENT`. [`teleport`] is the `/tp` half; [`load_map_on_demand`] is the
//! documented fallback that loads a map this world was not started with (`docs/03-protocol.md`
//! #5c, measured 0-2 ms per map).
//!
//! The client-driven half of travel - `AUTH_CMSG_REQUEST_GAME_INSTANCE` and the front door - is
//! in `asura-auth`; a world only decides where a player *already here* goes next.''' ,
    "creation": '''//! Character creation: the instance and the choices a new character is made of.
//!
//! The reference's `MapType_CharacterCreation` - the only map type that is a conversation
//! rather than a place (`docs/HANDOFF.md`, M5). [`Creation`] is what a creation connection has
//! chosen so far; the two blocks are the reference's own default equipment and inventory for
//! that choice (`GetDefaultEquipments`, generated into `default_equipment.rs`).''' ,
    "connection": '''//! One gameplay connection: hello, load, play, goodbye.
//!
//! The server's `ConnectedClient`, and [`run`], which owns it. A socket is
//! classified, its key exchange is completed, the token pair is claimed against a transfer, the
//! load block goes out unasked, and then [`run`] owns the socket: the world tick, the ping, the
//! client's messages (through `dispatch.rs`) and the broadcasts other players push at it.
//!
//! [`Play`] is what one connection knows about itself, and it is where the map, the agents and
//! the inventory currently live. That is the shape the instance object would replace: the state
//! belongs to the map, not to the socket. Recorded, not fixed here.''' ,
    "dispatch": '''//! The dispatch table: one header in, one domain's handler out.
//!
//! The reference's `GameSrv.c` is one switch over headers and so is this; a client cannot be told
//! about a command seam it does not speak. What this file is *not*: a home for domain logic. Each
//! arm validates, then calls into `item`, `chat`, `travel`, `creation`, `player` or `map`; the
//! only state it touches is the one the message is about.
//!
//! A header with no arm is framed and ignored - named in the trace, never guessed at.''',
    "world": '''//! The world: the registry row, the maps it publishes, and everyone standing in it.
//!
//! [`World`] is what a world server registers as and what `serve` hands each connection. [`Peer`]
//! and [`Peers`] are the players in it: the reference's `srv->agents`/`srv->players`, kept here
//! because a connection's tick has to tell everyone else what it is doing.
//!
//! This is the noun the party and the instance both hang off; `docs/02-architecture.md` describes
//! the instance as a per-map update thread with affinity, and that object is still missing
//! (`docs/HANDOFF.md`, the world server's domains).''' ,
}

MODULE_DECLS = '''// The domains: one file per noun.
pub mod chat;
pub mod connection;
pub mod creation;
pub mod dispatch;
pub mod item;
pub mod map;
pub mod player;
pub mod travel;
pub mod world;'''

FACADE = '''__HEAD__

// -------------------------------------------------------------------------------------------
// The domains. One file per noun; `lib.rs` is the
// facade that re-exports them, so `asura_world::MapEntry` and `asura_world::map::MapEntry` are
// the same type. A domain's file is where to look: `item.rs` for items, `chat.rs` for chat.
// -------------------------------------------------------------------------------------------

__MODULES__

__REEXPORTS__

__PRIVATE__

__ROOT_ITEMS__
'''

if __name__ == "__main__":
    main()
