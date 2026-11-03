"""Pin down the live server's arithmetic for the manual-walk answer (0x3D -> 0x25 + 0x29).

For each manual report followed by a direction and a destination, print:
  * r = |dest - report| / |dir|            (is the destination the report scaled by a constant?)
  * |unit_from_normalize(dir) - server_unit|   (is the direction the exact normalisation?)
  * |unit|                                  (how far from unit length the server's direction is)
  * angle between server_unit and normalize(dir), and between server_unit and normalize(dest-report)
"""

import json
import math
import struct
import sys


def f32(data, at):
    return struct.unpack_from("<f", data, at)[0]


def u32(data, at):
    return struct.unpack_from("<I", data, at)[0]


def rows(path):
    out = []
    with open(path, "r", encoding="utf-8") as source:
        for line in source:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            if row.get("record") != "capture":
                out.append(row)
    return out


def norm(v):
    return math.hypot(v[0], v[1])


def unit(v):
    n = norm(v)
    return (v[0] / n, v[1] / n)


def main(path):
    data = rows(path)
    manual = None
    printed = 0
    ratios = []
    unit_errors = []
    angles_norm = []
    angles_dest = []
    for row in data:
        blob = bytes.fromhex(row["data"])
        if row["direction"] == "CToS" and row["header"] == 0x3D:
            manual = {
                "t": row["t"],
                "pos": (f32(blob, 4), f32(blob, 8)),
                "plane": u32(blob, 12),
                "dir": (f32(blob, 16), f32(blob, 20)),
            }
            answer = {}
            continue
        if manual is None:
            continue
        if row["direction"] == "StoC" and row["header"] == 0x25:
            answer = {
                "unit": (f32(blob, 8), f32(blob, 12)),
                "t": row["t"],
            }
        elif row["direction"] == "StoC" and row["header"] == 0x29 and answer:
            dest = (f32(blob, 8), f32(blob, 12))
            d = (dest[0] - manual["pos"][0], dest[1] - manual["pos"][1])
            r = norm(d) / norm(manual["dir"])
            expected = unit(manual["dir"])
            err = norm((answer["unit"][0] - expected[0], answer["unit"][1] - expected[1]))
            angle_norm = math.acos(
                max(-1.0, min(1.0, (answer["unit"][0] * expected[0] + answer["unit"][1] * expected[1])))
            )
            dest_unit = unit(d)
            angle_dest = math.acos(
                max(-1.0, min(1.0, (answer["unit"][0] * dest_unit[0] + answer["unit"][1] * dest_unit[1])))
            )
            ratios.append(r)
            unit_errors.append(err)
            angles_norm.append(angle_norm)
            angles_dest.append(angle_dest)
            print(
                f"t={manual['t']:8.3f} |dir|={norm(manual['dir']):9.3f} |d|={norm(d):9.3f} "
                f"r={r:.6f} |unit|={norm(answer['unit']):.6f} "
                f"err_vs_normalize={err:.6f} angle_vs_normalize={math.degrees(angle_norm):.4f}deg "
                f"angle_vs_dest={math.degrees(angle_dest):.4f}deg"
            )
            printed += 1
            answer = {}
            if printed > 200:
                break
    print()
    print(f"{len(ratios)} samples")
    if ratios:
        print(f"r: min {min(ratios):.6f}  max {max(ratios):.6f}")
    if unit_errors:
        print(f"err_vs_normalize: min {min(unit_errors):.6f}  max {max(unit_errors):.6f}")
    if angles_norm:
        print(
            f"angle_vs_normalize: min {math.degrees(min(angles_norm)):.4f}deg "
            f"max {math.degrees(max(angles_norm)):.4f}deg"
        )
    if angles_dest:
        print(
            f"angle_vs_dest: min {math.degrees(min(angles_dest)):.4f}deg "
            f"max {math.degrees(max(angles_dest)):.4f}deg"
        )


if __name__ == "__main__":
    main(sys.argv[1] if 1 < len(sys.argv) else "recordings/client/feature/movement/movement-sniff.log")
