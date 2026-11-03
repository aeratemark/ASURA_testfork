# Multi-machine capability, proven on one machine (S6)

> The design says a world server is interchangeable and can live on another host - the client is
> told a real address, and the front door dials the **registered** address, so moving a world should
> be configuration, not code. This document proves that, without hardware, and states what a real
> second machine would still add.

## The claim, and its parts

| part | where it is |
|---|---|
| the world registers where a *client* reaches it (`--public`), separate from where it binds (`--bind`) | `crates/asura-db/src/placement.rs`, `register_world` |
| placement offers worlds by their registered address | `find_worlds` |
| the auth front door dials the **registered** address of the world the client named | `crates/asura-auth/src/frontdoor.rs` |
| a world at another address can be drained and leaves by itself | `SET draining` -> the world's heartbeat loop (S3) |

## The experiment on one machine

All of `127.0.0.0/8` is loopback on Windows and Linux, so `127.0.0.2` is a genuinely different
address to every component: the registry stores it, the front door dials it, and the world binds
it. It is the single-machine equivalent of a second host.

Config used (two worlds on the box, the second at the "other machine" address):

```
[services.world-8001]
args = ["--world", "8001", "--capacity", "8", "--bind", "127.0.0.1:6113", "--public", "127.0.0.1:6113"]

[services.world-8002]
args = ["--world", "8002", "--capacity", "8", "--bind", "127.0.0.2:6114", "--public", "127.0.0.2:6114"]
```

With `ASURA_TRACE=1` on the auth service:

1. **Both worlds register.** `SELECT id, address, port FROM world` ->
   `8001 | 127.0.0.1 | 6113` and `8002 | 127.0.0.2 | 6114`, both fresh.
2. **A game connection naming world 8002 arrives at the auth address** (127.0.0.1:6112 - the client
   always dials the launch address; see `docs/02-architecture.md`). The auth trace says:

   ```
   auth: routing the game connection to world 8002 (asura-dev) at 127.0.0.2:6114
   ```

   and the world at `127.0.0.2` receives it:

   ```
   world <- game version build=39112 world=8002 map=146 player token 0x12345678
   ```

   The front door could only dial `127.0.0.2` because that is the **registered** address - it never
   sees a world's bind address. (The probe had no pending transfer, so the world then refused it,
   which is the correct answer for a connection nobody issued a token for.)
3. **The second-address world drains across the "machine" boundary.** Marking it draining:
   `world: drained; the last player has left; leaving the registry`, its row disappears, world 8001
   is untouched. Placement, routing and the drain all treat the address as data.

## What a real second machine would add, and what it would not

- **Would add:** network latency, NAT and firewall operation, DNS or a realmlist-style address, and
  the operational work of starting the second supervisor with its own `config/services.toml`. None of
  that is protocol or design work.
- **Would not add:** a second auth service, a gateway, a new message, or a client change. The client
  dials its launch address; the front door dials the registered world address. A world on host B is
  one `--public` value and one registry row.
- **The one rule holds:** the client is only offered a world that is answering (the auth dials the
  candidate port before it is offered).

## Evidence to fill in with the real client

The last step is the owner's: point the real client at the auth address with a second-address world
registered, stand in a map on `127.0.0.1` (world 8001), travel to a map served by `127.0.0.2`
(world 8002), and confirm it lands - then the same with a real second host when one exists.

| step | expected | observed |
|---|---|---|
| both worlds register at their own addresses | `8001@127.0.0.1`, `8002@127.0.0.2` | yes (2026-10-28, one machine) |
| front door dials the registered address | trace names `127.0.0.2:6114` | yes (2026-10-28) |
| second-address world drains and leaves | row removed, the other world untouched | yes (2026-10-28) |
| real client travels between the two | lands, no kick | _to run_ |
