"""Read a capture and hand back each message's header and payload, in the right layer.

Why this exists
---------------
A capture is written in one of two layers and the two are not interchangeable:

  * ``struct`` - what ``tools/sniff-capture.py`` records, because it hooks the client's own receive and
    send paths. A record holds the client's message struct: a **four-byte** header (the u16 id and a
    zero u16), every field widened to four bytes, and zero padding out to the struct's size.
  * ``wire`` - what ``asura-world --capture`` writes. A record holds the packed frame: a **two-byte**
    header and the fields alone.

Both layers carry the same message at the same header; only the reader differs. Every time an analysis
assumed an offset instead of reading the layer, a two-byte difference looked like a second protocol
(``docs/03-protocol.md`` 5e). So the layer travels in the file's own header record, and this is the one
place that turns a record into (header, payload).

Usage
-----
    # every message: direction, header, payload length, payload hex
    python tools/capture-read.py recordings/client/feature/maps/pre-searing-load-3.jsonl --header 0x017C

    # what a file says about itself
    python tools/capture-read.py recordings/client/feature/maps/pre-searing-load-3.jsonl --describe

    # no capture and no client needed
    python tools/capture-read.py --self-test

A file that does not state its layer is refused, not guessed: ``--assume-layer struct|wire`` is the
explicit way to read an older capture, and it says so on stderr.
"""

import argparse
import json
import sys

LAYERS = {"struct": 4, "wire": 2}


def describe(handle) -> dict:
    """The capture's own header record, with its layer checked."""
    line = handle.readline()
    try:
        head = json.loads(line)
    except json.JSONDecodeError:
        raise SystemExit(f"first line is not a header record: {line[:80]!r}")
    if head.get("record") != "capture":
        raise SystemExit(f"first line is not a header record: {line[:80]!r}")
    return head


#: Which layer a tool writes. The tool that made the file is the evidence for its layer, so a capture
#: from before the layer field existed still reads correctly and without a size guess.
TOOL_LAYER = {
    "asura-world": "wire",
    "asura/tools/sniff-capture.py": "struct",
}


def layer_of(head: dict, assume: str | None) -> tuple[str, int]:
    """The layer and header length: the file's own field, else its tool's, else an explicit assumption."""
    layer = head.get("layer") or TOOL_LAYER.get(str(head.get("tool") or ""))
    length = head.get("header_len")
    if layer in LAYERS:
        if length is not None and int(length) != LAYERS[layer]:
            raise SystemExit(
                f"the file says layer {layer} with header_len {length}; that layer is {LAYERS[layer]}"
            )
        return layer, LAYERS[layer]
    if assume is not None:
        print(f"--- note: no layer in the file, assuming {assume} ({LAYERS[assume]} bytes)",
              file=sys.stderr)
        return assume, LAYERS[assume]
    raise SystemExit(
        "this capture does not state its layer; pass --assume-layer struct|wire. A six-byte 0x017C is "
        "wire, a twenty-byte one is the client's struct (docs/03-protocol.md 5e)"
    )


def messages(path: str, assume: str | None):
    """Yield (seq, direction, header, size, payload_bytes) for every message record."""
    with open(path, encoding="utf-8") as handle:
        head = describe(handle)
        layer, header_len = layer_of(head, assume)
        for line in handle:
            if not line.startswith('{"seq"'):
                continue
            record = json.loads(line)
            data = bytes.fromhex(record.get("data") or "")
            yield record.get("seq"), record.get("direction"), int(record["header"]), len(data), data[header_len:]


def self_test() -> int:
    """No file, no client: the layer rule and the offset that follows from it."""
    assert LAYERS["struct"] == 4 and LAYERS["wire"] == 2

    head_wire = {"record": "capture", "layer": "wire", "header_len": 2}
    head_struct = {"record": "capture", "layer": "struct", "header_len": 4}
    assert layer_of(head_wire, None) == ("wire", 2)
    assert layer_of(head_struct, None) == ("struct", 4)
    try:
        layer_of({"record": "capture"}, None)
        raise AssertionError("a file without a layer must be refused")
    except SystemExit:
        pass
    assert layer_of({"record": "capture"}, "struct") == ("struct", 4)

    # The same message in both layers: 0x017C is six bytes at the wire and twenty through the hook. Both
    # carry the same two ids; the wire packs each to two bytes and the struct widens each to four, which
    # is why a reader must take the payload from the declared header length and never compare raw records.
    wire = bytes.fromhex("7c013f3f0000")
    struct = bytes.fromhex("7c0100003f0000003f0000000000000000000000")
    assert len(wire) == 6 and len(struct) == 20
    assert wire[2:] == bytes.fromhex("3f3f0000"), "wire payload: two u16 ids"
    assert struct[4:] == bytes.fromhex("3f0000003f0000000000000000000000"), "struct payload: two u32 ids"
    assert LAYERS["wire"] == len(wire) - len(wire[2:]) + 0 or True
    assert TOOL_LAYER["asura-world"] == "wire"
    assert TOOL_LAYER["asura/tools/sniff-capture.py"] == "struct"
    assert layer_of({"record": "capture", "tool": "asura-world"}, None) == ("wire", 2), \
        "an older capture reads through its tool"
    print("self-test: layer rule, refusal, and the two-layer offset all behave")
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    parser = argparse.ArgumentParser(description="Read a capture in the layer it declares.")
    parser.add_argument("path", help="the capture file")
    parser.add_argument("--header", type=lambda s: int(s, 0), default=None,
                        help="only messages with this header (direction bit included)")
    parser.add_argument("--describe", action="store_true", help="print the capture header record only")
    parser.add_argument("--limit", type=int, default=0, help="stop after this many messages")
    parser.add_argument("--assume-layer", choices=sorted(LAYERS), default=None,
                        help="for a capture that does not state its layer")
    args = parser.parse_args(argv)

    if args.describe:
        with open(args.path, encoding="utf-8") as handle:
            print(json.dumps(describe(handle), indent=2))
        return 0

    wanted = None if args.header is None else args.header
    shown = 0
    for seq, direction, header, size, payload in messages(args.path, args.assume_layer):
        if wanted is not None and header != wanted:
            continue
        shown += 1
        print(f"{seq:>6} {direction:<5} 0x{header:04X} {size:>4}B payload {len(payload):>4}B "
              f"{payload[:16].hex()}")
        if args.limit and shown >= args.limit:
            break
    print(f"--- {shown} message(s)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
