# 20 - Rurik: the feature gap, and the port queue

This is an analysis, not a change. Nothing in this document is built yet. It answers one question:
**which capabilities does the Rurik checkout have that this tree does not, or handles worse, and in what
order should they be ported here.**

Governance is already settled: Rurik is a recorded source (`docs/adr/0011-rurik-is-a-source.md`). That
ADR decides five things which shape every row below, and none of them is re-opened here:

1. Rurik is read, trusted where it cites an instruction address or a capture, and its findings are
   restated in this project's own words with this project's own evidence.
2. It is never a dependency. Nothing in `Cargo.toml`, nothing under `crates/`, no test requires the
   checkout. `cargo build --workspace` succeeds on a machine without it, and the same holds for Python:
   **no Python crosses into this tree**. Server mechanics are implemented in Rust; skill behavior
   is declared in external Lua scripts, as clarified in `docs/35-skill-mechanics-and-behavior.md`.
3. Its vocabulary stays out. A module is named after the game's noun; no file under `crates/` describes
   itself as coming from Rurik.
4. Every fact taken from it is restated with our evidence: a recording under `recordings/`, a crash log,
   a test. Where Rurik is the only witness, the sentence says that.
5. A conflict resolves in favour of the client in the owner's hands.

## The client link: Py4GW Stealth (`docs/adr/0013-py4gw-stealth-is-our-link-to-the-client.md`)

Before any row below is read, one capability is not Rurik's and is not in doubt: **Py4GW Stealth is this
project's link to the client.** It injects a 4,736-byte bridge into a running `Gw.exe`, detours
`game_thread.leave_game_thread_func`, keeps a client-to-Python event ring, and resolves a catalogue of
28 subsystem offset files by byte pattern and by assert anchors. Two entries in that catalogue decide
the comparison in this document:

- **`gw_dat_reader.json`** reads the archive through the client's own open-file, image and `AMET` code.
  The client's parser answers, not ours.
- **`stoc.json`, `ctos.json`, `packet_sniffer.json`** resolve the server-to-client handler table pointer
  and the client-to-server send wrapper. That is the sniffer.

So **every row here that says "read this out of the running client" is answered by Py4GW Stealth, not by
Rurik**, and those rows are struck from the queue: the skill, attribute and area tables, string-to-text
resolution, the item modifier word, skill template strings, a chunk's contents read live, and packet
observation through the client's own tables. Rurik's versions of those are a cross-check at best. P14
keeps only the half a server needs with no client anywhere, and P7 only what a server must compose.

What remains is the narrow set that needs the client **closed**, which the catalogue does not touch: a
grep of all 28 offset files for a message format or field table returns nothing. Those four survive in
P2 below.

## Scope ruling: world simulation is a later stage

The owner's direction, recorded here because it decides what this document is for: **all world
simulation belongs to a later stage, and Rurik's simulation work is the source for that stage when it is
scheduled.** It is not the source for now.

`docs/adr/0003-simulation-in-scope.md` defines simulation as agents, movement, pathfinding, combat and
drops. Against that definition these rows are **deferred**. They stay listed so that nothing found in the
Rurik checkout is lost, and they are marked `DEFERRED` at their headings:

| row | what it is |
|---|---|
| P8 | non-player agents |
| P9 | the combat core |
| P10 | the skill substrate |
| P11 | party: henchmen and heroes |
| P12 | quests |
| P13 | progression persistence: attributes, ranks, unlocks, morale |

Every other row is not simulation - measuring tools, the archive, content authoring, map authoring, the
harness, the build discipline - and **that is the live queue**.

Movement and pathfinding are on ADR 0003's simulation list and are already built here
(`crates/asura-world/src/agent.rs`, `pathfind.rs`), so P3 measures what exists rather than porting
anything into it.

## The size of the two trees

| | Rurik | ASURA |
|---|---|---|
| language | Python 3, stdlib only | Rust |
| code | 732 `.py`, ~390,000 lines | 88 `.rs`, 38,585 lines (measured 2026-11-01) |
| research documents | 219 `.md`, 16.5 MB (`studies/`, 71 arcs) | 33 pages under `docs/`, 14 ADRs |
| server | one process: `toolkit/authsrv/authsrv.py`, 50,110 lines | 7 crates, tokio, PostgreSQL |
| world content | 11 declarative `.toml` tables, 480 KB | SQL migrations, `crates/asura-db/migrations/` |
| schema | 777 messages (`schema/messages.json`: AUTH_CMSG 57, AUTH_SMSG 39, GAME_CMSG 194, GAME_SMSG 487) | 782 rows, `crates/asura-protocol/src/packets.rs` (auth 60 + 40, game 194 + 488) |

The two schemas are the same catalogue at the same coverage, and that is the first finding: **the
message schema is not a gap**. Both descend from the same public table, Rurik validated its copy
field-for-field against build 38797 and recorded zero residual disagreement, and this tree's 782 rows
carry the same layouts. A port of the schema buys nothing; the gaps are elsewhere.

## What is already equal, and must not be ported

Reflex is the enemy here, so the equal list comes first. These are places the two trees do the same
thing and this one is not behind:

| area | where |
|---|---|
| auth handshake, ARC4, fixed-constant DH, file check, login burst | `crates/asura-protocol/src/client/handshake.rs`, `client/`, answered by `crates/asura-auth`, proven over a socket by `crates/asura-auth/tests/login.rs` |
| travel, placement, atomic reservation, leases, reaper | `crates/asura-db/src/placement.rs`, `crates/asura-auth/src/travel.rs` |
| instance load block, spawn point, spawn block, manifest | `crates/asura-world/src/map.rs`, `tests/load.rs` |
| path graph import and A\* over the trapezoid graph | `crates/asura-world/src/map_data.rs`, `pathfind.rs`, with the reference's own reciprocal/square-root tables |
| movement budget clamp, plane-change rules, cancel correction | `crates/asura-world/src/agent.rs` |
| the exploration grid (`0x008B` + `0x008A`, RLE bands, empty-band shape) | `crates/asura-world/src/cartography.rs` - and Rurik's `content/fog.toml` is what settled its position in the load block |
| chat channel split (`!`/`#`/`$`), `/stuck`, `/maps`, no server-generated chat on the wire | `crates/asura-world/src/chat.rs`, `docs/07-status.md` |
| inventory as rows, drop with the stack deducted, move, equip, position round trip | `crates/asura-world/src/item.rs`, `crates/asura-db/src/inventory.rs` |
| multi-player maps: introduction, withdrawal, broadcast | `crates/asura-world/src/instance.rs` (`PeerView`, `introduction`), `crates/asura-world/src/world.rs:166` |
| Gw.dat index and the FaCompress decompressor | `crates/asura-world/src/gwdat.rs`, `fa_compress.rs` |

Two of these are places this tree is ahead and the port must not regress: the crate split (Rurik's
server is one 50,110-line module) and the database (Rurik keeps characters in one JSON file per account,
`toolkit/authsrv/charstore.py`).

## The four kinds of gap

Every row of the queue is one of these. The distinction matters because they cost different things and
carry different risk:

- **B - behaviour gap.** Rurik sends or answers a message this tree does not. Changes what a client
  sees, needs a client run to accept.
- **I - instrument gap.** Rurik can measure something this tree cannot. Changes no behaviour, so it is
  safe to land unattended, and it shortens every later step.
- **A - authoring gap.** Rurik can write an artifact this tree can only read. Largest capability, least
  bearing on the packet stream.
- **H - handling gap.** Both trees do it; Rurik's arrangement is better and worth replacing ours.

## The port queue

Ordered by dependency and by value against the objective in `AGENTS.md` #1 (reproduce the packet stream,
name every packet), not by size. Each row gives what Rurik has, what this tree has, what a port means
here, what must not come along, and what accepts it.

### Phase 1 - instruments. No behaviour change, no client risk.

**P1 (I) - The opcode sweep: measure every server-to-client message the live server never showed us.**
Rurik: `toolkit/authsrv/smsgsweep.py`, `sweeploop.py`, `shotloop.py`, `shotlabel.py`; result recorded in
`studies/smsgsweep/FINDINGS.md` - 334 of 487 GAME_SMSG opcodes measured against a controlled client,
**82 ASSERTED, 3 FAULTED, 1 DROPPED_CHANNEL, 3 REPLIED, 235 SILENT**, every crash row paired with the
dialog its own run captured.
Here: nothing. `docs/07-status.md` reports 32 of the reference's client-message handlers present and
says nothing about the 488 server-to-client rows this tree has never sent.
A port: a tool under `tools/` that sends one catalogued opcode to a client bound to loopback, reads what
comes back, and records a verdict per opcode; the verdict table committed as a recording, not as prose.
**The two halves are different instruments**, and only one of them is ours to write: the *observation*
half is the client link's sniffer (`stoc.json`/`ctos.json`/`packet_sniffer.json`, ADR 0013); the
*stimulus* half - put one unknown message on the wire and read the client's reaction - is this row.
Not to take: the screenshot half before the cheap half proves itself, and any opcode fired inside a
client's own load traffic.
Accepts it: one row per opcode in `packets.rs` moves off `Known` with a recording named as its evidence.

**P2 (I) - The four questions that need the client closed.**
Rurik: `toolkit/clientscan/` - `msgshape.py` (a message's wire shape decoded from the client's own format
tables), `asserts.py` (the client's assertion strings read as a catalogue with file and line),
`pinned.py` and `buildid.py` (which build a byte pattern belongs to), `msghandler.py` (disassembles the
client's handler for one message), and `sigcorpus.py` (whether a byte-shape anchor still resolves).
Here: nothing.
Why these four and not the other six: **Py4GW Stealth answers everything that needs the client running**
(see "The client link" above). A message's wire shape, the assert table as a list, the build a pattern
was measured against, and one handler's disassembly are the questions the offsets catalogue does not ask
and cannot: grep all 28 of its files for a format or field table and nothing comes back.
A port: four readers behind `tools/`, each printing bytes rather than claims. `msgshape` and `asserts`
pay first - one gives a layout without a capture, the other is how twenty opcodes got named here.
Not to take: `capstone` or `pefile` as a build dependency of anything under `crates/` - a reader lives in
`tools/` and fails closed when the client is absent (`AGENTS.md` #6).
Accepts it: a number this tree currently guesses is printed by a tool from the owner's own `Gw.exe`, and
the guess is corrected or confirmed in the same change.

**P3 (I) - Movement-fidelity instruments.**
Rurik: the arc's instruments are `toolkit/clientscan/movesync.py` (separation between the client's own
position and the agent the server drives, with a two-armed hard bar), `movetap.py` (the client's live
movement solution), `noclipscore.py` (off-mesh walking and hole-crossing chords), `planecensus.py`,
`warpscan.py` (did the run test the teleport, and did it warp), `grantsim.py`/`resyncscore.py` (would a
different grant policy have snapped), and on the server side `toolkit/authsrv/leadgeom.py`,
`agtrack_mirror.py`, `speedwords.py`, `cancelwalk.py`, `castpolicy.py`.
Here: four scripts - `tools/analyze-live-move.py`, `analyze-live-move-math.py`, `live-client-speed.py`,
`window-live-move.py`.
Why this one is high: movement is where this tree's own incident ledger failed, six times
(`docs/HANDOFF.md`, rows 2 to 6). Every one of those was a change made without an instrument that could
say "the client and the server now disagree by this many units".
A port: `movesync`'s separation score and `warpscan`'s did-it-actually-warp verdict, over the recordings
already committed.
Not to take: Rurik's conclusions about the snap gate as *our* rule. Its finding is that no gate of the
client's test is a server lever; that is a fact to cite, not a design to adopt.
Accepts it: the score printed against an existing recording, agreeing with what that recording's own
trace says.

**P4 (I) - The tape player.**
Rurik: `toolkit/authsrv/tape.py` plus `studies/tape/FINDINGS.md`; R1.5 played 1,209 of 1,209 events of a
48.6 s recorded stream at recorded timing and drove a real client through Ascalon City, and a second run
rendered combat.
Here: `crates/asura-world/src/replay.rs` serves the reference's captured bytes per block - a bisection
aid, not a timed replay.
A port: replay a committed `recordings/` stream at recorded timing with zero messages of our own on the
channel. Rurik's own first run failed because 373 of its own `WORLD_SIMULATION_TICK`s were interleaved,
which is the defect to design out first.
Accepts it: a recording replays end to end with a measured count of non-tape sends, and that count is 0.

### Phase 2 - items, which is where this tree's next session already starts

`docs/HANDOFF.md` "Where the next session starts" item 3 lists five item behaviours as the next work.
Three of them have a Rurik source.

**P5 (B) - Pick a dropped item back up.**
Rurik: `toolkit/authsrv/loot.py` (ground drops and the pickup, `RANGERPRE-S15`) and `purse.py` (carried
gold).
Here: `docs/HANDOFF.md` records the decision "a dropped item becomes litter", taken because the reference
has no pick-up handler.
A port: `GAME_CMSG_INTERACT_ITEM` (`0x803F`) decoded and answered, so a drop is reversible.
Accepts it: drop an item, walk to it, pick it up, relog, and the item is still in the bag.

**P6 (B) - Buy and sell.**
Rurik: `toolkit/authsrv/merchant.py` (the purchase recipe and the gold/inventory messages it produces),
`toolkit/authsrv/probemerchant.py` (merchant window, stock declaration, prices, the accumulator drains).
Here: nothing; `GAME_CMSG_TRANSACT_ITEMS` (`0x804D`) is in the table and unhandled.
A port: the merchant window, the stock rows, and the gold/inventory pair a purchase produces.
Accepts it: a buy and a sell over a socket, with gold and the bag both correct afterwards.

**P7 (H) - Item modifiers and weapon sets.**
Rurik: `toolkit/clientscan/itemmods.py` decoded the client's item modifier word -
`{identifier: bits 29-20, arg: bits 17-8, arg2: bits 7-0}` over 157 identifier slots, validated on
5,266 of 5,266 real words - and a composed word was rendered by the retail client as
`Hammer Mastery +1 (Stacking)`. Also `visible in toolkit/authsrv/authsrv.py`: `configure_weapon_sets`,
`declare_weapon_sets`, `select_weapon_set`, `worn_armour`.
Here: items are rows (`asura_db::inventory::Item`) whose stats are copied from the reference's capture.
Handling, not a hole: the rows work, and the block is byte-equal to the capture.
**Narrowed by ADR 0013:** the word's *layout* is answered live by the client link, and Py4GW's own tree
ships a modifier parser (`Py4GWCoreLib/item_mods_src/item_modifier_parser.py`), so the decode is not
something to port. What stays is what a *server* must do with it: compose a word, send it, and see the
client render it.
A port: composing the modifier word rather than copying one, plus the four weapon-set messages.
Not to take: the 157 identifier meanings. Rurik records that bits 31, 30 and 19 are a fixed
per-identifier prefix that must be copied, not computed, and that meaning is recovered per identifier.
Accepts it: a composed modifier word renders in a live client, and the rendering is read back through
the client link rather than through a screenshot.

### Phase 3 - DEFERRED. World simulation: bodies (P8 to P11).

`docs/07-status.md`: "There are no NPCs, no combat and no quests". This is the largest behaviour gap and
it is five ports deep.

**P8 (B) - DEFERRED. Non-player agents.**
Rurik: `toolkit/authsrv/agents.py` ("what the client must be told to render one"), `npcdefs.py` (a capture
compiled into NPC definition rows), `npcaftercast.py`, `content/npcs.toml` (504 lines), and the spawn rows
in `content/world.toml`.
Here: `world_create_agent` exists in `crates/asura-protocol/src/game/mod.rs` and is called for players
only.
A port: a spawn table that yields agents that are not the player, with model, allegiance, level and
weapon, and the create burst around them.
Accepts it: an agent that is not the player stands in a map, in a live client, and does not move.

**P9 (B) - DEFERRED. Combat core: an attack that lands, and a death.**
Rurik: `toolkit/authsrv/combatmath.py` (armour and the swing arithmetic), and in `authsrv.py` the
dispatch for `0x0026` `ATTACK_AGENT`, the kill window as three messages - death, reward, and the attack
reply - plus revive. Ladder row R4a in `PLAN.md` section 3 records it working on the wire and on film, 2026-08-11,
including the crash that a wrong revive fraction caused (`CharPool.cpp:84`, `fraction <= 1.0f`).
Here: nothing.
A port: `GAME_CMSG_ATTACK_AGENT` handled, a swing resolved against the armour arithmetic, a death and a
revive, and the target re-acquired.
Not to take: Rurik's admission that `ENEMY_SKILL_FRACTION = 0.25` was invented - the skill ladder records
that constant as removed, and nothing invented goes in here.
Accepts it: a live client clicks an agent, the server drives the fight to a kill, and the client revives
it.

**P10 (B) - DEFERRED. The skill substrate.**
Rurik: the ordinary recharging cast lifecycle - `0x00E4 -> 0x00E5 -> 0x00E3 -> 0x00E6` -
and the **queue law**: a press during aftercast schedules from the aftercast's end. Damage is the skill's
scale endpoints interpolated by the client's own formula (`0x005A8920`:
`max(0, round(lo + (hi-lo)*rank/15.0))`). Hexes and enchantments came off the missing list 2026-08-20 via
`toolkit/authsrv/effects.py` (`0x0042` opens an episode, `0x0044` closes it), `hexjoin.py`, `bufflog.py`.
Energy and adrenaline are `toolkit/authsrv/pools.py`, `adrenjoin.py`, `adrenreplay.py`. Conditions from
`studies/isle/FINDINGS.md` (478 Bleeding .. 486 Weakness, +2077).
Here: `skillbar_update` and `update_unlocked_skills` exist as builders; no cast is answered.
A port: the evidenced cast lifecycle including its exceptions, the queue law, the scaling formula,
then conditions, then the effect episode pair. The pinned source review in
`docs/35-skill-mechanics-and-behavior.md` records zero-recharge exceptions, the queue divergence,
and the boundary between mechanics and scripted skill behavior.
Not to take: `type_code` as a discriminator for what a magnitude means - Rurik records that it cannot
discriminate, and that an unmodelled skill resolves to None rather than 0.
Accepts it: a skill on the bar casts, lands on a target, and a second press inside the aftercast is
queued rather than dropped - each measured, not asserted.

**P11 (B) - DEFERRED. Party: henchmen and heroes.**
Rurik: `henchparty.py` (`0x01BF` PARTY_HENCHMAN_ADD `[u16,u16,string16(20),u8,u8]`, `0x01C2`
PARTY_HERO_ADD), `henchjoin.py`, `herolib.py` (a hero's usable library and what may go in a slot),
`heroqueue.py`, and `heroes_table.py`. The slice ladder (R-SLICE, landed 2026-09-13) has a Monk hero that
walks the formation, fights the leader's target, obeys stance/lock/flag, and crosses a transfer.
Here: `party_create`, `party_player_add`, `party_member_stream_end` - one player, one party.
A port: the two party-add messages, a hero row with its own body, bar and ranks, and formation walking.
Accepts it: a hero walks behind the player across a map transition and fights the player's target.

### Phase 4 - DEFERRED (P12 and P13), live (P14): the world beyond bodies

**P12 (B) - DEFERRED. Quests.**
Rurik: `toolkit/authsrv/questdefs.py` (quest rows out of `content/`, and the coded string a field goes on
as), `content/quests.toml`, `quests.toml`'s schema mirrored in `toolkit/authsrv/authsrv.py`
(`accept_quest`, `turn_in_quest`, `grant_quest_reward`, `complete_objective`,
`kill_completes_objective`, `_quest_markers`, `_quest_screen`, `_dialog_window`), `probequest.py`,
`studies/quests/`.
Here: nothing.
A port: the dialog window, accept, objectives, completion, reward, and the marker messages.
Not to take: quest prose. Rurik's own provenance scoping
(`PLAN.md` section 7 Q17) distinguishes an id resolved at run time from an authored body of text, and authored
text is the one thing neither tree may copy.
Accepts it: take a quest from a stationary NPC, complete its objective by killing an agent, turn it in,
and the reward arrives.

**P13 (B/H) - DEFERRED. Progression persistence.**
Rurik: `charstore.py`, `summarycensus.py` (`charsummary.py` for the summary blob), `attribspend.py` (the
rank and point model and the client's own refusal rules), `attribpoints.py`, `skillunlock.py` (two
unlocked-skill bitmaps, on two scopes), `morale.py` (`0x00EE` attr 10 and `0x009C`).
Here: gold, level, both professions and the campaign are columns now (`0008_character_traits.sql`,
`0009_character_settings.sql`), and the instance unlocks are `instance_unlock` rows
(`0023_instance_unlocks.sql`). Attributes, ranks and the unlocked-skill bitmaps are not stored.
Rurik's storage is the one thing not to take: one JSON file per account is a design this tree replaced
with PostgreSQL rows.
A port: attributes and ranks as columns with the reference's point model, and the unlocked-skill bitmaps.
Accepts it: an attribute is spent in a live client, the ranks are right, and a relog keeps them.

**P14 (A) - The archive's other chunks: only the half a server needs with no client.**
Rurik: `toolkit/mapdata/terrain.py` (the Bloated terrain chunk, read and write, 349 of 349 maps),
`strippedterrain.py`, `pathchunk.py`, `props.py`, `envchunk.py`, `soundchunk.py`, `trnblend.py`,
`trnvariation.py`, `trnshadow.py`, `fogrle.py` (the fog-of-war RLE, decode and build), and
`toolkit/clientscan/worldmap.py` (the atlas: which archive file holds each 512x512 continent tile) plus
`tilerender.py`.
Here: `map_data.rs` reads exactly one chunk - `ChunkId::Path` (`map_data.rs:90`). Terrain, zones, props,
water, mission, environment, locations, light and shore are enumerated in the `ChunkId` enum at
`map_data.rs:69` and never read.
**Narrowed by ADR 0013:** what a chunk *contains* is now a question for the client link, which reads the
archive through the client's own open-file, image and `AMET` code (`gw_dat_reader.json`). Rurik's
readers are the cross-check. What stays is the half that must work with no client anywhere: a world
server reads geometry at start-up, and a `Gw.dat` we author must be accepted by the client's parser.
A port: the terrain height field and the props list offline, then the fog RLE, then the atlas.
Accepts it: an authored or retail map's own height at a chosen position, read offline, agrees with what
the client reports for ground Z through the client link.

### Phase 5 - authoring. The largest capability, and the least about the packet stream.

**P15 (A) - Declarative content with per-row provenance.**
Rurik: `toolkit/content.py` (559 lines) loading `content/*.toml`: `maps.toml` 840 lines, `npcs.toml` 504,
`items.toml` 933, `areas.toml` 1,130, `world.toml` 3,790, `fog.toml`, `quests.toml`, `drops.toml`,
`partycap.toml`, `movecode.toml`, `compose.toml`. Every row carries its own `provenance` table, and the
loader **refuses** a `source = "client-table"` row that does not name its extractor and its build.
Here: SQL migrations (`crates/asura-db/migrations/`, 24 files) plus two seed files the supervisor applies
(`tools/map-config.sql`, `tools/seed-dev.sql`); content literals are kept as data, and the reference's own
map list is generated into `map_config`.
Handling, not a hole: migrations are versioned, reversible and tested, which a TOML file is not.
A port: **the provenance column and the loader's refusal, not the file format.** A content row that
cannot name where it came from is the defect this tree's incident ledger is full of.
Accepts it: a content row added without a source is refused at startup, with the refusal tested.

**P16 (A) - Gw.dat write, reversibly.**
Rurik: `datplan.py` (compute every byte that would change; change none), `datwrite.py`, `datdelta.py`,
`datjournal.py` (append-only record of what the writer did), `datsnapshot.py`, `datcheck.py`,
`rowbytes.py`, `refindex.py` (who else reads this id, asked before a write), `datalloc.py`,
`datmove.py`, `overlay.py` and `overlayrefgate.py` (declared, reversible, identifiable-from-bytes archive
profiles), `compose.py`, and the compression **encoder** `gwenc.py`/`gwmatch.py`/`gwentropy.py`.
Here: `gwdat.rs` is the index half by its own doc comment, and `fa_compress.rs` is the decompressor.
Nothing writes.
A port: plan-diff-apply with a journal, then the encoder, then the overlay profile.
Not to take: any writer that cannot be reversed from its own journal. Rurik's `datwrite` refuses on
purpose for a move that does not fit, and `datmove` is the separate verb that does it.
Accepts it: an entry is replaced and then restored byte-identically, with the restoration proven by hash
and not by intent.

**P17 (A) - Author a map the client will load.**
Rurik: `mapbuild.py` (a whole Bloated map from typed parameters - 7,841 bytes, 9 chunks, 97.04%
generated), `stripbuild.py`, `mapgen.py`, `deploy.py` (one command from `content/` to a map you can walk
around), `deployrefuse.py`; and `mapscale.py` for how big an authored area may be.
Here: none. `map_config` rows can name a map; nothing can make one.
A port: last. It is the only row here whose value is a mod kit rather than the live packet stream, and
it depends on P16 and on P14's chunk readers existing first.
Accepts it: an authored map loads in a live client, and geometry we chose confines the character.

### Phase 6 - discipline. Cheap, and it is what keeps the rest honest.

**P18 (H) - A harness run that judges itself from the client's own messages.**
Rurik: `toolkit/harness/session.py` - one command that pre-flights the ports, starts the stack, proves
each service owns its port, launches the client, drives login through EULA to Play, and prints one
`[PASS]`/`[FAIL]` per checkpoint; plus `portclaim.py`, `read_error_dialog.py` (the full text out of the
client's fatal-error dialog), `runwatch.py`, `progress.py`, `runladder.py`.
Here: `tools/test-battery.ps1`, `tools/run-client-login.ps1`, `ASURA_TRACE=1`.
A port: the checkpoint ladder and the dialog reader. The dialog reader is the single highest-value item
in this phase: three messages that no capture contains have already crashed a live client
(`docs/HANDOFF.md`), and today the crash text is read by hand.
Accepts it: one command prints a per-checkpoint verdict for a login, and a client crash produces the
assertion text without a human looking at the screen.

**P19 (H) - A run that measured nothing fails.**
Rurik: `toolkit/checks.py` (every verdict routes through one ledger and declares a `floor`; fewer checks
than the floor is a FAIL naming the shortfall) and `run_suite.py` (discovers tests from disk, never from a
document). It exists because `test_codec.py` printed ALL CHECKS PASSED with its fixture glob matching
nothing.
Here: `tools/test-touched.ps1 -WholeSuite -Justification "..."` for scope, and no floor mechanism.
A port: the floor, applied to the standing suites.
Accepts it: deleting a fixture makes the suite red with a named shortfall, not green.

**P20 (H) - Client-build discipline.**
Rurik: `toolkit/clientpatch/dhbuild.py` reads the DH struct and answers `ours`/`stock`/`unknown`, and
`cage.py`'s `assert_launch_safe(exe, host)` refuses a launch whose parameters do not match the server it
points at. It exists because two tools once selected a build by filename and picked the wrong one.
`snapshot_client.py` and `updatecheck.py` handle the fact that the client's DH parameters rotate with
every ArenaNet build, and `buildpins.py` pins which build every byte pattern was measured against.
Here: `client-patcher/patch-gw.py` and `keys.txt`. No classification, no launch gate, no build pin on the
tooling.
A port: the classification and the gate. Both are small and both prevent a whole class of wasted session.
Not to take: the live-capture policy. Rurik's authorization to run automation against ArenaNet on a
secondary account (`CLAUDE.md`, owner's decision 2026-08-06) is Rurik's owner's decision about Rurik's
account. It is not ours to inherit, and this tree's recordings stay what they are: captures of our own
server and of the reference.
Accepts it: pointing a stock-DH client at this server is refused before the process starts, with a named
reason.

## What must not be ported, and why

- **`authsrv.py`.** 50,110 lines in one module. The crate split is not the thing to give up.
- **Any Python.** `AGENTS.md` #6 and ADR 0011 #2. A port is a restatement.
- **Rurik's provenance gate as written.** Rurik refuses to hold any ArenaNet bytes, and its `.gitignore`
  rationale is built on that. This tree commits decrypted captures under `recordings/` as its evidence,
  and that is this tree's decision; adopting Rurik's gate would delete the evidence base. Read its
  *method* - measurement is permitted, expression is not, every row names its extractor and build - and
  keep our own rule for what may be committed.
- **Its monster-AI conclusions.** `PLAN.md` section 3's R4c row records AI as **not recoverable** from the client,
  the wire or any capture campaign: 0 of 937 embedded source paths under any `\Srv\` tree, 33 AI-adjacent
  searches over two independent routes, all zero. That is a finding to cite, not a design to port. Do not
  put AI on this queue until someone has a source neither tree has.
- **Its ladder as our ladder.** Rurik's status table is authoritative for Rurik. This tree's milestones
  live in `MILESTONES.md` and `docs/roadmap.md`.
- **Its vocabulary.** `crates/` keeps naming things after the game's nouns. `PathGraphs`, not Rurik's
  spelling of the same shape.

## What Rurik itself calls open, so a port must not promise it

- No AI, no pathing for non-player bodies, no resurrection shrine, no agent table (R4a and R4c rows).
- Conditions, energy, adrenaline and every non-damage mechanic were still listed as absent in R4b at the
  time its row was written; the effect substrate (hexes, enchantments) landed after that.
- Skill *meaning* is per-skill and sourced from outside the client's table; an unmodelled skill resolves
  to nothing.
- The vertical slice and the sandbox are outposts, quests and a corridor - not a campaign.

A port list that reads as "Rurik has everything" is wrong, and the rows above are the boundary.

## The branch taken, and the order, in one line each

- **Instruments before behaviour** (P1 to P4), because they carry no client risk, they land unattended, and
  every later port is accepted by a measurement that already exists.
- **Items before bodies** (P5 to P7), because `docs/HANDOFF.md` already names the item behaviours as the
  next work, and three of the five have a Rurik source.
- **Simulation is not in this queue** (P8 to P13). It is a later stage by the owner's direction, and the rows are kept only as a record of what Rurik holds for it.
- **Authoring last** (P15 to P17), because it is a mod kit, and the objective is the packet stream.
- **Discipline alongside** (P18 to P20), because the dialog reader and the floor are the two that stop a
  session being wasted.

## What this document does not decide

It does not change a single byte of this tree. Every row above still needs, before its first edit, the
one-sentence pre-flight `AGENTS.md` #2 requires: what changes, in which message or code path, and why it
is not simply copying the reference. Rurik is a source, not an instruction.
