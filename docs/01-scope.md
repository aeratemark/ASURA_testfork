# Scope

Which component owns what, in an ASURA stack built on AzerothCore's structure
(`docs/adr/0004-port-azerothcore-structure.md`).

## The components

| component | responsiblity |
|---|---|
| **web gate** (`asura-webgate`) | the portal the client asks before any login: four `/Spawned/WebGate` endpoints and `/health` |
| **socket layer** (`asura-gateway`) | the program that listens on the client's address: accepts, answers the file check, classifies the connection, and either hands it to the auth conversation or relays it to a world |
| **auth service** (`asura-auth`) | the login conversation, as a library the socket layer calls: key exchange, session, character list and select, EULA, settings |
| **world server** | everything a player experiences: maps, instances, placement, character state |
| **database** | the shared truth and the coordination channel between the others |
| **supervisor** (`asura-supervisor`) | starts, stops, restarts, builds and reports the services above, from `config/services.toml` |

Two of those are libraries rather than processes: `asura-auth` and `asura-rules`, the crate that holds
a game rule a protocol answer needs (`docs/38-layer-decisions.md`, D25, D26).

These are the process kinds this machine runs, and `config/services.toml` is the list. The auth
service is small and long-lived; world servers are interchangeable and disposable; the database is what
they all agree on. There is no live control connection between the auth service and world servers for
state or placement - the database is the handoff.

## The auth service owns

### The login endpoint

The client's first connection. Key exchange, login against the web gate, character list, character
selection. The client uses this connection only for authentication; the gameplay connection is a
different one, to a world server.

### Sessions and login tokens

For every logged-in player: account id, session id, selected character, and the token the player's
*next* connection must present. The client drops its connection and reconnects on every travel, and
that reconnect is only accepted when it matches what was issued. Getting this wrong is a visible
client error, not a crash.

### The web gate

The portal the client talks to before it talks to us, which hands it a `user_id` and a `session_id`.
It is its own program, `crates/asura-webgate`, with its own port and log; the auth service checks the
pair it issued rather than owning the endpoints (`crates/asura-webgate/src/lib.rs:1-31`).

### Reading the registry - not owning it

The auth service reads the `world` table to answer "where should this client connect?" It does not
decide placement, and it does not keep the directory's truth in memory: the table is the truth, and
a restart loses nothing.

## The world servers own

### Placement and capacity

A travel is decided by the world server the player is on, not by a central allocator. It picks a
destination world with the map loaded and room available, and **reserves** a slot atomically in
`world_pool` (`docs/02-architecture.md`, "Travel, end to end"). No slot means the travel is refused
while the client is still standing in its current map.

### Character state

Characters, bags, items and instance records in the `game` database - read when entering an instance,
written when leaving and on a timer, never per frame. The world server is the owner; the auth service
never writes character data.

### Instances

Built from `map_config`: every map in the catalogue at start-up by default, as the reference pre-spawns
one server per configured map; `--maps` narrows the set on purpose. A map that was not preloaded is
built on demand the first time a client names it, 0-2 ms per map (`MILESTONES.md`, `docs/roadmap.md`).
What a world refuses is a map it cannot serve or one with no free seat.

### The gameplay connection

The client dials the auth address for its gameplay connection too, so the socket lands on the auth
service, which copies it to the world that hosts the instance (`crates/asura-gateway/src/relay.rs`,
`copy_bidirectional`). The auth service runs no map; it relays bytes and never looks inside them
(`docs/02-architecture.md`, "Components").

## The database owns

The schema, and therefore the contract between the programs. Three logical domains: `auth`, `game`,
`content` - one PostgreSQL database in development (`docs/02-architecture.md`, "The five structural
elements"). One owner of the schema definition (`asura-db`), migrations in version control, and no SQL
written anywhere else: the two seed files that are SQL (`tools/map-config.sql`, `tools/seed-dev.sql`)
are applied by `asura-supervisor seed`, which is `asura-db`'s own path.

## Rules that must not be quietly broken

1. **Identity validation stays out of the world servers' *discretion*.** A world may verify a token
   the auth service issued; it may not admit a player who never logged in, and it may not mint
   tokens.
2. **Capacity must be able to say no.** A refusal happens before the client is told to move. Any
   design where "no room" is discovered after the client has been redirected is a re-introduction of
   the original hang.
3. **The database is the only coordination point.** No side channel that carries state between the
   auth service and a world server; the moment one exists, restarts become ambiguous.

## What does not exist *yet*

Simulation - agents, movement, pathfinding, combat, skills, drops - is in this project's long-term
scope (`docs/adr/0003-simulation-in-scope.md`). The base structure it was deferred behind is built:
the auth service, the world server, the database, placement, travel and movement all run and are
recorded in `MILESTONES.md`; the simulation roadmap is `docs/32-world-simulation-plan.md`.

- **Simulation content.** No NPC, combat, quest or loot exists: an instance holds the clients playing
  in it and nothing else (`docs/07-status.md`).
- **Map data.** No component of ASURA stores map blobs; `Gw.dat` reading is a world-server concern.
- **Content delivery.** The client also talks to a file server for game assets: a separate service,
  not scheduled.

## Explicitly out of scope for now

- **Simulation's own design.** Deferred, not cancelled; no phase-1 decision may preclude it.
- Multiple regions or languages. One region, one language; districts are a capacity concept only.
- Cross-world chat. Chat is delivered by the world the player is on.
- Game balance and content.
