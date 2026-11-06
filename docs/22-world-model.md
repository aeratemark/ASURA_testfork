# 22 - The world model: account, character, instance, agent

**Status: an explanation, recorded.** The owner described how the game works on 2026-10-30, from the
contexts Py4GW Stealth exposes. `AGENTS.md` #2 is explicit that an explanation is not an instruction and
that it changes no behaviour, so nothing here is implemented because of this document. What it does is
fix the **words**, so that the next feature is written in the game's own nouns instead of ours.

Where this file and `docs/00-glossary.md` overlap, this file wins: it is the model the owner states, and
the glossary now points here.

## The five nouns, as the owner defines them

> "an account is our login, it encompasses everything in our side of the server, our characters, our
> account unlocks. Each character is now named a player, it should be named character because that is
> its name. Each character has levels, skills, armour, items, attributes; the character can move in the
> maps."

> "maps, they are called INTANCES [instances]. A map is an instance. We walk from instance to instance:
> from an outpost to an explorable, explorable to outpost, an explorable to an explorable, or outpost to
> outpost by crossing a portal. Instances start locked (all the bitmap is 0) and as we walk and touch or
> reach a map, we mark it unlocked. Unlocking a map can also allow functionality such as the in-game
> fast travel through the world map."

> "an agent is anything in the map which is not a prop. The player or character itself is an agent. We
> have an agent array; it is populated by the game simulator and populates enemies, characters, gadgets,
> items, etc. Everything is an agent. Maps are not agents."

| noun | what it is | owned by | what it is not |
|---|---|---|---|
| **account** | the login: the characters on it and the account's own unlocks | the auth service, `account` / `session` tables | not a place, not an agent, not a map |
| **character** | a body with levels, skills, armour, items, attributes that stands in an instance | the account, `characters` table | not the login, not a party slot |
| **instance** | the game's word for a map, realised as **one running copy of that map's simulation** (owner, 2026-11-02). A map is the static definition; the instance is the live copy a character or party stands in, shared by one character or many | the world server, `map_config` rows + the archive's geometry | not an agent, not a region; a district is an instance of the *same* map |
| **agent** | anything *in* an instance that is not a prop: enemies, characters, gadgets, items | the instance's simulator, the agent array | not a map, not a prop |
| **unlock** | a bit per instance, in a bitmap that starts all zero, set when the character reaches that instance | the account or the character (open question 1) | not a permission the client checks before walking |

Two consequences worth stating, because they are the whole reason the portal work stalled:

1. **Unlocking is a consequence of arriving, not a gate on arriving.** The bitmap starts at zero and the
   walk is what sets a bit. So the portal cannot be refusing because the destination is locked.
2. **A portal is crossed between instances, so it lives inside one of them.** It is a **prop** with a
   position and a portal model, read from the instance's own map data - answered below, and it means the
   client draws it without being told.

## What each noun is called in this tree today

| the game's noun | our name | where |
|---|---|---|
| account | `account`, `session`, `portal_user` | `crates/asura-db/migrations/`, `asura_db::portal` |
| character | **`Player`** | `crates/asura-world/src/player.rs`, `Player::standing`, `player_block` |
| instance | **`map`**, `MapEntry`, `map_id`, `map_file_id` | `crates/asura-world/src/map.rs`, `asura_db::game::spawn_map`, `map_config` |
| instance (running copy) | **`Instance`**, `instance.rs`, `INSTANCE_LOAD_*` | `crates/asura-world/src/instance.rs` |
| agent | `CreateAgent`, `AGENT_TYPE_LIVING`, `AGENT_TYPE_ITEM` | `crates/asura-protocol/src/game/mod.rs:1119` |
| agent array | not modelled - one player, no others | - |
| prop | not modelled | - |
| unlock | `instance_unlock` rows, one per (character, instance) | `crates/asura-db/migrations/0023_instance_unlocks.sql`, `UNLOCKED_MAP_ZEROS` at `crates/asura-world/src/map.rs:471`, read at `crates/asura-world/src/player.rs:196` |

Two of those names are the client's, not ours, and stay: `GAME_SMSG_PLAYER_CREATE`,
`GAME_SMSG_INSTANCE_LOAD_INFO`, `GAME_CMSG_INSTANCE_LOAD_REQUEST_*`. `AGENTS.md` #1 requires the packet
names to be transcribed from the evidence, so a rename there is not ours to make. Everything above that
is *our* vocabulary follows the game's nouns, per `AGENTS.md` #6.

## The conflicts this exposes

1. **"Instance" means two things in this tree.** The game's word for a map, and our word for a running
   copy of one (`docs/00-glossary.md`, "Instance"). The glossary has been corrected to carry both senses
   with the owner's one first.
2. **"Player" is our name for the character**, and the protocol also has a `player_id` that is a party
   slot, not a character (`PLAYER_CREATE`, `PLAYER_UPDATE_PARTY`). One word, two concepts, in the same
   files.
3. **The unlock bitmap had no store and no reader.** `characters.unlocked_maps` was a column nothing
   wrote, and the only unlock message carried two lists of 31 zero dwords copied from the reference
   (`crates/asura-world/src/player.rs`). Both are now the same thing: **built** - see below.
4. **The agent array is missing, and this model says it is not optional.** The world sends one agent, the
   character. Enemies, gadgets and items are absent, which is the population gap measured in
   `docs/21-surrounding-packets.md`.

## Answered from the client's own contexts (2026-10-30)

The owner's direction: the answers live in Py4GW Stealth, Py4GW Reforged and the native bridge, and they
do. What follows is restated in this project's words from reading those contexts; per
`docs/adr/0013-py4gw-stealth-is-our-link-to-the-client.md` no address, byte pattern or value table from
them is copied here - only what the client holds and how it is addressed.

1. **Where the unlock bitmap lives.** The in-game world context holds five arrays of `u32`, in this
   order: missions completed, mission bonuses, missions completed in hard mode, bonuses in hard mode,
   and **unlocked maps**. That is exactly the five counted lists of
   `GAME_SMSG_MAP_UPDATE_UNLOCKED_LIST` (`0x0094`) in our own table, in that order. So the fifth list is
   the instance bitmap, and the four before it are mission progress.
2. **How a bit is addressed.** Element = `map_id / 32`, bit = `map_id % 32`, within the unlocked-maps
   array. The index is therefore the number we call `map_id` (148), not `map_file_id` (113021), and 31
   dwords per list - the width the reference sends - covers map ids 0 to 991, which contains the
   `MapId_Count` sentinel of 877. **Our `UNLOCKED_MAP_ZEROS` width is correct and its content is the
   starting state the owner described.**
3. **What a portal is: a prop.** Travel portals are read from the instance's **props**, with a position
   and the model file id that marks them as a portal; the client knows three portal models (Prophecies and
   Factions, Nightfall and Eye of the North, and the Asura gate). They are not agents, which is why the
   agent list of map 148 has none that is the gate. **A prop needs no message from us** - the client
   draws it from the map data it already has - so the crossing is triggered client-side, and what we owe
   the client is the instance table and the population around it, not the portal.
4. **A correction to my own earlier reading.** I said the 42 agents in the map-148 capture were all NPCs
   and players because the high byte of the type field read `0x20` or `0x30`. That was wrong: the client
   distinguishes living, gadget and item agents by **flags inside the type field**, not by that byte, and
   most of those 42 are gadgets - which is what an outpost is full of (signs, doors, chests, portals'
   neighbours). The exact masks are the instrument's, not this tree's.
5. **Where spawn points come from.** The map's own data carries three spawn lists, each entry a position,
   a facing angle and a four-character zone tag, and a numeric tag names the instance that spawn belongs
   to. So the *positions* a population stands at are in the client's archive: they can be read offline,
   like the path graphs. What is not in the archive is the population itself - which model, which level,
   which items - and that is ours.
6. **That the map id and the archive id agree** was confirmed a third way: the client-side map-to-archive
   table gives 148 -> 113021, the same number our `map_config` sends and the live `0x0196` carried.

## Decided, 2026-10-30

**The code keeps `Player`.** The owner's ruling: "if the packets are named player, lets retain player for
clarity". So `Player`, `player.rs` and `player_block` stay, and *character* remains the game's noun in the
documents. One word, two concepts - the character, and the `player_id` a party message carries - and the
code says which one it means by the field, not by the noun.

## Answered since: what the client sends when it crosses

Both travel triggers are measured, in `captures/zone-ref2-012225.jsonl` (the reference checkout's own raw
capture of a real client): `0x80C1` fifteen times as `{portal, 0}` - a portal of the instance the
character stands in, not a map id - and `0x80B1` twice, beginning with the destination instance id. The
server answers either with the pair `/tp` already sends. The layout, the counts and the sequence are in
`docs/03-protocol.md` 5f. **The client does announce the crossing, so nothing about this feature waits on
a new measurement; the portal table is recorded** (`crates/asura-world/src/portals.rs`, generated from
`data/portals.json`), and a doorway a walk reaches starts the same `begin_transfer` the other triggers do
(`crates/asura-world/src/travel.rs:364-433`). The `0x80C1` arm now does the same thing: it names the touch
and then applies that crossing test (`crates/asura-world/src/dispatch.rs:56-60`). What is still open is the
agent-keyed lookup - the message names the portal's agent, and the doorway is found by position instead -
and the information the instance needs to be initialised
(`docs/23-initialization-protocol.md`).**

## Built: the instance unlock and the world-map travel, 2026-10-30

The first piece of "fill every bit of information possible" is in the tree, and the request it enables is
answered.

| piece | where |
|---|---|
| the store: one row per (character, instance), with the time it was reached | `crates/asura-db/migrations/0023_instance_unlocks.sql` |
| writing it on arrival, and reading a character's reached instances | `asura_db::game::{unlock_instance, unlocked_instances}` |
| the client's addressing: bit `map_id % 32` of word `map_id / 32`, 31 words | `asura_world::map::unlock_bitmap` |
| the **fifth** list of `0x0094` carrying it instead of being empty | `crates/asura-world/src/player.rs` (`player_block`) |
| the write, at the moment the character's load block is built | `crates/asura-world/src/world.rs` (`handle_request_players`) |
| the world map's request answered with the travel cycle | `crates/asura-world/src/travel.rs` (`handle_party_travel`, `begin_transfer`) |
| the travel stream itself: eight messages in the live order, not two | same file (`begin_transfer`), `docs/03-protocol.md` 5f |
| the client's instance table (`0x01A0`), ten batches in the load block | `crates/asura-protocol/src/game/instance_table.rs`, `crates/asura-world/src/map.rs` |

Two facts made the unlock the first piece: the client reads an instance's unlock from the fifth of the
five lists `0x0094` carries, and **this server sent that list empty**, which is an unlocked array of
length zero - every instance read as locked and the world map had nothing to offer. The creation instance
sets no bit: it is not a place a character reaches.

**The policy, in the owner's words:** "the game itself should decide if we can travel or not ... we should
simply send the data". No rule of the form "you may only travel to an outpost" or "only to an unlocked
instance" exists in this world, and none may be added: an outpost and the explorable it exits to can share
one map blob, so a rule about the *kind* of instance refuses travel the game allows. The world refuses a
travel for two reasons only, and both are about this server rather than the client's rights - it cannot
serve the instance (no row, no blob, a type it does not host), or it has no free seat.

Proof: `an_instance_is_unlocked_by_arriving_once` (`crates/asura-db/tests/inventory.rs`, live database),
`the_unlock_bitmap_puts_each_instance_at_the_bit_the_client_reads` (`crates/asura-world/src/lib.rs`), and
`the_world_map_request_travels_to_the_instance_it_names` (`crates/asura-world/tests/load.rs`, over a real
socket, with the bytes a real client sent). What a live client makes of it is still the acceptance test.

## What is next, in the order the document ranks it

1. **The population** - the 42 agent creates and their per-agent property streams. The instance table
   (`0x01A0`) is sent now.
2. **A portal table, and then `0x80C1`** - which portal of which instance leads where, keyed by the portal
   order in the instance's own prop list, so the crossing ends in the same `begin_transfer` the other two
   triggers use.

## Character spawn points

Where a created character is spawned is the world pack's
(`config/World/Vanilla/character_spawn.json`): one map per expansion source, read by
`crates/asura-world/src/spawns.rs`. The file states the values; this section records where each came from,
because provenance is a measurement and the pack is not the place for one.

| source | map | evidence |
|---|---|---|
| Prophecies | 148 Ascalon City (pre-Searing) | every campaign-1 character this server has created has loaded it - the world log prints `created character ... campaign 1 profession 3` |
| Factions | 212 Monastery Overlook | a live creation was redirected to map 212, blob 156969, at (-2132, 1054) plane 0: `recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`, the `0x01A6`, `0x0099` and `0x0196` after the confirm |
| Nightfall | 449 Kamadan, Jewel of Istan | **not measured.** In the catalogue, and the client's own ordering puts Nightfall after Factions; no creation has named it yet, so the first Nightfall character settles it |
| Eye of the North | 642 | **not measured**, same reasoning as Nightfall |
| PvP | none | **not implemented.** No live PvP character has been created against this server, so the pack names no map and the creation refuses one |

An unknown or newer source falls back to 148, stated in the pack's `fallback` entry, so a stand-in is
visible in the log rather than mistaken for a measurement.

## Map identity, and where it comes from

Every fact this server needs about a map - its id, its name, its region, its campaign, the `Gw.dat` blob it
loads - is in the client, and the **Py4GW link is the way to read it**. Two of the three come as complete
tables, not as a per-map lookup:

- **Names are a table, and the table is complete.** The client's own enum tables name every map: 278
  outposts and 378 explorables, keyed by map id, plus the reverse `name_to_map_id`
  (`py4gw.enums_src/map_enums.py`, transcribed there from Reforged's
  `Py4GWCoreLib/enums_src/Map_enums.py`). An id is named by reading that table, and a name is turned back
  into an id by the reverse one, with no running client involved.
- **Campaign, region, type and blob are one record per map id.** The client keeps an `AreaInfo` for each
  map id - `campaign`, `region` (`Region_Kryta` 0 ... `Region_DevRegion` 27), `type`, the party sizes and
  `file_id` - readable read-only through `AreaInfoStruct`, and `GetMapInfo(map_id)` reads it for any map,
  not only the current one. `CampaignName` and `RegionTypeName` name those numbers.

Two consequences, and they are binding:

- **No map fact is guessed.** Not the region, not the campaign, not the name, not the blob id. A name is a
  row of the enum table; everything else is a read of the `AreaInfo` record; never a scrape from a wiki and
  never an inference from a coordinate.
- **The pack's folder structure is filled from those sources.** `config/World/Vanilla/maps/<campaign>/<area>/<subarea>/<map
  id>/` holds 128 of the 137 ids today: the name tables resolved each id's place, and that place's own
  page gave the area and the kind (`docs/38-layer-decisions.md`, D38, and the table in `docs/39-map-list.md`).
  The other 9 sit in `maps/unplaced/`: their place was not answered, so they were not placed in a folder
  nobody measured. The fold is a cross-check, not a measurement - the `AreaInfo` record is what confirms
  all three, and a read of it has not been taken.

The wiki's `List of locations` is a cross-check for the shape of the hierarchy - campaign, then area, then
the area's explorable areas, missions and towns-and-outposts - and never the source of a byte.

The names themselves are copied into this repository as a reference, so the table is never lost again:
`docs/39-map-list.md` (278 outposts, 378 explorables, and the pack's ids none of them names).

## Spawn points, and the outpost rule (owner, 2026-10-31)

Stated by the owner, and stored here because both are base facts the whole server depends on:

> "each outpost has multiple spawn points from a list, they are randomized when you enter: you end up at
> some random spawn. We have not all the spawn points on our server, just one of them. About spawn points
> there is another set, the ones that result from coming from an explorable (to position near the
> entrance), and there is another positioning event that comes from quests: the quest will decide where
> you spawn in a map."
>
> "One change we need to make immediately in the whole server, and it will be a base rule: every time we
> load - even if we are in an instance - (unless it was a disconnect, which we will implement further) we
> need to be moved to the LAST outpost they visited. That means we need to keep track of where we travel."

**Three sources of a position, and this world has two of the three.**

| source | what it is | here |
|---|---|---|
| entry spawn list | each instance has several spawn points and entry picks one, randomised | **implemented 2026-11-05**: every entry into an **outpost** takes a **random point of the map's own list** (`config/World/<pack>/maps/.../spawns.json`, `crates/asura-world/src/map_spawns.rs`), lands **scattered up to 250 units around it** (the radius is `config/Server/spawns.json`'s `spawn_scatter`, a server setting, not a compiled constant) on the point's plane, and the agent is created facing that point's `angle` (`Standing::at_spawn`, `crates/asura-world/src/agent.rs`). The list arrives with the map entry (`MapEntry.spawns`); a map whose pack states no list falls back to `map_config.spawn_x/spawn_y/spawn_plane`. The reference's own extra rows are still stored and still unread: `map_spawn` holds 1-4 per map (migrations `0021`, `0022`) and `asura_db::game::spawn_points` has no caller |
| entrance spawns | the positions used when a character arrives from an explorable, near the door it came through | **implemented 2026-11-05**: the **receiving map** owns them - its `portals.json` states, per map it can be entered from, the point and the facing (`arrivals`). A crossing reads that entry and writes it as the character's position before the redirect; the arrival connection stands there and faces that angle (`Standing::arrival`, `crates/asura-world/src/portals.rs`). A doorway is a **wall** stated as two endpoints (`arches`): a crossing counts only when the step passes through it, and a point-shaped doorway still counts within `REACH`. An outpost's *other* ways in (the world map, `/tp`, a login) pick a random spawn point instead - see the row above |
| quest placement | a quest can decide where in the instance a character appears | not modelled |

The archive carries these: the client's map data has three spawn lists, each entry a position, a facing
angle and a four-character zone tag, with a numeric tag naming the instance it belongs to
(`docs/22-world-model.md`, answer 5). `Map.Pathing.GetSpawnData(map_id)` reads them through the same
instrument that produced `data/portals.json`, so they can be dumped per instance the same way; until
then the pack's `spawns.json` is the list a map is entered from.

**The base rule, and what it costs.**

On every load a character is placed in the **last outpost it visited**, even when the load is into an
instance - so a character that logs out inside an explorable comes back to the outpost it set out from.
The exception is a *disconnect*, which is to be implemented separately and is not this rule.

Two consequences:

1. **Travel has to be recorded, not inferred.** The world must write the last outpost it placed a
   character in (`characters.map_id` holds a map, but the rule needs the last map whose type is an
   outpost, so a travel into an explorable must not overwrite it). `asura_db::game::settings` reads
   `last_outpost`, and `crates/asura-auth/src/travel.rs:170-196` substitutes it for **every** request
   whose destination is not a main town (`MAP_TYPE_MAIN_TOWN`), with map 148 for a character that has
   never been placed in an outpost.
2. **Every load goes through it**, the first one after login and every reconnect a travel produces - which
   means the doorway crossings built on 2026-10-31 keep working, because their destination is an instance
   the character is being moved *into*, and the rule decides where a *session* starts, not where a travel
   goes.

**Implemented 2026-11-01.** `characters.last_outpost` is the memory
(`crates/asura-db/migrations/0024_last_outpost.sql`), written only when the placement is an outpost
(`asura_db::game::remember_outpost`, `crates/asura-db/src/game.rs:308`) and substituted when the auth
service answers the client's request for an instance (`crates/asura-auth/src/travel.rs:163-196`). The
disconnect exception is still open.

**The entry spawn, 2026-11-05 (owner).** "We should not travel and preserve position except on a
disconnect; every time we enter an outpost is on a random spawn point from the list", and "when we
spawn we should get our position randomized 250 units around a circle from the spawn point". So a
**travel** never carries a position: `/tp` and the world map's request clear the saved position before
the redirect (`asura_db::game::forget_position`), and a redirected connection's teardown writes nothing
(`crates/asura-world/src/connection.rs`). A connection that ends **without** a travel is a disconnect,
and that one still saves where the character stood. A world-map request or `/tp` into an outpost then
stands on a random point of the city's own spawn list, scattered over a 250-unit circle whose radius is
`config/Server/spawns.json`'s `spawn_scatter`, facing that point's angle. The pack's list for Ascalon
City (pre-Searing), map 148, states two points with their angles; the file is
`config/World/Vanilla/maps/Prophecies/Ascalon (pre-Searing)/Outposts/148/spawns.json`.

**A doorway arrival is the door's, not a spawn point (owner, 2026-11-05).** "Portal travels are not
respecting our portals files, nor our arrival point"; "the portal is an invisible wall we cross ... it
should be a plane". The doorways are the pack's `portals.json` files: `arches` state the wall as its two
endpoints and the map it leads to, and `arrivals` state, per map the character comes from, the point it
stands on and the facing. A crossing is the step that passes **through** the wall (a segment test, not a
distance), and the arrival stands where the receiving map's arrival entry says - so walking from Lakeside
County into Ascalon City puts the character at the city's stated point, facing its stated angle, while
the outpost's random spawn list only answers the entries that are not doorways.

## Instances, districts, and party travel (owner, 2026-11-02)

Status of every rule below, and of the older statements this page carries, is tracked in
[`36-game-rules-ledger.md`](36-game-rules-ledger.md). Where the two disagree, the ledger wins and
this page is corrected.

The game rules, stated by the owner with the district behaviour given in game terms:

| rule | the game's behaviour |
|---|---|
| **Map vs instance** | A map is the static definition. An **instance** is one running copy of that map's simulation. A character or a party stands in an instance, never in "a map". |
| **Outposts** | Shared: one instance of an outpost serves many characters. |
| **Districts** | An outpost may have several instances of itself, distinguished by district number. The client presents them as the same outpost. |
| **District creation** | **Automatic** when an outpost instance reaches its player allotment. The allotment follows the server's hardware and efficiency, is **configurable, and starts at 100**. An outpost opens with **one** district; when it fills, the next is created and shown - no pre-built list of many. |
| **Explorables** | Private: **each party gets its own instance** of the map. One party, one copy, no sharing. |
| **Travel** | The **party is the unit**. The leader drives it; the party moves together. |
| **Portal crossing** | When **any party member** crosses a portal into another map, the **whole party** is brought through: every member loads the destination instance and plays there. |
| **Leader district hop** | The whole party follows **after a countdown**. |
| **Non-leader district hop** | The character is asked whether it wants to **leave the current party**; the hop is not a party move. |
| **District list** | The client **asks** the auth link for district counts and receives `(language, count)` pairs. A hop is the game link's `CtoS 177 TravelMission`, and the server answers with a redirect (`StoC 421`); the load message carries the district the HUD shows. Message shapes in the ledger, rows E27-E31. |
| **Explorable teardown** | A **network disconnect** keeps the instance **15 minutes** for reconnection - **explorable-only**; outposts have no reconnection hold, and **logging out is a leave, not a disconnect**. Everyone leaving by any normal means (portal, scripted/history travel, quitting the client) disposes it **immediately**. When the 15 minutes expire the copy is disposed and the character reappears at its **last visited outpost** - no reconnection into a disposed instance. |

The two causes are distinguishable on the wire (ledger, E6-E7): a clean logout sends its disconnect
messages, and a lost member is held as **soft-disconnected** (`Player.flags` bit `0x04`, cleared by
`MemberConnectSoft`) rather than removed. A socket that ends without the clean-shutdown messages is a
disconnect.
| **Unlocks** | Travel is gated on unlocked maps, and a character starts with **one**. A scripted or forced travel **unlocks the destination at arrival** rather than being refused. |
| **World-map travel** | Reaches **outposts only**, and only an **unlocked** one. Anything else is reached by portal or by a scripted travel. |
| **Entry spawn** | Outposts carry many scattered spawn points. Any travel that is not a portal walk picks one **at random** and places the character **scattered around it** - random offset and facing. Soft spread, not stacking. A **portal walk** uses the arrival position the destination provides (the destination map's `portals.json` `arrivals` entry for the map it came from); with none, the destination's own spawn point answers. |
| **Party size** | Starts small, capped at **8** for most content, **12** in some game modes, **25** in some instances. A tunable, not a constant. |
| **Party persistence** | Travel does not break a party. It disbands only on a script or when the destination's **cap is smaller** than the party. Leadership never changes, except one focused dungeon that randomises positions. |
| **Member disconnect** | A disconnect is the same for the leader as for anyone else: the party remains, and that member's **heroes and henchmen deactivate until it reconnects**. |
| **Party composition** | Characters, plus the player's **heroes** (account-owned), plus **henchmen** hired for the party. The client holds players, heroes and henchmen as distinct lists (ledger, E5). |
| **Heroes** | Introduced in Nightfall but **usable everywhere**; the option stays open for any mode to allow them. A hero belongs to its player and **disconnects with it**. |
| **Henchmen** | Mercenaries hired like heroes, and they **travel with the party** to outposts and missions. On arrival at an outpost, one that **exists there is kept** and one that **does not is kicked** - the roster differs per outpost. |
| **Travel alone** | A player may enter with a party of one, with heroes, or any combination of heroes, henchmen and players. |
| **Party truncation** | Arriving at a **smaller cap** truncates the party: the **leader plus its heroes** hold the slots, and the remaining members are split into **parties of one**, each keeping its own heroes. |
| **District assignment** | A travel that **names** a district is placed in that district; a request with district 0 takes the **first district with available slots**. The language travels with the request (R57; R22 governs the district-0 case). **A full or closing district cannot be entered** (R58). |
| **Instance contents** | NPCs, enemies and props exist **when the instance is created**, before any player arrives, unless a script makes them appear later or disappear. Most NPCs are static; some walk. Paths and behaviour come from **Lua configuration**, not hardcoded rules. |
| **Enemies** | Spawn once. **No respawn unless scripted**, and a new instance **wipes their state**. Most enemies **only spawn and tick while inside a player's compass range (10,000 units)** - outside it they are not simulated. |
| **Instance lifetime** | Lives **only while a party is inside** (plus the 15-minute disconnect hold). Re-entering the same map creates a **new instance**. **Outposts** empty are disposed after **15 minutes of inactivity** - inactivity meaning *nobody inside*; the disconnect hold itself is **explorable-only**. |
| **One party per instance** | PvE instances hold **exactly one party**; a party always shares one instance and is never split across copies. **PvP** is the exception - different parties fight inside one instance. |
| **Missions** | An instance with its own success/fail conditions, entered only from an outpost that offers it ("Enter Mission", or an NPC). No resurrection shrines; a party wipe fails it and returns the party to the initiating outpost. |
| **Ticking** | The real game ticks an instance. Ours is **configurable** - we may choose not to tick one. |

**Identity of an instance** is therefore not the map: `(map_id, kind, owner, ordinal)` - outpost
`(map, outpost, region, district n)`, explorable `(map, explorable, party)`. A district is an instance
of the same map, not a different map. Whether district numbering is scoped per region (so district 3 of
an outpost exists once per region) is not yet observed; keying by region is the superset that collapses
to a single value if it is not, so no rule depends on the answer.

**Lifecycle.** An outpost instance persists for the server's life and gains siblings as it fills. An
explorable instance is created when its party enters and exists while that party is in it.

Compliance this sets for the code, to be done as the instance work lands (not a design choice, a
consequence of the rules above):

- an instance registry keyed by instance identity, with `map_id` as an attribute for lookups;
- a party that is the travel unit, crossing from instance to instance through the database's party
  ledger rather than living in `asura-auth` (owner, 2026-11-06; D25; `docs/45-party-travel.md`);
- a portal crossing that resolves the destination **once for the party** and brings every member into it,
  whoever crossed - not a per-character travel;
- a district hop that is party-wide with a countdown for the leader, and party-leaving for a non-leader;
- a capacity trigger that creates the next district instance of an outpost (allotment configurable,
  hardware-tied, starting at 100; one district per outpost at first, the next on overflow);
- instance teardown split by cause: **disconnect -> 15-minute hold**, **graceful exit -> dispose at once**,
  and **expiry of the hold -> dispose plus placement at the last visited outpost**;
- travel gated on the unlock bitmap, with scripted and forced travel unlocking at arrival;
- entry placement that picks a random spawn point and scatters the character around it;
- party capacity as configuration (8 / 12 / 25 by content), never a constant;
- a party that survives travel, disbands only on a script or a smaller destination cap, and keeps a
  fixed leader;
- a disconnect that keeps the party and deactivates that member's heroes and henchmen until it
  returns;
- world-map travel restricted to **unlocked outposts**; portal and scripted travel for the rest;
- district assignment: a **named district is honoured** when it can be entered, and a **district 0**
  request takes the first with a free slot; the language travels with the request;
- an instance whose NPC/enemy population is **created with the instance** and whose enemies **spawn and
  tick only inside a player's compass range** (10,000 units), with no respawn unless scripted.

## The one governance question the reading raised

The three portal model ids are the instrument's table, cross-sourced there from another project.
`docs/adr/0013-py4gw-stealth-is-our-link-to-the-client.md` says no table from it enters this tree. To read
map 148's portals at implementation time this project needs those three file ids, either transcribed as
data with this paragraph as their evidence, or re-measured through the instrument itself. That is the
owner's call, and it is asked rather than assumed.

## What this model already explains

- **`0x01A0` is the instance table**, not an anonymous list: ten counted batches `(instance id, region
  value)` pairs, 137 pairs over 147 instance ids, running to 877 - the same number the manifest calls
  `MapId_Count` (`docs/21-surrounding-packets.md`, "The map table";
  `crates/asura-protocol/src/game/instance_table.rs:10-14`). **This world sends it**: ten batches in every
  load block (`crates/asura-world/src/map.rs:432-437`, `GAME_SMSG_INSTANCE_TABLE`, `Emitted`).
- **The world-map travel is per-instance, not per-place**, which is why `is_explorable` travels exist in
  the live tours (`maps-tour3.jsonl` seq 16, instance 101, flag 1): the map screen offers instances, and
  what the flag describes is the destination instance's type.
- **Why a portal touch could do nothing in our world**: the client draws the portal from its own map
  data, so the world does not have to place it. What the world still does not send is the population;
  the instance table (`0x01A0`) it does send, ten batches per load (`crates/asura-world/src/map.rs:432-437`).
  The spawn `map_config` carries for 148 is `9541.873, 4796.456` (`tools/map-config.sql:16`), and a
  crossing into 148 lands on the point recorded for that door instead - `7739, 5900` for the doorway in
  146 (`crates/asura-world/src/portals.rs:40`).
