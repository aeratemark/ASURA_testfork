"""One-off check: every top-level item in the pre-split lib.rs still exists after the split."""

import glob
import re
import subprocess

old = subprocess.run(
    ["git", "show", "HEAD:crates/asura-world/src/lib.rs"],
    capture_output=True, text=True, encoding="utf-8",
).stdout

PAT = re.compile(r"^(pub |pub\(crate\) )?(async )?(fn|struct|enum|impl|mod|trait|const|static|type) ")


def names(text):
    out = set()
    for line in text.splitlines():
        m = PAT.match(line)
        if not m:
            continue
        kind = m.group(3)
        rest = line[m.end():].strip()
        if kind == "impl":
            out.add("impl " + rest.split("{")[0].strip())
        elif kind == "mod":
            out.add("mod " + re.match(r"[A-Za-z0-9_]+", rest).group(0))
        else:
            n = re.match(r"[A-Za-z0-9_]+", rest)
            if n:
                out.add(f"{kind} {n.group(0)}")
    return out


new = "".join(open(p, encoding="utf-8").read() for p in glob.glob("crates/asura-world/src/*.rs"))
before, after = names(old), names(new)
missing = sorted(before - after)
print(f"items before: {len(before)}, after: {len(after)}")
print(f"missing after the move: {len(missing)}")
for name in missing:
    print("  ", name)
