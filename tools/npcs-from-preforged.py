#!/usr/bin/env python3
"""Copy the preforged NPC population into this world pack.

    python tools/npcs-from-preforged.py --write

Source: `Other_Repos_Research/preforged-main/data/gw/npcs.txt` (`gw-npcs 1`: map, actor,
definition, model file, skin file, visual, appearance, flags, primary, level, plane, x, y,
facing, encoded name, model files; the `#` line before a row is its captured name). Only the
pre-Searing maps are written; the catalog also carries the other campaigns.

The catalog has no held items. For map 148 the importer reads the capture-built `npcs.json`
first and copies each armed body's `0x0161` fields onto the placement at the same position, so
the guards that were armed in the live load stay armed.

The catalog's definition id is local to its map (every map restarts at 1), so a pack row gets a
pack-wide `definition_index` and the id `npc-<map>-<definition>`. An `Ascalon Guard` in map 148
is given an authored four-point patrol at 220 u/s, cmonmeow's NPC speed.
"""

import argparse
import json
import math
import os
import re

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE = os.path.join(ROOT, "Other_Repos_Research", "preforged-main", "data", "gw", "npcs.txt")
PRE_SEARING = {145, 146, 147, 148, 160, 161, 162, 163, 164, 165, 166}
PATROL_MAPS = {148}
NPC_SPEED = 120.0
TEAM = "nonc"
H000B_PERSON = 9
PATROL_SIDE = 120.0
# The close pair of Ascalon Guards chooses among these spots; the catalog has no route and the
# live sniff puts the two guards on a shared set of stops. Coordinates are the named NPCs' own
# positions, except the gate corner, which is a walkable point beside the Ascalon gate.
PATROL_PAIR = {10014, 10015}
PATROL_SPOTS = (
    (11407.0, 5326.0, 0),  # The Purveyor [Bonus Merchant]
    (6759.0, 9721.0, 0),   # Namar
    (7800.0, 5300.0, 0),   # the corner to the right of the Ascalon gate
    (8842.0, 4650.0, 0),   # Gift-Giver Pinn
    (10640.0, 7831.0, 0),  # Lieutenant Langmar
)
PATROL_PAUSE = 3000
# The catalog's encoded names are 37130 string ids; the 39167 client's text table does not share
# them, and resolving one costs an `index < m_count` hangup. The definition carries this known-valid
# 39167 name (the capture's Ascalon Guard); each placement overrides it with literal text.
PLACEHOLDER_NAME = [3933, 50198, 44838, 5755]
ARMED_MATCH = 1.5

FIELDS = {
    "id": "The pack key a placement names. Never sent; the row's definition_index is what the client keys its NPC table by.",
    "definition_index": "0x0056/0x0057 field 1 and the low bits of the create's model word: the client's NPC table index, unique across this pack.",
    "file_id": "0x0056 field 2: the model file.",
    "h000a": "0x0056 field 3 (body offset 0x0A): 0 in every captured row; unnamed.",
    "visual": "0x0056 field 4: the visual word; its top byte is the scale in percent.",
    "h0012": "0x0056 field 5 (body offset 0x12): 0 in every captured row; unnamed.",
    "flags": "0x0056 field 6: a flags word.",
    "primary": "0x0056 byte 1: the primary profession.",
    "level": "0x0056 byte 2: the definition's level.",
    "name": "0x0056's name: encoded-string UTF-16 units, 1 to 8.",
    "model_files": "0x0057's model files, 0 to 8; empty sends no 0x0057.",
    "h000b": "The create's byte at body offset 0x0B: 9 for a person, 8 for an animal in the capture.",
    "speed": "The create's speed, in units per second. 220 is cmonmeow's NPC value.",
    "team": "The create's team dword as the client's four-character code.",
}


def units(text):
    if text == "-":
        return []
    if len(text) % 4:
        raise ValueError("encoded name %r is not whole units" % text)
    return [int(text[i:i + 4], 16) for i in range(0, len(text), 4)]


def model_files(text):
    if text == "-":
        return []
    return [int(part) for part in text.split(",")]


def literal_name(text):
    """A coded literal string: marker, text units, terminator; `0x009B` holds at most 32 units."""
    if not text:
        return None
    units = [0x0108, 0x0107]
    for character in text:
        code = ord(character)
        if 0x20 <= code < 0x10000:
            units.append(code)
    units = units[: 2 + 28]
    units.append(0x0001)
    return units


def read_catalog(path):
    rows = []
    name = ""
    with open(path, encoding="utf-8") as handle:
        header = False
        for number, line in enumerate(handle, 1):
            text = line.strip()
            if text.startswith("#"):
                name = text[1:].strip()
                continue
            if not text:
                continue
            if not header:
                if text != "gw-npcs 1":
                    raise ValueError("line %d: expected gw-npcs 1" % number)
                header = True
                continue
            fields = text.split()
            if len(fields) != 17 or fields[0] != "npc":
                raise ValueError("line %d: expected 17 NPC fields" % number)
            row = {
                "map": int(fields[1]),
                "actor": int(fields[2]),
                "definition": int(fields[3]),
                "model_file": int(fields[4]),
                "skin_file": int(fields[5]),
                "visual": int(fields[6]),
                "appearance": int(fields[7]),
                "flags": int(fields[8]),
                "primary": int(fields[9]),
                "level": int(fields[10]),
                "plane": int(fields[11]),
                "x": float(fields[12]),
                "y": float(fields[13]),
                "facing": float(fields[14]),
                "name": units(fields[15]),
                "model_files": model_files(fields[16]),
                "comment": name,
            }
            if row["map"] in PRE_SEARING:
                rows.append(row)
            name = ""
    if not header:
        raise ValueError("the catalog has no header")
    return rows


def find_map_folder(pack, map_id):
    for base, dirs, _files in os.walk(os.path.join(pack, "maps")):
        for directory in dirs:
            if directory.isdigit() and int(directory) == map_id:
                return os.path.join(base, directory)
    return None


def read_armed(pack, map_id):
    """Armed placements of the capture-built map, by position, before this tool replaces it."""
    for base, dirs, _files in os.walk(os.path.join(pack, "maps")):
        for directory in dirs:
            if directory.isdigit() and int(directory) == map_id:
                path = os.path.join(base, directory, "npcs.json")
                if not os.path.exists(path):
                    return []
                with open(path, encoding="ascii") as handle:
                    doc = json.load(handle)
                armed = []
                for placement in doc.get("placements", []):
                    if placement.get("items"):
                        armed.append(placement)
                return armed
    return []


def armed_at(armed, x, y):
    for placement in armed:
        if math.hypot(placement["x"] - x, placement["y"] - y) <= ARMED_MATCH:
            return {
                "items": placement["items"],
                "weapons": placement.get("weapons", [None, None]),
            }
    return None


def diamond(x, y, plane, radius):
    return [
        (x + radius, y, plane),
        (x, y + radius, plane),
        (x - radius, y, plane),
        (x, y - radius, plane),
    ]


def square(x, y, plane, facing, side, flip=1.0):
    fx, fy = math.cos(facing), math.sin(facing)
    px, py = -fy * flip, fx * flip
    return [
        (x, y, plane),
        (x + fx * side, y + fy * side, plane),
        (x + fx * side + px * side, y + fy * side + py * side, plane),
        (x + px * side, y + py * side, plane),
    ]


def patrol_points(row):
    return [(x, y, plane) for x, y, plane in PATROL_SPOTS]


def dump(value):
    text = json.dumps(value, indent=2, ensure_ascii=True)
    return re.sub(
        r"\[\s*(-?[0-9][-0-9.e+]*(?:,\s*-?[0-9][-0-9.e+]*)*)\s*\]",
        lambda match: "[" + ", ".join(part.strip() for part in match.group(1).split(",")) + "]",
        text,
    ) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--source", default=SOURCE)
    parser.add_argument("--pack", default=os.path.join(ROOT, "config", "World", "Vanilla"))
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()

    rows = read_catalog(args.source)
    if not rows:
        print("no pre-Searing rows in %s" % args.source)
        return 1

    # A definition is local to its map; give it a pack-wide index in map/definition order.
    definitions = {}
    for row in sorted(rows, key=lambda row: (row["map"], row["definition"])):
        key = (row["map"], row["definition"])
        if key in definitions:
            continue
        definitions[key] = len(definitions) + 1

    definition_rows = []
    for (map_id, definition_id), index in sorted(
        definitions.items(), key=lambda item: item[1]
    ):
        sample = next(
            row
            for row in rows
            if row["map"] == map_id and row["definition"] == definition_id
        )
        definition_rows.append(
            {
                "id": "npc-%d-%d" % (map_id, definition_id),
                "definition_index": index,
                "file_id": sample["model_file"],
                "h000a": 0,
                "visual": sample["visual"],
                "h0012": 0,
                "flags": sample["flags"],
                "primary": sample["primary"],
                "level": sample["level"],
                "name": PLACEHOLDER_NAME,
                "model_files": sample["model_files"],
                "h000b": H000B_PERSON,
                "speed": NPC_SPEED,
                "team": TEAM,
            }
        )

    by_map = {}
    for row in rows:
        by_map.setdefault(row["map"], []).append(row)

    written = []
    for map_id, map_rows in sorted(by_map.items()):
        folder = find_map_folder(args.pack, map_id)
        if folder is None:
            print("skipped map %d: no map folder in %s" % (map_id, args.pack))
            continue
        armed = read_armed(args.pack, map_id)
        placements = []
        routes = []
        for row in sorted(map_rows, key=lambda row: row["actor"]):
            placement = {
                "id": "%d-%d" % (map_id, row["actor"]),
                "definition": "npc-%d-%d" % (map_id, row["definition"]),
                "x": row["x"],
                "y": row["y"],
                "plane": row["plane"],
                "angle": row["facing"],
                "effects": 0,
                "h0023": 1.0,
            }
            display = literal_name(row["comment"])
            if display:
                placement["name"] = display
            gear = armed_at(armed, row["x"], row["y"])
            if gear:
                placement["items"] = gear["items"]
                placement["weapons"] = gear["weapons"]
            if (
                map_id in PATROL_MAPS
                and row["actor"] in PATROL_PAIR
            ):
                route_id = "patrol-%d-%d" % (map_id, row["actor"])
                points = patrol_points(row)
                routes.append(
                    {
                        "id": route_id,
                        "mode": "random",
                        "points": [
                            {
                                "x": x,
                                "y": y,
                                "plane": plane,
                                "pause_ms": PATROL_PAUSE,
                            }
                            for x, y, plane in points
                        ],
                    }
                )
                placement["behavior"] = "patrol"
                placement["route"] = route_id
            placements.append(placement)

        doc = {
            "map_id": map_id,
            "note": (
                "The NPC bodies of this map from preforged's gw-npcs 1 catalog, one placement per "
                "body, in actor order. Held items are the capture's 0x0161 rows where the same body "
                "was armed there. The name is a literal 0x009B because the catalog's encoded names "
                "are 37130 string ids. An Ascalon Guard's route is authored, not captured."
            ),
            "placements": placements,
            "routes": routes,
        }
        if args.write:
            with open(os.path.join(folder, "npcs.json"), "w", encoding="ascii", newline="\n") as handle:
                handle.write(dump(doc))
        written.append((map_id, len(placements), len(routes)))
        print(
            "map %3d: %3d placement(s), %d patrol(s)"
            % (map_id, len(placements), len(routes))
        )

    definitions_path = os.path.join(args.pack, "npcs", "definitions.json")
    doc = {
        "note": (
            "The NPC types of this pack, from preforged's gw-npcs 1 catalog. A definition id is "
            "local to its map there, so each row's definition_index is pack-wide and its id is "
            "npc-<map>-<definition>."
        ),
        "fields": FIELDS,
        "definitions": definition_rows,
    }
    if args.write:
        os.makedirs(os.path.dirname(definitions_path), exist_ok=True)
        with open(definitions_path, "w", encoding="ascii", newline="\n") as handle:
            handle.write(dump(doc))
    print(
        "%d definition(s), %d map(s), %d placement(s)"
        % (
            len(definition_rows),
            len(written),
            sum(count for _map, count, _routes in written),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
