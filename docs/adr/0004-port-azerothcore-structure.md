# ADR 0004 - Port AzerothCore's server structure

- **Status:** Accepted (owner's direction, 2026-10-25). AzerothCore is now the standing design
  authority, not only the source of this structure: `docs/adr/0006-azerothcore-is-the-reference.md`.
- **Date:** 2026-10-25
- **Context:** AzerothCore is a mature, multi-process Guild Wars-scale emulator that supports far
  larger populations than anything GWPS has attempted. Its server structure is proven in production
  by a large community, and it solves exactly the problems GWPS hit: concurrent database access from
  many processes, character state ownership, instance lifecycle, and scaling map simulation across
  threads and machines. Reinventing that structure would be a slow way to rediscover its decisions.

## Decision

Adopt AzerothCore's structure for ASURA. Five elements, ported as a whole rather than cherry-picked:

1. **Two program types.** An **auth service** that only handles login, and **world servers** that
   run everything else. They do not hold a live connection to each other for state.
2. **Three databases, separate concerns and separate connection pools:** `auth` (accounts, sessions,
   tokens), `game` (characters, items, instance records), `content` (map configuration, spawns -
   effectively read-only). This mirrors AzerothCore's Login / Character / World split.
3. **The database is the coordination channel.** A **registry table** lists every world server with
   its id, client-facing address, port, flags and population, exactly as `realmlist` does. A world
   server writes itself into that table at startup, keeps its row current, and marks itself offline
   on exit. The auth service reads it to tell the client where to go; nothing is negotiated live.
4. **Character state belongs to the world servers.** They read and write the `game` database
   directly, through a connection pool, with an asynchronous write queue so the simulation loop never
   blocks on a query, and transactions for anything multi-statement.
5. **Instances and map updates live inside the world server.** A map-instance manager keys instances
   by id, persists instance records, and destroys what is empty. Map simulation is to run on a bounded
   pool of update threads with map affinity - not one thread per map, which is how AzerothCore's
   `MapUpdater` works and what GWPS gets wrong. **Not built yet**: the tree starts one task per map
   (`crates/asura-world/src/instance.rs:376-385`), and the pool is `docs/32` phase 3.

## The one deviation, and why it is forced

AzerothCore creates an instance **on demand**, when a player enters it (`CreateInstanceForPlayer`).
We cannot copy that, because our client does not tolerate it: in GWPS, a travel to a map with no
server yet hung the loading screen in 7 of 53 hops, and the client recorder showed why - it tears its
world down and waits for a redirect from a server that is not answering, then never recovers.

So for explorables, ASURA prebuilds pools and **reserves** a slot:

```sql
UPDATE world_pool
   SET leased = leased + 1
 WHERE map_id = $1 AND world_id = $2 AND leased < capacity
 RETURNING world_id, address, port;
```

No row returned means no room, and the travel is refused while the client is still standing in the
outpost. World servers still create and destroy instances in AzerothCore's style - but they create
them **ahead of need**, into a pool, and never in response to a travel.

This is the only structural difference, and it is dictated by the client's behaviour, not by taste.

## Database vendor

**PostgreSQL**, recorded as a justified deviation in `docs/adr/0008-database-vendor.md`. The structure
in this ADR - pool, async write queue, transactions, registry table - is vendor-neutral and is exactly
what AzerothCore does; the vendor is a separate decision with its own reasoning.

The vendor lives behind one crate (`asura-db`); nothing else in the project writes SQL.

## Consequences

- The auth service **shrinks** to login, sessions, and reading the registry. It no longer allocates
  and no longer needs a live channel to world servers.
- **Placement moves into the world servers**, arbitrated by the database. A travel is decided by the
  world the player is on, reserving a destination slot; the central allocator disappears.
- The **world link protocol is reduced to nothing for state and placement.** What remains is a
  residue question: whether a small push channel is ever needed for events that should not wait for
  a heartbeat. Default answer: no, add it only if a measured case appears.
- The database becomes **load-bearing for correctness**, not just storage. It needs backups, a
  restore procedure and, eventually, a standby. That operational cost is accepted as the price of
  concurrent multi-process access.
- GWPS's SQLite schema is ported to PostgreSQL: three logical databases, UUIDs as native `uuid`,
  blobs as `bytea`. The write surface is small (11 functions in `Db.c`).
- Phase 2 (simulation inside ASURA) inherits this structure; the world server is written into it
  once, rather than twice.

## Alternatives rejected

- **A central allocator service with a custom control protocol** (this ADR's predecessor design).
  Rejected: it puts placement in one process, requires a live protocol to be built and kept in sync,
  and solves a coordination problem that a database transaction solves better and with less code.
- **Single writer: the auth service owns all writes, worlds ask for state.** Rejected: it makes the
  auth service a bottleneck and a single point of failure for gameplay, and it is not what a mature
  stack does.
- **SQLite, tuned for concurrency.** Rejected: no row-level locking, WAL does not work across
  machines, and lock contention surfaces as errors rather than waits. The database now carries
  correctness, so it must be a real concurrent database.
