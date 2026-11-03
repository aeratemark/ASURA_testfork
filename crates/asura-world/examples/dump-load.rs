//! Print the load sequence a client would receive, byte for byte, without a socket.
//!
//! This exists to be **diffed against the reference's own log**: when a client dies mid-load, the
//! question is which message differs, and the answer is in the bytes. `GWPS` writes one
//! `SEND 0x017C   6 bytes : 7C 01 3F 3F 00 00` line per message; this prints the same shape for the
//! same blocks, so `tools/compare-load.py` can walk the two side by side.
//!
//! ```text
//! cargo run -p asura-world --example dump-load -- --map 146 --name Appok
//! ```
//!
//! The defaults are the values the reference session used (map 146, the reference's character name,
//! its blob and spawn), and the inventory is the reference's own default set - read from the same rows
//! the server seeds, not replayed, so this dump is what the *built* blocks contain. A diff against the
//! reference's capture is therefore a diff of content and not of a replayed file.

use asura_protocol::client::Frame;
use asura_world::{items_block, load_block, player_block, spawn_block, MapEntry, Standing};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let map_id: i32 = argument(&args, "--map")
        .and_then(|value| value.parse().ok())
        .unwrap_or(146);
    let name = argument(&args, "--name").unwrap_or_else(|| "Appok".to_string());

    // The reference's own map: Lakeside County, blob 0x1B97D, spawn (6602, 4485) on plane 0. Its
    // spawn point message is the one that has to match for the client to load the map at all.
    let map = MapEntry {
        map_id,
        map_file_id: 0x1B97D,
        map_type: 0,
        spawn: (6602.0, 4485.0),
        spawn_plane: 0,
        name: "Lakeside County".to_string(),
        capacity: 8,
    };
    let standing = Standing::from_map(&map);
    // The reference's default inventory, as rows: the same six bags and nine items the server seeds.
    let inventory = asura_db::inventory::default_inventory();

    println!("# the four messages sent unasked");
    dump(&load_block(&map, &name, asura_world::AGENT_ID));
    println!("# the answer to 0x8091 (items)");
    dump(&items_block(&map, &inventory, 0));
    println!("# the answer to 0x8088 (spawn)");
    dump(&spawn_block(&map, standing));
    println!("# the answer to 0x8090 (players)");
    // The character the reference session used, as its own row held it: a Warrior (`appearance` 0x03100000,
    // `primary_profession` 1, `unlocked_professions` `1 << 1`), campaign Prophecies, level 1 -- the values
    // its character-settings blob decodes to (see `CHARACTER_SETTINGS_EXTENDED`). Passing the *default*
    // traits instead made `PLAYER_UPDATE_PROFESSION` and `_UNLOCKED_PROFESSIONS` differ from the capture by
    // exactly those two fields, which was a difference in this dump rather than in the server.
    let traits = asura_db::game::Traits {
        appearance: 0x0310_0000,
        profession: 1,
        unlocked_professions: 2,
        gold: 0,
        campaign: 1,
        secondary_profession: 0,
        level: 1,
    };
    dump(&player_block(
        &name,
        standing,
        &inventory,
        std::time::Instant::now(),
        asura_world::PLAYER_ID,
        asura_world::AGENT_ID,
        traits,
        asura_world::map::UNLOCKED_MAP_ZEROS,
    ));
}

fn dump(frames: &[Frame]) {
    for frame in frames {
        let bytes = frame.encode();
        let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
        println!(
            "SEND 0x{:04X} {:3} bytes : {}",
            frame.header(),
            bytes.len(),
            hex.join(" ")
        );
    }
}

fn argument(args: &[String], name: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == name)?;
    args.get(index + 1).cloned()
}
