"""Optional fast path: ASURA's own Rust decompressor as a tiny helper executable.

The pure-Python port in ``fa_decompress`` is correct but slow (tens of seconds for a large map
blob). If ``rustc`` is available - and it is wherever ASURA was built - the editor compiles a
standalone helper from the ASURA checkout's ``fa_compress.rs`` + ``fa_compress_tables.rs`` (no
crates, no network) into the cache folder once, and pipes compressed entries through it.
The helper reads ``u32 size`` + compressed bytes on stdin and writes the decoded bytes on stdout.
"""

from __future__ import annotations

import hashlib
import logging
import os
import shutil
import subprocess
from pathlib import Path

log = logging.getLogger(__name__)

MAIN = r'''
fn main() {
    use std::io::{Read, Write};
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).unwrap();
    if input.len() < 4 { eprintln!("short input"); std::process::exit(2); }
    let size = u32::from_le_bytes([input[0], input[1], input[2], input[3]]) as usize;
    match decompress(&input[4..], size) {
        Ok(out) => { std::io::stdout().write_all(&out).unwrap(); }
        Err(e) => { eprintln!("{:?}", e); std::process::exit(1); }
    }
}
'''


def _source(asura_root: Path) -> str | None:
    src = asura_root / "crates" / "asura-world" / "src"
    fa, tables = src / "fa_compress.rs", src / "fa_compress_tables.rs"
    if not fa.exists() or not tables.exists():
        return None
    body = fa.read_text(encoding="utf-8").replace("use crate::fa_compress_tables::", "use tables::")
    cut = body.find("#[cfg(test)]")
    if cut >= 0:
        body = body[:cut]
    tab = "\n".join(line for line in tables.read_text(encoding="utf-8").splitlines() if not line.startswith("//!"))
    return f"#![allow(warnings)]\n{body}\nmod tables {{\n{tab}\n}}\n{MAIN}"


def helper(asura_root: str | Path, cache_dir: str | Path) -> Path | None:
    """Path to a built helper, building it if needed. None when impossible (no rustc / no ASURA)."""
    if not asura_root:
        return None
    src = _source(Path(asura_root))
    rustc = shutil.which("rustc")
    if src is None or rustc is None:
        return None
    digest = hashlib.sha1(src.encode()).hexdigest()[:12]
    out_dir = Path(cache_dir) / "native"
    exe = out_dir / (f"fa_decompress_{digest}" + (".exe" if os.name == "nt" else ""))
    if exe.exists():
        return exe
    out_dir.mkdir(parents=True, exist_ok=True)
    rs = out_dir / f"fa_decompress_{digest}.rs"
    rs.write_text(src, encoding="utf-8")
    try:
        r = subprocess.run([rustc, "-O", "--edition", "2021", str(rs), "-o", str(exe)],
                           capture_output=True, text=True, timeout=300)
    except (OSError, subprocess.TimeoutExpired) as e:
        log.warning("could not build the native decompressor: %s", e)
        return None
    if r.returncode != 0 or not exe.exists():
        log.warning("rustc failed building the native decompressor:\n%s", r.stderr[-2000:])
        return None
    return exe


def decompress(exe: Path, data: bytes, size: int) -> bytes:
    r = subprocess.run([str(exe)], input=size.to_bytes(4, "little") + data, capture_output=True, timeout=120)
    if r.returncode != 0:
        raise ValueError(f"native decompressor: {r.stderr.decode(errors='replace').strip()}")
    return r.stdout
