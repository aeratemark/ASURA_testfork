# Starting ASURA's services: research for a decision

> **Status: research, and the decision it asked for is made.** The evidence below produced
> `crates/asura-supervisor`; the retraction at the end of this block records where that decision went.
> This document exists so that the decision - *how should ASURA's services be started, supervised and
> found?* - was made on evidence rather than from first principles.
>
> **Where the decision went (2026-10-28):** the level-2 supervisor it recommends was built
> (`crates/asura-supervisor`) and its decisions are recorded in `docs/16-implementation-plan.md` (S1
> and D1, S2 and D2), not as ADR 0011 - the owner judged an ADR unnecessary once every decision was
> written down there. The launcher scripts this document describes as "today's" state
> (`tools/start-server.ps1`, `play.ps1`, `stop-server.ps1`) were retired on 2026-10-28; the one run
> path is `docs/19-running.md`, on Windows and Linux.
>
> Sources: the owner's direction ("we need some way of starting microservices, so we don't have a
> single monolithic service providing us services"), the GDC 2017 talk *Guild Wars Microservices and
> 24/7 Uptime* (transcription at `PreSearing/build/gw2-text.txt`, distilled rules at
> `docs/05-gw2-lessons.md`), and the state of this repository at `ea00f60`.

## The question

ASURA already runs as separate operating-system processes, so the thing to fix is not "split one
service into many". The question is narrower and more useful:

**What should "start the stack" actually be - and what should it guarantee?**

When this was written, "start" was a set of PowerShell scripts, and everything after "the processes are
running" was
assumed rather than provided: nothing restarted a crashed service, nothing answered "is the auth
service healthy?", nothing drained a world except the fields a world itself writes, and the only
coordination a service could announce itself through was the `world` table (which speaks for world
servers only). The supervisor below was built for exactly those four gaps, and closed them.

## Plain words, so the decision can be discussed

These words recur in the talk and in the design; each one is defined here so that a conversation
never depends on knowing them already.

| word | what it means here |
|---|---|
| **service** | one program with one job (`asura-auth`, `asura-world`, `asura-webgate`), or the database. "Service" does not imply "small" - a world server is a large service. |
| **process** | one running copy of a program. Two world servers on a machine are two processes of one service type. |
| **supervisor** | a program whose only job is to start other programs, watch them, restart them when they die, and stop them cleanly. It is not part of the game; if it dies, the game keeps running until the next restart. |
| **orchestrator** | a program that decides *which machine* each process runs on, starts new builds, moves traffic, and shuts old builds down. A supervisor manages processes on one machine; an orchestrator manages machines. |
| **registry / service catalog** | a table (ours would live in the database) listing what services exist, where they are, what build they are, and whether they are alive. Consumers look things up instead of hard-coding addresses. |
| **manifest** | one file in the repository that names every service, its executable, arguments, ports and dependencies - the description a launcher or supervisor reads. Written by hand, versioned with the code. |
| **health check** | a cheap question with a yes/no answer ("are you ready?"), asked on a timer or at startup, used to decide whether to restart, advertise or drain. |
| **drain** | stop accepting new work but finish what is in hand, then exit. For a world: no new players, existing players keep playing until they leave. |
| **rolling deploy / concurrent builds** | a new build starts alongside the old one, new traffic goes to it, and the old one is drained and stopped. Players notice nothing; nothing is switched off just to switch on. |

## What the source says

The GDC 2017 talk (Stephen Clarke-Willson, ArenaNet) - slide numbers from
`docs/05-gw2-lessons.md`, which cites them so claims can be checked.

- **The definition** (slides 12-13): microservices are *asynchronous and separately restartable*;
  most of their services have process-level isolation, because restartability is much harder without
  it. This is the only part that matters when deciding what a service boundary is for: the boundary
  exists so that one thing can die and be rebuilt without the others.
- **What their estate looks like** (slide 15): about 80 types of services, but not 80 programs -
  some are libraries hosted by `ArenaSrv.exe` or `JsSrv.exe`, and `SpawnSrv.exe` hosts processes.
  Hosts listen on one port and delegate connections. So "many services" is a property of the design,
  not a requirement to ship many executables.
- **Orchestration** (slides 25-27): their definition is "allocation of services to hardware,
  permissions, routing, isolation". Configuration is a file per service: a permission section, a
  mapping from groups to IPs, changed by humans, not automatically. The minimum useful configuration
  is `127.0.0.1: all` - one line that lets the whole stack run on a laptop.
- **Registration** (slide 30): routing is *configuration plus self-registration*. Many services
  self-register with `RouteSrv`. "In an ideal world there would only be self-registration (and
  advertising over DNS)" - which the speaker explicitly says was not their reality. This is the
  honest version of service discovery for a company with a private datacenter: a registry table plus
  a config file, and humans make the changes.
- **SpawnSrv** (slides 19-21): starts the new build, routes new traffic to it, ensures it does not
  fall over, then shuts down the old build. Several builds run concurrently.
- **What "run your own" actually costs** (the datacenter slide): restarting all services at once
  (cold start), IP addresses and DHCP, automatic deployment, OS images, health monitoring, backup
  and restore. The talk frames these as complexity to challenge you - a research checklist, not
  features to copy blindly.
- **Restartability** (Part Four): authoritative services must be accurate about reality; RAM state
  is a fast cache and usually not authoritative; the pattern that works is *rebuild by asking*, not
  *reload from disk* (the GW1 tournament server keeps no state at all); where state must survive,
  two copies that do not restart together (GW2 WvW: load balancer plus maps). The retry rules and
  the newest-connection rule are already adopted in `docs/05-gw2-lessons.md`.
- **The game map server is a *macroservice*** (slide 18): nearly all the features of a microservice
  (dynamic configuration and deployment, multiple simultaneous builds) except independent
  restartability. This is the permission to keep "one world server = one big process".

## What existed when this was written, and what replaced it

The table is the state the research found: three PowerShell launchers, no restart, no shared log, no
registry row for the gate or auth. **None of those scripts is in the tree**, and the replacement is the
supervisor below them.

| piece | how it starts | health | restart if it crashes |
|---|---|---|---|
| PostgreSQL | a Windows service or a systemd unit, managed elsewhere; the supervisor checks it and waits, never starts it | a reachable connection | OS service recovery, outside this repository |
| `asura-webgate` :6601 | `asura-supervisor start`, from `config/services.toml` | a TCP connect on the port | bounded retry: 5 attempts, 2 s apart, then FAILED |
| `asura-auth` :6112 | the same | a TCP connect on the port | the same |
| `asura-world` :6113 (world `8001`) | the same; registers itself in `world`, heartbeats every 5 s, publishes `world_pool` capacity | the world's own row is fresh | the same |

Facts worth carrying into the decision:

- **The `world` table is already a miniature registry** - id, address, port, build, draining,
  population, heartbeat - and placement already refuses stale or draining worlds. The talk's
  registration model exists, but for one kind of service.
- **The `build` and `draining` columns are SpawnSrv's idea, and they have their pilot now**:
  `find_worlds` reads both, `set_draining` writes one, and `asura-supervisor deploy-world --old --new`
  performs the rolling deploy (`crates/asura-db/src/placement.rs:439-463`,
  `crates/asura-supervisor/src/supervise.rs:525`).
- **`asura-auth` is also the game-connection front door** (`crates/asura-auth/src/frontdoor.rs`):
  the real client dials the auth address regardless of what `GAME_SERVER_INFO` names, and auth
  routes the socket to the world it handed out. That is a routing responsibility sitting inside a
  service the architecture describes as "not in the data path".
- **The start is a script, and the scripts are the operational weak point**: port checks fail
  with "an earlier run is probably still up", logs go to `%TEMP%`, stopping is by process name, and
  a crashed world is simply gone until someone notices a client cannot connect.
- **A crashed service is restarted, and status is aggregated**: the supervisor retries at most
  `restart_max` times and then reports FAILED, and `asura-supervisor status` answers what is running
  and how fresh each service is (`crates/asura-supervisor/src/supervise.rs:355-411`,
  `status.rs`). Counters the architecture still asks for are not exported - that paragraph of
  `docs/02-architecture.md` remains unbuilt.

## The decision space

Four levels, ascending. Each one includes everything the previous level does. The names are
deliberately plain: they describe what the programs *do*, not products.

### Level 1 - one launcher, one manifest

A single command reads a manifest (which services, arguments, ports, dependencies, log files) and
starts them in the right order. It replaces `start-server.ps1` / `play.ps1` and makes "what runs"
visible in version control. **Buys:** one entry point, correct order, no duplicated port logic.
**Costs:** little. **Does not provide:** restart, health, drain.

### Level 2 - a supervisor

A small process (a Rust binary in this workspace is the natural choice, `asura-supervisor`) that
spawns every service from the manifest, restarts a crashed one with limited retries, starts/stops
one service without touching the others, drains and stops cleanly, and keeps one log per service.
**Buys:** the thing the current scripts are missing most - a dead world or gate comes back by
itself, and a world can be stopped without killing the login path. **Costs:** one more program to
maintain, and a rule: the supervisor must never be load-bearing for correctness (if it dies, the
services run on; it restarts itself like anything else). **Leaves open:** discovery (who finds
whom) - addresses still come from the manifest.

### Level 3 - supervisor plus a service catalog

Every service type registers in the database (a `service` table in the spirit of `world`), or keeps
its existing registry row (worlds) while the gate and auth get theirs. Consumers read the catalog;
the front door can route from it instead of hard-coded ports. **Buys:** the talk's registration
model generalized; services discover each other without config, and a service that moves (a world
on another port or machine) is a row, not a config edit. **Costs:** a new table and its lifecycle
rules (stale rows, reaping - the same machinery `world` already proves). **Leaves open:** nothing
structural; still one machine, or many - addresses are in the table.

### Level 4 - an orchestrator

Placement of service instances onto machines, concurrent builds with traffic switching, permission
isolation, health-based replacement - the SpawnSrv/RouteSrv picture at the scale of a real
datacenter. **Buys:** rolling upgrades of every service without downtime, and horizontal scale-out.
**Costs:** the whole checklist from the talk's datacenter slide (cold start, addressing, images,
monitoring, backup), plus a control plane that must itself stay up. **This is a decision about the
deployment, not about the code** - the protocol already survives a world living elsewhere
(`docs/04-from-gwps.md`, step 5).

### Sub-decisions the levels do not settle

1. **Granularity.** Keep webgate + auth + world(s) + database, or split further? Candidates that the
   design already hints at: pull the front-door routing out of auth (a tiny proxy service, or the
   client pointed at the world and the auth telling it *who it is* only); a content/file service
   (`Gw.dat` streaming, `docs/02-architecture.md`); later chat/presence if they ever exist.
   `docs/05-gw2-lessons.md` already argued for *deliberately coarser* services - this decision
   re-opens that on purpose or leaves it alone, and either way it should say so.
2. **Who starts the database.** Today the script starts the Windows service (elevation). A
   supervisor could own it, but the DB is the one process whose death interrupts everything
   (`docs/02-architecture.md`, failure table), so its supervision is an operational choice, not just
   another manifest line.
3. **Restart policy per service.** Auth: restart freely (players reconnect). World: restart and
   re-register (players on it lose their session, as today). Database: restart only deliberately.
   The talk's rule applies: rebuild by asking, never by reloading state from disk.
4. **What "healthy" means** per service, and which check a supervisor uses: the port is open (a
   process exists), the schema is current (world and auth both refuse stale schemas at startup -
   `asura_db::migrations::ensure_current`, `crates/asura-db/src/migrations.rs`), a heartbeat is recent (worlds), or an HTTP probe
   (webgate). A health check that only tests "the port is bound" is a false comfort; one that asks
   "can you answer one real request?" is what placement already does when it dials a world before
   offering it (`docs/roadmap.md`, M4 progress).

## Where ASURA already matches the talk, and where it does not

| their design | ours today | gap |
|---|---|---|
| login separate from worlds | `asura-auth` vs `asura-world` | none |
| worlds self-register; consumers read the registry | `world` + `world_pool`, heartbeat, reaper, reconciler | the model is right; only worlds have it |
| restartable worlds, disposable and interchangeable | a world rebuilds from the database; a dead one is skipped and reaped | nothing restarts a dead one automatically |
| build + drain described in the registry | `build` and `draining` columns exist | nothing performs a build or a drain |
| newest connection wins | adopted as rule 1 | none |
| retry only timeouts, 4-6 attempts, ~30 s | adopted as the retry policy | none |
| metrics cheap; rates not resets | specified in `02-architecture.md` | no exporter; no reader |
| SpawnSrv: start new build, switch traffic, verify, stop old | - | absent; this is the launcher/supervisor/level-3 work |
| RouteSrv: self-registration is the routing | partial (`world` only) | the gate and auth do not register anywhere |
| many small services, one job each | three kinds of service, deliberately coarse | a stated choice, not an accident - revisit consciously |

## Open questions the research must answer before an ADR

1. **What deployment is the target?** One machine (a private server on a home box) or several
   machines? This single answer decides whether levels 3-4 are ever needed, and it is the first
   thing to establish - everything else is cheap on one box, and orchestration is not.
2. **What must "robust" mean, concretely?** Rank the guarantees: (a) a crashed world comes back;
   (b) a crashed auth comes back; (c) an upgrade does not kick players; (d) I can see that everything
   is alive from one place; (e) services can be started individually. Each has a different owner
   (supervisor, auth design, drain protocol, status endpoint) and a different cost.
3. **Where does the front-door responsibility live?** Inside auth (today), as its own tiny service,
   or moved to the world (client pointed at the world, token validated by it)? Any supervisor /
   orchestrator design inherits this answer, because the front door is what a client dials first.
4. **Does the service catalog grow from `world`, or is `world` the template for a general table?**
   The lessons doc warns against turning the database into a network service layer; a catalog is
   less than that, but the warning should be answered, not ignored.
5. **What does the supervisor itself rely on?** The talk's rule - never a design where two
   components must restart together - means the supervisor must be startable, restartable and
   killable without any other service noticing. Its own state should be rebuilt by asking, like
   everything else.

## Suggested next research steps (not decisions)

1. Read how AzerothCore launches and supervises `authserver` and `worldserver` (its realmlist
   registration is the ancestor of our `world` table; their `.conf` files are the ancestor of a
   manifest) and write down the three things it does that our scripts do not.
2. Sketch the manifest for today's four services (one page): fields, order, ports, restart policy,
   log paths, and what a "cold start" of the whole stack means with a database that must be up
   first.
3. Translate SpawnSrv's loop to this schema: the world already advertises `build` and `draining`;
   the missing half is who starts `build` B, who switches traffic, and who stops build A. Write the
   smallest sequence that performs it.
4. Decide (with the owner) only the first open question - target deployment - and record it. The
   rest follow from it, and the ADR can then be written with a level in mind.

## What this document does not do

It does not choose a level, define the `service` table, write a supervisor, or re-open any recorded
ADR. `docs/05-gw2-lessons.md` still stands; this is the research its "Not adopting" section said
would be revisited only on purpose. If the owner's direction changes the shape of this brief, the
brief changes - the decision stays the owner's.
