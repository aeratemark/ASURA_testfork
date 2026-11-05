# Running the server on Linux

> **Status: a runbook, and honest about what it is not.** The server (web gate, auth, world servers,
> database) runs on Linux. The **game client remains Windows-only** - the client rig, its patching
> and its hooks are Windows work; a Linux machine is a server host. The services and the supervisor
> use no platform-specific APIs and the supervisor finds `asura-*` with or without the `.exe` suffix,
> so the same repository runs on both. What Linux has **not** had yet is a live run by this project:
> the evidence section at the end is empty on purpose, and the first operator fills it in.

## What runs where

| piece | runs on | listens |
|---|---|---|
| `asura-webgate` | the server host | 6601 |
| `asura-gateway` | the server host | 6112 |
| `asura-world` (one process per world) | the server host, or another machine | 6113 (and its registered public address) |
| PostgreSQL | the server host, or another machine | 5432 |
| the game client | a Windows machine | connects to the gate and the auth address |

The client is never told to connect to a world directly: it dials the address it was launched with
(the auth service), and the front door routes the gameplay connection (`docs/03-protocol.md` #4).
So a remote client only needs the **web gate** and the **auth service** reachable; the world can stay
on `127.0.0.1` when everything is on one machine.

## Prerequisites

- **Rust**: a current stable toolchain (`rustup`); the workspace declares `rust-version = "1.75"`.
- **Git**.
- **PostgreSQL 16**, running, with a role and a database for ASURA.
- A `.env` at the repository root (copy `.env.example`): `ASURA_DATABASE_URL` is the one every
  program reads. Secrets stay there; topology stays in `config/services.toml`.

## First time on a fresh machine

Run from the repository root.

1. **Role and database** (with a superuser, e.g. `sudo -u postgres psql`):

   ```sql
   CREATE ROLE asura LOGIN PASSWORD '<password>';
   CREATE DATABASE asura OWNER asura;
   ```

2. **Schema**: one command applies every migration, in order (they are idempotent, and a program
   started against an older schema refuses to run):

   ```bash
   cargo run -p asura-supervisor -- schema
   ```

   It drives `psql` - taken from `ASURA_PSQL` first, then PATH (`psql` on Linux), then the usual
   Windows install (`crates/asura-supervisor/src/psql.rs:13-31`).
   The equivalent by hand is a `psql -v ON_ERROR_STOP=1 -f` loop over
   `crates/asura-db/migrations/*.sql`.

3. **`.env`**: copy `.env.example` to `.env` and set `ASURA_DATABASE_URL` (and the `ASURA_DB_*`
   values if you keep them in sync).

4. **Build** - the first build needs the network for the crates; after that the supervisor's
   `--offline` build works from the cache:

   ```bash
   cargo build
   ```

5. **Seed** - one command writes both sets of rows: the dev identities the client presents, and the
   map catalogue. The SQL is in the repository (`tools/seed-dev.sql`, `tools/map-config.sql`) and
   is applied the same way on Windows and Linux:

   ```bash
   cargo run -p asura-supervisor -- seed
   ```

   The three portal pairs (they must resolve, or the login is refused):

   | account | portal user id | session id |
   |---|---|---|
   | `user1@example.com` | `fa520ee2-4419-4eb4-ae49-6e9abe6ef24f` | `d8b9bf5d-90b1-4cbd-9b76-88da7be763b6` |
   | `user2@example.com` | `5fb222bc-9d19-4308-ac3e-3c62685bc6ae` | `224aee30-d8a5-4f18-ad8e-b5fcbf121c4e` |
   | `user3@example.com` | `9957c79c-b93c-4b60-9d84-9f38a1f62be3` | `ccdb4b8d-1c88-48c0-bda0-0a5e8d87f4da` |

6. **Start** - one command, as on Windows:

   ```bash
   cargo run -p asura-supervisor -- start
   ```

   It builds what is missing, seeds the same rows step 5 writes (idempotent, so a running start
   re-seeds harmlessly), starts the services in dependency order - alphabetical while none declares
   `depends_on`, so auth, then the gate, then world (`crates/asura-supervisor/src/config.rs:192-200`) -
   waits for each to be healthy, and watches
   until Ctrl+C. The first build of the three services inside `start` uses `--offline`; because
   step 4 fetched everything, that succeeds.

7. **Verify**:

   ```bash
   cargo run -p asura-supervisor -- status        # webgate, auth, world: ready
   curl http://127.0.0.1:6601/health              # 200 ok
   ```

## Reaching it from a Windows client

The client is launched with the addresses it should dial:

```
Gw.custom.exe -authsrv <server-host> -portal <server-host>
```

- The **web gate** (`-portal`) and the **auth service** (`-authsrv`) must be reachable from the
  client. In `config/services.toml`, their `args` bind `127.0.0.1` - change to the reachable address
  (e.g. `"0.0.0.0:6601"` and `"0.0.0.0:6112"`) and allow the ports through the host firewall.
- The **world** can stay on `127.0.0.1` when the gate and auth run on the same machine: the front
  door dials the world's registered address from there. If a world runs on another machine, its
  `--public` must name an address the **auth host** can reach (it is what the front door dials).
- Game traffic is the client's second connection to the auth address; no wiring changes are needed
  for it (the front door routes it).

## Day to day

```bash
cargo run -p asura-supervisor -- start            # the one command; Ctrl+C stops
cargo run -p asura-supervisor -- status
cargo run -p asura-supervisor -- stop             # or stop --only <service>, or restart <service>
```

- Logs: `logs/webgate.log`, `logs/auth.log`, `logs/world-<id>.log` (appended).
- The supervisor stops, on exit, exactly what it started; the database is checked and waited for,
  never started (on Linux it is a systemd unit, managed elsewhere).
- A crashed service is restarted at most `restart_max` times (see `config/services.toml`), then
  reported `failed` - never forever. A stop waits for the service's port to be free.

## A release build

The supervisor's `build` step builds the services in **debug** (`start` also runs it, harmlessly, if
the programs already exist). For a production host:

```bash
cargo build --release
```

and point `config/services.toml`'s `exe` entries at `target/release/asura-*` (no `.exe` on Linux;
the supervisor finds either). Nothing else changes.

## What this document does not cover yet

- **No CI-built binaries** - Linux artifacts (CI) are a later item; today, build from source.
- **No live Linux run recorded** - the code is cross-platform by construction, not by measurement.
  The row below stays empty until someone runs it and writes down what they saw.
- **The client rig is Windows-only** - that is the client's own work, not the server's.
- The supervisor's own tests are live-database tests: they run with `ASURA_DATABASE_URL` set (and
  skip cleanly without one).

## Evidence (fill in when someone runs it)

| what | expected | observed |
|---|---|---|
| first start | `webgate`, `auth`, `world-<id>` all `ready` | |
| gate health | `GET /health` -> `200 ok` | |
| client login from Windows | character list, then select | |
| travel | `/tp` lands on the world | |
| stop | Ctrl+C (or `stop`); every port free | |
| notes | - | |
