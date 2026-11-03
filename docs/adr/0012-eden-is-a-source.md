# ADR 0012 - Eden is a recorded source: a client, read as evidence for what the client does

- **Status:** Accepted (owner's direction, 2026-10-30)
- **Date:** 2026-10-30
- Checkout: `C:\Users\Apo\Documents\GitHub\Eden`, extracted from `%USERPROFILE%\Downloads\eden.7z`
  (the archive carries no `.git`, so there is no upstream to pull from).

## Context

Eden is a Guild Wars 1 **client** reimplementation in Rust, and roughly half of it is notes rather
than code: `docs/re/` holds per-surface documents (`map-surfaces.md`, `fog-of-war.md`,
`world-map.md`, `mission-map.md`) that quote the client's own instruction addresses and its assert
lines, and `.reports/` holds the decompilation runs behind them.

That is the shape of evidence this project was missing. GWPS and Rurik answer "what does a **server**
send"; Eden answers "what does the **client** do with it", which is the side of every open question
that a server log cannot show. It settled four of ours on the day it was read:

- the map-view assertion, `worldMapDims.x == mapDims.x * DXT_BLOCK_SIZE` at `GmMapView.cpp(1731)`,
  is `CMapView::UpdateWorldMapAlpha`'s, and `DXT_BLOCK_SIZE` is 4;
- the client's field codec, an array being a `u16` element count followed by `count * 4` bytes;
- `mapDims` is written **only** by the exploration-init pair, and `dims.x % 32 == 0` is asserted while
  decompressing;
- the client's campaign enum: `0` is a PvP create, `1..=3` are Prophecies, Factions and Nightfall -
  which is why a character row with `campaign = 0` was reported as Factions.

## Decision

1. **Eden is a source, in the same table as GWPS and Rurik.** Its findings may be read, trusted where
   it cites an instruction address or an assert, and copied into this project's own words.
2. **It is not a dependency.** Nothing in `Cargo.toml`, nothing under `crates/`, and no test may
   require the checkout. `cargo build --workspace` succeeds without it on the machine.
3. **No code is copied from it.** `docs/PROVENANCE.md` in Eden records that its tree contains material
   under GWCA, GuildWarsMapBrowser and GWToolbox terms, so its own `MIT OR Apache-2.0` covers only part
   of what it ships. What crosses into ASURA is behaviour and layout, written as ours - never a
   translation of its Rust.
4. **Its vocabulary stays out of our code.** A module is named after the game's noun, and no file under
   `crates/` describes itself as derived from Eden.
5. **Every fact taken from it is re-stated with our own evidence** - a recording under `recordings/`, a
   crash log, a test. Where Eden is the only witness, the sentence says so.
6. **A conflict resolves in favour of the client in the owner's hands**, in Eden's favour over a note or
   a guess, and in nobody's favour over a measurement.

## It is not a test surface (measured 2026-10-30)

Eden was going to serve as a second client we could rebuild, instrument and break instead of crashing the
retail one. A private run was built for it and works at the transport layer - `EDEN_HOSTS` pins the
retail names, a line may pin a port, the gate echoes on the port the file names, and no live permit is
asked for - but the client cannot stand in for the retail client, so the idea is dropped:

- its pregame roster is **synthetic** - a list of invented characters, not the account's own;
- its in-world viewer is **not wired** to the session flow's world yet
  (`crates/eden-cli/src/shell.rs`: "the drawn world (`world_feed`) is not the viewer's yet"), so a
  private run reaches the world on the worker and cannot enter it.

So: **Eden is a source and nothing else.** The hosts work stays in its tree, documented there, and
ASURA depends on none of it. A client-under-test remains the retail binary in the owner's hands, and the
sources in this table are what shorten the distance to it.
## Consequences
- `docs/03-protocol.md`'s exploration-grid section now cites Eden for what the map view does with the
  grid, and a capture or a crash for what our server sends.
- Where `worldMapDims` itself comes from stays open, and is answerable from Eden's `docs/re/` and Rurik's
  `schema/messages.json` before a further client run.
- The instance messages (`0x017C`, `0x019A`, `0x0196`) are settled on the wire and open only on the client:
  the form to send is the narrow one - `recordings/gwps/world/load-stream.log` has `0x017C` at 6 bytes,
  `0x019A` 16, `0x0196` 26 and `0x0020` 99, and a session with those loaded, walked and travelled - while the
  wide form a sniffer capture shows is the client's own struct (`docs/03-protocol.md` 5e). Whether the client
  tolerates the wide form on the wire is unmeasured; nothing sends it, so nothing needs it.
