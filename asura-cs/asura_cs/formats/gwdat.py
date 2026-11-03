"""Read-only access to the client archive ``Gw.dat``.

Port of ASURA's ``crates/asura-world/src/gwdat.rs`` (index: header, file table, TOC hash) plus
``fa_decompress`` for entry contents. The archive is **never written**.

Decoding is slow in pure Python, so decoded entries are cached on disk under ``cache_dir`` keyed by
file id and the entry's checksum; the second open of a map is instant.
"""

from __future__ import annotations

import os
import struct
import threading
from dataclasses import dataclass
from pathlib import Path

from .fa_decompress import decompress

MAGIC = 0x1A4E4133
INDEX_TOC = 1


@dataclass(frozen=True, slots=True)
class Entry:
    offset: int
    size: int
    compressed: int
    flags: int
    stream: int
    next_stream: int
    checksum: int


def decode_filename(id0: int, id1: int) -> int:
    """The file id a two-u16 filename reference names (GWMB ``decode_filename``)."""
    return (id0 - 0xFF00FF) + (id1 * 0xFF00)


class GwDat:
    def __init__(self, path: str | Path, cache_dir: str | Path | None = None, native_helper: Path | None = None):
        self.path = Path(path)
        self.native_helper = native_helper
        self.cache_dir = Path(cache_dir) if cache_dir else None
        self._lock = threading.Lock()
        self._fh = open(self.path, "rb")
        head = self._fh.read(32)
        magic, header_size, block_size, checksum, mft_offset, mft_size, flags = struct.unpack("<IIIIQII", head)
        if magic != MAGIC:
            raise ValueError(f"{path} is not a Gw.dat (magic {magic:#010x})")
        self._fh.seek(mft_offset)
        mft_header = self._fh.read(24)
        count = struct.unpack_from("<I", mft_header, 12)[0]
        table = self._fh.read(count * 24)
        self.entries = [Entry(*struct.unpack_from("<QIHBBII", table, i * 24)) for i in range(count)]
        toc = self.entries[INDEX_TOC]
        self._fh.seek(toc.offset)
        toc_bytes = self._fh.read(toc.size)
        self.by_id: dict[int, int] = {}
        for fid, index in struct.iter_unpack("<II", toc_bytes[: len(toc_bytes) // 8 * 8]):
            if index:
                self.by_id[fid] = index - 1  # FaGetFile's decrement (see gwdat.rs)

    def close(self) -> None:
        self._fh.close()

    def __contains__(self, file_id: int) -> bool:
        return file_id in self.by_id

    def entry(self, file_id: int) -> Entry | None:
        i = self.by_id.get(file_id)
        return self.entries[i] if i is not None and i < len(self.entries) else None

    def stored(self, file_id: int) -> bytes | None:
        e = self.entry(file_id)
        if e is None:
            return None
        with self._lock:
            self._fh.seek(e.offset)
            return self._fh.read(e.size)

    def file(self, file_id: int) -> bytes | None:
        e = self.entry(file_id)
        if e is None:
            return None
        cached = self._cache_path(file_id, e)
        if cached and cached.exists():
            return cached.read_bytes()
        raw = self.stored(file_id)
        if e.compressed == 0:
            data = raw
        else:
            if len(raw) < 4:
                raise ValueError(f"entry {file_id:#x} too short to carry its size")
            size = struct.unpack_from("<I", raw, len(raw) - 4)[0]
            data = None
            if self.native_helper is not None:
                from . import native
                try:
                    data = native.decompress(self.native_helper, raw[:-4], size)
                except (OSError, ValueError):
                    data = None  # fall back to the Python port
            if data is None:
                data = decompress(raw[:-4], size)
        if cached:
            cached.parent.mkdir(parents=True, exist_ok=True)
            tmp = cached.with_suffix(".tmp")
            tmp.write_bytes(data)
            os.replace(tmp, cached)
        return data

    def head(self, file_id: int, n: int = 8) -> bytes | None:
        """First bytes of an entry (decompressing it if needed), for type sniffing."""
        data = self.file(file_id)
        return None if data is None else data[:n]

    def _cache_path(self, file_id: int, e: Entry) -> Path | None:
        if not self.cache_dir:
            return None
        return self.cache_dir / "dat" / f"{file_id:08x}_{e.checksum:08x}.bin"
