//! Run this repo's ported path finder over a list of cases, in the shape the reference's oracle prints.
//!
//! ```text
//! cargo run -p asura-world --example path-census -- <Gw.dat> <cases file>
//! ```
//!
//! The cases file is one case per line -- `blob src_x src_y src_plane dst_x dst_y dst_plane` -- and the
//! output is one CSV row per case:
//!
//! ```text
//! blob,src_x,src_y,src_plane,dst_x,dst_y,dst_plane,result,count,waypoints
//! ```
//!
//! with positions printed as raw IEEE-754 bit patterns in hex, exactly like `tools/path-oracle.c`, so the
//! two outputs can be compared byte for byte. That comparison is the whole point: an A* that agrees on
//! "yes, you can walk there" but picks a different corridor is a different server.

use std::collections::HashMap;

use asura_world::gwdat::GwDat;
use asura_world::map_data::PathStaticData;
use asura_world::pathfind::{PathContext, Pos, Waypoint};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(archive_path), Some(cases_path)) = (args.next(), args.next()) else {
        eprintln!("usage: path-census <Gw.dat> <cases file>");
        std::process::exit(2);
    };

    let cases = match std::fs::read_to_string(&cases_path) {
        Ok(cases) => cases,
        Err(err) => {
            eprintln!("could not read {cases_path}: {err}");
            std::process::exit(1);
        }
    };
    let mut archive = match GwDat::open(&archive_path) {
        Ok(archive) => archive,
        Err(err) => {
            eprintln!("could not open {archive_path}: {err}");
            std::process::exit(1);
        }
    };

    // Import a blob once. The finder's arrays are cheap to rebuild, and rebuilding them per case keeps the
    // borrow of the imported graph unambiguous.
    let mut maps: HashMap<u32, (std::sync::Arc<PathStaticData>, Vec<Waypoint>)> = HashMap::new();

    println!("blob,src_x,src_y,src_plane,dst_x,dst_y,dst_plane,result,count,waypoints");

    let mut ran = 0usize;
    let mut solved = 0usize;
    for (number, line) in cases.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() != 7 {
            eprintln!("line {}: expected seven fields: {line:?}", number + 1);
            continue;
        }
        let parse = |text: &str| -> f32 {
            text.parse::<f32>()
                .unwrap_or_else(|_| panic!("line {}: {text:?} is not a number", number + 1))
        };
        let blob = u32::from_str_radix(fields[0].trim_start_matches("0x"), 16)
            .unwrap_or_else(|_| panic!("line {}: {:?} is not a file id", number + 1, fields[0]));
        let src = Pos {
            x: parse(fields[1]),
            y: parse(fields[2]),
            plane: fields[3].parse().expect("a plane"),
        };
        let dst = Pos {
            x: parse(fields[4]),
            y: parse(fields[5]),
            plane: fields[6].parse().expect("a plane"),
        };

        if let std::collections::hash_map::Entry::Vacant(slot) = maps.entry(blob) {
            let content = match archive.file(blob) {
                Ok(Some(content)) => content,
                Ok(None) => {
                    eprintln!("{blob:#010X}: not in the archive");
                    continue;
                }
                Err(err) => {
                    eprintln!("{blob:#010X}: {err}");
                    continue;
                }
            };
            let data = match PathStaticData::import(&content) {
                Ok(data) => data,
                Err(err) => {
                    eprintln!("{blob:#010X}: {err}");
                    continue;
                }
            };
            slot.insert((std::sync::Arc::new(data), Vec::new()));
        }

        // The data lives in `maps` for the whole run; the finder borrows it for one case at a time, which
        // is the lifetime the reference's own context has.
        let (data, waypoints) = maps.get_mut(&blob).expect("just inserted");
        waypoints.clear();
        let mut finder = PathContext::new(std::sync::Arc::clone(data));
        let solved_this = finder.path_finding(src, dst, waypoints);

        let bits = |value: f32| value.to_bits();
        print!(
            "{blob:#010X},{:08X},{:08X},{},{:08X},{:08X},{},{},{},",
            bits(src.x),
            bits(src.y),
            src.plane,
            bits(dst.x),
            bits(dst.y),
            dst.plane,
            i32::from(solved_this),
            waypoints.len()
        );
        for (index, waypoint) in waypoints.iter().enumerate() {
            if index != 0 {
                print!(";");
            }
            print!(
                "{},{:08X},{:08X},{}",
                waypoint.trap_id,
                bits(waypoint.pos.x),
                bits(waypoint.pos.y),
                waypoint.pos.plane
            );
        }
        println!();

        ran += 1;
        if solved_this {
            solved += 1;
        }
    }

    eprintln!("done: {ran} case(s), {solved} solved");
}
