//! Character creation: the instance and the choices a new character is made of.
//!
//! The reference's `MapType_CharacterCreation` - the only map type that is a conversation
//! rather than a place (`docs/HANDOFF.md`, M5). [`Creation`] is what a creation connection has
//! chosen so far; the two blocks are the reference's own default equipment and inventory for
//! that choice (`GetDefaultEquipments`, generated into `default_equipment.rs`).

use crate::*;

/// The energy and health a character-creation screen's player-data block reports. **Not** the load block's
/// numbers: `GameSrv_SendPlayerHealthEnergy` (`GmPlayer.c:178-199`) sends 20 and 100, while the load path
/// sends 25 and 400 (measured from the reference's own send log). Two paths, two sets of numbers, and mixing
/// them -- which this server did until the log was read -- is a mismatched block.
pub const CREATION_ENERGY: u32 = 20;

/// See [`CREATION_ENERGY`].
pub const CREATION_HEALTH: u32 = 100;

/// `GM_ERROR_CHARACTER_NAME_ALREADY_EXIST` (`GmError.h`): the code the reference answers a taken name with
/// (`GameSrv.c:1939`).
pub const GM_ERROR_CHARACTER_NAME_ALREADY_EXIST: u32 = 0x0106;

/// What a character-creation connection has chosen so far.
///
/// The reference keeps exactly this on its player (`char_creation_campaign`, `primary_profession`) and
/// nothing else: the equipment is not stored either - it is re-derived from the generated table every time
/// the profession changes (`GetDefaultEquipments`) - and only becomes rows at confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Creation {
    pub campaign: u8,
    pub profession: u8,
}

impl Creation {
    /// The reference's starting state: `HandleCharCreationRequestPlayer` sets Warrior, and nothing sets the
    /// campaign until the client says so - so a zeroed `Campaign` (`None`, the PvP table) is what a fresh
    /// creation screen is built from.
    pub fn start() -> Creation {
        Creation {
            campaign: 0,
            profession: DEFAULT_PRIMARY_PROFESSION,
        }
    }
}

/// The bags and items a creation screen shows: the reference's own default set, with the equipped bag
/// holding the chosen profession's starting equipment.
///
/// Built in memory because that is where the reference keeps it too - `HandleCharCreationChangeProf`
/// frees and re-adds items in the player's `GmBag`s, and `HandleCharCreationConfirm` is the only place they
/// become database rows.
pub fn creation_inventory(creation: Creation) -> Inventory {
    use std::collections::HashMap;

    let equipped_bag = db_inventory::seed::BAGS
        .iter()
        .find(|bag| bag.bag_type == 2)
        .map(|bag| bag.bag_id);

    let mut items: Vec<db_inventory::Item> = Vec::new();
    // The capture's item ids mean nothing once rows are renumbered, so the mapping is kept explicitly:
    // a bag names its own item by seed id, and only this map knows which row that became.
    let mut by_seed: HashMap<i32, i32> = HashMap::new();
    let mut next_id = 1i32;

    // Everything the reference's default inventory holds outside the equipped bag -- the backpack's own
    // item and the belt pouch's -- so what the creation screen sees is what the confirm writes.
    for seed_item in db_inventory::seed::ITEMS
        .iter()
        .filter(|item| item.bag_id != equipped_bag)
    {
        by_seed.insert(seed_item.id, next_id);
        items.push(db_inventory::Item {
            id: next_id,
            bag_id: seed_item.bag_id,
            slot: seed_item.slot,
            file_id: seed_item.file_id,
            item_type: seed_item.item_type,
            model_id: seed_item.model_id,
            dye_tint: seed_item.dye_tint,
            dye_colors: seed_item.dye_colors,
            materials: seed_item.materials,
            unk1: seed_item.unk1,
            flags: seed_item.flags,
            value: seed_item.value,
            quantity: seed_item.quantity,
            profession: seed_item.profession,
            name: seed_item.name.to_vec(),
            modifiers: seed_item.modifiers.to_vec(),
        });
        next_id += 1;
    }

    for spec in crate::default_equipment::default_equipment(creation.campaign, creation.profession)
    {
        // `GetBagSlotForItemType`'s mapping, which is what `equip_item` uses too.
        let slot = game::equipped_slot(spec.item_type).map(i32::from);
        items.push(db_inventory::Item {
            id: next_id,
            bag_id: equipped_bag,
            slot,
            file_id: i64::from(spec.file_id),
            item_type: i32::from(spec.item_type),
            model_id: spec.model_id as i32,
            dye_tint: i32::from(spec.dye_tint),
            dye_colors: i32::from(spec.dye_colors),
            materials: i32::from(spec.materials),
            unk1: i32::from(spec.unk1),
            flags: i64::from(spec.flags),
            value: spec.value as i32,
            quantity: spec.quantity as i32,
            profession: i32::from(spec.profession),
            name: spec.name.iter().map(|unit| i32::from(*unit)).collect(),
            modifiers: spec.modifiers.iter().map(|word| i64::from(*word)).collect(),
        });
        next_id += 1;
    }

    let bags = db_inventory::seed::BAGS
        .iter()
        .map(|seed_bag| db_inventory::Bag {
            bag_id: seed_bag.bag_id,
            bag_model_id: seed_bag.bag_model_id,
            bag_type: seed_bag.bag_type,
            slot_count: seed_bag.slot_count,
            assoc_item_id: by_seed.get(&seed_bag.assoc_item).copied().unwrap_or(0),
        })
        .collect();

    Inventory { bags, items }
}

/// The equipped bag's items for a choice: what the creation screen shows, and therefore what has to be
/// taken off the client before another set is sent into the same slots.
fn equipped_items(creation: Creation) -> Vec<db_inventory::Item> {
    let inventory = creation_inventory(creation);
    let equipped_bag = inventory
        .bags
        .iter()
        .find(|bag| bag.bag_type == 2)
        .map(|bag| bag.bag_id);
    inventory
        .items
        .into_iter()
        .filter(|item| item.bag_id == equipped_bag)
        .collect()
}

/// The equipped set as rows the confirm writes, owning their slices.
pub fn creation_equipment_rows(creation: Creation) -> Vec<db_inventory::NewItem> {
    let equipped_bag = db_inventory::seed::BAGS
        .iter()
        .find(|bag| bag.bag_type == 2)
        .map(|bag| bag.bag_id);
    crate::default_equipment::default_equipment(creation.campaign, creation.profession)
        .iter()
        .map(|spec| db_inventory::NewItem {
            bag_id: equipped_bag,
            slot: game::equipped_slot(spec.item_type).map(i32::from),
            file_id: i64::from(spec.file_id),
            item_type: i32::from(spec.item_type),
            model_id: spec.model_id as i32,
            dye_tint: i32::from(spec.dye_tint),
            dye_colors: i32::from(spec.dye_colors),
            materials: i32::from(spec.materials),
            unk1: i32::from(spec.unk1),
            flags: i64::from(spec.flags),
            value: spec.value as i32,
            quantity: spec.quantity as i32,
            profession: i32::from(spec.profession),
            name: spec.name.iter().map(|unit| i32::from(*unit)).collect(),
            modifiers: spec.modifiers.iter().map(|word| i64::from(*word)).collect(),
        })
        .collect()
}

/// Re-read the inventory after a change, so the next message built from it is the new truth.
pub async fn reload(world: &World, player: &mut Player) -> Result<(), GameError> {
    player.inventory = db_inventory::load(&world.pool, player.character)
        .await
        .map_err(GameError::Database)?;
    Ok(())
}

/// ---- character creation (a connection in a `MapType_CharacterCreation` instance) -------------
///
/// The four messages `GameSrv_ProcessPlayerMessage` dispatches to `GameSrv_HandleCharCreation*`.
/// The block each answers with is the reference's own order; the equipment is memory, not rows,
/// until the confirm writes it (`GameSrv_CreatePlayerBags` runs there and nowhere else).
pub async fn handle_request_player(
    stream: &mut TcpStream,
    session: &mut Session,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(creation) = player.creation else {
        return Ok(true);
    };
    let agent_id = player.agent_id;
    let inventory = creation_inventory(creation);
    let mut frames = vec![game::instance_player_data_start()];
    frames.push(game::item_stream_create(ITEM_STREAM, 0));
    frames.extend(inventory_block(&inventory));
    frames.push(game::item_set_active_weapon_set(ITEM_STREAM, 0));
    for slot in 0..4u8 {
        frames.push(game::item_weapon_set(ITEM_STREAM, slot, 0, 0));
    }
    frames.push(game::update_gold_storage(ITEM_STREAM, 0));
    for faction in [
        game::FACTION_MAX_KURZICK,
        game::FACTION_MAX_LUXON,
        game::FACTION_MAX_BALTHAZAR,
        game::FACTION_MAX_IMPERIAL,
    ] {
        frames.push(game::faction_max(faction, 10_000));
    }
    frames.push(game::agent_update_attribute_points(agent_id, 50, 50));
    frames.push(game::player_update_profession(
        agent_id,
        creation.profession,
        0,
        0,
    ));
    frames.push(game::player_update_unlocked_professions(
        agent_id,
        creation_screen_unlocked_professions(),
    ));
    frames.push(game::skillbar_update(agent_id, &[0; 8], &[0; 8], 1));
    frames.push(game::update_unlocked_skills(&[]));
    frames.push(game::character_update_factions(
        &game::CharacterProgress::default(),
    ));
    frames.push(game::agent_update_attributes(agent_id, &[0; 42]));
    // `GameSrv_SendPlayerHealthEnergy` (`GmPlayer.c:178-199`): three **int** properties with the
    // creation path's own numbers -- energy 20, health 100, property 64 zero. The load block's numbers
    // differ (25, 400, a regen float and a level), because the reference sends different ones there.
    frames.push(game::agent_property_update_int(
        game::AGENT_PROPERTY_ENERGY,
        agent_id,
        CREATION_ENERGY,
    ));
    frames.push(game::agent_property_update_int(
        game::AGENT_PROPERTY_HEALTH,
        agent_id,
        CREATION_HEALTH,
    ));
    frames.push(game::agent_property_update_int(
        AGENT_PROPERTY_VALUE64,
        agent_id,
        0,
    ));
    frames.push(game::instance_player_data_done());
    send(stream, session, frames, trace).await?;
    Ok(true)
}

/// `GameSrv_HandleCharCreationRequestArmors`: the PvP half. This account has no unlocked skills,
/// no heroes and no PvP items, which is what an empty stream says.
pub async fn handle_request_armors(
    stream: &mut TcpStream,
    session: &mut Session,
    trace: bool,
) -> Result<bool, GameError> {
    let frames = vec![
        game::pvp_update_unlocked_skills(&[]),
        game::pvp_update_unlocked_heroes(&[]),
        game::pvp_item_stream_end(),
    ];
    send(stream, session, frames, trace).await?;
    Ok(true)
}

/// `GameSrv_HandleCharCreationChangeProf` (`0x8060`: campaign, profession): swap the equipped set
/// and tell the client, which is the whole of what the reference does here.
pub async fn handle_change_profession(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(mut creation) = player.creation else {
        return Ok(true);
    };
    if frame.body().len() < 2 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 4 }));
    }
    let previous = creation;
    creation.campaign = frame.body()[0];
    creation.profession = frame.body()[1];
    player.creation = Some(creation);

    let agent_id = player.agent_id;
    let inventory = creation_inventory(creation);
    let mut frames = Vec::new();
    // The client still holds the set the previous change put there, and it asserts if an item is
    // created into a slot it thinks is occupied - `Assertion: !bagParent->GetItem(slot)`,
    // `ItCliInv.cpp(105)`, build 39112, measured 2026-10-28 as the crash the moment "select campaign"
    // opened. The reference takes the old pieces off the client with `ITEM_REMOVE` before it sends the
    // new ones (`0x014D x5` in
    // `recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`,
    // right after the profession moved); this server renumbers its item ids per set, so it clears the
    // equipped bag on every change.
    for item in equipped_items(previous) {
        frames.push(game::item_remove(ITEM_STREAM, item.id as u32));
    }
    frames.push(game::update_unlocked_skills(&[]));
    frames.push(game::player_update_profession(
        agent_id,
        creation.profession,
        0,
        0,
    ));
    // `GameSrv_SendBagItems` for the equipped bag: each item, then where it sits.
    let equipped_bag = inventory
        .bags
        .iter()
        .find(|bag| bag.bag_type == 2)
        .map(|bag| bag.bag_id);
    for item in inventory
        .items
        .iter()
        .filter(|item| item.bag_id == equipped_bag)
    {
        // The **named** creator: live sends `CREATE_NAMED_ITEM` on this path, 180-184 bytes with the
        // item's name and its modifiers (the five `0x0161` after each `0x0060` in the capture above).
        // The unnamed form leaves the client an item it holds without a name, and the creation screen
        // asserts `ptr->IsDetailHigh()` (`ItCliApi.cpp(698)`) when it asks for the details.
        frames.push(game::create_named_item(&named_item(item)));
        if let (Some(bag), Some(slot)) = (item.bag_id, item.slot) {
            frames.push(game::item_move_to_location(
                ITEM_STREAM,
                item.id as u32,
                bag as u16,
                slot as u8,
            ));
        }
    }
    send(stream, session, frames, trace).await?;
    Ok(true)
}

/// `GameSrv_HandleCharCreationConfirm` (`0x808B`: name, appearance, flags): write the rows and say
/// so. The reference answers `CHAR_CREATION_ERROR` when the name is taken; this server has the same
/// unique-per-account rule to enforce, and answers the same way.
pub async fn handle_confirm(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(creation) = player.creation else {
        return Ok(true);
    };
    let body = &frame.body();
    if body.len() < 2 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 4 }));
    }
    let units = u16::from_le_bytes([body[0], body[1]]) as usize;
    let name = String::from_utf16_lossy(
        &(0..units.min(name_max_units()))
            .map(|index| {
                let at = 2 + index * 2;
                u16::from_le_bytes([
                    body.get(at).copied().unwrap_or(0),
                    body.get(at + 1).copied().unwrap_or(0),
                ])
            })
            .collect::<Vec<u16>>(),
    );
    if !asura_rules::name_is_acceptable(&name) {
        send(
            stream,
            session,
            vec![game::char_creation_error(
                GM_ERROR_CHARACTER_NAME_ALREADY_EXIST,
            )],
            trace,
        )
        .await?;
        return Ok(true);
    }

    let equipped = creation_equipment_rows(creation);
    // The traits the confirm implies: the packed appearance the client sent (`msg->config`, four
    // bytes of `Appearance`), the profession the screen last chose, and the one unlocked-profession
    // bit the reference sets (`character.unlocked_professions = 1 << app.primary_profession`).
    let config_at = 2 + units * 2;
    let appearance = u32::from_le_bytes([
        body.get(config_at).copied().unwrap_or(0),
        body.get(config_at + 1).copied().unwrap_or(0),
        body.get(config_at + 2).copied().unwrap_or(0),
        body.get(config_at + 3).copied().unwrap_or(0),
    ]);
    // What a new character is worth is the pack's (`config/World/Vanilla/character_defaults.json`):
    // level, gold and secondary profession are values the world states. The unlocked-profession bitmap is
    // not among them - it is one bit for the profession the player chose, which the confirm derives.
    let defaults = match crate::character_defaults::current() {
        Ok(defaults) => defaults,
        Err(err) => {
            eprintln!("world: cannot create {name:?}: {err}");
            send(
                stream,
                session,
                vec![game::char_creation_error(
                    GM_ERROR_CHARACTER_NAME_ALREADY_EXIST,
                )],
                trace,
            )
            .await?;
            return Ok(true);
        }
    };
    let traits = db_game::Traits {
        appearance,
        profession: creation.profession,
        unlocked_professions: 1 << creation.profession,
        gold: defaults.gold(),
        campaign: creation.campaign,
        secondary_profession: defaults.secondary_profession(),
        level: defaults.level(),
    };
    // Where the character starts, which is not where it was made. A live creation redirects a Factions
    // character to map 212 (`crates/asura-db/migrations/0017_starter_outpost.sql`). Every other
    // campaign is put in Ascalon City, which is the Prophecies starting outpost and is already in the
    // catalogue; the log says when that stand-in was used, so it is never mistaken for a measurement.
    // Where the character is spawned, by the expansion source the screen chose. It is the world pack's
    // file (`config/World/Vanilla/character_spawn.json`, `docs/38`, D23, D24), not a match in this file:
    // one map per expansion source. A source with no map at all (a PvP character) is refused rather
    // than guessed.
    let spawns = match crate::spawns::current() {
        Ok(spawns) => spawns,
        Err(err) => {
            eprintln!("world: cannot create {name:?}: {err}");
            send(
                stream,
                session,
                vec![game::char_creation_error(
                    GM_ERROR_CHARACTER_NAME_ALREADY_EXIST,
                )],
                trace,
            )
            .await?;
            return Ok(true);
        }
    };
    let Some(placed) = spawns.for_campaign(creation.campaign) else {
        eprintln!(
            "world: campaign {} has no spawn map in the pack; refusing to create {name:?} rather than \
             putting it somewhere nobody has measured",
            creation.campaign
        );
        send(
            stream,
            session,
            vec![game::char_creation_error(
                GM_ERROR_CHARACTER_NAME_ALREADY_EXIST,
            )],
            trace,
        )
        .await?;
        return Ok(true);
    };
    let outpost = placed.map_id;
    match db_inventory::create_character(
        &world.pool,
        player.account_id,
        &name,
        outpost,
        traits,
        &equipped,
    )
    .await
    {
        Ok(character) => {
            eprintln!(
                "world: created character {name:?} ({} items, campaign {} profession {}) in map {}",
                equipped.len(),
                creation.campaign,
                creation.profession,
                outpost
            );
            send(
                stream,
                session,
                vec![
                    game::char_creation_success(character.to_bytes_le(), &name, &[]),
                    // The client reconnects naming the map it believes it is in, and that belief came
                    // from the transfer, which named the creation instance: without this it dials back
                    // into the creation screen and waits there forever (measured in this server's own
                    // capture, 2026-10-28, and again after the first attempt with the wrong shape).
                    //
                    // The **login** shape, five bytes: `map_update_current_redirect` is the
                    // four-byte-header generation (a `u16` zero, then two dwords) and this client
                    // parses two-byte headers, so it ignored that one and kept dialling map 0
                    // (`docs/03-protocol.md` #5e).
                    game::map_update_current(outpost as u16),
                ],
                trace,
            )
            .await?;
        }
        Err(err) => {
            eprintln!("world: could not create {name:?}: {err}");
            send(
                stream,
                session,
                vec![game::char_creation_error(
                    GM_ERROR_CHARACTER_NAME_ALREADY_EXIST,
                )],
                trace,
            )
            .await?;
        }
    }
    Ok(true)
}
