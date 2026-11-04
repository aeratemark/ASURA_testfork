"""Dump every travel-portal prop of the pre-Searing maps straight from your Gw.dat.

    python tools/dump_portals.py                 (uses the Gw.dat set in the editor's Preferences)
    python tools/dump_portals.py "C:/Games/Guild Wars/Gw.dat"

Writes portals_dump.json next to this script and prints every portal per map.

Why a new script: the catalogue's map_file_id for most pre-Searing maps points at the post-Searing
blobs (3806xx/3807xx), which do not parse. The pre-Searing blobs below are the ones GWMB names its
exports after (map_<id>.json) and that its .dat browser lists as Lakeside / Catacombs / Northlands /
Barradin. A portal is the green arch prop; GWMB shows it as file hash 0xA825 (43045), and 0x858B
(34187) is the identical mesh.
"""
from __future__ import annotations

import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from asura_cs.formats.ffna_map import parse_map  # noqa: E402
from asura_cs.gamedata import GameData, Settings  # noqa: E402

PORTAL_MODELS = {0xA825, 0x858B}
# Some exits have no arch, only a sunray effect: the Catacombs' exit to Green Hills County is model
# 0x3BB1 (15281) at (-8246, 15074). Listed separately, as the same effect may also be plain decoration.
SUNRAY_MODELS = {0x3BB1}

# map_id -> (name, pre-Searing map blob). Maps sharing a blob share one terrain and its portals.
PRE_SEARING_BLOBS = {
    145: ("The Catacombs", 116016),
    146: ("Lakeside County", 113021),
    147: ("The Northlands", 116025),
    148: ("Ascalon City", 113021),
    160: ("Green Hills County", 113190),
    161: ("Wizard's Folly", 113355),
    162: ("Regent Valley", 113437),
    163: ("The Barradin Estate", 113190),
    164: ("Ashford Abbey", 113021),
    165: ("Foible's Fair", 113355),
    166: ("Fort Ranik", 113437),
}


def main() -> int:
    settings = Settings.load()
    if len(sys.argv) > 1:
        settings.gw_dat = sys.argv[1]
    if not settings.gw_dat or not Path(settings.gw_dat).is_file():
        print("No Gw.dat: pass its path, or set it in the editor (Edit > Preferences).")
        return 1
    gd = GameData(settings)
    out: dict = {}
    done: dict[int, dict] = {}
    for map_id, (name, blob) in PRE_SEARING_BLOBS.items():
        if blob not in done:
            # Straight from Gw.dat: GameData.scene() would also parse a 300 MB GWMB export per map.
            ffna, notes = None, []
            try:
                data = gd.dat.file(blob)
                ffna = parse_map(data, blob) if data else None
                notes.append("read" if ffna else "blob not in Gw.dat")
            except Exception as err:
                notes.append(f"could not read blob {blob}: {err!r}")
            props = ffna.props if ffna else []
            portals = [p for p in props if p.model_file_id in PORTAL_MODELS]
            done[blob] = {
                "props": len(props),
                "bounds": None if not (ffna and ffna.bounds) else vars(ffna.bounds),
                "portals": [{"x": round(p.x, 1), "y": round(p.y, 1), "z": round(p.z, 1),
                             "yaw": round(p.yaw, 3),
                             "scale": round(p.scale, 3), "model": p.model_file_id} for p in portals],
                "sunrays": [{"x": round(p.x, 1), "y": round(p.y, 1), "z": round(p.z, 1), "model": p.model_file_id}
                            for p in props if p.model_file_id in SUNRAY_MODELS],
                # In case a map uses another portal mesh: models that appear only a handful of times.
                "rare_models": sorted(((m, c) for m, c in Counter(p.model_file_id for p in props).items()
                                       if c <= 3), key=lambda mc: mc[1])[:30],
                "notes": notes,
            }
        info = done[blob]
        out[str(map_id)] = {"name": name, "map_file_id": blob, **info}
        print(f"{map_id:4} {name:20} blob {blob:6}  {info['props']:5} props, {len(info['portals'])} portal(s): "
              + ", ".join(f"({p['x']:.0f},{p['y']:.0f})" for p in info["portals"]))
        if info["sunrays"]:
            print("      sunrays: " + ", ".join(f"({p['x']:.0f},{p['y']:.0f})" for p in info["sunrays"]))
        if not info["props"]:
            print("      " + "; ".join(info["notes"]))
    dest = Path(__file__).with_name("portals_dump.json")
    dest.write_text(json.dumps(out, indent=1, default=str))
    print(f"\nWrote {dest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
