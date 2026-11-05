//! One agent's position, and the rules the reference uses to believe a client about it.
//!
//! A client moves itself: it walks where it wants and reports its position about twice a second
//! (`GAME_CMSG_POSITION_UPDATE`), and it can ask to stop (`CANCEL_MOVEMENT`) or to correct where it was
//! when a move was cancelled. The server's job is to keep its own idea of where the agent is, and to
//! refuse the parts of that which cannot be true.
//!
//! The rules are the reference's (`GmAgent.c`, `GameSrv_HandlePositionUpdate` and friends), and they are
//! copied rather than invented because they are what a real client's movement is shaped for:
//!
//! * with 50 units of slack, a client may be as far ahead as its own speed allows since the last report,
//!   and no further. Between two reports 2 seconds apart at 288 units a second that is a generous
//!   budget; a report after a five-minute stall is not a teleport, it is a client that resumed;
//! * an out-of-budget report is not dropped. The agent is moved as far along the reported direction as
//!   the budget allows - dropping it leaves the two views further apart than accepting part of it;
//! * a plane change is allowed only when it is short (stairs, a small portal);
//! * a correction after a cancelled move is accepted only within 100 units and only on the same plane.
//!
//! ## Walking
//!
//! The other half is the server's own idea of a walk, and it is the reference's too: a `MOVE_TO_COORD`
//! becomes a path (`pathfind.rs`, from `GmPaths.c`), the agent walks the first segment at its own
//! `speed_base` (288 units a second, `GameSrv.c:1088`), and every world tick moves it `speed_base * delta`
//! further along. When a segment ends the next waypoint becomes the destination and the world is told;
//! when the last one ends the walk is over and the agent stands still. That state is [`Motion`], which is
//! the reference's `GmAgent.destination`, `.direction`, `.speed`, `.waypoints` and `.waypoint_idx`.

use std::time::{Duration, Instant};

use crate::pathfind::{Pos, Waypoint};

/// The agent API: the questions a server answers about one agent, one question one name.
///
/// A member takes the agent (or the instance that owns the agents) instead of an `agent_id` looked
/// up in a process-wide client: the state here is [`AgentState`], owned by the [`Instance`].
#[allow(non_snake_case)]
pub struct Agent;

#[allow(non_snake_case)]
impl Agent {
    /// `Agent.IsValid()`: whether the instance's agents hold that id.
    pub fn IsValid(instance: &Instance, agent_id: u32) -> bool {
        instance.has_agent(agent_id)
    }

    /// `Agent.GetXY()`: the agent's position, as the server last believed it.
    pub fn GetXY(agent: &AgentState) -> (f32, f32) {
        agent.position
    }

    /// `Agent.GetZPlane()`: the plane the agent stands on.
    pub fn GetZPlane(agent: &AgentState) -> u16 {
        agent.plane
    }
}

/// The reference's `POSITION_UPDATE_TOLERANCE`: how far a client may be from where the server thought,
/// before distance is even considered.
pub const POSITION_UPDATE_TOLERANCE: f32 = 50.0;

/// The reference's `POSITION_UPDATE_MAX_ELAPSED_MS`: the longest gap between reports that counts as
/// elapsed time. A longer gap is treated as this, so a stalled client does not get an unbounded budget.
pub const POSITION_UPDATE_MAX_ELAPSED_MS: u32 = 2_000;

/// The correction ceiling in force: `config/Server/movement.json`'s `max_allowed_correction`.
///
/// The rule is the reference's `MAXIMUM_ALLOWED_CORRECTION` and the reference's own number is 100
/// (`GmAgent.c:547-563`), which is also the fallback `asura-config` carries for a checkout with no
/// `config/`. A compiled copy does not live here: this is a number a machine chooses, and
/// `docs/38-layer-decisions.md` (D33) keeps those in a file so they can be read and reviewed.
pub fn max_allowed_correction() -> f32 {
    asura_config::movement().max_allowed_correction()
}

/// What the server did with a reported position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adopted {
    /// The report was inside the budget and is now the agent's position.
    Accepted,
    /// The report was too far ahead; the agent moved as far as the budget allows, along the reported
    /// direction.
    Clamped,
    /// The report was refused (a plane change that is too far).
    Refused,
}

/// Where one agent is, and when it last said so.
#[derive(Debug, Clone)]
pub struct AgentState {
    pub agent_id: u32,
    pub position: (f32, f32),
    pub plane: u16,
    /// Which way the agent faces while it is not walking, as the unit vector the world announced when
    /// it created the agent. The spawn point's angle lands here.
    pub facing: (f32, f32),
    /// The reference's `last_position_update`, in milliseconds since the connection started.
    pub last_report: Option<Instant>,
    /// The agent's walking speed, which is what the budget is computed from.
    pub speed_base: f32,
    /// The walk in progress, if any (`GmAgent.destination`, `.direction`, `.speed`, `.waypoints`,
    /// `.waypoint_idx`).
    pub motion: Motion,
    /// Last announced movement type: 1 forward, 4 backward, 7/8 strafe, 9 stopped.
    pub move_type: u8,
    /// Movement speed factor sent with the type; separate from the base speed.
    pub move_speed: f32,
    /// Last announced manual direction, for suppressing unchanged direction replies.
    pub move_direction: (f32, f32),
}

/// The reference's `GmAgent` walking state: one segment being walked, and the waypoints after it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Motion {
    /// `GmAgent.waypoints`: the path the server chose. The segment being walked has already been taken
    /// off the front of it, exactly as `waypoint_idx` does in the reference.
    pub waypoints: Vec<Waypoint>,
    /// `GmAgent.waypoint_idx`: the next waypoint to take when the current segment ends.
    pub waypoint_idx: usize,
    /// `GmAgent.destination`: where the current segment ends, plane included.
    pub destination: Pos,
    /// `GmAgent.direction`: a unit vector from the agent toward `destination`.
    pub direction: (f32, f32),
    /// `GmAgent.speed`: zero when the agent is standing still.
    pub speed: f32,
}

impl AgentState {
    /// A new agent, standing where the connection placed it, facing the spawn point's direction.
    pub fn new(
        agent_id: u32,
        position: (f32, f32),
        plane: u16,
        facing: (f32, f32),
        speed_base: f32,
    ) -> AgentState {
        AgentState {
            agent_id,
            position,
            plane,
            facing,
            last_report: None,
            speed_base,
            motion: Motion::default(),
            move_type: 1,
            move_speed: 1.0,
            move_direction: (0.0, 0.0),
        }
    }

    /// The agent's position as a point on the map's graph.
    pub fn here(&self) -> Pos {
        Pos {
            x: self.position.0,
            y: self.position.1,
            plane: self.plane,
        }
    }

    /// Begin walking a path the finder produced (`GameSrv_HandleMoveToCoord`'s success half).
    ///
    /// The reference takes the first waypoint as the destination and leaves the rest in the list; this is
    /// that, and it returns the waypoint so the caller can broadcast it. `None` when the path is empty,
    /// which the reference also ignores (`if (0 < agent->waypoints.len)`).
    pub fn begin_walk(&mut self, waypoints: Vec<Waypoint>) -> Option<Waypoint> {
        self.motion.waypoints = waypoints;
        self.motion.waypoint_idx = 0;
        let first = *self.motion.waypoints.first()?;
        self.motion.waypoint_idx = 1;
        self.set_destination(first.pos);
        Some(first)
    }

    /// `GameSrv_UpdateAgentDestination`, without the broadcast: point at `destination` and start moving.
    ///
    /// The reference divides by the distance without checking it, so a destination equal to the current
    /// position produces a NaN direction and then a NaN position -- which would be saved to the database
    /// and reloaded on the next visit. This keeps the direction it had instead, which makes such a walk a
    /// no-op that the arrival test below clears on the next tick. That is a deliberate difference, and
    /// the only one in this file.
    pub fn set_destination(&mut self, destination: Pos) {
        self.motion.speed = self.speed_base * self.move_speed;
        self.motion.destination = destination;
        let dx = destination.x - self.position.0;
        let dy = destination.y - self.position.1;
        let norm = ((dx * dx) + (dy * dy)).sqrt();
        if norm > 0.0 {
            self.motion.direction = (dx / norm, dy / norm);
        }
    }

    /// `GameSrv_CancelAgentMovement`: stand where you are -- the destination collapses to the position and
    /// the speed to zero. The waypoint list is the caller's business, as it is in the reference.
    pub fn cancel_movement(&mut self) {
        self.motion.destination = self.here();
        self.motion.speed = 0.0;
    }

    /// Take a position the client reported as the agent's own, exactly.
    ///
    /// This is what the live servers do with `MOVE_TO_COORD_WITH_DIR` (`0x003D`): the message carries
    /// where the client is and the offset it is walking in, and the server answers from that position --
    /// no elapsed-time budget, no clamping (`captures/live-move.jsonl`: every answer's destination is the
    /// reported position plus the reported offset, to within the server's own float error). Adopting it
    /// exactly is what keeps the server's copy and the client's walk from drifting apart.
    pub fn adopt_exact(&mut self, position: (f32, f32), plane: u16) {
        self.position = position;
        self.plane = plane;
    }

    /// Start walking straight at `destination`, replacing any walk in progress and any waypoint list.
    ///
    /// The caller has already decided the point is reachable on this plane; the plane travels with the
    /// destination so an arrival on another plane is the same move the reference makes.
    pub fn walk_to(&mut self, destination: Pos) {
        self.motion.waypoints.clear();
        self.motion.waypoint_idx = 0;
        self.set_destination(destination);
    }

    /// Replace the previous order and announce a manual walk to every copy of this agent.
    /// Reply order: alternating-live.jsonl 327-329 (direction, type, destination).
    pub fn begin_manual_walk(
        &mut self,
        waypoints: Vec<Waypoint>,
        direction: (f32, f32),
        move_type: u8,
    ) -> Vec<Frame> {
        self.cancel_movement();
        self.motion.waypoints.clear();
        self.motion.waypoint_idx = 0;
        if waypoints.is_empty() {
            return Vec::new();
        }
        let speed = match move_type {
            4..=6 => 0.66,
            7..=8 => 0.75,
            _ => 1.0,
        };
        let mut reply = Vec::new();
        let dx = direction.0 - self.move_direction.0;
        let dy = direction.1 - self.move_direction.1;
        if dx * dx + dy * dy >= 0.0003 || self.move_type != move_type {
            reply.push(game::agent_update_direction(
                self.agent_id,
                direction,
                move_type,
            ));
        }
        if self.move_type != move_type || self.move_speed != speed {
            reply.push(game::agent_update_speed(self.agent_id, speed, move_type));
        }
        self.move_type = move_type;
        self.move_speed = speed;
        self.move_direction = direction;
        if let Some(first) = self.begin_walk(waypoints) {
            reply.push(game::agent_move_to_point(
                self.agent_id,
                (first.pos.x, first.pos.y),
                first.pos.plane,
                self.plane,
            ));
        }
        reply
    }

    /// alternating-live.jsonl 321-322: type 9, then a destination at the stopping point.
    pub fn stop_manual_walk(&mut self) -> Vec<Frame> {
        self.stand_still();
        vec![
            game::agent_update_speed(self.agent_id, 1.0, 9),
            game::agent_move_to_point(self.agent_id, self.position, self.plane, self.plane),
        ]
    }

    /// The state half of [`stop_manual_walk`]: the walk is over, at rest, with no waypoints left.
    ///
    /// Used when the stopping point is refused. A refused correction still means the agent is not
    /// walking, so the server's copy must stop -- otherwise it keeps advancing along the abandoned path
    /// and tells every client to keep walking it. Nothing goes on the wire.
    pub fn stand_still(&mut self) {
        self.cancel_movement();
        self.motion.waypoints.clear();
        self.motion.waypoint_idx = 0;
        self.move_type = 9;
        self.move_speed = 1.0;
    }

    /// `GameSrv_UpdateAgentMovement`: walk `delta` seconds of the current segment.
    ///
    /// Returns the waypoint that became the new destination, if the agent crossed one during the tick --
    /// the caller broadcasts it, because every client is drawing this agent from the last destination it
    /// was told. Arrival clears the walk, as the reference does.
    pub fn advance(&mut self, delta: f32) -> Option<Waypoint> {
        if self.motion.speed == 0.0 {
            return None;
        }

        let mut distance = delta * self.motion.speed;
        let destination = self.motion.destination;
        let to_destination = distance_between(self.position, (destination.x, destination.y));

        let mut crossed = None;
        if to_destination <= distance {
            distance -= to_destination;

            self.position = (destination.x, destination.y);
            self.plane = destination.plane;

            if self.motion.waypoint_idx < self.motion.waypoints.len() {
                let next = self.motion.waypoints[self.motion.waypoint_idx];
                self.motion.waypoint_idx += 1;
                self.set_destination(next.pos);
                crossed = Some(next);
            } else {
                distance = 0.0;
            }
        }

        if 0.0 < distance {
            let (dx, dy) = self.motion.direction;
            self.position.0 += dx * distance;
            self.position.1 += dy * distance;
        }

        if self.plane == destination.plane
            && self.position.0 == destination.x
            && self.position.1 == destination.y
        {
            self.motion.destination = Pos::default();
            self.motion.speed = 0.0;
            self.motion.waypoint_idx = 0;
            self.motion.waypoints.clear();
        }

        crossed
    }

    /// Adopt a position the client reported (`GAME_CMSG_POSITION_UPDATE`).
    pub fn adopt_report(&mut self, position: (f32, f32), plane: u16, now: Instant) -> Adopted {
        let elapsed_ms = match self.last_report {
            // The first report of a session gets the full budget rather than none: the client has been
            // standing still while it loaded, and the reference makes the same choice.
            None => POSITION_UPDATE_MAX_ELAPSED_MS,
            Some(last) => {
                let elapsed = now.saturating_duration_since(last).as_millis() as u32;
                if elapsed == 0 || elapsed > POSITION_UPDATE_MAX_ELAPSED_MS {
                    POSITION_UPDATE_MAX_ELAPSED_MS
                } else {
                    elapsed
                }
            }
        };
        self.last_report = Some(now);

        let allowed = self.speed_base * (elapsed_ms as f32 / 1000.0) + POSITION_UPDATE_TOLERANCE;
        let distance = self.distance_to(position);

        if plane != self.plane {
            // A plane change is legitimate when it is close by and suspicious when it is not.
            return if allowed < distance {
                Adopted::Refused
            } else {
                self.plane = plane;
                self.position = position;
                Adopted::Accepted
            };
        }

        if distance <= allowed {
            self.position = position;
            return Adopted::Accepted;
        }

        // Out of budget: move as far as the budget allows, along the reported direction.
        let step = if distance > 0.0 {
            allowed / distance
        } else {
            0.0
        };
        self.position = (
            self.position.0 + (position.0 - self.position.0) * step,
            self.position.1 + (position.1 - self.position.1) * step,
        );
        Adopted::Clamped
    }

    /// Adopt a correction the client sends after cancelling a walk
    /// (`GAME_CMSG_LAST_POS_BEFORE_MOVE_CANCELED`). Only a short correction on the same plane counts;
    /// how short is `config/Server/movement.json`'s `max_allowed_correction`.
    pub fn adopt_correction(&mut self, position: (f32, f32), plane: u16) -> bool {
        if plane != self.plane || max_allowed_correction() < self.distance_to(position) {
            return false;
        }
        self.position = position;
        true
    }

    /// How long ago the client last reported, for the trace.
    pub fn since_last_report(&self, now: Instant) -> Option<Duration> {
        self.last_report
            .map(|last| now.saturating_duration_since(last))
    }

    pub fn distance_to(&self, other: (f32, f32)) -> f32 {
        distance_between(self.position, other)
    }
}

/// `Vec2f_Dist2`: the real square root, which is what the reference's movement uses (its *path* finder
/// uses the table-driven one, and that is `pathfind::path_sqrt`).
pub fn distance_between(left: (f32, f32), right: (f32, f32)) -> f32 {
    let dx = right.0 - left.0;
    let dy = right.1 - left.1;
    ((dx * dx) + (dy * dy)).sqrt()
}

use crate::*;

/// `AgentProperty_Value64` (`GmAgentProperties.h`): the third property the creation path sends, always 0.
pub const AGENT_PROPERTY_VALUE64: u32 = 64;

/// Where a character stands as it enters a map.
///
/// Three rules, and which one applies is the whole of the arrival rule ([`Standing::arrival`]):
///
/// * a **doorway arrival** stands on the point the receiving map states for the map it came from
///   (`portals.json`'s `arrivals`), facing that arrival's angle - a portal walk is placed by the door,
///   not by a random spawn point;
/// * an **outpost** (a main town) is entered at a **random point of its own spawn list**, the pack's
///   `spawns.json`, scattered around it and facing the direction that point states: every other way
///   into an outpost - the world map, `/tp`, a login - stands on a random spawn point from that list,
///   which is the owner's rule (2026-10-31, restated 2026-11-05);
/// * every other map keeps the position the character was saved at when it last left **if it was
///   saved in this map**, and otherwise the map's own single spawn point. That is the disconnect
///   case: a character that comes back to the map it left stands where it left.
///
/// A character that has never been played has no saved position, and one that was saved in a
/// different map is arriving here for the first time (`asura_db::game::position`). The facing of a
/// saved position is not stored; a doorway arrival takes its facing from the map's arrival entry, and
/// a resumed character keeps the default direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Standing {
    pub position: (f32, f32),
    pub plane: u16,
    /// Which way the character faces on arrival, as the unit vector
    /// `GAME_SMSG_WORLD_CREATE_AGENT` carries.
    pub direction: (f32, f32),
}

impl Standing {
    /// The map's own single spawn point: where the catalogue row says a character arrives.
    pub fn from_map(map: &MapEntry) -> Standing {
        Standing {
            position: map.spawn,
            plane: map.spawn_plane,
            direction: (1.0, 0.0),
        }
    }

    /// One of the map's own spawn points, **scattered around it**: a random point inside a
    /// [`spawn_scatter`]-unit circle, on the point's plane, facing the point's own angle.
    ///
    /// A spawn point is not a pin - the owner's rule, 2026-11-05: "when we spawn we should get our
    /// position randomized 250 units around a circle from the spawn point" (`docs/36-game-rules-ledger.md`
    /// R13's soft scatter). The radius is `config/Server/spawns.json`'s `spawn_scatter`, not a compiled
    /// constant, so an operator changes it without a rebuild. The scatter keeps the point's plane: the
    /// graph that would say which plane a scattered coordinate lands on is not loaded on every machine
    /// (no `Gw.dat`, no graph), so the point's own plane is the one answer that is always right.
    pub fn at_spawn(spawn: &crate::map_spawns::MapSpawn) -> Result<Standing, GameError> {
        let (dx, dy) = scatter(spawn_scatter())?;
        Ok(Standing {
            position: (spawn.x + dx, spawn.y + dy),
            plane: spawn.plane,
            direction: spawn.direction(),
        })
    }

    /// Where a character stands as a connection enters `map`.
    ///
    /// Three rules, in this order:
    ///
    /// 1. an **arrival** the receiving map states for the doorway that brought the character here
    ///    (`portals.json`'s `arrivals`, keyed by the map it came from): the crossing wrote that point as
    ///    the saved position before the redirect, so a saved position that is one of the map's arrivals
    ///    is a doorway arrival, and it stands **there**, facing the arrival's angle;
    /// 2. an **outpost** with a spawn list is entered at a random point of that list, scattered around it
    ///    ([`Standing::at_spawn`]) - a world-map request or a `/tp` picks one at random;
    /// 3. every other map keeps the saved position when it belongs to this map (a disconnect coming
    ///    back), and the catalogue row's single coordinate otherwise.
    pub fn arrival(map: &MapEntry, saved: Option<db_game::Position>) -> Result<Standing, GameError> {
        if let Some(saved) = saved.filter(|saved| saved.map_id == map.map_id) {
            if let Some(arrival) = crate::portals::arrival_at(map.map_id, (saved.x, saved.y)) {
                return Ok(Standing {
                    position: (arrival.x, arrival.y),
                    plane: saved.plane.max(0) as u16,
                    direction: arrival.direction(),
                });
            }
        }
        if map.map_type == map_kinds().main_town && !map.spawns.is_empty() {
            let index = (crate::random_u32()? as usize) % map.spawns.len();
            return Standing::at_spawn(&map.spawns[index]);
        }
        Ok(Standing::resume(map, saved))
    }

    /// This character's standing, from what the database remembers about it.
    ///
    /// The saved position is used only when it is a position *in this map*: a character saved in another
    /// map has no coordinate here, and using one would put it somewhere the map's geometry does not
    /// contain. `None` is a character that has never been saved, which is every character's first visit.
    pub fn resume(map: &MapEntry, saved: Option<db_game::Position>) -> Standing {
        match saved {
            Some(saved) if saved.map_id == map.map_id => Standing {
                position: (saved.x, saved.y),
                plane: saved.plane.max(0) as u16,
                direction: (1.0, 0.0),
            },
            _ => Standing::from_map(map),
        }
    }
}

/// How far from its spawn point a character lands: the radius of the circle the position is scattered
/// over. It is a value this machine chooses, so it lives in `config/Server/spawns.json`
/// (`asura_config::spawns`, D34, D35) and this is the read; the built-in 250-unit fallback is what a
/// checkout with no `config/` runs on (`crates/asura-world/src/agent.rs`, [`Standing::at_spawn`]).
pub fn spawn_scatter() -> f32 {
    asura_config::spawns().spawn_scatter()
}

/// A random offset inside a `radius`-unit circle: an even angle and a radius whose square is uniform,
/// which spreads the points over the area instead of bunching them at the centre.
fn scatter(radius: f32) -> Result<(f32, f32), GameError> {
    let angle = (crate::random_u32()? as f64 / u32::MAX as f64) * std::f64::consts::TAU;
    let scale = (crate::random_u32()? as f64 / u32::MAX as f64).sqrt();
    let distance = f64::from(radius) * scale;
    Ok((
        (angle.cos() * distance) as f32,
        (angle.sin() * distance) as f32,
    ))
}

/// What the finder said about a walk.
#[derive(Debug, Clone)]
pub enum Walk {
    /// A path, in the reference's reduced form. It may hold a single waypoint naming the destination.
    Found(Vec<pathfind::Waypoint>),
    /// The finder ran and refused: the destination is not reachable from where the agent stands.
    NoPath,
    /// This world has no graph for the map, so nothing can be said about reachability. That happens when
    /// the map's blob could not be imported from `Gw.dat` -- a server whose archive is missing or older
    /// than the catalogue refuses walks rather than inventing them.
    NoGraph,
}

/// Counters the movement trace keeps for its session summary: what the client asked for, what the
/// server decided about each report, and every point where the agent's plane changed - which is what
/// a client shows as a terrain jump.
#[derive(Debug, Default, Clone, Copy)]
pub struct MoveStats {
    pub walk_requests: u32,
    pub reports: u32,
    pub accepted: u32,
    pub clamped: u32,
    pub refused: u32,
    pub plane_changes: u32,
    pub ticks: u32,
    /// The largest gap seen between a reported position and the server's own, before adoption.
    pub max_drift: f32,
}

/// Whether the movement trace is on (`ASURA_MOVE_TRACE=1`; set it in the shell before `start`).
///
/// Movement is the one part of the wire a capture cannot diagnose after the fact: drift and terrain
/// jumps are a sequence of requests, answers and ticks, and the question is always *what did the
/// server think the agent's position and plane were* at each step. This prints that sequence - every
/// move request with its parsed destination, the path the finder produced (each waypoint's position,
/// plane and trapezoid), the echoed waypoint, every tick that crosses one, and every position report
/// with the budget decision. It prints no packet log, so a session stays readable.
pub fn move_trace_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        matches!(
            std::env::var("ASURA_MOVE_TRACE").ok().as_deref(),
            Some("1" | "true" | "yes" | "on")
        )
    })
}

/// How far apart the live servers hand out destinations: **145 units**, measured from
/// `recordings/client/feature/movement/movement-sniff.log`.
///
/// The capture's 104 destination announcements for the player agent are spaced a median 0.51 s apart and
/// 145-158 units apart, i.e. a copy walking at the character's own 288 u/s and re-announcing the next
/// point about every half second. That cadence, rather than the corner of a trapezoid, is what keeps the
/// client and the server's copy together: announcing only at corners (1,000 units apart in the session
/// that drifted) gives a client that rounds a wall or is slowed seconds to drift, and the next correction
/// is the ~300-unit snap. The reference's own path is untouched; the points added between its corners lie
/// on the same straight segment.
pub const LIVE_CADENCE_STEP: f32 = 145.0;

/// The announcement step in units: [`LIVE_CADENCE_STEP`], or `ASURA_ANNOUNCE_STEP` when set, for a
/// session that wants to compare cadences without a rebuild.
pub fn live_cadence_step() -> f32 {
    std::env::var("ASURA_ANNOUNCE_STEP")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|step| *step >= 1.0)
        .unwrap_or(LIVE_CADENCE_STEP)
}

/// Sample a walk so no two points are farther apart than `step`, keeping every point the path finder
/// produced and adding the intermediate ones on the same straight segment.
///
/// The added points carry the plane the segment **starts** on, and only the path finder's own point
/// carries its plane. That is the reference's own stamping: a step before a portal is walked on the plane
/// being traversed and only the crossing is stamped with the plane it enters (`GmPaths.c:897-900`), so a
/// plane change is announced once, at the corner, exactly as it is without sampling. Sampling it with the
/// corner's plane instead told the client to change plane every 145 units, which is worse than not
/// sampling at all.
pub fn densify(
    waypoints: Vec<pathfind::Waypoint>,
    from: pathfind::Pos,
    step: f32,
) -> Vec<pathfind::Waypoint> {
    if waypoints.is_empty() || step <= 0.0 {
        return waypoints;
    }
    let mut dense = Vec::with_capacity(waypoints.len() * 4);
    let mut at = from;
    for waypoint in waypoints {
        let dx = waypoint.pos.x - at.x;
        let dy = waypoint.pos.y - at.y;
        let distance = ((dx * dx) + (dy * dy)).sqrt();
        let pieces = (distance / step).ceil();
        let pieces = if pieces.is_finite() && pieces > 1.0 {
            pieces as usize
        } else {
            1
        };
        for piece in 1..pieces {
            let fraction = piece as f32 / pieces as f32;
            dense.push(pathfind::Waypoint {
                pos: pathfind::Pos {
                    x: at.x + dx * fraction,
                    y: at.y + dy * fraction,
                    plane: at.plane,
                },
                trap_id: waypoint.trap_id,
            });
        }
        dense.push(waypoint);
        at = waypoint.pos;
    }
    dense
}

/// A path as the trace prints it: the first few waypoints with their planes and trapezoids.
pub fn waypoints_preview(waypoints: &[pathfind::Waypoint]) -> String {
    let shown: Vec<String> = waypoints
        .iter()
        .take(10)
        .map(|waypoint| {
            format!(
                "(x={:.1}, y={:.1}, plane={}, trap={})",
                waypoint.pos.x, waypoint.pos.y, waypoint.pos.plane, waypoint.trap_id
            )
        })
        .collect();
    let more = if waypoints.len() > 10 {
        format!(" ... {} more", waypoints.len() - 10)
    } else {
        String::new()
    };
    format!("[{}]{}", shown.join(" "), more)
}

/// The client asks to walk somewhere (`GameSrv_HandleMoveToCoord`, `GmAgent.c:371-398`).
///
/// The reference asks its path finder for the walk and then answers with **the first waypoint of
/// its own path**, not with the client's destination: the client predicts a straight line, and a
/// straight line through a wall is exactly what this message corrects. When the finder refuses, the
/// reference sends no refusal of its own -- it cancels the walk and broadcasts where the agent
/// really is, which is what makes the client snap back.
pub async fn handle_move_to_coord(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    if frame.body().len() < 12 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 16 }));
    }
    let destination = (
        f32::from_le_bytes([
            frame.body()[0],
            frame.body()[1],
            frame.body()[2],
            frame.body()[3],
        ]),
        f32::from_le_bytes([
            frame.body()[4],
            frame.body()[5],
            frame.body()[6],
            frame.body()[7],
        ]),
    );
    let plane = u32::from_le_bytes([
        frame.body()[8],
        frame.body()[9],
        frame.body()[10],
        frame.body()[11],
    ]) as u16;
    let agent_id = player.GetAgentID();
    let instance = Arc::clone(&player.instance);
    let map_id = instance.map.map_id;
    let Some((from, walk_active)) = instance.with_slot(player.player_id, |slot| {
        (slot.agent.here(), 0.0 < slot.agent.motion.speed)
    }) else {
        eprintln!("world: agent {agent_id} asked to walk but has no slot in its instance");
        return Ok(true);
    };
    let to = pathfind::Pos {
        x: destination.0,
        y: destination.1,
        plane,
    };
    if move_trace_on() {
        instance.with_slot(player.player_id, |slot| slot.move_stats.walk_requests += 1);
        eprintln!(
            "move[pathfind] agent {agent_id} asked to walk to (x={:.1}, y={:.1}, plane={plane}) \
             while the server thinks it stands at (x={:.1}, y={:.1}, plane={}){} \
             [0x803E, {} bytes]",
            destination.0,
            destination.1,
            from.x,
            from.y,
            from.plane,
            if walk_active {
                " - a server-side walk was still running (client has walked since its last report)"
            } else {
                ""
            },
            frame.body().len() + 2
        );
    }

    let walked = world.graphs.walk(map_id, from, to);
    if move_trace_on() {
        match &walked {
            Walk::Found(waypoints) => eprintln!(
                "move[pathfind] finder produced {} waypoint(s): {}",
                waypoints.len(),
                waypoints_preview(waypoints)
            ),
            Walk::NoPath => eprintln!("move[pathfind] finder refused: no path"),
            Walk::NoGraph => eprintln!(
                "move[pathfind] finder refused: this world has no path graph for map {map_id}"
            ),
        }
    }
    if trace {
        let said = match &walked {
            Walk::Found(waypoints) => format!("{} waypoint(s)", waypoints.len()),
            Walk::NoPath => "no path".to_string(),
            Walk::NoGraph => "no path graph for this map".to_string(),
        };
        eprintln!(
            "world: agent {agent_id} walks from ({:.1}, {:.1}) plane {} to ({:.1}, {:.1}) \
             plane {plane}: {said}",
            from.x, from.y, from.plane, destination.0, destination.1
        );
    }

    match walked {
        // An empty path sends nothing: the reference checks `0 < agent->waypoints.len`.
        Walk::Found(waypoints) => {
            // The path as the live servers hand it out: a destination about every 145 units,
            // which at the character's 288 u/s is the ~0.5 s cadence the live capture shows.
            let waypoints = densify(waypoints, from, live_cadence_step());
            // The reference's own point, as it is: no re-writing of its plane, no dropping of its
            // points. The plane id is whatever the map's pmaps call the surface the path is on
            // (floor is 0; slopes and bridges have their own ids), and that is the reference's
            // business, not this server's.
            let begun = instance
                .with_slot(player.player_id, |slot| {
                    let speed = (slot.agent.move_type != 1 || slot.agent.move_speed != 1.0)
                        .then(|| game::agent_update_speed(agent_id, 1.0, 1));
                    slot.agent.move_type = 1;
                    slot.agent.move_speed = 1.0;
                    let first = slot.agent.begin_walk(waypoints)?;
                    Some((
                        first,
                        slot.agent.plane,
                        slot.agent.motion.waypoints.len(),
                        slot.agent.position,
                        speed,
                    ))
                })
                .flatten();
            if let Some((first, current_plane, left, stood, speed)) = begun {
                let walk = game::agent_move_to_point(
                    agent_id,
                    (first.pos.x, first.pos.y),
                    first.pos.plane,
                    current_plane,
                );
                if move_trace_on() {
                    eprintln!(
                        "move[pathfind] answered with waypoint 0 (x={:.1}, y={:.1}, \
                             plane={}) current_plane={current_plane} [0x0029, {} bytes]; {left} \
                             waypoint(s) left to walk, agent stays at (x={:.1}, y={:.1}, plane={})",
                        first.pos.x,
                        first.pos.y,
                        first.pos.plane,
                        walk.len(),
                        stood.0,
                        stood.1,
                        current_plane
                    );
                }
                // Everyone else is told where this agent is walking: the reference broadcasts the
                // same message (`GameSrv_BroadcastAgentMoveToPoint`), and a client that is not told
                // keeps drawing the walker where it last stood.
                let mut reply: Vec<_> = speed.into_iter().collect();
                reply.push(walk);
                instance.broadcast(player.player_id, &reply);
                send(stream, session, reply, trace).await?;
            }
        }
        Walk::NoPath | Walk::NoGraph => {
            if trace {
                eprintln!("world: path finding failed for agent {agent_id}; standing still");
            }
            instance.with_slot(player.player_id, |slot| slot.agent.cancel_movement());
            let standing = player.standing();
            // **Nothing goes on the wire.** The reference answers a refused walk with
            // `AGENT_UPDATE_POSITION` (`0x002C`, `GmAgent.c:391-394`), and that message carries
            // the server's copy position: measured, it disconnected the client with 007 once
            // (capture-8001-20261027-221113 seq 519) and teleported it into a wall once
            // (capture of 22:16: `snapped to (9438.1, 7118.5)` after the clamp had dragged the
            // copy off the navmesh). Silence is what the reference sends in its other correction
            // path (`GameSrv_HandleLastPosOnMoveCanceled`), and the client's own reports settle
            // the truth.
            if move_trace_on() {
                eprintln!(
                    "move[pathfind] walk refused; the agent stands at (x={:.1}, y={:.1}, \
                     plane={}) and nothing is sent",
                    standing.position.0, standing.position.1, standing.plane
                );
            }
        }
    }
    Ok(true)
}

/// A manual movement order: position, plane, offset and movement type.
/// alternating-live.jsonl 90-94: the reporter needs the destination reply too, to
/// advance the client's sync copy while its drawn copy is under keyboard control.
pub async fn handle_position_update(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    if frame.body().len() < 24 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 26 }));
    }
    let position = (
        f32::from_le_bytes([
            frame.body()[0],
            frame.body()[1],
            frame.body()[2],
            frame.body()[3],
        ]),
        f32::from_le_bytes([
            frame.body()[4],
            frame.body()[5],
            frame.body()[6],
            frame.body()[7],
        ]),
    );
    let plane = u32::from_le_bytes([
        frame.body()[8],
        frame.body()[9],
        frame.body()[10],
        frame.body()[11],
    ]) as u16;
    let offset = (
        f32::from_le_bytes(frame.body()[12..16].try_into().unwrap()),
        f32::from_le_bytes(frame.body()[16..20].try_into().unwrap()),
    );
    let move_type = u32::from_le_bytes(frame.body()[20..24].try_into().unwrap()) as u8;
    if ![position.0, position.1, offset.0, offset.1]
        .iter()
        .all(|v| v.is_finite())
        || !(1..=8).contains(&move_type)
    {
        return Ok(true);
    }
    let now = Instant::now();
    let before = player.standing();
    let instance = Arc::clone(&player.instance);
    let Some(outcome) = instance.with_slot(player.player_id, |slot| {
        let since = slot.agent.since_last_report(now);
        let walk_was_running = 0.0 < slot.agent.motion.speed;
        let adopted = slot.agent.adopt_report(position, plane, now);
        let standing = Standing {
            position: slot.agent.position,
            plane: slot.agent.plane,
            direction: slot.agent.facing,
        };
        if move_trace_on() {
            slot.move_stats.reports += 1;
            match adopted {
                Adopted::Accepted => slot.move_stats.accepted += 1,
                Adopted::Clamped => slot.move_stats.clamped += 1,
                Adopted::Refused => slot.move_stats.refused += 1,
            }
            let distance = {
                let dx = position.0 - before.position.0;
                let dy = position.1 - before.position.1;
                (dx * dx + dy * dy).sqrt()
            };
            slot.move_stats.max_drift = slot.move_stats.max_drift.max(distance);
            eprintln!(
                "move[manual] report (x={:.1}, y={:.1}, plane={plane}) against the server's \
                 (x={:.1}, y={:.1}, plane={}) - d={distance:.1}, {} ms since the last report, \
                 server walk running={walk_was_running}: {adopted:?}, agent now \
                 (x={:.1}, y={:.1}, plane={}) [0x803D, {} bytes]",
                position.0,
                position.1,
                before.position.0,
                before.position.1,
                before.plane,
                since.map(|elapsed| elapsed.as_millis()).unwrap_or(0),
                standing.position.0,
                standing.position.1,
                standing.plane,
                frame.body().len() + 2
            );
            if adopted == Adopted::Accepted && plane != before.plane {
                slot.move_stats.plane_changes += 1;
                eprintln!(
                    "move[jump] the client's report changed the plane {before_plane} -> {plane} \
                     at (x={:.1}, y={:.1}){note}",
                    position.0,
                    position.1,
                    before_plane = before.plane,
                    note = if distance > 50.0 {
                        " - more than the tolerance away, so this is a real plane change, not stairs"
                    } else {
                        ""
                    }
                );
            }
            if adopted != Adopted::Accepted && distance > 50.0 {
                eprintln!(
                    "move[jump] report rejected or clamped {adopted:?}: client at (x={:.1}, \
                     y={:.1}, plane={plane}), server at (x={:.1}, y={:.1}, plane={}), d={distance:.1}",
                    position.0, position.1, before.position.0, before.position.1, before.plane
                );
            }
        }
        let reply = if adopted != Adopted::Refused {
            let from = slot.agent.here();
            // The report names a direction, not a surface. The point `offset` away can sit on a bridge
            // or slope plane the character is not standing on, so the destination's plane is the graph's
            // answer; stamping `from.plane` there made the search fail, and a failed search is answered
            // with silence (`pathfind::PathGraphs::walk_any_plane` has the measurement).
            let target = (from.x + offset.0, from.y + offset.1);
            let walked = world.graphs.walk_any_plane(instance.map.map_id, from, target);
            if move_trace_on() {
                match &walked {
                    Walk::Found(waypoints) => eprintln!(
                        "move[manual] answered with a walk to (x={:.1}, y={:.1}): {} waypoint(s) {}",
                        target.0,
                        target.1,
                        waypoints.len(),
                        waypoints_preview(waypoints)
                    ),
                    Walk::NoPath => eprintln!(
                        "move[manual] the finder refused a walk to (x={:.1}, y={:.1}) from \
                         (x={:.1}, y={:.1}, plane={}): nothing is sent",
                        target.0, target.1, from.x, from.y, from.plane
                    ),
                    Walk::NoGraph => eprintln!(
                        "move[manual] this world has no path graph for map {}: nothing is sent",
                        instance.map.map_id
                    ),
                }
            }
            let waypoints = match walked {
                Walk::Found(waypoints) => waypoints,
                Walk::NoPath | Walk::NoGraph => Vec::new(),
            };
            let norm = (offset.0 * offset.0 + offset.1 * offset.1).sqrt();
            let direction = if norm > 0.0 { (offset.0 / norm, offset.1 / norm) } else { (0.0, 0.0) };
            slot.agent.begin_manual_walk(waypoints, direction, move_type)
        } else {
            Vec::new()
        };
        (adopted, standing, reply)
    }) else {
        return Ok(true);
    };
    let (adopted, standing, reply) = outcome;
    // The step the report moved the agent along: the doorway test reads it after this handler returns
    // (`travel::portal_crossed`), because the crossing is judged by the path, not by where a report
    // happened to land.
    player.last_step = if adopted == Adopted::Refused {
        None
    } else if before.position == standing.position {
        None
    } else {
        Some((before.position, standing.position))
    };
    instance.broadcast(player.player_id, &reply);
    send(stream, session, reply, trace).await?;
    if trace {
        eprintln!(
            "world: agent {} reported ({:.1}, {:.1}) plane {plane}: {adopted:?} -> ({:.1}, {:.1})",
            player.GetAgentID(),
            position.0,
            position.1,
            standing.position.0,
            standing.position.1
        );
    }
    Ok(true)
}

/// The client stopped walking (`GameSrv_HandleCancelMovement`): cancel the agent's own walk --
/// `GameSrv_CancelAgentMovement` sets its destination to where it stands and its speed to zero --
/// and then tell everyone it stopped. Cancelling is the load-bearing half: without it the server
/// keeps advancing its copy along the abandoned path, and the next click is answered from a
/// position the client walked away from minutes ago (observed live: a click to x4 started from
/// the old x2 the client had already left, pulling the agent back to it).
pub async fn handle_cancel_movement(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let _ = world;
    let instance = Arc::clone(&player.instance);
    let Some((was_walking, stop, stood)) = instance.with_slot(player.player_id, |slot| {
        let was_walking = 0.0 < slot.agent.motion.speed;
        slot.agent.cancel_movement();
        let stood = Standing {
            position: slot.agent.position,
            plane: slot.agent.plane,
            direction: slot.agent.facing,
        };
        (
            was_walking,
            game::agent_stop_moving(slot.agent.agent_id),
            stood,
        )
    }) else {
        return Ok(true);
    };
    if move_trace_on() {
        eprintln!(
            "move[stop] client cancelled walking at (x={:.1}, y={:.1}, plane={}){} \
             [0x8028, {} bytes]",
            stood.position.0,
            stood.position.1,
            stood.plane,
            if was_walking {
                " - a server-side walk WAS running: cancelled (speed -> 0, destination cleared)"
            } else {
                " (no server-side walk was running)"
            },
            frame.body().len() + 2
        );
    }
    instance.broadcast(player.player_id, std::slice::from_ref(&stop));
    send(stream, session, vec![stop], trace).await?;
    Ok(true)
}

/// The keyboard stop includes its stopping point; an accepted point ends the old
/// path and answers with type 9 and that point (alternating-live.jsonl 482-486).
///
/// A refused correction sends **nothing at all**. The reference refuses a long or wrong-plane correction
/// and stops there (`GmAgent.c:547-563`, `docs/09-parity-audit.md` row 17); the frame it must not send is
/// the destination, because the only position it could name is its own stale guess -- and a client
/// obeys that message, so the character walks backwards to where it stood a report ago. Measured live:
/// `logs/world-8001.log` 6230-6246 is one refusal ordering the client 196.7 units back and the client
/// complying. The server's own copy still stops walking.
pub async fn handle_last_position(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    if frame.body().len() < 12 {
        return Err(GameError::Framing(FrameError::Incomplete { needed: 16 }));
    }
    let position = (
        f32::from_le_bytes([
            frame.body()[0],
            frame.body()[1],
            frame.body()[2],
            frame.body()[3],
        ]),
        f32::from_le_bytes([
            frame.body()[4],
            frame.body()[5],
            frame.body()[6],
            frame.body()[7],
        ]),
    );
    let plane = u32::from_le_bytes([
        frame.body()[8],
        frame.body()[9],
        frame.body()[10],
        frame.body()[11],
    ]) as u16;
    let instance = Arc::clone(&player.instance);
    let Some((taken, reply, believed, distance)) = instance.with_slot(player.player_id, |slot| {
        let believed = Standing {
            position: slot.agent.position,
            plane: slot.agent.plane,
            direction: slot.agent.facing,
        };
        let taken = slot.agent.adopt_correction(position, plane);
        let distance = {
            let dx = position.0 - believed.position.0;
            let dy = position.1 - believed.position.1;
            (dx * dx + dy * dy).sqrt()
        };
        let reply = if taken {
            slot.agent.stop_manual_walk()
        } else {
            // Refused: the agent stops, and no position is named, so the client is not moved.
            slot.agent.stand_still();
            Vec::new()
        };
        (taken, reply, believed, distance)
    }) else {
        return Ok(true);
    };
    if !reply.is_empty() {
        instance.broadcast(player.player_id, &reply);
        send(stream, session, reply, trace).await?;
    }
    // A correction the client was allowed to make is a movement it made while the reports lagged: the
    // step from where the server believed it stood to where it says it stood is a step the doorway
    // test reads (`travel::portal_crossed`).
    player.last_step = if taken && believed.position != position {
        Some((believed.position, position))
    } else {
        None
    };
    if move_trace_on() {
        // The distance is what decides and the ceiling is configuration, so the trace names both: a
        // refusal is otherwise indistinguishable from a plane mismatch without it.
        eprintln!(
            "move[stop] correction to (x={:.1}, y={:.1}, plane={plane}) against the server's \
             (x={:.1}, y={:.1}, plane={}) - d={distance:.1}, ceiling={:.1}: {}",
            position.0,
            position.1,
            believed.position.0,
            believed.position.1,
            believed.plane,
            max_allowed_correction(),
            if taken {
                "taken - type 9 and a destination at that point [0x802B, 0x8029]"
            } else if plane != believed.plane {
                "refused (the plane differs) - the agent stops and nothing is sent"
            } else {
                "refused (farther than the ceiling) - the agent stops and nothing is sent"
            }
        );
    }
    if trace {
        eprintln!(
            "world: agent {} correction to ({:.1}, {:.1}) plane {plane}: {}",
            player.GetAgentID(),
            position.0,
            position.1,
            if taken {
                "taken - type 9 and a destination at that point"
            } else {
                "refused - the agent stops and nothing is sent"
            }
        );
    }
    Ok(true)
}

// ---- the client asks where to stand --------------------------------------------------------

pub async fn handle_request_spawn(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let replay = world.replay.as_ref();
    let block = match replay {
        Some(replay) if replay.pick.wants(crate::replay::BlockName::Spawn) => replay.spawn.clone(),
        _ => spawn_block(&player.instance.map, player.standing()),
    };
    send(stream, session, block, trace).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn agent() -> AgentState {
        AgentState::new(1, (6602.0, 4485.0), 0, (1.0, 0.0), 288.0)
    }

    #[test]
    fn alternating_manual_and_click_walks_replace_the_old_path_and_reply_on_stop() {
        let mut agent = agent();
        agent.begin_walk(vec![
            waypoint(6702.0, 4485.0, 1),
            waypoint(6802.0, 4485.0, 2),
        ]);
        agent.adopt_report((6652.0, 4485.0), 0, Instant::now());
        let reply = agent.begin_manual_walk(vec![waypoint(6652.0, 4700.0, 3)], (0.0, 1.0), 1);
        assert_eq!(
            reply.iter().map(Frame::header).collect::<Vec<_>>(),
            vec![0x25, 0x29]
        );
        assert_eq!(agent.motion.waypoints.len(), 1);
        assert_eq!(agent.motion.destination.y, 4700.0);
        agent.advance(0.1);
        let stopped_at = agent.position;
        let stop = agent.stop_manual_walk();
        assert_eq!(
            stop.iter().map(Frame::header).collect::<Vec<_>>(),
            vec![0x2b, 0x29]
        );
        assert_eq!(stop[0].body()[8], 9);
        assert!(agent.motion.waypoints.is_empty());
        agent.advance(2.0);
        assert_eq!(agent.position, stopped_at);
        for message in reply.iter().chain(stop.iter()) {
            let bytes = message.encode();
            let (_, used) = Frame::decode_game_message(&bytes, false)
                .expect("movement replies use the packed game wire layout");
            assert_eq!(used, bytes.len());
        }
        let restart = agent.begin_manual_walk(vec![waypoint(6900.0, 4700.0, 4)], (1.0, 0.0), 1);
        assert_eq!(
            restart.iter().map(Frame::header).collect::<Vec<_>>(),
            vec![0x25, 0x2b, 0x29]
        );
    }

    #[test]
    pub fn a_report_inside_the_budget_is_believed() {
        let now = Instant::now();
        let mut agent = agent();
        let later = now + Duration::from_millis(500);
        assert_eq!(
            agent.adopt_report((6602.0 + 100.0, 4485.0), 0, later),
            Adopted::Accepted
        );
        assert_eq!(agent.position, (6702.0, 4485.0));
    }

    #[test]
    pub fn a_report_ahead_of_the_budget_is_clamped_not_dropped() {
        // The first report of a session gets the full budget, so the budget only bites from the second
        // one: 5000 units in 500 ms is not something the agent could have walked, and it is moved as far
        // as 500 ms of its own speed plus the tolerance allows, along the same line.
        let now = Instant::now();
        let mut agent = agent();
        agent.adopt_report((6602.0, 4485.0), 0, now); // the first report sets the clock
        let later = now + Duration::from_millis(500);
        assert_eq!(
            agent.adopt_report((6602.0 + 5000.0, 4485.0), 0, later),
            Adopted::Clamped
        );
        let travelled = agent.position.0 - 6602.0;
        let allowed = 288.0 * 0.5 + POSITION_UPDATE_TOLERANCE;
        assert!(
            (travelled - allowed).abs() < 0.01,
            "moved {travelled}, budget {allowed}"
        );
        assert!(agent.position.0 < 6602.0 + 5000.0, "and not the whole jump");
    }

    #[test]
    pub fn a_far_plane_change_is_refused_and_a_near_one_is_taken() {
        let now = Instant::now();
        let mut near = agent();
        let later = now + Duration::from_millis(100);
        assert_eq!(
            near.adopt_report((6602.0, 4485.0), 4, later),
            Adopted::Accepted,
            "a plane change in place is stairs or a small portal"
        );
        assert_eq!(near.plane, 4);

        let mut far = agent();
        assert_eq!(
            far.adopt_report((6602.0 + 900.0, 4485.0), 4, later),
            Adopted::Refused,
            "and a plane change far away is not"
        );
        assert_eq!(far.plane, 0, "the refusal leaves the agent where it was");
    }

    #[test]
    pub fn a_correction_is_short_on_the_same_plane() {
        let mut fresh = agent();
        assert!(fresh.adopt_correction((6602.0 + 50.0, 4485.0), 0));
        assert_eq!(fresh.position, (6652.0, 4485.0));

        let mut other = agent();
        assert!(
            !other.adopt_correction((6602.0 + 400.0, 4485.0), 0),
            "too far"
        );
        assert!(!other.adopt_correction((6602.0, 4485.0), 3), "wrong plane");
        assert_eq!(other.position, (6602.0, 4485.0));
    }

    #[test]
    pub fn the_ceiling_comes_from_the_movement_file_and_not_a_constant() {
        // `config/Server/movement.json` carries `max_allowed_correction`; the reference's own 100 is only
        // the fallback a checkout with no file runs on. What matters here is that the ceiling is read,
        // not that it is any particular number: an unchanged constant would make this test pass too, so
        // the file's shipped value is what is asserted.
        let ceiling = max_allowed_correction();
        let mut inside = agent();
        assert!(
            inside.adopt_correction((6602.0 + ceiling - 1.0, 4485.0), 0),
            "a point just inside the ceiling is taken"
        );
        let mut outside = agent();
        assert!(
            !outside.adopt_correction((6602.0 + ceiling + 1.0, 4485.0), 0),
            "a point just outside it is refused"
        );
    }

    #[test]
    pub fn a_refused_correction_stops_the_walk_and_names_no_point() {
        // The two frames `stop_manual_walk` builds are what a refused correction must NOT send: the only
        // position it could name is the server's stale guess, and the client walks to it. So a refusal
        // stops the copy and produces no frames at all -- the reference's own reply (`GmAgent.c:547-563`).
        let mut walker = agent();
        walker.begin_walk(vec![
            waypoint(6702.0, 4485.0, 1),
            waypoint(6802.0, 4485.0, 2),
        ]);
        walker.advance(0.1);
        let believed = walker.position;

        let far = (believed.0 + max_allowed_correction() + 50.0, believed.1);
        assert!(!walker.adopt_correction(far, 0), "out of budget");
        // `stand_still` returns no frames at all, so "nothing goes on the wire" is the signature's claim
        // and not an assertion that could be quiet about it.
        walker.stand_still();
        assert_eq!(walker.position, believed, "and the agent is not moved");
        assert_eq!(walker.motion.speed, 0.0, "the copy stops walking");
        assert!(walker.motion.waypoints.is_empty(), "the path is dropped");
        assert_eq!(walker.advance(2.0), None, "and it stays stopped");
        assert_eq!(walker.position, believed);
    }

    #[test]
    pub fn the_first_report_of_a_session_gets_the_full_budget() {
        // A client that stood still through the load then reports a position is not cheating: the
        // reference gives the first report the maximum elapsed time, and so does this.
        let mut agent = agent();
        assert_eq!(
            agent.adopt_report((6602.0 + 500.0, 4485.0), 0, Instant::now()),
            Adopted::Accepted
        );
    }

    pub fn waypoint(x: f32, y: f32, trap_id: u32) -> Waypoint {
        Waypoint {
            pos: Pos { x, y, plane: 0 },
            trap_id,
        }
    }

    #[test]
    pub fn a_walk_starts_at_the_paths_first_waypoint() {
        let mut walker = agent();
        let started = walker
            .begin_walk(vec![
                waypoint(6702.0, 4485.0, 1),
                waypoint(6802.0, 4485.0, 2),
            ])
            .expect("a walk with waypoints starts");
        assert_eq!(started.trap_id, 1, "the reference takes waypoint 0");
        assert_eq!(walker.motion.destination.x, 6702.0);
        assert_eq!(
            walker.motion.speed, 288.0,
            "and starts moving at speed_base"
        );
        assert_eq!(walker.motion.direction, (1.0, 0.0), "pointing at it");
        assert_eq!(walker.motion.waypoint_idx, 1, "with the rest still to walk");

        // An empty path is the reference's `0 < agent->waypoints.len` check: nothing starts.
        let mut empty = agent();
        assert!(empty.begin_walk(Vec::new()).is_none());
    }

    #[test]
    pub fn the_tick_walks_the_segment_and_hands_over_to_the_next_waypoint() {
        // 288 units a second, half a second at a time: 144 units a tick, and what is left of a tick after
        // a waypoint is spent on the next segment -- which is what the reference's `dist -=` does.
        let mut walker = agent();
        walker.begin_walk(vec![
            waypoint(6602.0 + 200.0, 4485.0, 1),
            waypoint(6602.0 + 500.0, 4485.0, 2),
        ]);

        assert_eq!(walker.advance(0.5), None, "still on the first segment");
        assert!((walker.position.0 - (6602.0 + 144.0)).abs() < 0.01);

        // The second tick covers the 56 units left to the first waypoint and spends the other 88 on the
        // segment after it, taking that waypoint as the new destination as it goes.
        let next = walker.advance(0.5).expect("a waypoint was crossed");
        assert_eq!(next.trap_id, 2);
        assert_eq!(walker.motion.destination.x, 6602.0 + 500.0);
        assert!(
            (walker.position.0 - (6602.0 + 200.0 + 88.0)).abs() < 0.01,
            "position {} after the overshoot",
            walker.position.0
        );
    }

    #[test]
    pub fn arriving_ends_the_walk() {
        let mut agent = agent();
        agent.begin_walk(vec![waypoint(6602.0 + 100.0, 4485.0, 1)]);
        // 100 units at 288 a second is inside one tick.
        assert_eq!(
            agent.advance(0.5),
            None,
            "the last waypoint is the destination"
        );
        assert_eq!(agent.position, (6602.0 + 100.0, 4485.0));
        assert_eq!(agent.motion.speed, 0.0, "and the walk is over");
        assert!(agent.motion.waypoints.is_empty());
        assert_eq!(agent.motion.destination, Pos::default());
        assert_eq!(agent.advance(0.5), None, "a standing agent does not move");
    }

    #[test]
    pub fn a_walk_to_where_the_agent_stands_does_not_poison_its_direction() {
        // The reference divides by the distance here without checking it, so this exact case gives it a
        // NaN direction and then a NaN position. This keeps the direction it had and ends the walk on the
        // next tick instead -- the one deliberate difference in the movement code.
        let mut walker = agent();
        walker.begin_walk(vec![waypoint(6602.0, 4485.0, 1)]);
        assert_eq!(
            walker.motion.direction,
            (0.0, 0.0),
            "unchanged (nothing to point at yet), not NaN"
        );
        walker.advance(0.5);
        assert_eq!(walker.position, (6602.0, 4485.0));
        assert_eq!(
            (walker.motion.speed, walker.motion.destination),
            (0.0, Pos::default()),
            "the walk is over immediately"
        );
        assert!(walker.position.0.is_finite() && walker.position.1.is_finite());
    }

    #[test]
    pub fn cancelling_stands_still_without_forgetting_the_path() {
        // `GameSrv_CancelAgentMovement` sets the destination to the position and the speed to zero; the
        // waypoint list is the caller's business, and `MOVE_TO_COORD` clears it first.
        let mut agent = agent();
        agent.begin_walk(vec![
            waypoint(6802.0, 4485.0, 1),
            waypoint(6902.0, 4485.0, 2),
        ]);
        agent.cancel_movement();
        assert_eq!(agent.motion.speed, 0.0);
        assert_eq!(agent.motion.destination.x, 6602.0);
        assert_eq!(agent.advance(0.5), None, "and nothing moves");
        assert_eq!(agent.position, (6602.0, 4485.0));
    }
}
