# 58 - Quests: the content, the state machine and the log wire

**Status, 2026-11-13:** the content, the stage rule, the log messages, the row that survives a
reconnect, the load restore and quest 80 end to end are built. The one link the capture does not
establish is which of the Town Crier's two captured buttons is the acceptance; the first button carries
the action and the client test is what confirms or corrects it. Nothing else accepts or rewards a quest
yet.

## 1. Where the data is

The extracted checkout at `D:\preforged-main\data` holds captures and `accounts.db`; it has no `gw/`
tree. The quest catalog is in the zip this repository already carries:

| what | where |
|---|---|
| the curated catalog, `FWQC3`, 51 rows over 22 ids | `Other_Repos_Research/preforged-main.zip`, `preforged-main/data/gw/quests/presearing.fwqc` |
| the live capture's quest snapshots, 254 over 27 ids | the same zip, `preforged-main/data/gw/quests/side-quests-2026-10-12-evidence.json` |
| the capture the evidence file describes | `data/captures/pre-searing-side-quests-2026-10-12/checkpoint-1815/world-43292-168890171.jsonl`, sha256 `194855cf498db3b91d9afee929785470c176596fc1fe149b33fe8b8c5a1b9757` |
| the 2022 wire fixtures | `tests/fixtures/quest-wire-2022.txt`, `quest-details-2022.txt`, `quest-objectives-2022.txt` |

The pack file is `config/World/Vanilla/quests.json`, 38 quests: 22 from `presearing.fwqc` and 16 that
only the October capture observed. Every unit in it is the capture's. The tool that writes it is
`tools/quests-from-preforged.py`; it fails if either source is absent, if a row is malformed, or if a
text is over the client's bound.

## 2. The content

| field | what it is |
|---|---|
| `id` | the client's quest id |
| `state` | the captured log state: bit 1 loaded, bit 2 completed, bit 32 primary |
| `marker` | exact IEEE-754 bits for x and y, and the plane; the pair `+infinity, +infinity` is the client's no-marker sentinel |
| `map_to`, `map_from` | the destination and the source map; 898 is a no-destination sentinel the capture carries |
| `location`, `name`, `npc` | encoded units; at most 7 units plus the client's terminator |
| `description`, `objectives` | encoded units for `0x004C`; at most 127 units plus the terminator |
| `character_argument` | the captured character name inside the text, replaced at send time |
| `completed`, `counted` | the details and the marker the capture shows at another stage |

Three catalog entries - 80, 85 and 89 - carry the captured character argument `Logger`. The rule
`asura_rules::quest::bind_character_argument` replaces the run between `0x0107` and `0x0001` with the
playing character's name; it does not search the encoded text for a substring.

A marker and its destination move as a quest advances, so a completed or counted variant carries its
own. The pack stores each one the capture showed; nothing derives a later marker from an earlier one.

The units are compatible with the client that sent them. Whether every string id resolves on this
client's own table is unverified until a client draws the log: a unit within the length bound is not a
promise that the id exists (`docs/quest-evidence.txt` in preforged makes the same limit explicit).

## 3. The stage rule

`asura_rules::quest` holds the transitions and no values:

- `active` -> `completed` -> `rewarded`, or `active`/`completed` -> `abandoned`;
- a reacceptance is allowed only from `abandoned`, or from `rewarded` when the caller marks the quest
  repeatable, and it increments the cycle;
- every transition names the attempt's cycle, so a delayed packet from an old attempt changes nothing;
- an objective credit is a stable identity the caller supplies; the last required credit completes the
  attempt;
- the storage bounds are `Limits`, handed in by the caller, not compiled into the rule.

The rule is preforged's `src/runtime/gw_quest_state.h` in this project's terms. Its planners do not
authorize an NPC, an objective or a reward; the caller validates those and persists the proposal with
the reward in one transaction.

## 4. The log messages

The layouts are preforged's `src/runtime/gw_quest_wire.h`, which was checked against the client's own
constructors, and the fixture bytes under `tests/fixtures/`. This client is one build later than the
2022 fixture, and each opcode a fixture covers moved by one:

| operation | 2022 fixture | this build's table | row |
|---|---|---|---|
| add an entry | 72 | `GAME_SMSG_QUEST_ADD` `0x0049` (73) | `packets.rs` |
| details, two strings | 75 | `GAME_SMSG_QUEST_DESCRIPTION` `0x004C` (76) | `packets.rs` |
| move a marker | 76 | unnamed row `0x004D` (77) | `packets.rs` |
| remove an entry | 81 | `GAME_SMSG_QUEST_REMOVE` `0x0052` (82) | `packets.rs` |
| replace the objectives | 83 | `GAME_SMSG_QUEST_UPDATE_NAME` `0x0054` (84) | `packets.rs` |

The marker's header is the one the live capture writes before quest 80's turn-in details and removal
(tick 169070859: `4d 00 00 00`, id, x, y, plane, destination). The table's
`GAME_SMSG_QUEST_UPDATE_MARKER` (`0x0051`) and `GAME_SMSG_QUEST_ADD_MARKER` (`0x0053`) are other marker
messages: the capture uses `0x0051` after a progress update (quest 83 at tick 174767718), and no capture
here uses `0x0053`.

The last row's table name is the reference's. Preforged's native work names what the message does:
it replaces the objectives text and raises `QuestUpdated`, not `QuestAdded`
(`docs/quest-evidence.txt`, 2026-10-13). The builder is `game::quest::quest_objectives`.

The add order is id, x, y, plane, destination, state, location, name, NPC, source map. Add precedes
details for one entry; the native details callback asserts on an entry that does not exist. A restore
masks the loaded and completed bits and lets the details message own bit 1
(`gw_quest_restore.h`).

The capture's packet log is the client's message struct, not the wire: its `0x0080` body is 248 bytes
where the wire body is 2 + 10 for the same five units. Sizes and layouts here come from the 2022 wire
fixtures and the packet table, never from that log (`AGENTS.md` #1, `docs/03-protocol.md` 5e).

## 5. The first quest, end to end

Quest 80, Message from a Friend, is the first complete path. Its content is the capture's; its offer and
reward values are preforged's `gw_presearing_quests.h` and
`docs/message-from-friend80-policy-20261030.txt`.

**Offer.** The Town Crier on map 148 says the page at `dialogs.json` (`map 148, npc "Town Crier"`),
whose body and two choice labels are the capture at tick 169041703. The first choice is
`0x805001` and carries the offer; the second is `0x805002` and closes. The offer is open while none of
quest 35 or 217 to 222 is completed or rewarded. Accepting writes one `t_character_quests` row (cycle 1,
stage 1) and sends `0x0049`, the dialog's sender and talked-to property, then `0x004C`. The page carries
a `when` condition, so it is not shown once the quest is active; Sir Tydus's page is shown only while
the turn-in is open.

**Reward.** Sir Tydus on map 148 says the page at tick 169069953; its one choice is `0x805007`. The
turn-in is accepted for an active or completed attempt at that cycle. One transaction moves the row to
rewarded and adds 100 experience and 10 gold. The client then gets the captured order: marker `0x004D`,
active details `0x004C`, removal `0x0052`, the experience award `0x00EE {0, 100}`, and the gold delta
`0x0140 {stream, 10}`. The capture's own turn-in burst at tick 169070859 carries the marker, details,
removal, `0x00EE {10, 0}` and `0x00EE {0, 100}` and the gold add in that order; preforged's policy names
the same batch (`message-from-friend80-policy-20261030.txt`: 76, 75, 81, 319).

**The load restore.** The log is the character's, and the client starts **every** map - a login's or
a travel's - with an empty one. The live server rebuilds it **inside the load**, while the loading
screen is up (four-professions-2026-10-11 packets, quest 222 accepted in map 148 and loaded into 146):

| what | where in the load | live tick |
|---|---|---|
| `0x0050` general info per quest (the add's id, state, names and source map, no marker) | the item answer, after the gold (`0x0140`, `0x0141`) and before the faction caps (`0x00EA`..`0x00ED`) | 81524875 |
| `0x0053` marker per quest | the player block, before `0x018E` finishes the load | 81525390 (finish at 81525406) |
| `0x004C` details per quest | just after the load | 81525500 |

`crate::quest::load_log` builds the three; `crate::item::handle_request_items` and
`crate::player::handle_request_players` put them there. `0x0050` raises the client's "quest added"
event (preforged `quest-evidence.txt`, 2026-10-13: the general-info handler emits QuestAdded), like
`0x0049`: inside the load it shows nothing, after it the client announced every quest. `0x0049` is
only for an acceptance.

**The marker request.** For every entry it is given the client asks once for the marker with `0x8012`
(`GAME_CMSG_QUEST_REQUEST_INFOS`, the quest id; preforged's marker request, `gw_quest_request.h`).
`crate::quest::handle_info_request` answers with the marker alone, `0x0053` (preforged
`gw_quest_restore.h`, `marker_reply_2022`), for a quest held active or completed with a finite marker,
and never for the same quest twice within two seconds.

What went wrong on the way, so it is not repeated: restoring only on a login left every other map's
log empty; restoring with `0x0049` or `0x0050` after the load announced every quest ("quest added",
with its sound for `0x0049`); answering the marker request with the whole entry made the client ask
again for every quest, without end.

**The one unverified link.** The capture holds the server's two offer buttons but not the client's
outgoing choice: preforged records the same gap ("no corresponding outgoing NPCInteract/DialogChoice
inputs were captured"). The first button carries the offer because the quest was added 937 ms later and
the first button is the acceptance the player saw. If the client shows the offer on the other button,
swap the two `action` blocks in `dialogs.json`; no code changes.

### The floating marker

The exclamation above an NPC is `0x009F` agent property 11: `{11, agent, value}`. The capture sets the
Town Crier to **5** before its offer (tick 169041703), Sir Tydus to **4** when quest 80 is accepted
(169042640), and Sir Tydus to **5** again after the turn-in (169070859, where he has another quest on
offer).

The values are not compiled in. `config/Server/quests.json` holds `marker_offer`, `marker_turn_in` and
`marker_none`, each `immutable` because they are the client's own states. The rule
`asura_rules::quest::marker` returns `Marker::Offer`, `Marker::TurnIn` or `Marker::None` from the
profile; `crate::quest::marker_for` aggregates a quest's offer and reward blocks for the NPC it stands
at, with a turn-in winning over an offer; `marker_value` maps the state through the file.

The load sends the marker after each NPC's entry burst, where the capture sends it after `0x006D`; an
acceptance or a turn-in sends it for the offer and turn-in givers again. A body that walks into range
later gets its marker from the connection when the create arrives (`with_npc_markers`), so the per-tick
sight path does not need the character's profile.

**A marker is cleared with property 12, never with property 11.** Every property-11 value in
preforged's captures is 5 or 4 (2 and 3 on NPCs this pack does not offer from); none is 0. The live
server clears instead with `{12, npc, 0}` - the talked-to property - on the giver right after an
acceptance (`0x0049`, the turn-in giver's `{11, ., 4}`, the sender, `{12, giver, 0}`;
four-professions-2026-10-11, tick 81485859) and on the trainer after a turn-in (81601156). The live
dialog burst also re-sends the speaker's marker after its choices (`{11, npc, 5}` after the Town
Crier's two choices, 81484703). `crate::quest::marker_update` follows that: property 11 with 5 or 4
for an offer or a turn-in, property 12 with 0 for none. The first version sent `{11, npc, 0}` for none,
which the client draws as a marker: the exclamation stayed on a giver after its quest was accepted,
and appeared on a quest NPC after a talk. `marker_none` in `config/Server/quests.json` is no longer
sent.

**The marker follows the quest, not the talk.** The client drops an NPC's floating marker when it is
talked to (the dialog burst ends with `0x009F {12, npc, 0}`). So after every answer to a talk and every
dialog choice, a quest NPC's marker is sent again from the character's profile
(`crate::quest::refresh_marker`, called from `crate::vendor::talk` and `handle_send_dialog`): the Town
Crier keeps his offer marker until quest 80 is accepted, Sir Tydus keeps his turn-in marker until it is
rewarded, and an NPC with nothing left is cleared. An NPC no quest's offer or reward block names gets
nothing, so merchants are not touched.

**Abandon.** `GAME_CMSG_QUEST_ABANDON` (`0x8011`, the log's Abandon button) moves an active or completed
attempt to stage 4 (`asura_db::quest::abandon`, fenced on the cycle like a turn-in), then sends the
removal `0x0052` and the offer and turn-in givers' markers on this map: the offer reopens and the turn-in
closes. A later acceptance starts the next cycle. No capture shows the live server's abandon burst; the
removal is the same message a turn-in ends with.

**Talk range.** A talk is answered once the server has walked the player to within the movement file's
`interact_range` (50 units). An NPC the player cannot walk up to - Sir Tydus stands on a stand - has a
`talk_range` in its `dialogs.json` entry (200 for Sir Tydus, chosen, not measured); the walk and the
reach check use the larger of the two.

### Every captured quest

`tools/npc-content-from-captures.py` (run after `quests-from-preforged.py`) gives **40 quests** an offer
and a reward block: every quest whose offer page and turn-in page preforged's captures both hold. It
also adds the log content of the ten quests the captures' snapshots carry and the pack did not (38, 72,
77, 87, War Preparations 217/218/220/221/222, and the Vanguard quest 1185).

| from | what |
|---|---|
| the captured dialogs | who offers and who takes the turn-in, on which map (the retail choice id `0x80QQ01` / `0x80QQ07`) |
| the quest's own reward text in the snapshot | `experience`, `gold`, `skill_points` |
| preforged `gw_presearing_quests.h` | the skills an acceptance or a turn-in teaches, professions, the profession-test prerequisite |
| preforged `gw_quest_menu.h`, the quest inventory | minimum levels (55 and 46 at 2, 1185 at 10), A New Elementalist Trainer after Further Adventures |
| the captures | War Preparations 217-222: Sir Tydus offers one per profession, the trainer who takes the turn-in fixes which |

An offer is open when the attempt rule allows it and the character qualifies (`Offer::eligible`): its
level, its professions (a primary in `professions`; unless `primary_only`, a matching secondary or no
secondary yet - preforged's rule, how a pre-Searing character finds its second profession) and the
prerequisite quests (`requires_rewarded`, `requires_any_rewarded`, `requires_none_completed`). The
dialog conditions and the markers use the same test, so an NPC shows a quest's offer and its marker
only to a character who may take it.

Skills are taught with the acceptance or the turn-in, after the row is stored
(`skill::character::teach`): the learned set is saved and the client gets it and one learned notice per
new skill.

**Talk objectives are modelled.** Unsettling Rumors (62, Meerak), A Second Profession (79, the trainer of
the second profession) and The Wayward Wizard (55, Orion Elek) complete when the player chooses the
captured talk choice (`credit`): the row moves to completed, the log gets the completed text and marker
(`0x0054`, `0x004D`), and their turn-ins wait for it (`requires_completed`). Every other quest's turn-in
is open as soon as it is accepted, because its objective is not modelled yet; see below.

## 6. What is not built

- **Kill, item, escort and timer objectives.** No kill, pickup or escort credits a quest, and quest 42's
  90-second deadline is not modelled. Those quests can be turned in once accepted, which is a shortcut
  and not the retail rule; the `counted` variants in the pack are the content waiting for that rule.
- **Quests without a captured turn-in.** 36, 39, 48, 53, 56, 61, 78 and 90 have a captured offer and no
  captured turn-in page, so the pack does not offer them: an offer that cannot be finished is worse than
  none. 40 and 60 have no log content.
- **Item rewards.** The reward text names weapons and shields (Bandit Raid's Flame Artifact, ...); no
  quest reward grants an item yet.
- **Second-profession unlocks.** A Mesmer's Burden, The Elementalist Experiment, Grawl Invasion, The
  Ranger's Companion, The Necromancer's Novice, A Monk's Mission and A Second Profession pay their
  experience and skills, but do not unlock or set a secondary profession.
- **The skill-point message.** A reward's `skill_points` (Tithe for Ashford Abbey's one) is stored; no
  message tells the client until the next load.
- **The completed log state.** The capture removed quest 80 directly from active; no snapshot shows it
  completed, so the pack has no completed variant for it. The reward dialog's completed text is stored
  as the dialog body, not as the quest's completed details.
- **Per-map markers.** The pack stores the marker the capture showed from one map. Preforged rebinds
  some markers by the character's current map (`gw_quest_message_from_friend.h`, `bind_markers`); that
  rule is not ported.

## 7. Acceptance

```powershell
cargo test -p asura-protocol quest --lib
cargo test -p asura-rules quest --lib
cargo test -p asura-world --lib quest
python tools/quests-from-preforged.py
python tools/npc-content-from-captures.py --preforged D:/preforged-main
asura-supervisor schema
```

The first three are the wire, the rule, the content and the operations. The fourth and fifth print
what they would write and write nothing. The last applies migration 0038 (`t_character_quests`); the
supervisor's `start` does the same.

On a real client:

1. Log in to Ascalon City (148) and talk to the Town Crier.
2. Choose the first offer choice. The quest appears in the log with its name, text and marker; the
   server logs `accepted quest 80`.
3. Disconnect while the quest is active and reconnect. The log restores the entry.
4. Walk to Sir Tydus and talk to him. Choose the turn-in. The log entry goes, experience rises by 100
   and gold by 10; the server logs the turn-in.

The client is the judge. A wrong button order or a wrong restore position shows there first.
