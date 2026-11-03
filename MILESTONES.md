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

## Not built

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
