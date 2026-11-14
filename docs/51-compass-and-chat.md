# 51 - Compass strokes, called targets and chat: the first slice from preforged

**Status, 2026-11-12:** built, not compiled in the workspace and not run against a client. None of the
messages below is in an ASURA capture. They are implemented from preforged
(`Other_Repos_Research/preforged-main`, a 37130 server) on this build's packet table, at the owner's
instruction, and every one is listed in section 3 as unmeasured.

## 1. What is built

| feature | client sends | server does | code |
|---|---|---|---|
| compass drawing | `GAME_CMSG_DRAW_MAP` (`0x802B`): stroke dword, 1-16 packed points | relays `0x0091` (drawer's player word, then the stroke unchanged) to the drawer's party members in the instance, not to the drawer | `compass::handle_draw_map` |
| called target | `GAME_CMSG_TARGET_CALL` (`0x8023`): kind byte, argument dword | kind 10 with an agent of this instance (player, NPC, ground item): `0x0034` (caller's player word, agent) to the whole party, caller included; other kinds logged | `compass::handle_target_call` |
| party chat | `#` line | the line goes to the speaker's party members in the instance (it went to the speaker alone) | `chat::handle_chat_message` |
| whisper | `"<name>,<message>` | the message on channel 14 to a player of that name in this instance; a server line back to the sender either way | `chat::handle_chat_message` |
| `/roll <sides>` | `/` line | `<name> rolls <face> on a <sides>-sided die.` on channel 6 to the instance | same |
| `/age` | `/` line | a server line: minutes this connection has been in this map | same |

The party is the one `crate::party` keeps: `compass::party_present` is the members standing in the
instance. A player the table does not know is a party of one.

The server keeps no stroke and no target. A late arrival is not shown earlier strokes.

## 2. Values

`config/World/Vanilla/chat.json`: `roll_min_sides` (2), `roll_max_sides` (100), `whisper_name_chars`
(19). A missing file refuses `/roll` and whispers with a logged reason.

The channel numbers (6 emote, 14 whisper) are the client's own and live in `asura-protocol`
(`CHANNEL_EMOTE`, `CHANNEL_WHISPER`).

## 3. Unmeasured: what a capture has to confirm

Each row is a question a live recording answers. Until then the packet table marks the four message
rows `Inferred`.

| # | claim | source | capture to take |
|---|---|---|---|
| 1 | `0x802B` is stroke dword + counted points, and `0x0091` is the drawer's player word in front of it | `gw_compass_wire.h` (37130 CMSG 41, SMSG 144); this build's table rows agree on the shape | two clients in one party, one draws |
| 2 | the drawer is not sent its own stroke | ASURA's choice; preforged echoes to the drawer too | the same recording, drawer's side |
| 3 | strokes go to the party only, not the instance | preforged `party_send` | a third client outside the party |
| 4 | `0x8023` kind 10 is a called target and `0x0034` is caller word + agent | `gw_callout.h` (37130 CMSG 33, SMSG 52) | Ctrl+click a target with a party member watching |
| 5 | the caller is sent its own `0x0034` | preforged | the same recording, caller's side |
| 6 | a whisper arrives as `0x005D` + `0x0061` on channel 14 | preforged `Kind::whisper` | a whisper between two clients in one outpost |
| 7 | the sender's own copy of a whisper | not known; this server sends a line of its own instead | the same recording, sender's side |
| 8 | a roll is `0x005D` + `0x0061` on channel 6 with the sentence as literal text | preforged `Kind::roll`; retail very likely sends a coded string | `/roll 100` where retail allows it |

`recordings/client/feature/movement/movement-live-20261109-200703.jsonl` holds 18 `0x0060`
(`CHAT_MESSAGE_GLOBAL`) records: channel 0, a sender name and a guild tag. That is the message a
cross-map whisper and guild chat would need. It is not used yet.

## 4. Not built from preforged's chat and callouts

| feature | why not |
|---|---|
| `/dance`, `/sit`, `/stand` | preforged sends its opcode 158 (kind 28 with a sequence hash) and an effects bit `0x2000`. The opcode's place in this build's table is not established, and the 127 captured `0x00F1` records carry other values (`0x00010000`, `0x00060000`). A wrong animation message can crash a client. Needs one capture of each emote. |
| health, energy, morale callouts | the server models health and energy in a fight, but there is no callout builder or measured opcode for a callout; experience has `/xp` (`docs/54-experience.md`) |
| cross-map whispers | `0x0061` names the speaker by an instance-local id; `0x0060` is the likely carrier (section 3) |
| `/health`, `/ip`, `/commands` | health is the fight's state and has no text answer yet; the others are text only and can follow. `/xp` now answers (`docs/54-experience.md`) |
| flood limits (8 lines, one more per 250 ms) | per-player state; the two numbers belong in `chat.json` |
| party ready status (`0x80AF` / `0x01D2`) | not in this slice |

## 5. The rest of preforged, by size

Read from the source tree on 2026-11-12: 819 files, about 7.4 MB.

| slice | files | size | depends on |
|---|---|---|---|
| compass, callouts, chat | 18 | 0.1 MB | this document |
| skill bar, attributes, skill catalogue, professions | 13 | 0.5 MB | nothing |
| quests (pre-Searing set, dialogue, rewards) | 35 | 0.2 MB | NPC interaction |
| NPC AI and dialogue | 25 | 0.1 MB | combat for most of it |
| merchant, collector, crafter, trade, storage, salvage | 23 | 0.1 MB | NPC interaction, the item framework |
| skill casting, per-skill effects, combat | 276 | 2.7 MB | skill bar, vitals |
| pets, arena, observer | 19 | 0.1 MB | combat |

## 6. Checks

| check | where | state |
|---|---|---|
| stroke and call parse, relay bytes, sizes | `asura-protocol` `game::tests` (2 new) | pass here |
| whisper split, roll bounds and faces | `chat::tests` | the pure functions pass here; the module needs a workspace run |
| dispatch names its two new arms | `dispatch::tests` (count 27 to 29) | needs a workspace run |
| two clients: draw, call, `#`, whisper, `/roll` | real client | to do |
