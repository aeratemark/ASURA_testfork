# 44 - Party invitations: what a live server sends

Measured 2026-11-06 against the live servers in pre-Searing Ascalon (map 148), with the client-side
sniffer (`tools/sniff-capture.py`, struct layer). Every size below is the **wire** size from the
client's own table (`crates/asura-protocol/src/packets.rs`); the sniffer's sizes are its widened
struct and are not used (`docs/03-protocol.md` 5e). Code: `crates/asura-world/src/party.rs`, rules in
`crates/asura-rules/src/party.rs`.

## The recordings

| file (under `recordings/client/feature/`) | side | what it holds |
|---|---|---|
| `titles/titles-live-20261106-025940.jsonl` | invited (player `0x26`) | leave (seq 2432), invitation received (2539), accept (2617) |
| `titles/titles-live-20261106-032434.jsonl` | inviter | invitation (375), cancel (1295), invitation (1818), decline by the other (2058) |
| `titles/titles-live-20261106-030218.jsonl` | inviter | invitation of a party with a hero (116), a join of the other party (201), leave (2629) |
| `multiplay/party.jsonl` | inviter | the other side accepts (seq 160), and a party load after a doorway (2662) |
| `titles/titles-live-20261106-034507.jsonl` | leader (player 1) | kick (seq 2990), leader leaves (3304), decline by this side (2620), an invited player leaving the map (2645) |
| `titles/titles-live-20261106-034032.jsonl` | ASURA client | leader `0x80A2` and kick `0x80A9 {2}`, answered with nothing by this server at the time (400-411, 931-964) |
| `titles/titles-live-20261106-024622.jsonl` | ASURA client | `0x80A0 {player 1}` twice, ignored by this server at the time (973, 1029) |

The sniffer's labels for headers `0x01C3`-`0x01D4` are **one opcode off** (it calls `0x01D3`
"stream end" and `0x01CA` "you are leader"). The batch order settles it for the packet table:
`0x01D3`, adds, `0x01D4` brackets a party (`party.jsonl` seq 2665-2668), so `0x01D3` is the create.
The rules ledger (E35-E45) uses the older numbering, one lower than this build in this block.

## The messages

All party messages name the **other** party from the receiving client's point of view.

| header | name in the table | wire | fields | meaning, measured |
|---|---|---|---|---|
| `0x80A0` | `PARTY_INVITE_PLAYER` | 4 | `u16` player | invite this player |
| `0x809D` | `PARTY_ACCEPT_CANCEL` | 4 | `u16` party | withdraw my invitation of that party |
| `0x809C` | `PARTY_ACCEPT_INVITE` | 4 | `u16` party | accept that party's invitation |
| `0x809E` | `PARTY_ACCEPT_REFUSE` | 4 | `u16` party | decline it |
| `0x80A2` | `PARTY_LEAVE_GROUP` | 2 | - | leave my party; the client follows it with `0x801F {0x28}`, not handled and not yet named |
| `0x80A9` | `PARTY_KICK_PLAYER` | 4 | `u16` player | put this member out |
| `0x01C5` | `PARTY_INVITE_ADD` | 4 | `u16` party | my party invited that one |
| `0x01C6` | `PARTY_JOIN_REQUEST` | 4 | `u16` party | that party invites mine |
| `0x01C7` | `PARTY_INVITE_CANCEL` | 4 | `u16` party | my invitation of it is closed |
| `0x01C8` | `PARTY_REQUEST_CANCEL` | 4 | `u16` party | its invitation of me is withdrawn (not captured) |
| `0x01C9` | `PARTY_REQUEST_RESPONSE` | 4 | `u16` party | I answered its invitation |
| `0x01CA` | `PARTY_INVITE_RESPONSE` | 4 | `u16` party | it answered my invitation |
| `0x01CB` | `PARTY_YOU_ARE_LEADER` | 3 | `u8` | `{1}` to the member that takes over a party |
| `0x01CC` | `PARTY_PLAYER_ADD` | 7 | `u16` party, `u16` player, `u8` flags | a member; flags 1 for a member in the map |
| `0x01D0` | unnamed | 6 | `u16` party, `u16` player | that player left the map (ledger E38 `MemberLeaveWorld`, one opcode up) |
| `0x01D1` | `PARTY_PLAYER_REMOVE` | 6 | `u16` party, `u16` player | a member is gone |
| `0x01D3` / `0x01D4` | `PARTY_CREATE` / `PARTY_MEMBER_STREAM_END` | 4 | `u16` party | bracket a party's member stream |
| `0x01B3` | `CHARACTER_UDPATE_PARTY` | 5 | `u16` party, `u8` | my party; 1 when I lead it, 0 when I do not |
| `0x01E0` | `PARTY_SEARCH_SEEK` | 4 | `u16` | `{0}` after every change of my own party; meaning not established |
| `0x01BF` | `PARTY_SET_DIFFICULTY` | 3 | `u8` | `{0}` right after `0x01E0` |
| `0x00B0` | `PLAYER_UPDATE_PARTY_SIZE` | 5 | `u16` leader, `u8` size | a party's leader and size, to everyone in the map |
| `0x00B1` | `PLAYER_UPDATE_PARTY` | 6 | `u16` player, `u16` leader | a player's leader, to everyone in the map |

## The batches

Each is one server tick.

**Invitation** (`032434` seq 375-384; `025940` seq 2539-2542):

- inviter: `0x01D3 {B}`, one `0x01CC {B, member, 1}` per member (and a hero's `0x01C3` after its
  owner when it has one), `0x01D4 {B}`, `0x01C5 {B}`;
- invited: `0x01D3 {A}`, `0x01CC {A, member, 1}` per member, `0x01D4 {A}`, `0x01C6 {A}`.

**Cancel by the inviter** (`032434` seq 1295-1300): inviter `0x01C7 {B}` alone.

**Decline by the invited** (`032434` seq 2058; `034507` seq 2620-2626): inviter `0x01CA {B}` alone,
invited `0x01C9 {A}` alone.

**Accept** (`025940` seq 2617-2638; `party.jsonl` seq 160-165), invited player `b` joining party `A` led
by `a`:

- `b`: `0x01C9 {A}`, `0x01CC {A, b, 1}`, `0x00B0 {a, n}`, `0x00B1 {b, a}`, `0x003C` x5,
  `0x01B3 {A, 0}`, `0x01E0 {0}`, `0x01BF {0}`;
- the party joined: `0x01CA {B}`, `0x01CC {A, b, 1}`, `0x00B0 {a, n}`, `0x00B1 {b, a}`, `0x003C {b}`.

**Leave** (`025940` seq 2432-2458), player `b` leaving party `A` led by `a`, into a new party `C`:

- `b`: `0x01D1 {A, b}`, `0x00B0 {a, n-1}`, `0x00B0 {b, 1}`, `0x00B1 {b, b}`, `0x003C` x5,
  `0x00B0 {b, 1}`, `0x00B1 {b, b}`, `0x01D3 {C}`, `0x01CC {C, b, 1}`, `0x01D4 {C}`, `0x01B3 {C, 1}`,
  `0x01E0 {0}`, `0x01BF {0}`.

**Leader leaves** (`034507` seq 3304-3346), leader `a` leaving party `A` with member `m`, into `C`. The
party is not disbanded: `m` leads it on under the same id (seq 3427 shows `A` streamed with `m` alone).

- `a`: `0x01D1 {A, a}`, `0x00B0 {m, n}`, `0x00B1 {a, m}`, `0x00B1 {m, m}`, `0x00B0 {m, n-1}`, then the
  rest of a member's leave from `0x00B0 {a, 1}` on.

**Kick** (`034507` seq 2990-3001), leader `a` kicking `k` from `A`:

- `a`: `0x01D1 {A, k}`, `0x00B0 {a, n-1}`, `0x00B0 {k, 1}`, `0x00B1 {k, k}`, `0x003C {k}`,
  `0x00B0 {k, 1}`, `0x00B1 {k, k}`.

**The member that takes over** (`025940` seq 627-631, a leader leaving the map): the handover pairs,
then `0x01CB {1}`.

**An invited player leaves the map** (`034507` seq 2645-2650), inviter's side: `0x01D0 {B, b}`, the
farewell (`0x00B0 {0, 1}`, `0x00B1 {b, 0}`, `0x005A {b}`), `0x01D1 {B, b}`, `0x01CA {B}`.

**Joining while invitations are open** (`030218` seq 201-236): before the add, every open invitation of
the joining party is closed with `0x01C7`. The client's request there was `0x80A0` naming a player
(`0x56`) who is not in the party joined; what made it a join is not established.

## What is not captured, and what this server does instead

| case | sent by `party.rs` |
|---|---|
| invited side of a cancel | `0x01C8 {A}` alone |
| the kicked player's own batch | a member's leave batch |
| the staying members of a leave | the leader's side of a kick: remove, new size, the pairs around the flags word |
| the new leader's batch | the staying members' batch with the handover pairs, then `0x01CB {1}` |
| bystanders of an accept, a leave or a kick | the `0x00B0` / `0x00B1` pairs only; every capture carries them for other parties |
| the four to six extra `0x003C` in an accept or a leave | not sent: which players they name is not established, so only the one joining or going is sent, with this server's value 0 (live sends the player's own value) |
| the inviter leaving the map, invited side | `0x01C8 {A}` |
| the farewell's place in the left-the-map batch | after the party messages, not between `0x01D0` and `0x01D1` |
| the party-size cap | refused (logged, nothing sent) |
| inviting a party of more than one (a merge) | the surviving party is the merge leader's; see `docs/45-party-travel.md` |

## The end of a load

Every live load carries the party state after the player's own party (`0x01B3`) and before
`INSTANCE_LOAD_FINISH`, every value 0, before and after the Searing:

| map kind | after `0x01B3` | recordings |
|---|---|---|
| outpost (148, 134, 163, 166, 194) | `0x01E0 {0}`, `0x01BF {0}`, `0x01BE {0}`, party search adverts (`0x01DF`), `0x01E0 {0}`, then `0x016E {0}` | `titles-live-20261106-034507.jsonl` seq 847-863, `025940` seq 1103-1115 |
| explorable (146) | `0x01BF {0}`, `0x01BE {0}`, `0x00BA`, then `0x016E {0}` | `doors-20261031-095754.jsonl` seq 322-327, `party.jsonl` seq 2669-2671 |

ASURA's load ended at `0x01B3`, and its Ascalon party window drew a normal / hard mode toggle and a
ready check after each name, which retail Ascalon does not (owner, 2026-11-06). The load now carries
the party messages of that tail (`crate::party::load_tail`): no adverts, because this server has no
party search. `0x016E`, `0x00BA` and the `0x0194` after the finish are not part of the party state and
are not sent yet. `0x01BE` is unnamed in the packet table (ledger E43 `GlobalStateFlags`, one opcode up).

## The party window's controls

Owner, 2026-11-06: ASURA's Ascalon party window drew a normal / hard mode toggle and a ready check
after each name, and allowed four players, where retail Ascalon draws none of it. A fresh login into
Ascalon on each server (`multiplay/party interface check.jsonl`, retail; `multiplay/privaat party
interface check.jsonl`, ASURA) differs in these places, all outside the party messages:

| message | retail | ASURA before | now |
|---|---|---|---|
| `0x0070` `HARD_MODE_UNLOCKED` | never sent, in any live capture | `{1}` in the inventory reply | not sent |
| `0x017C` `INSTANCE_LOAD_HEAD` | `{0x1F, 0x1F}` for this account (eleven loads); `{0x3F, 0x3F}` for another (`maps/pre-searing-load-3.jsonl`) | `{0x3F, 0x3F}` | `{0x1F, 0x1F}` in a map load; creation unchanged |
| `0x0030`, `0x0099`, `0x0141`, `0x0147`, `0x00EA`-`0x00ED`, `0x01AC` | never sent in the session | sent in the inventory reply (the reference's order) | unchanged |
| `0x0196` spawn point | a `u16` 163 and an 8-byte value after the position | zeros | unchanged |
| `0x016E {0}` before the finish, `0x0194 {0}` after it | sent | not sent | unchanged |

The map is not the difference: both servers name map 148, and the client reads what kind of map that
is from its own table. What differs is what each server says about the character. Which of the two
changes hides which control is for the client to show; the rows marked unchanged are the next
candidates.

## Party size

Owner, 2026-11-06: every pre-Searing map allows a party of at most two. The value lives in the world
pack, `config/World/Vanilla/party_sizes.json`, one entry per map id: 145-151, 160-166, 877 and 878, the
pre-Searing maps of `docs/39-map-list.md`. The four Heroes' Crypt ids (208, 321, 342, 828) sit under the
same area in that list but are an arena of the post-Searing game, so they are not listed; a map with no
entry has no answer, and no party grows there. An invitation and an accept past the size are refused
with a logged reason and no message; what a live server sends to a full party is not captured.

The party window's own size label comes from the client's map table, not from this server, so it can
show a different number from the one the server allows.

## Rules

Owner, 2026-11-06: parties work differently in explorable areas. An explorable is one party's own copy
(`docs/22-world-model.md`, "Explorables"), so every party request there is refused
(`asura_rules::party::may_change_party`). A member whose connection drops in an explorable is held
soft-disconnected (experimental, `docs/45-party-travel.md`).

The party table lives in the world's instance; a party travels as one through the database's party
ledger (owner, 2026-11-06, `docs/45-party-travel.md`).
