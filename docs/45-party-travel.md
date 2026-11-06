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
| the leader's world-map travel (`0x80B1`) in an outpost | `travel_request` -> `PartyAfterCountdown` | `0x01BC` to every member, then after `countdown_secs` the party goes; a member without the outpost unlocked stays behind in a party of its own (`follows_to_outpost`, owner 2026-11-06) |
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
| `0x01BC {u16 seconds, u8, text}` | the leader's countdown | **not captured**: layout from the client's table and Headquarter (`time_in_sec`, its `0x01BB`); the byte 0 and the empty text are guesses, both in the pack file |
| `0x01CD` / `0x01CE` / `0x01CF` | connect soft / disconnect soft / enter world | **not captured**: ledger E38 one opcode up, exactly as the measured `0x01D0` is |
| a merge's batches | | **not captured** |
| a full or refused party travel | | **not captured**; nothing extra is sent |

## Not built

- **`/resign` and the wipe return** - last (owner, 2026-11-06).
- **Cancelling the countdown from the client**: the cancel message is not known. A newer world-map travel
  by the leader replaces the countdown; a change of leader voids it.
- **One instance per party in an explorable** (ledger R3, R35): instances are still one per map per world,
  so two parties in the same explorable share it.
- **Heroes and henchmen** in a travelling party.
- **Districts**: a move goes to the destination map on the same world, as every travel does.

## What to capture next

1. A live leader's world-map travel with a party member in the outpost, from both clients: the real
   `0x01BC` and whatever the members see while it counts down.
2. The member side of a doorway crossing (the member that did not cross): its batch and its arrival load.
3. The second member arriving in the destination: what the first member is told.
4. A member losing its connection in an explorable, and logging back in.
