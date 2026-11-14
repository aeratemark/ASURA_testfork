"""Write the decoded item names into the pack and regenerate the item list.

    python tools/apply-item-names.py

`tools/decode-item-names.py` writes the client's own answer for each probe's
`name_enc` to `recordings/client/feature/items/items-live-names.json`. This tool
matches those answers to the pack by the wire identity
`(file_id, item_type, model_id)` and writes the name into the definition's
`description`.

The description is a human label and the lookup `/item` uses; it is not the
wire name. The definition's `name` units, `base`, `upgrades` and every wire
field stay untouched. A record the decoder did not answer keeps the description
it had.

When two definitions would carry the same description, every one after the
first gains `[0x<file>/<model>/<type>]`, the same suffix the importer uses, so
`/item` can still tell them apart by name.

With `--no-list` the generated `config/items-list.md` is left alone.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_NAMES = ROOT / "recordings" / "client" / "feature" / "items" / "items-live-names.json"
DEFAULT_PACK = ROOT / "config" / "World" / "Vanilla" / "items.json"
DEFAULT_LIST = ROOT / "config" / "items-list.md"


def identity(definition: dict) -> tuple[int, int, int]:
    """The pack's identity: file id without its high bit, item type, model id."""
    return (
        int(definition.get("file_id", 0)) & 0x7FFFFFFF,
        int(definition.get("item_type", 0)),
        int(definition.get("model_id", 0)),
    )


def singular(name: str) -> str:
    """The one-item form of a name the client pluralised for a stack."""
    if name.endswith("ies") and len(name) > 3:
        return name[:-3] + "y"
    if re.search(r"(s|x|z|ch|sh)es$", name):
        return name[:-2]
    if name.endswith("s") and not name.endswith("ss"):
        return name[:-1]
    return name


def plain_name(name: str, definition: dict) -> str:
    """Strip the client's colour tags and the quantity a stackable name carries."""
    name = re.sub(r"<[^>]*>", "", name)
    name = re.sub(r"\s+", " ", name).strip()
    if int(definition.get("max_stack", 1)) > 1:
        name = re.sub(r"^\d+\s+", "", name)
        name = singular(name)
    return name


def ascii_text(name: str) -> str:
    """The list is ASCII; the JSON keeps the exact text with \\u escapes."""
    return unicodedata.normalize("NFKD", name).encode("ascii", "ignore").decode("ascii")


def markdown_name(name: str) -> str:
    """A table cell that cannot end the row."""
    return ascii_text(name).replace("|", "\\|") if name else "-"


def write_list(pack: dict, path: Path) -> None:
    """Rewrite `config/items-list.md` in the format the committed file uses."""
    definitions = pack["definitions"]
    live = [definition for definition in definitions if definition.get("live")]
    data_only = [definition for definition in definitions if not definition.get("live")]
    lines = [
        "# Item list",
        "",
        f"Total definitions: {len(definitions)}; live: {len(live)}; data-only: {len(data_only)}.",
        "",
        "Generated from `config/World/Vanilla/items.json`. `live` means the client item probe has "
        "shown the wire identity and `/item` accepts it.",
        "",
        "## Live definitions",
        "",
        "| name | id | file | model | type | value | stack |",
        "|---|---|---|---|---|---|---|",
    ]
    for definition in sorted(live, key=lambda item: item.get("description", "").lower()):
        lines.append(
            "| {name} | `{id}` | 0x{file:x} | {model} | {type} | {value} | {stack} |".format(
                name=markdown_name(definition.get("description", "")),
                id=definition.get("id", ""),
                file=int(definition.get("file_id", 0)),
                model=int(definition.get("model_id", 0)),
                type=int(definition.get("item_type", 0)),
                value=int(definition.get("value", 0)),
                stack=int(definition.get("max_stack", 1)),
            )
        )
    lines.extend(
        [
            "",
            "## Data-only definitions",
            "",
            "| name | id | file | model | type | value | stack |",
            "|---|---|---|---|---|---|---|",
        ]
    )
    for definition in sorted(data_only, key=lambda item: item.get("description", "").lower()):
        lines.append(
            "| {name} | `{id}` | 0x{file:x} | {model} | {type} | {value} | {stack} |".format(
                name=markdown_name(definition.get("description", "")),
                id=definition.get("id", ""),
                file=int(definition.get("file_id", 0)),
                model=int(definition.get("model_id", 0)),
                type=int(definition.get("item_type", 0)),
                value=int(definition.get("value", 0)),
                stack=int(definition.get("max_stack", 1)),
            )
        )
    path.write_text("\n".join(lines) + "\n", encoding="ascii")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--names", type=Path, default=DEFAULT_NAMES)
    parser.add_argument("--pack", type=Path, default=DEFAULT_PACK)
    parser.add_argument("--list", dest="list_path", type=Path, default=DEFAULT_LIST)
    parser.add_argument("--no-list", action="store_true", help="leave config/items-list.md alone")
    parser.add_argument("--dry-run", action="store_true", help="print the changes and write nothing")
    args = parser.parse_args()

    if not args.names.is_file():
        print(f"{args.names} is not there; run tools/decode-item-names.py first", file=sys.stderr)
        return 2

    report = json.loads(args.names.read_text(encoding="utf-8"))
    names = {
        (int(record["file_id"]) & 0x7FFFFFFF, int(record["item_type"]), int(record["model_id"])):
        str(record.get("name", ""))
        for record in report.get("identities", [])
        if str(record.get("name", "")).strip()
    }
    base_names = {
        str(record.get("id", "")): str(record.get("base_name", ""))
        for record in report.get("base_names", [])
        if str(record.get("base_name", "")).strip()
    }
    pack = json.loads(args.pack.read_text(encoding="utf-8"))
    definitions = pack["definitions"]

    updated = 0
    kept = 0
    preview: list[tuple[str, str, str]] = []
    seen: dict[str, int] = {}
    for definition in definitions:
        item_id = str(definition.get("id", ""))
        if not item_id.startswith("live_"):
            kept += 1
            continue
        base = plain_name(base_names.get(item_id, ""), definition)
        display = plain_name(names.get(identity(definition), ""), definition)
        name = base or display
        if not name:
            kept += 1
            continue
        if name in seen:
            if display and display != name and display not in seen:
                name = display
            else:
                file_id, item_type, model_id = identity(definition)
                name = f"{name} [0x{file_id:x}/{model_id}/{item_type}]"
        seen[name] = seen.get(name, 0) + 1
        if definition.get("description") != name:
            if len(preview) < 25:
                preview.append((item_id, definition.get("description", ""), name))
            if not args.dry_run:
                definition["description"] = name
            updated += 1

    if args.dry_run:
        for item_id, old, new in preview:
            print(f"{item_id}: {old!r} -> {new!r}")
        print(f"{updated} description(s) would change, {kept} definition(s) kept")
        return 0

    args.pack.write_text(
        json.dumps(pack, indent=2, ensure_ascii=True) + "\n",
        encoding="ascii",
        newline="\r\n",
    )
    if not args.no_list:
        write_list(pack, args.list_path)
    print(
        f"{updated} description(s) updated, {kept} definition(s) kept -> {args.pack}"
        + ("" if args.no_list else f"; list -> {args.list_path}")
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
