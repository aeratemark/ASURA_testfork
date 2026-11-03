# A reference session, measured

A complete login, recorded from both servers on 2026-10-26. This is the yardstick: the order, the
headers, the sizes, and every message either side sent. Anything in ASURA that disagrees with a line
below is a bug, not a preference - the packets are the objective (`AGENTS.md` #1), and
`docs/adr/0010-gwps-is-the-floor.md` is where that reference-as-floor reading was argued.

How it was captured:

- GWPS: `GWPS_TRACE=1`, with the auth trace added by `tools/patch-gwps-auth-trace.py` (the reference
  logged game messages only; this adds both directions of the auth connection, plaintext, before
  encryption).
- ASURA: `ASURA_TRACE=1`, which logs the same four things.

## The session

Client build 39112, `-authsrv 127.0.0.1 -portal 127.0.0.1`, web gate on 6601, identity
`user1@example.com` = `fa520ee2-...` / `d8b9bf5d-...`.

| # | direction | header | size | what it is |
|---|---|---|---|---|
| 1 | client -> server | `0x0400` | 16 | version: build 39112, `h0008` 1, `h000C` 4 |
| 2 | client -> server | `0x4200` | 66 | client seed: the client's public value |
| 3 | server -> client | `0x1601` | 22 | server seed: `master XOR shared` |
| 4 | client -> server | `0x8001` | 42 | computer info: `03 00 "Apo" 0f 00 "DESKTOP-..."` |
| 5 | client -> server | `0x8002` | 22 | computer hash: build + 16-byte hash |
| 6 | server -> client | `0x0001` | 10 | session info: a random salt - **the answer to 5** |
| 7 | client -> server | `0x8000` | 6 | heartbeat |
| 8 | client -> server | `0x8023` | 6 | unnamed; the reference logs and ignores it |
| 9 | server -> client | `0x0000` | 6 | heartbeat, `tick = 16` |
| 10 | client -> server | `0x8038` | 54 | portal login: `req 1`, user id, session id, no name, `"Portal"` |
| 11 | server -> client | `0x0007` | 102 | character: id, `unk0`, u16 name count + name, u16 + 62-byte settings |
| 12 | server -> client | `0x0016` | 599 | account settings: u16 count 591 + the blob |
| 13 | server -> client | `0x0014` | 10 | friend stream end: the request id twice |
| 14 | server -> client | `0x0011` | 81 | account info: account id, character id, 8 features, eula, `unk5 = 3` |
| 15 | server -> client | `0x0003` | 10 | request response: `req 1`, status 0 |
| 16 | client -> server | `0x8026` | 3 | accept EULA (`1a`) |
| 17 | client -> server | `0x8035` | 6 | ask server response, `req 2` |
| 18 | server -> client | `0x0003` | 10 | request response: `req 2`, status 0 |
| 19 | client -> server | `0x800E` | 6 | set player status: online (`1`) |
| 20 | client -> server | `0x8029` | 26 | request game instance: `req 3`, type 1, **map 146**, region 0, district 2 |
| 21 | server -> client | `0x0009` | 42 | **game server info**: the redirect - address, port, map token, player token |
| 22 | server -> client | `0x0003` | 10 | request response: `req 3`, status 0 |
| 23 | client -> server | `0x4200` | 66 | **a new connection** - the game one, same handshake |

## What this settles

- **The login batch ends the auth conversation's first half.** After message 15 the client proceeds on
  its own: EULA, ask-response, player status, then it asks for a map. It sends nothing that waits on
  the server in between, so a client that stalls before 16 did not receive 11-15 correctly.
- **The character select screen appears before message 20 is answered**, and the client asks for its
  character's last map (146, the `last_outpost` in the settings block) immediately.
- **`0x8029` is where the auth service's job ends.** It must answer with `0x0009` - an address, a port
  and two tokens - and the client then dials that address and speaks the game protocol (message 23).
  That is the simulation, and it is M4 (`docs/roadmap.md`).

## What ASURA did, for comparison

At M3, `crates/asura-auth` reproduced 1-19 exactly (same order, same sizes, same replies) and refused
20 with `status 2` because no world server existed. **That is history**: request 20 is answered with a
world from the registry now (`crates/asura-auth/src/lib.rs:604-618`), and a real client logs in,
travels and stands in a map (`MILESTONES.md`). The recorded M3 behaviour is kept because it is what the
first comparison measured.

The M3 finish line was: the patched client reaches the character list against this service.
