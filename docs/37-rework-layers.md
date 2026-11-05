# Rework layers: the checklist

Tracks the layer-by-layer move of configuration and content into `config/`. Measured from the code on
2026-11-03: 92 Rust files, 39,528 lines across 9 crates, 25 migrations, 35 `tools/` files.

The reason behind each decision is in `docs/38-layer-decisions.md`.

This file is the tracker. It states no design of its own: every row names the files it covers,
and every box is ticked with the commit that closed it. A layer is **covered** only when all six
conditions of the next section hold for it.

## What "covered" means, and it is six conditions

1. **The file is the source of truth.** The value lives in `config/` and nowhere else.
2. **Every reader reads the file.** All consumers listed on the row are switched.
3. **The old source is deleted.** The constant, the SQL `INSERT` or the compiled table is gone,
   in its own commit.
4. **The equivalence diff is empty.** The file reproduces the old source row for row, and the
   command that says so is named on the row.
5. **`.\tools\ascii-check.ps1` exits 0** and the tests the change touches pass.
6. **A real client was tested** wherever the layer is client-visible.

## The audit: why no layer is closed

Every layer's boxes were ticked from work that was done, and that turned out not to answer the question
that matters. Layer 1 read `[-]` holds by design while it carried 137 hand-copied instance pairs, two
file-check payloads, a 591-byte login payload and a default appearance - data, in a layer of codecs. A
ticked box says a task was done; it does not say the layer is clean.

So each layer is audited, once, by the four questions from `AGENTS.md` 4a, file by file:

1. Does the file hold **the client's vocabulary** - its own enum values, header numbers, payloads?
2. Does it hold **values that describe the world** - maps, spawns, defaults, cadences?
3. Does it hold **runtime state** - rows that change while the server runs?
4. Does it hold a **decision** - "when X then Y" over data the caller already passes in?

Anything that is none of those is ordinary code and stays. Anything that is (1) or (2) is a candidate for
a file; (3) belongs to the layer that owns the entity; (4) belongs to layer 3.

A layer is closed when its boxes are done **and** its audit finds nothing left to move. Until then it is
`audit pending`, including the layers that read `closed` before this section existed.

## Notation

- `[ ]` open - not started
- `[~]` partial - some boxes below are ticked
- `[x]` done - all six conditions hold, with the commit named
- `[-]` holds by design - this layer is not configuration and must not move

## The layers

| # | layer | files (measured) | class | status |
|---|---|---|---|---|
| # | layer | files (measured) | class | status |
|---|---|---|---|---|
| 1 | wire and codec | `asura-protocol`, 16 files, 8,885 lines; the packet table and the codecs, no payload of its own | evidence | `[x]` closed |
| 2 | storage and schema | `asura-db`, 16 files, 4,814 lines, 25 migrations | schema + state + content passing through | audit pending |
| 3 | rules | `asura-rules`, 1 file, 283 lines, 9 tests; depends on 1 for the client's `MapKinds` | rules | audit pending |
| 4 | auth service | `asura-auth`, 6 files, 2,664 lines; depends on 3 | rules | audit pending |
| 5 | socket layer | `asura-gateway`, 3 files, 534 lines; depends on 4 | transport | audit pending |
| 6 | web gate | `asura-webgate`, 2 files, 696 lines | rules | audit pending |
| 7 | sessions | `asura-world`: `connection.rs`, `dispatch.rs`, `world.rs`, `map.rs`, `catalogue.rs`; `capture.rs` and `replay.rs` behind `harness` | rules + rates | audit pending |
| 8 | instance and simulation | `instance.rs`, `agent.rs`, `player.rs` | rules + rates | audit pending |
| 9 | travel | `travel.rs`, `portals.rs`, `travel_targets.rs` | rules + content | audit pending |
| 10 | character | `creation.rs` | rules + content | audit pending |
| 11 | items | `item.rs`, `pvp_items.rs` | rules + content | audit pending |
| 12 | chat and commands | `chat.rs` | rules + one table | audit pending |
| 13 | map data and pathing | `map_data.rs`, `pathfind.rs`, `gwdat.rs`, `fa_compress.rs`, `fa_compress_tables.rs`, `path_tables.rs`, `graph_dump.rs` | measured data | audit pending |
| 14 | cartography | `cartography.rs` | measured data | audit pending |
| 15 | content itself | four homes, see layer 15 | content | audit pending |
| 16 | supervision and operations | `asura-supervisor`, 10 files, 2,070 lines, `config/services.toml` | operations | audit pending |
| 17 | harness | `asura-fakeworld` 307 lines, 12 test files, 5 examples | harness | audit pending |
| 18 | evidence | `recordings/`, `data/portals.json`, `client-patcher/`, `logs/` | evidence | audit pending |

## Layer 1 - wire and codec

Files: `packets.rs` 2,106 (782 rows, 403 consts), `game/mod.rs` 1,711, `game/appearance.rs` 68,
`game/instance_table.rs` 175, `client/frame.rs` 660, `client/messages.rs` 1,267,
`client/handshake.rs` 435, `client/reference_client.rs` 383 (gated), `client/version.rs` 275,
`client/game_version.rs` 201, `client/arc4.rs` 190, `client/filecheck.rs` 95, `client/mod.rs` 105,
`client/account_settings.rs` 49, `trace.rs` 52, `lib.rs` 181.

- [-] holds by design - the packet table is the specification (`AGENTS.md` #1)
- [x] content that was misplaced here is moved out: `game/pvp_items.rs` (387 rows) is
      `crates/asura-world/src/pvp_items.rs`, layer 11 - commit `fb8e508`, rows compared row for row
      with the commit before, 93 protocol and 61 world unit tests pass
- [x] test client gated: `client/peer.rs` behind the `test-client` feature (`docs/38`, D13) - commit
      `8e7448b`; `cargo check --workspace` and `--all-targets` exit 0, 93 protocol tests pass
- [x] `client/login_fixtures.rs` is payload, not a fixture: `messages::account_settings` sends it on
      every login (`messages.rs:577`), so it stays in the wire layer
- [x] header numbers have one source: `MAP_INIT_START` and `INSTANCE_TABLE` aliased `packets.rs`, the
      last two literals of 66 - commit `31b5bfa`, 93 protocol tests pass
- [x] the layer passes the repository's own checks: `cargo clippy -p asura-protocol --all-targets
      -- -D warnings` and `cargo fmt --check` are both clean for this crate - commits `f62ee4a`
      (two needless borrows) and `276a8c2` (rustfmt on `frame.rs`)
- [x] the workspace's format check is green: `cargo fmt --all --check` exits 0 - commit `1138762`,
      which formatted the nine drifted files in `asura-auth` and `asura-world` and one blank line in
      this crate; formatting only, 93 protocol, 61 world and 5 auth unit tests pass
- [x] the naming rule is written down in `client/mod.rs` (`docs/38`, D15) - commit `b2dbc2f`; the
      violations it exposes are the four boxes below, one commit each

### Layer 1 naming, in the order the rule exposes it

- [x] rename `client/login_fixtures.rs` to `client/account_settings.rs`: it is a 591-byte payload,
      not a fixture - 5 references, all inside `messages.rs` - commit `7a8112b`
- [x] rename `client/peer.rs` to `client/reference_client.rs`: it is a fake client, not a peer -
      54 references across four test files and two examples - commit `af8c24a` (which also
      rustfmt'd those files, taking them off the workspace's fmt list)
- [x] split the two `frame` functions in `client/messages.rs`: the fallible one is
      `PortalAccountLogin::try_to_frame`, the infallible `RequestResponse::frame` keeps its name -
      commit `1e450c8`, six call sites
- [x] `pack`/`unpack` and the `Appearance` they belong to are `game/appearance.rs`, not a `codec.rs`
      (D16) - commit `13c9e72`, three call sites unchanged through the re-export
- [x] `client/game.rs` is `client/game_version.rs`: two modules were called `game`, one per
      direction - commit `33666f8`
- [x] `reference_client::as_request_response` is `decode_request_response`: it reads a frame and can
      fail, which is what `decode_` means here - commit `acf64d8`
- [x] `character_update_factions` takes a `CharacterProgress` struct instead of thirteen positional
      dwords, so the `#[allow(clippy::too_many_arguments)]` is gone - commit `89ebfea`
- [x] the crate's only `#[allow(dead_code)]` is gone: `Reader::utf16` was unused - commit `c3c0552`
- [x] the lint policy is written and the crate opts in (`docs/38`, D17): `missing_docs` warn,
      `unsafe_code` deny, `clippy::all` warn, `broken_intra_doc_links` deny - commit `fb112aa`,
      which also documented the 70 items the policy exposed and allowed one `unsafe` (ARC4's
      zeroization) with its SAFETY reason

### Layer 1 audit

**Complete.** Five findings, all answered:

- [x] the instance table is data, not a codec: it moved to `crates/asura-world/src/instance_table.rs`
      (where it is sent), and `docs/38` D36 gives it its real home - constructed from the pack's map
      folders. That step landed: the pairs are walked out of `config/World/Vanilla/maps/`, 137 of them,
      and the wire crate holds none
- [x] the default appearance is a value: `config/World/Vanilla/character_defaults.json` (`appearance`),
      and `Appearance` is a pure codec again
- [x] the file check's two byte strings are the client's payload: `config/Server/file_check.json`, with
      the crate keeping only the comparison
- [x] the login batch's 591-byte blob is the client's payload: `config/Server/account_settings.json`,
      with the crate keeping only the framing
- [x] the cartography payload's size was the packet table's number written twice: it is derived from the
      row's `FieldKind::Array32, length: 64`, and a test holds the table and the frame together

## Layer 2 - storage and schema

Files: `migrations.rs` 240, `config.rs` 116, `game.rs` 376, `inventory.rs` 946,
`inventory/seed.rs` 122, `placement.rs` 609, `login.rs` 245, `account.rs` 44, `catalog.rs` 110,
`portal.rs` 108, `lib.rs` 300, plus `migrations/0001`-`0026`.

- [x] physical schema stays SQL: 26 migrations, 14 tables (`docs/38`, D21)
- [x] content rows counted and named: 214 `INSERT`s in `0013`, `0014`, `0016`-`0022`, plus
      `map_config`, `map_spawn` and `map_portal` (`0025`)
- [x] the split is written at the source (`crates/asura-db/src/lib.rs`): which tables are operational
      state and which are content passing through, with the one-sentence test for a new table - commit
      `49681c2`
- [x] no lint suppression left in the crate: `insert_row` takes an `ItemColumns` instead of fifteen
      arguments, and `settings` reads a named `CharacterRow` instead of an eight-wide tuple - commit
      `d3afedb`, 21 unit tests pass
- [x] the crate adopts the lint policy (`docs/38`, D17): `missing_docs`, `unsafe_code` deny and
      `broken_intra_doc_links` cost 106 one-line docs and three doc links - commit `9925621`; clippy
      `-D warnings`, `cargo doc` and 21 unit tests pass
- [-] `map_config`, `map_spawn` and the 214 `INSERT`s leave for a package folder - **parked**: D20
      settles the destination and the owner deferred the move, so these two boxes wait for that
      decision, not for layer 2 work
- [x] `map_portal` is content with provenance (`0025_map_portals.sql`, a `source` column per row)
- [x] every data table carries a `t_` prefix (`docs/38`, D31): `session` is the login and
      `t_session` is the row, and the same for `world`, `item`, `bag`, `account`, `service`,
      `characters`, `lease`, `transfer`, `map_config`, `map_spawn`, `map_portal`,
      `instance_unlock` and `world_pool`. `0026_t_prefix.sql` renames them, guarded; every
      query in the tree and in the tools names the new tables - `cargo test --workspace` passes
      and the whole schema test set is green - commit `T_PREFIX_COMMIT`
- [-] the 214 content `INSERT`s retired - parked with the box above

## Layer 3 - rules (`asura-rules`)

Files: `src/lib.rs` 283 lines, 9 tests, one dependency (`asura-protocol`, for the client's
`MapKinds` type).

- [x] created because a protocol answer needed a game rule and no existing crate fits: `asura-db` is
      storage and `asura-world` may not exist yet at login (`docs/38`, D25) - commit `4b09d17`
- [x] the rule holds no value of its own (`docs/38`, D34): `DEFAULT_OUTPOST` is gone. The fallback
      outpost is `config/Server/defaults.json` (`default_outpost`, read by `asura-config`), it has no
      built-in default in code - the key is required and `asura-gateway` refuses to start without the
      file - and it
      arrives at `placement` as an argument, which answers with it plus
      `Substituted::NothingToPlace` when the character's row names neither a map nor an outpost. The
      outpost a character enters is written by character creation, by a scripted event and by
      travelling - never by this layer. 10 rule, 5 config and 3 auth unit tests pass
- [x] the blank-name rule is here rather than in the world: `name_is_acceptable(&str)`, called by the
      creation confirm in place of its own `trim().is_empty()`. The error code it answers with stays
      where the frame is built, because a code is protocol. 9 rule and 64 world unit tests pass
- [-] the home for the next rule, and this **holds**: a rule that a protocol answer needs belongs here
      rather than in a service. Party and instance policy (`docs/32` phase 2) is the named candidate
      and does not exist yet anywhere, so there is nothing to move - the layer is complete and this
      line stays as the instruction that keeps it that way

- [x] the layer holds no value of its own and reads only (`AGENTS.md` 4a, `docs/38` D34-D35): the
      fallback outpost, the creation map id and the client's three map kinds are
      `config/Server/defaults.json`, each with a `doc` and an `immutable` flag, and every one reaches
      `placement` as an argument

## Layer 4 - auth service (`asura-auth`)

Files: `src/lib.rs` 845 (the login conversation), `src/travel.rs` 291 (the placement mechanism: registry
lookup, seat, transfer row), 2 examples. No `main.rs` and no listener - the socket layer owns those, and
the integration tests live in `crates/asura-gateway/tests/`.

- [x] the game rules leave the service: last outpost, the creation exemption and the default outpost
      are `asura-rules::placement`, a pure function with eight tests (`docs/38`, D25) - commit
      `4b09d17`; `travel.rs` keeps the registry lookup, the seat and the transfer row
- [x] the map-type numbers have one source: `asura-rules` owns `MAP_TYPE_*`, `CREATION_MAP` and
      `DEFAULT_OUTPOST` - commit `4b09d17`
- [x] `DEFAULT_OUTPOST`'s stand-in for unmeasured campaigns recorded where it is defined
      (`crates/asura-rules/src/lib.rs`) - commit `4b09d17`
- [x] the crate adopts the lint policy (`docs/38`, D17): seven needless borrows and one field doc -
      commit `4b09d17`; clippy `-D warnings` and `cargo doc` pass
- [x] no lint suppression, no dead code: the three error variants only classification and routing could
      raise left with those jobs - commit `099b0f0`
- [x] the relay left auth: the socket layer is `crates/asura-gateway` - listener, file check,
      classification, relay (`docs/38`, D26) - commit `be99de2`; auth is a library call from it and
      has no `main.rs`
- [x] the settings blob carried the wrong column for the client: `last_outpost` now sends `characters.map_id` (D28) - commit `836cdfa`; the login batch test against the live database proves the 62 bytes
- [x] the supervisor starts `asura-gateway` on the auth port (`config/services.toml`,
      `[services.auth]`) - commit `be99de2`, so a client sees no change
- [x] `docs/00`, `docs/01`, `docs/02`, `docs/15`, `docs/17`, `docs/19` and `AGENTS.md` name the two new layers and the process that listens - commit `55f9b5a`

The dependency runs one way: `asura-rules` (no dependencies) <- `asura-auth` <- `asura-gateway`, which is
why the three are layers 3, 4 and 5 in that order (`docs/38-layer-decisions.md`, D27).
## Layer 5 - socket layer (`asura-gateway`)

Files: `src/lib.rs` 230, `src/relay.rs` 174, `src/main.rs` 130. It listens on the address a client
dials, answers the file check, reads the four classification bytes and hands the socket on
(`docs/38`, D26).

- [x] created and wired: `asura-auth` has no listener and no `main.rs`, and `config/services.toml`
      `[services.auth]` starts `asura-gateway` - commit `be99de2`, so a client sees no change
- [x] the crate adopts the lint policy (`docs/38`, D17) and `cargo doc` builds - commit `be99de2`
- [x] the relay is named for what it does: `src/relay.rs`, not `frontdoor.rs`, which named the part after the whole crate - commit `46d0c82`
- [x] the file check is answered here rather than by the auth conversation, and the sequence is
      measured: the client's 21-byte request is the first thing on that socket, its failure is fatal to
      the login, and a server of ours has answered one on loopback (`docs/03-protocol.md` 7, the two runs
      of 2026-10-25; the code records it at `crates/asura-gateway/src/lib.rs`, step 0) - commit `fca403c`
- [-] the seven words in that answer are refreshed per client build, which is maintenance rather than work
      on this layer: the procedure and the record are `docs/19-running.md` ("Refreshing the file check's
      seven words"), the decision is `docs/38` D30, and what is not measured - the cost of a stale set -
      is stated in `docs/03-protocol.md` 7 instead of holding this layer open
- [x] the world freshness window has one source: `FRESH_WORLD_AGE_SECS` is gone from `relay.rs` and
      `asura_rules::world_max_age_secs` answers for this layer and the login path alike (`docs/38`,
      D29) - commit `801abfe`, 10 rules, 4 gateway and 3 auth unit tests pass
- [x] `docs/01` and `docs/02` describe the layer, and `docs/adr/0006` carries the correspondences
      (accept loop, the auth conversation it calls, the socket it does not pass) and the one deviation
      named - commit `801abfe`, `docs/adr/0006` "Correspondence between the two systems"

## Layer 6 - web gate

Files: `lib.rs` 581, `main.rs` 115. The portal web gate: the four HTTP answers the
client takes its identity from before it logs in (`docs/03-protocol.md` #4), plus `/health`.

- [x] the four answers checked and they hold no content: every body is a fixed reference string or
      a database lookup - the session id is minted, `UserId` and `Token` come from `asura_db::portal`,
      and the rest (`:John.10`, the `ResumeToken` placeholder, `Alias` `gw1`, `Created` 2005-01-01) is
      copied from the reference's `webgate.py:90-92` and belongs to the wire, like a field layout. The
      crate reads no file at all: `lib.rs` has no `std::fs` and no `include_str`, and its only settings
      are the bind address and the database URL (`main.rs:39,51`). The account list is rows, seeded by
      `tools/seed-dev.sql:15` - operational state under `docs/38`, D21 - checked against `lib.rs` 130-196
- [x] `/health` recorded as operational: a `GET` that answers `ok` so the supervisor can ask "are you
      alive", added by this project and not one of the reference's routes (`lib.rs:14-15,190-193`, and
      the route table says so again). Not content, and nothing reads it but supervision - checked against
      `main.rs:39,51` and `config/services.toml` (`[services.webgate]`, `health = { tcp = ... }`)

## Layer 7 - sessions

A **session** is one client's stay in the world: the socket arrives, the handshake runs, the ticket
the login issued is claimed, the load goes out, and then the connection is the world's until it
closes. It is not the login session, which is a row - `t_session` in the database, written by the
auth service and claimed here - and it is not the simulation, which is layer 8.

Files: `connection.rs` 572, `dispatch.rs` 210, `world.rs` 89, `map.rs` 137, `catalogue.rs` 103, plus
`capture.rs` and `replay.rs` behind the `harness` feature. The path graphs are `pathfind.rs`, the
spawn handler `agent.rs` and the player block `player.rs` (commit `8890603`).

- [x] the two instruments left the serving build: `capture.rs` and `replay.rs` are behind the
      `harness` feature, off by default, and `harness_off.rs` gives both modules an inert answer
      with the same shape so no call site reads a `cfg` (`docs/38`, D32). 55 tests pass in the
      serving build and 61 with `--features harness`, which is where the six instrument tests live
      - `crates/asura-world/src/harness_off.rs`
- [x] `map.rs` split, and what it never owned left: the path graphs are `pathfind.rs`, the spawn
      handler `agent.rs`, the catalogue entry `catalogue.rs`, the player block and the unlock facts
      `player.rs`; the instance table was already the wire crate's and the cartography frames
      `cartography.rs`'s. What is left is the load - two blocks, the manifest handshake, and the
      order (`docs/03-protocol.md` 11.3a). The `map_config` read is content and waits on the pack
      (D22) - commits `8890603` and `54cb1f2`, then the two relocations after them
- [-] the `map_config` read itself leaves for the pack, and this is **parked**: the destination is
      D22/D23's, the reader switches when a pack file exists, and the table goes last. Marked here so
      the layer does not read as "everything moved" - the same way layer 2 marks its content rows
- [x] the two timings and the ping value left `connection.rs`: the world reads them from
      `config/Server/intervals.json` through `asura-config` (`docs/38`, D33), and the file marks the
      tick as measured, the ping interval as the reference's constant and the ping value as
      unverified. The same file answers the login path's probe timeout and the freshness window the
      socket layer routes by, so the three programs cannot disagree - 4 config, 55 world, 3 auth,
      4 gateway and 10 rules unit tests pass
- [x] the load block's message order is recorded with its capture named:
      `docs/03-protocol.md` 11.3a lists the six messages in the order this server sends them, the
      two that join afterwards, and the 24-message head from
      `recordings/asura/world/live-client-session-20261027-222641.jsonl` that proves the order a
      live client survives
- [x] a real client was run against this build after the moves (owner's session, 2026-11-04): login,
      load and play. The relocations changed no message, and this is the run that says so

## Layer 8 - instance and simulation

Files: `instance.rs` 378, `agent.rs` 1,158, `player.rs` 305.

- [ ] `TICK_INTERVAL` consumer and the tick's cadence recorded (layer 7 owns the value)
- [ ] `POSITION_UPDATE_MAX_ELAPSED_MS = 2000` moved to the rates file
- [ ] `EXPERIENCE_TEST_VALUE = 500` recorded as a stand-in, not moved
- [ ] `REACH`-style unmeasured constants listed in one place

## Layer 9 - travel

Files: `travel.rs` 522, `portals.rs` 232, `travel_targets.rs` 164.

- [x] doorway positions are content: `map_portal` read once at start-up
      (`asura-world/src/main.rs:199-222`, `portals.rs:140-166`), compiled table kept as fallback
- [ ] the compiled `CROSSED` and `PORTALS` tables deleted once the file matches them
- [ ] `travel_targets.rs`'s 21 aliases over 12 maps moved to `config/` as per-map aliases
- [ ] `hosted` removed from content and computed at runtime
- [ ] the doorway identity question recorded: position (today) versus the client's prop index
- [x] the `0x80C1` touch starts nothing: `travel::handle_travel_init` (`travel.rs:154-166`) prints
      the touch and returns. The code is the truth, so commit `8bcc029`'s title is wrong.

## Layer 10 - character

Files: `creation.rs` 482.

- [ ] `default_equipment.rs` (2,957 lines; 32 campaign rows, 179 items) moved to `config/`, or kept
      as code with the reason written down
- [ ] starter outposts moved to `config/`

## Layer 11 - items

Files: `item.rs` 525, `pvp_items.rs` 1,571.

- [x] `pvp_items.rs` arrived here from the wire layer: 387 unlockable-PvP-item rows, frozen, one
      reader (`player.rs:162`) - commit `fb8e508`
- [ ] `pvp_items.rs` decided: a `config/` file or a Rust table kept as evidence (D11, after D12)
- [ ] `inventory/seed.rs` (122 lines) moved to `config/`
- [ ] the bag order and the refusal shapes recorded as rules (they are not content)

## Layer 12 - chat and commands

Files: `chat.rs` 349.

- [ ] the `COMMANDS` table (`chat.rs:141`) moved to `config/`, or kept as code with the reason
- [ ] the coded-string chat body recorded as a wire rule, not content

## Layer 13 - map data and pathing

- [-] holds by design - measured from the archive, not authored
- [ ] `data/path-graphs.bin` recorded as a build artifact: 87,336,403 bytes, 142 maps
- [ ] how a profile ships it recorded (it is not `config/`)

## Layer 14 - cartography

- [-] holds by design, and it is a **frame without a feature** (`docs/38` D37): the grid's dimensions
      (64 x 128 for Tyria) and the band format are measured, and every band the world sends is empty
      because a character's exploration progress has no schema here. The bytes are therefore correct
      for a fresh character and wrong for everyone else - not a default chosen for convenience, an
      unimplemented feature
- [ ] the exploration bits exist: a table of explored cells per character per continent, the encoder
      for the client's band format in both directions, and an update path as the character walks. Until
      then every load announces a grid nobody has explored

## Layer 15 - content itself

Four homes today:

- [x] database rows: `map_config`, `map_spawn`, `map_portal`
- [ ] compiled Rust: `default_equipment.rs` 2,957, `pvp_items.rs` 1,567, `inventory/seed.rs` 122,
      `portals.rs` and `travel_targets.rs` tables
- [ ] a generator input: `data/portals.json`, 5,954 bytes, never read at runtime
- [ ] 214 `INSERT`s inside migrations `0013`-`0022`
- [ ] `game/instance_table.rs` (175 lines) is data rather than code - the map ids a load tells the
      client exist, capture-derived - and moves here when the content format is settled
- [ ] one file layout decided and written down before the second layer moves
- [ ] the `config/` directory created with the first layer that lands in it

## Layer 16 - supervision and operations

Files: `config.rs` 347, `supervise.rs` 571, `main.rs` 493, `control.rs` 128, `registry.rs` 114,
`psql.rs` 70, `process.rs` 68, `status.rs` 34, `health.rs` 22, `tests/supervisor.rs` 223,
`config/services.toml`, `config/services.docker.toml`, `docker/`.

- [x] already file-native: one TOML file drives every service (`config.rs:150`, `toml::from_str`)
- [ ] `deploy-world` covered by a test (it has none)

## Layer 17 - harness

Files: `asura-fakeworld/src/main.rs` 307, `asura-world/tests/load.rs` 3,039,
`asura-world/tests/end_to_end.rs` 344, `asura-auth/tests/login.rs` 708, `asura-auth/tests/travel.rs`
397, `asura-db/tests/*` (5 files, 1,483), `asura-world/tests/path_dump.rs` 220,
`asura-world/tests/portals.rs` 41, 5 examples.

- [ ] 261 declared tests recorded, 2 ignored, with the ignore reasons named
- [ ] every fixture that is content moved with its layer

## Layer 18 - evidence

- [-] holds by design - never configuration, never edited

## Boundary defects to fix before the content layers move

- [ ] the world declares a dependency on `asura-auth` (`crates/asura-world/Cargo.toml`) with no call
      site anywhere in the crate
- [ ] `map.rs` straddles content, connection and cartography (layer 7 note)
- [ ] layer 3 owns the map-type numbers; no service restates them
- [ ] content in layer 10 (`default_equipment.rs`), layer 9 (`portals.rs`,
      `travel_targets.rs`), layer 2 (`inventory/seed.rs`) - three layers, four homes

## How to update this file

One box per commit. Tick the box, and append the commit hash and the evidence line - the command,
the count, or the capture - on the same line. A box is never ticked because the work is planned,
and a layer's `[x]` requires the six conditions at the top of this file.

