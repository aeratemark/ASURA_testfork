"""Measure the live movement protocol from `recordings/client/feature/movement/movement-sniff.log`.

For every client message that moves the agent (`0x3D`, `0x3E`, `0x47`), print it next to the
server messages that follow it before the next client message, with every field decoded, and
derive the relationship the server's answer has with the client's report. The point is to state
the live rule as arithmetic, not as an impression: the numbers either close or they do not.
"""

import json
import math
import struct
import sys


def f32(data, at):
    return struct.unpack_from("<f", data, at)[0]


def u32(data, at):
    return struct.unpack_from("<I", data, at)[0]


def load(path):
    records = []
    with open(path, "r", encoding="utf-8") as source:
        for line in source:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            if row.get("record") == "capture":
                continue
            records.append(row)
    return records


def describe(row):
    data = bytes.fromhex(row["data"])
    header = row["header"]
    direction = row["direction"]
    if direction == "CToS" and header == 0x3D:
        return {
            "kind": "manual",
            "pos": (f32(data, 4), f32(data, 8)),
            "plane": u32(data, 12),
            "dir": (f32(data, 16), f32(data, 20)),
            "tail": u32(data, 24),
            "len": len(data),
        }
    if direction == "CToS" and header == 0x3E:
        return {
            "kind": "click",
            "pos": (f32(data, 4), f32(data, 8)),
            "plane": u32(data, 12),
            "len": len(data),
        }
    if direction == "CToS" and header == 0x47:
        return {
            "kind": "cancel",
            "pos": (f32(data, 4), f32(data, 8)),
            "plane": u32(data, 12),
            "len": len(data),
        }
    if direction == "StoC" and header == 0x29:
        return {
            "kind": "move_to_point",
            "agent": u32(data, 4),
            "pos": (f32(data, 8), f32(data, 12)),
            "zero": u32(data, 16),
            "plane": u32(data, 20),
            "len": len(data),
        }
    if direction == "StoC" and header == 0x25:
        return {
            "kind": "direction",
            "agent": u32(data, 4),
            "dir": (f32(data, 8), f32(data, 12)),
            "tail": u32(data, 16),
            "len": len(data),
        }
    if direction == "StoC" and header == 0x2B:
        return {
            "kind": "speed",
            "agent": u32(data, 4),
            "speed": f32(data, 8),
            "mode": u32(data, 12),
            "len": len(data),
        }
    return None


def norm(vec):
    n = math.hypot(vec[0], vec[1])
    return n


def main(path):
    rows = load(path)
    groups = []
    current = []
    for row in rows:
        described = describe(row)
        if described is None:
            continue
        described["t"] = row["t"]
        described["direction"] = row["direction"]
        if described["direction"] == "CToS":
            if current:
                groups.append(current)
            current = [described]
        elif current and current[0]["t"] + 1.5 >= described["t"]:
            current.append(described)
        else:
            if current:
                groups.append(current)
            current = []
    if current:
        groups.append(current)

    print(f"{len(groups)} client move(s) with a server answer within 1.5 s")
    kinds = {}
    for group in groups:
        client = group[0]
        answers = group[1:]
        print("-" * 100)
        print(f"t={client['t']:.3f} client {client['kind']}: {client}")
        for answer in answers:
            print(f"  t={answer['t']:.3f} server {answer['kind']}: {answer}")
        # The arithmetic: does the answer's destination have a constant offset from the report?
        client_pos = client.get("pos")
        for answer in answers:
            if answer["kind"] == "move_to_point" and client_pos is not None:
                dx = answer["pos"][0] - client_pos[0]
                dy = answer["pos"][1] - client_pos[1]
                print(
                    f"  dest - report = ({dx:.3f}, {dy:.3f}), |d| = {norm((dx, dy)):.3f}"
                )
            if answer["kind"] == "direction" and client_pos is not None and "dir" in client:
                print(
                    "  client dir norm = "
                    f"{norm(client['dir']):.6f}, server dir norm = {norm(answer['dir']):.6f}"
                )
        key = (client["kind"], tuple(a["kind"] for a in answers))
        kinds[key] = kinds.get(key, 0) + 1

    print()
    print("answer shapes per client message:")
    for key, count in sorted(kinds.items(), key=lambda item: -item[1]):
        print(f"  {count:5d}  {key[0]} -> {list(key[1])}")


if __name__ == "__main__":
    main(sys.argv[1] if 1 < len(sys.argv) else "recordings/client/feature/movement/movement-sniff.log")
