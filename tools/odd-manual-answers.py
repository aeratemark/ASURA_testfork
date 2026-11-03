"""Print the manual reports whose server answer is not the usual direction + destination."""

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


def main(path):
    data = rows(path)
    for index, row in enumerate(data):
        if row["direction"] != "CToS" or row["header"] != 0x3D:
            continue
        blob = bytes.fromhex(row["data"])
        following = []
        for later in data[index + 1 : index + 40]:
            if later["direction"] == "CToS":
                break
            if later["header"] not in (0x25, 0x29, 0x2B):
                continue
            if 1.5 < later["t"] - row["t"]:
                break
            other = bytes.fromhex(later["data"])
            if later["header"] == 0x25:
                following.append(f"0x25 dir=({f32(other, 8):.4f},{f32(other, 12):.4f})")
            elif later["header"] == 0x29:
                following.append(
                    f"0x29 ({f32(other, 8):.2f},{f32(other, 12):.2f}) "
                    f"trap={u32(other, 16)} plane={u32(other, 20)}"
                )
            else:
                following.append(f"0x2B speed={f32(other, 8)} mode={u32(other, 12)}")
        if not following or following[0].startswith("0x29"):
            print(
                f"t={row['t']:8.3f} report ({f32(blob, 4):.2f},{f32(blob, 8):.2f}) "
                f"plane={u32(blob, 12)} dir=({f32(blob, 16):.2f},{f32(blob, 20):.2f}) "
                f"tail={u32(blob, 24)}"
            )
            for line in following:
                print(f"    {line}")


if __name__ == "__main__":
    main(sys.argv[1] if 1 < len(sys.argv) else "recordings/client/feature/movement/movement-sniff.log")
