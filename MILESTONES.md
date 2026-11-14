# Features

The list of what this server does. One line per feature.

`[x]` a real client did it. `[o]` a test proved it, no client yet. `[~]` built, but no test and no client
yet. `[ ]` not done.

## Client

- [x] A stock `Gw.exe` is patched into `Gw.custom.exe` (`client-patcher/patch-gw.py`; rebuild is byte identical, 2026-10-28)
- [x] The patched client runs beside another copy, mutex check removed
- [x] The client asks the web gate who it is, four `/Spawned/WebGate` endpoints (`docs/03-protocol.md` #4)
- [x] Login: key exchange, session, character list (`docs/06-reference-session.md`)
- [x] EULA handling: accepted once per account, not on every login (2026-10-28, `account.eula_accepted` = 26, second login asked nothing; `logs/auth.log` is append-only and has rolled since, so re-check with `crates/asura-auth/src/lib.rs:587-601`)
- [x] The client reaches character select and enters an outpost (build 39112)

## Character

- [x] Position saved on leaving, resumed on entering (`asura-db/tests/inventory.rs`)
- [x] Items: block built from rows; drops, moves and equipment persist
- [x] Chat on `!`, `#`, `$`; `/stuck` and `/maps` answer
- [o] Character creation: name, appearance, profession, starting equipment (`a_character_is_created_in_a_creation_instance`)
- [ ] Character deletion by name, scoped to the owning account (implemented, no client session yet)
- [x] Character selection message `AUTH_CMSG_CHANGE_PLAY_CHARACTER` (`crates/asura-auth/src/lib.rs:416-462`; a real client selected "Asura One", `logs/auth.log` 2026-10-30)

## Maps and movement

- [x] A world hosts several maps in one process
- [x] Travel: request, redirect, reconnect into the destination spawn
- [x] Destination loaded on demand under `--on-demand`, measured at 0-2 ms per map against the real client; without the flag a world builds every map in `map_config` at start-up, which is the shipped default (`config/services.toml` passes no flag)
- [x] Click-to-walk answers with pathfound waypoints
- [x] Alternating click-to-move, keyboard movement and autorun across the Ascalon City bridge no longer teleports or breaks pathing (owner confirmed in the real client, 2026-11-01; `docs/HANDOFF.md`)
- [o] No path: `MOVE_TO_COORD` refuses and sends nothing - the reference's position broadcast disconnected a live client once and snapped it into a wall once (`crates/asura-world/src/agent.rs:722-743`); the test that asserted a broadcast is `#[ignore]`d pending the owner's ruling (`crates/asura-world/tests/load.rs:3169`)
- [x] World tick and movement cadence match the live client (`recordings/client/feature/movement/movement-sniff.log`)
- [o] Two clients in one map see each other

## Services

- [x] Auth answers login, keeps sessions, refuses a travel cleanly
- [x] The front door relays the game connection to the world the travel was placed on
- [o] Placement reads the registry, heartbeats decide freshness, a stale world is not offered
- [o] Pool capacity refuses a full world without disturbing a lease
- [o] The reaper frees leases a silent world leaves
- [o] The front door dials a world registered at a second address, not its bind address
- [o] The newest build is the only one offered; a full newest build refuses instead of falling back

## Travel commands and doorways

- [x] `/tp <alias>` travels: GWPS's 46-byte `0x01A6` plus the 12-byte `0x0099`, reconnect, destination spawn (`docs/HANDOFF.md`, "Where this stands")
- [x] `/maps` prints the alias table
- [o] Every pre-Searing travel portal: positions from `Gw.dat`, destinations, landing points (`crates/asura-world/tests/portals.rs`)
- [~] Doorways come only from the pack's `portals.json`; a wall triggers on a crossing or touching step, endpoints included; a point door uses a box from `config/Server/portals.json` (awaiting client confirmation, `docs/HANDOFF.md`)
- [~] A doorway transfer records its source map, so the arrival point matches the door used (migration `0031`); 165 and 166 name 161 and 162 as their source
- [~] The pack's `map_file_id` outranks `t_map_config`: The Northlands (147) loads as 116025 (load crash repair, awaiting client confirmation)
- [~] Every map carries `name_id`, the client's `AreaInfo` string id (`tools/add-map-name-ids.py`)
- [~] An outpost loads with `is_explorable 0`, an explorable with 1, as every load in `recordings/client/` does (`crates/asura-world/src/map.rs`)
- [~] Manual movement traces connected walkable cells (`pathfind/directional.rs`, `config/Server/directional_movement.json`); owner: "better but not perfect", not accepted

## Exploration and titles

- [~] Explored area saved per character (migration `0027`)
- [~] `ASURA_CARTOGRAPHY=full` sends the captured 105-byte retail stream (2,140 blocks)
- [x] Hero panel Titles tab: tracks, rank names and exploration points draw (live client, 2026-11-05; `docs/40-titles-system-research.md`)
- [x] Tyrian Cartographer earned from pre-Searing exploration (this server's own rule)
- [~] Title content in `config/Server/titles.json`: Cartographer, Drunkard, Survivor, Kind of a Big Deal, LDoA, Sweet Tooth
- [~] Title storage per character and per account, plus the shown title (migration `0028`)
- [~] `/title <name or id> <points>` sets points for testing
- [ ] A shown title draws under the player's name (being tested)
- [ ] Earning for Drunkard, Sweet Tooth, Party Animal, Survivor, LDoA, Kind of a Big Deal
- [ ] Rank names for Survivor 2, Sweet Tooth 2, Kind of a Big Deal 3 and 5, Cartographer 2-6, all of Party Animal

## Party (experimental)

- [~] Party window: invite, accept, kick, leader; merges pick the inviter on equal sizes, otherwise the larger party's leader (`docs/44-party-invites.md`)
- [~] A doorway crossing by any member moves the whole party
- [~] World-map travel by the leader: measured `0x01DA` then `0x01BC` 10 s countdown; leader cancel `0x80A3`
- [~] Member without the destination unlocked: leader asked with `0x01E7 {5}`; non-leader travel asked with `0x01E7 {4}` and leaves the party
- [~] Arrival rebuilds the party from the ledger; a member that never arrives is dropped after the grace time
- [~] Disconnect hold: a member dropped in an explorable stays 15 minutes and returns on login
- [~] `/resign`: each member dies after 5 s, `0x01D9` on the last, `0x80A7` returns the party to its last outpost
- [~] Storage `t_parties`, `t_party_members` (migration `0030`); values in `config/World/Vanilla/party_travel.json`, `party_sizes.json`
- [ ] One explorable per party, heroes and henchmen, the members' side of a cancel, `0x01DA` before a solo travel

## Guilds (experimental)

- [~] Guild block in the load and a guilded peer's badge (`crates/asura-protocol/src/game/guild.rs`)
- [~] Create `0x8036`, invite `0x80B5`, leave `0x80BB`, announcement `0x80BE` (`crates/asura-rules/src/guild.rs`)
- [~] `/guild create` opens the Guild Registration window (window messages are a guess)
- [~] Storage `t_guilds`, `t_guild_members`, `t_guild_invites` (migration `0029`); guilds are free for now
- [ ] The invitee's `0x0123` and accept, kicks, promotions, demotions, guild chat, capes, guild halls, full roster

## Account settings

- [x] The client's interface layout survives a full relog: `0x8021`/`0x8020` upload stored in `t_account.account_settings` (migration `0032`), returned in `0x0016` (owner-confirmed; `the_clients_settings_upload_is_returned_at_the_next_login`)
- [ ] `GAME_SMSG_PREFERENCES_BLOB` `0x00B5` at area load
- [ ] Character-select settings `0x8009` stored (acknowledged only)

## Tools

- [~] `run_server.py`: setup (build, schema, seed) and start in one window; finds 64-bit `psql`
- [~] `client-patcher/patch-gw-GUI.py`: quick setup, patch, hosts block, launch
- [~] `tools/sniff-capture-gui.py`: capture types, Go and a clean Stop
- [~] `tools/seed-local.sql`: a machine's own logins, applied after `seed-dev.sql`, kept out of git
- [o] `ASURA_MOVE_TRACE=1` movement log and `--capture` decrypted message log

## Not built

- [ ] NPCs in maps (research only: `docs/30`, `docs/32`)
- [ ] Skills, skill bar, attributes and combat (research only: `docs/35`)
- [ ] Merchants and vendor windows (research only: `docs/42`)
- [ ] Player trade and the friends list (research only: `docs/43`)
- [ ] Quests
- [ ] A brand-new account with no character creating one (the seed gives every account one character)

- [ ] Starting outposts for every campaign: only Factions is measured (map 212). Every other campaign is put in Ascalon City (148) as a stand-in - `STARTER_OUTPOST_STANDIN` in `creation.rs`, logged at every creation that uses it
- [ ] The creation screen's item list: live sends 60 items in 7 bags, this server sends the equipped set

- [ ] Pick up a dropped item (`0x803F`)
- [ ] Split a stack by hand (`0x8075`)
- [ ] Buy and sell (`0x804D`)
- [ ] Swap two items in one move
- [ ] Unequip an item (`0x804F`, layout still unmeasured)
- [ ] A player standing in an instance during a rolling deploy (real client)
- [ ] A travel to the second address (real client, `docs/18-multi-machine.md`)

## Operations

- [o] The supervisor starts, stops, restarts, builds, applies the schema and seeds (`docs/19-running.md`)
- [o] Every service appears in the `service` registry, fresh while running and `stopped` after a stop
- [~] Rolling deploy: a second world runs beside the first and the old one drains when it empties - `asura-supervisor deploy-world --old 8001 --new 8002` (`crates/asura-supervisor/src/supervise.rs:525`); nothing in `crates/asura-supervisor/tests/` drives it
- [o] The stack runs as two containers (`docker/`, optional)
- [o] CI runs format and lint on every push (`.github/workflows/ci.yml`)

Recordings of a client proof live in `recordings/`, one row per file in `recordings/index.jsonl`.
