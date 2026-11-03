"""Dump the zone-door props of every pre-Searing map from your Gw.dat.

    python tools/dump_doors.py                 (uses the Gw.dat set in the editor's Preferences)
    python tools/dump_doors.py "C:/Games/Guild Wars/Gw.dat"

Writes doors_dump.json next to this script and prints a summary. Send the json back: the door
positions decide the missing Regent Valley / Wizard's Folly / Green Hills / Catacombs links.
"""
from __future__ import annotations

import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from asura_cs.content.scope import PRE_SEARING  # noqa: E402
from asura_cs.gamedata import GameData, Settings  # noqa: E402

DOOR_MODELS = {43045}  # the region-door model every known pre-Searing doorway uses


def blob_info(gd: GameData, fid: int) -> dict:
    """What an unreadable map entry actually is: its index record and its first bytes."""
    dat = gd.dat
    out: dict = {"file_id": fid}
    for cand in dict.fromkeys((fid, fid & 0x7FFFFFFF)):
        e = dat.entry(cand)
        if e is None:
            out[str(cand)] = "not in Gw.dat"
            continue
        info = {k: getattr(e, k) for k in ("offset", "size", "compressed") if hasattr(e, k)}
        info["entry"] = repr(e)
        try:
            raw = dat.stored(cand) or b""
            info["stored_head"] = raw[:32].hex(" ")
            data = dat.file(cand) or b""
            info["decoded_len"] = len(data)
            info["decoded_head"] = data[:64].hex(" ")
            info["ffna_at"] = data.find(b"ffna")
        except Exception as err:  # the point is to learn what goes wrong
            info["error"] = repr(err)
        out[str(cand)] = info
    print(f"     blob {fid}: " + json.dumps(out, default=str)[:400])
    return out


def main() -> int:
    settings = Settings.load()
    if len(sys.argv) > 1:
        settings.gw_dat = sys.argv[1]
    if not settings.gw_dat or not Path(settings.gw_dat).is_file():
        print("No Gw.dat: pass its path, or set it in the editor (Edit > Preferences).")
        return 1
    gd = GameData(settings)
    for note in gd.load_asura():
        print(note)
    out: dict = {}
    for map_id in sorted(PRE_SEARING):
        info = gd.catalogue.maps.get(map_id)
        if info is None:
            print(f"{map_id}: not in the ASURA catalogue, skipped")
            continue
        sc = gd.scene(map_id)
        props = sc.ffna.props if sc.ffna else []
        doors = [p for p in props if p.model_file_id in DOOR_MODELS]
        models = Counter(p.model_file_id for p in props)
        out[str(map_id)] = {
            "name": info.name,
            "map_file_id": info.map_file_id,
            "bounds": None if not (sc.ffna and sc.ffna.bounds) else vars(sc.ffna.bounds),
            "doors": [{"x": round(p.x, 1), "y": round(p.y, 1), "z": round(p.z, 1),
                       "yaw": round(p.yaw, 3), "model": p.model_file_id} for p in doors],
            # the rarest models are the best door candidates if 43045 is absent from a map
            "rare_models": [{"model": m, "count": c,
                             "at": [[round(p.x), round(p.y)] for p in props if p.model_file_id == m][:4]}
                            for m, c in sorted(models.items(), key=lambda kv: kv[1]) if c <= 4][:40],
            "notes": sc.notes,
            "blob": blob_info(gd, info.map_file_id) if not props else None,
        }
        print(f"{map_id:4} {info.name:22} {len(props):5} props, {len(doors)} door(s): "
              + ", ".join(f"({d['x']:.0f},{d['y']:.0f})" for d in out[str(map_id)]["doors"]))
    dest = Path(__file__).with_name("doors_dump.json")
    dest.write_text(json.dumps(out, indent=1, default=str))
    print(f"\nWrote {dest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
