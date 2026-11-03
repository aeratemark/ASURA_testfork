"""Build the *base* profile from what ASURA already knows.

The base profile is a faithful snapshot: maps from ``map_config`` (via the SQL files), their spawn
points from ``map_spawn``, travel portals from ``data/portals.json`` with destinations and landing
points from the measured crossings in ``portals.rs``. No agents, dialogue or quests exist in ASURA
yet, so the base profile has none; that is the honest state, not a placeholder.

Per docs/34 an author copies the base profile and edits the copy; ``copy_profile`` does that.
"""

from __future__ import annotations

import copy
import math

from ..formats.catalogue import Catalogue
from .model import MapContent, Profile, SpawnPoint, TravelPortal
from .scope import SCOPE_NAME, in_scope

CROSSING_MATCH_RADIUS = 600.0  # a recorded crossing point within this of a portal prop is that portal


def map_from_catalogue(info) -> MapContent:
    m = MapContent(info.map_id, info.name, info.map_type, info.map_file_id)
    for i, s in enumerate(info.spawns):
        m.objects.append(SpawnPoint(id=f"spawn_{info.map_id}_{i}", name=f"Spawn {i}", x=s.x, y=s.y,
                                    plane=s.plane, index=i))
    if info.doors:  # ASURA's own map_portal rows: the doors exactly as the world will read them
        for i, d in enumerate(info.doors):
            portal = TravelPortal(id=f"portal_{info.map_id}_{i}", x=d.x, y=d.y, to_map=d.to_map or 0,
                                  measured="measured" in d.source, source=d.source,
                                  name=f"Portal to {d.to_map}" if d.to_map else f"Portal {i}")
            if d.arrive:
                portal.arrive_x, portal.arrive_y = d.arrive
            m.objects.append(portal)
        return m
    used = set()
    for i, p in enumerate(info.portals):
        portal = TravelPortal(id=f"portal_{info.map_id}_{i}", name=f"Portal {i}", x=p.x, y=p.y, z=p.z,
                              model_file_id=p.model_file_id)
        best, best_d = None, CROSSING_MATCH_RADIUS
        for j, c in enumerate(info.crossings):
            d = math.hypot(c.x - p.x, c.y - p.y)
            if c.to_map and j not in used and d < best_d:
                best, best_d = j, d
        if best is not None:
            c = info.crossings[best]
            used.add(best)
            portal.to_map = c.to_map
            portal.measured = True
            if c.arrive:
                portal.arrive_x, portal.arrive_y = c.arrive
            portal.name = f"Portal to {c.to_map}"
        m.objects.append(portal)
    # A crossing measured where no portal prop is listed (Green Hills County and Wizard's Folly have
    # none in data/portals.json) is still a working doorway in ASURA: keep it as one.
    for j, c in enumerate(info.crossings):
        if j in used or not c.to_map:
            continue
        if any(math.hypot(c.x - o.x, c.y - o.y) < 1.0 for o in m.objects if isinstance(o, TravelPortal)):
            continue
        portal = TravelPortal(id=f"portal_{info.map_id}_x{j}", name=f"Portal to {c.to_map}", x=c.x, y=c.y,
                              to_map=c.to_map, measured=True)
        if c.arrive:
            portal.arrive_x, portal.arrive_y = c.arrive
        m.objects.append(portal)
        used.add(j)
    return m


def build_base_profile(catalogue: Catalogue) -> Profile:
    p = Profile(id="base", name=f"Base game, {SCOPE_NAME} (from ASURA)",
                description="Snapshot of ASURA's map catalogue, spawn points and travel portals.")
    for info in catalogue.sorted():
        if not in_scope(info.map_id):
            continue
        if info.map_file_id == 0 and not info.spawns and not info.portals:
            continue
        p.maps[info.map_id] = map_from_catalogue(info)
    return p


def copy_profile(source: Profile, new_id: str, new_name: str) -> Profile:
    """A deep, independent copy (docs/34: no inheritance link back to the original)."""
    p = copy.deepcopy(source)
    p.id, p.name, p.based_on = new_id, new_name, source.id
    return p
