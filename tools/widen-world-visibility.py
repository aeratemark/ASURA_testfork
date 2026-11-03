"""One-off: make the split domain modules' items visible to each other.

The world's domains call into one another (`connection` runs `answer`, `reference`s
`Play`, `item` builds blocks the dispatcher sends). Before the split everything lived in
one module; after it, each item needs a visibility. This widens them, mechanically:

* a top-level `fn`/`struct`/`enum`/`const`/`static`/`type` becomes `pub`;
* a method inside an `impl` becomes `pub` (only the method line -- a multi-line
  signature's parameters are at the same indent and are not touched);
* a field inside a `struct` body becomes `pub`, tracked by a small state machine so
  function parameters are never mistaken for fields.

Tightening these to `pub(crate)` per item is a separate pass, for when the API is settled.
"""

import re

FILES = "agent connection creation dispatch item map player travel world chat".split()
ROOT = "crates/asura-world/src"

TOP = re.compile(r"^(async fn |fn |struct |enum |const |static |type )")
METHOD = re.compile(r"^(async fn |fn )")
FIELD = re.compile(r"^[a-z_][a-z0-9_]*: (?!:)")
STRUCT_OPEN = re.compile(r"^(pub )?struct \w+(<[^>]*>)? \{$")
TRAIT_IMPL = re.compile(r"^impl .* for ")

for name in FILES:
    path = f"{ROOT}/{name}.rs"
    out = []
    inside_struct = False
    in_trait_impl = False
    for line in open(path, encoding="utf-8").read().splitlines():
        stripped = line.lstrip()
        indent = len(line) - len(stripped)

        if indent == 0 and STRUCT_OPEN.match(stripped):
            inside_struct = True
            out.append("pub " + stripped if not stripped.startswith("pub ") else stripped)
            continue
        if indent == 0 and TRAIT_IMPL.match(stripped):
            in_trait_impl = True
            out.append(line)
            continue
        if indent == 0 and stripped == "}":
            inside_struct = False
            in_trait_impl = False
            out.append(line)
            continue

        if indent == 0 and TOP.match(stripped):
            line = "pub " + line if not stripped.startswith("pub ") else line
        elif indent == 4 and METHOD.match(stripped) and not in_trait_impl:
            line = "    pub " + stripped if not stripped.startswith("pub ") else line
        elif inside_struct and FIELD.match(stripped) and not stripped.startswith("//"):
            line = "    pub " + stripped if not stripped.startswith("pub ") else line
        out.append(line)

    open(path, "w", encoding="utf-8").write("\n".join(out) + "\n")
    print(f"{name}.rs: visibility widened")
