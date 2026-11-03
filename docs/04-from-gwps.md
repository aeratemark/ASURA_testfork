# What we learned from GWPS (research record)

> **GWPS is not a migration target.** ASURA is written from scratch, following AzerothCore's
> structure (`docs/adr/0006-azerothcore-is-the-reference.md`). This document records what GWPS
> taught us and where the reference code for each fact lives. The "steps" below describe the
> shape the new stack must have, not edits to the old project
> (`docs/adr/0005-gwps-reference-only.md`).

> **But when the user says "port it from GWPS", GWPS is copied verbatim.** That is a standing
> directive, not a design discussion: `AGENTS.md` #2. The user has the
> final say on what a feature must do; a port follows the source's code and bytes exactly, even
> where this document's own reasoning would choose otherwise. The `/tp` incident
> (`docs/HANDOFF.md`, "Why the command-is-the-requirement rule exists") is the worked example:
> GWPS's exact struct is what worked.

GWPS is not being thrown away. It is the working reference: every protocol fact, every client
quirk, and every "this made the client fail with error 007" was discovered there. This document
maps what to reuse as knowledge, what to port, and what to leave behind.

## What GWPS proved (keep as established facts)

- The client can be pointed at a private server: DH keys patched, mutex check disabled, launched
  with an auth server on loopback.
- Login, character list, character selection, and entering a map all work.
- Travel between maps works: request -> redirect (`0x01A6`) -> map update (`0x0099`) -> reconnect with
  a token that the server validates.
- Maps load from `Gw.dat`: the archive index, the map blob format, spawn points.
- The cost of a map server is measurable and small: ~12 MB, one thread, one database connection.
- A hop hangs when the destination server is built inside it; a hop is reliable when everything is
  prebuilt. That single experiment is the reason this repository exists.

## Component map

| GWPS file | what it becomes |
|---|---|
| `AuthSrv.c` | **auth service**: auth endpoint, handshake, login, character list, sessions, allocation decisions. Ported to Rust. |
| `AuthSrvCtrl.c`, `CtrlConn.c/.h` | **database**: retired. Placement and state coordination move into tables (`world`, `world_pool`, `session`), so the live control messages have nothing left to carry. |
| `main.c` | split: `asura` main and `world` main. Today one binary binds 6112/6113/1337 and runs everything. |
| `GameSrv.c` | **world server**, ported to Rust in `crates/asura-world`: connection, load block, movement, travel, chat, items. It is no longer C and no longer outside this project. |
| `GmMapDataImport.c`, `FaArchive.c`, `GmPaths.c`, `GmItem.c`, ... | **world server**, ported: the archive importer, decompressor and A* live in `crates/asura-world` (`map_data.rs`, `fa_compress.rs`, `pathfind.rs`, `item.rs`). |
| `Db.c`, `DbSchema.h` | **database**: ported from SQLite to PostgreSQL and split into `auth` / `game` / `content`, owned by `asura-db`. Character writes move from a connection per instance to a pooled async queue in each world server. |
| `GmCommands.c` | **world server**, plus the map list it carries is the input for outpost prebuild. |
| `tools/patch-gw.py`, `keys.txt` | **`client-patcher/`** in this repository, verbatim: the client patcher and the DH parameters the server copied. Nothing client-facing runs without them. |
| `tools/hosts-gwps.ps1` | **`client-patcher/hosts-redirect.py`**: the hosts-file companion, ported to Python so it is one command on Windows and Linux. It is the opt-in redirect, not a default: a plain patched client is live, and the block sends the six names (or all seven with `--file-server`, the content address included) to this machine. |
| `tools/`, `captures/` | test material for the port: packet traces to diff against. |

## What must not be inherited

These are the structures that cap GWPS, and carrying them over would recreate the same ceiling:

- **One thread per map instance.** Replace with a reactor and a fixed worker pool.
- **One SQLite connection per instance.** One per world server, behind a queue.
- **A private copy of the `Gw.dat` index per instance.** One per world server, memory-mapped.
- **A map server constructed on demand inside a travel.** Pools, prebuilt, assigned.
- **Sockets handed between components.** Each world accepts its own connections and does its own
  gameplay handshake.
- **Everything in one process.** The control plane is a separate program.

## Migration steps

Each step is independently testable against the real client, and each one leaves the system
working. Do not start a step before the previous one is proven.

### Step 1 - A world server that owns its connections

Give the game server its own listener and its own gameplay handshake, instead of having the auth
program accept the socket and pass it over. The client is told `127.0.0.1:<world port>` in the
redirect instead of the auth port. Nothing else changes.

- Uses: the same handshake code, now running in the world server.
- Test: log in, hop, on one machine, same client, same moves as GWPS.
- Why first: this is the one genuinely risky change, and everything else depends on it being true.

### Step 2 - Split the programs

`asura-auth` and `asura-world` become separate binaries. The world server registers itself in the
`world` table and publishes pool capacity; placement moves into the world server, arbitrated by the
database, instead of being decided inside the login process.

- Test: two processes, two terminals, same hops; kill the world mid-session and confirm the client
  fails cleanly rather than hanging.
- Acceptance: the auth service contains no `GameSrv` code and never reads `Gw.dat`.

### Step 3 - Capacity and refusal

The auth service knows how full each world is and can answer "no". The travel request path gains the refusal
branch, delivered before the client is redirected.

- Test: configure a world with one explorable slot, travel into it from two characters, confirm the
  second is refused *while still standing in the outpost*.

### Step 4 - Cheap instances inside a world server

Shared index, one database connection per world, reactor instead of a thread per instance.

- Test: 500 instances in one world server; measure memory and thread count against the numbers in
  `docs/02-architecture.md` (the targets are ~5 MB and a constant thread count).

### Step 5 - More machines

Because step 1 made the client-facing address real, this is configuration: run a world on another
host, point it at the auth service, verify a hop across hosts.

- Test: one world on the same machine and one on another; travel between them; the client should
  not notice anything except a longer loading screen.

## Order of risk

| step | risk | why |
|---|---|---|
| 2 | low | mechanical once the pieces are separate |
| 3 | low | our own code path, no client involvement |
| 4 | low | internal, measurable |
| 5 | medium | addresses, firewalls, latency - no protocol change |
| 1 | **high** | the gameplay handshake moves; failure looks like an opaque client error |

Step 1 is high risk *and* first, which is not a contradiction: it is the only one that cannot be
deferred, because it decides whether the rest of the architecture is possible. It is also the most
testable, since the client tells you immediately when it is wrong.
