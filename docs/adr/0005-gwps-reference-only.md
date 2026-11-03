# ADR 0005 - GWPS becomes reference only; ASURA is where things are built and tested

- **Status:** Accepted (owner's direction, 2026-10-25)
- **Date:** 2026-10-25
- Supersedes the migration-ledger role of `docs/04-from-gwps.md` ("steps" there are now a record of
  what was learned, not a plan of edits).

## Context

GWPS proved the client protocol and produced the measurements this project is built on: a map server
costs ~12 MB and one thread, 53 hops produced 7 hangs when the destination was built inside the hop,
and prebuilding removed them. It also has a ceiling we measured rather than guessed: one process, one
thread and one database connection per instance, and no boundary between authentication, placement and
simulation.

Two things follow from that. First, continuing to patch GWPS is spending effort on a structure we have
already decided to leave. Second, the most important open question - whether a map instance may be
created on demand or must always be prebuilt - cannot be answered with more GWPS experiments, because
the one experiment we ran was confounded: the settings acknowledgement and the prebuild landed in the
same binary. Splitting that apart in GWPS means another round of patching the very code we are
abandoning.

## Decision

1. **GWPS is frozen as reference.** No more features, no more architectural patches. It stays runnable
   because it is the oracle: when ASURA disagrees with the client, GWPS answers "what does the real
   thing send here, byte for byte?", and its captures and tooling stay.
2. **ASURA is the implementation.** Everything new is built and tested here: the auth service, the
   schema, the registry, placement, instance lifecycle, and eventually simulation.
3. **Instance creation policy is settled by experiment in ASURA, not by argument.** The design must
   support both answers, and the experiment must be cheap to run:
   - **Default: prebuilt pools.** A travel is answered only with something already running.
   - **Hypothesis to test:** on-demand creation is acceptable *if* the destination server is announced
     ready before the client is redirected. If it is, pools become an optimisation instead of a
     requirement, and the memory cost of hundreds of idle instances can be deferred to demand.
   - **Answered 2026-10-27: the hypothesis holds.** Measured on the real client with nothing preloaded,
     a travel to an unhosted map loads it in 0-2 ms and the client settles; precaching is therefore not
     a requirement (`docs/03-protocol.md` 5c, `docs/roadmap.md`). The flag that does it is
     `asura-world --on-demand` (`crates/asura-world/src/travel.rs:83`), and prebuilding stays the
     default because it is the reference's shape, not because it is needed.
   - The test that answered it: one world server, no preload, a real client, and the recorder.
4. **Instance creation has to live somewhere explicit.** It belongs to the world server, behind an
   interface that takes (map, party) and returns an instance - whether that instance came from a pool
   or was built on the spot is that server's business. No caller should be able to tell the difference,
   and no caller may construct one itself.

## Consequences

- `docs/04-from-gwps.md` is demoted from a migration plan to a record of what GWPS taught us; its
  "steps" describe the shape of the new stack, not edits to the old one.
- The roadmap starts at ASURA's own skeleton. There is no "port step 1" that is a GWPS patch.
- The instance-creation experiment becomes a first-class milestone, scheduled after the world server
  exists and before any capacity planning is taken seriously.
- What GWPS measures stay valid as *reference numbers* (memory per instance, handshake behaviour,
  redirect shape), and are cited as such. They are not promises about the new implementation.

## What this does not change

- The one rule survives as the default: a client is never told to travel to something that is not
  already answering. The experiment may relax it; nothing may relax it by accident.
- The protocol facts in `docs/03-protocol.md` still come from GWPS, and still have to be re-proven
  against the real client in ASURA.
