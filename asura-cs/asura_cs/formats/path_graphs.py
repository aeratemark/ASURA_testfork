"""Reader for ASURA's portable path-graph dump (``data/path-graphs.bin``, magic ``ASURAPG1``).

Mirror of ``crates/asura-world/src/graph_dump.rs`` (``decode``). The dump carries every plane of a
map's path graph: trapezoids with neighbour and portal ids, the x/y BSP nodes, sink nodes and the
plane-to-plane portals. That is the *walkable* surface of the map, in GW world units on the x/y plane,
which makes it the first real geometry the editor can show without a ``Gw.dat``.

Only what the editor uses is kept as objects (trapezoids, adjacency, portals); the BSP nodes are parsed
past and their counts kept, so a round trip of the format is not a goal of this module.
"""

from __future__ import annotations

import struct
from dataclasses import dataclass, field
from pathlib import Path

MAGIC = b"ASURAPG1"
VERSION = 1
NONE = 0xFFFFFFFF


@dataclass(slots=True)
class Trapezoid:
    trap_id: int
    top_left: int | None
    top_right: int | None
    bottom_left: int | None
    bottom_right: int | None
    portal_left: int
    portal_right: int
    yt: float
    yb: float
    xtl: float
    xtr: float
    xbl: float
    xbr: float

    def polygon(self) -> list[tuple[float, float]]:
        """The four corners in GW world x/y, clockwise from top-left."""
        return [(self.xtl, self.yt), (self.xtr, self.yt), (self.xbr, self.yb), (self.xbl, self.yb)]

    def center(self) -> tuple[float, float]:
        return ((self.xtl + self.xtr + self.xbl + self.xbr) / 4.0, (self.yt + self.yb) / 2.0)

    def neighbours(self) -> list[int]:
        return [n for n in (self.top_left, self.top_right, self.bottom_left, self.bottom_right) if n is not None]

    def contains(self, x: float, y: float) -> bool:
        if not (self.yb <= y <= self.yt):
            return False
        span = self.yt - self.yb
        t = 0.0 if span == 0 else (y - self.yb) / span
        left = self.xbl + (self.xtl - self.xbl) * t
        right = self.xbr + (self.xtr - self.xbr) * t
        return left <= x <= right


@dataclass(slots=True)
class PlanePortal:
    portal_plane_id: int
    neighbor_plane_id: int
    flags: int
    pair: tuple[int, int] | None
    traps_count: int
    traps: int


@dataclass(slots=True)
class Plane:
    plane_id: int
    trapezoids: list[Trapezoid]
    portals: list[PlanePortal]
    portals_traps: list[int | None]
    node_counts: tuple[int, int, int] = (0, 0, 0)  # x, y, sink

    def bounds(self) -> tuple[float, float, float, float] | None:
        if not self.trapezoids:
            return None
        xs = [v for t in self.trapezoids for v in (t.xtl, t.xtr, t.xbl, t.xbr)]
        ys = [v for t in self.trapezoids for v in (t.yt, t.yb)]
        return min(xs), min(ys), max(xs), max(ys)


@dataclass(slots=True)
class PathGraph:
    map_id: int
    map_file_id: int
    traps_count: int
    planes: list[Plane] = field(default_factory=list)

    def bounds(self) -> tuple[float, float, float, float] | None:
        boxes = [b for b in (p.bounds() for p in self.planes) if b]
        if not boxes:
            return None
        return (min(b[0] for b in boxes), min(b[1] for b in boxes),
                max(b[2] for b in boxes), max(b[3] for b in boxes))

    def find_trapezoid(self, x: float, y: float) -> tuple[int, Trapezoid] | None:
        """Linear search (the editor's use is interactive, not per-tick)."""
        for plane in self.planes:
            for trap in plane.trapezoids:
                if trap.contains(x, y):
                    return plane.plane_id, trap
        return None


class _Cursor:
    __slots__ = ("buf", "at")

    def __init__(self, buf: bytes):
        self.buf = buf
        self.at = 0

    def take(self, fmt: str):
        size = struct.calcsize(fmt)
        if self.at + size > len(self.buf):
            raise ValueError(f"path-graph dump truncated at byte {self.at}")
        value = struct.unpack_from(fmt, self.buf, self.at)
        self.at += size
        return value

    def u8(self) -> int: return self.take("<B")[0]
    def u16(self) -> int: return self.take("<H")[0]
    def u32(self) -> int: return self.take("<I")[0]
    def i32(self) -> int: return self.take("<i")[0]

    def skip(self, n: int) -> None:
        if self.at + n > len(self.buf):
            raise ValueError(f"path-graph dump truncated at byte {self.at}")
        self.at += n

    def node_ref(self) -> None:
        if self.u8() != 0:
            self.skip(4)


def _opt(v: int) -> int | None:
    return None if v == NONE else v


_TRAP = struct.Struct("<IIIIIHH6f")


def decode(buf: bytes) -> list[PathGraph]:
    if buf[:8] != MAGIC:
        raise ValueError("not an ASURA path-graph dump (bad magic)")
    c = _Cursor(buf)
    c.at = 8
    version = c.u32()
    if version != VERSION:
        raise ValueError(f"path-graph dump version {version}, this reader knows {VERSION}")
    graphs: list[PathGraph] = []
    for _ in range(c.u32()):
        map_id, map_file_id, traps_count, plane_count = c.take("<iIII")
        graph = PathGraph(map_id, map_file_id, traps_count)
        for _ in range(plane_count):
            plane_id = c.u16()
            c.skip(c.u32() * 8)  # vectors
            traps: list[Trapezoid] = []
            for _ in range(c.u32()):
                if c.at + _TRAP.size > len(buf):
                    raise ValueError("path-graph dump truncated in trapezoids")
                tid, tl, tr, bl, br, pl, pr, yt, yb, xtl, xtr, xbl, xbr = _TRAP.unpack_from(buf, c.at)
                c.at += _TRAP.size
                traps.append(Trapezoid(tid, _opt(tl), _opt(tr), _opt(bl), _opt(br), pl, pr,
                                       yt, yb, xtl, xtr, xbl, xbr))
            nx = c.u32()
            for _ in range(nx):
                c.skip(16)
                c.node_ref()
                c.node_ref()
            ny = c.u32()
            for _ in range(ny):
                c.skip(8)
                c.node_ref()
                c.node_ref()
            ns = c.u32()
            c.skip(ns * 4)
            portals: list[PlanePortal] = []
            for _ in range(c.u32()):
                pp, npid, flags = c.take("<HHB")
                tag = c.u8()
                if tag == 0:
                    pair = None
                elif tag == 1:
                    pair = (c.u16(), c.u32())
                else:
                    raise ValueError(f"portal pair tag {tag}")
                tcount, tfirst = c.take("<II")
                portals.append(PlanePortal(pp, npid, flags, pair, tcount, tfirst))
            c.u32()  # h000c
            ptraps = [_opt(c.u32()) for _ in range(c.u32())]
            c.node_ref()  # root
            graph.planes.append(Plane(plane_id, traps, portals, ptraps, (nx, ny, ns)))
        graphs.append(graph)
    return graphs


class PathGraphLibrary:
    """All graphs of a dump, keyed by Gw.dat blob id (several maps share one blob)."""

    def __init__(self, graphs: list[PathGraph]):
        self.by_file_id: dict[int, PathGraph] = {g.map_file_id: g for g in graphs}
        self.by_map_id: dict[int, PathGraph] = {g.map_id: g for g in graphs if g.map_id >= 0}

    @classmethod
    def load(cls, path: str | Path) -> "PathGraphLibrary":
        return cls(decode(Path(path).read_bytes()))

    def for_map(self, map_id: int, map_file_id: int | None = None) -> PathGraph | None:
        if map_file_id is not None and map_file_id in self.by_file_id:
            return self.by_file_id[map_file_id]
        return self.by_map_id.get(map_id)
