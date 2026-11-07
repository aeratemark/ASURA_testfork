"""Write each map's client name id into the pack: `"name_id"` in every `maps/.../<map id>/basic.json`.

Why this exists
---------------
A party countdown (`0x01BC`) names its destination: "travelling to <map>", where <map> is the map's
own name as the client's string id. The client keeps that id in its map table (`AreaInfo`, one
0x7C-byte entry per map inside Gw.exe), and the server reads it from the map's `basic.json`
(`MapRecord.name_id`, `party::travel_map_name`). **A map without `name_id` gets no countdown window**:
its party still moves after the countdown, but no client shows the timer or the cancel button.

This reads the ids straight from the client, so a new map folder only needs this run again.

How the table is found
----------------------
The ids of five maps were seen in live countdowns (`recordings/client/feature/multiplay/`):
55 -> 0x287A, 163 -> 0x28DE, 165 -> 0x28E6, 166 -> 0x28EA, 809 -> 0x15797. The table is the place in
Gw.exe where those five sit at `map * 0x7C` from one base. If the exe holds no such place, nothing is
written: a wrong id would draw the wrong name, or none.

What is written
---------------
Only the `"name_id"` line, inserted after `"instance_value"` (or updated where it is already there).
Every other byte of the file is kept.

Usage
-----
    python tools/add-map-name-ids.py                       # D:\\GWClone\\Gw.exe, config/World/Vanilla
    python tools/add-map-name-ids.py --exe path\\to\\Gw.exe --pack config/World/MyPack
    python tools/add-map-name-ids.py --dry-run             # report, change nothing
    python tools/add-map-name-ids.py --self-test           # needs no client
"""

import argparse
import re
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_EXE = Path(r"D:\GWClone\Gw.exe")
DEFAULT_PACK = ROOT / "config" / "World" / "Vanilla"

STRIDE = 0x7C
# Map id -> name id, each one read from a live 0x01BC (see the module docstring).
ANCHORS = {55: 0x287A, 163: 0x28DE, 165: 0x28E6, 166: 0x28EA, 809: 0x15797}

INSTANCE_LINE = re.compile(rb'^([ \t]*)"instance_value"[ \t]*:[ \t]*\d+[ \t]*,?[ \t]*\r?$', re.M)
NAME_LINE = re.compile(rb'^([ \t]*)"name_id"[ \t]*:[ \t]*\d+([ \t]*,?)', re.M)


def find_table(exe: bytes) -> int:
    """The file offset of map 0's name id, where every anchor agrees; raises when there is none."""
    first_map, first_id = min(ANCHORS.items())
    needle = struct.pack("<I", first_id)
    found = []
    at = exe.find(needle)
    while at != -1:
        base = at - first_map * STRIDE
        if base >= 0 and all(
            base + m * STRIDE + 4 <= len(exe)
            and struct.unpack_from("<I", exe, base + m * STRIDE)[0] == v
            for m, v in ANCHORS.items()
        ):
            found.append(base)
        at = exe.find(needle, at + 1)
    if len(found) != 1:
        raise SystemExit(
            f"the map table is not in this exe ({len(found)} places match the captured ids); "
            "nothing written"
        )
    return found[0]


def name_id(exe: bytes, base: int, map_id: int) -> int:
    return struct.unpack_from("<I", exe, base + map_id * STRIDE)[0]


def with_name_id(text: bytes, value: int):
    """The file with its `name_id` set to `value`, or `None` when it has no `instance_value` line."""
    if NAME_LINE.search(text):
        return NAME_LINE.sub(lambda m: m.group(1) + b'"name_id": %d' % value + m.group(2), text, 1)
    match = INSTANCE_LINE.search(text)
    if not match:
        return None
    line = match.group(0)
    crlf = line.endswith(b"\r")
    newline = b"\r\n" if crlf else b"\n"
    stripped = line.rstrip(b"\r").rstrip()
    # Keep the comma rule: instance_value now always has one, and name_id has one unless it ends the
    # object (when instance_value was last).
    if stripped.endswith(b","):
        tail = b","
    else:
        stripped += b","
        tail = b""
    insert = match.group(1) + b'"name_id": %d' % value + tail + (b"\r" if crlf else b"")
    return text[: match.start()] + stripped + newline + insert + text[match.end():]


def map_files(pack: Path):
    """Every `basic.json` whose folder is named after a map id, below `pack/maps`."""
    for path in sorted((pack / "maps").rglob("basic.json")):
        if path.parent.name.isdigit():
            yield int(path.parent.name), path


def run(exe_path: Path, pack: Path, dry_run: bool) -> int:
    exe = exe_path.read_bytes()
    base = find_table(exe)
    print(f"map table at file offset {base:#x} in {exe_path}")
    changed = same = skipped = 0
    for map_id, path in map_files(pack):
        value = name_id(exe, base, map_id)
        if value == 0:
            print(f"  {map_id}: the client has no name for it; left as is")
            skipped += 1
            continue
        text = path.read_bytes()
        new = with_name_id(text, value)
        if new is None:
            print(f"  {map_id}: {path} has no instance_value line; left as is")
            skipped += 1
        elif new == text:
            same += 1
        else:
            changed += 1
            if not dry_run:
                path.write_bytes(new)
    verb = "would change" if dry_run else "changed"
    print(f"{verb} {changed}, already right {same}, skipped {skipped}")
    return 0


def self_test() -> int:
    exe = bytearray(0x200 + 900 * STRIDE)
    base = 0x100
    for m, v in ANCHORS.items():
        struct.pack_into("<I", exe, base + m * STRIDE, v)
    struct.pack_into("<I", exe, base + 148 * STRIDE, 0x28EE)
    assert find_table(bytes(exe)) == base
    assert name_id(bytes(exe), base, 148) == 0x28EE
    middle = b'{\n  "map_id": 148,\n  "instance_value": 7,\n  "doc": "x"\n}\n'
    assert with_name_id(middle, 10478) == (
        b'{\n  "map_id": 148,\n  "instance_value": 7,\n  "name_id": 10478,\n  "doc": "x"\n}\n'
    )
    last = b'{\r\n  "map_id": 1,\r\n  "instance_value": 7\r\n}\r\n'
    assert with_name_id(last, 5) == b'{\r\n  "map_id": 1,\r\n  "instance_value": 7,\r\n  "name_id": 5\r\n}\r\n'
    once = with_name_id(middle, 10478)
    assert with_name_id(once, 10478) == once, "running twice changes nothing"
    assert with_name_id(once, 9) == once.replace(b"10478", b"9"), "an existing id is updated"
    assert with_name_id(b'{"map_id": 1}', 5) is None
    print("self-test passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--exe", type=Path, default=DEFAULT_EXE, help="the client, Gw.exe")
    parser.add_argument("--pack", type=Path, default=DEFAULT_PACK, help="the world pack folder")
    parser.add_argument("--dry-run", action="store_true", help="report, change nothing")
    parser.add_argument("--self-test", action="store_true", help="check this script, no client needed")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    return run(args.exe, args.pack, args.dry_run)


if __name__ == "__main__":
    sys.exit(main())
