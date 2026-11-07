//! Map descriptions shared by auth and world, read only from the selected world pack.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, serde::Deserialize)]
/// The values describing a map, stored in its basic.json.
pub struct MapBasic {
    /// The map whose numeric folder contains this record.
    pub map_id: i32,
    /// The name displayed by the map catalogue.
    pub name: String,
    /// The client's instance kind.
    pub map_type: i32,
    /// The client archive file, or unknown when null.
    pub map_file_id: Option<u32>,
}

/// Resolve the selected world pack for both services.
pub fn pack() -> PathBuf {
    super::resolve(
        &std::env::var("ASURA_WORLD_PACK").unwrap_or_else(|_| "config/World/Vanilla".to_string()),
    )
}

/// Read map descriptions recursively; malformed files fail instead of falling back to SQL.
pub fn load(pack: &Path) -> Result<BTreeMap<i32, MapBasic>, String> {
    fn visit(dir: &Path, maps: &mut BTreeMap<i32, MapBasic>) -> Result<(), String> {
        for entry in std::fs::read_dir(dir).map_err(|err| format!("{}: {err}", dir.display()))? {
            let path = entry.map_err(|err| err.to_string())?.path();
            if !path.is_dir() {
                continue;
            }
            let id = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.parse::<i32>().ok());
            let Some(id) = id else {
                visit(&path, maps)?;
                continue;
            };
            let file = path.join("basic.json");
            if !file.exists() {
                continue;
            }
            let text = std::fs::read_to_string(&file)
                .map_err(|err| format!("{}: {err}", file.display()))?;
            let value: serde_json::Value =
                serde_json::from_str(&text).map_err(|err| format!("{}: {err}", file.display()))?;
            if value["map_id"].as_i64() != Some(i64::from(id)) {
                return Err(format!("{}: map_id disagrees with folder", file.display()));
            }
            // Instance-table-only records remain useful, but cannot describe a hosted map.
            if value.get("map_type").is_none() || value["map_type"].is_null() {
                continue;
            }
            let map: MapBasic = serde_json::from_value(value)
                .map_err(|err| format!("{}: {err}", file.display()))?;
            if maps.insert(id, map).is_some() {
                return Err(format!("duplicate map {id} in {}", file.display()));
            }
        }
        Ok(())
    }
    let mut maps = BTreeMap::new();
    visit(&pack.join("maps"), &mut maps)?;
    Ok(maps)
}

/// Hold the selected pack's map catalogue for the lifetime of this process.
pub fn catalogue() -> Result<&'static BTreeMap<i32, MapBasic>, String> {
    static MAPS: OnceLock<Result<BTreeMap<i32, MapBasic>, String>> = OnceLock::new();
    MAPS.get_or_init(|| load(&pack()))
        .as_ref()
        .map_err(Clone::clone)
}

#[cfg(test)]
mod tests {
    #[test]
    fn northlands_uses_the_live_file_id_from_the_pack() {
        // Live capture northlands-live-20261106-014922.jsonl, record 511, StoC 0x0196.
        let maps = super::catalogue().expect("the committed pack parses");
        assert_eq!(maps.get(&147).unwrap().map_file_id, Some(116025));
    }
}
