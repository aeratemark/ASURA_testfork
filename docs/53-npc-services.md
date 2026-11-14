# NPC services: talk, merchants, collectors, Xunlai storage and gold

**Status:** built 2026-11-12, on the owner's instruction "implement vendors, xunlai, npc interaction, maybe
improve on items based on all the captured packets in preforged". Not run against a client yet: the
acceptance steps are at the end. This document replaces `docs/42-vendor-system-research.md` section 7
(what this repository owes); sections 1-6 there stay as the research record.

## 1. The evidence

Two recordings answer most of it. Record numbers count `"event": "packet"` lines from 0.

| what | recording | where |
|---|---|---|
| a dialog, a choice and the merchant window it opens, both directions | `recordings/client/feature/movement/movement-live-20261109-200703.jsonl` | seq 5171-5288 |
| seventeen pre-Searing trade windows, one purchase, one collector exchange, the gold of every load | preforged `data/captures/pre-searing-side-quests-2026-10-12/checkpoint-1815/packets-43292-168891734.jsonl` (live server, server-to-client, the client's decoded structs, this build's numbering) | below |
| a whole-stack sale and a partial purchase | Rurik's retail runs, `docs/42` sections 3-4 | |
| the client's own reading of the requests | preforged `src/runtime/gw_collector_transaction.h`, `gw_inventory_gold.h`, `gw_storage.h` | |

To list the preforged records: read the file, keep `event == "packet"`, number from 0, print `opcode`.

### What the live server sends

| event | messages, in order | records |
|---|---|---|
| merchant talk | `0x00C4 {npc}`, `0x0161` per stock row, `0x0084 {ids}`, `0x00CA {1, 1.0f}`, `0x00C3 {11, 0}`, `0x009F {12, npc, 0}` | 487607-487618; live seq 5281-5288 |
| collector talk | `0x00C4 {npc}`, `0x0161` cost row (the trophy, quantity = cost), `0x0161` per offer, `0x0084 {cost id, offer ids}`, `0x00C5 {2, line}`, `0x009F {12, npc, 0}` | 537230-537236 |
| the same window again | `0x00C4`, `0x0084`, `0x00C5`: the rows are not declared twice | 467378 then 470670 (collector 0x21) |
| purchase (4 gold) | `0x014F {stream, 4}`, `0x0162` the new item, `0x013E {stream, item, bag, slot}`, `0x00CC {1}` | 487667-487670 |
| collector exchange | `0x0139 {trophy stack, left}`, `0x0162` reward, `0x013E`, `0x00CC {2}` | 537263-537266 |
| load gold | after the weapon sets: `0x0140 {stream, carried}` then `0x0141 {stream, stored}`, each only when not 0 | 472025-472026; `maps/pre-searing-load-3.jsonl` has neither |

Facts read off these:

- **A pre-Searing merchant or collector opens its window at once.** No `0x0080` dialog precedes any of
  the seventeen windows; the walk (`0x002A`) does.
- **A bought item is `0x0162`, not `0x0161`.** Rurik's note in `docs/42` section 3 said `0x0161`; the live
  record says `0x0162` with a new id.
- **Every file id carries bit 31**, in a window and on a bought item. Preforged strips it; the pack keeps it.
- **`0x0140` and `0x0141` are amounts added, not balances.** The reference (GWPS) sends the character's
  gold as `0x0141`, so a client showed it as storage gold and showed 0 carried. The load now sends what the
  live server sends (item.rs, `items_block_with_stream`). This was item review B7.
- `0x009F` kind 12 on the NPC, value 0, closes every answer to a talk (dialog and trade windows). Its
  meaning is not established; it is copied.
- `0x00CA` kind 3 opens another trader's buy pane (record 460410); not built.

### What is not captured

- A sale. The answer is Rurik's retail order: `0x014D` (whole stack) or `0x0139` (part), `0x0140`, `0x00CC {11}`.
- The Xunlai window. Built as preforged opens it (`0x0083 {npc, 0, 3}`), which preforged ran against a
  client that deposited and withdrew. The one live `0x0083` (seq 5247) is kind 5, after a dialog choice:
  another window.
- A gold move at Xunlai: preforged's reading (`0x807C` carries the two final balances; the answer is the
  debits `0x014F`/`0x0150`, then the credits `0x0140`/`0x0141`).
- Any refusal. A refused request is not answered, as preforged answers none.

## 2. The client's requests

Matched to this build by layout (preforged's client number plus two, `docs/52-skills.md`):

| here | preforged | layout | use |
|---|---|---|---|
| `0x804D` `TRANSACT_ITEMS` | 75 | kind, gold given, ids + byte quantities, gold asked, ids + byte quantities | buy (1), sell part (11), collector (2) |
| `0x804A` `BUY_MATERIALS` | 72 | the same without quantities: whole stacks | sell a stack (11), collector (2) |
| `0x807C` `ITEM_CHANGE_GOLD` | 122 | carried, stored: the balances to end with | Xunlai gold |
| `0x803B` `SEND_DIALOG` | 57 | the dialog id | a dialog choice |

## 3. What is built

| part | where |
|---|---|
| builders and readers | `crates/asura-protocol/src/game/vendor.rs`, `game/dialog.rs`; rows marked in `packets.rs` |
| the pack | `config/World/Vanilla/vendors.json`, written by `tools/vendors-from-preforged.py` |
| the rules (pure, tested) | `crates/asura-world/src/vendor/rules.rs` |
| the loader | `crates/asura-world/src/vendor/content.rs` |
| talk and the handlers | `crates/asura-world/src/vendor/mod.rs` |
| arrival -> talk | `approach.rs` (`Steered::arrived`), `instance.rs` (`Order::Talk`), `travel.rs` |
| storage only through a window | `inventory_model.rs` (`Wearer::storage_open`, `Refusal::StorageClosed`) |
| one transaction for items and gold | `asura-db` `RowChange::Gold` / `StorageGold`, `item::commit_ops` |
| storage gold | migration `0035_storage_gold.sql`, `db_game::storage_gold`, `Player::storage_gold` |
| test command | `/xunlai`: the storage window on the player's own agent |

### The pack

- 6 merchants (Sanura 148, Riley 163, Vencel 162, Gram 166, Spence 164, Laren 165) with the one stock
  list preforged captured for Sanura and found identical for Riley, Vencel and Gram.
- 8 collectors, 24 offers (preforged's catalog; Hatcher and Varis from its completed52044 capture).
  A character sees the offers for its primary profession and those for any.
- Storage NPCs by name on every map: "Xunlai Agent", "Xunlai Chest". Pre-Searing has none (wiki, via
  `docs/42` section 6), so storage is tried with `/xunlai`.
- An NPC is found by map and by its placement's literal name, compared without a bracketed title:
  `Sanura [Merchant]` is `Sanura`.

Two windows come from `tools/npc-content-from-captures.py`, which reads preforged's captures directly:
Hamish in Ashford (map 146), whose captured window is the same seven rows as Sanura's, and Gift-Giver
Pinn in Ascalon City (148), whose captured window is a collector's exchange - one row of item 37798 for
any of seven gift rows (37799-37805). The rows are the capture's; what the gifts are is the client's own
string table, which the capture did not resolve, and no pre-Searing drop or quest in this pack gives the
item Pinn takes.

### The rules

- Buy: `value x 1.0 x 2` per item, and the client's quoted gold must equal it; a non-stacking row sells 1.
- Sell: `value` per item, the client's asked gold must equal it, the item must be carried, value above 0,
  model not in `unsellable_models` (Starter Shield and Flute: the wiki says they cannot be sold), and the
  carried gold must stay at or under 100,000.
- Collector: exactly the offer's cost in its trophy (type, model and coded name match), from stacks the
  request names, each once.
- Xunlai gold: the total is kept; carried <= 100,000, stored <= 1,000,000.
- A trade or a storage move needs the window's NPC within `rules.service_range` (300, chosen).

## 4. Divergences, said plainly

- **Storage is per character.** Retail storage and its gold are per account. The bags already were the
  character's rows (migration 0006), so the gold sits beside them; moving both to the account is one
  later change.
- **Collector armour is not customised.** Preforged gives Hatcher's and Varis's armour the owner's name
  (`0x013A`, `customize`); this schema has no column for it, so the reward is uncustomised.
- **One dialog is built.** The Purveyor [Bonus Merchant's captured menu (seven choices, two answers)
  opens before its window (`docs/55-npc-dialogs.md`). No other NPC of this pack has a captured greeting;
  a dialog choice with no page to answer it is logged and closes the window.
- **No live sale, Xunlai or gold-move capture** (section 1).

## 5. Acceptance, on a real client

1. `cargo test -p asura-protocol game::vendor game::dialog`, `cargo test -p asura-world vendor
   inventory_model approach dispatch`, `cargo test -p asura-db` (migration 35).
2. Start the server; the log says `6 merchant(s), 8 collector(s) and 2 storage NPC name(s)`.
3. Ascalon City (148): walk to Sanura. The window shows seven items; buy the Flute (4 gold). The gold drops
   by 4 and the Flute is in the backpack. Selling it back is refused (unsellable, nothing happens). Buy the
   Focus (100) and sell it back: the gold rises by 50 and the Focus is gone.
4. Close and reopen: the window comes back without re-declaring (trace shows no `0x0161`).
5. `/xunlai`: the storage window opens; drag an item into a storage pane and back; deposit and withdraw gold.
   Relog: the gold shows as carried and stored, not as storage only.
6. A capture of each step against the live server replaces the "not captured" rows in section 1.
