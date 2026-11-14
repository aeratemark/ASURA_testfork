#!/usr/bin/env python3
"""Merge the item probe JSONs and compare them with the item catalogue.

    python tools/merge-item-probes.py

Every run of the capture GUI's Items tab writes one timestamped file; this reads all of them,
deduplicates the client item records by their full identity `(model_file_id, model_id, item_type)`,
and answers three questions for `config/World/Vanilla/items.json`:

  * which definitions a live item confirms exactly;
  * which definitions share a file id or a model id with a live item but are not the same item;
  * which definitions no live item has shown yet.

It writes `recordings/client/feature/items/items-live-merged.json` and
`recordings/client/feature/items/items-verification.md`.
The probe's encoded name units travel with every record; `tools/item-id-probe.py` saves them even
when the client's game thread is not available, and they can be resolved with Rurik's
`codedstr.py` plus `textrec.py` when a name is needed.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_DEFINITIONS = ROOT / "config" / "World" / "Vanilla" / "items.json"
DEFAULT_DIR = ROOT / "recordings" / "client" / "feature" / "items"
MERGED_NAME = "items-live-merged.json"
REPORT_NAME = "items-verification.md"


def file_id(value) -> int:
    return int(value) & 0x7FFFFFFF


def identity(item: dict) -> tuple[int, int, int]:
    return (
        file_id(item.get("model_file_id", item.get("file_id", 0))),
        int(item.get("model_id", 0)),
        int(item.get("item_type", 0)),
    )


def probe_files(directory: Path) -> list[Path]:
    return sorted(
        path
        for path in directory.glob("items-live*.json")
        if path.name != MERGED_NAME
    )


def merge(paths: list[Path]) -> dict[tuple[int, int, int], dict]:
    records: dict[tuple[int, int, int], dict] = {}
    for path in paths:
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            print(f"skip {path.name}: {error}")
            continue
        map_id = data.get("map_id")
        for item in data.get("items", []):
            key = identity(item)
            if key not in records:
                record = dict(item)
                record["sources"] = []
                records[key] = record
            records[key]["sources"].append(
                {
                    "file": path.name,
                    "map_id": map_id,
                    "item_id": item.get("item_id"),
                    "quantity": item.get("quantity"),
                }
            )
    return records


def candidate(item: dict) -> str:
    return (
        f"file 0x{file_id(item.get('model_file_id', 0)):x}, "
        f"model {item.get('model_id')}, type {item.get('item_type')}"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--definitions", type=Path, default=DEFAULT_DEFINITIONS)
    parser.add_argument("--dir", type=Path, default=DEFAULT_DIR)
    parser.add_argument("--merged", type=Path, default=DEFAULT_DIR / MERGED_NAME)
    parser.add_argument("--report", type=Path, default=DEFAULT_DIR / REPORT_NAME)
    args = parser.parse_args()

    definitions = json.loads(args.definitions.read_text(encoding="utf-8"))["definitions"]
    paths = probe_files(args.dir)
    if not paths:
        print(f"no item probe JSONs in {args.dir}")
        return 2
    records = merge(paths)

    pack_identities = {identity(definition) for definition in definitions}
    curated = [
        definition
        for definition in definitions
        if not definition.get("id", "").startswith("live_")
    ]
    generated = [
        definition
        for definition in definitions
        if definition.get("id", "").startswith("live_")
    ]

    verified = []
    near = []
    missing = []
    for definition in curated:
        key = identity(definition)
        if key in records:
            verified.append((definition, records[key]))
            continue
        same_file = [
            item
            for item in records.values()
            if file_id(item.get("model_file_id", 0)) == file_id(definition.get("file_id", 0))
        ]
        model_id = definition.get("model_id")
        same_model = (
            []
            if not model_id
            else [item for item in records.values() if item.get("model_id") == model_id]
        )
        if same_file or same_model:
            near.append((definition, same_file, same_model))
        else:
            missing.append(definition)

    new_live = [
        record
        for key, record in sorted(records.items())
        if key not in pack_identities
    ]
    known_live = len(records) - len(new_live)

    merged = {
        "note": "merged from item probe runs; identity is (model_file_id & 0x7fffffff, model_id, item_type)",
        "files": [path.name for path in paths],
        "identities": [dict(record) for _, record in sorted(records.items())],
    }
    args.merged.write_text(json.dumps(merged, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")

    live_count = sum(1 for definition in definitions if definition.get("live"))
    lines = [
        "# Item definition verification",
        "",
        f"Probe files: {len(paths)}; distinct live item identities: {len(records)}.",
        f"Pack definitions: {len(definitions)} ({live_count} live, {len(generated)} generated).",
        f"Live identities already in the pack: {known_live}; new in these probes: {len(new_live)}.",
        f"Curated definitions: {len(curated)}.",
        "",
        f"- curated exact live match: {len(verified)}",
        f"- curated near match (same file id or model id, different item): {len(near)}",
        f"- curated with no live item yet: {len(missing)}",
        "",
        "## Curated exact matches",
        "",
    ]
    if verified:
        for definition, item in verified:
            sources = ", ".join(sorted({source["file"] for source in item["sources"]}))
            lines.append(
                f"- `{definition['id']}` - {candidate(item)}; seen in {sources}"
            )
    else:
        lines.append("- none in these probe files")

    lines.extend(["", "## Curated near matches", ""])
    if near:
        for definition, same_file, same_model in near:
            lines.append(
                f"- `{definition['id']}` - definition has file "
                f"0x{file_id(definition.get('file_id', 0)):x}, model "
                f"{definition.get('model_id')}, type {definition.get('item_type')}"
            )
            for item in same_file:
                lines.append(f"  - same file id: {candidate(item)}")
            for item in same_model:
                if item not in same_file:
                    lines.append(f"  - same model id: {candidate(item)}")
    else:
        lines.append("- none")

    lines.extend(["", "## Curated with no live item yet", ""])
    for definition in missing:
        lines.append(
            f"- `{definition['id']}` - file 0x{file_id(definition.get('file_id', 0)):x}, "
            f"model {definition.get('model_id')}, type {definition.get('item_type')}"
        )

    lines.extend(["", "## New live identities", ""])
    if new_live:
        for item in new_live:
            sources = ", ".join(sorted({source["file"] for source in item["sources"]}))
            lines.append(f"- {candidate(item)}; seen in {sources}")
    else:
        lines.append("- none; every live identity is already in the pack")

    args.report.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(
        f"{len(records)} live identities from {len(paths)} file(s): "
        f"{known_live} already in the pack, {len(new_live)} new; "
        f"curated {len(verified)} exact, {len(near)} near, {len(missing)} missing"
    )
    print(f"wrote {args.merged}")
    print(f"wrote {args.report}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
