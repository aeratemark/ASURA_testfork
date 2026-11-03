"""FFNA map blobs: terrain heightmap, map bounds and prop placements.

Chunk layouts follow Guild Wars Map Browser by Jonathan Bjørn Greve
(https://github.com/Jonathan-Greve/GuildWarsMapBrowser, ``SourceFiles/FFNA_MapFile.h`` and
``Terrain.cpp``), used under its license with attribution. Container walk as in ASURA's
``map_data.rs``: ``"ffna"``, a type byte, then ``(chunk_id u32, size u32, bytes)`` records.

Everything here is returned in **GW world coordinates**: x/y on the ground plane (the same plane the
path graph and spawn points use), z vertical with *negative = up*, exactly as the game stores it.
"""

from __future__ import annotations

import math
import struct
from dataclasses import dataclass, field

import numpy as np

from .gwdat import decode_filename

CHUNK_MAP_HEADER = 0x20000000
CHUNK_TERRAIN = 0x20000002
CHUNK_PROPS_INFO = 0x20000004
CHUNK_PATH = 0x20000008
CHUNK_MAP_INFO = 0x2000000C
CHUNK_TERRAIN_FILENAMES = 0x21000002
CHUNK_PROPS_FILENAMES0 = 0x21000003
CHUNK_PROPS_FILENAMES = 0x21000004

TERRAIN_TILE = 32


@dataclass
class MapBounds:
    min_x: float
    min_y: float
    max_x: float
    max_y: float


@dataclass
class Heightmap:
    """Terrain heights on a regular grid, rows from north (max y) to south.

    ``z[k, l]`` is the GW height (negative = up) at ``x = min_x + l*dx``, ``y = max_y - k*dy``.
    """
    bounds: MapBounds
    z: np.ndarray
    texture_index: np.ndarray | None = None

    @property
    def dx(self) -> float:
        return (self.bounds.max_x - self.bounds.min_x) / self.z.shape[1]

    @property
    def dy(self) -> float:
        return (self.bounds.max_y - self.bounds.min_y) / self.z.shape[0]

    def height_at(self, x: float, y: float) -> float:
        """Bilinear GW height at a ground position (clamped to the grid)."""
        rows, cols = self.z.shape
        gl = (x - self.bounds.min_x) / self.dx
        gk = (self.bounds.max_y - y) / self.dy
        l0 = int(min(max(math.floor(gl), 0), cols - 2))
        k0 = int(min(max(math.floor(gk), 0), rows - 2))
        fl = min(max(gl - l0, 0.0), 1.0)
        fk = min(max(gk - k0, 0.0), 1.0)
        z = self.z
        top = z[k0, l0] * (1 - fl) + z[k0, l0 + 1] * fl
        bot = z[k0 + 1, l0] * (1 - fl) + z[k0 + 1, l0 + 1] * fl
        return float(top * (1 - fk) + bot * fk)

    def grid_xy(self) -> tuple[np.ndarray, np.ndarray]:
        rows, cols = self.z.shape
        xs = self.bounds.min_x + np.arange(cols) * self.dx
        ys = self.bounds.max_y - np.arange(rows) * self.dy
        return xs, ys

    def raycast(self, origin, direction, max_dist: float = 200000.0, step: float | None = None):
        """First hit of a ray with the terrain, in GW coords. Returns (x, y, z) or None.

        Marches the ray and refines by bisection; good enough for picking and dragging.
        """
        o = np.asarray(origin, dtype=float)
        d = np.asarray(direction, dtype=float)
        n = np.linalg.norm(d)
        if n == 0:
            return None
        d = d / n
        step = step or max(min(self.dx, self.dy) * 0.5, 1.0)

        def above(p):  # GW z negative is up: a point is above ground when p.z < ground z
            return p[2] < self.height_at(p[0], p[1])

        t = 0.0
        if not above(o):
            return None
        while t < max_dist:
            t2 = t + step
            p = o + d * t2
            if not self._inside(p[0], p[1]) and t2 > step * 4 and not self._approaching(o, d, t2):
                return None
            if not above(p):
                lo, hi = t, t2
                for _ in range(20):
                    mid = (lo + hi) / 2
                    if above(o + d * mid):
                        lo = mid
                    else:
                        hi = mid
                hit = o + d * hi
                return float(hit[0]), float(hit[1]), self.height_at(hit[0], hit[1])
            t = t2
            step *= 1.01  # coarser far away
        return None

    def _inside(self, x, y) -> bool:
        b = self.bounds
        return b.min_x <= x <= b.max_x and b.min_y <= y <= b.max_y

    def _approaching(self, o, d, t) -> bool:
        b = self.bounds
        cx, cy = (b.min_x + b.max_x) / 2, (b.min_y + b.max_y) / 2
        p = o + d * t
        return (p[0] - cx) * d[0] + (p[1] - cy) * d[1] < 0


@dataclass
class PropPlacement:
    filename_index: int
    model_file_id: int | None
    x: float
    y: float
    z: float
    up: tuple[float, float, float]
    look: tuple[float, float, float]
    scale: float

    @property
    def yaw(self) -> float:
        """Facing angle on the ground plane, radians, measured from +x towards +y."""
        return math.atan2(self.look[1], self.look[0])


@dataclass
class FfnaMap:
    file_id: int
    chunks: dict[int, tuple[int, int]] = field(default_factory=dict)  # id -> (offset of payload, size)
    bounds: MapBounds | None = None
    heightmap: Heightmap | None = None
    prop_filenames: list[int] = field(default_factory=list)
    terrain_texture_filenames: list[int] = field(default_factory=list)
    props: list[PropPlacement] = field(default_factory=list)


def walk_chunks(data: bytes) -> dict[int, tuple[int, int]]:
    if data[:4] != b"ffna":
        raise ValueError("not an FFNA blob")
    chunks: dict[int, tuple[int, int]] = {}
    at = 5
    while at + 8 <= len(data):
        cid, size = struct.unpack_from("<II", data, at)
        if at + 8 + size > len(data):
            break
        chunks.setdefault(cid, (at + 8, size))
        at += 8 + size
    return chunks


def _filenames(data: bytes, payload: int, size: int) -> list[int]:
    count = (size - 5) // 6
    out = []
    at = payload + 5
    for _ in range(count):
        id0, id1, _f1 = struct.unpack_from("<HHH", data, at)
        out.append(decode_filename(id0, id1))
        at += 6
    return out


def parse_map(data: bytes, file_id: int = 0) -> FfnaMap:
    m = FfnaMap(file_id, walk_chunks(data))

    if CHUNK_MAP_INFO in m.chunks:
        p, _ = m.chunks[CHUNK_MAP_INFO]
        # payload: magic u32, always_2 u8, then min_x, min_y, max_x, max_y (GWMB MapBounds at +13 of chunk)
        mnx, mny, mxx, mxy = struct.unpack_from("<4f", data, p + 5)
        m.bounds = MapBounds(mnx, mny, mxx, mxy)

    for cid, target in ((CHUNK_PROPS_FILENAMES, "prop"), (CHUNK_PROPS_FILENAMES0, "prop"),
                        (CHUNK_TERRAIN_FILENAMES, "terrain")):
        if cid in m.chunks:
            names = _filenames(data, *m.chunks[cid])
            if target == "prop":
                m.prop_filenames.extend(names)
            else:
                m.terrain_texture_filenames = names

    if CHUNK_TERRAIN in m.chunks and m.bounds:
        m.heightmap = _parse_terrain(data, *m.chunks[CHUNK_TERRAIN], m.bounds)

    if CHUNK_PROPS_INFO in m.chunks:
        m.props = _parse_props(data, *m.chunks[CHUNK_PROPS_INFO], m.prop_filenames)
    return m


def _parse_terrain(data: bytes, p: int, size: int, bounds: MapBounds) -> Heightmap | None:
    at = p + 4 + 4 + 1 + 4  # magic, magic1, tag, some_size
    xdim, ydim = struct.unpack_from("<II", data, at)
    at += 8 + 4 + 4 + 2 + 4 + 4 + 1 + 4  # floats, u16, floats, tag1, height byte size
    n = xdim * ydim
    if xdim == 0 or ydim == 0 or at + n * 4 > p + size:
        return None
    raw = np.frombuffer(data, dtype="<f4", count=n, offset=at).astype(np.float32)
    at += n * 4
    tex = None
    try:
        at += 1  # tag2
        ntiles = struct.unpack_from("<I", data, at)[0]
        at += 4
        if ntiles == n:
            tex = np.frombuffer(data, dtype=np.uint8, count=n, offset=at)
    except struct.error:
        tex = None

    def untile(flat: np.ndarray) -> np.ndarray:
        # Stored as 32x32 tiles, tiles row-major, rows top (north) first within a tile.
        if xdim % TERRAIN_TILE or ydim % TERRAIN_TILE:
            return flat.reshape(ydim, xdim)
        tr, tc = ydim // TERRAIN_TILE, xdim // TERRAIN_TILE
        return (flat.reshape(tr, tc, TERRAIN_TILE, TERRAIN_TILE)
                .transpose(0, 2, 1, 3).reshape(ydim, xdim))

    return Heightmap(bounds, untile(raw), untile(tex) if tex is not None else None)


def _parse_props(data: bytes, p: int, size: int, filenames: list[int]) -> list[PropPlacement]:
    at = p + 4 + 2  # magic u32, magic2 u16
    array_bytes = struct.unpack_from("<I", data, at)[0]
    at += 4
    end = min(at + array_bytes, p + size)
    count = struct.unpack_from("<H", data, at)[0]
    at += 2
    props = []
    for _ in range(count):
        if at + 48 > end:
            break
        idx = struct.unpack_from("<H", data, at)[0]
        f = struct.unpack_from("<11f", data, at + 2)
        nsub = data[at + 47]
        x, y, z, u0, u1, u2, sa, ca, f9, scale, _f11 = f
        props.append(PropPlacement(
            filename_index=idx,
            model_file_id=filenames[idx] if idx < len(filenames) else None,
            x=x, y=y, z=z, up=(u0, u1, u2), look=(sa, ca, f9), scale=scale))
        at += 48 + nsub * 8
    return props
