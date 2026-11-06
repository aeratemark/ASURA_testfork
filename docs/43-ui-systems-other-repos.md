# UI systems in the other repos: party, trade, skills, attributes, chat, friends

**Status:** research note, 2026-11-06.
**Question:** for each player-facing panel other than titles (`docs/40`), guilds (`docs/41`) and vendors
(`docs/42`), which repo in `Other_Repos_Research/` has the best evidence, and what ASURA takes from it.

The answer: no server in `Other_Repos_Research/` handles player-to-player trade or a multi-player
party on the current protocol. The current-protocol message lists for both are in Headquarter, a bot
*client* (`Headquarter/code/client/opcodes.h`, `trade.c`, `party.c`). Entice/gw-interface has the only
party state machine (invite, join request, kick, merge), but on 2013-era opcodes. Rurik is the deepest
source for the skills panel, attributes and the hero/henchman roster, all measured against a real
client. `docs/20-rurik-gap.md` already lists the Rurik ports; this note does not repeat them.

Tags: **OBSERVED** (a capture in this repo), **SOURCED** (another repo, named), **WIKI**, **GUESS**.

## 1. Who has what

| System | Rurik (Python, build 38797) | OpenTyria (C) | preservation (Go) | Entice + gw-interface (Elixir + C#) | GWLP-R (Java) | Headquarter (C, client) |
|---|---|---|---|---|---|---|
| More than one player | no, by design | yes | yes | yes | partly | - |
| Party of players | leave only | own party sent on load | invite logged, ignored | **invite, join request, kick, merge, group travel** | message classes only | **client side of all of it, plus party search** |
| Heroes / henchmen | **add, kick, roster, commands, hero skill bars** | hero add message only | - | - | message classes only | add/remove handlers |
| Player trade | none | - | - | - | one message class | **full client side** |
| Skills panel | **unlock sets, bar edits, quest grants** | sends bar and unlocks; no edit handlers | sends bar and unlocks | equip, swap, move, cast | display only | `SKILLBAR_LOAD` |
| Attributes | **spend and refund, client's own cost rules** | sends values | - | raise, lower, change secondary | display only | `ATTRIBUTE_LOAD` |
| Chat | All, plus `/bow`; refuses other channels | `!` `#` `$` | local | all, group, emotes | yes | all channels |
| Friends | - | empty list | - | list with online status | list end only | **info, status, location, add** |
| Storage (Xunlai) | refused | sends storage gold | - | - | message classes only | - |

SOURCED for every cell; files are named in the sections below. ASURA today: chat on `!` `#` `$`,
items, titles; none of the rest (`MILESTONES.md`).

## 2. Party (players)

**Pre-Searing:** player parties exist; henchmen do not (one exception, the Academy exit mission's
auto-fill), and there are no heroes (WIKI via `Rurik-main/studies/presearing/MANIFEST.md`, "Henchmen: 0").

**Message list, current protocol** - SOURCED from `Headquarter/code/client/opcodes.h`:

- CtoS `0x009C` accept invite, `0x009D` cancel, `0x009E` refuse, `0x00A0` invite player, `0x00A1`
  invite by name, `0x00A2` leave, `0x00A9` kick player, `0x00AF` ready status, `0x00B1` party travel,
  `0x00AA`-`0x00AE` party search.
- StoC `0x01C4` invite add, `0x01C5` join request, `0x01C6`/`0x01C7` invite/request cancel,
  `0x01CA` you are leader, `0x01CB` player add, `0x01D0` player remove, `0x01D1` ready, `0x01D2`
  create, `0x01D3` member stream end, plus `0x00B0`/`0x00B1` per-player party size.
- **Numbering warning:** GWCA (`GWToolboxpp-master/Dependencies/GWCA/include/GWCA/Packets/Opcodes.h`)
  puts the same StoC names one higher from about `0x01C0` (GWCA `0x01D3` create, Headquarter `0x01D2`).
  This repo's own sniffer names `0x01BE` PARTY_SET_DIFFICULTY and `0x01BF` PARTY_HENCHMAN_ADD
  (OBSERVED, `recordings/client/feature/guild/entering, leaving guildhall and leaving guild.jsonl`
  seq 459-460), which matches Headquarter at those two. Settle each number from a capture before use.

**Behaviour** - SOURCED from Entice:

- `entice/gw-interface/GuildWarsInterface/Datastructures/Party.cs` - the client-facing state: one
  party object per group, members, outgoing invites, incoming join requests, and the order the
  window messages go out (create, add leader, create-end; then `ShowPartyWindow` with the leader flag).
- `entice/gw-interface/GuildWarsInterface/Controllers/GameControllers/PartyController.cs` - the rules
  the leader is held to: only the leader invites, kicks a member, accepts or kicks a join request.
- `entice/entice/topic_group.md` - the server side: `merge` and `kick` requests, `update {leader,
  members, invited}` broadcasts, and group map change (leader travels, members follow and are
  re-invited).
- The opcodes in gw-interface (150-163) are the old protocol's. Take the state machine, not the numbers.

**What ASURA owes, in order:** capture two players forming, readying and leaving a party on the
reference (no capture exists yet); then invite / accept / leave / kick, with party travel last.

## 3. Player-to-player trade

**Pre-Searing:** trade between players exists, and it is the only way to get dye (WIKI via MANIFEST,
"Dye trading: no NPC, player-to-player only").

**Message list** - SOURCED from `Headquarter/code/client/opcodes.h` lines 47-53, 90, 149-156, and
`trade.c`:

| CtoS | | StoC | |
|---|---|---|---|
| `0x0049` | initiate (partner agent) | `0x0000` | trade request (player id) |
| `0x0000` | acknowledge (player id) | `0x0005` | acknowledge |
| `0x0002` | add item (item id, quantity) | `0x0004` | add item |
| `0x0005` | remove item (item id, quantity) | `0x0008` | offered count |
| `0x0003` | send offer (gold) | `0x0003` | receive offer |
| `0x0006` | cancel offer | `0x0002` | change offer |
| `0x0007` | accept | `0x0006` | accept |
| `0x0001` | cancel | `0x0001` | terminate |

- Headquarter answers a trade request by acknowledging at once (`trade.c:6-26`) and closes its trade
  state on terminate (`trade.c:28-48`). Field layouts are in each handler's packed struct.
- GWCA `Context/TradeContext.h` and `Managers/TradeMgr.h` give the client's own trade window state.
- No server implements the exchange. The server-side order (both offers locked, both accept, items
  and gold move, terminate) needs a two-account capture.

## 4. Skills panel

SOURCED from Rurik: `toolkit/authsrv/skillunlock.py` (header), `studies/skills/FINDINGS.md`.

- **Two unlock sets, two scopes.** `0x00DB` is the character's learned skills (what the Skills panel
  lists); `0x001D` is the account's (what the client checks when a skill is dragged onto the bar).
  Neither contains the other: Rurik's capture `20260817T231139` had 21 character ids against 19 account
  ids.
- `0x00DB` is 128 dwords of bits. Three client crashes, all measured by Rurik: bit 0 set (skill 0)
  asserts `*skill` when the panel opens; a bit past the end of the build's skill table asserts in
  `ChCliSkill.cpp`; setting every row asserts `fileId` on the icons.
- Bar edits, casting, quest skill grants: `docs/20-rurik-gap.md` covers the ports.
- **Pre-Searing:** one skill trainer, Halbrik, level 10 and up (WIKI via MANIFEST); his window is
  vendor kind 10 (`docs/42` section 2), which needs a field nobody has measured.

## 5. Attributes

SOURCED from Rurik `toolkit/authsrv/attribspend.py` (header) and `studies/pvpui/FINDINGS.md` 32.

- The client predicts a spend locally, sends `0x000E` / `0x000F` with a sequence number, and waits for
  the server to retire the prediction with `0x0036`, then state the truth with `0x0038` and `0x003B`.
- The cost of reaching a rank is the client's own table (`s_attribPoints`); leaving a rank refunds the
  same. Nine live rank changes in Rurik's capture `20260818T132739` price out exactly.
- Rank 13 is refused (`s_attribPoints[12] = -1`). An attribute is refused when the character lacks its
  profession, or when it is another profession's primary attribute (no Strength on a secondary
  Warrior).
- Entice's `AbilitiesController.cs` shows the same requests on the old protocol (increase, decrease,
  change secondary profession). The numbers there are old.

## 6. Chat

SOURCED from Rurik `studies/chat/FINDINGS.md`.

- `0x005D` carries a body fragment; a commit message follows it: `0x005E` server line, `0x005F` agent
  line, `0x0061` player line (sender player id, channel). A bare `0x005D` renders nothing.
- Retail splits long bodies at **121** units per fragment. OpenTyria uses 122
  (`CHAT_MESSAGE_FRAGMENT_MAX_LENGTH`). ASURA's chat came from that lineage; check which cap
  `asura-world` uses against a long line on the reference.
- Guild and alliance chat use `0x0060` CHAT_MESSAGE_GLOBAL (sender name, guild tag); GWCA `StoC.h`
  `MessageGlobal`. Never seen in Rurik's corpus (0 of 0x0060).

## 7. Friends

- Friends live on the **auth** connection, not the game one. SOURCED from `Headquarter/code/client/opcodes.h`
  lines 23-44: AUTH_CMSG `0x001A` add friend; AUTH_SMSG `0x000A` info, `0x000B` status, `0x0020`
  location, `0x0014` stream end. Handlers in `friend.c:48-180`.
- OpenTyria sends only the stream end (an empty list); Entice built a list with online status and
  current character on its own protocol (`entice/entice/milestones.md`, milestone 11).
- The game connection's `0x000E` FRIENDLIST_MESSAGE also appears in this repo's guild capture
  (OBSERVED, seq 4542 and 4998, right after `/` chat commands); it is a whisper or friend-command
  answer, GUESS.

## 8. Heroes and henchmen

Rurik is the only implementation (`toolkit/authsrv/henchparty.py`, `heroqueue.py`,
`studies/heroes/FINDINGS.md`): add (`0x009F` with the henchman's agent), kick (`0x00A8`), the roster
row `0x01BF` after `0x00B0`, and the henchman pre-create burst order. Pre-Searing has no henchmen in
outposts and no heroes (WIKI), so this is out of scope apart from the Academy auto-fill.

## 9. Storage

Pre-Searing has no Xunlai agents or chests (WIKI via MANIFEST). Out of scope.

## 10. Next captures, in order

1. Two accounts in Ascalon City: invite, accept, ready, leave, kick (section 2).
2. Two accounts: a trade with one item and some gold, accepted; a second one cancelled (section 3).
3. One account: spend and refund attribute points; edit the skill bar (sections 4-5), to check
   Rurik's build 38797 numbers against this build.
4. A whisper and a friend add (section 7).

## 11. Sources

- `Other_Repos_Research/Headquarter/code/client/` - `opcodes.h`, `party.c`, `trade.c`, `friend.c`.
- `Other_Repos_Research/entice/` - `entice/milestones.md`, `entice/topic_group.md`,
  `gw-interface/GuildWarsInterface/Datastructures/Party.cs`, `Controllers/GameControllers/*.cs`.
- `Other_Repos_Research/Rurik-main/` - `toolkit/authsrv/skillunlock.py`, `attribspend.py`,
  `henchparty.py`, `studies/chat/FINDINGS.md`, `studies/skills/FINDINGS.md`,
  `studies/presearing/MANIFEST.md`.
- `Other_Repos_Research/OpenTyria-master/code/OpenTyria/` - `GmParty.c`, `GmChat.c`, `GmPlayer.c`.
- `Other_Repos_Research/preservation-main/gameservice/handlers.go` - `onPartyInvite` logs and returns.
- `Other_Repos_Research/GameRevision-GWLP-R-4208ffa/protocol/` - generated message classes (old build).
- `Other_Repos_Research/GWToolboxpp-master/Dependencies/GWCA/include/GWCA/` - `Packets/Opcodes.h`,
  `Packets/StoC.h`, `Context/TradeContext.h`.
