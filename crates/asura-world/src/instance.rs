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

/// One player in one instance: the agent it drives, what the client is told about it, and the
/// channel that reaches its connection.
pub struct Slot {
    pub player_id: u32,
    pub agent: AgentState,
    pub name: String,
    pub equipped: [u32; 9],
    pub outbox: tokio::sync::mpsc::UnboundedSender<Vec<Frame>>,
    /// Movement counters, for the trace's closing summary.
    pub move_stats: MoveStats,
    /// Set at join and cleared when the connecting player has its load block, so the clock does not
    /// put a tick inside the block: the reference finishes the load with `INSTANCE_LOAD_FINISH`
    /// (`GameSrv_HandleInstanceLoadRequestPlayers`) before it starts announcing the world clock.
    pub loading: bool,
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
}

impl Instance {
    pub fn new(map: MapEntry) -> Instance {
        Instance {
            map,
            slots: Mutex::new(HashMap::new()),
            next_agent_id: Mutex::new(2),
        }
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Slot>> {
        self.slots.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Put a player in the instance and hand out its ids.
    ///
    /// The reference allocates both from per-server tables (`GmIdAllocate`); with a list this short,
    /// the lowest free id is the same thing, and it keeps the single-player case at id 1 exactly as
    /// it was.
    pub fn join(
        &self,
        name: String,
        standing: Standing,
        equipped: [u32; 9],
        outbox: tokio::sync::mpsc::UnboundedSender<Vec<Frame>>,
    ) -> (u32, u32) {
        let mut slots = self.locked();
        let player_id = (1..).find(|id| !slots.contains_key(id)).unwrap_or(1);
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
                    game::PLAYER_SPEED_BASE,
                ),
                name,
                equipped,
                outbox,
                move_stats: MoveStats::default(),
                loading: true,
            },
        );
        (player_id, agent_id)
    }

    /// Mark a player loaded: the load block is out, so the clock may tick it.
    pub fn loaded(&self, player_id: u32) {
        self.with_slot(player_id, |slot| slot.loading = false);
    }

    /// Whether a player has finished loading.
    pub fn is_loaded(&self, player_id: u32) -> bool {
        self.with_slot(player_id, |slot| !slot.loading)
            .unwrap_or(false)
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

    /// Where a player's agent is now: the server's own copy, not where it entered.
    pub fn standing(&self, player_id: u32) -> Option<Standing> {
        self.with_slot(player_id, |slot| Standing {
            position: slot.agent.position,
            plane: slot.agent.plane,
        })
    }

    /// Everyone except one player, as plain data (no outbox, so the caller cannot hold the lock).
    pub fn others(&self, player_id: u32) -> Vec<PeerView> {
        self.locked()
            .values()
            .filter(|slot| slot.player_id != player_id)
            .map(PeerView::from)
            .collect()
    }

    /// Push frames to everyone except `except`. A slot whose channel is closed is gone; its own task
    /// is responsible for leaving, so a failed send is not an error here.
    pub fn broadcast(&self, except: u32, frames: &[Frame]) {
        for slot in self
            .locked()
            .values()
            .filter(|slot| slot.player_id != except)
        {
            let _ = slot.outbox.send(frames.to_vec());
        }
    }

    /// Push frames to everyone in the instance.
    pub fn broadcast_all(&self, frames: &[Frame]) {
        for slot in self.locked().values() {
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
        let mut tick = tokio::time::interval(TICK_INTERVAL);
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
            },
            equipped: slot.equipped,
        }
    }
}

/// The frames that introduce one player to another: `GameSrv_SendWorldAgents`'s order for a player
/// agent (`GmAgent.c:310-331`) - the player, their party size, their party, then the agent itself.
///
/// `GmAgent.c` also sends `BroadcastAddPlayerToPlayerParty` rather than the unicast sender, because
/// the party of a joining player is news to everyone already there. Here it is the same frame either
/// way.
pub fn introduction(peer: &PeerView) -> Vec<Frame> {
    vec![
        game::player_create(
            peer.player_id,
            peer.agent_id,
            game::Appearance::REFERENCE_DEFAULT.pack(),
            &peer.name,
        ),
        game::player_update_party_size(peer.player_id, 1),
        game::player_update_party(peer.player_id, peer.player_id),
        game::world_create_agent(&game::CreateAgent {
            agent_id: peer.agent_id,
            model_id: game::CHAR_CLASS_PLAYER_BASE | peer.player_id,
            agent_type: game::AGENT_TYPE_LIVING,
            position: peer.standing.position,
            plane: peer.standing.plane,
            direction: (1.0, 0.0),
            speed_base: game::PLAYER_SPEED_BASE,
            player_team_token: PLAYER_TEAM_TOKEN,

        }),
    ]
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
