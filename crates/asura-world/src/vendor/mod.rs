//! Talking to an NPC: the window it opens, the trades made in it, and the gold that moves.
//!
//! A talk starts with the walk (`crate::approach`): spacebar or a click on an NPC walks the player
//! there, and the arrival - or a player already standing there - is an order to this connection
//! (`crate::travel::Order::Talk`), which answers it here with [`talk`]. What an NPC opens is content:
//! `config/World/<pack>/vendors.json` names merchants, collectors and Xunlai storage NPCs by map and
//! name (`content`). The rules of a trade are pure functions (`rules`); this module runs them, writes
//! the result in one transaction (`crate::item::commit_ops`) and sends what the live server sends.
//!
//! What each answer is, and where it was measured (`docs/53-npc-services.md`):
//!
//! | event | frames | evidence |
//! |---|---|---|
//! | merchant talk | `0x00C4`, `0x0161` per stock row, `0x0084`, `0x00CA {1, 1.0}`, `0x00C3 {11, 0}`, `0x009F {12, npc, 0}` | live, seq 5281-5288; preforged capture 487607-487618 |
//! | collector talk | `0x00C4`, `0x0161` cost row and one per offer, `0x0084`, `0x00C5 {2, line}`, `0x009F {12, npc, 0}` | preforged capture 537230-537236 |
//! | purchase | `0x014F` price, `0x0162` new item, `0x013E` its place, `0x00CC {1}` | preforged capture 487667-487670 |
//! | exchange | `0x0139` or `0x014D` per trophy stack, `0x0162` reward, `0x013E`, `0x00CC {2}` | preforged capture 537263-537266 |
//! | sale | `0x014D` or `0x0139`, `0x0140` price, `0x00CC {11}` | Rurik's retail sales (`docs/42` section 4) |
//! | Xunlai talk | `0x0083 {npc, 0, 3}` | preforged's client run, not a live capture |
//! | dialog talk | `0x0080` body, `0x0081` sender, one `0x007E` per choice, `0x009F {12, npc, 0}` | live seq 5176-5185 (`docs/55`) |
//! | dialog choice | `0x0083 {npc, kind, 0xCCCCCCCC}` then `0x0081`, or the merchant window then `0x0081` and `0x009F` | live seq 5245-5248, 5277-5288 |
//! | gold at Xunlai | `0x014F` / `0x0150` debits, then `0x0140` / `0x0141` credits | preforged's reading |
//!
//! A refused request is not answered: no live refusal is captured, and preforged answers none.

pub mod content;
pub mod rules;

/// The protocol half of this module: builders and readers (`asura_protocol::game::vendor`).
pub use asura_protocol::game::vendor as protocol;

use crate::inventory_model::{BagKind, Change, Entry, ItemKind, Op};
use crate::*;
use content::Service;
use rules::{Offered, Plan, Trophy, Window};

/// The window a connection has open.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Open {
    /// None.
    #[default]
    Nothing,
    /// A trade window: a merchant's or a collector's, with what it offered under which id.
    Trade {
        /// The NPC it belongs to.
        npc: u32,
        /// What it offered.
        window: Window,
    },
    /// The Xunlai storage window. `npc` is the player's own agent for the `/xunlai` test command.
    Storage {
        /// The agent the window belongs to.
        npc: u32,
    },
    /// An NPC's dialog, on `page` of the dialog the pack gives that NPC.
    Dialog {
        /// The agent saying it.
        npc: u32,
        /// The page a choice moved to.
        page: usize,
    },
}

/// A connection's windows: the one open now, and the ids each NPC's window has already declared.
///
/// The live server declares a window's rows (`0x0161`) the first time a player opens it on a map and
/// lists the same ids again without declaring them when the player comes back: collector 0x21's window
/// is opened at preforged's record 467378 with three `0x0161` and at 470670 with none, the owner, `0x0084`
/// and `0x00C5` alone. Kept per connection, which is per map.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Windows {
    /// The window open now.
    pub open: Open,
    /// The ids each NPC's window declared, by the NPC's agent.
    declared: std::collections::BTreeMap<u32, Vec<u32>>,
}

impl Open {
    /// The agent whose window is open, if any.
    pub fn npc(&self) -> Option<u32> {
        match self {
            Open::Nothing => None,
            Open::Trade { npc, .. } | Open::Storage { npc } | Open::Dialog { npc, .. } => Some(*npc),
        }
    }
}

/// Whether the player stands within the pack's service range of `npc`. The player's own agent is always
/// in range (the `/xunlai` window).
fn in_reach(player: &Player, npc: u32, vendors: &content::Vendors) -> bool {
    if npc == player.agent_id {
        return true;
    }
    let Some(target) = player.instance.npc_target(npc) else {
        return false;
    };
    let mut range = vendors.rules.service_range.value;
    // An NPC whose dialog names a longer talk range (one on a stand) is in reach from that far too.
    if let Some(talk_range) = target
        .name_text()
        .and_then(|name| crate::dialog::talk_range_for(player.instance.map.map_id, &name))
    {
        range = range.max(talk_range);
    }
    let here = player.standing().position;
    crate::agent::distance_between(here, target.position) <= range
}

/// Whether a storage window is open and its owner still in reach: what lets an item move into or out of
/// storage (`crate::inventory_model::Wearer::storage_open`).
pub fn storage_open(player: &Player) -> bool {
    let Open::Storage { npc } = player.vendor.open else {
        return false;
    };
    content::current().is_some_and(|vendors| in_reach(player, npc, vendors))
}

/// The ids a window lists its rows under, and whether they still need declaring: the ids this NPC's
/// window declared before on this connection, or fresh item ids from the item sequence, so no row of a
/// window is ever an id the client holds for an owned item.
async fn window_ids(
    world: &World,
    player: &mut Player,
    npc: u32,
    count: usize,
) -> Result<(Vec<u32>, bool), GameError> {
    if let Some(ids) = player.vendor.declared.get(&npc) {
        if ids.len() == count {
            return Ok((ids.clone(), false));
        }
    }
    let ids = fresh_ids(world, count).await?;
    player.vendor.declared.insert(npc, ids.clone());
    Ok((ids, true))
}

/// Fresh item ids from the item sequence.
async fn fresh_ids(world: &World, count: usize) -> Result<Vec<u32>, GameError> {
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        let id = db_inventory::allocate_item_id(&world.pool)
            .await
            .map_err(GameError::Database)?;
        ids.push(u32::try_from(id).unwrap_or(0));
    }
    Ok(ids)
}

/// Open a merchant's window: declare its rows the first time, list the ids, then the buy pane and the
/// sell tab, exactly as the live server sends them (module docs). The caller sends the frames.
async fn open_merchant(
    world: &World,
    player: &mut Player,
    npc: u32,
    name: &str,
    vendors: &content::Vendors,
    merchant: &content::Merchant,
) -> Result<Vec<Frame>, GameError> {
    let rows: Vec<&content::ItemRow> = merchant
        .stock
        .iter()
        .filter_map(|key| vendors.item(key))
        .collect();
    let (ids, declare) = window_ids(world, player, npc, rows.len()).await?;
    let mut frames = vec![protocol::window_owner(npc)];
    let mut stock = Vec::with_capacity(rows.len());
    for (row, id) in rows.iter().zip(&ids) {
        if declare {
            frames.push(game::create_named_item(&row.named(*id, 1)));
        }
        stock.push(Offered {
            id: *id,
            key: row.id.clone(),
            value: row.value,
            flags: row.flags,
            profession: 0,
        });
    }
    frames.push(protocol::window_add_items(&ids));
    frames.push(protocol::window_buy_pane(
        protocol::BUY_PANE_MERCHANT,
        vendors.rules.buy_scale.value,
    ));
    frames.push(protocol::merchant_window_open(protocol::KIND_SELL, 0));
    frames.push(protocol::talked_to(npc));
    eprintln!(
        "world: {} opened {name:?}'s window: {} item(s)",
        player.name,
        stock.len()
    );
    player.vendor.open = Open::Trade {
        npc,
        window: Window::Merchant {
            stock,
            unsellable_models: vendors.unsellable_models.clone(),
        },
    };
    Ok(frames)
}

/// Send the quest marker a quest NPC should carry now, after an answer to a talk or a choice.
///
/// The client drops the marker when it is talked to; the marker follows the character's quest state,
/// not the talk, so it is drawn again (`crate::quest::refresh_marker`). Any other NPC gets nothing.
async fn refresh_quest_marker(
    stream: &mut TcpStream,
    session: &mut Session,
    player: &Player,
    npc: u32,
    trace: bool,
) -> Result<(), GameError> {
    if let Some(frame) = crate::quest::refresh_marker(player, npc) {
        send(stream, session, vec![frame], trace).await?;
    }
    Ok(())
}

/// Answer a talk: the player is at `npc`. A dialog the pack gives the NPC opens first; otherwise the
/// NPC's window, and an NPC with neither answers with nothing. Any window already open is closed. A quest
/// NPC's marker is drawn again afterwards, from the character's quest state.
pub async fn talk(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    npc: u32,
    trace: bool,
) -> Result<bool, GameError> {
    let answered = answer_talk(stream, session, world, player, npc, trace).await?;
    refresh_quest_marker(stream, session, player, npc, trace).await?;
    Ok(answered)
}

async fn answer_talk(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    npc: u32,
    trace: bool,
) -> Result<bool, GameError> {
    player.vendor.open = Open::Nothing;
    let Some(target) = player.instance.npc_target(npc) else {
        return Ok(true);
    };
    let map_id = player.instance.map.map_id;
    let Some(name) = target.name_text() else {
        if trace {
            eprintln!(
                "world: {} talks to {} ({}), which has no literal name; no window",
                player.name, npc, target.placement
            );
        }
        return Ok(true);
    };
    // A dialog comes first: The Purveyor's captured menu is what opens its merchant window, so the
    // window is behind a choice and not a talk (`movement-live-20261109-200703.jsonl` seq 5176-5288).
    if let Some(dialog) = crate::dialog::current().and_then(|file| file.find(map_id, &name)) {
        // The first page whose quest condition is open. A closed page is what the live server would
        // not have shown: the Town Crier's offer after a character has accepted the quest, for
        // instance. No open page falls through to a merchant or storage window.
        if let Some((page_index, page)) = dialog
            .pages
            .iter()
            .enumerate()
            .find(|(_, page)| page.is_open(&crate::quest::Who::of(player)))
        {
            // The reach check is the vendor rule's; a dialog does not need the trade file, so a pack
            // whose vendors.json does not load still talks (the rule is then the walk's own range).
            if !content::current().is_none_or(|vendors| in_reach(player, npc, vendors)) {
                eprintln!(
                    "world: {} is out of reach of {name:?}; no dialog",
                    player.name
                );
                return Ok(true);
            }
            let frames = page.frames(npc, &crate::quest::Who::of(player));
            eprintln!(
                "world: {} opened {name:?}'s dialog: {} choice(s)",
                player.name,
                page.buttons.len()
            );
            player.vendor.open = Open::Dialog {
                npc,
                page: page_index,
            };
            send(stream, session, frames, trace).await?;
            return Ok(true);
        }
    }
    let Some(vendors) = content::current() else {
        return Ok(true);
    };
    let Some(service) = vendors.service(map_id, &name) else {
        if trace {
            eprintln!(
                "world: {} talks to {name:?} ({}); it opens no window",
                player.name, target.placement
            );
        }
        return Ok(true);
    };
    if !in_reach(player, npc, vendors) {
        eprintln!(
            "world: {} is out of reach of {name:?}; no window",
            player.name
        );
        return Ok(true);
    }
    let frames = match service {
        Service::Merchant(merchant) => {
            open_merchant(world, player, npc, &name, vendors, merchant).await?
        }
        Service::Collector(collector) => {
            let Some(trophy) = vendors.item(&collector.trophy) else {
                return Ok(true);
            };
            // Only the offers for this character's primary profession, and those for any.
            let primary = player.traits.profession;
            let offers: Vec<(&content::Offer, &content::ItemRow)> = collector
                .offers
                .iter()
                .filter(|offer| offer.profession == 0 || offer.profession == primary)
                .filter_map(|offer| Some((offer, vendors.item(&offer.reward)?)))
                .collect();
            if offers.is_empty() {
                if trace {
                    eprintln!("world: {name:?} has no offer for profession {primary}");
                }
                return Ok(true);
            }
            let (ids, declare) = window_ids(world, player, npc, offers.len() + 1).await?;
            let mut frames = vec![protocol::window_owner(npc)];
            if declare {
                frames.push(game::create_named_item(
                    &trophy.named(ids[0], collector.cost),
                ));
            }
            let mut window_offers = Vec::with_capacity(offers.len());
            for ((offer, row), id) in offers.iter().zip(&ids[1..]) {
                if declare {
                    frames.push(game::create_named_item(&row.named(*id, 1)));
                    if offer.profession != 0 {
                        frames.push(game::item_set_profession(*id, offer.profession));
                    }
                }
                window_offers.push(Offered {
                    id: *id,
                    key: row.id.clone(),
                    value: row.value,
                    flags: row.flags,
                    profession: offer.profession,
                });
            }
            frames.push(protocol::window_add_items(&ids));
            frames.push(protocol::window_collector(
                protocol::KIND_COLLECTOR,
                &collector.heading,
            ));
            frames.push(protocol::talked_to(npc));
            eprintln!(
                "world: {} opened collector {name:?}: {} offer(s)",
                player.name,
                window_offers.len()
            );
            player.vendor.open = Open::Trade {
                npc,
                window: Window::Collector {
                    trophy: Trophy {
                        item_type: trophy.item_type,
                        model_id: trophy.model_id,
                        name: trophy.name.clone(),
                        cost: collector.cost,
                    },
                    offers: window_offers,
                },
            };
            frames
        }
        Service::Storage => {
            eprintln!("world: {} opened Xunlai storage at {name:?}", player.name);
            player.vendor.open = Open::Storage { npc };
            vec![protocol::storage_window_open(npc)]
        }
    };
    send(stream, session, frames, trace).await?;
    Ok(true)
}

/// What the rules read of the character's items: everything, with whether it sits in a carrying bag.
fn owned_items(player: &Player) -> Vec<rules::Owned> {
    player
        .inventory
        .items
        .iter()
        .filter_map(|item| {
            let carried = item
                .bag_id
                .and_then(|bag| player.inventory.bag(bag))
                .is_some_and(|bag| BagKind::of(bag.bag_model_id) == BagKind::Carrying);
            Some(rules::Owned {
                id: u32::try_from(item.id).ok()?,
                carried,
                item_type: u8::try_from(item.item_type).ok()?,
                model_id: u32::try_from(item.model_id).ok()?,
                name: item.name_units(),
                value: u32::try_from(item.value).unwrap_or(0),
                quantity: u32::try_from(item.quantity).unwrap_or(0),
            })
        })
        .collect()
}

/// `GAME_CMSG_TRANSACT_ITEMS` (`0x804D`) and `GAME_CMSG_BUY_MATERIALS` (`0x804A`): a trade in the open
/// window. Decided by `rules::authorize`, written in one transaction, answered as the live server
/// answers (module docs). A refused request changes nothing and is not answered.
pub async fn handle_transaction(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(request) = protocol::read_transaction(frame.header(), frame.body()) else {
        eprintln!(
            "world: a trade request of {} bytes does not read; ignored",
            frame.body().len()
        );
        return Ok(true);
    };
    let Open::Trade { npc, window } = player.vendor.open.clone() else {
        eprintln!(
            "world: {} sent a trade with no trade window open; ignored",
            player.name
        );
        return Ok(true);
    };
    let Some(vendors) = content::current() else {
        return Ok(true);
    };
    if !in_reach(player, npc, vendors) {
        eprintln!(
            "world: {} is out of reach of the window's NPC; trade refused",
            player.name
        );
        player.vendor.open = Open::Nothing;
        return Ok(true);
    }
    let plan = match rules::authorize(
        &window,
        &request,
        &owned_items(player),
        player.traits.gold,
        player.traits.profession,
        &vendors.limits(),
    ) {
        Ok(plan) => plan,
        Err(refused) => {
            eprintln!(
                "world: {}'s trade {request:?} refused: {refused:?}",
                player.name
            );
            return Ok(true);
        }
    };
    let gold = player.traits.gold;
    let gold_row = |to: u32| db_inventory::RowChange::Gold {
        from: i32::try_from(gold).unwrap_or(i32::MAX),
        to: i32::try_from(to).unwrap_or(i32::MAX),
    };
    let stream_id = crate::item::ITEM_STREAM;
    let frames = match plan {
        Plan::Buy {
            key,
            quantity,
            price,
            stack_max,
        } => {
            let Some(row) = vendors.item(&key) else {
                return Ok(true);
            };
            let id = db_inventory::allocate_item_id(&world.pool)
                .await
                .map_err(GameError::Database)?;
            let quantity_row = i32::try_from(quantity).unwrap_or(1);
            let new_row = row.owned(id, quantity_row, 0);
            let entry = Entry {
                id: u32::try_from(id).unwrap_or(0),
                at: None,
                kind: ItemKind::of(row.item_type),
                profession: 0,
                quantity,
                stack_max,
            };
            let changes = match crate::item::commit_ops(
                world,
                player,
                &[Op::Insert { entry }],
                std::slice::from_ref(&new_row),
                vec![gold_row(gold - price)],
            )
            .await?
            {
                Ok(changes) => changes,
                Err(refusal) => {
                    eprintln!(
                        "world: {}'s purchase of {key} refused: {refusal:?}",
                        player.name
                    );
                    return Ok(true);
                }
            };
            player.traits.gold = gold - price;
            let mut frames = vec![protocol::gold_character_remove(stream_id, price)];
            frames.push(game::item_update(&row.named(entry.id, quantity)));
            frames.extend(placed(&changes, stream_id));
            frames.push(game::guild::trader_transaction_done(protocol::KIND_BUY));
            eprintln!(
                "world: {} bought {quantity} {key} for {price} gold as item {id}",
                player.name
            );
            frames
        }
        Plan::Sell {
            item,
            quantity,
            whole: _,
            price,
        } => {
            let changes = match crate::item::commit_ops(
                world,
                player,
                &[Op::Take {
                    item,
                    count: quantity,
                }],
                &[],
                vec![gold_row(gold + price)],
            )
            .await?
            {
                Ok(changes) => changes,
                Err(refusal) => {
                    eprintln!(
                        "world: {}'s sale of item {item} refused: {refusal:?}",
                        player.name
                    );
                    return Ok(true);
                }
            };
            player.traits.gold = gold + price;
            let mut frames = taken(&changes, stream_id);
            frames.push(protocol::gold_character_add(stream_id, price));
            frames.push(game::guild::trader_transaction_done(protocol::KIND_SELL));
            eprintln!(
                "world: {} sold {quantity} of item {item} for {price} gold",
                player.name
            );
            frames
        }
        Plan::Exchange { debits, key } => {
            let Some(row) = vendors.item(&key) else {
                return Ok(true);
            };
            let profession = match &window {
                Window::Collector { offers, .. } => offers
                    .iter()
                    .find(|offer| offer.key == key)
                    .map_or(0, |offer| offer.profession),
                Window::Merchant { .. } => 0,
            };
            let id = db_inventory::allocate_item_id(&world.pool)
                .await
                .map_err(GameError::Database)?;
            let new_row = row.owned(id, 1, profession);
            let mut ops: Vec<Op> = debits
                .iter()
                .map(|&(item, count)| Op::Take { item, count })
                .collect();
            let reward = Entry {
                id: u32::try_from(id).unwrap_or(0),
                at: None,
                kind: ItemKind::of(row.item_type),
                profession,
                quantity: 1,
                stack_max: 1,
            };
            ops.push(Op::Insert { entry: reward });
            let changes = match crate::item::commit_ops(
                world,
                player,
                &ops,
                std::slice::from_ref(&new_row),
                Vec::new(),
            )
            .await?
            {
                Ok(changes) => changes,
                Err(refusal) => {
                    eprintln!(
                        "world: {}'s exchange for {key} refused: {refusal:?}",
                        player.name
                    );
                    return Ok(true);
                }
            };
            let mut frames = taken(&changes, stream_id);
            frames.push(game::item_update(&row.named(reward.id, 1)));
            if profession != 0 {
                frames.push(game::item_set_profession(reward.id, profession));
            }
            frames.extend(placed(&changes, stream_id));
            frames.push(game::guild::trader_transaction_done(
                protocol::KIND_COLLECTOR,
            ));
            eprintln!(
                "world: {} exchanged trophies for {key} as item {id}",
                player.name
            );
            frames
        }
    };
    send(stream, session, frames, trace).await?;
    Ok(true)
}

/// What left the bags: `0x0139` for a stack that stays, `0x014D` for one that is gone.
fn taken(changes: &[Change], stream_id: u16) -> Vec<Frame> {
    changes
        .iter()
        .filter_map(|change| match *change {
            Change::Quantity { item, quantity } => Some(game::item_update_quantity(item, quantity)),
            Change::Removed { item } => Some(game::item_remove(stream_id, item)),
            _ => None,
        })
        .collect()
}

/// Where a new item went: `0x013E`, as the live purchase places it (record 487669).
fn placed(changes: &[Change], stream_id: u16) -> Vec<Frame> {
    changes
        .iter()
        .filter_map(|change| match *change {
            Change::Inserted { item, at } => {
                Some(game::item_move_to_location(stream_id, item, at.0, at.1))
            }
            _ => None,
        })
        .collect()
}

/// `GAME_CMSG_ITEM_CHANGE_GOLD` (`0x807C`): at a Xunlai window, the carried and stored balances the
/// client asks to end with. Accepted when the total is kept and neither cap is passed
/// (`rules::move_gold`); written in one transaction; answered with the debits, then the credits.
pub async fn handle_change_gold(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(requested) = protocol::read_change_gold(frame.body()) else {
        eprintln!(
            "world: a gold change of {} bytes does not read; ignored",
            frame.body().len()
        );
        return Ok(true);
    };
    let Some(vendors) = content::current() else {
        return Ok(true);
    };
    if !storage_open(player) {
        eprintln!(
            "world: {} moved gold with no storage window open; refused",
            player.name
        );
        return Ok(true);
    }
    let before = (player.traits.gold, player.storage_gold);
    let after = match rules::move_gold(before, requested, &vendors.limits()) {
        Ok(after) => after,
        Err(refused) => {
            eprintln!(
                "world: {}'s gold move {before:?} -> {requested:?} refused: {refused:?}",
                player.name
            );
            return Ok(true);
        }
    };
    if after == before {
        return Ok(true);
    }
    let as_i32 = |gold: u32| i32::try_from(gold).unwrap_or(i32::MAX);
    db_inventory::apply_changes(
        &world.pool,
        player.character,
        &[
            db_inventory::RowChange::Gold {
                from: as_i32(before.0),
                to: as_i32(after.0),
            },
            db_inventory::RowChange::StorageGold {
                from: as_i32(before.1),
                to: as_i32(after.1),
            },
        ],
    )
    .await
    .map_err(GameError::Database)?;
    player.traits.gold = after.0;
    player.storage_gold = after.1;
    eprintln!(
        "world: {} moved gold: carried {} -> {}, stored {} -> {}",
        player.name, before.0, after.0, before.1, after.1
    );
    let frames = protocol::gold_deltas(crate::item::ITEM_STREAM, before, after);
    send(stream, session, frames, trace).await?;
    Ok(true)
}

/// `GAME_CMSG_SEND_DIALOG` (`0x803B`): the player chose a dialog button. The open dialog, the NPC it
/// belongs to and the choice's action decide the answer (`docs/55-npc-dialogs.md`):
///
/// * `close` answers nothing, as the capture never exercised it;
/// * `window` opens `0x0083 {npc, kind, 0xCCCCCCCC}` and names the speaker (`0x0081`), the live order at
///   seq 5247-5248;
/// * `merchant` opens the NPC's window through `vendors.json`, with the speaker and the talked-to
///   property in the live order (seq 5281-5288);
/// * `page` shows another page;
/// * `quest` with step `offer` accepts the quest and puts it in the log, and with step `reward` pays it
///   and removes it (`crate::quest`, `docs/58-quests.md`);
/// * `unanswered` is logged and answers nothing, because the capture never exercised it.
///
/// Whatever the choice did, a quest NPC's marker is drawn again afterwards from the character's quest
/// state: closing the Town Crier's offer leaves his marker up, accepting it takes it down.
pub async fn handle_send_dialog(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    frame: &Frame,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let speaker = match player.vendor.open {
        Open::Dialog { npc, .. } => Some(npc),
        _ => None,
    };
    let answered = answer_dialog(stream, session, world, frame, player, trace).await?;
    if let Some(npc) = speaker {
        refresh_quest_marker(stream, session, player, npc, trace).await?;
    }
    Ok(answered)
}

async fn answer_dialog(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    frame: &Frame,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(choice) = asura_protocol::game::dialog::read_send_dialog(frame.body()) else {
        eprintln!(
            "world: a dialog choice of {} bytes does not read; ignored",
            frame.body().len()
        );
        return Ok(true);
    };
    let Open::Dialog { npc, page } = player.vendor.open else {
        eprintln!(
            "world: {} chose dialog {choice:#x} with no dialog open; ignored",
            player.name
        );
        return Ok(true);
    };
    let Some(target) = player.instance.npc_target(npc) else {
        player.vendor.open = Open::Nothing;
        return Ok(true);
    };
    let Some(name) = target.name_text() else {
        player.vendor.open = Open::Nothing;
        return Ok(true);
    };
    let map_id = player.instance.map.map_id;
    let Some(dialog) = crate::dialog::current().and_then(|file| file.find(map_id, &name)) else {
        player.vendor.open = Open::Nothing;
        return Ok(true);
    };
    let Some(current) = dialog.pages.get(page) else {
        player.vendor.open = Open::Nothing;
        return Ok(true);
    };
    // A choice the character was not shown (a quest offer it is not eligible for, a turn-in that is not
    // open) is refused the same way as one the page does not have.
    let Some(button) = current
        .visible(&crate::quest::Who::of(player))
        .find(|button| button.id == choice)
    else {
        eprintln!(
            "world: {} chose dialog {choice:#x}, which {name:?}'s page {page} does not offer",
            player.name
        );
        return Ok(true);
    };
    player.vendor.open = Open::Nothing;
    if trace {
        eprintln!(
            "world: {} chose dialog {choice:#x} ({:?}) at {name:?}",
            player.name, button.action
        );
    }
    // A quest choice is an operation, not a frame batch: it persists a row and then sends. The NPC's
    // name and the map are what the pack's offer and reward blocks are checked against.
    if let crate::dialog::Action::Quest { quest, step, page } = button.action.clone() {
        match step {
            crate::dialog::QuestStep::Offer => {
                crate::quest::handle_offer(
                    stream, session, world, player, quest, npc, &name, map_id, trace,
                )
                .await?;
            }
            crate::dialog::QuestStep::Reward => {
                crate::quest::handle_reward(
                    stream, session, world, player, quest, npc, &name, map_id, trace,
                )
                .await?;
            }
            crate::dialog::QuestStep::Credit => {
                crate::quest::handle_credit(
                    stream, session, world, player, quest, &name, map_id, trace,
                )
                .await?;
            }
        }
        // What the NPC says next: the page the step names (a trainer's reply), or else the first page
        // that now offers or rewards a quest - the live server opened Sir Tydus's War Preparations
        // offer 750 ms after Message from a Friend's turn-in, and Van's Warrior Test after War
        // Preparations'. Nothing open closes the dialog.
        let who = crate::quest::Who::of(player);
        let next = match page {
            Some(index) => dialog.pages.get(index).map(|next| (index, next)),
            None => dialog
                .pages
                .iter()
                .enumerate()
                .find(|(_, next)| next.is_open(&who) && next.has_quest_step(&who)),
        };
        if let Some((index, next)) = next {
            let frames = next.frames(npc, &who);
            player.vendor.open = Open::Dialog { npc, page: index };
            send(stream, session, frames, trace).await?;
        }
        return Ok(true);
    }
    let frames = match &button.action {
        crate::dialog::Action::Close => Vec::new(),
        crate::dialog::Action::Unanswered => {
            eprintln!(
                "world: {name:?}'s dialog {choice:#x} has no captured answer; nothing opens"
            );
            Vec::new()
        }
        crate::dialog::Action::Quest { .. } => Vec::new(),
        crate::dialog::Action::Page { page } => {
            let Some(next) = dialog.pages.get(*page) else {
                return Ok(true);
            };
            player.vendor.open = Open::Dialog { npc, page: *page };
            next.frames(npc, &crate::quest::Who::of(player))
        }
        crate::dialog::Action::Window { window } => vec![
            protocol::window_open(npc, *window, protocol::WINDOW_DIALOG_PERMISSION),
            asura_protocol::game::dialog::dialog_sender(npc),
        ],
        crate::dialog::Action::Merchant => {
            let Some(vendors) = content::current() else {
                return Ok(true);
            };
            let Some(Service::Merchant(merchant)) = vendors.service(map_id, &name) else {
                eprintln!(
                    "world: {name:?}'s dialog {choice:#x} opens a merchant window, but vendors.json has no stock for it"
                );
                return Ok(true);
            };
            let mut frames = open_merchant(world, player, npc, &name, vendors, merchant).await?;
            // The live choice's order: the window, then the speaker, then the talked-to property
            // (seq 5281-5288). `open_merchant` ends with the property, so put the speaker before it.
            if let Some(talked_to) = frames.pop() {
                frames.push(asura_protocol::game::dialog::dialog_sender(npc));
                frames.push(talked_to);
            }
            frames
        }
    };
    send(stream, session, frames, trace).await?;
    Ok(true)
}

/// The `/xunlai` test command: open the storage window on the player's own agent, so storage and its
/// gold can be tried on a map with no Xunlai NPC (every pre-Searing map). Not the live server's: no NPC
/// owns this window, and the client's handling of its own agent as the owner is not measured.
pub async fn open_own_storage(
    stream: &mut TcpStream,
    session: &mut Session,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    player.vendor.open = Open::Storage {
        npc: player.agent_id,
    };
    eprintln!("world: {} opened storage with /xunlai", player.name);
    send(
        stream,
        session,
        vec![protocol::storage_window_open(player.agent_id)],
        trace,
    )
    .await?;
    Ok(true)
}
