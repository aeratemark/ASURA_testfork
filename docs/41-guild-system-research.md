# The guild system: first captures and the message family

**Status:** research note, 2026-11-06; the first implementation is section 6.
**Question:** what the live server sends and accepts when a guild is created, a player is invited, an
announcement is set, a guild hall is left and a guild is left - and what this repository owes.

The answer: guild state is one family of about fifteen messages, `0x0115`-`0x0134` server-to-client
plus five client requests (`0x0036`, `0x00B2`, `0x00B5`, `0x00BB`, `0x00BE`). Every guild request
except creation carries a request id, and the server answers each one with `0x0132 {request_id,
result}`. The client headers below are written as the sniffer prints them; the packet table and the
code carry them with the direction bit (`0x8036`, `0x80B5`, `0x80BB`, `0x80BE`).

Tags, as in `docs/40-titles-system-research.md`:

- **OBSERVED** - read from the two captures below (retail server, sniffer struct layer).
- **SOURCED** - read from another repo's table or source; the repo is named.
- **GUESS** - an inference; it says what would confirm it.

## 1. The recordings

| File | Taken | Sanitised | What happens in it |
|---|---|---|---|
| `recordings/client/feature/guild/entering, leaving guildhall and leaving guild.jsonl` | 2026-11-06 00:32:43 UTC, 990 records | yes | leave guild hall -> Kaineng Center (map 194); leave guild |
| `recordings/client/feature/guild/guild created, people invited.jsonl` | 2026-11-06 00:35:15 UTC, 6560 records | **no (raw)** | create guild "tgne"; five invites (four fail, one succeeds); two announcements |

- The second file is a `--raw` capture: guild UUIDs and other players' guild tags are in it. Its own
  header says "do not commit". Re-capture sanitised or keep it out of git.
- The leave capture is the earlier of the two, so the order of events was: leave guild hall, leave the
  old guild, create the new one.
- Entering the guild hall is not in either file; the first capture starts at the leave request (seq 6).
- Both files are the sniffer's **struct layer** (`header_len` 4). Per `AGENTS.md` section 0 and
  `docs/03-protocol.md` 5e, a struct size is not a wire size. Wire layouts below come from Rurik's
  `schema/messages.json` (build 38797) and are tagged SOURCED; every struct size in the captures matches
  that layout with 4-byte field slots.
- The struct buffer is reused: bytes after a string's terminator are leftovers from the previous
  message. Example: the empty announcement `0x012F` at creation (seq 323) carries the tag bytes of the
  `0x011F` before it. Read a field only up to its terminator.

## 2. Message table

Wire layout column: SOURCED from `Other_Repos_Research/Rurik-main/schema/messages.json`. Names: SOURCED
from `Headquarter/code/client/opcodes.h` and GWCA `Packets/Opcodes.h` where they exist; otherwise ours,
marked GUESS.

### Client to server

| Header | Name | Wire layout | Struct seen | What the capture shows |
|---|---|---|---|---|
| `0x0036` | GUILD_CREATE (GUESS) | dword, string16[32], string16[6] | 84 | seq 308: dword = 100, name masked, tag "tgne". Answered by the creation burst (section 3.1). |
| `0x00B0` | PARTY_ENTER_GUILD_HALL (SOURCED, Headquarter) | blob[16], byte | - | not captured |
| `0x00B2` | PARTY_LEAVE_GUILD_HALL (SOURCED, Headquarter) | byte | 8 | leave seq 6: byte = 1 |
| `0x00B5` | GUILD_INVITE (GUESS) | dword request_id, string16[32] name, byte | 76 | five sends, request_id 1..5, byte = 2 every time |
| `0x00BB` | GUILD_LEAVE (GUESS) | dword request_id | 8 | leave seq 721: request_id = 1 |
| `0x00BE` | GUILD_SET_ANNOUNCEMENT (GUESS) | dword, string16[256] | 520 | seq 5575 and 6069: dword = 0, text masked |

`0x00B3`, `0x00B4`, `0x00B8`, `0x00BA`, `0x00BD` share the `dword, string16` shape (SOURCED, schema) and
are the likely kick / promote / demote / guest requests. Not captured; unnamed.

### Server to client

| Header | Name | Wire layout | Struct seen | What the capture shows |
|---|---|---|---|---|
| `0x0118` | GUILD_PLAYER_ROLE (SOURCED) | word local_id, blob[16] guild key, byte role | 28 | seq 321: local_id 180, role 5 for the founder |
| `0x0119` | (unnamed) GUESS: "you have no guild" | header only | 4 | leave seq 731, right after GUILD_PLAYER_REMOVE |
| `0x011C` | (unnamed) GUESS: guild stream end | header only | 4 | seq 326, last message of the creation burst |
| `0x011F` | (unnamed) GUESS: guild history entry | dword, string16[64] | 136 | seq 322: dword 0x0023B4FA (same value as the founder's join date), encoded string |
| `0x0121` | GUILD_GENERAL_INFO (SOURCED) | section 2.1 | 160 | sent for every guild a player in view belongs to; 19 times in the raw file |
| `0x0127` | GUILD_PLAYER_INFO (SOURCED) | section 2.2 | 272 | founder at creation (seq 324); the invitee after the successful invite (seq 3351) |
| `0x0128` | GUILD_PLAYER_REMOVE (SOURCED) | string16[20] account name | 44 | leave seq 728 (redacted) |
| `0x012F` | (unnamed) GUESS: guild announcement | string16[256] text, string16[20] author | 556 | creation (empty text) and after each `0x00BE` |
| `0x0131` | (unnamed) GUESS: guild stream start | header only | 4 | seq 318, first guild message after creation |
| `0x0132` | (unnamed) GUESS: guild request result | dword request_id, byte result | 12 | answers `0x00B5`, `0x00BB`, `0x00BE` (section 3.2) |
| `0x0133` | (unnamed) | dword | 8 | values 0x0200, 0x2C00, 0x2F00, 0x3000, 0x3100; see section 4 |

Not guild messages, present in the same window: `0x0115` (dword, dword; values 8-23), `0x008C`, `0x0071`.
Leave them out of the guild family until a capture ties them to it.

### 2.1 `0x0121` GUILD_GENERAL_INFO field map

Wire (SOURCED, schema): word, blob[16], string16[32], string16[6], byte x7, word, byte, byte, dword x4,
byte. Field names SOURCED from `Headquarter/code/client/guild.c` `HandleGuildGeneralInfo`; values
OBSERVED in the raw capture.

| # | Field | Founder's new guild (seq 319) | A guild with a cape (seq 1148) |
|---|---|---|---|
| word | local_id | 180 | 5 |
| blob[16] | guild key (the guild hall key GWCA calls `GHKey`) | present | present |
| string16[32] | name | masked | masked |
| string16[6] | tag | "tgne" | "Fire" |
| byte | feature | 0 | 40 |
| byte | territory | 2 | 0 |
| byte x5 | cape background, detail colour, emblem colour, shape, detail | 0 0 0 0 0 | 240 175 255 3 29 |
| word | cape emblem | 0 | 155 |
| byte | cape trim | 0 | 0 |
| byte | allegiance | 4 | 0 |
| dword | faction points | 0 | 79061 |
| dword | qualifier points | 0 | 0 |
| dword | rating | 1000 | 1000 |
| dword | rank | 0 | 0 |
| byte | unknown | 0 | 0 |

- A new guild has no cape (all cape fields 0), allegiance 4, rating 1000, rank 0.
- `feature` is 40 or 20 on every established guild seen and 0 on the new one. GUESS: guild hall
  ownership or tier; a capture after buying a hall would settle it.
- `territory` is 2 on the founder's own guild (and one other guild, "FG", seq 5159), 0 elsewhere.
- One ranked guild (seq 2680) shows rating 923 and rank 326, so rank is a ladder position.

### 2.2 `0x0127` GUILD_PLAYER_INFO field map

Wire (SOURCED, schema): string16[20] x3, string16[64], dword x4, byte. Names SOURCED from GWCA
`StoC.h` `GuildPlayerInfo`.

| Field | Founder (seq 324) | Invitee (seq 3351) |
|---|---|---|
| invited_name, current_name, invited_by | masked | masked |
| context_info[64] | - | - |
| unk | 0 | 0 |
| minutes_since_login | 0 | 0 |
| join_date | 0x0023B4FA | leftover bytes, not read |
| status | 1 | 0 |
| member_type | 5 | 1 |

- `member_type` 5 = leader (also `0x0118` role 5). 1 = invited, not yet accepted (GUESS: the invitee's
  acceptance would move it to a member value; the invitee's own `0x0123` GUILD_INVITE_RECEIVED needs a
  second account in the capture).
- `status` 1 = online, 0 = offline (SOURCED, Headquarter `complete_guildmember_update`).

## 3. Sequences

### 3.1 Create (raw file, t = 15.3 s)

1. CtoS `0x0036` {100, name, "tgne"} (seq 308).
2. `0x014F` GOLD_CHARACTER_REMOVE {0xFA, 100} - creation costs 100 gold. 0xFA is the inventory key
   (Rurik `merchant.py`: `0x014F`'s first field is the `0x0144` stream key).
3. `0x005D` + `0x005E` - a server chat line (encoded string; content not decoded).
4. `0x00CC` TRANSACTION_DONE {8} - 8 is the vendor window kind for Guild Registration
   (`docs/42-vendor-system-research.md` section 2), so creation is a vendor transaction at the
   registrar's window.
5. `0x0131` (header only).
6. `0x0121` GUILD_GENERAL_INFO for the new guild (local_id 180).
7. `0x009F` AGENT_PROPERTY_UPDATE_INT {30, founder's agent 0x2F9, 180}: agent property 30 is the agent's
   guild local id. The same message follows every guilded peer's `0x0121` in a load
   (`pre-searing-load-3.jsonl` seq 301) and goes to 0 on leaving (leave file seq 730).
8. `0x0118` GUILD_PLAYER_ROLE {180, key, 5}.
9. `0x011F` {date, coded string}: the string is `0x0345`, `0x0107` (literal start), the founder's name,
   `0x0001` (literal end). GUESS: "founded by".
10. `0x012F` announcement (empty).
11. `0x0127` GUILD_PLAYER_INFO for the founder (status 1, type 5).
12. `0x0133` {0x0200}.
13. `0x011C` (header only).

No `0x0132` follows creation; the dword 100 in `0x0036` is not echoed as a request id. GUESS: the dword is
the quoted price, as field 2 of the merchant buy `0x004D` is (`docs/42` section 3); it equals the gold
removed. A capture that includes talking to the registrar (expect `0x00C3 [8, 0]`) confirms it.

### 3.2 Invite (raw file, five attempts)

| Attempt | Request seq | `0x0132` result | Gold | Other replies |
|---|---|---|---|---|
| 1 | 712 | {1, 14} | -100 then +100 (`0x014F`, `0x0140`) | a server chat line (seq 944/945) |
| 2 | 1103 | {2, 14} | -100 then +100 | - |
| 3 | 1502 | {3, 14} | -100 then +100 | - |
| 4 | 1824 | {4, 14} | -100 then +100 | - |
| 5 | 3338 | {5, 0} | -100, not returned | `0x0127` for the invitee, `0x0133` {0x2F00} |

- Result 0 = success; 14 = refused. The owner knows why attempts 1-4 failed (wrong name, offline, already
  guilded); that reason names code 14.
- Every invite is charged 100 gold and the charge is refunded when the server refuses: `0x0140` is a
  credit, not a balance (`docs/42` section 3), so the four refused attempts cost nothing. This contradicts
  what is generally known of the game (no invite fee). Not built: guilds are free (section 6).
- The order differs from the announcement's and the leave's: on an invite `0x0132` comes **before**
  `0x0127` and `0x0133` (seq 3350-3352).
- The invitee row (seq 3351): invited_name the invitee, current_name empty, invited_by the inviter,
  join_date `0x0024B4FA`, status 0, member_type 1.

### 3.3 Announcement (raw file, seq 5575 and 6069)

CtoS `0x00BE` {0, text} -> `0x012F` {text, author} -> `0x0133` {0x3000 / 0x3100} -> `0x0132` {0, 0}.

### 3.4 Leave guild hall (leave file, seq 6)

CtoS `0x00B2` {1} -> `0x01DA` -> `0x01A6` TRANSFER_GAME_SERVER_INFO (map 194) -> `0x0099`
MAP_UPDATE_CURRENT 194 -> a normal instance load. In the new outpost three `0x0121` arrive
(seq 331-381) between the agent stream and `INSTANCE_LOAD_FINISH`. The schema gives `0x01DA` as header
only but the struct is 256 bytes; the header numbering above `0x01A0` may differ from build 38797.
Confirm before porting.

### 3.5 Leave guild (leave file, seq 721)

CtoS `0x00BB` {1} -> `0x0128` GUILD_PLAYER_REMOVE {own account name} -> `0x0133` {0x2C00} -> `0x0119`
-> `0x0132` {1, 0}.

## 4. Open questions

Settled in this note's own captures:

- **The date dword** in `0x011F` and `0x0127`: the day since 1900-01-01 (UTC) in the low word, the
  minute of that day in the high word. `0x0023B4FA` is 2026-11-06 00:35 UTC, the creation; `0x0024B4FA`
  is 00:36, the invitation. Pinned by `asura_protocol::game::guild` test
  `the_date_is_days_since_1900_and_the_minute_of_the_day`.
- **A load with no guild** gets `0x011C` alone, right before the party block (`multiplay/party.jsonl`
  seq 2661); a load with a guild gets `0x0121`, `0x009F {30, ...}`, `0x0118`, `0x011C` there
  (`titles-live-20261106-034507.jsonl` seq 838-841, `titles-live-20261106-025940.jsonl` seq 1095-1098).

Still open:

1. `0x0133`'s dword: 0x0200 at creation, 0x2C00 on leaving, 0x2F00 / 0x3000 / 0x3100 on later changes.
   It moves in steps of 0x100. Unknown; reproduce the values verbatim until it is named.
2. `0x0036`'s dword (100): the quoted price (GUESS, section 3.1).
3. `0x0121` `feature` and `territory`.
4. Code 14 in `0x0132`, and whether the invite fee is real.
5. Pre-Searing scope: which guild messages the reference sends to a guilded pre-Searing character on
   load. A capture of a guilded pre-Searing character entering Ascalon City answers it. The game
   rule is known: a Guild Registrar stands in pre-Searing Ascalon City, and there is no guild hall
   access from the mainland (WIKI, via `Other_Repos_Research/Rurik-main/studies/presearing/MANIFEST.md`
   line 838).

## 5. Next captures, in order

1. A **sanitised** load of a guilded character into an outpost, wire layer (`asura-world --capture`
   against the reference, or a GWPS wire recording), to get wire sizes for `0x0118`, `0x0121`,
   `0x0127`.
2. Two accounts: invite, the invitee's `0x0123`, accept, and the roster changes on both sides
   (`0x012A`-`0x012D`).
3. Kick, promote and demote - names `0x00B3`/`0x00B4`/`0x00B8`/`0x00BA`/`0x00BD`.
4. Enter a guild hall (`0x00B0`), so the full hall round trip is on file.
5. Guild chat (`@`) from both sides.

## 6. What is built (2026-11-06)

Owner's rulings for this round, recorded in `docs/HANDOFF.md`'s decisions table: guilds are **free**, and
gold is a placeholder; creation opens from the chat command **`/guild create`** instead of a Guild
Registrar NPC.

| Piece | Where | What it does |
|---|---|---|
| Wire | `crates/asura-protocol/src/game/guild.rs` | builders for `0x0118`, `0x0119`, `0x011C`, `0x011F`, `0x0121`, `0x0127`, `0x0128`, `0x012F`, `0x0131`, `0x0132`, `0x0133`, `0x00CC` and the registration window; readers for `0x8036`, `0x80B5`, `0x80BB`, `0x80BE`; the date encoding |
| Rules | `crates/asura-rules/src/guild.rs` | who may create, invite, leave, announce |
| Storage | migration `0029_guilds.sql`, `crates/asura-db/src/guilds.rs` | `t_guilds`, `t_guild_members`, `t_guild_invites` |
| Values | `config/Server/guilds.json` | a new guild's bytes, the `0x0133` start and step, the placeholder prices |
| World | `crates/asura-world/src/guild.rs` | the load's guild block, a peer's badge, `/guild create`, the four requests |

Batches as sent, against the captures:

| Trigger | Sent | Differs from the capture |
|---|---|---|
| any load | `0x011C`, or `0x0121`, `0x009F {30}`, `0x0118`, `0x011C`, before `PARTY_CREATE` | none known; the roster and history block a load sometimes carries (`pre-searing-load-3.jsonl` seq 720-741) is not sent |
| a guilded peer in view | `0x0121`, `0x009F {30}` right after its player create | none known |
| `/guild create` | `0x00C4 {own agent}`, `0x0084 {[]}`, `0x00CA {1, 1.0f}`, `0x00C3 {8, 0}` | **no capture**: Rurik's merchant opener with kind 8 (`docs/42` sections 1-2), an empty stock list, and the player's own agent as the window's owner. GUESS; the live client decides |
| `0x8036` create | `0x00CC {8}`, `0x0131`, `0x0121`, `0x009F`, `0x0118`, `0x011F`, `0x012F`, `0x0127`, `0x0133 {0x0200}`, `0x011C` | no `0x014F` (free); no `0x005D`/`0x005E` chat line (its coded content is not decoded). A refusal sends nothing |
| `0x80B5` invite | `0x0132 {id, 0}`, `0x0127` (the invited row), `0x0133` | no `0x014F` (free). A refusal is `0x0132 {id, 14}` without the gold pair. The invitee is told nothing |
| `0x80BB` leave | `0x0128`, `0x0133`, `0x009F {30, agent, 0}`, `0x0119`, `0x0132 {id, 0}` | a leader leaves only as the last member, and the guild is deleted (no capture of a leader leaving) |
| `0x80BE` announcement | `0x012F {text, author}`, `0x0133`, `0x0132 {id, 0}` | other online members are not told |

This server's choices where no capture answers, each a GUESS:

- `0x0132` result 14 for every refusal: the only refusal code captured.
- `0x0133` is a per-guild counter: `0x0200` at creation, `+0x0100` per change. The live value went from
  `0x0200` to `0x2F00` between the creation and the first accepted invitation, which a per-guild counter
  does not explain.
- Only the leader invites and announces: officer and member types are not measured.
- "Online" for an invitation is any instance of this world process.
- A guild's local id is the lowest free id in the instance, kept for the instance's life.
- Others in the instance see the founder's badge after a creation, and property 30 go to 0 after a leave.

Not built: the invitee's `0x0123` and the accept (section 5 item 2), kicks, promotions and demotions
(item 3), guild halls (`0x80B0`, `0x80B2`; out of scope in pre-Searing), guild chat (item 5), capes, and
the roster and history beyond the founder's own row.

## 7. Sources

- `recordings/client/feature/guild/*.jsonl` (this repo).
- `Other_Repos_Research/Rurik-main/schema/messages.json` - wire layouts, build 38797. It is imported from
  OpenTyria's `msgdefs.c` and checked against a client dump (its own provenance note), so it is one
  lineage, not two.
- `Other_Repos_Research/Headquarter/code/client/guild.c`, `opcodes.h` - client-side handlers for
  `0x0118`, `0x0121`, `0x0127`, `0x012A`-`0x012D`, field names.
- `Other_Repos_Research/GWToolboxpp-master/Dependencies/GWCA/include/GWCA/Packets/StoC.h`,
  `GameEntities/Guild.h` - `GuildGeneral`, `GuildPlayerInfo`, `Guild`, `GuildPlayer`, `CapeDesign`.
- No server in `Other_Repos_Research` implements guilds (Rurik sends empty guild ids on purpose; Entice
  planned guilds in milestone 14 and never built them).
