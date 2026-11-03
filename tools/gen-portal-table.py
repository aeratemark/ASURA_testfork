r"""Dump the travel portals of every instance into `data/portals.json`.

Why this exists
---------------
A travel portal is a **prop** the client draws from the instance's own map data, and a portal crossing is
started by the server when the character reaches one (`docs/03-protocol.md` 5f, measured 2026-10-31). The
server therefore needs the portals' positions, and they are not in any recording: the recording has the
walk, not the geometry.

The instrument answers it. Its reader asks the client's own file code (`dat_reader`), so a connected client
is required even for an instance nobody is standing in - but the instance does not have to be the current
one.

Running it
----------
32-bit interpreter, elevated, with the client running (the bridge must be attached), from the repository
root:

    & "$env:LOCALAPPDATA\Programs\Python\Python313-32\python.exe" tools/gen-portal-table.py
    & "$env:LOCALAPPDATA\Programs\Python\Python313-32\python.exe" tools/gen-portal-table.py --map-ids 146,148,164
    & "$env:LOCALAPPDATA\Programs\Python\Python313-32\python.exe" tools/gen-portal-table.py --live

`--live` reads the instance the character is standing in instead of the catalogue, which is the only way to
tell the offline reader's answer from the live one.

What crosses back
-----------------
`data/portals.json`, and nothing else: one object per instance with its portals' `x`, `y`, `z` and
`model_file_id`. The instrument is read here and never becomes a dependency (`tools/` only, and this script
fails closed without it).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

#: The instrument's checkout, overridable because the path is per machine.
DEFAULT_PY4GW_ROOT = r"C:\Users\Apo\Py4GW_Stealth"

#: The twelve instances of the catalogue, plus 449 (Kamadan) - the set the world serves by default.
CATALOGUE = [148, 146, 145, 147, 160, 161, 162, 163, 164, 165, 166, 449]

REPO_ROOT = Path(__file__).resolve().parent.parent
OUT = REPO_ROOT / "data" / "portals.json"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--map-ids", default="", help="comma-separated instance ids (default: the catalogue)")
    parser.add_argument("--out", default=str(OUT), help="where to write the JSON")
    parser.add_argument("--live", action="store_true", help="read the instance the character stands in")
    parser.add_argument("--root", default=os.environ.get("PY4GW_STEALTH", DEFAULT_PY4GW_ROOT))
    parser.add_argument("--pid", type=int, default=None, help="the client's pid, if discovery picks the wrong one")
    args = parser.parse_args()

    if sys.maxsize <= 2**32:
        pass  # 32-bit: what the bridge expects
    else:
        print("this needs the 32-bit interpreter (the bridge reads the client's x86 contexts):", file=sys.stderr)
        print(r'  & "$env:LOCALAPPDATA\Programs\Python\Python313-32\python.exe" tools/gen-portal-table.py', file=sys.stderr)
        return 2

    if not Path(args.root).is_dir():
        print(f"no instrument at {args.root}; set PY4GW_STEALTH or --root", file=sys.stderr)
        return 2
    sys.path.insert(0, args.root)

    try:
        import py4gw  # noqa: PLC0415
        from py4gw.map import Map  # noqa: PLC0415
        from py4gw.win32 import Win32  # noqa: PLC0415
    except Exception as err:  # the tool fails closed, never installs anything
        print(f"cannot import the instrument: {err}", file=sys.stderr)
        return 2

    # `connect` wants the client's process, not just a live bridge: the same discovery the sniffer uses,
    # so a renamed or patched client (`Gw.custom.exe`) is found as well as `Gw.exe`.
    if args.pid is not None:
        target = {"pid": int(args.pid), "name": None, "path": None}
    else:
        win32 = Win32()
        clients = list(win32.find_guild_wars())
        if not clients:
            for entry in win32.list_processes():
                name = str(entry.get("name") or "")
                lowered = name.lower()
                if lowered.startswith("gw") and not lowered.startswith("asura"):
                    clients.append({"pid": int(entry["pid"]), "name": name, "path": None})
        if not clients:
            print("no Guild Wars client found; start it first", file=sys.stderr)
            return 3
        target = clients[0]

    try:
        client = py4gw.connect(target)
    except Exception as err:
        print(f"cannot connect to the client (elevated shell needed): {err}", file=sys.stderr)
        return 2

    def portals(getter) -> list[dict]:
        found = []
        for portal in getter():
            found.append(
                {
                    "x": float(portal.x),
                    "y": float(portal.y),
                    "z": float(portal.z),
                    "model_file_id": int(portal.model_file_id),
                }
            )
        return found

    table: dict[str, object] = {}
    try:
        if args.live:
            current = int(Map.GetMapID())
            table[str(current)] = portals(lambda: Map.Pathing.GetTravelPortals())
            print(f"live: instance {current}: {len(table[str(current)])} portal(s)")
        else:
            ids = CATALOGUE if not args.map_ids else [int(part) for part in args.map_ids.split(",") if part.strip()]
            for map_id in ids:
                try:
                    table[str(map_id)] = portals(lambda: Map.Pathing.GetTravelPortals(map_id))
                except Exception as err:
                    table[str(map_id)] = {"error": str(err)}
                    print(f"{map_id}: failed: {err}", file=sys.stderr)
                    continue
                count = table[str(map_id)]
                print(f"{map_id}: {len(count) if isinstance(count, list) else 0} portal(s)")
    finally:
        try:
            py4gw.disconnect()
        except Exception:
            pass

    path = Path(args.out)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(table, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
