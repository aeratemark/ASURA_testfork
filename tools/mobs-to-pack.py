#!/usr/bin/env python3
"""Write the pre-Searing creatures into the pack from a survey (`tools/mobs-from-capture.py --out`).

    python tools/mobs-to-pack.py survey.json config/World/Vanilla

For every hostile creature type the live server sent on the pre-Searing explorables it writes one
definition into `npcs/definitions.json` (id `mob-<definition index>-<label>`) with a combat block, and
for each of those maps the bodies of one real visit into the map's `npcs.json` (ids `mob-<map>-<n>`).
Running it again replaces what it wrote before and nothing else.

Where each number comes from:
- recorded: definition fields (0x0056), create fields (0x0020), maximum health (0x009F 42), attack
  interval (0x0035), the skills it was seen using (0x00A0 50/60, 0x009F 48), and every body's position;
- the wiki's creature formula: armor = 3 x level, +20 for a warrior, +10 for a ranger;
- assumed, said so in docs/52-skills.md "Mobs": attribute rank 0.6 x level; the damage range, fitted
  from recorded plain hits against an assumed victim armor of 25 where the survey has hits, else
  level-based; **ranged only where the capture recorded a projectile launch for that type** (a
  non-warrior profession is not evidence of range: a wolf is not an archer); caster energy 30 and 4
  pips; health 80 + 20 x level where nothing was recorded.
A skill the catalogue (`skills.json`) does not hold is left off the bar and listed on stdout.
ASCII output only.
"""

import collections
import json
import math
import os
import re
import sys

MAPS = {145: "Explorables", 146: "Explorables", 147: "Explorables", 160: "Explorables",
        161: "Explorables", 162: "Explorables"}
HOSTILE = {"mon1", "mons", "band", "unde", "aniv", "char"}

# Types the live client leaves neutral until a player hits them (owner, 2026-11-13: the River
# Skale Tad; owner + GWW "Plague Worm", 2026-11-14: the worms by default do not attack). The battle
# team does not say this: the catalog marks the tad mon1 like the always-hostile skales. A provoked
# one joins the fight against whoever damaged it and resets at home. 1442/1444 are the Plague Worms,
# 1445 the Worm Queen.
WHEN_PROVOKED = {1433, 1442, 1444, 1445}
# A projectile the survey could not see: the tad stands aside until hit, so a capture that never
# provoked it has no launch for it. The owner's live client shows its blue spit, and the adult River
# Skale (same file 82023) carries model 1; copy the family's measured spit rather than invent one.
PROJECTILE_OVERRIDES = {1433: (1, 0)}
# Types that never move: they stay on the placement point and spit from there. The owner's live
# client on 2026-11-14: the Plague Worms of Lakeside County do not walk; the worm definitions
# (1442 Lakeside, 1444 Regent Valley, 1445 the Worm Queen) fight from where they were placed.
STATIONARY = {1442, 1444, 1445}
# Quest-only creatures: the Vanguard bandits of Farmer Hamnet (level 11), and the Charr of the
# "Charr at the gate" event outside the Northlands.
EXCLUDED = {8657, 8658}
CHARR_MAPS = {147}
# Labels, for ids and docs only: the client names a body from the definition's own encoded string.
# "decoded": preforged's loot reports pair the string with the name; "model": preforged's raider
# placements; "wiki": the area's foe list at that level and profession.
LABELS = {
    1420: ("bandit-firestarter", "decoded"), 1421: ("bandit-raider", "model"),
    1422: ("bandit-blood-sworn", "wiki"), 1431: ("river-skale", "decoded"),
    1434: ("river-skale-tad", "decoded"), 1436: ("river-skale-brood", "decoded"),
    1432: ("skale-broodcaller", "wiki"), 1437: ("grawl", "decoded"),
    1438: ("grawl-longspear", "wiki"), 1439: ("grawl-shaman", "wiki"),
    1428: ("aloe-seed", "decoded"), 1444: ("plague-worm", "decoded"),
    1414: ("stone-elemental", "wiki"), 1415: ("hulking-stone-elemental", "wiki"),
    1412: ("ice-elemental", "wiki"),
}
VICTIM_ARMOR = 25
DAMAGE = {1: "slashing", 2: "piercing", 3: "holy", 4: "cold", 5: "piercing", 6: "fire"}
# A ranged type fires its captured projectile, so its damage follows the shot and not the
# profession's assumed word. GWW "Plague Worm": they spit but "actually fire normal arrows", and
# their 0x00A7 kind is the weapon's 587 (piercing), not a warrior's slashing.
RANGED_DAMAGE = {1442: "piercing", 1444: "piercing", 1445: "piercing"}
SWORD, AXE = 27, 2
WEAPON_OF_SKILL = {330: AXE, 382: SWORD, 384: SWORD}


# The townsfolk-team bodies preforged placed where these creatures live, by their literal name: the
# creatures replace them. A person sharing a creature's model file stays.
STAND_INS = {"River Skale Tad", "River Skale", "Stone Elemental", "Hulking Stone Elemental",
             "Ice Elemental", "Aloe Seed", "Restless Corpse"}


def literal(placement):
    units = placement.get("name") or []
    return "".join(chr(u) for u in units[2:-1] if u < 128)


def mode(counter):
    return max(counter.items(), key=lambda item: (item[1], item[0]))[0] if counter else None


def map_dir(pack, map_id):
    return os.path.join(pack, "maps", "Prophecies", "Ascalon (pre-Searing)", MAPS[map_id], str(map_id))


def main():
    survey_path, pack = sys.argv[1], sys.argv[2]
    survey = json.load(open(survey_path))
    catalogue = json.load(open(os.path.join(pack, "skills.json")))
    known = {row["id"] for row in catalogue["skills"]["value"]} if isinstance(catalogue.get("skills"), dict) \
        else {row["id"] for row in catalogue["skills"]}

    # Every surveyed type, merged across maps by definition index.
    merged = {}
    for kind in survey["types"]:
        if kind["map"] not in MAPS or not kind["definition"]:
            continue
        index = kind["definition_index"]
        teams = set(kind["teams"])
        if index in EXCLUDED or not teams & HOSTILE and not any(t.startswith("0x") for t in teams):
            continue
        if teams == {"char"} and kind["map"] not in CHARR_MAPS:
            continue
        m = merged.setdefault(index, {"definition": kind["definition"], "health": collections.Counter(),
                                      "intervals": collections.Counter(), "skills": collections.Counter(),
                                      "hits": [], "maps": set(), "projectiles": collections.Counter()})
        for k, v in kind.get("projectiles", {}).items():
            m["projectiles"][k] += v
        m["maps"].add(kind["map"])
        for k, v in kind["max_health"].items():
            m["health"][int(k)] += v
        for k, v in kind["attack_intervals"].items():
            m["intervals"][float(k)] += v
        for k, v in kind["skills_used"].items():
            m["skills"][int(k.split(":")[1])] += v
        m["hits"] += [s["points"] for s in kind["damage_dealt"]["samples"]
                      if s["kind"] == "armor" and s["skill"] is None and s["points"] and s["points"] > 0]

    definitions, missing = [], collections.Counter()
    ids = {}
    for index, m in sorted(merged.items()):
        d = m["definition"]
        level, prof = d["level"], d["primary"]
        label = LABELS.get(index, ("type", "unnamed"))[0]
        ident = "mob-%d-%s" % (index, label)
        ids[index] = ident
        health = mode(m["health"]) or (80 + 20 * level if level else 10)
        rank = int(round(0.6 * level))
        skills = []
        for skill, _ in m["skills"].most_common():
            (skills if skill in known else []).append(skill)
            if skill not in known:
                missing[skill] += 1
        skills = skills[:8]
        interval = mode(m["intervals"])
        period = int(round(interval * 1000)) if interval else (1750 if prof != 2 else 2000)
        mult = 2 ** ((5 * rank - VICTIM_ARMOR) / 40.0)
        if m["hits"]:
            low = max(1, int(math.floor(min(m["hits"]) / mult)))
            high = max(low, int(math.ceil(max(m["hits"]) / mult)))
        else:
            low, high = 2 + level, 4 + 2 * level
        item = next((WEAPON_OF_SKILL[s] for s in skills if s in WEAPON_OF_SKILL), 0)
        caster = prof in (3, 4, 5, 6) and bool(skills)
        shot = mode(m["projectiles"])
        if not shot and index in PROJECTILE_OVERRIDES:
            projectile = dict(zip(("projectile", "projectile_flag"), PROJECTILE_OVERRIDES[index]))
        else:
            projectile = dict(zip(("projectile", "projectile_flag"), map(int, shot.split(":")))) if shot else {}
        if projectile.get("projectile") == 0:
            projectile = {}
        # Range is measured, not inferred from the profession: a type the capture never saw launch
        # something fights in melee. Marking it ranged with no model is an invisible projectile.
        ranged = bool(projectile)
        team = next((t for t in sorted(m.get("teams", [])) if t in HOSTILE), None)
        definitions.append({
            "id": ident, "definition_index": index, "file_id": d["file_id"], "h000a": 0,
            "visual": d["visual"], "h0012": 0, "flags": d["flags"], "primary": prof, "level": level,
            "name": d["name"][:8], "model_files": [], "h000b": 8, "speed": 288.0, "team": "mon1",
            "combat": {
                "hostile": True,
                **({"stationary": True} if index in STATIONARY else {}),
                **({"aggression": "when_provoked"} if index in WHEN_PROVOKED else {}),
                "health": health, "armor": 3 * level + {1: 20, 2: 10}.get(prof, 0),
                "energy": 30 if caster else 0, "energy_regen": 4 if caster else 0, "rank": rank,
                "profession": prof,
                "attack": {"item_type": item, "period_ms": period,
                           "damage_type": RANGED_DAMAGE.get(index, DAMAGE.get(prof, "slashing")),
                           "ranged": ranged, "min": low, "max": high, **projectile},
                "skills": skills,
            },
        })

    # Placements: per map, the visit with the most bodies of these types.
    by_map = {}
    for visit in survey["populations"]:
        if visit["map"] not in MAPS:
            continue
        bodies = [b for b in visit["bodies"] if b["definition_index"] in ids
                  and (b["team"] in HOSTILE or b["team"].startswith("0x"))]
        if len(bodies) > len(by_map.get(visit["map"], (None, []))[1]):
            by_map[visit["map"]] = (visit, bodies)
    # The create fields a definition carries, from its first body.
    first = {}
    for _visit, bodies in by_map.values():
        for b in bodies:
            first.setdefault(b["definition_index"], b)
    for row in definitions:
        b = first.get(row["definition_index"])
        if b:
            row["h000b"], row["speed"] = b["h000b"], float(b["speed"])
            row["team"] = b["team"] if re.fullmatch(r"[ -~]{4}", b["team"]) else "mon1"
    definitions = [row for row in definitions if row["definition_index"] in first]
    # 0x0057's model files: the captured ones for the index, else another index's of the same model
    # file. A creature with none is drawn as a white box by the client.
    models = survey.get("model_files", {})
    by_file = {}
    for index, row in models.items():
        for d in survey["types"]:
            if d["definition"] and str(d["definition_index"]) == index:
                by_file.setdefault(d["definition"]["file_id"], row["files"])
    for row in definitions:
        files = (models.get(str(row["definition_index"])) or {}).get("files") or by_file.get(row["file_id"])
        if files:
            row["model_files"] = files
        else:
            print("no model files for %s: the client may draw it as a box" % row["id"])
    placed = {row["definition_index"] for row in definitions}

    path = os.path.join(pack, "npcs", "definitions.json")
    file = json.load(open(path))
    mob_models = {row["file_id"] for row in definitions}
    replaced = {row["id"] for row in file["definitions"]
                if row["team"] == "nonc" and row["file_id"] in mob_models and not row["id"].startswith("mob-")}
    file["definitions"] = [row for row in file["definitions"] if not row["id"].startswith("mob-")] + definitions
    json.dump(file, open(path, "w"), indent=2, ensure_ascii=True)
    open(path, "a").write("\n")

    for map_id, (visit, bodies) in sorted(by_map.items()):
        path = os.path.join(map_dir(pack, map_id), "npcs.json")
        mf = json.load(open(path))
        before = len(mf["placements"])
        mf["placements"] = [p for p in mf["placements"]
                            if not p["id"].startswith("mob-")
                            and not (p["definition"] in replaced and literal(p) in STAND_INS)]
        dropped = before - len(mf["placements"])
        n = 0
        for b in bodies:
            if b["definition_index"] not in placed:
                continue
            n += 1
            row = {"id": "mob-%d-%d" % (map_id, n), "definition": ids[b["definition_index"]],
                   "x": b["x"], "y": b["y"], "plane": b["plane"], "direction": b["direction"],
                   "effects": 0, "h0023": b["h0023"]}
            if b.get("level") is not None:
                row["level"] = b["level"]
            mf["placements"].append(row)
        json.dump(mf, open(path, "w"), indent=2, ensure_ascii=True)
        open(path, "a").write("\n")
        print("map %d: %d creatures from %s seq %d (%d townsfolk-team stand-ins removed)"
              % (map_id, n, os.path.basename(visit["file"]), visit["seq"], dropped))
    print("%d creature types" % len(definitions))
    if missing:
        print("skills seen but not in skills.json (left off the bars): %s" % dict(missing))
    return 0


if __name__ == "__main__":
    sys.exit(main())
