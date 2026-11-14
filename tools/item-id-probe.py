"""Read the running client's item records into JSON, without writing to it.

    python tools/item-id-probe.py --py4gw Other_Repos_Research/Py4GW_Stealth-master --out items.json

This is a research tool for item file/model ids. It uses Py4GW Stealth's read-only
item context (`game_thread=False`) and prints one record per item in the player's
bags. It does not create, move, equip or destroy anything.

The probe needs the same thing every Py4GW read needs: a running 32-bit Guild Wars
client and an elevated shell. It fails with a clear message when Py4GW or the
client is absent.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_PY4GW = ROOT / "Other_Repos_Research" / "Py4GW_Stealth-master"


def find_python32() -> str | None:
    override = os.environ.get("ASURA_PY32")
    if override and Path(override).is_file():
        return override
    for candidate in (
        Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "Python" / "Python313-32" / "python.exe",
        Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "Python" / "Python312-32" / "python.exe",
    ):
        if candidate.is_file():
            return str(candidate)
    try:
        result = subprocess.run(
            ["py", "-3.13-32", "-c", "import sys; print(sys.executable)"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        lines = (result.stdout or "").strip().splitlines()
        if result.returncode == 0 and lines:
            return lines[-1]
    except Exception:
        pass
    return None


def require_python32(argv: list[str]) -> int | None:
    if sys.maxsize <= 2**32:
        return None
    interpreter = find_python32()
    if interpreter is None:
        print("this probe needs 32-bit Python; install it or set ASURA_PY32", file=sys.stderr)
        return 2
    print(f"re-executing under the 32-bit interpreter: {interpreter}")
    child = subprocess.run([interpreter, str(Path(__file__).resolve()), *argv])
    return child.returncode


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--py4gw", type=Path, default=DEFAULT_PY4GW)
    parser.add_argument("--out", type=Path, default=None)
    args = parser.parse_args()

    if not (args.py4gw / "py4gw").is_dir():
        print(f"missing Py4GW source tree: {args.py4gw}", file=sys.stderr)
        return 2
    sys.path.insert(0, str(args.py4gw))

    try:
        import py4gw
        from py4gw.inventory import Inventory
        from py4gw.item import PyItem
        from py4gw.item_array import ItemArray
        from py4gw.map import Map
        from py4gw.win32 import Win32
    except Exception as error:
        print(f"cannot import Py4GW: {error}", file=sys.stderr)
        return 2

    win32 = Win32()
    if not win32.is_elevated():
        print("run this from an elevated shell; Py4GW refuses a read connection without it", file=sys.stderr)
        return 3
    clients = win32.find_guild_wars()
    if not clients:
        print("no Guild Wars client is running", file=sys.stderr)
        return 3

    def ask(call, default=None):
        try:
            return call()
        except Exception as error:
            return {"error": f"{type(error).__name__}: {error}"}

    with py4gw.connect(clients[0], game_thread=False) as client:
        report = {
            "pid": int(clients[0]["pid"]),
            "map_id": ask(Map.GetMapID),
            "note": "read-only: game_thread=False, no hook, no patch, no client call",
            "items": [],
        }
        bags = ask(ItemArray.GetAllBags, [])
        if not isinstance(bags, list):
            report["error"] = bags
        else:
            ids = ask(lambda: ItemArray.GetItemArray(bags), [])
            if not isinstance(ids, list):
                report["error"] = ids
            else:
                for raw_id in ids:
                    item_id = int(raw_id.value) if hasattr(raw_id, "value") else int(raw_id)
                    item = PyItem(item_id)
                    bag, slot = ask(lambda: Inventory.FindItemBagAndSlot(item_id), (None, None))
                    name = ""
                    name_error = None
                    try:
                        raw_name = [int(byte) for byte in item.GetCompleteNameEnc()]
                        name_enc = [
                            raw_name[index] | (raw_name[index + 1] << 8)
                            for index in range(0, len(raw_name) - 1, 2)
                        ]
                    except Exception as error:
                        name_enc = []
                        name_error = f"name_enc: {type(error).__name__}: {error}"
                    try:
                        item.RequestName()
                        deadline = time.perf_counter() + 1.2
                        while time.perf_counter() < deadline and not item.IsItemNameReady():
                            time.sleep(0.01)
                        name = item.GetName()
                    except RuntimeError as error:
                        name_error = f"name needs game_thread=True: {error}"
                    except Exception as error:
                        name_error = f"name: {type(error).__name__}: {error}"
                    report["items"].append(
                        {
                            "item_id": item_id,
                            "bag": bag,
                            "slot": slot if slot is not None else item.slot,
                            "model_id": item.model_id,
                            "model_file_id": item.model_file_id,
                            "item_type": item.item_type.ToInt(),
                            "item_type_name": item.item_type.GetName(),
                            "name": name,
                            "name_enc": name_enc,
                            "name_error": name_error,
                            "quantity": item.quantity,
                            "interaction": item.interaction,
                            "is_identified": item.is_identified,
                            "is_customized": item.is_customized,
                            "profession": item.profession,
                            "value": item.value,
                            "modifiers": [int(modifier.mod) for modifier in item.modifiers],
                        }
                    )

    text = json.dumps(report, indent=2, ensure_ascii=True)
    print(text)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(text + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    early = require_python32(sys.argv[1:])
    raise SystemExit(early if early is not None else main())
