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

/// One hosted map: its description, and everyone standing in it.
pub struct Instance {
    /// The map this instance runs: what a client is told it is entering (`MapEntry`).
    pub map: MapEntry,
    /// The players in it, by player id. The lock is the std one: the critical sections are a scan
    /// and a send into an unbounded channel, never an await point.
    slots: Mutex<HashMap<u32, Slot>>,
    /// The next agent id handed out for something that is not a joining avatar.
    next_agent_id: Mutex<u32>,
    /// The parties of the players in it (`crate::party`).
    pub parties: crate::party::Parties,
    /// The local ids its messages name guilds by (`crate::guild`).
    pub guild_ids: crate::guild::LocalIds,
    /// Player ids kept for characters that are not in the instance: a party member still travelling in,
    /// or one whose party holds its place after a drop (`crate::party`). Its lock is taken after the
    /// slots' and never the other way round.
    reserved: Mutex<HashMap<Uuid, u32>>,
}

impl Instance {
    pub fn new(map: MapEntry) -> Instance {
        Instance {
            map,
            slots: Mutex::new(HashMap::new()),
            next_agent_id: Mutex::new(2),
            parties: crate::party::Parties::default(),
            guild_ids: crate::guild::LocalIds::default(),
            reserved: Mutex::new(HashMap::new()),
        }
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Slot>> {
        self.slots.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn reservations(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, u32>> {
        self.reserved.lock().unwrap_or_else(|err| err.into_inner())
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
        let agent_id = (1..)
            .find(|id| !slots.values().any(|slot| slot.agent.agent_id == *id))
            .unwrap_or(1);
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
        self.locked()
            .values()
            .any(|slot| slot.name.eq_ignore_ascii_case(name))
    }

    /// Remove a player, and say whether it was there.
    pub fn leave(&self, player_id: u32) -> bool {
        self.locked().remove(&player_id).is_some()
    }

    /// The next free agent id for something the world creates that is not a joining avatar (a
    /// dropped item's ground agent). A counter, as the reference's `GmIdAllocate` is a table.
    pub fn allocate_agent_id(&self) -> u32 {
        let mut next = self
            .next_agent_id
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let id = *next;
        *next = next.saturating_add(1);
        id
    }

    /// Whether this instance's agents hold `agent_id`.
    pub fn has_agent(&self, agent_id: u32) -> bool {
        self.locked()
            .values()
            .any(|slot| slot.agent.agent_id == agent_id)
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
    frames.push(game::world_create_agent(&game::CreateAgent {
        agent_id: peer.agent_id,
        model_id: game::CHAR_CLASS_PLAYER_BASE | peer.player_id,
        agent_type: game::AGENT_TYPE_LIVING,
        position: peer.standing.position,
        plane: peer.standing.plane,
        direction: peer.standing.direction,
        speed_base: game::PLAYER_SPEED_BASE,
        player_team_token: PLAYER_TEAM_TOKEN,
    }));
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
/// the map id is the key. The first join to a map creates its instance and starts its clock.
#[derive(Default, Clone)]
pub struct Instances(Arc<Mutex<HashMap<i32, Arc<Instance>>>>);

impl Instances {
    fn locked(&self) -> std::sync::MutexGuard<'_, HashMap<i32, Arc<Instance>>> {
        self.0.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// The running instance for a map, created - with its clock running - on first use.
    pub fn for_map(&self, map: &MapEntry) -> Arc<Instance> {
        if let Some(instance) = self.locked().get(&map.map_id) {
            return instance.clone();
        }
        let instance = Arc::new(Instance::new(map.clone()));
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
