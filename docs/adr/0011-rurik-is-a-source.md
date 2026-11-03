# ADR 0011 - Rurik joins GWPS as a recorded source, not a dependency

- **Status:** Accepted (owner's direction, 2026-10-30)
- **Date:** 2026-10-30
- Checkout: `C:\Users\Apo\Documents\GitHub\Rurik` (sibling of this repository, never inside it),
  `https://github.com/GameJawnsInc/Rurik`, MIT with a wiki-content carve-out.

## Context

Rurik is a Guild Wars 1 server emulator and mod kit, written to be developed from an agentic harness,
and most of it is research: a message schema validated against a dumped build, per-feature study
documents with the instruction addresses that pin each claim, and content tables carrying provenance.

It is a *peer* of this project, not a library: it answers questions this tree has been answering by
measurement one client run at a time. Three of those questions were open when it arrived, and it
closed all three the same afternoon:

- the exploration grid (`0x008B` + `0x008A`), which this project had just built from a capture and a
  crash log: Rurik's `content/fog.toml` carries the same conclusion with the measurements behind it -
  continent 1 is `(64, 128)`, 8 of 8 live captures, and the pair is sent 0.58-0.73 s into the load,
  immediately after `INSTANCE_LOAD_INFO`. That last detail is why our first attempt did nothing: the
  messages were correct and in the wrong place in the block.
- the RLE's empty-band shape, which our encoder already produced, stated there as "every band an
  empty `u16` header, exactly the shape 7 of ArenaNet's own 8 bands have".
- the field layouts of `139` and `138` (`schema/messages.json`, imported from OpenTyria and validated
  field-for-field against the dumped build 38797 message tables).

## Decision

1. **Rurik is a source.** Its findings may be read, trusted where it cites an instruction address or a
   capture, and copied into this project's own words with its own evidence.
2. **It is not a dependency.** Nothing in `Cargo.toml`, nothing under `crates/`, and no test may
   require the checkout to exist. `cargo build --workspace` succeeds on a machine without it.
3. **Its vocabulary does not enter our code.** A module is named after the game's noun and documents
   the shape as this server's; nothing in `crates/` describes code as coming from Rurik, and no file
   under `crates/` names the project as its origin.
4. **Every fact taken from it is re-stated with our own evidence.** A doc comment says what the
   behaviour is and where our proof lives (a recording under `recordings/`, a crash log, a test); it
   does not say "Rurik says so". Where Rurik is the only witness, the sentence says that plainly.
5. **A conflict resolves in favour of the client.** Rurik's claims are measurements like any other: the
   real client in the owner's hands is the last word, and a disagreement is reported, not split.

## Consequences

- The exploration grid's position in the load block is now the load block's own concern
  (`crates/asura-world/src/map.rs`), not the player block's - a change Rurik's timing measurement
  settled after our own capture could not.
- The dangling reference in `crates/asura-world/src/map.rs` that used to cite
  `docs/10-rurik-comparison.md` - a document that was never committed - now cites this ADR.
- Reading its `studies/` before a client run is allowed and cheap; taking its numbers as proof is not.
