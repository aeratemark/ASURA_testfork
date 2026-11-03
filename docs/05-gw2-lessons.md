# What the GW2 microservices talk changes in this design

Source: *Guild Wars Microservices and 24/7 Uptime*, Stephen Clarke-Willson (ArenaNet), GDC 2017.
References below are to slide numbers, so any claim can be checked against the deck.

This is a comparison, not a summary. For each point: what they do, whether ASURA already does it, and
what we should change. The second half of the talk - restartability - is where we were weakest.

## Already covered

| their design | ASURA | note |
|---|---|---|
| Login separate from world simulation (slides 12-13: client -> Patching, Login, Map servers) | auth service vs world servers | same split, fewer services because we have fewer concerns |
| Many small services with one job each (slide 11, ~80 types on slide 15) | auth service, world servers, database | we are deliberately coarser; we have no commerce, tournament, social or web estate |
| Self-registration plus configuration (slide 30: map servers self-register; many services register with RouteSrv) | `world` registry table written by each world | ours is the "registration" half; the "configuration" half is the `world_pool` capacity rows |
| Orchestration is assignment of services to hardware, permissions, routing, isolation (slide 25) | placement = registry + pools | we have no permissions layer; see "Not adopting" |
| The game itself is a *macroservice*, not a microservice (slide 18) | world server | same conclusion: maps are big, stateful and not independently restartable |
| Maps are time-sliced over cores inside a blade (slide 20) | one update task per map, no pool yet | the same idea is the intended shape but **not built**: `crates/asura-world/src/instance.rs:376-385` starts one task per map, and the bounded pool with map affinity is `docs/32` phase 3 |
| Stable identity instead of physical addressing (slide 22: they moved from IP+port+ProgramId+build+map+PlayerId to "ProgramId implicit + player or group GUID") | **partly** - we have tokens, but see "Identity" below | they went further than we have |

## Adopted: the rules we were missing

### 1. Trust only the most recent connection (slides 49-50)

Their failure story: A is connected to B; A dies without a clean close; B waits for a TCP timeout; A
reconnects *before* the timeout expires; B now believes there are two connections and only the newer
one works. Their rule: **only ever honour the most recent connection from a machine or service**, even
if the older one looks alive and has work in flight.

This is exactly our situation, and we have already been bitten by its cousin. Our client drops its
connection and reconnects on **every travel**, so "a new connection arrives while the old one has not
been reaped yet" is the normal case, not the exception. GWPS handled it with an ad-hoc registry check
and a special case that cleared "a transfer that never completed" - three bugs came from that area
(error 007 on a rejected reconnect, the replayed transfer, the disconnected-during-transfer case).

**Rule for ASURA:** when a player authenticates a new connection, the new connection wins
unconditionally. Kill or ignore any older session for the same account/character; never reject the
new one because an old one still looks alive. Lease release follows the same rule: the newest session
owns the lease, and the older one's release is a no-op.

### 2. Restartable means "rebuild by asking", not "reload from disk" (slides 39-46)

Their argument, which is the most useful thing in the talk for us:

- Local state in RAM is a highly optimised cache; it is usually **not authoritative**.
- Persisting semi-transient state to disk and reloading it after a restart is usually wrong, because
  time passes while you are down and the state has moved on.
- The pattern that works: keep authoritative state in RAM and, on restart, **re-query the parties that
  know** - Heroes' Ascent's tournament server keeps *no state at all* between restarts and rebuilds by
  asking each map who is in it.
- Where state must survive, keep **two copies that do not restart together** (WvW: load balancer + maps,
  either may restart as long as not both; plus a rarely used emergency copy).

Applied to us, this is a correction to how we wrote `02-architecture.md`:

| state | we said | what it should be |
|---|---|---|
| sessions | in ASURA's memory, lost on restart | keep in memory, rebuild by asking worlds; a lost session costs a re-login, which is acceptable *because we say so* |
| pool leases | `world_pool.leased` is the truth | the **world server** is the truth about what it has handed out; `world_pool` is the reservation ledger that makes concurrent hops safe. On restart, ASURA reconciles `leased` from world reports instead of trusting the table |
| player position and inventory | saved in `game` | still saved (it must survive a crash), but understood as *durable*, not *authoritative in real time*; a restart loses the last tick, never the character |
| world registry rows | written by worlds, read by auth | unchanged, and it is the model the talk endorses |

The extra rule that falls out: **never design a state such that two components must restart
simultaneously to stay correct.** Leases are the place where this could go wrong, so the reconciler
above is not an optimisation - it is what makes a restart safe.

### 3. Identity should not be physical (slide 22)

They moved routing from `IP + port + ProgramId + build + map + PlayerId` to `ProgramId implicit +
player or group GUID`, precisely because a transient identifier (a per-map player id) breaks when the
player moves. We currently carry `server_id + player_token`, both transient.

**Rule for ASURA:** the durable identity is the **account/character GUID and the party GUID**. Server
ids, tokens and ports are *routing hints* with a lifetime, never something another component keys on
for correctness.

### 4. Retry policy: retry timeouts, fail everything else fast (slides 47-48)

A restart means 10-20 seconds offline, which is why retries matter more than failover for short
outages. Their rules:

- retry **only** on timeout; any other error is reported and the transaction abandoned;
- 4-6 automatic attempts, roughly 30 seconds apart;
- application knowledge earns exceptions (their patcher retries forever);
- exponential backoff with jitter at the application layer is described as wasted effort that adds
  unpredictability.

**Rule for ASURA:** the travel path retries **nothing** - a timeout produces a refusal, because the
client is standing in an outpost and must not be sent to a server that may not be there. Idempotent
database work may retry per the rules above. Login may retry a little; the client's own reconnect
loop is the outer retry.

### 5. Metrics are cheap, and counters must be transformed (slides 31-37)

They collect about a million sequenced values per minute across both games; `AtomicAdd`-style counters
are cheap; a separate service aggregates; Graphite for display. The sharpest point is slide 36-37: an
accumulative counter that resets on restart *hides the rate*. They publish the derivative instead.

**Rule for ASURA:** every world server and the auth service exposes a small fixed set of counters
(instances, leases, players, rejected travels, DB queue depth, tick latency) and a **rate** form for
anything cumulative. Restarts must not make a graph lie.

### 6. Deploy in parallel and drain (slides 19, 20, 21)

`SpawnSrv` starts a new build, routes new traffic to it, verifies it does not fall over, then shuts
the old one down. Several builds run concurrently, and the map macroservice runs several versions at
once.

**Rule for ASURA:** the registry carries a **build id** and a **draining** flag. Placement sends new
players to the newest healthy build; a draining world keeps its players and accepts nobody new.
Nothing else in the system needs to know a build happened.

## Not adopting, and why

- **~80 services.** Their service count is a consequence of a decade of product surface (commerce,
  tournaments, social, web). ASURA has one game and one small territory. Splitting into 80 services
  would buy isolation we do not need at a cost of coordination we would have to pay daily. The
  lessons above are about *rules*, not counts.
- **Their own hardware, no virtualisation** (slides 26-27). They pin processes to IPs and let the OS
  scheduler do the work, on hardware they own and have kept stable for years. That is a legitimate
  choice with a large capital commitment; the talk itself says it is not a general recommendation
  against virtualisation (our transcription's conclusion 9). Our equivalent of "stable hardware" is
  modest: a box, a database, and processes.
- **DNS-based service announcement** (slide 30: "in an ideal world, self-registration and DNS would be
  enough - that is not our reality yet"). We do the boring thing they actually do: a registry that is
  written by the servers and read by their consumers.
- **A separate DB service layer** (their `DbSrv` / `DbSqlSrv`). They front databases with services, so
  that caching, retries and observability have a home. We do this inside `asura-db` rather than as a
  network hop. If retry policy or metric volume ever justifies it, it becomes a service.

## What this does not answer

The talk has nothing to say about our client's peculiar demand - that a travel must land on an
already-running instance - because their player never reconnects per map change the way ours does.
That remains our deviation, and it stays documented in `docs/adr/0004-port-azerothcore-structure.md`.
