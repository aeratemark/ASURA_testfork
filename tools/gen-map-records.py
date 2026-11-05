"""Rewrite every map folder's basic.json with the current doc text, keeping its values.

The folder layout is `maps/<campaign>/<area>/<subarea>/<map id>/` (`docs/38-layer-decisions.md`, D38), and
the placement is a measurement, not something this tool decides: it finds the folders whose name is a map
id, reads the id and the instance value each one already states, and writes the file back with the doc
text below. The values are never chosen here - a folder with no value has no value to write.

    python tools/gen-map-records.py
"""

import json
import pathlib

PACK = pathlib.Path("config/World/Vanilla")

DOC = (
    "The instance value a live load sent with this map in GAME_SMSG_INSTANCE_TABLE (0x01A0): the client "
    "resolves a destination by this pair, so a map without one cannot be named. Copied from the recording "
    "of the load of instance 148 (recordings/client/feature/maps/pre-searing-load-3.jsonl). Maps that "
    "share a value belong to one base map or region, which is why the number is not always unique."
)


def folders() -> list[pathlib.Path]:
    return sorted(
        folder
        for folder in (PACK / "maps").rglob("*")
        if folder.is_dir() and folder.name.isdigit()
    )


def main() -> None:
    written, missing = 0, []
    for folder in folders():
        file = folder / "basic.json"
        if not file.exists():
            missing.append(str(folder))
            continue
        record = json.loads(file.read_text(encoding="utf-8"))
        record["doc"] = DOC
        file.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8", newline="")
        written += 1
    print(f"rewrote {written} map files")
    if missing:
        print(f"folders with no basic.json: {missing}")


if __name__ == "__main__":
    main()
