# 55 - NPC dialogs: the greeting, the choices, and what a choice opens

**Status, 2026-11-13:** built against one live capture. The Purveyor [Bonus Merchant on map 148 says a
greeting with seven choices; two of the choices have a measured answer. No other pre-Searing NPC has a
captured greeting, so no other NPC says anything yet.

## 1. The evidence

One live session, this repository's own recording:
`recordings/client/feature/movement/movement-live-20261109-200703.jsonl`.

| what | records | bytes |
|---|---|---|
| talk, first time | 5171 `INTERACT_LIVING` {0x39, agent 0x36} | |
| greeting | 5176 `0x0080` body, 5177 `0x0081` sender 0x36, 5178-5184 seven `0x007E` choices, 5185 `0x009F` {12, 0x36, 0} | body `[0x8102, 0x3E7E, 0xD7B5, 0xDBE4, 0x49F2]` |
| choice 0x8C | 5245 `0x803B` {0x8C}, 5247 `0x0083` {0x36, 5, 0xCCCCCCCC}, 5248 `0x0081` {0x36} | a window of kind 5 |
| talk, second time | 5260 `INTERACT_LIVING` | |
| greeting again | 5261-5269, same body and same seven choices | |
| choice 0x8B | 5277 `0x803B` {0x8B}, 5281-5286 the merchant window, 5287 `0x0081`, 5288 `0x009F` | two item rows, ids 0x46D and 0x45E |
| the NPC | 903 `0x009B`, 906 `0x0020` at (11416, 5326) | map 148 placement 148-10020, The Purveyor [Bonus Merchant |

The body and every choice's label are **encoded strings**: the live server sends client string ids and
control codes, not text. The capture's units are copied verbatim. Preforged's builder confirms the
shape (`src/runtime/gw_npc_dialogue.h`): `open_encoded` splits the body at 121 units, sends the actor,
then one button per choice; a button is icon, label, id, skill. The same file shows the literal form a
server-composed body uses (`0x0108 0x0107 <text> 0x0001`) and the 32-choice limit.

The seven choices, in the live order:

| id | icon | skill | label units | what the capture shows |
|---|---|---|---|---|
| 0x8C | 28 | 0 | `8102 3B47` | opens window kind 5 |
| 0x87 | 11 | -1 | `8102 3EB5 C39B 98EF 07F3` | never chosen |
| 0x8D | 11 | -1 | `8102 3E7F B0A6 8687 1D87` | never chosen |
| 0x8E | 11 | -1 | `8102 3E97 B5FA 4CC2 29E4` | never chosen |
| 0x8F | 11 | -1 | `8102 3E82 DC16 B361 38C5` | never chosen |
| 0x8B | 24 | -1 | `8102 3EB2 9838 CCE3 7ABF` | opens the merchant window |
| 0x85 | 15 | -1 | `8102 3E8B F972 9639 6A86` | never chosen |

## 2. What is built

| part | where |
|---|---|
| the file | `config/World/Vanilla/dialogs.json` |
| the loader and the rule | `crates/asura-world/src/dialog.rs` |
| the talk path | `crates/asura-world/src/vendor/mod.rs`, `talk` |
| the choice path | `vendor::handle_send_dialog` |
| the wire | `crates/asura-protocol/src/game/dialog.rs` (already measured), `game::vendor::window_open` |

A dialog is found by map and by the placement's literal name, the same rule `vendors.json` uses. A
dialog comes **before** the NPC's window: The Purveyor's merchant window is behind choice 0x8B, so a
talk that opens the window directly would be wrong for this NPC.

The actions:

- `close` answers nothing. The capture never chose a close, so this is the smallest answer;
- `window` sends `0x0083 {npc, kind, 0xCCCCCCCC}` then `0x0081 {npc}`, the live order at 5247-5248;
- `merchant` opens the NPC's window through `vendors.json` and the vendor rules, then adds the speaker
  before the talked-to property, the live order at 5281-5288;
- `page` shows another page of the same dialog;
- `quest` with step `offer` accepts the quest at the speaker and puts it in the log; with step `reward`
  it pays the quest and removes it; with step `credit` it completes the quest's talk objective
  (`docs/58-quests.md`). After the step the dialog shows the step's `page` when it names one (the NPC's
  reply), or else the first open page that offers or rewards a quest - the live server reopened Sir
  Tydus on War Preparations 750 ms after Message from a Friend's turn-in - or nothing;
- `unanswered` is a choice the capture never exercised; it is logged and nothing opens.

A page **and a choice** may carry a `when` condition: `quest_offer_open`, `quest_reward_open` or
`quest_active` (accepted, objective not done) for one quest id, or `answer` on a page only a choice
opens. A talk shows the first open page; a page with choices is open only while one of them is shown,
so a quest menu whose quests are all closed is not shown. A choice the character is not shown is
refused like a choice the page does not have. The offer and turn-in conditions are the quest pack's
(`crate::quest::offer_open`, `reward_open`): the character's level, professions and prerequisite quests
count, not only the attempt.

### Every captured NPC

`tools/npc-content-from-captures.py` writes the entries marked `generated` from preforged's six
pre-Searing captures (the collector pairs under `data/captures/`): 42 NPCs, every body and label the
live server's units. A quest choice's id is the retail one, `0x800000 + (quest << 8) + step`, and the
step is read from what the captures show around it:

| step | the choice | what the tool writes |
|---|---|---|
| 01 | accept | `quest` offer, on the offer page |
| 02 | decline | `close` |
| 03 | ask about the quest | `page` to the offer page, shown while the offer is open |
| 04 | a talk objective | `quest` credit for the quests whose objective is that talk (62 at Meerak, 79 at its trainers, 55 at Orion), else `page` to the reply; shown while the quest is active |
| 05 | "how is it going" | `page` to the reply, or `close`; shown while the quest is active |
| 06 | ask for the reward | `page` to the turn-in page, shown while the turn-in is open |
| 07 | take the turn-in | `quest` reward, on the turn-in page |

The NPC's own conversation choices (0x7F-0x8F) answer with the short page the capture shows next, or
`close`. An NPC's quest menu merges every variant the captures show into one page, each choice behind
its own condition. The order of an NPC's pages is its menu, its turn-ins, its offers, then its idle
greeting; a reply page waits for `answer`. NPCs with a service title (`[Merchant]`, `[Skills]`, ...)
are left to their service. Re-running the tool replaces the generated entries and keeps the rest (The
Purveyor's).

## 3. What this does not answer

- **The text.** The encoded units resolve through the client's string table; this server cannot print
  them, and the pack stores them so the client's own text is what appears.
- **Four choices.** 0x87, 0x8D, 0x8E and 0x8F were never chosen. Their labels are stored, their answers
  are not known, and they are marked `unanswered` rather than guessed.
- **The close choice.** 0x85 is likely the way out, but the capture never chose it; the client may close
  the dialog on its own, which is what `close` leaves it to do.
- **The kind-5 window's contents.** The live server sends the open message and the speaker, and nothing
  else before the player chooses again. What the client draws is its own; no server data follows.
- **The Purveyor's stock.** `vendors.json` has no merchant entry for this NPC, so choice 0x8B logs that
  it has no stock and opens nothing. The live window's two kit rows are a later item/service change; the
  dialog action is wired to the same merchant path Sanura uses.
- **Quest buttons.** The Town Crier and Sir Tydus pages carry them for quest 80; the other quest dialogs
  are not captured yet (`docs/58-quests.md`).

## 4. Acceptance, on a real client

1. `cargo test -p asura-world --lib dialog` (4 tests) and `cargo test -p asura-protocol dialog` (3).
2. Start the world and talk to The Purveyor once. The first talk logs `1 dialog(s) from .../Vanilla`.
3. Ascalon City (148), walk to The Purveyor at (11407, 5326) and talk. Seven choices appear under the
   greeting.
4. Choose 0x8C (the first, icon 28): window kind 5 opens.
5. Talk again, choose 0x8B (icon 24): the log says `vendors.json has no stock for it`; nothing opens.
   This is the one place the capture is ahead of the pack.
6. Talk again, choose 0x85 (icon 15): the dialog closes and nothing is sent.
