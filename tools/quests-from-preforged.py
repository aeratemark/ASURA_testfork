#!/usr/bin/env python3
"""Write the quest content this world pack carries.

    python tools/quests-from-preforged.py            print what would be written
    python tools/quests-from-preforged.py --write    write config/World/Vanilla/quests.json

Sources, both read from `Other_Repos_Research/preforged-main.zip` (or from an extracted tree with
`--root`):

* `data/gw/quests/presearing.fwqc` - preforged's catalog, `FWQC3`: one row per quest state with the
  source hash, id, log state, exact x/y bits, plane, destination and source map, the encoded
  location/name/NPC units and the description/objectives units, then the stage (`active`,
  `completed`, `count:N`) and, when the text names the player, the captured character argument.
  The format is `src/runtime/gw_quest_catalog.h` in preforged.
* `data/gw/quests/side-quests-2026-10-12-evidence.json` - the live capture's own quest snapshots
  (`data/captures/pre-searing-side-quests-2026-10-12/checkpoint-1815/world-43292-168890171.jsonl`,
  sha256 194855cf498db3b91d9afee929785470c176596fc1fe149b33fe8b8c5a1b9757). It carries 27 ids,
  including ids no fwqc row covers.

Every unit in the output is the capture's. Nothing here composes text, resolves a string id or
normalises a marker. A `<c=@Quest>` marker and a `{s}` prefix are the client's own control codes and
stay where the capture put them.

Run `tools/npc-content-from-captures.py --write` after this one: it adds the quests the captures carry
and this does not, and the offer and reward block of every quest whose offer and turn-in the captures
hold, with their dialogs.

The offer/reward block for quest 80 is authored from preforged's `gw_presearing_quests.h` and
`docs/message-from-friend80-policy-20261030.txt`: the Town Crier offers on map 148 while none of War
Preparations' ids is completed or rewarded, and Sir Tydus pays 100 experience and 10 gold on map 148.
"""

import argparse
import json
import os
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ZIP = os.path.join(ROOT, "Other_Repos_Research", "preforged-main.zip")
OUT = os.path.join(ROOT, "config", "World", "Vanilla", "quests.json")
FWQC = "data/gw/quests/presearing.fwqc"
EVIDENCE = "data/gw/quests/side-quests-2026-10-12-evidence.json"

# The client's own bounds: the quest name fields decode into 8-unit buffers and the detail fields into
# 128-unit buffers, each with a terminator reserved (`gw_quest_wire.h`, `encoded_name`/`encodet_text`).
NAME_UNITS = 7
DETAIL_UNITS = 127
INFINITY_BITS = 0x7F800000

FIELDS = {
    "quests": "One per quest id the catalog or the capture names.",
    "id": "The client's quest id: the number a log entry, a marker and a request carry.",
    "state": "The captured log state: bit 1 loaded, bit 2 completed, bit 32 primary. The server masks bits 1 and 2 when it restores an entry.",
    "marker": "The log marker: x_bits and y_bits are exact IEEE-754 bit patterns, and plane is the map plane. Both +infinity means no marker.",
    "map_to": "The map the marker points at. 898 is the no-destination sentinel these captures carry.",
    "map_from": "The map the quest was taken on.",
    "location": "The quest log's category, encoded units.",
    "name": "The quest's name, encoded units; at most 7 units plus the client's own terminator.",
    "npc": "The quest giver's name, encoded units; at most 7 units plus the terminator.",
    "description": "The 0x004C details text, encoded units; at most 127 units plus the terminator.",
    "objectives": "The 0x004C objectives text, encoded units; at most 127 units plus the terminator.",
    "character_argument": "The captured character name inside the text. The server replaces it with the playing character's name, between 0x0107 and 0x0001.",
    "completed": "The details the capture shows after the quest completed.",
    "counted": "The details the capture shows after this many objective credits, contiguous from 1.",
    "credits": "How many objective credits this variant was captured at.",
    "offer": "Who offers the quest, on which map, and the quest ids that must not be completed or rewarded before the offer is open.",
    "reward": "Who takes the turn-in, on which map, and the experience and gold it pays.",
}

# The one quest whose offer and reward values are authored today. The rest are content only: the
# quest log can show them, and nothing accepts or rewards them.
RULES = {
    80: {
        "offer": {
            "npc": "Town Crier",
            "map": 148,
            "requires_none_completed": [35, 217, 218, 219, 220, 221, 222],
        },
        "reward": {
            "npc": "Sir Tydus",
            "map": 148,
            "experience": 100,
            "gold": 10,
        },
    },
}


def number(text, maximum=0xFFFFFFFF):
    if not text or text.strip() != text:
        raise ValueError("empty or padded number %r" % text)
    value = int(text, 10)
    if value < 0 or value > maximum:
        raise ValueError("number out of range: %r" % text)
    return value


def units(text, limit):
    """A FWQC encoded string: '.' is captured empty, '-' is unloaded, anything else is 4 hex digits a unit."""
    if text == "-":
        return None
    if text == ".":
        return []
    if not text or len(text) % 4:
        raise ValueError("encoded text is not whole units: %r" % text[:40])
    out = []
    for at in range(0, len(text), 4):
        # FWQC writes the unit as its two little-endian bytes: `643d` is 0x3d64.
        unit = int(text[at:at + 2], 16) | (int(text[at + 2:at + 4], 16) << 8)
        if unit == 0:
            raise ValueError("encoded text embeds a terminator")
        out.append(unit)
    if len(out) > limit:
        raise ValueError("encoded text has %d units, over %d" % (len(out), limit))
    return out


def check_marker(x_bits, y_bits, plane):
    x = x_bits
    y = y_bits
    finite = (x & 0x7F800000) != 0x7F800000 and (y & 0x7F800000) != 0x7F800000
    sentinel = x == INFINITY_BITS and y == INFINITY_BITS
    if not (finite or sentinel):
        raise ValueError("marker is neither finite nor the +infinity pair: %08x %08x" % (x, y))
    if plane > 0xFFFF:
        raise ValueError("plane out of range: %d" % plane)
    return {"x_bits": x, "y_bits": y, "plane": plane}


def details(description, objectives, character_argument=None):
    if (description is None) != (objectives is None):
        raise ValueError("only half of the details pair is loaded")
    row = {}
    if description is not None:
        row["description"] = description
        row["objectives"] = objectives
    if character_argument is not None:
        row["character_argument"] = character_argument
    return row


def read_fwqc(text):
    lines = text.splitlines()
    if not lines or lines[0] not in ("FWQC1", "FWQC2", "FWQC3"):
        raise ValueError("unknown quest catalog version")
    staged = lines[0] in ("FWQC2", "FWQC3")
    counted = lines[0] == "FWQC3"
    quests = {}
    for number_of, line in enumerate(lines[1:], 2):
        if not line:
            continue
        fields = line.split("\t")
        if len(fields) not in (13, 14, 15):
            raise ValueError("line %d: %d fields" % (number_of, len(fields)))
        if len(fields) == 15 and not counted:
            raise ValueError("line %d: a character argument in a non-counted catalog" % number_of)
        id = number(fields[1], 0xFFFFFFFF)
        if not id:
            raise ValueError("line %d: quest id 0" % number_of)
        x_bits = number(fields[3])
        y_bits = number(fields[4])
        row = {
            "id": id,
            "state": number(fields[2]),
            "marker": check_marker(x_bits, y_bits, number(fields[5], 0xFFFF)),
            "map_to": number(fields[6], 0xFFFF),
            "map_from": number(fields[7], 0xFFFF),
            "location": units(fields[8], NAME_UNITS),
            "name": units(fields[9], NAME_UNITS),
            "npc": units(fields[10], NAME_UNITS),
        }
        for key in ("location", "name", "npc"):
            if not row[key]:
                raise ValueError("line %d: %s is empty" % (number_of, key))
        description = units(fields[11], DETAIL_UNITS)
        objectives = units(fields[12], DETAIL_UNITS)
        content = details(description, objectives)
        argument = units(fields[14], 19) if len(fields) == 15 else None
        if argument is not None:
            content["character_argument"] = argument
        stage = fields[13] if staged else "active"
        quest = quests.setdefault(id, dict(row))
        # The log state is per row: bit 1 is the details-loaded flag and bit 2 the completed flag,
        # both of which the restore masks. A marker and its destination move as the quest advances, so
        # a variant carries its own; the name fields never move.
        for key in ("id", "map_from", "location", "name", "npc"):
            if quest[key] != row[key]:
                raise ValueError("line %d: quest %d changes %s between rows" % (number_of, id, key))
        variant = dict(content)
        if stage != "active":
            variant["marker"] = row["marker"]
            variant["map_to"] = row["map_to"]
        if stage == "active":
            if "description" in quest or "completed" in quest or quest.get("counted"):
                raise ValueError("line %d: a second active row for quest %d" % (number_of, id))
            quest["state"] = row["state"]
            quest.update(content)
        elif stage == "completed":
            if "completed" in quest:
                raise ValueError("line %d: a second completed row for quest %d" % (number_of, id))
            quest["completed"] = variant
        elif counted and stage.startswith("count:"):
            credits = number(stage[6:], 63)
            if not credits:
                raise ValueError("line %d: count 0" % number_of)
            rows = quest.setdefault("counted", [])
            if credits != len(rows) + 1:
                raise ValueError("line %d: counted variant %d is not contiguous" % (number_of, credits))
            variant["credits"] = credits
            rows.append(variant)
        else:
            raise ValueError("line %d: stage %r" % (number_of, stage))
    for id, quest in quests.items():
        if "description" not in quest and "counted" not in quest and "completed" not in quest:
            raise ValueError("quest %d has no details" % id)
    return quests


def snapshot_text(field):
    if not field:
        return None
    status = field.get("status")
    if status != "decoded":
        return None
    return field.get("encoded_utf16")


def read_evidence(text, skip=()):
    doc = json.loads(text)
    quests = {}
    for entry in doc.get("quests", []):
        id = entry["quest_id"]
        if id in skip:
            continue
        states = entry.get("states", [])
        active = None
        completed = None
        for state in states:
            quest = state["quest"]
            marker = quest["marker"]
            row = {
                "id": id,
                "state": quest["log_state"],
                "marker": check_marker(marker["x_bits"], marker["y_bits"], marker["plane"]),
                "map_to": quest["map_to"],
                "map_from": quest["map_from"],
                "location": snapshot_text(quest.get("category")),
                "name": snapshot_text(quest.get("name")),
                "npc": snapshot_text(quest.get("npc")),
                "description": snapshot_text(quest.get("description")),
                "objectives": snapshot_text(quest.get("objectives")),
            }
            for key in ("location", "name", "npc"):
                if not row[key]:
                    raise ValueError("quest %d: no decoded %s" % (id, key))
                if len(row[key]) > NAME_UNITS:
                    raise ValueError("quest %d: %s has %d units" % (id, key, len(row[key])))
            if row["description"] is None or row["objectives"] is None:
                # A snapshot that resolved only one half is not usable content; it is not a reason to
                # invent the other half.
                row["description"] = None
                row["objectives"] = None
            else:
                if len(row["description"]) > DETAIL_UNITS:
                    raise ValueError("quest %d: description has %d units" % (id, len(row["description"])))
                if len(row["objectives"]) > DETAIL_UNITS:
                    raise ValueError("quest %d: objectives has %d units" % (id, len(row["objectives"])))
            is_completed = bool(quest.get("completed")) or bool(quest["log_state"] & 2)
            if is_completed:
                if completed is None and row["description"] is not None:
                    completed = row
            else:
                if active is None and row["description"] is not None:
                    active = row
                elif active is not None and not (active["state"] & 1) and (row["state"] & 1):
                    # A later snapshot with the entry loaded is the better active row.
                    active = row
        if active is None and completed is None:
            # A log-only observation with no resolved detail text is not content this pack can carry.
            continue
        base = active or completed
        quest = {key: base[key] for key in ("id", "state", "marker", "map_to", "map_from",
                                            "location", "name", "npc")}
        quest["description"] = base["description"]
        quest["objectives"] = base["objectives"]
        if completed is not None and completed is not active:
            quest["completed"] = {"marker": completed["marker"],
                                  "map_to": completed["map_to"],
                                  "description": completed["description"],
                                  "objectives": completed["objectives"]}
        quests[id] = quest
    return quests


def merge(catalog, capture):
    merged = dict(catalog)
    for id, quest in capture.items():
        if id not in merged:
            merged[id] = quest
    for id, rule in RULES.items():
        if id not in merged:
            raise ValueError("a rule names quest %d, which neither source carries" % id)
        merged[id].update(rule)
    return merged


def check_all(quests):
    for id, quest in quests.items():
        if quest["id"] != id:
            raise ValueError("quest %d carries id %d" % (id, quest["id"]))
        if not isinstance(quest["state"], int):
            raise ValueError("quest %d: state is not a number" % id)
        if "description" in quest and "objectives" not in quest:
            raise ValueError("quest %d: description without objectives" % id)
        if "counted" in quest:
            credits = [row["credits"] for row in quest["counted"]]
            if credits != list(range(1, len(credits) + 1)):
                raise ValueError("quest %d: counted variants are not contiguous" % id)
        if "offer" in quest or "reward" in quest:
            if "offer" not in quest or "reward" not in quest:
                raise ValueError("quest %d: half an offer/reward pair" % id)
            offer, reward = quest["offer"], quest["reward"]
            if not offer.get("npc") or not offer.get("map"):
                raise ValueError("quest %d: an offer needs an NPC and a map" % id)
            if not isinstance(offer.get("requires_none_completed", []), list):
                raise ValueError("quest %d: requires_none_completed is not a list" % id)
            for other in offer.get("requires_none_completed", []):
                if not isinstance(other, int) or other <= 0:
                    raise ValueError("quest %d: a prerequisite id is invalid" % id)
            if not reward.get("npc") or not reward.get("map"):
                raise ValueError("quest %d: a reward needs an NPC and a map" % id)
            for key in ("experience", "gold"):
                if not isinstance(reward.get(key), int) or reward.get(key, -1) < 0:
                    raise ValueError("quest %d: reward %s is invalid" % (id, key))


def inline(value):
    return "[" + ", ".join(str(item) for item in value) + "]"


def format_value(value, indent=0):
    pad = "  " * indent
    if isinstance(value, dict):
        items = []
        for key, item in value.items():
            items.append("%s  %s: %s" % (pad, json.dumps(key), format_value(item, indent + 1)))
        return "{\n" + ",\n".join(items) + "\n" + pad + "}"
    if isinstance(value, list):
        if all(not isinstance(item, (dict, list)) for item in value):
            return inline(value)
        items = ["%s  %s" % (pad, format_value(item, indent + 1)) for item in value]
        return "[\n" + ",\n".join(items) + "\n" + pad + "]"
    return json.dumps(value)


def render(quests):
    document = {
        "note": "The quest content this world pack carries (crates/asura-world/src/quest.rs, docs/58-quests.md). Every unit is the live capture's: the client resolves encoded units through its own string table, and this server never composes or resolves them. A quest's marker is the capture's exact bits; both +infinity means the capture showed no marker.",
        "fields": FIELDS,
        "quests": [quests[id] for id in sorted(quests)],
    }
    return format_value(document) + "\n"


def read_source(root):
    if root:
        with open(os.path.join(root, FWQC), encoding="utf-8") as handle:
            catalog = handle.read()
        with open(os.path.join(root, EVIDENCE), encoding="utf-8") as handle:
            evidence = handle.read()
        return catalog, evidence
    with zipfile.ZipFile(ZIP) as archive:
        return (archive.read("preforged-main/" + FWQC).decode("utf-8"),
                archive.read("preforged-main/" + EVIDENCE).decode("utf-8"))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", help="an extracted preforged tree; default is the committed zip")
    parser.add_argument("--write", action="store_true", help="write the pack file")
    args = parser.parse_args()

    catalog_text, evidence_text = read_source(args.root)
    catalog = read_fwqc(catalog_text)
    capture = read_evidence(evidence_text, skip=set(catalog))
    quests = merge(catalog, capture)
    check_all(quests)
    text = render(quests)
    if args.write:
        with open(OUT, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(text)
        print("wrote %s: %d quests (%d catalog, %d capture-only)" %
              (os.path.relpath(OUT, ROOT), len(quests), len(catalog),
               len(set(capture) - set(catalog))))
    else:
        print("%d quests (%d catalog, %d capture-only), %d bytes" %
              (len(quests), len(catalog), len(set(capture) - set(catalog)), len(text.encode("utf-8"))))


if __name__ == "__main__":
    main()
