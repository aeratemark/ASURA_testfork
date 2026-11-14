# The item system: what exists, where the boundary is, and what comes next

This document is the architecture assessment behind the first item-catalogue stage. It separates
what a capture measured, what client source establishes, what another emulator reconstructed, and
what is still unknown. The code and the tests are the authority; this page says why they have the
shape they do.

## 1. What the current implementation is

The current item path is small and works:

| piece | where | what it owns |
|---|---|---|
| item and bag rows | `crates/asura-db/migrations/0006_inventory.sql` | item ids from `item_id_seq`, bag id versus `bag_model_id`, placement, the wire fields of `CREATE_NAMED_ITEM` |
| row reads and writes | `crates/asura-db/src/inventory.rs` | `load`, `seed_default`, `create_character`, `find_in_carrying_bags`, `take_from_stack`, `move_item`, `first_free_carrying_slot` |
| the reference seed | `crates/asura-db/src/inventory/seed.rs` | six bags and nine items, copied from the reference's load stream |
| starting equipment | `crates/asura-world/src/default_equipment.rs` | the generated `GmDefaultArmors.c` table, per campaign and profession |
| PvP unlock table | `crates/asura-world/src/pvp_items.rs` | the reference's 387 rows, sent at PvP creation |
| wire builders | `crates/asura-protocol/src/game/mod.rs` | `CREATE_NAMED_ITEM`, `INVENTORY_CREATE_BAG`, `ITEM_MOVE_TO_LOCATION`, `ITEM_SET_PROFESSION`, `ITEM_WEAPON_SET` |
| world behaviour | `crates/asura-world/src/item.rs` | the load blocks, drop, move, equip, the visual-equipment message |
| dispatch | `crates/asura-world/src/dispatch.rs` | `0x8091`, `0x802C`, `0x8072`, `0x8030`, `0x804F` |

`db_inventory::Item` is already an owned instance: it has a client-visible id, a bag and slot, a
quantity, dye, flags and the actual modifier list. The same struct also carries every field the
wire needs, because a row that cannot rebuild its message is not a model of anything. That second
part is deliberate and stays.

`Bag` already keeps the two numbers apart: `bag_id` is the number the client is told, and
`bag_model_id` is the kind (`0` backpack, `5` material storage, `6` unclaimed, `7+` storage, `21`
equipped). The wire order is by model id, not id, and the byte test pins it.

What is missing is the other noun: there is no catalogue of definitions. The nine seed items and
the 180 starting-equipment rows are flattened instance records, and the same static facts
(file id, model id, type, default name, inherent modifiers) are repeated in each row.

## 2. Definition versus instance

The separation this fork needs is:

**Definition** - content, read from the world pack (`config/World/<pack>/items.json`):

- stable definition id;
- model id and file id;
- item type;
- default name (wire UTF-16 units) and a readable description;
- base value, materials, unknown byte and profession;
- inherent modifier words;
- stackability and maximum stack;
- allowed equipment positions;
- named restrictions.

**Instance** - operational state, a `t_item` row:

- client-visible item id;
- bag id and slot;
- definition reference (`definition_id`, nullable);
- quantity;
- dye tint and colours;
- flags (identified, customized, equipped, rarity and the rest);
- the actual modifier words on this item;
- the wire snapshot fields the client was sent (file id, type, model, name, value, materials, unk1,
  profession).

The instance keeps the wire snapshot even when its definition is present. Two reasons:

1. an item can be upgraded, dyed or renamed, so its actual modifier list and name are not the
   definition's defaults;
2. a database written before this stage has no `definition_id`, and it must still load.

The definition is never copied into `t_item`, and `t_item` is never used as the catalogue. The one
new column is a reference. A definition edit does not rewrite instances; a future migration can
choose which instances to update, deliberately.

## 3. The catalogue and the modifier foundation

`crates/asura-world/src/item_catalogue.rs` loads `items.json`, validates it, indexes it by id and
by `(file_id, item_type, model_id)`, and resolves a row:

- an explicit `definition_id` wins;
- a legacy row falls back to its wire identity;
- no match is not an error. The row still rebuilds its message.

The committed pack defines the nine seed items: the backpack, the Birthday Cupcake (one definition
for the two stacks) and the six Prophecies warrior starting pieces. The starting equipment table
has not been moved into the pack yet, so those created rows carry `definition_id = NULL`. That is
the next catalogue step, not a silent reinterpretation of the generated reference table.

`crates/asura-world/src/item_modifier.rs` decodes the modifier words. Every result carries the raw
`u32`; unknown and skipped words are `ModifierEffect::Unknown` and survive persistence and
serialization unchanged. Known identifiers and their evidence:

| id | meaning | evidence |
|---|---|---|
| 1, 633 | attribute requirement | client code reads 633 and falls back to 1; 570 live words validated |
| 542, 543 | attribute bonus, non-stacking / stacking | client handlers; a live client rendered a composed 543 |
| 555 | enchantment duration | Preservation's builder only; marked reconstructed |
| 556, 558 | energy, energy recovery | Rurik's tooltip table |
| 570 | half skill recharge | Rurik's tooltip table and Preservation |
| 572, 527 | armor, armor versus a damage type | Rurik's tooltip table |
| 580 | holds items | the backpack's live word and Rurik |
| 584 | damage range (argument max, second min) | live tooltip `Blunt Dmg: 3-5` |
| 587 | damage type (0 is Blunt) | live tooltip `Blunt` |

The 16-bit `raw >> 16` view is exposed, and so are Rurik's 10-bit identifier and argument. The
decoder matches on the identifier, because bits 31-30 and 19 are a per-identifier prefix and must
not split one id into several. This is why `0xA3C85000` (`raw >> 16 = 0xA3C8`) is armor 572, not a
different effect.

Tooltips are presentation. They are not the character sheet. The current load sends
`AGENT_UPDATE_ATTRIBUTES` as 42 zeroes (`crates/asura-world/src/player.rs:317`), and the client's
attribute panel reads that column, while an item tooltip reads the item's words. A later stage that
adds equipment bonuses must sum `ModifierEffect::AttributeBonus` into the server's attribute
column; it must not parse a tooltip string.

## 4. What can be reused without modification

- `0006_inventory.sql`'s tables, item-id sequence, bag model distinction and placement rules.
- `inventory.rs`'s load order, seed guard, carrying-bag rule, stack deduction, move validation and
  first-free-slot search.
- `item.rs`'s load block order, `named_item` wire conversion, refusal echo, equip displacement and
  `UPDATE_AGENT_VISUAL_EQUIPMENT`.
- The protocol builders and `game::equipped_slot`.
- `default_equipment.rs` and `pvp_items.rs` as content until a later stage moves them into a pack.
- The existing database and character-creation tests.

## 5. What changed in this stage

| change | why |
|---|---|
| `config/World/Vanilla/items.json` | the first definitions, in content rather than SQL or Rust |
| `item_catalogue.rs` | load, validate, index and resolve definitions |
| `item_modifier.rs` | decode known words, preserve every raw word, keep tooltips separate |
| `t_item.definition_id` (migration 0033) | the reference from an instance to its definition; nullable |
| `Item.definition_id`, `NewItem.definition_id`, `SeedItem.definition_id` | read and write that link |
| the seed's nine rows name their definitions | the tested default inventory is the first integration |
| start-up check | a malformed pack stops the world before a load block is built |
| a reference byte test | the 31 item-stream frames from `recordings/gwps/world/load-stream.log` |

No packet changed. `CREATE_NAMED_ITEM` still reads the row's wire fields, so a definition can never
alter a byte that a capture pinned.

## 6. Database design

`t_item` is operational state and stays the only item table. `definition_id text` is a key into
the pack, not a foreign key: content and state have different lifetimes. A missing definition is a
fact ("this pack does not describe the row"), not corruption, so the column is nullable and no
constraint refuses a row without one.

What deliberately did **not** happen:

- no `t_item_definition` table;
- no model/name/type/modifier columns added to `t_item`;
- no rewrite of old rows;
- no change to `assoc_item_id`, bag ids or the partial unique slot index.

Later movements, splits, merges and equipment changes continue to use one transaction each. A
merge must lock both stacks, refuse when the definition has no measured limit, and never leave a
quantity outside `1..=max_stack`.

## 7. Roadmap, in order

1. **Catalogue the starting equipment (next).** The Rurik and cmonmeow pre-Searing candidates are
   now curated into `items.json` (58 definitions; see `docs/48-item-research.md`). The generated
   180 `ItemSpec` rows still need stable definition ids and creation still reads that table.
   Move it into the pack and delete the duplicate, keeping the byte test that proves the created
   set still equals the reference's fields.
2. **Bag definitions.** Give each bag model a definition (capacity and default bag item) instead
   of hardcoding the six seed rows. The backpack's `HoldsItems(20)` is the first measured case.
3. **Inventory movement and swapping.** The current move refuses an occupied slot. Rurik's capture
   shows an equip onto an occupied slot exchanging the two (`0x0152` in the client's model);
   measure the server answer before changing the refusal. Split (`0x8075`) and merge need their
   captured requests first.
4. **Equipment and weapon sets.** `0x8030` and `0x804F` now move the piece and answer with
   `ITEM_CHANGE_LOCATION` plus the single visual slot (`0x006F`), measured in
   `items-live-20261108-174014.jsonl`. The live server also re-sends energy and energy regeneration
   (properties 41 and 43) after armour changes; there is no armour-to-energy model here yet, so the
   pools stay where the load block set them. Keep the equipped bag slot separate from the active
   weapon set: the four empty weapon sets are still the reference's own.
5. **Ground items and pickup.** Measured and implemented. A whole-stack drop answers `ITEM_REMOVE` and
   an item agent; a partial drop answers `ITEM_UPDATE_QUANTITY`, `ITEM_UPDATE` (`0x0162`) and an agent;
   `0x803F` merges a stack part or restores the row. The record lives in the instance, so it survives
   a relog but not a world restart (`crates/asura-world/src/item.rs`, `crates/asura-world/src/instance.rs`).
6. **Identification and customization.** The flags already travel. Their transitions need a
   captured identify/customize exchange, not a guess about which flag bit means what.
7. **Upgrades and salvaging.** Modifier words already survive. Applying a rune, splitting an item
   and destroying an upgrade need their own item operations and captures.
8. **Merchants and trading.** `0x804D` is in the table and unhandled. Rurik's merchant work is the
   first source; the stock, price and gold messages need a capture against this client.

## 8. Decisions that still need protocol research

| decision | why it cannot be guessed |
|---|---|
| what `0x804F` means | it carries no item id and no live client has sent one |
| how a swap is confirmed | the client moves both items in its own model; the server's echo is only measured for one item |
| how a stack split is requested | `0x8075` is named, the request is not decoded here |
| how weapon sets are selected | four empty sets are measured on load; a selection and its two item ids are not |
| what identifies an item | the flags exist, the request and reply do not |

## 9. What the client archive can and cannot supply

`Gw.dat` is not a second catalogue. Measured with the repository's own census
(`cargo run -p asura-world --example gwdat-census -- <Gw.dat>`) against the archive in this
session: **104,944 entries**, and an item's `CREATE_NAMED_ITEM.file_id` is a composite model id,
not a raw archive key. The three ids checked are `0x8001B536` (backpack), `0x80009B60` (starter
hammer) and `0x00047C34` (Birthday Cupcake). Each resolves to an entry: the two high-bit ids
after clearing bit 31, and `0x47C34` as written. Each entry begins with the same `ffna` container
header the map importer recognises. The archive
therefore supplies the model content, and the client resolves names and icons through its own
model and string tables. It does not carry the server's item type, stack limit, restrictions or
modifier list; those are the fields the wire defines and this catalogue states.

The useful future measurement is name enrichment: map a known `model_id` to the client's decoded
display name through the Py4GW item-name path, and write the result into the definition file once
it is observed. That does not require a definition table in the archive, and it does not move the
catalogue into the database.

## 10. Source note

The wire facts come from `recordings/gwps/world/load-stream.log` and the live recordings under
`recordings/client/`. Item types and bag positions come from the client's own enums as copied in
`Other_Repos_Research/Headquarter/include/client/object/item.h` and `bag.h`. The modifier layout and
the named identifiers come from `Other_Repos_Research/Rurik-main/studies/itemmods/FINDINGS.md`;
Preservation's `item/modifier.go` corroborates several builders and is marked where it is the only
witness. Guild Wars Wiki is the naming and item-behaviour source: `Category:Items` indexes the
items, the per-item pages give the name and type, and `Item stacking` states the 250 limit; the
`Sweet` and `Birthday Cupcake` pages identify the cupcake. The wiki does not carry the wire file
id, model id or modifier word, so those stay with the capture and the client archive. No fact from
another emulator is presented as retail behaviour.
