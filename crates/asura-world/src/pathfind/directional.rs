//! STATUS: PROVISIONAL / NOT ACCEPTED (owner client test, 2026-11-06).
//! Movement is better but not perfect; the owner is dissatisfied with this implementation.
//! Remaining movement/synchronization defects are unresolved. This is not proven live parity.
//! Further functional work is paused; see docs/HANDOFF.md.
//!
//! Manual movement follows a direction through adjacent cells. It never searches for a route.
//! Live bridge evidence: northlands-live-20261106-014922.jsonl, reports 256/263/278/282/334/345.
use super::{PathContext, Pos, Waypoint};
use crate::map_data::{Trapezoid, Vec2f};

fn sub(a: Vec2f, b: Vec2f) -> Vec2f {
    Vec2f {
        x: a.x - b.x,
        y: a.y - b.y,
    }
}
fn cross(a: Vec2f, b: Vec2f) -> f32 {
    a.x * b.y - a.y * b.x
}
fn dot(a: Vec2f, b: Vec2f) -> f32 {
    a.x * b.x + a.y * b.y
}
fn along(p: Vec2f, direction: Vec2f, distance: f32) -> Vec2f {
    Vec2f {
        x: p.x + direction.x * distance,
        y: p.y + direction.y * distance,
    }
}
fn length(v: Vec2f) -> f32 {
    v.x.hypot(v.y)
}

// Clockwise edges: top, right, bottom, left. Interior points are on their right.
fn edges(t: Trapezoid) -> [(Vec2f, Vec2f); 4] {
    let tl = Vec2f { x: t.xtl, y: t.yt };
    let tr = Vec2f { x: t.xtr, y: t.yt };
    let bl = Vec2f { x: t.xbl, y: t.yb };
    let br = Vec2f { x: t.xbr, y: t.yb };
    [(tl, tr), (tr, br), (br, bl), (bl, tl)]
}
fn contains(t: Trapezoid, p: Vec2f, tolerance: f32) -> bool {
    edges(t)
        .into_iter()
        .all(|(a, b)| cross(sub(b, a), sub(p, a)) <= tolerance * length(sub(b, a)))
}

fn neighbours(context: &PathContext, trap: u32, edge: usize) -> Vec<u32> {
    let t = context.trap(trap);
    if edge == 0 || edge == 2 {
        let links = if edge == 0 {
            [t.top_left, t.top_right]
        } else {
            [t.bottom_left, t.bottom_right]
        };
        return links
            .into_iter()
            .filter_map(|link| context.neighbour(trap, link))
            .collect();
    }
    let plane = context.plane_of(trap);
    let id = if edge == 3 {
        t.portal_left
    } else {
        t.portal_right
    };
    let Some(portal) = context.portal(plane, usize::from(id)) else {
        return Vec::new();
    };
    let Some((other_plane, other_id)) = portal.pair else {
        return Vec::new();
    };
    let Some(pair) = context.portal(other_plane, other_id) else {
        return Vec::new();
    };
    // Directional walking follows paired surface links; A*'s route filters do not select its plane.
    context.portal_pair_traps(&pair)
}

fn emit(points: &mut Vec<Waypoint>, source: Pos, p: Vec2f, plane: u16, trap: u32, tolerance: f32) {
    let pos = Pos {
        x: p.x,
        y: p.y,
        plane,
    };
    if points.last().is_some_and(|point| point.pos == pos) {
        return;
    }
    if points.is_empty()
        && source.plane == plane
        && (source.x - p.x).hypot(source.y - p.y) <= tolerance
    {
        return;
    }
    points.push(Waypoint { pos, trap_id: trap });
}

// Portion of an edge lying inside a neighbouring cell, in edge parameters [0, 1].
fn coverage(t: Trapezoid, a: Vec2f, b: Vec2f, tolerance: f32) -> Option<(f32, f32)> {
    let delta = sub(b, a);
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for (origin, end) in edges(t) {
        let edge = sub(end, origin);
        let value = cross(edge, sub(a, origin));
        let slope = cross(edge, delta);
        let margin = tolerance * length(edge);
        if slope == 0.0 {
            if value > margin {
                return None;
            }
        } else {
            let at = (margin - value) / slope;
            if slope > 0.0 {
                high = high.min(at);
            } else {
                low = low.max(at);
            }
            if low > high {
                return None;
            }
        }
    }
    Some((low.max(0.0), high.min(1.0)))
}

/// Trace the reported offset through connected cells, emitting plane transitions and wall bends.
/// A blocked direction slides along the exposed wall span; it does not navigate around the wall.
/// Work is bounded by the number of cell edges, so broken adjacency cannot loop indefinitely.
pub(super) fn walk(
    context: &PathContext,
    source: Pos,
    offset: (f32, f32),
) -> Option<Vec<Waypoint>> {
    let settings = asura_config::directional::current().ok()?;
    let tolerance = settings.edge_tolerance.value;
    let distance = offset.0.hypot(offset.1);
    if !distance.is_finite() || !source.x.is_finite() || !source.y.is_finite() {
        return None;
    }
    if distance == 0.0 {
        return Some(Vec::new());
    }
    let direction = Vec2f {
        x: offset.0 / distance,
        y: offset.1 / distance,
    };
    let mut remaining = distance + settings.distance_slack.value;
    let mut position = Vec2f {
        x: source.x,
        y: source.y,
    };
    let mut trap = context.find_trapezoid(source)?;
    if !contains(context.trap(trap), position, tolerance) {
        return None;
    }
    let mut points = Vec::new();
    let edge_count = edges(context.trap(trap)).len();
    let work_limit = (context.data.traps_count as usize).saturating_mul(edge_count);
    for _ in 0..work_limit {
        let current = context.trap(trap);
        let plane = context.plane_of(trap);
        let boundaries = edges(current);
        let mut exit_distance = f32::INFINITY;
        let mut exits = Vec::new();
        for (id, (a, b)) in boundaries.iter().copied().enumerate() {
            let edge = sub(b, a);
            let outward = cross(edge, direction);
            if outward <= 0.0 {
                continue;
            }
            let distance = -cross(edge, sub(position, a)) / outward;
            if distance < -tolerance {
                continue;
            }
            let distance = distance.max(0.0);
            if distance < exit_distance - tolerance {
                exit_distance = distance;
                exits.clear();
                exits.push(id);
            } else if (distance - exit_distance).abs() <= tolerance {
                exits.push(id);
            }
        }
        if remaining <= exit_distance {
            emit(
                &mut points,
                source,
                along(position, direction, remaining),
                plane,
                trap,
                tolerance,
            );
            return Some(points);
        }
        if !exit_distance.is_finite() {
            return None;
        }
        position = along(position, direction, exit_distance);
        remaining -= exit_distance;
        let probe = along(position, direction, tolerance);
        let next = exits
            .iter()
            .flat_map(|edge| neighbours(context, trap, *edge))
            .find(|next| *next != trap && contains(context.trap(*next), probe, tolerance));
        if let Some(next) = next {
            let next_plane = context.plane_of(next);
            if next_plane != plane {
                emit(&mut points, source, position, next_plane, next, tolerance);
            }
            trap = next;
            continue;
        }
        // No connected cell covers the crossing. Preserve the contact, then consume only the
        // exposed part of this edge while sliding at the movement speed.
        emit(&mut points, source, position, plane, trap, tolerance);
        let wall = *exits.first()?;
        let (a, b) = boundaries[wall];
        let edge = sub(b, a);
        let edge_length = length(edge);
        if edge_length == 0.0 {
            return Some(points);
        }
        let projection = dot(edge, direction);
        if projection.abs() <= f32::EPSILON {
            return Some(points);
        }
        let parameter = (dot(sub(position, a), edge) / (edge_length * edge_length)).clamp(0.0, 1.0);
        let (mut low, mut high) = (0.0_f32, 1.0_f32);
        for neighbour in neighbours(context, trap, wall) {
            if let Some((start, end)) = coverage(context.trap(neighbour), a, b, tolerance) {
                if end <= parameter {
                    low = low.max(end);
                }
                if start >= parameter {
                    high = high.min(start);
                }
            }
        }
        let endpoint = if projection > 0.0 { high } else { low };
        let target = along(a, edge, endpoint);
        let slide_delta = sub(target, position);
        let slide_distance = length(slide_delta);
        if slide_distance <= tolerance {
            return Some(points);
        }
        let slide_direction = Vec2f {
            x: slide_delta.x / slide_distance,
            y: slide_delta.y / slide_distance,
        };
        if remaining <= slide_distance {
            emit(
                &mut points,
                source,
                along(position, slide_direction, remaining),
                plane,
                trap,
                tolerance,
            );
            return Some(points);
        }
        position = target;
        remaining -= slide_distance;
        emit(&mut points, source, position, plane, trap, tolerance);
        // At the corner or opening, try the requested direction again in the current cell.
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathfind::{PathGraphs, Walk};

    #[test]
    fn manual_bridge_targets_match_live_plane_transitions() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let graphs =
            PathGraphs::from_dump(root.join("data/path-graphs.bin").to_str().unwrap()).unwrap();
        let map = crate::catalogue::MapEntry {
            map_id: 146,
            map_file_id: 113021,
            map_type: 1,
            spawn: (0.0, 0.0),
            spawn_plane: 0,
            spawns: Vec::new(),
            name: "Lakeside County".to_string(),
            capacity: 1,
        };
        graphs.bind(&[map]);
        // Client-struct capture northlands-live-20261106-014922.jsonl. Each row names the report
        // and its player destination reply; packet sizes are not treated as wire sizes.
        let cases = [
            (
                256, 260, -5011.6377, 11684.587, 0, -732.6177, 230.42427, -5507.68, 11840.604, 36,
            ),
            (
                263, 266, -5143.702, 11741.686, 0, -695.6177, 318.38318, -5353.847, 11837.868, 36,
            ),
            (
                278, 281, -5391.7485, 11884.838, 36, -526.7579, 558.88293, -5615.907, 12122.667, 37,
            ),
            (
                282, 287, -5459.6826, 11999.159, 36, -204.03519, 737.3069, -5493.7905, 12122.413,
                37,
            ),
            (
                334, 337, -5506.978, 12744.496, 37, 15.784103, 767.83777, -5491.184, 13512.834, 37,
            ),
            (
                345, 348, -5478.6016, 13093.659, 37, 10.120007, 764.9506, -5469.52, 13780.174, 36,
            ),
        ];
        for (report, reply, x, y, plane, dx, dy, expected_x, expected_y, expected_plane) in cases {
            let Walk::Found(points) = graphs.walk_direction(146, Pos { x, y, plane }, (dx, dy))
            else {
                panic!("manual report {report} produced no movement");
            };
            let first = points.first().expect("a live destination");
            assert_eq!(
                first.pos.plane, expected_plane,
                "report {report}, reply {reply}"
            );
            assert!(
                (first.pos.x - expected_x).hypot(first.pos.y - expected_y) < 0.05,
                "report {report}, reply {reply}: {:?}",
                first.pos
            );
        }
    }
}
