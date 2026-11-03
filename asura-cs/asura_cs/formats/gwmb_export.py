"""Import a Guild Wars Map Browser "Export full map" folder.

GWMB (Jonathan Bjørn Greve, https://github.com/Jonathan-Greve/GuildWarsMapBrowser) writes, per map:

* ``map_<filehash>.json``   terrain mesh (vertices/indices), prop transforms, bounds
* ``model_0x<HASH>_gwmb.json`` one file per prop model: submodels with vertices and indices
* ``<file_hash>.png`` / ``atlas_<filehash>.png`` textures

This is the richest source of real geometry the editor can show (actual prop meshes), and it needs no
Python port of GW's model format. GWMB works in a left-handed, y-up frame: ``(x, y_up, z) = (gw.x,
-gw.z, gw.y)``. Everything returned here is converted to **render space**, the right-handed z-up frame
the 3D view uses: ``render = (gw.x, gw.y, -gw.z)``.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np


def gwmb_to_render(arr: np.ndarray) -> np.ndarray:
    """(x, y_up, z) GWMB -> (x, y, z_up) render. Works on (..., 3) arrays."""
    return arr[..., [0, 2, 1]]


@dataclass
class Mesh:
    """Triangle soup in render space. ``vertices`` (N,3) float32, ``indices`` (M,) uint32."""
    vertices: np.ndarray
    indices: np.ndarray
    normals: np.ndarray | None = None
    uvs: np.ndarray | None = None
    texture: Path | None = None


@dataclass
class ModelPlacement:
    model_hash: int
    position: np.ndarray  # render space
    basis: np.ndarray     # 3x3, columns: right, up, look in render space (already scaled)


@dataclass
class GwmbMap:
    folder: Path
    filehash: int
    terrain: Mesh | None
    placements: list[ModelPlacement] = field(default_factory=list)
    bounds: tuple[float, float, float, float, float, float] = (0, 0, 0, 0, 0, 0)  # render min xyz, max xyz
    _models: dict[int, list[Mesh] | None] = field(default_factory=dict)

    def model(self, model_hash: int) -> list[Mesh] | None:
        """Submodel meshes of one model in its local (render-axis) space, loaded lazily."""
        if model_hash not in self._models:
            self._models[model_hash] = _load_model(self.folder, model_hash)
        return self._models[model_hash]


def _v3(d) -> list[float]:
    return [d["x"], d["y"], d["z"]]


def find_map_json(folder: Path, filehash: int | None = None) -> Path | None:
    folder = Path(folder)
    if filehash is not None:
        p = folder / f"map_{filehash}.json"
        return p if p.exists() else None
    found = sorted(folder.glob("map_*.json"))
    return found[0] if found else None


def load_map(folder: str | Path, filehash: int | None = None) -> GwmbMap:
    folder = Path(folder)
    path = find_map_json(folder, filehash)
    if path is None:
        raise FileNotFoundError(f"no GWMB map_*.json in {folder}")
    j = json.loads(path.read_text())
    t = j.get("terrain") or {}
    terrain = None
    if t.get("vertices"):
        verts = np.array([_v3(v["pos"]) for v in t["vertices"]], dtype=np.float32)
        norms = np.array([_v3(v["normal"]) for v in t["vertices"]], dtype=np.float32)
        uvs = np.array([[v["uv_coord0"]["x"], v["uv_coord0"]["y"]] for v in t["vertices"]], dtype=np.float32)
        atlas = folder / f"atlas_{j['filehash']}.png"
        terrain = Mesh(gwmb_to_render(verts), np.array(t["indices"], dtype=np.uint32),
                       gwmb_to_render(norms), uvs, atlas if atlas.exists() else None)
    placements = []
    for m in j.get("models", []):
        right = np.array(_v3(m["model_right"]))
        up = np.array(_v3(m["model_up"]))
        look = np.array(_v3(m["model_look"]))
        scale = float(m.get("scale", 1.0))
        # local GWMB vertex v -> world GWMB: pos + s*(vx*right + vy*up + vz*look); then axis swap.
        basis_gwmb = np.stack([right, up, look], axis=1) * scale
        basis = basis_gwmb[[0, 2, 1], :]  # rows to render axes; columns still act on local GWMB coords
        placements.append(ModelPlacement(int(m["model_hash"]), gwmb_to_render(np.array(_v3(m["world_pos"]))),
                                         basis))
    # GWMB y is up already (it negated GW z), so render z = gwmb y:
    b = (j.get("min_x", 0), j.get("min_z", 0), j.get("min_y", 0), j.get("max_x", 0), j.get("max_z", 0),
         j.get("max_y", 0))
    return GwmbMap(folder, int(j["filehash"]), terrain, placements, b)


def _load_model(folder: Path, model_hash: int) -> list[Mesh] | None:
    path = folder / f"model_0x{model_hash:X}_gwmb.json"
    if not path.exists():
        return None
    j = json.loads(path.read_text())
    textures = j.get("textures", [])
    meshes = []
    for sub in j.get("submodels", []):
        if not sub.get("vertices") or not sub.get("indices"):
            continue
        # Keep local coordinates in GWMB axes; ModelPlacement.basis maps them to render space.
        verts = np.array([_v3(v["pos"]) for v in sub["vertices"]], dtype=np.float32)
        uvs = None
        tex = None
        uv_lists = [v.get("texture_uv_coords") or [] for v in sub["vertices"]]
        if all(uv_lists):
            uvs = np.array([[u[0]["x"], u[0]["y"]] for u in uv_lists], dtype=np.float32)
        ti = sub.get("texture_indices") or []
        if ti and ti[0] < len(textures):
            png = folder / f"{textures[ti[0]]['file_hash']}.png"
            tex = png if png.exists() else None
        meshes.append(Mesh(verts, np.array(sub["indices"], dtype=np.uint32), None, uvs, tex))
    return meshes
