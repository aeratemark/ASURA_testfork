"""Extract item definitions from the Rurik and cmonmeow research checkouts.

    python tools/extract-item-research.py --out recordings/client/feature/items/items-rurik-cmonmeow.json

This is a research tool, not a server dependency. It reads the two external
trees only when their paths are given (or present at their usual places),
fails closed when a path is absent, and writes one normalized JSON file. The
output is a candidate table for review; it does not edit the world pack and
nothing under crates/ imports it.

What is normalized:

* Rurik `content/items.toml`, one row per `[item.<key>]`, with its `enc_name`,
  modifiers and provenance labels.
* cmonmeow captured `items::Definition` literals: fields named `file`, `model`,
  `type`, `encoded_name`, `modifiers`, and the surrounding base fields. The
  cmonmeow spelling of the wire fields differs: `unknown1` is dye tint, `dye`
  is dye colours, `unknown2` is the unverified byte, `interaction` is flags and
  `price` is value. The output uses the ASURA names.
* Collector offers in `gw_presearing_collectors.h` supply readable reward and
  trophy names for definitions the generic scan finds.

The output keeps each source row separate. It does not merge rows across
sources and it does not decide which of two candidates is retail truth.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_RURIK = ROOT / "Other_Repos_Research" / "Rurik-main" / "content" / "items.toml"
DEFAULT_CMONMEOW = ROOT / "Other_Repos_Research" / "cmonmeow-src"
DEFAULT_OUT = ROOT / "recordings" / "client" / "feature" / "items" / "items-rurik-cmonmeow.json"


def integer(text: str) -> int:
    text = text.strip().rstrip("uU")
    return int(text, 16) if text.lower().startswith("0x") else int(text, 10)


def array_of_ints(body: str) -> list[int]:
    return [integer(match) for match in re.findall(r"0x[0-9a-fA-F]+|\d+", body)]


def rurik_rows(path: Path) -> list[dict]:
    with path.open("rb") as handle:
        document = tomllib.load(handle)
    items = document.get("item", {})
    rows = []
    for key, item in items.items():
        provenance = item.get("provenance", {})
        rows.append(
            {
                "source": "rurik",
                "source_key": f"item.{key}",
                "source_file": "Rurik-main/content/items.toml",
                "name": key,
                "file_id": int(item.get("file_id", 0)),
                "model_id": int(item.get("model_id", 0)),
                "item_type": int(item.get("item_type", 0)),
                "encoded_name": [int(unit) for unit in item.get("enc_name", [])],
                "modifiers": [int(word) for word in item.get("modifiers", [])],
                "flags": item.get("flags"),
                "value": item.get("value"),
                "materials": item.get("materials"),
                "unk1": item.get("unk1"),
                "dye_tint": item.get("dye_tint"),
                "dye_colors": item.get("dye_colors"),
                "quantity": item.get("quantity"),
                "profession": item.get("profession", 0),
                "evidence": provenance.get("source", ""),
                "verified": provenance.get("verified", ""),
            }
        )
    return rows


FIELD_PATTERNS = {
    "file_id": re.compile(r"\.file\s*=\s*(\d+)u?"),
    "model_id": re.compile(r"\.model\s*=\s*(\d+)u?"),
    "item_type": re.compile(r"\.type\s*=\s*(\d+)u?"),
    "dye_tint": re.compile(r"\.unknown1\s*=\s*(\d+)u?"),
    "dye_colors": re.compile(r"\.dye\s*=\s*(\d+)u?"),
    "materials": re.compile(r"\.materials\s*=\s*(\d+)u?"),
    "unk1": re.compile(r"\.unknown2\s*=\s*(\d+)u?"),
    "flags": re.compile(r"\.interaction\s*=\s*(\d+)u?"),
    "value": re.compile(r"\.price\s*=\s*(\d+)u?"),
    "quantity": re.compile(r"\.quantity\s*=\s*(\d+)u?"),
    "profession": re.compile(r"\.profession\s*=\s*(\d+)u?"),
}
NAME_PATTERN = re.compile(r"encoded_name\s*=\s*\{([0-9u,\s]*)\}")
MODS_LITERAL = re.compile(r"\.modifiers\s*=\s*\{([0-9uxa-fA-F,\s]*)\}")
MODS_WORDS = re.compile(r"words\s*=\s*\{([0-9uxa-fA-F,\s]*)\}")


def last_field(window: str, name: str) -> int | None:
    matches = list(FIELD_PATTERNS[name].finditer(window))
    return integer(matches[-1].group(1)) if matches else None


def cmonmeow_definitions(root: Path) -> list[dict]:
    rows: list[dict] = []
    for path in sorted(root.rglob("*")):
        if not path.is_file() or path.suffix not in {".h", ".inc"}:
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for match in NAME_PATTERN.finditer(text):
            before = text[max(0, match.start() - 1200) : match.start()]
            after = text[match.end() : match.end() + 800]
            file_id = last_field(before, "file_id")
            model_id = last_field(before, "model_id")
            item_type = last_field(before, "item_type")
            if file_id is None or model_id is None or item_type is None:
                continue

            modifiers: list[int] = []
            literal = MODS_LITERAL.search(after)
            if literal:
                modifiers = array_of_ints(literal.group(1))
            elif "capture_to_wire" in after:
                window = text[max(0, match.start() - 1200) : match.end() + 800]
                words = list(MODS_WORDS.finditer(window))
                if words:
                    captured = array_of_ints(words[-1].group(1))
                    if captured and captured[-1] == 0xC0000000:
                        modifiers = captured[:-1]

            rows.append(
                {
                    "source": "cmonmeow",
                    "source_key": "",
                    "source_file": str(path.relative_to(root)),
                    "name": "",
                    "file_id": file_id,
                    "model_id": model_id,
                    "item_type": item_type,
                    "encoded_name": array_of_ints(match.group(1)),
                    "modifiers": modifiers,
                    "flags": last_field(before, "flags"),
                    "value": last_field(before, "value"),
                    "materials": last_field(before, "materials"),
                    "unk1": last_field(before, "unk1"),
                    "dye_tint": last_field(before, "dye_tint"),
                    "dye_colors": last_field(before, "dye_colors"),
                    "quantity": last_field(before, "quantity"),
                    "profession": last_field(before, "profession") or 0,
                    "evidence": "captured definition literal",
                    "verified": "",
                }
            )
    return rows


OFFER_BLOCK = re.compile(r"\{Offer o;(.*?)out\.push_back", re.S)


def collector_names(path: Path) -> dict[tuple[int, int, int], str]:
    if not path.is_file():
        return {}
    text = path.read_text(encoding="utf-8", errors="ignore")
    names: dict[tuple[int, int, int], str] = {}
    for match in OFFER_BLOCK.finditer(text):
        block = match.group(1)
        reward_name = re.search(r'o\.name="([^"]+)"', block)
        trophy_name = re.search(r'o\.trophy_name="([^"]+)"', block)
        for prefix, label in (("o.reward.definition", reward_name), ("o.trophy", trophy_name)):
            if not label:
                continue
            fields = {}
            for key, pattern in FIELD_PATTERNS.items():
                found = list(re.finditer(pattern, block))
                # Only assignments with this prefix count.
                scoped = [
                    item
                    for item in found
                    if block[max(0, item.start() - len(prefix)) : item.start()].endswith(prefix)
                ]
                if scoped:
                    fields[key] = integer(scoped[-1].group(1))
            if {"file_id", "model_id", "item_type"} <= fields.keys():
                names[(fields["file_id"] & 0x7FFFFFFF, fields["model_id"], fields["item_type"])] = label.group(1)
    return names


def apply_names(rows: list[dict], names: dict[tuple[int, int, int], str]) -> None:
    for row in rows:
        key = (row["file_id"] & 0x7FFFFFFF, row["model_id"], row["item_type"])
        readable = names.get(key)
        if readable:
            row["name"] = readable
        elif not row["name"] and row["source"] == "cmonmeow":
            row["name"] = f"{row['source_file'].rsplit('/', 1)[-1]}:{row['model_id']}"


def dedupe(rows: list[dict]) -> list[dict]:
    """Collapse exact repeats within one source, such as one collector trophy
    listed by several offers. Two sources stay separate because their evidence
    is separate; two different names or modifier lists stay separate because
    that is a candidate the reviewer has to choose between."""
    seen: dict[tuple, dict] = {}
    for row in rows:
        key = (
            row["source"],
            row["file_id"] & 0x7FFFFFFF,
            row["model_id"],
            row["item_type"],
            tuple(row["encoded_name"]),
            tuple(row["modifiers"]),
            row["flags"],
            row["value"],
            row["materials"],
            row["unk1"],
            row["dye_tint"],
            row["dye_colors"],
        )
        seen.setdefault(key, row)
    return list(seen.values())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rurik", type=Path, default=DEFAULT_RURIK)
    parser.add_argument("--cmonmeow", type=Path, default=DEFAULT_CMONMEOW)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    args = parser.parse_args()

    if not args.rurik.is_file():
        print(f"missing Rurik items file: {args.rurik}", file=sys.stderr)
        return 2
    if not args.cmonmeow.is_dir():
        print(f"missing cmonmeow source tree: {args.cmonmeow}", file=sys.stderr)
        return 2

    rows = rurik_rows(args.rurik)
    names = collector_names(args.cmonmeow / "runtime" / "gw_presearing_collectors.h")
    cmonmeow = cmonmeow_definitions(args.cmonmeow)
    apply_names(cmonmeow, names)
    rows.extend(cmonmeow)
    rows = dedupe(rows)

    # Gold is a currency, not an owned item definition. It travels as the
    # character's gold scalar and as a ground/agent amount, not as a normal
    # bag-row definition, so keep it out of the item candidates. Platinum is
    # only 1000 gold in the client's display; it has no item of its own.
    currencies = [row for row in rows if row["item_type"] == 20]
    rows = [row for row in rows if row["item_type"] != 20]

    for row in rows:
        if not row["file_id"] or not row["item_type"] or not row["name"]:
            raise SystemExit(f"incomplete item row: {row}")

    # Stable report order: source, then wire identity, then source key/file.
    rows.sort(key=lambda row: (row["source"], row["file_id"] & 0x7FFFFFFF, row["model_id"], row["item_type"], row["source_key"], row["source_file"]))
    currencies.sort(key=lambda row: (row["source"], row["file_id"] & 0x7FFFFFFF, row["model_id"]))

    document = {
        "note": (
            "Normalized research rows from Rurik and cmonmeow. Candidate table only: rows are not "
            "merged across sources and are not the world pack. Field names are ASURA's wire names; "
            "cmonmeow's unknown1/dye/unknown2 become dye_tint/dye_colors/unk1. Gold is kept out of "
            "rows: it is a currency scalar, and platinum is 1000 gold in the client's display."
        ),
        "rurik": str(args.rurik),
        "cmonmeow": str(args.cmonmeow),
        "counts": {
            "rurik": sum(1 for row in rows if row["source"] == "rurik"),
            "cmonmeow": sum(1 for row in rows if row["source"] == "cmonmeow"),
            "items": len(rows),
            "currencies": len(currencies),
        },
        "rows": rows,
        "currencies": currencies,
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(document, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")
    print(
        f"wrote {args.out}: {document['counts']['rurik']} Rurik, "
        f"{document['counts']['cmonmeow']} cmonmeow, {len(rows)} item rows, "
        f"{len(currencies)} currency row(s)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
