//! The default inventory, as seed rows. DO NOT EDIT BY HAND.
//!
//! The six bags and nine items the reference creates for the character it loads, with its own ids,
//! file ids, models, flags and placements.
//!
//! Why this data rather than invented items: a client given no items has no inventory model and
//! dies when it draws the character it has just created, and every field here (a file id into
//! `Gw.dat`, a model id, a flag word) is data, not something this project can derive.
//!
//! The data came from a capture of the reference at the parity point. The generator and the capture
//! were retired when parity closed (docs/16): this file is the data now, kept as rows that
//! `asura_db::inventory::seed_default` writes.
//!
//! The ids are the capture's. `asura_db::inventory::seed_default` renumbers them from the item
//! sequence when it writes them, because an item id is a runtime allocation in the reference too
//! (`GameSrv_AllocateItem`) and two characters cannot both own item 1. They are kept here so the
//! item that *is* the backpack can be named, and so a fresh database -- whose sequence starts at
//! one -- reproduces the capture exactly.
use crate::inventory::{SeedBag, SeedItem};

/// The bags, in the order the reference sends them: its bag array's order, which is by bag
/// model id (backpack, material storage, unclaimed, storage, equipped items).
#[rustfmt::skip]
pub const BAGS: &[SeedBag] = &[
    // Backpack: BagType_Bag, 20 slots
    SeedBag { bag_id: 1, bag_model_id: 0, bag_type: 1, slot_count: 20, assoc_item: 1 },
    // MaterialStorage: BagType_MatStorage, 41 slots
    SeedBag { bag_id: 2, bag_model_id: 5, bag_type: 5, slot_count: 41, assoc_item: 0 },
    // UnclaimedItems: BagType_NotCollected, 12 slots
    SeedBag { bag_id: 6, bag_model_id: 6, bag_type: 3, slot_count: 12, assoc_item: 0 },
    // Storage1: BagType_Storage, 25 slots
    SeedBag { bag_id: 3, bag_model_id: 7, bag_type: 4, slot_count: 25, assoc_item: 0 },
    // Storage2: BagType_Storage, 25 slots
    SeedBag { bag_id: 4, bag_model_id: 8, bag_type: 4, slot_count: 25, assoc_item: 0 },
    // EquippedItems: BagType_Equipped, 9 slots
    SeedBag { bag_id: 5, bag_model_id: 21, bag_type: 2, slot_count: 9, assoc_item: 0 },
];

/// The items, in the order the reference created them -- which is the order the item
/// sequence hands out ids, so a fresh database keeps the capture's numbering.
#[rustfmt::skip]
pub const ITEMS: &[SeedItem] = &[
    // item 1: type 3, model 32, no slot (the bag's own item)
    SeedItem {
        id: 1, bag_id: None, slot: None,
        file_id: 0x8001B536, item_type: 3, model_id: 32,
        dye_tint: 1, dye_colors: 0, materials: 0, unk1: 0,
        flags: 0x20001000, value: 5, quantity: 1, profession: 0,
        name: &[0x21A8, 0xD157, 0xB58F, 0x166F],
        modifiers: &[0x24481400],
    },
    // item 2: type 9, model 22269, bag 1 slot 0
    SeedItem {
        id: 2, bag_id: Some(1), slot: Some(0),
        file_id: 0x47C34, item_type: 9, model_id: 22269,
        dye_tint: 0, dye_colors: 0, materials: 0, unk1: 0,
        flags: 0x21080201, value: 0, quantity: 100, profession: 0,
        name: &[],
        modifiers: &[],
    },
    // item 9: type 9, model 22269, bag 1 slot 1
    SeedItem {
        id: 9, bag_id: Some(1), slot: Some(1),
        file_id: 0x47C34, item_type: 9, model_id: 22269,
        dye_tint: 0, dye_colors: 0, materials: 0, unk1: 0,
        flags: 0x21080201, value: 0, quantity: 100, profession: 0,
        name: &[],
        modifiers: &[],
    },
    // item 3: type 15, model 1699, bag 5 slot 0
    SeedItem {
        id: 3, bag_id: Some(5), slot: Some(0),
        file_id: 0x80009B60, item_type: 15, model_id: 1699,
        dye_tint: 6, dye_colors: 0, materials: 0, unk1: 0,
        flags: 0x22201000, value: 0, quantity: 1, profession: 1,
        name: &[],
        modifiers: &[],
    },
    // item 4: type 7, model 823, bag 5 slot 2
    SeedItem {
        id: 4, bag_id: Some(5), slot: Some(2),
        file_id: 0x8000005B, item_type: 7, model_id: 823,
        dye_tint: 19, dye_colors: 11, materials: 0, unk1: 0,
        flags: 0x20001006, value: 0, quantity: 1, profession: 1,
        name: &[],
        modifiers: &[],
    },
    // item 5: type 19, model 2440, bag 5 slot 3
    SeedItem {
        id: 5, bag_id: Some(5), slot: Some(3),
        file_id: 0x8000005E, item_type: 19, model_id: 2440,
        dye_tint: 19, dye_colors: 11, materials: 0, unk1: 0,
        flags: 0x20001006, value: 0, quantity: 1, profession: 1,
        name: &[],
        modifiers: &[],
    },
    // item 6: type 16, model 6136, bag 5 slot 4
    SeedItem {
        id: 6, bag_id: Some(5), slot: Some(4),
        file_id: 0x8000005D, item_type: 16, model_id: 6136,
        dye_tint: 19, dye_colors: 11, materials: 0, unk1: 0,
        flags: 0x20001006, value: 0, quantity: 1, profession: 1,
        name: &[],
        modifiers: &[],
    },
    // item 7: type 4, model 355, bag 5 slot 5
    SeedItem {
        id: 7, bag_id: Some(5), slot: Some(5),
        file_id: 0x8000005A, item_type: 4, model_id: 355,
        dye_tint: 19, dye_colors: 11, materials: 0, unk1: 0,
        flags: 0x20001006, value: 0, quantity: 1, profession: 1,
        name: &[],
        modifiers: &[],
    },
    // item 8: type 13, model 1598, bag 5 slot 6
    SeedItem {
        id: 8, bag_id: Some(5), slot: Some(6),
        file_id: 0x8000005C, item_type: 13, model_id: 1598,
        dye_tint: 19, dye_colors: 11, materials: 0, unk1: 0,
        flags: 0x20001006, value: 0, quantity: 1, profession: 1,
        name: &[],
        modifiers: &[],
    },
];
