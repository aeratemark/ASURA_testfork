# ADR 0007 - Rust, translated from AzerothCore's C++

- **Status:** Accepted. Decided on merits under `docs/adr/0006-azerothcore-is-the-reference.md`
  (the rule governs structure; the language is a non-structural choice). Overriding it is a matter of
  saying so and writing a replacement ADR -- it is not an open question.
- **Date:** 2026-10-25
- Extends `docs/adr/0001-rust.md` from the control plane to the whole stack.

## How this was decided

Asked directly whether the stack is C++ or Rust, the answer is **Rust**, and the reasons are the four
below that weigh on this project specifically: the bug families Rust eliminates are the ones this
system invites (session and instance ownership across threads); no AzerothCore code is reused either
way, so C++ buys familiarity with code we have agreed not to copy; a one-person project benefits from
`cargo test`/`clippy`/one manifest over CMake + MSVC + vcpkg; and a protocol whose travel path is
almost entirely refusal branches suits errors-as-values.

C++ would be the right call if we intended to adopt AzerothCore code fragments or to hand work to a
C++-only contributor. Neither is the plan.

## Context

AzerothCore is C++ - `CMAKE_CXX_STANDARD 20`, CMake, Boost.Asio for networking, `fmt`, MySQL through
its own `DatabaseWorkerPool`. The owner's stated preference is Rust, and the governing rule is that
*design* decisions follow AzerothCore (`docs/adr/0006-azerothcore-is-the-reference.md`).

Those two are compatible, and it is worth saying why plainly: their C++ is specific to a different
engine and a different client, and it is not ours to copy. What we take is the **structure** - which
processes exist, where state lives, how placement is arbitrated, how instances are managed. That
structure is language-neutral. Translating it to Rust keeps every structural decision theirs and only
changes the spelling.

## Decision

The whole stack is Rust, and every AzerothCore concept is mapped to a named Rust equivalent at the
start of the work it belongs to, so that "what does AzerothCore do?" always has an answer in our code.

| AzerothCore (C++) | ASURA (Rust) |
|---|---|
| `authserver`, `worldserver` executables | `asura-auth`, `asura-world` binaries |
| `boost::asio` networking, `SocketMgr` | `tokio` (or `mio` if a tighter loop is ever needed) |
| `DatabaseWorkerPool` (sync pool + async queue + worker) | `sqlx` pool plus an explicit writer task; the queue depth is a metric |
| `PreparedStatement`, `Transaction`, `QueryHolder` | typed query functions in `asura-db`, `BEGIN/COMMIT` around multi-statement work |
| `WorldPacket` / `ByteBuffer` (little-endian readers/writers) | explicit `to_le_bytes`/`from_le_bytes` in `asura-protocol`, with size assertions in tests |
| `ObjectGuid`, `InstanceId` | typed ids (newtypes over `u64`/`Uuid`), never bare integers |
| `MapUpdater` with a bounded thread pool | `tokio` worker tasks with map affinity, or a small custom pool |
| `MapInstancer` pools and instance saves | instance manager in `asura-world`; instance records in `game` |
| `realmlist` + realm flags | `world` + `world_pool` tables |
| `.conf` files | configuration from the database and environment, same values |
| their logging module | `tracing` with the same structured fields |
| `std::shared_mutex` for shared state | `RwLock`/`Mutex`, or message passing where it avoids the lock entirely |

**Four entries in that table are the target, not the tree.** The asynchronous write queue behind `sqlx`
is not built: writes are awaited on the calling task. Ids are bare integers, not newtypes
(`crates/asura-world/src/player.rs:24`). The map-affinity worker pool is not built: one task per map
(`crates/asura-world/src/instance.rs:376-385`). Logging is `eprintln!`, not `tracing` - no crate in the
workspace names that dependency.

Crates are added when the code that needs them exists, not before. The scaffold is dependency-free on
purpose, so the toolchain can be proven before any dependency decisions are baked in.

## Consequences

- Every structural decision can be cited to AzerothCore; the translation makes the citation concrete.
- No code is copied, so licensing and engine assumptions stay out of this repository.
- Rust's ownership model changes *how* some patterns are written (particularly shared session state and
  instance ownership), and the translation may reveal places where their design does not fit. Each of
  those becomes a recorded deviation, not an improvisation.
- The world server is the large piece: following their structure means writing an instance manager, a
  map-update pool, a session layer and a persistence layer before travel can be tested end to end. That
  is the cost of starting over, and it is the reason the scaffold and the milestone order matter.

## Rejected

- **C++ to stay close to AzerothCore.** Closeness would be cosmetic (their code is engine-specific and
  licensed), and it would give up the memory-safety and tooling advantages in the part of the system
  that is all concurrent state machines.
- **Rust for services, C++ for the map loop later.** Two languages, two build systems, and a boundary
  that has to be maintained forever, for a phase-2 optimisation that may not be needed.
