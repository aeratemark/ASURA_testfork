#!/usr/bin/env python3
"""Copy the NPCs a live load shows into a world pack: definitions, placements, exactly as sent.

    python tools/npcs-from-capture.py recordings/client/feature/maps/pre-searing-load-3.jsonl \
        --map 148 --pack config/World/Vanilla [--write]

Reads a client-side capture (`tools/sniff-capture.py` format: struct-layer records, four-byte header,
every field widened to four bytes) and finds every NPC create (`0x0020` whose model word's class nibble
is 2). For each it collects the entry burst the live server sent around the create (`docs/49` 3.5):
`0x0056`/`0x0057` for its definition, `0x009A`, `0x009B`, `0x009F` property 36, `0x00F0`, the create,
`0x006D`, the `0x0161` item records and the properties after it. It writes:

- `<pack>/npcs/definitions.json`: one row per definition used, merged with the rows already there (a
  row whose values differ from the capture is reported and left alone);
- `<pack>/maps/.../<map>/npcs.json`: one placement per NPC, in agent-id order.

An armed body keeps its `0x0161` rows and its `0x006D` weapons: `items` holds the item fields without
the live connection's arbitrary item id, and `weapons` names the lead and offhand as indices into
`items`. A definition whose agents disagree on a create field the pack keeps per definition (speed,
team, `h000b`) is refused.

Without `--write` it prints what it would write and changes nothing. Every value is the capture's;
nothing is chosen here. ASCII output only.
"""

import argparse
import json
import os
import re
import struct
import sys

NPC_CLASS = 2
LEVEL_PROPERTY = 36
DEFINITION_FIELDS = {
    "id": "The pack key a placement names. Never sent.",
    "definition_index": "0x0056/0x0057 field 1 and the low bits of the create's model word: the client's NPC table index.",
    "file_id": "0x0056 field 2: the model file.",
    "h000a": "0x0056 field 3 (body offset 0x0A): 0 in every captured row; unnamed.",
    "visual": "0x0056 field 4: the visual word; its top byte is the scale in percent.",
    "h0012": "0x0056 field 5 (body offset 0x12): 0 in every captured row; unnamed.",
    "flags": "0x0056 field 6: a flags word.",
    "primary": "0x0056 byte 1: the primary profession.",
    "level": "0x0056 byte 2: the definition's level.",
    "name": "0x0056's name: encoded-string UTF-16 units, 1 to 8.",
    "model_files": "0x0057's model files, 0 to 8; empty sends no 0x0057.",
    "h000b": "The create's byte at body offset 0x0B: 9 for a person, 8 for an animal in the captures.",
    "speed": "The create's speed, in units per second.",
    "team": "The create's team dword as the client's four-character code.",
}


def dwords(data_hex):
    raw = bytes.fromhex(data_hex)
    return [struct.unpack_from("<I", raw, at)[0] for at in range(0, len(raw) - 3, 4)]


def as_float(word):
    return struct.unpack("<f", struct.pack("<I", word))[0]


def short_float(word):
    """The shortest decimal that reads back to the same f32."""
    value = as_float(word)
    for digits in range(1, 12):
        text = "%.*g" % (digits, value)
        if struct.unpack("<I", struct.pack("<f", float(text)))[0] == word:
            return float(text)
    return value


def units(data_hex, offset, capacity):
    raw = bytes.fromhex(data_hex)[offset:offset + capacity * 2]
    out = []
    for at in range(0, len(raw) - 1, 2):
        unit = struct.unpack_from("<H", raw, at)[0]
        if unit == 0:
            break
        out.append(unit)
    return out


def fourcc(word):
    return struct.pack(">I", word).decode("ascii")


def dump(value):
    """Indented JSON with every list of numbers on one line."""
    text = json.dumps(value, indent=2, ensure_ascii=True)
    return re.sub(
        r"\[\s*(-?[0-9][-0-9.e+]*(?:,\s*-?[0-9][-0-9.e+]*)*)\s*\]",
        lambda match: "[" + ", ".join(part.strip() for part in match.group(1).split(",")) + "]",
        text,
    ) + "\n"


def read_capture(path):
    rows = []
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            record = json.loads(line)
            if "seq" in record and record.get("direction") == "StoC":
                rows.append(record)
    return rows


def find_map_folder(pack, map_id):
    for root, dirs, _files in os.walk(os.path.join(pack, "maps")):
        for name in dirs:
            if name.isdigit() and int(name) == map_id:
                return os.path.join(root, name)
    return None


def collect(rows):
    """Definitions by index, and one entry per NPC create with its burst."""
    definitions = {}
    for row in rows:
        if row["header"] == 0x56:
            w = dwords(row["data"])
            definitions.setdefault(w[1], {
                "definition_index": w[1], "file_id": w[2], "h000a": w[3], "visual": w[4],
                "h0012": w[5], "flags": w[6], "primary": w[7] & 0xFF, "level": w[8] & 0xFF,
                "name": units(row["data"], 36, 8), "model_files": [], "seq": row["seq"],
            })
        elif row["header"] == 0x57:
            w = dwords(row["data"])
            if w[1] in definitions and not definitions[w[1]]["model_files"]:
                definitions[w[1]]["model_files"] = w[3:3 + min(w[2], 8)]

    npcs = []
    for index, row in enumerate(rows):
        if row["header"] != 0x20:
            continue
        w = dwords(row["data"])
        agent, model = w[1], w[2]
        if model >> 28 != NPC_CLASS:
            continue
        pre = []
        back = index - 1
        while back >= 0 and rows[back]["header"] in (0x56, 0x57, 0x9A, 0x9B, 0x9F, 0xF0):
            record = rows[back]
            if record["header"] in (0x9A, 0x9B, 0xF0) and dwords(record["data"])[1] != agent:
                break
            if record["header"] == 0x9F and dwords(record["data"])[2] != agent:
                break
            pre.insert(0, record)
            back -= 1
        post = []
        ahead = index + 1
        while ahead < len(rows) and rows[ahead]["header"] in (0x161, 0x15A, 0x6D, 0x9F, 0xA2):
            record = rows[ahead]
            if record["header"] in (0x9F, 0xA2) and dwords(record["data"])[2] != agent:
                break
            post.append(record)
            ahead += 1
        npcs.append({"agent": agent, "create": row, "pre": pre, "post": post})
    return definitions, npcs


def placement_of(npc, map_id, ordinal):
    w = dwords(npc["create"]["data"])
    index = w[2] & 0x0FFFFFFF
    row = {
        "id": "%d-%04x-%d" % (map_id, index, ordinal),
        "definition": "npc-%04x" % index,
        "x": short_float(w[5]),
        "y": short_float(w[6]),
        "plane": w[7] & 0xFFFF,
        "direction": [short_float(w[8]), short_float(w[9])],
    }
    effects = 0
    for record in npc["pre"]:
        r = dwords(record["data"])
        if record["header"] == 0x9A:
            row["scale"] = r[2]
        elif record["header"] == 0x9B:
            row["name"] = units(record["data"], 8, 32)
        elif record["header"] == 0x9F and r[1] == LEVEL_PROPERTY:
            row["level"] = r[3]
        elif record["header"] == 0xF0:
            effects = r[2]
    row["effects"] = effects
    row["h0023"] = short_float(w[12])
    properties = []
    for record in npc["post"]:
        r = dwords(record["data"])
        if record["header"] == 0x9F:
            properties.append({"id": r[1], "int": r[3]})
        elif record["header"] == 0xA2:
            properties.append({"id": r[1], "float": short_float(r[3])})
    if properties:
        row["properties"] = properties
    return index, row


def held(npc):
    """The item records and weapon ids of one body, from the burst after its create.

    A `0x0161` record's first twelve dwords are the fields the wire carries; the client's struct
    carries more, but the rest of its buffer is stale (the record reuses the create's buffer), so
    only these are kept. The item id is the live connection's arbitrary handle and is not written:
    the world allocates one per body. `0x006D` names the lead and offhand by that handle, and those
    two are stored as indices into `items`.
    """
    items = []
    index_of = {}
    lead = offhand = 0
    for record in npc["post"]:
        if record["header"] == 0x161:
            w = dwords(record["data"])
            index_of[w[1]] = len(items)
            items.append({
                "file_id": w[2],
                "item_type": w[3],
                "dye_tint": w[4],
                "dye_colors": w[5],
                "materials": w[6],
                "unk1": w[7],
                "flags": w[8],
                "value": w[9],
                "model_id": w[10],
                "quantity": w[11],
            })
        elif record["header"] == 0x6D:
            w = dwords(record["data"])
            lead, offhand = w[2], w[3]
    weapons = []
    for item_id in (lead, offhand):
        if item_id == 0:
            weapons.append(None)
        elif item_id in index_of:
            weapons.append(index_of[item_id])
        else:
            raise ValueError("agent %#x names item %d, which its burst never declared"
                             % (dwords(npc["create"]["data"])[1], item_id))
    return items, weapons


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("capture")
    parser.add_argument("--map", type=int, required=True, help="the map the capture is a load of")
    parser.add_argument("--pack", required=True, help="the world pack, e.g. config/World/Vanilla")
    parser.add_argument("--write", action="store_true", help="write the files (default: print)")
    args = parser.parse_args()

    rows = read_capture(args.capture)
    definitions, npcs = collect(rows)
    placements, used, problems, ordinals = [], {}, [], {}
    for npc in sorted(npcs, key=lambda npc: npc["agent"]):
        index = dwords(npc["create"]["data"])[2] & 0x0FFFFFFF
        ordinals[index] = ordinals.get(index, 0) + 1
        index, row = placement_of(npc, args.map, ordinals[index])
        w = dwords(npc["create"]["data"])
        create = {"h000b": w[4] & 0xFF, "speed": short_float(w[11]), "team": fourcc(w[14])}
        if index not in definitions:
            problems.append("agent %#x: definition %#x was never sent in this capture" % (npc["agent"], index))
            continue
        seen = used.setdefault(index, create)
        if seen != create:
            problems.append("definition %#x: agents disagree on %r and %r" % (index, seen, create))
        try:
            items, weapons = held(npc)
        except ValueError as error:
            problems.append(str(error))
            continue
        if items:
            row["items"] = items
            if any(weapon is not None for weapon in weapons):
                row["weapons"] = weapons
        placements.append(row)
    if problems:
        print("\n".join("refused: " + p for p in problems), file=sys.stderr)
        return 1

    rows_out = []
    for index in sorted(used):
        d = dict(definitions[index])
        d.pop("seq")
        rows_out.append({"id": "npc-%04x" % index, **{k: d[k] for k in (
            "definition_index", "file_id", "h000a", "visual", "h0012", "flags", "primary", "level",
            "name", "model_files")}, **used[index]})

    definitions_path = os.path.join(args.pack, "npcs", "definitions.json")
    existing = {"note": None, "fields": DEFINITION_FIELDS, "definitions": []}
    if os.path.exists(definitions_path):
        with open(definitions_path, encoding="ascii") as handle:
            existing = json.load(handle)
    by_id = {row["id"]: row for row in existing["definitions"]}
    for row in rows_out:
        if row["id"] in by_id and by_id[row["id"]] != row:
            print("kept: %s differs from the capture; left as it is" % row["id"], file=sys.stderr)
            continue
        by_id[row["id"]] = row
    existing["definitions"] = [by_id[key] for key in sorted(by_id)]
    existing["fields"] = DEFINITION_FIELDS
    if not existing.get("note"):
        existing["note"] = ("The NPC types of this pack: what 0x0056 and 0x0057 carry, plus the create "
                            "fields every body of the type shares. A placement names a type by id.")

    folder = find_map_folder(args.pack, args.map)
    if folder is None:
        print("refused: no folder for map %d under %s/maps" % (args.map, args.pack), file=sys.stderr)
        return 1
    map_path = os.path.join(folder, "npcs.json")
    # Authors' routes and behavior assignments are content, not capture, so a rerun keeps them:
    # only the captured fields are rebuilt. A placement that disappeared from the capture loses them.
    authored = {}
    routes = []
    if os.path.exists(map_path):
        with open(map_path, encoding="ascii") as handle:
            old = json.load(handle)
        routes = old.get("routes", [])
        authored = {
            row["id"]: {key: row[key] for key in ("route", "behavior") if key in row}
            for row in old.get("placements", [])
        }
    for row in placements:
        row.update(authored.get(row["id"], {}))
    map_file = {
        "map_id": args.map,
        "note": ("The NPC bodies of this map, one placement per body, in the order their agent ids "
                 "are handed out. A placement's optional keys are messages: scale is 0x009A, name is "
                 "0x009B, level is 0x009F property 36, properties follow 0x006D. Routes and behavior "
                 "assignments are authored content and are preserved when this file is regenerated."),
        "placements": placements,
        "routes": routes,
    }

    outputs = [(definitions_path, existing), (os.path.join(folder, "npcs.json"), map_file)]
    for path, value in outputs:
        text = dump(value)
        if args.write:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="ascii", newline="\n") as handle:
                handle.write(text)
            print("wrote %s" % path)
        else:
            print("--- %s\n%s" % (path, text))
    print("%d placement(s), %d definition(s)" % (len(placements), len(rows_out)), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
