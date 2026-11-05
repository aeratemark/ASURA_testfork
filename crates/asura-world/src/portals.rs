//! The travel portals of each instance: the pack's `portals.json` files first, the measured tables
//! after that.
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
//! database carry: it has no wall, so a step counts as crossing it when it passes within [`REACH`] of
//! the point. A wall-shaped arch counts a crossing only when the step passes through the segment, which
//! is the owner's rule: "the portal is an invisible wall we cross ... should be a plane".
//!
//! **Two facts a reader must keep in mind.** Several instances share one geometry blob, so the same portal
//! coordinates appear under 146, 148 and 164 alike: a position says where a doorway is, not which instance
//! the character is in. And `to_instance` is filled only where a crossing was measured, because the
//! destination belongs to the zone - so a row with none must not fire.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One travel portal: where it stands, and which instance it leads to when that is known.
#[derive(Debug, Clone, Copy)]
pub struct Portal {
    /// The instance the doorway is in.
    pub from_instance: i32,
    pub x: f32,
    pub y: f32,
    /// The instance it leads to, or `None` when no crossing has been measured there yet.
    pub to_instance: Option<i32>,
    /// Where the live server put the character down on the other side of this door, or `None` when no
    /// crossing of this door was recorded. Per door, not per instance: two doors into one map can land in
    /// different places, and the recording has one landing point per crossing.
    pub arrive: Option<(f32, f32)>,
}

/// How close a character has to be for a doorway to count as reached, in the map's own units.
///
/// A starting value, measured against nothing: the doorway prop is a few metres wide and a reported
/// position is the body's centre, so the first walk with the trace on is what corrects this number.
pub const REACH: f32 = 250.0;

/// The doorways recorded from live crossings: where the character stood when the server changed the map,
/// and the instance it woke up in. These are measured, not derived - the position is the crossing point
/// itself (recording `doors-20261031-095754.jsonl`, 22 of its 28 crossings; the ones omitted had
/// inherited the previous instance's last position report).
#[rustfmt::skip]
pub const CROSSED: &[Portal] = &[
    Portal { from_instance: 146, x: 7198.0, y: 5177.0, to_instance: Some(148), arrive: Some((7739.0, 5900.0)) },
    Portal { from_instance: 148, x: 7401.0, y: 5543.0, to_instance: Some(146), arrive: Some((6602.0, 4485.0)) },
    Portal { from_instance: 146, x: -5499.0, y: 13795.0, to_instance: Some(147), arrive: Some((-11574.0, -16170.0)) },
    Portal { from_instance: 147, x: -11635.0, y: -16776.0, to_instance: Some(146), arrive: Some((-5434.0, 11799.0)) },
    Portal { from_instance: 146, x: -14124.0, y: 10037.0, to_instance: Some(160), arrive: Some((21932.0, 12966.0)) },
    Portal { from_instance: 160, x: 21932.0, y: 12966.0, to_instance: Some(146), arrive: Some((-13891.0, 10107.0)) },
    Portal { from_instance: 163, x: -7442.0, y: 1442.0, to_instance: Some(160), arrive: Some((-7459.0, 1458.0)) },
    Portal { from_instance: 160, x: -7315.0, y: 1468.0, to_instance: Some(163), arrive: Some((-6940.0, 1430.0)) },
    Portal { from_instance: 164, x: -11595.0, y: -6336.0, to_instance: Some(146), arrive: Some((-10853.0, -6221.0)) },
    Portal { from_instance: 146, x: -10962.0, y: -6225.0, to_instance: Some(164), arrive: Some((-11672.0, -6209.0)) },
    Portal { from_instance: 164, x: -13754.0, y: -7076.0, to_instance: Some(145), arrive: Some((14442.0, 2135.0)) },
    Portal { from_instance: 145, x: 14442.0, y: 2135.0, to_instance: Some(164), arrive: Some((-13151.0, -7080.0)) },
    Portal { from_instance: 164, x: -11716.0, y: -6244.0, to_instance: Some(146), arrive: Some((-10853.0, -6221.0)) },
    Portal { from_instance: 146, x: -13460.0, y: -19979.0, to_instance: Some(161), arrive: Some((9449.0, 19702.0)) },
    Portal { from_instance: 161, x: 10288.0, y: 19795.0, to_instance: Some(146), arrive: Some((-12914.0, -20062.0)) },
    Portal { from_instance: 165, x: 223.0, y: 8069.0, to_instance: Some(161), arrive: Some((633.0, 7270.0)) },
    Portal { from_instance: 161, x: 462.0, y: 7495.0, to_instance: Some(165), arrive: Some((159.0, 8456.0)) },
    Portal { from_instance: 163, x: -7339.0, y: 1458.0, to_instance: Some(160), arrive: Some((-7459.0, 1458.0)) },
    Portal { from_instance: 160, x: -6894.0, y: -16820.0, to_instance: Some(161), arrive: Some((-19103.0, 19512.0)) },
    Portal { from_instance: 161, x: -19064.0, y: 19796.0, to_instance: Some(160), arrive: Some((-6742.0, -16419.0)) },
    Portal { from_instance: 166, x: 22550.0, y: 7410.0, to_instance: Some(162), arrive: Some((22614.0, 6849.0)) },
    Portal { from_instance: 162, x: 22623.0, y: 6993.0, to_instance: Some(166), arrive: Some((22798.0, 10205.0)) },
];

/// Every measured portal, one row per doorway of each instance that has one.
///
/// `to_instance` is filled only where a crossing was measured: today that is Ascalon City's door to
/// Lakeside County, `(7378, 5429)` in instance 148 to 146, from a live walk recorded on 2026-10-31. The
/// coordinates in the log's "is standing in a travel portal" line are what the next row comes from, and a
/// doorway without one deliberately does nothing rather than guess a destination.
#[rustfmt::skip]
pub const PORTALS: &[Portal] = &[
    // instance 145
    Portal { from_instance: 145, x: 16823.0, y: -10892.0, to_instance: None, arrive: None },
    Portal { from_instance: 145, x: 19411.0, y: 2990.0, to_instance: None, arrive: None },
    Portal { from_instance: 145, x: 5961.0, y: -22656.0, to_instance: None, arrive: None },
    // instance 146
    Portal { from_instance: 146, x: 7378.0, y: 5429.0, to_instance: None, arrive: None },
    Portal { from_instance: 146, x: 4384.0, y: -19844.0, to_instance: None, arrive: None },
    Portal { from_instance: 146, x: -13897.0, y: -20108.0, to_instance: None, arrive: None },
    Portal { from_instance: 146, x: -5488.0, y: 13494.0, to_instance: None, arrive: None },
    Portal { from_instance: 146, x: -14540.0, y: 10054.0, to_instance: None, arrive: None },
    Portal { from_instance: 146, x: -11309.0, y: -6226.0, to_instance: None, arrive: None },
    // instance 147
    Portal { from_instance: 147, x: 26113.0, y: 4767.0, to_instance: None, arrive: None },
    Portal { from_instance: 147, x: -11632.0, y: -17226.0, to_instance: None, arrive: None },
    Portal { from_instance: 147, x: -20684.0, y: -20666.0, to_instance: None, arrive: None },
    // instance 148
    Portal { from_instance: 148, x: 7378.0, y: 5429.0, to_instance: None, arrive: None },
    Portal { from_instance: 148, x: 4384.0, y: -19844.0, to_instance: None, arrive: None },
    Portal { from_instance: 148, x: -13897.0, y: -20108.0, to_instance: None, arrive: None },
    Portal { from_instance: 148, x: -5488.0, y: 13494.0, to_instance: None, arrive: None },
    Portal { from_instance: 148, x: -14540.0, y: 10054.0, to_instance: None, arrive: None },
    Portal { from_instance: 148, x: -11309.0, y: -6226.0, to_instance: None, arrive: None },
    // instance 160
    // instance 161
    // instance 162
    Portal { from_instance: 162, x: -17120.0, y: 17020.0, to_instance: None, arrive: None },
    Portal { from_instance: 162, x: -26040.0, y: -13185.0, to_instance: None, arrive: None },
    Portal { from_instance: 162, x: 22595.0, y: 7257.0, to_instance: None, arrive: None },
    // instance 163
    Portal { from_instance: 163, x: 22967.0, y: -17036.0, to_instance: None, arrive: None },
    Portal { from_instance: 163, x: 22324.0, y: 13126.0, to_instance: None, arrive: None },
    Portal { from_instance: 163, x: 25555.0, y: -3154.0, to_instance: None, arrive: None },
    Portal { from_instance: 163, x: 6822.0, y: -17963.0, to_instance: None, arrive: None },
    Portal { from_instance: 163, x: -6793.0, y: -16845.0, to_instance: None, arrive: None },
    Portal { from_instance: 163, x: -3250.0, y: 9209.0, to_instance: None, arrive: None },
    Portal { from_instance: 163, x: -7361.0, y: 1435.0, to_instance: None, arrive: None },
    // instance 164
    Portal { from_instance: 164, x: 7378.0, y: 5429.0, to_instance: None, arrive: None },
    Portal { from_instance: 164, x: 4384.0, y: -19844.0, to_instance: None, arrive: None },
    Portal { from_instance: 164, x: -13897.0, y: -20108.0, to_instance: None, arrive: None },
    Portal { from_instance: 164, x: -5488.0, y: 13494.0, to_instance: None, arrive: None },
    Portal { from_instance: 164, x: -14540.0, y: 10054.0, to_instance: None, arrive: None },
    Portal { from_instance: 164, x: -11309.0, y: -6226.0, to_instance: None, arrive: None },
    // instance 165
    Portal { from_instance: 165, x: 10679.0, y: 19828.0, to_instance: None, arrive: None },
    Portal { from_instance: 165, x: 20040.0, y: -10113.0, to_instance: None, arrive: None },
    Portal { from_instance: 165, x: -5466.0, y: 18901.0, to_instance: None, arrive: None },
    Portal { from_instance: 165, x: 426.0, y: 7688.0, to_instance: None, arrive: None },
    Portal { from_instance: 165, x: -19081.0, y: 20019.0, to_instance: None, arrive: None },
    // instance 166
    Portal { from_instance: 166, x: -17120.0, y: 17020.0, to_instance: None, arrive: None },
    Portal { from_instance: 166, x: -26040.0, y: -13185.0, to_instance: None, arrive: None },
    Portal { from_instance: 166, x: 22595.0, y: 7257.0, to_instance: None, arrive: None },
    // instance 449
    Portal { from_instance: 449, x: -6751.0, y: 19576.0, to_instance: None, arrive: None },
    Portal { from_instance: 449, x: -6026.0, y: 16727.0, to_instance: None, arrive: None },
    Portal { from_instance: 449, x: -9263.0, y: 16821.0, to_instance: None, arrive: None },
    Portal { from_instance: 449, x: -3440.0, y: 16726.0, to_instance: None, arrive: None },
    Portal { from_instance: 449, x: -7396.0, y: 17058.0, to_instance: None, arrive: None },
    Portal { from_instance: 449, x: -5928.0, y: 14931.0, to_instance: None, arrive: None },
    Portal { from_instance: 449, x: -7297.0, y: 6184.0, to_instance: None, arrive: None },
];

/// The doorways read from the database (`map_portal`) at start-up, when there are any.
///
/// Their **positions** are the point: a doorway is where a prop stands, a profile edits it, and a restart
/// picks it up with no rebuild (docs/32, Phase 1). Nothing about how a crossing is detected changes: the
/// predicates in `portal_at` and `portal_along` and `REACH` are untouched.
static LOADED: std::sync::OnceLock<Vec<Portal>> = std::sync::OnceLock::new();

/// Install the database's doorways. Called once, by `main`, before the world accepts a connection; a
/// second call is ignored and answers `false`.
pub fn install(portals: Vec<Portal>) -> bool {
    LOADED.set(portals).is_ok()
}

/// Every doorway this world knows, in the order the trigger searches it: **`CROSSED` always first**, then
/// the database's rows when it has any, otherwise the compiled `PORTALS`.
///
/// `CROSSED` is never displaced. Those are the positions a live server was watched changing the map at, and
/// 14 of the 22 have no `map_portal` row anywhere near them; a table that replaced them would silently stop
/// those crossings firing (measured 2026-11-05, which is why this is a chain and not a substitution). The
/// database stands in for the compiled `PORTALS` half only - the doorways read from the archive.
fn table() -> Box<dyn Iterator<Item = &'static Portal>> {
    match LOADED.get() {
        Some(rows) if !rows.is_empty() => Box::new(CROSSED.iter().chain(rows.iter())),
        _ => Box::new(CROSSED.iter().chain(PORTALS.iter())),
    }
}

/// The doorway a position in `from_instance` is standing in, if any.
///
/// A doorway without a destination still answers: the caller decides what to do with it, and today that is
/// a log line naming the coordinates - the measurement that fills `to_instance` for the next session.
pub fn portal_at(from_instance: i32, x: f32, y: f32) -> Option<&'static Portal> {
    table().find(|portal| {
        portal.from_instance == from_instance
            && (portal.x - x).powi(2) + (portal.y - y).powi(2) <= REACH * REACH
    })
}

/// The doorway a **walk passes through**, given where the character stands and where it is going.
///
/// This is the reliable form of the trigger. A doorway is a threshold, so what matters is whether the
/// character's path meets it, not whether one position report happened to land inside it - and a report is
/// only sent while the character moves, which is why a click that stopped inside a door could cross without
/// firing. The test is the distance from the doorway's centre to the segment, so a path that merely passes
/// beside a door within `REACH` crosses it, and the segment is the one the client asked for.
pub fn portal_along(
    from_instance: i32,
    from: (f32, f32),
    to: (f32, f32),
) -> Option<&'static Portal> {
    table()
        .filter(|portal| portal.from_instance == from_instance)
        .find(|portal| distance_to_segment((portal.x, portal.y), from, to) <= REACH)
}

/// How far a point is from the segment `a`-`b`, in the map's own units.
fn distance_to_segment(point: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (px, py) = point;
    let (ax, ay) = a;
    let (bx, by) = b;
    let (dx, dy) = (bx - ax, by - ay);
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f32::EPSILON {
        return ((px - ax).powi(2) + (py - ay).powi(2)).sqrt();
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / length_squared).clamp(0.0, 1.0);
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

// ---- the pack's doorways: a wall to cross, and the arrival the target map owns -------------------

/// The folder inside a pack that holds one folder per map (the same tree `map_spawns` walks).
pub const MAPS: &str = "maps";

/// The file inside a map folder that holds that map's doorways and arrivals.
pub const PORTALS_FILE: &str = "portals.json";

/// How far a point-shaped doorway reaches, in the map's own units: a step within this distance of the
/// point crosses it. Wall-shaped doorways ignore this - they are crossed by passing through the segment.
pub const WALL_REACH: f32 = REACH;

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
    /// A wall is crossed when the step passes **through** the segment: the step's endpoints lie on
    /// opposite sides of the wall's line *and* the wall's endpoints lie on opposite sides of the step's
    /// line. Touching the wall, or walking beside it, is not crossing it. A point doorway has no wall,
    /// so it is crossed when the step passes within [`WALL_REACH`] of the point.
    pub fn crossed_by(&self, step_from: (f32, f32), step_to: (f32, f32)) -> bool {
        if !self.is_wall() {
            return distance_to_segment(self.from, step_from, step_to) <= WALL_REACH;
        }
        side(self.from, self.to, step_from) * side(self.from, self.to, step_to) < 0.0
            && side(step_from, step_to, self.from) * side(step_from, step_to, self.to) < 0.0
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
/// without `portals.json` has no pack doorways, and the measured tables answer for it instead. A file
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
        let name = folder.file_name().and_then(|name| name.to_str()).unwrap_or("");
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
        maps.insert(
            folder_id,
            MapPortals {
                arches,
                arrivals,
            },
        );
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
/// The **measured crossings come first and are never displaced**: they are the positions a live server
/// was watched changing the map at, and 14 of the 22 have no `map_portal` row anywhere near them. After
/// them the pack answers, when the map has a `portals.json`: its arches are that map's doorway table,
/// and a wall is crossed only by passing through it. A map the pack does not describe falls back to the
/// measured rows, whose doorways are points and are crossed within [`WALL_REACH`].
pub fn crossed(from_instance: i32, from: (f32, f32), to: (f32, f32)) -> Option<Crossed> {
    if let Some(portal) = CROSSED
        .iter()
        .filter(|portal| portal.from_instance == from_instance)
        .find(|portal| distance_to_segment((portal.x, portal.y), from, to) <= REACH)
    {
        return Some(Crossed {
            to_instance: portal.to_instance,
            from_instance,
        });
    }
    if let Some(arches) = pack_arches(from_instance) {
        return arches
            .iter()
            .find(|arch| arch.crossed_by(from, to))
            .map(|arch| Crossed {
                to_instance: arch.to_instance,
                from_instance,
            });
    }
    portal_along(from_instance, from, to).map(|portal| Crossed {
        to_instance: portal.to_instance,
        from_instance,
    })
}

/// Where a character arriving in `in_instance` from `from_instance` stands, when the data states it.
///
/// The receiving map owns this fact: the pack's `arrivals` for the destination are asked first, and the
/// measured tables' landing (`Portal::arrive`) answers for a map the pack does not describe.
pub fn arrival(in_instance: i32, from_instance: i32) -> Option<Arrival> {
    if let Some(map) = table_pack().get(&in_instance) {
        if let Some(arrival) = map.arrival_for(from_instance) {
            return Some(*arrival);
        }
    }
    table()
        .filter(|portal| portal.to_instance == Some(in_instance))
        .find(|portal| portal.from_instance == from_instance)
        .and_then(|portal| {
            portal.arrive.map(|(x, y)| Arrival {
                from: portal.from_instance,
                x,
                y,
                angle: None,
            })
        })
}

/// The arrival in `in_instance` whose point is `position`, within [`ARRIVAL_MATCH`].
///
/// A crossing writes the destination's arrival point as the character's saved position before the
/// redirect; the connection that then enters the map reads it back here, which is what carries the
/// arrival's **facing** across the reconnect (the position is already in the saved row).
pub fn arrival_at(in_instance: i32, position: (f32, f32)) -> Option<Arrival> {
    if let Some(map) = table_pack().get(&in_instance) {
        if let Some(arrival) = map.arrival_at(position) {
            return Some(*arrival);
        }
    }
    table()
        .filter(|portal| portal.to_instance == Some(in_instance))
        .filter_map(|portal| portal.arrive)
        .find(|(x, y)| {
            (x - position.0).abs() <= ARRIVAL_MATCH && (y - position.1).abs() <= ARRIVAL_MATCH
        })
        .map(|(x, y)| Arrival {
            from: 0,
            x,
            y,
            angle: None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_doorway_is_found_where_it_stands_and_nowhere_else() {
        let Some(portal) = PORTALS.first() else {
            panic!("the table is generated from data/portals.json");
        };
        let found = portal_at(portal.from_instance, portal.x, portal.y).expect("its own position");
        assert_eq!(found.x, portal.x);
        assert_eq!(found.y, portal.y);

        assert!(
            portal_at(portal.from_instance, portal.x + REACH * 2.0, portal.y).is_none(),
            "two reaches away is not the doorway"
        );
        assert!(
            portal_at(portal.from_instance + 1, portal.x, portal.y).is_none(),
            "a doorway belongs to the instance it stands in"
        );
    }

    /// Several instances share one geometry blob, so the same coordinates are listed under more than one of
    /// them - which is why the trigger is keyed by the instance as well as the position.
    #[test]
    fn one_blob_s_doors_are_listed_under_every_instance_that_shares_it() {
        let mut shared = None;
        for portal in PORTALS {
            let count = PORTALS
                .iter()
                .filter(|other| other.x == portal.x && other.y == portal.y)
                .count();
            if count > 1 {
                shared = Some((portal.x, portal.y, count));
                break;
            }
        }
        let (x, y, count) = shared.expect("146, 148 and 164 share the pre-Searing blob's doorways");
        assert!(count >= 2, "({x}, {y}) belongs to {count} instances");
    }

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
    fn a_wall_is_crossed_only_by_passing_through_it() {
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
            !wall().crossed_by((10.0, -1.0), (10.0, 1.0)),
            "touching the wall at its endpoint is not passing through it"
        );
    }

    #[test]
    fn a_point_doorway_keeps_its_reach() {
        let point = Arch {
            from: (0.0, 0.0),
            to: (0.0, 0.0),
            ..wall()
        };
        assert!(
            point.crossed_by((100.0, -200.0), (100.0, 200.0)),
            "a step passing 100 units from the point is inside its reach"
        );
        assert!(
            !point.crossed_by((400.0, -200.0), (400.0, 200.0)),
            "400 units away is outside it"
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
        assert!(arch.is_wall(), "the owner stated the arch as a wall: {arch:?}");
        let arrival = arrival(148, 146).expect("where a character arriving from 146 stands");
        assert_eq!(arrival.from, 146);
        let docked = arrival_at(148, (arrival.x, arrival.y)).expect("the same point reads back");
        assert_eq!((docked.x, docked.y), (arrival.x, arrival.y));
        assert_eq!(docked.angle, arrival.angle, "the facing is the arrival's");
    }

    #[test]
    fn a_step_far_from_every_doorway_crosses_nothing() {
        // Whatever answers for a map - the pack or the measured tables - a step nowhere near a doorway
        // fires no crossing.
        assert!(
            crossed(449, (0.0, 0.0), (1.0, 0.0)).is_none(),
            "a step far from every doorway crosses nothing"
        );
    }
}
