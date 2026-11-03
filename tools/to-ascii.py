#!/usr/bin/env python3
"""One-off: make every source and document file pure ASCII.

Two steps, in this order, because the second depends on the first:

1. **Undo the mojibake.** Text that was UTF-8 read as Windows-1252 and written back as UTF-8
   holds mojibake where an em dash belongs. Re-encoding those runs as cp1252 bytes and decoding
   them as UTF-8 recovers the original characters (repeated, because some spots went round
   twice). A run that does not decode cleanly is left alone rather than guessed at.
2. **Map what is left to ASCII.** Em dash and en dash to `-`, arrows to `->`/`=>`,
   section sign to `#`, ellipsis to `...`, box drawing and bullets to their ASCII neighbours.
   That is a rendering change to prose, and it is the point: an ASCII file cannot be corrupted
   by an ANSI read, which is how the mojibake happened.

Excluded: `recordings/` (captured evidence, bytes we must not rewrite), `logs/`, `target*/`,
`.git/`, and binary files.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

EXCLUDE_DIRS = {".git", "target", "target-verify", "logs", "recordings"}
BINARY = re.compile(r"\.(bin|exe|dll|pdb|png|jpg|jpeg|webp|gif|zip|rar|dat|pyc)$", re.I)

# Windows-1252's 0x80-0x9F block, inverted: char -> byte.
CP1252_INVERSE = {
    0x80: "\u20ac", 0x82: "\u201a", 0x83: "\u0192", 0x84: "\u201e", 0x85: "\u2026",
    0x86: "\u2020", 0x87: "\u2021", 0x88: "\u02c6", 0x89: "\u2030", 0x8a: "\u0160",
    0x8b: "\u2039", 0x8c: "\u0152", 0x8e: "\u017d", 0x91: "\u2018", 0x92: "\u2019",
    0x93: "\u201c", 0x94: "\u201d", 0x95: "\u2022", 0x96: "\u2013", 0x97: "\u2014",
    0x98: "\u02dc", 0x99: "\u2122", 0x9a: "\u0161", 0x9b: "\u203a", 0x9c: "\u0153",
    0x9e: "\u017e", 0x9f: "\u0178",
}
CHAR_TO_BYTE = {char: byte for byte, char in CP1252_INVERSE.items()}

# ASCII replacements for what survives the un-mojibake pass.
ASCII_MAP = {
    "\u2014": "-",      # em dash
    "\u2013": "-",      # en dash
    "\u2192": "->",     # right arrow
    "\u2190": "<-",     # left arrow
    "\u21d2": "=>",     # double arrow
    "\u2026": "...",    # ellipsis
    "\u00a7": "#",      # section sign
    "\u201c": '"', "\u201d": '"', "\u00ab": '"', "\u00bb": '"',
    "\u2018": "'", "\u2019": "'", "\u201a": "'", "\u201b": "'",
    "\u2022": "*", "\u00b7": "|",   # bullet, middot (used as a separator)
    "\u2605": "*", "\u25b6": ">", "\u25b8": ">", "\u25ba": ">",
    "\u2500": "-", "\u2501": "-", "\u2502": "|", "\u2534": "+", "\u2514": "+",
    "\u2518": "+", "\u250c": "+", "\u2510": "+", "\u251c": "+", "\u2524": "+",
    "\u252c": "+", "\u253c": "+", "\u2550": "=", "\u2551": "|",
    "\u00d7": "x", "\u00bf": "?", "\u00b0": "deg",
}


def undo_mojibake(text):
    """Recover the characters a cp1252 round trip turned into mojibake."""
    pattern = re.compile(
        "|".join(
            re.escape(c) for c in sorted(CHAR_TO_BYTE, key=len, reverse=True)
        )
    )

    def fix(match):
        run = match.group(0)
        raw = bytes(CHAR_TO_BYTE[c] for c in run)
        try:
            decoded = raw.decode("utf-8")
        except UnicodeDecodeError:
            return run
        # a decoded run that is still all control characters is not text
        if not decoded or all(ord(c) < 0x20 for c in decoded):
            return run
        return decoded

    for _ in range(4):
        fixed = pattern.sub(fix, text)
        if fixed == text:
            break
        text = fixed
    return text


def to_ascii(text):
    text = undo_mojibake(text)
    out = []
    for char in text:
        if ord(char) <= 0x7F:
            out.append(char)
        elif char in ASCII_MAP:
            out.append(ASCII_MAP[char])
        else:
            out.append(char)

    # What is left is mojibake that did not decode (a cp1252 byte the decoder could not
    # complete, or a stray non-breaking space). Those were punctuation, so a run of them
    # becomes one hyphen; a lone one is dropped.
    return re.sub(
        "([^\x00-\x7F]+)",
        lambda match: "-" if len(match.group(1)) > 1 else "",
        "".join(out),
    )


def main():
    write = "--write" in sys.argv
    limit = int(sys.argv[1]) if len(sys.argv) > 1 and sys.argv[1].isdigit() else 0
    files, changed, leftovers = [], 0, []
    for path in ROOT.rglob("*"):
        if not path.is_file() or path.suffix.lower() in (".bin", ".exe", ".dll", ".pdb",
                                                         ".png", ".jpg", ".webp", ".gif",
                                                         ".zip", ".rar", ".dat", ".pyc"):
            continue
        if any(part in EXCLUDE_DIRS for part in path.relative_to(ROOT).parts[:-1]):
            continue
        raw = path.read_bytes()
        if b"\x00" in raw:
            continue
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            leftovers.append((path, "not UTF-8"))
            continue
        if not any(ord(ch) > 0x7F for ch in text):
            continue
        fixed = to_ascii(text)
        if fixed == text:
            leftovers.append((path, "unchanged"))
            continue
        remaining = sorted({ch for ch in fixed if ord(ch) > 0x7F})
        if remaining:
            leftovers.append((path, "".join(remaining)))
        files.append(path)
        changed += 1
        if write and (not limit or changed <= limit):
            path.write_bytes(fixed.encode("utf-8"))

    print(f"processed {changed} file(s)")
    if leftovers:
        print(f"unresolved ({len(leftovers)}):")
        for path, what in leftovers[:40]:
            print(f"   {path.relative_to(ROOT)}: {ascii(what)}")
        raise SystemExit(1)


if __name__ == "__main__":
    main()
