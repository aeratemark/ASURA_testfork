//! The inventory: a character's bags and the items in them.
//!
//! Until this module existed the world server answered the client's item request with the reference's
//! own messages, replayed byte for byte from a capture of the reference (retired when parity closed, docs/16). That made a client
//! happy and made the server know nothing: there was no row, bag or slot to name, so nothing could be
//! dropped, equipped, moved or saved. These are the rows that replaced them, and the two operations the
//! world server needs them for:
//!
//! * **reading** a character's inventory into the shape the wire wants ([`load`]);
//! * **creating** the default inventory for a character that has none ([`seed_default`]), which is what
//!   the reference does for every player it creates (`GameSrv_CreateDefaultBags`, guarded by
//!   `IsValidBag` so a loaded character keeps what it had);
//! * **finding** the item a client asks to drop ([`find_in_carrying_bags`]), restricted to the bags the
//!   reference searches (`GmBag_FindItem` looks only at the backpack, the belt pouch and bags 1 and 2).
//!
//! The model, and where it differs from the reference's own `items`/`bags` tables, is argued in
//! `crates/asura-db/migrations/0006_inventory.sql`. In one line: a bag is named by the id the client is
//! told *and* by its kind, an item's name and modifiers are stored because `CREATE_NAMED_ITEM` carries
//! them, and a bag's own item is a row with no slot rather than a struct compiled into the server.

use crate::{PgConnection, PgPool, Uuid};

/// The default inventory the reference seeds, generated from a capture of it working.
pub mod seed;

/// One bag, as the wire tells a client about it (`INVENTORY_CREATE_BAG`, `0x013F`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bag {
    /// The number the client is told, and the one `ITEM_MOVED_TO_LOCATION` names.
    pub bag_id: i32,
    /// `BagModelId`: 0 backpack, 5 material storage, 6 unclaimed items, 7+ storage, 21 equipped items.
    /// The reference sends its bags in this order, so this is also the wire order.
    pub bag_model_id: i32,
    /// `BagType`: 1 bag, 2 equipped, 3 not collected, 4 storage, 5 material storage.
    pub bag_type: i32,
    pub slot_count: i32,
    /// The item that *is* the bag (the reference's `bag_item_id`), 0 for the bags that have none.
    pub assoc_item_id: i32,
}

/// One item, with every field `CREATE_NAMED_ITEM` (`0x0161`) carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The client-visible id: what a drop names, and what `UPDATE_AGENT_VISUAL_EQUIPMENT` carries.
    pub id: i32,
    /// Where it sits. `None` for an item that is not in a slot - a bag's own item.
    pub bag_id: Option<i32>,
    pub slot: Option<i32>,
    pub file_id: i64,
    pub item_type: i32,
    pub model_id: i32,
    pub dye_tint: i32,
    pub dye_colors: i32,
    pub materials: i32,
    pub unk1: i32,
    pub flags: i64,
    pub value: i32,
    pub quantity: i32,
    pub profession: i32,
    /// The name, as the UTF-16 code units the wire carries. Not text: the reference's bag name is a
    /// Gw.dat template id packed into units, and some of them are not valid Unicode.
    pub name: Vec<i32>,
    /// The modifier list, as the `u32` words the wire carries.
    pub modifiers: Vec<i64>,
}

impl Item {
    /// The name as the wire writes it: `u16` units.
    pub fn name_units(&self) -> Vec<u16> {
        self.name
            .iter()
            .map(|unit| u16::try_from(*unit).unwrap_or(0))
            .collect()
    }

    /// The modifiers as the wire writes them: `u32` dwords.
    pub fn modifier_dwords(&self) -> Vec<u32> {
        self.modifiers
            .iter()
            .map(|word| u32::try_from(*word).unwrap_or(0))
            .collect()
    }
}

/// A character's whole inventory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    /// In the reference's order: by bag model id, which is the order its bag array is walked in.
    pub bags: Vec<Bag>,
    pub items: Vec<Item>,
}

impl Inventory {
    /// Empty is the answer for a character that has never been loaded, and it is what
    /// [`seed_default`] fills.
    pub fn is_empty(&self) -> bool {
        self.bags.is_empty() && self.items.is_empty()
    }

    pub fn bag(&self, bag_id: i32) -> Option<&Bag> {
        self.bags.iter().find(|bag| bag.bag_id == bag_id)
    }

    pub fn item(&self, item_id: i32) -> Option<&Item> {
        self.items.iter().find(|item| item.id == item_id)
    }

    /// The items in one bag, by slot - the order the reference sends them in.
    pub fn items_in(&self, bag_id: i32) -> Vec<&Item> {
        let mut found: Vec<&Item> = self
            .items
            .iter()
            .filter(|item| item.bag_id == Some(bag_id))
            .collect();
        found.sort_by_key(|item| item.slot.unwrap_or(0));
        found
    }

    /// The nine item ids `UPDATE_AGENT_VISUAL_EQUIPMENT` carries: what the agent is wearing, by slot.
    ///
    /// Read out of the equipped-items bag rather than from a constant, because the ids belong to this
    /// character's items: a second character's armour has different ones.
    pub fn equipped_item_ids(&self) -> [u32; 9] {
        let mut ids = [0u32; 9];
        let Some(bag) = self
            .bags
            .iter()
            .find(|bag| bag.bag_model_id == EQUIPPED_ITEMS_MODEL)
        else {
            return ids;
        };
        for item in self.items_in(bag.bag_id) {
            let Some(slot) = item.slot.and_then(|slot| usize::try_from(slot).ok()) else {
                continue;
            };
            if slot < ids.len() {
                ids[slot] = u32::try_from(item.id).unwrap_or(0);
            }
        }
        ids
    }

    /// The item in one slot of the equipped-items bag, if anything is worn there.
    ///
    /// The slots are the reference's `EquippedItemSlot` (`GmInventory.h`): 2 body, 3 legs, 4 head, 5 boots,
    /// 6 gloves. The character settings blob is built from the five armour slots, which is why this asks by
    /// slot rather than by the ids `UPDATE_AGENT_VISUAL_EQUIPMENT` carries.
    pub fn item_in_equipped_slot(&self, slot: u8) -> Option<&Item> {
        let bag = self
            .bags
            .iter()
            .find(|bag| bag.bag_model_id == EQUIPPED_ITEMS_MODEL)?;
        self.items_in(bag.bag_id)
            .into_iter()
            .find(|item| item.slot == Some(i32::from(slot)))
    }
}

/// `BagModelId_EquippedItems` (`GmInventory.h`): the bag whose slots are the nine worn items.
pub const EQUIPPED_ITEMS_MODEL: i32 = 21;

/// `BagModelId_Backpack`.
pub const BACKPACK_MODEL: i32 = 0;

/// `BagModelId_Bag2`: the last bag `GmBag_FindItem` searches. A drop looks no further than this, and
/// neither does this module: the reference cannot drop from storage, so neither can we.
pub const LAST_CARRYING_MODEL: i32 = 3;

/// One bag as the seed describes it: the capture's bag id, and the capture id of its own item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedBag {
    pub bag_id: i32,
    pub bag_model_id: i32,
    pub bag_type: i32,
    pub slot_count: i32,
    /// The capture's item id of the item that is this bag, 0 when the bag has none.
    pub assoc_item: i32,
}

/// One item as the seed describes it. The slices are the wire's own units and dwords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedItem {
    /// The capture's item id. Renumbered from the sequence when it is written.
    pub id: i32,
    pub bag_id: Option<i32>,
    pub slot: Option<i32>,
    pub file_id: i64,
    pub item_type: i32,
    pub model_id: i32,
    pub dye_tint: i32,
    pub dye_colors: i32,
    pub materials: i32,
    pub unk1: i32,
    pub flags: i64,
    pub value: i32,
    pub quantity: i32,
    pub profession: i32,
    pub name: &'static [i32],
    pub modifiers: &'static [i64],
}

impl SeedItem {
    /// The seed row as a row: the shape both the reader and the wire builder use.
    pub fn item(&self) -> Item {
        Item {
            id: self.id,
            bag_id: self.bag_id,
            slot: self.slot,
            file_id: self.file_id,
            item_type: self.item_type,
            model_id: self.model_id,
            dye_tint: self.dye_tint,
            dye_colors: self.dye_colors,
            materials: self.materials,
            unk1: self.unk1,
            flags: self.flags,
            value: self.value,
            quantity: self.quantity,
            profession: self.profession,
            name: self.name.to_vec(),
            modifiers: self.modifiers.to_vec(),
        }
    }
}

impl From<&SeedBag> for Bag {
    fn from(seed: &SeedBag) -> Bag {
        Bag {
            bag_id: seed.bag_id,
            bag_model_id: seed.bag_model_id,
            bag_type: seed.bag_type,
            slot_count: seed.slot_count,
            assoc_item_id: seed.assoc_item,
        }
    }
}

/// The default inventory, as rows, with the ids the original capture used.
///
/// This is the seed: [`seed_default`] writes these rows (with ids taken from the sequence, because
/// two characters cannot both own item 1). The rows are the source of truth now; the world server
/// builds the item block out of them.
pub fn default_inventory() -> Inventory {
    Inventory {
        bags: seed::BAGS.iter().map(Bag::from).collect(),
        items: seed::ITEMS.iter().map(SeedItem::item).collect(),
    }
}

/// The item columns, in the order [`ItemRow`] expects. One list, so no query can select a shape the
/// row type does not decode.
const ITEM_COLUMNS: &str = "id, bag_id, slot, file_id, item_type, model_id, dye_tint, dye_colors, \
                            materials, unk1, flags, value, quantity, profession, name, modifiers";

/// The same columns, qualified for a query that has more than one table in it.
fn item_columns(alias: &str) -> String {
    ITEM_COLUMNS
        .split(',')
        .map(|column| format!("{alias}.{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One item row as both queries select it, in the order they select it (`load` and
/// [`find_in_carrying_bags`] share the column list so the two cannot drift apart).
type ItemRow = (
    i32,
    Option<i32>,
    Option<i32>,
    i64,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    i64,
    i32,
    i32,
    i32,
    Vec<i32>,
    Vec<i64>,
);

/// A row as [`Item`].
fn item_from_row(row: ItemRow) -> Item {
    let (
        id,
        bag_id,
        slot,
        file_id,
        item_type,
        model_id,
        dye_tint,
        dye_colors,
        materials,
        unk1,
        flags,
        value,
        quantity,
        profession,
        name,
        modifiers,
    ) = row;
    Item {
        id,
        bag_id,
        slot,
        file_id,
        item_type,
        model_id,
        dye_tint,
        dye_colors,
        materials,
        unk1,
        flags,
        value,
        quantity,
        profession,
        name,
        modifiers,
    }
}

/// Read a character's bags and items.
///
/// Bags come back in the reference's send order (`bag_model_id`), items in `(bag, slot)` order, so the
/// world server can build the block without re-sorting and without knowing why the order matters.
pub async fn load(pool: &PgPool, character: Uuid) -> sqlx::Result<Inventory> {
    let bags: Vec<(i32, i32, i32, i32, i32)> = sqlx::query_as(
        "SELECT bag_id, bag_model_id, bag_type, slot_count, assoc_item_id
           FROM bag WHERE character_id = $1
          ORDER BY bag_model_id, bag_id",
    )
    .bind(character)
    .fetch_all(pool)
    .await?;

    let items: Vec<ItemRow> = sqlx::query_as(&format!(
        "SELECT {ITEM_COLUMNS} FROM item WHERE character_id = $1
          ORDER BY bag_id NULLS FIRST, slot, id"
    ))
    .bind(character)
    .fetch_all(pool)
    .await?;

    Ok(Inventory {
        bags: bags
            .into_iter()
            .map(
                |(bag_id, bag_model_id, bag_type, slot_count, assoc_item_id)| Bag {
                    bag_id,
                    bag_model_id,
                    bag_type,
                    slot_count,
                    assoc_item_id,
                },
            )
            .collect(),
        items: items.into_iter().map(item_from_row).collect(),
    })
}

/// One item to write, with its own storage.
///
/// `SeedItem` borrows its name and modifiers from the generated seed, which is fine for a constant table
/// and wrong for anything computed at runtime: a creation screen's equipment is derived from the
/// profession the client picked, so it owns its slices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    pub bag_id: Option<i32>,
    pub slot: Option<i32>,
    pub file_id: i64,
    pub item_type: i32,
    pub model_id: i32,
    pub dye_tint: i32,
    pub dye_colors: i32,
    pub materials: i32,
    pub unk1: i32,
    pub flags: i64,
    pub value: i32,
    pub quantity: i32,
    pub profession: i32,
    pub name: Vec<i32>,
    pub modifiers: Vec<i64>,
}

/// Create a character and give it the reference's default inventory, with `equipped` in place of the
/// capture's equipment.
///
/// This is `GameSrv_HandleCharCreationConfirm`'s writing half: the reference creates the row and its bags
/// with `GameSrv_CreatePlayerBags`, and the bags already hold what the creation screen last chose
/// (`HandleCharCreationChangeProf` swapped them in memory). Here the choice arrives as rows.
///
/// `equipped` items carry their own `bag_id` and `slot`, which the caller takes from the reference's
/// `GetBagSlotForItemType` mapping.
pub async fn create_character(
    pool: &PgPool,
    account: Uuid,
    name: &str,
    map_id: i32,
    traits: crate::game::Traits,
    equipped: &[NewItem],
) -> sqlx::Result<Uuid> {
    let character = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    // The traits go in with the row: they are what the load block sends, and a character created from a
    // creation screen must load as what the screen chose. The last five columns are the reference's own
    // creation defaults (`GameSrv.c:1880-1886`): level 1, `is_pvp` when the campaign is none, the helmet
    // shown, the secondary profession the screen chose, and the unlocked-professions bit for the primary.
    sqlx::query(
        "INSERT INTO characters (id, account_id, name, map_id, appearance, primary_profession,
                                 unlocked_professions, gold, campaign, secondary_profession, level,
                                 is_pvp, helm_status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 1, $11, 1)",
    )
    .bind(character)
    .bind(account)
    .bind(name)
    .bind(map_id)
    .bind(i64::from(traits.appearance))
    .bind(i32::from(traits.profession))
    .bind(i64::from(traits.unlocked_professions))
    .bind(traits.gold as i32)
    .bind(i32::from(traits.campaign))
    .bind(i32::from(traits.secondary_profession))
    .bind(traits.campaign == 0) // `is_pvp`: `player->char_creation_campaign == Campaign_None`
    .execute(&mut *tx)
    .await?;

    // The equipped bag is the one the reference calls `equipped_items`; everything the seed puts there is
    // replaced, and every other bag keeps the reference's own contents.
    let equipped_bag = seed::BAGS
        .iter()
        .find(|bag| bag.bag_type == 2)
        .map(|bag| bag.bag_id)
        .unwrap_or(0);

    let mut allocated: Vec<(i32, i32)> = Vec::new();
    for seed_item in seed::ITEMS
        .iter()
        .filter(|item| item.bag_id != Some(equipped_bag))
    {
        allocated.push((
            seed_item.id,
            insert_item(tx.as_mut(), character, seed_item).await?,
        ));
    }
    for item in equipped {
        allocated.push((0, insert_new_item(tx.as_mut(), character, item).await?));
    }

    for seed_bag in seed::BAGS {
        let assoc_item_id = bag_own_item(seed_bag, &allocated)?;
        sqlx::query(
            "INSERT INTO bag (character_id, bag_id, bag_model_id, bag_type, slot_count, assoc_item_id)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(character)
        .bind(seed_bag.bag_id)
        .bind(seed_bag.bag_model_id)
        .bind(seed_bag.bag_type)
        .bind(seed_bag.slot_count)
        .bind(assoc_item_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(character)
}

/// One item row from the generated seed, returning the id the sequence gave it.
async fn insert_item(tx: &mut PgConnection, character: Uuid, seed: &SeedItem) -> sqlx::Result<i32> {
    insert_row(
        tx,
        character,
        seed.bag_id,
        seed.slot,
        seed.file_id,
        seed.item_type,
        seed.model_id,
        seed.dye_tint,
        seed.dye_colors,
        seed.materials,
        seed.unk1,
        seed.flags,
        seed.value,
        seed.quantity,
        seed.profession,
        seed.name,
        seed.modifiers,
    )
    .await
}

/// One item row from a runtime set, returning the id the sequence gave it.
async fn insert_new_item(
    tx: &mut PgConnection,
    character: Uuid,
    item: &NewItem,
) -> sqlx::Result<i32> {
    insert_row(
        tx,
        character,
        item.bag_id,
        item.slot,
        item.file_id,
        item.item_type,
        item.model_id,
        item.dye_tint,
        item.dye_colors,
        item.materials,
        item.unk1,
        item.flags,
        item.value,
        item.quantity,
        item.profession,
        &item.name,
        &item.modifiers,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn insert_row(
    tx: &mut PgConnection,
    character: Uuid,
    bag_id: Option<i32>,
    slot: Option<i32>,
    file_id: i64,
    item_type: i32,
    model_id: i32,
    dye_tint: i32,
    dye_colors: i32,
    materials: i32,
    unk1: i32,
    flags: i64,
    value: i32,
    quantity: i32,
    profession: i32,
    name: &[i32],
    modifiers: &[i64],
) -> sqlx::Result<i32> {
    sqlx::query_scalar(
        "INSERT INTO item (character_id, bag_id, slot, file_id, item_type, model_id, dye_tint,
                           dye_colors, materials, unk1, flags, value, quantity, profession,
                           name, modifiers)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
         RETURNING id",
    )
    .bind(character)
    .bind(bag_id)
    .bind(slot)
    .bind(file_id)
    .bind(item_type)
    .bind(model_id)
    .bind(dye_tint)
    .bind(dye_colors)
    .bind(materials)
    .bind(unk1)
    .bind(flags)
    .bind(value)
    .bind(quantity)
    .bind(profession)
    .bind(name.to_vec())
    .bind(modifiers.to_vec())
    .fetch_one(tx)
    .await
}

/// Give a character the reference's default inventory, unless it already has bags.
///
/// `Ok(false)` means there was nothing to do, and that is the common case: the reference calls its
/// `GameSrv_CreateDefaultBags` on every session and each of its `if (!IsValidBag(...))` guards turns it
/// into a no-op once the bags exist. Doing it here in one transaction means a half-seeded character is
/// not a state anyone can observe.
///
/// The item ids come from `item_id_seq` in the seed's own order, so the first character seeded in a
/// fresh database gets the capture's numbering (1..9) and a second one gets a fresh range.
pub async fn seed_default(pool: &PgPool, character: Uuid) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;

    // `FOR UPDATE` on the character row: two connections arriving for the same character at once must
    // not both decide it has no bags. The reference cannot hit this (one connection per player); a
    // server that reconnects fast can.
    let exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM characters WHERE id = $1 FOR UPDATE")
            .bind(character)
            .fetch_optional(&mut *tx)
            .await?;
    if exists.is_none() {
        return Ok(false);
    }

    let bags: i64 = sqlx::query_scalar("SELECT count(*) FROM bag WHERE character_id = $1")
        .bind(character)
        .fetch_one(&mut *tx)
        .await?;
    if bags > 0 {
        return Ok(false);
    }

    // The items first, in the seed's order, so that a bag can name its own item by the id the sequence
    // actually gave it rather than by the capture's. Neither `item.bag_id` nor `bag.assoc_item_id` is a
    // foreign key (the reference's two tables point at each other), so the order is free.
    let mut allocated: Vec<(i32, i32)> = Vec::with_capacity(seed::ITEMS.len());
    for seed in seed::ITEMS {
        let id: i32 = sqlx::query_scalar(
            "INSERT INTO item (character_id, bag_id, slot, file_id, item_type, model_id, dye_tint,
                               dye_colors, materials, unk1, flags, value, quantity, profession,
                               name, modifiers)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
             RETURNING id",
        )
        .bind(character)
        .bind(seed.bag_id)
        .bind(seed.slot)
        .bind(seed.file_id)
        .bind(seed.item_type)
        .bind(seed.model_id)
        .bind(seed.dye_tint)
        .bind(seed.dye_colors)
        .bind(seed.materials)
        .bind(seed.unk1)
        .bind(seed.flags)
        .bind(seed.value)
        .bind(seed.quantity)
        .bind(seed.profession)
        .bind(seed.name.to_vec())
        .bind(seed.modifiers.to_vec())
        .fetch_one(&mut *tx)
        .await?;
        allocated.push((seed.id, id));
    }

    for seed in seed::BAGS {
        let assoc_item_id = bag_own_item(seed, &allocated)?;

        sqlx::query(
            "INSERT INTO bag (character_id, bag_id, bag_model_id, bag_type, slot_count, assoc_item_id)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(character)
        .bind(seed.bag_id)
        .bind(seed.bag_model_id)
        .bind(seed.bag_type)
        .bind(seed.slot_count)
        .bind(assoc_item_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(true)
}

/// The id the seed's bag calls its own item, in the ids the sequence actually gave.
///
/// A seed that names an item it does not carry would write a bag pointing at nothing; that is refused
/// here rather than written, so the failure happens at the write and not in a client drawing an
/// inventory with a hole in it.
fn bag_own_item(seed: &SeedBag, allocated: &[(i32, i32)]) -> sqlx::Result<i32> {
    if seed.assoc_item == 0 {
        return Ok(0);
    }
    allocated
        .iter()
        .find(|(capture_id, _)| *capture_id == seed.assoc_item)
        .map(|(_, id)| *id)
        .ok_or_else(|| {
            sqlx::Error::Configuration(
                format!(
                    "the default inventory names item {} as bag {}'s own item, and no item in the seed \
                     has that id",
                    seed.assoc_item, seed.bag_id
                )
                .into(),
            )
        })
}

/// The item a drop names, if it is somewhere the reference would look for it.
///
/// `GmBag_FindItem` walks the bag array from the backpack to bag 2 and looks at nothing else, which is
/// why a drop from storage is not a thing the reference can do: the handler answers as if the item did
/// not exist. The join is that rule, in SQL, so the caller cannot forget it.
pub async fn find_in_carrying_bags(
    pool: &PgPool,
    character: Uuid,
    item_id: i32,
) -> sqlx::Result<Option<Item>> {
    let row: Option<ItemRow> = sqlx::query_as(&format!(
        "SELECT {} FROM item i
           JOIN bag b ON b.character_id = i.character_id AND b.bag_id = i.bag_id
          WHERE i.character_id = $1
            AND i.id = $2
            AND b.bag_model_id BETWEEN $3 AND $4",
        item_columns("i")
    ))
    .bind(character)
    .bind(item_id)
    .bind(BACKPACK_MODEL)
    .bind(LAST_CARRYING_MODEL)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(item_from_row))
}

/// What came off a stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    /// The item as it was before part of it was taken, so a caller can build the thing on the ground
    /// from the same fields the client knows it by.
    pub item: Item,
    /// What is left in the bag: 0 means the row is gone.
    pub remaining: i32,
}

/// Take `count` of an item out of its stack, and say what is left.
///
/// **This is the half the reference never wrote.** `GameSrv_HandleDropItem` bails out with
/// `TODO: we must still implement this` as soon as `count < quantity`, so a stacked item cannot be
/// dropped there at all - and the client sends `count = 1` for a stack of a hundred, which is why
/// dropping the reference's own cupcake never did anything.
///
/// `Ok(None)` means nothing was taken: no such item, not in a slot (a bag's own item is not litter), or
/// fewer than `count` in the stack.
pub async fn take_from_stack(
    pool: &PgPool,
    character: Uuid,
    item_id: i32,
    count: i32,
) -> sqlx::Result<Option<Taken>> {
    if count <= 0 {
        return Ok(None);
    }

    let mut tx = pool.begin().await?;
    let row: Option<ItemRow> = sqlx::query_as(&format!(
        "SELECT {ITEM_COLUMNS} FROM item WHERE character_id = $1 AND id = $2 FOR UPDATE"
    ))
    .bind(character)
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(item) = row.map(item_from_row) else {
        return Ok(None);
    };
    if item.bag_id.is_none() || item.quantity < count {
        return Ok(None);
    }

    let remaining = item.quantity - count;
    if remaining == 0 {
        sqlx::query("DELETE FROM item WHERE character_id = $1 AND id = $2")
            .bind(character)
            .bind(item_id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("UPDATE item SET quantity = $3 WHERE character_id = $1 AND id = $2")
            .bind(character)
            .bind(item_id)
            .bind(remaining)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(Some(Taken { item, remaining }))
}

/// Why an item was not moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// No such item for this character.
    NoSuchItem,
    /// It is not in a slot - a bag's own item, which is part of the bag rather than an item in it.
    NotInASlot,
    /// No such bag for this character.
    NoSuchBag,
    /// The slot is outside the bag: slots are `0..slot_count`.
    OutOfRange,
    /// Something else is in that slot. Equipping evicts; a move does not.
    Occupied,
}

/// What an attempt to put an item somewhere did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveOutcome {
    /// It is now at `(bag_id, slot)`.
    At { item: Item, bag_id: i32, slot: i32 },
    /// It was already there; nothing was written.
    Unchanged { item: Item },
    /// Nothing happened, and `item` is where it actually is - a caller uses that to put the client's
    /// own model back, because the client moves the item optimistically before it asks.
    Refused { item: Option<Item>, reason: Refused },
}

/// Move an item to a slot in one of its character's bags.
///
/// The rules are the ones a client can be held to, and no more: the bag must exist and be this
/// character's, the slot must be inside it, and the slot must be free. Equipping is a move into the
/// equipped-items bag, so it goes through here too.
pub async fn move_item(
    pool: &PgPool,
    character: Uuid,
    item_id: i32,
    bag_id: i32,
    slot: i32,
) -> sqlx::Result<MoveOutcome> {
    let mut tx = pool.begin().await?;
    let row: Option<ItemRow> = sqlx::query_as(&format!(
        "SELECT {ITEM_COLUMNS} FROM item WHERE character_id = $1 AND id = $2 FOR UPDATE"
    ))
    .bind(character)
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(item) = row.map(item_from_row) else {
        return Ok(MoveOutcome::Refused {
            item: None,
            reason: Refused::NoSuchItem,
        });
    };
    if item.slot.is_none() || item.bag_id.is_none() {
        return Ok(MoveOutcome::Refused {
            item: Some(item),
            reason: Refused::NotInASlot,
        });
    }
    if item.bag_id == Some(bag_id) && item.slot == Some(slot) {
        return Ok(MoveOutcome::Unchanged { item });
    }

    let bag: Option<(i32,)> =
        sqlx::query_as("SELECT slot_count FROM bag WHERE character_id = $1 AND bag_id = $2")
            .bind(character)
            .bind(bag_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((slot_count,)) = bag else {
        return Ok(MoveOutcome::Refused {
            item: Some(item),
            reason: Refused::NoSuchBag,
        });
    };
    if slot < 0 || slot >= slot_count {
        return Ok(MoveOutcome::Refused {
            item: Some(item),
            reason: Refused::OutOfRange,
        });
    }

    let occupant: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM item
          WHERE character_id = $1 AND bag_id = $2 AND slot = $3 AND id <> $4",
    )
    .bind(character)
    .bind(bag_id)
    .bind(slot)
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await?;
    if occupant.is_some() {
        return Ok(MoveOutcome::Refused {
            item: Some(item),
            reason: Refused::Occupied,
        });
    }

    sqlx::query("UPDATE item SET bag_id = $3, slot = $4 WHERE character_id = $1 AND id = $2")
        .bind(character)
        .bind(item_id)
        .bind(bag_id)
        .bind(slot)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(MoveOutcome::At {
        item: Item {
            bag_id: Some(bag_id),
            slot: Some(slot),
            ..item
        },
        bag_id,
        slot,
    })
}

/// The first free slot in the bags a player carries, backpack first.
///
/// The reference has the same idea (`GmBag_TryAddToBag`) and the same bug - its loop tests
/// `items[idx] != 0` where it means `== 0`, so it overwrites an occupied slot instead of finding an
/// empty one. This is the intent, not the bug: unequipping something has to have somewhere to put it.
pub async fn first_free_carrying_slot(
    pool: &PgPool,
    character: Uuid,
) -> sqlx::Result<Option<(i32, i32)>> {
    let row: Option<(i32, i32)> = sqlx::query_as(
        "SELECT b.bag_id, free.slot
           FROM bag b
           CROSS JOIN LATERAL generate_series(0, b.slot_count - 1) AS free(slot)
          WHERE b.character_id = $1
            AND b.bag_model_id BETWEEN $2 AND $3
            AND NOT EXISTS (
                SELECT 1 FROM item i
                 WHERE i.character_id = b.character_id
                   AND i.bag_id = b.bag_id
                   AND i.slot = free.slot
            )
          ORDER BY b.bag_model_id, b.bag_id, free.slot
          LIMIT 1",
    )
    .bind(character)
    .bind(BACKPACK_MODEL)
    .bind(LAST_CARRYING_MODEL)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seeded_inventory_is_the_reference_ones_bags_and_items() {
        let inventory = default_inventory();
        assert_eq!(
            inventory.bags.len(),
            6,
            "six bags, as GameSrv_CreateDefaultBags"
        );
        assert_eq!(inventory.items.len(), 9, "nine items in the capture");
        assert!(!inventory.is_empty());

        // The backpack is the first bag the reference sends, and it is the one that carries an item.
        assert_eq!(inventory.bags[0].bag_id, 1);
        assert_eq!(inventory.bags[0].bag_model_id, BACKPACK_MODEL);
        assert_eq!(inventory.bags[0].assoc_item_id, 1);
        assert!(
            inventory.bags[1..].iter().all(|bag| bag.assoc_item_id == 0),
            "only the backpack is an item"
        );

        // Bags are in the reference's send order: by model id, which is not the order of their ids.
        let models: Vec<i32> = inventory.bags.iter().map(|bag| bag.bag_model_id).collect();
        assert_eq!(models, vec![0, 5, 6, 7, 8, EQUIPPED_ITEMS_MODEL]);
        assert_eq!(
            inventory
                .bags
                .iter()
                .map(|bag| bag.bag_id)
                .collect::<Vec<_>>(),
            vec![1, 2, 6, 3, 4, 5],
            "the ids are the reference's own, handed out in a different order"
        );
    }

    #[test]
    fn the_items_are_where_the_capture_put_them() {
        let inventory = default_inventory();
        let backpack = inventory.items_in(1);
        assert_eq!(
            backpack.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![2, 9],
            "two cupcakes, in slots 0 and 1"
        );
        assert!(backpack.iter().all(|item| item.quantity == 100));
        assert_eq!(backpack[0].file_id, 0x47C34, "the cupcake");
        assert_eq!(backpack[0].model_id, 22269);

        // The bag's own item is not in a slot at all.
        let bag_item = inventory.item(1).expect("the backpack's item");
        assert_eq!(bag_item.bag_id, None);
        assert_eq!(bag_item.slot, None);
        assert_eq!(bag_item.item_type, 3, "ItemType_Bag");
        assert_eq!(bag_item.name.len(), 4, "its name is four code units");
        assert_eq!(bag_item.modifier_dwords(), vec![0x2448_1400]);

        // The equipment, in the equipped-items bag, by slot.
        let worn = inventory.equipped_item_ids();
        assert_eq!(worn, [3, 0, 4, 5, 6, 7, 8, 0, 0]);
        assert!(
            inventory
                .items_in(5)
                .iter()
                .all(|item| item.profession == 1),
            "a piece of equipment carries the profession it is for"
        );
    }

    #[test]
    fn a_drop_looks_only_at_the_bags_the_reference_searches() {
        // The rule `GmBag_FindItem` encodes, as the bounds the query uses: it walks from
        // `BagModelId_Backpack` to `BagModelId_Bag2` and looks nowhere else.
        let searched = [BACKPACK_MODEL, LAST_CARRYING_MODEL];
        assert_eq!(searched, [0, 3]);
        assert!(
            !searched.contains(&EQUIPPED_ITEMS_MODEL),
            "worn equipment is not a place a drop comes from"
        );
    }

    #[test]
    fn an_inventory_with_no_equipment_hands_the_client_nine_zeroes() {
        let inventory = Inventory::default();
        assert_eq!(inventory.equipped_item_ids(), [0; 9]);
        assert!(inventory.is_empty());
    }
}
