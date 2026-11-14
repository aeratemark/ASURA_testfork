# The item system: review, and the framework to replace it

Reviewed 2026-11-09 against the tree as it stood that night: `crates/asura-world/src/item.rs`,
`item_catalogue.rs`, `item_modifier.rs`, `creation.rs`, `crates/asura-db/src/inventory.rs`, `inventory/seed.rs`,
migrations 0006 and 0033, `config/World/Vanilla/items.json` (546 definitions) and docs 08, 47 and 48.

This document replaces the roadmap in `docs/47-item-system.md` §7. Docs 08, 47 and 48 stay as
research records. Where they disagree with this one, this one wins (see §5).

## 0. Status (2026-11-09)

What has landed, and what it fixed:

| piece | where | fixes |
|---|---|---|
| The rules core: `ItemKind`, `BagKind`, `InvModel::apply(Op) -> Vec<Change> \| Refusal`, atomic, no dependencies, 20 unit tests | `crates/asura-world/src/inventory_model.rs` | B2, B4, B5, B8 (stack limit on pickup and grant); B3 still open |
| One transaction per operation (`apply_changes`, character row locked, swaps written in two steps), and one `t_item` INSERT instead of three | `crates/asura-db/src/inventory.rs` | D2 (partly), B2 |
| Every item handler runs rules → transaction → reload → frames, with refusals snapping the client back | `crates/asura-world/src/item.rs` | D1 |
| Drops, pickups and equipment changes are broadcast to the rest of the instance (`ground_frames`, `WORLD_REMOVE_AGENT`, `0x006F`) | `item.rs`, `player.rs` uses the same `ground_frames` | B1 |
| Rows that cannot be wire values are refused when the model is built, not sent as 0 | `item.rs` `model_of` / `entry_of` | B6 (for operations; the load block still uses `wire_*`) |
| `instantiate(definition, id, qty)`: the one place a definition becomes a row (used by `/item`) | `item.rs` | D2 (partly) |
| The Py4GW-style `Item::Get*` / `Inventory::Get*` façade is gone; `reload` moved from `creation.rs` to `item.rs` | | D5 |
| Tests: `one_operation_is_written_whole_or_not_at_all` (DB); `an_item_can_be_taken_off_and_worn_again` now expects the measured `0x006F` after an equip, which the handler already sent | `asura-db/tests/inventory.rs`, `asura-world/tests/load.rs` | |

Behaviour that changed on purpose:

- **An equip onto a worn slot** sends the old piece to a free carrying slot that is never the one the new piece is leaving. An in-place swap (that slot) put two items in one slot in the client's model and crashed a live client on 2026-11-09. With no other free slot the equip is refused, as before. B3 stays open until a full-bag swap is captured.
- **A two-handed weapon and an off-hand** push each other off. The displaced piece goes to the first free carrying slot.
- **Armour of another primary profession** is refused. A character with profession 0 (it predates the column) is not held to this.
- **Equipping from storage, unclaimed or material storage** is refused. So is any item dragged into the equipped bag outside its own slot, and anything that isn't a material dragged into material storage.
- **A pickup that would push its stack past the limit** becomes a row of its own instead of overflowing the stack.

Still the same, on purpose: a drag onto an occupied carrying slot is refused and snapped back,
because retail swaps or merges and that answer has not been captured. A drag in or out of the
equipped bag still answers this client with the bulk `0x006E`.

Not done yet:

- **Storage gold (B7).** Needs a capture.
- **The catalogue split and `ItemKind` overrides per definition (P3).** `item_catalogue::equip_positions` is still unused; the model's kind table is the rule.
- **Starting equipment as definitions.**
- **Deleting the old single-purpose DB functions.** That's `move_item`, `take_from_stack`, `merge_into_stack`, `restore_item`, `create_item`, `destroy_item` and `find_in_carrying_bags`. The world no longer calls them, and their DB tests still do.
- **Newtypes in the DB layer.**

## 0.1 Base + upgrade (2026-11-09)

An item is a **base** with **upgrades** fitted to it. "Knight's Wyvern Boots of Superior Absorption"
is the Wyvern Boots base with two upgrades: Knight's (insignia, `0x1f9`) and Superior Absorption (rune,
`0xfe`). Until now each probed copy was its own unrelated definition, with the client's composed
display name stored as its name. The client composed the upgrade names a second time from the
modifier words, which is how "Knight's Knight's Templar Helm of Vitae of Vitae" appeared.

**What the live server sends** (584 live `CREATE_NAMED_ITEM` records in
`recordings/client/feature/{titles,multiplay,maps}`): 83 of them carry upgrades, and **none** has a
composed name. An upgraded staff goes out named `0x28c7`, with its upgrades only in the modifier
words. (These records are the client's struct: name at offset 48, 64 units; modifier count at 176;
words after it. The sniffer drops the final 4 bytes, so the last word of each record is missing.)

**The word grammar**, read from 173 upgraded probes and checked on every one:

| words | meaning |
|---|---|
| `0x2408XXXX` (identifier 576) | an upgrade fitted to the item, id `XXXX` |
| `0x2530`/`0x2532` + `YYYY` (identifier 595), `YYYY` = 2×id or 2×id+1 | that upgrade's pair word; exactly **one** effect word follows it |
| the same with bit 31 set (`0xa530…`), at the end of the list | an insignia's pair word and effect (`a53003f2 a7f80300` is Knight's) |
| everything else | the base's own words: armour rating `a3c8…`, the warrior armour pair `80400000 a0f81400`, energy `62c8…`, requirement `2798…` |

An attribute rune repeats its pair word, so it has two effects (the bonus `21e8…` and its health cost
`20d8…`).

**Values roll.** The *type* of each upgrade word is fixed per upgrade, and all 139 upgrades agree on
every item they appear on. The *value* is the item's own: Vitae is always Health +10, but weapon mod
`0x9d` is Health +28, +10 and +30 on three items, and two Dragon Swords have requirement 9 vs 12. A
base also does not have one wire identity: those two Dragon Swords have model ids 402 and 2249. So a
catalogue entry is an **observed copy** of a base, not the base itself.

**What changed:**

- `config/World/Vanilla/upgrades.json` (new, 139 upgrades): id, role (`prefix`/`suffix`/`unknown`),
  the name fragment the client composes in, and the word shape. 61 have a role and fragment; the rest
  are ones the client never names (inscriptions, built-in mods).
- `items.json`: every entry gained `base` (e.g. `t16_8102_a19`, Templar Helm) and `upgrades` (the ids
  its words carry). The 489 probed entries' `name` is now the bare base name. Wire identity and
  modifier words are unchanged.
- `item_catalogue.rs` loads `upgrades.json` and refuses to start when:
  - an entry's `upgrades` differs from the upgrade ids in its words;
  - an entry lists an upgrade that `upgrades.json` does not define;
  - an entry with upgrades carries a composed name (the doubling bug, as a startup error).
- `item_modifier.rs` decodes identifiers 576 and 595 (`Upgrade { id }`, `UpgradePair { id }`), and
  `fitted_upgrades(words)` lists an item's upgrade ids. Every owned row can now answer "which base,
  which upgrades" from its own words.
- `tools/split-item-upgrades.py` does the split and is idempotent. `tools/import-live-items.py` runs
  it after every probe import, so a re-import cannot bring the composed names back.

**Rows already in the database** keep the name they were written with. Items granted with `/item`
before this change still show the doubled name; trash them, or `/item clear`, and grant again.

**Not done yet:**

- Applying an upgrade to an item and salvaging one out. These need the `Op`s and a capture of the
  live answer.
- Rolling values for new loot.
- Collapsing copies into one base row with value ranges.
- Where a newly fitted upgrade's words go in the list. For an observed copy the probe's order is kept
  exactly. A composed item needs a rule, and the data suggests runes after the base's leading words
  and insignias at the end.

## 0.2 Upgrade rules, from the Guild Wars Wiki (read 2026-11-09)

Sources: `Upgrade component`, `Rune`, `Insignia`, `Inherent modifier`, `Salvage` and `Identification`
on wiki.guildwars.com. `List of weapon upgrades` answered with a maintenance notice and still needs
reading. These are retail behaviour to implement against, not measurements of the wire; each one
still needs a capture before the server sends anything new.

**Slots per item type** (P = prefix, S = suffix):

| item | P | S | notes |
|---|---|---|---|
| armour | insignia | rune | profession insignias and runes only on that profession's armour |
| axe / hammer / sword / daggers / scythe / bow / spear | haft / haft / hilt / tang / snathe / string / spearhead | grip / grip / pommel / handle / grip / grip / grip | |
| staff | staff head | staff wrapping | |
| wand / shield / focus | — | wrapping / handle / core | only on inscribable (Nightfall+) items |

- **Prophecies/Factions weapons:** 1 inherent mod + 1 suffix. **Prophecies/Factions off-hands:** 2
  inherent mods, no slots. **Nightfall+:** inscription + suffix. Inscriptions are Nightfall, Eye of
  the North and PvP only, so they are not needed for pre-Searing.
- **Inherent mods** are part of the item. They cannot be changed or salvaged, and they never rename
  it. In our data, every one of the 21 upgrades whose pair word is `0x2532` is unnamed, while 46 of
  the 96 `0x2530` ones are named. **Hypothesis:** `0x2532` marks an inherent mod and `0x2530` a
  removable one. To confirm: salvage from one of each.

**Naming:** prefix before the base, suffix after it ("Fiery Sword of Fortitude"). Inscriptions do not
rename. That is exactly the client's `0xa30`/`0xa31`/`0xa33` templates, and the reason the server
sends only the base name.

**Applying:**
- Double-click the component, then click the target item.
- A new component overwrites the one in that slot, and the old one is lost unless salvaged first.
- Weapons must be identified first.
- Green (unique) weapons and Prophecies/Factions off-hands take no mods.

**Stacking** (for the future equipment-bonus sum, not for the inventory):
- Same-attribute runes do not stack: the largest applies, but their health penalties do add up.
- Absorption does not stack, and the condition-reduction runes do not stack with each other.
- Vitae and Attunement do stack.
- Attribute runes stack with a headpiece's bonus.
- An insignia counts only for the piece it is on.

**Salvage:**
- Upgrades need an Expert, Superior, Infinite Superior, Charr or Perfect kit. A basic kit gives
  materials only.
- The player picks which component to salvage. The component always comes out; the item survives
  50% of the time (more with Treasure Hunter or Wisdom). A Perfect kit never destroys the item.
- Starter, quest, unique and PvP items, and upgrade components themselves, yield nothing.
- **Pre-Searing:** the only routes are **Charr Salvage Kits**, which drop from charr bosses in the
  Northlands, and an Infinite Superior kit.

**Identification:** an unidentified item hides its upgrades and inherent mods. Identifying reveals
them and makes them salvageable. Stackables, quest items and vendor or collector items cannot be
identified.

**Holding runes** go on bags, not on armour: +5 or +10 slots, and they add no name suffix. They are
the bag-capacity case of the same system, and phase P2's bag definitions should model them.

## 1. Verdict

The wire layer is good and should be kept. That covers the byte-exact load block, the captured
answers to drop, pickup, trash, equip and unequip, the modifier decoder, and the rule that a
message nobody has measured is never sent.

Everything above the wire is a set of handlers, each written on its own: a SQL call, then frames
built by hand, then a full reload of the inventory from the database. There is no inventory
model. Rules are split between SQL (`move_item`), handler code (`equip_item`), a protocol-crate
table (`game::equipped_slot`) and catalogue fields nobody reads (`equip_positions`, `max_stack`,
`restrictions`). The definition/instance split exists in name only, and the catalogue file has
turned into a dump of probe output.

## 2. What is wrong, ranked

### 2.1 Correctness bugs (a player can hit these today)

| # | bug | where | what happens |
|---|---|---|---|
| B1 | **A drop or pickup never reaches other players.** `drop_item` sends the ground agent only to the player who dropped it. `interact_item` sends `WORLD_REMOVE_AGENT` only to the player who picked it up. Nothing calls `instance.broadcast`. | `item.rs:328-340`, `432-447` | Others see the item only if they enter the map later (`player.rs:665`). After a pickup, everyone else still sees a ghost item. If they click it, the server says "not a ground item" and they get no answer. |
| B2 | **Equip is two transactions.** The occupant is moved off first, then the new piece is moved on. If the second move is refused, the first has already been committed, and the occupant's visual-clear frame has already been queued. | `item.rs:569`, `600` | The character ends up wearing nothing in that slot. |
| B3 | **Equip needs a free bag slot even when it is a swap.** The occupant is sent to `first_free_carrying_slot`, not to the slot the incoming item is about to free. | `item.rs:558` | With a full backpack you cannot swap armour. Retail swaps the two items in place. |
| B4 | **Equip has no rules beyond item type → slot.** Nothing checks the profession on armour, nothing stops a two-handed weapon being worn with an off-hand, and nothing checks which bag the item comes from. | `item.rs:518-531` | A warrior can wear monk armour. A bow and a shield can be worn together. An item can be equipped straight out of storage, material storage or the unclaimed-items bag. |
| B5 | **`ITEM_MOVE` skips even those rules.** A move into the equipped bag only checks slot range and emptiness. A move into material storage or unclaimed items accepts any item. | `inventory.rs:920-1006` | A cupcake can be dragged into the helmet slot, and the server stores it there. The bulk `0x006E` (`equipment_frame`) is then sent mid-session, which the live capture never does. |
| B6 | **Wire values are silently turned into 0.** Every conversion is `try_from(..).unwrap_or(0)`. | `inventory.rs:89,97`; `item.rs:183-195` | A bad row sends file id 0 or name unit 0 instead of failing. That is the same class of fault as the missing-texture crashes in doc 48. It should be refused when the row is loaded. |
| B7 | **Storage gold is sent as character gold.** `update_gold_storage(ITEM_STREAM, player.traits.gold)` | `item.rs:96` | The Xunlai panel shows the character's purse. Verify against a capture; I think the field is wrong. |
| B8 | **No stack rules anywhere in the live path.** Picking up merges with `saturating_add` and no limit. A move onto an identical stack is refused instead of merged. | `inventory.rs:1101`; `inventory.rs:981` | Can't overflow yet, because nothing else adds to stacks. It becomes a duplication or overflow bug as soon as loot or merchants exist. |

### 2.2 Design problems (why it keeps getting sloppier)

- **D1 – No domain layer.** Each handler is DB call → hand-built frames → `reload()`, which
  sits in `creation.rs`. The in-memory `player.inventory` is a cache rebuilt by a full `SELECT`
  after every action. Some handlers decide from the cache (`equip_item`) and some from the
  database (`drop_item`). None of the rules can be unit-tested without Postgres.
- **D2 – Five ways to make an item, three copies of the INSERT.** `seed_default` (its own inline
  17-column INSERT), `create_character` (`insert_row`), `restore_item` (a third INSERT),
  `grant_definition` (builds a `NewItem` by hand from a definition), and `creation.rs` (builds rows
  from the seed and from `ItemSpec`, twice: `creation_inventory` and `creation_equipment_rows`).
  Adding one column means editing all of them.
- **D3 – The definition/instance split is nominal.** Migration 0033 warns against copying
  definition fields into `t_item`, yet `t_item` already holds every one of them. Starting
  equipment writes `definition_id = NULL`, so most items in a real database have no definition.
  The catalogue's rule fields have no callers: `equip_positions`, `can_equip_in`,
  `accepts_quantity` and `restrictions` are used nowhere outside tests. The live rule is
  `game::equipped_slot`, and it sits in the protocol crate.
- **D4 – The catalogue is a probe dump.** Of 546 definitions, 497 are auto-generated
  `live_<file>_<model>_<type>` rows with boilerplate `doc` text and decoded-name junk as the
  description ("-Birthday Present Voucher"). `restrictions` is empty in all 546. Many entries are
  irrelevant to a pre-Searing server (vouchers, minipets, costumes). An id that encodes the wire
  identity is not a stable name. The file is also being appended to by another agent while this
  review is written: `items-verification.md` says 297 definitions, the file now has 546.
- **D5 – A dead façade.** `Item::GetModelID`, `Inventory::GetBags` and the rest copy Py4GW's
  naming and have no callers. Delete them.
- **D6 – Types follow Postgres, not the wire.** The domain passes `i32` and `i64` around because
  the columns are signed. Ids, bags and slots should be newtypes (`ItemId(u32)`, `BagId(u16)`,
  `Slot(u8)`), converted once at the database boundary.
- **D7 – Content that a pre-Searing server doesn't need sits in the item path.** That's the
  180-row, four-campaign starting-equipment table (with no definitions) and the 387-row PvP
  unlock table. Keep them, but not inside the inventory code.

### 2.3 Documentation drift

- Doc 08 says a drop sends nothing and that `ITEM_REMOVE` crashed the client. The code (and doc
  48) send `ITEM_REMOVE` on a whole-stack drop.
- Doc 47 §8 says `0x804F` has never been seen from a client. `item.rs` implements it from
  `items-live-20261108-174014.jsonl`.
- The rule should be one current document (this one), with the older pages marked as history.

## 3. The framework

Four layers. Each one depends only on the layer below it.

```text
  handlers (dispatch arms)      decode -> Inventory::apply -> repo.commit -> wire::frames -> send/broadcast
        |
  wire mapping                  Change -> Vec<Frame>, split into self / instance-visible
        |
  domain (pure, sync)           Inventory aggregate + rules; Op in, Vec<Change> or Refusal out
        |
  content + persistence         Catalogue (items.json, kinds) | Repo (one transaction per Op)
```

### 3.1 Content: definitions and kinds

Most rules follow from the item type, so they belong to the type, not to each definition.

```rust
/// From `item_type`, in one table. Replaces `game::equipped_slot` as the authority.
pub enum ItemKind {
    Weapon { hands: Hands },          // 2 axe, 5 bow, 15 hammer, 22 wand, 26 staff, 27 sword, 32 daggers, 35 scythe, 36 spear
    OffHand,                          // 12 focus, 24 shield
    Armor(ArmorSlot),                 // 7 body, 19 legs, 16 head, 4 boots, 13 gloves
    Costume(CostumeSlot),             // 44, 45
    Bag, Material, Trophy, Usable, Quest, Key, Kit, Rune, Dye, Other(u8),
}
pub enum Hands { One, Two }           // bow, hammer, staff, scythe, daggers: Two

pub struct ItemDef {
    pub id: DefId,                    // stable, human: "birthday_cupcake"
    pub wire: WireIdentity,           // file_id, model_id, item_type
    pub name: NameUnits,
    pub value: u32, pub materials: u16, pub unk1: u8,
    pub default_flags: ItemFlags,
    pub profession: Profession,
    pub inherent: Vec<ModifierWord>,
    pub stack: StackRule,             // Single | UpTo(250); defaulted from kind, override per def
    pub kind: ItemKind,               // derived from wire.item_type at load
    pub verified: Verification,       // Unverified | Live { evidence: path }
}
```

- `items.json` holds only **curated** definitions with human ids. Fields that equal the kind's
  default are left out.
- Probe output goes to a separate `items.observed.json`. The server never serves from it. A tool
  promotes an observed entry into `items.json` once it has a real name and id. The other agent
  should write there, not to `items.json`.
- Starting equipment becomes `starting_equipment.json`: profession → definition ids. A
  pre-Searing server needs Prophecies only.
- At load, the catalogue refuses any definition whose wire identity is not `verified: Live`
  if the server would ever send it. That turns the doc-48 texture crash into a startup error
  instead of a client crash.

### 3.2 Domain: the `Inventory` aggregate

A plain struct in memory, loaded once per session, and the only place rules live.

```rust
pub struct Inventory { bags: BTreeMap<BagId, Bag>, items: HashMap<ItemId, ItemInstance>, slots: HashMap<Place, ItemId> }
pub enum Place { Slot(BagId, Slot), BagSelf(BagId) }

pub enum Op {
    Move    { item: ItemId, to: (BagId, Slot) },     // swaps or merges when the target is occupied
    Equip   { item: ItemId },
    Unequip { slot: EquipSlot, to: (BagId, Slot) },
    Split   { item: ItemId, count: u16, to: (BagId, Slot) },
    Drop    { item: ItemId, count: u16 },
    PickUp  { ground: GroundItem },
    Destroy { item: ItemId },
    Grant   { def: DefId, qty: u16 },                 // loot, quest reward, merchant, /item
}

pub enum Change {
    Created  { item: ItemInstance, at: Place },
    Moved    { item: ItemId, from: Place, to: Place },
    Quantity { item: ItemId, qty: u16 },
    Removed  { item: ItemId },
    Worn     { slot: EquipSlot, item: Option<ItemId> },   // drives the 0x006F visual
    ToGround { ground: GroundItem },
}

impl Inventory {
    pub fn apply(&mut self, op: Op, cat: &Catalogue, who: &Wearer) -> Result<Vec<Change>, Refusal>;
}
```

The rules, each in one place:

- **Bag rules.** Which kinds a bag accepts: equipped bag → equippable kinds in their own slot;
  material storage → materials only; unclaimed → server-only; storage → anything, but only when
  the server allows storage at all. (Pre-Searing has no Xunlai agent, so whether storage bags
  should even be reachable is a design decision. See §6.)
- **Equip rules.** Slot from kind. Armour must match the wearer's primary profession. A
  two-handed weapon unequips the off-hand, and an off-hand unequips a two-handed weapon. The
  source must be a carrying bag. A swap goes into the slot the incoming item frees.
- **Stack rules.** A move onto the same definition merges up to `StackRule`, with any remainder
  staying behind. Split checks `1..qty`. No quantity can ever leave `1..=max`.
- **Atomicity.** `apply` either returns every change or a `Refusal` and leaves `self` untouched.
  That fixes B2 by construction.

Because `apply` is pure, every rule gets a plain `#[test]` with no database: swap with a full
backpack, bow + shield, the cupcake into the helmet slot, merging to 250 with a remainder.

### 3.3 Persistence: one repository, one transaction per Op

```rust
impl InventoryRepo {
    pub async fn load(&self, character: CharacterId) -> Result<Inventory, LoadError>;  // validates, no unwrap_or(0)
    pub async fn commit(&self, character: CharacterId, changes: &[Change]) -> sqlx::Result<()>;
}
```

- One `INSERT` for items, used by seed, creation, pickup-restore and grant (fixes D2).
- `commit` takes `SELECT ... FOR UPDATE` on the character row, then applies the changes in
  order. If the commit fails, the session reloads and resyncs the client. The cache is never
  trusted after a failed write.
- `load` refuses out-of-range values and missing definitions. Backfill `definition_id` for
  existing rows by wire identity. Once every row resolves, make the column `NOT NULL`.
- Keep the wire snapshot columns on `t_item` (doc 47's argument still holds for renamed and
  upgraded items). The definition is required, and the snapshot is the instance's override.

### 3.4 Wire mapping: one table, citing captures

```rust
pub fn frames(changes: &[Change], ctx: &WireCtx) -> Outgoing { own: Vec<Frame>, instance: Vec<Frame> }
```

| change | own client | rest of the instance | evidence |
|---|---|---|---|
| `Moved` | `ITEM_CHANGE_LOCATION` (`0x014B`) | — | `items-live-20261108-174014` |
| `Worn` | — (the move covers it) | `0x006F` visual slot | seq 22-24, 31-32 |
| `Quantity` | `ITEM_UPDATE_QUANTITY` | — | seq 132-136 |
| `Removed` | `ITEM_REMOVE` | — | `173913` seq 193 |
| `ToGround` | item record (`0x0162` for a part) + agent + drop line | **item agent** | `173913` seq 41-45 |
| pickup | hold, line, `ITEM_PICKED_UP`, placement, `WORLD_REMOVE_AGENT` | **`WORLD_REMOVE_AGENT`** | `173913` seq 55-63 |

The "rest of the instance" column fixes B1. A change with no captured answer has no row in this
table, so it cannot be sent. That carries over the project's existing rule.

### 3.5 What a handler becomes

```rust
pub async fn handle_equip_item(ctx: &mut Ctx, frame: &Frame) -> Result<bool, GameError> {
    let item = decode::equip(frame)?;
    ctx.run(Op::Equip { item }).await          // apply -> commit -> frames -> send + broadcast; Refusal -> resync
}
```

`Ctx::run` is written once. It owns the refusal path (send `ITEM_CHANGE_LOCATION` for where the
item really is) and the logging.

## 4. Plan, in order

Each phase leaves the server working, and the existing byte test
(`the_item_block_from_rows_is_the_capture_byte_for_byte`) passes at every step.

| phase | work | done when |
|---|---|---|
| **P0 Hygiene** (no behaviour change) | Delete the `Item`/`Inventory` façade. Move `reload` out of `creation.rs`. One INSERT path. Newtypes at the database boundary. Replace `unwrap_or(0)` with load-time validation. Mark docs 08/47 §7-8 as superseded. | Byte test and DB tests green. `grep unwrap_or(0)` empty in the item path. |
| **P1 Domain core** | `Inventory::apply` with move / equip / unequip / drop / pickup / destroy / grant and the rules in §3.2. Rewire the six handlers through `Ctx::run`. Golden tests replay `items-live-20261108-173913/174014.jsonl` requests and compare the answers. | B2–B5, B8 have tests that failed before and pass now. Handlers are under 15 lines each. |
| **P2 Broadcast** | `Outgoing.instance` goes through `instance.broadcast`: ground agents, pickups, visual equipment. | Two-client test: B sees A's drop appear and disappear. |
| **P3 Catalogue** | `ItemKind` table, curated `items.json`, `items.observed.json`, a promotion tool, Prophecies `starting_equipment.json` with definitions, backfill and `NOT NULL`. | Every `t_item` row resolves to a definition. Creation writes no NULL. |
| **P4 Features** | Split (`0x8075`), weapon-set selection, merchant (`0x804D`, Rurik's `merchant.py`), identify/salvage. Each starts with a capture. | One capture and one `Op` per feature. |

## 5. What this supersedes

- `docs/47-item-system.md` §7 (roadmap) and §8 (open decisions): replaced by §4 and §6 here.
  `0x804F` is measured.
- `docs/08-next-inventory-model.md` "traps": the drop/`ITEM_REMOVE` paragraphs describe a
  build that no longer exists.

## 6. Open questions

| question | why it matters |
|---|---|
| Storage in pre-Searing | Pre-Searing has no Xunlai agent. Should the storage bags be sent but locked, or not sent at all? A capture of a live pre-Searing load decides. |
| Is `UPDATE_GOLD_STORAGE` storage gold or character gold? | B7 |
| How a swap is confirmed | The client moves both items itself. One capture of equip-onto-occupied settles whether the server echoes one move or two. |
| Split request layout (`0x8075`) | Not decoded. Needed for P4. |
| Weapon-set selection | Four empty sets are measured, but a selection is not. |
| Armour → energy (properties 41/43) | The live server re-sends them on armour change. That needs an armour model, so it stays out of P1. |
