"""Check: every arm of the dispatch table calls a handler that exists in its domain.

The table is the single place a header is mapped to an owner; this asserts that each `0xNNNN =>
module::handle_x(...)` names a `pub fn handle_x` in that module's file, and that no arm hides
logic of its own (the only block left is the default, which ignores an unhandled header).
"""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "crates/asura-world/src"
DISPATCH = SRC / "dispatch.rs"

table = DISPATCH.read_text(encoding="utf-8")
calls = re.findall(r"0x([0-9A-Fa-f]{4}) => ([a-z_]+)::(handle_[a-z_]+)\(", table)
if not calls:
    raise SystemExit("no dispatch calls found: the table was rewritten")

missing = []
for header, module, handler in calls:
    path = SRC / f"{module}.rs"
    if not path.exists():
        missing.append(f"0x{header}: module {module}.rs does not exist")
        continue
    body = path.read_text(encoding="utf-8")
    if not re.search(rf"pub (async )?fn {re.escape(handler)}\(", body):
        missing.append(f"0x{header}: {module}::{handler} does not exist")

# one-line arms are fine (disconnect and the two keep-alives); an arm with a block of its own is
# the old dispatcher growing back
blocks = re.findall(r"0x[0-9A-Fa-f]{4} => \{", table)

print(f"table entries: {len(calls)}, inline blocks: {len(blocks)}")
for problem in missing:
    print("  ", problem)
if missing or blocks:
    raise SystemExit(1)
print("every dispatch arm resolves to a handler in its domain")
