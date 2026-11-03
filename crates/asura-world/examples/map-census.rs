//! Import every map the reference's config names out of a real `Gw.dat`, and say whether its spawn point
//! lands inside the graph, using `asura_world::map_data`.
//!
//! ```text
//! cargo run -p asura-world --example map-census -- C:\Users\Apo\Documents\GitHub\PreSearing\Gw.dat
//! cargo run -p asura-world --example map-census -- C:\Users\Apo\Documents\GitHub\PreSearing\Gw.dat --fingerprint
//! ```
//!
//! The catalogue is `tools/map-config.sql`, generated from the reference's own tables (`GmMaps.h`,
//! `GmMapsConfig.c`, the alias table in `GmCommands.c`). Each row names a blob and the position a client is
//! put down at, so this is the measurement that a ported importer produced the reference's graph rather
//! than bytes that merely parse: the reference's spawn point has to be in it.
//!
//! `--fingerprint` prints the row shape the reference's own `tools/gwdat-pathing.c` prints --
//! `file_id,size,planes,trapezoids,min_x,min_y,max_x,max_y` -- so the two can be compared line for line.
//! That is what `tools/parity-pathing.py` does, and what the fixture beside the tests holds.
//!
//! Exit code 0 only when every row's spawn point resolves to a trapezoid -- the battery runs it for that.

use std::collections::HashMap;

use asura_world::gwdat::GwDat;
use asura_world::map_data::{PathStaticData, Vec2f};

/// One row of the catalogue: what `gen-map-config.py` writes, minus the columns this does not need.
struct MapRow {
    map_id: i32,
    file_id: u32,
    spawn: Vec2f,
    plane: u16,
    name: String,
}

/// The `VALUES` rows of the generated catalogue.
///
/// Splitting on commas is not enough -- one name is `Kamadan, Jewel of Istan` -- so the walk keeps single
/// quotes in mind and only treats a comma outside them as a separator.
fn parse_catalogue(sql: &str) -> Vec<MapRow> {
    let mut rows = Vec::new();
    for line in sql.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('(') else {
            continue;
        };
        let Some(body) = rest.split(')').next() else {
            continue;
        };

        let mut fields = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        for character in body.chars() {
            match character {
                '\'' => {
                    quoted = !quoted;
                    current.push(character);
                }
                ',' if !quoted => {
                    fields.push(current.trim().to_string());
                    current.clear();
                }
                _ => current.push(character),
            }
        }
        fields.push(current.trim().to_string());
        if fields.len() != 8 {
            continue;
        }

        let (Ok(map_id), Ok(file_id), Ok(x), Ok(y), Ok(plane)) = (
            fields[0].parse::<i32>(),
            fields[2].parse::<u32>(),
            fields[4].parse::<f32>(),
            fields[5].parse::<f32>(),
            fields[6].parse::<u16>(),
        ) else {
            continue;
        };
        rows.push(MapRow {
            map_id,
            file_id,
            spawn: Vec2f { x, y },
            plane,
            name: fields[7].trim_matches('\'').to_string(),
        });
    }
    rows
}

/// The fingerprint the reference's `tools/gwdat-pathing.c` prints: plane count, trapezoid count, and the
/// box every trapezoid corner falls in. Three numbers that must agree exactly when both sides parsed the
/// same blob.
fn fingerprint_row(file_id: u32, size: usize, data: &PathStaticData) -> String {
    let mut trapezoids = 0usize;
    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for plane in &data.planes {
        for trapezoid in &plane.trapezoids {
            for x in [trapezoid.xtl, trapezoid.xtr, trapezoid.xbl, trapezoid.xbr] {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
            }
            for y in [trapezoid.yt, trapezoid.yb] {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
            trapezoids += 1;
        }
    }
    format!(
        "{file_id:#010X},{size},{},{trapezoids},{min_x:.3},{min_y:.3},{max_x:.3},{max_y:.3}",
        data.planes.len()
    )
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: map-census <Gw.dat> [--fingerprint] [map-config.sql]");
        std::process::exit(2);
    };
    let mut fingerprint = false;
    let mut catalogue = None;
    for arg in args {
        match arg.as_str() {
            "--fingerprint" => fingerprint = true,
            other => catalogue = Some(other.to_string()),
        }
    }
    let catalogue_path = catalogue.unwrap_or_else(|| {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|crates| crates.parent())
            .expect("the crate is inside the repository");
        repo.join("tools")
            .join("map-config.sql")
            .to_string_lossy()
            .into_owned()
    });

    let sql = match std::fs::read_to_string(&catalogue_path) {
        Ok(sql) => sql,
        Err(err) => {
            eprintln!("could not read {catalogue_path}: {err}");
            std::process::exit(1);
        }
    };
    let rows = parse_catalogue(&sql);
    if rows.is_empty() {
        eprintln!("{catalogue_path} has no map rows");
        std::process::exit(1);
    }

    let mut archive = match GwDat::open(&path) {
        Ok(archive) => archive,
        Err(err) => {
            eprintln!("could not open {path}: {err}");
            std::process::exit(1);
        }
    };
    if !fingerprint {
        println!("archive:   {}", archive.path());
        println!("catalogue: {catalogue_path}, {} map(s)", rows.len());
        println!();
        println!(
            "  {:>4}  {:<10}  {:>6}  {:>7}  {:>7}  {:>7}  {:>6}  {:>7}  {:<28}  spawn",
            "map", "blob", "planes", "traps", "xnodes", "ynodes", "sinks", "portals", "name"
        );
    } else {
        println!("file_id,size,planes,trapezoids,min_x,min_y,max_x,max_y");
    }

    let mut imported: HashMap<u32, PathStaticData> = HashMap::new();
    let mut decompressed: HashMap<u32, usize> = HashMap::new();
    let mut printed: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut landed = 0usize;
    let mut failures = 0usize;

    for row in &rows {
        if let std::collections::hash_map::Entry::Vacant(slot) = imported.entry(row.file_id) {
            match archive.file(row.file_id) {
                Ok(Some(content)) => match PathStaticData::import(&content) {
                    Ok(data) => {
                        decompressed.insert(row.file_id, content.len());
                        slot.insert(data);
                    }
                    Err(err) => {
                        println!(
                            "  {:>4}  {:#010X}  {:<10}  import failed: {err}  {}",
                            row.map_id, row.file_id, "-", row.name
                        );
                        failures += 1;
                        continue;
                    }
                },
                Ok(None) => {
                    println!(
                        "  {:>4}  {:#010X}  {:<10}  not in the archive  {}",
                        row.map_id, row.file_id, "-", row.name
                    );
                    failures += 1;
                    continue;
                }
                Err(err) => {
                    println!(
                        "  {:>4}  {:#010X}  {:<10}  read failed: {err}  {}",
                        row.map_id, row.file_id, "-", row.name
                    );
                    failures += 1;
                    continue;
                }
            }
        }
        let data = &imported[&row.file_id];

        let found = data.trapezoid_at(row.spawn, row.plane);
        let verdict = match found {
            Some((plane, trap)) => {
                landed += 1;
                format!(
                    "({:.1}, {:.1}) plane {plane} -> trapezoid {trap}",
                    row.spawn.x, row.spawn.y
                )
            }
            None => {
                failures += 1;
                format!(
                    "({:.1}, {:.1}) plane {} NOT IN THE GRAPH",
                    row.spawn.x, row.spawn.y, row.plane
                )
            }
        };
        if fingerprint {
            // One row per blob, printed at the first map that names it: the reference's own shape, and the
            // order its scanner prints.
            let first = !printed.contains(&row.file_id);
            if first {
                printed.insert(row.file_id);
                println!(
                    "{}",
                    fingerprint_row(row.file_id, decompressed[&row.file_id], data)
                );
            }
            continue;
        }
        let count = |pick: fn(&asura_world::map_data::Plane) -> usize| {
            data.planes.iter().map(pick).sum::<usize>()
        };
        println!(
            "  {:>4}  {:#010X}  {:>6}  {:>7}  {:>7}  {:>7}  {:>6}  {:>7}  {:<28}  {verdict}",
            row.map_id,
            row.file_id,
            data.planes.len(),
            count(|plane| plane.trapezoids.len()),
            count(|plane| plane.xnodes.len()),
            count(|plane| plane.ynodes.len()),
            count(|plane| plane.sink_nodes.len()),
            count(|plane| plane.portals.len()),
            row.name
        );
    }
    if fingerprint {
        if failures > 0 {
            std::process::exit(1);
        }
        return;
    }

    // The same blob answers more than one map (Lakeside County is also Ascalon City and Ashford Abbey), so
    // the import count is per blob and the spawn count is per row.
    println!();
    println!(
        "{} of {} map(s) have their spawn point inside the graph; {} blob(s) imported, {} trapezoid(s) in all",
        landed,
        rows.len(),
        imported.len(),
        imported
            .values()
            .map(|data| data.traps_count as usize)
            .sum::<usize>()
    );
    if failures > 0 {
        std::process::exit(1);
    }
}
