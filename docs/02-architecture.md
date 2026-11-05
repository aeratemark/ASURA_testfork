# Architecture

The structure is AzerothCore's (`docs/adr/0006-azerothcore-is-the-reference.md`): it is the design
authority, and this document describes the structure we take from it. The one deviation is forced by
our client and is under experiment (`docs/adr/0005-gwps-reference-only.md`).

## Components

| component | how many | what it does | state |
|---|---|---|---|
| **socket layer** (`asura-gateway`) | 1 per auth address | accepts every client connection, answers the file check, reads the four classification bytes, and hands the socket to the auth conversation or to a world | no state; it is the program that listens on 6112 (`config/services.toml`, `[services.auth]`) |
| **auth service** (`asura-auth`) | a library inside the socket layer | login, key exchange, sessions, character list/select; tells the client which world to join | sessions in memory, accounts in `auth` |
| **rules** (`asura-rules`) | a library, called by any service | the game rules a protocol answer needs, as pure functions over data the caller read: where a character enters the game | none - no I/O, no state |
| **world server** (`asura-world`) | many | maps, players, instances, placement, character persistence | instances in memory; nothing that survives a restart |
| **database** (`asura-db`) | 1 (+ standby) | the shared truth and the coordination channel | everything |
| **content service** | 1+ | game assets (`Gw.dat` streaming) | outside ASURA |

The first three are separate because their jobs are separate (`docs/38-layer-decisions.md`, D25,
D26). A client opens two connections to the same address, so something has to classify before any
service speaks: that is the socket layer, and it is why the auth service has no listener of its own.
A rule that a protocol answer needs - "where does this character enter" - is neither protocol nor
storage, so it lives in `asura-rules` and the service supplies the data.

Correspondence with AzerothCore, so that any structural question can be answered by reading their
tree: `authservice` -> auth service; `worldserver` (a realm) -> world server; `realmlist` + realm flags ->
`world` registry; Login/Character/World databases -> `auth`/`game`/`content`;
`DatabaseWorkerPool` -> `asura-db`; `MapInstancer`/`MapUpdater` -> instance manager and update-thread
pool. Full table in `docs/adr/0006-azerothcore-is-the-reference.md`.

The auth service never runs a map. It **is** in the gameplay data path for every gameplay session,
and that is a consequence of the client, not of this design: the client dials the address the game
was launched with for its gameplay connection (measured), so the socket lands on the auth service,
which relays it to the world that hosts the instance (`crates/asura-gateway/src/relay.rs`,
`copy_bidirectional`). The reference did the same thing through its control channel, inside one
process; across a process boundary a socket cannot be handed over cleanly, so the relay is a byte
copy. World servers are interchangeable and disposable. The database is the only thing both kinds
of program agree on.

## The five structural elements

Ported from AzerothCore, deliberately not cherry-picked:

1. **Two program types**, with no live state channel between them for placement or character state.
2. **Three logical databases** - `auth` (accounts, sessions, login tokens), `game` (characters,
   items, bags, instance records) and `content` (map configuration and spawns, effectively
   read-only). Development runs all three domains in one PostgreSQL database, one URL and one pool
   (`crates/asura-db/migrations/0001_init.sql`: their union, split later); the split into three
   databases with a pool each is the target shape, not today's.
3. **A registry table** that world servers write themselves into, exactly as `realmlist` works.
4. **Character state owned and written by world servers**, through a pool with an asynchronous
   write queue.
5. **Instance management and pooled map-update threads inside the world server.**

## The registry

```sql
CREATE TABLE world (                                   -- PostgreSQL; abridged, see the migration
    id            int          PRIMARY KEY,            -- world server id, per process
    name          text         NOT NULL,
    address       text         NOT NULL,               -- what the client is told to connect to
    port          int          NOT NULL,
    build         text         NOT NULL,               -- placement prefers the newest build
    draining      boolean      NOT NULL DEFAULT false, -- finishes its players, accepts nobody new
    population    int          NOT NULL DEFAULT 0,
    heartbeat_at  timestamptz  NOT NULL                -- last time this world said it was alive
);

CREATE TABLE world_pool (                       -- capacity, per world, per map
    world_id   int  NOT NULL REFERENCES world (id) ON DELETE CASCADE,
    map_id     int  NOT NULL,
    capacity   int  NOT NULL CHECK (capacity >= 0),   -- instances prebuilt and ready
    leased     int  NOT NULL DEFAULT 0 CHECK (leased >= 0),  -- currently assigned
    PRIMARY KEY (world_id, map_id),
    CONSTRAINT world_pool_never_oversold CHECK (leased <= capacity)
);
```

A world server inserts or updates its `t_world` row at startup (`draining = false`) and keeps
`heartbeat_at` current; it does not mark itself draining or offline on exit - a world that stops is
discovered by its heartbeat going stale, and the reaper frees its leases. The auth service reads
`t_world` to answer "where is this player's map?" - the registry is data, so it survives restarts, and
no handshake is needed to keep it honest.

The general service registry sits beside `world`: a `t_service` table (migration 0010, S2) holding a
row per service kind - kind, name, address, port, build, state, `heartbeat_at`. The supervisor
writes a row for every service it starts and refreshes it every 5 s, so consumers read the catalog
instead of hard-coding addresses; worlds keep their own `t_world` rows as the load-bearing registry,
and the catalog writes theirs beside it (`crates/asura-db/src/catalog.rs`).

`t_world_pool` is what makes allocation safe across processes: it is the prebuilt capacity, and only
one process can win a given row.

### The lease ledger

`t_world_pool.leased` is a counter, and a counter cannot say *which* lease to free. Beside it sits
`t_lease`, one row per reserved slot (`crates/asura-db/migrations/0002_leases.sql`):

- it is what the reaper deletes for a world that stops heartbeating, so the counter drops by exactly
  the slots that world held;
- it is what the reconciler counts when it rebuilds `leased` from the worlds after a restart
  (`docs/05-gw2-lessons.md`, rule 2 - the table is a ledger, not the truth);
- it is unique per session, so a session's newest reservation supersedes its previous one and the
  older session's release is a no-op (`docs/05-gw2-lessons.md`, rule 1).

The counter and the ledger are written in one transaction, so `count(lease) <= leased <= capacity`
holds at every commit: the counter makes the claim, the ledger makes it identifiable, and the CHECK
constraint still guarantees nobody oversells.

## Travel, end to end

1. The player asks to travel, either from inside a map (on their current world server) or at login
   (on the auth connection).
2. The deciding process looks for a destination world that has the map and a free slot, then
   **reserves** it atomically:

   ```sql
   UPDATE world_pool SET leased = leased + 1
    WHERE world_id = $1 AND map_id = $2 AND leased < capacity
    RETURNING world_id;
   ```

   Choosing the world is a `SELECT` (same world preferred, then least loaded); the `UPDATE` is what
   makes the choice binding. Two simultaneous hops cannot both take the last slot.
3. **No row returned means no room** - the travel is refused *now*, while the client is still in its
   current map. Nothing is constructed, and the client is never told to move.
4. On success the client is redirected: the destination world's `address` and `port` from the
   registry, a map token, and a player token the destination will validate
   (`docs/03-protocol.md`, "the redirect").
5. The player leaves the old world and connects to the new one. The old world records the departure;
   the new world clears the instance it assigned and marks it leased to that session.
6. **Lease hygiene.** A lease belongs to a player session. When the player disconnects, leaves the
   instance, or the owning world stops heartbeating, a reaper returns the slot (`leased - 1`). A
   stuck lease costs capacity, never correctness, and the reaper bounds it.

## Identity and reconnection

**The newest connection wins, unconditionally.** The client drops its connection and reconnects on
every travel, and its previous connection may not have been reaped yet, so this is the normal case
rather than an edge case (`docs/05-gw2-lessons.md`, rule 1). When a player authenticates a new
connection, any older session for the same account and character is killed - never the reverse. The
older session's lease release becomes a no-op, so it cannot free a lease the newer session now holds.

Note what "killed" means, and does not: it is a live *connection* that is closed, not a database row
that is spent. GWPS only ever reads its session table, and a login must leave the row resolvable, or a
client that disconnects and reconnects would be refused (`docs/03-protocol.md`, section 4). The
registry of live connections is in memory, in the auth service, and is lost on restart - which is
acceptable because a restart already means every player re-authenticates.

**Durable identity is never physical.** Accounts, characters and parties are identified by GUID. A
server id, a port and a player token are routing hints with a lifetime; no correctness decision may be
keyed on one. This is what lets a world move to another machine without anything else noticing.

**The login credential is a pair, and both halves are checked.** The client presents a portal user id
and a session id; the `session` table holds them in separate columns (`portal_user`, `id`) because the
lookup is on both - GWPS's `Db_GetSession(user_id, session_id)` and the same shape here
(`crates/asura-db/src/login.rs`). A session that is closed never resolves again, and a session without
a portal user is not a session: `crates/asura-db/migrations/0003_portal_identity.sql` refuses to
invent one. Conflating the two ids into one column is the failure this structure exists to prevent -
it would read fine until the day a stale or forged pair was accepted.

## What is authoritative

Each piece of state is classified, and the classification decides how a restart is recovered - reload
from disk, or rebuild by asking (`docs/05-gw2-lessons.md`, rule 2):

| state | authoritative home | after a restart |
|---|---|---|
| who is in which instance | the **world server**, in memory | ASURA rebuilds from world reports |
| pool leases | the **world server**'s assignment | `world_pool.leased` is reconciled *from* the worlds, not trusted blindly |
| characters, items | the `game` database | loaded from the database; durability, not real-time truth |
| sessions, tokens | the auth service, in memory | lost; the player logs in again |
| registry rows | each world server, rewritten on startup | stale rows are reaped |

The consequence to hold onto: **no design may require two components to restart together.** That is
why leases are reconciled from the worlds instead of being treated as the database's own truth - a
table that is *always* right only stays right if nothing restarts, and things restart.

## Deployment and draining

Builds run side by side: start the new one, advertise it, send it new players, and let the old one
drain before it exits. The **newest build is the only one placement offers**: `find_worlds` takes
the build of the most recently started fresh world hosting the map (`world.started_at`) and offers
only that build - `build` and `started_at` are the deploy's comparison, which is why each build
stamps a git description (`--build`; `ASURA_BUILD`; the compiled `build.rs` value). A full newest
build refuses rather than falling back to an older one, and a draining world keeps the players it
has and accepts nobody new (placement excludes it). A player keeps their world until the client
reconnects - on their next travel - and that reconnect is placed on the newest build; nothing pushes
a transfer and nothing kicks. A draining world leaves the registry itself once its last player has
disconnected. Because the registry carries `build` and `draining`, no other component needs to know
a deployment is happening (`docs/05-gw2-lessons.md`, rule 6); the supervisor's
`deploy-world --old N --new M` starts the new world, proves it is registered, and only then drains
the old one.

## Retry policy

- **The travel path retries nothing.** A timeout is a refusal: the client is in an outpost and must
  not be sent somewhere that may not answer.
- Idempotent database work may retry on timeout only - 4 to 6 attempts, roughly 30 s apart. Any other
  error is reported and abandoned; no exponential backoff with jitter.
- Login may retry a little; the client's own reconnect loop is the outer retry.

## Observability

The intended shape, not yet implemented: each world server and the auth service should expose a small
fixed set of counters - instances, leases, players, refused travels, database queue depth, tick
latency - and a **rate** form for anything cumulative, because a counter that resets on restart hides
the rate it is supposed to show (`docs/05-gw2-lessons.md`, rule 5). Nothing exports a counter today
and nothing reads one. Counters are cheap and always on; aggregation is a separate task, not
something a simulation thread does.

## Instances

| map type | lifecycle | count |
|---|---|---|
| outpost / town | an instance built at start-up, or on first request under `--on-demand`; keyed by map id and never destroyed | bounded by content |
| explorable | the same, one instance per map id today; a pool per party is the phase-2 target (`docs/32`) | configured |

- **The instance is created on first use, and its clock starts with it** (`crates/asura-world/src/
  instance.rs:376-385`). A world started without `--on-demand` builds an instance for every map in
  `map_config` at start-up, which is the reference's pre-spawned shape; with `--on-demand` an
  instance is built the first time a client names its map (`crates/asura-world/src/travel.rs:83`),
  measured at 0-2 ms per map against the real client. AzerothCore's on-demand creation is therefore
  available here as an opt-in, and the reference's prebuild is the default.
- The pool-per-party lifecycle below is the target for explorables: today a map id keys one instance
  (`Instances`, `instance.rs:368`), and independent copies of one map are `docs/32` phase 2.
- Instance records persist in `game`, so a restart rebuilds the same pools and "who is where"
  survives an auth-service restart.

## Inside a world server

- **One IO reactor** accepts and serves gameplay connections - the ones the front door relays from
  the client (the client dials the auth address, and the auth service copies bytes to this world).
- **An update task per map.** The first join to a map starts its clock on its own task
  (`Instance::run`, `tokio::spawn` at `instance.rs:382`), and the task ends with the instance. The
  bounded pool of update threads with map affinity that AzerothCore uses is **not built**: nothing
  in `crates/asura-world` pins a map to a worker. It is the shape `docs/32` phase 3 and
  `docs/adr/0006` name for the simulation stage, where per-instance state stops being lock-free.
- **Database access through a pool per database**, with writes enqueued to an asynchronous worker so
  the simulation loop never waits on the disk. Multi-statement changes are transactions. Queue depth
  is a metric, because a growing queue is the first sign of trouble.
- **Character state** is loaded on entering an instance, saved on leaving it and on a timer, never
  per frame. Position and inventory need to survive a crash within a minute, not every tick.

## Failure behaviour

| event | consequence |
|---|---|
| world server crashes | heartbeat stops; the auth service stops sending players to it; leases held by its players expire and are reaped; players log in again; the database is not corrupted, because saves are transactional |
| world server restarts | re-registers, rebuilds its pools from the database, becomes a candidate again once ready |
| auth service restarts | every gameplay connection it is relaying drops with it: players in maps are disconnected and must log in again. New logins wait, and in-memory sessions are lost, so a travelling player must re-authenticate |
| database unavailable | fail closed: no logins, no travels. Players already in a map stay there, and the world server keeps what it can in memory until it must stop |
| stale registry row | the reaper marks the world offline and frees its leases; the auth service skips it |

The asymmetry is deliberate: losing the auth service interrupts *entry*, losing a world interrupts
*one map*, losing the database interrupts *everything* - which is why the database gets the
operational attention (backups, a restore test, later a standby).

## Startup order

1. Database reachable, schema current.
2. The supervisor starts the services in dependency order: with no `depends_on` declared, that is
   alphabetical, so the auth service binds first, then the web gate, then the world
   (`crates/asura-supervisor/src/config.rs:192-200`, `config/services.toml`). Nothing in the order is a
   correctness requirement today: the auth service reads the registry per request, so a world that
   registers later is simply not offered until it has.
3. A world connects, builds its maps (every map in `map_config`, or none under `--on-demand`), and
   registers itself in `world`; it is offered as soon as its row is fresh.
4. Players log in; the auth service answers with a world that is registered, online and has capacity.
5. Shutdown: worlds flush character state, mark themselves draining, exit; the auth service stops
   accepting, then exits.

A world that is still prebuilding is not advertised, so a request arriving during startup gets a
correct "no room" instead of a race.

"Schema current" is a query, not an assumption: each migration records its own version in
`schema_version`, and `asura_db::migrations::ensure_current` refuses to run against a database that is
missing one, naming what is missing (`crates/asura-db/src/migrations.rs`). A service that starts
against a stale schema fails later, on some unrelated query, saying nothing about the cause.

## Cost model

Measured from GWPS, per map instance, against what this structure changes:

| item | GWPS today | ASURA target |
|---|---|---|
| private memory per instance | 11.9 MB | ~5 MB (shared `Gw.dat` index, database pool per world not per instance) |
| OS threads | 1 per instance | bounded pool, constant with instance count |
| database connections | 1 per instance | a small pool per world server |
| `Gw.dat` index | private copy per instance (4.7 MB) | one memory-mapped copy per world server |
| SQLite page cache | 2 MB per instance | gone - pooled PostgreSQL connections |

Sizing: a world server holding 500 instances is roughly 2.5 GB. More worlds, more machines, same
code - the client is told a real address, so nothing in the protocol changes when a world moves.

## Deliberate non-goals

- Orchestration and autoscaling. Adding a world is a config entry and a process.
- Distributed consensus. The database is the arbiter; there is no second source of truth.
- Multi-region. One region; districts are capacity, not geography.
