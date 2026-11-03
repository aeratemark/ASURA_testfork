"""Print the movement messages around a moment, to read the plane/trap dwords in context."""

import json
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


def main(path, low, high):
    for row in rows(path):
        t = row["t"]
        if not (low <= t <= high):
            continue
        blob = bytes.fromhex(row["data"])
        name = row["direction"]
        header = row["header"]
        if header == 0x29:
            print(
                f"t={t:7.3f} 0x29 agent={u32(blob,4)} pos=({f32(blob,8):.2f},{f32(blob,12):.2f}) "
                f"d4={u32(blob,16)} d5={u32(blob,20)}"
            )
        elif header == 0x25:
            print(
                f"t={t:7.3f} 0x25 agent={u32(blob,4)} dir=({f32(blob,8):.4f},{f32(blob,12):.4f}) "
                f"tail={u32(blob,16)}"
            )
        elif header == 0x2B:
            print(f"t={t:7.3f} 0x2B agent={u32(blob,4)} speed={f32(blob,8)} mode={u32(blob,12)}")
        elif header == 0x3D:
            print(
                f"t={t:7.3f} 0x3D pos=({f32(blob,4):.2f},{f32(blob,8):.2f}) plane={u32(blob,12)} "
                f"dir=({f32(blob,16):.2f},{f32(blob,20):.2f}) tail={u32(blob,24)}"
            )
        elif header == 0x3E:
            print(f"t={t:7.3f} 0x3E click=({f32(blob,4):.2f},{f32(blob,8):.2f}) plane={u32(blob,12)}")
        elif header == 0x47:
            print(f"t={t:7.3f} 0x47 cancel=({f32(blob,4):.2f},{f32(blob,8):.2f}) plane={u32(blob,12)}")


if __name__ == "__main__":
    main(sys.argv[1], float(sys.argv[2]), float(sys.argv[3]))
