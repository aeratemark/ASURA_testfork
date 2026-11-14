//! The inventory rules, as a pure model: an operation in, a list of changes or a refusal out.
//!
//! This is the one place item rules live (`docs/50-item-framework.md` §3.2). It has no database, no
//! socket and no dependency outside `std`, so every rule is a plain unit test. The handlers in
//! `crate::item` build an [`InvModel`] from the character's rows, call [`InvModel::apply`], write the
//! [`Change`]s in one transaction and turn them into frames.
//!
//! [`InvModel::apply`] is atomic: it either returns every change and updates the model, or returns a
//! [`Refusal`] and leaves the model exactly as it was. That is what makes "the old piece came off but
//! the new one was refused" impossible.
//!
//! What is deliberately **not** here yet, because no capture shows the server's answer:
//! a drag onto an occupied carrying slot (retail swaps or merges; this refuses, and the handler snaps
//! the client back), splitting a stack by hand, and selecting a weapon set.

use std::collections::BTreeMap;

/// The client-visible item id.
pub type ItemId = u32;
/// The bag id the client is told (`INVENTORY_CREATE_BAG.bag_id`), not the bag's model.
pub type BagId = u16;
/// A slot inside one bag.
pub type Slot = u8;

/// What the client's `ItemType` means for the rules. One table, so equip, bag and stack rules all
/// read the same answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A main-hand weapon.
    Weapon(Hands),
    /// A focus or a shield.
    OffHand,
    /// An armour piece, worn in this equipped slot.
    Armor(Slot),
    /// A costume piece, worn in this equipped slot.
    Costume(Slot),
    /// A bag's own item.
    Bag,
    /// A crafting material (`ItemType_Materials`).
    Material,
    /// Anything else: trophies, consumables, quest items, keys, kits, dyes.
    Other,
}

/// How many hands a weapon takes. A two-handed weapon and an off-hand cannot be worn together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hands {
    One,
    Two,
}

/// Equipped-items bag slots (`EquippedItemSlot`).
pub mod equip_slot {
    use super::Slot;
    pub const WEAPON: Slot = 0;
    pub const OFF_HAND: Slot = 1;
    pub const BODY: Slot = 2;
    pub const LEGS: Slot = 3;
    pub const HEAD: Slot = 4;
    pub const BOOTS: Slot = 5;
    pub const GLOVES: Slot = 6;
    pub const COSTUME_BODY: Slot = 7;
    pub const COSTUME_HEAD: Slot = 8;
}

impl ItemKind {
    /// The kind of a client `ItemType`. The numbers are the client's enum (`GmItem.h`, Headquarter's
    /// `item.h`); the slot numbers agree with `asura_protocol::game::equipped_slot`.
    pub fn of(item_type: u8) -> ItemKind {
        use equip_slot::*;
        match item_type {
            // Axe, sword, wand, spear, and the leadhand half of a pair.
            1 | 2 | 22 | 27 | 36 => ItemKind::Weapon(Hands::One),
            // Bow, hammer, staff, daggers, scythe.
            5 | 15 | 26 | 32 | 35 => ItemKind::Weapon(Hands::Two),
            // Focus, shield.
            12 | 24 => ItemKind::OffHand,
            7 => ItemKind::Armor(BODY),
            19 => ItemKind::Armor(LEGS),
            16 => ItemKind::Armor(HEAD),
            4 => ItemKind::Armor(BOOTS),
            13 => ItemKind::Armor(GLOVES),
            44 => ItemKind::Costume(COSTUME_BODY),
            45 => ItemKind::Costume(COSTUME_HEAD),
            3 => ItemKind::Bag,
            11 => ItemKind::Material,
            _ => ItemKind::Other,
        }
    }

    /// The equipped-items bag slot this kind is worn in, if it is worn at all.
    pub fn equip_slot(self) -> Option<Slot> {
        match self {
            ItemKind::Weapon(_) => Some(equip_slot::WEAPON),
            ItemKind::OffHand => Some(equip_slot::OFF_HAND),
            ItemKind::Armor(slot) | ItemKind::Costume(slot) => Some(slot),
            _ => None,
        }
    }
}

/// What a bag is for, from its `BagModelId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BagKind {
    /// Backpack, belt pouch, bag 1, bag 2: what a character carries (`GmBag_FindItem`'s range).
    Carrying,
    /// The equipment pack: carried, but equipment only.
    EquipmentPack,
    /// Xunlai material storage: materials only.
    MaterialStorage,
    /// Items the server is holding for the player. The server fills it; a player never moves into it.
    Unclaimed,
    /// Xunlai storage panes.
    Storage,
    /// The nine worn slots.
    Equipped,
}

impl BagKind {
    /// The kind of a `BagModelId` (`GmInventory.h`).
    pub fn of(bag_model_id: i32) -> BagKind {
        match bag_model_id {
            0..=3 => BagKind::Carrying,
            4 => BagKind::EquipmentPack,
            5 => BagKind::MaterialStorage,
            6 => BagKind::Unclaimed,
            21 => BagKind::Equipped,
            _ => BagKind::Storage,
        }
    }

    /// Whether an item of `kind` may sit in a bag of this kind. The equipped bag has its own,
    /// stricter rule (the slot must match); see [`InvModel::apply`].
    fn accepts(self, kind: ItemKind) -> bool {
        match self {
            BagKind::Carrying | BagKind::Storage => true,
            BagKind::EquipmentPack => kind.equip_slot().is_some(),
            BagKind::MaterialStorage => kind == ItemKind::Material,
            BagKind::Unclaimed => false,
            BagKind::Equipped => kind.equip_slot().is_some(),
        }
    }

    /// Whether the character has this bag on them, so an item in it may be dropped, worn or used.
    pub fn is_carried(self) -> bool {
        matches!(self, BagKind::Carrying | BagKind::EquipmentPack)
    }

    /// Whether this is Xunlai storage: a storage pane or material storage, reached only through a
    /// storage window.
    pub fn is_storage(self) -> bool {
        matches!(self, BagKind::Storage | BagKind::MaterialStorage)
    }
}

/// One bag, as the rules need it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BagInfo {
    pub id: BagId,
    pub model: i32,
    pub kind: BagKind,
    pub slots: u8,
}

/// One item, as the rules need it. The full row stays with the caller; this is only what a rule reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub id: ItemId,
    /// `None` for a bag's own item, which is part of its bag and never moves.
    pub at: Option<(BagId, Slot)>,
    pub kind: ItemKind,
    /// The profession the item was made for, 0 for none. Armour with a profession is that profession's.
    pub profession: u8,
    pub quantity: u32,
    /// The largest stack this item may form; 1 for an item that does not stack.
    pub stack_max: u32,
}

/// Who is wearing the equipment: the rules that depend on the character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wearer {
    pub primary_profession: u8,
    /// A Xunlai storage window is open for this character (`crate::vendor`): only then may an item move
    /// into, out of or inside storage and material storage. The client only offers those bags while the
    /// window is open; this is the server holding it to that.
    pub storage_open: bool,
}

/// An operation a client (or the server) asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// `ITEM_MOVE`: put an item in a slot. Into the equipped bag this is an equip into that exact slot.
    Move { item: ItemId, bag: BagId, slot: Slot },
    /// `EQUIP_ITEM`: wear an item in the slot its kind belongs to.
    Equip { item: ItemId },
    /// `UNEQUIP_ITEM`: take off what is worn in `equipped` and put it at `(bag, slot)`.
    Unequip { equipped: Slot, bag: BagId, slot: Slot },
    /// `DROP_ITEM` and the like: take `count` off a carried stack.
    Take { item: ItemId, count: u32 },
    /// `ITEM_DESTROY`: delete an item in a slot.
    Destroy { item: ItemId },
    /// Add `count` back onto an existing stack (a picked-up part of it).
    Merge { into: ItemId, count: u32 },
    /// Put a new item (picked up, granted, looted) into the first free carrying slot. The caller
    /// chooses the id; the entry's `at` is ignored.
    Insert { entry: Entry },
}

/// One thing that changed. The handler writes these in one transaction and builds frames from them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The item now sits at `to`.
    Moved { item: ItemId, from: (BagId, Slot), to: (BagId, Slot) },
    /// The stack now holds `quantity`.
    Quantity { item: ItemId, quantity: u32 },
    /// The item's row is gone (destroyed, or a whole stack taken off).
    Removed { item: ItemId },
    /// A new item now sits at `at`.
    Inserted { item: ItemId, at: (BagId, Slot) },
    /// What is worn in equipped slot `slot` changed; `None` means nothing is worn there now.
    /// One per slot per operation, carrying the final state.
    Worn { slot: Slot, item: Option<ItemId> },
}

/// Why nothing happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NoSuchItem,
    /// A bag's own item, which is not in a slot.
    NotInASlot,
    NoSuchBag,
    /// The slot is outside the bag.
    OutOfRange,
    /// Something else is in that slot. (Retail swaps or merges; that answer is not measured yet.)
    Occupied,
    /// This bag does not take this kind of item (a cupcake in material storage, anything in unclaimed).
    WrongBag,
    /// Into the equipped bag, but not into the slot this item is worn in.
    WrongSlot,
    /// Not something a character wears.
    NotEquippable,
    /// Armour made for another profession.
    WrongProfession,
    /// Wearing or taking this would need a free carrying slot, and there is none.
    NoRoom,
    /// The item is not in a bag the character carries (storage, unclaimed, or worn when it must not be).
    NotCarried,
    /// More than the stack holds, zero, or past the stack's limit.
    BadQuantity,
    /// Nothing is worn in that equipped slot.
    NothingWorn,
    /// The id is already in use.
    DuplicateId,
    /// Storage or material storage, with no Xunlai window open.
    StorageClosed,
}

/// A character's inventory as the rules see it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvModel {
    bags: BTreeMap<BagId, BagInfo>,
    items: BTreeMap<ItemId, Entry>,
    slots: BTreeMap<(BagId, Slot), ItemId>,
}

impl InvModel {
    /// Build a model. Two items in one slot, or an item in a slot its bag does not have, is a
    /// corrupt inventory and is refused here rather than guessed around.
    pub fn new(
        bags: impl IntoIterator<Item = BagInfo>,
        items: impl IntoIterator<Item = Entry>,
    ) -> Result<InvModel, String> {
        let mut model = InvModel::default();
        for bag in bags {
            if model.bags.insert(bag.id, bag).is_some() {
                return Err(format!("bag {} appears twice", bag.id));
            }
        }
        for entry in items {
            if let Some((bag, slot)) = entry.at {
                let info = model
                    .bags
                    .get(&bag)
                    .ok_or_else(|| format!("item {} is in bag {bag}, which does not exist", entry.id))?;
                if slot >= info.slots {
                    return Err(format!(
                        "item {} is in slot {slot} of bag {bag}, which has {} slots",
                        entry.id, info.slots
                    ));
                }
                if let Some(other) = model.slots.insert((bag, slot), entry.id) {
                    return Err(format!(
                        "items {other} and {} are both in bag {bag} slot {slot}",
                        entry.id
                    ));
                }
            }
            if model.items.insert(entry.id, entry).is_some() {
                return Err(format!("item {} appears twice", entry.id));
            }
        }
        Ok(model)
    }

    pub fn item(&self, id: ItemId) -> Option<&Entry> {
        self.items.get(&id)
    }

    pub fn bag(&self, id: BagId) -> Option<&BagInfo> {
        self.bags.get(&id)
    }

    /// The item in a slot.
    pub fn at(&self, bag: BagId, slot: Slot) -> Option<&Entry> {
        self.slots.get(&(bag, slot)).and_then(|id| self.items.get(id))
    }

    /// The equipped-items bag.
    pub fn equipped_bag(&self) -> Option<BagId> {
        self.bags.values().find(|bag| bag.kind == BagKind::Equipped).map(|bag| bag.id)
    }

    /// What is worn in an equipped slot.
    pub fn worn(&self, slot: Slot) -> Option<&Entry> {
        self.equipped_bag().and_then(|bag| self.at(bag, slot))
    }

    /// The first free carrying slot, backpack first, as `first_free_carrying_slot` orders it
    /// (by bag model, then bag id, then slot).
    pub fn first_free_carrying(&self) -> Option<(BagId, Slot)> {
        self.first_free_carrying_except(&[])
    }

    /// The first free carrying slot that is not one of `avoid`.
    fn first_free_carrying_except(&self, avoid: &[(BagId, Slot)]) -> Option<(BagId, Slot)> {
        let mut bags: Vec<&BagInfo> = self
            .bags
            .values()
            .filter(|bag| bag.kind == BagKind::Carrying)
            .collect();
        bags.sort_by_key(|bag| (bag.model, bag.id));
        bags.into_iter().find_map(|bag| {
            (0..bag.slots)
                .find(|slot| {
                    !self.slots.contains_key(&(bag.id, *slot)) && !avoid.contains(&(bag.id, *slot))
                })
                .map(|slot| (bag.id, slot))
        })
    }

    /// Apply one operation. On success the model reflects it and the changes say what to write and
    /// send; on refusal the model is untouched.
    pub fn apply(&mut self, op: Op, wearer: Wearer) -> Result<Vec<Change>, Refusal> {
        let mut next = self.clone();
        let changes = next.plan(op, wearer)?;
        *self = next;
        Ok(changes)
    }

    fn plan(&mut self, op: Op, wearer: Wearer) -> Result<Vec<Change>, Refusal> {
        match op {
            Op::Move { item, bag, slot } => self.do_move(item, bag, slot, wearer),
            Op::Equip { item } => {
                let entry = *self.items.get(&item).ok_or(Refusal::NoSuchItem)?;
                let slot = entry.kind.equip_slot().ok_or(Refusal::NotEquippable)?;
                let bag = self.equipped_bag().ok_or(Refusal::NoSuchBag)?;
                self.do_equip(entry, bag, slot, wearer)
            }
            Op::Unequip { equipped, bag, slot } => {
                let worn = *self.worn(equipped).ok_or(Refusal::NothingWorn)?;
                self.do_move(worn.id, bag, slot, wearer)
            }
            Op::Take { item, count } => self.do_take(item, count),
            Op::Destroy { item } => {
                let entry = *self.items.get(&item).ok_or(Refusal::NoSuchItem)?;
                let at = entry.at.ok_or(Refusal::NotInASlot)?;
                self.remove(item);
                let mut changes = vec![Change::Removed { item }];
                self.note_worn(at, &mut changes);
                Ok(changes)
            }
            Op::Merge { into, count } => {
                let entry = self.items.get_mut(&into).ok_or(Refusal::NoSuchItem)?;
                if entry.at.is_none() {
                    return Err(Refusal::NotInASlot);
                }
                let total = entry.quantity.checked_add(count).ok_or(Refusal::BadQuantity)?;
                if count == 0 || total > entry.stack_max {
                    return Err(Refusal::BadQuantity);
                }
                entry.quantity = total;
                Ok(vec![Change::Quantity { item: into, quantity: total }])
            }
            Op::Insert { entry } => {
                if self.items.contains_key(&entry.id) {
                    return Err(Refusal::DuplicateId);
                }
                if entry.quantity == 0 || entry.quantity > entry.stack_max.max(1) {
                    return Err(Refusal::BadQuantity);
                }
                let at = self.first_free_carrying().ok_or(Refusal::NoRoom)?;
                self.place_new(Entry { at: Some(at), ..entry });
                Ok(vec![Change::Inserted { item: entry.id, at }])
            }
        }
    }

    /// A move into any slot. Into the equipped bag it is an equip into exactly that slot.
    fn do_move(&mut self, item: ItemId, bag: BagId, slot: Slot, wearer: Wearer) -> Result<Vec<Change>, Refusal> {
        let entry = *self.items.get(&item).ok_or(Refusal::NoSuchItem)?;
        let from = entry.at.ok_or(Refusal::NotInASlot)?;
        if from == (bag, slot) {
            return Ok(Vec::new());
        }
        let info = *self.bags.get(&bag).ok_or(Refusal::NoSuchBag)?;
        if slot >= info.slots {
            return Err(Refusal::OutOfRange);
        }
        let from_kind = self.bags.get(&from.0).map(|bag| bag.kind).ok_or(Refusal::NoSuchBag)?;
        if !wearer.storage_open && (from_kind.is_storage() || info.kind.is_storage()) {
            return Err(Refusal::StorageClosed);
        }
        if info.kind == BagKind::Equipped {
            if entry.kind.equip_slot() != Some(slot) {
                return Err(if entry.kind.equip_slot().is_none() {
                    Refusal::NotEquippable
                } else {
                    Refusal::WrongSlot
                });
            }
            return self.do_equip(entry, bag, slot, wearer);
        }
        if !info.kind.accepts(entry.kind) {
            return Err(Refusal::WrongBag);
        }
        if self.slots.contains_key(&(bag, slot)) {
            return Err(Refusal::Occupied);
        }
        self.relocate(item, (bag, slot));
        let mut changes = vec![Change::Moved { item, from, to: (bag, slot) }];
        self.note_worn(from, &mut changes);
        Ok(changes)
    }

    /// Wear `entry` in equipped slot `slot` of bag `bag`.
    ///
    /// * The item must come from a bag the character carries (or already be worn there).
    /// * Armour made for a profession needs that primary profession.
    /// * Whatever is worn in that slot, and the other hand's piece when a two-handed weapon and an
    ///   off-hand cannot be worn together, go to free carrying slots - **never** the slot the incoming
    ///   item is leaving. The frames go out in change order, so the displaced piece is moved before the
    ///   incoming item leaves its slot; sending it into that slot put two items in one slot in the
    ///   client's model and crashed a live client (2026-11-09, equipping a hammer over the starter
    ///   weapon). Retail swaps in place even with full bags, but that answer has not been captured, so
    ///   with no other free slot the equip is refused.
    fn do_equip(&mut self, entry: Entry, bag: BagId, slot: Slot, wearer: Wearer) -> Result<Vec<Change>, Refusal> {
        let from = entry.at.ok_or(Refusal::NotInASlot)?;
        if from == (bag, slot) {
            return Ok(Vec::new());
        }
        let from_kind = self.bags.get(&from.0).map(|bag| bag.kind).ok_or(Refusal::NoSuchBag)?;
        if !from_kind.is_carried() {
            return Err(Refusal::NotCarried);
        }
        if let ItemKind::Armor(_) = entry.kind {
            // A wearer with no recorded profession (a character that predates the column) is not
            // held to it: refusing would leave it unable to put its own starting armour back on.
            if entry.profession != 0
                && wearer.primary_profession != 0
                && entry.profession != wearer.primary_profession
            {
                return Err(Refusal::WrongProfession);
            }
        }

        // The piece the other hand cannot keep.
        let conflicting = match entry.kind {
            ItemKind::Weapon(Hands::Two) => self.at(bag, equip_slot::OFF_HAND).copied(),
            ItemKind::OffHand => self
                .at(bag, equip_slot::WEAPON)
                .filter(|weapon| weapon.kind == ItemKind::Weapon(Hands::Two))
                .copied(),
            _ => None,
        };
        let occupant = self.at(bag, slot).copied();

        let mut changes = Vec::new();
        let mut touched: Vec<Slot> = Vec::new();

        // Free the target slot, then the other hand, each into a free carrying slot that is not the one
        // the incoming item is still in on the client.
        if let Some(occupant) = occupant {
            let to = self.first_free_carrying_except(&[from]).ok_or(Refusal::NoRoom)?;
            self.relocate(occupant.id, to);
            changes.push(Change::Moved { item: occupant.id, from: (bag, slot), to });
        }
        if let Some(conflict) = conflicting {
            let to = self.first_free_carrying_except(&[from]).ok_or(Refusal::NoRoom)?;
            let conflict_from = conflict.at.ok_or(Refusal::NotInASlot)?;
            self.relocate(conflict.id, to);
            changes.push(Change::Moved { item: conflict.id, from: conflict_from, to });
            touched.push(conflict_from.1);
        }
        self.relocate(entry.id, (bag, slot));
        changes.push(Change::Moved { item: entry.id, from, to: (bag, slot) });
        touched.push(slot);

        for slot in touched {
            changes.push(Change::Worn { slot, item: self.at(bag, slot).map(|entry| entry.id) });
        }
        Ok(changes)
    }

    fn do_take(&mut self, item: ItemId, count: u32) -> Result<Vec<Change>, Refusal> {
        let entry = *self.items.get(&item).ok_or(Refusal::NoSuchItem)?;
        let at = entry.at.ok_or(Refusal::NotInASlot)?;
        let kind = self.bags.get(&at.0).map(|bag| bag.kind).ok_or(Refusal::NoSuchBag)?;
        if kind != BagKind::Carrying {
            // `GmBag_FindItem` looks from the backpack to bag 2 and nowhere else; worn items and
            // storage are not something a drop names.
            return Err(Refusal::NotCarried);
        }
        if count == 0 || count > entry.quantity {
            return Err(Refusal::BadQuantity);
        }
        if count == entry.quantity {
            self.remove(item);
            Ok(vec![Change::Removed { item }])
        } else {
            let remaining = entry.quantity - count;
            if let Some(entry) = self.items.get_mut(&item) {
                entry.quantity = remaining;
            }
            Ok(vec![Change::Quantity { item, quantity: remaining }])
        }
    }

    /// After something left `at`, say so if `at` was a worn slot.
    fn note_worn(&self, at: (BagId, Slot), changes: &mut Vec<Change>) {
        if Some(at.0) == self.equipped_bag() {
            changes.push(Change::Worn { slot: at.1, item: self.at(at.0, at.1).map(|entry| entry.id) });
        }
    }

    fn relocate_out(&mut self, item: ItemId) {
        if let Some(entry) = self.items.get_mut(&item) {
            if let Some(at) = entry.at.take() {
                self.slots.remove(&at);
            }
        }
    }

    fn relocate(&mut self, item: ItemId, to: (BagId, Slot)) {
        self.relocate_out(item);
        if let Some(entry) = self.items.get_mut(&item) {
            entry.at = Some(to);
            self.slots.insert(to, item);
        }
    }

    fn remove(&mut self, item: ItemId) {
        self.relocate_out(item);
        self.items.remove(&item);
    }

    fn place_new(&mut self, entry: Entry) {
        if let Some(at) = entry.at {
            self.slots.insert(at, entry.id);
        }
        self.items.insert(entry.id, entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BACKPACK: BagId = 1;
    const MATERIALS: BagId = 2;
    const STORAGE: BagId = 3;
    const EQUIPPED: BagId = 5;
    const UNCLAIMED: BagId = 6;
    const WARRIOR: Wearer = Wearer { primary_profession: 1, storage_open: true };

    /// The reference's default set: backpack (20), material storage, unclaimed, one storage pane, the
    /// equipped bag, two cupcake stacks and a warrior in his starting gear with a hammer.
    fn reference() -> InvModel {
        let bags = [
            BagInfo { id: BACKPACK, model: 0, kind: BagKind::of(0), slots: 20 },
            BagInfo { id: MATERIALS, model: 5, kind: BagKind::of(5), slots: 41 },
            BagInfo { id: UNCLAIMED, model: 6, kind: BagKind::of(6), slots: 12 },
            BagInfo { id: STORAGE, model: 7, kind: BagKind::of(7), slots: 25 },
            BagInfo { id: EQUIPPED, model: 21, kind: BagKind::of(21), slots: 9 },
        ];
        let items = [
            entry(1, None, 3, 0, 1),                          // the backpack's own item
            stack(2, (BACKPACK, 0), 100),                     // cupcakes
            stack(9, (BACKPACK, 1), 100),                     // cupcakes
            entry(3, Some((EQUIPPED, 0)), 15, 1, 1),          // hammer
            entry(4, Some((EQUIPPED, 2)), 7, 1, 1),           // body
            entry(5, Some((EQUIPPED, 3)), 19, 1, 1),          // legs
            entry(6, Some((EQUIPPED, 4)), 16, 1, 1),          // head
            entry(7, Some((EQUIPPED, 5)), 4, 1, 1),           // boots
            entry(8, Some((EQUIPPED, 6)), 13, 1, 1),          // gloves
        ];
        InvModel::new(bags, items).expect("the reference inventory is valid")
    }

    fn entry(id: ItemId, at: Option<(BagId, Slot)>, item_type: u8, profession: u8, quantity: u32) -> Entry {
        Entry { id, at, kind: ItemKind::of(item_type), profession, quantity, stack_max: 1 }
    }

    fn stack(id: ItemId, at: (BagId, Slot), quantity: u32) -> Entry {
        Entry { id, at: Some(at), kind: ItemKind::Other, profession: 0, quantity, stack_max: 250 }
    }

    fn add(model: &mut InvModel, entry: Entry) {
        model.place_new(entry);
    }

    fn fill_backpack(model: &mut InvModel) {
        let mut id = 1000;
        while let Some(at) = model.first_free_carrying() {
            add(model, stack(id, at, 1));
            id += 1;
        }
    }

    #[test]
    fn kinds_agree_with_the_protocol_slot_table() {
        // `asura_protocol::game::equipped_slot`, copied: the model must answer the same.
        let table: &[(u8, Option<Slot>)] = &[
            (1, Some(0)), (2, Some(0)), (5, Some(0)), (15, Some(0)), (22, Some(0)), (26, Some(0)),
            (27, Some(0)), (32, Some(0)), (35, Some(0)), (36, Some(0)), (12, Some(1)), (24, Some(1)),
            (7, Some(2)), (19, Some(3)), (16, Some(4)), (4, Some(5)), (13, Some(6)), (44, Some(7)),
            (45, Some(8)), (3, None), (9, None), (11, None), (30, None),
        ];
        for (item_type, slot) in table {
            assert_eq!(ItemKind::of(*item_type).equip_slot(), *slot, "item type {item_type}");
        }
    }

    #[test]
    fn a_corrupt_inventory_is_refused_when_it_is_built() {
        let bags = [BagInfo { id: 1, model: 0, kind: BagKind::Carrying, slots: 2 }];
        assert!(InvModel::new(bags, [stack(1, (1, 0), 1), stack(2, (1, 0), 1)]).is_err(), "two in one slot");
        assert!(InvModel::new(bags, [stack(1, (1, 5), 1)]).is_err(), "past the bag's end");
        assert!(InvModel::new(bags, [stack(1, (9, 0), 1)]).is_err(), "no such bag");
    }

    #[test]
    fn a_move_to_a_free_slot_is_one_change() {
        let mut model = reference();
        let changes = model.apply(Op::Move { item: 2, bag: BACKPACK, slot: 5 }, WARRIOR).unwrap();
        assert_eq!(changes, vec![Change::Moved { item: 2, from: (BACKPACK, 0), to: (BACKPACK, 5) }]);
        assert_eq!(model.at(BACKPACK, 5).map(|e| e.id), Some(2));
        assert!(model.at(BACKPACK, 0).is_none());
    }

    #[test]
    fn a_move_onto_an_occupied_slot_is_refused_and_changes_nothing() {
        let mut model = reference();
        let before = model.clone();
        assert_eq!(model.apply(Op::Move { item: 2, bag: BACKPACK, slot: 1 }, WARRIOR), Err(Refusal::Occupied));
        assert_eq!(model, before);
    }

    #[test]
    fn a_move_to_the_same_place_changes_nothing() {
        let mut model = reference();
        assert_eq!(model.apply(Op::Move { item: 2, bag: BACKPACK, slot: 0 }, WARRIOR), Ok(vec![]));
    }

    #[test]
    fn the_impossible_moves_are_refused() {
        let mut model = reference();
        assert_eq!(model.apply(Op::Move { item: 77, bag: BACKPACK, slot: 3 }, WARRIOR), Err(Refusal::NoSuchItem));
        assert_eq!(model.apply(Op::Move { item: 1, bag: BACKPACK, slot: 3 }, WARRIOR), Err(Refusal::NotInASlot));
        assert_eq!(model.apply(Op::Move { item: 2, bag: 40, slot: 3 }, WARRIOR), Err(Refusal::NoSuchBag));
        assert_eq!(model.apply(Op::Move { item: 2, bag: BACKPACK, slot: 20 }, WARRIOR), Err(Refusal::OutOfRange));
    }

    #[test]
    fn a_cupcake_cannot_be_worn_as_a_helmet() {
        let mut model = reference();
        assert_eq!(
            model.apply(Op::Move { item: 2, bag: EQUIPPED, slot: equip_slot::HEAD }, WARRIOR),
            Err(Refusal::NotEquippable)
        );
        assert_eq!(model.apply(Op::Equip { item: 2 }, WARRIOR), Err(Refusal::NotEquippable));
    }

    #[test]
    fn bags_only_take_what_they_are_for() {
        let mut model = reference();
        assert_eq!(model.apply(Op::Move { item: 2, bag: MATERIALS, slot: 0 }, WARRIOR), Err(Refusal::WrongBag));
        assert_eq!(model.apply(Op::Move { item: 2, bag: UNCLAIMED, slot: 0 }, WARRIOR), Err(Refusal::WrongBag));
        add(&mut model, Entry { kind: ItemKind::Material, ..stack(50, (BACKPACK, 7), 10) });
        assert!(model.apply(Op::Move { item: 50, bag: MATERIALS, slot: 0 }, WARRIOR).is_ok());
        assert!(model.apply(Op::Move { item: 2, bag: STORAGE, slot: 0 }, WARRIOR).is_ok());
    }

    #[test]
    fn storage_is_reached_only_through_a_storage_window() {
        let closed = Wearer { storage_open: false, ..WARRIOR };
        let mut model = reference();
        assert_eq!(
            model.apply(Op::Move { item: 2, bag: STORAGE, slot: 0 }, closed),
            Err(Refusal::StorageClosed)
        );
        assert!(model.apply(Op::Move { item: 2, bag: STORAGE, slot: 0 }, WARRIOR).is_ok());
        assert_eq!(
            model.apply(Op::Move { item: 2, bag: BACKPACK, slot: 5 }, closed),
            Err(Refusal::StorageClosed),
            "out of storage too"
        );
        assert!(model.apply(Op::Move { item: 2, bag: BACKPACK, slot: 5 }, WARRIOR).is_ok());
        // Moves between carried bags do not need the window.
        assert!(model.apply(Op::Move { item: 2, bag: BACKPACK, slot: 6 }, closed).is_ok());
    }

    #[test]
    fn dragging_a_worn_piece_out_says_the_slot_is_empty() {
        let mut model = reference();
        let changes = model.apply(Op::Move { item: 3, bag: BACKPACK, slot: 4 }, WARRIOR).unwrap();
        assert_eq!(
            changes,
            vec![
                Change::Moved { item: 3, from: (EQUIPPED, 0), to: (BACKPACK, 4) },
                Change::Worn { slot: equip_slot::WEAPON, item: None },
            ]
        );
    }

    #[test]
    fn unequip_and_equip_round_trip() {
        let mut model = reference();
        let changes = model
            .apply(Op::Unequip { equipped: equip_slot::BODY, bag: BACKPACK, slot: 2 }, WARRIOR)
            .unwrap();
        assert_eq!(
            changes,
            vec![
                Change::Moved { item: 4, from: (EQUIPPED, 2), to: (BACKPACK, 2) },
                Change::Worn { slot: equip_slot::BODY, item: None },
            ]
        );
        let changes = model.apply(Op::Equip { item: 4 }, WARRIOR).unwrap();
        assert_eq!(
            changes,
            vec![
                Change::Moved { item: 4, from: (BACKPACK, 2), to: (EQUIPPED, 2) },
                Change::Worn { slot: equip_slot::BODY, item: Some(4) },
            ]
        );
        assert_eq!(model.apply(Op::Unequip { equipped: 7, bag: BACKPACK, slot: 9 }, WARRIOR), Err(Refusal::NothingWorn));
    }

    #[test]
    fn the_displaced_piece_never_goes_into_the_slot_the_new_one_is_leaving() {
        // The live crash, 2026-11-09: a hammer in backpack slot 6 with slots 0-5 taken, equipped over
        // the worn weapon. The old weapon was sent to slot 6 before the hammer left it.
        let mut model = reference();
        for slot in 2..6 {
            add(&mut model, stack(100 + u32::from(slot), (BACKPACK, slot), 1));
        }
        add(&mut model, entry(20, Some((BACKPACK, 6)), 15, 0, 1));
        assert_eq!(model.first_free_carrying(), Some((BACKPACK, 7)));

        let changes = model.apply(Op::Equip { item: 20 }, WARRIOR).unwrap();
        assert_eq!(
            changes,
            vec![
                Change::Moved { item: 3, from: (EQUIPPED, 0), to: (BACKPACK, 7) },
                Change::Moved { item: 20, from: (BACKPACK, 6), to: (EQUIPPED, 0) },
                Change::Worn { slot: equip_slot::WEAPON, item: Some(20) },
            ]
        );
        // In change order, no move lands on a slot that is still occupied at that moment.
        let mut occupied: Vec<(BagId, Slot)> = reference_slots(&reference());
        occupied.extend((2..7).map(|slot| (BACKPACK, slot)));
        for change in &changes {
            if let Change::Moved { from, to, .. } = *change {
                assert!(!occupied.contains(&to), "{to:?} is still occupied when the move arrives");
                occupied.retain(|at| *at != from);
                occupied.push(to);
            }
        }
    }

    fn reference_slots(model: &InvModel) -> Vec<(BagId, Slot)> {
        model.slots.keys().copied().collect()
    }

    #[test]
    fn a_swap_with_no_other_free_slot_is_refused_until_the_swap_answer_is_captured() {
        let mut model = reference();
        add(&mut model, entry(20, Some((BACKPACK, 2)), 7, 1, 1)); // a second body piece
        fill_backpack(&mut model);
        let before = model.clone();
        assert_eq!(model.apply(Op::Equip { item: 20 }, WARRIOR), Err(Refusal::NoRoom));
        assert_eq!(model, before);
    }

    #[test]
    fn another_professions_armour_cannot_be_worn() {
        let mut model = reference();
        add(&mut model, entry(21, Some((BACKPACK, 2)), 16, 3, 1)); // a monk's head piece
        let before = model.clone();
        assert_eq!(model.apply(Op::Equip { item: 21 }, WARRIOR), Err(Refusal::WrongProfession));
        assert_eq!(model, before, "nothing came off either");
        // A character with no recorded profession is not held to it.
        assert!(model.clone().apply(Op::Equip { item: 21 }, Wearer { primary_profession: 0, storage_open: true }).is_ok());
        // Weapons are not profession-locked.
        add(&mut model, entry(22, Some((BACKPACK, 3)), 27, 3, 1));
        assert!(model.apply(Op::Equip { item: 22 }, WARRIOR).is_ok());
    }

    #[test]
    fn a_shield_takes_a_two_handed_weapon_off_and_a_bow_takes_the_shield_off() {
        let mut model = reference();
        add(&mut model, entry(30, Some((BACKPACK, 2)), 24, 0, 1)); // shield
        add(&mut model, entry(31, Some((BACKPACK, 3)), 5, 0, 1)); // bow

        // The hammer is two-handed: the shield takes it off, into the first free carrying slot that is
        // not the one the shield is leaving.
        let changes = model.apply(Op::Equip { item: 30 }, WARRIOR).unwrap();
        assert_eq!(
            changes,
            vec![
                Change::Moved { item: 3, from: (EQUIPPED, 0), to: (BACKPACK, 4) },
                Change::Moved { item: 30, from: (BACKPACK, 2), to: (EQUIPPED, 1) },
                Change::Worn { slot: equip_slot::WEAPON, item: None },
                Change::Worn { slot: equip_slot::OFF_HAND, item: Some(30) },
            ]
        );

        // The bow is two-handed: it takes the weapon slot and the shield comes off.
        let changes = model.apply(Op::Equip { item: 31 }, WARRIOR).unwrap();
        assert_eq!(
            changes,
            vec![
                Change::Moved { item: 30, from: (EQUIPPED, 1), to: (BACKPACK, 2) },
                Change::Moved { item: 31, from: (BACKPACK, 3), to: (EQUIPPED, 0) },
                Change::Worn { slot: equip_slot::OFF_HAND, item: None },
                Change::Worn { slot: equip_slot::WEAPON, item: Some(31) },
            ]
        );
        assert!(model.worn(equip_slot::OFF_HAND).is_none());
    }

    #[test]
    fn a_two_handed_swap_that_needs_room_and_has_none_is_refused_whole() {
        let mut model = reference();
        add(&mut model, entry(30, Some((EQUIPPED, 1)), 24, 0, 1)); // shield worn with the hammer (legacy)
        add(&mut model, entry(31, Some((BACKPACK, 2)), 5, 0, 1)); // bow
        fill_backpack(&mut model);
        let before = model.clone();
        // Neither the hammer nor the shield has anywhere to go but the bow's slot.
        assert_eq!(model.apply(Op::Equip { item: 31 }, WARRIOR), Err(Refusal::NoRoom));
        assert_eq!(model, before, "the hammer did not come off");
    }

    #[test]
    fn nothing_is_worn_straight_out_of_storage() {
        let mut model = reference();
        add(&mut model, entry(40, Some((STORAGE, 0)), 7, 1, 1));
        assert_eq!(model.apply(Op::Equip { item: 40 }, WARRIOR), Err(Refusal::NotCarried));
    }

    #[test]
    fn a_take_leaves_the_rest_or_removes_the_row() {
        let mut model = reference();
        assert_eq!(model.apply(Op::Take { item: 2, count: 1 }, WARRIOR), Ok(vec![Change::Quantity { item: 2, quantity: 99 }]));
        assert_eq!(model.apply(Op::Take { item: 2, count: 99 }, WARRIOR), Ok(vec![Change::Removed { item: 2 }]));
        assert!(model.item(2).is_none());
        assert!(model.at(BACKPACK, 0).is_none());
    }

    #[test]
    fn a_take_refuses_bad_counts_and_places_a_drop_cannot_come_from() {
        let mut model = reference();
        assert_eq!(model.apply(Op::Take { item: 9, count: 0 }, WARRIOR), Err(Refusal::BadQuantity));
        assert_eq!(model.apply(Op::Take { item: 9, count: 101 }, WARRIOR), Err(Refusal::BadQuantity));
        assert_eq!(model.apply(Op::Take { item: 3, count: 1 }, WARRIOR), Err(Refusal::NotCarried), "worn");
        assert_eq!(model.apply(Op::Take { item: 1, count: 1 }, WARRIOR), Err(Refusal::NotInASlot), "a bag");
    }

    #[test]
    fn a_merge_respects_the_stack_limit() {
        let mut model = reference();
        assert_eq!(model.apply(Op::Merge { into: 2, count: 150 }, WARRIOR), Ok(vec![Change::Quantity { item: 2, quantity: 250 }]));
        assert_eq!(model.apply(Op::Merge { into: 2, count: 1 }, WARRIOR), Err(Refusal::BadQuantity));
        assert_eq!(model.apply(Op::Merge { into: 3, count: 1 }, WARRIOR), Err(Refusal::BadQuantity), "a hammer does not stack");
    }

    #[test]
    fn an_insert_goes_to_the_first_free_carrying_slot() {
        let mut model = reference();
        let new = stack(500, (0, 0), 3);
        assert_eq!(model.apply(Op::Insert { entry: new }, WARRIOR), Ok(vec![Change::Inserted { item: 500, at: (BACKPACK, 2) }]));
        assert_eq!(model.apply(Op::Insert { entry: new }, WARRIOR), Err(Refusal::DuplicateId));
        assert_eq!(model.apply(Op::Insert { entry: stack(501, (0, 0), 251) }, WARRIOR), Err(Refusal::BadQuantity));
        fill_backpack(&mut model);
        assert_eq!(model.apply(Op::Insert { entry: stack(502, (0, 0), 1) }, WARRIOR), Err(Refusal::NoRoom));
    }

    #[test]
    fn destroying_a_worn_piece_empties_its_slot_and_a_bag_cannot_be_destroyed() {
        let mut model = reference();
        assert_eq!(
            model.apply(Op::Destroy { item: 6 }, WARRIOR),
            Ok(vec![Change::Removed { item: 6 }, Change::Worn { slot: equip_slot::HEAD, item: None }])
        );
        assert_eq!(model.apply(Op::Destroy { item: 1 }, WARRIOR), Err(Refusal::NotInASlot));
    }
}
