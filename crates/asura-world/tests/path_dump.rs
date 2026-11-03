//! The path-graph dump: a graph written once (`asura-world --dump-path-graphs`) must answer exactly
//! the walks the `Gw.dat` import answers - otherwise the dump would be a different pathfinder.
//!
//! These run against a local client archive (`ASURA_GW_DAT`, `./Gw.dat` or `data/Gw.dat`) and skip
//! cleanly without one, like the other live-database tests.

use std::path::PathBuf;

use asura_world::graph_dump::{self, DumpedGraph};
use asura_world::gwdat::GwDat;
use asura_world::map_data::PathStaticData;
use asura_world::pathfind::Pos;
use asura_world::{MapEntry, PathGraphs, Walk};

/// The reference's Lakeside County blob: maps 146 and 148 share it, which is why a graph is keyed by
/// blob id and a dump of it binds both maps.
const BLOB: u32 = 0x1B97D;

fn gw_dat_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ASURA_GW_DAT") {
        return Some(PathBuf::from(path));
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .to_path_buf();
    [PathBuf::from("Gw.dat"), repo.join("data").join("Gw.dat")]
        .into_iter()
        .find(|candidate| candidate.exists())
}

fn map(map_id: i32) -> MapEntry {
    MapEntry {
        map_id,
        map_file_id: BLOB,
        map_type: 1,
        spawn: (6602.0, 4485.0),
        spawn_plane: 0,
        name: "Lakeside County".to_string(),
        capacity: 2,
    }
}

fn pos(x: f32, y: f32) -> Pos {
    Pos { x, y, plane: 0 }
}

/// A walk reduced to a comparable string: the waypoints' trapezoid ids and exact float bits.
fn digest(walk: &Walk) -> String {
    match walk {
        Walk::Found(waypoints) => format!(
            "found {}",
            waypoints
                .iter()
                .map(|waypoint| format!(
                    "{}:{}:{}:{}",
                    waypoint.trap_id,
                    waypoint.pos.x.to_bits(),
                    waypoint.pos.y.to_bits(),
                    waypoint.pos.plane
                ))
                .collect::<Vec<_>>()
                .join(";")
        ),
        other => format!("{other:?}"),
    }
}

#[test]
fn a_dumped_graph_answers_the_same_walks_as_the_archive() {
    let Some(archive) = gw_dat_path() else {
        eprintln!("skipped: no local Gw.dat (set ASURA_GW_DAT or copy one into data/)");
        return;
    };
    let archive = archive.to_string_lossy().into_owned();
    let maps = vec![map(146), map(148)];

    let original = PathGraphs::load(&archive, &maps).expect("open the archive");
    let mut bound = original.maps();
    bound.sort_unstable();
    assert_eq!(
        bound,
        vec![146, 148],
        "both maps are bound to the one shared blob"
    );

    // Dump the shared blob once, the way `--dump-path-graphs` writes it.
    let mut archive_file = GwDat::open(&archive).expect("open the archive");
    let content = archive_file
        .file(BLOB)
        .expect("read the blob")
        .expect("the blob is in this archive");
    let data = PathStaticData::import(&content).expect("import the blob");
    let dumped = vec![DumpedGraph {
        map_id: 146,
        map_file_id: BLOB,
        data,
    }];
    let path = std::env::temp_dir().join(format!("asura-path-dump-{}.bin", std::process::id()));
    std::fs::write(&path, graph_dump::encode(&dumped)).expect("write the dump");

    let from_dump = PathGraphs::from_dump(path.to_str().expect("a path")).expect("read the dump");
    assert_eq!(
        from_dump.bind(&maps),
        2,
        "the dump binds both maps that share the blob"
    );
    std::fs::remove_file(&path).ok();

    // Points on and around Lakeside County, including the reference's spawn and one pair that is
    // the same point twice.
    let pairs = [
        (6602.0, 4485.0, 2781.3, 7637.7),
        (2781.3, 7637.7, 9541.9, 4796.5),
        (633.0, 7270.0, 22614.0, 6849.0),
        (0.0, 0.0, 100.0, 100.0),
        (6602.0, 4485.0, 6602.0, 4485.0),
    ];
    for map_id in [146, 148] {
        for (sx, sy, dx, dy) in pairs {
            let from = pos(sx, sy);
            let to = pos(dx, dy);
            assert_eq!(
                digest(&original.walk(map_id, from, to)),
                digest(&from_dump.walk(map_id, from, to)),
                "map {map_id}: ({sx}, {sy}) -> ({dx}, {dy}) must answer identically"
            );
        }
    }
}

/// The bundled dump works out of the box: no archive, no database - `data/path-graphs.bin` is
/// committed with the server, so the repository alone can answer pathfinding.
#[test]
fn the_bundled_dump_answers_walks_without_an_archive() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("a crate directory")
        .parent()
        .expect("a repository directory")
        .to_path_buf();
    let dump = repo.join("data").join("path-graphs.bin");
    if !dump.exists() {
        eprintln!("skipped: the bundled dump is absent");
        return;
    }
    let graphs =
        PathGraphs::from_dump(dump.to_str().expect("a path")).expect("the bundled dump loads");
    let maps = vec![map(146), map(148)];
    assert_eq!(
        graphs.bind(&maps),
        2,
        "the bundled dump binds both pre-Searing maps on the shared blob"
    );
    let walk = graphs.walk(146, pos(6602.0, 4485.0), pos(2781.3, 7637.7));
    assert!(
        !matches!(walk, Walk::NoGraph),
        "the bundled dump must answer walks, got {walk:?}"
    );
}

/// The real command: `asura-world --dump-path-graphs` writes a file this crate can read back, and
/// the catalogue binds to it - so a world can serve maps from the dump with no `Gw.dat` at all.
///
/// Needs the database (the catalogue is read for the map list) and the archive; skips without either.
#[tokio::test]
async fn the_dump_command_writes_a_loadable_file() {
    let Some(archive) = gw_dat_path() else {
        eprintln!("skipped: no local Gw.dat (set ASURA_GW_DAT or copy one into data/)");
        return;
    };
    let Some(url) = asura_db::config::database_url() else {
        eprintln!("skipped: ASURA_DATABASE_URL not set");
        return;
    };
    let pool = asura_db::placement::connect(&url, 2)
        .await
        .expect("connect the database");
    let out = std::env::temp_dir().join(format!("asura-dump-cli-{}.bin", std::process::id()));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_asura-world"))
        .args([
            "--dump-path-graphs",
            out.to_str().expect("a path"),
            "--gwdat",
            archive.to_str().expect("a path"),
        ])
        .status()
        .expect("run asura-world");
    assert!(status.success(), "the dump command exited {status}");

    let graphs =
        PathGraphs::from_dump(out.to_str().expect("a path")).expect("read the file it wrote");
    std::fs::remove_file(&out).ok();
    assert!(graphs.imported() >= 1, "the dump loaded no graph");

    // The world binds its catalogue to the dump, exactly as `--path-graphs` does at startup.
    let mut entries = Vec::new();
    for map_id in asura_db::game::configured_map_ids(&pool)
        .await
        .expect("read the catalogue")
    {
        if let Some(configured) = asura_db::game::spawn_map(&pool, map_id)
            .await
            .expect("read a map row")
        {
            entries.push(MapEntry {
                map_id,
                map_file_id: u32::try_from(configured.map_file_id).unwrap_or(0),
                map_type: configured.map_type,
                spawn: (configured.spawn_x, configured.spawn_y),
                spawn_plane: configured.spawn_plane as u16,
                name: configured.name.clone(),
                capacity: 2,
            });
        }
    }
    let bound = graphs.bind(&entries);
    assert!(
        bound > 0,
        "no catalogue map is bound to the dump ({} graph(s) loaded)",
        graphs.imported()
    );
    // A bound map answers a walk from the dump rather than refusing it for lack of a graph.
    let map_id = graphs.maps()[0];
    let entry = entries
        .iter()
        .find(|entry| entry.map_id == map_id)
        .expect("the bound map is in the catalogue");
    if let Walk::NoGraph = graphs.walk(
        map_id,
        pos(entry.spawn.0, entry.spawn.1),
        pos(entry.spawn.0 + 50.0, entry.spawn.1 + 50.0),
    ) {
        panic!("map {map_id} is bound but its dump answered NoGraph");
    }
}
