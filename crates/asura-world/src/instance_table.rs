//! The client's instance table, constructed from the pack's map folders.
//!
//! A client cannot ask to travel to a map it does not know exists: `0x01A0` is how it learns, as
//! `(instance, value)` pairs. Those pairs are no longer a transcribed constant - every map folder under
//! `config/World/Vanilla/maps/` states its own `instance_value`, and this module walks them and builds
//! the batches a load sends.
//!
//! A map is reached by three names before its own: `maps/<campaign>/<area>/<subarea>/<map id>/`
//! (`docs/38-layer-decisions.md`, D38). Only the folder whose name is a map id is a map, and the walk
//! goes as deep as the tree does, so adding a level there changes no code here.
//!
//! **What a map without a value means:** it is left out, and the log says so. Answering with a wrong
//! value is worse than a destination the client cannot resolve yet.

use std::path::{Path, PathBuf};

use asura_protocol::client::Frame;
use asura_protocol::game;

/// The folder inside a pack that holds one folder per map.
pub const MAPS: &str = "maps";

/// The file each map folder holds.
pub const BASIC: &str = "basic.json";

/// The pairs one `0x01A0` message carries, which is the capacity a live server sends.
pub const BATCH: usize = 15;

/// One map as its folder states it.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct MapRecord {
    /// The map id, which is also the folder's name.
    pub map_id: i32,
    /// The value a live load sent with it in `0x01A0`.
    pub instance_value: u32,
    /// The client's string id of the map's name: the name id in `Gw.exe`'s `AreaInfo` entry for this
    /// map. A party countdown to this map names it in its window (`party::travel_map_name`); a map
    /// without one moves a party on the server's timer alone, with no window.
    #[serde(default)]
    pub name_id: Option<u32>,
    /// Free text for whoever opens the file.
    #[serde(default)]
    pub doc: Option<String>,
}

impl MapRecord {
    /// Parse one map's file.
    pub fn from_json(text: &str) -> Result<MapRecord, String> {
        serde_json::from_str(text).map_err(|err| err.to_string())
    }
}

/// The pairs to send, in the batch shape a load uses.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InstanceTable {
    /// `(map id, instance value)` per map, ordered by map id.
    pub pairs: Vec<(u16, u32)>,
}

impl InstanceTable {
    /// The frames: `0x01A0` messages of at most [`BATCH`] pairs each.
    pub fn frames(&self) -> Vec<Frame> {
        self.pairs.chunks(BATCH).map(game::instance_table).collect()
    }
}

/// Every map record under `pack`, ordered by map id.
///
/// Only a folder named after a map id is a map: `Prophecies/Ascalon (pre-Searing)/Missions/` holds maps
/// but is not one, so it is walked through rather than read. A folder without `basic.json`, a file that
/// does not parse, or a `map_id` that disagrees with the folder's name is reported and skipped: the table
/// is what a client can resolve, and one bad folder must not empty it.
pub fn walk(pack: &Path) -> (Vec<MapRecord>, Vec<String>) {
    let mut records: Vec<MapRecord> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let maps = pack.join(MAPS);
    if !maps.is_dir() {
        notes.push(format!("{} is not readable", maps.display()));
    } else {
        visit(&maps, &mut records, &mut notes);
    }
    records.sort_by_key(|record| record.map_id);
    (records, notes)
}

/// Read one map folder, or walk into a folder that is a level of the tree.
fn visit(dir: &Path, records: &mut Vec<MapRecord>, notes: &mut Vec<String>) {
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
            visit(&folder, records, notes);
            continue;
        };
        let file = folder.join(BASIC);
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(err) => {
                notes.push(format!("{}: {err}", file.display()));
                continue;
            }
        };
        let record = match MapRecord::from_json(&text) {
            Ok(record) => record,
            Err(err) => {
                notes.push(format!("{}: {err}", file.display()));
                continue;
            }
        };
        if folder_id != record.map_id {
            notes.push(format!(
                "{}: folder is {folder_id}, file says {}; skipped",
                file.display(),
                record.map_id
            ));
            continue;
        }
        records.push(record);
    }
}

/// The table the world sends: every map the pack names, in map-id order, in batches a load uses.
pub fn table_for(pack: &Path) -> (InstanceTable, Vec<String>) {
    let (records, notes) = walk(pack);
    let pairs = records
        .into_iter()
        .filter_map(|record| {
            u16::try_from(record.map_id)
                .ok()
                .map(|map_id| (map_id, record.instance_value))
        })
        .collect();
    (InstanceTable { pairs }, notes)
}

/// The pack folder a world reads: `ASURA_WORLD_PACK`, or the default.
pub fn pack() -> PathBuf {
    crate::spawns::pack()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pack_names_every_map_the_recording_carried() {
        let (table, notes) = table_for(&pack());
        assert!(
            notes.is_empty(),
            "a folder that does not parse must not be silent: {notes:?}"
        );
        assert_eq!(
            table.pairs.len(),
            137,
            "the pairs the live load of instance 148 carried"
        );
        assert!(
            table.pairs.contains(&(148, 0xBB14_F539)),
            "the instance the recording was taken in is in the table"
        );
        assert!(
            table.pairs.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "ordered by map id, so a load is stable between runs"
        );
    }

    /// A party countdown names its destination, and a map without `name_id` gets no window
    /// (`party::travel_map_name`). Every pre-Searing outpost is a destination, so each must have one;
    /// `tools/add-map-name-ids.py` writes them from the client.
    #[test]
    fn every_pre_searing_outpost_has_a_name_for_the_countdown() {
        let (records, _) = walk(&pack());
        for map_id in [148, 163, 164, 165, 166] {
            let record = records
                .iter()
                .find(|record| record.map_id == map_id)
                .unwrap_or_else(|| panic!("map {map_id} is in the pack"));
            assert!(
                record.name_id.is_some(),
                "map {map_id} has no name_id: run tools/add-map-name-ids.py"
            );
        }
    }

    #[test]
    fn the_table_frames_in_batches_a_client_expects() {
        let (table, _) = table_for(&pack());
        let frames = table.frames();
        assert_eq!(frames.len(), table.pairs.len().div_ceil(BATCH));
        // A batch is a u16 count then the pairs: two-byte header, count, six bytes each.
        for (index, frame) in frames.iter().enumerate() {
            let expected = table.pairs.chunks(BATCH).nth(index).expect("a batch").len();
            assert_eq!(frame.body().len(), 1 + expected * 6);
        }
    }

    #[test]
    fn a_folder_that_disagrees_with_its_file_is_skipped_and_reported() {
        let dir = std::env::temp_dir().join("asura-instance-table-test");
        let folder = dir.join(MAPS).join("001");
        std::fs::create_dir_all(&folder).expect("create");
        std::fs::write(folder.join(BASIC), r#"{"map_id": 2, "instance_value": 1}"#).expect("write");
        let (records, notes) = walk(&dir);
        assert!(records.is_empty(), "the disagreement is skipped");
        assert_eq!(notes.len(), 1, "and it is reported: {notes:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_map_is_found_below_the_three_names_that_place_it() {
        let dir = std::env::temp_dir().join("asura-instance-table-nested");
        let folder = dir
            .join(MAPS)
            .join("Prophecies")
            .join("Ascalon (pre-Searing)")
            .join("Missions")
            .join("148");
        std::fs::create_dir_all(&folder).expect("create");
        std::fs::write(
            folder.join(BASIC),
            r#"{"map_id": 148, "instance_value": 3138712889}"#,
        )
        .expect("write");
        let (records, notes) = walk(&dir);
        assert!(notes.is_empty(), "the levels above a map are not errors: {notes:?}");
        assert_eq!(records.len(), 1, "the map is found four levels down");
        assert_eq!(records[0].map_id, 148);
        std::fs::remove_dir_all(&dir).ok();
    }
}
