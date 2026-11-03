//! Walk a path out of the bundled graph and print its waypoints, to compare with a capture.
//!
//! ```text
//! cargo run -p asura-world --example walk_probe -- 113021 10381.7 6476.0 0 11000.0 6000.0 0
//! ```
//!
//! The first argument is the map's **blob file id** (Lakeside County is `0x1B97D` = 113021, shared by
//! maps 146 and 148). The point of the tool is the planes and traps a walk comes back with: a capture
//! shows a walk's waypoints in order, and a difference here is a difference in what the client is told.
//! `tools/path-oracle.exe` prints the same thing from GWPS's own `PathFinding`, for comparison.

use asura_world::pathfind::Pos;
use asura_world::{MapEntry, PathGraphs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 6 {
        eprintln!(
            "usage: walk_probe <blob file id> <from x> <from y> <from plane> <to x> <to y> [to plane]"
        );
        return Ok(());
    }
    let blob: u32 = args[0].parse()?;
    let from = Pos {
        x: args[1].parse()?,
        y: args[2].parse()?,
        plane: args[3].parse()?,
    };
    let to = Pos {
        x: args[4].parse()?,
        y: args[5].parse()?,
        plane: args
            .get(6)
            .map(|plane| plane.parse())
            .transpose()?
            .unwrap_or(0),
    };

    let graphs = PathGraphs::from_dump("data/path-graphs.bin")?;
    // The dump is keyed by blob id; bind it under a map id equal to itself so `walk` finds it.
    let map_id = i32::try_from(blob)?;
    let bound = graphs.bind(&[MapEntry {
        map_id,
        map_file_id: blob,
        map_type: 0,
        spawn: (from.x, from.y),
        spawn_plane: from.plane,
        name: "probe".to_string(),
        capacity: 1,
    }]);
    println!("bound {bound} map(s) for blob {blob}");

    match graphs.walk(map_id, from, to) {
        asura_world::Walk::Found(waypoints) => {
            println!("{} waypoint(s)", waypoints.len());
            for (index, waypoint) in waypoints.iter().enumerate() {
                println!(
                    "  {index:2} x={:9.3} y={:9.3} plane={:3} trap={}",
                    waypoint.pos.x, waypoint.pos.y, waypoint.pos.plane, waypoint.trap_id
                );
            }
        }
        asura_world::Walk::NoPath => println!("no path"),
        asura_world::Walk::NoGraph => println!("this blob is not in the dump"),
    }
    Ok(())
}
