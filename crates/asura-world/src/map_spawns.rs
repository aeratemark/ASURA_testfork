//! The spawn points a map carries, read from the world pack.
//!
//! A map is entered at one of the points its own file states: one `spawns.json` per map folder
//! (`config/World/<pack>/maps/<campaign>/<area>/<subarea>/<map id>/`, `docs/38-layer-decisions.md`
//! D38, D39), one entry per spawn, in `idx` order. An entry is a coordinate, the plane inside the
//! map blob, and - where the pack states one - a facing angle in radians. The angle becomes the
//! direction vector the agent is created with, which is what `GAME_SMSG_WORLD_CREATE_AGENT` carries.
//!
//! The map itself (its blob and its single fallback coordinate) still arrives from `t_map_config`
//! today; this module is the list beside it, and a map without a file has no list at all, so the
//! caller falls back to that one coordinate.
//!
//! This is the list the owner filled for Ascalon City (pre-Searing) on 2026-11-05, and the reason
//! the outpost entry is randomised: "each outpost has multiple spawn points from a list, they are
//! randomized when you enter" (`docs/22-world-model.md`, "Spawn points, and the outpost rule").

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The folder inside a pack that holds one folder per map.
pub const MAPS: &str = "maps";

/// The file inside a map folder that holds that map's spawn list.
pub const SPAWNS: &str = "spawns.json";

/// One spawn point of one map, as `spawns.json` states it.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct MapSpawn {
    /// The point's own index in the map's list. The file states it, so entries are sorted by it
    /// rather than by the order they happen to be written in.
    #[serde(default)]
    pub idx: u32,
    /// The coordinate the client is placed at.
    pub x: f32,
    /// The coordinate that goes with it.
    pub y: f32,
    /// The plane inside the map blob, not a height.
    #[serde(default)]
    pub plane: u16,
    /// The facing, in radians, when the pack states one. `None` keeps the world's default facing.
    #[serde(default)]
    pub angle: Option<f32>,
}

impl MapSpawn {
    /// The direction vector the agent is created facing: the angle as `(cos, sin)`, which is the
    /// unit-vector form `GAME_SMSG_WORLD_CREATE_AGENT` carries. A point with no angle keeps `(1, 0)`,
    /// the direction this world has always sent.
    pub fn direction(&self) -> (f32, f32) {
        match self.angle {
            Some(angle) => (angle.cos(), angle.sin()),
            None => (1.0, 0.0),
        }
    }
}

/// One map's file: the map id, and its spawn list.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct SpawnFile {
    /// The map the file is about; checked against the folder's name.
    pub map_id: i32,
    /// The map's spawn points.
    pub spawns: Vec<MapSpawn>,
}

impl SpawnFile {
    /// Parse one map's spawn file.
    pub fn from_json(text: &str) -> Result<SpawnFile, String> {
        serde_json::from_str(text).map_err(|err| err.to_string())
    }
}

/// Every map's spawn list under `pack`, keyed by map id.
///
/// Only a folder named after a map id is a map: `Prophecies/Ascalon (pre-Searing)/Missions/` holds
/// maps but is not one, so it is walked through rather than read. A map folder without a
/// `spawns.json` has no list and is not an error - most maps carry none yet. A file that does not
/// parse, or a `map_id` that disagrees with the folder's name, is reported and skipped: a list
/// nobody can trust is worse than the single coordinate the catalogue row carries.
pub fn walk(pack: &Path) -> (HashMap<i32, Vec<MapSpawn>>, Vec<String>) {
    let mut lists: HashMap<i32, Vec<MapSpawn>> = HashMap::new();
    let mut notes: Vec<String> = Vec::new();
    let maps = pack.join(MAPS);
    if !maps.is_dir() {
        notes.push(format!("{} is not readable", maps.display()));
    } else {
        visit(&maps, &mut lists, &mut notes);
    }
    (lists, notes)
}

/// Read one map folder, or walk into a folder that is a level of the tree.
fn visit(dir: &Path, lists: &mut HashMap<i32, Vec<MapSpawn>>, notes: &mut Vec<String>) {
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
            visit(&folder, lists, notes);
            continue;
        };
        let file = folder.join(SPAWNS);
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
        let parsed = match SpawnFile::from_json(&text) {
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
        let mut spawns = parsed.spawns;
        spawns.sort_by_key(|spawn| spawn.idx);
        lists.insert(folder_id, spawns);
    }
}

/// The pack in force, exactly as the other pack readers resolve it (`ASURA_WORLD_PACK`, then the
/// default, then the workspace root a crate test is built from).
pub fn pack() -> PathBuf {
    crate::spawns::pack()
}

/// Every map's spawn list, read once per process.
///
/// A file read per arrival would be a live link to the disk in the middle of serving. The pack is
/// content: read at first use, held, and its problems printed once, so a bad edit fails where it is
/// visible instead of on somebody's next travel.
pub fn table() -> &'static HashMap<i32, Vec<MapSpawn>> {
    static TABLE: std::sync::OnceLock<HashMap<i32, Vec<MapSpawn>>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let pack = pack();
        let (lists, notes) = walk(&pack);
        for note in &notes {
            eprintln!("world: map spawns: {note}");
        }
        let points: usize = lists.values().map(|spawns| spawns.len()).sum();
        eprintln!(
            "world: map spawns from {}: {} map(s), {} spawn point(s)",
            pack.display(),
            lists.len(),
            points
        );
        lists
    })
}

/// One map's spawn list, if the pack states one.
pub fn for_map(map_id: i32) -> Option<&'static [MapSpawn]> {
    table().get(&map_id).map(Vec::as_slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_pack_parses_and_names_ascalon_city() {
        let (lists, notes) = walk(&pack());
        assert!(
            notes.is_empty(),
            "a file that does not parse must not be silent: {notes:?}"
        );
        let asc = lists
            .get(&148)
            .expect("the outpost the list was filled for");
        assert_eq!(asc.len(), 2, "two spawn points, ordered by idx");
        assert_eq!((asc[0].idx, asc[1].idx), (0, 1));
        assert_eq!((asc[0].x, asc[0].y), (9497.82, 4835.33));
        assert_eq!((asc[1].x, asc[1].y), (9686.74, 7979.27));
    }

    #[test]
    fn an_angle_becomes_the_agents_direction_vector() {
        let spawn = MapSpawn {
            idx: 0,
            x: 1.0,
            y: 2.0,
            plane: 0,
            angle: Some(0.0),
        };
        let (dx, dy) = spawn.direction();
        assert!((dx - 1.0).abs() < 1e-6 && dy.abs() < 1e-6, "0 radians is +x");
        let spawn = MapSpawn {
            angle: Some(std::f32::consts::FRAC_PI_2),
            ..spawn
        };
        let (dx, dy) = spawn.direction();
        assert!(
            dx.abs() < 1e-6 && (dy - 1.0).abs() < 1e-6,
            "a quarter turn is +y"
        );
        assert_eq!((1.0, 0.0), MapSpawn { angle: None, ..spawn }.direction());
    }

    #[test]
    fn a_map_without_a_spawn_file_has_no_list_and_is_not_reported() {
        let dir = std::env::temp_dir().join("asura-map-spawns-test");
        let folder = dir.join(MAPS).join("Prophecies").join("Kryta").join("001");
        std::fs::create_dir_all(&folder).expect("create");
        std::fs::write(
            folder.join("basic.json"),
            r#"{"map_id": 1, "instance_value": 7}"#,
        )
        .expect("write");
        let (lists, notes) = walk(&dir);
        assert!(notes.is_empty(), "no spawns.json is not a problem: {notes:?}");
        assert!(lists.is_empty(), "and it names no spawn list");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_that_disagrees_with_its_folder_is_skipped_and_reported() {
        let dir = std::env::temp_dir().join("asura-map-spawns-mismatch");
        let folder = dir.join(MAPS).join("001");
        std::fs::create_dir_all(&folder).expect("create");
        std::fs::write(
            folder.join(SPAWNS),
            r#"{"map_id": 2, "spawns": [{"x": 1.0, "y": 2.0}]}"#,
        )
        .expect("write");
        let (lists, notes) = walk(&dir);
        assert!(lists.is_empty(), "the disagreement is skipped");
        assert_eq!(notes.len(), 1, "and it is reported: {notes:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
