"""Measure the live client's own walking speed from its position reports, and the live server's
destination cadence, from `recordings/client/feature/movement/movement-sniff.log`.

Two numbers matter for the server's own copy of an agent:
  * the speed the client actually walks at (u/s), from consecutive 0x3D reports;
  * how far the live server's announced destinations are from the report that preceded them.
"""

import json
import math
import struct

PATH = "recordings/client/feature/movement/movement-sniff.log"


def f32(blob, at):
    return struct.unpack_from("<f", blob, at)[0]


def u32(blob, at):
    return struct.unpack_from("<I", blob, at)[0]


rows = []
with open(PATH, encoding="utf-8") as source:
    for line in source:
        line = line.strip()
        if not line:
            continue
        row = json.loads(line)
        if row.get("record") != "capture":
            rows.append(row)

# 1. The client's own walking speed, from its manual reports.
previous = None
speeds = []
for row in rows:
    if row["direction"] != "CToS" or row["header"] != 0x3D:
        continue
    blob = bytes.fromhex(row["data"])
    position = (f32(blob, 4), f32(blob, 8))
    if previous is not None:
        elapsed = row["t"] - previous[1]
        if elapsed > 0:
            distance = math.dist(position, previous[0])
            speeds.append((distance / elapsed, distance, elapsed))
    previous = (position, row["t"])

if speeds:
    values = sorted(speed[0] for speed in speeds)
    print(f"client speed over {len(values)} report gaps (u/s):")
    print(f"  min {values[0]:.1f}   median {values[len(values) // 2]:.1f}   max {values[-1]:.1f}")
    print("  samples:", ", ".join(f"{v:.0f}" for v in values[:20]))

# 2. How often the live server announces a destination for the player agent, and how the
#    announced destination relates to the report that came before it.
announcements = [row for row in rows if row["direction"] == "StoC" and row["header"] == 0x29]
player_agent = None
if announcements:
    player_agent = u32(bytes.fromhex(announcements[0]["data"]), 4)
mine = [
    row for row in announcements if u32(bytes.fromhex(row["data"]), 4) == player_agent
]
print(f"\nannounced destinations: {len(announcements)} total, {len(mine)} for agent {player_agent}")
gaps = [
    mine[index]["t"] - mine[index - 1]["t"] for index in range(1, len(mine))
]
if gaps:
    ordered = sorted(gaps)
    print(
        f"  spacing between them (s): min {ordered[0]:.2f} median {ordered[len(ordered) // 2]:.2f} "
        f"max {ordered[-1]:.2f}"
    )

# 3. Between two announcements, how far apart are the two destinations?
steps = []
for index in range(1, len(mine)):
    before = bytes.fromhex(mine[index - 1]["data"])
    after = bytes.fromhex(mine[index]["data"])
    steps.append(
        (
            mine[index]["t"] - mine[index - 1]["t"],
            math.dist((f32(before, 8), f32(before, 12)), (f32(after, 8), f32(after, 12))),
        )
    )
print("\n  first ten (gap s, distance between destinations):")
for gap, distance in steps[:10]:
    speed = distance / gap if gap > 0 else 0
    print(f"    {gap:5.2f}s  {distance:8.1f}u  -> {speed:6.1f} u/s")
