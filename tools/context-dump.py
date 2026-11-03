"""Dump the client's own structures - what an instance's initialization filled - into JSONL.

Why this exists
---------------
A live server fills the client's structures at initialization: the agent array, the party, the
instance, the guild and town-alliance block, the item tables. Fiddling with a window afterwards
proves nothing about them, because they are not built on demand - they were built once, by the
stream that entered the instance. So the two halves of one measurement are:

  * the stream, from ``tools/sniff-capture.py`` (both directions, no filter, attached before the
    load); and
  * the structures, from this script, read immediately after the load settles, again after a
    travel, and again after some idle time, so whatever does not change is what the load set.

Pairing the two field by field is how an initialization message is identified instead of guessed
at. This project's own documents name only the sniffer's catalogues of the instrument
(``docs/adr/0013-py4gw-stealth-is-our-link-to-the-client.md``); the contexts are the other half
and they are read through the instrument, never copied from it (ADR 0013 #4: no file, offset,
byte pattern or table from it enters this tree - a measurement written as ours, with the
recording named as the proof, is what crosses back).

This script is a front end, the same shape as the sniffer's
-----------------------------------------------------------
The readers are the library's own, exactly the set its ``examples/all_contexts.py`` walks. This
script does not reimplement one; it resolves the library root the same way the sniffer does,
connects **read-only**, and writes one JSONL record per context. Nothing here reads memory on its
own, and nothing here writes to the client.

Read-only by construction
-------------------------
The library documents ``connect(..., game_thread=False)`` as the read-only connection: contexts,
``Map``, ``Player`` and ``Party`` only read. This script asks for that explicitly and refuses to
fall back to the writing connection, so a run of it cannot patch the client.

Names are hashed, not printed
-----------------------------
Every string a context carries is a name, a tag or a title. They are replaced by a stable
``sha256`` prefix plus the string's length, so the same character, guild or hero can be followed
across a dump and a capture without a name being written down. Numbers, booleans and numeric
arrays are kept verbatim: they are the measurement.

Usage
-----
    # after a load, into the default file under recordings/asura/world/
    python tools/context-dump.py --label after-load-148

    # a chosen file, and only the contexts named
    python tools/context-dump.py --label after-travel --only agentarray,partycontext,map
    python tools/context-dump.py --label after-load --out D:\\dumps\\load-148.jsonl

    # neither a client nor the library is needed for this
    python tools/context-dump.py --self-test

Exit codes: 0 dumped, 2 this interpreter cannot connect, 3 no client or no library,
4 a dump that failed part way (the records written before it are kept).
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

#: The library root, resolved the way ``tools/sniff-capture.py`` resolves it: the same constant,
#: the same environment variable and the same flag, so one machine has one answer.
DEFAULT_PY4GW_ROOT = r"C:\Users\Apo\Py4GW_Stealth"

#: The readers, by the names the library's own all-contexts example uses. A reader that raises or
#: returns nothing is recorded as such rather than ending the dump.
READERS = (
    ("charcontext", "py4gw.context.charcontext.get"),
    ("gamecontext", "py4gw.context.gamecontext.get"),
    ("worldcontext", "py4gw.context.worldcontext.get"),
    ("mapcontext", "client.read_map_context"),
    ("missionmap", "client.read_mission_map_context"),
    ("worldmap", "client.read_world_map_context"),
    ("salvage", "client.read_salvage_session"),
    ("gameplaycontext", "py4gw.context.gameplaycontext.get"),
    ("pregamecontext", "py4gw.context.pregamecontext.get"),
    ("instanceinfo", "py4gw.context.instanceinfo.get"),
    ("serverregion", "py4gw.context.serverregion.get"),
    ("textparser", "py4gw.context.textparser.get"),
    ("cinematic", "py4gw.context.cinematic.get"),
    ("camera", "py4gw.context.camera.get"),
    ("partycontext", "py4gw.context.partycontext.get"),
    ("guildcontext", "py4gw.context.guildcontext.get"),
    ("friendlist", "py4gw.context.friendlist.get"),
    ("chatbuffer", "py4gw.context.chatbuffer.get"),
    ("tradecontext", "py4gw.context.tradecontext.get"),
    ("itemcontext", "py4gw.context.itemcontext.get"),
    ("accountcontext", "py4gw.context.accountcontext.get"),
    ("gadgetcontext", "py4gw.context.gadgetcontext.get"),
    ("accagentcontext", "py4gw.context.accagentcontext.get"),
    ("availablecharacters", "py4gw.context.availablecharacters.get"),
    ("agentarray", "client.agent_array.get_context"),
)

#: A context nests a little and never deeply; a cap keeps a cyclic struct from hanging the dump.
MAX_DEPTH = 6

#: Elements kept per array. The true length is recorded beside them, so a table larger than this is
#: a count with its head, never a silent truncation.
MAX_ELEMENTS = 64

#: Fields kept per element when the element has no dict view and has to be probed attribute by
#: attribute. A ceiling, so one deeply nested record cannot crowd out the arrays beside it.
MAX_FIELDS_PER_ELEMENT = 48

#: Strings are hashed to this many hex characters. Eight is enough to tell two hundred agents
#: apart and short enough to read in a diff.
HASH_HEX = 8


def hash_text(text: str) -> str:
    """A stable, non-reversible tag for a string a context carries."""
    digest = hashlib.sha256(text.encode("utf-8", "surrogatepass")).hexdigest()
    return f"h:{digest[:HASH_HEX]}:{len(text)}"


def flatten(value, depth: int = 0):
    """Turn one field into something JSON can hold, hashing every string on the way.

    Numbers, booleans and ``None`` pass through: they are the measurement. A string becomes a
    hash. A list or tuple keeps its length and its elements' order, because the order of an agent
    array or a party is part of what the load set.
    """
    if depth > MAX_DEPTH:
        return "elided: too deep"
    if value is None or isinstance(value, (bool, int, float)):
        return value
    if isinstance(value, str):
        return hash_text(value)
    if isinstance(value, (bytes, bytearray)):
        return {"bytes": len(value), "sha256": hashlib.sha256(bytes(value)).hexdigest()[:HASH_HEX]}
    if isinstance(value, (list, tuple)):
        return [flatten(item, depth + 1) for item in value]
    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            name = str(key)
            # A buffer pointer is an address, not a measurement: it moves every run and says
            # nothing about the structure. The size and capacity beside it are the measurement.
            if name in ("m_buffer",) or name.endswith("_ptr") or name.endswith("_address"):
                out[name] = "pointer"
            else:
                out[name] = flatten(item, depth + 1)
        return out
    # An object the library did not turn into a dict: name it and keep its repr's shape, not its
    # content, because the content may be a pointer or a name.
    return {"object": type(value).__name__}


def probe_one(name: str, context):
    """What this object can be asked for, and how big the arrays it holds are.

    Written for the pass after the first dump: `to_dict()` gives a context's scalars but shows an
    array as its container header. This records the public attribute names, and the size of every
    attribute that looks like an array view, so the element reader can be written against names
    that exist rather than names I assumed.
    """
    record = {"record": "probe", "name": name, "class": type(context).__name__}
    attrs = [attr for attr in dir(context) if not attr.startswith("_")]
    record["attrs"] = attrs
    sizes = {}
    for attr in attrs:
        try:
            value = getattr(context, attr)
        except Exception as error:  # noqa: BLE001 - a property that raises is worth naming
            sizes[attr] = f"raised {type(error).__name__}"
            continue
        size = getattr(value, "size", None) if not callable(value) else None
        if isinstance(size, int):
            sizes[attr] = {"size": size, "view": type(value).__name__}
    record["arrays"] = sizes
    return record


def element_fields(element):
    """One element of one array, with its strings hashed and its pointers dropped.

    Three routes, in order of how much the library tells us: the element's own ``to_dict``, its
    field iterator, and finally its public attributes read one by one. The last one is what makes
    an element whose struct has no dict view (a travel portal, a spawn point) still measurable
    instead of a bare type name.
    """
    for extract in (
        lambda: element.to_dict(),
        lambda: dict(element.iter_fields()),
    ):
        try:
            return flatten(extract())
        except Exception:  # noqa: BLE001 - try the next route
            continue
        finally:
            pass
    probed = {}
    for attr in dir(element):
        if attr.startswith("_"):
            continue
        try:
            value = getattr(element, attr)
        except Exception:  # noqa: BLE001 - a property that raises is simply absent
            continue
        if callable(value):
            continue
        if isinstance(value, (int, float, bool, str, type(None))):
            probed[attr] = flatten(value)
        elif isinstance(value, (list, tuple)) and len(value) <= 32:
            probed[attr] = flatten(list(value))
        if len(probed) >= MAX_FIELDS_PER_ELEMENT:
            break
    return probed or {"object": type(element).__name__}


def element_arrays(context):
    """Every array a context exposes, with its length and the elements themselves.

    `to_dict()` shows an array as its container header - a buffer pointer, a capacity and a size -
    which names the array but not what is in it. The element reads live on the context's own
    views (`worldcontext.agent_name_info`, `guildcontext.player_roster`,
    `agentarray.GetAgentArray()` and their siblings); this walks them, records each one's true
    length, and keeps the first `MAX_ELEMENTS` so one large table cannot bury the rest.
    """
    out = {}
    for attr in dir(context):
        if attr.startswith("_"):
            continue
        try:
            value = getattr(context, attr)
        except Exception:  # noqa: BLE001 - a property that raises is not the dump's end
            continue
        if callable(value):
            continue
        elements = None
        try:
            if hasattr(value, "to_list"):
                elements = list(value.to_list())
            elif isinstance(value, (list, tuple)) and value and not isinstance(value[0], (int, float, str)):
                elements = list(value)
        except Exception:  # noqa: BLE001 - an array view that will not convert is recorded as absent
            continue
        if elements is None:
            continue
        if not elements:
            out[attr] = {"length": 0}
            continue
        out[attr] = {
            "length": len(elements),
            "kept": min(len(elements), MAX_ELEMENTS),
            "elements": [element_fields(item) for item in elements[:MAX_ELEMENTS]],
        }
    return out


def dump_one(name: str, context):
    """One JSON-ready record for one context: its scalars, and the elements of every array."""
    record = {"record": "context", "name": name, "class": type(context).__name__}
    try:
        fields = context.to_dict()
    except Exception as error:  # noqa: BLE001 - a context without to_dict is still worth naming
        record["state"] = "no-to-dict"
        record["error"] = f"{type(error).__name__}: {error}"
        return record
    record["state"] = "read"
    record["field_count"] = len(fields)
    record["fields"] = {str(key): flatten(value) for key, value in fields.items()}
    arrays = element_arrays(context)
    if arrays:
        record["arrays"] = arrays
    return record


def _find_32_bit_interpreter() -> str | None:
    """The 32-bit interpreter this tool must run under, if one can be found."""
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
        result = subprocess.run(
            ["py", "-3.13-32", "-c", "import sys; print(sys.executable)"],
            capture_output=True, text=True, timeout=30,
        )
        line = (result.stdout or "").strip().splitlines()
        if result.returncode == 0 and line:
            return line[-1]
    except Exception:  # noqa: BLE001 - no launcher, or no 32-bit install
        pass
    return None


def require_32_bit(argv: list[str]) -> int | None:
    """Re-exec under the 32-bit interpreter, or explain and refuse."""
    if sys.maxsize <= 2**32:
        return None
    interpreter = _find_32_bit_interpreter()
    if interpreter is None:
        print("this tool is 32-bit only: the client and the bridge are x86.", file=sys.stderr)
        print("Install the 32-bit Python 3.13, or set ASURA_PY32, and run it again.", file=sys.stderr)
        return 2
    print(f"re-executing under the 32-bit interpreter: {interpreter}")
    try:
        child = subprocess.run([interpreter, str(Path(__file__).resolve()), *argv])
    except KeyboardInterrupt:
        return 130
    return child.returncode


def find_clients(win32, pid: int | None) -> list[dict]:
    """Find the client, including a renamed or patched one."""
    if pid is not None:
        return [{"pid": int(pid), "name": None, "path": None}]
    clients = list(win32.find_guild_wars())
    if clients:
        return clients
    for entry in win32.list_processes():
        name = str(entry.get("name") or "")
        if name.lower().startswith("gw") and not name.lower().startswith("asura"):
            clients.append({"pid": int(entry["pid"]), "name": name, "path": None})
    return clients


def resolve(expr: str, namespace: dict):
    """Resolve ``py4gw.context.x.get`` or ``client.agent_array.get_context`` by walking attributes."""
    head, _, rest = expr.partition(".")
    if head not in namespace:
        raise LookupError(f"unknown namespace {head!r} in {expr!r}")
    value = namespace[head]
    for part in rest.split("."):
        value = getattr(value, part)
    return value


def run_dump(args) -> int:
    root = Path(args.py4gw_root)
    if not (root / "py4gw").is_dir():
        print(f"py4gw not found under {root}", file=sys.stderr)
        print("pass --py4gw-root, or set PY4GW_STEALTH", file=sys.stderr)
        return 3
    sys.path.insert(0, str(root))

    import py4gw                                       # noqa: E402
    from py4gw.win32 import Win32                      # noqa: E402

    win32 = Win32()
    clients = find_clients(win32, args.pid)
    if not clients:
        print("no Guild Wars client is running (looked for Gw*.exe; pass --pid)", file=sys.stderr)
        return 3
    target = clients[0]
    print(f"--- client: pid {target.get('pid')} {target.get('name') or ''}")

    # Read-only, explicitly. `game_thread=True` installs hooks in the client; a dump must not.
    try:
        client = py4gw.connect(target, game_thread=False)
    except TypeError:
        print("this library build has no read-only connection (connect(..., game_thread=False));",
              file=sys.stderr)
        print("refusing to run, because the default connection patches the client.", file=sys.stderr)
        return 2
    except Exception as error:  # noqa: BLE001 - elevation and a lost client both land here
        print(f"connect failed: {type(error).__name__}: {error}", file=sys.stderr)
        print("if this says the token is not elevated, start an administrator PowerShell.",
              file=sys.stderr)
        return 2

    only = set(part.strip() for part in (args.only or "").split(",") if part.strip())
    out = Path(args.out) if args.out else (
        Path(__file__).resolve().parent.parent / "recordings" / "asura" / "world"
        / f"context-{args.label or 'dump'}.jsonl"
    )
    out.parent.mkdir(parents=True, exist_ok=True)

    read = 0
    empty = 0
    failed = 0
    try:
        with open(out, "w", encoding="utf-8") as handle:
            handle.write(json.dumps({
                "record": "dump",
                "tool": "asura/tools/context-dump.py",
                "label": args.label,
                "started_at": datetime.now(timezone.utc).isoformat(),
                "client": {"pid": target.get("pid"), "name": target.get("name")},
                "connection": "read-only",
                "names": "hashed",
                "mode": "probe" if args.probe else "dump",
            }) + "\n")

            for name, expr in READERS:
                if only and name not in only:
                    continue
                try:
                    reader = resolve(expr, {"py4gw": py4gw, "client": client})
                    context = reader()
                except Exception as error:  # noqa: BLE001 - one broken reader is not the dump
                    handle.write(json.dumps({
                        "record": "context", "name": name, "state": "error",
                        "error": f"{type(error).__name__}: {error}",
                    }) + "\n")
                    failed += 1
                    print(f"  {name:<20} ERROR {type(error).__name__}")
                    continue
                if context is None:
                    handle.write(json.dumps({"record": "context", "name": name, "state": "none"}) + "\n")
                    empty += 1
                    print(f"  {name:<20} none")
                    continue
                record = probe_one(name, context) if args.probe else dump_one(name, context)
                handle.write(json.dumps(record) + "\n")
                read += 1
                if args.probe:
                    print(f"  {name:<20} {record.get('class', '?'):<28} {len(record.get('arrays', {})):>3} array(s)"
                          f" of {len(record.get('attrs', []))} attr(s)")
                else:
                    print(f"  {name:<20} {record.get('class', '?'):<28} {record.get('field_count', 0):>3} fields")
            handle.write(json.dumps({
                "record": "summary", "read": read, "empty": empty, "failed": failed,
            }) + "\n")
    finally:
        try:
            py4gw.disconnect()
        except Exception as error:  # noqa: BLE001 - a disconnect that fails says nothing about the dump
            print(f"--- note: disconnect reported {error}", file=sys.stderr)

    print(f"--- {read} context(s) read, {empty} empty, {failed} failed -> {out}")
    return 0 if failed == 0 else 4


def self_test() -> int:
    """No client, no library: the hashing and the flattening, which is where a wrong dump hides."""
    assert flatten(7) == 7
    assert flatten(True) is True
    assert flatten(None) is None
    tag = flatten("Koss")
    assert tag.startswith("h:") and tag.endswith(":4"), tag
    assert flatten("Koss") == tag, "the same name must hash the same way twice"
    assert flatten("Koss") != flatten("koss"), "a different name must not collide"
    assert flatten([1, "A", 2])[1] == flatten("A"), "a party keeps its order, hashes its names"
    assert flatten({"x": 1, "y": "B"})["x"] == 1
    deep = cur = {}
    for _ in range(MAX_DEPTH + 3):
        cur["next"] = {}
        cur = cur["next"]
    assert flatten(deep) is not None, "a deep struct must not hang the dump"
    assert flatten(b"\x01\x02")["bytes"] == 2

    class _Context:
        def to_dict(self):
            return {"name": "Koss", "level": 20, "tags": ["a", "b"]}

    record = dump_one("partycontext", _Context())
    assert record["state"] == "read" and record["field_count"] == 3
    assert record["fields"]["level"] == 20
    assert record["fields"]["name"].startswith("h:")
    assert record["fields"]["tags"] == [flatten("a"), flatten("b")], "a list keeps order"

    class _Broken:
        def to_dict(self):
            raise RuntimeError("no fields")

    assert dump_one("x", _Broken())["state"] == "no-to-dict"
    print("self-test: hashing, ordering, depth cap and a context without to_dict all behave")
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    code = require_32_bit(argv)
    if code is not None:
        return code
    parser = argparse.ArgumentParser(description="Dump the client's structures to JSONL, read-only.")
    parser.add_argument("--py4gw-root", default=os.environ.get("PY4GW_STEALTH", DEFAULT_PY4GW_ROOT),
                        help="the library root that contains py4gw/ (default: %(default)s)")
    parser.add_argument("--pid", type=int, default=None, help="the client's pid, else auto-detect")
    parser.add_argument("--label", default=None, help="what moment this dump is, e.g. after-load-148")
    parser.add_argument("--only", default=None, help="comma-separated context names to read")
    parser.add_argument("--out", default=None, help="write here instead of recordings/asura/world/")
    parser.add_argument("--probe", action="store_true",
                        help="record each context public attribute names and the size of its arrays")
    return run_dump(parser.parse_args(argv))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
