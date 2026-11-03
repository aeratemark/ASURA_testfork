# 28 - The game headers still to be named, and the test that names each

Stage 3 of the owner's staging (`docs/HANDOFF.md`, "The staging"). This page is the working list: every
game header a real session used whose row in `crates/asura-protocol/src/packets.rs` has no name, what the
committed captures prove about it, and the one action that would name it.

Written 2026-11-01 after the offline pass closed. Fourteen headers were named in that pass (`23de10e`,
`d17a60a`), and `0x8093` was named `GAME_CMSG_INSTANCE_QUERY` from a world-map capture after it; the
seventeen below are what is left, and every one of them needs a provoked capture.

## How a name is earned here

A name is a measurement, not a guess (`AGENTS.md` #1). It needs two witnesses that agree:

1. **the wire**: which capture, which sequence number, what the payload's fields are, which ids or values
   they carry, and which messages it always arrives between;
2. **the function**: either the client's own code naming it (a handler's asserts, read through a source
   that is cited by file and line - `docs/adr/0011-rurik-is-a-source.md`) or the player making the
   feature happen while the capture runs.

Where the first exists and the second does not, the row keeps its evidence and stays `Unnamed`. That is
not a failure: it is the difference between "we know the shape of this message" and "we know what the
game calls it", and writing the second without the first is how a wrong name gets into a table and stays
there.

## Why this cannot be finished offline

- The message catalogue of this lineage cannot name them. `Rurik schema/messages.json` is imported from
  the same OpenTyria `msgdefs.c` this table came from, and its own provenance note says so - "a
  convenience index of ONE lineage, NOT corroboration" - so it carries the same headers unnamed
  (`"table": "GAME_SMSG_0431"` for the ones that have no name).
- The handler studies are silent or negative on all seventeen: Rurik's `studies/smsgnames`,
  `studies/smsg` and `studies/newopcodes` return **NOT FOUND** for the item and agent rows and say
  nothing at all about the rest. The Eden checkout carries no opcode catalogue: seven `GAME_SMSG`
  mentions in its whole documentation tree, none of them about these.
- The reference server never sends any of them: `recordings/gwps/world/load-stream.log` has no `SEND`
  line for one of the seventeen, so the reference's own sequence cannot place them either.
- The captures are exhausted as evidence of *function*. Each header below has its payload, its
  neighbours and its rate recorded; nothing further can be read out of them.

## The seventeen

`wire` is the size in `packets.rs` (the reference's `max_size`, which for these fixed layouts is the wire
size). Counts are sightings in the committed captures, per file.

| header | wire | sightings | shape the capture proves | arrives between | what would name it |
|---|---|---|---|---|---|
| `0x0035` | 14 | doors 4 | agent id, then two floats whose second is 1.0 (2.472 / 1.0, 1.75 / 1.0) | `0x006D` `NPC_UPDATE_WEAPONS` and `0x00A0`; also after `0x0021` | a weapon or henchman gear change on a body whose block is re-sent |
| `0x0067` | 12 | character_creation 1 | vec2 of two `FLT_MAX` then a zero word | `0x009F` and `0x0100` `CINEMATIC_START` | any cinematic: the mission intro or a scripted scene |
| `0x007C` | 6 | doors 27, and 1 to 2 in each of three others | one dword, 1 in 31 of 32 sightings and 2 in one | `0x00F2` `INSTANCE_LOADED` and `0x007D` | one district entry with the header on a filter alone |
| `0x007D` | 6 | doors 28, and 1 to 2 in each of three others | one dword that reads as a player or agent id (2, `0x3d`, `0x1f`) | immediately after `0x007C` | the same entry; the pair is one declaration |
| `0x008C` | 11 | doors 36, movement 15 | an id (`0x20`, `0x21`, 32, 33), a second number 20-22, a byte 1 | `0x0029` `AGENT_MOVE_TO_POINT` and the tick stream | stand still, then walk, then fight, in one capture |
| `0x00A7` | 14 | movement 354, login-to-portal 38 | three dwords: 9-38, then 1-12, then 1-11; about three per second | between ticks, and next to `0x0021`, `0x0029` | walk 30 s, stand 30 s, fight 30 s |
| `0x00CF` | 10 | doors 9 | two dwords: 10, then 25, 1 or 0 | `0x009F` and `0x009B` / `0x00A3` | press an adrenal skill on a warrior |
| `0x00D0` | 6 | doors 1 | one dword, 10 | `0x0025`, `0x0029` and the tick stream | the same press; the source reading ties the trio to adrenaline spends |
| `0x0107` | 83 | doors 15 | ends in the same block `0x0020` ends with (scale 1.0, 250.0, 1.0, 12.0, name) | always immediately before a `0x0020` create | something appearing in front of the player: a monster spawn or a summoned body |
| `0x011F` | 138 | 20 in each of three loads | mostly zero dwords, a masked name run, then 0x28, 0xF0, 0x3F, 0x3F, 2, 0x16, 0x5F | `0x0118` `GUILD_PLAYER_ROLE` and `0x012E`; followed by `0x012F` | a load capture paired with a dump of `guild_context`/`party_context` right after it: the block is built at init, so a window opened afterwards reads what is already there and proves nothing (`docs/29-initialization-capture.md`) |
| `0x012F` | 562 | 1 per load | zeros, one 0xF1, then a counted string run | after the twenty `0x011F` rows, before `0x0127` | the same dump: it is that block's tail |
| `0x015B` | 10 | character_creation 25 | two identical dwords, an item id twice | between `0x0161` and `0x015A`, per item | one pass through the character-creation screen |
| `0x0168` | 6 | doors 6 | two words (0x31 / 0x81, 0x26 / 0x106) | after an item `0x0161`, before a `0x0020` create | a kill whose drop the player sees |
| `0x0194` | 6 | doors 9, login-to-portal 2 | one dword, always 0 | after `0x016E` and `0x018E` `INSTANCE_LOAD_FINISH` | a load into a new instance with the filter on this header |
| `0x01AE` | 55 | doors 17 | a leading dword 1, then a masked player name and a run of asset ids | after a `0x8090` players request and a ping, before the `0x000F` burst | enter an instance with another player present |
| `0x01BE` | 6 | doors 9, login-to-portal 2 | one dword, always 0 | `0x01B3` and `0x01BF`, before `0x00BA` | the party window, with a henchman added |
| `0x8092` | 122 | first-crossing 48, slow-crossing 24 | a u16 near the instance id, then a run of DAT-shaped ids | with `0x8093` while an instance is chosen or crossed | open the world map, click one destination, close it |

## The test protocol

One action per capture, so the new headers can be attributed to it:

```powershell
python tools/sniff-capture.py --dir both --seconds 90 --out recordings/client/feature/<name>.jsonl
```

- Start the capture, perform exactly the one action, stop it, and name the file after the action.
- `--header` narrows a capture to one header when the question is "did this appear at all"; leave it
  off for a naming run, because the neighbours are half the evidence.
- Read the result back with the counts: a header that appears only in this capture, at the moment of the
  action, is one of the four or five the action named.
- The verdict goes into the row's `name` and `evidence` in `crates/asura-protocol/src/packets.rs`, one
  subject per commit, with `docs/24-stream-parity.md` updated in the same change.

## What the named fourteen look like, for the pattern to copy

`0x0026` and `0x015E` are the shape of a good row: a capture gives the sightings and the field values, the
client's own handler names the function, and both go into the evidence string with the file and the
sequence number. `0x016E` shows the weaker case - the source reading names the function and the capture
confirms the shape, so the status is `Inferred` and the evidence says which half is which. `0x009C` was
that case and is `Emitted` since this tree sends it.

## Not this list, and deliberately

- The 306 unnamed game rows no capture contains (323 unnamed game rows in
  `crates/asura-protocol/src/packets.rs`, 17 of them on this page): they need a stimulus, not an
  observation - one opcode on the wire and the client's reaction read (`docs/20-rurik-gap.md`, P1).
- The client-to-server headers this tree frames but ignores (`0x0084` 35 per live session, `0x0012` 12,
  `0x0051` 3): they are named already and belong to stage 4, `docs/21-surrounding-packets.md`.
