//! Write the map content tables into the world pack, one folder per map.
//!
//! The move `docs/38-layer-decisions.md` D22 records, first step: **write the package**. This reads
//! `t_map_config`, `t_map_spawn` and `t_map_portal` and writes, in the folder each map id already has
//! under the pack's `maps/`:
//!
//! - `basic.json` - the map itself: `map_id`, `name`, `map_type`, `map_file_id`, and the
//!   `instance_value` and `doc` text the file already carried. **No spawn and no `spawn_count`**: the
//!   count is the length of the spawn list, and a second copy of it is a number to keep true by hand.
//! - `spawns.json` - that map's spawn list, `(idx, x, y, plane)` per entry, and the only place a spawn is
//!   stated.
//! - `portals.json` - that map's arches `(idx, x, y, to_map)`, and the landings **other maps get when
//!   they send somebody here**: the arrival belongs to the map that receives it (D39), so an origin's
//!   arch never states where the far end puts you.
//!
//! A map id with no folder is reported and skipped: the folder is the placement, and this tool does not
//! decide one. Nothing in the database is changed or dropped here - that is the last step, after the
//! readers have switched.
//!
//! ```powershell
//! $env:ASURA_DATABASE_URL = (Get-Content .env | Select-String ASURA_DATABASE_URL).Line.Split('=')[1]
//! cargo run -p asura-db --example export-map-content
//! ```

use std::collections::BTreeMap;

use asura_db::placement;
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("ASURA_DATABASE_URL")?;
    let pool = placement::connect(&url, 4).await?;
    let pack = pack_root();

    // Every integer is cast to `integer` and every coordinate to `float8` here, so the row type does not
    // depend on which width a migration happened to declare.
    let maps: Vec<(i32, i32, i64, i32, String, f64, f64, i32)> = sqlx::query_as(
        "SELECT map_id, map_type, map_file_id, spawn_count::int, name,
                spawn_x::float8, spawn_y::float8, spawn_plane::int
           FROM t_map_config ORDER BY map_id",
    )
    .fetch_all(&pool)
    .await?;

    let spawns: Vec<(i32, i32, f64, f64, i32)> = sqlx::query_as(
        "SELECT map_id, idx::int, x::float8, y::float8, plane::int
           FROM t_map_spawn ORDER BY map_id, idx",
    )
    .fetch_all(&pool)
    .await?;
    let portals: Vec<(i32, i32, f64, f64, Option<i32>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT map_id, idx::int, x::float8, y::float8, to_map, arrive_x::float8, arrive_y::float8
           FROM t_map_portal ORDER BY map_id, idx",
    )
    .fetch_all(&pool)
    .await?;

    let mut spawn_list: BTreeMap<i32, Vec<Value>> = BTreeMap::new();
    for (map_id, idx, x, y, plane) in spawns {
        spawn_list.entry(map_id).or_default().push(json!(
            {"idx": idx, "x": x, "y": y, "plane": plane}
        ));
    }

    let mut arches: BTreeMap<i32, Vec<Value>> = BTreeMap::new();
    let mut arrivals: BTreeMap<i32, Vec<Value>> = BTreeMap::new();
    for (map_id, idx, x, y, to_map, arrive_x, arrive_y) in portals {
        arches.entry(map_id).or_default().push(json!({
            "idx": idx, "x": x, "y": y, "to_map": to_map
        }));
        if let (Some(to), Some(ax), Some(ay)) = (to_map, arrive_x, arrive_y) {
            arrivals.entry(to).or_default().push(json!({
                "from": map_id, "x": ax, "y": ay
            }));
        }
    }

    let folders = map_folders(&pack);
    let mut written = 0;
    let mut missing = Vec::new();
    for (map_id, map_type, map_file_id, _, name, _, _, _) in &maps {
        let Some(folder) = folders.get(map_id) else {
            missing.push(*map_id);
            continue;
        };
        // The map itself and nothing about where a character stands: no count, no one-spawn copy. A
        // spawn lives in `spawns.json` and nowhere else, and its count is that list's length.
        let basic_path = folder.join("basic.json");
        let mut basic: Value = match std::fs::read_to_string(&basic_path) {
            Ok(text) => serde_json::from_str(&text)?,
            Err(_) => json!({ "map_id": map_id }),
        };
        if let Some(object) = basic.as_object_mut() {
            for stale in ["spawn_count", "spawn_x", "spawn_y", "spawn_plane"] {
                object.remove(stale);
            }
        }
        basic["map_id"] = json!(map_id);
        basic["name"] = json!(name);
        basic["map_type"] = json!(map_type);
        basic["map_file_id"] = json!(map_file_id);
        write(&basic_path, &basic)?;

        let spawns_here = spawn_list.remove(map_id).unwrap_or_default();
        write(
            &folder.join("spawns.json"),
            &json!({ "map_id": map_id, "spawns": spawns_here }),
        )?;

        let arches_here = arches.remove(map_id).unwrap_or_default();
        let arrivals_here = arrivals.remove(map_id).unwrap_or_default();
        write(
            &folder.join("portals.json"),
            &json!({ "map_id": map_id, "arches": arches_here, "arrivals": arrivals_here }),
        )?;
        written += 1;
    }

    println!(
        "wrote {written} map folders; {} ids have no folder: {missing:?}",
        missing.len()
    );
    let leftovers: Vec<i32> = spawn_list.keys().copied().collect();
    if !leftovers.is_empty() {
        println!("map ids with spawns but no map row: {leftovers:?}");
    }
    Ok(())
}

fn write(path: &std::path::Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    std::fs::write(path, text)?;
    Ok(())
}

fn pack_root() -> std::path::PathBuf {
    if let Ok(pack) = std::env::var("ASURA_WORLD_PACK") {
        return std::path::PathBuf::from(pack);
    }
    let cwd = std::env::current_dir().expect("a working directory");
    for candidate in [cwd.join("config/World/Vanilla"), cwd.join("../../config/World/Vanilla")] {
        if candidate.is_dir() {
            return candidate;
        }
    }
    cwd.join("config/World/Vanilla")
}

fn map_folders(pack: &std::path::Path) -> BTreeMap<i32, std::path::PathBuf> {
    let mut found = BTreeMap::new();
    walk(&pack.join("maps"), &mut found);
    found
}

fn walk(dir: &std::path::Path, found: &mut BTreeMap<i32, std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<i32>().ok())
        {
            Some(map_id) => {
                found.insert(map_id, path);
            }
            None => walk(&path, found),
        }
    }
}

