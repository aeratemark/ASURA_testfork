//! One instance's NPCs: the bodies, what each client has been told about them, and their tick.
//!
//! The instance owns this table (`crate::instance::Instance`), seeds it when the instance is created -
//! before any player arrives (`docs/36` R29) - and drops it with the instance. Everything mutable about
//! an NPC lives here and nowhere else: its agent id, its body ([`AgentState`], the same record a player
//! drives), its behavior's running tree, and per client which definitions and which bodies that client
//! has been sent.
//!
//! Three states are kept apart (`docs/49-npc-framework.md` section 3.11): a body *exists* while the
//! instance does; it is *visible* to one client while that client stands within the compass range of
//! it, which is a create on entering and a remove on leaving; and it is *active* - its walk advances and
//! its tree ticks - only while some client is within that range (`docs/36` R34). A dormant body keeps
//! its state and resumes where it stopped.
//!
//! The range is a 2D distance; the plane does not enter it. The live server creates a body on plane 26
//! for a client standing on plane 0 (`pre-searing-load-3.jsonl` seq 233 against seq 221).

use super::behavior::{self, BurrowRequest, Ctx, Pathing, Script, Spec, Status, Tree};
use super::burst::{self, Msg};
use super::content::{MapNpcs, NpcContent, NpcPlacement, SightRules};
use super::fight::{Fight, Order, Style};
use crate::agent::{AgentState, Walk};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// Where behavior trees come from: the pack's constructor. A pack with a Lua script uses
/// [`super::lua::LuaBehaviors`]; a pack without one has [`PackBehaviors`].
pub trait BehaviorSource {
    /// The tree a behavior name builds, or `None` for a name the source does not know.
    fn spec(&self, name: &str) -> Option<Spec>;

    /// The per-agent Lua state for a script entry, or `None` when the source has no such function.
    /// Each call creates a fresh state and context, because each agent runs its own copy.
    fn script(&self, _name: &str) -> Option<Script> {
        None
    }
}

/// The pack's built-in behavior names, used when a pack has no Lua script: `patrol` walks the
/// placement's own route. A pack with a script declares `patrol` itself (`behaviors.lua`), so the
/// source is explicit about every name the pack uses.
pub struct PackBehaviors;

impl BehaviorSource for PackBehaviors {
    fn spec(&self, name: &str) -> Option<Spec> {
        (name == "patrol").then_some(Spec::Patrol { route: None })
    }
}

/// The 2.00 s emerge and submerge animations. Measured, not authored: 373 transition intervals at a
/// 2.000 s median, every sample inside +/-60 ms (Rurik `studies/tape/FINDINGS.md` T3,
/// `studies/cmsg/FINDINGS.md` T16). The visible and hidden spans are the script's terms.
const BURROW_TRANSITION_MS: u32 = 2000;

/// Where one body is in its vanish/appear cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BurrowPhase {
    /// Created with status 0x1000; the 2.00 s emerge animation.
    Emerging,
    /// Status 0; visible and targetable for the script's `visible_ms`.
    Out,
    /// Status 0x1000; the 2.00 s submerge animation.
    Submerging,
    /// Removed from every client for the script's `hidden_ms`.
    Hidden,
}

/// One body's vanish/appear state and the terms the script chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BurrowState {
    phase: BurrowPhase,
    left_ms: u32,
    visible_ms: u32,
    hidden_ms: u32,
    /// A script owns the surface/hidden choice: the spans are commands, not a cycle.
    manual: bool,
    /// A transition a script asked for, applied by the next `advance`.
    request: Option<BurrowRequest>,
}

/// A phase change that has wire to send or a view to repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BurrowEvent {
    /// The emerge animation ended: `0x00F1 [agent, 0]`.
    Out,
    /// The visible span ended: `0x00F1 [agent, 0x1000]`.
    Submerging,
    /// The submerge animation ended: a bare `0x0021`.
    Hidden,
    /// The hidden span ended: the five-message re-create.
    Emerging,
}

impl BurrowState {
    fn new(visible_ms: u32, hidden_ms: u32) -> BurrowState {
        BurrowState {
            phase: BurrowPhase::Emerging,
            left_ms: BURROW_TRANSITION_MS,
            visible_ms,
            hidden_ms,
            manual: false,
            request: None,
        }
    }

    /// A script-owned cycle: the body starts emerging like a `Burrow` body, then waits for
    /// `ctx:burrow()`/`ctx:emerge()` instead of cycling on its own.
    fn manual() -> BurrowState {
        BurrowState {
            phase: BurrowPhase::Emerging,
            left_ms: BURROW_TRANSITION_MS,
            visible_ms: 0,
            hidden_ms: 0,
            manual: true,
            request: None,
        }
    }

    /// The phase name a script reads as `ctx.phase`.
    fn phase_name(self) -> &'static str {
        match self.phase {
            BurrowPhase::Emerging => "emerging",
            BurrowPhase::Out => "out",
            BurrowPhase::Submerging => "submerging",
            BurrowPhase::Hidden => "hidden",
        }
    }

    /// Record a script's request. The next `advance` applies it through the measured transitions.
    fn request(&mut self, request: BurrowRequest) {
        self.request = Some(request);
    }

    /// The status word a client entering now must be created with.
    fn status(self) -> u32 {
        match self.phase {
            BurrowPhase::Emerging | BurrowPhase::Submerging => burst::BURROW_TRANSITION,
            BurrowPhase::Out | BurrowPhase::Hidden => 0,
        }
    }

    fn hidden(self) -> bool {
        self.phase == BurrowPhase::Hidden
    }

    /// Advance the phase clock by one tick, returning the transition it crossed, if any. A tick
    /// larger than the remaining span crosses one phase; the next tick carries the remainder.
    fn advance(&mut self, elapsed_ms: u32) -> Option<BurrowEvent> {
        if let Some(request) = self.request.take() {
            match request {
                BurrowRequest::Submerge { hidden_ms } if self.phase == BurrowPhase::Out => {
                    self.phase = BurrowPhase::Submerging;
                    self.left_ms = BURROW_TRANSITION_MS;
                    self.hidden_ms = hidden_ms;
                    return Some(BurrowEvent::Submerging);
                }
                BurrowRequest::Emerge if self.phase == BurrowPhase::Hidden => {
                    self.phase = BurrowPhase::Emerging;
                    self.left_ms = BURROW_TRANSITION_MS;
                    return Some(BurrowEvent::Emerging);
                }
                _ => {}
            }
        }
        if self.manual {
            // A script decides when a surfaced body submerges. A hidden body with a positive span
            // emerges by itself when the span runs out; a zero span waits for `ctx:emerge()`.
            match self.phase {
                BurrowPhase::Out => return None,
                BurrowPhase::Hidden if self.hidden_ms == 0 => return None,
                _ => {}
            }
        }
        if elapsed_ms < self.left_ms {
            self.left_ms -= elapsed_ms;
            return None;
        }
        let over = elapsed_ms - self.left_ms;
        let (phase, left_ms, event) = match self.phase {
            BurrowPhase::Emerging => (BurrowPhase::Out, self.visible_ms, BurrowEvent::Out),
            BurrowPhase::Out => (
                BurrowPhase::Submerging,
                BURROW_TRANSITION_MS,
                BurrowEvent::Submerging,
            ),
            BurrowPhase::Submerging => (BurrowPhase::Hidden, self.hidden_ms, BurrowEvent::Hidden),
            BurrowPhase::Hidden => (
                BurrowPhase::Emerging,
                BURROW_TRANSITION_MS,
                BurrowEvent::Emerging,
            ),
        };
        self.phase = phase;
        self.left_ms = left_ms.saturating_sub(over);
        Some(event)
    }
}

/// What drives a body.
#[derive(Debug, Clone)]
enum Mind {
    /// No behavior: the body stands where it was created.
    Still,
    /// A running tree.
    Tree(Tree),
    /// The tree succeeded; the body stays where it ended.
    Done,
    /// The tree failed, or could not be built; the body stands, and the reason was logged once.
    Parked,
}

/// One NPC body in one instance.
#[derive(Debug, Clone)]
pub struct NpcAgent {
    /// Its placement, by index into the map's list.
    placement: usize,
    /// Where it is and how it is moving.
    pub body: AgentState,
    mind: Mind,
    /// A `/npc stop` asked for this body; the next tick sends `0x0028` once.
    stop_requested: bool,
    /// What the fight made it do since the last tick (a chase leg, a halt), for its viewers.
    pending: Vec<Msg>,
    /// It died in the fight and has not stood up: its entry burst shows a corpse.
    dead: bool,
    /// Its vanish/appear cycle, when its behavior has one.
    burrow: Option<BurrowState>,
}

/// A client the table may tell something: a player that has entered the instance and finished loading,
/// and where it stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewer {
    /// The player.
    pub player_id: u32,
    /// Where its agent is.
    pub position: (f32, f32),
}

/// What one client has been told.
#[derive(Debug, Clone, Default)]
struct Sight {
    /// The definition indices this connection has been sent.
    definitions: HashSet<u32>,
    /// The bodies it has been sent a create for and no remove since.
    visible: BTreeSet<u32>,
    /// Bodies it has ever been sent a create for, so a burrow re-create knows to use the five-message
    /// burst instead of the full entry.
    created: BTreeSet<u32>,
}

/// What one tick produced.
#[derive(Debug, Default)]
pub struct TickOutput {
    /// The messages for each player, in the order they are to be sent.
    pub deliveries: Vec<(u32, Vec<Msg>)>,
    /// Lines for the log: a body parked by a failing tree, with its placement and the reason.
    pub notes: Vec<String>,
    /// Bodies that began a vanish transition this tick: the instance stops their swings before the
    /// next combat tick.
    pub stopped: Vec<u32>,
}

/// One instance's NPCs. The default is a table with no bodies: a map the pack has no NPCs for.
#[derive(Debug, Clone, Default)]
pub struct NpcTable {
    map: Arc<MapNpcs>,
    rules: SightRules,
    agents: BTreeMap<u32, NpcAgent>,
    by_placement: HashMap<String, u32>,
    sights: HashMap<u32, Sight>,
    /// The hostile creatures' minds (`crate::npc::fight`).
    pub fight: Fight,
    /// Bodies that stood up again: every client that sees one is sent a remove and a fresh entry.
    reenter: BTreeSet<u32>,
}

/// The tree a placement runs, built: `Ok(None)` for a body with no behavior.
fn tree_for(
    placement: &NpcPlacement,
    map: &MapNpcs,
    behaviors: &dyn BehaviorSource,
) -> Result<Option<Tree>, String> {
    let Some(name) = &placement.behavior else {
        return Ok(None);
    };
    let spec = behaviors
        .spec(name)
        .ok_or_else(|| format!("behavior {name} is not one the behavior source can build"))?;
    Tree::build(&spec, placement.route.as_deref(), &map.routes, behaviors).map(Some)
}

/// Build every placement's tree once, so an unknown behavior or a construction error refuses the pack
/// at load instead of parking a body on its first instance.
pub fn check_trees(content: &NpcContent, behaviors: &dyn BehaviorSource) -> Vec<String> {
    let mut maps: Vec<&Arc<MapNpcs>> = content.maps.values().collect();
    maps.sort_by_key(|map| map.map_id);
    let mut problems = Vec::new();
    for map in maps {
        for placement in &map.placements {
            if let Err(err) = tree_for(placement, map, behaviors) {
                problems.push(format!(
                    "map {} placement {}: {err}",
                    map.map_id, placement.id
                ));
            }
        }
    }
    problems
}

/// `msgs` with the body's pace in front of each leg, for a placement that states a `walk_speed`.
fn paced(msgs: Vec<Msg>, agent_id: u32, walk_speed: Option<f32>) -> Vec<Msg> {
    let Some(speed) = walk_speed else {
        return msgs;
    };
    let mut out = Vec::with_capacity(msgs.len() + 1);
    for msg in msgs {
        if matches!(msg, Msg::MoveTo { .. }) {
            out.push(Msg::Pace { agent_id, speed });
        }
        out.push(msg);
    }
    out
}

impl NpcTable {
    /// Create every body of a map, in its file's order, under consecutive agent ids from `first_id`.
    /// A tree that does not build parks its body (and says why in the returned notes); the pack check
    /// at load ([`check_trees`]) is what keeps that from happening.
    pub fn seed(
        map: Arc<MapNpcs>,
        rules: SightRules,
        behaviors: &dyn BehaviorSource,
        first_id: u32,
    ) -> (NpcTable, Vec<String>) {
        let mut table = NpcTable {
            rules,
            ..NpcTable::default()
        };
        let mut notes = Vec::new();
        for (index, placement) in map.placements.iter().enumerate() {
            let agent_id = first_id + index as u32;
            let (mind, burrow) = match tree_for(placement, &map, behaviors) {
                Ok(None) => (Mind::Still, None),
                Ok(Some(tree)) => {
                    let burrow = tree
                        .burrow_terms()
                        .map(|(visible_ms, hidden_ms)| BurrowState::new(visible_ms, hidden_ms))
                        .or_else(|| tree.script_burrow().then(BurrowState::manual));
                    (Mind::Tree(tree), burrow)
                }
                Err(err) => {
                    notes.push(format!(
                        "map {} placement {}: parked: {err}",
                        map.map_id, placement.id
                    ));
                    (Mind::Parked, None)
                }
            };
            let body = AgentState::new(
                agent_id,
                placement.position,
                placement.plane,
                placement.direction,
                placement.walk_speed.unwrap_or(placement.definition.speed),
            );
            table.agents.insert(
                agent_id,
                NpcAgent {
                    placement: index,
                    body,
                    mind,
                    stop_requested: false,
                    pending: Vec::new(),
                    dead: false,
                    burrow,
                },
            );
            table.by_placement.insert(placement.id.clone(), agent_id);
            if let Some(combat) = placement.definition.combat.as_ref().filter(|c| c.hostile) {
                let style = match &combat.attack {
                    Some(attack) if attack.ranged => Style::Ranged,
                    Some(_) => Style::Melee,
                    None => Style::Caster,
                };
                table
                    .fight
                    .add(agent_id, placement.position, style, combat.skills.clone(), combat.aggression);
                if combat.stationary {
                    table.fight.pin(agent_id);
                }
            }
        }
        table.map = map;
        (table, notes)
    }

    /// Whether `agent_id` is one of this table's bodies.
    pub fn holds(&self, agent_id: u32) -> bool {
        self.agents.contains_key(&agent_id)
    }

    /// Whether this client has been sent `agent_id`'s create and not its remove: the bodies its agent
    /// array holds now. A fight message for a body outside this set would name an id the client has
    /// no slot for.
    pub fn knows(&self, player_id: u32, agent_id: u32) -> bool {
        self.sights
            .get(&player_id)
            .is_some_and(|sight| sight.visible.contains(&agent_id))
    }

    /// How many bodies the instance has.
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    /// Whether the instance has no bodies.
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    /// The agent a placement became in this instance: how a script names a body.
    pub fn agent_of(&self, placement: &str) -> Option<u32> {
        self.by_placement.get(placement).copied()
    }

    /// The agent whose placement carries this literal name, as the pack writes it in the
    /// `0x0108 0x0107 ... 0x0001` form. `None` for a name no body has, or one the server cannot read.
    pub fn agent_named(&self, name: &str) -> Option<u32> {
        self.agents.iter().find_map(|(agent_id, agent)| {
            let placement = self.map.placements.get(agent.placement)?;
            let units = placement.name.as_deref()?;
            let inner = units.strip_prefix(&[0x0108, 0x0107])?;
            let inner = inner.strip_suffix(&[0x0001]).unwrap_or(inner);
            let text: String = char::decode_utf16(inner.iter().copied())
                .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect();
            (text == name).then_some(*agent_id)
        })
    }

    /// One body, by agent id.
    pub fn agent(&self, agent_id: u32) -> Option<&NpcAgent> {
        self.agents.get(&agent_id)
    }

    /// The placement a body was created from: what an interaction, a script or a log names it by.
    pub fn placement_of(&self, agent_id: u32) -> Option<&NpcPlacement> {
        let agent = self.agents.get(&agent_id)?;
        self.map.placements.get(agent.placement)
    }

    /// Every body whose type has a combat block, with that block, its level and where it stands: what an
    /// instance puts in its fight when it is created (`crate::skill`).
    pub fn combatants(&self) -> Vec<(u32, crate::npc::content::NpcCombat, u32, (f32, f32))> {
        self.agents
            .iter()
            .filter_map(|(agent_id, agent)| {
                let placement = self.map.placements.get(agent.placement)?;
                let combat = placement.definition.combat.clone()?;
                let level = placement
                    .level
                    .unwrap_or(u32::from(placement.definition.level));
                Some((*agent_id, combat, level, agent.body.position))
            })
            .collect()
    }

    /// Where every body stands now.
    pub fn positions(&self) -> Vec<(u32, (f32, f32))> {
        self.agents
            .iter()
            .map(|(agent_id, agent)| (*agent_id, agent.body.position))
            .collect()
    }

    /// Where every body stands now, plane included: what a player walking to one is told
    /// (`crate::approach`).
    pub fn spots(&self) -> Vec<(u32, crate::pathfind::Pos)> {
        self.agents
            .iter()
            .map(|(agent_id, agent)| (*agent_id, agent.body.here()))
            .collect()
    }

    /// Carry out the walking half of the fight's orders (`crate::npc::fight`); the fight's own half is
    /// the instance's to run on the engine. A walk asks the path finder like a patrol's leg does, and a
    /// creature the finder cannot route stays where it is: no walk is invented.
    pub fn obey(&mut self, orders: &[(u32, Order)], pathing: &dyn Pathing) {
        let map_id = self.map.map_id;
        for (agent_id, order) in orders {
            let Some(agent) = self.agents.get_mut(agent_id) else {
                continue;
            };
            match order {
                Order::Walk { to } => {
                    let goal = crate::pathfind::Pos {
                        x: to.0,
                        y: to.1,
                        plane: agent.body.plane,
                    };
                    if let Walk::Found(waypoints) = pathing.leg(map_id, agent.body.here(), goal) {
                        if let Some(first) = agent.body.begin_walk(waypoints) {
                            agent.pending.push(Msg::MoveTo {
                                agent_id: *agent_id,
                                to: (first.pos.x, first.pos.y),
                                plane: first.pos.plane,
                                current_plane: agent.body.plane,
                            });
                        }
                    }
                }
                Order::Halt => behavior::stop_walk(&mut agent.body, &mut agent.pending),
                Order::Fall => agent.dead = true,
                Order::Revive { at } => {
                    agent.dead = false;
                    behavior::stop_walk(&mut agent.body, &mut Vec::new());
                    agent.body.position = *at;
                    agent.pending.clear();
                    self.reenter.insert(*agent_id);
                }
                Order::Attack { .. }
                | Order::UseSkills { .. }
                | Order::StopAttack
                | Order::Restore => {}
            }
        }
    }

    /// Answer an owner's `/npc` test command, as console lines.
    ///
    /// This is not the reference's chat surface. The commands drive the table this server owns, so a
    /// tester can see what a body is and start or stop its authored route without editing the pack and
    /// restarting the world.
    pub fn command(&mut self, argument: &str) -> Vec<String> {
        let (verb, rest) = argument.split_once(' ').unwrap_or((argument, ""));
        match (verb, rest.trim()) {
            ("list", "") => self.describe(),
            ("patrol", what) if !what.is_empty() => {
                self.each(what, true, |table, id| match table.start_patrol(id) {
                    Ok(agent_id) => format!("patrol on agent {agent_id}"),
                    Err(err) => err,
                })
            }
            ("stop", what) if !what.is_empty() => {
                self.each(what, false, |table, id| match table.body_of(id) {
                    Ok(agent) => {
                        agent.mind = Mind::Still;
                        agent.burrow = None;
                        agent.stop_requested = true;
                        format!("stopped (agent {})", agent.body.agent_id)
                    }
                    Err(err) => err,
                })
            }
            ("help" | "", _) => vec![
                "usage: /npc list | /npc patrol <placement|all> | /npc stop <placement|all>"
                    .to_string(),
            ],
            _ => vec![format!("unknown /npc verb {verb:?}; try /npc help")],
        }
    }

    /// Run `act` for the placement `what` names, or for every placement (`routed`: every placement
    /// with a route) when it is `all`, and answer one `id: result` line each.
    fn each(
        &mut self,
        what: &str,
        routed: bool,
        mut act: impl FnMut(&mut NpcTable, &str) -> String,
    ) -> Vec<String> {
        let ids: Vec<String> = match what {
            "all" => (self.map.placements.iter())
                .filter(|placement| !routed || placement.route.is_some())
                .map(|placement| placement.id.clone())
                .collect(),
            one => vec![one.to_string()],
        };
        if ids.is_empty() {
            return vec!["no placement on this map has a route".to_string()];
        }
        ids.iter()
            .map(|id| format!("{id}: {}", act(self, id)))
            .collect()
    }

    fn body_of(&mut self, placement: &str) -> Result<&mut NpcAgent, String> {
        self.by_placement
            .get(placement)
            .and_then(|agent_id| self.agents.get_mut(agent_id))
            .ok_or_else(|| "no such placement on this map".to_string())
    }

    /// One line per body: its placement, where it is, what drives it, its route and its held items.
    fn describe(&self) -> Vec<String> {
        self.agents
            .values()
            .map(|agent| {
                let placement = &self.map.placements[agent.placement];
                let state = match agent.mind {
                    Mind::Tree(_) => "tree",
                    Mind::Still => "still",
                    Mind::Done => "done",
                    Mind::Parked => "parked",
                };
                let route = (placement.route.as_deref())
                    .map(|route| format!(" route {route}"))
                    .unwrap_or_default();
                format!(
                    "{} [{}] ({:.1}, {:.1}) plane {} agent {} {state}{route} {} item(s)",
                    placement.id,
                    placement.definition.id,
                    agent.body.position.0,
                    agent.body.position.1,
                    agent.body.plane,
                    agent.body.agent_id,
                    placement.items.len(),
                )
            })
            .collect()
    }

    fn start_patrol(&mut self, id: &str) -> Result<u32, String> {
        let map = Arc::clone(&self.map);
        let agent = self.body_of(id)?;
        let route = map.placements[agent.placement].route.as_deref();
        agent.mind = Mind::Tree(Tree::build(
            &Spec::Patrol { route: None },
            route,
            &map.routes,
            &PackBehaviors,
        )?);
        agent.burrow = None;
        agent.stop_requested = false;
        Ok(agent.body.agent_id)
    }

    /// The burst that makes one body appear on one client, with its current walk when it is walking,
    /// and the client's record of it.
    fn enter(map: &MapNpcs, sight: &mut Sight, agent: &NpcAgent) -> Vec<Msg> {
        let agent_id = agent.body.agent_id;
        let placement = &map.placements[agent.placement];
        let known = !sight
            .definitions
            .insert(placement.definition.definition_index);
        sight.visible.insert(agent_id);
        sight.created.insert(agent_id);
        let status = agent
            .burrow
            .map_or(placement.effects, |burrow| burrow.status());
        let mut msgs = burst::entry_with_status(placement, &agent.body, known, status);
        if agent.dead {
            // The effect word's dead bit, as the fight sent it when the body died
            // (`crate::skill::engine::flags::DEAD`).
            for msg in &mut msgs {
                if let Msg::InitialEffects { effects, .. } = msg {
                    *effects |= crate::skill::engine::flags::DEAD;
                }
            }
        }
        if 0.0 < agent.body.motion.speed {
            let to = agent.body.motion.destination;
            msgs.push(Msg::MoveTo {
                agent_id,
                to: (to.x, to.y),
                plane: to.plane,
                current_plane: agent.body.plane,
            });
        }
        paced(msgs, agent_id, placement.walk_speed)
    }

    /// Bring one client's record up to where it stands: a remove for each body that left its range,
    /// and an entry burst, by agent id, for each that came into it.
    fn look(&mut self, viewer: Viewer) -> (Vec<Msg>, Vec<(u32, Vec<Msg>)>) {
        let sight = self.sights.entry(viewer.player_id).or_default();
        let wanted: BTreeSet<u32> = (self.agents.iter())
            .filter(|(_, agent)| {
                !agent.burrow.is_some_and(BurrowState::hidden)
                    && self.rules.sees(viewer.position, agent.body.position)
            })
            .map(|(agent_id, _)| *agent_id)
            .collect();
        let leaving: Vec<u32> = sight.visible.difference(&wanted).copied().collect();
        let entering: Vec<u32> = wanted.difference(&sight.visible).copied().collect();
        let removes = leaving
            .into_iter()
            .map(|agent_id| {
                sight.visible.remove(&agent_id);
                Msg::Remove { agent_id }
            })
            .collect();
        let entries = entering
            .into_iter()
            .map(|agent_id| {
                let burst = Self::enter(&self.map, sight, &self.agents[&agent_id]);
                (agent_id, burst)
            })
            .collect();
        (removes, entries)
    }

    /// A client is loading into the instance: the bodies within its range, each as its entry burst, by
    /// agent id. Its record starts empty, so the first body of each definition carries the definition.
    pub fn arrive(&mut self, player_id: u32, position: (f32, f32)) -> Vec<(u32, Vec<Msg>)> {
        self.sights.remove(&player_id);
        self.look(Viewer {
            player_id,
            position,
        })
        .1
    }

    /// A client has left: forget what it was told.
    pub fn forget(&mut self, player_id: u32) {
        self.sights.remove(&player_id);
    }

    /// One tick: advance and think for every active body, then tell each viewer what changed - the
    /// walks of the bodies it can see, a remove for each body that left its range, and an entry burst
    /// for each that came into it.
    pub fn tick(&mut self, delta: f32, viewers: &[Viewer], pathing: &dyn Pathing) -> TickOutput {
        let mut output = TickOutput::default();
        if self.agents.is_empty() {
            return output;
        }
        let elapsed_ms = (delta * 1000.0).round() as u32;

        // What every active body did.
        let mut said: Vec<(u32, Vec<Msg>)> = Vec::new();
        let mut removed: Vec<u32> = Vec::new();
        let mut submerging: Vec<u32> = Vec::new();
        let mut emerged: Vec<u32> = Vec::new();
        for (agent_id, agent) in self.agents.iter_mut() {
            let position = agent.body.position;
            if !viewers
                .iter()
                .any(|viewer| self.rules.sees(viewer.position, position))
            {
                agent.pending.clear();
                continue;
            }
            let placement = &self.map.placements[agent.placement];
            let mut out = std::mem::take(&mut agent.pending);
            if std::mem::take(&mut agent.stop_requested) {
                behavior::stop_walk(&mut agent.body, &mut out);
            }
            if let Some(waypoint) = agent.body.advance(delta) {
                out.push(Msg::MoveTo {
                    agent_id: *agent_id,
                    to: (waypoint.pos.x, waypoint.pos.y),
                    plane: waypoint.pos.plane,
                    current_plane: agent.body.plane,
                });
            }
            // A burrower's phase clock runs while the body is active. Death interrupts the cycle:
            // a corpse does not submerge (Rurik `studies/cmsg/FINDINGS.md` T16).
            if !agent.dead {
                if let Some(event) = agent.burrow.as_mut().and_then(|borrow| borrow.advance(elapsed_ms)) {
                    match event {
                        BurrowEvent::Out => out.push(Msg::Status {
                            agent_id: *agent_id,
                            status: 0,
                        }),
                        BurrowEvent::Submerging => {
                            out.push(Msg::Status {
                                agent_id: *agent_id,
                                status: burst::BURROW_TRANSITION,
                            });
                            submerging.push(*agent_id);
                        }
                        BurrowEvent::Hidden => removed.push(*agent_id),
                        BurrowEvent::Emerging => emerged.push(*agent_id),
                    }
                }
            }
            // A body the fight drives (a chase, a walk home, death) leaves its own behavior waiting.
            let fighting = self.fight.drives(*agent_id);
            if let Mind::Tree(tree) = &mut agent.mind {
                // A fight leaves ordinary behavior waiting; a script leaf is what lets a script react
                // to the fight (a worm that emerges while it is being attacked), so it keeps ticking.
                if fighting && !tree.is_script_only() {
                    if !out.is_empty() {
                        said.push((*agent_id, paced(out, *agent_id, placement.walk_speed)));
                    }
                    continue;
                }
                let burrow_phase = agent.burrow.as_ref().map(|burrow| burrow.phase_name());
                let mut ctx = Ctx {
                    body: &mut agent.body,
                    map_id: self.map.map_id,
                    elapsed_ms,
                    pathing,
                    out: &mut out,
                    failure: None,
                    in_combat: fighting,
                    burrow_phase,
                    burrow_request: None,
                };
                match tree.tick(&mut ctx) {
                    Status::Running => {}
                    Status::Success => agent.mind = Mind::Done,
                    Status::Failure => {
                        let reason = ctx.failure.take().unwrap_or_else(|| "no reason".into());
                        output.notes.push(format!(
                            "map {} placement {}: parked: {reason}",
                            self.map.map_id, placement.id
                        ));
                        agent.mind = Mind::Parked;
                    }
                }
                if let Some(request) = ctx.burrow_request.take() {
                    if let Some(burrow) = agent.burrow.as_mut() {
                        burrow.request(request);
                    }
                }
            }
            if !out.is_empty() {
                said.push((*agent_id, paced(out, *agent_id, placement.walk_speed)));
            }
        }

        output.stopped = submerging;

        // A body that stood up again is a new body to every client that sees it: removed here, it is
        // in range and not visible, so the look below sends its entry afresh.
        let reenter = std::mem::take(&mut self.reenter);

        // Each viewer: what its visible bodies did, then the bodies that left, then the ones that came.
        for viewer in viewers {
            let seen = self.sights.entry(viewer.player_id).or_default();
            let mut gone = Vec::new();
            for agent_id in &reenter {
                if seen.visible.remove(agent_id) {
                    gone.push(Msg::Remove {
                        agent_id: *agent_id,
                    });
                }
            }
            // A submerging body is removed after its 2.00 s animation; the remove belongs to the
            // viewers that saw the animation, not to the range sweep.
            for agent_id in &removed {
                if seen.visible.remove(agent_id) {
                    gone.push(Msg::Remove {
                        agent_id: *agent_id,
                    });
                }
            }
            // A hidden body re-appears with the five-message burst Rurik measured, sent only to a
            // client that was already told this body once. A client that has not is handled by the
            // range sweep below, which sends the ordinary entry with the emerge status.
            for agent_id in &emerged {
                if !seen.created.contains(agent_id)
                    || seen.visible.contains(agent_id)
                    || !self.rules.sees(viewer.position, self.agents[agent_id].body.position)
                {
                    continue;
                }
                let agent = &self.agents[agent_id];
                let placement = &self.map.placements[agent.placement];
                gone.extend(burst::burrow_recreate(placement, &agent.body));
                seen.visible.insert(*agent_id);
            }
            let mut msgs: Vec<Msg> = said
                .iter()
                .filter(|(agent_id, _)| seen.visible.contains(agent_id))
                .flat_map(|(_, out)| out.iter().cloned())
                .collect();
            msgs.extend(gone);
            let (removes, entries) = self.look(*viewer);
            msgs.extend(removes);
            msgs.extend(entries.into_iter().flat_map(|(_, burst)| burst));
            if !msgs.is_empty() {
                output.deliveries.push((viewer.player_id, msgs));
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::super::behavior::tests::{square, Flat};
    use super::super::content::tests::{definition, map, placement, sight};
    use super::super::content::{MapRecord, RouteMode};
    use super::*;

    /// A source that knows no behavior, as a pack whose scripts are missing would be.
    struct NoBehaviors;
    impl BehaviorSource for NoBehaviors {
        fn spec(&self, _name: &str) -> Option<Spec> {
            None
        }
    }

    fn content(file: MapRecord) -> NpcContent {
        NpcContent::assemble(
            sight(),
            vec![definition("guard", 0x5B3), definition("cook", 0x5C8)],
            vec![file],
        )
        .expect("assembles")
    }

    fn town() -> NpcTable {
        let file = map(
            148,
            vec![
                placement("a", "guard", (0.0, 0.0)),
                placement("b", "guard", (100.0, 0.0)),
                placement("c", "cook", (9000.0, 0.0)),
            ],
        );
        let content = content(file);
        let (table, notes) =
            NpcTable::seed(content.map(148).expect("map"), sight(), &NoBehaviors, 41);
        assert!(notes.is_empty());
        table
    }

    fn headers(msgs: &[Msg]) -> Vec<&'static str> {
        msgs.iter()
            .map(|msg| match msg {
                Msg::Definition(_) => "def",
                Msg::Model(_) => "model",
                Msg::Create(_) => "create",
                Msg::Remove { .. } => "remove",
                Msg::MoveTo { .. } => "move",
                Msg::Pace { .. } => "pace",
                _ => "-",
            })
            .filter(|name| *name != "-")
            .collect()
    }

    /// A source that builds the measured vanish/appear cycle, as a pack's Lua script does.
    struct Burrowing;

    impl BehaviorSource for Burrowing {
        fn spec(&self, name: &str) -> Option<Spec> {
            (name == "burrow").then_some(Spec::Burrow {
                visible_ms: 5000,
                hidden_ms: 4000,
            })
        }
    }

    fn burrower() -> NpcTable {
        let mut record = placement("worm", "guard", (0.0, 0.0));
        record.behavior = Some("burrow".to_string());
        let content = content(map(146, vec![record]));
        let (table, notes) = NpcTable::seed(
            content.map(146).expect("map"),
            sight(),
            &Burrowing,
            9,
        );
        assert!(notes.is_empty());
        table
    }

    fn kinds(msgs: &[Msg]) -> Vec<String> {
        msgs.iter()
            .map(|msg| match msg {
                Msg::Definition(_) => "def",
                Msg::Model(_) => "model",
                Msg::Create(_) => "create",
                Msg::Remove { .. } => "remove",
                Msg::Status { status: 0, .. } => "out",
                Msg::Status { .. } => "transition",
                Msg::InitialEffects { effects, .. } if *effects == burst::BURROW_TRANSITION => {
                    "transition"
                }
                Msg::IntProperty { id: 66, .. } => "prop66",
                Msg::Weapons { .. } => "weapons",
                Msg::Flags { .. } => "flags",
                _ => "other",
            })
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn a_burrower_cycles_with_the_measured_transitions_and_no_definition_resend() {
        let mut table = burrower();
        let bursts = table.arrive(1, (0.0, 0.0));
        assert_eq!(headers(&bursts[0].1), vec!["def", "model", "create"]);
        let status = bursts[0].1.iter().find_map(|msg| match msg {
            Msg::InitialEffects { effects, .. } => Some(*effects),
            _ => None,
        });
        assert_eq!(status, Some(burst::BURROW_TRANSITION));

        let viewer = Viewer {
            player_id: 1,
            position: (0.0, 0.0),
        };
        let out = table.tick(2.0, &[viewer], &Flat);
        assert_eq!(kinds(&out.deliveries[0].1), vec!["out"]);

        let out = table.tick(5.0, &[viewer], &Flat);
        assert_eq!(kinds(&out.deliveries[0].1), vec!["transition"]);
        assert_eq!(out.stopped, vec![9]);

        let out = table.tick(2.0, &[viewer], &Flat);
        assert_eq!(kinds(&out.deliveries[0].1), vec!["remove"]);

        let out = table.tick(4.0, &[viewer], &Flat);
        assert_eq!(
            kinds(&out.deliveries[0].1),
            vec!["prop66", "transition", "create", "weapons", "flags"],
            "the five-message re-create, with no 0x0056/0x0057"
        );

        let out = table.tick(2.0, &[viewer], &Flat);
        assert_eq!(kinds(&out.deliveries[0].1), vec!["out"]);
    }

    #[test]
    fn a_manual_burrow_waits_for_the_script_and_honours_its_hidden_span() {
        let mut burrow = BurrowState::manual();
        assert_eq!(burrow.phase_name(), "emerging");
        assert_eq!(burrow.advance(2000), Some(BurrowEvent::Out));
        assert_eq!(burrow.phase_name(), "out");
        assert_eq!(burrow.advance(10_000), None, "a script owns the surface");
        burrow.request(BurrowRequest::Submerge { hidden_ms: 4000 });
        assert_eq!(burrow.advance(0), Some(BurrowEvent::Submerging));
        assert_eq!(burrow.advance(2000), Some(BurrowEvent::Hidden));
        assert_eq!(burrow.advance(1000), None);
        assert_eq!(burrow.advance(3000), Some(BurrowEvent::Emerging));
        assert_eq!(burrow.advance(2000), Some(BurrowEvent::Out));
        assert_eq!(burrow.advance(10_000), None);

        let mut held = BurrowState::manual();
        held.advance(2000);
        held.request(BurrowRequest::Submerge { hidden_ms: 0 });
        held.advance(0);
        held.advance(2000);
        assert_eq!(held.phase_name(), "hidden");
        assert_eq!(held.advance(60_000), None, "0 waits for ctx:emerge()");
        held.request(BurrowRequest::Emerge);
        assert_eq!(held.advance(0), Some(BurrowEvent::Emerging));
    }

    #[test]
    fn bodies_take_ids_in_file_order_and_are_found_by_placement() {
        let table = town();
        assert_eq!(table.len(), 3);
        assert_eq!(table.agent_of("a"), Some(41));
        assert_eq!(table.agent_of("c"), Some(43));
        assert!(table.holds(42) && !table.holds(44));
        assert_eq!(table.placement_of(42).map(|p| p.id.as_str()), Some("b"));
        assert!(table.placement_of(44).is_none());
    }

    #[test]
    fn an_arriving_client_gets_each_definition_once_before_its_first_body() {
        let mut table = town();
        let bursts = table.arrive(1, (0.0, 0.0));
        let ids: Vec<u32> = bursts.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![41, 42], "the cook is out of range");
        assert_eq!(headers(&bursts[0].1), vec!["def", "model", "create"]);
        assert_eq!(headers(&bursts[1].1), vec!["create"], "guard already known");
    }

    #[test]
    fn walking_into_range_creates_and_walking_out_removes() {
        let mut table = town();
        table.arrive(1, (0.0, 0.0));
        let viewer = |x| Viewer {
            player_id: 1,
            position: (x, 0.0),
        };
        let out = table.tick(0.1, &[viewer(8000.0)], &Flat);
        let (_, msgs) = &out.deliveries[0];
        assert_eq!(
            headers(msgs),
            vec!["remove", "remove", "def", "model", "create"],
            "both guards leave, the cook arrives with its definition: {msgs:?}"
        );
        let out = table.tick(0.1, &[viewer(0.0)], &Flat);
        let (_, msgs) = &out.deliveries[0];
        assert_eq!(
            headers(msgs),
            vec!["remove", "create", "create"],
            "the guards come back without their definition"
        );
        let out = table.tick(0.1, &[viewer(0.0)], &Flat);
        assert!(out.deliveries.is_empty(), "nothing changed, nothing sent");
    }

    #[test]
    fn two_clients_have_separate_records_and_a_forgotten_one_starts_over() {
        let mut table = town();
        table.arrive(1, (0.0, 0.0));
        let second = table.arrive(2, (0.0, 0.0));
        assert_eq!(headers(&second[0].1), vec!["def", "model", "create"]);
        table.forget(2);
        let again = table.arrive(2, (0.0, 0.0));
        assert_eq!(headers(&again[0].1), vec!["def", "model", "create"]);
    }

    fn patrolling(behaviors: &dyn BehaviorSource) -> (NpcTable, Vec<String>) {
        let mut walker = placement("w", "guard", (0.0, 0.0));
        walker.behavior = Some("patrol".to_string());
        walker.route = Some("square".to_string());
        let mut file = map(148, vec![walker]);
        file.routes.push(square(RouteMode::Loop, 0));
        let content = content(file);
        NpcTable::seed(content.map(148).expect("map"), sight(), behaviors, 9)
    }

    #[test]
    fn a_walk_speed_sends_the_speed_pair_before_each_leg() {
        let mut walker = placement("w", "guard", (0.0, 0.0));
        walker.behavior = Some("patrol".to_string());
        walker.route = Some("square".to_string());
        walker.walk_speed = Some(220.0);
        let mut file = map(148, vec![walker]);
        file.routes.push(square(RouteMode::Loop, 0));
        let content = content(file);
        let (mut table, _) =
            NpcTable::seed(content.map(148).expect("map"), sight(), &PackBehaviors, 9);
        let near = Viewer {
            player_id: 1,
            position: (0.0, 0.0),
        };
        table.arrive(1, near.position);
        let out = table.tick(0.0, &[near], &Flat);
        assert_eq!(
            headers(&out.deliveries[0].1),
            vec!["pace", "move"],
            "0x0027 and 0x002B precede the leg"
        );
        assert_eq!(
            table.agent(9).expect("body").body.motion.speed,
            220.0,
            "the server walks at the same speed it tells the client"
        );
    }

    #[test]
    fn a_patrol_moves_only_while_someone_is_in_range() {
        let (mut table, notes) = patrolling(&PackBehaviors);
        assert!(notes.is_empty());
        table.arrive(1, (0.0, 0.0));
        let near = Viewer {
            player_id: 1,
            position: (0.0, 0.0),
        };
        let out = table.tick(0.0, &[near], &Flat);
        assert_eq!(headers(&out.deliveries[0].1), vec!["move"]);
        table.tick(0.5, &[near], &Flat);
        let walked = table.agent(9).expect("body").body.position;
        assert_eq!(walked, (100.0, 0.0), "the first leg ends at its point");
        let far = Viewer {
            player_id: 1,
            position: (20000.0, 0.0),
        };
        let out = table.tick(0.5, &[far], &Flat);
        assert_eq!(headers(&out.deliveries[0].1), vec!["remove"]);
        assert_eq!(
            table.agent(9).expect("body").body.position,
            walked,
            "dormant: nobody in range, nothing moved"
        );
    }

    #[test]
    fn the_npc_command_lists_starts_and_stops_a_body() {
        let (mut table, _) = patrolling(&PackBehaviors);
        let listed = table.command("list");
        assert!(
            listed.iter().any(|line| line.contains("w [guard]")),
            "{listed:?}"
        );
        let started = table.command("patrol w");
        assert_eq!(started, vec!["w: patrol on agent 9"]);
        let near = Viewer {
            player_id: 1,
            position: (0.0, 0.0),
        };
        table.arrive(1, near.position);
        table.tick(0.0, &[near], &Flat);
        let stopped = table.command("stop w");
        assert!(stopped[0].contains("stopped"), "{stopped:?}");
        let out = table.tick(0.1, &[near], &Flat);
        assert!(
            out.deliveries
                .iter()
                .flat_map(|(_, msgs)| msgs)
                .any(|msg| matches!(msg, Msg::Stop { agent_id: 9 })),
            "the next tick sends the stop"
        );
        let listed = table.command("list");
        assert!(
            listed.iter().any(|line| line.contains("still")),
            "{listed:?}"
        );
    }

    #[test]
    fn a_body_that_enters_mid_walk_is_created_with_its_walk() {
        let (mut table, _) = patrolling(&PackBehaviors);
        let watcher = Viewer {
            player_id: 1,
            position: (0.0, 0.0),
        };
        table.arrive(1, watcher.position);
        table.tick(0.0, &[watcher], &Flat);
        let bursts = table.arrive(2, (0.0, 0.0));
        assert_eq!(
            headers(&bursts[0].1),
            vec!["def", "model", "create", "move"]
        );
    }

    #[test]
    fn a_tree_that_cannot_be_built_parks_its_body_and_is_caught_at_load() {
        let (table, notes) = patrolling(&NoBehaviors);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("placement w: parked"), "{notes:?}");
        assert_eq!(table.len(), 1, "the body still exists and stands");
        let mut walker = placement("w", "guard", (0.0, 0.0));
        walker.behavior = Some("patrol".to_string());
        let content = content(map(148, vec![walker]));
        let problems = check_trees(&content, &PackBehaviors);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("names no route"), "{problems:?}");
    }

    #[test]
    fn a_failing_leg_parks_the_body_and_says_so_once() {
        let mut walker = placement("w", "guard", (0.0, 0.0));
        walker.behavior = Some("patrol".to_string());
        walker.route = Some("square".to_string());
        let mut file = map(148, vec![walker]);
        let mut route = square(RouteMode::Loop, 0);
        route.points[0].position = (-1.0, 0.0);
        file.routes.push(route);
        let content = content(file);
        let (mut table, _) =
            NpcTable::seed(content.map(148).expect("map"), sight(), &PackBehaviors, 9);
        let near = Viewer {
            player_id: 1,
            position: (0.0, 0.0),
        };
        let out = table.tick(0.1, &[near], &Flat);
        assert_eq!(out.notes.len(), 1, "{:?}", out.notes);
        assert!(out.notes[0].contains("no path"));
        let out = table.tick(0.1, &[near], &Flat);
        assert!(out.notes.is_empty(), "parked once, not every tick");
    }
}
