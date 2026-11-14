"""Split probed items into base items and upgrades (docs/50-item-framework.md, "Base + upgrade").

The item probe reads each item's *display* name from the client: the name the client composed from the
base name and the fitted upgrades (`0xa31(base, insignia, rune)` and its kin). The live server never
sends that composed form - across 584 live CREATE_NAMED_ITEM records not one name starts with a
template (recordings/client/feature/{titles,multiplay,maps}) - and sending it makes the client add the
upgrades a second time ("Knight's Knight's Templar Helm of Vitae of Vitae").

This tool reads config/World/<pack>/items.json and writes a review file,
recordings/client/feature/items/items-split.json: one row per probed item with its base name units, its
base words and its fitted upgrade ids.

The word grammar, read from the probes and the captures:

* an upgrade id word is 0x2408XXXX (XXXX the upgrade id);
* its pair words have identifier 0x2530/0x2532 (or the same with bit 31 set) and low 16 bits equal to
  2*id or 2*id+1; 0x8070 is a second pair form some insignias carry;
* each pair word is followed by exactly one effect word of the same half (bit 31 clear after a low
  pair, set after a high pair). An attribute rune repeats its pair word, so it has two (pair, effect)
  couples: `21e8...` the bonus and `20d8...` its health cost;
* the effect word's *type* (its top 16 bits) is the upgrade's; its *value* is the instance's. Vitae is
  always 0x23480a00, but a weapon mod rolls: upgrade 0x9d carries Health +28, +10 and +30 on three
  probed items. So upgrades.json records each upgrade's word shape, and items.split.json keeps each
  item's actual words;
* everything else is the base's own.

Run:  python tools/split-item-upgrades.py [--pack config/World/Vanilla] [--write]

Without --write it only writes the review file. With --write it also writes the pack's
`upgrades.json` (what the world server loads) and rewrites `items.json` in place: every probed item's
`name` becomes the bare base name the live server sends, and it gains `base` (the base it is a copy of)
and `upgrades` (the upgrade ids fitted to it). Its wire identity and its modifier words - the actual,
rolled words - are not touched. It touches nothing under crates/.
"""

import argparse
import collections
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
REVIEW = ROOT / "recordings" / "client" / "feature" / "items" / "items-split.json"
ARG_MARKERS = {0x10A, 0x10B, 0x10C, 0x10D, 0x10E, 0x10F}
ID_WORD = 0x2408
PAIR_IDS = {0x2530, 0x2532, 0xA530, 0xA532, 0x8070}
# The client's composed-name templates (first word of the probe name), and which argument holds what.
WRAPPERS = {0xA3D, 0xA3F, 0xA40, 0xA42, 0xA43}
# `0xa30(base, prefix)`, `0xa31(base, prefix, suffix)`, `0xa33(base, suffix)`. On armour the prefix is
# the insignia and the suffix the rune; on a weapon they are the weapon's prefix and suffix mods.
COMPOSE = {0xA30: ("prefix",), 0xA31: ("prefix", "suffix"), 0xA33: ("suffix",)}


def parse(units, i=0):
    """One encoded string: id words (bit 15 continues), then (marker, string, 0x0001) arguments."""
    ids = []
    while i < len(units):
        word = units[i]
        ids.append(word)
        i += 1
        if not word & 0x8000:
            break
    # An encrypted string carries key words after its id (the backpack's 0x21a8 0xd157 0xb58f 0x166f);
    # they belong to the string, up to the next argument marker or the argument's end.
    while i < len(units) and units[i] not in ARG_MARKERS and units[i] != 1:
        ids.append(units[i])
        i += 1
    args = []
    while i < len(units) and units[i] in ARG_MARKERS:
        i += 1
        sub, i = parse(units, i)
        args.append(sub)
        if i < len(units) and units[i] == 1:
            i += 1
    return (ids, args), i


def flatten(node):
    ids, args = node
    out = list(ids)
    for marker, sub in zip(sorted(ARG_MARKERS), args):
        out.append(marker)
        out.extend(flatten(sub))
        out.append(1)
    return out


def split_name(units):
    """(base units, {role: fragment units}) for a probe display name."""
    units = [u for u in units if u != 0]
    if not units or units[0] not in WRAPPERS:
        return units, {}
    node, _ = parse(units)
    ids, args = node
    if len(ids) == 1 and ids[0] in WRAPPERS and args:
        node = args[0]
        ids, args = node
    roles = {}
    if len(ids) == 1 and ids[0] in COMPOSE and args:
        base = args[0]
        for role, arg in zip(COMPOSE[ids[0]], args[1:]):
            roles[role] = flatten(arg)
        return flatten(base), roles
    return flatten(node), roles


def is_id(word):
    return word >> 16 == ID_WORD


def pair_of(word, ids):
    if word >> 16 not in PAIR_IDS:
        return None
    low = word & 0xFFFF
    for upgrade in ids:
        if low in (2 * upgrade, 2 * upgrade + 1):
            return upgrade
    return None


def runs(words):
    """For one item: upgrade ids in order, and for each pair word the run of words after it up to the
    next id or pair word (a candidate effect list, refined across items)."""
    ids = [w & 0xFFFF for w in words if is_id(w)]
    found = []
    i = 0
    while i < len(words):
        upgrade = pair_of(words[i], ids)
        if upgrade is None:
            i += 1
            continue
        j = i + 1
        while j < len(words) and not is_id(words[j]) and pair_of(words[j], ids) is None:
            j += 1
        found.append((upgrade, i, words[i], words[i + 1:j]))
        i = j
    return ids, found


def common_prefix(lists):
    out = []
    for column in zip(*lists):
        if all(c == column[0] for c in column):
            out.append(column[0])
        else:
            break
    return out


def base_key(row):
    """A stable, readable key for the base an item is a copy of: its type and its base name units."""
    return f"t{row['item_type']}_" + "_".join(f"{u:x}" for u in row["base_name"])


def write_pack(pack, definitions, rows, upgrades):
    """The server-facing files. `upgrades.json` holds each upgrade's identity, role, name fragment and
    word shape; `items.json` keeps every field it had, with the bare base name and two new fields."""
    by_id = {row["id"]: row for row in rows}
    for d in definitions:
        row = by_id[d["id"]]
        if row["upgrades"] or row["name_fragments"] or (d["name"] and d["name"][0] in WRAPPERS):
            d["name"] = row["base_name"]
        d["base"] = base_key(row)
        d["upgrades"] = row["upgrades"]
    path = pack / "items.json"
    original = json.loads(path.read_text(encoding="utf-8"))
    original["definitions"] = definitions
    with open(path, "w", encoding="utf-8", newline="\r\n") as out:
        out.write(json.dumps(original, indent=2) + "\n")

    # Roles and name fragments are learned from composed display names, which this tool removes from
    # items.json. A later run sees only bare names, so what an earlier run learned is kept from the
    # existing upgrades.json rather than forgotten.
    previous = {}
    existing = pack / "upgrades.json"
    if existing.exists():
        previous = {u["id"]: u for u in json.loads(existing.read_text(encoding="utf-8"))["upgrades"]}
    served = []
    for u in sorted(upgrades.values(), key=lambda u: u["id"]):
        roles = {f["role"] for f in u["fragments"]}
        role = roles.pop() if len(roles) == 1 else "unknown"
        name = u["fragments"][0]["units"] if len(u["fragments"]) == 1 else []
        earlier = previous.get(u["id"])
        if earlier and role == "unknown":
            role, name = earlier["role"], earlier["name"]
        served.append({"id": u["id"], "role": role, "name": name, "shape": u["shape"], "seen_on": u["seen_on"]})
    note = (
        "Item upgrades: insignias, runes and weapon mods. An upgrade is identified by the 0x2408XXXX word "
        "it puts in an item's modifier list; `shape` is the top 16 bits of each word it contributes, in "
        "order (id word, then pair word and effect word per effect). The effect word's value is rolled per "
        "item and lives in the item, not here. `name` is the fragment the client composes into the item's "
        "display name (role prefix or suffix); the server never sends it. Generated by "
        "tools/split-item-upgrades.py from items.json; see docs/50-item-framework.md."
    )
    with open(pack / "upgrades.json", "w", encoding="utf-8", newline="\r\n") as out:
        out.write(json.dumps({"note": note, "upgrades": served}, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--pack", default="config/World/Vanilla")
    parser.add_argument("--write", action="store_true", help="write upgrades.json and rewrite items.json")
    args = parser.parse_args()
    pack = pathlib.Path(args.pack)
    definitions = json.loads((pack / "items.json").read_text(encoding="utf-8"))["definitions"]

    def effect_of(pair, after):
        """The one word after a pair word, when it is of the pair's half."""
        if after and bool(after[0] & 0x80000000) == bool(pair & 0x80000000):
            return [after[0]]
        return []

    upgrades = {}
    rows = []
    problems = []
    for d in definitions:
        words = list(d["modifiers"])
        ids, found = runs(words)
        upgrade_words = collections.defaultdict(list)
        taken = set()
        for upgrade in ids:
            i = next(k for k, w in enumerate(words) if is_id(w) and w & 0xFFFF == upgrade and k not in taken)
            taken.add(i)
            upgrade_words[upgrade].append(words[i])
        for upgrade, at, pair, _after in found:
            effect = effect_of(pair, _after)
            taken.add(at)
            upgrade_words[upgrade].append(pair)
            for k, word in enumerate(effect, start=at + 1):
                if k >= len(words) or words[k] != word:
                    problems.append(f"{d['id']}: upgrade {upgrade:#x} effect does not line up")
                    break
                taken.add(k)
                upgrade_words[upgrade].append(word)
        base_words = [w for k, w in enumerate(words) if k not in taken]
        base_name, fragments = split_name(d["name"])
        for upgrade, uwords in upgrade_words.items():
            shape = [w >> 16 for w in uwords]
            known = upgrades.setdefault(
                upgrade, {"id": upgrade, "shape": shape, "words": uwords, "values": [], "seen_on": [], "fragments": []}
            )
            if uwords not in known["values"]:
                known["values"].append(uwords)
            if known["shape"] != shape:
                problems.append(
                    f"upgrade {upgrade:#x}: {d['id']} has word types {[hex(w) for w in shape]}, "
                    f"earlier {[hex(w) for w in known['shape']]}"
                )
            known["seen_on"].append(d["id"])
        rows.append(
            {
                "id": d["id"],
                "description": d["description"],
                "item_type": d["item_type"],
                "file_id": d["file_id"],
                "model_id": d["model_id"],
                "base_name": base_name,
                "name_fragments": fragments,
                "base_words": base_words,
                "upgrades": ids,
            }
        )

    # Name fragments. An item with one upgrade and one fragment names it outright; an item with two of
    # each names the second once the first is known. Repeat until nothing new is learned. Upgrades the
    # client does not name (an inscription, a weapon's built-in mod) stay unnamed.
    def attach(upgrade, role, units):
        frag = {"role": role, "units": units}
        if frag not in upgrades[upgrade]["fragments"]:
            upgrades[upgrade]["fragments"].append(frag)
            return True
        return False

    def known_role(upgrade):
        roles = {f["role"] for f in upgrades[upgrade]["fragments"]}
        return roles.pop() if len(roles) == 1 else None

    changed = True
    while changed:
        changed = False
        for row in rows:
            ups, frags = row["upgrades"], row["name_fragments"]
            if not ups or not frags:
                continue
            if len(ups) == 1 and len(frags) == 1:
                (role, units), = frags.items()
                changed |= attach(ups[0], role, units)
                continue
            if len(frags) == len(ups):
                unknown = [u for u in ups if known_role(u) is None]
                free_roles = [r for r in frags if all(known_role(u) != r for u in ups)]
                if len(unknown) == 1 and len(free_roles) == 1:
                    changed |= attach(unknown[0], free_roles[0], frags[free_roles[0]])

    # A base is its name; the same base must always come out with the same words.
    by_base = collections.defaultdict(set)
    for row in rows:
        if row["upgrades"]:
            by_base[tuple(row["base_name"])].add(tuple(row["base_words"]))
    inconsistent = {k: v for k, v in by_base.items() if len(v) > 1}

    review = REVIEW if REVIEW.parent.is_dir() else pack / "items-split.json"
    review.write_text(json.dumps(rows, indent=1), encoding="utf-8")
    if args.write and not problems:
        write_pack(pack, definitions, rows, upgrades)

    with_upgrades = sum(1 for r in rows if r["upgrades"])
    print(f"{len(rows)} items, {with_upgrades} with upgrades, {len(upgrades)} distinct upgrades")
    print(f"{len(by_base)} upgraded bases; {len(inconsistent)} base(s) with differing base words")
    for k, v in list(inconsistent.items())[:10]:
        print("  base", [hex(u) for u in k], [[hex(w) for w in ws] for ws in v])
    print(f"{len(problems)} problem(s)")
    for p in problems[:20]:
        print("  " + p)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
