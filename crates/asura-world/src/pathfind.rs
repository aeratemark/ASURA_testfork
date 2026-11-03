//! The reference's path finder: `GmPaths.c` and `GmPathHeap.c`, ported.
//!
//! `map_data.rs` answers "which trapezoid is this position in"; this answers the question the game server
//! actually asks: *can this agent walk from here to there, and by what waypoints*. It is an A* over the
//! trapezoid graph, where the cost is distance and the heuristic is the distance to the destination from
//! the top or bottom edge of the neighbour being considered, followed by a second pass
//! (`PathCreateWaypoints`) that walks the chosen chain of trapezoids and reduces it to a handful of
//! waypoints by following the tightest corridor between the two sides of the path.
//!
//! ## Why the arithmetic is table-driven here too
//!
//! The reference does not use the FPU for a reciprocal or a square root: `PathReciproc` and `PathSqrt`
//! are 256-entry tables of float bit patterns combined with integer arithmetic (`path_tables.rs`, generated
//! from `GmPaths.c`). Costs are built from them, so `1.0 / x` and `x.sqrt()` would be "the same algorithm"
//! and a *different path*: different tie-breaks, different corridor, different waypoints. They are ported
//! as they are, and `tools/parity-path.py` checks the result against the reference running the same cases.
//!
//! ## What is kept because the reference does it
//!
//! - `PathAddNode` overwrites a trapezoid's node unconditionally and pushes to the heap every time, so a
//!   trapezoid can be in the queue more than once; the pop side closes it on first sight.
//! - `GetClosestPointOnLineSegment` records the *offset* vector rather than the projected point. The only
//!   use of that value is the distance it accompanies, but it is what the reference computes.
//! - `PathVisitPortalLeft`/`Right` assert that a portal's pair is its mirror and then walk the pair's
//!   trapezoid list; a portal without a pair is a crash in the reference, refused here.
//! - `PathCreateWaypoints` gives up after `0xBB8` steps (an assertion in the reference, so a debug abort).

use crate::map_data::{PathStaticData, Portal, Trapezoid, Vec2f};
use crate::path_tables::{RECIPROCAL, SQUARE_ROOT};

/// `fclampf`.
fn clampf(value: f32, min: f32, max: f32) -> f32 {
    value.min(max).max(min)
}

/// `Vec2fIsZero`.
fn is_zero(vector: Vec2f) -> bool {
    vector.x == 0.0 && vector.y == 0.0
}

/// `Vec2fEqual`.
fn equal(left: Vec2f, right: Vec2f) -> bool {
    left.x == right.x && left.y == right.y
}

/// `Vec2fAdd`.
fn add(left: Vec2f, right: Vec2f) -> Vec2f {
    Vec2f {
        x: left.x + right.x,
        y: left.y + right.y,
    }
}

/// `Vec2fSub`.
fn sub(left: Vec2f, right: Vec2f) -> Vec2f {
    Vec2f {
        x: left.x - right.x,
        y: left.y - right.y,
    }
}

/// `Vec2fDot`.
fn dot(left: Vec2f, right: Vec2f) -> f32 {
    (left.x * right.x) + (left.y * right.y)
}

/// `Vec2fCross`: negative for "left of", positive for "right of" (`GmMath.h:25`).
fn cross(left: Vec2f, right: Vec2f) -> f32 {
    (left.x * right.y) - (left.y * right.x)
}

/// `Vec2fDiv`.
fn div(vector: Vec2f, scalar: f32) -> Vec2f {
    Vec2f {
        x: vector.x / scalar,
        y: vector.y / scalar,
    }
}

/// `PathReciproc` (`GmPaths.c`): the game's table-driven reciprocal, not `1.0 / value`.
pub fn path_reciproc(value: f32) -> f32 {
    let bits = value.to_bits();
    let high = (bits & 0xFF80_0000).wrapping_add(0x817F_FFFF);
    let low = RECIPROCAL[((bits & 0x007F_FFFF) >> 15) as usize];
    f32::from_bits(low.wrapping_sub(high))
}

/// `PathSqrt` (`GmPaths.c`): the game's table-driven square root, not `value.sqrt()`.
pub fn path_sqrt(value: f32) -> f32 {
    let bits = value.to_bits();
    let high = ((bits >> 24).wrapping_add(0x3F)) << 23;
    let low = SQUARE_ROOT[((bits >> 16) & 0xFF) as usize];
    f32::from_bits(high.wrapping_add(low))
}

/// `PathVec2fDist`.
fn vec2f_dist(left: Vec2f, right: Vec2f) -> f32 {
    let dx = left.x - right.x;
    let dy = left.y - right.y;
    path_sqrt((dx * dx) + (dy * dy))
}

/// `PathVec2fUnit`.
fn vec2f_unit(vector: Vec2f) -> Vec2f {
    let length = path_sqrt(dot(vector, vector));
    div(vector, length)
}

/// `GmPos` (`GmDefs.h:15`): a position and the plane it is on.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
    pub plane: u16,
}

impl Pos {
    /// The two floats, as the reference's `pos.v2` sees them.
    fn v2(self) -> Vec2f {
        Vec2f {
            x: self.x,
            y: self.y,
        }
    }
}

/// One waypoint of a walk: where to go and which trapezoid it leads into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Waypoint {
    pub pos: Pos,
    pub trap_id: u32,
}

/// `PathFindPoint`: a position and the trapezoid it was crossed into. The reference stores a
/// `PathTrapezoid *`; trapezoid ids are unique across a map's planes, so the id is the same identity.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct FindPoint {
    pos: Pos,
    trap: u32,
}

/// `PathFindNode`: one per trapezoid, filled in as the search reaches it.
#[derive(Debug, Clone, Copy, Default)]
struct FindNode {
    closed: bool,
    cost_to_node: f32,
    point: FindPoint,
    next: Option<u32>,
}

/// `PathHeapNode` (`GmPathHeap.h`).
#[derive(Debug, Clone, Copy)]
struct HeapNode {
    cost: f32,
    trap_id: u32,
}

/// `PathBuildStep` (`GmPaths.h:107`): a corridor edge the waypoint builder may cut back to.
#[derive(Debug, Clone, Copy)]
struct BuildStep {
    pos: Vec2f,
    dir: Vec2f,
    plane: u16,
    next_trap: u32,
}

/// `PathHeapPush` (`GmPathHeap.c:3`).
fn heap_push(heap: &mut Vec<HeapNode>, node: HeapNode) {
    heap.push(node);
    if heap.len() < 2 {
        return;
    }
    let mut hole = heap.len() - 1;
    while hole > 0 {
        let parent = (hole - 1) >> 1;
        if heap[parent].cost < node.cost {
            break;
        }
        heap[hole] = heap[parent];
        hole = parent;
    }
    heap[hole] = node;
}

/// `PathHeapPop` (`GmPathHeap.c:24`): a binary heap whose sift-down is spelled with the extra "last full
/// depth" case rather than a plain two-children comparison. Kept as written, because the order it pops
/// equal-cost nodes in is the order the search visits them in.
fn heap_pop(heap: &mut Vec<HeapNode>) -> Option<HeapNode> {
    if heap.is_empty() {
        return None;
    }
    let len = heap.len() - 1;
    let last_node = heap[len];
    let popped = heap[0];
    if len == 0 {
        heap.clear();
        return Some(popped);
    }

    let mut idx = 0usize;
    let mut hole = 0usize;
    let last_full_depth_idx = len >> 1;
    while idx < last_full_depth_idx {
        idx = (idx * 2) + 2;
        if heap[idx - 1].cost < heap[idx].cost {
            idx -= 1;
        }
        if last_node.cost <= heap[idx].cost {
            break;
        }
        heap[hole] = heap[idx];
        hole = idx;
    }

    if idx == last_full_depth_idx && (len % 2) == 0 && heap[len - 1].cost < last_node.cost {
        heap[hole] = heap[len - 1];
        hole = len - 1;
    }

    heap[hole] = last_node;
    heap.truncate(len);
    Some(popped)
}

/// `PathReversePathFindNode`: the chain of `next` links, reversed in place.
fn reverse_path_find_node(nodes: &mut [FindNode], head: u32) -> Option<u32> {
    let mut previous = None;
    let mut current = Some(head);
    while let Some(id) = current {
        let next = nodes[id as usize].next;
        nodes[id as usize].next = previous;
        previous = Some(id);
        current = next;
    }
    previous
}

/// `IsPointInTrap`.
fn is_point_in_trap(pos: Pos, trap: &Trapezoid) -> bool {
    if pos.y <= trap.yb || trap.yt <= pos.y {
        return false;
    }

    let mut top = Vec2f {
        x: trap.xtl,
        y: trap.yt,
    };
    let mut bottom = Vec2f {
        x: trap.xbl,
        y: trap.yb,
    };
    let mut point = sub(top, bottom);
    if cross(pos.v2(), point) < 0.0 {
        return false;
    }

    top = Vec2f {
        x: trap.xtr,
        y: trap.yt,
    };
    bottom = Vec2f {
        x: trap.xbr,
        y: trap.yb,
    };
    point = sub(top, bottom);
    if 0.0 < cross(pos.v2(), point) {
        return false;
    }

    true
}

/// `GetClosestPointOnLineSegment`.
///
/// The reference stores `oproj` -- the offset from the projected point back to `pos` -- rather than the
/// projected point itself. Nothing reads the point afterwards, only the distance it travels with, so the
/// quirk is invisible; it is reproduced rather than corrected.
fn closest_point_on_segment(
    pos: Pos,
    p1: Vec2f,
    p2: Vec2f,
    distance: &mut f32,
    closest: &mut Vec2f,
) {
    let p = sub(p2, p1);
    let v = sub(pos.v2(), p1);

    if p.x == 0.0 && p.y == 0.0 {
        let norm = dot(v, v);
        if norm < *distance {
            *distance = norm;
            *closest = p1;
        }
        return;
    }

    let coeff = dot(v, p) / dot(p, p);
    let projection = if (0.0..=1.0).contains(&coeff) {
        Vec2f {
            x: (coeff * p.x) + p1.x,
            y: (coeff * p.y) + p1.y,
        }
    } else if 1.0 <= coeff {
        p2
    } else {
        p1
    };

    let offset = sub(pos.v2(), projection);
    let norm = dot(offset, offset);
    if norm < *distance {
        *distance = norm;
        *closest = offset;
    }
}

/// `DistToTrap`: squared distance to the trapezoid's edges, or zero inside it.
fn dist_to_trap(pos: Pos, trap: &Trapezoid) -> f32 {
    if is_point_in_trap(pos, trap) {
        return 0.0;
    }

    let mut distance = f32::INFINITY;
    let mut point = Vec2f::default();

    let tl = Vec2f {
        x: trap.xtl,
        y: trap.yt,
    };
    let tr = Vec2f {
        x: trap.xtr,
        y: trap.yt,
    };
    let bl = Vec2f {
        x: trap.xbl,
        y: trap.yb,
    };
    let br = Vec2f {
        x: trap.xbr,
        y: trap.yb,
    };
    closest_point_on_segment(pos, tl, tr, &mut distance, &mut point);
    closest_point_on_segment(pos, tr, br, &mut distance, &mut point);
    closest_point_on_segment(pos, br, bl, &mut distance, &mut point);
    closest_point_on_segment(pos, bl, tl, &mut distance, &mut point);

    distance
}

/// `FindPointOnNextTrap`: where the line from the current position to the destination crosses the
/// neighbour's edge at `next_y`, clamped into the shared span.
fn find_point_on_next_trap(
    min_left_x: f32,
    max_right_x: f32,
    next_y: f32,
    src_pos: Vec2f,
    dst_pos: Vec2f,
) -> Vec2f {
    let mut percent = -1.0f32;

    if src_pos.y != dst_pos.y {
        let divisor = path_reciproc(dst_pos.y - src_pos.y);
        percent = (next_y - src_pos.y) * divisor;
    }

    if percent < 0.0 {
        let src_x = clampf(src_pos.x, min_left_x, max_right_x);
        let dst_x = clampf(dst_pos.x, min_left_x, max_right_x);
        // 0.9 and 0.1: the reference spells them as the doubles nearest those floats
        // (`0.8999999761581421f`, `0.1000000238418579f`), which is what these are.
        Vec2f {
            x: (src_x * 0.9) + (dst_x * 0.1),
            y: next_y,
        }
    } else {
        let new_x = ((dst_pos.x - src_pos.x) * percent) + src_pos.x;
        Vec2f {
            x: clampf(new_x, min_left_x, max_right_x),
            y: next_y,
        }
    }
}

/// `FindIntersectionPoint`: `t1`/`t2` such that `start1 + t1*dir1 == start2 + t2*dir2`.
fn find_intersection_point(
    start1: Vec2f,
    dir1: Vec2f,
    start2: Vec2f,
    dir2: Vec2f,
) -> Option<(f32, f32)> {
    let determinant = cross(dir1, dir2);
    if determinant == 0.0 {
        return None;
    }
    let between = sub(start1, start2);
    Some((
        cross(dir2, between) / determinant,
        cross(dir1, between) / determinant,
    ))
}

/// `PickNextPoint`: where a walk crosses the segment from `point1` to `point2`.
fn pick_next_point(point1: Vec2f, point2: Vec2f, cur_pos: Vec2f, dst_pos: Vec2f) -> Vec2f {
    let cur_to_dst = sub(dst_pos, cur_pos);
    let point1_to_point2 = sub(point2, point1);

    if let Some((_, t2)) = find_intersection_point(cur_pos, cur_to_dst, point1, point1_to_point2) {
        if 0.0 <= t2 {
            let t2 = t2.min(1.0);
            return Vec2f {
                x: point1.x + (point1_to_point2.x * t2),
                y: point1.y + (point1_to_point2.y * t2),
            };
        }
    }

    let norm2 = dot(point1_to_point2, point1_to_point2);
    let divisor = path_reciproc(norm2);

    let t1 = clampf(dot(point1, point1_to_point2) * divisor, 0.0, 1.0);
    let t2 = clampf(dot(point2, point1_to_point2) * divisor, 0.0, 1.0);
    let t = (t2 * 0.1) + (t1 * 0.9);
    Vec2f {
        x: point1.x + (point1_to_point2.x * t),
        y: point1.y + (point1_to_point2.y * t),
    }
}

/// `PathVec2fPreciseCross`: a cross product that treats near-parallel as parallel, using unit vectors to
/// decide when the raw product is too small to be meaningful.
fn precise_cross(left: Vec2f, right: Vec2f) -> f32 {
    if left.x == 0.0 && left.y == 0.0 {
        return 0.0;
    }
    if right.x == 0.0 && right.y == 0.0 {
        return 0.0;
    }

    let product = cross(left, right);
    if -0.01 < product && product < 0.01 {
        return 0.0;
    }
    if product <= -1.0 || 1.0 <= product {
        return product;
    }

    let product = cross(vec2f_unit(left), vec2f_unit(right));
    if product <= -0.01 || 0.01 <= product {
        return product;
    }
    0.0
}

/// A path finder over one map's imported graph.
///
/// The node array, the priority queue and the corridor steps are kept between searches, because the
/// reference keeps them in its context: a second search on the same map does not reallocate, and the
/// node array is cleared rather than rebuilt.
///
/// The graph itself is shared (`Arc`) rather than borrowed: a world keeps one finder per map for as long
/// as the map is served, and the graph is the same object for every walk. That is the reference's
/// `srv->map_context` with one difference -- it has one context per server because it runs one server per
/// map, while this hosts several maps in one process.
pub struct PathContext {
    data: std::sync::Arc<PathStaticData>,
    /// Trap id to `(plane id, position in that plane's trapezoids)`. Trap ids are dense and assigned in
    /// plane order by the importer, so this is the reference's `array_at(&nodes, trap->trap_id)` and its
    /// `PathTrapezoid *` in one table.
    index: Vec<(u16, u32)>,
    nodes: Vec<FindNode>,
    prioq: Vec<HeapNode>,
    steps: Vec<BuildStep>,
}

impl PathContext {
    pub fn new(data: std::sync::Arc<PathStaticData>) -> PathContext {
        let mut index = vec![(0u16, 0u32); data.traps_count as usize];
        for plane in &data.planes {
            for (position, trapezoid) in plane.trapezoids.iter().enumerate() {
                if let Some(slot) = index.get_mut(trapezoid.trap_id as usize) {
                    *slot = (plane.plane_id, position as u32);
                }
            }
        }
        PathContext {
            data,
            index,
            nodes: Vec::new(),
            prioq: Vec::new(),
            steps: Vec::new(),
        }
    }

    /// The trapezoid a trap id names.
    fn trap(&self, trap_id: u32) -> Trapezoid {
        let (plane, position) = self.index[trap_id as usize];
        self.data.planes[usize::from(plane)].trapezoids[position as usize]
    }

    /// The plane a trap id lives on.
    fn plane_of(&self, trap_id: u32) -> u16 {
        self.index[trap_id as usize].0
    }

    /// A trapezoid's neighbour field (`Option<usize>`, a position in its own plane) as a trap id.
    fn neighbour(&self, trap_id: u32, local: Option<usize>) -> Option<u32> {
        local.map(|position| {
            let plane = usize::from(self.plane_of(trap_id));
            self.data.planes[plane].trapezoids[position].trap_id
        })
    }

    /// A portal of the plane `plane`.
    fn portal(&self, plane: u16, portal_id: usize) -> Option<Portal> {
        self.data
            .planes
            .get(usize::from(plane))?
            .portals
            .get(portal_id)
            .copied()
    }

    /// The trapezoids a portal leads to, as trap ids: the reference walks `portal->pair->traps` and
    /// dereferences each entry, so an entry the importer left NULL is skipped here rather than crashing.
    fn portal_pair_traps(&self, portal: &Portal) -> Vec<u32> {
        let Some(plane) = self.data.planes.get(usize::from(portal.portal_plane_id)) else {
            return Vec::new();
        };
        let mut traps = Vec::with_capacity(portal.traps_count as usize);
        for offset in 0..portal.traps_count as usize {
            let entry = plane
                .portals_traps
                .get(portal.traps + offset)
                .copied()
                .flatten();
            if let Some(position) = entry {
                traps.push(plane.trapezoids[position].trap_id);
            }
        }
        traps
    }

    /// `PathFindTrapezoid`, as a trap id.
    pub fn find_trapezoid(&self, pos: Pos) -> Option<u32> {
        let (plane, position) = self.data.trapezoid_at(pos.v2(), pos.plane)?;
        Some(self.data.planes[usize::from(plane)].trapezoids[position].trap_id)
    }

    /// `PathAddNode`: record the cost of reaching `trap` and queue it.
    fn add_node(
        &mut self,
        parent: Option<u32>,
        trap: u32,
        pos: Pos,
        current_cost: f32,
        estimated_cost: f32,
    ) {
        let node = &mut self.nodes[trap as usize];
        node.closed = false;
        node.cost_to_node = current_cost;
        node.point.pos = pos;
        node.point.trap = trap;
        node.next = parent;
        heap_push(
            &mut self.prioq,
            HeapNode {
                cost: estimated_cost,
                trap_id: trap,
            },
        );
    }

    /// `PathVisitTrap`: cost a neighbour reached through a crossing point.
    fn visit_trap(
        &mut self,
        curr_node: u32,
        dst_pos: Vec2f,
        neighbour: u32,
        cross_pos: Pos,
        max_cost: f32,
    ) {
        let current = self.nodes[curr_node as usize];
        let distance = vec2f_dist(current.point.pos.v2(), cross_pos.v2());
        let cost_to_trap = current.cost_to_node + distance;

        if max_cost <= cost_to_trap {
            return;
        }

        let trap = self.trap(neighbour);

        let bottom_point = Vec2f {
            x: clampf(dst_pos.x, trap.xbl, trap.xbr),
            y: trap.yb,
        };
        let estimated_dest_bottom = vec2f_dist(bottom_point, dst_pos);

        let top_point = Vec2f {
            x: clampf(dst_pos.x, trap.xtl, trap.xtr),
            y: trap.yt,
        };
        let estimated_dest_top = vec2f_dist(top_point, dst_pos);

        let mut best_estimate = estimated_dest_bottom.min(estimated_dest_top);
        if trap.yb <= dst_pos.y && dst_pos.y <= trap.yt {
            // The reference subtracts `0.009999999776482582`, the double nearest the float 0.01.
            best_estimate -= 0.01;
        }

        let estimated_cost = cost_to_trap + best_estimate;
        self.add_node(
            Some(curr_node),
            neighbour,
            cross_pos,
            cost_to_trap,
            estimated_cost,
        );
    }

    /// `PathVisitAbove`.
    fn visit_above(&mut self, curr_node: u32, dst_pos: Vec2f, neighbour: u32, max_cost: f32) {
        let trap = self.trap(self.nodes[curr_node as usize].point.trap);
        let next = self.trap(neighbour);
        let xl = next.xbl.max(trap.xtl);
        let xr = next.xbr.min(trap.xtr);
        let src = self.nodes[curr_node as usize].point.pos;
        let crossing = find_point_on_next_trap(xl, xr, next.yb, src.v2(), dst_pos);
        self.visit_trap(
            curr_node,
            dst_pos,
            neighbour,
            Pos {
                x: crossing.x,
                y: crossing.y,
                plane: src.plane,
            },
            max_cost,
        );
    }

    /// `PathVisitBellow`.
    fn visit_bellow(&mut self, curr_node: u32, dst_pos: Vec2f, neighbour: u32, max_cost: f32) {
        let trap = self.trap(self.nodes[curr_node as usize].point.trap);
        let next = self.trap(neighbour);
        let xl = next.xtl.max(trap.xbl);
        let xr = next.xtr.min(trap.xbr);
        let src = self.nodes[curr_node as usize].point.pos;
        let crossing = find_point_on_next_trap(xl, xr, next.yt, src.v2(), dst_pos);
        self.visit_trap(
            curr_node,
            dst_pos,
            neighbour,
            Pos {
                x: crossing.x,
                y: crossing.y,
                plane: src.plane,
            },
            max_cost,
        );
    }

    /// `PathVisitPortalLeft` (and, with `left` false, `PathVisitPortalRight`): the trapezoids on the far
    /// side of a portal, entered where the walk's line crosses the portal's own edge.
    fn visit_portal(
        &mut self,
        curr_node: u32,
        dst_pos: Vec2f,
        portal_id: u16,
        left: bool,
        max_cost: f32,
    ) {
        let src = self.nodes[curr_node as usize].point.pos;
        let Some(portal) = self.portal(src.plane, usize::from(portal_id)) else {
            return;
        };
        // The reference dereferences `portal->pair` and asserts it is the mirror; a portal without one is
        // a crash there, so refusing here is the closest thing to parity that does not abort.
        let Some(pair) = portal.pair else {
            return;
        };
        let Some(pair_portal) = self.portal(pair.0, pair.1) else {
            return;
        };
        debug_assert_eq!(pair_portal.portal_plane_id, portal.neighbor_plane_id);
        debug_assert_eq!(pair_portal.neighbor_plane_id, portal.portal_plane_id);

        if (portal.flags & 0x4) != 0 {
            return;
        }

        let traps = self.portal_pair_traps(&pair_portal);
        let curr_trap = self.trap(self.nodes[curr_node as usize].point.trap);

        for portal_trap in traps {
            if self.nodes[portal_trap as usize].closed {
                continue;
            }
            let trap = self.trap(portal_trap);

            // The top point is the higher of the two trapezoids' top edges on the side the portal is on;
            // the bottom point is the lower of the two bottom edges. `left` picks which corner.
            let (top, bottom) = if left {
                let top = if trap.yt <= curr_trap.yt {
                    Vec2f {
                        x: trap.xtr,
                        y: trap.yt,
                    }
                } else {
                    Vec2f {
                        x: curr_trap.xtl,
                        y: curr_trap.yt,
                    }
                };
                let bottom = if curr_trap.yb <= trap.yb {
                    Vec2f {
                        x: trap.xbr,
                        y: trap.yb,
                    }
                } else {
                    Vec2f {
                        x: curr_trap.xbl,
                        y: curr_trap.yb,
                    }
                };
                (top, bottom)
            } else {
                let top = if trap.yt <= curr_trap.yt {
                    Vec2f {
                        x: trap.xtl,
                        y: trap.yt,
                    }
                } else {
                    Vec2f {
                        x: curr_trap.xtr,
                        y: curr_trap.yt,
                    }
                };
                let bottom = if curr_trap.yb <= trap.yb {
                    Vec2f {
                        x: trap.xbl,
                        y: trap.yb,
                    }
                } else {
                    Vec2f {
                        x: curr_trap.xbr,
                        y: curr_trap.yb,
                    }
                };
                (top, bottom)
            };

            if bottom.y <= top.y {
                let crossing = pick_next_point(top, bottom, src.v2(), dst_pos);
                self.visit_trap(
                    curr_node,
                    dst_pos,
                    portal_trap,
                    Pos {
                        x: crossing.x,
                        y: crossing.y,
                        plane: pair_portal.portal_plane_id,
                    },
                    max_cost,
                );
            }
        }
    }

    /// `PathFinding` (`GmPaths.c:944`): the A* itself.
    ///
    /// Returns whether a walk exists; `waypoints` receives the reduced path, which is one waypoint naming
    /// the destination when both positions are in the same trapezoid.
    pub fn path_finding(&mut self, src: Pos, dst: Pos, waypoints: &mut Vec<Waypoint>) -> bool {
        // 10,000 is the ingame constant.
        const MAX_COST: f32 = 10_000.0;

        let Some(src_trap) = self.find_trapezoid(src) else {
            return false;
        };
        let Some(dst_trap) = self.find_trapezoid(dst) else {
            return false;
        };

        if src_trap == dst_trap {
            waypoints.push(Waypoint {
                pos: dst,
                trap_id: src_trap,
            });
            return true;
        }

        self.prioq.clear();
        self.nodes = vec![FindNode::default(); self.data.traps_count as usize];
        self.steps.clear();

        let mut closest_trap: Option<u32> = None;
        let mut closest_distance = f32::INFINITY;

        self.add_node(None, src_trap, src, 0.0, f32::INFINITY);

        while let Some(top) = heap_pop(&mut self.prioq) {
            let curr_node = top.trap_id;
            self.nodes[curr_node as usize].closed = true;

            let curr_trap = self.trap(curr_node);
            if curr_node == dst_trap {
                reverse_path_find_node(&mut self.nodes, curr_node);
                self.create_waypoints(
                    FindPoint {
                        pos: src,
                        trap: src_trap,
                    },
                    FindPoint {
                        pos: dst,
                        trap: dst_trap,
                    },
                    waypoints,
                );
                return true;
            }

            let distance = dist_to_trap(dst, &curr_trap);
            if distance < closest_distance {
                closest_distance = distance;
                closest_trap = Some(curr_node);
            }

            for id in [
                self.neighbour(curr_node, curr_trap.top_left),
                self.neighbour(curr_node, curr_trap.top_right),
            ]
            .into_iter()
            .flatten()
            {
                if !self.nodes[id as usize].closed {
                    self.visit_above(curr_node, dst.v2(), id, MAX_COST);
                }
            }
            for id in [
                self.neighbour(curr_node, curr_trap.bottom_left),
                self.neighbour(curr_node, curr_trap.bottom_right),
            ]
            .into_iter()
            .flatten()
            {
                if !self.nodes[id as usize].closed {
                    self.visit_bellow(curr_node, dst.v2(), id, MAX_COST);
                }
            }

            if curr_trap.portal_left != u16::MAX {
                self.visit_portal(curr_node, dst.v2(), curr_trap.portal_left, true, MAX_COST);
            }
            if curr_trap.portal_right != u16::MAX {
                self.visit_portal(curr_node, dst.v2(), curr_trap.portal_right, false, MAX_COST);
            }
        }

        // Nothing reached the destination's trapezoid: the reference still builds a path to the closest
        // trapezoid it did reach, if the destination is inside that one, and reports failure either way.
        if let Some(trap) = closest_trap {
            if is_point_in_trap(dst, &self.trap(trap)) {
                let closest = FindPoint { pos: dst, trap };
                reverse_path_find_node(&mut self.nodes, trap);
                self.create_waypoints(
                    FindPoint {
                        pos: src,
                        trap: src_trap,
                    },
                    closest,
                    waypoints,
                );
            }
        }

        false
    }

    /// `PathCreateWaypoints` (`GmPaths.c:822`): walk the chain of trapezoids the search chose and reduce
    /// it to waypoints.
    fn create_waypoints(
        &mut self,
        src_point: FindPoint,
        dst_point: FindPoint,
        waypoints: &mut Vec<Waypoint>,
    ) {
        self.steps.clear();

        let mut helper = BuildHelper {
            context: self,
            left_bound: Bound {
                point: src_point,
                step_id: 0,
                vec: Vec2f::default(),
            },
            right_bound: Bound {
                point: src_point,
                step_id: 0,
                vec: Vec2f::default(),
            },
            curr_start_point: src_point,
            current_plane: src_point.pos.plane,
            last_pos: Pos::default(),
            dst_point,
            waypoints,
        };
        helper.run(src_point.trap);
    }
}

/// `PathFindBound` (`GmPaths.c:552`): one side of the corridor being followed, and the step it began at.
#[derive(Debug, Clone, Copy)]
struct Bound {
    point: FindPoint,
    step_id: usize,
    vec: Vec2f,
}

/// `PathBuildHelper` (`GmPaths.c:558`).
struct BuildHelper<'c> {
    context: &'c mut PathContext,
    left_bound: Bound,
    right_bound: Bound,
    curr_start_point: FindPoint,
    current_plane: u16,
    last_pos: Pos,
    dst_point: FindPoint,
    waypoints: &'c mut Vec<Waypoint>,
}

impl BuildHelper<'_> {
    /// The loop of `PathCreateWaypoints`: follow `next` links and decide, per edge, whether the corridor
    /// can continue or has to be cut.
    fn run(&mut self, src_trap: u32) {
        const MAX_LOOP: i32 = 0xBB8;

        let mut curr_node = src_trap;
        let mut loop_count = 0i32;
        loop {
            loop_count += 1;
            assert!(
                loop_count < MAX_LOOP,
                "the reference asserts the walk terminates"
            );

            let curr_trap_id = self.context.nodes[curr_node as usize].point.trap;
            let curr_trap = self.context.trap(curr_trap_id);
            let next_node = self.context.nodes[curr_node as usize].next;

            if next_node.is_none() || curr_trap_id == self.dst_point.trap {
                if self.add_last(&mut curr_node, self.dst_point) {
                    break;
                }
                continue;
            }

            let next = self.context.nodes[next_node.expect("checked above") as usize];
            let next_trap_id = next.point.trap;
            let next_trap = self.context.trap(next_trap_id);

            let top_left = self.context.neighbour(curr_trap_id, curr_trap.top_left);
            let top_right = self.context.neighbour(curr_trap_id, curr_trap.top_right);
            let bottom_left = self.context.neighbour(curr_trap_id, curr_trap.bottom_left);
            let bottom_right = self.context.neighbour(curr_trap_id, curr_trap.bottom_right);

            if Some(next_trap_id) == top_left || Some(next_trap_id) == top_right {
                let left = Vec2f {
                    x: curr_trap.xtl.max(next_trap.xbl),
                    y: curr_trap.yt,
                };
                let right = Vec2f {
                    x: curr_trap.xtr.min(next_trap.xbr),
                    y: curr_trap.yt,
                };
                self.process_next(left, right, next_trap_id, &mut curr_node);
            } else if Some(next_trap_id) == bottom_left || Some(next_trap_id) == bottom_right {
                let left = Vec2f {
                    x: curr_trap.xbl.max(next_trap.xtl),
                    y: curr_trap.yb,
                };
                let right = Vec2f {
                    x: curr_trap.xbr.min(next_trap.xtr),
                    y: curr_trap.yb,
                };
                // Note the swap: going down, the reference passes `right` as the left edge.
                self.process_next(right, left, next_trap_id, &mut curr_node);
            } else if let Some(portal) = self.portal_adjacent(curr_trap.portal_right, next_trap_id)
            {
                let top = if next_trap.yt <= curr_trap.yt {
                    Vec2f {
                        x: next_trap.xtl,
                        y: next_trap.yt,
                    }
                } else {
                    Vec2f {
                        x: curr_trap.xtr,
                        y: curr_trap.yt,
                    }
                };
                let bottom = if curr_trap.yb <= next_trap.yb {
                    Vec2f {
                        x: next_trap.xbl,
                        y: next_trap.yb,
                    }
                } else {
                    Vec2f {
                        x: curr_trap.xbr,
                        y: curr_trap.yb,
                    }
                };

                self.current_plane = portal.neighbor_plane_id;
                self.context.steps.push(BuildStep {
                    pos: top,
                    dir: sub(bottom, top),
                    plane: self.current_plane,
                    next_trap: next_trap_id,
                });
                self.process_next(top, bottom, next_trap_id, &mut curr_node);
            } else if let Some(portal) = self.portal_adjacent(curr_trap.portal_left, next_trap_id) {
                let top = if next_trap.yt <= curr_trap.yt {
                    Vec2f {
                        x: next_trap.xtr,
                        y: next_trap.yt,
                    }
                } else {
                    Vec2f {
                        x: curr_trap.xtl,
                        y: curr_trap.yt,
                    }
                };
                let bottom = if curr_trap.yb <= next_trap.yb {
                    Vec2f {
                        x: next_trap.xbr,
                        y: next_trap.yb,
                    }
                } else {
                    Vec2f {
                        x: curr_trap.xbl,
                        y: curr_trap.yb,
                    }
                };

                self.current_plane = portal.neighbor_plane_id;
                self.context.steps.push(BuildStep {
                    pos: top,
                    dir: sub(bottom, top),
                    plane: self.current_plane,
                    next_trap: next_trap_id,
                });
                self.process_next(bottom, top, next_trap_id, &mut curr_node);
            } else {
                // The reference asserts "shouldn't be possible" here and stops.
                break;
            }
        }
    }

    /// `PathBuildAddWaypointAndReduce`: flush the corridor steps to waypoints, dropping any that repeat.
    fn add_waypoint_and_reduce(&mut self, new_point: FindPoint, from_pos: Vec2f) {
        let steps = std::mem::take(&mut self.context.steps);

        for step in &steps {
            let best_pos = if !is_zero(step.dir) {
                let to_point = sub(new_point.pos.v2(), from_pos);
                if let Some((t, _s)) =
                    find_intersection_point(step.pos, step.dir, from_pos, to_point)
                {
                    if 1.01 < t {
                        add(step.pos, step.dir)
                    } else if t < -0.01 {
                        step.pos
                    } else {
                        Vec2f {
                            x: step.pos.x + (t * step.dir.x),
                            y: step.pos.y + (t * step.dir.y),
                        }
                    }
                } else {
                    // `FindIntersectionPoint` writes nothing when the lines are parallel; the reference then
                    // reads its uninitialised `t` and `s`. That is undefined behaviour and it shows: over the
                    // 96 cases `tools/gen-path-cases.py` writes, exactly one walk has a step parallel to its
                    // own from-to line, and the reference answers it with 11 waypoints on some runs and 12 on
                    // others. Leaving `t` at zero -- this branch -- reproduces the 11-waypoint answer byte
                    // for byte, which is the closest a deterministic port can come.
                    step.pos
                }
            } else {
                step.pos
            };

            if !equal(self.last_pos.v2(), best_pos) || self.last_pos.plane != step.plane {
                self.last_pos = Pos {
                    x: best_pos.x,
                    y: best_pos.y,
                    plane: step.plane,
                };

                if let Some(previous) = self.waypoints.last() {
                    if equal(best_pos, previous.pos.v2()) {
                        self.waypoints.pop();
                    }
                }

                self.waypoints.push(Waypoint {
                    pos: self.last_pos,
                    trap_id: step.next_trap,
                });
            }
        }

        if !equal(self.last_pos.v2(), new_point.pos.v2()) {
            self.last_pos = new_point.pos;

            if let Some(previous) = self.waypoints.last() {
                if equal(new_point.pos.v2(), previous.pos.v2()) {
                    self.waypoints.pop();
                }
            }

            self.waypoints.push(Waypoint {
                pos: new_point.pos,
                trap_id: new_point.trap,
            });
        }
    }

    /// `PathBuildAddWaypoint`: the corridor has to be cut at one of the two bounds.
    fn add_waypoint(&mut self, left: Vec2f, right: Vec2f, node: &mut u32) {
        let bound_used;
        if equal(self.left_bound.point.pos.v2(), left) {
            bound_used = self.left_bound;
            self.right_bound.point = self.left_bound.point;
            self.right_bound.step_id = self.left_bound.step_id;
            *node = self.left_bound.point.trap;
        } else if equal(self.right_bound.point.pos.v2(), right) {
            bound_used = self.right_bound;
            self.left_bound.point = self.right_bound.point;
            self.left_bound.step_id = self.right_bound.step_id;
            *node = self.right_bound.point.trap;
        } else {
            let mut dx = left.x - self.curr_start_point.pos.x;
            let mut dy = left.y - self.curr_start_point.pos.y;
            let left_value = (dy * dy) + (dx * dx);

            dx = right.x - self.curr_start_point.pos.x;
            dy = right.y - self.curr_start_point.pos.y;
            let right_value = (dy * dy) + (dx * dx);

            bound_used = if left_value < right_value {
                self.left_bound
            } else {
                self.right_bound
            };
        }

        self.context.steps.truncate(bound_used.step_id);

        let from_pos = self.curr_start_point.pos.v2();
        self.add_waypoint_and_reduce(bound_used.point, from_pos);
        self.curr_start_point = bound_used.point;
        self.right_bound.vec = Vec2f::default();
        self.left_bound.vec = Vec2f::default();
        self.current_plane = bound_used.point.pos.plane;
    }

    /// `PathBuildProcessNext`: extend the corridor across the edge into `next_trap`, or cut it.
    fn process_next(&mut self, left: Vec2f, right: Vec2f, next_trap: u32, node: &mut u32) {
        let mut to_left = sub(left, self.curr_start_point.pos.v2());
        let mut to_right = sub(right, self.curr_start_point.pos.v2());

        let new_left_bound;
        if precise_cross(to_left, self.left_bound.vec) < 0.0 {
            // `to_left` is left of the left bound, so the bound is tighter and stays.
            to_left = self.left_bound.vec;
            new_left_bound = self.left_bound;
        } else {
            new_left_bound = Bound {
                vec: to_left,
                point: FindPoint {
                    pos: Pos {
                        x: left.x,
                        y: left.y,
                        plane: self.current_plane,
                    },
                    trap: next_trap,
                },
                step_id: self.context.steps.len(),
            };
        }

        let new_right_bound;
        if 0.0 < precise_cross(to_right, self.right_bound.vec) {
            to_right = self.right_bound.vec;
            new_right_bound = self.right_bound;
        } else {
            new_right_bound = Bound {
                vec: to_right,
                point: FindPoint {
                    pos: Pos {
                        x: right.x,
                        y: right.y,
                        plane: self.current_plane,
                    },
                    trap: next_trap,
                },
                step_id: self.context.steps.len(),
            };
        }

        if is_zero(self.left_bound.vec) || precise_cross(to_left, to_right) <= 0.0 {
            self.left_bound = new_left_bound;
            self.right_bound = new_right_bound;
            *node = self.context.nodes[*node as usize]
                .next
                .expect("the walk follows the chain it was given");
        } else {
            let left = new_left_bound.point.pos.v2();
            let right = new_right_bound.point.pos.v2();
            self.add_waypoint(left, right, node);
        }
    }

    /// `PathAddLast`: the chain has run out; close the corridor at the destination.
    fn add_last(&mut self, node: &mut u32, new_point: FindPoint) -> bool {
        let to_new_point = sub(new_point.pos.v2(), self.curr_start_point.pos.v2());
        if 0.0 <= precise_cross(to_new_point, self.left_bound.vec) {
            if precise_cross(to_new_point, self.right_bound.vec) <= 0.0 {
                let from_pos = self.curr_start_point.pos.v2();
                self.add_waypoint_and_reduce(new_point, from_pos);
                return true;
            }

            self.right_bound.vec = Vec2f::default();
            self.left_bound.point = self.right_bound.point;
            *node = self.right_bound.point.trap;
            self.context.steps.truncate(self.right_bound.step_id);
            let from_pos = self.curr_start_point.pos.v2();
            let point = self.right_bound.point;
            self.add_waypoint_and_reduce(point, from_pos);
            self.curr_start_point = self.right_bound.point;
            self.current_plane = self.right_bound.point.pos.plane;
            false
        } else {
            self.left_bound.vec = Vec2f::default();
            self.right_bound.point = self.left_bound.point;
            *node = self.left_bound.point.trap;
            self.context.steps.truncate(self.left_bound.step_id);
            let from_pos = self.curr_start_point.pos.v2();
            let point = self.left_bound.point;
            self.add_waypoint_and_reduce(point, from_pos);
            self.curr_start_point = self.left_bound.point;
            self.current_plane = self.left_bound.point.pos.plane;
            false
        }
    }

    /// `GetPortalAndCheckTrapAdjacent`: the portal of the current plane on the given side, if it leads to
    /// `trap`.
    fn portal_adjacent(&self, portal_id: u16, trap: u32) -> Option<Portal> {
        if portal_id == 0xFFFF {
            return None;
        }
        let portal = self
            .context
            .portal(self.current_plane, usize::from(portal_id))?;
        let pair = portal.pair?;
        let pair_portal = self.context.portal(pair.0, pair.1)?;
        for candidate in self.context.portal_pair_traps(&pair_portal) {
            if candidate == trap {
                return Some(portal);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(cost: f32, trap_id: u32) -> HeapNode {
        HeapNode { cost, trap_id }
    }

    #[test]
    fn the_heap_pops_in_cost_order_and_keeps_every_entry() {
        // The search's tie-breaking depends on this heap's exact sift, so the property to hold onto is the
        // contract: whatever order entries go in, they come out cheapest first and none is lost.
        let mut heap: Vec<HeapNode> = Vec::new();
        let mut pushed = Vec::new();
        for (index, cost) in [8.0f32, 3.0, 10.0, 1.0, 5.0, 5.0, 2.0, 9.0, 0.5, 7.0]
            .into_iter()
            .enumerate()
        {
            let entry = node(cost, index as u32);
            heap_push(&mut heap, entry);
            pushed.push(entry);
        }
        assert_eq!(heap.len(), pushed.len(), "every entry is in the queue");

        let mut popped = Vec::new();
        while let Some(entry) = heap_pop(&mut heap) {
            popped.push(entry);
        }
        assert!(heap.is_empty(), "the queue drains");
        assert_eq!(popped.len(), pushed.len(), "and loses nothing on the way");

        for pair in popped.windows(2) {
            assert!(
                pair[0].cost <= pair[1].cost,
                "{} came out before {}",
                pair[0].cost,
                pair[1].cost
            );
        }
        let mut seen: Vec<u32> = popped.iter().map(|entry| entry.trap_id).collect();
        seen.sort_unstable();
        let expected: Vec<u32> = (0..pushed.len() as u32).collect();
        assert_eq!(seen, expected, "each entry comes out exactly once");
    }

    #[test]
    fn an_empty_heap_pops_nothing_and_a_single_entry_comes_back_out() {
        let mut heap: Vec<HeapNode> = Vec::new();
        assert!(heap_pop(&mut heap).is_none());
        heap_push(&mut heap, node(4.0, 7));
        let popped = heap_pop(&mut heap).expect("the one entry");
        assert_eq!(popped.trap_id, 7);
        assert_eq!(popped.cost, 4.0);
        assert!(heap_pop(&mut heap).is_none());
    }

    #[test]
    fn reversing_the_chain_turns_the_path_around() {
        // `PathReversePathFindNode`: three nodes linked 0 -> 1 -> 2 become 2 -> 1 -> 0, and the head is
        // whatever used to be the tail.
        let mut nodes = vec![FindNode::default(); 3];
        nodes[0].next = Some(1);
        nodes[1].next = Some(2);
        nodes[2].next = None;

        let head = reverse_path_find_node(&mut nodes, 0);
        assert_eq!(head, Some(2), "the old tail is the new head");
        assert_eq!(nodes[2].next, Some(1));
        assert_eq!(nodes[1].next, Some(0));
        assert_eq!(nodes[0].next, None);
    }

    #[test]
    fn a_trapezoid_contains_what_the_walk_says_it_contains() {
        // `IsPointInTrap` as the reference writes it -- and it is not the textbook test. The height check
        // is ordinary, but the two side checks take the cross product of the *absolute position* with the
        // edge vector, where a correct test would use the position relative to the edge. The region that
        // accepts is therefore a wedge from the map origin, not the trapezoid's own interior, and the
        // reference reads it that way in `DistToTrap` and in `PathFinding`'s last-resort path. Kept,
        // because those two callers are what a walk's refusal depends on.
        //
        // For this trapezoid (top edge -10..10 at y = 10, bottom edge -5..5 at y = 0) the accepted region
        // works out as 0 < y < 10 and |2x| <= y.
        let trap = Trapezoid {
            trap_id: 0,
            yt: 10.0,
            yb: 0.0,
            xtl: -10.0,
            xtr: 10.0,
            xbl: -5.0,
            xbr: 5.0,
            ..Trapezoid::default()
        };
        let inside = |x: f32, y: f32| is_point_in_trap(Pos { x, y, plane: 0 }, &trap);

        assert!(inside(0.0, 5.0), "the middle of the wedge");
        assert!(inside(4.0, 9.0), "and its wide end");
        assert!(!inside(5.0, 9.0), "but not past the right side's line");
        assert!(inside(-4.0, 9.0), "and the wedge is symmetric about x = 0");
        assert!(!inside(-5.0, 9.0), "nor past the left one");
        assert!(!inside(0.0, 10.0), "the top edge itself is not inside");
        assert!(!inside(0.0, 0.0), "nor the bottom edge");
        assert!(!inside(0.0, -1.0), "and nothing below it");
    }
}
