# 10 - The parity work queue

> **History.** The tooling named below was retired when parity closed (docs/16); nothing here is runnable.

The whole surface, measured, in one place. `docs/09-parity-audit.md` answers "is behaviour X
equivalent?"; this document answers "**what still has to be built, in what order, and why**".

Two rules, both learned the hard way in this repo:

1. **A gap enters this queue only with evidence** - a capture, a dispatch table, a generated diff, or
   a line of the reference's own source. "I read it and it looked finished" is how three wrong claims
   got into `docs/09` (`/tp` "in no capture", state parity, the party messages).
2. **The reference's own tests are the acceptance criteria.** GWPS publishes T0-T15 in
   `PreSearing/docs/04-scope.md`. Where a test exists, closing a queue item means passing that test,
   not "feels done".

## The yardstick: GWPS's own test plan

The reference's sixteen items (`PreSearing/docs/04-scope.md`) are the closest thing either project has to a
definition of the observable surface, and the mapping from them to this repo's checks is now **generated**:
`docs/14-test-plan.md`, by `tools/parity-testplan.py`. It parses the items and their pass criteria out of the
reference's document, names the check that covers each one, and verifies that every named check still
exists -- a renamed test breaks the check rather than quietly rotting the page. Today: **12 measured, 3 hand,
1 blocked**.

What is left, in that page's own words: **T12** (the ground item) is blocked on a capture of a working drop,
which the reference cannot produce because its own handler is the unfinished one; **T9, T10 and T11** are
`hand` -- the server halves are asserted (a second agent, the reference's own path and refusals, the chat
pair) and the *seeing* of them needs a second real client. The table this section used to carry by hand went
stale three times; the generated page is what replaced it.

## The four axes, as measured today

| axis | how it is measured | current state |
|---|---|---|
| client -> server dispatch | `tools/parity-report.py` vs `GameSrv.c`/`AuthSrv.c` switches | **complete**: auth **15/15**, game **18/18**; the battery reports "nothing missing" |
| server -> client emit | `tools/parity-emit.py` | **complete for every message the reference sends**: 76 of 81 reachable, **0 real gaps**, 5 explained absences |
| state kept per player | `docs/09` #9, hand-checked against `GmPlayer.h:21-33` and `DbCharacterColumnsDef` | the four traits a message reads are stored (migration 0008) and the block sends them instead of constants; the rest of the reference's ~44 character columns are queue item **B5**, listed with the reason |
| schema | migrations vs `Db.c` queries | equivalent for what is used; no `friendships` table (the stream is empty anyway) |
| the map catalogue | `tools/gen-map-config.py` against the reference's three tables | **12 maps, the reference's own list**: aliases and `MapType` from its teleport table (`GmCommands.c:32-53`), numbers from `GmMaps.h`, file ids and spawns from `GmMapsConfig.c`. The seed used to carry two hand-typed maps |
| the load block | `tools/parity-load.py` against the reference's own `GWPS_TRACE=1` log, as a generated fixture (`crates/asura-world/src/load_fixture.rs`) | **measured: 480 messages, 0 missing, 0 extra, 0 size disagreements** - it matches message for message |

The emit tool counts a message as emitted only when a **send path** reaches the builder: a builder
nothing calls is a gap, and so is one only a unit test calls. That rule is why the four party/leave
builders (A4) still appear in the gap list - they exist, and nothing sends them yet.

## The queue

Ordered by dependency, not by size. Every row cites its evidence.

### A. Multi-player maps - the reason half the missing messages were missing

GWPS handles **N players per map**: it broadcasts a player's arrival to everyone, tells everyone when
a player leaves, and keeps a party per player. ASURA used to be single-player **by construction** -
`PLAYER_ID` and `PARTY_ID` constants, one agent per connection, `Agents` with one slot - and that is why
`PLAYER_DESTROY`, `WORLD_REMOVE_AGENT`, `PLAYER_UPDATE_PARTY`, `PLAYER_UPDATE_PARTY_SIZE` and the
`player_block`'s pair were missing (T9, T12 consistency, T14). Evidence for the old state:
`tools/parity-emit.py` output; `GmPlayer.c:314`, `GmAgent.c:60`,
`GmPlayer.c:322`, `GmPlayer.c:346`; capture `zone-ref-005829.jsonl` seq 254-259.

**A1-A5 landed.** Each connection now allocates its own player and agent id instead of using the
constants, a `Peers` registry holds everyone in the map with a channel per connection, a joining player
is introduced to the others and they to it in `GameSrv_SendWorldAgents`'s order, a leaving one is
withdrawn with the farewell pair, moves and position reports are broadcast, and the test
`a_second_player_is_introduced_to_the_first_and_withdrawn_again` covers all of it. `PLAYER_ID`/`AGENT_ID`
remain only as the ids the first player gets.

The four messages it needed turned out to be wrong in the **generated table**, not just missing: the
client's own table says 5, 6, 6 and 6 bytes for them while every live capture says 12, 12, 8 and 8,
because they carry a four-byte game header (`TYPE_MSG_HEADER_GAME`, `msgdefs.h:19-23`). The generator
now applies that per message, with the captures named in `OVERRIDES`.

| # | item | state |
|---|---|---|
| A1 | an agent table per map instead of `PLAYER_ID`; each connection owns its agent | done |
| A2 | a joining player is broadcast (`PLAYER_CREATE`, `WORLD_CREATE_AGENT`) to the others | done |
| A3 | a leaving player is withdrawn (`PLAYER_DESTROY`, `WORLD_REMOVE_AGENT`) | done |
| A4 | per-player party messages (`PLAYER_UPDATE_PARTY_SIZE`, `PLAYER_UPDATE_PARTY`) | done |
| A5 | movement of one player is broadcast to the rest | done |

### B. Character creation (T6)

Four handlers, and the instance they arrive in (`MapType_CharacterCreation`, 11). Evidence:
`GameSrv.c:1691-1976`, `GetBagSlotForItemType` (`GameSrv.c:1741`), `GmDefaultArmors.c` (2858 lines of
data), `CHAR_CREATION_START/SUCCESS/ERROR` in the emit diff.

| # | item | done when |
|---|---|---|
| B1 | host the character-creation instance (`MapType_CharacterCreation`) | a fresh account reaches the creation screen |
| B2 | `CHAR_CREATION_REQUEST_PLAYER` + `_REQUEST_ARMORS` (the player-data and PvP blocks) | no `Unhandled GameSrv packet` in the log |
| B3 | `_CHANGE_PROF` with the default-equipment table ported from `GmDefaultArmors.c` | picking a profession swaps the equipped items |
| B4 | `_CONFIRM`: write `characters`, `bags`, `items` rows | the same three `count(*)` that T6 prints |

**B landed, measured.** The *data* is ported and generated: `tools/gen-default-equipment.py`
reads `GmDefaultArmors.c` and writes `crates/asura-world/src/default_equipment.rs` - **32 (campaign,
profession) rows, 179 items**, the field set being the reference's own `GmItem` literal, which is also what
`CREATE_NAMED_ITEM` writes. `every_profession_starts_with_the_references_own_equipment` pins a Prophecies
Warrior's chest piece to the reference's bytes, and the battery re-generates with `--check`.

The *messages* are built too: `char_creation_start` (2 bytes), `char_creation_error` (6, header plus the
reference's `GM_ERROR_CHARACTER_NAME_ALREADY_EXIST` slot), `char_creation_success` (uuid + counted name +
the reference's own `0x2211` + counted settings) and `instance_player_data_done` (2), all sized by the
table and asserted in `the_fixed_size_messages_match_the_size_the_reference_sends`.

**B1-B4 landed.** The data is generated, the messages are built, and the flow itself is in: the world hosts
map type 11, all four handlers answer (`0x8089`, `0x808A`, `0x808B` and the profession change), and the
confirm writes the character's row, bags and item rows in one transaction before sending
`CHAR_CREATION_SUCCESS` with the settings block. What is *not* verified is the screen itself - a live client
walking the creation UI - which is why `docs/14-test-plan.md` records T6 as measured with the layout as the
rig's job.

### B5. The rest of the character row (~40 columns)

The reference's `DbCharacter` (`DbSchema.h:66-110`) stores about forty-four columns per character. This
server stored the ones a message read *when the column was added* - appearance, primary profession, unlocked
professions, gold (migration `0008_character_traits.sql`) - and now also the five the **character settings
blob** needs: `level`, `campaign`, `is_pvp`, `secondary_profession`, `helm_status` (migration
`0009_character_settings.sql`).

**What B5 actually was: one captured blob sent for everybody.** `AUTH_SMSG_CHARACTER_INFO` carries a 62-byte
`CharacterSettings` block per character - the appearance, level, campaign, professions, helmet status, last
outpost and five armour pieces with their dyes, which is what the character-select screen draws. The auth
service was sending a constant copied from a live reference reply: *the reference's seeded Warrior*, level 1,
last seen in outpost 146, wearing five pieces dyed the same colour. Every character in the list looked like
that one. Now the block is built from the row (`messages::character_settings`, `GmChar.c`'s packing), and the
armour pieces come from the equipped bag instead of the reference's denormalised `file_id_*`/`colors_*`
columns - same answer, one copy of it.

The measurement that the packing is right: `the_settings_block_built_from_a_row_is_the_references_own_for_that_character`
builds the block from that captured Warrior's fields and requires it to equal the reference's own bytes, all
62 of them. The auth test then requires the *seeded* character's block to differ from that constant and to
carry its own row's values, and the database test that `create_character` writes what `settings` reads back.

**Most of the remaining columns are still *not* work, and that was measured rather than assumed.** Checking
them against what the reference actually does split the list in two:

| column(s) | the reference's own behaviour | verdict |
|---|---|---|
| `skill1`..`skill8` | `GameSrv_SendSkillbarUpdate` sends them from the row, and `HandleCharCreationConfirm` builds that row with `= {0}` and never assigns a skill - nothing in GWPS writes one | zeros are parity |
| `unlocked_skills` | `GameSrv_SendUpdateUnlockedSkills` sends the bitmap, which is written as all-zero by the same confirm and assigned nowhere | an empty list is parity |
| skill/attribute points | `GameSrv_SendAgentAttributePoints` **hardcodes** unused 50 / used 50 | the constant is parity |
| the attribute spread | `GameSrv_SendPlayerAttributes` sends `data_len = Attribute_Count` and no values at all | the zeros are parity |
| `unlocked_maps`, the four completion bitmaps | stored, never read into a message that exists | no observable difference |
| `experience`, `skill_points`, `skill_points_total` | read by `GameSrv_SendPlayerFactions` (`0x00E9`) and written by nothing: no handler awards experience | the zeros this server sends are parity |
| faction points (twelve `balthazar`/`kurzick`/`luxon`/`imperial` columns) | read into `0x00E9` and the four `FACTION_MAX_*` messages | this server sends the fresh-account defaults (max 10000, everything else zero), which is what the reference's own columns hold for an account that has done nothing |
| `last_guild_hall_id` | read into the settings blob | zero: nothing here creates guilds, and the reference's own value for a lone character is the null uuid |
| `storage_gold` | `GameSrv.c:825` puts it in the storage-gold message | not sent: this server sends the character's gold, and an account-level storage pane needs storage bags that the reference never fills either |
| `eula_accepted` | read from the row, but `AuthSrv.c:684` sends a **hardcoded 24** to the client | unobservable: the value on the wire is a constant in the reference too |

One thing that *was* wrong surfaced through the same reading, and is fixed: the character-creation block sent
the **load** path's health and energy (25, 400 and a regen float) where the reference's creation path sends
its own three int properties - energy 20, health 100, property 64 zero (`GameSrv_SendPlayerHealthEnergy`,
`GmPlayer.c:178-199`). Asserted: the creation test checks those three values and the absence of a float
property.

**The schema axis is now generated rather than argued.** `tools/parity-schema.py` parses `DbSchema.h` into
its five tables and 95 columns and compares them with this database, printing `docs/12-schema-parity.md` and
failing on any reference column that is neither stored, nor derived, nor explained. The numbers today:
**22 stored, 11 mapped under a different name, 7 packed into `appearance`, 55 explained**. That is the check
which would have caught this round's gap on its own - a reference column with no counterpart and no reason
is a failure, not a paragraph.

### C. Teleport (`/tp`, `/teleport`) - measured, not blocked

The redirect is known: `TRANSFER_GAME_SERVER_INFO` (`0x01A6`, 46 bytes as GWPS sends it) then
`MAP_UPDATE_CURRENT` (`0x0099`, 12 bytes), both in `captures/zone-ref-005829.jsonl`. What is missing is
*authority*: only the client can start a travel today. Evidence: `docs/09` #4, `GameSrv.c:1360-1435`,
`docs/08-gm-commands.md`.

| # | item | done when |
|---|---|---|
| C1 | keep `account_id` (and the session) on the connection - state parity, `docs/09` #9 | the world can write a transfer row at all |
| C2 | a world-side `reserve` + `begin_transfer` for the target map | a transfer row appears with the right token and map |
| C3 | send `0x01A6` + `0x0099`; mark the connection *moved* so its teardown does not release the new lease | `/tp lakeside -> /tp ascalon` lands in Ascalon and the seat accounting is right |
| C4 | alias resolution and the reference's three refusals (unknown, no config, already here) | `/tp nonsense` prints the reference's line and sends nothing |

**C1-C4 landed.** `Play` carries the session and account, `/tp <alias>` (and `/teleport`) reserves the
destination's seat, mints a token, writes the transfer row and sends the pair; the connection is marked
*moved* so its teardown leaves the destination's seat alone. The test
`a_teleport_redirects_to_the_target_map_and_keeps_its_seat` drives the whole cycle: two refusals that send
nothing, the 46-byte redirect and 12-byte map update read by hand (the table's 5-byte form is the *login*
shape - the reference's "size per context" trap), the seat moving from one map to the other, a reconnect
with the handed-out token landing at the destination's own spawn point, and the old connection's exit
leaving the seat taken.

The one thing that is still this server's: it never constructs another world, so `/tp` reaches only the
maps it already hosts. The reference's auth service can place a player on any pre-spawned map server.

### D. Movement: validation, not just echo (T10)

ASURA echoes the client's destination, so the client's own prediction carries the walk and a wall is
not a wall. GWPS pathfinds (`GmPaths.c`, A* over the map's path graph from `Gw.dat`) and refuses what
it cannot reach. Evidence: `GmAgent.c:371-398`, `GmAgent.c:409-462`.

**D is sized, and two of its three units are now ported.** The reference's refusal is its pathfinder, and
the pathfinder is three units of C, all readable, plus the archive they read from:

| part | lines | what it is |
|---|---|---|
| `FaArchive.c` + `.h` | 204 + 72 | the `Gw.dat` archive: seek, read, MFT lookup by hash, and `FaDecompress` for compressed entries |
| `GmMapDataImport.c` + `.h` | 590 + 43 | the graph's chunk format: planes, trapezoids, vectors, root/X/Y/sink nodes, portal traps |
| `GmPaths.c` | 1,057 | trapezoid A*: `PathFindTrapezoid`, portal walking, waypoint reduction |

And the two things a port needs to be verified against both exist:

- **the data**: `Gw.dat` is present beside the reference at 3,655,804,520 bytes;
- **an oracle**: the reference ships built tools (`build/RelWithDebInfo/gwdat_pathing.exe`,
  `gwdat_scan.exe`) that dump the graph, so a ported reader can be compared message for message with the
  reference's own output rather than with a guess.

So `MOVE_TO_COORD` could not validate anything while those units were absent, and this was always a
**milestone, not an unknown**: the archive could be read and tested against `gwdat_pathing.exe`'s dump of
one entry before any A* was written - which is what D1 did, followed by D2 and D3 below.

What *can* be matched without it is matched, and asserted: the reference's position budget
(`speed_base * elapsed + tolerance`, clamped at 2 s of elapsed time) is the same rule on the same message, and
`a_move_is_echoed_as_sent_and_a_far_report_is_clamped` says so on the wire.

**D1 landed: the archive index.** `crates/asura-world/src/gwdat.rs` is the index half of `FaArchive.c` -
header, file table, TOC hash, entry lookup, and the stored bytes of one entry. No decompressor yet, so it
answers "is this file id in the archive and where", which is what the map catalogue depends on.

`gwdat-census` (an example) prints what the real archive says about itself:

```
header:  magic 0x1A4E4133, header_size 32, block_size 512, MFT at 3631642624 (3730656 bytes)
entries: 155444
  0x0001B97D  stored 1300044  compressed 8  decompressed 2925267  Lakeside County / Ascalon City / Ashford Abbey
  0x000345CC  stored  515444  compressed 8  decompressed 1734079  Kamadan, Jewel of Istan
```

Two independent checks come out of that. **2,925,267 bytes** for `0x1B97D` is the "2.9 MB region blob" the
reference's own roadmap describes; and every blob the generated catalogue names is present, which
`the_archive_index_finds_the_blobs_the_map_catalogue_names` asserts along with a bogus id being `None`.

A detail that had to be measured rather than read: `FILE_ARCHIVE_MAGIC` is the C multi-character literal
`'\x1ANA3'`, so the four bytes `1A 4E 41 33` collapse to `0x1A4E4133`, not to their byte-reversed reading -
the first attempt compared against the reversed value and failed against the real file.

**D2 landed: the decompressor.** `crates/asura-world/src/fa_compress.rs` is `FaCompress.c`'s decoder, ported
function for function - the MSB-first bit window (`ConsumeBits`/`GetBits`), `SetupNodesandTree`, `GetNextCode`
and `FaDecompress`'s block loop with literals and back-references. Its six static tables are generated from the
reference by `tools/gen-fa-compress-tables.py` (and `--check`ed in the battery), because the first hand
transcription of `Table1` produced values that appear nowhere in the reference.

It is verified against the archive, not against my reading of the C. `GwDat::file()` decodes an entry against
the size in its last four bytes - the pair `FaGetFile` performs - and `a_map_blob_decompresses_to_the_containers_own_signature`
checks three real map blobs for the things a one-bit error would break:

- the decoded length is exactly the announced size (Kamadan: 1,734,079 bytes from 515,444 stored);
- the first four bytes are `"ffna"`, which is the reference's `RIFF_SIGNATURE` (`GmMapDataImport.c:3`);
- the fifth byte is a chunk type, which its `RiffCheckHeader` also requires.

Two details the port had to get right rather than nearly right. The C reads its input as a `u32 *`, so a tail
shorter than a word makes it read the four bytes it was told to ignore - padding reproduces that without the
out-of-bounds read. And `next_bit_encoding + 1` relies on **unsigned wraparound** (the counter runs below zero
and wraps to `u32::MAX`); C wraps silently, Rust panics, so the port wraps explicitly with the reason in a
comment.

**D3 landed: the map importer.** `crates/asura-world/src/map_data.rs` is `GmMapDataImport.c` (590 lines)
a function at a time - `RiffCheckHeader`, `RiffParseChunks`, `RiffGetChunk`, `PathImport_CheckHeader`,
`PathImport_Step2`, and the ten plane readers (counts, tag 11, vectors, trapezoids, root node, x nodes, y
nodes, sink nodes, portal traps, portals) - plus `PathFindTrapezoid` from the top of `GmPaths.c`, which is
the query "which trapezoid is this position in".

The verification is the reference's own tool, not my reading of its C. `tools/gwdat_pathing.c` imports a
blob with `GmImportMap` and prints `file_id,size,planes,trapezoids,min_x,min_y,max_x,max_y`; if both sides
read the same file the same way, all eight numbers agree. They do, for every blob the catalogue names:

```
0x0001B97D,2925267,58,6120,-18432.000,-24576.000,21504.000,24576.000    Lakeside / Ascalon / Ashford
0x000345CC,1734079,39,1270,-18432.000,1726.000,9081.000,21504.000       Kamadan
0x0005CEE6,3320363,51,5246,-21504.000,-27648.000,21504.000,20736.000    The Catacombs
0x0005CEE7,3389269,18,5241,-24576.000,-21302.662,24576.000,25240.000    Wizard's Folly / Foibles Fair
0x0005CF29,3252319,45,4772,-23328.000,-21504.000,27648.000,16479.221    Green Hills / Barradin
0x0005CF2A,3713598,24,5267,-30720.000,-16608.000,26592.000,22168.000    Regent Valley / Fort Ranik
0x8005EA96,3657363,48,6890,-20640.000,-21504.000,24279.000,24331.000    The Northlands
```

Those are 283 planes and 34,806 trapezoids, and the odd-looking corners (`16479.221`, `-21302.662`) agree
to three decimals too - which is the point: a misplaced field or an off-by-one in a row size moves a
bounding box even when the counts happen to survive.

`map-census` (an example) is the runnable form: it imports all twelve catalogue rows out of a real
`Gw.dat` and prints each one's planes, trapezoids, node counts and where its spawn point lands, plus
`--fingerprint` for the eight-column shape above.

Three things make that a measurement rather than a claim:

- The reference's rows were held in `crates/asura-world/tests/fixtures/gwdat-pathing.csv`, which is gone from the tree with the rest of the parity fixtures (`docs/16`);
  it compared all eight columns of every catalogue blob against a real `Gw.dat`. The surviving
  equivalent is `the_spawn_points_the_reference_configures_land_in_the_graphs_in_the_archive`
  (`crates/asura-world/src/map_data.rs:1045`);
- `tools/parity-pathing.py --check` re-runs the reference binary, compares it to the fixture, and compares
  both to this repo's importer - the third leg is what catches the fixture going stale *and* the importer
  drifting in one run;
- `the_spawn_points_the_reference_configures_land_in_the_graphs_in_the_archive` walked every map's configured
  spawn point (`tools/map-config.sql`, generated from `GmMapsConfig.c`) and requires a trapezoid: all
  twelve land inside, three of them from the same blob (Lakeside County serving Ascalon City and Ashford
  Abbey as well).

Three reference behaviours were kept rather than corrected, and the reason is the same for all three:
they are what the file holds.

- `PathImport_PlaneTag11` advances `h000C * 8` bytes of a section whose declared length is `sub_length`,
  and in **all 283 measured planes** `sub_length` is exactly twice that. The next section's tag sits at the
  midpoint. A reader advancing `sub_length` would step over the vector section; the reference reads
  *inside* section 11's bounds and so does this port.
- `PathImport_Step2` takes its length from byte 1 of a five-byte header - an unaligned `u32` that
  swallows the tag byte. It is what the format is.
- `GetNodeFromDatId` answers a null pointer, not an error, for a node index past its array, so a graph can
  import with a dead end in it; `trapezoid_at` stops at a dead end like the reference's walk does.

Pointers became array positions (`NodeRef`, `Option<usize>`, `Option<(plane, portal)>`) - the same graph
without a self-referential struct.

**D3's other half landed: the search.** `crates/asura-world/src/pathfind.rs` is `GmPaths.c` and
`GmPathHeap.c` ported - the A* itself, the corridor/walk builder that turns a chain of trapezoids into
waypoints, the four neighbour visits and the two portal visits, and the heap. Its fixed-point
`PathReciproc`/`PathSqrt` tables (256 entries each, float *bit patterns* combined with integer
arithmetic) are generated from `GmPaths.c` by `tools/gen-path-tables.py`, because an A* fed by
`1.0 / x` and `x.sqrt()` is the same algorithm with different tie-breaks and therefore a different
walk.

**How it is measured.** `tools/path-oracle.c` is a host tool that compiles the reference's single
translation unit *unmodified* (`#define COMPILE_TOOLS 1`, the same trick the reference's own tools use)
and calls `PathFinding` directly; `tools/build-oracle.ps1` compiles it with the MSVC the reference
was built with, linking the static libraries its CMake build already produced. Nothing under
`PreSearing/` is written to. Both sides then run the same 96 cases
(a fixture file, `path-cases.txt` under `crates/asura-world/tests/fixtures/`, since removed with the other parity fixtures) and
print one CSV row per case with coordinates as raw IEEE-754 bit patterns:

```
the path finder matches the reference on 96 case(s): 41 walk(s) found,
127 waypoint(s) identical, 1 case(s) compared on the answer alone
```

So 95 of 96 walks - found or refused - come back with the same `result`, the same waypoint count, and
waypoints identical down to the last bit of every coordinate, including the paths that cross planes
through portals. The case-by-case comparison is
was `every_case_produces_the_references_own_waypoints` (`tests/pathfind.rs`); both the fixture and that
file are gone, and the surviving comparison is `tests/path_dump.rs` against `data/path-graphs.bin`, and
`tools/parity-path.py --check` re-runs the reference itself.

**The one case that cannot be pinned is a finding, not a gap.** `PathBuildAddWaypointAndReduce` reads `t`
and `s` after `FindIntersectionPoint` *fails* - the two lines are parallel, the function leaves its
outputs untouched, and the reference then branches on whatever was in those two stack slots. Exactly one
of the 96 cases has a step parallel to its own start-to-destination line, and the reference answers it with
11 waypoints on some runs and 12 on others (measured: two different answers for the same input, in the same
binary, differing only in one waypoint's y). No port can match an undefined value; leaving `t` at zero
reproduces the 11-waypoint answer byte for byte, so that is what the port does, and `tools/parity-path.py`
compares that case on its `result` alone and says so in the fixture's header.

Two other reference behaviours are kept for the same reason: `IsPointInTrap` runs its side tests on the
*absolute position* rather than the offset from the edge - so the region it accepts is a wedge from the map
origin, not the trapezoid's interior - and `PathAddNode` overwrites a trapezoid's node unconditionally and
queues it again on every visit.

**What D still owes the server.** The finder is measured but not yet *adopted*: `MOVE_TO_COORD` still echoes
the client's destination, and the reference instead pathfinds, refuses what it cannot reach, and walks the
agent waypoint by waypoint (`GmAgent.c:371-398`, `GmAgent.c:409-462`). Wiring it in changes behaviour the
load tests and the movement tests already pin, so it is the next unit.

### D4. Adopting the finder (`MOVE_TO_COORD` and the tick)

**D4 landed.** `MOVE_TO_COORD` now asks the finder, and the world walks agents itself:

- `lib.rs` keeps a `PathGraphs` - one imported graph per map plus one finder per map, behind one lock -
  loaded at startup from `Gw.dat` (`--gwdat`, `ASURA_GW_DAT`, then `Gw.dat`, then `..\PreSearing\Gw.dat`).
  That is the reference's `srv->map_context`, with one difference: the reference runs one server per map,
  so it has one context per process, while this hosts several maps in one process.
- The handler (`GameSrv_HandleMoveToCoord`) pathfinds from **the server's own position**, takes waypoint 0
  as the destination and broadcasts `AGENT_MOVE_TO_POINT`. On a refusal the reference cancels the walk and
  broadcasts `AGENT_UPDATE_POSITION`; **this tree cancels and sends nothing**, because that broadcast
  disconnected a live client once and snapped it into a wall once, both measured
  (`crates/asura-world/src/agent.rs:722-743`). The owner's ruling on whether the silence is the port is
  open (`crates/asura-world/tests/load.rs:3169`, ignored for that reason). An empty path sends nothing,
  as the reference's `0 < agent->waypoints.len` does.
- The world tick advances the agent `speed_base * delta` along the path (`GameSrv_UpdateAgentMovement`),
  hands over to the next waypoint when a segment ends - broadcasting the new destination as the reference's
  `GameSrv_UpdateAgentDestination` does - and ends the walk on arrival. `agent.rs` keeps that state as
  `Motion`, the reference's `destination`/`direction`/`speed`/`waypoints`/`waypoint_idx`.

What is asserted: `a_walk_is_answered_with_the_first_waypoint_of_the_servers_own_path` asks `World::graphs`
what the path is, sends a `MOVE_TO_COORD` that needs more than one waypoint, and requires the wire answer to
be waypoint 0 (and *not* the requested destination), the other player to see the same waypoint, and the
agent to have moved within a few ticks; `a_walk_the_finder_refuses_is_answered_with_the_agents_position`
sends one off the map's geometry and requires a position and no move message.

One deliberate difference, and the only one in the movement code: the reference divides by the distance when
it points an agent at a destination, so a walk to exactly where the agent stands gives it a NaN direction and
then a NaN position - which would be saved to the database and reloaded on the next visit. This keeps the
direction it had and lets the walk end immediately instead.

Still not integrated, and recorded rather than hidden: the reference's tick also regenerates
`health_per_sec`/`energy_per_sec` (`GmAgent.c:524-529`), which this sends once per load rather than once per
tick - invisible with no combat and no second player spending energy, and the audit's #3 row says so.

### E. Item lifecycle (T12)

`CREATE_UNNAMED_ITEM` cannot be emitted; the ground item cannot be sent; nothing picks anything up
(GWPS has no pick-up handler either).

**E2 landed.** `CREATE_UNNAMED_ITEM` turned out not to be a new feature but a *shape*: the reference has two item creators and uses both - `GameSrv_SendItemById` sends the named one for the inventory stream, and`GameSrv_SendItemsInBag` (`GmItem.c:111-127`) plus `GameSrv_BroadcastPlayerEquippedItems`
(`GmItem.c:129-160`) send the unnamed one for bag slots and equipped items. This repo had used the named form everywhere, so the creation screen's profession change was sending the wrong message. Fixed at the builder and at both call sites; the multi-player introduction now also broadcasts a newcomer's equipped items, visual equipment and cape in the reference's own order (`GameSrv.c:1643-1650`).

**E1 is decided, not done.** The ground item stays a log line: GWPS's own drop handler never removes the item and gives the ground agent the item's id as its model, and no capture contains a working drop. A capture is the only thing that would change this, and the reference cannot produce one - its handler is the unfinished one. Recorded in `docs/08-next-inventory-model.md`.

### F. The load block, byte-compared (T8, T15)

Only the item block is compared against a capture today. The player block is not - which is how two
missing party messages survived an audit that called the block equivalent.

| # | item | done when |
|---|---|---|
| F1 | a generated fixture from the capture (headers + sizes only, no names or UUIDs) | a test that fails when a message is dropped from the block |
| F2 | reconcile the block with the fixture | T15's audit is clean for the load |

**F1-F2 landed.** `tools/parity-load.py` reads the reference's own `GWPS_TRACE=1` log, drops the periodic
traffic (pings and simulation ticks, which the log has and a block dump does not - the reference's own "N
(non-feature)" tier), and compares the ordered `(header, size)` stream. Result: **480 messages, 0 missing,
0 extra, 0 size disagreements**. That measurement was committed as `crates/asura-world/src/load_fixture.rs`, which is gone from the tree
(headers and sizes only, no payload), `the_load_stream_is_the_references_message_for_message` compares the
four blocks against it, and `tools/test-battery.ps1` step 8 re-measures with `--check` so a drift fails the
battery.

The finding that started this item is closed by it: the two party messages are in the stream, in the
reference's position, at the reference's size.

**F3 landed: the payloads are compared too, and three messages differ for measured reasons.**
`tools/parity-load-bytes.py` walks the same 480 messages and compares the bytes - a wrong *value* is the same
size, which is exactly what a `(header, size)` diff cannot see. The result:

```
91 of 480 message(s) are byte-identical to the reference; 389 allowed difference(s) in 3 message(s)
```

The three, each with the reference's own code as the evidence:

- **`0x001A` `PVP_ITEM_STREAM_ADD` (387 messages).** The reference sends `params[0]` once per parameter:
  `pack_helper`'s nested-struct case (`msgpack.c:411-436`) calls **`unpack_helper`** - the other direction -
  and never advances `consumed`, so every element is read from the same place. The capture shows
  `0x24080081` three times for item 1, where the reference's own table has
  `{0x24080081, 0x25300102, 0x24B80300}`. This server sends the table's real values (generated from that same
  array by `tools/gen-pvp-items.py`), so the check compares the header, the item id and the count.
- **`0x0094` `MAP_UPDATE_UNLOCKED_LIST` (1).** 260 bytes of which the reference fills the list lengths and
  the bitmap copies of a fresh character - the rest is whatever the previous message left in its per-server
  buffer. The capture's payload carries armour file ids and dye colours in those bytes. This server sends
  zeros, and the check requires every byte it *does* set to be the reference's.
- **`0x001F` `WORLD_UPDATE_LOAD_TIME` (1).** A wall-clock measurement; only the size is comparable.

Everything else - the agent record, the player record, the whole item stream and the tail of the block - is
the reference's bytes. Two *dump* bugs surfaced on the way and are fixed: the example passed the default
traits, so `PLAYER_UPDATE_PROFESSION` sent 0 where the reference's character row held 1 (Warrior) and
`_UNLOCKED_PROFESSIONS` sent 0 where it held 2 - a difference in the dump rather than in the server.

`tools/pack-oracle.c` came out of the work and stays: it puts a message struct through the reference's own
`pack_msg` with the reference's format table (and prints that table's field list with `pack-oracle fields
<header>`), so a message's bytes can be asked for rather than reasoned about. `tools/build-oracle.ps1` now
builds either oracle.

### G. Chat (T11) - equal to the reference, and that is the ceiling

Both servers send a player's line and print a generated answer; neither has ever rendered chat on
screen. `0x005D`/`0x005E`/`0x0061` appear in no capture.

| # | item | done when |
|---|---|---|
| G1 | none, unless a capture of a working chat message appears | recorded, not worked |

### H. Operations parity (T13, T15)

**H1 landed as a measurement, all three clauses of it.** The cadence and the accounting are asserted by
`an_idle_pair_keeps_its_rhythm_and_its_count`: two connections idle for six seconds, the tick and ping on the
reference's own intervals, an idle client told nothing else, and the world's player list a pair that empties
when they leave. `a_ten_minute_idle_pair_keeps_its_rhythm` is the same assertion at the reference's own
duration - against a real **pair**, marked `#[ignore]` because a ten-minute test is a test nobody runs - and
it also **samples the working set** every fifteen seconds through `K32GetProcessMemoryInfo` and fails if it
grows by more than 8 MiB after the warm-up. The sampler checks itself first (16 MiB allocated and touched, and
the sample has to see it), because a flat line from an instrument that never moves is not a measurement.
The procedure, and the `ASURA_SOAK_SECONDS` short form, are in `docs/HANDOFF.md`.
| H2 | the generated audits run in `tools/test-battery.ps1` (dispatch, emit, state) and their output is in `docs/07-status.md` | a regression in any axis fails the battery |

## What is deliberately *not* in the queue

Recorded so nobody adds it later: matching GWPS where GWPS is unfinished or does not fit this design.

- **GWPS's drop handler** - `TODO` for partial stacks, item never removed, model id set to the item id.
- **GWPS's server-side chat** - gated off because it crashed its client twice.
- **`GmParty.c` hero support** (`PARTY_HERO_ADD`) - no heroes exist here; the captures carry it 0 times.
- **Download manifest data** (`INSTANCE_MANIFEST_DATA`) - 0 times in the captures; nothing to download.
- **Campaigns, PvP, guild halls** - the reference's own `X - Out of scope` tier.

## Order of work

1. **A1 + A2 + A3 + A4 + A5** - multi-player is what most of the missing surface is *for*; it also
   makes T9, T12 and T14 measurable for the first time.
2. **C1 -> C4** - `/tp` is measured end to end and unblocks the second map without a rebuild.
3. **F1 -> F2** - the load block becomes provable, which is what keeps the rest honest.
4. **B1 -> B4** - character creation, the largest data port.
5. **D1 -> D2 -> D3** (archive, decompressor, importer, finder - landed) -> **D4, adopting the finder in
   `MOVE_TO_COORD`** -> **E1/E2** -> **H1/H2**.

Each step lands with its test, and the audit documents are regenerated rather than edited:
`python tools/parity-report.py --write docs/07-status.md`, `python tools/parity-emit.py`.
