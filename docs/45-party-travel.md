# 45 - Party travel: a party moves as one (experimental)

Owner, 2026-11-06: "party interface works, let's properly implement it with the auth-server so that in
fact players can travel around with each other." The rules are the owner's notes of the same day (the
party mechanics write-up: portals, explorable restrictions, map travel, the leader's countdown, `/resign`,
the merge leader rule) and ledger rows R6-R9, R11, R17, R26. Everything here is **experimental** (owner:
"most of this is experimental").

Code: `crates/asura-world/src/party.rs` (the table, arrival, the move, the hold),
`crates/asura-world/src/travel.rs` (the three triggers, `Order`, `move_party`),
`crates/asura-rules/src/party.rs` (the decisions), `crates/asura-db/src/parties.rs` and migration
`0030_parties.sql` (the ledger), `crates/asura-auth/src/travel.rs` (placing a held member back).
Values: `config/World/Vanilla/party_travel.json`.

## Where a party lives

A party lives in the instance of the world that runs it (`asura_world::party`), as before. A travel ends
that instance's knowledge of it: every member reconnects, maybe to another world process, and a member
that logs in again is placed by the auth service. Those programs share only PostgreSQL
(`docs/02-architecture.md`), and D25 keeps game state out of auth, so the party crosses through a
**ledger** (owner's choice, 2026-11-06):

| table | one row per | holds |
|---|---|---|
| `t_parties` | move of a party | leader (a character), world, map, when |
| `t_party_members` | character (primary key) | its party, its seat (leader 0), its state, `held_until` |

States: 0 travelling (told to go, not arrived), 1 in the map, 2 held (dropped in an explorable). A new
move deletes the old party of every traveller, held places included: a party that changes areas leaves a
held member behind. Nothing reads the ledger while a party stays in one instance.

## The three triggers

| trigger | decision (`asura_rules::party`) | what happens |
|---|---|---|
| any member crosses a doorway (`portal_crossed`) | always the party | the destination and the door's arrival point are resolved once; every member in the map goes at once |
| the leader's world-map travel (`0x80B1`) in an outpost | `travel_request` -> `PartyAfterCountdown` | `0x01DA` then `0x01BC` to every member (only when the destination's `basic.json` has a `name_id`, see "The countdown and the destination"), then after `countdown_secs` the party goes; a member without the outpost unlocked stays behind in a party of its own (`follows_to_outpost`, owner 2026-11-06) |
| anyone else's world-map travel, or anyone's in an explorable | `travel_request` -> `LeavesParty` | the leave batch (`docs/44`), then the player travels alone; the client asks the player first |
| `/tp` | always alone | the leave batch, then the teleport |

A move (`party::begin_move`) marks the party as leaving, orders the travellers leader first, then the
member that started it, then the rest, and cuts at the destination's size from `party_sizes.json`
(`travelling_count`, ledger R26): the rest stay in parties of one. A destination with no size moves only
the player that started it, logged. `travel::move_party` writes the ledger, then sends each traveller's
own connection an `Order::Travel` through a second channel beside its outbox (`Instance::order`), and
each connection runs the ordinary redirect cycle (`begin_transfer`) on its own socket. A member whose
transfer is refused stays behind in a party of its own (`party::stays_behind`). A traveller's teardown
is quiet: its redirect batch and farewell already said it.

**The redirect batch carries every traveller's travel effect.** `multiplay/party.jsonl` seq 2512-2515, a
leader whose party of two crossed into Lakeside County: `0x00F1 {0xF1, 0x100}`, `0x00F1 {0x4C, 0x100}`,
`0x0028 {0xF1}`, then the redirect. Each traveller's batch now carries its own effect, then the other
travellers', then its own stop.

## Arriving

Each member's game connection reads the ledger before it joins (`connection.rs`). When the ledger names
this map on this world and the character as travelling (or held, with a live hold), the join is the
party's arrival (`party::arrive`):

- the **first** member builds the party from the ledger: every other member still travelling or held
  gets a **reserved player id** (`Instance::reserve`) and stands in the party as not in the map; its
  leader is the ledger's leader;
- a **later** member takes its reserved id (`Instance::join` hands it out, and never hands a kept id to
  anyone else) and its place;
- a member that has not arrived after `arrival_grace_secs` is removed (`party::expire`): the members in
  the map get `0x01D1`, the new size, and - if it led - the handover and `0x01CB {1}`.

The load carries the party as the live server does after a doorway (`party.jsonl` seq 2662-2669):
`0x00B0 {leader, n}`, `0x00B1 {member, leader}` per member, `0x01D3`, `0x01CC {party, member, 1 or 0}` -
0 for a member not in the map - `0x01D4`, `0x01B3 {party, 1 when leading}` (`party::load_section`). A
party of one is the same shape, which retail sends too (`multiplay/party interface check.jsonl` seq
588-591) and which this server did not send before: `0x00B0 {me, 1}` and `0x00B1 {me, me}` are new in
every load.

**Two members loading together** (2026-11-06, `logs/world-8001.log`). Two clients crashed:

1. In map 146 both were introduced to each other twice - by broadcast while still loading, then again in
   their own players block - and one was sent `0x01CF` before its load had created the party. Now a
   player *enters* the map when its players block takes the list of the others (`Instance::enter`, one
   lock): its block introduces only players that entered before it, it is introduced only to those, and
   broadcasts reach only players that have entered.
2. In map 163 the second to load was sent the first with flag 1 in its party list, whose
   `PLAYER_CREATE` comes later in the same block, and dropped; the first, sent its absent leader with 0,
   stayed. Now a client's own load shows only itself in the map; each member it was introduced to
   follows as `0x01CF {party, member}` after the introductions (`party::in_map_notices`), and the
   members that entered before it are told `0x01CF` for it (`party::announce_arrival`).

**Doorways and click-walking.** A click through a gate moves the agent on the instance's tick, and the
next position report already stands past the wall, so the doorway test now also runs on the step the
walk poll finds (`connection::run`).

A ledger row that names somewhere else is stale (the character travelled alone) and is forgotten on
arrival.

## The explorable disconnect hold

A connection that ends **without** `GAME_CMSG_DISCONNECT` and without a travel is a network drop
(`Player::said_goodbye`). In an explorable, with other members in the map and a ledger party, the member
stays in the party (`holds_on_disconnect`): its player id is kept, the others get `0x01CE {party,
member}` (drawn grey), and the ledger writes `held_until = now + disconnect_hold_secs` (900 s, ledger
R11). A clean logout is a leave.

When the character logs in again, the auth service reads the ledger: a live hold places it back in the
party's map on the party's world (`rejoin_held`; the request for a creation visit is left alone, and a
silent world falls back to the ordinary rule). The arrival takes the kept id and the others get `0x01CD`
then `0x01CF`. Any other login forgets the character's ledger row, so it starts in a party of one. A hold
that runs out removes the member as an arrival that never came does.

## Merges

An invitation may now name a party of more than one, and an accept may bring one, within the map's size.
The party that survives is the one whose leader leads the merge, by the owner's rule (2026-11-06): equal
sizes, the inviter leads; different sizes, the larger party's leader leads (`merge_leader`). Every member
of the other party moves into it. A party of one joining is unchanged and still the measured batch.

## Messages, and what is evidence

| header | meaning | evidence |
|---|---|---|
| `0x00F1` per traveller | the travel effect, in every traveller's redirect batch | `party.jsonl` seq 2512-2513 |
| `0x01CC {party, member, 0}` | a member not in the map | `party.jsonl` seq 2667 |
| `0x00B0`/`0x00B1` before the party stream in a load | the party's leader and size | `party.jsonl` seq 2662-2664; `party interface check.jsonl` seq 588-589 |
| `0x01DA {u8 kind, u8 initiator, ""}` | kind 2: the party lock before a world-map travel or countdown; kind 0: the unlock that answers a cancel | `multiplay-live-20261106-231251.jsonl` seq 109 (`{2, 1}`) and 124 (`{0, 0}`); `...-232449.jsonl` seq 356 (`{2, 0}`, a member); `...-195109.jsonl` seq 1196 (solo travel). Wire: `da 01 | 02 | 01 | 00 00` |
| `0x01BC {u16 seconds, u8 initiator, String16 message}` | the countdown window; `message` is "travelling to <map>" | `...-231251.jsonl` seq 110: wire `bc 01 | 0a 00 | 01 | 04 00 | d1 05 0a 01 7a 29 01 00`; every capture carries the destination's name |
| `0x80A3` (no body) | the leader cancels the countdown | `...-231251.jsonl` seq 123, answered by `0x01DA {0, 0}` at seq 124 and no move |
| `0x01E7 {u8 4}` | "the travel would leave your party; ask first" | `multiplay-live-20261106-200444.jsonl` seq 769, 892: the member's unconfirmed request, answered with nothing else; the confirmed request follows at seq 924 |
| `0x01D9` | the party is defeated | `multiplay-live-20261106-200638.jsonl` seq 42, at the same tick as the last two resign deaths |
| `0x002D {u32 agent}` / `0x0026 {u32 agent, u8 4}` | a resigning member's death | `multiplay-live-20261106-200638.jsonl` seq 52-55 |
| `0x80A7` | the defeated leader's return confirmation | inferred: it is the only client message between the captured member's `0x01D9` and the server's `0x01DA` + redirect, and the capturing client sent none |
| `0x01CD` / `0x01CE` / `0x01CF` | connect soft / disconnect soft / enter world | `0x01CE` is the grey flag and is sent on a travel departure; the remaining side of a travel is **not captured**, so that choice is a guess (ledger E38) |
| a merge's batches | | **not captured** |
| a full or refused party travel | | **not captured**; nothing extra is sent |

## The countdown and the destination

Measured 2026-11-06 (`multiplay-live-20261106-231251.jsonl` on the leader, `...-232449.jsonl` and
`...-232827.jsonl` on a member). The leader's `0x80B1` is answered in one tick by **two** messages to
every present member: `0x01DA {2, initiator, ""}` and `0x01BC {10, initiator, message}`. `initiator` is
1 in the client that asked (its window has the cancel button) and 0 in the others. The redirect
(`0x01A6`, then `0x0099`) follows ten seconds later. The leader's `0x80A3` cancels: the answer is
`0x01DA {0, 0, ""}` and nobody moves.

**Reading the captures.** The sniffer records the client's decoded *struct*, not the wire: a
four-byte header, each field widened to four bytes, then the 244-byte `String16` buffer, 256 bytes in
all. Everything after the string's terminator is stale buffer memory (it is byte for byte the same in
the `0x01DA` decoded in the same tick) and is not sent. Both messages are counted: the table's 252 and
253 are maxima, and the wire is the two-byte header plus the fields at the table's widths. Padding to
the struct size answers error 7; a `u32` seconds makes the client read `01 00` as a 256-unit string and
crash. (An earlier reading of this section, with a four-byte wire header, empty strings and a
"roster tail", was wrong.)

**The message names the destination, so the window depends on the map's `basic.json`.** The `0x01BC`
string is the encoded template `0x05D1` with one encoded-string argument, the destination map's name:
`05D1 010A <name units> 0001`. The name is the client's own string id (`Gw.exe`'s `AreaInfo` table),
stored in the pack as `"name_id"` in `maps/.../<map id>/basic.json` and read once per process
(`party::travel_map_name`). **A destination without a `name_id` gets no countdown window**: no
`0x01DA`/`0x01BC` is sent, the party still moves after `countdown_secs`, and the world log says
`map N has no name_id in its basic.json`. `tools/add-map-name-ids.py` writes every map's `name_id`
from the client; run it again after adding a map folder. The test
`every_pre_searing_outpost_has_a_name_for_the_countdown` fails if a pre-Searing outpost loses its id.

## The individual travel confirmation

Measured in the same file. A member of a party asks the world map for a destination while its
request's trailing flag is 1: the server sends only `0x01E7 {4}` (seq 769, 892) and starts no
travel. The client shows "You will leave your party if you travel there. Are you sure you wish to
do this?"; on Yes it sends the same request with the flag 0, and that one travels alone (seq 924,
redirect at seq 928). A solo player's flag-1 request travels at once and carries `0x01DA` first
(`195109` seq 1189-1198).

The member that stays keeps the departing player's name, greyed: the live side of that is
`0x00B1 {player, 0}` at a player leaving the map (`200444` seq 746). This server gives the departing
player the full leave batch of its own new party before the redirect - the path that loaded
immediately in every earlier travel - and sends the staying members `0x01D0 {party, player}` (the
measured "left the map" notice), `0x00B0 {new leader, new size}`, `0x00B1 {player, 0}` and
`0x01CE {party, player}` instead of a remove, so the client keeps the name and sets the grey flag.
The departing side's leave batch is **not** in the live capture (seq 924 is followed only by the
redirect); it is kept because the client needs it before it reconnects. **The staying side of an
individual travel was not captured**; the `0x01D0` and `0x01CE` choice is ledger E38's measured
meaning, not a byte from this file.

## `/resign`

Measured 2026-11-06 (`multiplay-live-20261106-200638.jsonl`, a party of two in map 165). `/resign`
is a **chat line**, not a GAME_CMSG: the captured client sends `SEND_CHAT_MESSAGE` at t=11.334 with
the masked run of seven UTF-16 units that is `/resign`. Its death follows at t=16.888, 5.55 s later,
as `0x002D {agent}` then `0x0026 {agent, 4}`; the other member's death is in the same tick. The
server then sends `0x01D9` (party defeated) to both. At t=21.500 it sends `0x01DA {2, 0, ""}` and
the redirect (`0x01A6` then `0x0099` naming the same outpost 165) at t=21.550. The captured client
sent nothing between `0x01D9` and the redirect, so the return is the **leader's**
`GAME_CMSG_PARTY_RETURN_TO_OUTPOST` (`0x80A7`) and this file is the member's side of it.

This server now answers the chat line by marking the member, killing it after the pack's
`resign_secs` (5), sending `0x01D9` when the last member in the map has died, and moving the party
to the leader's `last_outpost` when the leader sends `0x80A7`.

## Not built

- **A map without `name_id` in its `basic.json`**: a party travelling there moves on the server's timer alone.
- **The members' side of a cancel** is not captured; they are sent the leader's `0x01DA {0, 0}`.
- **A lock or unlock when a countdown is voided** (a leader leaving, a newer travel): not captured, not sent.
- **`0x01DA` before a solo travel and before a defeated party's return**: live sends it; this server does
  not yet, and the move works without it.
- **One instance per party in an explorable** (ledger R3, R35): instances are still one per map per world,
  so two parties in the same explorable share it.
- **Heroes and henchmen** in a travelling party.
- **Districts**: a move goes to the destination map on the same world, as every travel does.

## What to capture next

1. The staying member's side of an individual travel: the exact grey batch, and whether it is `0x01CE`
   or only `0x00B1 {player, 0}`.
2. The leader's own client during a resign: the `0x80A7` request and the destination it carries.
3. The member side of a doorway crossing (the member that did not cross): its batch and its arrival load.
4. The second member arriving in the destination: what the first member is told.
5. A member losing its connection in an explorable, and logging back in.
