#!/usr/bin/env python3
"""Survey the creatures a set of live recordings show: what each NPC type is, where its bodies stand,
and what it did in a fight.

    python tools/mobs-from-capture.py recordings/client/feature/presearing [more files or folders]
        [--out survey.json] [--map 146]
    python tools/mobs-from-capture.py D:/captures.zip --out survey.json

A `.zip` is read in place: every `.jsonl` inside it is streamed line by line, so a 56 GB archive needs
no room to extract and no more memory than one survey. Folders are walked for `.jsonl` and `.zip` files.

Reads client-side captures (`tools/sniff-capture.py` format: struct-layer records, every field widened
to four bytes; preforged's PacketLogger `packets-*.jsonl`, the same layout under `opcode`/`hex`, with the
line number as seq) and follows each connection's map (`0x019A`). Per map and NPC definition it reports:

- the definition (`0x0056`): model file, level, primary profession, flags, name units;
- every body created with it (`0x0020`): team code, speed, first position, and the file and seq;
- maximum health (`0x009F` property 42) and level (property 36) as sent for its bodies;
- its attacks: attack timing (`0x0035`), attack-skill and cast starts (`0x00A0` kinds 50 and 60), instant
  skills (`0x009F` kind 48), and the skill ids they named;
- damage it dealt and took (`0x00A3` kinds 16, 17 and 55, as a fraction of the target's maximum health,
  turned into points where that maximum was sent), and the skill a hit was attributed to (`0x009F` kind
  10);
- deaths (`0x00F1` with the dead bit) and removals (`0x0021`);
- every visit (a `0x019A` and what followed it) that created a body on a fighting team, most first per
  map, with every body's create fields (position, plane, direction, speed, team, `h000b`, `h0023`) and
  the level sent for it: a population the live server really had, which `tools/mobs-to-pack.py` places.

It writes nothing into the pack. The output is evidence for `docs/52-skills.md` "Mobs": every number names
the file and seq it came from, so a pack value can cite it. The kinds are the ones
`asura_protocol::game::skill` names; a capture that disagrees with them shows up here as kinds the
survey did not expect, listed under `unexpected`.

Names: a sanitised capture masks text, and a definition's name units are the client's string ids, so a
human name comes from elsewhere (the wiki's roster for the area). `--names` takes a JSON file mapping
"map:definition_index" to a name to label the output. ASCII output only.
"""

import argparse
import collections
import io
import json
import os
import struct
import sys
import zipfile

NPC_CLASS = 2
PROPERTY_LEVEL = 36
PROPERTY_MAX_HEALTH = 42
PROPERTY_DAMAGE_SKILL = 10
PROPERTY_INSTANT = 48
PROPERTY_ATTACK_SKILL = 50
PROPERTY_CAST = 60
DAMAGE_KINDS = {16: "armor", 17: "critical", 55: "armor_ignoring"}
DEAD = 0x10
# How many samples of each kind a type keeps: the counts are exact, the samples are evidence.
MAX_KEPT = 50
HEADER_LOAD_INFO = 0x19A
HEADER_DEFINITION = 0x56
HEADER_CREATE = 0x20
HEADER_MODEL = 0x57
HEADER_PROJECTILE = 0xA4
HEADER_REMOVE = 0x21
HEADER_ATTACK_TIMING = 0x35
HEADER_INT = 0x9F
HEADER_INT_TARGET = 0xA0
HEADER_FLOAT = 0xA2
HEADER_FLOAT_TARGET = 0xA3
HEADER_EFFECTS = 0xF1
HEADER_ALLEGIANCE = 0x2F
# The team codes of bodies nobody fights: townsfolk and players.
PEACEFUL = {"nonc", "nonn", "play"}


def dwords(data_hex):
    raw = bytes.fromhex(data_hex)
    return [struct.unpack_from("<I", raw, at)[0] for at in range(0, len(raw) - 3, 4)]


def as_float(word):
    return struct.unpack("<f", struct.pack("<I", word & 0xFFFFFFFF))[0]


def fourcc(word):
    try:
        return struct.pack(">I", word).decode("ascii")
    except UnicodeDecodeError:
        return "0x%08x" % word


def units(data_hex, offset, capacity):
    raw = bytes.fromhex(data_hex)[offset:offset + capacity * 2]
    out = []
    for at in range(0, len(raw) - 1, 2):
        unit = struct.unpack_from("<H", raw, at)[0]
        if unit == 0:
            break
        out.append(unit)
    return out


def capture_files(paths):
    """(label, opener) for every capture: plain files, files in folders, and members of zips."""
    def from_zip(path):
        try:
            archive = zipfile.ZipFile(path)
        except zipfile.BadZipFile as err:
            print("skipped %s: %s" % (path, err), file=sys.stderr)
            return
        with archive:
            for member in archive.infolist():
                if member.filename.endswith(".jsonl") and not member.is_dir():
                    yield (
                        "%s!%s" % (os.path.basename(path), member.filename),
                        lambda member=member: io.TextIOWrapper(
                            archive.open(member), encoding="utf-8", errors="replace"
                        ),
                    )

    for path in paths:
        if os.path.isdir(path):
            for root, _dirs, files in os.walk(path):
                for name in sorted(files):
                    full = os.path.join(root, name)
                    if name.endswith(".jsonl"):
                        yield full, lambda full=full: open(full, encoding="utf-8", errors="replace")
                    elif name.endswith(".zip"):
                        yield from from_zip(full)
        elif path.endswith(".zip"):
            yield from from_zip(path)
        else:
            yield path, lambda path=path: open(path, encoding="utf-8", errors="replace")


def new_type():
    return {
        "definition": None,
        "bodies": 0,
        "positions": {},
        "max_health": collections.Counter(),
        "levels": collections.Counter(),
        "teams": collections.Counter(),
        "speeds": collections.Counter(),
        "attack_intervals": collections.Counter(),
        "skills_used": collections.Counter(),
        "damage_dealt": [],
        "dealt_hits": 0,
        "dealt_min": None,
        "dealt_max": None,
        "taken_hits": 0,
        "deaths": 0,
        "evidence": [],
        "projectiles": collections.Counter(),
    }


class Survey:
    def __init__(self, only_map=None):
        self.only_map = only_map
        self.types = collections.defaultdict(new_type)
        self.unexpected = collections.Counter()
        self.files = []
        # One record per map visit (a 0x019A), with every NPC body the visit created, so the pack can
        # place the population one visit really had instead of the union of many.
        self.visits = []
        # 0x0057's model files by definition index: a sanitised client recording carries them, the
        # PacketLogger records drop the array.
        self.model_files = {}

    def run(self, label, opener):
        name = label
        count = 0
        # Per connection: its map, the definitions it was sent, its NPC agents (agent id -> (map,
        # definition index)), their maximum health, and the skill a kind-10 line named for a source.
        state = {"map": None, "definitions": {}, "agents": {}, "max_health": {}, "pending_skill": {},
                 "visit": None, "levels": {}}
        with opener() as handle:
            for number, line in enumerate(handle, 1):
                try:
                    row = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if row.get("event") == "packet":
                    # preforged's PacketLogger: the same decoded-struct layout under other keys.
                    row = {"seq": number, "direction": row.get("direction"),
                           "header": row.get("opcode"), "data": row.get("hex")}
                if "seq" not in row or row.get("direction") != "StoC" or not row.get("data"):
                    continue
                count += 1
                self.step(row, name, state)
        self.files.append({"file": label, "records": count})

    def step(self, row, name, state):
        definitions, agents, max_health, pending_skill = (
            state["definitions"], state["agents"], state["max_health"], state["pending_skill"])
        current_map = state["map"]
        header = row["header"]
        try:
            w = dwords(row["data"])
        except ValueError:
            return
        where = "%s#%d" % (name, row["seq"])
        if header == HEADER_LOAD_INFO and len(w) > 2:
            current_map = state["map"] = w[2]
            state["visit"] = {"file": name, "seq": row["seq"], "map": w[2], "bodies": {}}
            self.visits.append(state["visit"])
            state["levels"].clear()
            agents.clear()
            max_health.clear()
            pending_skill.clear()
        elif header == HEADER_DEFINITION and len(w) > 8:
            definitions[w[1]] = {
                "definition_index": w[1], "file_id": w[2], "visual": w[4], "flags": w[6],
                "primary": w[7] & 0xFF, "level": w[8] & 0xFF, "name": units(row["data"], 36, 8),
                "seen": where,
            }
        elif header == HEADER_MODEL and len(w) > 3:
            count = w[2]
            if 0 < count <= 8 and len(w) >= 3 + count:
                self.model_files.setdefault(w[1], {"files": w[3:3 + count], "seen": where})
        elif header == HEADER_PROJECTILE and len(w) > 8:
            key = agents.get(w[1])
            if key:
                self.types[key]["projectiles"]["%d:%d" % (w[6], w[8] & 0xFF)] += 1
        elif header == HEADER_CREATE and len(w) > 14:
            agent, model = w[1], w[2]
            if model >> 28 != NPC_CLASS:
                return
            index = model & 0x0FFFFFFF
            if self.only_map is not None and current_map != self.only_map:
                return
            key = (current_map, index)
            agents[agent] = key
            kind = self.types[key]
            if kind["definition"] is None and index in definitions:
                kind["definition"] = definitions[index]
            team = fourcc(w[14])
            kind["teams"][team] += 1
            kind["speeds"][round(as_float(w[11]), 1)] += 1
            kind["bodies"] += 1
            # Spawn spots, deduplicated to 50 units: a body re-created on every load is one spot.
            spot = (round(as_float(w[5]) / 50) * 50, round(as_float(w[6]) / 50) * 50, w[7] & 0xFFFF)
            if spot not in kind["positions"] and len(kind["positions"]) < MAX_KEPT * 4:
                kind["positions"][spot] = where
            visit = state["visit"]
            if visit is not None and agent not in visit["bodies"]:
                visit["bodies"][agent] = {
                    "definition_index": index, "x": round(as_float(w[5]), 2),
                    "y": round(as_float(w[6]), 2), "plane": w[7] & 0xFFFF,
                    "direction": [round(as_float(w[8]), 6), round(as_float(w[9]), 6)],
                    "h000b": w[4] & 0xFF, "speed": round(as_float(w[11]), 2),
                    "h0023": round(as_float(w[12]), 6), "team": team,
                    "level": state["levels"].get(agent), "seen": where,
                }
        elif header == HEADER_INT and len(w) > 3:
            prop, agent, value = w[1], w[2], w[3]
            key = agents.get(agent)
            if prop == PROPERTY_MAX_HEALTH:
                max_health[agent] = value
                if key:
                    self.types[key]["max_health"][value] += 1
                    self.evidence(key, ["max_health", value, where])
            elif prop == PROPERTY_LEVEL:
                # Sent before the create for the animals, so it is kept by agent until one comes.
                state["levels"][agent] = value
                if key:
                    self.types[key]["levels"][value] += 1
            elif prop == PROPERTY_DAMAGE_SKILL:
                pending_skill[agent] = value
            elif prop == PROPERTY_INSTANT and key:
                self.types[key]["skills_used"]["instant:%d" % value] += 1
                self.evidence(key, ["instant", value, where])
        elif header == HEADER_INT_TARGET and len(w) > 4:
            prop, caster, target, value = w[1], w[2], w[3], w[4]
            key = agents.get(caster)
            if prop in (PROPERTY_ATTACK_SKILL, PROPERTY_CAST) and key:
                word = "attack" if prop == PROPERTY_ATTACK_SKILL else "cast"
                self.types[key]["skills_used"]["%s:%d" % (word, value)] += 1
                self.evidence(key, [word, value, where])
        elif header == HEADER_FLOAT_TARGET and len(w) > 4:
            prop, target, source, fraction = w[1], w[2], w[3], as_float(w[4])
            if prop not in DAMAGE_KINDS:
                self.unexpected["0x00A3 kind %d" % prop] += 1
                return
            points = None
            if target in max_health:
                points = round(-fraction * max_health[target], 1)
            hit = {
                "kind": DAMAGE_KINDS[prop], "fraction": round(fraction, 4), "points": points,
                "skill": pending_skill.pop(target, None), "seen": where,
            }
            if source in agents:
                kind = self.types[agents[source]]
                kind["dealt_hits"] += 1
                if len(kind["damage_dealt"]) < MAX_KEPT:
                    kind["damage_dealt"].append(hit)
                if points is not None:
                    kind["dealt_min"] = points if kind["dealt_min"] is None else min(kind["dealt_min"], points)
                    kind["dealt_max"] = points if kind["dealt_max"] is None else max(kind["dealt_max"], points)
            if target in agents:
                self.types[agents[target]]["taken_hits"] += 1
        elif header == HEADER_ATTACK_TIMING and len(w) > 3:
            key = agents.get(w[1])
            if key:
                self.types[key]["attack_intervals"][round(as_float(w[2]), 3)] += 1
        elif header == HEADER_EFFECTS and len(w) > 2:
            key = agents.get(w[1])
            if key and w[2] & DEAD:
                self.types[key]["deaths"] += 1
                self.evidence(key, ["dead", w[2], where])

    def evidence(self, key, entry):
        if len(self.types[key]["evidence"]) < MAX_KEPT:
            self.types[key]["evidence"].append(entry)

    def report(self, names):
        out = []
        for (map_id, index), kind in sorted(self.types.items(), key=lambda item: (item[0][0] or 0, item[0][1])):
            label = names.get("%s:%d" % (map_id, index))
            out.append({
                "map": map_id,
                "definition_index": index,
                "name": label,
                "definition": kind["definition"],
                "bodies": kind["bodies"],
                "teams": dict(kind["teams"]),
                "speeds": dict(kind["speeds"]),
                "levels": dict(kind["levels"]),
                "max_health": dict(kind["max_health"]),
                "attack_intervals": dict(kind["attack_intervals"]),
                "skills_used": dict(kind["skills_used"]),
                "damage_dealt": {
                    "hits": kind["dealt_hits"],
                    "points_min": kind["dealt_min"],
                    "points_max": kind["dealt_max"],
                    "samples": kind["damage_dealt"],
                },
                "damage_taken_hits": kind["taken_hits"],
                "deaths": kind["deaths"],
                "positions": [
                    {"x": x, "y": y, "plane": plane, "seen": seen}
                    for (x, y, plane), seen in kind["positions"].items()
                ],
                "evidence": kind["evidence"],
                "projectiles": dict(kind["projectiles"]),
            })
        # Every visit that created a body on a fighting team, most such bodies first per map: each is
        # one population the live server really had.
        visits = []
        for visit in self.visits:
            fighters = sum(1 for body in visit["bodies"].values() if body["team"] not in PEACEFUL)
            if fighters:
                visits.append((visit["map"], -fighters, visit))
        visits.sort(key=lambda item: (item[0], item[1], item[2]["file"], item[2]["seq"]))
        return {
            "note": "Generated by tools/mobs-from-capture.py. Evidence for the pack, not pack content.",
            "files": self.files,
            "model_files": {str(index): row for index, row in sorted(self.model_files.items())},
            "populations": [
                {"map": map_id, "file": visit["file"], "seq": visit["seq"], "fighters": -negative,
                 "bodies": [dict(body, agent=agent) for agent, body in sorted(visit["bodies"].items())]}
                for map_id, negative, visit in visits
            ],
            "unexpected": dict(self.unexpected),
            "types": out,
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("paths", nargs="+", help="capture files or folders of them")
    parser.add_argument("--out", help="write the survey here (default: a summary on stdout)")
    parser.add_argument("--map", type=int, help="only this map")
    parser.add_argument("--names", help='JSON: {"<map>:<definition_index>": "name"}')
    args = parser.parse_args()

    names = {}
    if args.names:
        with open(args.names, encoding="utf-8") as handle:
            names = json.load(handle)
    survey = Survey(args.map)
    for label, opener in capture_files(args.paths):
        print("reading %s" % label, file=sys.stderr)
        survey.run(label, opener)
    report = survey.report(names)
    if args.out:
        with open(args.out, "w", encoding="ascii") as handle:
            json.dump(report, handle, indent=1, ensure_ascii=True)
            handle.write("\n")
    print("%d file(s), %d NPC type(s)" % (len(report["files"]), len(report["types"])))
    for kind in report["types"]:
        d = kind["definition"] or {}
        print("map %-4s def %-5s model %-7s lvl %-3s bodies %-3d teams %-14s hp %-12s skills %-3d hits %-4d deaths %d%s" % (
            kind["map"], kind["definition_index"], d.get("file_id"), d.get("level"), kind["bodies"],
            ",".join(kind["teams"]), ",".join(str(h) for h in kind["max_health"]) or "-",
            len(kind["skills_used"]), kind["damage_dealt"]["hits"], kind["deaths"],
            "  " + kind["name"] if kind["name"] else ""))
    if report["unexpected"]:
        print("unexpected: %s" % report["unexpected"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
