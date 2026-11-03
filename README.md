# A.S.U.R.A. - Automated Server Utility for Remote Authentication

A private Guild Wars 1 server. **Language: Rust. Database: PostgreSQL.**

## The shape of the server

The client talks to the **auth service** (login, session, character list, and the address the client
dials for its game connection), gets its portal identity from the **web gate**, and is placed into one
of many **instance servers** that host the maps. Everything durable - accounts, sessions, characters,
items, the list of running servers - lives in **PostgreSQL**. Each machine runs a **supervisor** that
starts and watches its local services.

```
client --portal login--> web gate  :6601  (who am I?)
client --login--------> auth       :6112  (key exchange, session, character)
client --gameplay----> auth       :6112 --relays--> instance server :6113+  (one of many)
                                                          |
           sessions | characters | items | server list ---+--> PostgreSQL (the shared truth)
```

Every player is always in an instance. An **outpost** is a shared hub instance; an **explorable** is a
per-party instance; travel is an instance change, which the client performs by reconnecting. The client
dials the address it was launched with for both of its connections, so the auth service is the only
program at that address and it relays the gameplay connection to the instance server it picked.

## The components

| component | what it is | what it does |
|---|---|---|
| **auth service** (`asura-auth`) | one program | the client's login: key exchange, session, character list, character select; looks up an instance server for a travel or refuses it cleanly; relays the game connection to the server it picked |
| **instance servers** (`asura-world`) | many programs | host maps for players: outposts, explorables, agents, movement, items, chat. Each creates its state at startup and can be replaced by a newer build without touching the players inside it |
| **web gate** (`asura-webgate`) | one program | the portal login the client asks before it logs in: four `/Spawned/WebGate` endpoints, answered from the database |
| **supervisor** (`asura-supervisor`) | one program per machine | starts and watches this machine's services from `config/services.toml`: start, stop, status, restart, rolling deploy |
| **database** (`asura-db`) | PostgreSQL | the shared truth: accounts, sessions, characters, bags and items, plus the list of servers and the player slots they hold |
| **libraries** (`asura-protocol`, `asura-db`) | Rust code the services share | the client-facing message definitions, and the database access layer every service uses |

Every service reads and writes the database; that is how the services coordinate.

## How it grows

- **More instances, anywhere.** An instance server keeps nothing of its own: characters, their
  positions and items are in the database, and map geometry comes bundled with the build. So an outpost
  or explorable can be hosted by any running server, and replacing a server is just starting another
  process - on this machine, on a second machine, or a newer build during an upgrade.
- **The database decides where a travel goes.** Every instance server tells the database it is alive
  every few seconds. When a player travels, the auth service picks a server that reported recently and
  sends them there; if none did, the travel is refused and the player stays where they are. A server
  that stops reporting stops being picked, and the player slots it had claimed are freed automatically.
- **Adding a machine** means starting a supervisor on it with its own list in `config/services.toml`.
  Its outposts and explorables show up in the same database as every other machine's.
- **An upgrade does not interrupt anyone.** The new build starts beside the old one, new players go to
  the new one, and the old one shuts down when its last player logs out.
- **The limits:** the auth service is the only address the client talks to, and gameplay goes through
  it, so restarting it drops the connections it is carrying; the database is a single piece everything
  depends on; and the two-machine case was tested as two addresses on one computer, not across a real
  network.

## Status

The whole path works against a real client: the patched client (build 39112) logs in through the web
gate and `asura-auth`, enters an outpost on an instance server, walks, travels between maps, and uses
its inventory - items, drops, moves, equipment - against the live server. The supervisor starts and
stops the stack, the database tracks every running service, an upgrade beside the old build was proven
on one machine, and the auth service was proven to dial a server registered at a second address rather
than its bind address. The open verification is the same two things done with the client in hand: a
player standing in an instance during an upgrade, and a client travel to the second address.

## Operating systems

**The server runs on Windows and Linux.** The supervisor, the auth service, the instance servers and
the web gate are the same Rust programs on both; the database is PostgreSQL on either; `config/services.toml`
is the only thing that differs between machines. The same supervisor commands start the stack on both
operating systems - `docs/17-running-on-linux.md` is the Linux runbook (what runs where, the ports,
the database, and the supervisor's cross-platform setup commands).

**The game client is Windows-only** - the client rig, its patching and its hooks are Windows work, so a
Linux machine is a server host.

**The client patcher is in `client-patcher/`** - `patch-gw.py` plus `keys.txt`, the DH parameters that
match the server's handshake. The stock `Gw.exe` cannot talk to this server; `Gw.custom.exe` is built
with that folder, and its README has the commands. A plain build leaves every ArenaNet hostname live -
only the auth and portal addresses come from the client's command line - and redirecting anything else
is an operator's opt-in pair: the machine's hosts block and a file server. Without both, a repointed
build cannot connect; `client-patcher/README.md`, "Read this first", is the checklist.

**A Docker bundle lives in `docker/`** (`docker compose -f docker/compose.yml up -d --build`): two
containers, PostgreSQL and one machine container running the supervisor, with the client reaching the
same `127.0.0.1:6112` and `6601`. It is the optional second runner; `docs/19-running.md` describes it.

**Linux has not had a live run recorded yet.** It compiles and the runbook is written, but the
cross-platform claim is by construction, not by measurement; the Linux runbook's evidence section is
empty on purpose.

## Read in this order

| document | what it settles |
|---|---|
| [`AGENTS.md`](AGENTS.md) | **the binding rules** - the objective, the answering gate, the test rule, the encoding rule. The only place a rule is stated |
| [`docs/HANDOFF.md`](docs/HANDOFF.md) | **the evidence behind those rules** - the incident ledger, what each rule cost, environment status, and where the next session starts |
| [`MILESTONES.md`](MILESTONES.md) | **the feature list** - one line per feature, and whether a real client proved it |
| [`docs/19-running.md`](docs/19-running.md) | **how to run it** - what you need once, the one command, what success looks like, troubleshooting |
| [`docs/17-running-on-linux.md`](docs/17-running-on-linux.md) | **the Linux runbook** - the same supervisor commands on a Linux server host |
| [`client-patcher/README.md`](client-patcher/README.md) | **the client patcher** - how `Gw.custom.exe` is built, and the keys that match the server |
| [`docs/02-architecture.md`](docs/02-architecture.md) | the components and travel, in more detail |
| [`docs/03-protocol.md`](docs/03-protocol.md) | the protocol facts: framing, key exchange, login, travel, the game connection |
| [`docs/01-scope.md`](docs/01-scope.md) | the boundary: what each service owns and what it explicitly does not |
| [`docs/roadmap.md`](docs/roadmap.md) | milestones, in the order they can be tested |
