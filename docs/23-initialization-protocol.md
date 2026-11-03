# 23 - The initialization protocol: everything a client is fed when it enters an instance

**Status: the protocol, written down; the gap table says what is missing.** The owner's direction,
2026-10-31: "this must mean we are still ignoring data on initialization, we will need parity, we are
ignoring too many messages and our functionality is being ignored there ... lets keep a full
initialization protocol so we know all our structures are fed appropriately."

So this document is the standing specification for one thing: **the set and order of messages an instance
sends**. A feature that works against a live server but not here is a difference in this table until the
table says otherwise.

## The rule

An instance's initialization is not a bag of messages a client tolerates; it is a sequence that fills a
fixed set of client structures. A world that sends a subset leaves structures empty, and the client then
does nothing where it would otherwise do something - the exploration grid was the first instance of this
(`docs/03-protocol.md` 11.4), the empty unlocked list was the second, and the portal is the third
candidate.

The source for the sequence is a live load, not the reference's C: `GAME_SMSG_*` order and content are
read from `recordings/client/feature/maps/pre-searing-load-3.jsonl` (instance 148, a real client against a
live server) and cross-checked against today's own run in `logs/world-8001.log`. **Order, count and trigger
are what that file is read for**; a size in it is the client's message struct and never a wire size, so no
size and no field layout comes from it (`docs/03-protocol.md` 5e) - those come from a wire capture, and a
message added here is built at the size `crates/asura-protocol/src/packets.rs` declares.

## The live sequence, in order

Non-tick messages of that load, grouped in the order they arrived; the counts are that one capture's.

| # | messages | count | what structure it fills |
|---|---|---|---|
| 1 | `0x017C` `INSTANCE_LOAD_HEAD` | 1 | the load begins |
| 2 | `0x0186` `INSTANCE_PLAYER_DATA_START` | 1 | |
| 3 | `0x017D` `INSTANCE_LOAD_PLAYER_NAME` | 1 | |
| 4 | `0x019A` `INSTANCE_LOAD_INFO` | 1 | which instance this is |
| 5 | `0x0144` `ITEM_STREAM_CREATE`, `0x0148` `ITEM_SET_ACTIVE_WEAPON_SET` | 1 + 1 | the item stream opens |
| 6 | `0x0161` `CREATE_NAMED_ITEM`, `0x013E` `ITEM_MOVED_TO_LOCATION`, `0x013F` `INVENTORY_CREATE_BAG`, `0x013A` `ITEM_UPDATE_CUSTOMIZED_NAME`, `0x015A` `ITEM_SET_PROFESSION` | 76 + 57 + 6 + 2 + 2 | the inventory |
| 7 | `0x0196` `INSTANCE_LOAD_SPAWN_POINT` | 1 | where the character stands |
| 8 | `0x001D` `PVP_UPDATE_UNLOCKED_SKILLS`, `0x0018` `PVP_UPDATE_UNLOCKED_HEROES`, `0x001A` `PVP_ITEM_STREAM_ADD` | 1 + 1 + 62 | the PvP unlock tables |
| 9 | `0x0059` `PLAYER_CREATE`, `0x0020` `WORLD_CREATE_AGENT`, `0x0056` `NPC_UPDATE_PROPERTIES`, `0x0057` `NPC_UPDATE_MODEL`, `0x006D` `NPC_UPDATE_WEAPONS`, `0x00A6` `AGENT_UPDATE_PROFESSION`, `0x009A` `AGENT_UPDATE_SCALE`, `0x009B` `AGENT_UPDATE_NPC_NAME`, `0x00F0` `AGENT_INITIAL_EFFECTS`, `0x003C` (unnamed), `0x009F` `AGENT_PROPERTY_UPDATE_INT`, `0x00A2` `AGENT_PROPERTY_UPDATE_FLOAT`, `0x0048` `AGENT_DISPLAY_CAPE`, `0x006E` `UPDATE_AGENT_VISUAL_EQUIPMENT`, `0x00B0` `PLAYER_UPDATE_PARTY_SIZE`, `0x00B1` `PLAYER_UPDATE_PARTY` | 25 + 42 + 14 + 13 + 19 + 31 + 10 + 22 + 42 + 29 + 82 + 4 + 23 + 23 + 30 + 29 | **the agent array** - every body in the instance |
| 10 | `0x004C` `QUEST_DESCRIPTION`, `0x0053` `QUEST_ADD_MARKER` | 1 + 1 | the quest log |
| 11 | `0x0118` `GUILD_PLAYER_ROLE`, `0x0121` `GUILD_GENERAL_INFO` | 1 + 14 | the guild window |
| 12 | `0x00F3` `TITLE_RANK_DATA`, `0x00F4` `TITLE_RANK_DISPLAY` | 3 + 8 | the title track |
| 13 | `0x01A0` (unnamed) | 10 | the instance table (`docs/21-surrounding-packets.md`) |
| 14 | `0x006B` `POST_PROCESS`, `0x003E` `AGENT_ALLY_DESTROY`, `0x0021` `WORLD_REMOVE_AGENT`, `0x005A` `PLAYER_DESTROY` | 1 each | the tail |

## The two layers, measured (2026-10-31)

One run entered the same outpost twice, with a different-district load of that outpost between them
(`recordings/asura/world/load-layers-20261031.jsonl`, five loads with their per-header counts). The
district load carried **0 agent creates** and 260 messages, against 8 creates and 539 messages for the
full load of the same map. That splits the initialization into two layers by measurement rather than
by guess:

| layer | what it is | sent on | headers |
|---|---|---|---|
| per character | items, titles, party, player flags, guild, the instance table | every load, a district change included | `0x0161`, `0x015E`?, `0x013E`, `0x013F`, `0x00F3`, `0x00F4`, `0x00B0`, `0x00B1`, `0x003C`, `0x0118`, `0x0121`, `0x01A0` |
| per map population | every body in the instance and its streams | only when the world sends a population | `0x0020`, `0x0021`, `0x0025`, `0x0029`, `0x002B`, `0x0028`, `0x0048`, `0x0056`, `0x0057`, `0x0059`, `0x005A`, `0x006D`, `0x006E`, `0x008C`, `0x009B`, `0x00A6`, `0x00AA`, `0x00F0`, `0x00F1`, `0x0115`, `0x0107` |

The same run measured the per-map shape of a load: 166 loaded with 8 creates and 538 messages, 162
with 4 and 3 creates and 342 and 238 messages, and `0x008A` carried 132 bytes on every load but the
session's first. `0x0196` carried a different position per map. So the population block is
`population = creates` and it is per instance, never a constant to copy from one capture.

What the client asks while this happens, in the same run: `0x8093` eight times, one per instance
(`166, 145, 146, 148, 160, 161, 162, 164`) when the world map opened; `0x00B1` once, naming 166 as the
chosen instance; `0x00C1` twice at a portal, `{1, 0}` then `{0, 0}`. Of the three, the world-map request
is answered - `GAME_CMSG_PARTY_TRAVEL` ends in the same transfer cycle (`crates/asura-world/src/
dispatch.rs:58`, `crates/asura-world/src/travel.rs:298`). `0x8093` is named
(`GAME_CMSG_INSTANCE_QUERY`) and read, and `0x80C1` is read: neither starts anything
(`crates/asura-world/src/travel.rs:154-166`).

## What this world sends today, in order

`load_block` then `items_block` then `player_block`, from `crates/asura-world/src/{map,item,player}.rs`:

| # | messages | state |
|---|---|---|
| 1-4 | `0x017C`, `0x0186`, `0x017D`, `0x019A` | sent, same order |
| 5 | `0x008B` `MAP_INIT_START` + `0x008A` `CARTOGRAPHY_DATA` | sent (this client needs it; the capture predates it) |
| 6 | `0x01A0` instance table, ten batches | **sent (2026-10-31)** |
| 7 | `0x0144`, `0x0148`, items | sent on the client's `0x8091` |
| 8 | `0x0196` | sent on the client's `0x8088` |
| 9 | PvP streams | sent |
| 10 | `0x0094` with the reached instances in the fifth list | **sent (2026-10-31)** |
| 11 | `0x0099`, `0x01AC`, manifest `0x0199`/`0x0198`, `0x00F2`, `0x018E` | sent |
| 12 | **the agent array: 21 or 42 bodies with their per-agent streams** | **not sent - one agent, the player** |
| 13 | quests, guild, titles | **not sent** |

## The explorable: one field decides which load the client runs (measured 2026-10-31)

`is_explorable` in the redirect is not a label a client stores. It selects **which load the client runs**, and
an explorable's load is a different procedure from an outpost's. Measured on the patched client against this
world, with one field moved and nothing else (`ASURA_TRAVEL_EXPLORABLE`,
`crates/asura-world/src/travel.rs`), over the same walk through the same portal:

| what the redirect carried | destination | redirect to the players block |
|---|---|---|
| `is_explorable 1` - the destination's own kind | 146 Lakeside County, explorable | **5138 ms** |
| `is_explorable 0` - forced | 146, the same walk | **849 ms** |
| `is_explorable 0` - forced | 148 Ascalon City, outpost | 849 ms |

The client's own clock agrees: in `recordings/client/feature/travel/explorable-flag-on-20261031-223636.jsonl`
the load with the flag at 1 spends **4984 ticks (7.0 s)** between its spawn request and its players request,
and the crossing back into the outpost in the same file spends 640 ticks (0.90 s). The live figure for that
step, over 36 loads in five captures, is 0.507-1.015 s, median 0.763 s.

**Why the flag costs 4.3 s here and nothing on a live server.** What is measured is that the flag selects a
load path, and that both of this world's paths are fed empties: 16 bytes of unexplored grid
(`crates/asura-world/src/cartography.rs`), a party of one, no agents at all, no quests, guild or titles. What
is **not** established is which of those empties the 4.3 s is spent on, and the wire argues against the
obvious answer: the client asks for the players block - the message that carries the population - only
*after* the wait (`0x0196` at tick 291072750, `0x8090` at 291077734 in the capture above), so the population
cannot be what it waits for. It asks for nothing at all during the wait, and neither server sends anything
in it. The wait is therefore inside the client's own load of the explorable, and the candidates are the
structures it builds before it asks: the map view from the grid, and the place itself.

A live server does not pay it: the same step there is 0.76 s with the same flag.

**The experiment that names it, and it needs no data.** `tools/context-dump.py` reads the client's contexts
without elevation. A dump taken before the crossing, one about a second after the spawn point (inside the
wait) and one after the load, in both modes, is what tells the empties apart: whichever context is being
worked on during the wait is the one the flag's path is waiting for. Until that is run this section names the
path, not the cause.

**What that makes this table.** Row 9 is no longer only an instance-completeness gap: it is what an explorable
is missing before the flag can honestly say 1 again, together with rows 10 to 12 and the grid. It is not yet
proven to be the cause of the 4.3 s - see the paragraph above - and the population work should not start until
the context dump says which structure the wait is spent on.

**The default, and what it costs.** `is_explorable` is sent as **0 for every travel**
(`crates/asura-world/src/travel.rs`, `explorable_flag_for`). That removes the wait and tells the client an
explorable is an outpost: no minimap for the map, no henchman or party behaviour, no explorable rules.
`ASURA_TRAVEL_EXPLORABLE=auto` restores the destination's own kind, which is what a live server sends and what
this default becomes once an explorable carries the initialisation this table says it is missing; `=1` forces
1 for a measurement. So this is a recorded deviation from live, taken 2026-10-31 so that travelling works, and
the world's startup line states which value a run used.

**What was ruled out** (all measured 2026-10-31 on the patched client, one variable at a time, each with the
5.1 s unchanged): the four withdrawer messages of the travel burst, restored and timed; the world clock running
inside the load, 49 ticks before the gate and 0 after it; the exploration grid and the instance table, removed
from the unasked block entirely (`ASURA_LOAD_EXTRA=off`, a measurement switch); and the database, which shows no
stall (a 400-iteration probe, the slowest round trip 156 ms). The wait is therefore inside the client's own
explorable load and its cause is still missing; the context dump named above is the measurement that finds it.

**What is not settled.** Live does not treat the field as a pure function of the destination either:
`recordings/client/feature/character_creation/maps-tour3.jsonl` carries 0, 1 and 0 for destination 29. So the
field's live rule is measured only for the pre-Searing redirects, where it follows the kind. One destination,
several visits, a live server is the measurement that closes it.

## The gap, and the order it is paid

The order below is the order of *completeness*. The explorable's 4.3 s wait is not proven to sit in any of
these rows - the section above says why, and the context dump is the measurement that names it - so the
population work is aimed by that dump rather than by this list.

1. **The agent array (live row 9).** The largest gap and the one that changes what the client builds for an
   instance: every body with its name, model, properties, weapons, profession, scale and effects. Source:
   `pre-searing-load-3.jsonl`, message by message, 42 creatures in that outpost. **It is also the one an
   explorable's load waits for**: with `is_explorable` at 1 and no agents the client falls back after 4.3 s
   (the section above), so this row is now a travel requirement and not only an instance-completeness one.
2. **Quests, guild and titles (rows 10 to 12).** Each is its own structure; a live instance sends them
   even for a character with none (empty lists and zero rows), because an unsent structure is not an empty
   one.
3. **The `0x0196` tail.** The reference zeroes the last eight bytes (`GameMsg.h:346`); a live server fills
   them with two dwords that change per visit. Unmeasured meaning, recorded here as a gap rather than
   guessed at.
4. **The client's own initialization messages we do not answer:** `0x8092` (120 bytes, still `Unnamed`)
   and `0x8093` (4 bytes, `{u16 instance}`, now named `GAME_CMSG_INSTANCE_QUERY` and `Observed`,
   `crates/asura-protocol/src/packets.rs:480`), sent 25 times each in one session while browsing the world
   map. Both are unhandled. A live capture of the *retail* build does not contain them, so what they are
   owed has to be measured against this client, not copied.

## How this document is used

- A new feature starts by finding its row here and asking what fills it.
- A message added to an instance sends the whole set its row names, not the one field the feature wanted.
- The row that changes says so in `docs/03-protocol.md` with the measurement behind it.
- When the whole table matches, the initialization is in parity, and that is the state the portal question
  is asked from.
