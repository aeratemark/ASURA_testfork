#!/usr/bin/env python3
"""Fill the pack's NPC interactions from preforged's pre-Searing captures.

    python tools/npc-content-from-captures.py --preforged D:/preforged-main          print a summary
    python tools/npc-content-from-captures.py --preforged D:/preforged-main --write  write the pack

Run it after `tools/quests-from-preforged.py --write` and `tools/vendors-from-preforged.py --write`: it
reads the pack files those write and adds to them.

Sources, all read-only, under `<preforged>/data/captures/` (`CAPTURES` below). Each is a collector pair:
`packets-*.jsonl` holds the live server's messages in the client's struct layer (a four-byte header,
every scalar widened to four bytes, strings as fixed u16 buffers), `world-*.jsonl` holds the
collector's observations - which agent is which named NPC, on which map, and the quest log snapshots.

What it writes:

* `quests.json`: the quests the captures' log snapshots carry and the pack does not (War Preparations
  217/218/220-222, Rites of Remembrance 38, ...), and an offer and a reward block for every quest whose
  offer page *and* turn-in page the captures hold. The experience, gold and skill points are the
  game's own reward text in the snapshot ("Reward: 250 Experience"); the taught skills, professions and
  prerequisites are `RULES` below, each with its source.
* `dialogs.json`: every quest NPC's pages, and a talk page for every other NPC the captures show
  talking. The NPC owns no window service here; the hand-written entries that are not generated (The
  Purveyor) are kept.
* `vendors.json`: Hamish, whose window is the same captured stock as Sanura's, and Gift-Giver Pinn,
  whose captured window is a collector's exchange.

How a dialog button is read. A quest button's id is the retail one: `0x800000 + (quest << 8) + step`.
Every step is identified by what the captures show around it: 01 accepts (the quest is added),
02 declines, 03 asks about the quest (the offer page follows), 04 is a talk objective (a short reply
follows), 05 is the giver's "how is it going" (a short reply follows), 06 asks for the reward (the
turn-in page follows) and 07 takes the turn-in (the quest is removed). Any other id is the NPC's own
conversation; its answer is the short page the capture shows next, or nothing.
"""

import argparse
import bisect
import collections
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PACK = os.path.join(ROOT, "config", "World", "Vanilla")

CAPTURES = [
    ("pre-searing-side-quests-2026-10-12/checkpoint-1815/packets-43292-168891734.jsonl",
     "pre-searing-side-quests-2026-10-12/checkpoint-1815/world-43292-168890171.jsonl"),
    ("pre-searing-four-professions-2026-10-11/packets-30352-81363781.jsonl",
     "pre-searing-four-professions-2026-10-11/world-30352-81344734.jsonl"),
    ("pre-searing-ranger-partial-2026-10-10/packets-7108-102222937.jsonl",
     "pre-searing-ranger-partial-2026-10-10/world-7108-102222171.jsonl"),
    ("pre-searing-ele-warrior-2026-10-09/packets-21116-95422890.jsonl",
     "pre-searing-ele-warrior-2026-10-09/world-21116-95410734.jsonl"),
    ("pre-searing-warrior-ele-2026-10-09/packets-13908-99235906.jsonl",
     "pre-searing-warrior-ele-2026-10-09/world-13908-99220828.jsonl"),
    ("farmer-hamnet-2026-10-30/packets-7528-73582328.jsonl",
     "farmer-hamnet-2026-10-30/world-7528-73583093.jsonl"),
]

# This build's numbering, as every capture's own schema records it.
BODY, BUTTON, SENDER = 128, 126, 129
WINDOW_OWNER, ITEM, WINDOW_IDS, MERCHANT_OPEN, COLLECTOR_OPEN = 196, 353, 132, 195, 197

ACCEPT, DECLINE, ASK, TALK, REMIND, ASK_REWARD, TURN_IN = 1, 2, 3, 4, 5, 6, 7

PROFESSION_TESTS = [81, 82, 83, 84, 85, 86]

# What the captures cannot say: who may take a quest and which skills it teaches. Every entry names its
# source. Professions are the client's numbers: 1 warrior, 2 ranger, 3 monk, 4 necromancer, 5 mesmer,
# 6 elementalist. `primary_only` and the secondary rule are preforged's `eligible`.
#   P = preforged src/runtime/gw_presearing_quests.h (rules[], acceptance_skills, reward_skills)
#   M = preforged src/runtime/gw_quest_menu.h (the level and admission gates)
#   I = preforged docs/presearing-quest-inventory-20261030.txt
#   C = the captures: Sir Tydus offers War Preparations 217-222, one per trainer who takes its turn-in
RULES = {
    80: {"offer": {"requires_none_completed": [35, 217, 218, 219, 220, 221, 222]}},  # P, I
    217: {"offer": {"professions": [5], "primary_only": True}},  # C: Sebedoh takes the turn-in
    218: {"offer": {"professions": [4], "primary_only": True}},  # C: Verata
    219: {"offer": {"professions": [6], "primary_only": True}},  # C: Howland
    220: {"offer": {"professions": [3], "primary_only": True}},  # C: Ciglo
    221: {"offer": {"professions": [1], "primary_only": True}},  # C: Van
    222: {"offer": {"professions": [2], "primary_only": True}},  # C: Artemis
    81: {"offer": {"professions": [5], "primary_only": True}, "reward": {"skills": [26, 40]}},  # P
    82: {"offer": {"professions": [4], "primary_only": True}, "reward": {"skills": [153, 105]}},  # P
    83: {"offer": {"professions": [6], "primary_only": True}, "reward": {"skills": [194, 180]}},  # P
    84: {"offer": {"professions": [3], "primary_only": True}, "reward": {"skills": [288, 252]}},  # P
    85: {"offer": {"professions": [1], "primary_only": True}, "reward": {"skills": [346, 1]}},  # P
    86: {"offer": {"professions": [2], "primary_only": True}, "reward": {"skills": [394, 446]}},  # P
    54: {"offer": {"requires_any_rewarded": PROFESSION_TESTS}},  # P requires_primary_test
    65: {"offer": {"professions": [6], "primary_only": True, "requires_rewarded": [54]}},  # I, M
    62: {"offer": {"requires_any_rewarded": PROFESSION_TESTS},
         "reward": {"requires_completed": True}},  # P; the scribe's talk is modelled
    79: {"offer": {"requires_any_rewarded": PROFESSION_TESTS},
         "reward": {"requires_completed": True}},  # P; the trainer's talk is modelled
    89: {"offer": {"requires_any_rewarded": PROFESSION_TESTS}},  # P
    55: {"offer": {"min_level": 2}, "reward": {"requires_completed": True}},  # M; Orion's talk
    46: {"offer": {"min_level": 2}},  # I: "Level2 Rurik148 acceptance"
    76: {"offer": {"professions": [2], "requires_any_rewarded": PROFESSION_TESTS,
                   "skills": [436, 446, 411]}},  # P
    75: {"offer": {"professions": [1], "requires_any_rewarded": PROFESSION_TESTS,
                   "skills": [382, 384, 1]}},  # P
    73: {"offer": {"professions": [6], "requires_any_rewarded": PROFESSION_TESTS,
                   "skills": [197, 180, 200]}},  # P
    71: {"offer": {"professions": [5], "requires_any_rewarded": PROFESSION_TESTS,
                   "skills": [31, 40, 76]}},  # P
    72: {"offer": {"professions": [4], "requires_any_rewarded": PROFESSION_TESTS}},  # I: Munne trial
    74: {"offer": {"professions": [3], "requires_any_rewarded": PROFESSION_TESTS,
                   "skills": [296, 281]}},  # I: "Mhenlo164 acceptance296/281"
    42: {"offer": {"professions": [2], "skills": [407, 432]}},  # P
    43: {"offer": {"professions": [4], "skills": [115, 156]}},  # P
    49: {"offer": {"professions": [5], "skills": [28, 27]}},  # P
    37: {"offer": {"professions": [6], "skills": [230, 220]}},  # P
    77: {"offer": {"professions": [3]}},  # Grazden the Protector is the monk's skill quest giver
    1185: {"offer": {"min_level": 10}},  # I: the Vanguard dailies are level 10 and up
}

# The turn-in NPC when the captures show the turn-in at more than one (A Gift for Althea's step 07 is
# on both pages; the quest's own objective returns to Captain Osric).
REWARD_NPC = {58: "Captain Osric"}

# Talk objectives that complete their quest (step 04 becomes a `credit` step). Every other step 04 is a
# conversation on the way, answered with its reply.
CREDITS = {62: {"Meerak the Scribe"},
           79: {"Ralena Stormbringer", "Vassar", "Grazden the Protector", "Kasha Blackblood"},
           55: {"Orion Elek"}}

# NPCs whose own service is a window this pack does not model; their talk is left to that service.
SKIP_TALK = re.compile(r"\[")

NAME_UNITS = 7
DETAIL_UNITS = 127


# ---- reading a capture pair ------------------------------------------------------------------------

def units(raw, start, cap):
    out = []
    for i in range(cap):
        off = start + 2 * i
        if off + 2 > len(raw):
            break
        u = raw[off] | (raw[off + 1] << 8)
        if u == 0:
            break
        out.append(u)
    return out


def u32(raw, off):
    return int.from_bytes(raw[off:off + 4], "little")


def world_index(path):
    """The (tick, map, epoch) timeline, every named sighting of an agent, and the quest snapshots.

    An agent id is reused inside one instance once a body is gone, so a name is the sighting nearest in
    time. Observations carry the instance clock; events that carry both clocks give each instance's
    offset to the process tick the packets use.
    """
    timeline, raw, snapshots = [], [], []
    offsets = {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            try:
                d = json.loads(line)
            except json.JSONDecodeError:
                continue
            m, ep, tick = d.get("map"), d.get("instance_epoch"), d.get("tick_ms")
            if m is not None and ep is not None and tick is not None:
                timeline.append((tick, m, ep))
                if d.get("instance_ms") is not None:
                    offsets.setdefault((m, ep), tick - d["instance_ms"])
            event = d.get("event")
            if event == "local_map_transition_observed":
                for side in ("from", "to"):
                    end = d.get(side) or {}
                    if None not in (end.get("map"), end.get("instance_epoch"), end.get("instance_ms"),
                                    end.get("observed_ms")):
                        offsets.setdefault((end["map"], end["instance_epoch"]),
                                           end["observed_ms"] - end["instance_ms"])
            elif event == "npc_observations" and m is not None:
                raw.append((m, ep, d.get("instance_ms"), d.get("npcs", [])))
            elif event == "quest_snapshot_changed":
                snapshots.append(d)
    sightings = {}
    for m, ep, ims, npcs in raw:
        off = offsets.get((m, ep))
        if off is None or ims is None:
            continue
        timeline.append((ims + off, m, ep))
        for npc in npcs:
            if npc.get("name"):
                sightings.setdefault((m, ep, npc["agent"]), []).append((ims + off, npc["name"]))
    for seen in sightings.values():
        seen.sort()
    timeline.sort()
    return timeline, sightings, snapshots


def where(timeline, tick):
    i = bisect.bisect_right(timeline, (tick, 1 << 30, 1 << 30)) - 1
    return timeline[i][1:] if i >= 0 else (None, None)


def name_at(sightings, m, ep, agent, tick):
    seen = sightings.get((m, ep, agent))
    if not seen:
        return None
    i = bisect.bisect_left(seen, (tick, ""))
    near = [seen[j] for j in (i - 1, i) if 0 <= j < len(seen)]
    return min(near, key=lambda s: abs(s[0] - tick))[1]


def item_row(hexs):
    """One `0x0161` in the struct layer: every field widened, a 64-unit name, a counted dword list."""
    raw = bytes.fromhex(hexs).ljust(436, b"\0")
    w = lambda o: u32(raw, o)
    count = w(176)
    return {
        "file_id": w(8), "item_type": w(12) & 0xFF, "dye_tint": w(16) & 0xFF,
        "dye_colors": w(20) & 0xFFFF, "materials": w(24) & 0xFFFF, "unk1": w(28) & 0xFF,
        "flags": w(32), "value": w(36), "model_id": w(40), "quantity": w(44),
        "name": units(raw, 48, 64),
        "modifiers": [w(180 + 4 * i) for i in range(min(count, 64))],
    }


def read_pair(root, packets, world):
    timeline, sightings, snapshots = world_index(os.path.join(root, world))
    dialogs, windows = [], []
    body, sender, buttons, start = [], None, [], None
    window = None
    source = os.path.basename(packets)

    def flush():
        nonlocal body, sender, buttons, start
        if sender is not None and body:
            m, ep = where(timeline, start)
            dialogs.append({"tick": start, "map": m, "npc": name_at(sightings, m, ep, sender, start),
                            "body": body, "buttons": buttons, "source": source})
        body, sender, buttons, start = [], None, [], None

    with open(os.path.join(root, packets), encoding="utf-8") as handle:
        for line in handle:
            d = json.loads(line)
            if d.get("direction") != "StoC":
                continue
            op = d["opcode"]
            if op not in (BODY, BUTTON, SENDER, WINDOW_OWNER, ITEM, WINDOW_IDS, MERCHANT_OPEN,
                          COLLECTOR_OPEN):
                continue
            raw = bytes.fromhex(d["hex"])
            if op == BODY:
                if sender is not None:
                    flush()
                if start is None:
                    start = d["tick_ms"]
                body += units(raw, 4, 122)
            elif op == SENDER:
                if sender is not None:
                    flush()
                sender = u32(raw, 4)
                if start is None:
                    start = d["tick_ms"]
            elif op == BUTTON and sender is not None:
                buttons.append({"icon": u32(raw, 4) & 0xFF, "label": units(raw, 8, 128),
                                "id": u32(raw, 264), "skill": u32(raw, 268)})
            elif op == WINDOW_OWNER:
                m, ep = where(timeline, d["tick_ms"])
                window = {"map": m, "npc": name_at(sightings, m, ep, u32(raw, 4), d["tick_ms"]),
                          "tick": d["tick_ms"], "rows": [], "kind": None}
                windows.append(window)
            elif window is not None and d["tick_ms"] - window["tick"] <= 300:
                if op == ITEM:
                    window["rows"].append(item_row(d["hex"]))
                elif op == MERCHANT_OPEN:
                    window["kind"] = ("merchant", u32(raw, 4))
                    window = None
                elif op == COLLECTOR_OPEN:
                    window["kind"] = ("collector", u32(raw, 4) & 0xFF)
                    window["heading"] = units(raw, 8, 122)
                    window = None
    flush()
    return dialogs, windows, snapshots


# ---- the quest log content the pack lacks ------------------------------------------------------------

def encoded(field):
    if not field:
        return None
    return field.get("encoded_utf16") or None


def character_argument(text, names):
    """The captured character name between 0x0107 and 0x0001, when the text carries one of `names`."""
    for start, unit in enumerate(text):
        if unit != 0x0107:
            continue
        try:
            end = text.index(0x0001, start + 1)
        except ValueError:
            return None
        run = text[start + 1:end]
        if run and "".join(chr(u) for u in run) in names:
            return run
    return None


def log_content(snapshots, missing, names):
    rows = {}
    for snap in snapshots:
        for q in snap.get("quests", []):
            qid = q["quest_id"]
            if qid not in missing:
                continue
            desc, objectives = encoded(q.get("description")), encoded(q.get("objectives"))
            if desc is None or objectives is None:
                continue
            stage = "completed" if (q.get("completed") or q["log_state"] & 2) else "active"
            rows.setdefault(qid, {}).setdefault(stage, q)
    out = {}
    for qid, stages in rows.items():
        base = stages.get("active") or stages["completed"]
        marker = base["marker"]
        row = {
            "id": qid, "state": base["log_state"],
            "marker": {"x_bits": marker["x_bits"], "y_bits": marker["y_bits"], "plane": marker["plane"]},
            "map_to": base["map_to"], "map_from": base["map_from"],
            "location": encoded(base.get("category")), "name": encoded(base.get("name")),
            "npc": encoded(base.get("npc")),
            "description": encoded(base["description"]), "objectives": encoded(base["objectives"]),
        }
        for key in ("location", "name", "npc"):
            if not row[key] or len(row[key]) > NAME_UNITS:
                raise SystemExit("quest %d: %s is not 1 to %d units" % (qid, key, NAME_UNITS))
        for key in ("description", "objectives"):
            if len(row[key]) > DETAIL_UNITS:
                raise SystemExit("quest %d: %s has %d units" % (qid, key, len(row[key])))
        argument = character_argument(row["description"], names)
        if argument:
            row["character_argument"] = argument
        done = stages.get("completed")
        if done is not None and done is not base:
            variant = {"marker": {"x_bits": done["marker"]["x_bits"], "y_bits": done["marker"]["y_bits"],
                                  "plane": done["marker"]["plane"]},
                       "map_to": done["map_to"],
                       "description": encoded(done["description"]),
                       "objectives": encoded(done["objectives"])}
            argument = character_argument(variant["description"], names)
            if argument:
                variant["character_argument"] = argument
            row["completed"] = variant
        out[qid] = row
    return out


def reward_text(snapshots):
    """The experience, gold and skill points each quest's own reward text names."""
    found = {}
    for snap in snapshots:
        for q in snap.get("quests", []):
            text = (q.get("description") or {}).get("decoded") or ""
            if "Reward:" not in text:
                continue
            tail = text[text.index("Reward:"):]

            def amount(word):
                m = re.search(r"([\d,]+) " + word, tail)
                return int(m.group(1).replace(",", "")) if m else 0

            found.setdefault(q["quest_id"], {"experience": amount("Experience"),
                                             "gold": amount("Gold"),
                                             "skill_points": amount("Skill Point")})
    return found


# ---- dialogs -----------------------------------------------------------------------------------------

def quest_step(button_id):
    if 0x800000 <= button_id < 0x1000000:
        return (button_id - 0x800000) >> 8, button_id & 0xFF
    return None


def merge_dialogs(dialogs):
    merged = {}
    for d in dialogs:
        key = (d["map"], d["npc"], tuple(d["body"]),
               tuple((b["icon"], tuple(b["label"]), b["id"], b["skill"]) for b in d["buttons"]))
        rec = merged.setdefault(key, {"map": d["map"], "npc": d["npc"], "body": d["body"],
                                      "buttons": d["buttons"], "seen": 0})
        rec["seen"] += 1
    return list(merged.values())


def replies(dialogs):
    """For each (map, npc, button id), the short page the capture shows next, most often first.

    A page is the answer to the page before it from the same NPC within 15 s; when the earlier page has
    several candidates the talk objective is credited first, then the reminder, then the first
    conversation choice: those are the choices whose answer is a short page.
    """
    found = collections.defaultdict(collections.Counter)
    ordered = sorted(dialogs, key=lambda d: (d["source"], d["tick"]))
    for before, after in zip(ordered, ordered[1:]):
        if (before["source"], before["npc"], before["map"]) != (after["source"], after["npc"],
                                                                 after["map"]):
            continue
        if after["tick"] - before["tick"] > 15000 or after["buttons"]:
            continue
        steps = [(b["id"], quest_step(b["id"])) for b in before["buttons"]]
        talk = [i for i, s in steps if s and s[1] == TALK]
        remind = [i for i, s in steps if s and s[1] == REMIND]
        chat = [i for i, s in steps if not s]
        for choice in (talk or remind or chat)[:1]:
            found[(before["map"], before["npc"], choice)][tuple(after["body"])] += 1
    return {key: list(counter.most_common(1)[0][0]) for key, counter in found.items()}


def button_json(b, action, when=None):
    out = {"icon": b["icon"], "label": b["label"], "id": b["id"]}
    if b["skill"] != 0xFFFFFFFF:
        out["skill"] = b["skill"]
    out["action"] = action
    if when:
        out["when"] = when
    return out


def npc_dialog(npc, map_id, records, enabled, answers):
    """One NPC's dialog: its quest menu, its offer and turn-in pages, the short replies, and its idle
    page. A talk opens the first open page, so the order is the menu, the turn-ins, the offers, then the
    idle page; a reply is only an answer."""
    reply_index, reply_index_order = {}, []
    menus, offers, rewards, idle, leaves = [], {}, {}, [], []
    for rec in sorted(records, key=lambda r: -r["seen"]):
        steps = [quest_step(b["id"]) for b in rec["buttons"]]
        quest_steps = [s for s in steps if s]
        if any(s[1] == ACCEPT for s in quest_steps):
            q = next(s[0] for s in quest_steps if s[1] == ACCEPT)
            if q in enabled and enabled[q]["offer_npc"] == npc:
                offers.setdefault(q, rec)
        elif any(s[1] == TURN_IN for s in quest_steps):
            q = next(s[0] for s in quest_steps if s[1] == TURN_IN)
            if q in enabled and enabled[q]["reward_npc"] == npc:
                rewards.setdefault(q, rec)
        elif any(s[1] in (ASK, TALK, REMIND, ASK_REWARD) for s in quest_steps):
            menus.append(rec)
        elif rec["buttons"]:
            idle.append(rec)
        else:
            leaves.append(rec)

    # The menu: the most-seen body, every choice any capture showed, each behind its quest's state.
    menu_buttons, menu_body = [], None
    if menus:
        menu_body = collections.Counter(tuple(m["body"]) for m in menus).most_common(1)[0][0]
        seen_ids = set()
        for rec in menus:
            for b in rec["buttons"]:
                if b["id"] in seen_ids:
                    continue
                seen_ids.add(b["id"])
                menu_buttons.append(b)

    layout = []  # (kind, payload) in page order
    if menus:
        layout.append(("menu", None))
    for q in sorted(rewards):
        layout.append(("reward", q))
    for q in sorted(offers):
        layout.append(("offer", q))
    if idle:
        layout.append(("idle", idle[0]))
    elif not menus and not offers and not rewards and leaves:
        layout.append(("talk", leaves[0]))
    index = {entry: i for i, entry in enumerate((k, p if k in ("reward", "offer") else None)
                                               for k, p in layout)}

    def page_of(kind, q=None):
        return index.get((kind, q))

    def answer(button_id):
        body = answers.get((map_id, npc, button_id))
        if body is None:
            return None
        key = tuple(body)
        if key not in reply_index:
            reply_index[key] = len(layout) + len(reply_index_order)
            reply_index_order.append(key)
        return reply_index[key]

    def menu_action(b):
        step = quest_step(b["id"])
        if not step:
            target = answer(b["id"])
            return ({"kind": "page", "page": target} if target is not None else {"kind": "close"}), None
        q, s = step
        if q not in enabled:
            return None, None
        if s == ASK:
            target = page_of("offer", q)
            if target is None:
                return None, None
            return {"kind": "page", "page": target}, {"kind": "quest_offer_open", "quest": q}
        if s == ASK_REWARD:
            target = page_of("reward", q)
            if target is None:
                return None, None
            return {"kind": "page", "page": target}, {"kind": "quest_reward_open", "quest": q}
        if s in (TALK, REMIND):
            target = answer(b["id"])
            when = {"kind": "quest_active", "quest": q}
            if s == TALK and npc in CREDITS.get(q, ()):
                action = {"kind": "quest", "quest": q, "step": "credit"}
                if target is not None:
                    action["page"] = target
                return action, when
            return ({"kind": "page", "page": target} if target is not None
                    else {"kind": "close"}), when
        return None, None

    built = []
    for kind, payload in layout:
        if kind == "menu":
            buttons = []
            for b in menu_buttons:
                action, when = menu_action(b)
                if action is not None:
                    buttons.append(button_json(b, action, when))
            built.append({"body": list(menu_body), "buttons": buttons})
        elif kind == "offer":
            rec = offers[payload]
            buttons = []
            for b in rec["buttons"]:
                step = quest_step(b["id"])
                if step and step[1] == ACCEPT:
                    buttons.append(button_json(b, {"kind": "quest", "quest": payload, "step": "offer"}))
                else:
                    buttons.append(button_json(b, {"kind": "close"}))
            built.append({"when": {"kind": "quest_offer_open", "quest": payload},
                          "body": rec["body"], "buttons": buttons})
        elif kind == "reward":
            rec = rewards[payload]
            buttons = [button_json(b, {"kind": "quest", "quest": payload, "step": "reward"})
                       for b in rec["buttons"]]
            built.append({"when": {"kind": "quest_reward_open", "quest": payload},
                          "body": rec["body"], "buttons": buttons})
        elif kind == "idle":
            buttons = []
            for b in payload["buttons"]:
                target = answer(b["id"])
                buttons.append(button_json(b, {"kind": "page", "page": target} if target is not None
                                           else {"kind": "close"}))
            built.append({"body": payload["body"], "buttons": buttons})
        elif kind == "talk":
            built.append({"body": payload["body"], "buttons": []})
    for key in reply_index_order:
        built.append({"when": {"kind": "answer"}, "body": list(key), "buttons": []})
    # A menu that lost every choice (its quests are not in this pack) is dropped, with the indices
    # after it moved down.
    if built and menus and not built[0]["buttons"]:
        built = built[1:]
        for page in built:
            for b in page["buttons"]:
                if b["action"].get("page") is not None:
                    b["action"]["page"] -= 1
    return built


# ---- the pack ----------------------------------------------------------------------------------------

def inline(value):
    return "[" + ", ".join(json.dumps(item) for item in value) + "]"


def format_block(value, indent=0):
    """quests.json's and vendors.json's own layout: every object and string list on its lines, every
    number list inline."""
    pad = "  " * indent
    if isinstance(value, dict):
        items = ["%s  %s: %s" % (pad, json.dumps(key), format_block(item, indent + 1))
                 for key, item in value.items()]
        return "{\n" + ",\n".join(items) + "\n" + pad + "}"
    if isinstance(value, list):
        if all(isinstance(item, (int, float)) and not isinstance(item, bool) for item in value):
            return inline(value)
        items = ["%s  %s" % (pad, format_block(item, indent + 1)) for item in value]
        return "[\n" + ",\n".join(items) + "\n" + pad + "]"
    return json.dumps(value)


def format_value(value, indent=0):
    """dialogs.json's layout: a short object (a choice's action or condition) on one line."""
    pad = "  " * indent
    if isinstance(value, dict):
        if all(not isinstance(item, (dict, list)) or (isinstance(item, list) and all(
                not isinstance(x, (dict, list)) for x in item)) for item in value.values()) and \
                len(json.dumps(value)) <= 100:
            return json.dumps(value)
        items = ["%s  %s: %s" % (pad, json.dumps(key), format_value(item, indent + 1))
                 for key, item in value.items()]
        return "{\n" + ",\n".join(items) + "\n" + pad + "}"
    if isinstance(value, list):
        if all(not isinstance(item, (dict, list)) for item in value):
            return inline(value)
        items = ["%s  %s" % (pad, format_value(item, indent + 1)) for item in value]
        return "[\n" + ",\n".join(items) + "\n" + pad + "]"
    return json.dumps(value)


def ascii_check(text, path):
    bad = [c for c in text if ord(c) > 127]
    if bad:
        raise SystemExit("%s would not be ASCII: %r" % (path, bad[:5]))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--preforged", required=True, help="an extracted preforged tree")
    parser.add_argument("--write", action="store_true", help="write the pack files")
    args = parser.parse_args()
    root = os.path.join(args.preforged, "data", "captures")

    dialogs, windows, snapshots = [], [], []
    for packets, world in CAPTURES:
        d, w, s = read_pair(root, packets, world)
        dialogs += d
        windows += w
        snapshots += s
    unnamed = [d for d in dialogs if not d["npc"]]
    if unnamed:
        raise SystemExit("%d dialog(s) have no speaker name" % len(unnamed))

    with open(os.path.join(PACK, "quests.json"), encoding="utf-8") as handle:
        quests_doc = json.load(handle)
    with open(os.path.join(PACK, "dialogs.json"), encoding="utf-8") as handle:
        dialogs_doc = json.load(handle)
    with open(os.path.join(PACK, "vendors.json"), encoding="utf-8") as handle:
        vendors_doc = json.load(handle)

    # Quests: the log content the pack lacks, then who offers and who takes each one.
    have = {q["id"]: q for q in quests_doc["quests"]}
    seen_ids = {q["quest_id"] for s in snapshots for q in s.get("quests", [])}
    names = {"".join(chr(u) for u in q["character_argument"]) for q in have.values()
             if q.get("character_argument")}
    added = log_content(snapshots, seen_ids - set(have), names)
    have.update(added)
    rewards = reward_text(snapshots)

    givers = collections.defaultdict(lambda: collections.defaultdict(set))
    for d in dialogs:
        for b in d["buttons"]:
            step = quest_step(b["id"])
            if step:
                givers[step[0]][step[1]].add((d["map"], d["npc"]))
    enabled = {}
    for qid in sorted(have):
        offer = sorted(givers[qid].get(ACCEPT, ()))
        reward = sorted(givers[qid].get(TURN_IN, ()))
        if not offer or not reward or qid not in rewards:
            continue
        if qid in REWARD_NPC:
            reward = [r for r in reward if r[1] == REWARD_NPC[qid]]
        if len(offer) != 1 or len(reward) != 1:
            raise SystemExit("quest %d: offered by %s, turned in at %s" % (qid, offer, reward))
        enabled[qid] = {"offer_npc": offer[0][1], "offer_map": offer[0][0],
                        "reward_npc": reward[0][1], "reward_map": reward[0][0]}
    for qid, rule in RULES.items():
        if qid not in enabled:
            raise SystemExit("a rule names quest %d, which has no captured offer and turn-in" % qid)

    for qid, e in enabled.items():
        rule = RULES.get(qid, {})
        offer = {"npc": e["offer_npc"], "map": e["offer_map"]}
        offer.update(rule.get("offer", {}))
        reward = {"npc": e["reward_npc"], "map": e["reward_map"]}
        reward.update(rewards[qid])
        reward.update(rule.get("reward", {}))
        have[qid]["offer"] = offer
        have[qid]["reward"] = reward
    quests_doc["quests"] = [have[qid] for qid in sorted(have)]
    fields = quests_doc.setdefault("fields", {})
    fields["offer"] = ("Who offers the quest, on which map, and to whom: requires_none_completed, "
                       "requires_rewarded, requires_any_rewarded, professions (primary_only), "
                       "min_level; and the skills accepting teaches. Written by "
                       "tools/npc-content-from-captures.py.")
    fields["reward"] = ("Who takes the turn-in, on which map, and what it pays: experience, gold and "
                        "skill_points (the quest's own reward text), the skills it teaches, and "
                        "requires_completed when the turn-in waits for a modelled objective.")

    # Dialogs: generated for every NPC the captures show talking, except the hand-written ones.
    answers = replies(dialogs)
    records = collections.defaultdict(list)
    for rec in merge_dialogs(dialogs):
        records[(rec["map"], rec["npc"])].append(rec)
    generated = []
    for (map_id, npc) in sorted(records):
        if SKIP_TALK.search(npc):
            continue
        pages = npc_dialog(npc, map_id, records[(map_id, npc)], enabled, answers)
        if not pages:
            continue
        quests_here = sorted({q for q, e in enabled.items()
                              if (e["offer_npc"], e["offer_map"]) == (npc, map_id)
                              or (e["reward_npc"], e["reward_map"]) == (npc, map_id)})
        note = ("Generated by tools/npc-content-from-captures.py from preforged's pre-Searing "
                "captures: every body and label is the live server's encoded units.")
        if quests_here:
            note += " Quests: %s." % ", ".join(str(q) for q in quests_here)
        entry = {"map": map_id, "npc": npc, "generated": True, "note": note}
        if npc == "Sir Tydus" and map_id == 148:
            entry["talk_range"] = 200.0
        entry["pages"] = pages
        generated.append(entry)
    # A hand-written entry is kept unless the captures now give that NPC its pages (the Town Crier's
    # and Sir Tydus's hand-written quest 80 pages are the generated ones' first pages).
    replaced = {(d["map"], d["npc"].split(" [")[0]) for d in generated}
    hand = [d for d in dialogs_doc["dialogs"] if not d.get("generated")
            and (d["map"], d["npc"].split(" [")[0]) not in replaced]
    dialogs_doc["dialogs"] = hand + generated
    dialogs_doc["fields"]["generated"] = ("True for an entry tools/npc-content-from-captures.py "
                                          "writes; the tool replaces those and keeps the rest.")
    dialogs_doc["fields"]["when"] = ("A page's or a choice's condition: quest_offer_open, "
                                     "quest_reward_open, quest_active (accepted, objective not done), "
                                     "or answer (a page only a choice opens). Absent is always.")
    dialogs_doc["fields"]["action"] = (
        "close answers nothing; window opens 0x0083 with that kind; merchant opens the NPC's window "
        "through vendors.json; page shows another page; quest with step offer, reward or credit "
        "accepts the quest, takes its turn-in or completes its talk objective, then shows `page` or "
        "the first page that offers or rewards a quest; unanswered is a choice the capture never "
        "exercised and is only logged.")

    # Vendors: Hamish's captured stock, and Gift-Giver Pinn's captured exchange.
    items = {row["id"]: row for row in vendors_doc["items"]}
    by_shape = {}
    for key, row in items.items():
        shape = tuple(row[k] for k in ("file_id", "item_type", "model_id", "value", "flags"))
        by_shape.setdefault(shape, key)
    merchant_names = {(m["map"], m["npc"]) for m in vendors_doc["merchants"]}
    collector_names = {(c["map"], c["npc"]) for c in vendors_doc["collectors"]}
    for w in windows:
        if not w["npc"] or not w["kind"] or not w["rows"]:
            continue
        # A window whose rows did not survive the capture (a row of file id 0, which the client asserts
        # on) is not evidence of anything.
        if any(row["file_id"] == 0 for row in w["rows"]):
            continue
        name = w["npc"].split(" [")[0].strip()
        new_merchant = w["kind"][0] == "merchant" and (w["map"], name) not in merchant_names
        new_collector = w["kind"] == ("collector", 2) and (w["map"], name) not in collector_names
        if not new_merchant and not new_collector:
            continue
        keys = []
        for row in w["rows"]:
            shape = tuple(row[k] for k in ("file_id", "item_type", "model_id", "value", "flags"))
            key = by_shape.get(shape)
            if key is None:
                key = "%s_%d" % (re.sub(r"[^a-z]+", "_", name.lower()).strip("_"), row["model_id"])
                items[key] = {"id": key, **{k: row[k] for k in (
                    "file_id", "item_type", "dye_tint", "dye_colors", "materials", "unk1", "flags",
                    "value", "model_id", "name", "modifiers")}}
                by_shape[shape] = key
            keys.append(key)
        if new_merchant:
            vendors_doc["merchants"].append({"map": w["map"], "npc": name, "stock": keys})
            merchant_names.add((w["map"], name))
        else:
            vendors_doc["collectors"].append({
                "map": w["map"], "npc": name, "trophy": keys[0], "cost": w["rows"][0]["quantity"],
                "heading": w["heading"],
                "offers": [{"reward": key, "profession": 0} for key in keys[1:]]})
            collector_names.add((w["map"], name))
    vendors_doc["items"] = sorted(items.values(), key=lambda row: row["id"])

    texts = {
        "quests.json": format_block(quests_doc) + "\n",
        "dialogs.json": format_value(dialogs_doc) + "\n",
        "vendors.json": format_block(vendors_doc) + "\n",
    }
    for path, text in texts.items():
        ascii_check(text, path)
    print("quests: %d in the pack, %d added from the captures (%s), %d offered and rewarded" %
          (len(quests_doc["quests"]), len(added), ", ".join(str(q) for q in sorted(added)),
           len(enabled)))
    print("dialogs: %d hand-written, %d generated (%d pages)" %
          (len(hand), len(generated), sum(len(d["pages"]) for d in generated)))
    print("vendors: %d merchants, %d collectors, %d item rows" %
          (len(vendors_doc["merchants"]), len(vendors_doc["collectors"]), len(vendors_doc["items"])))
    if args.write:
        for path, text in texts.items():
            with open(os.path.join(PACK, path), "w", encoding="utf-8", newline="\n") as handle:
                handle.write(text)
        print("wrote", ", ".join(texts))


if __name__ == "__main__":
    main()
