# The vendor system: merchant windows, buy and sell

**Status:** research note, 2026-11-06.
**Question:** what a server sends to open an NPC vendor window, what the client sends to buy and sell,
and what this repository owes for pre-Searing.

The answer: a vendor window is four server messages - `0x00C4` owner, `0x0161` per stock item, `0x0084`
stock list, `0x00CA` open - plus `0x00C3 {kind, 0}`, which says what kind of vendor it is (11 =
merchant with Buy and Sell tabs). A buy is CtoS `0x004D`, answered with pay `0x014F`, mint `0x0161`,
place `0x013E`, confirm `0x00CC`. A sell is CtoS `0x004A`, answered with remove `0x014D`, credit
`0x0140`, confirm `0x00CC`. Rurik built and ran both directions against a real client; nothing here
handles either (`MILESTONES.md`: "Buy and sell (`0x804D`)" unchecked; `docs/20-rurik-gap.md` P6).

Tags, as in `docs/40-titles-system-research.md`:

- **OBSERVED** - a capture in this repo.
- **SOURCED** - another repo's source, capture or measurement; the repo and file are named. Rurik's
  runs are against its own server and a real client (build 38797/38833), so they are SOURCED here,
  not OBSERVED.
- **WIKI** - Guild Wars Wiki, via Rurik's pre-Searing manifest.
- **GUESS** - an inference; it says what would confirm it.

**No vendor capture exists in this repo yet.** A scan of every file under `recordings/client/feature/`
for `0x00C3`-`0x00CD`, `0x004A`, `0x004D` finds none (2026-11-06). The one vendor-family message seen is
`0x00CC {8}` after guild creation (`docs/41-guild-system-research.md` 3.1). Section 6 says what to
capture.

## 1. Opening a merchant window

SOURCED: `Other_Repos_Research/Rurik-main/studies/newopcodes/FINDINGS.md`, sections "`0x00CA` - IT OPENS
THE SHOP" and "`0x00C3` IS THE SELL TAB" (harness runs `20260818T211036`, `20260819T173300`), replaying
the order of retail's own sighting (`20260817T183756`, conn 58389).

```
0x00C4 [owner agent]           window owner; also turns the player to face the NPC
0x0161 x N                     declare each stock item (price is the item's value field)
0x0084 [[id, id, ...]]         stage the stock ids into the client's list buffer
0x00CA [1, 0x3F800000]         open: draws the panel with the staged rows (second field 1.0f, as retail)
0x00C3 [11, 0]                 window kind 11 = merchant: adds the Buy / Sell tab pair
```

- `0x00CA` alone opens a buy-only panel. Without `0x00C3` the panel has no Sell tab and the client
  never sends `0x004A`.
- `0x00C4` is a generic **window owner**, not merchant-specific: one writer, eight reader opcodes
  (`0x00C3`, `0x00C5`, `0x00C7`-`0x00CB`), covering merchants and traders.
- In all six retail windows in Rurik's corpus the owner field was 272 or 273, the stock was 11 items,
  and `0x00C3` was `[11, 0]`.
- The client rendered names, icons, stats and prices from the `0x0161` declarations alone. The price
  column showed each item's `value` (Rurik calls it `F9`); the buy price the client quotes is value x 2.
- The first stock row is pre-selected when the window opens.
- Crash found by Rurik: sending `0x00C3` before the stock is staged and opened asserts `item`
  (`ItCliApi.cpp(859)`). Keep the order above.

## 2. The window kind enum (`0x00C3` field 1)

SOURCED: Rurik, same file, "THE WHOLE `0x00C3` ENUM, 0..15" (one run per kind, 2026-08-19), and
`toolkit/clientscan/vendorscreens.py`, which lists the client's own vendor screens from the source file
names under `Ui\Game\Vendor`.

| Kind | Second tab | Client screen | Result |
|---|---|---|---|
| 0, 1, 3 | - | an item-list vendor (collector / crafter / trader family, GUESS) | client dies: `item`, `ItCliApi.cpp(859)` |
| 2 | - | - | client dies: access violation at address 2 |
| 4 | Customize | `VnCustomize` | draws |
| 5 | - | `VnGuildAddService` | dies: `service < GUILD_SERVICES` |
| 6 | - | `VnGuildAdjustFaction` | dies: no `faction` field sent |
| 7 | (no label) | unidentified | draws |
| 8 | Guild Registration | `VnGuildRegister`, tall panel | draws |
| 9 | Guild Cape | `VnGuildTabard*`, tall panel | draws |
| 10 | - | `VnLearnSkill` (skill trainer) | dies: `index < arrsize(s_skill)` |
| **11** | **Sell** | `VnBuy` / `VnSell` | **merchant, both directions work** |
| 12 | Buy | a second buy pane | draws |
| 13 | Sell | a second buy/sell pair | draws |
| 14 | Unlock Hero | `VnUnlockHero` | draws |
| 15 | Unlock Item | `VnUnlockItem` | draws |

- "Dies" means the kind reached its own screen and refused a field Rurik never sent - not that the kind
  is invalid. Each fatal kind needs one more field, which is unmeasured.
- The collector window is not on this list: it is `0x00C5`, a different message (Rurik saw it once in
  retail, owner 279, three stock items).
- The same number comes back in the transaction: `0x004A` carries 11 and `0x00CC` echoes the kind
  (1 for a buy, 11 for a sell, 8 for guild registration - see section 5).

## 3. Buying

SOURCED: Rurik `toolkit/authsrv/merchant.py` `handle_item_purchase`, and FINDINGS "THE PURCHASE
COMPLETES" (`20260819T170427`). Retail's order was read by byte offset from one retail purchase
(`20260819T132414`, conn 55414, build 38833).

Request, CtoS `0x004D` (this repo's `0x804D`, `GAME_CMSG_TRANSACT_ITEMS`):

```
retail  [1, 40, [], b'', 0, [2474], b'\x01']
         |  |                 |       quantity byte per id
         |  |                 item ids (stock ids)
         |  quoted price (value x 2)
         kind: 1 = buy
```

Reply, in this order:

```
0x014F [inventory key, price]          debit, before the item exists
0x0161 [new id, ...]                   mint a copy of the stock row under a NEW item id
0x013E [inventory key, new id, bag, slot]   place it (retail: the backpack)
0x00CC [1]                             transaction done, echoing the kind, always last
```

- Bought items get a new id (retail 2474 -> 4130): stock is a catalogue, a buy mints a copy.
- Rurik's first version sent the four replies in the wrong order and the client died on `0x013E`
  (`Assertion: item`, `ItCliApi.cpp(1883)`), because the item was not declared yet. The wrong order came
  from a script that sorted same-timestamp messages by opcode. Order these by stream offset only.
- The client refuses a purchase locally when it cannot afford it (Buy greyed at 0 gold; observed by
  Rurik at zero funds only) or has nowhere to put it - no `0x004D` is sent. No retail
  "insufficient funds" reply was ever seen.
- `0x014F`'s first field is the inventory key (the `0x0144` stream key). The same 0xFA appears in this
  repo's guild capture (`docs/41`, 3.1), so in that session the inventory key was 0xFA.
- `0x014F` debit is n = 1 on retail's wire; `0x0140` credit is corroborated across many loads, sells
  and quest hand-ins (Rurik merchant.py header).

## 4. Selling

SOURCED: Rurik `merchant.py` `handle_item_sale`; eight retail sales in `20260819T132414`.

Request, CtoS `0x004A`: `[11, 0, [item id], price, []]` - kind 11, then 0, the item ids, the price at
index 4 (not index 2 as in a buy), an empty list.

Reply:

```
0x014D [inventory key, item id]   the item is gone
0x0140 [inventory key, price]     credit
0x00CC [11]                       done
```

- The client asks half the buy price (retail 40 -> 20; Rurik 10 -> 5) and retail credited exactly the
  asked price 8 of 8 times. Whether retail recomputes it or trusts the client cannot be told from the
  wire. A server must compute it itself.
- No `0x013E` on a sell: the item stops existing.

## 5. The vendor kind ties into guilds

This repo's guild capture (`recordings/client/feature/guild/guild created, people invited.jsonl`,
seq 308-317) shows CtoS `0x0036 {100, name, tag}` answered by `0x014F {0xFA, 100}` and `0x00CC {8}`.
OBSERVED. With section 2 (kind 8 = Guild Registration) this reads as: guild creation is a
vendor transaction at the Guild Registrar's window, kind 8, price 100. The `0x0036` dword is then the
quoted price, as field 2 of `0x004D` is. GUESS until a capture shows the registrar's `0x00C3 [8, 0]`.

## 6. Pre-Searing scope

WIKI via `Other_Repos_Research/Rurik-main/studies/presearing/MANIFEST.md` lines 800-860:

| Service | In pre-Searing | Window kind |
|---|---|---|
| Merchant buy/sell (no bags, no expert salvage kits) | yes | 11 |
| Weaponsmith (weak stock) | yes | unknown (a crafter kind, GUESS 0/1/3) |
| Collectors, general and armor (13 collectors in the census) | yes | `0x00C5`, not `0x00C3` |
| Skill trainer (Halbrik only, level 10+) | yes | 10 |
| Guild Registrar (Ascalon City; no hall from the mainland) | yes | 8, 9 |
| Xunlai storage | no | - |
| Armorer | no | - |
| Dye trader | no | - |

## 7. What this repository owes

In order, smallest first:

1. Capture first: a sanitised `sniff-capture.py` session on the reference that talks to the Ascalon
   City merchant, buys one item and sells one item; then the same against Halbrik and a collector.
   That gives the window kinds and the exact retail `0x0161` stock fields for pre-Searing, which Rurik
   does not have (its stock was Factions armour).
2. `asura-protocol`: structs for `0x00C4`, `0x0084`, `0x00CA`, `0x00C3`, `0x00CC`, `0x014D`, `0x014F`,
   `0x0140` (check which exist), and the two requests `0x804D` and `0x804A`.
3. `asura-world`: open the merchant window on NPC interaction (the `0x0039` INTERACT_LIVING path),
   stock from config (one JSON per merchant, per `AGENTS.md` section 4), never from code.
4. `asura-rules`: price = value x 2 to buy, value to sell back (the client's ask is checked, not
   trusted); refuse when gold or bag space is short.
5. `asura-db`: gold and the new item persisted in the same transaction.
6. Tests for both reply orders; the acceptance test is `docs/20-rurik-gap.md` P6: a buy and a sell
   over a socket, with gold and the bag both correct afterwards.

## 8. Sources

- `Other_Repos_Research/Rurik-main/studies/newopcodes/FINDINGS.md` - window open sequence, `0x00C3`
  enum, purchase and sale round trips, crash sites.
- `Other_Repos_Research/Rurik-main/toolkit/authsrv/merchant.py`, `purse.py` - buy/sell handlers and the
  retail reply orders.
- `Other_Repos_Research/Rurik-main/toolkit/clientscan/vendorscreens.py` - the client's vendor screen list.
- `Other_Repos_Research/Rurik-main/studies/presearing/MANIFEST.md` - pre-Searing services (WIKI).
- `Other_Repos_Research/Headquarter/code/client/opcodes.h` - names `GAME_SMSG_MERCHANT_WINDOW_OPEN`
  (`0x00C3`), `TRADER_WINDOW_OPEN`, `WINDOW_ADD_ITEMS`; `item.c` client-side handlers.
- `Other_Repos_Research/GWToolboxpp-master/Dependencies/GWCA/include/GWCA/Managers/MerchantMgr.h` -
  the client-side transaction API.
- `recordings/client/feature/guild/guild created, people invited.jsonl` - the only vendor-family
  message in this repo (`0x00CC {8}`).
