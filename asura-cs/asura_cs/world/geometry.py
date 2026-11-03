"""Renderer-independent 3D math and mesh building (numpy only, unit-tested).

Render space is right-handed, z up: ``render = (gw.x, gw.y, -gw.z)``.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np

from ..formats.ffna_map import Heightmap, MapBounds


def gw_to_render(x: float, y: float, z: float) -> np.ndarray:
    return np.array([x, y, -z], dtype=float)


def render_to_gw(p) -> tuple[float, float, float]:
    return float(p[0]), float(p[1]), float(-p[2])


# ----------------------------------------------------------------------------------------------
@dataclass
class OrbitCamera:
    """An orbit/fly camera. Angles in degrees; ``pitch`` < 0 looks down."""
    target: np.ndarray
    yaw: float = 45.0
    pitch: float = -35.0
    distance: float = 6000.0
    hfov: float = 60.0
    aspect: float = 16 / 10

    @property
    def vfov(self) -> float:
        return math.degrees(2 * math.atan(math.tan(math.radians(self.hfov) / 2) / self.aspect))

    def forward(self) -> np.ndarray:
        y, p = math.radians(self.yaw), math.radians(self.pitch)
        return np.array([math.cos(p) * math.cos(y), math.cos(p) * math.sin(y), math.sin(p)])

    def position(self) -> np.ndarray:
        return np.asarray(self.target, float) - self.forward() * self.distance

    def basis(self):
        f = self.forward()
        r = np.cross(f, [0, 0, 1.0])
        n = np.linalg.norm(r)
        r = r / n if n > 1e-9 else np.array([1.0, 0, 0])
        u = np.cross(r, f)
        return f, r, u

    def ray(self, nx: float, ny: float):
        """Ray through normalised screen coords (-1..1, y up). Returns (origin, unit direction)."""
        f, r, u = self.basis()
        tx = math.tan(math.radians(self.hfov) / 2)
        ty = math.tan(math.radians(self.vfov) / 2)
        d = f + nx * tx * r + ny * ty * u
        return self.position(), d / np.linalg.norm(d)

    def project(self, p) -> tuple[float, float, float] | None:
        """Render-space point -> (nx, ny, depth); None when behind the camera."""
        f, r, u = self.basis()
        v = np.asarray(p, float) - self.position()
        depth = float(v @ f)
        if depth <= 1e-3:
            return None
        tx = math.tan(math.radians(self.hfov) / 2)
        ty = math.tan(math.radians(self.vfov) / 2)
        return float(v @ r) / (depth * tx), float(v @ u) / (depth * ty), depth

    # interaction
    def orbit(self, dyaw: float, dpitch: float) -> None:
        self.yaw = (self.yaw + dyaw) % 360
        self.pitch = max(-89.0, min(89.0, self.pitch + dpitch))

    def pan(self, dx: float, dy: float) -> None:
        """Pan by screen fractions (of the view width/height)."""
        _, r, u = self.basis()
        scale = self.distance * math.tan(math.radians(self.hfov) / 2) * 2
        self.target = np.asarray(self.target, float) - r * dx * scale + u * dy * scale / self.aspect

    def dolly(self, factor: float) -> None:
        self.distance = max(50.0, min(400000.0, self.distance * factor))

    def fly(self, forward: float, right: float, up: float) -> None:
        f, r, _ = self.basis()
        flat = np.array([f[0], f[1], 0.0])
        n = np.linalg.norm(flat)
        flat = flat / n if n > 1e-9 else flat
        self.target = np.asarray(self.target, float) + flat * forward + r * right + np.array([0, 0, up])


# ----------------------------------------------------------------------------------------------
def ray_ground(origin, direction, heightmap: Heightmap | None, plane_z_render: float = 0.0):
    """Intersect a render-space ray with the terrain (or a flat plane). Returns render point or None."""
    o, d = np.asarray(origin, float), np.asarray(direction, float)
    if heightmap is not None:
        hit = heightmap.raycast(render_to_gw(o), render_to_gw(d) * np.array([1, 1, 1]))
        if hit is not None:
            return gw_to_render(*hit)
    if abs(d[2]) < 1e-9:
        return None
    t = (plane_z_render - o[2]) / d[2]
    return o + d * t if t > 0 else None


def heightmap_from_mesh(vertices_render: np.ndarray, cols: int, rows: int) -> Heightmap | None:
    """Rebuild a regular heightmap from a terrain mesh (e.g. a GWMB export) for picking/ground snap."""
    if vertices_render is None or len(vertices_render) == 0 or cols <= 1 or rows <= 1:
        return None
    v = np.asarray(vertices_render, float)
    mnx, mny = v[:, 0].min(), v[:, 1].min()
    mxx, mxy = v[:, 0].max(), v[:, 1].max()
    if mxx <= mnx or mxy <= mny:
        return None
    b = MapBounds(mnx, mny, mxx, mxy)
    z = np.full((rows, cols), np.nan, np.float32)
    l = np.clip(np.round((v[:, 0] - mnx) / (mxx - mnx) * cols).astype(int), 0, cols - 1)
    k = np.clip(np.round((mxy - v[:, 1]) / (mxy - mny) * rows).astype(int), 0, rows - 1)
    z[k, l] = -v[:, 2]  # back to GW z (negative up)
    if np.isnan(z).any():  # fill holes from the nearest filled cell in the row/column
        mean = np.nanmean(z)
        z = np.where(np.isnan(z), mean, z)
    return Heightmap(b, z)


# ----------------------------------------------------------------------------------------------
# meshes: (vertices (N,3) float32, normals (N,3) float32, colors (N,4) float32, indices (M,) uint32)

def terrain_mesh(hm: Heightmap, max_side: int = 384):
    z = hm.z
    rows, cols = z.shape
    step = max(1, int(math.ceil(max(rows, cols) / max_side)))
    zs = z[::step, ::step]
    r, c = zs.shape
    xs = hm.bounds.min_x + np.arange(c) * hm.dx * step
    ys = hm.bounds.max_y - np.arange(r) * hm.dy * step
    X, Y = np.meshgrid(xs, ys)
    up = -zs.astype(np.float32)
    verts = np.stack([X, Y, up], axis=-1).reshape(-1, 3).astype(np.float32)
    gy, gx = np.gradient(up, hm.dy * step, hm.dx * step)
    # ys decrease with the row index, so d(up)/dy = -gy
    nrm = np.stack([-gx, gy, np.ones_like(up)], axis=-1)
    nrm /= np.linalg.norm(nrm, axis=-1, keepdims=True)
    lo, hi = np.percentile(up, 2), np.percentile(up, 98)
    t = np.clip((up - lo) / max(hi - lo, 1e-3), 0, 1)[..., None]
    low = np.array([0.30, 0.38, 0.24], np.float32)
    high = np.array([0.66, 0.62, 0.50], np.float32)
    rgb = low + (high - low) * t
    if hm.texture_index is not None:
        ti = hm.texture_index[::step, ::step].astype(np.float32)
        rgb = rgb * (0.92 + 0.08 * np.sin(ti * 1.7)[..., None])
    cols4 = np.concatenate([rgb, np.ones_like(t)], axis=-1).reshape(-1, 4).astype(np.float32)
    idx = np.arange(r * c, dtype=np.uint32).reshape(r, c)
    a, b_, cc, d = idx[:-1, :-1], idx[:-1, 1:], idx[1:, :-1], idx[1:, 1:]
    # rows go south, so (a, cc, b) is counter-clockwise seen from above
    tris = np.stack([a, cc, b_, b_, cc, d], axis=-1).reshape(-1).astype(np.uint32)
    return verts, nrm.reshape(-1, 3).astype(np.float32), cols4, tris


def flat_mesh_from_trapezoids(graph, z_render: float = 0.0):
    verts, idx = [], []
    for plane in graph.planes:
        for t in plane.trapezoids:
            base = len(verts)
            for x, y in t.polygon():
                verts.append((x, y, z_render))
            idx += [base, base + 3, base + 1, base + 1, base + 3, base + 2]
    if not verts:
        return None
    v = np.array(verts, np.float32)
    n = np.tile(np.array([0, 0, 1], np.float32), (len(v), 1))
    c = np.tile(np.array([0.35, 0.55, 0.4, 1.0], np.float32), (len(v), 1))
    return v, n, c, np.array(idx, np.uint32)


def box_soup(centers: np.ndarray, sizes: np.ndarray, color=(0.62, 0.62, 0.66, 1.0)):
    """Axis-aligned boxes (render space; centre at the base) merged into one mesh."""
    if len(centers) == 0:
        return None
    unit = np.array([[x, y, z] for z in (0, 1) for y in (-0.5, 0.5) for x in (-0.5, 0.5)], np.float32)
    faces = [(0, 1, 3, 2), (4, 6, 7, 5), (0, 4, 5, 1), (2, 3, 7, 6), (0, 2, 6, 4), (1, 5, 7, 3)]
    normals = [(0, 0, -1), (0, 0, 1), (0, -1, 0), (0, 1, 0), (-1, 0, 0), (1, 0, 0)]
    fv, fn, fi = [], [], []
    for (a, b, c, d), n in zip(faces, normals):
        base = len(fv)
        fv += [unit[a], unit[b], unit[c], unit[d]]
        fn += [n] * 4
        fi += [base, base + 1, base + 2, base, base + 2, base + 3]
    fv, fn, fi = np.array(fv, np.float32), np.array(fn, np.float32), np.array(fi, np.uint32)
    count = len(centers)
    sizes = np.asarray(sizes, np.float32).reshape(count, -1)
    if sizes.shape[1] == 1:
        sizes = np.repeat(sizes, 3, axis=1)
    verts = (fv[None, :, :] * sizes[:, None, :] + np.asarray(centers, np.float32)[:, None, :]).reshape(-1, 3)
    norms = np.tile(fn, (count, 1))
    cols = np.tile(np.array(color, np.float32), (len(verts), 1))
    idx = (fi[None, :] + (np.arange(count, dtype=np.uint32) * len(fv))[:, None]).reshape(-1)
    return verts.astype(np.float32), norms, cols, idx.astype(np.uint32)


def pawn_mesh(radius: float = 45.0, height: float = 190.0, segments: int = 12):
    """A marker 'pawn' standing on the origin with a nose pointing +x (facing)."""
    verts, norms, idx = [], [], []

    def ring(z, r):
        return [(r * math.cos(2 * math.pi * i / segments), r * math.sin(2 * math.pi * i / segments), z)
                for i in range(segments)]

    body_bottom, body_top = ring(0, radius), ring(height * 0.7, radius * 0.75)
    for i in range(segments):
        j = (i + 1) % segments
        base = len(verts)
        quad = [body_bottom[i], body_bottom[j], body_top[j], body_top[i]]
        verts += quad
        ang = 2 * math.pi * (i + 0.5) / segments
        norms += [(math.cos(ang), math.sin(ang), 0.2)] * 4
        idx += [base, base + 1, base + 2, base, base + 2, base + 3]
    # head (cone) and nose
    apex = (0, 0, height)
    for i in range(segments):
        j = (i + 1) % segments
        base = len(verts)
        verts += [body_top[i], body_top[j], apex]
        ang = 2 * math.pi * (i + 0.5) / segments
        norms += [(math.cos(ang), math.sin(ang), 0.7)] * 3
        idx += [base, base + 1, base + 2]
    base = len(verts)
    nz = height * 0.55
    verts += [(radius * 0.6, -radius * 0.35, nz), (radius * 2.2, 0, nz), (radius * 0.6, radius * 0.35, nz),
              (radius * 0.6, 0, nz + radius * 0.4)]
    norms += [(0.3, -0.8, 0.5), (1, 0, 0.3), (0.3, 0.8, 0.5), (0.2, 0, 1)]
    idx += [base, base + 1, base + 3, base + 1, base + 2, base + 3, base, base + 2, base + 1]
    v = np.array(verts, np.float32)
    n = np.array(norms, np.float32)
    n /= np.linalg.norm(n, axis=1, keepdims=True)
    c = np.ones((len(v), 4), np.float32)
    return v, n, c, np.array(idx, np.uint32)


def ring_points(cx: float, cy: float, r: float, heightmap: Heightmap | None, gw_z: float, segments: int = 48):
    """Points of a ground ring (render space) following the terrain if known."""
    pts = []
    for i in range(segments + 1):
        a = 2 * math.pi * i / segments
        x, y = cx + r * math.cos(a), cy + r * math.sin(a)
        z = heightmap.height_at(x, y) if heightmap is not None else gw_z
        pts.append((x, y, -z + 8))
    return pts
