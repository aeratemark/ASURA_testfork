# ADR 0013 - Py4GW Stealth is this project's link to the client

- **Status:** Accepted (owner's direction, 2026-10-30)
- **Date:** 2026-10-30
- On this machine: `C:\Users\Apo\Downloads\py4gw_stealth_game_thread_bridge.zip` and
  `py4gw_stealth_game_thread_bridge_callbacks.zip` (the hardened ABI v3 build), over the Py4GW native
  tree at `C:\Users\Apo\Desktop\Py4GW`.

## Context

Everything this project knows about the client came through one of three doors: the reference server's
C source, a capture of our own server, or a crash dialog read by hand. The first is not the client, the
second only shows what a server sent, and the third is a page of text a person has to look at.

Py4GW Stealth is a fourth door, and it is the one that reads the client itself. Read from the packages,
it is: a small bridge (4,736 bytes) allocated inside a running `Gw.exe`; a 9-byte detour at
`game_thread.leave_game_thread_func`, pattern-resolved at run time; a relocation-free i686 dispatcher
with a 128-slot client-to-Python event ring and a 64-bit subscription mask; and a catalogue of 28
subsystem offset files that resolve addresses by byte pattern **and by assert anchors** - `OsAnsi.cpp`
`chars>=MAX_PATH`, `GrImage.cpp` `bits || !palette`, `Amet.cpp` `data`, `File.cpp` on its open path.

Two of those catalogues are the reason this ADR exists, because they are capabilities this tree
otherwise does not have at all:

- **`gw_dat_reader.json`** reads the archive **through the client's own code** - its open-file, image
  and `AMET` paths. That is the client's parser answering, not ours.
- **`stoc.json`, `ctos.json` and `packet_sniffer.json`** resolve the server-to-client handler table
  pointer and the client-to-server send wrapper. That is the sniffer: what the client receives and what
  it sends, read at the place the client reads and writes it.

The rest of the catalogue covers `agent`, `item`, `skillbar`, `party`, `quest`, `merchant`, `map`,
`pathing`, `effects`, `chat`, `camera`, `memory`, `events`, `ui`. The owner's instruction is what turns
this list into a decision: **it is our link to the client.**

## Decision

1. **Py4GW Stealth is this project's link to the client.** For anything that needs the running game -
   its state, the contents of its archive, what it sends and receives - this is the instrument of record.
   Its archive reader and its sniffer are the reason: they ask the client's own code instead of a
   reimplementation of it, which is the strongest evidence class this project can hold short of a crash.
2. **It answers what the client does. It does not answer what our server must send.** A reading taken
   through it is evidence of the same class as a capture: a fact, restated in our words, with the
   recording on disk named beside it.
3. **It is not a dependency, and nothing under `crates/` may reach it.** No crate, test, service or start
   path links it, imports it or requires it to exist. `cargo build --workspace` and
   `asura-supervisor start` work on a machine without it (`AGENTS.md` #6).
4. **No file, offset, byte pattern or table from it enters this tree.** Neither package carries a
   `LICENSE` (checked), so nothing may be copied. What crosses back is a measurement - a number, a
   shape, a byte string - written as ours, with our recording named as the proof.
5. **A conflict resolves in favour of the client in the owner's hands**, and in the client's own code
   over any of our inferences.
6. **Its operating requirements are its own business.** It needs 32-bit Python, an administrator shell
   and code injection into a running `Gw.exe`. No build, test, CI or start path in this repository
   invokes it, and none may begin to.

## Consequences

- **What the sniffer reports is the client's message buffer, not the wire.** Its `size` and `data` are the
  struct the client reads into and writes from: a four-byte header, native-width fields, zero padding, and
  in one case absolute pointers. A size or a field layout read there is the client's bookkeeping; the wire
  size comes from a capture of the socket (`recordings/gwps/world/load-stream.log`, a `--capture` run under
  `recordings/asura/`) - `docs/03-protocol.md` 5e. What the sniffer uniquely knows is a header, a count, an
  order, a name and a trigger.
- **The questions left to our own readers are the ones that need the client closed.** A message's wire
  shape from the client's format tables, the assert table read as a catalogue rather than used as an
  anchor, which build a byte pattern was measured on, and one handler's disassembly. Nothing in the
  offsets catalogue offers any of those: a grep over all 28 files for a format or field table returns
  nothing.
- **The server still needs an offline archive path.** A live DAT reader answers for the loaded map while
  a client is injected; a world server reads geometry at start-up with no client anywhere. The two do
  not replace each other, and the second may never depend on the first.
- **Any third-party archive reader becomes a cross-check.** Rurik's decompressor and map readers are now
  something to check against what the client reports, not the primary source.
- **One hook owns `leave_game_thread_func`.** A component that patches the same entry at the same time
  corrupts both. The packages restore the original bytes in a `finally`, and a Python process killed
  mid-run leaves the client patched until it is restarted - their own `README.md` says so, and it is
  the failure to expect rather than investigate.
- **Its licence position is recorded, not resolved.** With no licence grant, its permitted use here is
  the one Rurik's own rule states for `Py4GW_Reforged`: verify a value we derived ourselves.

## What this does not change

- `AGENTS.md` #6 - no external project is a dependency of this repository's code. This ADR is about an
  instrument used by hand, and the three conditions that keep a tool a tool still apply.
- The objective: the packet stream of the live server and client remains the specification.
- The standing rule that a client carrying our Diffie-Hellman parameters is never pointed at the live
  service. The bridge does not alter a client's parameters, so it does not relax that rule.
