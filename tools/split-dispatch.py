#!/usr/bin/env python3
"""One-off: move every dispatch arm's body into the domain that owns the message.

`dispatch.rs` was a 935-line match whose arms did the domain's work in place. This moves each
body, unchanged, into a `handle_*` function in the noun's own file (`item.rs` for the item
messages, `chat.rs` for chat, ...), and leaves `dispatch.rs` as a table of calls - which is what
its header already claimed it was.

The move is textual: an arm's body is dedented and wrapped, the arm becomes one call. Doc
comments above the arm travel with it. What the body touches (`stream`, `session`, `frame`,
`world`, `play`, `trace`) decides the handler's argument list, so no handler carries an unused
parameter and clippy still passes with `-D warnings`.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "crates/asura-world/src"
DISPATCH = SRC / "dispatch.rs"

# header -> (module, handler name)
MAPPING = {
    "0x8091": ("item", "handle_request_items"),
    "0x8088": ("map", "handle_request_spawn"),
    "0x8090": ("world", "handle_request_players"),
    "0x800D": ("connection", "handle_ping_request"),
    "0x803E": ("agent", "handle_move_to_coord"),
    "0x803D": ("agent", "handle_position_update"),
    "0x8028": ("agent", "handle_cancel_movement"),
    "0x8047": ("agent", "handle_last_position"),
    "0x8064": ("chat", "handle_chat_message"),
    "0x8089": ("creation", "handle_request_player"),
    "0x808A": ("creation", "handle_request_armors"),
    "0x808B": ("creation", "handle_confirm"),
    "0x8060": ("creation", "handle_change_profession"),
    "0x802C": ("item", "handle_drop_item"),
    "0x8072": ("item", "handle_item_move"),
    "0x8030": ("item", "handle_equip_item"),
    "0x804F": ("item", "handle_unequip_item"),
}

# handler argument order, and the type each one has
ARGS = [
    ("stream", "stream: &mut TcpStream"),
    ("session", "session: &mut Session"),
    ("frame", "frame: &Frame"),
    ("world", "world: &World"),
    ("play", "play: &mut Play"),
    ("trace", "trace: bool"),
]

ARM = re.compile(r"^        (0x[0-9A-Fa-f]{4}) => \{$")
ONE_LINE_ARM = re.compile(r"^        (0x[0-9A-Fa-f]{4}) => ")
CLOSE = re.compile(r"^        \}$")
DEFAULT = re.compile(r"^        other => \{$")


def dedent(lines, amount=12):
    out = []
    for line in lines:
        out.append(line[amount:] if line[:amount].strip() == "" else line.lstrip())
    return out


def main():
    lines = DISPATCH.read_text(encoding="utf-8").splitlines()
    result = []
    handlers = {}   # module -> list of (comment lines, handler text)
    index = 0

    while index < len(lines):
        line = lines[index]
        arm = ARM.match(line)
        if arm and arm.group(1) in MAPPING:
            header = arm.group(1)
            module, name = MAPPING[header]
            # doc/comments immediately above the arm
            comments = []
            position = len(result)
            while position > 0 and re.match(r"^        //", result[position - 1]):
                position -= 1
                comments.insert(0, result.pop())
            # body: up to the arm's closing brace at the arm indent
            index += 1
            body = []
            while index < len(lines) and not CLOSE.match(lines[index]):
                body.append(lines[index])
                index += 1
            closing = lines[index]
            index += 1
            body = dedent(body)

            code = "\n".join(line for line in body if not line.lstrip().startswith("//"))
            used = {key for key, _ in ARGS if re.search(rf"\b{key}\b", code)}
            if re.search(r"\breplay\b", code):
                used.add("world")
                body = ["let replay = world.replay.as_ref();"] + body
            is_async = "await" in code
            signature = ", ".join(sig for key, sig in ARGS if key in used)
            call_args = []
            for key, _sig in ARGS:
                if key not in used:
                    continue
                call_args.append({"frame": "&frame"}.get(key, key))
            call = f"{module}::{name}({', '.join(call_args)})"
            if is_async:
                call += ".await"

            doc_lines = []
            for comment in comments:
                match = re.match(r"^        //\s?(.*)$", comment)
                if match:
                    doc_lines.append(("/// " + match.group(1)).rstrip())
            doc = "\n".join(doc_lines)
            kind = "async fn" if is_async else "fn"
            handler = (
                (doc + "\n" if doc else "")
                + f"pub {kind} {name}({signature}) -> Result<bool, GameError> {{\n"
                + "\n".join("    " + body_line for body_line in body)
                + "\n}"
            )
            handlers.setdefault(module, []).append(handler)
            result.append(f"        {header} => {call},")
            continue

        result.append(line)
        index += 1

    # ---- write the domains ------------------------------------------------------------
    for module, texts in handlers.items():
        path = SRC / f"{module}.rs"
        current = path.read_text(encoding="utf-8").rstrip()
        split = current.find("#[cfg(test)]")
        block = "\n\n".join(texts)
        if split >= 0:
            current = current[:split].rstrip() + "\n\n" + block + "\n\n" + current[split:]
        else:
            current = current + "\n\n" + block
        path.write_text(current + "\n", encoding="utf-8")
        print(f"{module}.rs: {len(texts)} handler(s)")

    # ---- write the table --------------------------------------------------------------
    text = "\n".join(result)
    text = text.replace("    let replay = world.replay.as_ref();\n", "")
    DISPATCH.write_text(text + "\n", encoding="utf-8")
    print(f"dispatch.rs: {text.count(chr(10)) + 1} lines")


if __name__ == "__main__":
    main()
