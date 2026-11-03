"""Where the editor's *read-only* game data comes from, and the per-map scene it assembles.

Sources, all optional except the ASURA checkout:

* ASURA checkout - map catalogue (SQL), spawn points, portals, measured crossings, path graphs.
* ``Gw.dat``      - terrain heightmap and prop placements for a map, decoded by ``formats.ffna_map``.
* GWMB export folder(s) - terrain mesh *and real prop meshes* from Guild Wars Map Browser's
  "Export full map" (``map_<filehash>.json`` + ``model_0x..._gwmb.json``).

Nothing here is ever written back. Decoded Gw.dat entries are cached under ``cache_dir``.
"""

from __future__ import annotations

import json
import logging

import numpy as np
from dataclasses import asdict, dataclass, field
from pathlib import Path

from .formats import gwmb_export
from .formats.catalogue import Catalogue
from .formats.ffna_map import FfnaMap, Heightmap, parse_map
from .formats.path_graphs import PathGraph, PathGraphLibrary

log = logging.getLogger(__name__)

SETTINGS_PATH = Path.home() / ".asura_cs" / "settings.json"


@dataclass
class Settings:
    asura_root: str = ""
    gw_dat: str = ""
    gwmb_dirs: list = field(default_factory=list)   # folders containing map_<hash>.json exports
    cache_dir: str = str(Path.home() / ".asura_cs" / "cache")
    last_profile: str = ""
    grid_size: float = 100.0
    snap: bool = False
    angle_snap: float = 15.0

    @classmethod
    def load(cls) -> "Settings":
        try:
            data = json.loads(SETTINGS_PATH.read_text())
            known = {k: v for k, v in data.items() if k in cls.__dataclass_fields__}
            return cls(**known)
        except (OSError, ValueError, TypeError):
            return cls()

    def save(self) -> None:
        SETTINGS_PATH.parent.mkdir(parents=True, exist_ok=True)
        SETTINGS_PATH.write_text(json.dumps(asdict(self), indent=2))


@dataclass
class MapScene:
    """Everything the viewports draw that is *not* profile content (read-only)."""
    map_id: int
    map_file_id: int
    path_graph: PathGraph | None = None
    ffna: FfnaMap | None = None
    gwmb: "gwmb_export.GwmbMap | None" = None
    notes: list = field(default_factory=list)
    _derived_hm: Heightmap | None = None

    @property
    def heightmap(self) -> Heightmap | None:
        """Gw.dat heightmap, else one rebuilt from a GWMB terrain mesh, else None."""
        if self.ffna is not None and self.ffna.heightmap is not None:
            return self.ffna.heightmap
        if self._derived_hm is None and self.gwmb is not None and self.gwmb.terrain is not None:
            from .world.geometry import heightmap_from_mesh
            v = self.gwmb.terrain.vertices
            # GWMB terrain is a regular grid; estimate its resolution from the distinct x values.
            cols = max(2, len(np.unique(np.round(v[:, 0], 1))) - 1)
            rows = max(2, len(np.unique(np.round(v[:, 1], 1))) - 1)
            self._derived_hm = heightmap_from_mesh(v, cols, rows)
        return self._derived_hm

    def ground_z(self, x: float, y: float) -> float | None:
        hm = self.heightmap
        if hm is not None:
            return hm.height_at(x, y)
        return None

    def bounds(self) -> tuple[float, float, float, float]:
        """min_x, min_y, max_x, max_y on the ground plane."""
        if self.ffna and self.ffna.bounds:
            b = self.ffna.bounds
            return b.min_x, b.min_y, b.max_x, b.max_y
        if self.path_graph and self.path_graph.bounds():
            return self.path_graph.bounds()
        if self.gwmb:
            b = self.gwmb.bounds
            return b[0], b[1], b[3], b[4]
        return -10000.0, -10000.0, 10000.0, 10000.0


class GameData:
    def __init__(self, settings: Settings):
        self.settings = settings
        self.catalogue = Catalogue()
        self.path_graphs: PathGraphLibrary | None = None
        self._dat = None
        self._scenes: dict[int, MapScene] = {}

    # -- loading ------------------------------------------------------------------------------
    def load_asura(self) -> list[str]:
        """(Re)read the ASURA checkout. Returns human-readable notes for the log."""
        notes = []
        root = Path(self.settings.asura_root)
        if not root.is_dir():
            return ["ASURA checkout not set (Edit > Preferences)."]
        self.catalogue = Catalogue.from_asura(root)
        notes.append(f"Catalogue: {len(self.catalogue.maps)} maps from {root}")
        pg = root / "data" / "path-graphs.bin"
        if pg.exists():
            self.path_graphs = PathGraphLibrary.load(pg)
            notes.append(f"Path graphs: {len(self.path_graphs.by_file_id)} map blobs")
        self._scenes.clear()
        return notes

    @property
    def dat(self):
        if self._dat is None and self.settings.gw_dat and Path(self.settings.gw_dat).is_file():
            from .formats import native
            from .formats.gwdat import GwDat
            helper = native.helper(self.settings.asura_root, self.settings.cache_dir)
            if helper is None:
                log.warning("No rustc: Gw.dat entries decode with the (slow) Python port; first opens take a while.")
            self._dat = GwDat(self.settings.gw_dat, self.settings.cache_dir, helper)
        return self._dat

    def scene(self, map_id: int, map_file_id: int | None = None) -> MapScene:
        if map_id in self._scenes:
            return self._scenes[map_id]
        info = self.catalogue.maps.get(map_id)
        fid = map_file_id or (info.map_file_id if info else 0)
        sc = MapScene(map_id, fid)
        if self.path_graphs:
            sc.path_graph = self.path_graphs.for_map(map_id, fid)
        if fid and self.dat is not None:
            try:
                blob = self.dat.file(fid)
                if blob:
                    sc.ffna = parse_map(blob, fid)
                    sc.notes.append(f"Gw.dat: terrain {'yes' if sc.ffna.heightmap is not None else 'no'}, "
                                    f"{len(sc.ffna.props)} props")
            except Exception as e:  # keep the editor usable when a decode fails
                sc.notes.append(f"Gw.dat: could not read map blob {fid}: {e}")
                log.exception("map blob %s", fid)
        for d in self.settings.gwmb_dirs:
            if fid and gwmb_export.find_map_json(Path(d), fid):
                try:
                    sc.gwmb = gwmb_export.load_map(Path(d), fid)
                    sc.notes.append(f"GWMB export: {len(sc.gwmb.placements)} model placements from {d}")
                except Exception as e:
                    sc.notes.append(f"GWMB export in {d} unreadable: {e}")
                break
        if not sc.notes:
            sc.notes.append("No terrain source for this map: set Gw.dat or a GWMB export folder in Preferences.")
        self._scenes[map_id] = sc
        return sc
