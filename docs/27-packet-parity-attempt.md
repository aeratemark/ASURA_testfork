# The packet-parity attempt: objective, what was done, and why it failed

Written 2026-10-31, after reverting the working tree to `e96a909 "up to date and working"`.

Every tool, fixture and file this page names lives on that branch, not on `main` unless the text says
otherwise; the few things that were later taken from it are marked. The work itself is not lost: it is
committed on the branch **`packet-parity-attempt`** (`9818bf2`). `main`
is the working version, and this document lives on `main` so the failure is not the only thing that
survives.

## The objective

The owner's words, which are the specification: *"reproduce faithfully the packet stream of the live
server/client"*. The instruction given at the start of this attempt: *"need you to fix the packets then,
notify when client is ready to get tested, instrument the logging obsessively so we can troubleshoot after if
needed"*.

Nothing in that asks for new features, new world content, or a redesign of how messages are built. It asks
for the byte stream to match the reference, and for the server to be able to say what it sent.

## What was done

**Measured the reference, at the wrong layer.** Every server-to-client size the captures under
`recordings/client/` contain was collected into a generated table (`crates/asura-protocol/src/live_sizes.rs`,
`tools/gen-live-sizes.py`), and the world compares every frame it sends against it at run time. Those sizes
are the client's message struct, not the wire (`docs/03-protocol.md` 5e), so the table said "the reference's
size" about numbers that are 2 bytes or more above what the reference sends. The run-time line it prints is
worth keeping; its reference is not.

**Corrected sizes and field widths, message by message.** Around thirty messages were brought to the
sniffer's shape, each with its captured bytes recorded next to the row in
`crates/asura-protocol/src/packets.rs`: `0x0028`, `0x0099`, `0x00F1`, `0x017C`, `0x0196`, `0x019A`,
`0x00DA`, `0x00DB`, `0x001D`, `0x0018`, `0x001A`, `0x01A0`, `0x0029`, `0x01A6`, `0x006E`, `0x008B`,
`0x009F`, `0x00A2`, `0x00B6`, `0x00F0`, `0x0186`, `0x0189`, `0x018E`, `0x01D3`, `0x01D4`, `0x01CC`,
`0x01B3`, `0x0022`, `0x00E9`, `0x0020`, and others. Two conclusions came out of it, one measured and one
wrong: that the live server writes **one element fewer than it counts** on `0x001A` and `0x01A0` (126 and
10 samples, from the same capture layer, so it needs a wire confirmation before it is a rule), and that a
game message's header is four bytes - which was the client's struct read as the wire, and is what "What was
actually wrong" below is about.

**Measured the order.** `tools/stream-diff.py` compares our stream with a live one. It showed the
exploration grid and the instance table arriving in the wrong place, and both were moved to where the
captures put them.

**Built the instrumentation the instruction asked for.** The world records every message of every session to
`logs/capture-world-8001.jsonl`; each session ends with a `stream summary` line; `tools/stream-sizes.ps1`,
`tools/stream-diff.py`, `tools/client-check.ps1`, `tools/live-gaps.py` and `tools/gen-npc-spawns.py` read
those files back. `docs/25-client-readiness.md` and `docs/26-packet-gaps.md` describe the state and the
remaining work.

**Built one thing that was not asked for.** A harvested NPC population (`npc_spawns`), switched off by
default behind `ASURA_NPC_POPULATION`. It is on the branch and it is not part of the objective.

## The failure

**The client still answers `007`.** That was true before the attempt, and it was still true after the last
build of it (2026-10-31 16:25). The last run did reach a served session and was not usable from it: the
client loaded into 148, asked for its items, players and spawn point, took 1,870 ticks, and left
(`logs/world-8001.log`, `logs/capture-world-8001.jsonl`).

What that build was serving was the sniffer's head: `0x017C` at 20 bytes, `0x0186` at 4, `0x019A` at 32
(`logs/capture-world-8001.jsonl`, seq 1-4), where the reference and this tree's own wire send 6, 2 and 16
(`recordings/gwps/world/load-stream.log`, `recordings/asura/world/live-client-session-20261027-223528.jsonl`).
The name message was rewritten three times inside one hour - `u16` count and variable units (22 bytes), then
`u16` count padded to a fixed forty bytes (44), then `u32` count and variable units (24) - and no version of
it was checked against a wire capture, which shows the reference sending a `u16` count and that many units,
14 bytes for a five-character name.

The `0x8000` frames were read as a verdict and are not one: the last build's client acked `0x0186`, `148` and
`0x001E` (`logs/capture-world-8001.jsonl`, seq 13-18), while in the session of 2026-10-27 that loaded, walked
and travelled all 360 of its `0x8000` frames carry a zero payload and the client worked. Neither capture names
`0x017D` in an ack, working or not.

## What was actually wrong (written 2026-11-01, after the reversion)

**The attempt aimed at the wrong layer.** The sizes it moved ~30 messages to came from
`tools/sniff-capture.py`'s `entry.size`, and that is the client's message struct: a four-byte header, fields
at native width, zero padding, and in `0x000B` absolute pointers. The wire is the packed form - two-byte
header, the fields alone - and the server that works with this client writes it: `recordings/gwps/world/load-stream.log`
has `0x017C` 6 bytes, `0x019A` 16, `0x001E` 6, `0x017D` 14. This tree already sent those sizes, and a
session with them loaded, walked, chatted and travelled.

So the loop was closed on itself: the table was generated from the struct layer, every message was bent
towards it, and the run-time line "this size is not the reference's" fired on correct frames. The one
client-measured fact the attempt already had - an eight-byte `0x001E` answers `007`, a six-byte one plays -
says exactly what the reference's send log says, and was read as an exception instead of as the answer.

What survives from the attempt, and what it is evidence of:

- the client tells you where it stops, and one message per build is the only way to read that answer;
- the offline suites cannot see this class of defect, because the harness frames by this tree's own table;
- the instrumentation is sound and its reference was not: a capture of what this server sent
  (`--capture`, `logs/capture-world-8001.jsonl`) is wire evidence, and a capture of what the client held is
  not. That distinction is now the rule in `docs/03-protocol.md` 5e.

## Why it failed, in process terms

1. **I changed many messages per step and asked the client to judge a batch.** `AGENTS.md` says one change,
   one subject, and the client is the judge. Each of those is a different message in a different phase; a
   client failure after a batch cannot be attributed to one of them, which is exactly what happened.
2. **The offline checks cannot see this class of defect.** The test client frames the stream by our own
   packet table, so a message of the wrong size is internally consistent and the tests pass. `cargo test`
   was therefore never evidence about the client, and I reported it as if it were.
3. **The live-size table's reference was the wrong layer.** It was generated from the sniffer captures, so
   it enforced the client's struct sizes: it reported "every one at a size the reference uses" while every
   corrected message was 2 or more bytes longer than the wire, and the client was refusing the load.
4. **I invented a feature.** The NPC population was built because the gap list ranked it first, which is a
   decision about the product, not about the stream. It was not requested and it was not reviewed.

## What to take from the branch

```powershell
git log --oneline main..packet-parity-attempt          # one commit: 9818bf2
git diff main packet-parity-attempt -- <path>          # any single message, tool or document
git checkout packet-parity-attempt -- crates/asura-protocol/src/packets.rs   # a piece, without the rest
```

Worth taking first, in this order:

1. `crates/asura-protocol/src/live_sizes.rs` + `tools/gen-live-sizes.py` + the `send()` check in
   `crates/asura-world/src/connection.rs`: the run-time "this size is not the reference's" line, **re-pointed
   at the wire** - generated from `recordings/gwps/world/load-stream.log` and the `recordings/asura/`
   sessions, never from a sniffer capture. As the branch generates it, from the client's struct, it is not
   worth taking.
2. `tools/stream-diff.py`, `tools/stream-sizes.ps1`, `tools/client-check.ps1`: reading a session back.
3. ~~`crates/asura-world/src/capture.rs` wiring~~ - **already taken, and differently**: the session
   capture is on `main` and switched by the command line, `--capture PATH` or `ASURA_CAPTURE`
   (`crates/asura-world/src/main.rs:131-136`, used at `connection.rs:485,534`), not by
   `config/services.toml`.
4. The individual message corrections, one at a time, each verified with the client before the next.

Not worth taking: the NPC population, unless it is wanted as a feature in its own right.

## If this is attempted again

- One message per build, and the client in the loop before the next. No batch of sizes, no ordering change
  and a message change in the same build.
- Before any of that: work out which layer the number came from. A wire size is a size this server sent
  (`--capture`, `ASURA_TRACE`) or the reference sent (`recordings/gwps/world/load-stream.log`); a sniffer
  capture is a count, an order, a name and a trigger, and never a size (`docs/03-protocol.md` 5e).
- When the client stops, the answer is which message it refused, not which size is wrong in theory. The
  `0x8000` frames are not that answer: they are identical in a session that works.
- A counted field (a name, an item's modifiers, a blob) is settled by the wire, where the count is, never by
  a sniffer capture - `--raw` included. `0x017D` is 14 bytes for a five-character name
  (`recordings/gwps/world/load-stream.log`), and that is the whole of what this message needs.
- A message the client refuses tells you more than ten that it accepts, and the eight-byte `0x001E` that
  answered `007` was that fact all along.
