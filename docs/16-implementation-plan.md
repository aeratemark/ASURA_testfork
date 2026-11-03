# Implementation plan: the seven improvements from the analysis

> **Status: delivered.** S1, S2, S3, S5a, S6 and S7 are delivered; S5b and S4 are closed. Every item
> the analysis listed has its stamp below. This plan contains **only** what the owner-approved
> analysis lists under "What we can improve" - seven suggestions, no eighth. Every item below quotes
> the suggestion it implements. If something is not in that list, it is not in this plan.

## The rules this plan is written under (from `docs/HANDOFF.md`, binding)

- **Do exactly the command.** No additions "while we're here".
- **The pre-flight per change:** one sentence - what changes, in which message or code path, and why
  it is not simply copying the reference; the source of truth named with `file:line` or bytes; if it
  contains anything not asked for, ask instead.
- **One change, one subject.**
- **The real client is the test; the suite is not a gate.** Test what the change touches, nothing
  else. No growing scaffolding around a small change.
- **Report, do not decide.**
- **Additive:** no wire changes; schema changes are new numbered migrations, never edits; every
  existing script keeps working until a phase proves its replacement, with the owner's word.

## The mapping: suggestion -> item -> evidence -> proof

| analysis suggestion ("What we can improve") | item | evidence it is real today | done-when |
|---|---|---|---|
| **1.** Replace the scripts with a launcher and a babysitter | S1 *(delivered 2026-10-28)* | `play.ps1` runs a `while` loop that throws when either child exits; args hard-coded; logs in `%TEMP%` | one command starts and stops the stack; a crashed local service comes back; real client login/travel unchanged |
| **2.** Make the database's service registry general | S2 *(delivered 2026-10-28)* | `world`/`world_pool` exist and are load-bearing; `asura-webgate` and `asura-auth` register nowhere | every service kind has a registry row; a consumer reads it; live-database proof |
| **3.** Give the `build`/`draining` columns a pilot | S3 *(delivered 2026-10-28)* | both columns exist (`crates/asura-db/src/placement.rs`); `register_world` writes `build` (`asura-world/src/main.rs:286`); `find_worlds` never consults it (`placement.rs:435`); nothing reads `draining`; a world cannot be stopped without a kill | a new build takes new players, the old world drains without kicking anyone, proven with the real client |
| **4.** Add a status that tells the truth | S4 *(closed 2026-10-28: not built - what the operating layer owns is already reported, and nothing else is in its scope)* | `docs/02-architecture.md` asks for counters and rates; nothing exports per-service counters, and the operating layer is not where they belong | a status view of the operating layer: what is running, is it healthy, heartbeat age, capacity, uptime |
| **5.** Untie the receptionist knot; fix the stale documents | S5a *(delivered 2026-10-28)* + S5b *(closed 2026-10-28)* | `crates/asura-auth/src/frontdoor.rs` is the router inside auth; the docs claims in S5a are false today | the five corrections land (S5a); a recorded decision on the front door (S5b) |
| **6.** Prove multi-machine capability on one machine, no hardware | S6 *(delivered 2026-10-28)* | the front door dials the **registered** address (`frontdoor.rs:82-96`), so cross-host routing already exists on paper; nothing has ever run at a second address | a travel to a world registered at a second loopback address, recorded |
| **7.** One "how do I run this" file, and CI's place | S7 *(delivered 2026-10-28)* | README's run instructions are stale (see S5a); `tools/test-battery.ps1` is the local battery; the repo's testing policy says the suite is not a gate | one accurate run document; a CI decision recorded (or none, with the reason) |

## S1 - Launcher and babysitter (the supervisor)

**The suggestion.** "Replace the scripts with a launcher and a babysitter."

**What existed before this item, as it read on 2026-10-28.** `tools/start-server.ps1` (elevation for
PostgreSQL, started the gate, handed over), `tools/play.ps1` (seeded, built, started auth and world with
`Start-Process`, tailed two `%TEMP%` logs, threw the moment either child exited),
`tools/stop-server.ps1` (killed by process name). All three were retired by this item; none is in the
tree. There was
no manifest, no state file, no per-service log directory, no way to start or stop one service
alone, and no restart.

**What the item delivers (nothing beyond the suggestion):**

- one command that starts the local stack in the right order, and one that stops it;
- supervision of the local services: a crashed one is restarted (the "babysitter");
- one log file per service, in one place;
- the list of services read from the config folder (`config/`), so what runs on this machine is
  data, not code;
- the old scripts keep working throughout; retiring them is a separate step with the owner's word.

**Delivered 2026-10-28.** `crates/asura-supervisor` (a Rust binary: `start`, `stop`, `status`,
`restart`, `build`, `schema`, `seed`), the list in `config/services.toml`, one log per service under
`logs/`, a loopback control channel, bounded restarts (5 attempts, 2 s apart, then FAILED - D1.3),
the port rule, and the process-group rule so Ctrl+C stops services *through* the supervisor.
Verified by 12 tests (10 unit + 2 integration) and a live run: start -> all three ready -> status ->
stop -> every port free. The cross-platform `schema`/`seed` commands live in the same crate.

**Settled by the owner** (recorded in their words, 2026-10-28):

- **It is a supervisor** (a service manager). It starts this machine's services and watches them -
  not an orchestrator. Across machines the database remains the only coordinator (S6), so no
  orchestrator is needed.
- **Configs live in one folder.** A dedicated config folder in the repository (`config/`) holds this
  machine's service list - the supervisor's "recipe" - and any future non-secret configuration, so
  nothing is scattered. Secrets (the database URL) stay in `.env`, gitignored, as today.
- **Windows and Linux.** The manager is a Rust binary so one codebase runs on both; the current
  scripts are Windows-only. Verifying it on Linux needs a Linux environment (WSL, a container, or
  CI); until then Linux is built but untested, and the run document says so.
- **When the manager exits, it stops what it started, and nothing else.** The database is out of
  its scope: checked and waited for, never started (a Windows service on Windows, systemd on
  Linux - "managed elsewhere").
- **A restart stops the old process first**: if it does not exit, kill it; wait until its port is
  free; then start the new one. No new process binds while the old one might still hold the port.

**Open decisions - D1 (the rest):**

- the restart policy when a local service crashes (retry a few times, then report it dead - the
  proposal - or retry forever).

**Must not change.** The services' command-line flags, their wire behaviour, the log lines the
owner reads.

**Proof (the testing policy):** the new command starts and stops the stack; killing one local
service makes it come back; the real client still logs in and travels. Targeted tests only for the
supervisor's own new behaviour.

**Risk.** Process control on Windows: the settle rule above (stop the old one, wait for its port,
then start the new one) is exactly what the current scripts lack.

## S2 - A general service registry

**The suggestion.** "Make the database's service registry general - every service announces itself,
not just worlds."

**What exists.** `world` (id, name, address, port, build, draining, population, heartbeat) and
`world_pool` (per-map capacity/leases) are proven and load-bearing: placement and the front door
read them. `asura-webgate` and `asura-auth` register nowhere; their addresses live in scripts.

**What the item delivers:**

- a registry every service kind can write itself into, in the same database, on the same pattern as
  `world`;
- a reader (placement, the front door, or status) that consumes it;
- worlds keep their existing rows and the proven `world_pool` machinery (they are not moved).

**Delivered 2026-10-28.** Migration 0010 adds `service` (kind, name, address, port, build, state,
started_at, heartbeat_at; unique per kind+name); `asura-db::catalog` holds the upsert, list, state
and delete queries; the supervisor publishes a row for every service it starts - kind from the
config or the name prefix, address from `--bind` - every 5 s, and a final publish on exit leaves
every row at `stopped`; `asura-supervisor status` prints the registry table with STALE marking.
Verified against the live database: the catalog test (register/refresh/staleness/stop/delete) and a
live run (rows for auth, webgate and world-8001, fresh; stopped after `stop`).

**Decisions - D2 (settled by the owner's go-ahead):** a separate catalog beside `world` (the proven
world machinery stays untouched), and the supervisor writes a row for each service it starts.

**Must not change.** `world`/`world_pool` semantics and the atomic reservation; the one rule
(never offer a client something that is not answering).

**Proof:** live-database test - a row is written, read back, and a stale row is reported as stale;
the consumer uses it. No client involvement.

## S3 - A rolling deploy (build id + drain)

**The suggestion.** "Give the upgrade columns an operator - a rolling deploy: start the new build
beside the old one; nobody gets kicked; a player keeps their build until the client reconnects (on
their next travel, or one we push - a transfer is a travel); and a draining world ends when its last
player has disconnected."

**What exists.** `world.build` is written as `asura-world/{version}`
(`crates/asura-world/src/main.rs:286`); `world.draining` is written by nothing and read by
placement (`placement.rs:432`) and by nothing else; `find_worlds` orders by `leased, id`
(`placement.rs:435`) and never looks at `build`; a world cannot be stopped without a kill, and
killing it leaves its row for the reaper.

**What the item delivers:**

- a way to run a new build beside the old one on one machine (no hardware);
- a placement rule that sends new players to the new build;
- a drain that lets the old world finish its players and leave - proven with the real client on the
  old world: nobody is kicked;
- the decision and the evidence recorded (the columns are no longer vocabulary without an operator).

**Decisions - D3 (settled by the owner, 2026-10-28):**

- where the new build runs: **a second world process beside the old one**; nobody gets kicked;
- a player's migration: **they keep playing until they need to reconnect by any means; on that
  reconnect they are moved to the new build** - no pushed transfers, no client restart (a transfer
  is a travel: the client tears down, reconnects, lands on the new build with a loading screen);
- placement among builds: **the newest build serves new arrivals and reconnects; the old build
  serves only the players already inside it, and a full new world refuses rather than falling back
  to the old build** (the one rule already refuses instead of hanging);
- how a draining world ends: **when its last player has disconnected** - no forced kick, no timeout
  that kills players.

**Delivered 2026-10-28.** `crates/asura-world/build.rs` stamps each build with
`git describe --always --dirty` (overridable with `--build`/`ASURA_BUILD`); migration 0011 adds
`world.started_at`, set on every registration, so "newest build" means "the build of the most
recently started fresh world hosting the map"; `find_worlds` offers only that build (a full newest
build refuses; no fallback to an older one - proven by `the_newest_build_is_the_only_one_offered`
against the live database); the world's heartbeat now publishes its real population and leaves the
registry and exits once a draining world's last player has disconnected; the supervisor gained
`deploy-world --old N --new M`, which starts the new world, proves it is registered, and only then
marks the old one draining. Verified live on one machine: two worlds on ports 6113/6114 both
registered with the same git build id; `deploy-world 8001 8002` reported ok; world 8001 drained and
its row disappeared without a kick, world 8002 stayed fresh. The real-client half (a player standing
in the old world while the deploy runs, and a reconnect landing on the new build) is the owner's
test with the client.

**Must not change.** The one rule: a client is never sent to something that is not answering. The
wire. The reaper stays the safety net for ungraceful deaths.

**Proof (the real client, per the testing policy):** a client in the old world is not interrupted by
the deploy; a new client's travel lands on the new build; a targeted database test proves the
placement rule.

## S4 - System health: keepalives and the status view (closed, not built)

> **Closed by the owner, 2026-10-28: not to be built.** The operating layer's liveness is already
> reported where it belongs - `asura-supervisor status` prints each local service's state, pid and
> restart count, and the registry table marks every row fresh or STALE with its age. What S4 would
> add beyond that (heartbeat age as a number on fresh rows, `world_pool` capacity/leases, the
> supervisor's uptime) is display polish, not a capability the operating layer lacks. Per the owner's
> scope ruling, the operating layer does system keepalives only - no player data, no metric table -
> and the services that own player numbers keep them in their own terms. The section below stays as
> the record of the scoping; nothing in it is being implemented.

**The suggestion.** "Add a 'status' that tells the truth about the operating layer - what is running,
is it healthy, and is it alive - and shows the *rate* of anything cumulative rather than a total
that resets. Player-domain data is not this item's business: the services that own it keep it in
their own terms."

**What exists.** `asura-supervisor status` prints the local process table (state, pid, restart
count) and the service registry with a STALE marker; the world server's registry row carries its
build, draining flag and population, and `world_pool` carries the per-map capacity and leases. What
it does not show: how old each heartbeat is, the capacity/leases of a world, and how long the
supervisor itself has been up. Nothing exports per-service counters, and nothing should from here.

**What the item delivers:**

- one command that answers, about the system: what is running, is it healthy, how long since its
  last heartbeat, what capacity it publishes and how much is leased, and how long the supervisor
  has been up;
- the world's own population and build are shown **because the world reported them** (its registry
  row) - a read of what the service said, never the operating layer measuring players;
- the *rate* form for anything cumulative that lives in this layer (restart counts; nothing else
  is a counter here - the supervisor starts and stops processes, it does not count gameplay);
- the one write it must not do: no metric table, no counters from the operating layer. The
  operational numbers that already exist live in the registry (world row, `world_pool`); the
  player-domain ones (logins, travels, moves, ticks) stay inside the services that own them.

**Scoped by the owner, 2026-10-28:** "everything regarding the player needs to go in the world
service, not here - I understand keepalives and metrics for the systems, but not player data." So
this item is the operating layer's health view only. The auth service and the world server keep
their own numbers in their own terms (their log lines today; whatever they do with them is their
item, not this one).

**Decisions - D4 (collapsed by the scope):** no new table, no counters in the operating layer. The
status view reads the registry and `world_pool`; the world's population is displayed as the world's
own report.

**Must not change.** The simulation loop's timing; the wire; the existing logs.

**Proof:** start the stack -> status shows each service with its state and heartbeat age; stop one ->
it shows the stop; a world that leaves the registry disappears from the view; the supervisor's
uptime is shown. A targeted check, live. No player-domain number is produced by the operating
layer.

## S5a - Fix the stale internal documentation

**The suggestion** (the second half of improvement 5). "...fix the stale documents to match reality."

**What exists.** Five claims in the repository's own documents are false today. Each is a
`file:line` fact, not a preference, so each correction is a factual change to internal
documentation:

| claim | where | truth |
|---|---|---|
| "the worlds stay in C and keep simulating" / "the world servers remain the existing C implementation" | `README.md` (~line 57, #Language) | `crates/asura-world` is the live world in Rust; M4 was delivered with the real client |
| "placement prefers the newest healthy build" | `docs/02-architecture.md` (deployment section) | `find_worlds` orders by `leased, id`; `build` is not consulted (`placement.rs:435`) |
| "each world server and the auth service exposes a small fixed set of counters ... and a rate form" | `docs/02-architecture.md` (observability section) | nothing exports anything |
| "a world server ... marks itself draining or offline on exit" (flags = online) | `docs/02-architecture.md` (the registry section) | no exit path, no flags column; the row is left for the reaper |
| "6 migrations applied / 11 tables" | `docs/HANDOFF.md` (environment status) | nine migration files (`0001`-`0009`) existed when this was written; the tree now carries 24 (`0001`-`0024`) |

**What the item delivers:** the five corrections, one subject per change, each verified against the
tree before the edit. This is internal documentation only.

**No open decisions.** Every correction is a fact; if the owner disputes one, that correction is not
made, and the claim stays until the owner decides.

**Must not change.** Code, behaviour, the wire - this item touches documents only.

**Proof:** each corrected line points at the `file:line` that proves the correction; `git diff`
shows documents only.

**Delivered 2026-10-28.** The five claims are corrected in `README.md`, `docs/02-architecture.md`
and `docs/HANDOFF.md`, each citing its evidence; the change touches documents only.

## S5b - The front-door decision (closed by the owner, 2026-10-28)

**The suggestion** (the first half of improvement 5). "Untie the receptionist knot - decide whether
routing belongs inside login, as its own tiny program, or inside the world."

**What exists.** `crates/asura-auth/src/frontdoor.rs` routes the game connection from the auth
service to the world, dialing the world's **registered** address. A measured fact decides the
options: the client dials the address it was launched with for its game connection, regardless of
what `GAME_SERVER_INFO` names - so "the world opens its own door and the client dials it" is not
available without changing the client.

**Closed by the owner (2026-10-28):** the relay is the **auth server's job**. "If you want to do
microservices you can, but they are part of the auth server" - no gateway process, no second
component: the only program at the address the client dials is the auth server, so it relays, as it
already does. A gateway would be the auth server under another name, with one more process and a
client-facing re-proof. What the item did leave behind is the correction of the claims that said
otherwise: `docs/02-architecture.md` (three places) and `docs/03-protocol.md` #11 now state the
measured fact and its consequence (the auth service is in the gameplay data path; a restart of it
drops the connections it relays).

**Must not change.** The wire; the client path.

**Proof:** the decision is recorded; the false claims are corrected; no code changed.

## S6 - Multi-machine capability, proven without hardware

**The suggestion.** "Prove the two-machine hop - on one machine, no hardware, since that's the
deployment."

**What exists.** The front door already dials the registered world address
(`frontdoor.rs:82-96`), and `asura-world --public` exists - so a world at another address is
"configuration" by design, and unproven in fact. Nothing has ever run at a second address.

**What the item delivers:**

- a second world, genuinely separate, registered at a **second loopback address** (all of
  `127.0.0.0/8` is loopback on Windows and Linux), so the registry, the front door and the client
  treat it as a world on another machine;
- a recorded experiment: a travel to it lands; the drain crosses it; the limits are written down
  (same OS, same NIC, no latency/NAT/firewall - a real LAN adds those, operationally, not in the
  design).

**Open decisions - D6:**

- the exact scenario (a second world at `127.0.0.2`; anything more is the owner's call).

**Must not change.** Nothing: this item changes no code unless the proof finds a defect (and then
the defect is the change, restored to the reference's behaviour where applicable).

**Proof:** the recorded run, with the client, plus the test that the front door used the registered
address and not the bind address.

## S7 - One "how do I run this" file, and CI's place

**The suggestion.** "A real 'how do I run this' in one file, and CI so the test battery runs without
a human remembering."

**What exists.** The README's run section is stale and scattered; `tools/test-battery.ps1` is the
local "everything" command. The repo's testing policy (`.HANDOFF.md`, 2026-10-28) says the real
client is the test and the suite is not a gate - which is directly relevant to what "CI" should be
here.

**What the item delivers:**

- one document that says, in order, what to run, what each command needs, and what success looks
  like (it replaces the stale run instructions and is verified against the tree);
- a recorded CI decision.

**Open decisions - D7:**

- CI: none (the local battery remains the tool), lint/format only, the suite on a runner with a
  database, or the suite nightly. The testing policy's "the suite is not a gate" argues against
  gating feature work; the owner decides what "CI" means before the item starts.

**Must not change.** The testing policy. The battery's usefulness.

**Proof:** the run document is followed end to end by someone who has not run the stack; CI, if
chosen, does exactly what the decision says and no more.

**Delivered 2026-10-28. D7.1 settled by the owner: format + lint only.** The one run document is
`docs/19-running.md` - what you need once, the one command (`asura-supervisor start`), what success
looks like, the everyday commands, the rolling deploy, the client, CI, and a troubleshooting table;
it is verified against the tree (ports from `config/services.toml`, the command list from
`crates/asura-supervisor/src/main.rs`). `README.md` and `docs/HANDOFF.md` now point at it instead of
repeating instructions. CI is `.github/workflows/ci.yml`: `cargo fmt --all -- --check` and
`cargo clippy --workspace --all-targets -- -D warnings` on every push and pull request - no
database, no test suite, per the testing policy. The gate needed the tree to be clean, so the five
clippy warnings it surfaced (two dangling doc comments left by the parity tooling's removal, and two
lints in `tests/path_dump.rs`) were cleared first; the Linux runner doubles as the first Linux
signal for the supervisor, which has not had a live Linux run yet.

## Ordering (dependency order, not preference)

1. **S5a - the stale-document corrections** - factual, each backed by `file:line`; no code, no
   decisions. *(delivered 2026-10-28)*
2. **S1** - the launcher/babysitter everything else builds on. *(delivered 2026-10-28)*
3. **S2** - the service registry; a consumer for S4. *(delivered 2026-10-28)*
4. **S3** - the rolling deploy (needs S1 to run two worlds; decisions settled). *(delivered 2026-10-28)*
5. **S4** - the operating layer's health view (needs S2's registry to read). *(closed 2026-10-28: not built - liveness is already reported by the supervisor and the registry)*
6. **S6** - the multi-machine proof (needs S1/S3 to run and drain a second world). *(delivered 2026-10-28: docs/18-multi-machine.md)*
7. **S5b - the front-door decision** - closed: the relay is the auth server's job, and the claims that said otherwise are corrected.
8. **S7** - the run document anytime; CI last, with the testing policy in view. *(delivered 2026-10-28: docs/19-running.md + .github/workflows/ci.yml)*

## Decisions needed (answer each before its item starts)

| id | item | decision | options | proposal (rejectable) |
|---|---|---|---|---|
| D1.1 | S1 | what the launcher is | Rust binary in this workspace / one improved script / third-party manager | **settled**: the supervisor - a Rust program, Windows and Linux |
| D1.2 | S1 | how it learns what to run | a config file in the repo / `.env` / command-line flags | **settled**: a dedicated config folder (`config/`) with this machine's service list inside |
| D1.3 | S1 | restart policy for a crashed local service | restart with a fixed retry cap, then report / restart always / no automatic restart | restart with a fixed cap; no exponential backoff |
| D2.1 | S2 | the registry shape | one general table / a separate catalog beside `world` (worlds unchanged) / worlds only, others stay in config | separate catalog beside `world` - additive; `world` stays load-bearing |
| D2.2 | S2 | who writes a row | the service itself / the launcher on its behalf | the launcher for the services it starts; worlds keep writing their own |
| D3.1 | S3 | where the new build runs | a second world process beside the old / restart in place | a second world process - additive, no kick |
| D3.2 | S3 | placement among builds | newest build first, older if newest is full / newest only / no build rule yet | newest first - the architecture already says so |
| D3.3 | S3 | how a draining world ends | the world notices `draining` and exits when empty / the launcher stops it after a wait / the registry row goes stale | the world exits itself when empty - same mechanism on one machine and across |
| D4.1 | S4 | where the numbers live | a table written by the services / an endpoint on each service / the launcher reads what exists | **closed**: not built - the launcher reads what exists (the registry), and nothing new is written |
| D5.1 | S5b | the front door | **closed**: the relay stays in the auth server; no gateway, no second component | - |
| D6.1 | S6 | the proof scenario | a second world at `127.0.0.2`, client travel to it / more | the second world at `127.0.0.2` |
| D7.1 | S7 | CI's place | none / lint+format only / the suite on a runner with a database / nightly | **settled**: format + lint only - `cargo fmt --check` and clippy; no database, no test suite |

## What this plan does not contain

- Any gameplay work (NPCs, combat, quests, items) - not in the analysis's improvement list.
- Any hardware requirement - every item is provable on the one machine everything already runs on.
- Any schema, format, or policy that was not asked for: the decisions table is the only place those
  appear, and each is the owner's.
- Any date or effort estimate: the owner's time is the cost, and the analysis did not price it.
