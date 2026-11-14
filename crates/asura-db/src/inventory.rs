//! The inventory: a character's bags and the items in them.
//!
//! Until this module existed the world server answered the client's item request with the reference's
//! own messages, replayed byte for byte from a capture of the reference (retired when parity closed, docs/16). That made a client
//! happy and made the server know nothing: there was no row, bag or slot to name, so nothing could be
//! dropped, equipped, moved or saved. These are the rows that replaced them, and the two operations the
//! world server needs them for:
//!
//! * **reading** a character's inventory into the shape the wire wants (`load`);
//! * **creating** the default inventory for a character that has none (`seed_default`), which is what
//!   the reference does for every player it creates (`GameSrv_CreateDefaultBags`, guarded by
//!   `IsValidBag` so a loaded character keeps what it had);
//! * **finding** the item a client asks to drop (`find_in_carrying_bags`), restricted to the bags the
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
    /// How many slots the bag holds, as `GmBag.slots`.
    pub slot_count: i32,
    /// The item that *is* the bag (the reference's `bag_item_id`), 0 for the bags that have none.
    pub assoc_item_id: i32,
}

/// One item, with every field `CREATE_NAMED_ITEM` (`0x0161`) carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The client-visible id: what a drop names, and what `UPDATE_AGENT_VISUAL_EQUIPMENT` carries.
    pub id: i32,
    /// The static definition this instance names in the world pack, when the row records one.
    /// Null is a complete row: every wire field below still rebuilds its message. The definition
    /// itself is never stored here (`crates/asura-world/src/item_catalogue.rs`).
    pub definition_id: Option<String>,
    /// Where it sits. `None` for an item that is not in a slot - a bag's own item.
    pub bag_id: Option<i32>,
    /// Which slot inside that bag; `None` for a bag`s own item.
    pub slot: Option<i32>,
    /// The `Gw.dat` file id the client draws the model from.
    pub file_id: i64,
    /// The item`s kind code; the client groups bags and dyes by it.
    pub item_type: i32,
    /// The model id the client draws, which for some kinds differs from `file_id`.
    pub model_id: i32,
    /// Which dye slots apply; 0 for an undyed item.
    pub dye_tint: i32,
    /// The four packed dye colours, four bits each.
    pub dye_colors: i32,
    /// Crafting materials the item carries or needs.
    pub materials: i32,
    /// The reference`s own unverified byte; copied, not reasoned about.
    pub unk1: i32,
    /// Item flags: identified, customized, equipped and the rest.
    pub flags: i64,
    /// The item`s value in gold, as the client shows it.
    pub value: i32,
    /// How many are in the stack; 1 for a non-stackable item.
    pub quantity: i32,
    /// The profession the item was made for, or 0.
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
    /// Every item the character owns, in the order the reference`s bag walk produces.
    pub items: Vec<Item>,
}

impl Inventory {
    /// Empty is the answer for a character that has never been loaded, and it is what
    /// `seed_default` fills.
    pub fn is_empty(&self) -> bool {
        self.bags.is_empty() && self.items.is_empty()
    }

    /// The bag with this id, if the character has it.
    pub fn bag(&self, bag_id: i32) -> Option<&Bag> {
        self.bags.iter().find(|bag| bag.bag_id == bag_id)
    }

    /// The item with this id, if the character has it.
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
    /// The capture's bag id.
    pub bag_id: i32,
    /// `BagModelId`, as `Bag` carries it: 0 backpack, 5 material storage, 7+ storage, 21 equipped.
    pub bag_model_id: i32,
    /// `BagType`: 1 bag, 2 equipped, 3 not collected, 4 storage, 5 material storage.
    pub bag_type: i32,
    /// How many slots the bag holds, as `GmBag.slots`.
    pub slot_count: i32,
    /// The capture's item id of the item that is this bag, 0 when the bag has none.
    pub assoc_item: i32,
}

/// One item as the seed describes it. The slices are the wire's own units and dwords.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedItem {
    /// The capture's item id. Renumbered from the sequence when it is written.
    pub id: i32,
    /// The id of this item's static definition in `config/World/<pack>/items.json`. This is a
    /// reference, not a copy: the definition's facts stay in the pack.
    pub definition_id: &'static str,
    /// Which bag the record belongs to; `None` for an item that is not in a slot.
    pub bag_id: Option<i32>,
    /// Which slot inside that bag; `None` for a bag`s own item.
    pub slot: Option<i32>,
    /// The `Gw.dat` file id the client draws the model from.
    pub file_id: i64,
    /// The item`s kind code; the client groups bags and dyes by it.
    pub item_type: i32,
    /// The model id the client draws, which for some kinds differs from `file_id`.
    pub model_id: i32,
    /// Which dye slots apply; 0 for an undyed item.
    pub dye_tint: i32,
    /// The four packed dye colours, four bits each.
    pub dye_colors: i32,
    /// Crafting materials the item carries or needs.
    pub materials: i32,
    /// The reference`s own unverified byte; copied, not reasoned about.
    pub unk1: i32,
    /// Item flags: identified, customized, equipped and the rest.
    pub flags: i64,
    /// The item`s value in gold, as the client shows it.
    pub value: i32,
    /// How many are in the stack; 1 for a non-stackable item.
    pub quantity: i32,
    /// The profession the item was made for, or 0.
    pub profession: i32,
    /// The name, as the UTF-16 code units the wire carries.
    pub name: &'static [i32],
    /// The modifier list, in the client`s own dwords.
    pub modifiers: &'static [i64],
}

impl SeedItem {
    /// The seed row as a row: the shape both the reader and the wire builder use.
    pub fn item(&self) -> Item {
        Item {
            id: self.id,
            definition_id: Some(self.definition_id.to_string()),
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
/// This is the seed: `seed_default` writes these rows (with ids taken from the sequence, because
/// two characters cannot both own item 1). The rows are the source of truth now; the world server
/// builds the item block out of them.
pub fn default_inventory() -> Inventory {
    Inventory {
        bags: seed::BAGS.iter().map(Bag::from).collect(),
        items: seed::ITEMS.iter().map(SeedItem::item).collect(),
    }
}

/// The item columns every query selects. One list, so no query can select a shape [`ItemRow`] does
/// not decode; the row decodes by column name, not by position.
const ITEM_COLUMNS: &str =
    "id, definition_id, bag_id, slot, file_id, item_type, model_id, dye_tint, \
                            dye_colors, materials, unk1, flags, value, quantity, profession, name, \
                            modifiers";

/// The same columns, qualified for a query that has more than one table in it.
fn item_columns(alias: &str) -> String {
    ITEM_COLUMNS
        .split(',')
        .map(|column| format!("{alias}.{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One item row as both queries select it (`load` and `find_in_carrying_bags` share the column list
/// so the two cannot drift apart).
///
/// A named struct rather than a tuple: `sqlx` decodes tuples only up to sixteen fields, and the
/// instance gained `definition_id` as its seventeenth. The columns are selected by name here, which
/// also keeps a query's order from silently becoming the row's order.
#[derive(Debug)]
struct ItemRow {
    id: i32,
    definition_id: Option<String>,
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
    name: Vec<i32>,
    modifiers: Vec<i64>,
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for ItemRow {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(ItemRow {
            id: row.try_get("id")?,
            definition_id: row.try_get("definition_id")?,
            bag_id: row.try_get("bag_id")?,
            slot: row.try_get("slot")?,
            file_id: row.try_get("file_id")?,
            item_type: row.try_get("item_type")?,
            model_id: row.try_get("model_id")?,
            dye_tint: row.try_get("dye_tint")?,
            dye_colors: row.try_get("dye_colors")?,
            materials: row.try_get("materials")?,
            unk1: row.try_get("unk1")?,
            flags: row.try_get("flags")?,
            value: row.try_get("value")?,
            quantity: row.try_get("quantity")?,
            profession: row.try_get("profession")?,
            name: row.try_get("name")?,
            modifiers: row.try_get("modifiers")?,
        })
    }
}

/// A row as [`Item`].
fn item_from_row(row: ItemRow) -> Item {
    Item {
        id: row.id,
        definition_id: row.definition_id,
        bag_id: row.bag_id,
        slot: row.slot,
        file_id: row.file_id,
        item_type: row.item_type,
        model_id: row.model_id,
        dye_tint: row.dye_tint,
        dye_colors: row.dye_colors,
        materials: row.materials,
        unk1: row.unk1,
        flags: row.flags,
        value: row.value,
        quantity: row.quantity,
        profession: row.profession,
        name: row.name,
        modifiers: row.modifiers,
    }
}

/// Read a character's bags and items.
///
/// Bags come back in the reference's send order (`bag_model_id`), items in `(bag, slot)` order, so the
/// world server can build the block without re-sorting and without knowing why the order matters.
pub async fn load(pool: &PgPool, character: Uuid) -> sqlx::Result<Inventory> {
    let bags: Vec<(i32, i32, i32, i32, i32)> = sqlx::query_as(
        "SELECT bag_id, bag_model_id, bag_type, slot_count, assoc_item_id
           FROM t_bag WHERE character_id = $1
          ORDER BY bag_model_id, bag_id",
    )
    .bind(character)
    .fetch_all(pool)
    .await?;

    let items: Vec<ItemRow> = sqlx::query_as(&format!(
        "SELECT {ITEM_COLUMNS} FROM t_item WHERE character_id = $1
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
    /// The static definition this instance names, when its caller has one. A caller that has only
    /// wire values leaves it `None`; the row is still complete.
    pub definition_id: Option<String>,
    /// Which bag the record belongs to; `None` for an item that is not in a slot.
    pub bag_id: Option<i32>,
    /// Which slot inside that bag; `None` for a bag`s own item.
    pub slot: Option<i32>,
    /// The `Gw.dat` file id the client draws the model from.
    pub file_id: i64,
    /// The item`s kind code; the client groups bags and dyes by it.
    pub item_type: i32,
    /// The model id the client draws, which for some kinds differs from `file_id`.
    pub model_id: i32,
    /// Which dye slots apply; 0 for an undyed item.
    pub dye_tint: i32,
    /// The four packed dye colours, four bits each.
    pub dye_colors: i32,
    /// Crafting materials the item carries or needs.
    pub materials: i32,
    /// The reference`s own unverified byte; copied, not reasoned about.
    pub unk1: i32,
    /// Item flags: identified, customized, equipped and the rest.
    pub flags: i64,
    /// The item`s value in gold, as the client shows it.
    pub value: i32,
    /// How many are in the stack; 1 for a non-stackable item.
    pub quantity: i32,
    /// The profession the item was made for, or 0.
    pub profession: i32,
    /// The name, as the UTF-16 code units the wire carries.
    pub name: Vec<i32>,
    /// The modifier list, in the client`s own dwords.
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
        "INSERT INTO t_characters (id, account_id, name, map_id, appearance, primary_profession,
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
            "INSERT INTO t_bag (character_id, bag_id, bag_model_id, bag_type, slot_count, assoc_item_id)
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
    insert_row(tx, character, None, ItemColumns::from(seed)).await
}

/// One item row from a runtime set, returning the id the sequence gave it.
async fn insert_new_item(
    tx: &mut PgConnection,
    character: Uuid,
    item: &NewItem,
) -> sqlx::Result<i32> {
    insert_row(tx, character, None, ItemColumns::from(item)).await
}

/// The seventeen columns one `item` INSERT writes, borrowed from whichever record produced them.
///
/// A struct rather than sixteen positional arguments: `SeedItem` and `NewItem` carry the same columns
/// with different ownership, so one borrow view serves both and a swapped pair cannot compile
/// (`docs/38-layer-decisions.md`, D16).
struct ItemColumns<'a> {
    definition_id: Option<&'a str>,
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
    name: &'a [i32],
    modifiers: &'a [i64],
}

impl<'a> From<&'a SeedItem> for ItemColumns<'a> {
    fn from(item: &'a SeedItem) -> Self {
        ItemColumns {
            definition_id: Some(item.definition_id),
            bag_id: item.bag_id,
            slot: item.slot,
            file_id: item.file_id,
            item_type: item.item_type,
            model_id: item.model_id,
            dye_tint: item.dye_tint,
            dye_colors: item.dye_colors,
            materials: item.materials,
            unk1: item.unk1,
            flags: item.flags,
            value: item.value,
            quantity: item.quantity,
            profession: item.profession,
            name: item.name,
            modifiers: item.modifiers,
        }
    }
}

impl<'a> From<&'a NewItem> for ItemColumns<'a> {
    fn from(item: &'a NewItem) -> Self {
        ItemColumns {
            definition_id: item.definition_id.as_deref(),
            bag_id: item.bag_id,
            slot: item.slot,
            file_id: item.file_id,
            item_type: item.item_type,
            model_id: item.model_id,
            dye_tint: item.dye_tint,
            dye_colors: item.dye_colors,
            materials: item.materials,
            unk1: item.unk1,
            flags: item.flags,
            value: item.value,
            quantity: item.quantity,
            profession: item.profession,
            name: &item.name,
            modifiers: &item.modifiers,
        }
    }
}

impl<'a> From<&'a Item> for ItemColumns<'a> {
    fn from(item: &'a Item) -> Self {
        ItemColumns {
            definition_id: item.definition_id.as_deref(),
            bag_id: item.bag_id,
            slot: item.slot,
            file_id: item.file_id,
            item_type: item.item_type,
            model_id: item.model_id,
            dye_tint: item.dye_tint,
            dye_colors: item.dye_colors,
            materials: item.materials,
            unk1: item.unk1,
            flags: item.flags,
            value: item.value,
            quantity: item.quantity,
            profession: item.profession,
            name: &item.name,
            modifiers: &item.modifiers,
        }
    }
}

/// The one `t_item` INSERT. `id` is `None` for a new row (the sequence picks it) and `Some` for a row
/// whose id the client already knows: a picked-up stack, or an item whose id was allocated first so
/// the frames could be built before the write.
async fn insert_row(
    tx: &mut PgConnection,
    character: Uuid,
    id: Option<i32>,
    item: ItemColumns<'_>,
) -> sqlx::Result<i32> {
    sqlx::query_scalar(
        "INSERT INTO t_item (id, character_id, definition_id, bag_id, slot, file_id, item_type,
                           model_id, dye_tint, dye_colors, materials, unk1, flags, value, quantity,
                           profession, name, modifiers)
         VALUES (COALESCE($1, nextval('item_id_seq')::int), $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 $11, $12, $13, $14, $15, $16, $17, $18)
         RETURNING id",
    )
    .bind(id)
    .bind(character)
    .bind(item.definition_id)
    .bind(item.bag_id)
    .bind(item.slot)
    .bind(item.file_id)
    .bind(item.item_type)
    .bind(item.model_id)
    .bind(item.dye_tint)
    .bind(item.dye_colors)
    .bind(item.materials)
    .bind(item.unk1)
    .bind(item.flags)
    .bind(item.value)
    .bind(item.quantity)
    .bind(item.profession)
    .bind(item.name.to_vec())
    .bind(item.modifiers.to_vec())
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
        sqlx::query_scalar("SELECT id FROM t_characters WHERE id = $1 FOR UPDATE")
            .bind(character)
            .fetch_optional(&mut *tx)
            .await?;
    if exists.is_none() {
        return Ok(false);
    }

    let bags: i64 = sqlx::query_scalar("SELECT count(*) FROM t_bag WHERE character_id = $1")
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
        let id = insert_item(tx.as_mut(), character, seed).await?;
        allocated.push((seed.id, id));
    }

    for seed in seed::BAGS {
        let assoc_item_id = bag_own_item(seed, &allocated)?;

        sqlx::query(
            "INSERT INTO t_bag (character_id, bag_id, bag_model_id, bag_type, slot_count, assoc_item_id)
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
        "SELECT {} FROM t_item i
           JOIN t_bag b ON b.character_id = i.character_id AND b.bag_id = i.bag_id
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
        "SELECT {ITEM_COLUMNS} FROM t_item WHERE character_id = $1 AND id = $2 FOR UPDATE"
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
        sqlx::query("DELETE FROM t_item WHERE character_id = $1 AND id = $2")
            .bind(character)
            .bind(item_id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("UPDATE t_item SET quantity = $3 WHERE character_id = $1 AND id = $2")
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
    At {
        /// The item, with its new place filled in.
        item: Item,
        /// Where it now sits.
        bag_id: i32,
        /// Which slot of that bag.
        slot: i32,
    },
    /// It was already there; nothing was written.
    Unchanged {
        /// The item, exactly as it was.
        item: Item,
    },
    /// Nothing happened, and `item` is where it actually is - a caller uses that to put the client's
    /// own model back, because the client moves the item optimistically before it asks.
    Refused {
        /// The item where it really is, when the character still has it.
        item: Option<Item>,
        /// Why the move did not happen.
        reason: Refused,
    },
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
        "SELECT {ITEM_COLUMNS} FROM t_item WHERE character_id = $1 AND id = $2 FOR UPDATE"
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
        sqlx::query_as("SELECT slot_count FROM t_bag WHERE character_id = $1 AND bag_id = $2")
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
        "SELECT id FROM t_item
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

    sqlx::query("UPDATE t_item SET bag_id = $3, slot = $4 WHERE character_id = $1 AND id = $2")
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

/// Add one item row for a character, in one transaction, returning its id.
///
/// The caller chooses a free bag slot; this only writes the row. It exists for
/// the world's verified-item test command, which is not a reference handler.
pub async fn create_item(pool: &PgPool, character: Uuid, item: &NewItem) -> sqlx::Result<i32> {
    let mut tx = pool.begin().await?;
    let id = insert_new_item(tx.as_mut(), character, item).await?;
    tx.commit().await?;
    Ok(id)
}

/// The next item id, without writing a row.
///
/// A part of a stack dropped on the ground needs an id the client can name in its item table, but it
/// is not a bag row; the live server's ground items use the same id space as carried items
/// (`items-live-20261108-174014.jsonl` seq 134: item 622 dropped five, the ground record is item 359).
pub async fn allocate_item_id(pool: &PgPool) -> sqlx::Result<i32> {
    sqlx::query_scalar("SELECT nextval('item_id_seq')::int")
        .fetch_one(pool)
        .await
}

/// Put a whole stack back into a bag under the id it had before it was dropped.
///
/// A full-stack drop deletes the row and keeps the item's id; the client still knows that id from
/// `ITEM_REMOVE`, so the pickup has to restore the same row id rather than take a new one
/// (`items-live-20261108-173913.jsonl` seq 61: item 609 returns under its own id).
pub async fn restore_item(
    pool: &PgPool,
    character: Uuid,
    item: &Item,
    bag_id: i32,
    slot: i32,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    let columns = ItemColumns {
        bag_id: Some(bag_id),
        slot: Some(slot),
        ..ItemColumns::from(item)
    };
    insert_row(tx.as_mut(), character, Some(item.id), columns).await?;
    tx.commit().await?;
    Ok(())
}

/// One row-level write of an inventory operation (`crate`-level counterpart of the world's
/// `inventory_model::Change`). [`apply_changes`] writes a whole operation's list in one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowChange {
    /// The item now sits at `(bag_id, slot)`.
    Place {
        /// The item.
        item_id: i32,
        /// Its bag.
        bag_id: i32,
        /// Its slot in that bag.
        slot: i32,
    },
    /// The stack now holds `quantity` (at least one).
    Quantity {
        /// The item.
        item_id: i32,
        /// What the stack holds now.
        quantity: i32,
    },
    /// The row is gone.
    Delete {
        /// The item.
        item_id: i32,
    },
    /// A new row, under the id the item carries (allocated beforehand with [`allocate_item_id`], or
    /// the id a picked-up stack had). Its `bag_id` and `slot` say where it goes.
    Insert {
        /// The whole row.
        item: Item,
    },
    /// The gold the character carries goes from `from` to `to`. Refused unless the row still holds
    /// `from`, so a purchase cannot spend gold another writer has already spent.
    Gold {
        /// The balance the operation was decided on.
        from: i32,
        /// The balance it leaves.
        to: i32,
    },
    /// The gold in the character's storage goes from `from` to `to` (migration 0035), checked the same way.
    StorageGold {
        /// The balance the operation was decided on.
        from: i32,
        /// The balance it leaves.
        to: i32,
    },
}

/// Write one inventory operation's changes, all or nothing.
///
/// The character row is locked first, so two writers for one character cannot interleave. Every
/// change must hit exactly one row of this character, or the whole operation is rolled back and the
/// error says which change missed - the world then reloads and puts the client back.
///
/// Slot moves are written in two steps (lift every moved row out of its slot, then place them all),
/// because a swap is a cycle: the two rows trade slots, and the one-item-per-slot index would refuse
/// either order of two plain updates.
pub async fn apply_changes(
    pool: &PgPool,
    character: Uuid,
    changes: &[RowChange],
) -> sqlx::Result<()> {
    if changes.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    let locked: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM t_characters WHERE id = $1 FOR UPDATE")
            .bind(character)
            .fetch_optional(&mut *tx)
            .await?;
    if locked.is_none() {
        return Err(sqlx::Error::RowNotFound);
    }

    fn missed(what: &str, item_id: i32) -> sqlx::Error {
        sqlx::Error::Protocol(format!(
            "inventory write: {what} of item {item_id} matched no row of this character"
        ))
    }

    // 1. Deletions, so their slots are free for what follows.
    for change in changes {
        if let RowChange::Delete { item_id } = change {
            let hit = sqlx::query("DELETE FROM t_item WHERE character_id = $1 AND id = $2")
                .bind(character)
                .bind(*item_id)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            if hit != 1 {
                return Err(missed("delete", *item_id));
            }
        }
    }
    // 2. Lift every row that moves out of its slot.
    for change in changes {
        if let RowChange::Place { item_id, .. } = change {
            let hit = sqlx::query(
                "UPDATE t_item SET bag_id = NULL, slot = NULL
                  WHERE character_id = $1 AND id = $2 AND bag_id IS NOT NULL",
            )
            .bind(character)
            .bind(*item_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if hit != 1 {
                return Err(missed("move", *item_id));
            }
        }
    }
    // 3. Quantities, new rows, the moved rows' new slots and the gold, in the operation's order.
    for change in changes {
        match change {
            RowChange::Delete { .. } => {}
            RowChange::Gold { from, to } | RowChange::StorageGold { from, to } => {
                let (column, what) = match change {
                    RowChange::Gold { .. } => ("gold", "carried gold"),
                    _ => ("gold_storage", "storage gold"),
                };
                if *to < 0 {
                    return Err(sqlx::Error::Protocol(format!(
                        "inventory write: {what} cannot go below 0 ({to})"
                    )));
                }
                let hit = sqlx::query(&format!(
                    "UPDATE t_characters SET {column} = $3 WHERE id = $1 AND {column} = $2"
                ))
                .bind(character)
                .bind(*from)
                .bind(*to)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if hit != 1 {
                    return Err(sqlx::Error::Protocol(format!(
                        "inventory write: {what} was not {from} any more"
                    )));
                }
            }
            RowChange::Quantity { item_id, quantity } => {
                if *quantity < 1 {
                    return Err(missed("an empty quantity", *item_id));
                }
                let hit = sqlx::query(
                    "UPDATE t_item SET quantity = $3 WHERE character_id = $1 AND id = $2",
                )
                .bind(character)
                .bind(*item_id)
                .bind(*quantity)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                if hit != 1 {
                    return Err(missed("quantity", *item_id));
                }
            }
            RowChange::Insert { item } => {
                insert_row(tx.as_mut(), character, Some(item.id), ItemColumns::from(item)).await?;
            }
            RowChange::Place {
                item_id,
                bag_id,
                slot,
            } => {
                sqlx::query(
                    "UPDATE t_item SET bag_id = $3, slot = $4 WHERE character_id = $1 AND id = $2",
                )
                .bind(character)
                .bind(*item_id)
                .bind(*bag_id)
                .bind(*slot)
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Add `count` back to a stack that stayed in a bag, and say the quantity it now holds.
///
/// The live server does this instead of writing a new row when part of a stack is picked up: the drop
/// kept the stack and gave the ground part its own id, so the pickup names the stack and its new
/// quantity (`items-live-20261108-174014.jsonl` seq 162: item 622 goes from 5 back to 10).
/// `Ok(None)` means the stack is gone; the caller has to restore the ground part as its own row.
pub async fn merge_into_stack(
    pool: &PgPool,
    character: Uuid,
    item_id: i32,
    count: i32,
) -> sqlx::Result<Option<i32>> {
    if count <= 0 {
        return Ok(None);
    }
    let mut tx = pool.begin().await?;
    let row: Option<(i32,)> = sqlx::query_as(
        "SELECT quantity FROM t_item
          WHERE character_id = $1 AND id = $2 AND bag_id IS NOT NULL
          FOR UPDATE",
    )
    .bind(character)
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((quantity,)) = row else {
        return Ok(None);
    };
    let new_quantity = quantity.saturating_add(count);
    sqlx::query("UPDATE t_item SET quantity = $3 WHERE character_id = $1 AND id = $2")
        .bind(character)
        .bind(item_id)
        .bind(new_quantity)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(new_quantity))
}

/// Delete one item row the character owns in a bag slot.
///
/// `Ok(false)` means nothing was deleted: no such item, or the id names a bag's
/// own item (which has no slot). The live server answers a trash drag with
/// `ITEM_REMOVE` (`items-live-20261108-173913.jsonl` seq 193); the world handler
/// sends it after this returns true.
pub async fn destroy_item(pool: &PgPool, character: Uuid, item_id: i32) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let deleted = sqlx::query(
        "DELETE FROM t_item WHERE character_id = $1 AND id = $2 AND bag_id IS NOT NULL",
    )
    .bind(character)
    .bind(item_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(deleted > 0)
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
           FROM t_bag b
           CROSS JOIN LATERAL generate_series(0, b.slot_count - 1) AS free(slot)
          WHERE b.character_id = $1
            AND b.bag_model_id BETWEEN $2 AND $3
            AND NOT EXISTS (
                SELECT 1 FROM t_item i
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
