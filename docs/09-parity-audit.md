# 09 - GWPS parity audit

> **History.** The tooling named below was retired when parity closed (docs/16); nothing here is runnable.
> The verdicts are the 2026-10-28 snapshot. Where a row says a behaviour is a stub, missing or shaped one
> way, `docs/07-status.md` and `MILESTONES.md` hold the answer as the tree stands now; several rows here
> are superseded (`ACCEPT_EULA` writes the version, character creation is delivered, `/tp` is implemented).

The question this answers: **for every behaviour GWPS has, does ASURA have it, and is it the same
behaviour?** Nothing here is a plan to rewrite ASURA like GWPS. The reference is
`C:\Users\Apo\Documents\GitHub\PreSearing` (read-only, never modified); the measure is what GWPS
*observably does*: the messages it puts on the wire, the lines it prints, and the rows it writes.

Citations are `File.c:line` inside `PreSearing\code\OpenTyria` for GWPS and `crate/src/file.rs:line`
for ASURA, so every claim below can be checked rather than believed.

## How to read the verdicts

| Verdict | Meaning |
|---|---|
| **1:1** | Same wire messages and same printed lines for the same input. |
| **1:1 (effect)** | Same observable outcome; the internals differ (e.g. ASURA answers from Postgres where GWPS keeps an in-memory index). Nothing a player or a log reader can tell apart. |
| **AHEAD** | ASURA does something GWPS does not. Always labelled, never called parity. |
| **DEVIATION** | Both answer, differently, and the difference is written down and defended. |
| **BEHIND** | GWPS does something a client can observe that ASURA does not do. |
| **STUB** | The handler exists on both sides and neither does anything a client can see. |
| **DEAD** | GWPS's own handler is unfinished (`TODO`, returns early, sends nothing) - there is nothing to match yet. |

A missing feature is only a gap if GWPS *has* it. Where GWPS is unfinished, ASURA matching GWPS would
mean copying a dead end; those rows are marked **DEAD** and are not work items.

---

## Scoreboard

| Domain | Behaviours audited | 1:1 (incl. effect) | AHEAD | DEVIATION | BEHIND | STUB/DEAD |
|---|---|---|---|---|---|---|
| Auth connection | 15 | 14 | - | - | - | 1 |
| Game connection dispatch | 18 | 12 | - | 2 | 4 | - |
| Movement & agents | 7 | 5 | - | 2 | - | - |
| Chat, commands & system messages | 8 | 7 | - | - | 1 | - |
| Items & inventory | 7 | 3 | 3 | 1 | - | - |
| Character creation | 5 | - | - | - | 5 | - |
| World hosting & map config | 6 | 5 | 1 | - | - | - |
| Persistence & schema | 6 | 3 | 3 | - | - | - |
| Tooling & observability | 6 | 3 | 3 | - | - | - |
| **Total** | **78** | **52** | **10** | **5** | **10** | **1** |

Reading it: **52 of 78 behaviours are indistinguishable from GWPS**, ten are things GWPS cannot do at
all, five differ on purpose and are argued below, ten are behind, and one is a stub on both sides.

The ten BEHIND rows are **two** work items, not ten. Nine of them are character creation: the four
`CHAR_CREATION_*` rows in #2 and the five in #6 describe the same five handlers. The tenth is `/tp`,
which needs one message pair this server cannot yet originate.

Two things this table deliberately does *not* count: GWPS's pathfinding and party modules, which are
library code with no dispatch entry point (see #12), and the five creation rows twice over.

---

## 1. Auth connection

GWPS dispatch: `AuthSrv.c:1728-1784`. ASURA dispatch: `crates/asura-auth/src/lib.rs:348-578`.

| # | Message | GWPS behaviour | ASURA state | Verdict |
|---|---|---|---|---|
| 1 | `SEND_COMPUTER_INFO` | Stores the reported user name and PC name on the connection (`AuthSrv.c:1729-1734`); sends nothing. | Ignored (`lib.rs:352`); the frame is consumed structurally before framing. Nothing observable reads the stored names in GWPS either. | 1:1 (effect) |
| 2 | `SEND_COMPUTER_HASH` | Decodes the hash, then answers `SESSION_INFO` with a salt (`AuthSrv.c:1735-1738`). | Same: random salt, `SESSION_INFO` (`lib.rs:359-373`). | 1:1 |
| 3 | `PORTAL_ACCOUNT_LOGIN` | `AuthSrv_HandlePortalAccountLogin` (`AuthSrv.c:774`): account/password against the DB, then the response the client gates on. | `answer_login` (`lib.rs:375-386`) against the same-shaped schema. | 1:1 (effect) |
| 4 | `ASK_SERVER_RESPONSE` | Answers the request with status 0 (`AuthSrv.c:837-841`), logs the header. | Answers status 0 (`lib.rs:389-403`); the header is only logged under `ASURA_TRACE`. | 1:1 (effect) |
| 5 | `SET_PLAYER_STATUS` | Validates the status and stores it on the connection (`AuthSrv.c:844-864`); nothing reads it and nothing is sent. | No-op (`lib.rs:556`). | 1:1 (effect) |
| 6 | `CHANGE_PLAY_CHARACTER` | Re-reads the character list from the DB, finds the name, remembers the index, answers 0 / network error (`AuthSrv.c:866-897`). | Reads characters from the DB, finds the name, records the selection on the session and in `sessions`, answers 0 / auth error (`lib.rs:414-468`). | 1:1 (effect) |
| 7 | `REQUEST_GAME_INSTANCE` | `AuthSrv_HandleRequestGameInstance` (`AuthSrv.c:899`): picks a live world, sends `TRANSFER_GAME_SERVER_INFO`, then success. | `travel::answer` (`crates/asura-auth/src/travel.rs:96`) probes worlds for reachability, sends the same two messages. | 1:1 |
| 8 | `HEARTBEAT` | Answers a server heartbeat (`AuthSrv.c:1263-1268`). | Answers `messages::heartbeat(16)` (`lib.rs:407-410`). | 1:1 |
| 9 | `DISCONNECT` | Logs and closes the connection (`AuthSrv.c:1758-1761`). | `Ok(false)` - the socket closes (`lib.rs:578`). | 1:1 |
| 10 | `ACCEPT_EULA` | Nothing (`AuthSrv.c:1270-1275`). | Records the accepted version, logs it (`crates/asura-auth/src/lib.rs:587-601`, `account.eula_accepted` from `0012_account_eula.sql`). | 1:1 (effect) |
| 11 | `ADD_ACCESS_KEY` | `AuthSrv_HandleAddAccessKey` (`AuthSrv.c:1277-1290`) logs the key text and keeps nothing. | Counts the key's units and keeps nothing (`lib.rs:485-509`); nothing is logged unless `ASURA_TRACE` is on. | 1:1 (effect) |
| 12 | `DELETE_CHARACTER` | `AuthSrv_HandleDeleteCharacter` (`AuthSrv.c:1292`): deletes when the reasons match, answers per reason. | `db_login::delete_character` with the same reason codes (`lib.rs:511-554`). | 1:1 (effect) |
| 13 | `UPDATE_CHARACTER_SETTINGS` | Answers status 0, stores nothing (`AuthSrv.c:1323-1329`). | Answers status 0 (`lib.rs:470-484`). | 1:1 |
| 14-15 | `SETTING_UPDATE_SIZE`, `SETTING_UPDATE_CONTENT` | `AuthSrv_HandleSettingUpdate` (`AuthSrv.c:656`): logs and answers status 0. | Answer status 0 (`lib.rs:470-484`). | 1:1 (effect) |

**Domain verdict: complete.** All fifteen handlers have a counterpart with the same observable result;
the two effect-level differences (stored computer name, logged access key) are not observable to a
client and are recorded here rather than "fixed" by inventing behaviour.

---

## 2. Game connection dispatch

GWPS dispatch: `GameSrv.c:2007-2074`. ASURA dispatch: `crates/asura-world/src/dispatch.rs:26-101` (22 `GAME_CMSG_` arms).

| # | Message | GWPS behaviour | ASURA state | Verdict |
|---|---|---|---|---|
| 1 | `DISCONNECT` | Closes the connection (`GameSrv.c:1999`). | `Ok(false)` (`lib.rs:1105`). | 1:1 |
| 2 | `HEARTBEAT` | Ignored (`GameSrv.c:2014-2015`). | Ignored (`lib.rs:1194`). | 1:1 |
| 3 | `PING_REPLY` | Ignored (`GameSrv.c:2016-2017`). | Ignored (`lib.rs:1191`). | 1:1 |
| 4 | `PING_REQUEST` | Answers `PING_REPLY` (`GameSrv.c:1978`). | Answers `ping_reply` (`lib.rs:863-866`). | 1:1 |
| 5 | `INSTANCE_LOAD_REQUEST_SPAWN` | The agent's position and the map file id (`GameSrv.c:1566`). | `spawn_block` (`lib.rs:838-847`, block built at `lib.rs:767`). | 1:1 |
| 6 | `INSTANCE_LOAD_REQUEST_PLAYERS` | The player block ending on `INSTANCE_LOAD_FINISH` (`GameSrv.c:1600`). | `player_block` (`lib.rs:851-860`, built at `lib.rs:1602`). | 1:1 |
| 7 | `INSTANCE_LOAD_REQUEST_ITEMS` | Item stream + inventory + weapon slots + gold + faction caps + character info (`GameSrv.c:1660-1690`). | `items_block` (`lib.rs:825-834`, built at `lib.rs:641`). | 1:1 |
| 8 | `CHAR_CREATION_REQUEST_PLAYER` | Warrior default, then a full player-data start/stop block (`GameSrv.c:1691-1719`). | The player-data block with the reference's own creation numbers (energy 20, health 100, property 64 zero) and the attributes (`lib.rs`, `0x8089`) | 1:1 |
| 9 | `CHAR_CREATION_REQUEST_ARMORS` | PvP skills, PvP heroes, PvP items, account features (`GameSrv.c:1721-1739`). | The PvP half as the reference sends it for an account with none: empty skill and hero streams, its PvP item table and the account features (`0x808A`) | 1:1 |
| 10 | `CHAR_CREATION_CHANGE_PROF` | Re-equips the profession's defaults from `GetDefaultEquipments` (`GameSrv.c:1786-1845`, table in `GmDefaultArmors.c`). | The same table, generated from `GmDefaultArmors.c` (`tools/gen-default-equipment.py`: 32 rows, 179 items) | 1:1 |
| 11 | `CHAR_CREATION_CONFIRM` | Creates the character's bags **and writes the item rows once**, then enters the map (`GameSrv.c:1855-1976`; `GameSrv_CreatePlayerBags` at `GameSrv.c:1230`). | The same: the row with the reference's creation defaults, bags and item rows in one transaction, then `CHAR_CREATION_SUCCESS` with its settings block (`0x808B`, `asura_db::inventory::create_character`) | 1:1 |
| 12 | `ITEM_UPDATE_EQUIPPED_COLOR` | Declared as a handler but a no-op: it ignores all three parameters (`GameSrv.c:1847-1853`) | No arm: the fallthrough frames it and ignores it, naming it under trace (`lib.rs:1200-1208`). `docs/07-status.md` lists it as a missing arm because it counts dispatch entries, not behaviour | 1:1 (effect) |
| 13 | `SEND_CHAT_MESSAGE` | `GameSrv_HandleChatMessage` (`GmChat.c:279-306`). | `0x8064` (`lib.rs:982-1102`). | 1:1 (effect) |
| 14 | `MOVE_TO_COORD` | Pathfinds and broadcasts the first waypoint (`GmAgent.c:371-398`). | The same finder, the same first waypoint, the same refusal (`lib.rs`, `0x803E`; docs #3). | 1:1 |
| 15 | `POSITION_UPDATE` | Adopts / clamps / refuses by a speed budget (`GmAgent.c:409-462`). | `adopt_report` with the same budget (`lib.rs:909-932`). | 1:1 (effect) |
| 16 | `CANCEL_MOVEMENT` | Tick, cancel, broadcast `AGENT_STOP_MOVING` (`GmAgent.c:533-545`). | `agent_stop_moving` (`lib.rs:936-945`). | 1:1 |
| 17 | `LAST_POS_BEFORE_MOVE_CANCELED` | Accepts a short same-plane correction, refuses a long one, sends nothing (`GmAgent.c:547-563`). | `adopt_correction` (`lib.rs:950-972`). | 1:1 (effect) |
| 18 | `DROP_ITEM` | Finds the item, refuses count < quantity with a `TODO`, else creates an item agent and returns (`GmInventory.c:236-300`). | Deducts for real, persists, prints, sends nothing (`lib.rs:1120-1129`, `drop_item` at `lib.rs:1218`). | DEVIATION (deliberate - see #5) |

---

## 3. Movement & agents

GWPS: `GmAgent.c` (564 lines) plus `GmPaths.c` (A*, 1057 lines) and `GmPathHeap.c`.
ASURA: `crates/asura-world/src/lib.rs` arms + `Player::adopt_report`/`adopt_correction`, plus
`crates/asura-world/src/gwdat.rs` + `fa_compress.rs` + `map_data.rs` + `pathfind.rs` for the graph and the
search over it.

The navmesh is no longer the unknown it was: `Gw.dat`'s index, its decompressor, `GmMapDataImport.c` and
`GmPaths.c`'s A* are all ported and measured against the reference's own code (docs/10-parity-queue.md,
item D) - 96 walks, 41 of them found, with every waypoint identical to the last bit except the one case the
reference answers differently on different runs. The finder is also **adopted**: `MOVE_TO_COORD` pathfinds,
answers with the first waypoint, and refuses what it cannot reach by broadcasting where the agent is, while
the world tick walks the agent along the path (`lib.rs`, `agent.rs`). The walk verdict below reflects that.

| Behaviour | GWPS | ASURA | Verdict |
|---|---|---|---|
| Walk to a coordinate | A* over the map's path graph; the answer is the **first waypoint of its own path**, and the refusal is a tick, a cancel and a position broadcast (`GmAgent.c:371-398`) | The same finder (`pathfind.rs`, `GmPaths.c` ported and compared case by case), the same first waypoint; a refused walk is cancelled and **nothing is sent**, measured (`crates/asura-world/src/agent.rs:722-743`) | DELIBERATE DEVIATION, owner ruling open (`crates/asura-world/tests/load.rs:3169`); also the reference divides by zero when a walk's destination is where the agent stands, which poisons its position with NaN, and this keeps the agent where it is |
| World tick | Every `GAME_SRV_MAX_TIME_BETWEEN_WORLD_TICK_MS`, broadcast + advance movement + regen health/energy (`GmAgent.c:502-531`) | `WORLD_SIMULATION_TICK` on the reference's interval; movement advances `speed_base * delta` along the path (`agent.rs`, `advance`) | 1:1 for movement; regen is not integrated - see the health/energy row below |
| Position reports | Adopted within a speed budget, clamped beyond it, refused across a long plane change (`GmAgent.c:409-462`) | Same three outcomes (`lib.rs:920-931`) | 1:1 (effect) |
| Cancel a walk | `AGENT_STOP_MOVING` after a tick (`GmAgent.c:533-545`) | `AGENT_STOP_MOVING` (`lib.rs:936-945`) | 1:1 |
| Correct after a cancel | Short correction on the same plane taken, long refused, nothing sent (`GmAgent.c:547-563`) | Same (`lib.rs:950-972`) | 1:1 (effect) |
| `/stuck` | `WorldTick` + cancel + broadcast position (`GmChat.c:243-251`) | `AGENT_UPDATE_POSITION` (`lib.rs:1038-1062`); the 500 ms ticker covers the tick | 1:1 |
| Agent health/energy regen | Server-side integration during the tick (`GmAgent.c:524-529`) | Fixed values sent once in `player_block`; no regen integration | DEVIATION - invisible with one player and no combat; recorded, not fixed |

---

## 4. Chat, commands & system messages

GWPS: `GmChat.c` (306 lines), `GmCommands.c` (259 lines), `GmText.c`.
ASURA: `lib.rs:974-1102`, `maps_answer` at `lib.rs:1497`.

The single most important finding of this audit: **GWPS puts no chat text on the wire for anything the
*server* generates.** Every server-side answer funnels through `GameSrv_SendSystemChatMessage`
(`GmChat.c:195-233`) or `GameSrv_SendInvalidCommand` (`GmChat.c:145-180`), and both return early at
`GmChat.c:209-212` / `GmChat.c:150-153` unless `GWPS_CHAT=1`. The reference's own comment
(`GmChat.c:129-138`) says why: sending chat crashed its client twice, and its layouts are still
unsettled. A player's `!`/`#`/`$` line *is* sent (`GmChat.c:259-277`, not gated), and the user has
never seen one appear - in either server.

| Behaviour | GWPS | ASURA | Verdict |
|---|---|---|---|
| Player speaks on `!` | Ungated broadcast of the text + `CHAT_MESSAGE_LOCAL` on `Channel_All` (`GmChat.c:259-277`, `290`) | `chat_message_core` + `chat_message_local(PLAYER_ID, CHANNEL_ALL)` (`lib.rs:1010-1030`) | 1:1 (effect) - bytes on the wire in both, visible in neither |
| Player speaks on `#` / `$` | Same path with `Channel_Group` / `Channel_Trade` (`GmChat.c:293-294`) | Same (`lib.rs:1011-1015`) | 1:1 (effect) |
| Player speaks on `@`, `%`, `"` | Silently ignored (`GmChat.c:291-298`) | Ignored (`lib.rs:1086-1095`) | 1:1 |
| Player speaks on any other channel | Error log, `ERR_BAD_USER_DATA` (`GmChat.c:300-302`) | Prints a refusal line (`lib.rs:1096-1099`) | 1:1 (effect) |
| `/maps` | Chat aliases via the gated sender **plus** an unconditional `log_info` table: `Teleport targets (N aliases, every one with a map config):` then `  /tp %-15s -> map %3d  %s` (`GmCommands.c:113-152`) | The same table, printed, character for character (`maps_answer`, `lib.rs:1497-1529`); the chat half is printed only under trace and never sent (`lib.rs:1063-1078`) | 1:1 |
| Unknown `/command` | "Unknown command" through the gated sender => log line only (`GmChat.c:145-180`) | `world: Unknown command` (`lib.rs:1082`) | 1:1 |
| `/stuck` | Cancel + position broadcast (`GmChat.c:243-251`) | `AGENT_UPDATE_POSITION` (`lib.rs:1038-1062`) | 1:1 |
| `/tp`, `/teleport` | `GameSrv_HandleTeleportCommand` (`GmCommands.c:155-216`): refuses a missing or unknown alias, refuses a map with no config (it would silently fall back to Kamadan, `GmCommands.c:180-190`), refuses "already here", then `GameSrvCtrl_StartPlayerTransferTo` starts a real server transfer. Both refusals go through the gated chat sender, so with the gate off they are log lines. | The same four outcomes, in the same order, with the same lines (`teleport`, `lib.rs`): unknown alias, no config, already here, then a `transfer` row and the reference's redirect. The alias table is the reference's own, generated (`travel_targets.rs`) | 1:1 |
| The alias table `/tp` and `/maps` read | `s_teleport_targets` (`GmCommands.c:32-53`): twenty aliases over twelve maps, two for most (`lakeside` and `lakeside county`) | The same twenty, generated from that table by `tools/gen-map-config.py` and compared row for row by `tools/parity-commands.py` (`docs/13-command-parity.md`). Until it was generated, `/tp` matched the map's **name**, so six of the twenty worked | 1:1 |

### What a teleport is, measured

`/tp` is not a position change; it is a server transfer, and the reference has it working. The wire
evidence, none of which is a guess:

- **`TRANSFER_GAME_SERVER_INFO` (`0x01A6`)**, 46 bytes as GWPS sends it and 52 in the live capture (the
  extra 4 are the client's own 4-byte game header): `{header, host[24], map_token, region, map_id,
  is_explorable, player_token}`. The 24-byte host is a `sockaddr_in`: family (little endian), port (big
  endian), four address bytes, then padding. The two captured packets carry `map_id = 161,
  is_explorable = 1` and `map_id = 165, is_explorable = 0` - which is how the rule "the flag describes
  the *destination*, not the source" was found (`GameSrv.c:1394-1403`).
- **`MAP_UPDATE_CURRENT` (`0x0099`, 12 bytes)** follows in the same millisecond, in 10 of 10 captured
  transitions (`GameSrv.c:1429-1432`).
- **A real client-announced travel begins with `TRAVEL_INIT` (`0x80C1`, 10 bytes on the wire,
  client -> server, two agent ids)** - the message `docs/03-protocol.md` 5f measures; GWPS's `/tp` skips
  it, that is its documented race: the redirect arrives in a map the client never asked to leave, and
  under latency the hop can fail (`docs/08-gm-commands.md`).

Sources: `captures/zone-ref-005829.jsonl` seq 42 and 424 (raw, gitignored), `docs/08-gm-commands.md`
("What a teleport actually does", "Known limitation"), `code/OpenTyria/GameSrv.c:1360-1435`,
`opcodes.h:358`. ASURA already carries the builder and the sizes
(`crates/asura-protocol/src/game/mod.rs:59-61`) and the whole cycle is written down in
`docs/03-protocol.md` #5 - what the world lacks is the ability to *start* one.

---

## 5. Items & inventory

GWPS: `GmInventory.c` (drop handler), `GameSrv_CreatePlayerBags` (`GameSrv.c:1230`), `GmItem`.
ASURA: `crates/asura-db/src/inventory.rs` (rows + moves), `crates/asura-world/src/item.rs` (blocks),
`crates/asura-protocol/src/game/mod.rs` (builders).

| Behaviour | GWPS | ASURA | Verdict |
|---|---|---|---|
| Where items live | In-memory `GmBag`/`GmItem` arrays on the player, written to the DB once at character creation (`GameSrv.c:1230`, called only from `HandleCharCreationConfirm`) | Rows in `bag`/`item` (`crates/asura-db/migrations/0006_inventory.sql`), read at login (`inventory.rs:314`) | AHEAD - ASURA's items survive a restart; GWPS's do not |
| The item block on entering a map | Item stream, one `CREATE_NAMED_ITEM` per item with a **`u8`** modifier count, bags in the reference's own fixed order | Byte-identical block, proven against the capture fixture (`crates/asura-world/src/lib.rs:631` `the_item_block_opens_with_the_stream_and_carries_the_capture`) | 1:1 |
| Move an item to a slot | **No handler.** `GAME_CMSG_ITEM_MOVE` is not in GWPS's dispatch | `0x8072` -> `move_item` -> echo `ITEM_MOVE_TO_LOCATION`, persisted (`item.rs:256` `move_item`, `item.rs:498` `handle_item_move`, `dispatch.rs:75`, persisted in `inventory.rs`) | AHEAD |
| Equip an item | **No handler.** No unequip, trade, split, salvage, identify or use handler either | `0x8030` -> equipped bag + `UPDATE_AGENT_VISUAL_EQUIPMENT` (`item.rs:315` `equip_item`, `item.rs:535` `handle_equip_item`, `dispatch.rs:77`, `equipped_slot` at `mod.rs:200`) | AHEAD |
| Drop an item | Refuses `count < quantity` (`TODO`), never removes the row, creates an agent whose model id is the *item* id (`GmInventory.c:236-300`) | Deducts, persists, prints; sends nothing because both candidate messages crashed a live client and neither is in a capture (`item.rs:187` `drop_item`, `item.rs:477` `handle_drop_item`, `dispatch.rs:73`; `drop_messages` at `item.rs:251` is the shape the crash ruled out) | DEVIATION - deliberately not the reference's dead end |
| Unequip | **No handler.** | `0x804F` logged as unimplemented - its layout carries no item id (`item.rs:555` `handle_unequip_item`, `dispatch.rs:79`) | 1:1 (both do nothing, ASURA says so) |
| Item colour update | Declared, ignores all three parameters (`GameSrv.c:1847-1853`) | No arm; the fallthrough frames it and ignores it (`dispatch.rs:26-101`, the arm that names no handler) | 1:1 (effect) - neither does anything |

**The honest summary of this domain:** GWPS's item support is a load request and a broken drop. Three of
ASURA's item behaviours have no GWPS counterpart at all, so "parity" here means ASURA is ahead and the
gap list has one real entry: the **ground-item message** ASURA cannot send yet because no capture
contains it.

---

## 6. Character creation

GWPS: `GameSrv.c:1691-1976`, `GetBagSlotForItemType` (`GameSrv.c:1741-1784`), `GmDefaultArmors.c`.

| Behaviour | GWPS | ASURA | Verdict |
|---|---|---|---|
| Creation instance | `MapType_CharacterCreation` (11), its own map type in `MapType` | Hosted: the world serves type 11 (`MAP_TYPE_CHARACTER_CREATION`, `main.rs`) and a fresh account reaches the creation screen (`a_character_is_created_in_a_creation_instance`) | 1:1 |
| `CHAR_CREATION_REQUEST_PLAYER` | Warrior by default, then a full player-data block | The player-data block, with the creation path's own three int properties (`0x8089`) | 1:1 |
| `CHAR_CREATION_REQUEST_ARMORS` | PvP skills/heroes/items, account features | The same four blocks (`0x808A`) | 1:1 |
| `CHAR_CREATION_CHANGE_PROF` | Re-equips from a 2858-line generated table | The same table, generated (`default_equipment.rs`, 32 rows / 179 items) | 1:1 |
| `CHAR_CREATION_CONFIRM` | Writes the item rows once, then enters the map | The same, in one transaction, with the reference's creation defaults (`0x808B`) | 1:1 |

This used to be the largest genuine gap - one *feature*, "new characters can be created", wearing five
dispatch names. It closed when the creation instance, the four handlers and the generated armour table
landed; `docs/14-test-plan.md` records T6 as measured, and the creation test asserts the three int
properties the reference's creation path sends rather than the load path's numbers.

---

**The catalogue is data, and it is now generated.** The reference's three tables describe twelve maps a
client can be sent to: `GmCommands.c:32-53` (every map with a config, its aliases and its **`MapType`**),
`GmMaps.h` (the numbers) and `GmMapsConfig.c` (file id and spawn point). `tools/gen-map-config.py` reads all
three and writes `tools/map-config.sql`, which `tools/seed-map.ps1` applies; the battery re-generates with
`--check`. Before that, this repo's seed carried two hand-typed maps and called Lakeside County (146) an
**outpost** - the reference says `MapType_MainExplorable`, and its roadmap records that map as *verified*
loading.

That correction exposed a second liberty: this world refused to host explorables at all, on the theory that
it "cannot spawn anything into one". A world server does not spawn content - it hands the client the blob and
the spawn point, which is what the reference does with 146. Explorable hosting is allowed now (types 0, 1 and
11), and a world hosts **every map in the catalogue** by default: the reference pre-spawns one server per
configured map (`AuthSrv_PreSpawnMapServers`), so `/maps` lists all twelve and `/tp` reaches all twelve
without a flag. `--maps` narrows the set on purpose - a soak, a single-map test - and is never required.

That default was wrong for a long time in a way nobody had written down: `asura-world` without `--maps`
hosted **one** map (146, the constant from the first single-map server), so `/maps` printed one alias and
`/tp` to any other map was refused. The reference has no such state. The regression test
`the_default_map_list_is_the_whole_catalogue_and_tp_reaches_it` holds that line now.

## 7. World hosting & map config

GWPS: `GmMapsConfig.c` (252 lines) + pre-spawned map servers + `CtrlConn` (`GameSrv.c:48-75`,
`1327-1389`). ASURA: `crates/asura-world/src/main.rs`, `crates/asura-db/src/game.rs`.

| Behaviour | GWPS | ASURA | Verdict |
|---|---|---|---|
| Where a map's facts come from | `GmMapsConfig.c`: file id, type, spawn point, spawn plane, capacity, aliases | `map_config` rows + migration 0007's `name` column | 1:1 (effect) |
| Aliases for `/tp` and `/maps` | Config string, spaces collapsed | The reference's own table, generated (`travel_targets.rs`); derived from the map name only for a map the table does not know | 1:1 |
| Refusing a map a client cannot load | `map_file_id == 0` refused | Refused with a reason (`main.rs:161-176`) | 1:1 |
| Refusing explorable areas as hosts | Type check | Same check (`main.rs:170`) | 1:1 |
| Advertising a world | `CtrlConn` message to the auth service; heartbeat keeps it alive | `register_world`, heartbeat task (`main.rs:206-240`) | 1:1 (effect) |
| Starting the map server for a travel | Auth service spawns the map server on demand | Auth service registers/chooses a world; `start-server.ps1` starts the process | AHEAD (process control by script vs by the auth service) |

---

## 8. Persistence & schema

GWPS schema (read at `Db.c:269-360`): sessions, accounts, characters, bags, items, friendships; the
binaries check a schema version. ASURA: migrations 0001-0007 - `schema_version`, `account`, `session`,
`world`, `world_pool`, `characters`, `map_config`, `lease`, `transfer`, `bag`, `item`.

| Table / behaviour | GWPS | ASURA | Verdict |
|---|---|---|---|
| accounts / sessions | Present, same shape (`Db.c:278-287`) | `account`, `session` (`0001`) | 1:1 |
| characters | Present (`Db.c:296`) | Present + `pos_plane` for saved positions (`0006`) | AHEAD (positions persist) |
| bags / items | Present (`Db.c:325`, `Db.c:355`); rows written once at character creation | `bag`, `item`; read per map entry, moved and persisted per action | AHEAD |
| friendships | Table read, but the login response only sends the stream **terminator** (`AuthSrv.c:831`); no entry is ever sent | No table; `friend_stream_end` is sent at the same point in the login reply (`crates/asura-auth/src/lib.rs:698`) | 1:1 (effect) |
| Migrations | A schema version the binaries check | `schema_version` row; the world refuses to start against a stale schema (`main.rs:142-147`) | 1:1 (effect) |
| SQL ownership | Queries scattered through the C code | Only `asura-db` writes SQL (`docs/08`) | AHEAD |

---

## 9. State parity: what each server keeps for a connected player

**This section exists because the first version of this audit did not have it, and that omission hid a
real liberty.** The rest of the document compares dispatch arms and wire messages - what goes in and what
comes out. It never compared **what the two servers hold between messages**, and that is where a missing
capability hides: a server that keeps less cannot do what the other one does, even when every message it
*can* answer is identical.

GWPS keeps twelve fields per player (`GmPlayer.h:21-33`). ASURA keeps the same twelve as the `Player` struct in `crates/asura-world/src/player.rs:18-50`, with the ids allocated per connection (`instance.rs:70`) rather than fixed constants.

| kept | GWPS | ASURA | Verdict |
|---|---|---|---|
| player id | `GmPlayer.player_id` | `Player.player_id`, allocated per join (`instance.rs:70`, `:291`) | 1:1 |
| agent id | `GmPlayer.agent_id` | `Player.agent_id`, allocated with the player id | 1:1 |
| connection | `GmPlayer.conn_token` | the connection is the task; no id | 1:1 (effect) |
| **character id** | `GmPlayer.char_id` | `Player.character` | 1:1 |
| **account id** | `GmPlayer.account_id` - used 63 times on the game side, including every bag and item read (`GameSrv.c:1136`, `GameSrv.c:1174`) and the transfer request (`GameSrv.c:1456`) | `Player.account_id`, kept from the claimed transfer; every bag and item query and the `/tp` transfer name it | 1:1 - it was the one real liberty in this table until `/tp` needed it |
| character row | `GmPlayer.character` (a whole `DbCharacter`: ~44 columns) | the columns a message reads - `appearance`, `primary_profession`, `unlocked_professions`, `gold` (migration 0008) and `level`, `campaign`, `is_pvp`, `secondary_profession`, `helm_status` (migration 0009), plus the position this schema keeps for itself - and the settings blob built from them | **1:1 for what is sent, and it used to be worse**: until migration 0008 the load block sent the reference's *default* character to everyone, and until migration 0009 the login reply's character list sent **one captured 62-byte blob for every character** (a Warrior, level 1, outpost 146, five dyed armour pieces) instead of each character's own |
| account row | `GmPlayer.account` (a whole `DbAccount`) | nothing; read per request | 1:1 (effect) |
| bags | `GmPlayer.bags` (in memory, written once) | `Player.inventory` (rows, reloaded) | 1:1 (effect) |
| campaign | `GmPlayer.char_creation_campaign` | `characters.campaign` (migration 0009), written by the creation confirm and carried in the settings blob | 1:1 |
| profession | `GmPlayer.primary_profession` | `characters.primary_profession`: `PLAYER_UPDATE_PROFESSION` sends the row's, and the creation screen's choice is what the row holds | 1:1 |
| team token | `GmPlayer.player_team_token` (an arbitrary constant) | `PLAYER_TEAM_TOKEN` | 1:1 (effect) |
| party id | `GmPlayer.party_id` | `PARTY_ID` (1) | 1:1 (effect) |

Two conclusions, and they are the answer to "why does this project take liberties":

1. **Almost every difference is storage, not behaviour** - ASURA derives from rows what GWPS caches. That is
   legitimate, but only because it *is* recorded here: the rows are the same rows, read from the same place,
   and nothing a client can see differs. Stated as a rule: **caching nothing is fine; being unable to answer
   is not.**
2. **The one liberty this table used to have - `account_id` - is gone.** GWPS keeps it because work needs it:
   every bag and item query is scoped by it, and the transfer request carries it
   (`GameSrv.c:1450-1472`: `player_id`, `account_id`, `char_id`, `district`). This server dropped it until
   `/tp` needed exactly that, which is when it was kept; the rows above are the record of it.

**What is still not stored, and why that is written down rather than fixed.** The reference's
`DbCharacter` carries about forty-four columns. This server stores the one every message that exists reads:
`appearance`, `primary_profession`, `unlocked_professions`, `gold` (migration 0008) and `level`, `campaign`,
`is_pvp`, `secondary_profession`, `helm_status` (migration 0009), and the armour pieces the settings blob
carries are *derived* from the equipped bag rather than denormalised. The rest - skills, the 1024-bit
unlocked-skill bitmap, the four 256-bit completion bitmaps, skill points, experience - stay out because
nothing writes them (GWPS's own confirm sets them to zero and no handler ever assigns one) so the zeros are
parity; argument and evidence in `docs/10-parity-queue.md`, item B5. A column nothing loads is the same
liberty in a new place, which is the rule this section exists to enforce.

ASURA additionally needs its own `session_id` on the connection. That is **not** parity with `GmPlayer` -
GWPS keeps no session id on the game side; its analogue lives auth-side, on the auth connection
(`AuthConnection.account_id`, `account.current_char_id`), which is what mints the token. ASURA's `transfer`
and `lease` rows are keyed by session, so the session is the thing it must carry to mint one itself.

---

## 10. Tooling & observability

| Behaviour | GWPS | ASURA | Verdict |
|---|---|---|---|
| Packet trace | `GWPS_TRACE=1` | `ASURA_TRACE=1` (`tools/play.ps1 -Trace`) | 1:1 |
| Event log that is not a packet log | `log_info` lines for dispatch and GM answers | Unconditional `eprintln!` event lines (`docs/03` #11) | 1:1 |
| Captures in the repository | `GWPS_TRACE=1` output kept as text | `crates/asura-db/src/inventory.rs:243` (`default_inventory()`) is a capture turned into a fixture | AHEAD (a capture that fails the build when it drifts) |
| Web login gate | `tools/webgate.py` on 6601 | The same script, started by `tools/start-server.ps1` | 1:1 |
| One-command server | Several consoles | `tools/start-server.ps1` / `stop-server.ps1` / `play.ps1` | AHEAD |
| Test battery | None in the reference | `tools/test-battery.ps1` + the workspace suite (`cargo test`) | AHEAD |

---

## 11. ASURA behaviour GWPS does not have (explicitly AHEAD)

Recorded here so nobody later "restores parity" by deleting working features:

1. Items survive a restart, and a drop deducted from a stack is gone after a reconnect.
2. Item moves and equips are handled, persisted and echoed.
3. Character positions persist across sessions (`pos_plane IS NULL` means never saved).
4. `map_config.name`, and a `/maps` table that names the map as well as aliasing it.
5. Integration tests that assert the wire (`the_item_block_from_rows_is_the_capture_byte_for_byte`,
   `maps_prints_its_table_and_puts_nothing_on_the_wire`).
6. The world refuses to start against a stale schema instead of failing later.

---

## 12. Where GWPS is itself unfinished (the "missing" that is not a gap)

These are GWPS rows that look like features and are not. Matching them would be copying a dead end:

1. **Drop with `count < quantity`** - `log_warn("TODO: we must still implement this")`
   (`GmInventory.c:283-288`). The client always sends `count = 1` for a stack, so dropping a stack does
   nothing at all in GWPS.
2. **The item agent** - `item_agent->model_id = item->item_id` (`GmInventory.c:300`), so the thing on
   the ground is drawn as nothing; and the item is never removed from the bag.
3. **All server chat** - gated off by default because it crashed the client twice (`GmChat.c:129-143`).
4. **`GmPaths.c` / `GmPathHeap.c`** - A* with no dispatch entry point; `GameSrv_HandleMoveToCoord` is its
   only caller. (Both are ported here regardless: the refusal is a behaviour a player notices, so the queue
   adopts them - see item D.)
5. **`GmParty.c`** (120 lines) - `GameSrv_`-prefixed writers with no `GAME_CMSG_PARTY_*` handler.
6. **`GmDefaultArmors.c`** - a table with one caller, `CharCreationChangeProf`.

---

## 13. The real gap list

Ordered by what a player or tester would notice first. Each entry says what it needs, and nothing else
is added to it. **Everything that used to be on this list and is not here now has a measurement behind it** -
dispatch, the emit surface, the load block, movement and pathfinding, `/tp`, character creation, multi-player,
the schema and the command surface - and the generated pages are where to check rather than this paragraph:
`docs/11-parity-state.md`, `docs/12-schema-parity.md`, `docs/13-command-parity.md`, `docs/14-test-plan.md`.

| # | Gap | What it needs | Blocked on |
|---|---|---|---|
| 1 | The ground item for a drop | The message that spawns a dropped item on the ground | **A measurement that does not exist**: a capture of dropping a quantity-1 item. The reference's own handler is the unfinished one (a `TODO` for partial stacks, the item never removed, its model id the item's id), so it cannot produce one |
| 2 | Unequip (`0x804F`) | The item id the client means | **A measurement that does not exist**: a capture. The message's layout carries no item id, and dragging an item out of the equipped bag arrives as `ITEM_MOVE`, which works |
| 3 | The live halves of T9, T10, T11 and T13 | A second real client rendering the other player, the walk and the chat, and a human watching memory over ten minutes | **Not a measurement gap**: it is a rig run. The server halves are asserted (a second agent introduced and withdrawn, the reference's own path and refusals, the chat pair, the tick and ping rhythm) and `docs/14-test-plan.md` records each as `hand` |

Two things are worth stating plainly because they look like gaps and are not. **Chat rendering (T11)** is not
achievable by the reference either: its own chat send path is gated off because it crashed its client twice, so
"both clients display it" is a criterion GWPS cannot meet. This server matches what GWPS *does* - sends a
spoken line, prints a generated answer - and that is the ceiling. And **walking** is no longer a gap: the path
finder is ported and compared case by case, and `MOVE_TO_COORD` answers with the reference's own waypoint.

---

## 14. How to check this audit

Most of this document is now re-derived by tools, and the tools win where they disagree with it:

```powershell
# every axis, re-measured, in one page
python tools/parity-summary.py --write docs/11-parity-state.md --fast
# dispatch and the emit surface
python tools/parity-report.py --write docs/07-status.md
python tools/parity-emit.py
# the database schema, column by column
python tools/parity-schema.py --check
# the /tp aliases and the whole command surface
python tools/parity-commands.py --check
# the reference's own test plan, item by item
python tools/parity-testplan.py --check
# the load block: order and size, then the bytes
python tools/parity-load.py --check crates/asura-world/src/load_fixture.rs
python tools/parity-load-bytes.py --check
# the map data and the path finder, against the reference's own code
python tools/parity-pathing.py --check
python tools/parity-path.py --check
# everything at once, which is what a change has to keep green
.\tools\test-battery.ps1
```

The hand checks that remain -- the ones the tools cannot make -- are listed in `docs/14-test-plan.md` as
`hand`, with what each needs.
