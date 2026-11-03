# 11 - The state of parity, measured

> **History.** The tooling named below was retired when parity closed (docs/16); nothing here is runnable.

Produced by `tools/parity-summary.py`, which is gone from the tree. Every number here is the
**2026-10-31 reading** and cannot be re-derived: the generators that measured the axes and the fixtures
some of them compared against were retired with the parity tooling (`docs/16`). Where a number here
disagrees with a test or a file that still exists, that test or file is right.

## The axes

| axis | what it measures | state |
|---|---|---|
| dispatch | the reference's client-message handlers | auth **15/15**, game **18/18** - **complete**; 1 of them are no-ops in the reference itself |
| emit | every message the reference can send | **76 of 81** reachable, **0 real gap(s)**, 5 explained |
| the load block | `(header, size)` against the reference's own trace | **480 of 480** matched, **0 missing**, **0 extra**, **0 size** disagreement(s) |
| the load block's bytes | the same messages, payload against payload | **91 of 480 message(s) are byte-identical to the reference; 389 allowed difference(s) in 3 message(s), each with a recorded reason** |
| generated files | `--check` against the reference's sources | **12 of 12 current** |
| the map data | `Gw.dat` through the ported importer, against the reference's own tool | **the importer matches the reference on 7 blob(s): 283 plane(s), 34806 trapezoid(s), same bounding boxes** |
| the path finder | the same walks through both A*s, waypoint bits compared | **the path finder matches the reference on 96 case(s): 41 walk(s) found, 127 waypoint(s) identical, 1 case(s) compared on the answer alone** |
| the schema | every column `DbSchema.h` declares, against this database | **the schema accounts for all 95 reference column(s): 55 explained, 11 mapped, 7 packed, 22 stored** |
| tests | `cargo test --workspace` | **210 passed**, 1 failed at the 2026-10-31 reading; later partial runs are recorded in `docs/HANDOFF.md` |

## What the numbers say

The two lookup axes are complete: every client message the reference dispatches has an arm here, and
every message it can put on the wire is reachable from a send path here. The load block and the
generated tables match their sources message for message, and the seeds are regenerated from the same
sources rather than typed. The map-data axis is the odd one out: it measures a *port* against the
reference's own importer output for the same bytes, so a disagreement there is a reading bug, not a
missing feature.

## What is not measured here, and is written down elsewhere

Everything in this table is mechanical. The parts of parity that are not are enumerated in
`docs/10-parity-queue.md`, and the ones that remain are all of one kind - they need something this
machine cannot supply:

- **T10's search** is ported: `GmPaths.c`'s A* over the imported graph lives in
  `crates/asura-world/src/pathfind.rs`, click-to-walk answers with the first waypoint of this server's
  own path, and a walk the finder refuses is answered with the agent's position (`MILESTONES.md`,
  `docs/07-status.md`). What is not compared here is the finder's output against the reference's on
  live client walks: the recorded comparison covers 96 cases (`docs/10-parity-queue.md`).
- **T9 and T14's live halves** need a second real client rendering the other player.
- **T13's memory clause** needs a human watching the process during the ten-minute idle run.
- **The ground item** waits on a capture of a working drop, which the reference cannot produce because
  its own drop handler is the unfinished one.

## The generated checks

The twelve checks this table listed were `--check` runs of generators that are no longer in the tree
(`docs/09`, `docs/10`, `docs/16`). None of them can be re-run, so "current" is not a claim this page can
still make. What replaces them is narrower and real: the packet table is the implementation
(`crates/asura-protocol/src/packets.rs:34-40`), the migrations are versioned and applied by
`asura_db::migrations::ensure_current`, and the path regression is
`crates/asura-world/tests/path_dump.rs` against the bundled `data/path-graphs.bin`.
