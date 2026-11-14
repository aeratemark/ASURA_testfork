# 49 - The NPC framework: definitions, placements, instances and behavior

**Status, 2026-11-09:** built through the static population and the behavior runtime. The whole
pre-Searing is populated from preforged's `gw-npcs 1` catalog: 79 bodies across maps 145-148 and
160-166, with Ascalon City's 23 bodies including Sir Tydus, the Town Crier and Gift-Giver Pinn. The
capture's eight armed Ascalon bodies keep their `0x0161` item records and `0x006D` weapon ids. The
close pair of Ascalon Guards chooses at random among five patrol spots at 120 u/s; the other guards
stand. Section 8 lists what is open.

**Read first:** `docs/30-world-content-and-scripting.md` (the mechanics/behavior boundary),
`docs/32-world-simulation-plan.md` Phase 3, `docs/36-game-rules-ledger.md` R29-R34, and
`docs/adr/0014-behavior-tree-execution.md`.

## 0. The answer

An NPC is three records. The **definition** is a type shared across the pack. The **placement** puts one
body on one map. The **agent** is the running body, and the instance owns it. Each client is sent the
bodies within its compass range, in the live server's measured entry burst. The bodies sit in the load's
agent array, in agent-id order with the players. The instance tick drives them through a Rust
behavior-tree runtime, and the tree's shape comes from outside Rust.

```text
config/World/<pack>/npcs/{definitions,sight}.json   maps/.../<map>/npcs.json
                 |                                              |
        npc::file (parse) -> npc::content (check, resolve) -> NpcContent  (start-up; one bad row refuses)
                                                                 |
   Instance::with_npcs -> npc::table::NpcTable::seed   (R29: before any player)
                                                                 |
   load:  player::handle_request_players -> Instance::npc_arrival -> burst::entry -> frames
   tick:  Instance::step -> NpcTable::tick: advance, think (behavior::Tree), sight churn -> frames
```

## 1. What is already decided

These are binding inputs. This document does not reopen them.

| decision | source | consequence for NPCs |
|---|---|---|
| Rust executes mechanics; external Lua defines behavior | `docs/30`; `docs/35`; `AGENTS.md` 4a | Rust owns state, movement, packets and the tree tick; tree shapes come from outside |
| BehaviorTree.CPP is the execution reference; Lua constructs, Rust ticks | ADR 0014 | one tree per agent; halting cancels running work |
| Content is a self-contained pack; one pack; no inheritance | `docs/34`; `docs/38` D20, D23, D24 | NPC content lives in `config/World/<pack>/`; the database holds none |
| Values are files; provenance lives in `docs/` | `AGENTS.md` 4a; D35 | the JSON says what a value is; this document says where it came from |
| The instance owns mutable state; one owner updates it | `docs/22`; `docs/02` | the NPC table is per instance, ticked by the instance task |
| Everything exists before a player arrives; most NPCs are static; paths are Lua's | `docs/36` R29, R30 | the table is seeded when the instance is created |
| Spawn once, no respawn unless scripted; tick only within compass range (10,000) | `docs/36` R31, R34 | exists, visible and active are separate; dormant bodies freeze |
| The packet stream is the specification | `AGENTS.md` 1 | every message and order below is read from a capture |

## 2. What the live server sends

Source: `recordings/client/feature/maps/pre-searing-load-3.jsonl`, a live load of Ascalon City (map 148),
decoded record by record on 2026-11-09. It holds 42 creates: 24 players and 18 NPCs. The records are the
client's struct layer (`docs/03-protocol.md` 5e). Wire sizes below are the table's.

### 2.1 One agent array, interleaved

Players and NPCs are not two blocks. The load carries one agent array in ascending agent id: player
0x28 (seq 210-228), NPCs 0x29 to 0x3C (seq 229-298), player 0x3E (seq 299-317), NPCs 0x3F and 0x41, and
so on. ASURA builds the same array: peers and NPC bursts sorted together by agent id
(`player::handle_request_players`).

A client looks an agent up in that array on every agent-addressed message, and asserts if the slot
is empty (`AgMsg.cpp:513`, `ptr`). So a message that names an NPC goes only to the clients that were
sent that body and not its remove: the fight's broadcasts carry the agents they name
(`skill::wire::subjects`) and `Instance::step` drops the batch for a client that does not know one
of them. A client crash on 2026-11-14 (a wolf closing on the player) was this lookup.

### 2.2 The entry burst

All 18 NPC creates follow one order:

```text
[0x0056 [0x0057]]  the definition, the first time this connection needs it; 0x0057 only with model files
[0x009A]           the agent's scale word
[0x009B]           the agent's own name
[0x009F 36]        the agent's level
0x00F0             initial effects (0 for every body)
0x0020             the create
[0x0161 ...]       the item records of what it holds
0x006D             weapons: the two item ids, or 0 and 0
[0x009F | 0x00A2]  per-agent properties
```

| shape | bodies | example |
|---|---|---|
| `56 57 9B F0 20 6D` | 0x31, 0x33, 0x39, 0x64, 0x65 | seq 253-258 |
| `56 57 9B F0 20 6D 9F(11)` | 0x2B, 0x38 | seq 246-252 |
| `56 9B F0 20 6D` | 0x3C (definition 0x22A6 has no model files) | seq 294-298 |
| `9B F0 20 ...` | 0x34, 0x3F, 0x41, 0x63 (definition 0x5C8 already sent) | seq 265-270 |
| `56 57 9A 9F(36) F0 20 [161] 6D A2(44)` | 0xF5, 0x196 (animals) | seq 573-580 |
| `9A 9F(36) F0 20 6D A2(44)` | 0x1A9 (second animal of 0x107) | seq 581-586 |
| any of the above with item records before `6D` | 0x29, 0x2A, 0x34, 0x3B, 0x3F, 0x41, 0x63, 0xF5 (armed) | seq 233-236 |

The order has four consequences:

- **A definition is sent once per connection, right in front of its first body.** It is not sent in a
  block ahead of the array. A second body of the same definition starts at `0x009A`/`0x009B`.
- **No NPC create carries `0x00A6` (profession) or `0x0026` (flags).** The profession is a byte of
  `0x0056`. A person's level is a byte of `0x0056`; an animal's comes as `0x009F` property 36 before its
  create.
- **Every NPC gets `0x006D`.** An unarmed body sends `{agent, 0, 0}` (seq 251).
- **No create carries max health.** No `0x009F` 42 appears in any NPC burst.

### 2.3 The fields

`0x0056` (the table: six dwords, two bytes, `String16` of 8). Across the 14 records:

| field | name here | values seen | reading |
|---|---|---|---|
| 1 | `definition_index` | 0x107 to 0x22BF | the create's model word is `0x20000000` with this index |
| 2 | `file_id` | 0x1C601, 0x2CEE9, ... | a model file |
| 3 | `h000a` | 0 in all 14 | unnamed |
| 4 | `visual` | `0x64000000`, `0x28000000`, `0x1B000000` | top byte = scale percent; the animals' `0x009A` repeats it |
| 5 | `h0012` | 0 in all 14 | unnamed |
| 6 | `flags` | `0x20C`, `0x200`, `0x22C`, `0x18204` | a flags word, unread |
| byte 1 | `primary` | 1, 3 | the profession |
| byte 2 | `level` | 0, 1, 2, 5, 20 | the level |
| name | `name` | 4 or 5 units | an encoded string; the client resolves it |

The `0x0020` create of an NPC (seq 233, all 18 agree except where noted):

| field | NPC value | player value | note |
|---|---|---|---|
| model | `0x20000000` with the definition index | `0x30000000` with the player id | class nibble |
| type | 1 | 1 | living |
| `h000B` | 9 for a person, 8 for an animal | 5 | a type byte; 8 is not death (see 2.5) |
| `h001E` | 1 | 1, 4, 7, 9 | |
| speed | 200, 250, 288, 300 | 288 | per definition |
| `h0023` | 1.0; 0.32 on three bodies of 0x5C8; 0.167 on the animals | 1.0, 0.66, 0.75 | per body, unnamed |
| `h0027` | `0x41400000` | `0x41400000` | every living create |
| team | `'nonc'` | `'play'` | four-character code |
| `h003B` | non-zero, varies (`0x25DE3604`) | non-zero in the load; 0 after it (seq 885) | unexplained; ASURA sends 0 |
| destination pair | infinity | infinity | a walking animal carries its destination there (seq 517, 584) |

### 2.4 After the load

These measurements are for later behaviors (seq 744-1369). None of them is implemented as a behavior.

| body | what it does |
|---|---|
| 0x196, 0x1A9, 0xF5 (animals) | wander: one `0x0029` leg of 30 to 280 units every 2 to 10 s around home; no `0x0028` |
| 0x33, 0x3B, 0x3C, 0x65 | turn in place: `0x002E` `{agent, angle, 0x40060A92}` every few seconds |
| 0x41 | `0x0029` to the same point (8406, 6085) every 2 s |

The session holds no `0x0028` (stop) at all. A body that reaches its `0x0029` point stops by itself.

**Not yet explained:** seq 681-713 send `0x009A`, `0x009B`, `0x009F` 36 and `0x00A6` for seven agents
(0x159, 0x165, 0x1D6, ...) that are never created. They use definition 0x22BF and share one name.

### 2.5 What the previous version of this document had wrong

The proposal of 2026-11-09 took its message order from Rurik and left ASURA's own capture undecoded. The
capture overturns these claims:

| claim | what the capture shows |
|---|---|
| burst: `0x00F0`, `0x0020`, `0x00A6`, `0x009F` 36, `0x0026` | no `0x00A6` and no `0x0026`; `0x009B` and `0x006D` per body (2.2) |
| definitions go out before agents, as a block | interleaved, each in front of its first body (2.2) |
| an NPC create has `h000B = 0`, `h0027 = 0x34000000` (also in `CreateAgent`'s doc comment) | 9 or 8, and `0x41400000`; those are the item agent's values |
| a pause ends with `0x0028` | no `0x0028` in the session; a leg ends on its own (2.4) |
| "kind 8" marks a corpse | `h000B = 8` is every living animal, walking (seq 517) |
| the plane rule is unmeasured | a body on plane 26 is created for a client on plane 0 (seq 233 against seq 221), so distance is flat |
| max health timing is open | no live NPC burst sends it; this server adds property 42 and a full 34 after a combat body's entry burst so the client can turn a damage fraction into a number (`docs/57-damage-numbers.md`) |
| `facing` in radians | an angle does not reproduce the live bits: 6 of 18 directions differ in the last place, so a copied row stores the vector |
| definition fields `skin_file`, `appearance`, `secondary`, `allegiance` | fields 3 and 5 are 0 in every record (unnamed, `h000a`/`h0012`); `0x0056` has one profession byte; the team is the code itself |

The proposal also carried scaffolding that this revision drops: placement fields with exactly one legal
value (`condition`, `lifetime`, `respawn`, `group`), and a placement-count bound in `config/Server/`.
Each arrives with the phase that gives it a second value. Until then `deny_unknown_fields` refuses it.
Its separate visibility cadence and per-plane grid move to Phase 4 (3.9): per-tick sight over a town's
bodies is a scan of tens, and the grid serves combat's actor-relative queries.

## 3. The framework

### 3.1 Records and identities

| record | scope | lifetime | identity | mutable | code |
|---|---|---|---|---|---|
| definition | pack | process | `id` string; `definition_index` on the wire | no | `content::NpcDefinition` |
| placement | one map | process | `id` string, unique in its map | no | `content::NpcPlacement` |
| route | one map | process | `id` string, unique in its map | no | `content::Route` |
| agent | one instance | instance | agent id, allocated at seeding | yes | `table::NpcAgent` |
| sight | one client in one instance | until the client leaves | player id | yes | `table::Sight` |
| tree | one agent | instance | its agent | yes | `behavior::Tree` |

- A script, a log line or a quest names a **placement id**. It never stores an agent id.
  `NpcTable::agent_of(placement)` resolves one placement id to an agent id.
- **Agent ids** come from the instance's counter, in file order, when the instance is created. A
  player's join and a dropped item skip every NPC id (`Instance::join`, `allocate_agent_id`).
- **The definition index** is the client's table index, set in content. The class nibble bounds it: an
  index at or above `0x10000000` is refused.

### 3.2 Content

```text
config/World/<pack>/
  npcs/definitions.json      the types
  npcs/sight.json            compass_range
  maps/<campaign>/<area>/<subarea>/<map id>/npcs.json   placements and routes
```

**`npcs/definitions.json`:** a `note`, a `fields` object with one sentence per key, and `definitions`.
Every key is required: `id`, `definition_index`, `file_id`, `h000a`, `visual`, `h0012`, `flags`,
`primary`, `level`, `name` (1-8 units), `model_files` (0-8), `h000b`, `speed`, `team` (four
characters).

**`npcs/sight.json`:** `compass_range` as `{value, doc}`, the same shape as `party_sizes.json`.

**`<map>/npcs.json`:** `map_id` (checked against the folder), `note`, `placements` and `routes`.

| placement key | required | message or use |
|---|---|---|
| `id`, `definition`, `x`, `y`, `plane` | yes | the body and where it stands |
| `direction` (`[dx, dy]`) or `angle` (radians) | no, not both | the create's direction; `(1, 0)` without either |
| `scale` | no | `0x009A` |
| `name` (1-32 units) | no | `0x009B` |
| `level` | no | `0x009F` property 36 before the create |
| `effects` | yes | `0x00F0` |
| `h0023` | yes | the create's float |
| `properties` (`{id, int}` or `{id, float}`) | no | `0x009F` / `0x00A2` after `0x006D` |
| `items` | no | one `0x0161` per item, after the create and before `0x006D` |
| `weapons` | no | the lead and offhand as indices into `items`; `0x006D` |
| `route` | no | the route its `Patrol` walks |
| `behavior` | no | the behavior that drives it; without one it stands |

- A route is `{id, mode: "loop", "once" or "random", points: [...]}`. `random` picks a new point
  after every pause, never the one just left; the placement's own route is the candidate set.

Weapons are expressible. An armed body declares its held items in the placement's `items` array, each
with the `0x0161` fields (`file_id`, `item_type`, `dye_tint`, `dye_colors`, `materials`, `unk1`,
`flags`, `value`, `model_id`, `quantity`, `name`, `modifiers`), and `weapons` names the lead and
offhand as indices into that array. The world allocates the wire's item ids per body
(`npc::burst::held_item_id`), because the client indexes its item table by id and the live
connection's arbitrary handles must not collide with the character's own items.

A placement may state `walk_speed`. When it does, the world sends the 37130 `0x0027` base speed and
`0x002B` gait pair before a leg (`gw_agent_wire.h:35-46`). No current placement sets it: the 39167
client took that pair as error `007`. Until 2026-11-12 that `0x002B` went out through
`agent_update_speed_live`, which writes 16 bytes where the packet table says 11; it now goes out
through `agent_update_speed`, the form a player's walk start uses. Whether error `007` was that size
has not been retested. The imported definitions instead carry 120 in the create, an
authored value: cmonmeow's NPC speed is 220 (`gw_agent_wire.h:70`,
`gw_npc_owned_ordinary_descriptor.h:16`), and the owner judged it too fast in the live client.

`tools/npcs-from-capture.py` writes these files from a load capture. It copies every value, merges
definitions, keeps armed bodies and their `0x0161` rows, and refuses a definition whose bodies
disagree on a per-definition create field.

### 3.3 Loading

`npc::current()` runs once at start-up, beside the item and spawn checks in `main.rs`, and refuses the
start on any problem, listing every problem with its file key and field:

1. Parse `npcs/sight.json`, `npcs/definitions.json` and every map folder's `npcs.json`. A pack with
   none of them has no NPCs. Unknown keys are refused.
2. Definitions: unique id; unique index below the class nibble; name 1-8 units with no zero unit; at
   most 8 model files; a finite non-negative speed; a four-character team.
3. Placements: unique id in the map; a definition that resolves; finite position, direction and
   `h0023`; name 1-32 units; finite float properties; a route on the same map; a behavior the source
   knows.
4. Routes: unique id; at least one point; finite points.
5. Every placement's tree is built once (`table::check_trees`), so a construction error refuses the
   start instead of parking a body later.
6. Once the path graphs are loaded, every route point and every placement a route or behavior can move
   must lie on its map's walkable surface (`npc::check_ground`). A placement with neither route nor
   behavior is noted and passes even when it is off the surface - it only stands where the live server
   put it - and a map without a graph is noted and passes for the same reason: a standing body needs no
   graph, and its walk will be refused. Map 148's `148-0107-2`, an animal the live load creates at
   `(9435.076, 3593.231)` off the walkable geometry, is the measured case.

No hot reload. A content change takes effect on restart.

### 3.4 Runtime ownership

`Instances::with_npcs(NpcWorld)` is how `main.rs` builds the world's instances. `NpcWorld` holds the
content, the graphs and the behavior source, all shared and immutable. `Instance::with_npcs` seeds an
`NpcTable` from its map's content before the instance is returned, so the bodies exist before the first
join. `Instances::default()` makes instances without NPCs. The tests' harnesses use it.

The table sits behind the instance's own mutex. Its lock is taken after the slots' and the ground
items', never before them. No critical section awaits.

A body is an `AgentState`, the same record a player drives. `advance` moves it and `begin_walk` starts
a leg. No client reports for it.

### 3.5 Wire mapping

`burst` holds the order as data (`Msg`), and `frames` turns each `Msg` into bytes through the wire
crate's builders. The new builders are `npc_update_properties`, `npc_update_model`,
`agent_update_npc_name`, `agent_update_scale` and `npc_update_weapons`. Their bytes are pinned to the
capture's records (`game::tests::the_npc_messages_are_the_captured_bytes`). `CreateAgent` now carries
`h001e` and `h0023`; every existing caller passes the old 1 and 1.0.

| event | messages |
|---|---|
| a client loads | per body in range, in agent-id order with the players: the entry burst (2.2) |
| a body comes into range | its entry burst; if it is walking, a `0x0029` to its current destination |
| a body leaves range | `0x0021` |
| a body crosses a waypoint, or a leg starts | `0x0029` (the 18-byte form players use) |
| a running walk is halted | `0x0028`; not measured for an NPC (the session has none) |

A body created mid-walk is created standing and then told its leg. The live create carries the
destination in its last vectors instead (2.3). That field is not modelled.

### 3.6 The tick

`Instance::step` runs the NPC tick after the players' own:

1. **Viewers** are players that have entered the instance and finished loading. A loading client gets
   its bodies in its own block.
2. A body is **active** while some viewer is within `compass_range` (flat distance). An active body's
   walk advances, then its tree ticks. A dormant body keeps its state and resumes where it stopped.
3. Each viewer gets the messages of the bodies it already sees, then a remove for each body that left
   its range, then an entry burst for each that came into it.
4. A tree that fails parks its body and logs the placement and the reason once.

The tree clock is the tick's elapsed time. No wall clock is read, so a run is reproducible from its
inputs.

### 3.7 Behavior

`behavior::Spec` is a tree's shape: what a constructor returns. `behavior::Tree` is one agent's running
copy. `table::BehaviorSource` maps a placement's `behavior` name to a `Spec`, or to a `Script` when the
entry is a Lua function. `LuaBehaviors` (`npc/lua.rs`) reads the pack's `npcs/behaviors.lua`; table
entries become `Spec`s, function entries stay live and are called once per agent per tick. `NpcWorld::new`
selects it. A pack without a script runs `PackBehaviors`. A placement that names a behavior neither
source can build refuses the start.

| node | semantics (ADR 0014) |
|---|---|
| `Sequence` | `Sequence`: resumes at the running child; a failure resets it and halts the rest |
| `Fallback` | `Fallback`: resumes at the running child; a success resets it |
| `Wait { ms }` | runs for `ms` of tree clock, then succeeds |
| `Stand` | runs forever, sends nothing |
| `Patrol { route }` | walks the route (its own, or the placement's) leg by leg; pauses where a point says; `loop` keeps going, `once` succeeds at the end |
| `Burrow { visible_ms, hidden_ms }` | starts in the 2.00 s emerge transition, stays out, submerges for 2.00 s, removes, hides, and re-appears at the same id and coordinates (`docs/59`) |
| `Script { name, burrow }` | a Lua function called once per tick with `ctx.in_combat`, `ctx.phase`, a Rust-owned `ctx.state` table and `ctx:burrow()`/`ctx:emerge()`; it returns running/success/failure (`docs/59`) |

A halted `Patrol` cancels the walk, sends `0x0028` and keeps its target point, so a restart resumes the
abandoned leg. That matches Rurik's measured patroller behavior, as cited in `docs/20`. A leg with no
path fails with its reason. On a map without a graph a leg fails, and the body parks.

### 3.8 Visibility and lifecycle

| state | meaning | owner | wire |
|---|---|---|---|
| exists | in the instance's table | instance | none |
| visible | in one client's sight record | per client | entry burst on entering, `0x0021` on leaving |
| active | some viewer is in range | computed per tick | its walk and tree run |
| alive | (Phase 4) | | not modelled; no death, no corpse, no respawn |

A leaving player's sight record is dropped (`Instance::leave`). The bodies stay. A return is a fresh
entry with the definition again, because the client's record is per connection. Instance teardown drops
the table.

### 3.9 Seams for later phases

- **Combat (Phase 4):** a per-instance spatial index for actor-relative queries, aggro memory with two
  radii, an action token with epoch, and vitals. A leash returns a body to its placement's position.
- **Interaction (Phase 6 or the vendor work):** `GAME_CMSG_INTERACT_LIVING` (`0x8039`) names an agent.
  The service finds its placement (`NpcTable::placement_of`) and checks
  that the client can see it and is in range. Dialog content is later work.
- **Armed NPCs:** built. `0x0161` item records go between the create and `0x006D`; the placement's
  `items`/`weapons` hold the captured fields, and the item id is allocated per body.
- **Measured motion (2.4):** wander, turn-in-place (`0x002E`) and the repeated move are behaviors for
  the Lua source. `0x002E` needs a builder and a name for its third dword. The pack's
  `npcs/behaviors.lua` declares `patrol` and the Plague Worm's script; every other scripted behavior
  still needs its Rust operation before the script can name it.

## 4. Code map

| file | what |
|---|---|
| `crates/asura-world/src/npc/mod.rs` | the framework's entry: `load`, `current()`, `check_ground`, `NpcWorld::new`, `PathGraphs` as `Pathing` |
| `npc/content.rs` | records, `NpcContent::assemble`, `check_ground` (pure) |
| `npc/file.rs` | the JSON shapes and the pack walk |
| `npc/burst.rs` | `Msg`, `entry` (the measured order, pure) |
| `npc/lua.rs` | the Lua loader: one script run, a returned table, `Spec` conversion |
| `npc/frames.rs` | `Msg` to `Frame` |
| `npc/behavior.rs` | `Spec`, `Tree`, `Patrol`, `Pathing` (pure over `AgentState`) |
| `npc/table.rs` | `NpcTable`: seed, arrive, forget, tick, the `/npc` command; `BehaviorSource`, `PackBehaviors`, `check_trees` |
| `instance.rs` | ownership, id allocation, `npc_arrival`, the tick call, `Instances::with_npcs` |
| `player.rs` | the agent array: peers and NPCs by agent id |
| `pathfind.rs` | `PathGraphs::on_ground` |
| `main.rs` | the start-up checks and `Instances::with_npcs` |
| `asura-protocol/src/game/mod.rs` | the five builders, `CHAR_CLASS_NPC_BASE`, `CreateAgent::LIVING`, `player_create_agent` |
| `asura-protocol/src/packets.rs` | `0x0056`, `0x0057`, `0x009A`, `0x009B`, `0x006D` now `Emitted`, with evidence |
| `tools/npcs-from-capture.py` | capture to content, armed bodies included; authored routes preserved |
| `tools/npcs-from-preforged.py` | preforged's `gw-npcs 1` catalog to every pre-Searing map, with captured held items |
| `config/World/Vanilla/npcs/`, `.../Outposts/148/npcs.json` | 79 definitions, 79 placements over 11 maps, 2 authored patrols |

## 5. Stages

| stage | work | state |
|---|---|---|
| A | builders, content, loader, instance table, the load's agent array | **built**; the real client is the judge (6) |
| B | ownership, id allocation, per-client sight, range churn, dormancy, join/leave | **built** |
| C | tree runtime, `Patrol`, `Burrow`, halt, parking | **built**; the pack's Lua script declares `patrol` and the Plague Worm cycle |
| D | the pre-Searing population: `tools/npcs-from-preforged.py` | **built**: 79 bodies over maps 145-148 and 160-166 |
| E | armed NPCs (item records, `0x0161` + `0x006D`) | **built** on map 148 from the capture |
| later | Phase 4 combat, Phase 5 HeroAI, Phase 6 interaction, Phase 7 capacity | `docs/32` |

## 6. Acceptance

| check | automated | real client | state |
|---|---|---|---|
| the five builders and the NPC create are the captured bytes | `game::tests::the_npc_messages_are_the_captured_bytes` | | passes |
| a burst's headers and sizes | `npc::frames::tests` | | passes |
| burst order per body shape (2.2) | `npc::burst::tests` | | passes |
| content refusal: duplicates, references, sizes, class nibble, ground | `npc::content::tests`, `npc::file::tests` | | content passes; file tests need a workspace run |
| ids never collide; bodies exist before players; leave keeps them | `tests::npc_bodies_exist_before_players_and_share_no_agent_id` | | needs a workspace run |
| range churn, per-client definitions, dormancy, mid-walk entry, parking | `npc::table::tests` | | passes |
| sequence, fallback, wait, patrol, halt, construction errors | `npc::behavior::tests` | | passes |
| an armed body sends its items between the create and `0x006D`, ids matching | `npc::burst::tests::an_armed_body_declares_its_items_before_its_weapons` | | passes |
| the committed pack and its routes stand on the bundled graphs | `npc::tests::the_committed_pack_stands_and_patrols_on_the_bundled_graphs` | | passes |
| the 18 copied bodies round-trip to the capture's bits | a one-off check against `pre-searing-load-3.jsonl` | | done |
| 79 NPCs stand across the pre-Searing; Ascalon's 23 bodies, the close guard pair patrolling | | `ASURA_CAPTURE` on map 148 | **to do** |
| a second client sees the same bodies; leaving and returning does not duplicate | | two clients | **to do** |
| walking to the edge of range removes and re-creates a body | | one client | **to do** (148 is smaller than the range) |

## 7. Risks

1. **The client asserts on a missing definition.** The per-connection record sends each definition in
   front of its first body. Only the client can prove it accepts them.
2. **`h003B` goes out as 0.** The live load sends a varying non-zero value, and a live late join sends
   0. If the client reads it, the load is the case to watch.
3. **Agent ids are not identities.** Ours differ from retail's (the bodies take 2 and up). Nothing
   stores one.
4. **Do not invent AI.** Every motion of 2.4 is a measurement of an envelope, not a decision function.
5. **Movement parity is not claimed.** The close pair's random five-spot route is an ASURA authored
   route from the owner's observation, not a capture. The 300 s sniff caught two other guards
   (`0x3E`/`0x3F`) moving among a shared set of stops, but the close pair (`0x63`/`0x64`) stood for the
   whole window. When a full-loop capture of the close pair exists, measure the drift between it and
   these five spots.

## 8. Open questions for the owner

1. **The behavior constructor (answered).** `LuaBehaviors` now reads `npcs/behaviors.lua` and builds
   the trees ADR 0014 describes; `patrol` and the Plague Worm cycle are the first entries. The next
   node set - a trigger line or proximity, dialogue and dialogue choices, spawn/despawn, skill use -
   is still open, and each needs its Rust operation before a script can name it.
2. **The patrol routes.** A 2026-11-09 300 s sniff caught two guards (`0x3E`/`0x3F`) moving among a
   shared set of stops, but the close pair (`0x63`/`0x64`) stood for the whole window. The five spots
   for the close pair are the owner's observation (bonus merchant, Namar, the corner right of the
   Ascalon gate, Gift-Giver Pinn, Lieutenant Langmar); the random choice among them is ASURA's.
   Should it stay, or wait for a full-loop capture of the close pair?
3. **Armed NPCs.** Built on map 148: the eight armed bodies carry their captured `0x0161` fields and
   `0x006D` hands. The remaining question is whether a later item catalogue should replace the raw
   copied fields or keep them byte for byte.
4. **The layout.** `npcs/definitions.json`, `npcs/sight.json` and per-map `npcs.json` are what is built.
   Confirm or move.
5. **The first interaction.** Answered 2026-11-13: a talk opens a captured dialog first, and the
   window is behind a choice (`docs/55-npc-dialogs.md`).

## 9. Sources

ASURA: `recordings/client/feature/maps/pre-searing-load-3.jsonl` (seq 210-298, 384-395, 512-519,
573-586, 681-713, 744-1369); `docs/02`, `docs/22`, `docs/23`, `docs/27`, `docs/30`, `docs/32`,
`docs/34`, `docs/36` R29-R34, `docs/38`, ADR 0014; the code in section 4.

Preforged: `Other_Repos_Research/preforged-main/data/gw/npcs.txt` (the `gw-npcs 1` population, 79
pre-Searing rows over maps 145-148 and 160-166, imported by `tools/npcs-from-preforged.py`),
`src/runtime/gw_npc_wire.h` and `src/runtime/gw_agent_wire.h` for the 2022 builders, and `README.md`
for the six minipets it excludes. It is a catalog, not a replay: the capture remains the authority for
map 148's armed item records and their weapon ids.

Other repositories (read-only, never dependencies): Appendix A.

## Appendix A - What the other repositories do

Eight checkouts under `Other_Repos_Research/` were read for the first version of this design. This is a
read-only source inspection: no other server was built or run. A statement credited to Rurik or cmonmeow
is that project's measurement or authored rule, not an ASURA replay. Where ASURA's own capture disagrees,
section 2.5 says so and the capture wins.

| repository | definitions | placements | runtime owner | spawn order | behavior | visibility and life |
|---|---|---|---|---|---|---|
| Rurik (Python) | `content/npcs.toml` | `[spawn.*]` rows in `content/world.toml` | per connection | `0x0056`+`0x0057` once per definition, then per agent `0x00F0`, `0x0020`, `0x00A6`, `0x0026`, `0x002B` | authored policy: targets, leash, scatter | creates/removes are visibility; death `0x0026` |
| cmonmeow (C++) | `gw-npcs 1` rows | the same row | per instance, atomic replacement | definitions, then creates | per-agent modules: aggro, pursuit, wander, spellbook | per-player visibility set |
| preservation (Go) | `agentDefinition` | `agentSpawnInfo` | instance | instance creation | none | teardown only |
| GameRevision (Java) | `Npc` entity | `Spawnpoint` | entity manager | per client on `CanSee` | none | O(n^2) every 500 ms |
| entice (Elixir) | `Npc` struct | add-NPC event | process per NPC | map instance spawns | open question | process lifetime |
| OpenTyria (C) | `GmAgent` | map config | one world array | player only | none | position broadcast |
| Headquarter, GWToolbox | client-side structs | | | | | cross-checks for field names |

**Rurik** (`toolkit/authsrv/agents.py:283`, `authsrv.py:41309`, `:42034`, `respawn.py:15-66`,
`studies/monsterai/FINDINGS.md`, `studies/npctrack/FINDINGS.md`):

- A definition index is a raw client array index. Creating an agent whose definition was never sent
  crashes the client. ASURA's per-connection record is that rule.
- Creates are visibility churn. 19 of 45 ids were re-created in one tape, and creates outnumber deaths
  about 2,900 to 24. Do not key respawn on a create.
- The AI decision function is not in the client or on the wire. Its envelope is measurable: notice
  about 1,000 units, reach 65 to 600 per model, a stander returning 14-65 units home, a patroller
  resuming its leg. Its policy is not.
- A chase uses `0x002A` naming the target; the client parks the follower at its own 80-unit disc;
  drift at the halt has a median of 53.8 units.
- Its spawn-burst order (`0x00A6`, `0x0026` per agent) does not match ASURA's capture (2.5).

**cmonmeow** (`runtime/gw_npc_*.h`, `gw_world_simulation.h:15217`):

- Policy is kept apart from per-agent state: pursuit, aggro memory (two radii, nearest-first, id
  tie-break), wander, an action token with epoch, a spellbook and a ledger. These are the Phase 4/5
  shapes.
- The population is replaced atomically: validate everything, then commit.
- An optional trait needs evidence. ASURA keeps evidence in `docs/` instead (`AGENTS.md` 4a).
- Its creates use `h000B = 0`, `h0027 = 0`, which disagrees with the live capture.
- Dialogue: body chunks `127`, sender `128`, buttons `125`, selection command 57 (build 37130).

**preservation** (`gameservice/instance.go:29-415`): the simplest correct instance-owned population.
Its interaction definitions are keyed by agent and action.

**GameRevision** (`NPCFactory.java`, `AgentVisibilitySystem.java:67`): the component split and the
visibility events are the model; its O(n^2) loop is the warning.

**entice** (`npc.ex`, `map_instance.ex`, `milestones.md`): one process per NPC. Its AI question
("periodic functions or external client?") is answered here by the Rust/Lua split.

**OpenTyria** (`GmAgent.h:14`): a flat agent record with a waypoint list, and one world tick.

What the eight agree on, and the capture confirms:

1. The type, the placement and the running agent are three records.
2. A definition precedes its first agent, per connection.
3. An agent id is a handle, not an identity.
4. Create and remove are visibility, not life.
5. The instance owns the population.
6. AI policy is authored content and is labelled as authored.
