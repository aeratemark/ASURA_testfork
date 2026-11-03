//! The item and the inventory: what a character carries, and what a client may do with it.
//!
//! [`Item`] and [`Inventory`] are the two nouns. [`items_block`] and [`inventory_block`] are the
//! load-time blocks; the four
//! client operations below are `DROP_ITEM`, `ITEM_MOVE`, `EQUIP_ITEM` and the refusal path, and
//! the row-level work is `asura_db::inventory`.
//!
//! [`move_item`] and [`equip_item`] are messages the reference does not dispatch at all - their
//! shapes are measured, not ported (`docs/HANDOFF.md`, M5).

use crate::*;

/// The item API: the questions a server answers about one item row, one question one name. A
/// member takes the item row (`asura_db::inventory::Item`); the wire side is
/// `asura_protocol::game`'s builders.
#[allow(non_snake_case)]
pub struct Item;

#[allow(non_snake_case)]
impl Item {
    /// `Item.GetModelID()`: the model the client draws.
    pub fn GetModelID(item: &db_inventory::Item) -> i32 {
        item.model_id
    }

    /// `Item.GetModelFileID()`: the `Gw.dat` entry the model comes from.
    pub fn GetModelFileID(item: &db_inventory::Item) -> i64 {
        item.file_id
    }

    /// `Item.GetItemType()`: the reference's `ItemType`.
    pub fn GetItemType(item: &db_inventory::Item) -> i32 {
        item.item_type
    }

    /// `Item.GetSlot()`: where the item sits, `None` for an item that is not in a slot.
    pub fn GetSlot(item: &db_inventory::Item) -> Option<i32> {
        item.slot
    }
}

/// The inventory API: the questions a server answers about what a character carries. A member
/// takes the character's row-set (`asura_db::inventory::Inventory`).
#[allow(non_snake_case)]
pub struct Inventory;

#[allow(non_snake_case)]
impl Inventory {
    /// `Inventory.GetBags()`: the bags, in the order the reference walks them.
    pub fn GetBags(inventory: &db_inventory::Inventory) -> &[db_inventory::Bag] {
        &inventory.bags
    }

    /// `Inventory.GetItemCount()`: how many items the character carries.
    pub fn GetItemCount(inventory: &db_inventory::Inventory) -> usize {
        inventory.items.len()
    }

    /// `Inventory.GetGoldOnCharacter()`: the gold the load block reports.
    pub fn GetGoldOnCharacter(player: &Player) -> u32 {
        player.traits.gold
    }
}

/// The answer to the client's item request (`GameSrv_HandleInstanceLoadRequestItems`).
///
/// A pure function of the map and the character's inventory so that `examples/dump-load.rs` can print
/// exactly what a client would receive, byte for byte, without a socket - which is how this stream is
/// compared against the reference's own log.
pub fn items_block(map: &MapEntry, inventory: &db_inventory::Inventory, gold: u32) -> Vec<Frame> {
    let mut frames = vec![
        game::item_stream_create(ITEM_STREAM, 0),
        game::item_set_active_weapon_set(ITEM_STREAM, 0),
    ];
    // The inventory itself, from the character's rows. It is the reference's own walk: for every bag, the
    // item that *is* the bag, the bag, and then its items with their placements.
    frames.extend(inventory_block(inventory));
    // Four empty weapon slots, exactly as `GameSrv_SendWeaponSlots` sends them.
    for slot in 0..4u8 {
        frames.push(game::item_weapon_set(ITEM_STREAM, slot, 0, 0));
    }
    frames.push(game::update_gold_storage(ITEM_STREAM, gold));
    for faction in [
        game::FACTION_MAX_KURZICK,
        game::FACTION_MAX_LUXON,
        game::FACTION_MAX_BALTHAZAR,
        game::FACTION_MAX_IMPERIAL,
    ] {
        // 10000, which is what the reference sends (`10 27 00 00`).
        frames.push(game::faction_max(faction, 10_000));
    }
    frames.push(game::hard_mode_unlocked(1));
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

/// `GameSrv_HandleDropItem`: take the items off the stack for real, and reserve the agent id a ground
/// item will use once a capture shows what a drop actually sends.
///
/// The shape that agent will have is the reference's own, recorded here so it is not re-derived later:
/// the player's position plus (30, 30), the same plane, facing x, everything `GameSrv_CreateAgent`'s
/// `memset` leaves at zero (speed, team token), and the item's *model* where the reference puts the
/// item's *id*. Nothing is sent from here today: `drop_messages` carries the whole argument.
pub async fn drop_item(
    world: &World,
    player: &mut Player,
    item_id: i32,
    count: i32,
) -> Result<(), GameError> {
    if item_id <= 0 || count <= 0 {
        eprintln!("world: a drop of {count} of item {item_id} names something that is not an item");
        return Ok(());
    }

    let Some(item) = db_inventory::find_in_carrying_bags(&world.pool, player.character, item_id)
        .await
        .map_err(GameError::Database)?
    else {
        eprintln!(
            "world: a drop names item {item_id}, which {0:?} does not carry; ignored",
            player.name
        );
        return Ok(());
    };
    if count > item.quantity {
        eprintln!(
            "world: a drop of {count} of item {item_id} is more than the {} it holds; ignored",
            item.quantity
        );
        return Ok(());
    }

    let Some(taken) = db_inventory::take_from_stack(&world.pool, player.character, item_id, count)
        .await
        .map_err(GameError::Database)?
    else {
        eprintln!("world: item {item_id} could not be taken out of its stack; ignored");
        return Ok(());
    };

    let agent_id = player.instance.allocate_agent_id();
    eprintln!(
        "world: {} dropped {count} of item {item_id} at ({:.1}, {:.1}), agent {agent_id} reserved, {} \
         left in the bag -- and nothing is sent: both messages a drop could send have crashed a client",
        player.name,
        player.standing().position.0 + DROP_OFFSET,
        player.standing().position.1 + DROP_OFFSET,
        taken.remaining
    );
    reload(world, player).await
}

/// The messages a drop puts on the wire: **none**.
///
/// This is not caution and it is not a switch any more -- it is the whole of what has been measured. Both
/// candidates put a live client on the floor:
///
/// * `ITEM_REMOVE` (`0x014D`) with the item re-created at what was left: the client crashed, and in
///   another build cloned the item;
/// * the ground agent (`WORLD_CREATE_AGENT`, `0x0020`, `AgentType_Item`): the client crashed, and the
///   log's last line was that message.
///
/// Neither message appears in any capture, and an *item* agent never can: the only `0x0020` a capture
/// carries is a player's (type 1), and the one code path that would have produced an item agent is the
/// reference's own unfinished drop handler. So a drop deducts the stack, writes it down, says so in the
/// log -- and sends nothing, because a client that is running is worth more than an item on the ground.
/// When a capture of a working drop exists, this function grows the message it shows.
pub fn drop_messages() -> Vec<Frame> {
    Vec::new()
}

/// `GAME_CMSG_ITEM_MOVE`: put an item in a slot, and tell the client where it ended up.
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
    let before = player.inventory.equipped_item_ids();
    let outcome = db_inventory::move_item(&world.pool, player.character, item_id, bag_id, slot)
        .await
        .map_err(GameError::Database)?;

    let mut frames = Vec::with_capacity(2);
    match outcome {
        db_inventory::MoveOutcome::At { bag_id, slot, .. } => {
            eprintln!("world: item {item_id} moved to bag {bag_id} slot {slot}");
            // Whether the client wants this confirmed is not settled, and both reports are real: with
            // the echo a live client moved the item (that same session also reported a clone, but it was
            // a build that re-created items on a drop too), and with silence the item sprang back and the
            // move did nothing. `MoveEcho` keeps both, and defaults to the echo - the one a client was
            // seen to act on.
            frames.push(game::item_move_to_location(
                ITEM_STREAM,
                wire_u32(item_id),
                wire_u16(bag_id),
                wire_u8(slot),
            ));
            reload(world, player).await?;
        }
        db_inventory::MoveOutcome::Unchanged { .. } => return Ok(()),
        db_inventory::MoveOutcome::Refused { item, reason } => {
            eprintln!("world: item {item_id} to bag {bag_id} slot {slot} refused: {reason:?}");
            // The client moved it in its own model before asking, so put it back where it really is.
            if let Some(item) = item {
                if let (Some(bag), Some(slot)) = (item.bag_id, item.slot) {
                    frames.push(game::item_move_to_location(
                        ITEM_STREAM,
                        wire_u32(item_id),
                        wire_u16(bag),
                        wire_u8(slot),
                    ));
                }
            }
        }
    }

    if player.inventory.equipped_item_ids() != before {
        frames.push(equipment_frame(player));
    }
    send(stream, session, frames, trace).await
}

/// `GAME_CMSG_EQUIP_ITEM`: wear an item, evicting whatever the equipped slot already held.
pub async fn equip_item(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    item_id: i32,
    trace: bool,
) -> Result<(), GameError> {
    let Some(item) = player.inventory.item(item_id).cloned() else {
        eprintln!(
            "world: an equip names item {item_id}, which {0:?} does not have",
            player.name
        );
        return Ok(());
    };
    let Some(slot) = game::equipped_slot(wire_u8(item.item_type)) else {
        eprintln!(
            "world: item {item_id} (type {}) is not something a character wears; equip ignored",
            item.item_type
        );
        return Ok(());
    };
    let Some(bag) = player
        .inventory
        .bags
        .iter()
        .find(|bag| bag.bag_model_id == db_inventory::EQUIPPED_ITEMS_MODEL)
        .copied()
    else {
        eprintln!(
            "world: {0:?} has no equipped-items bag; equip ignored",
            player.name
        );
        return Ok(());
    };

    let slot = i32::from(slot);
    let before = player.inventory.equipped_item_ids();
    let mut frames = Vec::with_capacity(3);

    // Whatever is worn in that slot has to come off first, and there has to be somewhere to put it.
    let occupant = player
        .inventory
        .items_in(bag.bag_id)
        .into_iter()
        .find(|item| item.slot == Some(slot) && item.id != item_id)
        .cloned();
    if let Some(occupant) = occupant {
        let Some((to_bag, to_slot)) =
            db_inventory::first_free_carrying_slot(&world.pool, player.character)
                .await
                .map_err(GameError::Database)?
        else {
            eprintln!(
                "world: no free carrying slot to take item {} off, so item {item_id} cannot be worn",
                occupant.id
            );
            return Ok(());
        };
        match db_inventory::move_item(&world.pool, player.character, occupant.id, to_bag, to_slot)
            .await
            .map_err(GameError::Database)?
        {
            db_inventory::MoveOutcome::At { bag_id, slot, .. } => {
                eprintln!(
                    "world: item {} taken off to bag {bag_id} slot {slot}",
                    occupant.id
                );
                frames.push(game::item_move_to_location(
                    ITEM_STREAM,
                    wire_u32(occupant.id),
                    wire_u16(bag_id),
                    wire_u8(slot),
                ));
            }
            outcome => {
                eprintln!(
                    "world: could not take item {} off to make room for {item_id}: {outcome:?}",
                    occupant.id
                );
                return Ok(());
            }
        }
    }

    match db_inventory::move_item(&world.pool, player.character, item_id, bag.bag_id, slot)
        .await
        .map_err(GameError::Database)?
    {
        db_inventory::MoveOutcome::At { bag_id, slot, .. } => {
            eprintln!("world: item {item_id} worn in equipped slot {slot}");
            // The same switch as a plain move, and the same open question. What the client cannot work
            // out by itself is what the agent now wears, and that is sent below either way.
            frames.push(game::item_move_to_location(
                ITEM_STREAM,
                wire_u32(item_id),
                wire_u16(bag_id),
                wire_u8(slot),
            ));
        }
        db_inventory::MoveOutcome::Unchanged { .. } => {}
        db_inventory::MoveOutcome::Refused { item, reason } => {
            eprintln!("world: equipping item {item_id} refused: {reason:?}");
            if let Some(item) = item {
                if let (Some(bag), Some(slot)) = (item.bag_id, item.slot) {
                    frames.push(game::item_move_to_location(
                        ITEM_STREAM,
                        wire_u32(item_id),
                        wire_u16(bag),
                        wire_u8(slot),
                    ));
                }
            }
        }
    }

    reload(world, player).await?;
    if player.inventory.equipped_item_ids() != before {
        frames.push(equipment_frame(player));
    }
    send(stream, session, frames, trace).await
}

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
    let replay = world.replay.as_ref();
    let block = match replay {
        Some(replay) if replay.pick.wants(crate::replay::BlockName::Items) => replay.items.clone(),
        _ => items_block(&player.instance.map, &player.inventory, player.traits.gold),
    };
    send(stream, session, block, trace).await?;
    Ok(true)
}

/// The client drops an item (`GameSrv_HandleDropItem`, `0x802C`: `{item_id: u32, count: u8}`).
///
/// **This is where this port leaves the reference behind, deliberately.** The reference's handler
/// creates an agent for the item and returns: it never removes the item from its bag, never
/// frees its id, never sends `ITEM_REMOVE`, and bails out entirely when the count is smaller than
/// the stack (`TODO: we must still implement this`) -- so dropping a stacked item there does
/// nothing at all, and the client always sends `count = 1` for a stack. Its item agent also
/// carries the item's *id* as its model id, so the thing on the ground is drawn as nothing. A
/// measured live session (`docs/08-next-inventory-model.md`) showed all three in one log.
///
/// What this does instead: take `count` off the stack for real, write it down, and say so in the
/// log. It sends nothing -- both messages it could send (`ITEM_REMOVE` and an item agent) crashed a
/// live client, and neither is in any capture. `drop_messages` carries the whole argument.
pub async fn handle_drop_item(
    frame: &Frame,
    world: &World,
    player: &mut Player,
) -> Result<bool, GameError> {
    let body = &frame.body();
    if body.len() < 5 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 7 }));
    }
    let item_id = i32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let count = i32::from(body[4]);
    drop_item(world, player, item_id, count).await?;
    Ok(true)
}

/// The client moves an item to a slot (`GAME_CMSG_ITEM_MOVE`, `0x8072`: `{item_id: u32,
/// bag_id: u16, slot: u8}` -- the exact inverse of the `ITEM_MOVE_TO_LOCATION` we send). Measured
/// from a live client: dragging an item produced two of these and nothing else.
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
/// The reference has no handler for this at all, so the behaviour here is this project's: the
/// item moves into the equipped-items bag slot its type belongs to, whatever was worn there goes
/// into the first free carrying slot, and the client is told what it is now wearing. The
/// type-to-slot table is `game::equipped_slot`.
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

/// `GAME_CMSG_UNEQUIP_ITEM` (`0x804F`, 6 bytes: `{u8, u16, u8}`). Its layout carries no item id,
/// and nothing here has seen a live one, so this does not guess: unequipping by dragging an item
/// out of the equipped-items bag arrives as `ITEM_MOVE` above and is handled there.
pub fn handle_unequip_item() -> Result<bool, GameError> {
    eprintln!(
        "world: 0x804F (unequip) is not implemented: it carries no item id and no live capture \
         has shown what it means; dragging the item out of the equipped bag works instead"
    );
    Ok(true)
}
