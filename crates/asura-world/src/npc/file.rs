//! The pack's NPC files, read: `npcs/definitions.json`, `npcs/sight.json`, and one `npcs.json` per
//! map folder beside its `spawns.json` and `portals.json` (`docs/49-npc-framework.md` section 3.2).
//!
//! This module only parses and walks; every check is [`NpcContent::assemble`]'s, so a test can build the
//! same content without a disk. Unlike the spawn lists, a bad NPC file is not skipped: one file that
//! does not parse refuses the whole pack, because a town with half its people is a second truth.

use super::content::{
    MapRecord, NpcContent, NpcDefinition, PlacementRecord, Property, PropertyValue, Route,
    RouteMode, RoutePoint, SightRules,
};
use crate::game::NamedItem;
use std::collections::BTreeMap;
use std::path::Path;

/// The pack-level folder that holds the NPC types and the world-wide NPC values.
pub const DIR: &str = "npcs";
/// The NPC types, inside [`DIR`].
pub const DEFINITIONS: &str = "definitions.json";
/// The world-wide NPC values, inside [`DIR`].
pub const SIGHT: &str = "sight.json";
/// One map's bodies and routes, inside the map's own folder.
pub const MAP_FILE: &str = "npcs.json";

/// `npcs/definitions.json`.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionsFile {
    /// For whoever opens the file.
    #[serde(default)]
    #[allow(dead_code)]
    note: Option<String>,
    /// One sentence per field, for whoever opens the file.
    #[serde(default)]
    #[allow(dead_code)]
    fields: BTreeMap<String, String>,
    definitions: Vec<NpcDefinition>,
}

/// A value with the sentence that says what it means.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Valued<T> {
    value: T,
    #[allow(dead_code)]
    doc: String,
}

/// `npcs/sight.json`.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SightFile {
    #[serde(default)]
    #[allow(dead_code)]
    note: Option<String>,
    compass_range: Valued<f32>,
}

/// One map's `npcs.json`.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MapFile {
    map_id: i32,
    #[serde(default)]
    #[allow(dead_code)]
    note: Option<String>,
    #[serde(default)]
    placements: Vec<PlacementRow>,
    #[serde(default)]
    routes: Vec<RouteRow>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PlacementRow {
    id: String,
    definition: String,
    x: f32,
    y: f32,
    plane: u16,
    /// The facing in radians, for a hand-written row.
    #[serde(default)]
    angle: Option<f32>,
    /// The direction vector, for a row copied from a capture: an angle does not reproduce the live
    /// server's bits (6 of the 18 NPC creates of `pre-searing-load-3.jsonl` differ in the last place).
    #[serde(default)]
    direction: Option<[f32; 2]>,
    #[serde(default)]
    scale: Option<u32>,
    #[serde(default)]
    name: Option<Vec<u16>>,
    #[serde(default)]
    level: Option<u32>,
    effects: u32,
    h0023: f32,
    #[serde(default)]
    properties: Vec<PropertyRow>,
    #[serde(default)]
    items: Vec<ItemRow>,
    #[serde(default)]
    weapons: [Option<usize>; 2],
    #[serde(default)]
    route: Option<String>,
    #[serde(default)]
    walk_speed: Option<f32>,
    #[serde(default)]
    behavior: Option<String>,
}

/// One held item as the pack states it; the wire's item id is allocated by the world.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemRow {
    file_id: u32,
    item_type: u8,
    #[serde(default)]
    dye_tint: u8,
    #[serde(default)]
    dye_colors: u16,
    #[serde(default)]
    materials: u16,
    #[serde(default)]
    unk1: u8,
    #[serde(default)]
    flags: u32,
    #[serde(default)]
    value: u32,
    model_id: u32,
    #[serde(default = "one")]
    quantity: u32,
    #[serde(default)]
    name: Vec<u16>,
    #[serde(default)]
    modifiers: Vec<u32>,
}

fn one() -> u32 {
    1
}

/// A property: its id and exactly one of `int` or `float`.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PropertyRow {
    id: u32,
    #[serde(default)]
    int: Option<u32>,
    #[serde(default)]
    float: Option<f32>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteRow {
    id: String,
    mode: String,
    points: Vec<PointRow>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PointRow {
    x: f32,
    y: f32,
    plane: u16,
    #[serde(default)]
    pause_ms: u32,
    #[serde(default)]
    angle: Option<f32>,
}

fn map_record(file: MapFile, problems: &mut Vec<String>) -> MapRecord {
    let map_id = file.map_id;
    let placements = file
        .placements
        .into_iter()
        .map(|row| {
            let properties = row
                .properties
                .iter()
                .filter_map(|property| {
                    let value = match (property.int, property.float) {
                        (Some(value), None) => PropertyValue::Int(value),
                        (None, Some(value)) => PropertyValue::Float(value),
                        _ => {
                            problems.push(format!(
                                "map {map_id} placement {}: property {} needs exactly one of int and float",
                                row.id, property.id
                            ));
                            return None;
                        }
                    };
                    Some(Property {
                        id: property.id,
                        value,
                    })
                })
                .collect();
            // Without either, `(1, 0)`, as a spawn point without an angle faces.
            let direction = match (row.angle, row.direction) {
                (None, None) => (1.0, 0.0),
                (Some(angle), None) => (angle.cos(), angle.sin()),
                (None, Some([dx, dy])) => (dx, dy),
                (Some(_), Some(_)) => {
                    problems.push(format!(
                        "map {map_id} placement {}: give an angle or a direction, not both",
                        row.id
                    ));
                    (1.0, 0.0)
                }
            };
            PlacementRecord {
                id: row.id,
                definition: row.definition,
                position: (row.x, row.y),
                plane: row.plane,
                direction,
                scale: row.scale,
                name: row.name,
                level: row.level,
                effects: row.effects,
                h0023: row.h0023,
                properties,
                items: row
                    .items
                    .into_iter()
                    .map(|item| NamedItem {
                        id: 0,
                        file_id: item.file_id,
                        item_type: item.item_type,
                        dye_tint: item.dye_tint,
                        dye_colors: item.dye_colors,
                        materials: item.materials,
                        unk1: item.unk1,
                        flags: item.flags,
                        value: item.value,
                        model_id: item.model_id,
                        quantity: item.quantity,
                        name: item.name,
                        modifiers: item.modifiers,
                    })
                    .collect(),
                weapons: row.weapons,
                route: row.route,
                walk_speed: row.walk_speed,
                behavior: row.behavior,
            }
        })
        .collect();
    let routes = file
        .routes
        .into_iter()
        .map(|row| {
            let mode = match row.mode.as_str() {
                "loop" => RouteMode::Loop,
                "once" => RouteMode::Once,
                "random" => RouteMode::Random,
                other => {
                    problems.push(format!(
                        "map {map_id} route {}: mode {other:?} is neither \"loop\", \"once\" nor \
                         \"random\"",
                        row.id
                    ));
                    RouteMode::Once
                }
            };
            Route {
                id: row.id,
                mode,
                points: row
                    .points
                    .into_iter()
                    .map(|point| RoutePoint {
                        position: (point.x, point.y),
                        plane: point.plane,
                        pause_ms: point.pause_ms,
                        angle: point.angle,
                    })
                    .collect(),
            }
        })
        .collect();
    MapRecord {
        map_id,
        placements,
        routes,
    }
}

/// Read a JSON file into `T`, naming the file in the error.
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("{} is not readable: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("{}: {err}", path.display()))
}

/// Every map folder's `npcs.json`, with its folder's map id. Only a folder named after a map id is a
/// map; the others are levels of the tree (`crate::map_spawns::walk`'s rule).
fn walk_maps(dir: &Path, found: &mut Vec<(i32, std::path::PathBuf)>, problems: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            problems.push(format!("{} is not readable: {err}", dir.display()));
            return;
        }
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
        match name.parse::<i32>() {
            Ok(map_id) => {
                let file = folder.join(MAP_FILE);
                if file.exists() {
                    found.push((map_id, file));
                }
            }
            Err(_) => walk_maps(&folder, found, problems),
        }
    }
}

/// Read and check the pack's NPC content. A pack with no `npcs/` folder and no map file has no NPCs,
/// which is valid; a pack with either needs `npcs/sight.json` and `npcs/definitions.json`.
pub fn load(pack: &Path) -> Result<NpcContent, Vec<String>> {
    let mut problems = Vec::new();
    let mut files = Vec::new();
    let maps_dir = pack.join(crate::map_spawns::MAPS);
    if maps_dir.is_dir() {
        walk_maps(&maps_dir, &mut files, &mut problems);
    }
    files.sort();
    let dir = pack.join(DIR);
    if !dir.is_dir() && files.is_empty() && problems.is_empty() {
        return Ok(NpcContent::default());
    }

    let sight = read::<SightFile>(&dir.join(SIGHT))
        .map(|file| SightRules {
            compass_range: file.compass_range.value,
        })
        .unwrap_or_else(|err| {
            problems.push(err);
            SightRules::default()
        });
    let definitions = read::<DefinitionsFile>(&dir.join(DEFINITIONS))
        .map(|file| file.definitions)
        .unwrap_or_else(|err| {
            problems.push(err);
            Vec::new()
        });
    let mut maps = Vec::new();
    for (folder_id, path) in files {
        match read::<MapFile>(&path) {
            Ok(file) if file.map_id != folder_id => problems.push(format!(
                "{}: folder is {folder_id}, file says {}",
                path.display(),
                file.map_id
            )),
            Ok(file) => maps.push(map_record(file, &mut problems)),
            Err(err) => problems.push(err),
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    NpcContent::assemble(sight, definitions, maps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join(DIR)).expect("create");
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("create");
        std::fs::write(path, text).expect("write");
    }

    const SIGHT_JSON: &str = r#"{"compass_range": {"value": 5000.0, "doc": "d"}}"#;
    const DEFINITIONS_JSON: &str = r#"{"definitions": [{"id": "guard", "definition_index": 1459,
        "file_id": 116227, "h000a": 0, "visual": 1677721600, "h0012": 0, "flags": 524, "primary": 1,
        "level": 2, "name": [3915, 35665, 58079, 18830], "model_files": [129945], "h000b": 9,
        "speed": 250.0, "team": "nonc"}]}"#;

    #[test]
    fn the_committed_pack_loads() {
        let content = load(&crate::spawns::pack()).expect("the pack's NPCs load");
        let ascalon = content.map(148).expect("Ascalon City has NPCs");
        assert!(!ascalon.placements.is_empty());
        let tad = content
            .definitions
            .get("mob-1433-type")
            .expect("the River Skale Tad");
        let combat = tad.combat.as_ref().expect("the tad's combat block");
        assert_eq!(
            combat.aggression,
            crate::npc::content::Aggression::WhenProvoked
        );
        let attack = combat.attack.as_ref().expect("the tad's attack");
        assert!(attack.ranged, "the tad spits from range");
        assert_eq!(attack.projectile, Some(1), "the adult's captured spit");
    }

    #[test]
    fn a_pack_without_npcs_has_none() {
        let dir = std::env::temp_dir().join("asura-npc-none");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("maps").join("148")).expect("create");
        let content = load(&dir).expect("valid");
        assert_eq!(content.counts(), (0, 0, 0));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_map_file_is_read_from_its_folder_and_checked_against_it() {
        let dir = pack("asura-npc-folder");
        write(&dir.join(DIR).join(SIGHT), SIGHT_JSON);
        write(&dir.join(DIR).join(DEFINITIONS), DEFINITIONS_JSON);
        let folder = dir.join("maps").join("Prophecies").join("Town").join("148");
        write(
            &folder.join(MAP_FILE),
            r#"{"map_id": 148, "placements": [{"id": "gate", "definition": "guard", "x": 1.0,
                "y": 2.0, "plane": 0, "effects": 0, "h0023": 1.0,
                "properties": [{"id": 11, "int": 5}]}]}"#,
        );
        let content = load(&dir).expect("loads");
        let map = content.map(148).expect("map");
        assert_eq!(map.placements[0].definition.team.text(), "nonc");
        assert_eq!(
            map.placements[0].properties,
            vec![Property {
                id: 11,
                value: PropertyValue::Int(5)
            }]
        );
        write(
            &folder.join(MAP_FILE),
            r#"{"map_id": 149, "placements": []}"#,
        );
        let problems = load(&dir).expect_err("refused");
        assert!(
            problems[0].contains("folder is 148, file says 149"),
            "{problems:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unknown_key_a_bad_team_and_a_two_valued_property_are_refused() {
        let dir = pack("asura-npc-bad");
        write(&dir.join(DIR).join(SIGHT), SIGHT_JSON);
        write(
            &dir.join(DIR).join(DEFINITIONS),
            &DEFINITIONS_JSON.replace("\"nonc\"", "\"no\""),
        );
        write(
            &dir.join("maps").join("148").join(MAP_FILE),
            r#"{"map_id": 148, "placements": [{"id": "gate", "definition": "guard", "x": 1.0,
                "y": 2.0, "plane": 0, "effects": 0, "h0023": 1.0,
                "properties": [{"id": 11, "int": 5, "float": 1.0}]}]}"#,
        );
        let problems = load(&dir).expect_err("refused");
        let text = problems.join("\n");
        assert!(text.contains("team \"no\""), "{text}");
        assert!(text.contains("exactly one of int and float"), "{text}");
        write(
            &dir.join("maps").join("148").join(MAP_FILE),
            r#"{"map_id": 148, "placements": [], "colour": 1}"#,
        );
        let problems = load(&dir).expect_err("refused");
        assert!(problems.join("\n").contains("colour"), "{problems:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
