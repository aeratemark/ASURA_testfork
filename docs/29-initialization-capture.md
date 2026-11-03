# 29 - The initialization capture: what to run, and what it answers

The initialization phase of the client is not built on demand. A live server fills its structures
once, as the instance is entered - the agent array, the party, the instance, the item tables, the
guild and town-alliance block - and a window opened afterwards reads what is already there. So a
window proves nothing about the messages that built it, and the measurement has to be the stream
and the structures taken at the same moment.

This page is the run sheet for that. It is the input to stage 3 (`docs/HANDOFF.md`) and its
working list is `docs/28-headers-to-name.md`.

## The two halves

| half | instrument | what it gives | what it needs |
|---|---|---|---|
| the stream | `tools/sniff-capture.py` | every message both directions, with seq, time, header, size and bytes | elevated shell, 32-bit Python, the library root |
| the structures | `tools/context-dump.py` | the contexts as the client holds them, one JSONL record each, names hashed | 32-bit Python, the library root; **read-only**, no elevation |

Both resolve the library root the same way (`tools/sniff-capture.py:89`, `--py4gw-root`,
`PY4GW_STEALTH`), so there is nothing to configure twice. The dump connects with
`game_thread=False`, which the library documents as the read-only connection, and refuses to fall
back to the writing one.

## A gap is read from `tick`, never from `t`

Each record carries two clocks and only one of them is the packet's. `t` is stamped when this tool's
drain loop reads the entry out of Py4GW's ring, so every record in one drain shares a late time, and a
stall in the loop hides a gap. `tick` is the client's own counter at the hook, taken from the entry
itself. **A duration is a difference of `tick`**, and the summary record carries `tick_first` and
`tick_last` so the unit is read off the file: span over `seconds`. Two readings of one travel bug were
wrong because they were taken from `t` (2026-10-31); the same file read from `tick` gave 7.0 s and
0.90 s for two crossings the drain clock could not tell apart from a 0.05 s step.

## One command

`.\tools\live-session.ps1 -Preset init` is the run below, driven: it relaunches itself elevated once,
starts the raw both-direction capture, dumps the structures on a cadence (ten seconds for this preset -
`tools/live-session.ps1:78`, default documented at `:17` - and only the contexts an instance entry
changes), prints the six actions, and records a `MARK` line every time Enter is pressed in its console.

The six actions for this preset are the ones a client performs anyway and nobody has to stage: enter an
outpost from character select, travel to the explorable it exits to, come back, change district, open
the world map and close it, walk to a portal and cross it. Falls back to the manual steps below when a
particular run needs a different shape.

## The run, by hand

Terminal A, elevated (the sniffer patches the client, the dump does not):

```powershell
# 1. start this BEFORE the character enters the instance, and leave it running
python tools/sniff-capture.py --dir both --seconds 600 --raw --out D:\captures\init-148.jsonl
```

`--raw` keeps every byte, which is what the field work needs, and it also keeps names and UUIDs:
that file stays off this repository. `--dir both` and no `--header` are deliberate; the neighbours
are half the evidence.

Terminal B, a normal shell:

```powershell
# the client is logged in and standing at character select, or in an outpost
python tools/context-dump.py --label before-load

# 2. now walk the character into the instance under test (outpost 148)
#    wait about ten seconds after the load settles, then:
python tools/context-dump.py --label after-load-148

# 3. open the world map, close it, and dump again
python tools/context-dump.py --label after-worldmap

# 4. travel to a second instance (146), wait ten seconds, and dump again
python tools/context-dump.py --label after-travel-146
```

Then stop terminal A with Ctrl+C. Four dumps and one capture, in the order above.

## What each file answers

- **`before-load` against `after-load-148`** is the initialization, field by field: whatever changed
  was set by that load, and whatever did not change was set earlier (login) or never set at all.
  A field that appears in the first dump and not the second is a field this server will have to
  fill some other way.
- **`after-worldmap`** separates the map panels' own state (`world_map_context`,
  `mission_map_context`) from the instance's. The exploration grid and the instance table live here.
- **`after-travel-146`** shows which fields a *second* load rewrites and which ones persist, which is
  what tells a per-instance structure from a per-character one.
- **The capture** carries the messages, in order, with the timings that place each one inside the
  load.

## What I do with it

1. For each context, the fields that changed across the dumps are the target list; the capture gives
   the messages that arrived in the same window.
2. A field is tied to a message by value: an agent's model id, level, profession and name-hash in
   `agent_array` against the fields of `0x0020`, `0x0056`, `0x0057`, `0x006D`, `0x009B`, `0x00A6`,
   `0x009A` in that window. That is the two-witness rule `docs/28` states, with the structure as the
   second witness instead of a window.
3. What that produces, in order: the agent-array field map (the ten messages this tree never sends),
   then items (`0x015E`, `0x013A`), then the guild and town-alliance block (`0x011A`, `0x011B`,
   `0x011C`, and the twenty rows of `0x011F` with their `0x012F` tail - `guild_context` carries
   `TownAllianceStruct` and `GuildHistoryEventStruct`, which is the structure that block fills).
4. Each finding lands as one commit: the row in `crates/asura-protocol/src/packets.rs` with its
   evidence, and the order and set recorded in `docs/23-initialization-protocol.md`.

## What this does not settle

- The 306 game rows no capture contains (of 323 unnamed game rows in
  `crates/asura-protocol/src/packets.rs`): they need a stimulus, one opcode at a time
  (`docs/20-rurik-gap.md`, P1; the arithmetic is `docs/28-headers-to-name.md`).
- The map's population itself - which body stands where, with which model and name - is data, not
  protocol. The capture says what a live server sends for it; producing it for our own maps is the
  archive job, and the reason the agent array cannot simply be copied by hand.
- The contexts are the instrument's; nothing from it enters this tree as a file, offset or table
  (`docs/adr/0013-py4gw-stealth-is-our-link-to-the-client.md` #4). What is committed here is the
  dump under `recordings/asura/world/`, names hashed, and the field map derived from it.
