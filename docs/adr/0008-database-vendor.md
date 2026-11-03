# ADR 0008 - Database vendor: PostgreSQL, a justified deviation

- **Status:** Accepted (owner's direction: keep the mechanisms, choose the better design)
- **Date:** 2026-10-25
- **Relationship to the rule:** `docs/adr/0006-azerothcore-is-the-reference.md` makes AzerothCore the
  design authority and permits a deviation **if it is recorded before it is implemented**. This ADR is
  that record. It changes no mechanism: the registry, the atomic reservation, the pools, the pooled
  connections with an async write queue and the transaction discipline are all exactly what
  AzerothCore does.

## Decision

**PostgreSQL** (15+). The structure stays AzerothCore's; the vendor is ours, for the reasons below.

## Why the vendor is separable from the structure

AzerothCore's persistence *structure* is: separate database domains with separate pools; a synchronous
pool plus an asynchronous write queue so the game loop never blocks; transactions for multi-statement
changes; prepared statements; a registry table that servers write themselves into. Every one of those
is vendor-neutral and is adopted in full (`docs/adr/0004-port-azerothcore-structure.md`).

What remains is which engine. That is a deployment and ergonomics decision, and it is worth deciding on
merits rather than by inheritance, because nothing downstream depends on it: no query in ASURA is
copied from AzerothCore, and we do not import their world or character databases.

## The comparison, on this workload

Our profile: one small server to start, fourteen tables at this reading (`crates/asura-db/migrations`,
files `0001`-`0024`), low write volume (sessions at login, character save
on exit and on a timer, a lease on each travel), reads dominated by registry lookups and character
loads, a rapid schema iteration phase ahead of us, and an administrator who is also the developer.

| concern | PostgreSQL 15 | MySQL 8.0.16+ | who wins for us |
|---|---|---|---|
| atomic slot reservation | `UPDATE ... RETURNING` returns the winner in one statement | same atomicity via affected-rows | tie |
| row locking under contention | row locks, no gap locking | row locks **and gap locks** | PostgreSQL - fewer surprise deadlocks once there are several processes updating pools |
| migrations while the schema churns | DDL is transactional: a failed migration leaves nothing half-applied | atomic per statement, not per migration | **PostgreSQL - this is the one that will actually bite us at M2-M5** |
| partial index (`session ... WHERE ended_at IS NULL`) | supported | not supported; composite index instead | PostgreSQL |
| types we actually use | native `uuid`, `timestamptz`, `inet`, `real`, `jsonb` | `CHAR(36)`/`BINARY(16)`, `DATETIME(6)` with UTC discipline, `VARCHAR(45)` | PostgreSQL - fewer conventions to remember, so fewer bugs |
| `CHECK` constraints | always | 8.0.16+ | tie at our floor |
| Rust driver maturity (`sqlx`) | the reference driver of the crate | good, but the less-travelled path | PostgreSQL |
| reading AzerothCore's SQL as reference | translates trivially (both are standard SQL for our six tables) | identical dialect | MySQL, marginally - and irrelevant, since we copy no queries |
| operational familiarity with their backup/tooling docs | `pg_dump`/`pg_restore` instead | same tools as theirs | MySQL |

The tally is not close once you weight by what will actually happen to this project in the next few
months: we are about to write and rewrite migrations repeatedly, and we will run several processes
that contend on a handful of hot rows. Those are exactly the two places where PostgreSQL is better,
and they are concrete rather than aesthetic. The MySQL column's advantage is familiarity with code we
have decided not to use.

## What this costs, stated honestly

- One translation layer of knowledge: when reading AzerothCore's SQL or its `DatabaseWorkerPool`
  behaviour, the types and DDL idioms differ. Our six tables are standard SQL, so the drift is small,
  and `asura-db` is the only place it can appear.
- The operational reference for backups is PostgreSQL's, not theirs.
- If a future maintainer knows MySQL and not PostgreSQL, that is a real cost. It is documented here so
  it is a known cost rather than a surprise.

## What would reverse this

A measured problem, not a preference: for example, tooling we cannot run on PostgreSQL, a failover
requirement their stack meets more easily, or a workload where their dialect becomes materially
cheaper. It would be a new ADR with numbers. The vendor lives behind `asura-db`, so the change is
contained to that crate's queries and one migration file.

## Process note

This decision has been wrong twice, and both times in instructive ways. First it was PostgreSQL
as a *drift*: chosen before the "follow AzerothCore" rule existed and never revisited. Then it was
MySQL as *rule-worship*: the rule governs structure, and applying it to the vendor threw away a real
engineering advantage to satisfy a principle that did not cover it. The rule now says what it should
have said from the start: **structure follows the authority; a non-structural choice is decided on
merits and recorded.**
