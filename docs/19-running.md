# Running ASURA

One page, in order: what you need once, how to start the stack, what success looks like, and what to do when it does not. Everything here is checked against the tree (`config/services.toml`, `crates/asura-supervisor`, the ports below); if a command here disagrees with the code, the code is right and this page is a bug.

## What you need, once

- **A checkout of this repository**, on Windows (where the game client runs) or Linux (a server host, `docs/17-running-on-linux.md`).
- **Rust with cargo** - the whole stack is built from it.
- **PostgreSQL, running.** The supervisor checks for it and waits, but never starts it: on Windows it is a service, on Linux systemd. "Managed elsewhere" is the boundary (`config/services.toml`).
- **A `.env` file**: copy `.env.example` to `.env` and fill in the password. `ASURA_DATABASE_URL` is the one mandatory value; the auth bind and the trace switch are optional.
- Optional: a real `Gw.dat`, only for regenerating the path-graph dump (`asura-world --dump-path-graphs FILE`, which writes the maps of `map_config`; the flag takes the output file and does nothing without it). The worlds serve the bundled `data/path-graphs.bin`, so nothing needs it to run.

## First time: the schema

```powershell
cargo run -p asura-supervisor -- schema
```

Applies the numbered migrations, in order, through `psql` (the same command on Windows and Linux). A fresh database needs this once; re-running it is harmless.

## Start the stack: one command

```powershell
cargo run -p asura-supervisor -- start
```

What that one command does, in order:

1. reads `config/services.toml` (what this machine runs, with arguments and log paths);
2. proves the database is reachable;
3. builds the services (fast when up to date);
4. seeds the dev identities and the map catalogue (the rows a fresh database needs);
5. starts the services in their dependency order - the order a service's `depends_on` names, and
   alphabetical among services that declare none, which today is all three: auth `127.0.0.1:6112`,
   web gate `127.0.0.1:6601`, world `8001` on `6113` (`crates/asura-supervisor/src/config.rs:192-200`,
   `config/services.toml`);
6. registers every service it started in the database's `service` table, and keeps the rows fresh;
7. watches them until Ctrl+C or `stop` - a crashed service is retried at most 5 times, 2 s apart, then reported FAILED (it never restarts forever).

Ctrl+C stops the services *through* the supervisor (they run in their own process group, so they never see the console's Ctrl+C themselves). Closing the console still ends everything attached to it; a hard kill of the supervisor (no Ctrl+C, no `stop`) orphans the services - there is no code that can run on SIGKILL.

## What success looks like

```powershell
cargo run -p asura-supervisor -- status
```

- the local table shows `webgate`, `auth` and `world-8001` as `ready`;
- the registry table shows a row for each, marked `fresh` (a row whose heartbeat is older than 60 s prints `STALE` with its age);
- the three ports above accept connections.

With the game client, success is: the client asks the web gate on `6601` who it is, sends its login to auth on `6112`, reaches character select, and entering the world sends it to `world-8001` on `6113`. `tools/run-client-login.ps1` automates that attempt and prints a verdict (LOGIN ACCEPTED / REFUSED / NO LOGIN ARRIVED).

The logs are one file per service: `logs/webgate.log`, `logs/auth.log`, `logs/world-8001.log`. They are quiet by default - events only. For a diagnosis, set `ASURA_TRACE=1` in the shell before `start` (every service inherits it) and every packet is written to the logs.

## Everyday commands

```powershell
cargo run -p asura-supervisor -- status                # what is running, ready, restarting, or failed
cargo run -p asura-supervisor -- stop                  # stop everything (and the supervisor)
cargo run -p asura-supervisor -- stop --only NAME      # stop one service
cargo run -p asura-supervisor -- restart NAME          # stop and start one service
cargo run -p asura-supervisor -- build                 # build only
cargo run -p asura-supervisor -- schema                # apply the migrations
cargo run -p asura-supervisor -- seed                  # the dev identities and the map catalogue
cargo run -p asura-supervisor -- deploy-world --old 8001 --new 8002
```

`--config FILE` names a different service file (default `config/services.toml`); `build` does not read
it, and `schema` and `seed` accept it without using it (`crates/asura-supervisor/src/main.rs:52-74`).
The control channel answers on `127.0.0.1:7711`; a second supervisor on the same machine is refused.

## Rolling deploy

Uncomment the `[services.world-8002]` block in `config/services.toml` (a second world on this machine needs its own `--bind`, `127.0.0.1:6114`), then:

```powershell
cargo run -p asura-supervisor -- deploy-world --old 8001 --new 8002
```

The new build starts and is proven registered **before** the old world is marked draining. From then on the newest build serves new arrivals and reconnects; the old world keeps the players already inside it, and leaves when its last player has disconnected. Nobody is kicked. The rules and the why: `docs/02-architecture.md` (Deployment and draining).

## Starting the database yourself

The supervisor checks for PostgreSQL and waits for it, but never starts it: it is "managed elsewhere" by design. Start it the way your operating system manages services:

- **Windows:** `Start-Service 'postgresql-x64-<version>'` (find the exact name with `Get-Service *postgres*`). Starting a service may need an elevated shell; the supervisor is not elevated, so it cannot do this for you.
- **Linux:** `sudo systemctl start postgresql`.

If it is not running, `start` stops with a clear database error instead of pretending.

`tools/test-battery.ps1` is the local "everything" command (it ends printing the real-client steps, never automating them). The testing policy is in `docs/HANDOFF.md`: the real client is the test; the suite is not a gate.

## The client

The game client is Windows-only. The stock `Gw.exe` cannot talk to this server: it is patched first
with `client-patcher/patch-gw.py` and the keys in `client-patcher/keys.txt`, which are the same DH
parameters `crates/asura-protocol/src/client/handshake.rs` uses - the patched `Gw.custom.exe` and the
server agree on the handshake only as a pair. The patcher's README (`client-patcher/README.md`) has
the exact commands, the `portal.dll` step and what each patch changes.

Launch the patched client against the web gate, e.g. `-portal 127.0.0.1`, and log in as one of the
seeded accounts (`user1@example.com` / `user2@example.com` / `user3@example.com`; the web gate does
not check the password). What a good session looks like on the wire is in `recordings/`
(`recordings/README.md`).

The patcher leaves every baked-in ArenaNet hostname alone by default: the client stays live, and only
the auth and portal addresses come from its command line. That is the working configuration and the one
the seeded accounts were proven against.

Nothing else is redirected by a patch. A hostname reaches this machine only through the machine's hosts
file, put there by an operator, and the content request is answered only by a file server someone
installed. Both are deliberate and both are reversible:

1. `python client-patcher/hosts-redirect.py enable --file-server` (elevated; machine-wide block)
2. install the file server that answers `File1` content
3. build the matching executable with `--detach`, which rewrites the seven hostnames, the DNS base
   entry included

All three or none: a `--detach` build with the block off, or without the file server, cannot connect -
the client completes the auth handshake, receives the session info, then closes before the portal login
(`0x8038`) and `logs/webgate.log` never grows. Say which build is running before every session with
`python client-patcher/hosts-redirect.py status`; the block is machine-wide and removed with
`hosts-redirect.py disable`. `client-patcher/README.md`, "Read this first", is the same checklist with
the commands.

## Linux

`docs/17-running-on-linux.md`: the same supervisor commands; the client stays on Windows; the Linux host has not had a live run recorded yet, and the runbook says so. A world registered at a second address (the multi-machine proof) is `docs/18-multi-machine.md`.

## Docker (optional)

A second runner lives in `docker/` and is optional: the non-Docker path above is unchanged and
remains the default. The bundle is two containers - PostgreSQL, and one "machine" container that runs
`asura-supervisor` exactly as a host does: same start/stop, same schema and seed, same registry rows,
same logs. Docker only supplies the machine and the network around it.

```powershell
docker compose -f docker/compose.yml up -d --build
docker compose -f docker/compose.yml exec machine target/debug/asura-supervisor status
docker compose -f docker/compose.yml logs -f machine
docker compose -f docker/compose.yml down
```

- The Windows client dials `127.0.0.1:6112` and `127.0.0.1:6601` as it always does; those are the
  only published ports. Worlds are dialled by the auth service inside the same container.
- `config/services.docker.toml` is the same service list as `config/services.toml`, with one
  difference: auth (`:6112`) and the web gate (`:6601`) bind `0.0.0.0` inside the container, because a
  published port forwards to the container's network address, not its loopback. The world stays on
  loopback - the client never dials it directly.
- The database password is `${ASURA_DB_PASSWORD}` (default `asura`); set it in the shell or a `.env`
  next to the compose file before the first `up`.
- The image stamps the world's build id as `docker` (`ASURA_BUILD`), since a Docker build has no
  `.git`; a rolling deploy still works because placement uses `world.started_at`.
- The supervisor treats `docker stop` (SIGTERM) as the same quit as Ctrl+C, so a stop marks the
  registry rows `stopped` instead of being killed blind. Verified: `docker compose stop machine`
  left every `service` row at `stopped`.

**What was run (2026-10-28, Docker Desktop 4.97.0 on Windows):** the image builds, both containers
start, the supervisor applies the migrations, seeds the identities and maps, starts webgate, auth and
world-8001 (all `ready`), the registry rows read `ready`/`fresh`, and from the host the gate answers
`GET /Spawned/WebGate/session/create.xml` with a session UUID and the auth port accepts connections.
**Not yet run: the real game client against the bundle** - that is the owner's session, exactly as
for the host path.

## CI: format and lint, nothing else

`.github/workflows/ci.yml` runs two checks on every push:

- `cargo fmt --all -- --check` - the code is formatted;
- `cargo clippy --workspace --all-targets -- -D warnings` - the linter is clean.

Nothing else runs automatically. The test suite needs a live PostgreSQL database to mean anything, and the repository's testing policy says the game client is the real test and the suite is not a gate (`docs/HANDOFF.md`) - so CI is the cheap, database-free half, and the acceptance runs stay in the battery and with the real client. The decision is recorded in `docs/16-implementation-plan.md`, S7.

## When it goes wrong

| what you see | what it is | what to do |
|---|---|---|
| `cannot listen on 127.0.0.1:7711 ... another supervisor is probably running` | a supervisor is already up | `cargo run -p asura-supervisor -- stop`, then `start` |
| `could not bind 127.0.0.1:6113` | something else holds a port | `stop` waits for its ports to free; a leftover process needs killing once (`Get-NetTCPConnection -LocalPort 6113` on Windows names it) |
| the client shows error `007` | the server and client disagree on a message | `ASURA_TRACE=1`, then compare the trace with a good session in `recordings/` |
| the client closes shortly after the login screen appears, and `logs/auth.log` ends the session after the unnamed `0x8023` with no `AUTH_CMSG_PORTAL_ACCOUNT_LOGIN` | a repointed client without the provisioning: the hosts block is off, or there is no file server | `python client-patcher/hosts-redirect.py status`; enable the block and install the file server, or run a plain build, which is live |
| registry rows say `STALE` | a service stopped publishing without leaving | `status` shows the state; the reaper frees orphaned pool leases; `stop`, then `start` |
| `ASURA_DATABASE_URL is not configured` | there is no `.env`, or it is missing the URL | copy `.env.example` to `.env` and fill it in |
| `cargo cannot reach crates.io` | no network, or a sandbox token blocking TLS | the stack runs offline once dependencies are cached; from a normal shell, or with network access allowed |
