"""Capture the live client's Guild Wars packets into a JSONL file.

Why this exists
---------------
A drift, a terrain jump, or a client that dies mid-load is not visible from the
server's own log alone: the server says what IT believes, and the capture says what
the client actually sent and received, with timings. Diffing the two -- and diffing
a capture of this server against a capture of the reference at the same spot -- is
how a movement or load problem is diagnosed instead of guessed at.

The sniffer itself lives in the client-side injection (Py4GW_Stealth). This script
is the front end: it says "capture for N seconds, from this client, into this file".
ASURA has no other dependency on that project; nothing is read from it at runtime.

Elevation is required, and not by choice
----------------------------------------
`py4gw.connect()` asserts an elevated token and refuses without one, because
connecting is a write -- shared block, dispatcher, handler entries. A process cannot
raise its own token, so:

    Start an elevated PowerShell (approve the UAC prompt), then run this script.

Running it unelevated prints the reason and exits 2.

32-bit only
-----------
The client rig patches the 32-bit client's threads and reads the instruction pointer
through the x86 ``CONTEXT``. A 64-bit interpreter sees a zero instruction pointer
from that client and the patcher refuses to write. So this tool is 32-bit only:

- started with a 64-bit interpreter, it runs itself again under the 32-bit one and waits
  for it, so the console and the exit code are the capture's, not the wrapper's (found via
  ``ASURA_PY32``, the standard per-user 32-bit 3.12/3.13 install, or ``py -3.13-32``);
- with no 32-bit interpreter available, it says so and exits 2;
- a 64-bit client is refused for the same reason.

`--self-test` needs neither a client nor elevation.

The log is a ring, so this drains it
------------------------------------
The client-side sniffer keeps a capped log and drops the oldest entry first. A long
session would therefore silently truncate itself. This script reads the log,
**clears it**, and repeats -- so the file is complete regardless of how long it runs.

Records are sanitised by default
--------------------------------
Captures contain UTF-16 player names and UUIDs, so a capture must be sanitised before
anyone shares it. This script therefore, by default:

  * replaces the payload of the 15 headers the sniffer itself names as carrying
    UTF-16 names with a marker, keeping `size`;
  * masks every run of printable UTF-16LE code units (>= 6) anywhere else, zeroing
    the bytes and recording the ranges in `masked`, so offsets stay meaningful.

`--raw` turns both off, keeps every byte, and prints a warning. A raw capture must
not be committed.

Usage
-----
    # both directions, 90 seconds, into a named file
    py tools/sniff-capture.py --seconds 90 --dir both --out recordings/client/feature/movement/movement-sniff-20260101-000000.jsonl

    # only the movement messages
    py tools/sniff-capture.py --seconds 60 --dir both --header 0x0029,0x002D,0x0030,0x803D,0x803E,0x8028

    # no client, no elevation: prove the sanitizer and the writer work
    py tools/sniff-capture.py --self-test

Exit codes: 0 captured (or self-test passed), 2 not elevated, 3 no client found,
4 capture failed, 130 stopped from the 64-bit wrapper by Ctrl+C (the 32-bit capture stops
itself and prints its summary).

This file stays ASCII-only, like every script in tools/ (docs/HANDOFF.md).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

#: Where the client-side injection lives by default. Overridable with ``--py4gw-root``
#: or the ``PY4GW_STEALTH`` environment variable, because the path is per machine.
DEFAULT_PY4GW_ROOT = r"C:\Users\Apo\Py4GW_Stealth"

#: This repository, so the default capture lands in the recording archive
#: (recordings/client/feature/movement/, see recordings/README.md).
REPO_ROOT = Path(__file__).resolve().parent.parent

#: The shortest run of printable UTF-16LE code units treated as text. Six characters
#: is above the false-positive rate of binary payloads read as pairs of printable
#: bytes (~1e-5 per six-unit run) and below every player name.
UTF16_RUN_MIN = 6

#: Code units that count as text. Tab is included because client strings can hold it.
PRINTABLE_RANGES = ((0x20, 0x7E), (0x09, 0x09))

#: Seconds of silence after which the run says so once. A character-creation screen legitimately
#: sends nothing while a person reads it, so this is a heartbeat, not an error.
QUIET_NOTE_SECONDS = 60.0


# --------------------------------------------------------------------------- #
# sanitising
# --------------------------------------------------------------------------- #

def _is_printable(unit: int) -> bool:
    return any(low <= unit <= high for low, high in PRINTABLE_RANGES)


def mask_utf16_runs(data: bytes) -> tuple[bytes, list[list[int]]]:
    """Zero every printable UTF-16LE run of at least ``UTF16_RUN_MIN`` characters.

    Returns the masked bytes and the ``[offset, byte_length]`` ranges that were
    zeroed. Offsets are byte offsets into the packet as captured, so a masked
    capture still lines up with an unmasked one field for field.
    """
    masked = bytearray(data)
    ranges: list[list[int]] = []

    start = -1          # byte offset where the current run began
    units = 0           # code units in the current run

    for offset in range(0, len(data) - 1, 2):
        unit = data[offset] | (data[offset + 1] << 8)
        if _is_printable(unit):
            if start < 0:
                start = offset
            units += 1
            continue

        if start >= 0 and units >= UTF16_RUN_MIN:
            length = units * 2
            ranges.append([start, length])
            masked[start:start + length] = b"\x00" * length
        start = -1
        units = 0

    # A run that reaches the end of the packet.
    if start >= 0 and units >= UTF16_RUN_MIN:
        length = min(units * 2, len(data) - start)
        ranges.append([start, length])
        masked[start:start + length] = b"\x00" * length

    return bytes(masked), ranges


def sanitise(direction: str, header: int, data: bytes, name_headers: frozenset[int]) -> dict:
    """Produce the ``data`` half of a record: hex, plus what was removed from it."""
    if direction == "StoC" and header in name_headers:
        # These headers are known to carry UTF-16 names; drop the payload wholesale
        # rather than trusting the run scan to find all of it.
        return {"data": None, "redacted": "utf16-name-header", "masked": []}

    masked, ranges = mask_utf16_runs(data)
    return {"data": masked.hex(), "redacted": None, "masked": ranges}


# --------------------------------------------------------------------------- #
# writing
# --------------------------------------------------------------------------- #

def open_capture(path: Path):
    path.parent.mkdir(parents=True, exist_ok=True)
    return path.open("w", encoding="utf-8", newline="\n")


def write_record(handle, record: dict) -> None:
    handle.write(json.dumps(record, separators=(",", ":")) + "\n")


def exe_identity(exe_path: str | None) -> dict:
    """Identify the client executable, because that is what a capture must state.

    Packet layouts are build-specific, so a capture with no client identity is not
    evidence of anything. The version resource cannot be trusted for this: on a real
    install it reads `1.0.0.1`, while the build this tooling documents itself against
    is `39112`. So the **hash** is what identifies the build, and the version string
    is recorded beside it for the human reader.
    """
    identity = {"path": exe_path, "file_version": None, "sha256": None, "size": None}
    if not exe_path:
        return identity

    try:
        import hashlib

        digest = hashlib.sha256()
        with open(exe_path, "rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        identity["sha256"] = digest.hexdigest()
        identity["size"] = os.path.getsize(exe_path)
    except Exception:  # noqa: BLE001 - an unreadable exe must not fail a capture
        pass

    try:
        import ctypes
        from ctypes import wintypes

        version = ctypes.WinDLL("version")
        size = version.GetFileVersionInfoSizeW(exe_path, None)
        if size:
            buffer = ctypes.create_string_buffer(size)
            if version.GetFileVersionInfoW(exe_path, 0, size, buffer):
                value = ctypes.c_void_p()
                length = wintypes.UINT()
                if version.VerQueryValueW(
                    buffer, "\\", ctypes.byref(value), ctypes.byref(length)
                ):
                    class FixedFileInfo(ctypes.Structure):
                        _fields_ = [
                            ("signature", wintypes.DWORD),
                            ("struc_version", wintypes.DWORD),
                            ("file_version_ms", wintypes.DWORD),
                            ("file_version_ls", wintypes.DWORD),
                            ("product_version_ms", wintypes.DWORD),
                            ("product_version_ls", wintypes.DWORD),
                            ("flags_mask", wintypes.DWORD),
                            ("flags", wintypes.DWORD),
                            ("os", wintypes.DWORD),
                            ("type", wintypes.DWORD),
                            ("subtype", wintypes.DWORD),
                            ("date_ms", wintypes.DWORD),
                            ("date_ls", wintypes.DWORD),
                        ]

                    info = ctypes.cast(value, ctypes.POINTER(FixedFileInfo)).contents
                    ms, ls = info.file_version_ms, info.file_version_ls
                    identity["file_version"] = f"{ms >> 16}.{ms & 0xFFFF}.{ls >> 16}.{ls & 0xFFFF}"
    except Exception:  # noqa: BLE001 - a missing version string is not worth failing
        pass

    return identity


# --------------------------------------------------------------------------- #
# both halves of the capture loop
# --------------------------------------------------------------------------- #

def parse_headers(text: str | None) -> set[int] | None:
    if not text:
        return None
    headers = set()
    for token in text.split(","):
        token = token.strip()
        if not token:
            continue
        headers.add(int(token, 0))
    return headers


def find_clients(win32, pid: int | None) -> list[dict]:
    """Find the client, including a renamed/patched one.

    The shipped matcher looks for ``Gw.exe``; test rigs run ``Gw.custom.exe``. Fall
    back to any process whose name starts with ``gw``, excluding this repository's own
    services; ``--pid`` skips discovery entirely.
    """
    if pid is not None:
        return [{"pid": int(pid), "name": None, "path": None}]

    clients = list(win32.find_guild_wars())
    if clients:
        return clients

    for entry in win32.list_processes():
        name = str(entry.get("name") or "")
        lowered = name.lower()
        if lowered.startswith("gw") and not lowered.startswith("asura"):
            path = None
            try:
                path, _ = win32._get_image_path(int(entry["pid"]))
            except Exception:  # noqa: BLE001 - the path is a nicety
                pass
            clients.append({"pid": int(entry["pid"]), "name": name, "path": path})
    return clients


def exe_machine(exe_path: str | None) -> int | None:
    """The PE COFF machine field: ``0x14C`` is 32-bit x86, ``0x8664`` is 64-bit AMD64."""
    if not exe_path:
        return None
    try:
        with open(exe_path, "rb") as handle:
            data = handle.read(64 * 1024)
        if data[:2] != b"MZ":
            return None
        e_lfanew = int.from_bytes(data[0x3C:0x40], "little")
        if data[e_lfanew:e_lfanew + 4] != b"PE\x00\x00":
            return None
        return int.from_bytes(data[e_lfanew + 4:e_lfanew + 6], "little")
    except Exception:  # noqa: BLE001 - an unreadable exe is just "unknown"
        return None


def _find_32_bit_interpreter() -> str | None:
    """The 32-bit interpreter this tool must run under, if one can be found.

    ``ASURA_PY32`` overrides. Otherwise the standard per-user 32-bit install is tried,
    then ``py -3.13-32``. The rig's thread patching reads the instruction pointer
    through the x86 ``CONTEXT``, so this tool is 32-bit only: a 64-bit interpreter
    gets a zero instruction pointer from a 32-bit target and the patcher refuses.
    """
    override = os.environ.get("ASURA_PY32")
    if override and Path(override).is_file():
        return override
    candidates = [
        Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "Python" / "Python313-32" / "python.exe",
        Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "Python" / "Python312-32" / "python.exe",
    ]
    for candidate in candidates:
        if candidate.is_file():
            return str(candidate)
    try:
        import subprocess

        result = subprocess.run(
            ["py", "-3.13-32", "-c", "import sys; print(sys.executable)"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        line = (result.stdout or "").strip().splitlines()
        if result.returncode == 0 and line:
            return line[-1]
    except Exception:  # noqa: BLE001 - no launcher, or no 32-bit install
        pass
    return None


def require_32_bit(argv: list[str]) -> int | None:
    """Re-exec under the 32-bit interpreter, or explain and refuse.

    Returns an exit code when the tool cannot continue under this interpreter.
    """
    if sys.maxsize <= 2**32:
        return None
    interpreter = _find_32_bit_interpreter()
    if interpreter is None:
        print(
            "this tool is 32-bit only: the rig patches the 32-bit client's threads and reads",
            file=sys.stderr,
        )
        print(
            "the instruction pointer through the x86 CONTEXT, so a 64-bit interpreter sees a",
            file=sys.stderr,
        )
        print(
            "zero instruction pointer and refuses. Install the 32-bit Python 3.13, or set",
            file=sys.stderr,
        )
        print("ASURA_PY32 to a 32-bit python.exe, and run it again.", file=sys.stderr)
        return 2
    print(f"re-executing under the 32-bit interpreter: {interpreter}")
    # Not `os.execv`: Windows cannot overlay a process, so the C runtime starts the child and ends
    # this one immediately. The console then gets its prompt back while the capture is still
    # running, and the exit code belongs to the parent - a failed capture looked like "nothing
    # happened". A blocking run keeps the console honest and hands the child's code back out.
    try:
        child = subprocess.run([interpreter, str(Path(__file__).resolve()), *argv])
    except KeyboardInterrupt:
        # Ctrl+C goes to the whole console group, so the child saw it as well and is running its
        # own cleanup. Waiting on it here printed a second traceback after every capture; 130 is
        # the shell's own code for "stopped by Ctrl+C".
        return 130
    return child.returncode


def run_capture(args) -> int:
    root = Path(args.py4gw_root)
    if not (root / "py4gw").is_dir():
        print(f"py4gw not found under {root}", file=sys.stderr)
        print("pass --py4gw-root, or set PY4GW_STEALTH", file=sys.stderr)
        return 3
    sys.path.insert(0, str(root))

    import py4gw                                    # noqa: E402
    from py4gw.packet_sniffer import NAME_RE_STOC_HEADERS, SNIFFER   # noqa: E402
    from py4gw.win32 import Win32                   # noqa: E402

    win32 = Win32()
    if not win32.is_elevated():
        print("this needs an elevated shell: py4gw.connect() is a write.", file=sys.stderr)
        print("start PowerShell as administrator (approve the UAC prompt) and run it again.",
              file=sys.stderr)
        return 2

    clients = find_clients(win32, args.pid)
    if not clients:
        print("no Guild Wars client is running (looked for Gw*.exe; pass --pid if it is "
              "named something else)", file=sys.stderr)
        return 3
    # Say which client this is before touching it. Discovery is automatic and takes the first one;
    # printing every candidate makes it obvious when the window was watching the other copy, which
    # is otherwise indistinguishable from a capture that simply recorded nothing.
    for index, candidate in enumerate(clients):
        verb = "using" if index == 0 else "also running"
        print(
            f"--- {verb}: pid {candidate.get('pid')} {candidate.get('name') or ''} "
            f"{candidate.get('path') or '(path unreadable)'}"
        )
    target = clients[0]
    if target.get("path") is None and target.get("pid"):
        try:
            target["path"], _ = win32._get_image_path(int(target["pid"]))
        except Exception:  # noqa: BLE001 - the path is a nicety
            pass

    interpreter_64 = sys.maxsize > 2**32
    machine = exe_machine(target.get("path"))
    if machine is not None and machine != 0x014C:
        print(
            f"the client is a {('64' if machine == 0x8664 else 'non-x86')}-bit binary; this rig",
            file=sys.stderr,
        )
        print(
            "patches a 32-bit client's threads and reads the instruction pointer through the",
            file=sys.stderr,
        )
        print("x86 CONTEXT, so only a 32-bit client is supported.", file=sys.stderr)
        return 2
    if interpreter_64:
        # require_32_bit() re-execs before this point; reaching it is a bug.
        print("this interpreter is 64-bit and the re-exec did not happen", file=sys.stderr)
        return 2

    names = frozenset(NAME_RE_STOC_HEADERS) if not args.raw else frozenset()
    headers = parse_headers(args.header)

    out = Path(args.out) if args.out else REPO_ROOT / "recordings" / "client" / "feature" / "movement" / (
        f"movement-sniff-{datetime.now().strftime('%Y%m%d-%H%M%S')}.jsonl"
    )

    client = py4gw.connect(target)
    # Py4GW starts its own listeners on connect, and they share the callback thread that feeds the
    # sniffer. The merchant listener indexes `words[1]` on every packet it is given and raises
    # `IndexError` on a short one (`Py4GW_Stealth/py4gw/listeners.py:291`), which kills that thread:
    # the capture then records nothing more while the tool keeps polling, and the only sign is the
    # "tuple index out of range" it reports when it closes. This tool wants bytes, not listeners.
    try:
        from py4gw import listeners as listeners_module

        listeners_module.Shutdown()
    except Exception as exc:  # noqa: BLE001 - a listener that will not stop is not the capture's end
        print(f"--- note: could not disable Py4GW's listeners ({exc})", file=sys.stderr)
    identity = exe_identity(target.get("path"))
    started = time.time()
    counts: dict[str, int] = {}
    seq = 0
    captured = 0
    # The client's own clock, as the hook recorded it (`PacketLogEntry.tick`). `t` below is the tool's
    # drain clock: it says when this loop read the entry, not when the packet crossed, so a burst that
    # sat in Py4GW's ring carries one late stamp for all of it and a stall in this loop hides a gap.
    # Measured 2026-11-01: a load whose client-side steps are 0.05 s apart and 4.97 s apart cannot be
    # told from `t` alone once the ring is behind. The tick travels in every record so a reader can
    # subtract the client's own numbers instead; the span is in the summary so its unit is readable.
    first_tick: int | None = None
    last_tick: int | None = None

    try:
        with open_capture(out) as handle:
            write_record(handle, {
                "record": "capture",
                "tool": "asura/tools/sniff-capture.py",
                "started_at": datetime.fromtimestamp(started, timezone.utc).isoformat(),
                "client": {
                    "pid": int(target["pid"]),
                    "name": target.get("name"),
                    **identity,
                },
                # The layer this file is written in. The sniffer hooks the client's own receive and send
                # paths, so a record holds the client's message *struct*: a four-byte header (the u16 id
                # and a zero u16), every field widened to four bytes, and zero padding out to the
                # struct's size. A wire capture - `asura-world --capture` - holds the packed frame with
                # its two-byte header instead. The two are not interchangeable and the difference is not
                # a second protocol (docs/03-protocol.md 5e), so the layer travels with the file and a
                # reader takes the header length from here rather than assuming four.
                "layer": "struct",
                "header_len": 4,
                # Two clocks, and only one of them is the packet's. `t` is when this loop drained the
                # entry; `tick` is the client's own counter at the hook. Use `tick` for a gap.
                "timing": ("'t' is the tool's drain clock; 'tick' is the client's own counter at the "
                           "hook, so a gap between two records is a gap in the client's time"),
                "directions": args.dir,
                "header_filter": sorted(headers) if headers else None,
                "seconds": args.seconds,
                "sanitised": not args.raw,
                "notes": (
                    "sanitised: name-headers dropped, UTF-16 runs masked (offsets preserved)"
                    if not args.raw else
                    "RAW: contains names/UUIDs -- do not commit"
                ),
            })

            SNIFFER.initialize(args.dir)
            print(f"--- capturing {args.dir} from pid {target['pid']} "
                  f"(client sha256 {(identity['sha256'] or '?')[:16]}, "
                  f"version {identity['file_version']}) -> {out}")

            deadline = started + args.seconds if args.seconds else None
            # A capture that goes quiet is the failure that looks like success: the callback thread
            # can stop delivering while this loop keeps polling an empty ring. Say so once, with the
            # number of seconds, instead of letting the run look healthy until Ctrl+C.
            last_packet = started
            warned = False
            try:
                while True:
                    entries = SNIFFER.get_logs(args.dir)
                    if entries:
                        last_packet = time.time()
                        warned = False
                        for entry in entries:
                            header = int(entry.header)
                            if headers is not None and header not in headers:
                                continue
                            seq += 1
                            captured += 1
                            name = SNIFFER.get_packet_name(entry.direction, header)
                            counts[f"{entry.direction} {header:#06x} {name}"] = (
                                counts.get(f"{entry.direction} {header:#06x} {name}", 0) + 1
                            )
                            payload = sanitise(entry.direction, header, bytes(entry.data), names)
                            tick = int(getattr(entry, "tick", 0) or 0)
                            if first_tick is None:
                                first_tick = tick
                            last_tick = tick
                            write_record(handle, {
                                "seq": seq,
                                "t": round(time.time() - started, 3),
                                "tick": tick,
                                "direction": entry.direction,
                                "header": header,
                                "name": name,
                                "size": int(entry.size),
                                **payload,
                            })
                        handle.flush()
                        # Drain: the client-side log is a ring with a fixed cap.
                        SNIFFER.clear_logs(args.dir)

                    if not warned and time.time() - last_packet > QUIET_NOTE_SECONDS:
                        warned = True
                        print(
                            f"--- {captured} packet(s) so far; nothing for "
                            f"{int(time.time() - last_packet)}s - still capturing",
                            file=sys.stderr,
                        )

                    if deadline is not None and time.time() >= deadline:
                        break
                    # The GUI's Stop button (`tools/sniff-capture-gui.py`): a window cannot send this
                    # console-less process a Ctrl+C, and killing it would leave the handler entries
                    # patched, so it asks by creating this file and the loop ends the normal way.
                    if args.stop_file and Path(args.stop_file).exists():
                        print("--- stop requested")
                        break
                    time.sleep(args.poll)
            except KeyboardInterrupt:
                print("\n--- interrupted")

            # The inventory goes into the file as well as the console: a capture that carries its own
            # header counts means nobody has to re-read it, or remember the console, to know what it
            # holds. It is written **inside** the `with` that owns the handle: placed after that
            # block, every capture that held a packet ended in
            # `ValueError: I/O operation on closed file` instead of its last record (measured
            # 2026-11-01: a 344-packet capture of the patched client lost its summary that way).
            if counts:
                write_record(handle, {
                    "record": "summary",
                    "seconds": round(time.time() - started, 3),
                    "packets": captured,
                    "tick_first": first_tick,
                    "tick_last": last_tick,
                    "headers": [
                        {"seen": key, "count": counts[key]}
                        for key in sorted(counts, key=lambda k: counts[k], reverse=True)
                    ],
                    "raw": bool(args.raw),
                })
    except Exception as exc:  # noqa: BLE001 - always disconnect before leaving
        print(f"capture failed: {exc}", file=sys.stderr)
        return 4
    finally:
        # Cleanup of a capture that already happened. Both steps can fail in a way that says
        # nothing about the recording - a client that died under the hooks, and Py4GW's own
        # listeners re-raising their last callback failure on close - so each failure is one line
        # and the packets that were captured stay good evidence.
        try:
            SNIFFER.terminate(args.dir)     # puts every handler entry back
        except Exception as exc:  # noqa: BLE001
            print(f"--- note: the sniffer did not stop cleanly ({exc})", file=sys.stderr)
        try:
            py4gw.disconnect()
        except Exception as exc:  # noqa: BLE001
            print(f"--- note: disconnect reported {exc}", file=sys.stderr)


    print(f"--- {captured} packet(s) in {time.time() - started:.1f}s -> {out}")
    if first_tick is not None and last_tick is not None:
        span = last_tick - first_tick
        print(f"--- client tick {first_tick} -> {last_tick} (span {span}) over "
              f"{time.time() - started:.1f}s: a gap in client time is a tick gap, not a 't' gap")
    if counts:
        print("--- by header:")
        for key in sorted(counts, key=lambda k: counts[k], reverse=True):
            print(f"      {counts[key]:5d}  {key}")
    if args.raw:
        print("--- WARNING: raw capture. It contains player names and UUIDs. Do not commit it.")
    return 0


# --------------------------------------------------------------------------- #
# self-test: no client, no elevation
# --------------------------------------------------------------------------- #

def run_self_test() -> int:
    """Exercise the sanitizer and the writer against synthetic packets."""
    failures = 0

    def check(label: str, condition: bool, detail: str = "") -> None:
        nonlocal failures
        if condition:
            print(f"  ok   {label}")
        else:
            failures += 1
            print(f"  FAIL {label}{(' -- ' + detail) if detail else ''}")

    name = "Test Player One"
    utf16 = name.encode("utf-16le")
    payload = b"\x11\x22\x33\x44" + utf16 + b"\x55\x66\x77\x88"

    masked, ranges = mask_utf16_runs(payload)
    check("a UTF-16 name is found and masked", ranges == [[4, len(utf16)]], f"ranges={ranges}")
    check("masking preserves length", len(masked) == len(payload))
    check("the name is gone from the masked bytes", name.encode("utf-16le") not in masked)
    check("the surrounding words survive", masked[:4] == b"\x11\x22\x33\x44")

    short = "ab".encode("utf-16le")
    _, short_ranges = mask_utf16_runs(b"\x01\x02" + short + b"\x03\x04")
    check("a run shorter than the floor is left alone", short_ranges == [])

    odd, _ = mask_utf16_runs(b"\x00")
    check("a one-byte payload does not crash", odd == b"\x00")

    record = sanitise("StoC", 0x0084, payload, frozenset({0x0084}))
    check("a name header is dropped wholesale",
          record["data"] is None and record["redacted"] == "utf16-name-header")

    plain = sanitise("CToS", 0x003D, b"\x3d\x00\x00\x00" + b"\x00" * 24, frozenset({0x0084}))
    check("a non-name header keeps its bytes", plain["data"] is not None and plain["redacted"] is None)

    target = Path(os.environ.get("TEMP", ".")) / "asura-sniff-selftest.jsonl"
    with open_capture(target) as handle:
        write_record(handle, {"record": "capture", "tool": "self-test"})
        write_record(handle, {"seq": 1, "header": 0x3D, "size": 28, **plain})
    lines = target.read_text(encoding="utf-8").splitlines()
    check("the writer emits one JSON object per line", len(lines) == 2)
    check("the first line is the capture header", json.loads(lines[0])["record"] == "capture")
    check("the record round-trips through JSON",
          json.loads(lines[1])["header"] == 0x3D and json.loads(lines[1])["size"] == 28)
    target.unlink(missing_ok=True)

    print(f"\n--- self-test: {'PASS' if failures == 0 else f'{failures} FAILURE(S)'}")
    return 0 if failures == 0 else 1


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Capture the live client's Guild Wars packets (needs an elevated shell).",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__.split("Usage\n-----")[1] if "Usage\n-----" in __doc__ else None,
    )
    parser.add_argument("--dir", choices=("StoC", "CToS", "both"), default="StoC",
                        help="which half of the sniffer to start (default: StoC)")
    parser.add_argument("--seconds", type=float, default=30.0,
                        help="capture window; 0 waits for Ctrl+C (default: 30)")
    parser.add_argument("--header", type=str, default=None,
                        help="comma-separated header allow-list, e.g. 0x3D,0x3E")
    parser.add_argument("--out", type=str, default=None,
                        help="JSONL path (default: recordings/client/feature/movement/movement-sniff-<timestamp>.jsonl)")
    parser.add_argument("--pid", type=int, default=None, help="target a specific Gw*.exe pid")
    parser.add_argument("--poll", type=float, default=0.05, help="drain interval in seconds")
    parser.add_argument("--raw", action="store_true",
                        help="do not sanitise -- keeps names and UUIDs; do not commit the result")
    parser.add_argument("--py4gw-root", type=str,
                        default=os.environ.get("PY4GW_STEALTH", DEFAULT_PY4GW_ROOT),
                        help="where the client-side injection lives (default: PY4GW_STEALTH "
                             "or the machine's usual location)")
    parser.add_argument("--stop-file", type=str, default=None,
                        help="end the capture cleanly once this file exists (the GUI's Stop button)")
    parser.add_argument("--self-test", action="store_true",
                        help="check the sanitizer and the writer without a client or elevation")
    args = parser.parse_args(argv)

    if args.self_test:
        return run_self_test()
    blocked = require_32_bit(argv)
    if blocked is not None:
        return blocked
    return run_capture(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
