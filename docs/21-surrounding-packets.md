# 21 - The surrounding packets: what a live stream carries that this tree ignores

Analysis, and partly a change: the first ranked item below (the map table, `0x01A0`) and the portal
touch are in the tree now; the population and the remaining names are not (`docs/22-world-model.md`,
`docs/28-headers-to-name.md`).

The owner's direction, 2026-10-30:

> "there seem to be a lot of information that is pushed on account handling and other on map handling
> and login ... all this issues we have been having are because when we try to add a functionality, we
> ignore the surrounding non important packets, but those packets are the ones that we need to identify
> and put to use now"

The cartography work is the proof he is describing: the mission map and the world map did nothing, no
client message appeared, and the cause was a message earlier in the load that we never sent
(`docs/03-protocol.md` 11.4, `crates/asura-world/src/cartography.rs`). The rule this document produces:
**before a feature is written, inventory every message of the flow it lives in, and name the ones we are
not sending.**

## Method

Two sides are joined here, and only measured numbers appear:

- **the client's view of a live server**, from `recordings/client/` (`recordings/index.jsonl`), produced
  by `tools/sniff-capture.py`;
- **our side**, from `logs/world-8001.log` (the trace in `logs/`, not the console:
  `crates/asura-supervisor/src/process.rs:41`) and `crates/asura-protocol/src/packets.rs` for the names and
  the status column.

**The sniffer reports the client's struct sizes, not the wire sizes** (`docs/03-protocol.md` 5e), so the
comparison below is message by message and never size against size. A live row that is 2 bytes larger
than ours is the four-byte header; one that is 17 larger (`0x0020`) is struct padding.

## Flow A - entering a map (live capture of map 148)

`recordings/client/feature/maps/pre-searing-load-3.jsonl` (map 148, `0x019A` carries `0x94`) against a
session of our world on the same map (`logs/world-8001.log`, from the `== world-8001: started ==` at
line 10797; our counts cover the whole session, which includes ten minutes of standing still).

| header | our name | our status | live | ours | verdict |
|---|---|---|---|---|---|
| `0x000C` | `PING_REQUEST` | Emitted | 3 | 78 | 1:1 |
| `0x000D` | `PING_REPLY` | Emitted | 3 | 1 | 1:1 |
| `0x0018` | `PVP_UPDATE_UNLOCKED_HEROES` | Emitted | 1 | 1 | 1:1 |
| `0x001A` | `PVP_ITEM_STREAM_ADD` | Emitted | 62 | 387 | 1:1 |
| `0x001D` | `PVP_UPDATE_UNLOCKED_SKILLS` | Emitted | 1 | 1 | 1:1 |
| `0x001E` | `WORLD_SIMULATION_TICK` | Emitted | 492 | 3859 | 1:1 |
| `0x0020` | `WORLD_CREATE_AGENT` | Emitted | 42 | 1 | **population missing** |
| `0x0021` | `WORLD_REMOVE_AGENT` | Emitted | 1 | 0 | never sent |
| `0x0025` | `AGENT_UPDATE_DIRECTION` | Emitted | 2 | 0 | never sent |
| `0x0029` | `AGENT_MOVE_TO_POINT` | Emitted | 35 | 0 | not sent for other agents |
| `0x002A` | `AGENT_UPDATE_DESTINATION` | Known | 1 | 0 | never sent |
| `0x002B` | `AGENT_UPDATE_SPEED` | Emitted | 1 | 0 | never sent here |
| `0x002E` | `AGENT_UPDATE_ROTATION` | Known | 9 | 0 | never sent |
| `0x003C` | `GAME_SMSG_PLAYER_UPDATE_FLAGS` | Emitted | 29 | 1 | sent once per load, the player's own flags (`crates/asura-world/src/player.rs`) |
| `0x003E` | `AGENT_ALLY_DESTROY` | Known | 1 | 0 | never sent |
| `0x0048` | `AGENT_DISPLAY_CAPE` | Emitted | 23 | 1 | only for the player |
| `0x004C` | `QUEST_DESCRIPTION` | Known | 1 | 0 | never sent |
| `0x0053` | `QUEST_ADD_MARKER` | Known | 1 | 0 | never sent |
| `0x0056` | `NPC_UPDATE_PROPERTIES` | Known | 14 | 0 | never sent |
| `0x0057` | `NPC_UPDATE_MODEL` | Known | 13 | 0 | never sent |
| `0x0059` | `PLAYER_CREATE` | Emitted | 25 | 1 | only for the player |
| `0x005A` | `PLAYER_DESTROY` | Emitted | 1 | 0 | never sent |
| `0x006B` | `POST_PROCESS` | Known | 1 | 0 | never sent |
| `0x006D` | `NPC_UPDATE_WEAPONS` | Known | 19 | 0 | never sent |
| `0x006E` | `UPDATE_AGENT_VISUAL_EQUIPMENT` | Emitted | 23 | 1 | only for the player |
| `0x009A` | `AGENT_UPDATE_SCALE` | Known | 10 | 0 | never sent |
| `0x009B` | `AGENT_UPDATE_NPC_NAME` | Known | 22 | 0 | never sent |
| `0x009F` | `AGENT_PROPERTY_UPDATE_INT` | Emitted | 82 | 3 | only for the player |
| `0x00A2` | `AGENT_PROPERTY_UPDATE_FLOAT` | Emitted | 4 | 1 | only for the player |
| `0x00A6` | `AGENT_UPDATE_PROFESSION` | Known | 31 | 0 | never sent |
| `0x00B0` | `PLAYER_UPDATE_PARTY_SIZE` | Emitted | 30 | 0 | sent only on a join |
| `0x00B1` | `PLAYER_UPDATE_PARTY` | Emitted | 29 | 0 | sent only on a join |
| `0x00F0` | `AGENT_INITIAL_EFFECTS` | Emitted | 42 | 1 | only for the player |
| `0x00F3` | `TITLE_RANK_DATA` | Known | 3 | 0 | never sent |
| `0x00F4` | `TITLE_RANK_DISPLAY` | Known | 8 | 0 | never sent |
| `0x0118` | `GUILD_PLAYER_ROLE` | Known | 1 | 0 | never sent |
| `0x011F` | **unnamed** | Unnamed | 20 | 0 | **not named, never sent** |
| `0x0121` | `GUILD_GENERAL_INFO` | Known | 14 | 0 | never sent |
| `0x012E` | `GAME_SMSG_LOCATION_TAG` | Inferred | 1 | 0 | named from the source reading, never sent |
| `0x012F` | **unnamed** | Unnamed | 1 | 0 | **not named, never sent** |
| `0x013A` | `ITEM_UPDATE_CUSTOMIZED_NAME` | Known | 2 | 0 | never sent |
| `0x013E` | `ITEM_MOVED_TO_LOCATION` | Emitted | 57 | 7 | 1:1 shape, fewer items |
| `0x013F` | `INVENTORY_CREATE_BAG` | Emitted | 6 | 6 | 1:1 |
| `0x0144` | `ITEM_STREAM_CREATE` | Emitted | 1 | 1 | 1:1 |
| `0x0148` | `ITEM_SET_ACTIVE_WEAPON_SET` | Emitted | 1 | 1 | 1:1 |
| `0x015A` | `ITEM_SET_PROFESSION` | Emitted | 2 | 6 | 1:1 |
| `0x015E` | `GAME_SMSG_ITEM_LOW_DETAIL` | Observed | 118 | 0 | named, never sent |
| `0x0161` | `CREATE_NAMED_ITEM` | Emitted | 76 | 8 | 1:1 shape, fewer items |
| `0x017C` | `INSTANCE_LOAD_HEAD` | Emitted | 1 | 1 | 1:1 |
| `0x017D` | `INSTANCE_LOAD_PLAYER_NAME` | Emitted | 1 | 1 | 1:1 |
| `0x0186` | `INSTANCE_PLAYER_DATA_START` | Emitted | 1 | 1 | 1:1 |
| `0x0196` | `INSTANCE_LOAD_SPAWN_POINT` | Emitted | 1 | 1 | 1:1, and the file id agrees |
| `0x019A` | `INSTANCE_LOAD_INFO` | Emitted | 1 | 1 | 1:1 |
| `0x01A0` | `GAME_SMSG_INSTANCE_TABLE` | Emitted | 10 | 10 | sent: ten batches in every load block (`crates/asura-world/src/map.rs:432-437`) |
| `0x008B` `0x008A` | `MAP_INIT_START`, `CARTOGRAPHY_DATA` | Emitted | 0 | 1 + 1 | sent by us, absent from this capture |
| `0x0094` | `MAP_UPDATE_UNLOCKED_LIST` | Emitted | 0 | 1 | sent by us, absent from this capture |
| `0x0099` `0x01AC` `0x0198` `0x0199` `0x00F2` `0x018E` | map update, ready, manifest, loaded | Emitted | 0 | 1 + 1 + 2 + 4 + 1 + 1 | sent by us, absent from this capture |

Two readings of the bottom two rows, and the difference matters: either this capture began after those
messages had already gone by, or the retail build puts those roles on other headers. Evidence for the
first reading: the file's first record is already `0x019A` at `t = 24.454`, so the capture starts inside
the load and proves nothing about what preceded it. Evidence for the second: the character-creation
capture does contain two `0x008B` and **no `0x008A` at all**
(`recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`). That
anomaly is unresolved and is listed as an open question, not as a defect.

The 148 spawn point agrees with ours on the number that matters: the live `0x0196` carries
`0x0001B97D` = 113021, which is exactly the `map_file_id` our world sends for map 148
(`logs/world-8001.log`, `joined map 148 (file id 113021)`).

## The map table: `0x01A0`, decoded

Ten records in the load of map 148, each a `u32` count of 16 followed by 16 pairs of
`(map id, u32 value)`, 132 bytes each (four-byte header included):

| record | pairs (map id -> value) |
|---|---|
| seq 751 | 148->`BB14F539` 145->`F15FBD2A` 146->`140AB7CE` 147->`4106ABB8` 149->`53E130C4` 150->`53E0B8E1` 151->`4C94ECA3` 160->`5808F8FE` 161->`127956FA` 162->`3252A63F` 163->`B9642F2E` 164->`8302BE3A` 165->`BAC5F32C` 166->`D50F5EEF` 779->`2ECE130E` |
| seq 763 | 877, 878, 879, 880, 881, 882, 4, 5, 6, 52, 65, 167, 168, 170, 171 |
| seq 766 | 173 to 188, then 208 |

and seven more, in order, to `seq 791` (count 3, ids 598 and 599). Together: **137 pairs over 147 map
ids, running up to 877** (`crates/asura-protocol/src/game/instance_table.rs:10-14` says the same), which
is the number the reference's manifest calls `MapId_Count`
(`crates/asura-world/src/map.rs:464`) - the sentinel for "every map up to here".

The second column is not unique per map: `0x197A39CC` belongs to 65, 318, 823 and 339; `0x7082A801` to
182, 319, 824 and 340; `0xBF665D0E` to 180, 320, 826 and 341. Groups of four maps share a value, so the
value is a **region or continent of the map, not the map itself** (a guess, and it is what the ids say,
nothing more).

**This message is sent now**: ten batches in every load block
(`crates/asura-world/src/map.rs:432-437`), because a client that is never told which map ids exist has no
table to resolve an exit against - the shape of the cartography failure.

## The unnamed rows, with their measured shape

Names below the line are **guesses and say so**; the columns are measurements.

| header | records | size (client) | shape | what it looks like | confidence |
|---|---|---|---|---|---|
| `0x003C` | 29 | 16 | `header, agent id, u32, u32 = 7` | per player agent, right after its create: `0x35`->0, `0x14`->5, `0x2B`->4, `0x3A`->5 | a per-agent number that is not profession; a guess |
| `0x011F` | 20 | 136 | mostly zero dwords, a three-unit UTF-16 run `G D H`, then `0x28, 0xF0, 0x3F, 0x3F, 2, 0x16, 0x5F` | a fixed-size record with a short label and a run of small numbers | unknown; a guess would be wrong |
| `0x012E` | 1 | 8 | header only | a toggle with no payload | unknown |
| `0x012F` | 1 | 556 | zeros, one `0xF1`, then `fa 00 6e 00 20 00 65 00 73 00 74 00` | one non-zero dword and one counted string | unknown |
| `0x015E` | 118 | 40 | nine dwords: `id, 0x29x, n, 0x14, m, 0xE7, k, 0x20xxxxxx, 9` with `0x14`, `0xE7`, `9` constant | per item: five varying numbers around three constants | a per-item property message |
| `0x01A0` | 10 | 132 / 28 | counted pairs, see above | **the map table: map id -> region** | high, and acted on below |

`0x015E` is the largest single unknown in a load (118 records against 76 items), and it is the one to
decode first on the item side.

## Flow B - a travel, 117 of them, and what the world must produce

`recordings/client/feature/character_creation/maps-tour3.jsonl` is 585 records and **five headers**:
`0x01A6` 117, `0x0099` 117, `0x017C` 117, `0x019A` 117, `0x0196` 117. Every travel is exactly that
cycle, on the game connection, and the capture contains **zero client-to-server records** - the request
that caused each travel went on the auth connection, which the sniffer did not see.

So the world side of a travel is fully specified by data we hold: the redirect (`0x01A6`, host[24] then
`map_token, region, map_id, is_explorable, player_token, 0`), then `0x0099` with the destination, then the
new connection's head. Our `travel::teleport` already produces the first two; `maps-tour3.jsonl` seq 16
is a travel with `is_explorable = 1`, so a live server does place a client in a non-outpost.

## Flow C - the character-creation instance

`character_creation-live-necro_factions.jsonl` (1109 records, 19 agent creates) and
`character_creation-live-warrior_prophecies.jsonl` (196 records, no agents). Same population pattern as
flow A, plus the creation exchanges this tree already answers.

## What the instrument answers, and what only a client can

`docs/adr/0013-py4gw-stealth-is-our-link-to-the-client.md` is the instrument, and its catalogue already
names the contexts the questions need. No offset, pattern or value from it belongs in this tree; the
names below are what to *read* with it:

| question | context to read |
|---|---|
| is the outpost gate an agent? | the agent table: every entry's type, model, name and position, live, next to the gate |
| what does the client believe about the map we sent? | the instance info: map id, file id, the values our `0x019A`/`0x0196` feed |
| is the map table ours to send? | the same instance info after a `0x01A0` arrives versus without one |
| what arms an exit? | the gameplay context, sampled before and after walking into the gate |

## The ranked next changes, each with its source

1. **~~Send `0x01A0`, the map table.~~ Delivered**: ten batches, 137 pairs over 147 ids, in every load
   block (`crates/asura-world/src/map.rs:432-437`); the client's map and mission surfaces open with it.
2. **Send the population.** Source: the 42 agent creates and their per-agent streams in the same
   capture. Accepts it: an outpost looks like the live one, and the gate question becomes one walk.
3. **Name the rest of the unnamed rows** from the same capture and move them off `Unnamed`: `0x011F`,
   `0x012F` and `0x8092` are what is left of this page's list (`docs/28-headers-to-name.md`).
4. **~~The portal touch.~~ Partly delivered**: a doorway a walk reaches starts the transfer
   (`crates/asura-world/src/travel.rs:364-433`); the `0x80C1` arm still only logs (`travel.rs:154-166`).

Items 1 and 2 change what a client sees and need the client to accept them; item 3 changes nothing on the
wire. None of them is a guess about behaviour: each reproduces messages a live server sent, in the order
it sent them.
