# The title system: cross-repo research

**Status:** research note, 2026-11-05.
**Question:** how the Guild Wars title system works, what the wire actually carries, and what this repository still owes.

This note is deliberately evidence-first. Claims are tagged where their status matters; the source
index in section 12 gives the full citations. The tags are:

- **OBSERVED** - read from a packet capture in `recordings/` (client build 39112) or a live-client
  run recorded in another repo.
- **SOURCED** - read out of a client binary, a repo's packet table, or a server's source.
- **UPSTREAM** - a name or shape that only a mirror lineage carries.
- **WIKI** - Guild Wars Wiki, used only for the game rules the wire does not state.

The primary sources are:

| Source | What it contributes |
|---|---|
| `Other_Repos_Research/OpenTyria-master` | the 48-title enum, `TitleConstData` (42 tracks), the orphaned `titles` SQL table, the commented-out send site |
| `Other_Repos_Research/Rurik-main` | the only end-to-end title implementation and probe; measured crash rules; retail-traffic field map |
| `Other_Repos_Research/GWToolboxpp-master` | the client-side consumer: `TitleID`, `Title`, `TitleTier`, max-point table, map->title mapping, `/title` |
| `Other_Repos_Research/GameRevision-GWLP-R-4208ffa` | `P231`-`P234` protocol shells (older opcode build) |
| `Other_Repos_Research/GWLP-R-Utils-master` | `PacketTemplates.xml` field names and descriptions |
| `Other_Repos_Research/Headquarter` | `0x00F3`/`0x00F4`/`0x00F6` structs and `TitleModeFlags` |
| `Other_Repos_Research/preservation-main` | `fields_2008.txt` / `fields_2025.txt` descriptor dumps - proof that opcodes move between builds |
| `recordings/client/...` (this repo) | live-server title packets from client build 39112 (the server is not part of the capture) |
| `docs/36-game-rules-ledger.md` | this repo's earlier title ledger, E22-E23 and E156-E175 |

If you are implementing rather than reading, the shortest path is section 3.4 (field map and corrections),
section 6 (crash rules), and section 10 (ASURA plan).

---

## 0. The ten things that matter

1. **There are 48 title tracks, indexed 0-47.** Three independent 48s agree: the client's
   `s_titleClientData` table, OpenTyria's `TitleId` enum, and Py4GW's shared-memory mirror
   (`Rurik-main/studies/character/STORAGE.md:198-202`). The id space is fixed; the names are
   compiled into the client, not sent on the wire.
2. **The wire family is four StoC messages and two CtoS messages.**
   `0x00F3` rank definition, `0x00F4` displayed rank binding, `0x00F5` point patch,
   `0x00F6` full track record; CtoS `0x0058` set title, `0x0059` hide title.
3. **The numbers move between client builds.** The same family sits at `0x00E0-0x00E3` in a 2008
   descriptor dump, `0x00E7-0x00EA` in GWLP-R's old templates, `0x00F3-0x00F6` in OpenTyria/GWCA
   and in build 39112, and `0x00F2-0x00F5` in a 2025 descriptor dump. Hardcoding an opcode without
   the build is a bug waiting to happen.
4. **The track record grew.** Older builds carry 9 dwords + 2 strings (`P234`, the 2008 and 2025
   dumps). Modern builds carry **10** dwords + 2 strings; the extra dword is
   `title_mode_flags` (`Headquarter/code/client/title.c:56`), value 0 = none, 1 = Dhuum's
   Covenant, 2 = Reforged Mode. Build 39112 sends 0 in every observed track.
5. **`0x00F3` is a rank *vocabulary*, not a title record.** It defines one rank by id:
   `rank_id, flags, value, coded name`. `0x00F6` and `0x00F4` then reference those ids.
6. **Ranks must be sent before they are referenced.** A `0x00F6` that names an unseeded rank is
   accepted silently and kills the client at render time (`Assertion: index < m_count`,
   `Array.h:587`; Rurik `studies/character/RUNS.md:177-186`). `0x00F4` uses the same rank table,
   so seed it too, but its own render path is the open display question.
7. **The string fields are coded strings and the small ones are a trap.** `string16(8)` admits at
   most 7 units on receive; an exactly-at-cap literal is an instant `Code=007` hangup. Template
   framing spends 3 units, so Rurik caps rank names at 4 characters
   (`Rurik-main/toolkit/authsrv/charstore.py:73-85`, `questdefs.py:95-103`).
8. **Only tracks with progress are sent.** A retail load sends a handful of titles, not all 48;
   for each one it sends the current and next rank records, then the track. Build 39112's complete
   load had 14 tracks and 28 rank records. A Factions character's capture had 13 tracks, including
   a new Canthan Cartographer at 1 point, while the account-wide titles carried existing progress.
9. **The server owns the title-id -> rank-id mapping.** The client sends `0x0058` with a title id;
   the server answers with `0x00F4` carrying the rank id it wants displayed
   (`docs/36-game-rules-ledger.md:308`, E160).
10. **ASURA has the foundation, and no earning yet (2026-11-05).** The four StoC builders and the
    `0x8058` reader are `crates/asura-protocol/src/game/title.rs`; the rung, load-group, point-change
    and display decisions are `crates/asura-rules/src/titles.rs`; the definitions are
    `config/Server/titles.json`; progress is `t_character_titles` / `t_account_titles` (migration
    0028); the world side is `crates/asura-world/src/titles.rs`. No ASURA recording has ever carried a
    title message from a client: the `0x0059` in ASURA's own captures is the server's
    `GAME_SMSG_PLAYER_CREATE`, not the client's hide (section 7).

---

## 1. What a title is (game model)

A title is an achievement track displayed on the Hero Panel's **Titles** tab and, optionally,
under the character's name in a staging area. The wiki's rules are the ones the client implements:

- **Character-based vs account-based.** Character titles progress separately per character;
  account titles share progress across the account (`WIKI`, `Title`).
- **A track appears only after progress.** Two exceptions: Legendary Defender of Ascalon at level
  12 and Survivor at level 5 (`WIKI`; ledger E171).
- **Ranks.** A title is divided into ranks; some titles have only one. The rank name is specific
  to that rank, and the rank number is shown in parentheses beside the name on the Hero Panel
  (`WIKI`).
- **Display.** A rank becomes displayable at a title-specific minimum. A checkbox appears; checking
  it shows that rank's name under the character's name. Only one title is displayed at a time.
  Hover/target reveals it in staging areas only (`WIKI`; ledger E172).
- **Progress is automatic and simultaneous.** No acceptance step; every applicable track accrues
  at once (`WIKI`; ledger E173).

The client's own split is visible in OpenTyria's `TitleType` enum
(`OpenTyria-master/code/OpenTyria/GmTitles.h:83-89`):

```c
typedef enum TitleType {
    TitleType_Points = 0,        // character-based points track
    TitleType_Cartographer = 1,  // character-based percentage track
    TitleType_Missions = 2,      // character-based mission/count track
    TitleType_Deprecated = 3,    // retired
    TitleType_Account = 4,       // account-wide
} TitleType;
```

That enum is a useful cross-check: every title OpenTyria marks `Account` is one the wiki lists as
account-wide, and the `Points`/`Cartographer`/`Missions` titles are character-based. OpenTyria's
const data covers 42 of the 48 tracks; the six it omits are Champion, the three deprecated
tracks, LDoA and Commander.

---

## 2. The 48 title tracks

Ids and enum names are from OpenTyria `GmTitles.h:3-51` and GWCA
`Constants.h:162-179`; the two lists are byte-identical in order. Scope is OpenTyria's
`TitleType` where present, otherwise the wiki. Max points are GWToolbox's
`TitleTrackerWidget.cpp:54-96` (the client-side "points to max" table). Max rank / flags are
**OBSERVED** in build 39112 where a value is shown.

| id | enum | scope / type | GWToolbox max points | observed max rank / track flags |
|---:|---|---|---:|---|
| 0 | `Hero` | account | 100,000 | 15 / 4 |
| 1 | `TyrianCarto` | character, percentage | - | - |
| 2 | `CanthanCarto` | character, percentage | - | 6 / 1 |
| 3 | `Gladiator` | account | 200,000 | 12 / 4 |
| 4 | `Champion` | account | 10,000 | - |
| 5 | `Kurzick` | account | 10,000,000 | 12 / 4 |
| 6 | `Luxon` | account | 10,000,000 | 12 / 4 |
| 7 | `Drunkard` | character, points | 10,000 | - |
| 8 | `Deprecated_SkillHunter` | retired | - | - |
| 9 | `Survivor` | character, points | 1,337,500 | - |
| 10 | `KoaBD` | character, points | 30 | 6 / 0 |
| 11 | `Deprecated_TreasureHunter` | retired | - | - |
| 12 | `Deprecated_Wisdom` | retired | - | - |
| 13 | `ProtectorTyria` | character, missions | - | - |
| 14 | `ProtectorCantha` | character, missions | - | - |
| 15 | `Lucky` | account | 2,500,000 | 6 / 4 |
| 16 | `Unlucky` | account | 500,000 | 7 / 4 |
| 17 | `Sunspear` | character, points | 50,000 | - |
| 18 | `ElonianCarto` | character, percentage | - | - |
| 19 | `ProtectorElona` | character, missions | - | - |
| 20 | `Lightbringer` | character, points | 50,000 | - |
| 21 | `LDoA` | character, single rank | - | 1 / 2 |
| 22 | `Commander` | account, retired | - | 12 / 4 |
| 23 | `Gamer` | account | 135,000 | 12 / 4 |
| 24 | `SkillHunterTyria` | character, missions | - | - |
| 25 | `VanquisherTyria` | character, missions | - | - |
| 26 | `SkillHunterCantha` | character, missions | - | - |
| 27 | `VanquisherCantha` | character, missions | - | - |
| 28 | `SkillHunterElona` | character, missions | - | - |
| 29 | `VanquisherElona` | character, missions | - | - |
| 30 | `LegendaryCarto` | character, missions | - | - |
| 31 | `LegendaryGuardian` | character, missions | - | - |
| 32 | `LegendarySkillHunter` | character, missions | - | - |
| 33 | `LegendaryVanquisher` | character, missions | - | - |
| 34 | `Sweets` | character, points | 10,000 | 2 / 0 |
| 35 | `GuardianTyria` | character, missions | - | - |
| 36 | `GuardianCantha` | character, missions | - | - |
| 37 | `GuardianElona` | character, missions | - | - |
| 38 | `Asuran` | character, points | 160,000 | - |
| 39 | `Deldrimor` | character, points | 160,000 | - |
| 40 | `Vanguard` | character, points | 160,000 | - |
| 41 | `Norn` | character, points | 160,000 | - |
| 42 | `MasterOfTheNorth` | character, points | 1,000 | - |
| 43 | `Party` | character, points | 10,000 | - |
| 44 | `Zaishen` | account | 100,000 | 12 / 4 |
| 45 | `TreasureHunter` | account | 10,000 | 7 / 4 |
| 46 | `Wisdom` | account | 10,000 | 7 / 4 |
| 47 | `Codex` | account | 200,000 | - |

Notes:

- The observed `max rank` field is the highest **rank number** (1-based in game terms); rank 0
  is the "not yet ranked" tier that every track starts on. `KoaBD` shows 6 because its rank names
  run 1-6 (5/10/15/20/25/30 maxed titles); LDoA shows 1 because it has one rank.
- GWCA's enum adds `None = 0xff` as the "no title" sentinel. The wire's title-id space is still
  0-47; `0xff` is a client-side value, not a `0x00F6` title id.
- `flags` here is the `0x00F6` field 2 word. Bit meanings are in section 3.4.
- The percent tracks (Cartographer) and the mission/count tracks have no single "max points" in
  GWToolbox because their progress is a percentage or a count.

---

## 3. Wire protocol

### 3.1 The six messages

| Direction | Opcode | Name | Shape |
|---|---|---|---|
| S->C | `0x00F3` (243) | `GAME_SMSG_TITLE_RANK_DATA` | 3 x u32 + `string16(8)` |
| S->C | `0x00F4` (244) | `GAME_SMSG_TITLE_RANK_DISPLAY` | u16 player + u32 rank id |
| S->C | `0x00F5` (245) | `GAME_SMSG_TITLE_UPDATE` | u32 title id + u32 new points |
| S->C | `0x00F6` (246) | `GAME_SMSG_TITLE_TRACK_INFO` | 10 x u32 + 2 x `string16(8)` (modern) |
| C->S | `0x0058` (88) | `GAME_CMSG_TITLE_DISPLAY` | u32 title id |
| C->S | `0x0059` (89) | `GAME_CMSG_TITLE_HIDE` | no fields |

The ASURA tables already carry the first four at these numbers
(`crates/asura-protocol/src/packets.rs:771-774`, `:1990-2000`) and the two CtoS at
`:422-423`, `:1441-1445`. Rurik's measured schema has the same shapes
(`Rurik-main/studies/character/STORAGE.md:178-186`).

The declared maximum sizes independently confirm the layouts: `0x00F3` is 34 bytes =
2 header + 3x4 payload + a 20-byte string (2-byte length + 9 u16 units), and `0x00F6` is
82 bytes = 2 header + 10x4 payload + two 20-byte strings. That is how the "10 dwords" and the
"8 characters plus terminator" in section 6 rule 2 are known without a packed frame.

### 3.2 Field layouts

**`0x00F3` - rank definition.** `rank_id` is the global key, `flags` is per-rank (bit 0 =
percentage value), `value` is the rank number for a points track or the per-mille threshold for a
percentage track, and `name` is a coded string. Headquarter's struct is
`rank_id, unk0, rank, name` (`Headquarter/code/client/title.c:9-15`); Rurik's measured names are
`rank_id, flags, threshold/value, coded name` (`studies/character/STORAGE.md:180`).

**`0x00F4` - display binding.** `player_id` (word on the wire, widened to a dword in the client's
in-memory struct) and `rank_id`. It writes the rank id into the player record and posts UI message
`0x10000064`; this is what puts a rank name under another player's nameplate
(`Rurik-main/schema/overrides.json:2370-2372`).

**`0x00F5` - point patch.** `title_id`, `new_points`. It patches only the current-points dword of
an existing track and posts `0x10000065`. It is **inert unless a prior `0x00F6` set the track's
description pointer** (`entry+0x28`); this guard was measured, not inferred
(`Rurik-main/studies/newopcodes/FINDINGS.md:1433-1437`, `:1479-1480`).

**`0x00F6` - track record.** The modern layout, confirmed by both Rurik's disassembly and the
build-39112 captures:

| wire field | meaning | notes |
|---:|---|---|
| 1 | title id (0-47) | indexes `s_titleClientData` and the client's title array at `WorldContext+0x81C` |
| 2 | flags / group word | bit 0 = percentage; `(flags & 3) == 2` = "x of y" count; observed 0/1/2/4 |
| 3 | current points | the only field `0x00F5` patches |
| 4 | current rank id | must exist in the `0x00F3` table |
| 5 | current rank minimum | points at which the current rank was reached |
| 6 | `title_mode_flags` | modern only: 0 none, 1 Dhuum's Covenant, 2 Reforged Mode |
| 7 | next rank id | must exist in the `0x00F3` table |
| 8 | next rank minimum | `0xFFFFFFFF` means "no next rank" / maxed |
| 9 | max rank | highest rank number on the track (0-based count) |
| 10 | base rank id | see section 3.4; behaves like the rank-1 record, not "max rank id" |
| 11 | progress template | coded string, printf-style, composed with the value |
| 12 | description template | coded string, composed with the next-tier name and number |

The older 9-dword layout is exactly this without field 6. That is why GWLP-R `P234` has nine
dwords (`GWLP-R-Utils-master/Templates/PacketTemplates.xml:3119-3183`), while Headquarter's
struct has ten (`Headquarter/code/client/title.c:48-63`).

### 3.3 Opcode drift - do not hardcode without a build

The title family is stable in **order** and unstable in **number**. Four snapshots:

| Source | Build era | RankData | RankDisplay | TitleUpdate | TrackInfo |
|---|---|---:|---:|---:|---:|
| `preservation-main/data/fields_2008.txt:1829-1844` | 2008 client | `0xE0` | `0xE1` | `0xE2` | `0xE3` (9 dwords) |
| `GameRevision-GWLP-R` `P231`-`P234` | old GWLP-R client | `0xE7` (P231) | `0xE8` (P232) | `0xE9` (P233) | `0xEA` (P234, 9 dwords) |
| OpenTyria / GWCA / build 38833 / build 39112 | modern | `0xF3` | `0xF4` | `0xF5` | `0xF6` (10 dwords in 39112) |
| `preservation-main/data/fields_2025.txt:1873-1897` | 2025 descriptor dump | `0xF2` | `0xF3` | `0xF4` | `0xF5` (9 dwords) |

Two conclusions:

- **The family moves as a block.** The descriptor dumps show RankData -> RankDisplay -> Update ->
  TrackInfo in that order in every era; only the base changes. The P-prefix in GWLP-R is the
  header number, not a stable protocol id.
- **The modern track record is wider.** The 2008, GWLP-R and 2025 dumps have nine dwords; build
  39112 has ten. The tenth is `title_mode_flags`, which matches the Dhuum's Covenant / Reforged
  Mode features. A server targeting a modern client must send ten; a server targeting an older
  client must send nine.

ASURA's packet table uses `0xF3-0xF6`, which is correct for the client it records (39112), but the
table should be read as build-scoped, not as a constant of the protocol.

### 3.4 Measured field map - and four corrections

Rurik's disassembly of build 38797/38833 is the best field map available
(`studies/newopcodes/FINDINGS.md:1463-1480`). Build 39112's captures add four corrections:

**Correction 1 - `0x00F6` fields 5/8 are thresholds; they do not generally restate `0x00F3`
field 3.** Rurik's retail sample used title 2, a percentage track, where the rank record's value
and the track's threshold happen to be the same number (600). For point tracks they differ. In
build 39112:

```
title 0 Hero:   0x00F3 rank 6 value = 6,   0x00F6 current minimum = 1000
title 15 Lucky: 0x00F3 rank 104 value = 3, 0x00F6 current minimum = 250000
```

So `0x00F3` field 3 is the **rank number** (the number shown in parentheses) when its own flags
bit 0 is clear, and the **per-mille threshold** when bit 0 is set. For point tracks the threshold
lives only in `0x00F6`; for percentage tracks the rank record's value and the track's threshold are
the same number, which is why Rurik's one percentage sample made them look interchangeable. The
parenthesized number is this value; bit 0 makes the renderer divide by 10 and print `N.M`, so a
Cartographer value of 600 renders as `60.0` while a point track's value 6 renders as `6`.

**Correction 2 - `0x00F6` field 10 is not "max rank id".** In 14 of the 15 tracks observed in
build 39112, it is the **rank id of rank 1** (the base of the title's rank block), and
`current_rank_id = field10 + (current_rank - 1)`:

```
Kurzick: base 74, current rank 12 -> 74 + 11 = 85   (observed current id 85)
Luxon:   base 88, current rank 11 -> 88 + 10 = 98   (observed current id 98)
Lucky:   base 102, current rank 3 -> 102 + 2 = 104  (observed current id 104)
Wisdom:  base 361, current rank 7 -> 361 + 6 = 367  (observed current id 367)
```

Hero is the one exception (`base 3`, current rank 6, observed id 6), so the arithmetic is a retail
allocation habit, not a protocol rule. GWCA's own comment guessed "base rank"
(`StoC.h:1019`); Rurik and GWLP-R call it "max rank id", which the data contradicts.

**Correction 3 - `0x00F6` field 6 is not unread.** Rurik marked it `UNVERIFIED` / read by nothing.
Headquarter names it `title_mode_flags` and gives the enum
`None = 0, DhuumCovenant = 1, ReforgedMode = 2` (`Headquarter/code/client/title.c:56`,
`Headquarter/include/client/constants.h:258-262`). Build 39112 sends 0 in every observed track.

**Correction 4 - `0x00F6` field 2 is both a flag word and a "group" word, depending on the
reader.** GWLP-R calls it `TitleGroupId` (0 = no group); Rurik measured bit 0 = percentage and
bits 0-1 == 2 = "x of y" count; GWToolbox reads it as `props` and tests those bits
(`Title.h:24-25`). Observed values 0, 1, 2, 4 are consistent with a bitfield, not with a small
group enum. Treat it as a flag word and preserve unknown bits.

---

## 4. Client-side data model

GWCA's headers give the memory layout that the wire feeds:

```c
struct Title {                 // 0x2C bytes
    uint32_t props;            // 0x00
    uint32_t current_points;   // 0x04
    uint32_t current_title_tier_index;   // 0x08
    uint32_t points_needed_current_rank; // 0x0C
    uint32_t h0010;            // 0x10
    uint32_t next_title_tier_index;      // 0x14
    uint32_t points_needed_next_rank;    // 0x18
    uint32_t max_title_rank;   // 0x1C
    uint32_t max_title_tier_index;       // 0x20
    wchar_t* points_desc;      // 0x24
    wchar_t* h0028;            // 0x28
};

struct TitleTier {             // 0x0C bytes
    uint32_t props;
    uint32_t tier_number;
    wchar_t* tier_name_enc;
};
```

`GWCA/GameEntities/Title.h:11-43`; the header comment says `0x28` but the `static_assert` says
`0x2c` - the comment is stale. The `WorldContext` arrays are
`titles` at `+0x81C` and `title_tiers` at `+0x82C`
(`GWCA/Context/WorldContext.h:303-304`).

Consequences:

- `0x00F6` grows the **title array** to `title_id + 1` and copies nine dwords into the record at
  `+0x00..+0x20`; the two wire strings become fresh buffers at `+0x24`/`+0x28`
  (`Rurik-main/studies/newopcodes/FINDINGS.md:1429-1432`).
- `0x00F3` grows the **tier array** at `+0x82C` (stride 12) keyed by `rank_id`.
- `0x00F4` writes the rank id into the player record at `+0x30` and posts `0x10000064`.
- `0x00F5` writes `entry+0x04` and posts `0x10000065`.
- A title id outside 0-47 is refused by the client's own assert
  (`ConstTitle:81 index < arrsize(s_titleClientData)`), and an unseeded rank id is refused at
  render by `Array.h(587)`.

The client also has a compiled `s_titleClientData` table of 48 rows
`{title_flags, title_id, name_id}` (`GWCA/GameEntities/Title.h:38-42`). That is where the
**title** name comes from. The **rank** name comes from the wire's `0x00F3` string, which is why
Rurik's probe was able to render a row named "Ruri" (`studies/character/RUNS.md:188-195`).

---

## 5. Real packet data - build 39112

The captures are client-side sniff files; the server is not part of the capture, but the traffic
is live-server traffic (the recordings index calls it "a live server"). Their `data` is the
client's in-memory message **struct**, not the packed wire frame: a four-byte header (u16 id + zero
u16), every field widened to four bytes, and zero padding out to the struct size
(`tools/sniff-capture.py`, the note above `sniff_counts`; also `docs/03-protocol.md` section 5e). That is
why a `0x00F4` record is 12 bytes here (4-byte header + 2 widened dwords) while the wire message
is 8 bytes (2-byte header + word + dword). For brevity, "retail" below means the live server in
these captures.

### 5.1 The observed tracks

From `recordings/client/feature/travel/portal-battery-20261031-013856.jsonl`, plus title 2 from
`recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`:

| id | title | flags | points | cur rank id (value) | cur min | next rank id (value) | next min | max rank | field10 |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | Hero | 4 | 1,448 | 6 (6) | 1,000 | 7 (7) | 1,680 | 15 | 3 |
| 2 | CanthanCarto | 1 | 1 | 199 (0) | 0 | 200 (600) | 600 | 6 | 200 |
| 3 | Gladiator | 4 | 680 | 18 (1) | 500 | 19 (2) | 1,000 | 12 | 18 |
| 5 | Kurzick | 4 | 10,645,000 | 85 (12) | 10,000,000 | 86 (max) | max | 12 | 74 |
| 6 | Luxon | 4 | 7,580,000 | 98 (11) | 7,500,000 | 99 (12) | 10,000,000 | 12 | 88 |
| 10 | KoaBD | 0 | 3 | 150 (0) | 0 | 151 (1) | 5 | 6 | 151 |
| 15 | Lucky | 4 | 315,119 | 104 (3) | 250,000 | 105 (4) | 500,000 | 6 | 102 |
| 16 | Unlucky | 4 | 83,505 | 113 (4) | 50,000 | 114 (5) | 100,000 | 7 | 110 |
| 21 | LDoA | 2 | 13 | 176 (0) | 0 | 177 (1) | 20 | 1 | 177 |
| 22 | Commander | 4 | 100 | 45 (0) | 0 | 46 (1) | 125 | 12 | 46 |
| 23 | Gamer | 4 | 572 | 118 (0) | 0 | 119 (1) | 1,000 | 12 | 119 |
| 34 | Sweets | 0 | 3 | 273 (0) | 0 | 274 (1) | 1,000 | 2 | 274 |
| 44 | Zaishen | 4 | 75 | 337 (0) | 0 | 338 (1) | 250 | 12 | 338 |
| 45 | TreasureHunter | 4 | 10,592 | 358 (7) | 10,000 | 359 (max) | max | 7 | 352 |
| 46 | Wisdom | 4 | 19,708 | 367 (7) | 10,000 | 368 (max) | max | 7 | 361 |

`max` = `0xFFFFFFFF`. The values cross-check GWToolbox's max-point table: Kurzick 10.645M is past
its 10M cap; Treasure Hunter 10,592 and Wisdom 19,708 are past their 10,000 caps; Luxon 7.58M is
still rank 11 -> 12; Lucky 315k is rank 3 -> 4. The model and the packet agree.

### 5.2 The rank vocabulary

All distinct `0x00F3` records in the client captures:

| rank id | flags | value |
|---:|---:|---:|
| 6 | 0 | 6 |
| 7 | 0 | 7 |
| 18 | 0 | 1 |
| 19 | 0 | 2 |
| 23 | 0 | 6 |
| 45 | 0 | 0 |
| 46 | 0 | 1 |
| 74 | 0 | 1 |
| 85 | 0 | 12 |
| 86 | 0 | max |
| 98 | 0 | 11 |
| 99 | 0 | 12 |
| 104 | 0 | 3 |
| 105 | 0 | 4 |
| 113 | 0 | 4 |
| 114 | 0 | 5 |
| 118 | 0 | 0 |
| 119 | 0 | 1 |
| 134 | 0 | 2 |
| 150 | 0 | 0 |
| 151 | 0 | 1 |
| 152 | 0 | 2 |
| 176 | 0 | 0 |
| 177 | 0 | 1 |
| 199 | 1 | 0 |
| 200 | 1 | 600 |
| 273 | 0 | 0 |
| 274 | 0 | 1 |
| 337 | 0 | 0 |
| 338 | 0 | 1 |
| 349 | 0 | 12 |
| 358 | 0 | 7 |
| 359 | 0 | max |
| 361 | 0 | 1 |
| 367 | 0 | 7 |
| 368 | 0 | max |

Only rank ids 199/200 have `flags = 1` - the two Cartographer ranks. Every other rank is a point
track. The `max` value is the sentinel "no next rank" record that retail sends after a maxed
track's last real rank.

### 5.3 Display packets

The same captures contain 36 `0x00F4` records. Decoding `{player, rank_id}`:

```
player  1 -> rank   6  x11
player  2 -> rank 177  x2
player  3 -> rank   6
player  5 -> rank 349
player  7 -> rank 147
player  8 -> rank 152
player 16 -> rank  23
player 17 -> rank   6
player 23 -> rank 114  x2
player 24 -> rank 349
player 25 -> rank 361
player 29 -> rank   6  x2
player 31 -> rank 177
player 34 -> rank 177  x2
player 49 -> rank  74
player 51 -> rank 177
player 53 -> rank 152
player 58 -> rank   7
player 63 -> rank 134
player 64 -> rank 152
player 76 -> rank  74
player 77 -> rank 177
```

36 records in total, 22 unique player->rank pairs.

This is the under-nameplate title binding for the local player and the other players in the
district. The server sends
`0x00F3` for a rank id before the `0x00F4` that first references it; the captures show the pair
adjacent (`login-to-portal-20261031-003720.jsonl`, e.g. rank 349 -> player 24, rank 152 -> player
8/64, rank 74 -> player 76, rank 23 -> player 16, rank 361 -> player 25, rank 134 -> player 63).
Rank 147 appears in a display packet with no matching `0x00F3` in these files - likely seeded
before the capture started; it is a reminder that a capture's first packet is not a session's
first packet.

### 5.4 Ordering in a load

A normal load groups **per title**: rank records first, then the track. The first three title
groups of the portal-battery capture:

```
244 player 1 rank 6
243 rank 6 value 6
243 rank 7 value 7
246 title 0 ...
243 rank 18 value 1
243 rank 19 value 2
246 title 3 ...
243 rank 85 value 12
243 rank 86 value max
246 title 5 ...
...
```

Retail's order is not "all ranks, then all tracks"; it is "this title's ranks, then this title's
track", title by title. Rurik's server sends all ranks first and then all tracks
(`authsrv.py:43570-43594`); that also works because the requirement is only that a rank exists
before any reference to it, but it is not retail's shape.

A load sends only tracks with progress. The complete build-39112 load had 14 tracks and 28 rank
records; the earlier load in the same capture had 13, with title 46 (Wisdom) absent. The Factions
character's capture had 13 tracks, including title 2 at 1 point (the only newly-progressing
character track in that set) while the account-wide titles carried existing progress. No
zero-progress track was sent in any of these loads.

### 5.5 Where a binding sits, and a pre-Searing load

`recordings/client/feature/maps/pre-searing-load-3.jsonl` (a pre-Searing outpost) carries no
`0x00F6` at all: 8 `0x00F4` and 3 `0x00F3`, every one inside another player's introduction. Each
introduction runs `... 0x009F, 0x00B0, 0x00B1, 0x00A6, 0x009F, [0x00F3], 0x00F4, 0x003C, 0x00F0,
0x0020`: the record only when the rank is new to this connection (seq 217-221, 500-503, 672-676),
the binding only for a player who shows a title (seq 616-624 has none). The player's own binding,
in the portal-battery capture's first load, sits after `0x0022` and `0x009F` (seq 205-208).

### 5.6 Raw record example

`0x00F6` title 0 from the portal-battery capture, client struct:

```
f6000000 00000000 04000000 a8050000 06000000 e8030000
00000000 07000000 90060000 0f000000 03000000
2441 5ed0 c0cb 8056 0000 1400 0c00 0000
2541 dba1 bafc 2c6c 0000 1305 061e 0d00
```

Decoded:

```
header            0x00F6
title_id          0
flags             4
current_points    1448
current_rank      6
current_min       1000
title_mode_flags  0
next_rank         7
next_min          1680
max_rank          15
field10/base      3
string 1          8 u16 code units (coded string; sanitizer zeroed the printable run)
string 2          8 u16 code units
```

The strings are coded-string tokens, not UTF-16 text. The capture sanitiser zeroes printable
UTF-16 runs of six or more units, so the literal characters are gone but the surrounding code
units remain (`tools/sniff-capture.py`, `mask_utf16_runs`).

### 5.7 Decoding a client capture

The client-side records are in-memory structs: a 4-byte header, every dword widened to four
bytes, and each `string16(8)` held as 8 u16 units. A minimal decoder for `0x00F6`:

```python
import json, struct

for line in open("recordings/client/feature/travel/portal-battery-20261031-013856.jsonl"):
    rec = json.loads(line)
    if rec["header"] != 246 or not rec["data"]:
        continue
    b = bytes.fromhex(rec["data"])
    fields = struct.unpack_from("<10I", b, 4)
    (title_id, flags, points, current_rank, current_min,
     mode_flags, next_rank, next_min, max_rank, base_rank_id) = fields
    string_1 = struct.unpack_from("<8H", b, 44)
    string_2 = struct.unpack_from("<8H", b, 60)
```

For a packed wire capture (an `asura-world` capture rather than a client sniff), the header is
two bytes and the fields are packed, not widened; take the header length from the recording's
layer, as `tools/sniff-capture.py` says. The two layers are not interchangeable
(`docs/03-protocol.md` section 5e).

---

## 6. Server rules and crash constraints

These are the non-negotiable rules, all measured:

1. **Rank before reference.** Every rank id named in `0x00F6` field 4/7 must have been sent in a
   `0x00F3` first. The measured case is a `0x00F6`: an unseeded rank is silent on receive and fatal
   on first render (`Assertion: index < m_count`, `Array.h(587)`, stack in the `AttribTitles`
   render path; `Rurik-main/studies/character/RUNS.md:177-186`). `0x00F4` field 2 uses the same
   rank table, so it should be treated the same way; its own render path is the open display
   question. Field 10 is different: in the portal-battery capture, Kurzick's base rank 74 is not
   among the rank records sent, and the client did not crash. Field 10 is therefore a display
   anchor rather than a hard reference. Sending its record anyway is the safe choice.
2. **`string16(8)` admits at most 7 units on receive.** A literal filling the field exactly to its
   declared cap is an instant `Code=007` hangup, no assert. Template framing spends 3 units
   (`0x0BA9, 0x0107` ... `0x0001`), so Rurik's store caps rank names at 4 characters
   (`Rurik-main/toolkit/authsrv/charstore.py:73-85, 100-101`;
   `questdefs.py:95-141`; `studies/character/RUNS.md:165-176`). The declared sizes support the
   explanation: `0x00F3` is 34 bytes = 2 header + 12 payload + a 20-byte string field
   (2-byte length + 9 u16 units), i.e. 8 characters plus a terminator slot. At-cap 8 leaves no
   room for the terminator, which is the measured hangup.
3. **`0x00F5` is a patch, not a create.** It only works after a `0x00F6` created the track and
   set the description pointer; otherwise it is inert
   (`studies/newopcodes/FINDINGS.md:1433-1437`).
4. **Do not reference a title id outside 0-47.** The client's `s_titleClientData` is 48 rows;
   Rurik's store refuses anything else (`charstore.py:324-327`).
5. **Do not confuse `max_rank` with a faction cap.** `0x00F6` field 9 is the title's own rank
   count. Kurzick/Luxon faction *caps* are the separate `0x00EA-0x00ED` messages
   (`Rurik-main/studies/character/STORAGE.md:203-206`).
6. **A maxed track needs a sentinel next rank.** Retail sends a rank record whose `value` is
   `0xFFFFFFFF`, references it as `next_rank_id`, and sets `next_min = 0xFFFFFFFF`. GWToolbox's
   progress code treats `points_needed_next_rank == 0xFFFFFFFF` as "maxed"
   (`TitleTrackerWidget.cpp:104-106`). The captures show this for Kurzick (rank 86), Treasure
   Hunter (359) and Wisdom (368).
7. **Retail sends only tracks with progress.** A zero-progress `0x00F6` is likely to make a row
   appear on the Hero Panel contrary to the game's own "track appears only after points" rule, and
   it costs rank records. The exception is a track the game shows at a threshold without progress
   (LDoA at level 12, Survivor at level 5) - those are client-side display rules, not zero rows.

Two things not measured:

- What `0x0059` (hide) makes the server send back. `0x00F4` with rank id 0? A rank sentinel? No
  capture shows the reply. GWToolbox's `RemoveActiveTitle()` sends `0x0059`
  (`PlayerMgr.h:27`), but the server side of that exchange is open.
- Whether `0x00F4` requires the rank to be in the *displayable* range. The client accepts the
  binding; the under-nameplate draw is the part that needs the rank record.

---

## 7. What each repository actually does

### OpenTyria - the catalogue, not the server

- `GmTitles.h:3-51` is the complete 48-id enum with a `Title_ToString`.
- `GmTitleConstData.c:1-634` has a `TitleConstData` for 42 tracks: type, rank slice (all rank
  arrays are placeholders `{0}`), and coded string buffers for the point name and description.
  The first coded id in each buffer decodes to a string id, and the name/description pairs are
  adjacent: `Asuran` 70086/70087, `Drunkard` 32105/32106, `Hero` 16420/16421. That is consistent
  with them being the title's name and description; the remaining code units are not plain text
  and are not decoded by OpenTyria.
- `db/seed.sql:159-170` creates an orphaned `titles` table
  `(account_id, char_id, title_id, current_rank, current_points)` - note it carries **both**
  scope keys, so it can represent either scope but does not choose one.
- `GameSrv.c:1472-1478` has `// GameSrv_SendPlayerTitles` and
  `// GAME_SMSG_TITLE_RANK_DISPLAY` - the send site is commented out and the function does not
  exist. OpenTyria scaffolds the data and sends nothing.

### GWLP-R - the protocol shells, no table

- `P231_TitleRankData`, `P232_TitleRankDisplay`, `P233_Unknown`, `P234_TitleTrackStatus`
  (`protocol/src/main/java/gwlpr/protocol/gameserver/outbound/`).
- `P233` sits between RankDisplay and TrackStatus and has the two-`long` shape of TitleUpdate,
  but the code generator named it `Unknown`; `P234` carries the nine-dword older track layout.
- `GWLP-R-Utils-master/Templates/PacketTemplates.xml:3076-3183` has the field names and
  descriptions. Header numbers 231-234 are an older build (see section 3.3).
- No title table, no persistence, no send path.

### gw-preservation - no title code, but the build-drift evidence

- No `Title` in Go source, no title table in `db/schema.go`, no title opcode handling.
- `data/fields_2008.txt` and `data/fields_2025.txt` are descriptor dumps that happen to include
  the title family. They are the clearest proof that the opcode numbers move:
  - 2008: `0xE0` 3xu32+string8, `0xE1` u16+u32, `0xE2` 2xu32, `0xE3` 9xu32+2xstring8.
  - 2025: `0xF2` 3xu32+string8, `0xF3` u16+u32, `0xF4` 2xu32, `0xF5` 9xu32+2xstring8.
  The shapes are identical; only the base changed, and both are 9-dword track records.

### Rurik - the only end-to-end implementation

- Account store: `toolkit/authsrv/charstore.py` keeps `title_ranks` and `titles` in the account
  object, validates both crash rules at load, and refuses a bad store loudly
  (`:73-85`, `:303-336`).
- Send path: `authsrv.py:43564-43594` sends every rank first, then each track with current/next
  rank ids and their values, `flags`, points, `rank_count`, `max_rank`, and two coded templates.
  Its `title_ranks` value serves as both the rank number and the threshold, which is safe for the
  percentage track it observed but would be wrong for a point track under correction 1.
- Probe: `probecharacter.py:946-1046` is the `title_track` experiment. It sends a verbatim retail
  rank, our ids, a 7-unit literal, a second rank, a full track, a `0x00F5` patch, and a `0x00F4`
  display. Its run records are `studies/character/RUNS.md:124-208`.
- Result: the client rendered the row **"Ruri (1,000)"**, bar at 6,000 of 8,400; the row name is
  the current rank's `0x00F3` string, not a compiled title name; `0x00F5` updated the bar; the
  unseeded-rank stress step crashed the client at render (`Array.h:587`).
- Field map: `studies/newopcodes/FINDINGS.md:1423-1512`; overrides
  `schema/overrides.json:2347-2385`.
- Retail traffic: `studies/character/STORAGE.md:172-251` - `0x00F3` x170, `0x00F4` x215 in
  towns, and `0x00F5` x5 / `0x00F6` x7 on the Factions character (title id 2).

### GWToolbox / GWCA - the client consumer and the data tables

- `TitleID` enum (`Constants.h:162-179`), `Title` / `TitleTier` / `TitleClientData`
  (`GameEntities/Title.h`), `WorldContext.titles` / `title_tiers`
  (`Context/WorldContext.h:303-304`).
- `PlayerMgr::SetActiveTitle`, `RemoveActiveTitle`, `GetActiveTitleId`, `GetTitleTrack`,
  `GetTitleData` (`Managers/PlayerMgr.h:25-55`) - the client-side calls that ultimately send
  CtoS 88/89.
- `TitleTrackerWidget.cpp:54-116` has max points per title and the progress-ratio logic:
  percentage tracks are `current_points / 1000`; maxed tracks are `1.0`; otherwise
  `current_points / per-title max`, falling back to `next_rank_points` in current-tier mode.
- `ToolboxUtils.cpp:108-192` maps maps/regions/continents to the title a player should wear
  (Luxon in The Deep, Kurzick in Urgoz's Warren, Deldrimor in the Depths, Lightbringer in
  Margonite maps, Vanguard in Charr territory, Asuran/Norn by region, Sunspear by Elona
  continent, etc.). This is the best available "which title is relevant here" table.
- `ChatCommands.cpp:2871-2908` implements `/title [id]`: pick the map's title, else the
  configured default; `RemoveActiveTitle()` then `SetActiveTitle(id)`.
- `BountyKillTrackerWidget.cpp:52-184` maps bounty skills to title ids and gives points per
  kill by bounty rank (rank 1 = 1, rank 2 = 2, rank 3 = 3, rank 4 = 4). This is the only
  earning-rate table in the repos.
- `CartographyData.h` / `CartographerWidget.cpp` bake the game's map exploration rules; useful
  for the Cartographer track's earning side, but they do not touch the title wire.

### Headquarter - the structs and the mode flags

- `code/client/title.c` has `RankData`, `RankDisplay`, `TrackInfo` behind size asserts. It does
  not consume the fields.
- `TrackInfo` names the modern ten-dword layout and is the only repo that names field 6:
  `int32_t title_mode_flags; // see TitleModeFlags`.
- `include/client/constants.h:258-262` gives `None/DhuumCovenant/ReforgedMode`.
- `code/client/title.h` has a simplified 4-int `Title { title_id, rank_max, rank_current,
  points_in_cur_rank, points_for_rank_up }`, not the client's 0x2C record.

### The remaining repos - no title-system code

`GuildWarsMapBrowser-master` is a `.dat` browser, `gw-interface-master` is a client controller, and
`WorldCompletion-main` is a map-exploration route planner. None of them contains title ids,
title packets, or title persistence. WorldCompletion is still useful as an *earning-side* reference
for the Cartographer tracks (it plans the ground you must uncover), but it does not touch the title
wire.

### ASURA - the foundation (2026-11-05)

- Wire: `crates/asura-protocol/src/game/title.rs` builds `0x00F3`-`0x00F6` and reads `0x8058`;
  a coded string over 7 units is refused, never cut.
- Rules: `crates/asura-rules/src/titles.rs` - the rung a point count is on, the load group
  (current record, next record, track), patch-or-resend on a point change, whether a title may be
  shown, and `validate` for the crash rules of section 6.
- Content: `config/Server/titles.json`, read by `asura_config::titles()`. A file that fails
  `validate` is refused whole and titles are off. Shipped with one row, title 21 (LDoA), every value
  copied from the portal-battery capture.
- State: `t_character_titles`, `t_account_titles` and `t_characters.display_title` (migration 0028).
- World: `crates/asura-world/src/titles.rs` - the title block after the player's `0x003C`, the
  player's own binding after `0x0022`, a peer's binding in its introduction, the `0x8058`/`0x8059`
  arms in `dispatch.rs`, and `add_points`, the one call an earning rule makes.
- Not built: any earning rule, and a reply to `0x8059` (section 11).
- **Correction.** This section used to say ASURA's world captures carry CtoS `0x0059` and that a
  live pre-Searing sniff shows 25 of them. Both are StoC `0x0059`, `GAME_SMSG_PLAYER_CREATE`:
  `recordings/asura/world/live-client-session-20261027-220258.jsonl` has one `StoC 0x0059` of 43
  bytes, and `recordings/client/feature/maps/pre-searing-load-3.jsonl`'s 25 are all `StoC` (the
  sniffer labels them `PLAYER_UPDATE_AGENT_INFO`). No recording holds a client title request.

---

## 8. Persistence - what the reference servers store

| Repo | Storage | Scope | State |
|---|---|---|---|
| OpenTyria | SQLite `titles` table `(account_id, char_id, title_id, current_rank, current_points)` | both keys present, unused | orphaned; no send path |
| gw-preservation | none | - | no title code |
| GWLP-R | none | - | protocol shells only |
| Rurik | JSON account store: `account.title_ranks`, `account.titles` | account-level in the store | validated at load, sent per connection |
| ASURA | Postgres `t_character_titles`, `t_account_titles`, `t_characters.display_title` (migration 0028) | one table per scope; the title file says which | read at connect, written on change |

Design notes for ASURA if it implements this:

- The wiki/OpenTyria split is real: Cartographer, Guardian, Protector, Skill Hunter, Vanquisher,
  Survivor, LDoA, Sunspear, Lightbringer, the EotN ranks, Drunkard, Party and Sweets are
  character-based; Hero, Gladiator, Champion, Kurzick, Luxon, Lucky, Unlucky, Gamer, Codex,
  Zaishen, Treasure Hunter, Wisdom and Commander are account-based. A single table with both keys
  (OpenTyria's shape) can represent both, but the scope should be explicit rather than inferred
  from a null.
- The Cartographer store already exists per character (`t_character_cartography`,
  `crates/asura-db/migrations/0027_character_cartography.sql`), and its comment explicitly says
  the Cartographer titles live on the character. A title store can consume `explored` directly
  rather than recomputing the percentage.
- `titles` rows should be written when progress changes, not only at logout; a title is
  operational state like cartography, not static content.

---

## 9. Earning rules - what is known and what is not

The wire carries *progress*, not *how progress was earned*. The repos contribute these earning
facts:

- **Cartographer** is a percentage track. Prophecies thresholds are 600/700/800/900/950/1000
  per-mille (`docs/36-game-rules-ledger.md:318`, E170; Py4GW `title_enums.py`). Build 39112's
  Canthan track uses the same shape: rank 1 at value 600, max rank 6.
- **KoaBD** thresholds are 5/10/15/20/25/30 maxed titles (wiki; the capture's `next_min = 5`
  and `max_rank = 6` agree).
- **Prophecies count tracks:** Protector of Tyria and Guardian of Tyria are 25 missions, Tyrian
  Vanquisher is 54 areas, Tyrian Elite Skill Hunter is 90 skills, and LDoA is one rank at level 20
  (`docs/36-game-rules-ledger.md:318, 322`, E170/E174; capture `next_min = 20`, `max_rank = 1`).
- **EotN bounties** give 1/2/3/4 points per kill by bounty rank
  (`GWToolbox BountyKillTrackerWidget.cpp:52-74`).
- **Max points** per track are in GWToolbox's table (section 2): Kurzick/Luxon 10M, Lucky 2.5M,
  Unlucky 500k, Hero/Zaishen 100k, Gladiator/Codex 200k, Gamer 135k, EotN 160k,
  Sunspear/Lightbringer 50k, Drunkard/Party/Sweets/Champion/Treasure Hunter/Wisdom 10k,
  Survivor 1,337,500, Master of the North 1,000, KoaBD 30.
- **Not known from the repos:** the per-track earning rates for most PvE titles, the exact counts
  for the Canthan and Elonian mission/area/skill tracks, the display-eligibility minimum for each
  title, and the account/character split for every track. `docs/36-game-rules-ledger.md:410`
  states this plainly: the record, tiers, display rules and Prophecies thresholds are evidenced;
  how points are earned per track, and the rates, are not.

The wiki fills some of the display rules (only one title at a time; hover/target in staging areas;
checkbox at the displayable rank; Hero Panel grouping maxed/displayable/not-yet), but it is not a
substitute for the server rules that ASURA would need to implement earning.

---

## 10. ASURA implementation plan

Built on 2026-11-05 (section 7, ASURA), with two departures from the text below, both toward the
capture: a load seeds the current and next records only, not `base_rank_id` (Kurzick's base 74 is
not sent by retail and did not crash the client, section 6 rule 1), and `0x8059` is stored and
answered with nothing until a capture shows the reply.

### 10.1 Data model

```rust
struct TitleRank {
    rank_id: u32,     // global key into the client's title_tiers array
    flags: u32,       // bit 0 = percentage
    value: u32,       // rank number, or per-mille threshold for percentage tracks
    name: String,     // coded string; <= 4 chars with template framing
}

struct TitleTrack {
    title_id: u8,          // 0..=47
    flags: u32,            // 0x00F6 field 2
    points: u32,
    current_rank: u32,
    current_min: u32,
    next_rank: u32,        // 0xFFFFFFFF when maxed
    next_min: u32,         // 0xFFFFFFFF when maxed
    max_rank: u32,
    base_rank_id: u32,     // field 10; rank-1 record
    mode_flags: u32,       // 0 none, 1 Dhuum, 2 Reforged
    point_template: String,
    desc_template: String,
}
```

Keep the rank table separate from the track table. A rank id can be referenced by a track or a
display binding, and a single rank record may be reused by more than one reference. The client's
tier array is sparse and keyed by id; the server's table should be explicit.

### 10.2 Persistence

Add `t_character_titles` (character scope) and `t_account_titles` (account scope), or one table
with an explicit `scope` column. Columns: `title_id`, `points`, `current_rank`, `updated_at`.
Do not store the derived next-rank thresholds; they are content, not state. Keep the rank
vocabulary (`rank_id`, `flags`, `value`, `name`) in content/config, because it is the same for
every character and changes with the game data, not with play.

### 10.3 Send sequence

On entering an instance (per character load):

1. For each track with progress, ensure the rank records for `current_rank`, `next_rank` (if not
   maxed) and `base_rank_id` exist in the outbound vocabulary. Emit `0x00F3` for any that are
   new.
2. Emit `0x00F6` for the track.
3. Emit `0x00F4` for the local player's active title, if any, after its rank record.
4. For other players in the instance, emit their `0x00F4` after their rank records. Retail does
   this for everyone in a town; the capture's 36 display packets are exactly that.

On a point change:

1. Patch the stored track.
2. Emit `0x00F5` with the new points, but only if the track has already been sent with a
   `0x00F6` in this connection. If not, send the full `0x00F6` instead.

On CtoS `0x8058` (set title):

1. Validate the title id (0-47) and that the track is displayable.
2. Choose the rank id for the current rank.
3. Broadcast `0x00F4` to the player and to the other players in the instance.

On CtoS `0x8059` (hide title):

1. Clear the active title.
2. Broadcast the hide. The exact retail reply is unmeasured; the safe implementation is a
   `0x00F4` with rank id 0 or a rank that renders nothing, but this needs a live measurement
   before it is called correct.

### 10.4 Validation

- Refuse to send a track that references an unseeded rank id (crash rule 1).
- Cap every rank/description string at 7 units, and at 4 characters when template framing is
  used. Do not truncate silently; refuse the row at load (Rurik's `charstore` is the model).
- Refuse title ids outside 0-47.
- Preserve unknown flag bits in `0x00F6` field 2; do not normalize 4 to 0.
- Set `mode_flags = 0` unless the server deliberately implements Dhuum/Reforged titles.

### 10.5 Test plan

- **Loopback render, Rurik-shaped:** send `0x00F3` rank 1 (`value=1000`, name "Ruri"), rank 2
  (`value=8400`, name "Eld"), `0x00F6` title 7 (`points=4200`, current 1, next 2,
  current_min=1000, next_min=8400), open the Hero Panel's Titles tab, and expect a row
  **"Ruri (1,000)"** with a bar at 4,200/8,400. This is Rurik's run 3, and it is the cheapest
  end-to-end proof.
- **Loopback render, retail-shaped:** the same record but rank values `1` and `2` (the rank
  numbers) with the thresholds only in `0x00F6`; expect **"Ruri (1)"** and the same bar. This is
  the shape build 39112's real tracks use (correction 1 in section 3.4).
  **Passed on the owner's live client, 2026-11-05**, with a retail record rather than a made-up
  one: `/title ldoa 13` sent title 21's two rank records (176, 177) and its track, copied from the
  portal-battery capture into `config/Server/titles.json`, and the Hero panel drew the Legendary
  Defender of Ascalon row correctly. That settles three things on this client: the packed layout
  (a `u16` count, then the units), rank records before the track, and the retail coded strings.
- **Patch:** send `0x00F5` title 7 -> 6,000 and confirm the bar moves without a reload.
- **Display:** send `0x00F4` player 1 -> rank 1 and target yourself in an outpost; expect "Ruri"
  under the nameplate. Rurik's version of this step was inconclusive because the scripted
  self-click became a ground move order, so this remains the one live loose end.
- **Crash guards:** a `0x00F6` referencing rank 999 must be refused by the server, not sent.
  A rank name of 8 units must be refused, not sent.
- **Order:** a load with 14 tracks should show those 14 rows, not all 48, and a fresh character
  should show only the tracks the game shows at creation.
- **Hide:** CtoS `0x8059` should remove the displayed title; measure the reply before trusting it.

---

## 11. Open questions and contradictions

| Question | Why it is open | What would answer it |
|---|---|---|
| What does the server reply to CtoS `0x8059` (hide)? | No capture shows the reply; GWToolbox only sends the request | live capture of a title toggle in a town |
| Is `0x00F6` field 10 a base rank id, a max rank id, or something else? | 14/15 observed tracks fit "rank 1 id"; Hero does not | a capture with a title whose rank block is non-contiguous, or a client disassembly of the field's use |
| Does `0x00F4` require the rank to be displayable, or only seeded? | The display path is not fully read | a display test with a rank below the title's display threshold |
| What are the per-track earning rates? | The wire carries progress, not causes; repos only have EotN bounties and max points | server-side content/rule data, or measurements per track |
| What is the exact wire byte layout of `string16(8)`? | Answered by the live client on 2026-11-05: a `u16` count and that many units, no terminator, is drawn correctly (section 10.5). The 7-unit cap is Rurik's measurement and is not re-measured here | a packed `asura-world` capture (`ASURA_CAPTURE`) of a `/title` run, to keep the bytes beside the result |
| Why does retail send only current/next ranks and not the whole block? | It is observed, not explained | client disassembly of the Hero Panel's rank lookup |
| Do all 48 titles use the same rank-id allocation? | Only 15 tracks observed | a capture with every title at a non-zero rank |
| Is `TitleModeFlags` per track or per character? | Headquarter names it in the track; OpenTyria has the same bits on the character | a capture with a Dhuum/Reforged character |

Known contradictions to keep visible:

- **Rurik vs build 39112 on fields 5/8.** Rurik's retail sample (title 2, percentage) made
  field 3 of `0x00F3` and fields 5/8 of `0x00F6` look identical. They are not for point tracks.
- **Rurik/GWLP-R/Headquarter vs build 39112 on field 10.** "Max rank id" does not fit the
  observed values; "base rank id" fits 14/15.
- **GWLP-R vs OpenTyria on the opcode numbers.** `P231-P234` are an older build; `0x00F3-0x00F6`
  are the modern ones. Both are correct for their client.
- **OpenTyria vs the modern track width.** OpenTyria's `TitleConstData` has no track-record shape
  at all, but its `P234`-era siblings and the 2025 dump use 9 dwords; build 39112 uses 10.
- **GWToolbox's `Title` comment.** The comment says 0x28; the `static_assert` says 0x2c. Trust the
  assert.

---

## 12. Source index

Paths are relative to the repository root. The third-party repos (`Rurik-main/`,
`OpenTyria-master/`, `GWToolboxpp-master/`, etc.) live under `Other_Repos_Research/`; the top
table in this note gives the full path for each.

**Wire and schemas**

- `crates/asura-protocol/src/packets.rs:422-423, 771-774, 1441-1445, 1990-2000`
- `docs/21-surrounding-packets.md:75-76`
- `docs/23-initialization-protocol.md:44, 58, 89, 163`
- `docs/24-stream-parity.md:94-96`
- `docs/36-game-rules-ledger.md:114-115, 300-323, 410`
- `docs/03-protocol.md` section 5e (client struct vs packed wire layer)
- `tools/sniff-capture.py` (client struct capture and sanitiser)
- `Rurik-main/schema/messages.json:11361-11460` (243-246)
- `Rurik-main/schema/overrides.json:2347-2385`
- `Rurik-main/studies/newopcodes/FINDINGS.md:1423-1512`
- `Rurik-main/studies/character/STORAGE.md:172-264`
- `Rurik-main/studies/character/RUNS.md:124-208`
- `Rurik-main/toolkit/authsrv/authsrv.py:43564-43594`
- `Rurik-main/toolkit/authsrv/charstore.py:73-85, 100-101, 303-336`
- `Rurik-main/toolkit/authsrv/probecharacter.py:946-1046`
- `Rurik-main/toolkit/authsrv/questdefs.py:60-141`
- `Headquarter/code/client/title.c:6-71`
- `Headquarter/code/client/title.h:6-14`
- `Headquarter/include/client/constants.h:258-262`
- `GameRevision-GWLP-R-4208ffa/protocol/src/main/java/gwlpr/protocol/gameserver/outbound/P231_TitleRankData.java`
- `.../P232_TitleRankDisplay.java`
- `.../P233_Unknown.java`
- `.../P234_TitleTrackStatus.java`
- `GWLP-R-Utils-master/Templates/PacketTemplates.xml:3076-3183`
- `preservation-main/data/fields_2008.txt:1829-1844`
- `preservation-main/data/fields_2025.txt:1873-1897`

**Client model and data**

- `GWToolboxpp-master/Dependencies/GWCA/include/GWCA/Constants/Constants.h:162-179`
- `.../GameEntities/Title.h:11-43`
- `.../Context/WorldContext.h:303-304`
- `.../Managers/PlayerMgr.h:25-55`
- `.../Packets/StoC.h:1001-1020`
- `GWToolboxpp-master/GWToolboxdll/Widgets/TitleTrackerWidget.cpp:54-116`
- `.../Widgets/BountyKillTrackerWidget.cpp:52-184`
- `.../Utils/ToolboxUtils.cpp:108-192`
- `.../Modules/ChatCommands.cpp:2871-2908`
- `.../Windows/PlaystyleRestrictionsWindow.cpp:89-95` (campaign->title groups)
- `.../Widgets/CartographyData.h` and `CartographerWidget.cpp` (earning side only)

**Catalogue and persistence**

- `OpenTyria-master/code/OpenTyria/GmTitles.h:3-116`
- `.../GmTitleConstData.c:1-634`
- `.../GmTitleConstData.h:3-46`
- `.../opcodes.h:99-100, 280-283`
- `.../GameSrv.c:1472-1478`
- `.../db/seed.sql:159-170`
- `crates/asura-db/migrations/0027_character_cartography.sql:1-63`
- `crates/asura-db/src/cartography.rs:1-80`

**Captures**

- `recordings/client/feature/travel/portal-battery-20261031-013856.jsonl`
- `recordings/client/feature/travel/login-to-portal-20261031-003720.jsonl`
- `recordings/client/feature/travel/doors-20261031-095754.jsonl`
- `recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`
- `recordings/client/feature/maps/pre-searing-load-3.jsonl`
- `recordings/asura/world/live-client-session-*.jsonl` (only CtoS `0x0059`)

**Wiki (game rules, not wire)**

- <https://wiki.guildwars.com/wiki/Title>
- <https://wiki.guildwars.com/wiki/Kind_of_a_Big_Deal>

---

## 13. Content without a capture (2026-11-05)

What a title entry in `config/Server/titles.json` needs, and where each piece comes from when no capture
of that title exists.

| Piece | Source | Evidence |
|---|---|---|
| `progress_template`, `description_template` | OpenTyria `GmTitleConstData.c` (42 titles) | Identical to retail, both strings, on 13 of 13 captured titles (0, 2, 3, 5, 6, 10, 15, 16, 23, 34, 44, 45, 46; portal-battery and necro_factions captures) |
| Rank ids | This server's choice | The client keys its rank table on the ids it is sent; Rurik's own ids rendered (`studies/character/RUNS.md` Run 3). Retail numbers a title's rungs as one block: rank 0, rank 1, ..., then the maxed record |
| Rank 0 record | `value 0`, name `0101` | 6 of 6 captured rank-0 records (45, 118, 150, 176, 273, 337) |
| Maxed record | `value 4294967295`, name `0101`, id = last rank + 1 | 3 of 3 captured (Kurzick 85 -> 86, Treasure Hunter 358 -> 359, Wisdom 367 -> 368) |
| Thresholds | Wiki; Prophecies Cartographer from `docs/36` E170 | Drunkard 1,000 / 10,000 (wiki) |
| Rank name, string id | Original ranks: the title's description string id + rank number | Hero 6 and 7, Gladiator 1, 2 and 6, Kurzick 1, Lucky 3 and 4, Unlucky 4 and 5, Gamer 1, KoaBD 1, LDoA 1, Zaishen 1, Wisdom 1 and 7, Treasure Hunter 7, Drunkard 2. Ranks added later use other ids (Kurzick 12 `8102 20DC`, Luxon 11 and 12, KoaBD 2 `8101 49CC`, Zaishen 12) |
| Rank name, trailing words | Captures only | Every named rank carries three words after its id (one continued number, top bit set on the first two), fixed per string (Hero rank 6 identical in two loads) and unrelated between neighbouring ranks. Whether the client needs them to draw the name is the test below |

Cartographer rank names are not next to their templates: Canthan Cartographer's rank 1 is `6C85`, its
templates are `7D87`/`7D88`.

### The shipped entries

- **Legendary Defender of Ascalon (21).** Every value captured, except the maxed record's id 178, which
  follows the last-rank-plus-one numbering.
- **Drunkard (7).** Templates from OpenTyria. Rank 134 (rank 2, Incorrigible Ale-Hound) is captured
  whole: `7E6C BB0E C315 37C5`, `recordings/client/feature/travel/login-to-portal-20261031-003720.jsonl`,
  shown by player 63. Rank ids 132, 133 and 135 follow the numbering around it. Track flags 0, as
  Sweets', the other two-rank character points title. Rank 133's name is `0101` (empty) until a
  capture carries it: its id is `7E6B`, its trailing words are not captured.
- **Tyrian Cartographer (1).** Templates from OpenTyria; flags 1 and value = minimum in per-mille, as
  Canthan Cartographer's captured records; thresholds 600/700/800/900/950/1000. Rank ids 900-907 are
  this server's own. The six rank names are `0101` (empty) until a capture carries them. Earned by pre-Searing exploration on the owner's rule (2026-11-05):
  per-mille of the blocks of `cartography::pre_searing_union` the character has explored. Retail counts
  no pre-Searing area toward a Cartographer title, and the union includes ground nobody walks, so a
  full walk stays below 1000.

- **Survivor (9).** Captured on 2026-11-05
  (`recordings/client/feature/titles/titles-live-20261105-232119.jsonl` seq 3683): track flags 0,
  max rank 3, base 146, rank 145 -> 146 with rank 1 at 140,600; rank 146 `7E7A B15E C09D 371B`. Rank
  ids 147 and 148 and the maxed 149 fill the gap to Kind of a Big Deal's 150; a district player shown
  with rank 147 in `login-to-portal` is Survivor rank 2. Ranks 2 (587,500) and 3 (1,337,500) are the
  wiki's thresholds with names `0101` until captured.

- **Capture `titles-live-20261105-232508.jsonl`** (10,473 packets, 263 s, district hopping): 76 rank
  records, 16 tracks, 83 display bindings. It confirms the Drunkard entry field for field (the
  character's own track: flags 0, 132 -> 133, next 1,000, max rank 2, base 133) and carries rank 133
  `7E6B E7B4 C2FE 4B26` (Drunkard), 148 `7E7C 864C 9BAF 7C45` (Legendary Survivor) and retail's Tyrian
  Cartographer track (flags 1, 179 -> 180, max rank 6, base 180; rank 180 `6C7D D89D BA33 4EFC`), so the
  Cartographer entry now uses retail's ids 179-185 and maxed 186 (Protector of Tyria starts at 187).
  Sweet Tooth (34: 273, 274, track) and Kind of a Big Deal (10: 151, 152, 154, 156; thresholds 5 per
  rank) are added from it. Still uncaptured: Drunkard none, Survivor rank 2 (147), Sweet Tooth rank 2
  (275), Kind of a Big Deal ranks 3 and 5 (153, 155), Cartographer ranks 2-6 (181-185), and every Party
  Animal rank.

### Result, owner's live client, 2026-11-05

- **The trailing words are needed.** `/title drunkard 1000` drew the row as `M]f?aT 6 (1)` (screenshot
  `gw013.jpg`): the id alone decodes to garbage, so a rank name is its id *and* its three words, and a
  name that was never captured cannot be built. The "(1)" and "1,000 points" are right.
- **OpenTyria's templates draw.** Below rank 1 the rows read "Defender of Ascalon Title Track (no title
  earned)" and "Tyria Exploration Title Track (no title earned)": a rank-0 track needs no rank name,
  only the two templates, so every OpenTyria title is complete up to its first rank.
- **Exploration earning works.** The Cartographer row read "14.0% explored" from the stored grid.
- The Cartographer rank-name guess is not tested: after the Drunkard result a bare id garbles whatever
  its layout, so the guess was replaced by `0101`.
- The capture `titles-live-20261105-232119.jsonl` (5,690 packets, 120 s) also carries rank-0 and
  rank-1 records for Kurzick (73, 74), Lucky (101, 102), Treasure Hunter (351, 352) and Wisdom
  (360, 361), Hero ranks 4 and 9 (records 4, 9) and Gladiator rank 4 (21). Its templates for titles 5,
  9, 15, 45 and 46 match OpenTyria: 14 of 14 now.

### What the two tests answer

- `/title drunkard 1000` - the Hero panel row reads "Drunkard": a string id without its trailing words
  draws, and every original rank name of the 42 OpenTyria titles is derivable. Blank or garbled: the
  trailing words are needed and the remaining names come from captures.
- `/title carto 600` - the row reads "Tyrian Explorer": the Cartographer layout guess holds. Another
  name: the layout is something else, and the name shown says where. Blank: as above, or the guess is
  wrong; the Drunkard test separates the two.
