# ADR 0002 - A separate project instead of another fork of GWPS

- **Status:** Accepted (carried as a made decision in `docs/HANDOFF.md`, "Decisions already made")
- **Date:** 2026-10-25
- **Context:** GWPS works as a proof of concept and is the accumulated knowledge base for the client
  protocol. Its process model is what limits it: one program that answers the client, allocates, and
  runs every map, with a thread and a private copy of the game-file index per map. Rewriting that
  inside the same tree was the alternative.

## Decision

Start ASURA as its own project, with its own build, its own history, and its own vocabulary, and
keep GWPS as the reference implementation and the home of the world servers.

## Why

- **The two halves have different lifetimes.** The control plane is a new design that will churn.
  The game server is a large body of working code that mostly should not move. Sharing one tree
  means every control-plane experiment touches the simulation's history.
- **The reference must stay runnable.** The single most useful debugging tool for the Rust port is
  "run the C build and capture what it sends". That requires GWPS to remain buildable and
  unmodified by ASURA work.
- **Different language, different toolchain.** A C project built with a custom single-translation-unit
  build script and a Rust cargo workspace do not want to share a build root.
- **Clearer contract.** Two repositories force the interface to be explicit: a versioned protocol,
  documented, with both sides able to fail loudly. In one tree, that boundary erodes into shared
  headers and in-process shortcuts.

## What this costs

- Two repositories to keep in sync, and a protocol that must be versioned (accepted; see
  `docs/02-architecture.md`).
- Duplicate knowledge: the protocol facts live in one repo and the implementation in the other.
  Mitigated by `docs/03-protocol.md` naming the exact GWPS files it derives from.
- Any change to the schema or to a client-facing protocol requires a matching change in C.
  Accepted: that is the point.

## Consequences

- GWPS keeps the world servers and the client tooling.
- ASURA owns auth, sessions, directory, allocation, and persistence.
- The world-link protocol is versioned from the first commit, not retrofitted.
- Nothing about the game client, and nothing about map data, is duplicated here.
