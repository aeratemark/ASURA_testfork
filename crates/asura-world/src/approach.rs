//! Walking a player to another agent: spacebar or a click on an NPC (`GAME_CMSG_INTERACT_LIVING`,
//! `0x8039`), on a foe (`GAME_CMSG_ATTACK_AGENT`, `0x8026`), or on a party member
//! (`GAME_CMSG_INTERACT_PLAYER`, `0x8033`).
//!
//! What the live server does, measured (`docs/52-skills.md`, "Walking to a target"):
//!
//! * It answers each of the three with `GAME_SMSG_AGENT_UPDATE_DESTINATION` (`0x002A`) for the
//!   requesting player's own agent: the target's current position, two planes and the target's agent
//!   id (`movement-live-20261109-200703.jsonl` seq 626 -> 630; `multiplay-live-20261106-195109.jsonl`
//!   seq 2154 -> 2158; `titles-live-20261106-034507.jsonl` seq 122 -> 124). The client walks itself
//!   there: no `AGENT_MOVE_TO_POINT` for the player and no position report from it follow.
//! * While the target walks, the same message goes out again about every half second with the
//!   target's new position (`movement-live-20261109-200703.jsonl` seq 721, 729, 747, 752: steps of 40
//!   units along the NPC's own walk). A target that stands still is not announced again.
//! * When the walker is there, `GAME_SMSG_AGENT_STOP_MOVING` (`0x0028`) for it (seq 637, 5129, 5557).
//! * A player already at the NPC gets no walk at all (seq 5260: the dialog at once).
//!
//! The server keeps its own copy of the walk, along the path finder's route, because nothing else tells
//! it where the walker is. An interaction ends on arrival; an attack lasts while the fight's attack is
//! on that foe; a follow lasts until the player moves by itself.

use crate::agent::{distance_between, AgentState, Walk};
use crate::npc::behavior::Pathing;
use crate::pathfind::{Pos, Waypoint};
use crate::*;

/// Why the player walks to the agent: what ends the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// `INTERACT_LIVING`: walk there, stop, done.
    Interact,
    /// `INTERACT_PLAYER`: walk there, and walk again whenever the target walks away.
    Follow,
    /// `ATTACK_AGENT`: like a follow, for as long as the fight's attack is on the target.
    Attack,
}

/// How often a walking target's new position is announced: the live cadence (module docs).
pub const ANNOUNCE_MS: u32 = 500;

/// How far a target must have moved since the last announcement, or the last route, to be announced
/// or routed again.
pub const MOVED: f32 = 1.0;

/// A stopped walker sets off again only once the target is this many ranges away, so a target
/// shuffling at the edge of the range does not start and stop it every tick. The fight's own
/// `range_tolerance` is the same 1.25: a melee swing still lands inside it.
pub const RESUME_FACTOR: f32 = 1.25;

/// The route ends this far inside the range, so the walk's end is an arrival.
pub const STOP_SHORT: f32 = 0.9;

/// One player's walk to an agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Approach {
    /// The agent walked to.
    pub target: u32,
    /// What ends the walk.
    pub purpose: Purpose,
    /// How near is there.
    pub range: f32,
    /// Where the target was when the walk was last routed; `None` before the first route.
    routed_to: Option<(f32, f32)>,
    /// Where the target was when it was last announced; `None` before the first announcement and
    /// after a stop, so the next walk is announced at once.
    announced: Option<(f32, f32)>,
    since_announce_ms: u32,
    /// The walker is standing at the target.
    stopped: bool,
    /// A walk this approach started has not been stopped yet. The server's copy can arrive at the
    /// route's end on its own (`AgentState::advance`), and the stop is still owed to the clients.
    under_way: bool,
}

impl Approach {
    /// A walk to `target` that is there within `range`, not yet routed or announced.
    pub fn new(target: u32, purpose: Purpose, range: f32) -> Approach {
        Approach {
            target,
            purpose,
            range,
            routed_to: None,
            announced: None,
            since_announce_ms: 0,
            stopped: false,
            under_way: false,
        }
    }
}

/// What one tick of an approach produced: frames for everyone in the instance, and whether it is over.
#[derive(Debug, Default)]
pub struct Steered {
    /// What everyone in the instance is told.
    pub frames: Vec<Frame>,
    /// The approach is over and leaves the slot.
    pub done: bool,
    /// An interaction's walker is at the agent: the connection answers the talk now
    /// (`crate::vendor::talk`). Never set for a follow or an attack.
    pub arrived: bool,
}

/// Stop the server's copy of the walk, keeping the walker's movement type.
fn halt(walker: &mut AgentState) {
    walker.cancel_movement();
    walker.motion.waypoints.clear();
    walker.motion.waypoint_idx = 0;
}

/// End an approach the player did not finish (the target is gone, the attack is over): a walk in
/// progress stops, and everyone is told.
pub fn abandon(walker: &mut AgentState) -> Vec<Frame> {
    if 0.0 < walker.motion.speed {
        halt(walker);
        vec![game::agent_stop_moving(walker.agent_id)]
    } else {
        Vec::new()
    }
}

/// Cut a route where it first comes within `stop` of `target`, so the walker stops in front of the
/// target instead of inside it. A route that never comes that near is kept whole.
pub fn shorten(
    waypoints: Vec<Waypoint>,
    from: Pos,
    target: (f32, f32),
    stop: f32,
) -> Vec<Waypoint> {
    let mut kept = Vec::with_capacity(waypoints.len());
    let mut at = from;
    for waypoint in &waypoints {
        let end = (waypoint.pos.x, waypoint.pos.y);
        if stop < distance_between(end, target) {
            kept.push(*waypoint);
            at = waypoint.pos;
            continue;
        }
        // The segment `at -> end` enters the circle: the first point on it at `stop` from the target,
        // the smaller root of |at + t (end - at) - target| = stop.
        let (dx, dy) = (end.0 - at.x, end.1 - at.y);
        let (fx, fy) = (at.x - target.0, at.y - target.1);
        let a = dx * dx + dy * dy;
        let b = 2.0 * (fx * dx + fy * dy);
        let c = fx * fx + fy * fy - stop * stop;
        let t = if a > 0.0 && c > 0.0 {
            let disc = (b * b - 4.0 * a * c).max(0.0);
            ((-b - disc.sqrt()) / (2.0 * a)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if t > 0.0 {
            kept.push(Waypoint {
                pos: Pos {
                    x: at.x + dx * t,
                    y: at.y + dy * t,
                    plane: at.plane,
                },
                trap_id: waypoint.trap_id,
            });
        }
        return kept;
    }
    waypoints
}

/// One tick of one approach: the walker's copy, the target where it stands now, the elapsed time.
pub fn steer(
    approach: &mut Approach,
    walker: &mut AgentState,
    target: Pos,
    map_id: i32,
    pathing: &dyn Pathing,
    elapsed_ms: u32,
) -> Steered {
    approach.since_announce_ms = approach.since_announce_ms.saturating_add(elapsed_ms);
    let there = (target.x, target.y);
    let distance = walker.distance_to(there);
    let mut steered = Steered::default();

    if distance <= approach.range {
        // There. A walk that brought it here is stopped and said so; a player that was already here
        // hears nothing (module docs, seq 5260).
        if approach.under_way || 0.0 < walker.motion.speed {
            halt(walker);
            steered
                .frames
                .push(game::agent_stop_moving(walker.agent_id));
        }
        approach.under_way = false;
        approach.stopped = true;
        approach.routed_to = None;
        approach.announced = None;
        steered.done = approach.purpose == Purpose::Interact;
        steered.arrived = steered.done;
        return steered;
    }
    if approach.stopped && distance <= approach.range * RESUME_FACTOR {
        return steered;
    }

    let walking = 0.0 < walker.motion.speed;
    let moved = !matches!(approach.routed_to, Some(at) if distance_between(at, there) <= MOVED);
    if moved || !walking {
        let route = match pathing.leg(map_id, walker.here(), target) {
            Walk::Found(waypoints) => {
                shorten(waypoints, walker.here(), there, approach.range * STOP_SHORT)
            }
            // No route: the finder decides reachability, as it does for a click, and no walk is
            // invented. A walk in progress stops.
            Walk::NoPath | Walk::NoGraph => {
                steered.frames = abandon(walker);
                steered.done = true;
                return steered;
            }
        };
        if walker.move_type != 1 || walker.move_speed != 1.0 {
            walker.move_type = 1;
            walker.move_speed = 1.0;
            steered
                .frames
                .push(game::agent_update_speed(walker.agent_id, 1.0, 1));
        }
        if walker.begin_walk(route).is_none() {
            steered.done = true;
            return steered;
        }
        approach.routed_to = Some(there);
        approach.stopped = false;
        approach.under_way = true;
    }

    let first = approach.announced.is_none();
    let changed = !matches!(approach.announced, Some(at) if distance_between(at, there) <= MOVED);
    if first || (changed && ANNOUNCE_MS <= approach.since_announce_ms) {
        steered.frames.push(game::agent_update_destination(
            walker.agent_id,
            there,
            target.plane,
            walker.plane,
            approach.target,
        ));
        approach.announced = Some(there);
        approach.since_announce_ms = 0;
    }
    steered
}

// ---- the client's requests -----------------------------------------------------------------------

/// Put the player on its way to `target`, replacing any approach it had. Refused, with a trace line,
/// for its own agent and for an agent the instance does not hold.
pub fn start(player: &Player, target: u32, purpose: Purpose, range: f32, trace: bool) {
    if target == player.agent_id || !player.instance.has_agent(target) {
        if trace {
            eprintln!(
                "world: {} cannot walk to agent {target}: not another agent in this instance",
                player.name
            );
        }
        return;
    }
    player.instance.with_slot(player.player_id, |slot| {
        slot.approach = Some(Approach::new(target, purpose, range));
    });
}

/// The player moved by itself (a click, the keyboard, a stop): the walk to an agent is over.
pub fn forget(player: &Player) {
    player
        .instance
        .with_slot(player.player_id, |slot| slot.approach = None);
}

/// `GAME_CMSG_INTERACT_LIVING` (`0x8039`): an agent id and a byte (0 in every capture).
pub fn handle_interact_living(
    frame: &Frame,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let body = frame.body();
    if body.len() != 5 {
        if trace {
            eprintln!("world: an interact request of {} bytes", body.len());
        }
        return Ok(true);
    }
    let target = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let mut range = asura_config::movement().interact_range();
    // An NPC the player cannot walk up to (Sir Tydus on his stand) is answered from further out, by
    // the talk range its dialog names; the usual range is never shortened.
    if let Some(talk_range) = player
        .instance
        .npc_target(target)
        .and_then(|npc| npc.name_text())
        .and_then(|name| crate::dialog::talk_range_for(player.instance.map.map_id, &name))
    {
        range = range.max(talk_range);
    }
    start(player, target, Purpose::Interact, range, trace);
    Ok(true)
}

/// `GAME_CMSG_INTERACT_PLAYER` (`0x8033`): an agent id.
pub fn handle_interact_player(
    frame: &Frame,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let body = frame.body();
    if body.len() != 4 {
        if trace {
            eprintln!("world: a follow request of {} bytes", body.len());
        }
        return Ok(true);
    }
    let target = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let range = asura_config::movement().interact_range();
    start(player, target, Purpose::Follow, range, trace);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat world: one straight leg, except to a point with a negative x.
    struct Flat;
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

    fn walker() -> AgentState {
        AgentState::new(276, (0.0, 0.0), 0, (1.0, 0.0), 288.0)
    }

    fn at(x: f32, y: f32) -> Pos {
        Pos { x, y, plane: 0 }
    }

    fn headers(frames: &[Frame]) -> Vec<u16> {
        frames.iter().map(|frame| frame.header()).collect()
    }

    #[test]
    fn a_far_target_is_announced_and_walked_to_short_of_it() {
        let mut approach = Approach::new(48, Purpose::Interact, 50.0);
        let mut me = walker();
        let out = steer(&mut approach, &mut me, at(1000.0, 0.0), 1, &Flat, 100);
        assert_eq!(headers(&out.frames), vec![game::AGENT_UPDATE_DESTINATION]);
        let body = out.frames[0].body();
        assert_eq!(&body[0..4], &276u32.to_le_bytes());
        assert_eq!(&body[4..8], &1000.0f32.to_le_bytes());
        assert_eq!(&body[16..20], &48u32.to_le_bytes());
        assert!(!out.done);
        assert!(0.0 < me.motion.speed);
        let end = me.motion.destination;
        assert!((end.x - 955.0).abs() < 0.5, "stops 45 units short: {end:?}");
    }

    #[test]
    fn arriving_stops_the_walker_and_ends_an_interaction() {
        let mut approach = Approach::new(48, Purpose::Interact, 50.0);
        let mut me = walker();
        steer(&mut approach, &mut me, at(300.0, 0.0), 1, &Flat, 100);
        for _ in 0..10 {
            me.advance(0.1);
        }
        let out = steer(&mut approach, &mut me, at(300.0, 0.0), 1, &Flat, 100);
        assert_eq!(headers(&out.frames), vec![game::AGENT_STOP_MOVING]);
        assert!(out.done);
        assert!(out.arrived, "an interaction that arrives is answered");
        assert_eq!(me.motion.speed, 0.0);
    }

    #[test]
    fn a_player_already_there_hears_nothing() {
        let mut approach = Approach::new(54, Purpose::Interact, 50.0);
        let mut me = walker();
        let out = steer(&mut approach, &mut me, at(30.0, 0.0), 1, &Flat, 100);
        assert!(out.frames.is_empty());
        assert!(out.done);
        assert!(out.arrived, "the dialog at once (seq 5260)");
    }

    #[test]
    fn a_walking_target_is_announced_again_at_the_cadence_and_a_still_one_is_not() {
        let mut approach = Approach::new(48, Purpose::Follow, 50.0);
        let mut me = walker();
        steer(&mut approach, &mut me, at(2000.0, 0.0), 1, &Flat, 100);
        // Still: nothing more, however long.
        for _ in 0..10 {
            let out = steer(&mut approach, &mut me, at(2000.0, 0.0), 1, &Flat, 100);
            assert!(out.frames.is_empty());
        }
        // Walking: rerouted at once, announced once the half second is up.
        let out = steer(&mut approach, &mut me, at(2000.0, 40.0), 1, &Flat, 100);
        assert_eq!(headers(&out.frames), vec![game::AGENT_UPDATE_DESTINATION]);
        let out = steer(&mut approach, &mut me, at(2000.0, 80.0), 1, &Flat, 100);
        assert!(out.frames.is_empty(), "within the cadence");
        for step in 0..3 {
            let y = 120.0 + 40.0 * step as f32;
            let out = steer(&mut approach, &mut me, at(2000.0, y), 1, &Flat, 100);
            assert!(out.frames.is_empty());
        }
        let out = steer(&mut approach, &mut me, at(2000.0, 240.0), 1, &Flat, 100);
        assert_eq!(headers(&out.frames), vec![game::AGENT_UPDATE_DESTINATION]);
    }

    #[test]
    fn a_follow_sets_off_again_when_the_target_walks_away() {
        let mut approach = Approach::new(48, Purpose::Follow, 50.0);
        let mut me = walker();
        let out = steer(&mut approach, &mut me, at(40.0, 0.0), 1, &Flat, 100);
        assert!(!out.done && out.frames.is_empty());
        // Inside the slack: still standing.
        let out = steer(&mut approach, &mut me, at(60.0, 0.0), 1, &Flat, 100);
        assert!(out.frames.is_empty());
        assert_eq!(me.motion.speed, 0.0);
        // Beyond it: walking and announced.
        let out = steer(&mut approach, &mut me, at(400.0, 0.0), 1, &Flat, 100);
        assert_eq!(headers(&out.frames), vec![game::AGENT_UPDATE_DESTINATION]);
        assert!(0.0 < me.motion.speed);
    }

    #[test]
    fn no_route_ends_the_approach_without_a_walk() {
        let mut approach = Approach::new(48, Purpose::Attack, 144.0);
        let mut me = walker();
        let out = steer(&mut approach, &mut me, at(-500.0, 0.0), 1, &Flat, 100);
        assert!(out.done);
        assert!(!out.arrived);
        assert!(out.frames.is_empty());
        assert_eq!(me.motion.speed, 0.0);
    }

    #[test]
    fn a_route_is_cut_where_it_enters_the_range() {
        let route = vec![
            Waypoint {
                pos: at(100.0, 0.0),
                trap_id: 1,
            },
            Waypoint {
                pos: at(100.0, 100.0),
                trap_id: 2,
            },
            Waypoint {
                pos: at(100.0, 200.0),
                trap_id: 3,
            },
        ];
        let cut = shorten(route, at(0.0, 0.0), (100.0, 200.0), 50.0);
        assert_eq!(cut.len(), 3);
        assert_eq!(cut[1].pos, at(100.0, 100.0));
        assert_eq!(cut[2].pos.x, 100.0);
        assert!((cut[2].pos.y - 150.0).abs() < 0.01, "{:?}", cut[2]);
        assert_eq!(cut[2].trap_id, 3);
    }
}
