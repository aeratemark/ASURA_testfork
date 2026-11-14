#!/usr/bin/env python3
"""Write the pack's trade windows (config/World/Vanilla/vendors.json) from preforged's catalogs.

    python tools/vendors-from-preforged.py --preforged D:/preforged-main --write

Sources, both read-only:

* `src/runtime/gw_merchant_panel.h`: `sanura_stock()` (seven rows, every field of each `0x0161`) and
  `shops[]` (the six pre-Searing merchants that sell that stock, by map and name);
* `src/runtime/gw_presearing_collectors.h`: the 24 collector offers (trophy row, cost, reward row,
  profession).

Preforged numbers its rows for the 2022 client and strips bit 31 of a file id. Every item a live server
sends this build carries bit 31 (`recordings/client/feature/maps/pre-searing-load-3.jsonl`, every
`0x0161`; preforged's own `packets-43292-168891734.jsonl`, the merchant stock at record 487608), so the
tool sets it. Preforged's field names map onto `game::NamedItem` as: unknown1 -> dye_tint,
dye -> dye_colors, unknown2 -> unk1, interaction -> flags, price -> value, model -> model_id.

The collector's line over its offers is the captured formula preforged keeps per collector
(`gw_collector_panel.h`, `captured_heading`); the live server sent it for Brownlow at record 537235.
"""

import argparse
import json
import os
import re

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PACK = os.path.join(ROOT, "config", "World", "Vanilla")
FILE_BIT = 0x80000000

# The stock rows, keyed by model, with the names Guild Wars gives them (preforged
# presearing-merchant-collector-support.txt and its wiki snapshot).
STOCK_KEYS = {
    957: "focus_pre_searing",
    2822: "starter_shield",
    918: "vial_of_dye_gray",
    2993: "kit_2993",
    2989: "kit_2989",
    2110: "merchant_stock_2110",
    1101: "flute",
}
# The pack's existing items.json ids for the collector rows, by (model, type), so an owned row can
# name its definition. A row with no entry here carries no definition id.
TROPHY_DEFINITIONS = {
    429: "skale_fin",
    433: "baked_husk",
    432: "grawl_necklace",
    428: "unnatural_seed",
    424: "icy_lodestone",
    427: "worn_belt",
    422: "spider_leg",
}


def parse_definition(block, prefix):
    row = {}
    for key, value in re.findall(re.escape(prefix) + r"\.(\w+)=([^;]+);", block):
        value = value.strip()
        if value.startswith("{"):
            row[key] = [int(part.strip().rstrip("u")) for part in value.strip("{}").split(",") if part.strip()]
        else:
            row[key] = int(value.rstrip("u"))
    return row


def wire(row, key, definition=None):
    item = {
        "id": key,
        "file_id": row["file"] | FILE_BIT,
        "item_type": row["type"],
        "dye_tint": row.get("unknown1", 0),
        "dye_colors": row.get("dye", 0),
        "materials": row.get("materials", 0),
        "unk1": row.get("unknown2", 0),
        "flags": row["interaction"],
        "value": row["price"],
        "model_id": row["model"],
        "name": row.get("encoded_name", []),
        "modifiers": row.get("modifiers", []),
    }
    if definition:
        item["definition"] = definition
    return item


def read_stock(path):
    text = open(path, encoding="utf-8").read()
    rows = []
    for line in re.findall(r"\{items::Definition d;(.*?)out\.push_back", text, re.S):
        row = {}
        for key, value in re.findall(r"d\.(\w+)=([^;]+);", line):
            value = value.strip()
            if value.startswith("{"):
                row[key] = [int(part.strip().rstrip("u")) for part in value.strip("{}").split(",") if part.strip()]
            else:
                row[key] = int(value.rstrip("u"))
        rows.append(row)
    shops = re.findall(r'\{(\d+),"(\w+)",StockEvidence::\w+\}', text)
    return rows, [(int(map_id), name) for map_id, name in shops]


def read_collectors(path):
    text = open(path, encoding="utf-8").read()
    offers = []
    for block in re.findall(r"\{Offer o;(.*?)out\.push_back\(std::move\(o\)\);\}", text, re.S):
        offer = {}
        for key, value in re.findall(r"o\.(key|map|profession|collector|name|trophy_name|cost)=([^;]+);", block):
            offer[key] = value.strip('"') if value.startswith('"') else int(value)
        offer["trophy"] = parse_definition(block, "o.trophy")
        offer["reward"] = parse_definition(block, "o.reward.definition")
        offer["reward_profession"] = int(re.search(r"o\.reward\.profession=(\d+)", block).group(1))
        offers.append(offer)
    return offers


def slug(text):
    return re.sub(r"[^a-z0-9]+", "_", text.lower()).strip("_")


def heading(trophy_unit, cost):
    return [11059, 55010, 55495, 2666, 266, trophy_unit, 1, 257, 256 + cost]


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--preforged", required=True)
    parser.add_argument("--pack", default=PACK)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    runtime = os.path.join(args.preforged, "src", "runtime")

    stock_rows, shops = read_stock(os.path.join(runtime, "gw_merchant_panel.h"))
    offers = read_collectors(os.path.join(runtime, "gw_presearing_collectors.h"))

    items = {}
    stock = []
    for row in stock_rows:
        key = STOCK_KEYS[row["model"]]
        items[key] = wire(row, key)
        stock.append(key)

    collectors = {}
    for offer in offers:
        trophy_key = TROPHY_DEFINITIONS.get(offer["trophy"]["model"], "trophy_%d" % offer["trophy"]["model"])
        items.setdefault(trophy_key, wire(offer["trophy"], trophy_key, TROPHY_DEFINITIONS.get(offer["trophy"]["model"])))
        reward_key = "%s_%d" % (slug(offer["name"]), offer["reward"]["model"])
        items.setdefault(reward_key, wire(offer["reward"], reward_key))
        group = (offer["map"], offer["collector"])
        entry = collectors.setdefault(group, {
            "map": offer["map"],
            "npc": offer["collector"],
            "trophy": trophy_key,
            "cost": offer["cost"],
            "heading": heading(offer["trophy"]["encoded_name"][0], offer["cost"]),
            "offers": [],
        })
        if entry["trophy"] != trophy_key or entry["cost"] != offer["cost"]:
            raise SystemExit("collector %s mixes trophies or costs" % offer["collector"])
        entry["offers"].append({"reward": reward_key, "profession": offer["profession"]})

    doc = {
        "note": (
            "The NPCs that open a trade window or the Xunlai storage window when a player talks to them "
            "(crates/asura-world/src/vendor/). An NPC is found by its map and its name: the placement's "
            "literal name in maps/.../npcs.json, compared without a bracketed title such as ' [Merchant]'. "
            "Every item row is the full CREATE_NAMED_ITEM a window declares; a purchase or an exchange "
            "gives the player a copy under a new id. Written by tools/vendors-from-preforged.py."
        ),
        "fields": {
            "items": "Item rows by key: file_id, item_type, dye_tint, dye_colors, materials, unk1, flags, value, model_id, name (coded units), modifiers (dwords), and the items.json id it is, when it is one.",
            "merchants": "One per merchant: its map, its name, and the keys of the items it sells, in the order its window lists them. A merchant also buys what a player carries (see sell).",
            "collectors": "One per collector: its map, its name, the trophy it takes, how many per offer, the coded line it says over its offers, and its offers (a reward key and the primary profession it is for, 0 for any).",
            "storage": "The NPCs that open Xunlai storage: a map (null for every map) and a name.",
            "sell": "What a merchant buys back: every item in a carrying bag with a value above 0, except the models listed in unsellable_models.",
        },
        "rules": {
            "service_range": {
                "value": 300.0,
                "doc": "How far, in map units, a player may stand from the NPC whose window is open for a transaction or a storage move to be accepted. The client closes the window when the player walks away; the server checks it again. Chosen, not measured.",
            },
            "buy_scale": {
                "value": 1.0,
                "doc": "The price scale a merchant's buy pane carries (0x00CA). The client asks value x scale x 2 for an item, and so does the server. 1.0 in every live merchant window.",
                "immutable": True,
            },
            "carried_gold_max": {
                "value": 100000,
                "doc": "The most gold a character carries. A sale or a withdrawal that would pass it is refused.",
                "immutable": True,
            },
            "stored_gold_max": {
                "value": 1000000,
                "doc": "The most gold storage holds. A deposit that would pass it is refused.",
                "immutable": True,
            },
            "stack_max": {
                "value": 250,
                "doc": "The largest stack of a stackable item (flags bit 0x80000) a purchase may make.",
                "immutable": True,
            },
        },
        "unsellable_models": [2822, 1101],
        "items": [items[key] for key in sorted(items)],
        "merchants": [{"map": map_id, "npc": name, "stock": stock} for map_id, name in shops],
        "collectors": [collectors[key] for key in sorted(collectors)],
        "storage": [
            {"map": None, "npc": "Xunlai Agent"},
            {"map": None, "npc": "Xunlai Chest"},
        ],
    }
    text = json.dumps(doc, indent=2, ensure_ascii=True) + "\n"
    text = re.sub(
        r"\[\s*(-?[0-9][0-9]*(?:,\s*-?[0-9][0-9]*)*)\s*\]",
        lambda match: "[" + ", ".join(part.strip() for part in match.group(1).split(",")) + "]",
        text,
    )
    out = os.path.join(args.pack, "vendors.json")
    if args.write:
        with open(out, "w", encoding="ascii", newline="\n") as handle:
            handle.write(text)
        print("wrote %s: %d items, %d merchants, %d collectors" % (out, len(items), len(shops), len(collectors)))
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
