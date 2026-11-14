"""Decode the probe's encoded item names with the client's own text decoder.

    python tools/decode-item-names.py

`tools/item-id-probe.py` connects with `game_thread=False`, so the client's own
`GetName()` is not available. Every record it writes carries `name_enc`, the
coded display name the client composes from the base and its fitted upgrades.
This tool reads the merged probe and feeds each `name_enc` to the client's own
`AsyncDecodeStr` through Py4GW, then writes the decoded text to
`recordings/client/feature/items/items-live-names.json`.

The decode is a client call, not a read: the connection is opened with
`game_thread=True`, which installs Py4GW's game-thread hook for the run and
restores it on exit. It needs a running 32-bit Guild Wars client, 32-bit
Python, and an elevated shell. It changes no client state beyond the decoder's
own callback.

Without Py4GW, a client, or elevation the tool fails closed. A record the
decoder answers with an empty string is written with its error and is not
invented.

`tools/apply-item-names.py` reads the output and writes the names into the
pack's `description` fields.
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_PY4GW = ROOT / "Py4GW_Stealth-master"
DEFAULT_MERGED = ROOT / "recordings" / "client" / "feature" / "items" / "items-live-merged.json"
DEFAULT_PACK = ROOT / "config" / "World" / "Vanilla" / "items.json"
DEFAULT_OUT = ROOT / "recordings" / "client" / "feature" / "items" / "items-live-names.json"


def find_python32() -> str | None:
    """The 32-bit interpreter the probe uses, or None."""
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
    """Re-exec under 32-bit Python, or report that it is missing."""
    if sys.maxsize <= 2**32:
        return None
    interpreter = find_python32()
    if interpreter is None:
        print("this tool needs 32-bit Python; install it or set ASURA_PY32", file=sys.stderr)
        return 2
    print(f"re-executing under the 32-bit interpreter: {interpreter}")
    child = subprocess.run([interpreter, str(Path(__file__).resolve()), *argv])
    return child.returncode


def wide_bytes(units: list[int]) -> bytes:
    """The encoded string as the client's decoder takes it, terminator included."""
    return b"".join(struct.pack("<H", int(unit) & 0xFFFF) for unit in units) + b"\0\0"


def decode_with_poll(encoded: bytes) -> tuple[str, str | None]:
    """Decode one encoded string through the client, take its answer, free its slot.

    This is the slot machinery native's `RequestName` uses: begin a slot, let
    the client's own `AsyncDecodeStr` fill it, poll the completion, and take the
    text. The take is what frees the slot; a timeout releases it too, so a
    record the client never answers cannot exhaust the 32-slot bridge and turn
    every later record into a false failure.
    """
    from py4gw.client import require_client
    from py4gw.ui.async_decode import (
        DecodeState,
        async_decode_str,
        begin_string_decode,
        decode_state,
        decoded_text,
    )

    client = require_client()
    try:
        slot = begin_string_decode(encoded)
    except Exception as error:  # noqa: BLE001 - the client's refusal is the answer
        return "", f"begin decode: {type(error).__name__}: {error}"
    if not async_decode_str(encoded, slot):
        return "", "the client did not start the decode"
    deadline = time.perf_counter() + 1.2
    while time.perf_counter() < deadline:
        state = decode_state(slot)
        if state in (DecodeState.DONE, DecodeState.FAILED):
            text, _truncated = decoded_text(slot)
            if not text:
                return "", f"the decoder answered {state.name.lower()}"
            return text, None
        time.sleep(0.005)
    client.bridge.release_decode(slot)
    return "", "the decoder did not answer within 1.2 s"


def find_client(win32) -> list[dict]:
    """Py4GW's client list, with the owner's `Gw.custom.exe` accepted.

    `Win32.find_guild_wars` matches the executable name `Gw.exe` and nothing
    else. The Reforged client this repository runs beside is `Gw.custom.exe`;
    it is the same process shape, so a process whose name starts with `gw.` is
    read the same way. The extra name adds no behaviour, only the process the
    owner is actually running.
    """
    clients = win32.find_guild_wars()
    if clients:
        return clients
    for process in win32.list_processes():
        name = str(process.get("name", ""))
        folded = name.casefold()
        if folded == "gw.exe" or folded.startswith("gw."):
            clients.append(
                {
                    "pid": int(process["pid"]),
                    "name": name,
                    "path": None,
                    "path_error": None,
                }
            )
    return clients


def identity(item: dict) -> tuple[int, int, int]:
    """The probe's identity: file id without its high bit, model id, item type."""
    file_id = int(item.get("model_file_id", item.get("file_id", 0))) & 0x7FFFFFFF
    return file_id, int(item.get("item_type", 0)), int(item.get("model_id", 0))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--py4gw", type=Path, default=DEFAULT_PY4GW)
    parser.add_argument("--merged", type=Path, default=DEFAULT_MERGED)
    parser.add_argument("--pack", type=Path, default=DEFAULT_PACK)
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument(
        "--limit",
        type=int,
        default=0,
        help="decode at most this many records, for a first test; zero means every record",
    )
    args = parser.parse_args()

    if not (args.py4gw / "py4gw").is_dir():
        print(f"missing Py4GW source tree: {args.py4gw}", file=sys.stderr)
        return 2

    sys.path.insert(0, str(args.py4gw))
    try:
        import py4gw
        from py4gw.win32 import Win32
    except Exception as error:
        print(f"cannot import Py4GW: {error}", file=sys.stderr)
        return 2

    win32 = Win32()
    if not win32.is_elevated():
        print("run this from an elevated shell; Py4GW refuses a client connection without it", file=sys.stderr)
        return 3
    clients = find_client(win32)
    if not clients:
        print("no Guild Wars client is running", file=sys.stderr)
        return 3

    merged = json.loads(args.merged.read_text(encoding="utf-8"))
    records = list(merged.get("identities", []))
    if args.limit > 0:
        records = records[: args.limit]
    if not records:
        print(f"{args.merged} holds no identities", file=sys.stderr)
        return 2

    decoded = []
    failures = 0
    with py4gw.connect(clients[0], game_thread=True):
        for index, item in enumerate(records, start=1):
            file_id, item_type, model_id = identity(item)
            units = [int(unit) & 0xFFFF for unit in item.get("name_enc", [])]
            name = ""
            error = None
            if not units:
                error = "the probe recorded no name_enc"
            else:
                name, error = decode_with_poll(wide_bytes(units))
                name = name.strip()
            if not name:
                failures += 1
            decoded.append(
                {
                    "file_id": file_id,
                    "item_type": item_type,
                    "model_id": model_id,
                    "name": name,
                    "error": error,
                }
            )
            if index % 25 == 0 or index == len(records):
                print(f"decoded {index}/{len(records)}; {failures} without a name", file=sys.stderr)

        base_names = []
        base_failures = 0
        if args.pack.is_file():
            pack = json.loads(args.pack.read_text(encoding="utf-8"))
            generated = [
                definition
                for definition in pack.get("definitions", [])
                if str(definition.get("id", "")).startswith("live_")
            ]
            for index, definition in enumerate(generated, start=1):
                units = [int(unit) & 0xFFFF for unit in definition.get("name", [])]
                name = ""
                error = None
                if not units:
                    error = "the definition has no name units"
                else:
                    name, error = decode_with_poll(wide_bytes(units))
                    name = name.strip()
                if not name:
                    base_failures += 1
                base_names.append(
                    {
                        "id": definition.get("id", ""),
                        "base_name": name,
                        "error": error,
                    }
                )
                if index % 50 == 0 or index == len(generated):
                    print(
                        f"decoded base {index}/{len(generated)}; {base_failures} without a name",
                        file=sys.stderr,
                    )
        else:
            print(f"{args.pack} is not there; only display names were decoded", file=sys.stderr)

    report = {
        "note": (
            "Decoded item names. `identities[].name` is the client's answer for the probe's "
            "complete display name; `definitions[].base_name` is the client's answer for the "
            "pack's bare base name units. Both come from the client's own decoder through "
            "Py4GW on a game_thread=True connection. An empty name is a record the decoder did "
            "not answer; it is not a guess."
        ),
        "source": str(args.merged),
        "records": len(decoded),
        "without_name": failures,
        "definitions": len(base_names),
        "without_base_name": base_failures,
        "identities": decoded,
        "base_names": base_names,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")
    print(
        f"{len(decoded) - failures} display names, "
        f"{len(base_names) - base_failures} base names -> {args.out}"
    )
    return 1 if failures or base_failures else 0


if __name__ == "__main__":
    early = require_python32(sys.argv[1:])
    raise SystemExit(early if early is not None else main())
