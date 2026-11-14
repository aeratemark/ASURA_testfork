# A.S.U.R.A. - experimental pre-Searing fork

A private Guild Wars 1 server (Rust + PostgreSQL) that only runs pre-Searing. This is an experimental fork:
things change fast and some features are half-built on purpose.

**Feature checklist:** [`MILESTONES.md`](MILESTONES.md) lists every feature built so far and what is not
built yet, each marked `[x]` client-confirmed, `[o]` test-proven, `[~]` built but unproven, `[ ]` not done.

## What works today

- **Login and characters:** patched client, web gate, key exchange, character list and selection, EULA
  once per account, character creation, position and items saved between sessions.
- **The 11 pre-Searing maps:** outposts and explorables in one world process, every travel portal,
  click-to-walk pathing, keyboard movement, two players seeing each other.
- **Travel:** doorways, the world map, `/tp <map>` and `/maps`; a travel moves the character the
  transfer names, not the id the client presents (`docs/56-character-on-travel.md`).
- **NPCs:** 79 bodies across the 11 maps from preforged's `gw-npcs 1` catalog, with per-map placements,
  routes, a behavior runtime and one captured dialog (`docs/49-npc-framework.md`).
- **Merchants, collectors and storage:** six merchants, eight collectors with 24 offers, Xunlai storage
  and gold; buy, sell and collector requests `0x804A`/`0x804D` (`docs/53-npc-services.md`).
- **Skills and combat:** the 44 pre-Searing skills, attributes and the skill bar, casting, recharge,
  adrenaline, armor damage, conditions, maintained effects, fields, melee and ranged attacks, death and
  resurrection (`docs/52-skills.md`).
- **Items:** 546 definitions, the catalogue and modifier foundation; drops, pickups, equips, unequips
  and moves, one transaction per operation (`docs/47-item-system.md`, `docs/50-item-framework.md`).
- **Quests and experience:** the captured quest pack with offer, accept, turn-in, log and per-character
  state, quest 80 end to end; the experience curve to level 20 and skill points, with `/.exp` and `/xp`
  test commands (`docs/58-quests.md`, `docs/54-experience.md`).
- **Chat and compass:** `!`, `#`, `$` channels, whispers, `/roll`, `/age`, `/stuck`, party compass
  strokes and called targets (`docs/51-compass-and-chat.md`).
- **Titles:** the Hero panel's Titles tab with retail's data; Tyrian Cartographer earned from exploration.
- **Party and guilds:** a party window and party travel, a guild panel - both highly experimental.
- **Interface layout:** the client's saved window layout survives a full relog.

The NPC, skill, merchant, quest, experience and item paths are built and unit-tested, but only what
`MILESTONES.md` marks client-confirmed has run against a real client.

**Not built yet:** splitting a stack by hand (`0x8075`), swapping two items in one move, creature loot
and kill experience, projectile flight, pets, corpses and minions, quest objectives other than a talk and
a turn-in (kill, item, escort, timer), item rewards, second-profession unlocks, player trade, the friends
list, heroes and henchmen, and the guild features listed below. The bear, hunter and spider behaviors are
research only (`docs/60-pre-searing-behaviors.md`).

## Run it

1. Install Rust and PostgreSQL.
2. Double-click `run_server.py`. On the Setup tab: **Run all setup steps** the first time; after that
   **Build** (code changed), **Create tables (schema)** (a new migration), **Add test accounts and maps
   (seed)** (the seed file changed). Then **Start server** on the Server tab.
3. Patch the client: run `client-patcher/run_patch_gui.bat` and use **Quick setup**, or see
   [`client-patcher/README.md`](client-patcher/README.md) for the command line.
4. Log in with an email from `tools/seed-dev.sql` (`user1@example.com` ... `user3@example.com`), or
   with your own from `tools/seed-local.sql` (not committed). The password is not checked.

Logs are in `logs/`. A config file in `config/Server/` is read once at start: restart the server after
changing one. After pulling a change that adds a migration, run **Create tables (schema)** once.

## How it fits together

```
client --portal login--> web gate  :6601  (who am I?)
client --login--------> gateway/auth :6112  (key exchange, session, character, settings)
client --gameplay----> gateway :6112 --relays--> world (instance server) :6113+
                                     everything durable --> PostgreSQL
```

| crate | what it does |
|---|---|
| `asura-gateway` | the socket layer: one listener, hands login to auth and relays gameplay to a world |
| `asura-auth` | login, characters, account settings, travel decisions |
| `asura-world` | the maps: outposts, explorables, movement, items, NPCs, skills, combat, vendors, quests, chat, titles, party, guilds |
| `asura-webgate` | the portal login the client asks before it logs in |
| `asura-supervisor` | starts and watches the services (`config/services.toml`) |
| `asura-protocol` | the client's messages, byte for byte |
| `asura-rules` | game decisions as pure functions over values handed in (skills, quests, experience, titles, party, guilds) |
| `asura-config` | reads `config/Server/*.json` |
| `asura-db` | the database, its migrations (`0001`-`0038`) and queries |
| `asura-fakeworld` | a test harness world: registers, heartbeats, goes silent, gets reaped |

**Where values live:** `config/Server/` holds what this machine runs on (intervals, movement, portals,
titles, guilds, quests, the default account settings); `config/World/Vanilla/` holds what the world
contains (one folder per map with its NPC placements, NPC definitions, skills, combat, attributes,
experience, items, upgrades, vendors, dialogs, quests, character defaults, party sizes and travel
timings). No game value is compiled in.

## Features in detail

### Maps, doorways and movement

- **Doorways:** the pack's `portals.json` (`config/World/Vanilla/maps/`) is the only source. A wall
  triggers on a step that crosses or touches it, endpoints included, and every simulated step is tested;
  a point doorway uses a box edge (`config/Server/portals.json`). Every pre-Searing travel portal is
  there, with positions read from `Gw.dat`.
- **Arrivals:** a doorway transfer writes its source map to `t_transfer.arrival_from_map_id` (migration
  `0031`), so you land at the door you came through.
- **Maps:** the pack's `basic.json` `map_file_id` outranks `t_map_config` (Northlands 147 is 116025,
  fixing its load crash), and every map carries `name_id`, the client's `AreaInfo` string id
  (`tools/add-map-name-ids.py`). An outpost loads with `is_explorable 0`, as the live server sends.
- **Manual movement:** a reported direction is traced through connected walkable cells
  (`crates/asura-world/src/pathfind/directional.rs`); distances are in
  `config/Server/directional_movement.json`. Provisional: better, not perfect, not client-confirmed.
- **Cartography:** explored area is saved per character (migration `0027`). `ASURA_CARTOGRAPHY=full`
  sends a captured 105-byte retail stream (2,140 blocks).

### NPCs and services

The framework reads one `npcs.json` per map beside the pack's `npcs/definitions.json`, `npcs/sight.json`
and `npcs/behaviors.lua`. A definition is a type, a placement is one body on one map, and the instance
owns the running agent; the load's agent array and the measured entry burst are in
`docs/49-npc-framework.md`. The whole pre-Searing is populated from preforged's `gw-npcs 1` catalog:
79 bodies across 145-148 and 160-166, including Ascalon City's 23.

- **Behavior:** a Rust behavior tree drives each agent; Lua in `config/World/Vanilla/npcs/behaviors.lua`
  supplies patrols, the Plague Worm cycle and the boss emerges (`crates/asura-world/src/npc/`).
- **Dialogs:** the Purveyor [Bonus Merchant's captured greeting and seven choices, two with measured
  answers; no other pre-Searing NPC has a captured greeting (`crates/asura-world/src/dialog.rs`,
  `docs/55-npc-dialogs.md`).
- **Services:** six merchants (Sanura, Riley, Vencel, Gram, Spence, Laren), eight collectors with 24
  offers, and Xunlai storage. The pack is `config/World/Vanilla/vendors.json`; the handlers and the pure
  rules are `crates/asura-world/src/vendor/` (`docs/53-npc-services.md`).
- **Wire:** the merchant window is `0x00C4` owner, `0x0161` per stock row, `0x0084` stock list, `0x00CA`
  open and `0x00C3 {11, 0}`; buys and sells are `0x804D` with quantities and `0x804A` for whole stacks.
- **Divergences:** storage and its gold are per character, not per account; collector armour is not
  customised; no live sale, Xunlai or gold-move capture exists yet.

### Skills, attributes and combat

- **Content:** the 44 skills obtainable in pre-Searing from preforged's 2022 table, their attributes and
  the combat constants in `config/World/Vanilla/skills.json`, `attributes.json` and `combat.json`.
- **Rules:** whether a bar may stand, the four bar edits, attribute edits, rank costs and the refund rule
  are pure functions in `crates/asura-rules/src/skills.rs`.
- **Engine:** `crates/asura-world/src/skill/engine.rs` runs casts, payment, recharge, adrenaline, damage
  against armor, healing, conditions, lasting and maintained effects, fields, chains, interrupts,
  knockdown, death and resurrection. `docs/52-skills.md` is the design and the not-built list.
- **Rows and wire:** migration `0034_character_skills.sql` stores learned ids, the bar and ranks;
  `crates/asura-world/src/skill/wire.rs` and `crates/asura-protocol/src/game/skill.rs` turn engine
  events into messages.
- **Unmeasured:** no ASURA capture contains a skill being used, so every skill message is `Inferred` in
  `crates/asura-protocol/src/packets.rs`; projectile flight, creature loot, kill experience, pets,
  corpses and minions are not built, and runes do not change effective ranks yet.

### Items

- **Definitions:** 546 item definitions in `config/World/Vanilla/items.json`, with the catalogue and
  modifier foundation in `crates/asura-world/src/item_catalogue.rs` and `item_modifier.rs`;
  `config/World/Vanilla/upgrades.json` holds the upgrade rows.
- **Operations:** `crates/asura-world/src/inventory_model.rs` is an atomic `InvModel::apply(Op)` over
  `ItemKind`, `BagKind` and the client's own limits; every handler runs rules, one transaction and a
  reload (`docs/50-item-framework.md`).
- **Wire:** drops, pickups (`0x803F`), moves, equips and unequips (`0x804F`) are handled in
  `crates/asura-world/src/item.rs`; migration `0033_item_definitions.sql` and the item rows store the
  result.
- **Not built:** splitting a stack by hand (`0x8075`) and swapping two items in one move are not built;
  the protocol table names the split request.

### Quests and experience

- **Quests:** the captured pre-Searing pack carries each quest's log state, marker, encoded name, text
  and objectives. `crates/asura-rules/src/quest.rs` holds the state machine,
  `crates/asura-world/src/quest.rs` runs the offers, accepts, turn-ins and log messages, and migration
  `0038_character_quests.sql` keeps them. Quest 80 (the Town Crier to Sir Tydus) runs end to end
  (`docs/58-quests.md`).
- **Not built:** kill, item, escort and timer objectives; item rewards; second-profession unlocks; the
  skill-point message; per-map markers.
- **Experience:** `0x00E9` carries total experience, level and skill points; the curve answers the level
  to 20, and `/.exp` and `/xp` set or print it for testing (`crates/asura-world/src/experience.rs`,
  migration `0036_character_experience.sql`, `docs/54-experience.md`).
- **Not built:** a kill awards nothing; the level-up moment on a live client has not been measured.

### Compass and chat

- **Compass:** `0x802B DRAW_MAP` relays the stroke as `0x0091` to the drawer's party, and
  `0x8023 TARGET_CALL` sends `0x0034` for an agent in the instance
  (`crates/asura-world/src/compass.rs`).
- **Chat:** party `#`, whispers, `/roll <sides>` and `/age` are handled in
  `crates/asura-world/src/chat.rs`; the values are in `config/World/Vanilla/chat.json`.
- **Unmeasured:** both compass messages and these chat additions come from preforged and are not in an
  ASURA capture (`docs/51-compass-and-chat.md`).

### Pre-Searing behaviors

- **Plague Worm:** the Lakeside cycle - emerge, stand out, submerge, remove, re-appear at the same id and
  coordinates - is implemented as `worm_lakeside` in `npcs/behaviors.lua` with the captured projectile
  (`docs/59-plague-worm-burrow.md`).
- **Research only:** the Black Bear taming trick, the Bear Hunters' minigame and the Deadly Crypt
  Spiders' ceiling drop need triggers, spawn/despawn and skill mechanics that are not built
  (`docs/60-pre-searing-behaviors.md`).

### Titles

The Hero panel's Titles tab works, with retail's own data. Design and evidence:
`docs/40-titles-system-research.md` (section 13 says where every value comes from).

- **Wire:** `crates/asura-protocol/src/game/title.rs` - the four title messages (`0x00F3` rank record,
  `0x00F4` shown rank, `0x00F5` point update, `0x00F6` track) and the show request (`0x8058`).
- **Rules:** `crates/asura-rules/src/titles.rs` - which rank a point count is on, what a load or a point
  change sends, whether a title may be shown, and checks for the client's crash rules.
- **Content:** `config/Server/titles.json` - Tyrian Cartographer, Drunkard, Survivor, Kind of a Big Deal,
  Legendary Defender of Ascalon, Sweet Tooth. A rank name not captured yet is `0101` (empty).
- **Storage:** migration `0028_titles.sql` - `t_character_titles`, `t_account_titles`, and
  `t_characters.display_title`.
- **World:** `crates/asura-world/src/titles.rs` - the title block in the load, a shown title in
  introductions, show/hide (`0x8058`/`0x8059`), and `add_points`/`set_points`.
- **Cartographer** earns from the share of pre-Searing's map you have explored (this server's own rule -
  retail does not count pre-Searing).
- **`/title <name or id> <points>`** sets points for testing, e.g. `/title ldoa 20`. The answer is in the
  world log.

### Party travel (experimental)

**Highly experimental, far from finished.** The rules below are what is built; a case no capture covers is
a guess. Design and evidence: `docs/44-party-invites.md`, `docs/45-party-travel.md`.

- **Party window:** invite, accept, kick. An invitation may name a party of more than one; the merge
  leader is the inviter on equal sizes, otherwise the larger party's leader.
- **Doorways:** any member crossing brings the whole party, to the door's arrival point.
- **World map:** the leader in an outpost starts a measured countdown (`0x01DA` then `0x01BC`, 10 s),
  then the party goes; anyone else - or anyone in an explorable - leaves the party and travels alone.
  The leader cancels with `0x80A3`, answered by `0x01DA {0, 0}`.
- **Members left behind:** a member without the destination unlocked stays; the leader is asked first
  with `0x01E7 {5}`. A non-leader's world-map travel gets `0x01E7 {4}`, the "you will leave your party"
  popup.
- **Arrival:** the first member builds the party from the ledger; one that never arrives is dropped
  after the grace time.
- **Disconnect hold:** a member that drops in an explorable stays in the party, grey, for 15 minutes;
  logging back in puts it back in the party's map.
- **`/resign`:** each member dies after `resign_secs` (5 s); the last death sends `0x01D9`, and the
  leader's `0x80A7` returns the party to its last outpost.
- **Storage and values:** migration `0030_parties.sql`; `config/World/Vanilla/party_travel.json` and
  `party_sizes.json`.
- **Not built:** one explorable per party, heroes and henchmen, the members' side of a cancel, and
  `0x01DA` before a solo travel or a defeated party's return.

### Guilds (experimental)

**Highly experimental, far from finished.** Only what the two captures show is answered; no live client
has run the guild panel against this build yet. Design and evidence: `docs/41-guild-system-research.md`.

- **Wire:** `crates/asura-protocol/src/game/guild.rs` - the load's guild block, a guilded peer's badge,
  the registration window, and readers for create `0x8036`, invite `0x80B5`, leave `0x80BB` and
  announcement `0x80BE`.
- **Rules and world:** `crates/asura-rules/src/guild.rs`, `crates/asura-world/src/guild.rs`.
- **`/guild create`** opens the Guild Registration window in place of a Guild Registrar NPC.
- **Storage and values:** migration `0029_guilds.sql`; `config/Server/guilds.json`. Guilds are free for
  now: the two prices are read and logged only.
- **Not built:** the invitee's `0x0123` and accept, kicks, promotions, demotions, guild chat, capes,
  guild halls, and the roster beyond the founder's own row.

### Interface layout (account settings)

- The client uploads its interface state on the auth connection: `0x8021` announces the size, `0x8020`
  carries it in chunks of up to 512 bytes. The server stores it in `t_account.account_settings`
  (migration `0032`) and returns it in `0x0016` at the next login. Owner-confirmed: the layout survives a
  full relog. Evidence: `docs/46-ui-settings-persistence.md`.
- A new account, or an upload outside `1..=2048` bytes, gets `config/Server/account_settings.json`.
- **Not done:** the `0x00B5` preferences blob at area load, and storing the character-select settings
  (`0x8009`, acknowledged only).

## Tools

| tool | what it does |
|---|---|
| `run_server.py` | setup (build, schema, seed) and start/stop in one window |
| `client-patcher/run_patch_gui.bat` | patch `Gw.exe`, manage the hosts block, launch the client |
| `tools/sniff-capture-gui.py` | capture a live client by type (titles, movement, travel, guild, ...); needs 32-bit Python 3.13 and admin |
| `tools/seed-local.sql` | your own logins, applied after `seed-dev.sql`, kept out of git |
| `ASURA_MOVE_TRACE=1` | log every movement event in the world log |
| `--capture PATH` / `ASURA_CAPTURE` | write every game message both ways, decrypted, as JSONL |
| `tools/test-touched.ps1` | run only the tests a change touches |

## Open

- Titles: rank names still missing for Survivor 2, Sweet Tooth 2, Kind of a Big Deal 3 and 5,
  Cartographer 2-6, and all of Party Animal. No earning yet for Drunkard, Sweet Tooth, Party Animal,
  Survivor, LDoA or Kind of a Big Deal. A shown title under the player's name does not draw yet, and the
  server's reply to hiding a title is not captured.
- Skills: no live cast capture yet; projectile flight, creature loot and kill experience, pets, corpses
  and minions, rune bonuses and Deep Wound's health cap are not built.
- Quests: kill, item, escort and timer objectives, item rewards, second-profession unlocks and per-map
  markers are not built.
- NPCs and merchants: the framework, the one dialog and the vendor windows are built but have not run
  against a client; only the Purveyor's greeting is captured.
- Items: splitting a stack and swapping two items in one move are not built; creature loot is not built.
- Experience: a kill awards nothing and the level-up moment has not been measured on a live client.
- A brand-new account with no character cannot create one (login refuses travel without a selected
  character, as the reference does); the seed gives every account one character as a workaround.
- Starting outposts: only Factions is measured; every other campaign starts in Ascalon City (148).
- Doorway triggers, the Northlands fix and manual movement are waiting for a client check.

## Read more

| document | what it settles |
|---|---|
| [`MILESTONES.md`](MILESTONES.md) | the feature checklist: what is built, proven and still open |
| [`AGENTS.md`](AGENTS.md) | the binding rules for anyone (or any agent) changing this repo |
| [`docs/HANDOFF.md`](docs/HANDOFF.md) | the evidence behind those rules and where work stands |
| [`docs/19-running.md`](docs/19-running.md) | how to run it, and troubleshooting |
| [`docs/02-architecture.md`](docs/02-architecture.md) | processes, scaling, where state lives |
| [`docs/17-running-on-linux.md`](docs/17-running-on-linux.md) | running on Linux and in Docker |
| [`docs/03-protocol.md`](docs/03-protocol.md) | the protocol |
| [`docs/49-npc-framework.md`](docs/49-npc-framework.md) | the NPC framework and the pre-Searing population |
| [`docs/53-npc-services.md`](docs/53-npc-services.md) | merchants, collectors, Xunlai storage and gold |
| [`docs/52-skills.md`](docs/52-skills.md) | skills, attributes and combat |
| [`docs/50-item-framework.md`](docs/50-item-framework.md) | the item framework and its operations |
| [`docs/58-quests.md`](docs/58-quests.md) | quests and their log wire |
| [`docs/54-experience.md`](docs/54-experience.md) | experience, levels and skill points |
| [`docs/51-compass-and-chat.md`](docs/51-compass-and-chat.md) | compass strokes, called targets and chat |
| [`docs/60-pre-searing-behaviors.md`](docs/60-pre-searing-behaviors.md) | the worm, bears, hunters and spiders |
| [`docs/40-titles-system-research.md`](docs/40-titles-system-research.md) | the title system |
| [`docs/45-party-travel.md`](docs/45-party-travel.md) | party travel |
| [`docs/41-guild-system-research.md`](docs/41-guild-system-research.md) | guilds |
| [`docs/46-ui-settings-persistence.md`](docs/46-ui-settings-persistence.md) | the saved interface layout |
| [`client-patcher/README.md`](client-patcher/README.md) | patching the client |
