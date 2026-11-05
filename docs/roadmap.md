# Roadmap

Milestones in the order they can be tested. Each one ends with something a real client, a test
harness, or a measurement can prove - not with "the code compiles".

## M0 - Specification

This repository's documents, and the decisions they depend on.

**Done when:** `docs/01-scope.md`, `docs/02-architecture.md`, `docs/03-protocol.md`,
`docs/04-from-gwps.md`, the ADRs, and this roadmap are complete enough to answer two questions
without asking the author: *what does the auth service do?* and *what does a world server decide?*

**Test:** a reader who has never seen GWPS can describe the allocation flow and the one rule.

## M1 - Cargo workspace, no networking

The skeleton follows AzerothCore's shape (auth service, world server, database domains, registry),
not GWPS's single-process layout.

The workspace is now seven members (`Cargo.toml`): `asura-auth`, `asura-world`, `asura-webgate`,
`asura-protocol`, `asura-db`, `asura-fakeworld`, `asura-supervisor`. M1's skeleton was smaller and this
list has grown with each milestone; there is no `asura` binary.

**Done when:** every world-link message round-trips byte-for-byte in a unit test, the protocol
version mismatch path is tested, and a service binary starts and exits cleanly against `--help` or the
real entry point.

**Test:** `cargo test`. Nothing depends on a client.

## M2 - Schema, registry, and reservations against a fake world

Schema first, then coordination through it: world servers register in `world`, publish pool capacity
in `world_pool`, and travel decisions reserve slots atomically.

**Done when:** a test harness (a "fake world" binary in the workspace) can register, heartbeat, be
marked stale when it goes silent, and have its leases reaped. Reservation is proven atomic under
concurrency: with a pool of capacity N-1, N parallel reservations succeed exactly N-1 times.

**Test:** harness-only, fully automated, no game client involved. The concurrency case runs the
reservation from many tasks at once.

**Delivered.** `asura-fakeworld` is the harness; `crates/asura-db/src/placement.rs` holds the queries;
`crates/asura-db/tests/placement.rs` proves the race, the refusal, the reaper and the reconciler
against the live database; `tools/m2-verify.ps1` runs both in one command. A `lease` ledger sits
beside `world_pool` so a lease can be identified, reaped and reconciled (`docs/02-architecture.md`).
The suite runs repeatedly without flakiness - it is the first thing to re-run after any change to
placement.

## M3 - The auth endpoint (the hard one)

Port the handshake and login: framing, ARC4, DH with the fixed constants, login against the web
gate, character list, character select.

**Done when:** the patched `Gw.custom.exe` reaches the character list against the auth service, and the
handshake produces traffic that diffs clean against the GWPS reference for the same actions.

**Test:** real client, login only - no worlds needed yet. Diff with
`PreSearing/tools/diff-transitions.py` and the `GWPS_TRACE` logs as the reference side.

**Risk:** the highest in the project. Failure is an opaque client error (`007`), so keep the packet
trace and compare every message. This is why the reference implementation stays runnable.

**Delivered 2026-10-26.** The patched client (build 39112) logged in against this service, took the
EULA and reached character select. `docs/06-reference-session.md` records the whole session from both
servers - every direction, header and size - and `asura-auth` reproduced it line for line up to the
client's travel request, which at that point was refused because no world server existed (it is answered`r
now, `docs/22-world-model.md`): `REQUEST_GAME_INSTANCE`
(`0x8029`) needs `GAME_SERVER_INFO` (`0x0009`, 42 bytes: address, port, tokens) and then a second
connection speaking the game protocol, which is M4.

The pieces behind that: connection classification from the first four bytes; framing by walking each
message's field layout (u16 counts, no padding - the wire is variable-length in **both** directions,
which the table's sizes do not describe; see `docs/03-protocol.md` #1); the full auth message table
generated out of the GWPS reference; the key exchange (ARC4, the fixed-constant Diffie-Hellman, the
protocol's own key derivation, pinned by fixtures **compiled from the reference's own C**,
`docs/adr/0009-crypto-primitives.md`); the file check; typed layouts for every message on the login
path, including the 591-byte settings blob and the 62-byte character settings block copied from a live
reply; the login decision against the database; and the listener that puts them together with a
per-message trace behind `ASURA_TRACE=1`. `crates/asura-auth/tests/login.rs` proves it over a socket
without a game, and `tools/m3-verify.ps1` runs the whole path in one command. `tools/inspect-reply.py`
is still in the tree; the comparison tools it was used with (`probe`, `tools/compare-replies.py`) are
retired with the parity tooling (`docs/16`) - they are what settled the shapes when reading the
reference was not enough.

## M4 - End-to-end travel

The auth service allocates, a world grants an instance, the client is redirected to that world's own address
and reconnects with the token.

**Depends on:** GWPS step 1 - a world server that accepts and authenticates its own gameplay
connections (`docs/04-from-gwps.md`, step 1).

**Done when:** one C world server, one the auth service, one client: log in -> enter a map -> travel to another
map, and the client's world rebuilds (the in-client recorder should show a tear-down followed by a
rebuild, as it does on GWPS: 100-204 ms).

**Test:** real client, plus the client-state recorder to prove the client actually settled.

**Progress.** The auth half is in (`crates/asura-auth/src/travel.rs`): `0x8029` is decoded,
`asura_db::placement::find_worlds` looks for a world that publishes the requested map and is fresh,
not draining and not full, the candidate's port is **dialled before it is offered** (a world can be
fresh in the database and dead on the socket, and the one rule is that a client is never sent to a
port nothing answers on), the slot is taken with the atomic `reserve`, and the answer is
`GAME_SERVER_INFO` (42 bytes, host blob in the reference's own asymmetry) plus a status-0 response. A
refusal is status 2 with no info message, and it leaves the counter untouched. `tests/travel.rs`
proves all of that over a socket, including the refusal for a registered-but-silent world.

**Progress (world half).** `asura-world` listens, and the whole path exists
(`crates/asura-world/src/lib.rs`):

* it classifies the connection, reads `GAME_CMSG_VERSION` (64 bytes: world id, map, player token, both
  UUIDs), and **claims** the pending transfer - a token nobody issued, a token already claimed, a token
  for another world, or the wrong character closes the socket with no message, which is what the
  reference does;
* it completes the key exchange, sends the reference's four-message load block unasked, answers the
  item/spawn/players requests (the last ending with `INSTANCE_LOAD_FINISH`), and keeps the world tick
  and ping running;
* the spawn point comes from `map_config` (migration 0005) and holds the coordinate the reference rig
  verified against a live client, not `(0,0)`;
* leaving releases the pool slot immediately rather than waiting for the reaper;
* the game message table is the one packet table (`crates/asura-protocol/src/packets.rs`, 682 rows), so
  client messages are framed by the reference's own field layouts rather than by a hand-written list.
**DELIVERED 2026-10-26 with the real client (build 39112): log in, choose the character, travel, and
stand in Lakeside County with its armour on, with the world server keeping the clock and answering
pings for as long as the client stays.**

The auth service is also the game connection's front door: a real client dials the auth address
whatever `GAME_SERVER_INFO` names (measured - it ignored the port in that message), so the auth routes
the socket to the world it named (`crates/asura-gateway/src/relay.rs`).

`crates/asura-world/tests/load.rs` proves the load over a socket - a placed client is loaded into its
map, a token nobody issued is closed without a word, a transfer naming a different character is
refused - and `tests/end_to_end.rs` drives login -> travel -> the routed game connection ->
`INSTANCE_LOAD_FINISH` in one process. `tools/m4-verify.ps1` runs all of it, and
`asura-supervisor start` starts the stack for the real client (`docs/19-running.md`).

**How the last bugs were found, because the method is the durable part.** Three tools did it, and all
three are retired with the parity tooling (`docs/16`), so this is the record and not a recipe:
`tools/patch-gwps-full-trace.py`
made the reference dump whole messages (its own trace cut at 24 bytes, which hid every field that
mattered); `tools/compare-load.py` diffed our stream against its trace by size, bytes **and order**;
`ASURA_WORLD_REPLAY` + `ASURA_WORLD_REPLAY_PICK` serve the reference's captured bytes per block, which
is what separated "our payloads differ" from "our connection is wrong". The three bugs that survived
every earlier guess were, in order:

1. **a missing second `PLAYER_UPDATE_PROFESSION`** after `PLAYER_CREATE` and before the agent exists -
   an *order* bug, invisible to a diff that matched messages by header;
2. **an all-zero appearance word** (`PROFESSION_None`, hair style 0) - a model the client cannot build;
3. **no inventory at all** - bags and nothing else, and a client with no items has nothing to draw the
   character from. The reference's own default inventory was generated from its trace by
   `tools/gen-default-inventory.py` (retired with the parity tooling, `docs/16`) - now as **rows**
   rather than as replayed bytes, and the block rebuilt from them is compared to that capture byte for
   byte (`docs/08-next-inventory-model.md`).

## M5 - Capacity, refusal, and persistence

The auth service sizes the world, refuses when full, and owns player state.

**Done when:** with one explorable slot configured, a second character travelling there is refused
*while still standing in the outpost* - the client never enters a loading screen. Player state is
written on leaving an instance and read back on entering.

**Test:** two clients, or one client and a harness that holds the slot.

**Progress (persistence half).** A character's *position* is saved when it leaves and read back when it
enters (`asura_db::game::record_position`/`position`, `Standing::resume`), and it is used only in the map
it was taken in; a character that has never been played spawns where `map_config` says. The inventory is
rows too (`crates/asura-db/migrations/0006_inventory.sql`), so an item survives a session - which is what
"player state" meant and did not exist before. What is still *not* saved is everything else about a
character: level, profession, gold, equipment changes.

**Progress (capacity half).** Placement already publishes per-map capacity, reserves atomically and
refuses a full map (`docs/HANDOFF.md`, M2), and `docs/02-architecture.md` records the one rule: a client
is never sent to a port nothing answers on. What is missing is the *explorable* instance this milestone's
"refused while still standing in the outpost" needs - the world hosts outposts only - so a refusal at
travel time is proven (`asura-auth/tests/travel.rs`) but never with a client in a loading screen.

## M6 - More than one machine

A world on a second host, reachable because step 1 made the client-facing address real.

**Done when:** a hop takes a character from a world on host A to a world on host B, and back, with
nothing but a longer loading screen to show for it.

**Test:** real client, two hosts.

## M6.5 - The instance-creation experiment (answered 2026-10-27)

The one question we cannot answer from GWPS without more work there, so it is answered here
(`docs/adr/0005-gwps-reference-only.md`):

- Default: a travel is answered only with an already-running instance (prebuilt pool).
- Test: create the instance during the travel, redirect only after the world reports the instance
  ready, and watch the real client with the recorder. Rebuilt world = on-demand creation is
  acceptable; torn down and never rebuilt = the pool requirement is real, and we have a reproduction
  in our own code instead of an inference from a confounded one.

**Done when:** the answer is recorded with the client state that proves it, either way.

**Answered 2026-10-27: on-demand creation works with the client; the prebuilt pool is not a client
requirement.** The world ran with nothing preloaded (the retired `start-server.ps1 -OnDemand`) and the
real client took a loop of teleports - 148 -> 163 -> 164 -> 165 -> 166 -> 148 - with every destination
loaded on demand in 0-2 ms. The hang that forced the reference to pre-spawn one server per map
(`AuthSrv_PreSpawnServers`) was OpenTyria building the instance inside the hop, not a property of the
client. The runbook and numbers are `docs/03-protocol.md` #5c; the server half is pinned by
`a_travel_to_an_unhosted_map_is_loaded_on_demand` and
`a_connection_to_an_unhosted_map_is_loaded_on_demand` (`crates/asura-world/tests/load.rs`). The
prebuilt pool stays the normal path; on-demand creation is the proven fallback.

## Phase 2 - simulation

The detailed implementation handoff is `docs/32-world-simulation-plan.md`. Its first assignment
is configurable map spawns and portal arrivals with a small content manager, followed by isolated
instances, NPCs, combat, HeroAI, a Lua quest, and capacity measurement. Read its owner intent,
scope boundaries, acceptance checks, and unresolved decision gates before implementing a phase.
The manager technology and JSON authoring workflow have not been selected by the owner.

The working login/travel/movement path is the starting point; preserve the client-confirmed
baseline. The S1-S4 rows below describe architectural outcomes. Use the detailed handoff for the
current implementation order; its seven phases are distinct from the infrastructure milestones
and the packet-parity stages. None is marked implemented merely by recording this plan.

| milestone | outcome |
|---|---|
| S1 | Rust provides simulation mechanics and execution; external Lua scripts hold the game's decisions and rules across the simulation (Rhai was selected first and replaced, 2026-11-01). Existing HeroAI decisions are behavior to express in Lua, using Rust world operations and execution machinery. Event-driven scripting is proposed; implementation and open decisions are recorded in `docs/30-world-content-and-scripting.md` |
| S2 | the world server builds and runs from this repository, with its own maps and player load/save |
| S3 | the simulation reads what it needs from the database through `asura-db`, with no shortcuts and no private SQL |
| S4 | instance teardown: an empty explorable is cleared and returned to its pool, and memory does not grow across a session |

## After M6

- Districts as capacity (a town that is full gains a second instance) - same machinery as
  explorable pools, different policy.
- Chat across worlds, if it is ever wanted.
- Observability worth keeping: per-world instance/player/RSS/tick metrics, exported and logged.

## The tests that must exist before each milestone is called done

| milestone | automated | real client | measurement |
|---|---|---|---|
| M1 | unit round-trips, version mismatch | - | - |
| M2 | harness register/heartbeat/stale/refuse | - | - |
| M3 | every packet row recomputes to its recorded size (`crates/asura-protocol/src/packets.rs` tests), key exchange matches fixtures compiled from the reference's C, framing round-trips, identity-pair refusals, and a client logging in over a socket (`asura-auth/tests/login.rs`) | login to character list | packet diff vs GWPS |
| M4 | - | hop, and client state settles | tear-down/rebuilt, memory |
| M5 | the inventory rebuilt from rows equals the reference's capture byte for byte; seeding, drop scoping and the position round trip against the live database (`asura-db/tests/inventory.rs`); a client dropping an item and returning where it left over a socket (`asura-world/tests/load.rs`) | refusal while in outpost | saved state round-trip |
| M6 | - | cross-host hop | latency |
