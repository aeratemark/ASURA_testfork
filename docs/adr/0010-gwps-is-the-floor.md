# ADR 0010 - GWPS is the floor, not a reference

- **Status:** Accepted (owner's direction, 2026-10-26)
- **Date:** 2026-10-26
- **Supersedes in part:** `docs/adr/0005-gwps-reference-only.md`. That ADR said GWPS is research
  material and that ASURA is written from scratch. It is still true that nothing is copied from GWPS
  *code*. What is now decided is different and stricter: **anything GWPS implements on a path ASURA
  claims is a requirement, not an option.**

## The rule

> GWPS is the absolute minimum required to function. If GWPS sets up a structure, a handler or a
> packet, ASURA needs one too. If we have less than GWPS on a path we claim to support, we do not
> support it.

The failure that produced this rule: a real client hung at "Connecting" against this service for
exactly one reason - the reference answers `AUTH_CMSG_SEND_COMPUTER_HASH` with
`AUTH_SMSG_SESSION_INFO`, and this service logged the message as "unhandled" and sent nothing. No
amount of internal correctness helped; the client was waiting for a packet we had decided we did not
need. Everything else about that connection was right.

## What the rule means in practice

1. **The comparison is per handler, not per crate.** For every `case` in the reference's auth dispatch,
   there must be either a corresponding behaviour here or a recorded entry in the table below saying
   why not.
2. **"Unhandled, warn and carry on" is not a behaviour.** It is a symptom. If the reference answers,
   we answer; if it stores, we store.
3. **A missing reply is worse than a wrong one.** A wrong reply produces a client error; a missing
   reply produces a hang with no information at all, which is what cost this session its afternoon.
4. **The audit table is maintained, not written once.** A new message the reference handles goes in the
   table the moment it is found.

## The audit, on the auth path (2026-10-26)

The reference's auth dispatch, from `AuthSrv.c`, against this service:

| reference behaviour | here | note |
|---|---|---|
| listen on 6112 **and** 6113 | yes | one process per address is fine; both must be bound |
| file check, 21 bytes -> 32 bytes, before the version message | yes | `client::filecheck` |
| `AUTH_CMSG_VERSION` | yes | `client::version` |
| client seed -> server seed (fixed DH, ARC4) | yes | `client::handshake` |
| `AUTH_CMSG_SEND_COMPUTER_INFO` (variable length in this client) | yes | `messages::decode_computer_info` |
| `AUTH_CMSG_SEND_COMPUTER_HASH` -> `AUTH_SMSG_SESSION_INFO` | yes | the reply that unblocks a real client |
| `AUTH_CMSG_PORTAL_ACCOUNT_LOGIN` -> the five-message batch | yes | `answer_login` |
| `AUTH_CMSG_ASK_SERVER_RESPONSE` -> request response | yes | |
| `AUTH_CMSG_SET_PLAYER_STATUS` (stores, no reply) | yes | stored nowhere yet; accepted |
| `AUTH_CMSG_CHANGE_PLAY_CHARACTER` | yes | selects, records on the session, answers |
| `AUTH_CMSG_HEARTBEAT` -> `AUTH_SMSG_HEARTBEAT` (tick 16) | yes | |
| `AUTH_CMSG_DISCONNECT` | yes | closes the connection |
| `AUTH_CMSG_ACCEPT_EULA` | yes | the version is stored (`account.eula_accepted`, `0012_account_eula.sql`) |
| `AUTH_CMSG_SETTING_UPDATE_SIZE` / `..._CONTENT` | yes | acknowledged |
| `AUTH_CMSG_UPDATE_CHARACTER_SETTINGS` | partial | acknowledged, **not stored** |
| `AUTH_CMSG_ADD_ACCESS_KEY` | yes (nothing stored) | logged and dropped, exactly as the reference does (`crates/asura-auth/src/lib.rs:487`) |
| `AUTH_CMSG_DELETE_CHARACTER` | yes | deletes this account's character by name (`crates/asura-auth/src/lib.rs:513-532`) |
| `AUTH_CMSG_REQUEST_GAME_INSTANCE` | yes | answered with a world from the registry (`crates/asura-auth/src/lib.rs:604-618`) |
| pre-spawned map servers, ctrl connections, travel | yes | the whole travel path exists: `begin_transfer` (`crates/asura-world/src/travel.rs:441`), relayed by `crates/asura-auth/src/frontdoor.rs` |

Two of the three gaps named in this table at the time are closed: `ACCEPT_EULA` now stores its version, and
`DELETE_CHARACTER` is implemented. What is still open is the storage behind
`AUTH_CMSG_UPDATE_CHARACTER_SETTINGS` (acknowledged, not stored) and `ADD_ACCESS_KEY`, which stores
nothing because the reference stores nothing. None of them is on the login path.

## What this rule does not say

- **It is not "copy GWPS".** The structure stays AzerothCore's (`docs/adr/0004-port-azerothcore-structure.md`),
  and nothing is transcribed from GWPS source. The rule governs *behaviour coverage* on the client
  path, not code.
- **It is not "GWPS is correct".** The reference mis-frames `SEND_COMPUTER_INFO` itself (it logs a
  bogus `header 35` and only survives by tolerating it) and its file-check ids differ from a real
  capture by `0x100`. Where GWPS is wrong, we are still allowed to be right - but we say so in
  writing, and we keep the reference's behaviour in mind as the thing the client will accept.
- **It does not raise the bar for anything client-invisible.** Internal structure is judged by the
  architecture documents. The floor is about what the client can observe.
