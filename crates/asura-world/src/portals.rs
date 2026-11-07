//! The travel portals of each instance, read only from the selected pack's portals.json files.
//!
//! A map's doorways are content, one file per map folder (`config/World/<pack>/maps/.../<map
//! id>/portals.json`, `docs/38-layer-decisions.md` D39): an **arch** is a wall the character crosses,
//! stated as its two endpoints and the map it leads to; an **arrival** states where a character
//! arriving in this map **from** a named map stands, and which way it faces. The target map owns its
//! arrivals, so the map that receives a traveller decides where that traveller lands.
//!
//! The file shape, as the owner wrote it (2026-11-05):
//!
//! ```json
//! { "map_id": 148,
//!   "arches":   [ { "idx": 0, "to_map": 146, "from_x": 7550.0, "from_y": 55123.0,
//!                   "to_x": 7060.0, "to_y": 5621.0 } ],
//!   "arrivals": [ { "from": 146, "x": 7755.18, "y": 5887.82, "angle": 0.57 } ] }
//! ```
//!
//! A point-shaped arch (`x`/`y` and no `from_x`/`from_y`) is the older shape the archive and the
//! database carry: it has no wall, so a step counts as crossing it when it intersects a box edge at [`point_half_extent`] of
//! the point. A wall-shaped arch counts a crossing only when the step passes through the segment, which
//! is the owner's rule: "the portal is an invisible wall we cross ... should be a plane".
//!
//! **Two facts a reader must keep in mind.** Several instances share one geometry blob, so the same portal
//! coordinates appear under 146, 148 and 164 alike: a position says where a doorway is, not which instance
//! the character is in. And `to_instance` is filled only where a crossing was measured, because the
//! destination belongs to the zone - so a row with none must not fire.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Half the side of a single-coordinate doorway box, from the server configuration.
pub fn point_half_extent() -> f32 {
    asura_config::portal_box_half_extent()
}

/// Whether actual movement intersects a finite edge of a point doorway's box.
fn box_crossed(point: (f32, f32), from: (f32, f32), to: (f32, f32)) -> bool {
    let extent = point_half_extent();
    if extent <= 0.0 || from == to {
        return false;
    }
    let (left, right) = (point.0 - extent, point.0 + extent);
    let (bottom, top) = (point.1 - extent, point.1 + extent);
    // Test each finite edge. A report wholly inside the box crosses no boundary.
    for x in [left, right] {
        if from.0 != to.0 {
            let t = (x - from.0) / (to.0 - from.0);
            let y = from.1 + t * (to.1 - from.1);
            if (0.0..=1.0).contains(&t) && (bottom..=top).contains(&y) {
                return true;
            }
        }
    }
    for y in [bottom, top] {
        if from.1 != to.1 {
            let t = (y - from.1) / (to.1 - from.1);
            let x = from.0 + t * (to.0 - from.0);
            if (0.0..=1.0).contains(&t) && (left..=right).contains(&x) {
                return true;
            }
        }
    }
    false
}

// ---- the pack's doorways: a wall to cross, and the arrival the target map owns -------------------

/// The folder inside a pack that holds one folder per map (the same tree `map_spawns` walks).
pub const MAPS: &str = "maps";

/// The file inside a map folder that holds that map's doorways and arrivals.
pub const PORTALS_FILE: &str = "portals.json";

/// How close a written arrival point has to be to a saved position to be the same arrival, in the map's
/// own units. The position written for a crossing comes from the same entry, so this only absorbs the
/// float round trip through the database.
pub const ARRIVAL_MATCH: f32 = 1.0;

/// One arch of a map: the wall a character crosses, and the map it leads to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arch {
    /// The instance the wall stands in.
    pub from_instance: i32,
    /// The arch's own index in the map's list, as the file states it.
    pub idx: u32,
    /// The map the crossing leads to, or `None` when the file names none.
    pub to_instance: Option<i32>,
    /// The wall's first endpoint. Equal to [`Arch::to`] when the file states a point.
    pub from: (f32, f32),
    /// The wall's second endpoint.
    pub to: (f32, f32),
}

impl Arch {
    /// Whether this arch is a wall (two distinct endpoints) rather than a point.
    pub fn is_wall(&self) -> bool {
        self.from != self.to
    }

    /// Whether the step `step_from` -> `step_to` crosses this arch.
    ///
    /// Movement touching or crossing the finite wall starts travel, including its endpoints.
    /// A point doorway uses the edges of its configured box.
    pub fn crossed_by(&self, step_from: (f32, f32), step_to: (f32, f32)) -> bool {
        if !self.is_wall() {
            return box_crossed(self.from, step_from, step_to);
        }
        if step_from == step_to {
            return false;
        }
        // Bounding boxes also reject disjoint collinear segments. Non-strict side checks include
        // a step ending on the wall, an endpoint touch, and movement along the wall itself.
        self.from.0.min(self.to.0) <= step_from.0.max(step_to.0)
            && step_from.0.min(step_to.0) <= self.from.0.max(self.to.0)
            && self.from.1.min(self.to.1) <= step_from.1.max(step_to.1)
            && step_from.1.min(step_to.1) <= self.from.1.max(self.to.1)
            && side(self.from, self.to, step_from) * side(self.from, self.to, step_to) <= 0.0
            && side(step_from, step_to, self.from) * side(step_from, step_to, self.to) <= 0.0
    }
}

/// The signed area of the triangle `a`-`b`-`c`: positive on one side of `a`-`b`, negative on the other.
fn side(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Where a character arriving in one map from another stands, as the receiving map states it.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub struct Arrival {
    /// The map the character is arriving **from**.
    pub from: i32,
    /// The coordinate it stands at.
    pub x: f32,
    /// The coordinate that goes with it.
    pub y: f32,
    /// The facing, in radians, when the file states one.
    pub angle: Option<f32>,
}

impl Arrival {
    /// The direction vector the agent is created facing: `(cos, sin)`, or the default `(1, 0)`.
    pub fn direction(&self) -> (f32, f32) {
        match self.angle {
            Some(angle) => (angle.cos(), angle.sin()),
            None => (1.0, 0.0),
        }
    }
}

/// The doorway a step crossed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crossed {
    /// The map the crossing leads to, when the data names one. A crossing with none does not fire.
    pub to_instance: Option<i32>,
    /// The instance the wall stands in.
    pub from_instance: i32,
}

/// One map's `portals.json`, as read: its arches and its arrivals.
#[derive(Debug, Clone, PartialEq)]
pub struct MapPortals {
    /// The walls of this map.
    pub arches: Vec<Arch>,
    /// Where a character arriving in this map from another one stands.
    pub arrivals: Vec<Arrival>,
}

impl MapPortals {
    /// The arrival this map states for a character coming from `from`.
    pub fn arrival_for(&self, from: i32) -> Option<&Arrival> {
        self.arrivals.iter().find(|arrival| arrival.from == from)
    }

    /// The arrival whose point is `position` (within [`ARRIVAL_MATCH`]), if this map states one.
    pub fn arrival_at(&self, position: (f32, f32)) -> Option<&Arrival> {
        self.arrivals.iter().find(|arrival| {
            (arrival.x - position.0).abs() <= ARRIVAL_MATCH
                && (arrival.y - position.1).abs() <= ARRIVAL_MATCH
        })
    }
}

/// One arch as a `portals.json` states it: a wall (`from_x`/`from_y` -> `to_x`/`to_y`) or a point
/// (`x`/`y`), in the older shape the archive's dump wrote.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct ArchEntry {
    #[serde(default)]
    idx: u32,
    #[serde(default)]
    to_map: Option<i32>,
    #[serde(default)]
    from_x: Option<f32>,
    #[serde(default)]
    from_y: Option<f32>,
    #[serde(default)]
    to_x: Option<f32>,
    #[serde(default)]
    to_y: Option<f32>,
    #[serde(default)]
    x: Option<f32>,
    #[serde(default)]
    y: Option<f32>,
}

/// The file, as written.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct PortalsFile {
    map_id: i32,
    #[serde(default)]
    arches: Vec<ArchEntry>,
    #[serde(default)]
    arrivals: Vec<Arrival>,
}

/// Every map's doorways under `pack`, keyed by map id.
///
/// The walk is the same one `map_spawns` uses: only a folder named after a map id is a map. A folder
/// without `portals.json` has no doorways. A file
/// that does not parse, or a `map_id` that disagrees with its folder, is reported and skipped.
pub fn walk(pack: &Path) -> (HashMap<i32, MapPortals>, Vec<String>) {
    let mut maps: HashMap<i32, MapPortals> = HashMap::new();
    let mut notes: Vec<String> = Vec::new();
    let root = pack.join(MAPS);
    if !root.is_dir() {
        notes.push(format!("{} is not readable", root.display()));
    } else {
        visit(&root, &mut maps, &mut notes);
    }
    (maps, notes)
}

/// Read one map folder, or walk into a folder that is a level of the tree.
fn visit(dir: &Path, maps: &mut HashMap<i32, MapPortals>, notes: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        notes.push(format!("{} is not readable", dir.display()));
        return;
    };
    for entry in entries.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        let name = folder
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let Ok(folder_id) = name.parse::<i32>() else {
            visit(&folder, maps, notes);
            continue;
        };
        let file = folder.join(PORTALS_FILE);
        if !file.exists() {
            continue;
        }
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(err) => {
                notes.push(format!("{}: {err}", file.display()));
                continue;
            }
        };
        let parsed: PortalsFile = match serde_json::from_str(&text) {
            Ok(parsed) => parsed,
            Err(err) => {
                notes.push(format!("{}: {err}", file.display()));
                continue;
            }
        };
        if folder_id != parsed.map_id {
            notes.push(format!(
                "{}: folder is {folder_id}, file says {}; skipped",
                file.display(),
                parsed.map_id
            ));
            continue;
        }
        let mut arches = Vec::with_capacity(parsed.arches.len());
        for entry in parsed.arches {
            match (entry.from_x, entry.from_y, entry.to_x, entry.to_y, entry.x, entry.y) {
                (Some(fx), Some(fy), Some(tx), Some(ty), _, _) => arches.push(Arch {
                    from_instance: folder_id,
                    idx: entry.idx,
                    to_instance: entry.to_map,
                    from: (fx, fy),
                    to: (tx, ty),
                }),
                (_, _, _, _, Some(x), Some(y)) => arches.push(Arch {
                    from_instance: folder_id,
                    idx: entry.idx,
                    to_instance: entry.to_map,
                    from: (x, y),
                    to: (x, y),
                }),
                _ => notes.push(format!(
                    "{}: arch {} states neither a wall (from_x/from_y/to_x/to_y) nor a point (x/y); \
                     skipped",
                    file.display(),
                    entry.idx
                )),
            }
        }
        let arrivals = parsed.arrivals;
        maps.insert(folder_id, MapPortals { arches, arrivals });
    }
}

/// The pack a world reads: `ASURA_WORLD_PACK`, or the default (`crate::spawns::pack`).
pub fn pack() -> PathBuf {
    crate::spawns::pack()
}

/// Every map's doorways, read once per process.
pub fn table_pack() -> &'static HashMap<i32, MapPortals> {
    static TABLE: std::sync::OnceLock<HashMap<i32, MapPortals>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let pack = pack();
        let (maps, notes) = walk(&pack);
        for note in &notes {
            eprintln!("world: doorways: {note}");
        }
        let arches: usize = maps.values().map(|map| map.arches.len()).sum();
        let arrivals: usize = maps.values().map(|map| map.arrivals.len()).sum();
        eprintln!(
            "world: doorways from {}: {} map(s), {} arch(es), {} arrival(s)",
            pack.display(),
            maps.len(),
            arches,
            arrivals
        );
        maps
    })
}

/// The pack's doorways for one instance, when the pack states any.
pub fn pack_arches(from_instance: i32) -> Option<&'static [Arch]> {
    table_pack()
        .get(&from_instance)
        .map(|map| map.arches.as_slice())
        .filter(|arches| !arches.is_empty())
}

/// The doorway the step `from` -> `to` crosses in `from_instance`.
///
/// **A map the pack describes is judged by its own walls, and nothing else.** Its `portals.json` is
/// that map's doorway table, so a step that crosses none of its arches crosses nothing - even if it
/// passes near a measured crossing point. No other table supplies a missing doorway.
pub fn crossed(from_instance: i32, from: (f32, f32), to: (f32, f32)) -> Option<Crossed> {
    pack_arches(from_instance)?
        .iter()
        .find(|arch| arch.crossed_by(from, to))
        .map(|arch| Crossed {
            to_instance: arch.to_instance,
            from_instance,
        })
}

/// Where a character arriving in `in_instance` from `from_instance` stands, when the data states it.
///
/// The receiving map owns this fact. A missing arrival remains unknown.
pub fn arrival(in_instance: i32, from_instance: i32) -> Option<Arrival> {
    table_pack()
        .get(&in_instance)?
        .arrival_for(from_instance)
        .copied()
}

/// The arrival in `in_instance` whose point is `position`, within [`ARRIVAL_MATCH`].
///
/// A crossing writes the destination's arrival point as the character's saved position before the
/// redirect; the connection that then enters the map reads it back here, which is what carries the
/// arrival's **facing** across the reconnect (the position is already in the saved row).
pub fn arrival_at(in_instance: i32, position: (f32, f32)) -> Option<Arrival> {
    table_pack()
        .get(&in_instance)?
        .arrival_at(position)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall() -> Arch {
        // A wall along y = 0 from x = 0 to x = 10.
        Arch {
            from_instance: 1,
            idx: 0,
            to_instance: Some(2),
            from: (0.0, 0.0),
            to: (10.0, 0.0),
        }
    }

    #[test]
    fn a_wall_triggers_on_crossing_or_touching() {
        assert!(
            wall().crossed_by((-1.0, -5.0), (5.0, 5.0)),
            "a step from one side to the other, inside the wall's length, crosses it"
        );
        assert!(
            !wall().crossed_by((-5.0, -1.0), (-5.0, 1.0)),
            "a step that crosses the wall's line beyond its endpoint does not pass through the wall"
        );
        assert!(
            !wall().crossed_by((-2.0, 3.0), (8.0, 3.0)),
            "a step beside the wall on the same side does not cross it"
        );
        assert!(
            wall().crossed_by((10.0, -1.0), (10.0, 1.0)),
            "touching the wall at its endpoint starts travel"
        );
        assert!(
            wall().crossed_by((5.0, -1.0), (5.0, 0.0)),
            "landing on the wall triggers"
        );
        assert!(
            !wall().crossed_by((-5.0, 0.0), (-1.0, 0.0)),
            "disjoint collinear segments miss"
        );
    }

    #[test]
    fn the_failed_ascalon_reports_cross_the_authored_wall() {
        // logs/world-8001.log:410655 and :410664: accepted reports crossed the wall, but the
        // tick-advanced starting position lost the segment before it reached the doorway check.
        let arch = table_pack().get(&148).unwrap().arches[0];
        assert!(arch.crossed_by((7277.4, 5417.9), (7184.3, 5307.3)));
    }

    #[test]
    fn a_point_doorway_uses_box_edges() {
        let point = Arch {
            from: (0.0, 0.0),
            to: (0.0, 0.0),
            ..wall()
        };
        assert!(
            point.crossed_by((100.0, -400.0), (100.0, 400.0)),
            "a step through the box crosses its edges"
        );
        assert!(
            !point.crossed_by((0.0, 0.0), (10.0, 10.0)),
            "inside is not a crossing"
        );
        assert!(
            !point.crossed_by((400.0, -200.0), (400.0, 200.0)),
            "400 units away misses the box"
        );
    }

    #[test]
    fn the_committed_pack_states_a_wall_and_an_arrival_for_ascalon_city() {
        let map = table_pack()
            .get(&148)
            .expect("Ascalon City (pre-Searing) has a portals.json");
        let arch = map
            .arches
            .iter()
            .find(|arch| arch.to_instance == Some(146))
            .expect("the arch to Lakeside County");
        assert!(
            arch.is_wall(),
            "the owner stated the arch as a wall: {arch:?}"
        );
        let arrival = arrival(148, 146).expect("where a character arriving from 146 stands");
        assert_eq!(arrival.from, 146);
        let docked = arrival_at(148, (arrival.x, arrival.y)).expect("the same point reads back");
        assert_eq!((docked.x, docked.y), (arrival.x, arrival.y));
        assert_eq!(docked.angle, arrival.angle, "the facing is the arrival's");
    }

    #[test]
    fn a_step_far_from_every_doorway_crosses_nothing() {
        // A step nowhere near a doorway fires no crossing, whichever table answers for the map.
        assert!(
            crossed(449, (0.0, 0.0), (1.0, 0.0)).is_none(),
            "a step far from every doorway crosses nothing"
        );
    }

    #[test]
    fn a_map_the_pack_describes_is_judged_by_its_own_walls() {
        // 146's own wall to another map, and a step built to pass through it (the file's coordinates
        // decide the direction, so editing the wall keeps this test honest). Nothing about the measured
        // crossing table is consulted for a map the pack describes.
        let arches = pack_arches(146).expect("146 has a portals.json");
        let wall = arches
            .iter()
            .find(|arch| arch.is_wall())
            .expect("146 draws a wall");
        let midpoint = (
            (wall.from.0 + wall.to.0) / 2.0,
            (wall.from.1 + wall.to.1) / 2.0,
        );
        let (dx, dy) = (wall.to.0 - wall.from.0, wall.to.1 - wall.from.1);
        let norm = (dx * dx + dy * dy).sqrt();
        let (nx, ny) = (-dy / norm, dx / norm);
        let step_from = (midpoint.0 + 50.0 * nx, midpoint.1 + 50.0 * ny);
        let step_to = (midpoint.0 - 50.0 * nx, midpoint.1 - 50.0 * ny);
        let crossed = crossed(146, step_from, step_to).expect("a step through the wall crosses it");
        assert_eq!(crossed.to_instance, wall.to_instance);
    }
}
