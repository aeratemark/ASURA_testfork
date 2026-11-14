//! The instance: one hosted map, running.
//!
//! `docs/02-architecture.md` "Inside a world server": a map is updated by one owner, its state needs
//! no locking across tasks, and the update threads have map affinity. This is that owner - the
//! object a `MapEntry` describes, and the thing a `Player` stands in.
//!
//! It holds who is in the map, the agent each one drives, and the clock. The tick is the reference's
//! `GameSrv_WorldTick`: announce the elapsed time to every client, advance every agent along its
//! walk, and broadcast a crossed waypoint to everyone in the instance. One clock per map, as the
//! reference has one loop for the whole server.

use crate::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// An actual movement segment that touched a doorway during simulation.
pub type PortalStep = ((f32, f32), (f32, f32));

/// One player in one instance: the agent it drives, what the client is told about it, and the
/// channel that reaches its connection.
pub struct Slot {
    pub player_id: u32,
    pub agent: AgentState,
    pub name: String,
    pub equipped: [u32; 9],
    pub outbox: tokio::sync::mpsc::UnboundedSender<Vec<Frame>>,
    /// The channel that reaches its connection with something to *do* rather than to send: a party's
    /// travel, which each member's own connection carries out (`travel::Order`).
    pub orders: tokio::sync::mpsc::UnboundedSender<crate::travel::Order>,
    /// Crossing events go to the connection that owns the transfer, without waiting for a report.
    pub portal_outbox: Option<tokio::sync::mpsc::UnboundedSender<PortalStep>>,
    /// Movement counters, for the trace's closing summary.
    pub move_stats: MoveStats,
    /// Set at join and cleared when the connecting player has its load block, so the clock does not
    /// put a tick inside the block: the reference finishes the load with `INSTANCE_LOAD_FINISH`
    /// (`GameSrv_HandleInstanceLoadRequestPlayers`) before it starts announcing the world clock.
    pub loading: bool,
    /// Set when this player's players block took the list of the others (`Instance::enter`): from then
    /// on the others in the map have been introduced to it and it to them, so what is broadcast about
    /// an agent reaches it. A client still loading is not sent another player's introduction or agent
    /// traffic: its own players block carries the map as it is then, and a second `PLAYER_CREATE` or
    /// agent create for one it already has is what crashed both clients of a party arriving together
    /// (`logs/world-8001.log`, 2026-11-06, map 146 and map 163).
    pub entered: bool,
    /// The title rank this player shows under its name, which a joining player is told in the
    /// introduction (`crate::titles`). `None` until the connection says otherwise.
    pub shown_title: Option<asura_rules::titles::Rank>,
    /// What another client needs in order to draw this player: its own appearance, its professions and
    /// its level. Default (all zero) until the connection has read the character's traits.
    pub look: Look,
    /// The items it is wearing, as another client is told them (`0x015E` each, before its visual
    /// equipment). Empty until the connection has read the inventory.
    pub wearing: Vec<game::NamedItem>,
    /// The guild another client is told this player is in (`crate::guild`). `None` until the connection
    /// has read it, and for a player in no guild.
    pub guild: Option<crate::guild::Badge>,
    /// The agent this player is walking to, if any (`crate::approach`): an NPC it interacts with, a foe
    /// it attacks, a player it follows. The tick steers it.
    pub approach: Option<crate::approach::Approach>,
}

/// A player as another client draws it. Retail introduces every other player with its profession
/// (`0x00A6`) and its level (`0x009F` property 36) before the agent exists
/// (`recordings/client/feature/maps/pre-searing-load-3.jsonl` seq 215-216, 304-305, 1007-1008,
/// 1341-1342), and with its own appearance in `PLAYER_CREATE`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Look {
    /// The packed appearance bits; 0 means "none recorded", and the introduction falls back to the
    /// character defaults, as the player's own block does.
    pub appearance: u32,
    pub primary: u8,
    pub secondary: u8,
    pub level: u32,
}

impl Look {
    /// A character's look, from its stored traits.
    pub fn from_traits(traits: &db_game::Traits) -> Look {
        Look {
            appearance: traits.appearance,
            primary: traits.profession,
            secondary: traits.secondary_profession,
            level: u32::from(traits.level),
        }
    }
}

/// An NPC body as a talk sees it ([`Instance::npc_target`]).
#[derive(Debug, Clone, PartialEq)]
pub struct NpcTarget {
    /// Its agent.
    pub agent_id: u32,
    /// The map-local key of the placement it was made from (`"148-10050"`).
    pub placement: String,
    /// The placement's own name, the coded units its `0x009B` carries; `None` for a body that has none.
    pub name: Option<Vec<u16>>,
    /// Where it stands now.
    pub position: (f32, f32),
}

impl NpcTarget {
    /// The name as text, when it is a literal: the `0x0108 0x0107 ... 0x0001` form a pack writes for a
    /// placement (`tools/npcs-from-preforged.py`). `None` for a coded string id the server cannot read.
    pub fn name_text(&self) -> Option<String> {
        let units = self.name.as_deref()?;
        let inner = units.strip_prefix(&[0x0108, 0x0107])?;
        let inner = inner.strip_suffix(&[0x0001]).unwrap_or(inner);
        Some(
            char::decode_utf16(inner.iter().copied())
                .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect(),
        )
    }
}

/// One item lying on the ground: the record the client draws, where it lies, and the agent it is.
///
/// A drop is not a bag row. The live server takes the whole stack's row out of the bag and keeps its id
/// for the ground agent (`items-live-20261108-173913.jsonl` seq 41-43), or keeps a partial stack and
/// gives the ground part a fresh id (`items-live-20261108-174014.jsonl` seq 133-136). Both forms are
/// carried here: `source_item_id` is `Some` exactly for the partial form, the stack the pickup merges
/// back into.
#[derive(Debug, Clone)]
pub struct GroundItem {
    /// The agent the client interacts with and `WORLD_REMOVE_AGENT` takes away.
    pub agent_id: u32,
    /// The item record the ground agent's `model_id` names. Its `bag_id` and `slot` are `None`.
    pub item: db_inventory::Item,
    /// The bag row the dropped part came from, when that row stayed behind. `None` for a whole drop.
    pub source_item_id: Option<i32>,
    /// Where it lies.
    pub position: (f32, f32),
    /// The pathing plane, not a height.
    pub plane: u16,
    /// Which way it faces, as the unit vector the live server uses for a dropped item: `(1, 0)`.
    pub direction: (f32, f32),
}

/// One hosted map: its description, and everyone standing in it.
pub struct Instance {
    /// The map this instance runs: what a client is told it is entering (`MapEntry`).
    pub map: MapEntry,
    /// The players in it, by player id. The lock is the std one: the critical sections are a scan
    /// and a send into an unbounded channel, never an await point.
    slots: Mutex<HashMap<u32, Slot>>,
    /// The next agent id handed out for something that is not a joining avatar.
    next_agent_id: Mutex<u32>,
    /// Items lying on the ground, by agent id. The instance owns them, not the bag: a client that
    /// leaves and comes back finds them where it left them, and every client in the map is told.
    ground_items: Mutex<HashMap<u32, GroundItem>>,
    /// The parties of the players in it (`crate::party`).
    pub parties: crate::party::Parties,
    /// The local ids its messages name guilds by (`crate::guild`).
    pub guild_ids: crate::guild::LocalIds,
    /// Player ids kept for characters that are not in the instance: a party member still travelling in,
    /// or one whose party holds its place after a drop (`crate::party`). Its lock is taken after the
    /// slots' and never the other way round.
    reserved: Mutex<HashMap<Uuid, u32>>,
    /// The NPCs, seeded when the instance is created (`crate::npc`). Its lock is taken after the slots'
    /// and the ground items', never before them.
    npcs: Mutex<crate::npc::NpcTable>,
    /// The graphs an NPC's walk and a player's walk to an agent ask (`crate::approach`); empty for an
    /// instance made without the world's NPCs ([`Instance::new`]).
    graphs: PathGraphs,
    /// The fight: every player and every NPC with a combat block, their vitals, casts and effects
    /// (`crate::skill`). Its lock is taken after the slots' and the NPCs', never before them, and a
    /// request handler that takes it alone drops it before it delivers anything.
    combat: Mutex<crate::skill::Combat>,
}

impl Instance {
    /// An instance with no NPCs.
    pub fn new(map: MapEntry) -> Instance {
        Instance::with_npcs(map, None)
    }

    /// An instance whose map's NPCs, if the pack has any, exist from now on (`docs/36` R29): each
    /// placement becomes a body under the next free agent id, before any player can join.
    pub fn with_npcs(map: MapEntry, npcs: Option<&crate::npc::NpcWorld>) -> Instance {
        // The reference's first free agent id; the bodies take the ids from it, in file order.
        let first_agent_id = 2;
        let content = npcs.and_then(|world| Some((world, world.content.map(map.map_id)?)));
        let (table, graphs) = match content {
            Some((world, content)) => {
                let (table, notes) = crate::npc::NpcTable::seed(
                    content,
                    world.content.sight,
                    world.behaviors.as_ref(),
                    first_agent_id,
                );
                for note in notes {
                    eprintln!("world: npc: {note}");
                }
                (table, world.graphs.clone())
            }
            // No NPCs here, but a player still walks to another player on the world's graphs
            // (`crate::approach`).
            None => (
                crate::npc::NpcTable::default(),
                npcs.map(|world| world.graphs.clone())
                    .unwrap_or_else(PathGraphs::empty),
            ),
        };
        let next_agent_id = first_agent_id + table.len() as u32;
        // The NPCs that fight are in the fight from the start, like the bodies themselves.
        let mut combat = crate::skill::Combat::default();
        for (agent_id, block, level, position) in table.combatants() {
            combat
                .engine
                .join(agent_id, crate::skill::npc_spec(&block, level), position);
        }
        Instance {
            map,
            slots: Mutex::new(HashMap::new()),
            next_agent_id: Mutex::new(next_agent_id),
            ground_items: Mutex::new(HashMap::new()),
            parties: crate::party::Parties::default(),
            guild_ids: crate::guild::LocalIds::default(),
            reserved: Mutex::new(HashMap::new()),
            npcs: Mutex::new(table),
            graphs,
            combat: Mutex::new(combat),
        }
    }

    /// The instance's fight (`crate::skill`). Never held across an await, and never while taking the
    /// slots' or the NPCs' lock.
    pub fn combat(&self) -> std::sync::MutexGuard<'_, crate::skill::Combat> {
        self.combat.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn npc_table(&self) -> std::sync::MutexGuard<'_, crate::npc::NpcTable> {
        self.npcs.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Slot>> {
        self.slots.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn reservations(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, u32>> {
        self.reserved.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn grounds(&self) -> std::sync::MutexGuard<'_, HashMap<u32, GroundItem>> {
        self.ground_items
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    /// Keep a player id for a character that is not here yet, and return it. A character that already
    /// has one keeps it. The id is the lowest that no player in the instance and no other reservation
    /// holds.
    pub fn reserve(&self, character: Uuid) -> u32 {
        let slots = self.locked();
        let mut reserved = self.reservations();
        if let Some(player_id) = reserved.get(&character) {
            return *player_id;
        }
        let player_id = (1..)
            .find(|id| !slots.contains_key(id) && !reserved.values().any(|kept| kept == id))
            .unwrap_or(1);
        reserved.insert(character, player_id);
        player_id
    }

    /// Keep the id of a player that is leaving for its character, so it comes back under the same one.
    pub fn reserve_as(&self, character: Uuid, player_id: u32) {
        self.reservations().insert(character, player_id);
    }

    /// Give a character's kept id back.
    pub fn release(&self, character: Uuid) {
        self.reservations().remove(&character);
    }

    /// Hand one player's connection something to do. Returns whether it is still there to take it.
    pub fn order(&self, player_id: u32, order: crate::travel::Order) -> bool {
        self.with_slot(player_id, |slot| slot.orders.send(order).is_ok())
            .unwrap_or(false)
    }

    /// Put a player in the instance and hand out its ids.
    ///
    /// The reference allocates both from per-server tables (`GmIdAllocate`); with a list this short,
    /// the lowest free id is the same thing, and it keeps the single-player case at id 1 exactly as
    /// it was. A character its party kept an id for (`Instance::reserve`) gets that one; nobody else
    /// gets a kept one.
    pub fn join(
        &self,
        name: String,
        character: Uuid,
        standing: Standing,
        equipped: [u32; 9],
        outbox: tokio::sync::mpsc::UnboundedSender<Vec<Frame>>,
        orders: tokio::sync::mpsc::UnboundedSender<crate::travel::Order>,
    ) -> (u32, u32) {
        let mut slots = self.locked();
        let mut reserved = self.reservations();
        let player_id = match reserved.remove(&character) {
            Some(kept) if !slots.contains_key(&kept) => kept,
            _ => (1..)
                .find(|id| !slots.contains_key(id) && !reserved.values().any(|kept| kept == id))
                .unwrap_or(1),
        };
        drop(reserved);
        let ground = self.grounds();
        let npcs = self.npc_table();
        let agent_id = (1..)
            .find(|id| {
                !slots.values().any(|slot| slot.agent.agent_id == *id)
                    && !ground.values().any(|item| item.agent_id == *id)
                    && !npcs.holds(*id)
            })
            .unwrap_or(1);
        drop(npcs);
        drop(ground);
        slots.insert(
            player_id,
            Slot {
                player_id,
                agent: AgentState::new(
                    agent_id,
                    standing.position,
                    standing.plane,
                    standing.direction,
                    game::PLAYER_SPEED_BASE,
                ),
                name,
                equipped,
                outbox,
                orders,
                portal_outbox: None,
                move_stats: MoveStats::default(),
                loading: true,
                entered: false,
                shown_title: None,
                look: Look::default(),
                wearing: Vec::new(),
                guild: None,
                approach: None,
            },
        );
        (player_id, agent_id)
    }

    /// A player's players block is being built: mark it entered and return the players that entered
    /// before it - the ones its block introduces, and the only ones it is introduced to. One lock, so of
    /// two players loading together exactly one introduces the other, and each pair meets once.
    pub fn enter(&self, player_id: u32) -> Vec<u32> {
        let mut slots = self.locked();
        let entered: Vec<u32> = slots
            .values()
            .filter(|slot| slot.entered && slot.player_id != player_id)
            .map(|slot| slot.player_id)
            .collect();
        if let Some(me) = slots.get_mut(&player_id) {
            me.entered = true;
        }
        entered
    }

    /// Mark a player loaded: the load block is out, so the clock may tick it.
    pub fn loaded(&self, player_id: u32) {
        self.with_slot(player_id, |slot| slot.loading = false);
    }

    /// Connect simulation crossings to this player's connection before loading finishes.
    pub fn watch_portals(
        &self,
        player_id: u32,
        outbox: tokio::sync::mpsc::UnboundedSender<PortalStep>,
    ) {
        self.with_slot(player_id, |slot| slot.portal_outbox = Some(outbox));
    }

    /// Whether a player has finished loading.
    pub fn is_loaded(&self, player_id: u32) -> bool {
        self.with_slot(player_id, |slot| !slot.loading)
            .unwrap_or(false)
    }

    /// Set or clear the title rank a player shows, for the introductions that follow.
    pub fn show_title(&self, player_id: u32, rank: Option<asura_rules::titles::Rank>) {
        self.with_slot(player_id, |slot| slot.shown_title = rank);
    }

    /// Record how a player looks, for the introductions that follow.
    pub fn describe(&self, player_id: u32, look: Look) {
        self.with_slot(player_id, |slot| slot.look = look);
    }

    /// Record what a player is wearing, for the introductions that follow.
    pub fn dress(&self, player_id: u32, wearing: Vec<game::NamedItem>) {
        self.with_slot(player_id, |slot| slot.wearing = wearing);
    }

    /// Record the guild a player is in, for the introductions that follow.
    pub fn wear_guild(&self, player_id: u32, badge: Option<crate::guild::Badge>) {
        self.with_slot(player_id, |slot| slot.guild = badge);
    }

    /// Whether a player with this character name is in the instance, matched without regard to case.
    pub fn has_player_named(&self, name: &str) -> bool {
        self.player_named(name).is_some()
    }

    /// The player with this character name in the instance, matched without regard to case.
    pub fn player_named(&self, name: &str) -> Option<u32> {
        self.locked()
            .values()
            .find(|slot| slot.name.eq_ignore_ascii_case(name))
            .map(|slot| slot.player_id)
    }

    /// Remove a player, and say whether it was there. What its client was told about the NPCs goes
    /// with it: the bodies stay, and a return is a fresh entry with their current state.
    pub fn leave(&self, player_id: u32) -> bool {
        let gone = self.locked().remove(&player_id);
        let left = gone.is_some();
        if let Some(slot) = gone {
            self.combat().leave(slot.agent.agent_id);
        }
        self.npc_table().forget(player_id);
        left
    }

    /// The next free agent id for something the world creates that is not a joining avatar (a
    /// dropped item's ground agent). A counter, as the reference's `GmIdAllocate` is a table.
    pub fn allocate_agent_id(&self) -> u32 {
        let slots = self.locked();
        let ground = self.grounds();
        let npcs = self.npc_table();
        let mut next = self
            .next_agent_id
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        loop {
            let id = *next;
            *next = next.saturating_add(1);
            if !slots.values().any(|slot| slot.agent.agent_id == id)
                && !ground.values().any(|item| item.agent_id == id)
                && !npcs.holds(id)
            {
                return id;
            }
        }
    }

    /// The NPC bodies within a loading client's range, each as the frames of its entry burst, by agent
    /// id - the NPC half of the load's agent array (`player::handle_request_players`). From here on the
    /// instance's tick keeps that client's view of them current.
    pub fn npc_arrival(&self, player_id: u32, position: (f32, f32)) -> Vec<(u32, Vec<Frame>)> {
        self.npc_table()
            .arrive(player_id, position)
            .into_iter()
            .map(|(agent_id, msgs)| (agent_id, crate::npc::frames::frames(&msgs)))
            .collect()
    }

    /// How many NPC bodies the instance has.
    pub fn npc_count(&self) -> usize {
        self.npc_table().len()
    }

    /// Answer an owner's `/npc` test command against this instance's table.
    pub fn npc_command(&self, argument: &str) -> Vec<String> {
        self.npc_table().command(argument)
    }

    /// The agent whose placement carries this literal name, for a quest marker or a script.
    pub fn npc_agent_named(&self, name: &str) -> Option<u32> {
        self.npc_table().agent_named(name)
    }

    /// One NPC body as a talk needs it (`crate::vendor`): the placement it was made from, its name and
    /// where it stands now. `None` for an agent that is not an NPC of this instance.
    pub fn npc_target(&self, agent_id: u32) -> Option<NpcTarget> {
        let table = self.npc_table();
        let placement = table.placement_of(agent_id)?;
        let body = &table.agent(agent_id)?.body;
        Some(NpcTarget {
            agent_id,
            placement: placement.id.clone(),
            name: placement.name.clone(),
            position: body.position,
        })
    }

    /// Put one dropped item on the ground under an agent id from [`Instance::allocate_agent_id`].
    pub fn put_ground_item(&self, item: GroundItem) {
        self.grounds().insert(item.agent_id, item);
    }

    /// One ground item, as a copy, without taking it.
    pub fn ground_item(&self, agent_id: u32) -> Option<GroundItem> {
        self.grounds().get(&agent_id).cloned()
    }

    /// Take a ground item off the map, so a pickup can claim it before it awaits.
    pub fn take_ground_item(&self, agent_id: u32) -> Option<GroundItem> {
        self.grounds().remove(&agent_id)
    }

    /// Every ground item, by agent id, for a client that is entering the map.
    pub fn ground_items(&self) -> Vec<GroundItem> {
        let mut items: Vec<GroundItem> = self.grounds().values().cloned().collect();
        items.sort_by_key(|item| item.agent_id);
        items
    }

    /// Whether this instance's agents - its players' and its NPCs' - hold `agent_id`.
    pub fn has_agent(&self, agent_id: u32) -> bool {
        let slots = self.locked();
        slots.values().any(|slot| slot.agent.agent_id == agent_id)
            || self.npc_table().holds(agent_id)
    }

    /// The player ids of everyone in the instance.
    pub fn player_ids(&self) -> Vec<u32> {
        self.locked().keys().copied().collect()
    }

    /// How many players are in the instance.
    pub fn len(&self) -> usize {
        self.locked().len()
    }

    /// Whether the instance is empty.
    pub fn is_empty(&self) -> bool {
        self.locked().is_empty()
    }

    /// Run one closure against one slot, under the lock and without an await inside.
    pub fn with_slot<R>(&self, player_id: u32, f: impl FnOnce(&mut Slot) -> R) -> Option<R> {
        self.locked().get_mut(&player_id).map(f)
    }

    /// The agent one player drives, as a copy (the record behind it stays in the slot).
    pub fn agent(&self, player_id: u32) -> Option<AgentState> {
        self.with_slot(player_id, |slot| slot.agent.clone())
    }

    /// One player's movement counters, for the trace's teardown summary.
    pub fn move_stats(&self, player_id: u32) -> Option<MoveStats> {
        self.with_slot(player_id, |slot| slot.move_stats)
    }

    /// Where a player's agent is now: the server's own copy, not where it entered. The direction is
    /// the facing the agent was created with, so a spawn point's angle survives into the load block.
    pub fn standing(&self, player_id: u32) -> Option<Standing> {
        self.with_slot(player_id, |slot| Standing {
            position: slot.agent.position,
            plane: slot.agent.plane,
            direction: slot.agent.facing,
        })
    }

    /// Everyone except one player, as plain data (no outbox, so the caller cannot hold the lock). Each
    /// peer's party is read after the slots' lock is released: the party table's lock is never taken
    /// under the slots' (`crate::party` takes them in the other order).
    pub fn others(&self, player_id: u32) -> Vec<PeerView> {
        let mut peers: Vec<PeerView> = self
            .locked()
            .values()
            .filter(|slot| slot.player_id != player_id)
            .map(PeerView::from)
            .collect();
        for peer in &mut peers {
            (peer.party_leader, peer.party_size) = self.parties.leader_and_size(peer.player_id);
        }
        peers
    }

    /// Push frames to everyone except `except` who has entered (`Instance::enter`). A slot whose channel
    /// is closed is gone; its own task is responsible for leaving, so a failed send is not an error here.
    pub fn broadcast(&self, except: u32, frames: &[Frame]) {
        for slot in self
            .locked()
            .values()
            .filter(|slot| slot.player_id != except && slot.entered)
        {
            let _ = slot.outbox.send(frames.to_vec());
        }
    }

    /// Push frames to one player's own connection, the way the clock and the walk marks reach it. For a
    /// caller that holds the player but not its socket (`player::walked`).
    pub fn send_to(&self, player_id: u32, frames: &[Frame]) {
        self.with_slot(player_id, |slot| {
            let _ = slot.outbox.send(frames.to_vec());
        });
    }

    /// Push frames to everyone in the instance who has entered (`Instance::enter`).
    pub fn broadcast_all(&self, frames: &[Frame]) {
        for slot in self.locked().values().filter(|slot| slot.entered) {
            let _ = slot.outbox.send(frames.to_vec());
        }
    }

    /// Push frames about `subjects` to every client that knows every one of them. A player's own
    /// agent is known to every entered client; an NPC must have been sent to that client and not
    /// removed (`NpcTable::knows`).
    pub fn broadcast_known(&self, subjects: &[u32], frames: &[Frame]) {
        let slots = self.locked();
        let table = self.npc_table();
        for slot in slots.values().filter(|slot| slot.entered) {
            if knows_all(&table, slot.player_id, subjects) {
                let _ = slot.outbox.send(frames.to_vec());
            }
        }
    }

    /// One tick, `GameSrv_WorldTick`: announce the elapsed time, walk every agent, and broadcast each
    /// crossed waypoint to the rest of the instance (`GameSrv_UpdateAgentDestination`).
    ///
    /// The announced number is the **real** elapsed time since the previous tick, which is what the
    /// reference sends (`srv->current_frame_time - srv->last_world_tick`).
    pub fn step(&self, elapsed: Duration) {
        let delta_ms = u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX);
        let delta = elapsed.as_secs_f32();
        let mut slots = self.locked();

        // Advance every agent first, so the announcements below carry where each one is now.
        let mut crossed = Vec::new();
        for slot in slots.values_mut() {
            if slot.loading {
                continue;
            }
            let was_moving = 0.0 < slot.agent.motion.speed;
            let before = slot.agent.position;
            let before_plane = slot.agent.plane;
            let cross = slot.agent.advance(delta);
            let after = slot.agent.position;
            if before != after && crate::portals::crossed(self.map.map_id, before, after).is_some()
            {
                if let Some(outbox) = &slot.portal_outbox {
                    let _ = outbox.send((before, after));
                }
            }
            if move_trace_on() {
                slot.move_stats.ticks += 1;
                if was_moving {
                    let moved = {
                        let dx = slot.agent.position.0 - before.0;
                        let dy = slot.agent.position.1 - before.1;
                        (dx * dx + dy * dy).sqrt()
                    };
                    let expected = game::PLAYER_SPEED_BASE * delta;
                    eprintln!(
                        "move[tick] walked {moved:.1} units (expected {expected:.1} at \
                         {:.0}/s), agent (x={:.1}, y={:.1}, plane={})",
                        game::PLAYER_SPEED_BASE,
                        slot.agent.position.0,
                        slot.agent.position.1,
                        slot.agent.plane
                    );
                    if slot.agent.plane != before_plane {
                        slot.move_stats.plane_changes += 1;
                        eprintln!(
                            "move[jump] the walk changed the agent's plane {before_plane} -> {} \
                             at (x={:.1}, y={:.1}) - the client will place it on that plane now",
                            slot.agent.plane, slot.agent.position.0, slot.agent.position.1
                        );
                    }
                }
                if let Some(waypoint) = &cross {
                    eprintln!(
                        "move[tick] crossed waypoint {}/{} (x={:.1}, y={:.1}, plane={}, trap={}); \
                         agent now (x={:.1}, y={:.1}, plane={})",
                        slot.agent.motion.waypoint_idx,
                        slot.agent.motion.waypoints.len(),
                        waypoint.pos.x,
                        waypoint.pos.y,
                        waypoint.pos.plane,
                        waypoint.trap_id,
                        slot.agent.position.0,
                        slot.agent.position.1,
                        slot.agent.plane
                    );
                } else if was_moving && slot.agent.motion.speed == 0.0 {
                    eprintln!(
                        "move[tick] arrival at (x={:.1}, y={:.1}, plane={}); the walk is over",
                        slot.agent.position.0, slot.agent.position.1, slot.agent.plane
                    );
                }
            }
            if let Some(waypoint) = cross {
                crossed.push((
                    slot.player_id,
                    slot.agent.agent_id,
                    slot.agent.plane,
                    waypoint,
                ));
            }
        }

        let tick = game::world_simulation_tick(delta_ms);
        for slot in slots.values().filter(|slot| !slot.loading) {
            let _ = slot.outbox.send(vec![tick.clone()]);
        }
        for (player_id, agent_id, plane, waypoint) in crossed {
            // The walker hears it too: a crossed waypoint is a new destination, and every client is
            // drawing this agent from the last destination it was told.
            let walk = game::agent_move_to_point(
                agent_id,
                (waypoint.pos.x, waypoint.pos.y),
                waypoint.pos.plane,
                plane,
            );
            for slot in slots.values().filter(|slot| !slot.loading) {
                let _ = slot.outbox.send(vec![walk.clone()]);
            }
            let _ = player_id;
        }

        // The NPCs: what each active body did this tick, and which bodies came into or left each client's
        // range (`crate::npc::table`). A client is a viewer once it has entered and finished loading; a
        // loading one has the bodies in range in its own block.
        let viewers: Vec<crate::npc::Viewer> = slots
            .values()
            .filter(|slot| slot.entered && !slot.loading)
            .map(|slot| crate::npc::Viewer {
                player_id: slot.player_id,
                position: slot.agent.position,
            })
            .collect();
        // The fight, then the NPCs, then the fight's clock. The creatures think on where everyone stood
        // after the walks above (`crate::npc::fight`): their attacks and skills go to the engine, their
        // walks to the table, which sends them with the bodies' other moves. Locks: slots (held), NPCs,
        // then the fight, never the other way round.
        let pack = crate::skill::catalog::current().ok();
        let mut table = self.npc_table();
        if let Some(pack) = pack {
            let rules = crate::npc::fight::rules().copied().unwrap_or_default();
            let reach = crate::npc::fight::Reach {
                melee: pack.rules.melee_range,
                ranged: pack.rules.bow_range,
                spell: pack.rules.spell_range,
            };
            let bodies = table.positions();
            let orders = {
                let mut combat = self.combat();
                // A player's damage last tick is what wakes a body that only fights when provoked.
                for (mob, attacker) in combat.engine.take_provoked() {
                    table.fight.provoke(mob, attacker);
                }
                for slot in slots.values() {
                    combat
                        .engine
                        .set_position(slot.agent.agent_id, slot.agent.position);
                }
                for (agent_id, position) in &bodies {
                    combat.engine.set_position(*agent_id, *position);
                }
                let seen = |agent: u32| {
                    combat
                        .engine
                        .actor(agent)
                        .map(|actor| crate::npc::fight::Seen {
                            agent,
                            position: actor.position,
                            alive: actor.alive,
                        })
                };
                let players: Vec<crate::npc::fight::Seen> = slots
                    .values()
                    .filter(|slot| slot.entered && !slot.loading)
                    .filter_map(|slot| seen(slot.agent.agent_id))
                    .collect();
                let creatures: std::collections::BTreeMap<u32, crate::npc::fight::Seen> = table
                    .fight
                    .agents()
                    .into_iter()
                    .filter_map(|agent| Some((agent, seen(agent)?)))
                    .collect();
                let orders = table.fight.think(
                    u64::from(delta_ms),
                    &rules,
                    reach,
                    &creatures,
                    &players,
                );
                for (agent, order) in &orders {
                    use crate::npc::fight::Order;
                    match order {
                        Order::Attack { target } => {
                            let _ = combat.engine.attack(*agent, *target);
                        }
                        Order::UseSkills { skills, target } => {
                            for skill in skills {
                                let used = combat.engine.use_skill(
                                    *agent,
                                    *skill,
                                    0,
                                    *target,
                                    &pack.skills,
                                    &pack.rules,
                                );
                                if used.is_ok() {
                                    break;
                                }
                            }
                        }
                        Order::StopAttack => combat.engine.stop_attack(*agent),
                        Order::Restore => combat.engine.restore(*agent, &pack.rules),
                        Order::Revive { at } => combat.engine.revive(*agent, *at, &pack.rules),
                        Order::Walk { .. } | Order::Halt | Order::Fall => {}
                    }
                }
                orders
            };
            table.obey(&orders, &self.graphs);
        }
        let mut ticked = table.tick(delta, &viewers, &self.graphs);
        let stopped = std::mem::take(&mut ticked.stopped);
        let bodies = table.positions();
        let spots = table.spots();
        drop(table);
        for note in &ticked.notes {
            eprintln!("world: npc: {note}");
        }
        for (player_id, msgs) in ticked.deliveries {
            if let Some(slot) = slots.get(&player_id) {
                let _ = slot.outbox.send(crate::npc::frames::frames(&msgs));
            }
        }

        // Players walking to an agent (`crate::approach`): steered on where everyone stands now. The
        // fight's lock comes after the slots' and the NPCs' (released above).
        let approached = self.steer_approaches(&mut slots, spots, delta_ms);
        if !approached.is_empty() {
            for slot in slots.values().filter(|slot| slot.entered && !slot.loading) {
                let _ = slot.outbox.send(approached.clone());
            }
        }

        // The fight's clock: casts that finish, swings that land, recharges that end, effects that run
        // out, areas that pulse and regeneration (`crate::skill::engine`). Positions are the walk's, as
        // of this tick.
        if let Some(pack) = pack {
            let deliveries = {
                let mut combat = self.combat();
                for agent in &stopped {
                    combat.engine.stop_attack(*agent);
                }
                for slot in slots.values() {
                    combat
                        .engine
                        .set_position(slot.agent.agent_id, slot.agent.position);
                }
                for (agent_id, position) in bodies {
                    combat.engine.set_position(agent_id, position);
                }
                combat
                    .engine
                    .tick(u64::from(delta_ms), &pack.skills, &pack.rules);
                combat.deliveries(pack)
            };
            // A fight message names agent ids. A client that was never sent a body cannot look one
            // up, and the client asserts on the lookup (`docs/49` 2.1), so a broadcast goes only to
            // the clients that know every body it names.
            let table = self.npc_table();
            for (recipient, subjects, frames) in deliveries {
                match recipient {
                    crate::skill::Recipient::Player(player_id) => {
                        if let Some(slot) = slots.get(&player_id) {
                            let _ = slot.outbox.send(frames);
                        }
                    }
                    crate::skill::Recipient::Everyone => {
                        for slot in slots.values().filter(|slot| slot.entered && !slot.loading) {
                            if !knows_all(&table, slot.player_id, &subjects) {
                                continue;
                            }
                            let _ = slot.outbox.send(frames.clone());
                        }
                    }
                }
            }
        }
    }

    /// One tick of every player's walk to an agent (`crate::approach`), with the frames everyone is told.
    /// An attack's walk ends with the fight's attack; a walk to an agent that is gone ends there.
    fn steer_approaches(
        &self,
        slots: &mut HashMap<u32, Slot>,
        npcs: Vec<(u32, crate::pathfind::Pos)>,
        delta_ms: u32,
    ) -> Vec<Frame> {
        use crate::approach::{abandon, steer, Purpose};
        if !slots.values().any(|slot| slot.approach.is_some()) {
            return Vec::new();
        }
        let targets_npc: std::collections::HashSet<u32> =
            npcs.iter().map(|(agent, _)| *agent).collect();
        let mut targets: HashMap<u32, crate::pathfind::Pos> = npcs.into_iter().collect();
        for slot in slots.values() {
            targets.insert(slot.agent.agent_id, slot.agent.here());
        }
        let attacking: HashMap<u32, Option<u32>> = {
            let combat = self.combat();
            slots
                .values()
                .filter(|slot| {
                    slot.approach
                        .as_ref()
                        .is_some_and(|approach| approach.purpose == Purpose::Attack)
                })
                .map(|slot| {
                    let agent = slot.agent.agent_id;
                    let on = combat
                        .engine
                        .actor(agent)
                        .and_then(|actor| actor.attack_target);
                    (agent, on)
                })
                .collect()
        };
        let mut frames = Vec::new();
        for slot in slots.values_mut() {
            if slot.loading {
                continue;
            }
            let Some(mut approach) = slot.approach.take() else {
                continue;
            };
            let attack_over = approach.purpose == Purpose::Attack
                && attacking.get(&slot.agent.agent_id).copied().flatten() != Some(approach.target);
            let Some(target) = targets
                .get(&approach.target)
                .copied()
                .filter(|_| !attack_over)
            else {
                frames.extend(abandon(&mut slot.agent));
                continue;
            };
            let steered = steer(
                &mut approach,
                &mut slot.agent,
                target,
                self.map.map_id,
                &self.graphs,
                delta_ms,
            );
            frames.extend(steered.frames);
            // At the NPC: its own connection answers the talk, with the database and the socket the
            // tick does not hold (`crate::vendor::talk`). A player is never talked to.
            if steered.arrived && targets_npc.contains(&approach.target) {
                let _ = slot.orders.send(crate::travel::Order::Talk {
                    npc: approach.target,
                });
            }
            if !steered.done {
                slot.approach = Some(approach);
            }
        }
        frames
    }

    /// The instance's clock: one task per map, ticking while the instance exists.
    pub async fn run(self: Arc<Self>) {
        let mut tick = tokio::time::interval(crate::connection::tick_interval());
        let mut last_tick = Instant::now();
        loop {
            tick.tick().await;
            let elapsed = last_tick.elapsed();
            last_tick = Instant::now();
            self.step(elapsed);
        }
    }
}

/// A peer as it is copied out of an instance: everything a joining player needs to be told.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerView {
    pub player_id: u32,
    pub agent_id: u32,
    pub name: String,
    pub standing: Standing,
    pub equipped: [u32; 9],
    /// The title rank the peer shows, if any.
    pub shown_title: Option<asura_rules::titles::Rank>,
    /// How the peer is drawn: appearance, professions, level.
    pub look: Look,
    /// The items it is wearing.
    pub wearing: Vec<game::NamedItem>,
    /// The player who leads its party, and how many players are in it (`0x00B0`, `0x00B1`).
    pub party_leader: u32,
    /// See [`PeerView::party_leader`].
    pub party_size: u32,
    /// The guild it is in, if any (`0x0121` and agent property 30 right after its player create).
    pub guild: Option<crate::guild::Badge>,
}

impl From<&Slot> for PeerView {
    fn from(slot: &Slot) -> Self {
        PeerView {
            player_id: slot.player_id,
            agent_id: slot.agent.agent_id,
            name: slot.name.clone(),
            standing: Standing {
                position: slot.agent.position,
                plane: slot.agent.plane,
                direction: slot.agent.facing,
            },
            equipped: slot.equipped,
            shown_title: slot.shown_title.clone(),
            look: slot.look,
            wearing: slot.wearing.clone(),
            // A party of one until `Instance::others` reads the party table.
            party_leader: slot.player_id,
            party_size: 1,
            guild: slot.guild.clone(),
        }
    }
}

/// The frames that introduce one player to another: `GameSrv_SendWorldAgents`'s order for a player
/// agent (`GmAgent.c:310-331`) - the player, their party size, their party, then the agent itself.
///
/// `GmAgent.c` also sends `BroadcastAddPlayerToPlayerParty` rather than the unicast sender, because
/// the party of a joining player is news to everyone already there. Here it is the same frame either
/// way.
///
/// A peer that shows a title has its rank record and binding (`0x00F3`, `0x00F4`) right before the
/// agent, where a live load carries them (`recordings/client/feature/maps/pre-searing-load-3.jsonl`
/// seq 217-221: record, binding, `0x003C`, effects, agent).
pub fn introduction(peer: &PeerView) -> Vec<Frame> {
    // The peer's own appearance, or the defaults for a character with none recorded - the same rule as
    // the player's own block: all-zero bits are `Profession_None`, which the client cannot draw.
    let appearance = if peer.look.appearance != 0 {
        peer.look.appearance
    } else {
        crate::character_defaults::current()
            .map(|defaults| defaults.appearance().pack())
            .unwrap_or_default()
    };
    let mut frames = vec![game::player_create(
        peer.player_id,
        peer.agent_id,
        appearance,
        &peer.name,
    )];
    // A guilded player's guild and its agent's guild property, between its player create and its party
    // pair (`pre-searing-load-3.jsonl` seq 299-302, `crate::guild`).
    if let Some(badge) = &peer.guild {
        frames.extend(crate::guild::badge_frames(badge, peer.agent_id));
    }
    frames.extend([
        // The leader and its party's size, then this player and its leader: a live load's order for every
        // other player (`titles-live-20261106-025940.jsonl` seq 2088-2091: player 6 in a party of three
        // led by player 0x40 is `0x00B0 {0x40, 3}`, `0x00B1 {6, 0x40}`).
        game::player_update_party_size(peer.party_leader, peer.party_size),
        game::player_update_party(peer.player_id, peer.party_leader),
        // Retail's next two for every other player: the agent's professions (`0x00A6`), then its level
        // (`0x009F`, property 36). The player's own block sends its profession as `0x00B7` for the same
        // reason - a player agent with no profession is one the client has no model for.
        game::agent_update_profession(peer.agent_id, peer.look.primary, peer.look.secondary),
        game::agent_property_update_int(
            game::AGENT_PROPERTY_PUBLIC_LEVEL,
            peer.agent_id,
            peer.look.level,
        ),
    ]);
    if let Some(rank) = &peer.shown_title {
        // The record, the binding, and the binding's own `0x003C`.
        frames.extend(crate::titles::binding_for_peer(peer.player_id, rank));
    } else {
        // Retail sends this player's flags word for every other player it introduces, mask 7, title or
        // not (pre-searing-load-3.jsonl seq 219, 306, 1009, 1343).
        frames.push(game::player_update_flags(peer.player_id as u16, 0, 7));
    }
    // And the agent's initial effects immediately before the agent itself (seq 220, 307, 1010, 1344;
    // `GameSrv_SendAgentInitialEffects` in the reference's `GameSrv_SendWorldAgents`).
    frames.push(game::agent_initial_effects(peer.agent_id, 0));
    frames.push(game::player_create_agent(
        peer.agent_id,
        peer.player_id,
        peer.standing.position,
        peer.standing.plane,
        peer.standing.direction,
        PLAYER_TEAM_TOKEN,
    ));
    // What it takes to draw it dressed, in retail's order right after the agent: each equipped item as
    // the low-detail record, the visual equipment that names them, the cape (pre-searing-load-3.jsonl
    // seq 222-228, 309-317, 1347-1353). Both directions go through here, so a newcomer sees the players
    // already in the map dressed exactly as they see it.
    for item in &peer.wearing {
        frames.push(game::item_low_detail(item));
    }
    frames.push(game::update_agent_visual_equipment(
        peer.agent_id,
        peer.equipped,
    ));
    frames.push(game::agent_display_cape(peer.agent_id, 1));
    frames
}

/// The equipped items of an inventory, as another client is told them: the rows of the bag of type 2.
pub fn wearing(inventory: &Inventory) -> Vec<game::NamedItem> {
    let equipped_bag = inventory
        .bags
        .iter()
        .find(|bag| bag.bag_type == 2)
        .map(|bag| bag.bag_id);
    inventory
        .items
        .iter()
        .filter(|item| item.bag_id == equipped_bag)
        .map(named_item)
        .collect()
}

/// The frames that take one player back out of another's world: `GameSrv_BroadcastPlayerDestroy`
/// (`GmPlayer.c:314`) and `GameSrv_BroadcastAgentRemove` (`GmAgent.c:60`), in that order.
pub fn farewell(player_id: u32, agent_id: u32) -> Vec<Frame> {
    vec![
        game::player_destroy(player_id),
        game::world_remove_agent(agent_id),
    ]
}
/// Every instance this world is running, by map id.
///
/// A world hosts one instance per map (outposts never end; explorable pools are a later policy), so
/// the map id is the key. The first join to a map creates its instance - with its NPCs - and starts
/// its clock.
#[derive(Default, Clone)]
pub struct Instances {
    running: Arc<Mutex<HashMap<i32, Arc<Instance>>>>,
    /// What a new instance seeds its NPCs from; `None` runs every instance without NPCs.
    npcs: Option<crate::npc::NpcWorld>,
}

impl Instances {
    /// Instances that seed their maps' NPCs from `npcs`.
    pub fn with_npcs(npcs: crate::npc::NpcWorld) -> Instances {
        Instances {
            running: Arc::default(),
            npcs: Some(npcs),
        }
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, HashMap<i32, Arc<Instance>>> {
        self.running.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// The running instance for a map, created - with its NPCs and its clock running - on first use.
    pub fn for_map(&self, map: &MapEntry) -> Arc<Instance> {
        if let Some(instance) = self.locked().get(&map.map_id) {
            return instance.clone();
        }
        let instance = Arc::new(Instance::with_npcs(map.clone(), self.npcs.as_ref()));
        let ticking = Arc::clone(&instance);
        tokio::spawn(async move { ticking.run().await });
        self.locked().insert(map.map_id, instance.clone());
        instance
    }

    /// The instance for a map, if one is running.
    pub fn get(&self, map_id: i32) -> Option<Arc<Instance>> {
        self.locked().get(&map_id).cloned()
    }

    /// Whether a map has an instance running.
    pub fn has(&self, map_id: i32) -> bool {
        self.locked().contains_key(&map_id)
    }

    /// Whether a character with this name is in any instance of this world: the "online" a guild
    /// invitation asks about (`crate::guild`). A character in another world process reads as offline.
    pub fn is_online(&self, name: &str) -> bool {
        let instances: Vec<Arc<Instance>> = self.locked().values().cloned().collect();
        instances
            .iter()
            .any(|instance| instance.has_player_named(name))
    }

    /// How many players are in this world, across its instances.
    pub fn len(&self) -> usize {
        self.locked().values().map(|instance| instance.len()).sum()
    }

    /// Whether no map has an instance.
    pub fn is_empty(&self) -> bool {
        self.locked().is_empty()
    }

    /// Everyone in this world except one player, across all instances (tests and population).
    pub fn others(&self, except: u32) -> Vec<PeerView> {
        self.locked()
            .values()
            .flat_map(|instance| instance.others(except))
            .collect()
    }
}

/// Whether a client knows every agent a message names. The table holds only NPC bodies; a player's
/// agent id is not in it, and every entered client has been sent every player.
fn knows_all(table: &crate::npc::NpcTable, player_id: u32, subjects: &[u32]) -> bool {
    subjects
        .iter()
        .all(|agent| !table.holds(*agent) || table.knows(player_id, *agent))
}
