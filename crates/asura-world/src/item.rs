//! The item and the inventory: what a character carries, and what a client may do with it.
//!
//! [`items_block`] and [`inventory_block`] are the load-time blocks. The client operations below -
//! `DROP_ITEM`, `INTERACT_ITEM`, `ITEM_MOVE`, `EQUIP_ITEM`, `UNEQUIP_ITEM`, `ITEM_DESTROY` and the `/item`
//! test command - all run the same way: the rules in [`crate::inventory_model`], one transaction in
//! `asura_db::inventory::apply_changes`, then frames for this client and the rest of the instance
//! (`docs/50-item-framework.md`).
//!
//! [`move_item`] and [`equip_item`] are messages the reference does not dispatch at all - their
//! shapes are measured, not ported (`docs/HANDOFF.md`, M5).

use crate::inventory_model::{
    BagInfo, BagKind, Change, Entry, InvModel, ItemId, ItemKind, Op, Refusal, Wearer,
};
use crate::*;

/// The answer to the client's item request (`GameSrv_HandleInstanceLoadRequestItems`).
///
/// A pure function of the map and the character's inventory so that `examples/dump-load.rs` can print
/// exactly what a client would receive, byte for byte, without a socket - which is how this stream is
/// compared against the reference's own log.
pub fn items_block(
    map: &MapEntry,
    inventory: &db_inventory::Inventory,
    purse: impl Into<Purse>,
) -> Vec<Frame> {
    items_block_with_stream(map, inventory, purse, true)
}

/// The character's gold: what it carries, and what its storage holds (`crate::vendor`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Purse {
    /// The gold the character carries.
    pub carried: u32,
    /// The gold in its storage.
    pub stored: u32,
}

impl From<u32> for Purse {
    /// Carried gold alone, as the load block's callers that know no storage pass it.
    fn from(carried: u32) -> Purse {
        Purse { carried, stored: 0 }
    }
}

/// The item answer, with the stream create left out when this connection has already opened stream 1.
///
/// The client registers one inventory per stream id and asserts `!inventory` on a second
/// `0x0144` (`ItCliApi.cpp(2010)`, build 39167, measured 2026-11-09: the client asked `0x8091`
/// twice on one connection and the second full answer disconnected it). Everything after the
/// create is safe to repeat: `0x013F` replaces a bag whose id it already has (`ItCliBag.cpp(167)`),
/// and the items are then placed into the fresh bag. The first answer is the reference's own.
pub fn items_block_with_stream(
    map: &MapEntry,
    inventory: &db_inventory::Inventory,
    purse: impl Into<Purse>,
    open_stream: bool,
) -> Vec<Frame> {
    let purse = purse.into();
    let mut frames = Vec::new();
    if open_stream {
        frames.push(game::item_stream_create(ITEM_STREAM, 0));
    }
    frames.push(game::item_set_active_weapon_set(ITEM_STREAM, 0));
    // The inventory itself, from the character's rows. It is the reference's own walk: for every bag, the
    // item that *is* the bag, the bag, and then its items with their placements.
    frames.extend(inventory_block(inventory));
    // Four empty weapon slots, exactly as `GameSrv_SendWeaponSlots` sends them.
    for slot in 0..4u8 {
        frames.push(game::item_weapon_set(ITEM_STREAM, slot, 0, 0));
    }
    // The gold, as the live server sends it: the carried gold (`0x0140`) and then the stored gold
    // (`0x0141`), each only when it is not 0, after the weapon sets (preforged
    // `packets-43292-168891734.jsonl` records 472025-472026, `{stream, 6}` and `{stream, 1000000}`; a load
    // with neither, `recordings/client/feature/maps/pre-searing-load-3.jsonl`). Both are amounts the
    // client adds to the balances its fresh inventory starts at, so a repeated answer on a stream that is
    // already open sends neither: it would count the gold twice. The reference sends the character's
    // gold as `0x0141`, which the client shows as storage gold (`docs/53-npc-services.md`).
    if open_stream {
        if purse.carried != 0 {
            frames.push(game::vendor::gold_character_add(ITEM_STREAM, purse.carried));
        }
        if purse.stored != 0 {
            frames.push(game::vendor::gold_storage_add(ITEM_STREAM, purse.stored));
        }
    }
    for faction in [
        game::FACTION_MAX_KURZICK,
        game::FACTION_MAX_LUXON,
        game::FACTION_MAX_BALTHAZAR,
        game::FACTION_MAX_IMPERIAL,
    ] {
        // 10000, which is what the reference sends (`10 27 00 00`).
        frames.push(game::faction_max(faction, 10_000));
    }
    // No `GAME_SMSG_HARD_MODE_UNLOCKED` (`0x0070`). The reference sends `{1}` here, but no live load in
    // `recordings/client/feature/` carries the message at all (eleven loads of a pre-Searing character,
    // `multiplay/party interface check.jsonl` among them), and with it the client drew a normal / hard
    // mode toggle in the party window of pre-Searing Ascalon that the live game does not draw (owner,
    // 2026-11-06). A character that has unlocked hard mode is a later layer's question.
    // The reference sends an **empty** name here (`GameSrv_SendCharacterInfo`), not the character's:
    // the client already has the name from the load block.
    frames.push(game::character_update_info(""));
    frames.push(game::map_update_current(map.map_id as u16));
    frames.push(game::ready_for_map_spawn());
    frames.extend(manifest(map.map_id as u16));
    frames
}

/// The item stream's id. The reference hardcodes `1` in every message that carries one
/// (`GameSrv_SendInventory`, `GameSrv_SendBagItems`), and a second stream is a thing it never opens.
pub const ITEM_STREAM: u16 = 1;

/// The inventory itself (`GameSrv_SendInventory`), built from rows instead of replayed bytes.
///
/// The order is the reference's and it matters twice over:
///
/// * **bags, by model id** - its `GmBagArray` is walked in declaration order, which is backpack, belt
///   pouch, bags, equipment pack, material storage, unclaimed items, storage, equipped items. That is
///   not the order the bag ids were handed out in (the capture's bags are 1, 2, 6, 3, 4, 5), so a server
///   that sorts by id sends a different stream;
/// * **a bag's own item before the bag** - `GameSrv_SendInventory` sends `bag_item_id` first, because the
///   bag message names it.
///
/// Within a bag, items go out by slot, each followed by where it sits and, if it was made for a
/// profession, what profession (`GameSrv_SendBagItems`). A missing row for a bag's own item is logged and
/// skipped rather than fatal, exactly as the reference's `GameSrv_SendItemById` logs and returns.
pub fn inventory_block(inventory: &db_inventory::Inventory) -> Vec<Frame> {
    let mut frames = Vec::with_capacity(inventory.bags.len() * 2 + inventory.items.len() * 3);
    for bag in &inventory.bags {
        if bag.assoc_item_id != 0 {
            match inventory.item(bag.assoc_item_id) {
                Some(item) => frames.push(game::create_named_item(&named_item(item))),
                None => eprintln!(
                    "world: bag {} names item {} as its own, and no such item exists",
                    bag.bag_id, bag.assoc_item_id
                ),
            }
        }

        frames.push(game::inventory_create_bag(
            ITEM_STREAM,
            wire_u8(bag.bag_type),
            wire_u8(bag.bag_model_id),
            wire_u16(bag.bag_id),
            wire_u8(bag.slot_count),
            wire_u32(bag.assoc_item_id),
        ));

        for item in inventory.items_in(bag.bag_id) {
            frames.push(game::create_named_item(&named_item(item)));
            if item.profession != 0 {
                frames.push(game::item_set_profession(
                    wire_u32(item.id),
                    wire_u8(item.profession),
                ));
            }
            frames.push(game::item_move_to_location(
                ITEM_STREAM,
                wire_u32(item.id),
                wire_u16(bag.bag_id),
                wire_u8(item.slot.unwrap_or(0)),
            ));
        }
    }
    frames
}

/// A row as `CREATE_NAMED_ITEM` carries it.
pub fn named_item(item: &db_inventory::Item) -> game::NamedItem {
    game::NamedItem {
        id: wire_u32(item.id),
        file_id: u32::try_from(item.file_id).unwrap_or(0),
        item_type: wire_u8(item.item_type),
        dye_tint: wire_u8(item.dye_tint),
        dye_colors: u16::try_from(item.dye_colors).unwrap_or(0),
        materials: u16::try_from(item.materials).unwrap_or(0),
        unk1: wire_u8(item.unk1),
        flags: u32::try_from(item.flags).unwrap_or(0),
        value: wire_u32(item.value),
        model_id: wire_u32(item.model_id),
        quantity: wire_u32(item.quantity),
        name: item.name_units(),
        modifiers: item.modifier_dwords(),
    }
}

/// The four coded-string words the live server puts before an item's own name units on the drop line
/// (`items-live-20261108-173913.jsonl` seq 44: `F2 07 B4 AC 04 A7 4B 31`).
const DROP_LINE: [u16; 4] = [0x07F2, 0xACB4, 0xA704, 0x314B];
/// And on the pickup line (`items-live-20261108-173913.jsonl` seq 58: `FC 07 59 C5 E7 A2 E3 05`).
const PICKUP_LINE: [u16; 4] = [0x07FC, 0x59C5, 0xE7A2, 0xE305];

/// The chat line the live server commits on a drop or a pickup: a fixed four-word message prefix and
/// the item's own coded name, then the server tag on `Channel_Global`
/// (`items-live-20261108-173913.jsonl` seq 44-45 and 58-59).
fn item_line(prefix: [u16; 4], item: &db_inventory::Item, player: &Player) -> Vec<Frame> {
    let mut units = prefix.to_vec();
    units.extend(item.name_units());
    units.truncate(122);
    vec![
        game::chat_message_core_units(&units),
        game::chat_message_server(player.player_id, game::CHANNEL_GLOBAL),
    ]
}

/// The item agent a ground item is drawn as. The agent's `model_id` is the item's **id**, not its
/// model: that is what makes the client look the item up in its own table
/// (`items-live-20261108-173913.jsonl` seq 43: agent 13, item 609).
pub fn ground_agent(ground: &crate::instance::GroundItem) -> Frame {
    game::item_agent(
        ground.agent_id,
        wire_u32(ground.item.id),
        ground.position,
        ground.plane,
        ground.direction,
    )
}


/// Every item that lies on the ground goes to the rest of the instance the way a client entering the
/// map is told about it (`player.rs`, the players block): the record the client draws - create for a
/// whole drop, update for the part of a stack - then its agent.
pub fn ground_frames(ground: &crate::instance::GroundItem) -> Vec<Frame> {
    let named = named_item(&ground.item);
    vec![
        if ground.source_item_id.is_some() {
            game::item_update(&named)
        } else {
            game::create_named_item(&named)
        },
        ground_agent(ground),
    ]
}

// ---- the operations ---------------------------------------------------------------------------
//
// Every client item operation runs the same way (`docs/50-item-framework.md` §3):
//
// 1. build the rules model from the character's rows ([`model_of`]);
// 2. `InvModel::apply` - the rules, all in `inventory_model`, atomic;
// 3. one transaction for the whole operation (`db_inventory::apply_changes`);
// 4. a fresh read, so `player.inventory` is what the database holds;
// 5. frames for this client and, for what others can see, a broadcast to the rest of the instance.
//
// A refusal changes nothing and, for an item the client already moved in its own model, puts it back.

/// The stack limit Guild Wars uses for every stackable item (Guild Wars Wiki, `Item stacking`). Used
/// only for a row whose definition the pack does not name and that is already a stack.
pub const STACK_LIMIT: u32 = 250;

/// What one operation sends.
#[derive(Default)]
struct Outgoing {
    /// To this client.
    own: Vec<Frame>,
    /// To everyone else in the instance who has entered it.
    others: Vec<Frame>,
}

/// The character's rows as the rules model. A row that cannot be a wire value (a negative id, a bag id
/// past `u16`, a slot past `u8`) or two rows in one slot is a broken inventory; it is reported, not
/// coerced to zero.
pub fn model_of(inventory: &db_inventory::Inventory) -> Result<InvModel, String> {
    let catalogue = item_catalogue::current().ok();
    let mut bags = Vec::with_capacity(inventory.bags.len());
    for bag in &inventory.bags {
        bags.push(BagInfo {
            id: u16::try_from(bag.bag_id)
                .map_err(|_| format!("bag id {} is not a wire bag id", bag.bag_id))?,
            model: bag.bag_model_id,
            kind: BagKind::of(bag.bag_model_id),
            slots: u8::try_from(bag.slot_count)
                .map_err(|_| format!("bag {} has {} slots", bag.bag_id, bag.slot_count))?,
        });
    }
    let mut items = Vec::with_capacity(inventory.items.len());
    for item in &inventory.items {
        items.push(entry_of(item, catalogue)?);
    }
    InvModel::new(bags, items)
}

/// One row as the rules see it. The stack limit comes from the row's definition; a row the pack does
/// not define is a stack of [`STACK_LIMIT`] when it already holds more than one, and a single item
/// otherwise.
pub fn entry_of(item: &db_inventory::Item, catalogue: Option<&Catalogue>) -> Result<Entry, String> {
    let bad = |what: &str| format!("item {} has {what}", item.id);
    let at = match (item.bag_id, item.slot) {
        (Some(bag), Some(slot)) => Some((
            u16::try_from(bag).map_err(|_| bad("a bag id past u16"))?,
            u8::try_from(slot).map_err(|_| bad("a slot past u8"))?,
        )),
        _ => None,
    };
    let quantity = u32::try_from(item.quantity).map_err(|_| bad("a negative quantity"))?;
    let from_definition = catalogue
        .and_then(|catalogue| catalogue.for_item(item))
        .and_then(|definition| definition.stack_limit());
    let stack_max = from_definition.unwrap_or(if quantity > 1 { STACK_LIMIT } else { 1 });
    Ok(Entry {
        id: u32::try_from(item.id).map_err(|_| bad("a negative id"))?,
        at,
        kind: ItemKind::of(u8::try_from(item.item_type).map_err(|_| bad("an item type past u8"))?),
        profession: u8::try_from(item.profession).map_err(|_| bad("a profession past u8"))?,
        quantity,
        stack_max,
    })
}

/// The row writes one operation's changes amount to. `new_rows` carries the full row of anything an
/// `Op::Insert` placed; the model only knows the fields its rules read.
fn row_changes(changes: &[Change], new_rows: &[db_inventory::Item]) -> Vec<db_inventory::RowChange> {
    let row_id = |item: ItemId| i32::try_from(item).unwrap_or(i32::MAX);
    changes
        .iter()
        .filter_map(|change| match *change {
            Change::Moved { item, to, .. } => Some(db_inventory::RowChange::Place {
                item_id: row_id(item),
                bag_id: i32::from(to.0),
                slot: i32::from(to.1),
            }),
            Change::Quantity { item, quantity } => Some(db_inventory::RowChange::Quantity {
                item_id: row_id(item),
                quantity: i32::try_from(quantity).unwrap_or(i32::MAX),
            }),
            Change::Removed { item } => Some(db_inventory::RowChange::Delete {
                item_id: row_id(item),
            }),
            Change::Inserted { item, at } => new_rows
                .iter()
                .find(|row| row.id == row_id(item))
                .map(|row| db_inventory::RowChange::Insert {
                    item: db_inventory::Item {
                        bag_id: Some(i32::from(at.0)),
                        slot: Some(i32::from(at.1)),
                        ..row.clone()
                    },
                }),
            Change::Worn { .. } => None,
        })
        .collect()
}

/// Steps 1-4: the rules, the transaction, the fresh read. The outer `Result` is a failure of the
/// server (the database); the inner one is the rules' answer.
async fn commit(
    world: &World,
    player: &mut Player,
    op: Op,
    new_rows: &[db_inventory::Item],
) -> Result<Result<Vec<Change>, Refusal>, GameError> {
    commit_ops(world, player, &[op], new_rows, Vec::new()).await
}

/// [`commit`] for several operations that stand or fall together, with row writes of the caller's
/// beside them in the same transaction: a purchase's insert and its gold, a collector's trophies and
/// its reward (`crate::vendor`). The operations apply in order on one model; a refusal of any leaves
/// everything as it was.
pub(crate) async fn commit_ops(
    world: &World,
    player: &mut Player,
    ops: &[Op],
    new_rows: &[db_inventory::Item],
    extra: Vec<db_inventory::RowChange>,
) -> Result<Result<Vec<Change>, Refusal>, GameError> {
    let mut model = match model_of(&player.inventory) {
        Ok(model) => model,
        Err(err) => {
            eprintln!(
                "world: {:?}'s inventory does not hold together ({err}); {ops:?} refused",
                player.name
            );
            return Ok(Err(Refusal::NoSuchItem));
        }
    };
    let wearer = Wearer {
        primary_profession: player.traits.profession,
        storage_open: crate::vendor::storage_open(player),
    };
    let mut changes = Vec::new();
    for op in ops {
        match model.apply(*op, wearer) {
            Ok(more) => changes.extend(more),
            Err(refusal) => return Ok(Err(refusal)),
        }
    }
    if changes.is_empty() && extra.is_empty() {
        return Ok(Ok(changes));
    }
    let mut rows = row_changes(&changes, new_rows);
    rows.extend(extra);
    let written = db_inventory::apply_changes(&world.pool, player.character, &rows).await;
    // Read back either way: after a failed write the cache must be the database again, not the model.
    reload(world, player).await?;
    written.map_err(GameError::Database)?;
    Ok(Ok(changes))
}

/// Step 5 for the changes that place things: where items now sit, what stacks hold, what is gone, and
/// what is worn. A worn change goes to the rest of the instance too, as the single visual slot
/// (`0x006F`), so others see the character change clothes. `own_visual` is false where this client's
/// own answer is the bulk visual message instead (see [`move_item`]).
fn placement_frames(changes: &[Change], agent_id: u32, own_visual: bool) -> Outgoing {
    let mut out = Outgoing::default();
    for change in changes {
        match *change {
            Change::Moved { item, to, .. } => {
                out.own
                    .push(game::item_change_location(ITEM_STREAM, item, to.0, to.1));
            }
            Change::Quantity { item, quantity } => {
                out.own.push(game::item_update_quantity(item, quantity));
            }
            Change::Removed { item } => out.own.push(game::item_remove(ITEM_STREAM, item)),
            Change::Worn { slot, item } => {
                let frame = game::update_agent_visual_equipment_slot(
                    agent_id,
                    game::visual_equipment_slot(slot),
                    item.unwrap_or(0),
                );
                if own_visual {
                    out.own.push(frame.clone());
                }
                out.others.push(frame);
            }
            // What a new item is announced with depends on why it appeared; its caller sends it.
            Change::Inserted { .. } => {}
        }
    }
    out
}

/// Send to this client, then to the rest of the instance.
async fn deliver(
    stream: &mut TcpStream,
    session: &mut Session,
    player: &Player,
    out: Outgoing,
    trace: bool,
) -> Result<(), GameError> {
    if !out.others.is_empty() {
        player.instance.broadcast(player.player_id, &out.others);
    }
    send(stream, session, out.own, trace).await
}

/// The client moved `item_id` in its own model before it asked; after a refusal, tell it where the item
/// really is. Nothing for an item the character no longer has or that is not in a slot.
fn snap_back(player: &Player, item_id: i32) -> Vec<Frame> {
    player
        .inventory
        .item(item_id)
        .and_then(|item| Some((item.id, item.bag_id?, item.slot?)))
        .map(|(id, bag, slot)| {
            game::item_change_location(ITEM_STREAM, wire_u32(id), wire_u16(bag), wire_u8(slot))
        })
        .into_iter()
        .collect()
}

/// Re-read the inventory after a change, so the next message built from it is the new truth.
pub async fn reload(world: &World, player: &mut Player) -> Result<(), GameError> {
    player.inventory = db_inventory::load(&world.pool, player.character)
        .await
        .map_err(GameError::Database)?;
    Ok(())
}

/// `GameSrv_HandleDropItem`: take the items off the stack and put them on the ground.
///
/// The live server answers two ways, and the capture shows both
/// (`items-live-20261108-173913.jsonl`, `items-live-20261108-174014.jsonl`):
///
/// * **a whole stack** - `ITEM_REMOVE {stream, item_id}`, then an item agent whose model is that same
///   id. The row is gone; the client kept the item's record and the agent names it.
/// * **part of a stack** - `ITEM_UPDATE_QUANTITY {item_id, remaining}`, then `ITEM_UPDATE` (`0x0162`)
///   for a ground record with a fresh id and the dropped quantity, then the item agent for that id.
///   The stack's row stays.
///
/// Both end with the drop line on `Channel_Global`. Everyone else in the instance is sent the ground
/// record and its agent, so the item is on their ground too. A pickup is [`interact_item`].
pub async fn drop_item(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    item_id: i32,
    count: i32,
    trace: bool,
) -> Result<(), GameError> {
    if item_id <= 0 || count <= 0 {
        eprintln!("world: a drop of {count} of item {item_id} names something that is not an item");
        return Ok(());
    }
    let Some(item) = player.inventory.item(item_id).cloned() else {
        eprintln!(
            "world: a drop names item {item_id}, which {0:?} does not have; ignored",
            player.name
        );
        return Ok(());
    };

    let op = Op::Take {
        item: wire_u32(item_id),
        count: wire_u32(count),
    };
    let changes = match commit(world, player, op, &[]).await? {
        Ok(changes) => changes,
        Err(refusal) => {
            eprintln!("world: a drop of {count} of item {item_id} refused: {refusal:?}");
            return Ok(());
        }
    };
    let remaining = changes
        .iter()
        .find_map(|change| match *change {
            Change::Quantity { quantity, .. } => Some(quantity),
            _ => None,
        })
        .unwrap_or(0);

    let agent_id = player.instance.allocate_agent_id();
    let standing = player.standing();
    let position = (
        standing.position.0 + DROP_OFFSET,
        standing.position.1 + DROP_OFFSET,
    );
    let mut out = Outgoing::default();
    let ground = if remaining == 0 {
        out.own.push(game::item_remove(ITEM_STREAM, wire_u32(item.id)));
        crate::instance::GroundItem {
            agent_id,
            item: db_inventory::Item {
                bag_id: None,
                slot: None,
                ..item.clone()
            },
            source_item_id: None,
            position,
            plane: standing.plane,
            direction: (1.0, 0.0),
        }
    } else {
        let ground_id = db_inventory::allocate_item_id(&world.pool)
            .await
            .map_err(GameError::Database)?;
        let ground_item = db_inventory::Item {
            id: ground_id,
            bag_id: None,
            slot: None,
            quantity: count,
            ..item.clone()
        };
        out.own.push(game::item_update_quantity(
            wire_u32(item.id),
            remaining,
        ));
        out.own.push(game::item_update(&named_item(&ground_item)));
        crate::instance::GroundItem {
            agent_id,
            item: ground_item,
            source_item_id: Some(item.id),
            position,
            plane: standing.plane,
            direction: (1.0, 0.0),
        }
    };
    player.instance.put_ground_item(ground.clone());
    out.own.push(ground_agent(&ground));
    out.own.extend(item_line(DROP_LINE, &ground.item, player));
    out.others.extend(ground_frames(&ground));
    eprintln!(
        "world: {} dropped {count} of item {item_id} at ({:.1}, {:.1}) as item {} agent {agent_id}; {remaining} left in the bag",
        player.name, position.0, position.1, ground.item.id
    );
    deliver(stream, session, player, out, trace).await
}

/// The client picks something up (`GAME_CMSG_INTERACT_ITEM`, `0x803F`: `{agent_id: u32, u8}`).
///
/// The live server answers with the action hold and its target, the pickup line, `ITEM_PICKED_UP`, and
/// then one of two bag updates (`items-live-20261108-173913.jsonl` seq 55-63, 156-164,
/// `items-live-20261108-174014.jsonl` seq 156-164):
///
/// * **part of a stack** - `ITEM_UPDATE_QUANTITY` says the stack it came from grew back; no new row.
///   Only when that stack is still this character's and has room under its limit.
/// * **otherwise** - `ITEM_MOVE_TO_LOCATION` puts the item, under its own id, in the first free
///   carrying slot, and the row is written.
///
/// Both end with `WORLD_REMOVE_AGENT` for the ground item, and the action hold cleared. The rest of the
/// instance is sent the `WORLD_REMOVE_AGENT` too. With no room, the item stays on the ground.
pub async fn interact_item(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    agent_id: u32,
    trace: bool,
) -> Result<(), GameError> {
    // Claimed before any await, so two players cannot both pick it up.
    let Some(ground) = player.instance.take_ground_item(agent_id) else {
        eprintln!("world: interact names agent {agent_id}, which is not a ground item; ignored");
        return Ok(());
    };
    match pick_up(world, player, &ground).await {
        Ok(Ok(changes)) => {
            let mut out = Outgoing::default();
            out.own.push(game::agent_property_update_int(
                game::AGENT_PROPERTY_ACTION,
                player.agent_id,
                1,
            ));
            out.own.push(game::agent_property_update_int(
                game::AGENT_PROPERTY_TARGET,
                player.agent_id,
                agent_id,
            ));
            out.own.extend(item_line(PICKUP_LINE, &ground.item, player));
            out.own.push(game::item_picked_up(
                wire_u32(ground.item.id),
                wire_u16(i32::try_from(player.agent_id).unwrap_or(0)),
            ));
            for change in &changes {
                match *change {
                    Change::Quantity { item, quantity } => {
                        out.own.push(game::item_update_quantity(item, quantity));
                    }
                    Change::Inserted { item, at } => {
                        out.own
                            .push(game::item_move_to_location(ITEM_STREAM, item, at.0, at.1));
                    }
                    _ => {}
                }
            }
            out.own.push(game::world_remove_agent(agent_id));
            out.others.push(game::world_remove_agent(agent_id));
            // The live server clears the hold about 0.8 s later; the client accepts it at once and the
            // item is already back, so this does not wait on the tick.
            out.own.push(game::agent_property_update_int(
                game::AGENT_PROPERTY_ACTION,
                player.agent_id,
                0,
            ));
            eprintln!(
                "world: {} picked up item {} from agent {agent_id}",
                player.name, ground.item.id
            );
            deliver(stream, session, player, out, trace).await
        }
        Ok(Err(refusal)) => {
            player.instance.put_ground_item(ground);
            eprintln!("world: a pickup of agent {agent_id} refused ({refusal:?}); it stays on the ground");
            Ok(())
        }
        Err(err) => {
            player.instance.put_ground_item(ground);
            Err(err)
        }
    }
}

/// The bag half of a pickup: back onto the stack it came from when that works, else a row of its own.
async fn pick_up(
    world: &World,
    player: &mut Player,
    ground: &crate::instance::GroundItem,
) -> Result<Result<Vec<Change>, Refusal>, GameError> {
    if let Some(source) = ground.source_item_id {
        if player.inventory.item(source).is_some() {
            let merge = Op::Merge {
                into: wire_u32(source),
                count: wire_u32(ground.item.quantity),
            };
            if let Ok(changes) = commit(world, player, merge, &[]).await? {
                return Ok(Ok(changes));
            }
        }
    }
    let entry = match entry_of(&ground.item, item_catalogue::current().ok()) {
        Ok(entry) => entry,
        Err(err) => {
            eprintln!("world: the ground item is not a valid row: {err}");
            return Ok(Err(Refusal::NoSuchItem));
        }
    };
    commit(
        world,
        player,
        Op::Insert { entry },
        std::slice::from_ref(&ground.item),
    )
    .await
}

/// `GAME_CMSG_ITEM_MOVE`: put an item in a slot, and tell the client where it ended up.
///
/// A move into the equipped-items bag is an equip into exactly that slot, with every equip rule. The
/// answer is `ITEM_CHANGE_LOCATION` for each item that moved; when what the character wears changed,
/// this client also gets the bulk `UPDATE_AGENT_VISUAL_EQUIPMENT` (`0x006E`) it has always been sent
/// for a drag (`tests/load.rs`, `an_item_can_be_taken_off_and_worn_again`), and the rest of the instance
/// gets the single slot. A drag onto an occupied carrying slot is refused (retail swaps or merges; that
/// answer has not been captured), and a refusal puts the client's copy back.
pub async fn move_item(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    placement: Placement,
    trace: bool,
) -> Result<(), GameError> {
    let Placement {
        item_id,
        bag_id,
        slot,
    } = placement;
    let op = Op::Move {
        item: wire_u32(item_id),
        bag: wire_u16(bag_id),
        slot: wire_u8(slot),
    };
    let out = match commit(world, player, op, &[]).await? {
        Ok(changes) => {
            eprintln!("world: item {item_id} to bag {bag_id} slot {slot}: {changes:?}");
            let mut out = placement_frames(&changes, player.agent_id, false);
            if changes.iter().any(|change| matches!(change, Change::Worn { .. })) {
                out.own.push(equipment_frame(player));
            }
            out
        }
        Err(refusal) => {
            eprintln!("world: item {item_id} to bag {bag_id} slot {slot} refused: {refusal:?}");
            Outgoing {
                own: snap_back(player, item_id),
                others: Vec::new(),
            }
        }
    };
    deliver(stream, session, player, out, trace).await
}

/// `GAME_CMSG_EQUIP_ITEM`: wear an item.
///
/// The rules are `inventory_model`'s: the item must be carried, armour must be the wearer's
/// profession, whatever was worn there goes to the slot the item left, and a two-handed weapon and an
/// off-hand push each other off. The answer is the move plus `UPDATE_AGENT_VISUAL_EQUIPMENT_SLOT`
/// (`0x006F`) - the live server's pair (`items-live-20261108-174014.jsonl` seq 31-32) - and the visual
/// slot goes to the rest of the instance as well.
pub async fn equip_item(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    item_id: i32,
    trace: bool,
) -> Result<(), GameError> {
    let op = Op::Equip {
        item: wire_u32(item_id),
    };
    let out = match commit(world, player, op, &[]).await? {
        Ok(changes) => {
            eprintln!("world: item {item_id} worn: {changes:?}");
            placement_frames(&changes, player.agent_id, true)
        }
        Err(refusal) => {
            eprintln!("world: equipping item {item_id} refused: {refusal:?}");
            Outgoing {
                own: snap_back(player, item_id),
                others: Vec::new(),
            }
        }
    };
    deliver(stream, session, player, out, trace).await
}

/// The client takes an item off (`GAME_CMSG_UNEQUIP_ITEM`, `0x804F`: `{equipped_slot: u8, bag: u16, slot: u8}`).
///
/// The live server moves the item out of the equipped bag to the destination the client names, echoes
/// the move, and clears that piece's single visual slot (`items-live-20261108-174014.jsonl` seq 22-25:
/// slot 2 -> item 639 to bag 6 slot 4, then `0x006F {agent 7, visual 2, 0}`). For armour it also
/// re-sends the character's energy and energy regeneration (properties 41 and 43); this server has no
/// armour-to-energy model, so it updates the piece and leaves the character's pools as the load block
/// set them.
pub async fn unequip_item(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    equipped_slot: u8,
    dest_bag: i32,
    dest_slot: i32,
    trace: bool,
) -> Result<(), GameError> {
    let worn = player
        .inventory
        .item_in_equipped_slot(equipped_slot)
        .map(|item| item.id);
    let op = Op::Unequip {
        equipped: equipped_slot,
        bag: wire_u16(dest_bag),
        slot: wire_u8(dest_slot),
    };
    let out = match commit(world, player, op, &[]).await? {
        Ok(changes) => {
            eprintln!("world: equipped slot {equipped_slot} taken off: {changes:?}");
            placement_frames(&changes, player.agent_id, true)
        }
        Err(refusal) => {
            eprintln!(
                "world: unequip of slot {equipped_slot} to bag {dest_bag} slot {dest_slot} refused: {refusal:?}"
            );
            Outgoing {
                own: match worn {
                    Some(id) => snap_back(player, id),
                    None => Vec::new(),
                },
                others: Vec::new(),
            }
        }
    };
    deliver(stream, session, player, out, trace).await
}

/// The client destroys an item by dragging it to the trash (`0x8069`: `{item_id: u32}`).
///
/// The row is deleted and the live server answers `ITEM_REMOVE` with the same id
/// (`items-live-20261108-173913.jsonl` seq 192-193); without it the client's own removal is only in
/// its model and the item returns on the next load. Something the character does not hold in a slot
/// is answered with nothing.
pub async fn destroy_item(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    item_id: i32,
    trace: bool,
) -> Result<(), GameError> {
    if item_id <= 0 {
        return Ok(());
    }
    let op = Op::Destroy {
        item: wire_u32(item_id),
    };
    match commit(world, player, op, &[]).await? {
        Ok(changes) => {
            eprintln!("world: item {item_id} destroyed by {0:?}", player.name);
            let out = placement_frames(&changes, player.agent_id, true);
            deliver(stream, session, player, out, trace).await
        }
        Err(refusal) => {
            eprintln!(
                "world: a destroy of item {item_id} by {0:?} refused: {refusal:?}",
                player.name
            );
            Ok(())
        }
    }
}

/// `/item <live name or id> [quantity]`: give the character one live-verified catalogue definition in
/// the first free carrying slot.
///
/// This is an ASURA test command, not a reference handler. It writes the row, then sends the measured
/// pair `CREATE_NAMED_ITEM` and `ITEM_CHANGE_LOCATION` (`0x014B`). Only definitions with `live: true`
/// are accepted: the flag means the client's own item probe has shown that wire identity, and an
/// unverified file id can crash the client on its UI texture.
pub async fn grant_definition(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    argument: &str,
    trace: bool,
) -> Result<(), GameError> {
    let Some((query, quantity)) = grant_request(argument) else {
        eprintln!("world: /item needs a name or id and a positive quantity");
        return Ok(());
    };
    let catalogue = match item_catalogue::current() {
        Ok(catalogue) => catalogue,
        Err(err) => {
            eprintln!("world: /item: {err}");
            return Ok(());
        }
    };
    let wanted = grant_key(query);
    let matches: Vec<&ItemDefinition> = catalogue
        .definitions()
        .iter()
        .filter(|definition| {
            definition.live
                && (grant_key(&definition.id) == wanted
                    || grant_key(&definition.description) == wanted
                    || grant_key(&definition.id).contains(&wanted)
                    || grant_key(&definition.description).contains(&wanted))
        })
        .collect();
    let definition = match matches.as_slice() {
        [definition] => *definition,
        [] => {
            eprintln!(
                "world: /item: {query:?} is not a live-verified definition; the client's item probe \
                 has not shown its wire identity"
            );
            return Ok(());
        }
        many => {
            eprintln!(
                "world: /item: {query:?} matches {} live definitions; use an id: {}",
                many.len(),
                many.iter()
                    .map(|definition| definition.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return Ok(());
        }
    };

    let item_id = db_inventory::allocate_item_id(&world.pool)
        .await
        .map_err(GameError::Database)?;
    let row = instantiate(definition, item_id, quantity);
    let entry = match entry_of(&row, Some(catalogue)) {
        Ok(entry) => entry,
        Err(err) => {
            eprintln!("world: /item: {} is not a valid row: {err}", definition.id);
            return Ok(());
        }
    };
    let changes = match commit(world, player, Op::Insert { entry }, std::slice::from_ref(&row)).await? {
        Ok(changes) => changes,
        Err(refusal) => {
            eprintln!("world: /item: {} x{quantity} refused: {refusal:?}", definition.id);
            return Ok(());
        }
    };
    let Some((bag, slot)) = changes.iter().find_map(|change| match *change {
        Change::Inserted { at, .. } => Some(at),
        _ => None,
    }) else {
        return Ok(());
    };
    let Some(item) = player.inventory.item(item_id).cloned() else {
        eprintln!("world: /item: wrote item {item_id} but could not read it back");
        return Ok(());
    };

    let mut frames = Vec::with_capacity(3);
    frames.push(game::create_named_item(&named_item(&item)));
    if item.profession != 0 {
        frames.push(game::item_set_profession(
            wire_u32(item.id),
            wire_u8(item.profession),
        ));
    }
    frames.push(game::item_change_location(ITEM_STREAM, wire_u32(item.id), bag, slot));
    eprintln!(
        "world: /item gave {} x{quantity} to {:?} in bag {bag} slot {slot} (item {item_id}; base {}, upgrades {:x?})",
        definition.id,
        player.name,
        definition.base.as_deref().unwrap_or("-"),
        definition.upgrades
    );
    send(stream, session, frames, trace).await
}

/// `/item clear`: destroy everything in the carrying bags (not worn items, not storage, not a bag's own
/// item), each through the same rules and transaction as a trash drag, answered with `ITEM_REMOVE`
/// per item. A test helper for sweeping the catalogue, not a reference handler.
pub async fn clear_carrying(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<(), GameError> {
    let carried: Vec<i32> = player
        .inventory
        .items
        .iter()
        .filter(|item| {
            item.slot.is_some()
                && item
                    .bag_id
                    .and_then(|bag| player.inventory.bag(bag))
                    .is_some_and(|bag| BagKind::of(bag.bag_model_id) == BagKind::Carrying)
        })
        .map(|item| item.id)
        .collect();
    let mut frames = Vec::with_capacity(carried.len());
    for item_id in &carried {
        if let Ok(changes) = commit(world, player, Op::Destroy { item: wire_u32(*item_id) }, &[]).await? {
            frames.extend(placement_frames(&changes, player.agent_id, true).own);
        }
    }
    eprintln!(
        "world: /item clear removed {} item(s) from {:?}'s carrying bags",
        frames.len(),
        player.name
    );
    send(stream, session, frames, trace).await
}

/// A new owned row from a definition: the one place a definition becomes an instance. The row is not
/// placed (`bag_id`/`slot` are set when the rules choose a slot) and starts with the definition's
/// defaults - no dye, its inherent modifiers, its flags.
pub fn instantiate(definition: &ItemDefinition, item_id: i32, quantity: u32) -> db_inventory::Item {
    db_inventory::Item {
        id: item_id,
        definition_id: Some(definition.id.clone()),
        bag_id: None,
        slot: None,
        file_id: i64::from(definition.file_id),
        item_type: i32::from(definition.item_type),
        model_id: i32::try_from(definition.model_id).unwrap_or(i32::MAX),
        dye_tint: 0,
        dye_colors: 0,
        materials: i32::from(definition.materials),
        unk1: i32::from(definition.unk1),
        flags: i64::from(definition.flags),
        value: i32::try_from(definition.value).unwrap_or(i32::MAX),
        quantity: i32::try_from(quantity).unwrap_or(i32::MAX),
        profession: i32::from(definition.profession),
        name: definition.name.iter().map(|unit| i32::from(*unit)).collect(),
        modifiers: definition.modifiers.iter().map(|word| i64::from(*word)).collect(),
    }
}

/// The bulk `UPDATE_AGENT_VISUAL_EQUIPMENT` (`0x006E`): all nine worn ids, as the load block sends them.
pub fn equipment_frame(player: &Player) -> Frame {
    game::update_agent_visual_equipment(player.GetAgentID(), player.inventory.equipped_item_ids())
}

/// How far from the player the reference puts an item it drops: `Vec2fAdd(position, (30, 30))`.
pub const DROP_OFFSET: f32 = 30.0;

/// The client asks for the item stream. The reference answers with the inventory, the weapon
/// slots, the gold, the faction caps, the character info, the map, the spawn readiness and the
/// download manifest -- in that order (`GameSrv_HandleInstanceLoadRequestItems`).
pub async fn handle_request_items(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let open_stream = !player.items_sent;
    if !open_stream {
        eprintln!(
            "world: {:?} asked for items again; stream {} is already open, so its create is not \
             resent",
            player.name, ITEM_STREAM
        );
    }
    let replay = world.replay.as_ref();
    let block = match replay {
        Some(replay) if replay.pick.wants(crate::replay::BlockName::Items) => replay.items.clone(),
        _ => {
            let mut block = items_block_with_stream(
                &player.instance.map,
                &player.inventory,
                Purse {
                    carried: player.traits.gold,
                    stored: player.storage_gold,
                },
                open_stream,
            );
            // The quest log's entries, where the live server puts them: after the gold, before the
            // faction caps, while the client is still loading (`crate::quest::LoadLog`). The client
            // starts every map with an empty log; sent after the load, the same messages announce
            // every quest as new.
            if open_stream {
                let entries = crate::quest::load_log(&player.quests, &player.name).entries;
                let caps = block
                    .iter()
                    .position(|frame| frame.header() == game::FACTION_MAX_KURZICK)
                    .unwrap_or(block.len());
                block.splice(caps..caps, entries);
            }
            block
        }
    };
    player.items_sent = true;
    send(stream, session, block, trace).await?;
    Ok(true)
}

/// The client drops an item (`GameSrv_HandleDropItem`, `0x802C`: `{item_id: u32, count: u8}`).
///
/// The reference's own handler never removes the item and bails out when `count` is smaller than the
/// stack (`TODO: we must still implement this`), so dropping a stacked item there does nothing. The
/// live server has no such gap: `items-live-20261108-174014.jsonl` seq 132-136 drops five of ten and
/// answers `ITEM_UPDATE_QUANTITY` plus a ground record; [`drop_item`] follows that capture.
pub async fn handle_drop_item(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let body = &frame.body();
    if body.len() < 5 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 7 }));
    }
    let item_id = i32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let count = i32::from(body[4]);
    drop_item(stream, session, world, player, item_id, count, trace).await?;
    Ok(true)
}

/// The client destroys an item by dragging it to the trash (`0x8069`: `{item_id: u32}`); see
/// [`destroy_item`].
pub async fn handle_item_destroy(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let body = &frame.body();
    if body.len() < 4 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 6 }));
    }
    let item_id = i32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    destroy_item(stream, session, world, player, item_id, trace).await?;
    Ok(true)
}

/// The client interacts with an agent (`GAME_CMSG_INTERACT_ITEM`, `0x803F`:
/// `{agent_id: u32, u8}`). A ground item is the one kind this world answers; every other target is
/// ignored, as the reference's own unfinished handler does.
pub async fn handle_interact_item(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let body = &frame.body();
    if body.len() < 4 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 7 }));
    }
    let agent_id = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    interact_item(stream, session, world, player, agent_id, trace).await?;
    Ok(true)
}

/// The client moves an item to a slot (`GAME_CMSG_ITEM_MOVE`, `0x8072`: `{item_id: u32,
/// bag_id: u16, slot: u8}`). Measured from a live client: dragging an item produced two of these
/// and nothing else. The answer is `ITEM_CHANGE_LOCATION` (`0x014B`); `0x013E` is the load block's
/// placement and put a second copy of the item in the destination when sent mid-session.
///
/// The client has already moved the item in its own model by the time it asks, so a refusal is
/// answered by telling it where the item really is rather than by silence.
pub async fn handle_item_move(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let body = &frame.body();
    if body.len() < 7 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 9 }));
    }
    let item_id = i32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let bag_id = i32::from(u16::from_le_bytes([body[4], body[5]]));
    let slot = i32::from(body[6]);
    move_item(
        stream,
        session,
        world,
        player,
        Placement {
            item_id,
            bag_id,
            slot,
        },
        trace,
    )
    .await?;
    Ok(true)
}

/// The client equips an item (`GAME_CMSG_EQUIP_ITEM`, `0x8030`: `{item_id: u32}`).
///
/// The item moves into the equipped-items bag slot its type belongs to, whatever was worn there goes into
/// the first free carrying slot, and the answer is the move plus `UPDATE_AGENT_VISUAL_EQUIPMENT_SLOT`
/// (`0x006F`) for that piece - the live server's pair (`items-live-20261108-174014.jsonl` seq 31-32).
/// The type-to-slot table is `game::equipped_slot`, and the bag slot is not the visual slot
/// (`game::visual_equipment_slot`).
pub async fn handle_equip_item(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let body = &frame.body();
    if body.len() < 4 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 6 }));
    }
    let item_id = i32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    equip_item(stream, session, world, player, item_id, trace).await?;
    Ok(true)
}

/// `GAME_CMSG_UNEQUIP_ITEM` (`0x804F`, 6 bytes: `{equipped_slot: u8, bag: u16, slot: u8}`).
///
/// The three fields are the equipped bag slot to empty and where the client wants the piece to go;
/// [`unequip_item`] moves it and answers with the capture's pair.
pub async fn handle_unequip_item(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let body = &frame.body();
    if body.len() < 4 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 6 }));
    }
    let equipped_slot = body[0];
    let dest_bag = i32::from(u16::from_le_bytes([body[1], body[2]]));
    let dest_slot = i32::from(body[3]);
    unequip_item(
        stream,
        session,
        world,
        player,
        equipped_slot,
        dest_bag,
        dest_slot,
        trace,
    )
    .await?;
    Ok(true)
}

/// The comparison spelling for `/item`: case-folded, with underscores and
/// hyphens treated as spaces, so `skale_fin`, `skale-fin` and `Skale Fin` are
/// one query.
fn grant_key(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '_' | '-' => ' ',
            other => other.to_ascii_lowercase(),
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse `/item`'s argument: the last token is a quantity only when it is all
/// digits. A name may contain spaces, so `skale fin 5` is five Skale Fins.
fn grant_request(argument: &str) -> Option<(&str, u32)> {
    let argument = argument.trim();
    if argument.is_empty() {
        return None;
    }
    if let Some((name, quantity)) = argument.rsplit_once(' ') {
        if !quantity.is_empty() && quantity.chars().all(|ch| ch.is_ascii_digit()) {
            let quantity = quantity.parse::<u32>().ok()?;
            let name = name.trim();
            return (!name.is_empty() && quantity > 0).then_some((name, quantity));
        }
    }
    Some((argument, 1))
}

#[cfg(test)]
mod tests {
    use super::{grant_key, grant_request};

    #[test]
    fn an_item_name_may_contain_spaces_and_an_optional_quantity() {
        assert_eq!(grant_request("skale fin"), Some(("skale fin", 1)));
        assert_eq!(grant_request("skale fin 5"), Some(("skale fin", 5)));
        assert_eq!(grant_request("  worn belt  "), Some(("worn belt", 1)));
        assert_eq!(grant_request(""), None);
        assert_eq!(grant_request("skale fin 0"), None);
    }

    #[test]
    fn an_item_query_ignores_case_and_separator_spelling() {
        assert_eq!(grant_key("Skale Fin"), "skale fin");
        assert_eq!(grant_key("skale_fin"), "skale fin");
        assert_eq!(grant_key("skale-fin"), "skale fin");
    }
}
