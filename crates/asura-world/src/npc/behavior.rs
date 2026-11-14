//! The behavior-tree runtime: Rust ticks, a constructor describes.
//!
//! `docs/adr/0014-behavior-tree-execution.md` fixes the division: the runtime, its node semantics and
//! every running state are Rust's; the tree's *shape* comes from external configuration, which Lua
//! constructs. [`Spec`] is that shape - the value a constructor returns - and [`Tree`] is one agent's
//! running copy of it. Two agents never share a [`Tree`]: sharing one would mix their progress.
//!
//! The node set is the one `docs/49-npc-framework.md` section 3.8 needs and no more:
//!
//! | node | kind | semantics (BehaviorTree.CPP 4.8, pinned in ADR 0014) |
//! |---|---|---|
//! | [`Spec::Sequence`] | control | `Sequence`: resumes at the running child; a failure resets it |
//! | [`Spec::Fallback`] | control | `Fallback`: resumes at the running child; a success resets it |
//! | [`Spec::Wait`] | leaf | holds for a duration on the tree clock, then succeeds |
//! | [`Spec::Stand`] | leaf | runs forever and sends nothing: a body that stays where it is |
//! | [`Spec::Patrol`] | leaf | walks a route leg by leg, pausing where a point says |
//! | [`Spec::Burrow`] | leaf | runs forever while the table cycles the body's vanish/appear wire |
//!
//! Halting is a cancel, not a memory reset (ADR 0014, "Halt and reset"): a halted `Patrol` stops the
//! body and tells its clients, and the next start resumes the leg it abandoned.
//!
//! The clock is the instance tick's elapsed time, passed in. Nothing here reads a wall clock, so a
//! tree's run is a function of its inputs.

use super::burst::Msg;
use super::content::{Route, RouteMode};
use super::table::BehaviorSource;
use crate::agent::{AgentState, Walk};
use crate::pathfind::Pos;
use mlua::{Function, Table};
use std::collections::BTreeMap;

/// What a node says after a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Not finished; tick again.
    Running,
    /// Finished and done.
    Success,
    /// Finished and could not.
    Failure,
}

/// A tree's shape, as a constructor returns it.
#[derive(Debug, Clone, PartialEq)]
pub enum Spec {
    /// See the module table.
    Sequence(Vec<Spec>),
    /// See the module table.
    Fallback(Vec<Spec>),
    /// See the module table.
    Wait {
        /// How long, in milliseconds of tree clock.
        ms: u32,
    },
    /// See the module table.
    Stand,
    /// See the module table.
    Patrol {
        /// The route by id; `None` walks the placement's own route.
        route: Option<String>,
    },
    /// A vanish/appear cycle: the body starts in its emerge transition, stands visible for
    /// `visible_ms`, submerges for the measured 2.00 s, is removed for `hidden_ms`, and re-appears.
    /// The table owns the wire and the phase timers; this node only keeps the tree running.
    Burrow {
        /// Milliseconds the body stands out and targetable after the emerge transition.
        visible_ms: u32,
        /// Milliseconds the body stays removed before the next emerge transition.
        hidden_ms: u32,
    },
    /// A Lua function called once per tick (`docs/59`). `burrow` starts the body in the emerge
    /// transition, so a script that owns the cycle begins where a [`Spec::Burrow`] body does.
    Script {
        /// The name the script table uses; the source resolves it to the function.
        name: String,
        /// Whether the body starts emerging and accepts `ctx:burrow()`/`ctx:emerge()`.
        burrow: bool,
    },
}

/// A burrow transition a script asked for. The table applies it on the next tick, through the same
/// measured wire a [`Spec::Burrow`] body uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BurrowRequest {
    /// Leave the surface: the 2.00 s submerge animation, then hidden. A `hidden_ms` of 0 stays hidden
    /// until `Emerge`; a positive value emerges by itself after that many milliseconds.
    Submerge { hidden_ms: u32 },
    /// Come back early from hidden: the 2.00 s emerge animation.
    Emerge,
}

/// One script leaf's per-agent Lua state: the function, its persistent state table, the context it is
/// called with, and the command table the context's methods append to.
#[derive(Clone)]
pub struct Script {
    /// The Lua function.
    pub function: Function,
    /// The table passed as `ctx` on every call.
    pub context: Table,
    /// The persistent `ctx.state` table.
    pub state: Table,
    /// The commands the context's methods wrote this tick.
    pub commands: Table,
}

impl std::fmt::Debug for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Script").finish_non_exhaustive()
    }
}

/// The path question a walk asks: the map's own finder, or a test's.
pub trait Pathing {
    /// A walk from `from` to `to` on `map_id`.
    fn leg(&self, map_id: i32, from: Pos, to: Pos) -> Walk;
}

/// What a tick can see and change: the agent's body, its map, the tick's elapsed time, the path finder,
/// and where its messages go.
pub struct Ctx<'a> {
    /// The body the tree drives.
    pub body: &'a mut AgentState,
    /// The map the body is on.
    pub map_id: i32,
    /// Tree-clock milliseconds since the previous tick.
    pub elapsed_ms: u32,
    /// The finder a walk asks.
    pub pathing: &'a dyn Pathing,
    /// The messages this tick produced, for every client that can see the body.
    pub out: &'a mut Vec<Msg>,
    /// Why the last leaf that failed failed, for the log line that parks the agent.
    pub failure: Option<String>,
    /// Whether a fight is driving this body: what `ctx.in_combat` reads.
    pub in_combat: bool,
    /// The body's burrow phase, when it has one: what `ctx.phase` reads.
    pub burrow_phase: Option<&'static str>,
    /// A transition a script asked for; the table applies it on the next tick.
    pub burrow_request: Option<BurrowRequest>,
}

/// One agent's running tree.
#[derive(Debug, Clone)]
pub struct Tree {
    nodes: Vec<Node>,
    /// The first [`Spec::Burrow`] in the tree, if it has one: the terms the table's cycle runs on.
    burrow: Option<(u32, u32)>,
    /// Whether a [`Spec::Script`] asked to own the burrow cycle.
    script_burrow: bool,
}

#[derive(Debug, Clone)]
struct Node {
    kind: Kind,
    children: Vec<usize>,
    running: bool,
}

#[derive(Debug, Clone)]
enum Kind {
    Sequence { cursor: usize },
    Fallback { cursor: usize },
    Wait { ms: u32, left: Option<u32> },
    Stand,
    Patrol(Patrol),
    /// The table owns the vanish/appear wire; this node stays running so the tree is not parked.
    Burrow,
    /// A Lua function called once per tick with the context table.
    Script { script: Script },
}

impl Tree {
    /// Build one agent's tree from a spec. A `Patrol` with no route of its own takes `own_route`; a
    /// route that does not resolve is a construction error, which refuses the pack at load.
    pub fn build(
        spec: &Spec,
        own_route: Option<&str>,
        routes: &BTreeMap<String, Route>,
        behaviors: &dyn BehaviorSource,
    ) -> Result<Tree, String> {
        let mut tree = Tree {
            nodes: Vec::new(),
            burrow: None,
            script_burrow: false,
        };
        tree.add(spec, own_route, routes, behaviors)?;
        Ok(tree)
    }

    /// The vanish/appear terms this tree runs on, when it has a [`Spec::Burrow`]. The table starts
    /// the cycle when the body is seeded, so a `Burrow` belongs at the root of a body's tree.
    pub fn burrow_terms(&self) -> Option<(u32, u32)> {
        self.burrow
    }

    /// Whether a script leaf asked for the body to start emerging and own its burrow cycle.
    pub fn script_burrow(&self) -> bool {
        self.script_burrow
    }

    /// Whether the tree is one script leaf: the shape that may tick while a fight drives the body.
    pub fn is_script_only(&self) -> bool {
        self.nodes.len() == 1 && matches!(&self.nodes[0].kind, Kind::Script { .. })
    }

    fn add(
        &mut self,
        spec: &Spec,
        own_route: Option<&str>,
        routes: &BTreeMap<String, Route>,
        behaviors: &dyn BehaviorSource,
    ) -> Result<usize, String> {
        let index = self.nodes.len();
        let (kind, children) = match spec {
            Spec::Sequence(children) => (Kind::Sequence { cursor: 0 }, children.as_slice()),
            Spec::Fallback(children) => (Kind::Fallback { cursor: 0 }, children.as_slice()),
            Spec::Wait { ms } => (
                Kind::Wait {
                    ms: *ms,
                    left: None,
                },
                &[][..],
            ),
            Spec::Stand => (Kind::Stand, &[][..]),
            Spec::Patrol { route } => {
                let id = route
                    .as_deref()
                    .or(own_route)
                    .ok_or("a Patrol names no route and its placement has none")?;
                let route = routes
                    .get(id)
                    .ok_or_else(|| format!("Patrol route {id} is not on this map"))?;
                (Kind::Patrol(Patrol::new(route.clone())), &[][..])
            }
            Spec::Burrow {
                visible_ms,
                hidden_ms,
            } => {
                if self.burrow.replace((*visible_ms, *hidden_ms)).is_some() {
                    return Err("a tree names more than one Burrow".to_string());
                }
                (Kind::Burrow, &[][..])
            }
            Spec::Script { name, burrow } => {
                let script = behaviors
                    .script(name)
                    .ok_or_else(|| format!("script {name} is not a Lua function"))?;
                self.script_burrow |= *burrow;
                (Kind::Script { script }, &[][..])
            }
        };
        if matches!(&kind, Kind::Sequence { .. } | Kind::Fallback { .. }) && children.is_empty() {
            return Err("a Sequence or a Fallback has no children".to_string());
        }
        self.nodes.push(Node {
            kind,
            children: Vec::new(),
            running: false,
        });
        for child in children {
            let child = self.add(child, own_route, routes, behaviors)?;
            self.nodes[index].children.push(child);
        }
        Ok(index)
    }

    /// Tick the tree once from its root.
    pub fn tick(&mut self, ctx: &mut Ctx<'_>) -> Status {
        self.tick_node(0, ctx)
    }

    /// Halt every running node: running work is cancelled, and the tree starts afresh on its next tick
    /// (a `Patrol` keeps the point it was walking to).
    pub fn halt(&mut self, ctx: &mut Ctx<'_>) {
        self.halt_node(0, ctx);
    }

    fn tick_node(&mut self, index: usize, ctx: &mut Ctx<'_>) -> Status {
        let status = match self.nodes[index].kind {
            Kind::Sequence { .. } => self.tick_control(index, ctx, Status::Success),
            Kind::Fallback { .. } => self.tick_control(index, ctx, Status::Failure),
            Kind::Wait { ms, left } => {
                let left = match left {
                    None => ms,
                    Some(left) => left.saturating_sub(ctx.elapsed_ms),
                };
                if left == 0 {
                    self.nodes[index].kind = Kind::Wait { ms, left: None };
                    Status::Success
                } else {
                    self.nodes[index].kind = Kind::Wait {
                        ms,
                        left: Some(left),
                    };
                    Status::Running
                }
            }
            Kind::Stand | Kind::Burrow => Status::Running,
            Kind::Patrol(ref mut patrol) => patrol.tick(ctx),
            Kind::Script { ref script, .. } => tick_script(script, ctx),
        };
        self.nodes[index].running = status == Status::Running;
        status
    }

    /// A Sequence (`proceed` = Success) or a Fallback (`proceed` = Failure): children in order from the

    /// running one; a child returning `proceed` moves on, the other result ends the node and resets it.
    fn tick_control(&mut self, index: usize, ctx: &mut Ctx<'_>, proceed: Status) -> Status {
        let mut cursor = match self.nodes[index].kind {
            Kind::Sequence { cursor } | Kind::Fallback { cursor } => cursor,
            _ => 0,
        };
        let count = self.nodes[index].children.len();
        let result = loop {
            if count <= cursor {
                break proceed;
            }
            let child = self.nodes[index].children[cursor];
            match self.tick_node(child, ctx) {
                Status::Running => break Status::Running,
                status if status == proceed => cursor += 1,
                other => break other,
            }
        };
        if result != Status::Running {
            cursor = 0;
            for child in self.nodes[index].children.clone() {
                self.halt_node(child, ctx);
            }
        }
        match &mut self.nodes[index].kind {
            Kind::Sequence { cursor: at } | Kind::Fallback { cursor: at } => *at = cursor,
            _ => {}
        }
        result
    }

    fn halt_node(&mut self, index: usize, ctx: &mut Ctx<'_>) {
        if !self.nodes[index].running {
            return;
        }
        for child in self.nodes[index].children.clone() {
            self.halt_node(child, ctx);
        }
        let node = &mut self.nodes[index];
        match &mut node.kind {
            Kind::Sequence { cursor } | Kind::Fallback { cursor } => *cursor = 0,
            Kind::Wait { left, .. } => *left = None,
            Kind::Stand | Kind::Burrow => {}
            Kind::Patrol(patrol) => patrol.halt(ctx),
            Kind::Script { .. } => {}
        }
        node.running = false;
    }
}

/// One script leaf's tick: fill the context, call the function, and turn its commands into the tick.
fn tick_script(script: &Script, ctx: &mut Ctx<'_>) -> Status {
    let prepared = script
        .context
        .set("agent_id", ctx.body.agent_id)
        .and_then(|_| script.context.set("map_id", ctx.map_id))
        .and_then(|_| script.context.set("delta_ms", ctx.elapsed_ms))
        .and_then(|_| script.context.set("in_combat", ctx.in_combat))
        .and_then(|_| script.context.set("phase", ctx.burrow_phase))
        .and_then(|_| script.commands.clear());
    if let Err(err) = prepared {
        ctx.failure = Some(format!("script context: {err}"));
        return Status::Failure;
    }
    let result: mlua::Result<Option<String>> = script.function.call(script.context.clone());
    let status = match result {
        Ok(Some(action)) if action == "success" => Status::Success,
        Ok(Some(action)) if action == "failure" => Status::Failure,
        Ok(_) => Status::Running,
        Err(err) => {
            ctx.failure = Some(format!("script: {err}"));
            return Status::Failure;
        }
    };
    for command in script
        .commands
        .sequence_values::<Table>()
        .filter_map(Result::ok)
    {
        let op: String = command.get("op").unwrap_or_default();
        match op.as_str() {
            "burrow" => {
                let hidden_ms: u32 = command.get("hidden_ms").unwrap_or(0);
                ctx.burrow_request = Some(BurrowRequest::Submerge { hidden_ms });
            }
            "emerge" => ctx.burrow_request = Some(BurrowRequest::Emerge),
            _ => {}
        }
    }
    status
}

/// Cancel a body's walk where it stands and tell its clients, if it is walking.
pub(crate) fn stop_walk(body: &mut AgentState, out: &mut Vec<Msg>) {
    if 0.0 < body.motion.speed {
        body.cancel_movement();
        body.motion.waypoints.clear();
        body.motion.waypoint_idx = 0;
        out.push(Msg::Stop {
            agent_id: body.agent_id,
        });
    }
}

/// A route walk: one leg at a time, a pause where a point states one, round again or done.
#[derive(Debug, Clone)]
struct Patrol {
    route: Route,
    /// The point the current leg walks to.
    next: usize,
    phase: Phase,
    /// The little generator that picks a random route point; seeded from the route id so a route's
    /// choice is reproducible and two bodies on different routes do not move in lock step.
    rng: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// No leg in progress: the next tick starts one.
    Idle,
    /// The body is walking to `next`.
    Walking,
    /// The body is at `next`, waiting.
    Pausing { left_ms: u32 },
}

impl Patrol {
    fn new(route: Route) -> Patrol {
        // FNV-1a of the route id, so a route's choices are reproducible and differ between routes.
        let mut rng = 2_166_136_261u32;
        for byte in route.id.bytes() {
            rng ^= u32::from(byte);
            rng = rng.wrapping_mul(16_777_619);
        }
        if rng == 0 {
            rng = 1;
        }
        let mut patrol = Patrol {
            route,
            next: 0,
            phase: Phase::Idle,
            rng,
        };
        if patrol.route.mode == RouteMode::Random {
            patrol.pick_other();
        }
        patrol
    }

    /// Move the cursor to a random point of the route that is not the current one (xorshift32).
    fn pick_other(&mut self) {
        let count = self.route.points.len();
        if count < 2 {
            return;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        let pick = self.rng as usize % count;
        self.next = if pick == self.next {
            (pick + 1) % count
        } else {
            pick
        };
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>) -> Status {
        loop {
            match self.phase {
                Phase::Idle => return self.start_leg(ctx),
                Phase::Walking if 0.0 < ctx.body.motion.speed => return Status::Running,
                Phase::Walking => {
                    let point = self.route.points[self.next];
                    if let Some(angle) = point.angle {
                        ctx.body.facing = (angle.cos(), angle.sin());
                    }
                    if 0 < point.pause_ms {
                        self.phase = Phase::Pausing {
                            left_ms: point.pause_ms,
                        };
                        return Status::Running;
                    }
                    if self.advance() {
                        return Status::Success;
                    }
                }
                Phase::Pausing { left_ms } => {
                    let left_ms = left_ms.saturating_sub(ctx.elapsed_ms);
                    if 0 < left_ms {
                        self.phase = Phase::Pausing { left_ms };
                        return Status::Running;
                    }
                    if self.advance() {
                        return Status::Success;
                    }
                }
            }
        }
    }

    /// Move the cursor past the point just reached. True when a `Once` route has ended.
    fn advance(&mut self) -> bool {
        self.phase = Phase::Idle;
        if self.next + 1 < self.route.points.len() {
            self.next += 1;
            return false;
        }
        match self.route.mode {
            RouteMode::Loop => {
                self.next = 0;
                false
            }
            RouteMode::Once => {
                self.next = 0;
                true
            }
            RouteMode::Random => {
                self.pick_other();
                false
            }
        }
    }

    fn start_leg(&mut self, ctx: &mut Ctx<'_>) -> Status {
        let mut attempts = 0;
        loop {
            let point = self.route.points[self.next];
            let to = Pos {
                x: point.position.0,
                y: point.position.1,
                plane: point.plane,
            };
            match ctx.pathing.leg(ctx.map_id, ctx.body.here(), to) {
                Walk::Found(waypoints) => {
                    if let Some(first) = ctx.body.begin_walk(waypoints) {
                        ctx.out.push(Msg::MoveTo {
                            agent_id: ctx.body.agent_id,
                            to: (first.pos.x, first.pos.y),
                            plane: first.pos.plane,
                            current_plane: ctx.body.plane,
                        });
                    }
                    self.phase = Phase::Walking;
                    return Status::Running;
                }
                Walk::NoPath
                    if self.route.mode == RouteMode::Random
                        && attempts + 1 < self.route.points.len() =>
                {
                    // A random spot the path graph cannot reach is retired, not a parked body:
                    // preforged's policy tries the next offset after a failed idle leg.
                    attempts += 1;
                    self.pick_other();
                }
                Walk::NoPath => {
                    ctx.failure = Some(format!(
                        "route {} point {}: no path from ({:.1}, {:.1})",
                        self.route.id, self.next, ctx.body.position.0, ctx.body.position.1
                    ));
                    return Status::Failure;
                }
                Walk::NoGraph => {
                    ctx.failure = Some(format!(
                        "route {}: this world has no path graph for map {}",
                        self.route.id, ctx.map_id
                    ));
                    return Status::Failure;
                }
            }
        }
    }

    fn halt(&mut self, ctx: &mut Ctx<'_>) {
        stop_walk(ctx.body, ctx.out);
        self.phase = Phase::Idle;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::content::RoutePoint;
    use super::*;
    use crate::pathfind::Waypoint;

    /// A flat world: every leg is one straight waypoint, except to a point with a negative x.
    pub(crate) struct Flat;
    impl Pathing for Flat {
        fn leg(&self, _map_id: i32, _from: Pos, to: Pos) -> Walk {
            if to.x < 0.0 {
                return Walk::NoPath;
            }
            Walk::Found(vec![Waypoint {
                pos: to,
                trap_id: 0,
            }])
        }
    }

    pub(crate) fn square(mode: RouteMode, pause_ms: u32) -> Route {
        let point = |x: f32, y: f32| RoutePoint {
            position: (x, y),
            plane: 0,
            pause_ms,
            angle: None,
        };
        Route {
            id: "square".to_string(),
            mode,
            points: vec![point(100.0, 0.0), point(100.0, 100.0)],
        }
    }

    fn routes(route: Route) -> BTreeMap<String, Route> {
        BTreeMap::from([(route.id.clone(), route)])
    }

    fn body() -> AgentState {
        AgentState::new(7, (0.0, 0.0), 0, (1.0, 0.0), 100.0)
    }

    /// Build through the same shape the table uses, with no script source.
    fn tree(
        spec: &Spec,
        own_route: Option<&str>,
        routes: &BTreeMap<String, Route>,
    ) -> Result<Tree, String> {
        Tree::build(spec, own_route, routes, &crate::npc::table::PackBehaviors)
    }

    /// Advance the body as the table does, then tick the tree.
    fn step(tree: &mut Tree, body: &mut AgentState, ms: u32, out: &mut Vec<Msg>) -> Status {
        body.advance(ms as f32 / 1000.0);
        let mut ctx = Ctx {
            body,
            map_id: 148,
            elapsed_ms: ms,
            pathing: &Flat,
            out,
            failure: None,
            in_combat: false,
            burrow_phase: None,
            burrow_request: None,
        };
        tree.tick(&mut ctx)
    }

    #[test]
    fn a_patrol_walks_one_leg_at_a_time_and_loops() {
        let mut tree = tree(
            &Spec::Patrol { route: None },
            Some("square"),
            &routes(square(RouteMode::Loop, 0)),
        )
        .expect("builds");
        let mut body = body();
        let mut out = Vec::new();
        assert_eq!(step(&mut tree, &mut body, 0, &mut out), Status::Running);
        assert_eq!(
            out,
            vec![Msg::MoveTo {
                agent_id: 7,
                to: (100.0, 0.0),
                plane: 0,
                current_plane: 0
            }]
        );
        out.clear();
        // 100 units at 100 units a second: one second to the first point, which starts the second leg.
        assert_eq!(step(&mut tree, &mut body, 1000, &mut out), Status::Running);
        assert_eq!(body.position, (100.0, 0.0));
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], Msg::MoveTo { to: (x, y), .. } if x == 100.0 && y == 100.0));
        out.clear();
        step(&mut tree, &mut body, 1000, &mut out);
        assert!(matches!(out[0], Msg::MoveTo { to: (x, y), .. } if x == 100.0 && y == 0.0));
    }

    #[test]
    fn a_once_route_succeeds_at_its_last_point_after_its_pause() {
        let mut tree = tree(
            &Spec::Patrol {
                route: Some("square".to_string()),
            },
            None,
            &routes(square(RouteMode::Once, 500)),
        )
        .expect("builds");
        let mut body = body();
        let mut out = Vec::new();
        step(&mut tree, &mut body, 0, &mut out);
        assert_eq!(step(&mut tree, &mut body, 1000, &mut out), Status::Running);
        assert_eq!(out.len(), 1, "arrived, now pausing: no new leg yet");
        assert_eq!(step(&mut tree, &mut body, 499, &mut out), Status::Running);
        assert_eq!(step(&mut tree, &mut body, 1, &mut out), Status::Running);
        assert_eq!(out.len(), 2, "the pause is over: the second leg starts");
        step(&mut tree, &mut body, 1000, &mut out);
        assert_eq!(step(&mut tree, &mut body, 500, &mut out), Status::Success);
        assert_eq!(body.position, (100.0, 100.0));
    }

    #[test]
    fn a_halt_stops_the_body_and_the_restart_resumes_the_same_leg() {
        let mut tree = tree(
            &Spec::Patrol { route: None },
            Some("square"),
            &routes(square(RouteMode::Loop, 0)),
        )
        .expect("builds");
        let mut body = body();
        let mut out = Vec::new();
        step(&mut tree, &mut body, 0, &mut out);
        step(&mut tree, &mut body, 500, &mut out);
        out.clear();
        let mut ctx = Ctx {
            body: &mut body,
            map_id: 148,
            elapsed_ms: 0,
            pathing: &Flat,
            out: &mut out,
            failure: None,
            in_combat: false,
            burrow_phase: None,
            burrow_request: None,
        };
        tree.halt(&mut ctx);
        assert_eq!(out, vec![Msg::Stop { agent_id: 7 }]);
        assert_eq!(body.motion.speed, 0.0);
        assert_eq!(body.position, (50.0, 0.0));
        out.clear();
        step(&mut tree, &mut body, 0, &mut out);
        assert!(
            matches!(out[0], Msg::MoveTo { to: (x, y), .. } if x == 100.0 && y == 0.0),
            "the abandoned leg, not the start of the route: {out:?}"
        );
    }

    #[test]
    fn a_leg_with_no_path_fails_with_its_reason() {
        let mut route = square(RouteMode::Loop, 0);
        route.points[0].position = (-5.0, 0.0);
        let mut tree = tree(
            &Spec::Patrol { route: None },
            Some("square"),
            &routes(route),
        )
        .expect("builds");
        let mut body = body();
        let mut out = Vec::new();
        let mut ctx = Ctx {
            body: &mut body,
            map_id: 148,
            elapsed_ms: 0,
            pathing: &Flat,
            out: &mut out,
            failure: None,
            in_combat: false,
            burrow_phase: None,
            burrow_request: None,
        };
        assert_eq!(tree.tick(&mut ctx), Status::Failure);
        assert!(ctx.failure.expect("a reason").contains("no path"));
    }

    #[test]
    fn a_sequence_resumes_at_its_running_child() {
        let spec = Spec::Sequence(vec![Spec::Wait { ms: 100 }, Spec::Wait { ms: 100 }]);
        let mut tree = tree(&spec, None, &BTreeMap::new()).expect("builds");
        let mut body = body();
        let mut out = Vec::new();
        let mut tick = |ms| {
            let mut ctx = Ctx {
                body: &mut body,
                map_id: 148,
                elapsed_ms: ms,
                pathing: &Flat,
                out: &mut out,
                failure: None,
            in_combat: false,
            burrow_phase: None,
            burrow_request: None,
            };
            tree.tick(&mut ctx)
        };
        assert_eq!(tick(0), Status::Running, "first wait starts");
        assert_eq!(tick(100), Status::Running, "first wait done, second starts");
        assert_eq!(tick(60), Status::Running, "second wait still has 40");
        assert_eq!(tick(40), Status::Success, "and the sequence succeeds");
        assert_eq!(tick(0), Status::Running, "a finished sequence starts over");
    }

    #[test]
    fn a_fallback_takes_the_first_child_that_does_not_fail() {
        let mut route = square(RouteMode::Loop, 0);
        route.points[0].position = (-5.0, 0.0);
        let spec = Spec::Fallback(vec![Spec::Patrol { route: None }, Spec::Stand]);
        let mut tree = tree(&spec, Some("square"), &routes(route)).expect("builds");
        let mut body = body();
        let mut out = Vec::new();
        let mut ctx = Ctx {
            body: &mut body,
            map_id: 148,
            elapsed_ms: 0,
            pathing: &Flat,
            out: &mut out,
            failure: None,
            in_combat: false,
            burrow_phase: None,
            burrow_request: None,
        };
        assert_eq!(
            tree.tick(&mut ctx),
            Status::Running,
            "the patrol failed; it stands"
        );
        assert!(out.is_empty());
    }

    #[test]
    fn construction_errors_are_named() {
        let empty = tree(&Spec::Sequence(Vec::new()), None, &BTreeMap::new());
        assert!(empty.expect_err("refused").contains("no children"));
        let unrouted = tree(&Spec::Patrol { route: None }, None, &BTreeMap::new());
        assert!(unrouted.expect_err("refused").contains("names no route"));
        let missing = tree(
            &Spec::Patrol {
                route: Some("x".to_string()),
            },
            None,
            &BTreeMap::new(),
        );
        assert!(missing.expect_err("refused").contains("route x"));
    }
}
