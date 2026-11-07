//! A world server -'- AzerothCore calls this a *realm*.
//!
//! Owns: the gameplay connection to the client, the instances it hosts, and (from M5) character
//! persistence. The auth service owns placement; this service publishes what it can host and answers
//! the clients that are sent to it.
//!
//! The startup order is enforced here rather than described (`docs/02-architecture.md`), and it is
//! the sequence a restart has to follow:
//!
//! 1. read the configuration: world id, bind address, the address to advertise, and the maps;
//! 2. open the database pool;
//! 3. **apply the migrations the database is missing**, then **refuse to run against a database still
//!    older than this build** (a migration that failed);
//! 4. load each map's config (a map without a `map_file_id` cannot be loaded by a client);
//! 5. register in `world` and publish pool capacity, start the heartbeat, then accept connections.
//!
//! Step 3 first, and step 5 last: a world that is in the registry is a world that is ready. That is
//! what makes a travel arriving two seconds after boot get a correct answer instead of a race.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use asura_db::migrations::{self, EnsureError};
use asura_db::{game as db_game, placement};
use asura_protocol::Trace;
use asura_world::{serve, MapEntry, World};

/// How often the world tells the database it is alive. Placement stops offering a world whose
/// heartbeat is older than a minute, so this is comfortably inside that window.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

const DEFAULT_WORLD_ID: i32 = 8001;
const DEFAULT_BIND: &str = "127.0.0.1:6113";
const DEFAULT_CAPACITY: i32 = 8;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("asura-world {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }

    let world_id = match argument(&args, "--world") {
        Some(value) => match value.parse::<i32>() {
            Ok(id) => id,
            Err(err) => {
                eprintln!("asura-world: --world {value:?} is not an id: {err}");
                return ExitCode::from(2);
            }
        },
        None => DEFAULT_WORLD_ID,
    };

    let bind = match argument(&args, "--bind").or_else(|| std::env::var("ASURA_WORLD_BIND").ok()) {
        Some(value) => match value.trim().parse::<SocketAddr>() {
            Ok(address) => address,
            Err(err) => {
                eprintln!("asura-world: --bind {value:?} is not an address: {err}");
                return ExitCode::from(2);
            }
        },
        None => DEFAULT_BIND.parse().expect("the default address parses"),
    };

    // The address a *client* dials. It is separate from `--bind` on purpose: a server behind a NAT
    // listens on a private address and must publish the public one, exactly as a realmlist does.
    let public: SocketAddr =
        match argument(&args, "--public").or_else(|| std::env::var("ASURA_WORLD_PUBLIC").ok()) {
            Some(value) => match value.trim().parse::<SocketAddr>() {
                Ok(address) => address,
                Err(err) => {
                    eprintln!("asura-world: --public {value:?} is not an address: {err}");
                    return ExitCode::from(2);
                }
            },
            None => bind,
        };

    let capacity = argument(&args, "--capacity")
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(DEFAULT_CAPACITY);

    // `--on-demand`: load a catalogue map this world was not started with, instead of refusing it -
    // for a `/tp` into it, and for a login whose character stands in it (both are the same refusal
    // when this is off). Off by default, because preloading every destination is what the reference
    // does (one pre-spawned server per map) and what this world does at startup; the switch exists
    // to measure whether the client actually needs the destination ready before the hop.
    //
    // With no `--maps`, on-demand means **nothing is preloaded**: the world starts empty and every
    // map is loaded the first time a client names it. That is the "no caching" side of the
    // experiment, and it needs no map ids on the command line at all.
    let load_on_demand = args.iter().any(|arg| arg == "--on-demand")
        || matches!(
            std::env::var("ASURA_WORLD_ON_DEMAND").ok().as_deref(),
            Some("1" | "true" | "yes" | "on")
        );

    // `--maps` names a subset to preload; without it the world preloads **every** map the catalogue
    // has, which is what the reference does (one server per map, all of them pre-spawned at startup) -
    // unless `--on-demand` asks for the opposite (nothing preloaded).
    let maps_arg: Option<Vec<i32>> = argument(&args, "--maps").map(|value| {
        value
            .split(',')
            .filter(|part| !part.trim().is_empty())
            .filter_map(|part| match part.trim().parse::<i32>() {
                Ok(map_id) => Some(map_id),
                Err(_) => {
                    eprintln!("asura-world: --maps {value:?} is not a comma-separated list of ids");
                    None
                }
            })
            .collect::<Vec<i32>>()
    });
    if maps_arg.as_ref().is_some_and(|maps| maps.is_empty()) {
        eprintln!("asura-world: --maps named no map");
        return ExitCode::from(2);
    }

    let trace = Trace::from_environment();

    // The packet capture: every game message both directions, plaintext JSONL. What a drift or a
    // load failure is diagnosed from when a client-side sniffer is unavailable or in the way.
    let capture_path = argument(&args, "--capture").or_else(|| std::env::var("ASURA_CAPTURE").ok());
    if let Some(path) = &capture_path {
        if let Err(err) = asura_world::capture::install(path) {
            eprintln!("asura-world: cannot capture to {path}: {err}");
            return ExitCode::FAILURE;
        }
        eprintln!("asura-world: capturing both directions to {path}");
    }

    eprintln!("asura-world {} starting", env!("CARGO_PKG_VERSION"));
    eprintln!("asura-world: database ready, connecting");

    // Said once, and never gated. Chat commands answer **here**, in this console, which is the half that
    // has always worked: the reference's `/maps` printed its table to its own log, and no chat text has
    // ever reached a client's screen in either server. Command answers are not sent -- sending them crashed
    // a live client -- while a line a player *speaks* is sent, as the reference sends it.
    eprintln!(
        "asura-world: /maps, /stuck and unknown commands answer in this console; a spoken chat line is sent"
    );

    let Some(database_url) = asura_db::config::database_url() else {
        eprintln!(
            "asura-world: no {}. Set it, or copy .env.example to .env and run tools/db-setup.ps1.",
            asura_db::config::DATABASE_URL_VARIABLE
        );
        return ExitCode::from(2);
    };
    let pool = match placement::connect(&database_url, 8).await {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("asura-world: could not connect to the database: {err}");
            return ExitCode::FAILURE;
        }
    };

    // Bring the schema up to this build before checking it: a feature whose tables were never created
    // fails on its first query, mid-session, far from the cause (a guild creation that disconnected
    // the client with error 7, 2026-11-06). Every migration is idempotent and recorded, so this is a
    // no-op on a current database.
    match migrations::apply_pending(&pool).await {
        Ok(done) if done.applied.is_empty() => {}
        Ok(done) => eprintln!("asura-world: applied migration(s) {:?}", done.applied),
        Err(err) => {
            eprintln!("asura-world: could not apply the pending migrations: {err}");
            return ExitCode::FAILURE;
        }
    }

    match migrations::ensure_current(&pool).await {
        Ok(()) => {}
        Err(EnsureError::TooOld(too_old)) => {
            eprintln!("asura-world: {too_old}");
            eprintln!("asura-world: refusing to start against a stale schema");
            return ExitCode::from(3);
        }
        Err(EnsureError::Database(err)) => {
            eprintln!("asura-world: could not read the schema version: {err}");
            return ExitCode::FAILURE;
        }
    }

    // The catalogue is always read: it is what pool rows are published for, and what an on-demand
    // world can serve. It is not the same thing as preloading - a catalogue entry is a row in a
    // database, an instance is the map's data in memory.
    let catalogue: Vec<i32> = match db_game::configured_map_ids(&pool).await {
        Ok(ids) => ids,
        Err(err) => {
            eprintln!("asura-world: could not read the map catalogue: {err}");
            return ExitCode::FAILURE;
        }
    };
    if catalogue.is_empty() {
        eprintln!(
            "asura-world: map_config is empty, so there is no map to host; run \
             tools/seed-map.ps1 to write the reference's catalogue"
        );
        return ExitCode::from(4);
    }

    // The maps, with what a client needs to load them. A map named on the command line that the
    // database does not describe is a map a client would load forever, so it is fatal here.
    let map_ids = match maps_arg {
        Some(explicit) => explicit,
        None if load_on_demand => {
            eprintln!(
                "asura-world: no --maps given and --on-demand is on, so NOTHING is preloaded: every \
                 map is loaded the first time a client names it"
            );
            Vec::new()
        }
        None => {
            eprintln!(
                "asura-world: no --maps given, so every map in the catalogue is hosted ({} of them, \
                 the reference's own list)",
                catalogue.len()
            );
            catalogue.clone()
        }
    };

    // The pack's `basic.json` is the authority for the blob a client streams. The database row
    // remains the fallback for a map the pack does not describe; the Northlands crash of 2026-11-06
    // was a database row naming the wrong file id while the pack named the live one (116025).
    let pack_maps = match asura_config::world::catalogue() {
        Ok(maps) => maps,
        Err(err) => {
            eprintln!("asura-world: map descriptions: {err}");
            return ExitCode::from(4);
        }
    };
    let mut maps = Vec::new();
    for map_id in &map_ids {
        match db_game::spawn_map(&pool, *map_id).await {
            Ok(Some(configured)) => {
                let map_file_id = pack_maps
                    .get(map_id)
                    .and_then(|map| map.map_file_id)
                    .unwrap_or_else(|| u32::try_from(configured.map_file_id).unwrap_or(0));
                // A character-creation instance is the one hostable map that loads no blob: measured on a
                // live creation, it sends no spawn point and no map file id at all, only the load head,
                // the player-data block and `CHAR_CREATION_START`. Every other map needs one, because
                // the client is told which blob to load.
                if map_file_id == 0
                    && configured.map_type != asura_world::map_kinds().character_creation
                {
                    eprintln!(
                        "asura-world: map {map_id} has no map_file_id; a client cannot load it"
                    );
                    return ExitCode::from(4);
                }
                let kinds = asura_world::map_kinds();
                if configured.map_type != kinds.main_town
                    && configured.map_type != kinds.main_explorable
                    && configured.map_type != kinds.character_creation
                {
                    // A world hosts what the reference hosts: main towns (0), explorable areas (1) and the
                    // character-creation instance (11). Explorable hosting used to be refused here on the
                    // theory that this server "cannot spawn anything into one" - but a world server does not
                    // spawn content, it hands the client the blob and the spawn point, which is exactly what
                    // the reference does with Lakeside County (146), its verified explorable. Refusing it made
                    // this server unable to host the reference's flagship map.
                    eprintln!(
                        "asura-world: map {map_id} is type {} (main towns, explorable areas and the \
                         character-creation instance are hosted), skipping it",
                        configured.map_type
                    );
                    continue;
                }
                maps.push(MapEntry {
                    map_id: *map_id,
                    map_file_id,
                    map_type: configured.map_type,
                    spawn: (configured.spawn_x, configured.spawn_y),
                    spawn_plane: configured.spawn_plane as u16,
                    spawns: asura_world::map_spawns::for_map(*map_id)
                        .map(<[_]>::to_vec)
                        .unwrap_or_default(),
                    name: configured.name.clone(),
                    capacity,
                });
            }
            Ok(None) => {
                eprintln!(
                    "asura-world: map {map_id} is not in map_config; run tools/seed-map.ps1 to add it"
                );
                return ExitCode::from(4);
            }
            Err(err) => {
                eprintln!("asura-world: could not read map {map_id}: {err}");
                return ExitCode::FAILURE;
            }
        }
    }
    if maps.is_empty() && !load_on_demand {
        eprintln!(
            "asura-world: no map can be hosted; refusing to register a world that cannot serve"
        );
        return ExitCode::from(4);
    }

    // ---- the path-graph dump: write the graphs once and exit -----------------------------------
    // `asura-world --dump-path-graphs FILE` writes the path graphs for the maps this world handles
    // (the catalogue) so a server can run without a Gw.dat. `--dump-all` writes every pathing map in
    // the archive instead. The dump is derived game data: keep it out of the repository, like Gw.dat.
    //
    // **Here, before the bind.** It used to sit after `TcpListener::bind`, which meant a world already
    // running on its port took the bind failure and exited before the dump was ever reached - which is
    // every time, because dumping is something you do while the stack is up. Found 2026-10-29.
    if let Some(out) = argument(&args, "--dump-path-graphs") {
        let all = args.iter().any(|arg| arg == "--dump-all");
        return dump_path_graphs(&pool, &catalogue, &archive_path(&args), &out, all).await;
    }

    // ---- step 5: register, publish, heartbeat, then accept -----------------------------------
    // The pool rows this world publishes. One per map it preloads; with `--on-demand`, one per
    // catalogue map - a database line saying "this world can host map X", so placement can hand it
    // a client for any map. A pool row is not an instance: nothing is loaded by publishing it.
    let capacities: Vec<(i32, i32)> = if load_on_demand {
        catalogue.iter().map(|map_id| (*map_id, capacity)).collect()
    } else {
        maps.iter().map(|map| (map.map_id, capacity)).collect()
    };
    // The build identity (S3): what the rolling deploy compares. `--build` wins, then ASURA_BUILD,
    // then the git description compiled in - never the crate version alone, which was the same
    // string ("asura-world/0.1.0") for every build and told two builds apart by nothing.
    let build = argument(&args, "--build")
        .or_else(|| std::env::var("ASURA_BUILD").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("asura-world/{}", env!("ASURA_BUILD_ID")));
    if let Err(err) = placement::register_world(
        &pool,
        world_id,
        "asura-dev",
        &public.ip().to_string(),
        i32::from(public.port()),
        &build,
        &capacities,
    )
    .await
    {
        eprintln!("asura-world: could not register the world: {err}");
        return ExitCode::FAILURE;
    }
    let preloaded = if maps.is_empty() {
        "none (on-demand: each map loads the first time a client names it)".to_string()
    } else {
        maps.iter()
            .map(|map| map.map_id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    eprintln!(
        "asura-world: registered world {world_id} at {public} for map(s) {preloaded} ({} slot(s) each)",
        capacity
    );

    // The heartbeat. A world that stops sending it stops being offered, which is the point: a dead
    // world that still accepts TCP connections is worse than one that is plainly gone. It is
    // started below, once the world exists: it now also carries the player count and the drain
    // (S3), which need the world itself.

    // The pack is configuration: read once here, and a bad file stops the world before it serves anyone
    // rather than on the first character creation or the first load block.
    if let Err(err) = asura_world::character_defaults::current() {
        eprintln!("asura-world: {err}");
        eprintln!("asura-world: refusing to start without the character defaults");
        return ExitCode::from(4);
    }
    if let Err(err) = asura_world::spawns::current() {
        eprintln!("asura-world: {err}");
        eprintln!("asura-world: refusing to start without the character spawn table");
        return ExitCode::from(4);
    }
    if let Err(err) = asura_config::directional::current() {
        eprintln!("asura-world: directional movement configuration: {err}");
        return ExitCode::from(4);
    }
    // The map spawn lists are content as well - `spawns.json` in each map folder - and they are read
    // here so a bad edit is reported at start-up rather than on somebody's next travel. A map without
    // a file is not an error: its catalogue row's single coordinate answers instead (`Standing`).
    asura_world::map_spawns::table();
    // The doorways and the arrival points are the pack's `portals.json` files, the same tree. Read here
    // for the same reason: only the pack supplies them, and a map without one has no doorways.
    asura_world::portals::table_pack();

    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("asura-world: could not bind {bind}: {err}");
            return ExitCode::FAILURE;
        }
    };

    let replay = match asura_world::replay::Replay::from_environment() {
        Ok(replay) => replay,
        Err(err) => {
            eprintln!("asura-world: could not read the replay capture: {err}");
            return ExitCode::from(5);
        }
    };
    if let Some(replay) = &replay {
        eprintln!(
            "asura-world: replaying the reference's captured stream ({} / {} / {} / {} messages)",
            replay.load.len(),
            replay.items.len(),
            replay.spawn.len(),
            replay.players.len()
        );
    }

    // The path graphs, which is what a `MOVE_TO_COORD` is answered from (`GameSrv_LoadMap` reads the same
    // blob). A map whose blob cannot be imported is served without one, and every walk in it is refused.
    let archive = archive_path(&args);
    // The graphs, in priority order: a dump (a few megabytes, no archive needed) when one was named
    // or a local `data/path-graphs.bin` exists, then the `Gw.dat` import. A dump is written by
    // `--dump-path-graphs`; `ASURA_PATH_GRAPHS` names it, and `--path-graphs` overrides both.
    let dump_path = path_graphs_dump_path(&args);
    let graphs = match &dump_path {
        Some(path) => match asura_world::PathGraphs::from_dump(path) {
            Ok(graphs) => {
                let bound = graphs.bind(&maps);
                eprintln!(
                    "asura-world: path graphs from the dump {path}: {bound} of {} map(s) bound",
                    maps.len()
                );
                graphs
            }
            Err(err) => {
                eprintln!(
                    "asura-world: could not read the path-graph dump {path}: {err}; falling back to {archive}"
                );
                load_graphs_from_archive(&archive, &maps)
            }
        },
        None => load_graphs_from_archive(&archive, &maps),
    };

    // `--on-demand` was read above, before the pool rows were published, so that an on-demand world
    // lists every catalogue map as servable while it preloads only the maps it was started with.

    let world = Arc::new(World {
        pool,
        world_id,
        maps,
        replay,
        instances: Default::default(),
        public,
        graphs,
        lazy_maps: Default::default(),
        load_on_demand,
        capacity,
        archive,
    });

    // The heartbeat, and the drain (docs/16-implementation-plan.md, S3). The heartbeat says the world
    // is alive and how many players it holds; the drain says whether this world should finish its
    // players and leave. Placement excludes a draining world from every offer, and once its last
    // player has gone the world removes its registry row and exits - the deploy never kills a world
    // with players in it, and nobody is kicked.
    {
        let heartbeat_world = Arc::clone(&world);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
            loop {
                ticker.tick().await;
                let population = heartbeat_world.instances.len() as i32;
                if let Err(err) =
                    placement::heartbeat(&heartbeat_world.pool, world_id, population).await
                {
                    eprintln!("asura-world: heartbeat failed: {err}");
                }
                match placement::is_draining(&heartbeat_world.pool, world_id).await {
                    Ok(true) if population == 0 => {
                        if let Err(err) =
                            placement::remove_world(&heartbeat_world.pool, world_id).await
                        {
                            eprintln!("asura-world: could not leave the registry: {err}");
                        }
                        eprintln!("world: drained; the last player has left; leaving the registry");
                        std::process::exit(0);
                    }
                    Ok(_) => {}
                    Err(err) => eprintln!("asura-world: cannot read the draining flag: {err}"),
                }
            }
        });
    }

    if load_on_demand {
        eprintln!(
            "asura-world: on-demand destination loading is ON: a login or a /tp that names a \
             catalogue map this world was not started with loads it there and then and measures \
             the cost"
        );
    }
    eprintln!(
        "asura-world: a travel's is_explorable: {}",
        asura_world::explorable_flag_mode()
    );
    if asura_world::extra_load_messages_suppressed() {
        eprintln!(
            "asura-world: ASURA_LOAD_EXTRA is off: the unasked load block carries no exploration grid \
             and no instance table (measurement only; a client's map surfaces need the grid)"
        );
    }
    if let Err(err) = serve(listener, world, trace.on()).await {
        eprintln!("asura-world: the listener stopped: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Where the client's archive is, for the path graphs: `--gwdat`, then `ASURA_GW_DAT`, then the two
/// places a local copy lives (the working directory, and `data/Gw.dat` in this repository).
///
/// The archive is the game's own asset file and is not distributed with this repository; copy a
/// `Gw.dat` here (or point `ASURA_GW_DAT` at one) and the world imports every map's graph at startup.
/// GWPS reads the same file beside its own binary (`FaArchive.c`), so this is the reference's rule.
/// A world that cannot find it still serves (a client loads a map from its own copy of the same
/// archive), but it refuses every walk instead of inventing one, and says so at startup.
fn archive_path(args: &[String]) -> String {
    if let Some(given) = argument(args, "--gwdat") {
        return given;
    }
    if let Ok(from_env) = std::env::var("ASURA_GW_DAT") {
        return from_env;
    }
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    for candidate in [
        std::path::PathBuf::from("Gw.dat"),
        repository.join("data").join("Gw.dat"),
    ] {
        if candidate.exists() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    "Gw.dat".to_string()
}

/// Where a path-graph dump lives: `--path-graphs`, then `ASURA_PATH_GRAPHS`, then a local
/// `data/path-graphs.bin` (written by `--dump-path-graphs`). `None` means import from the archive.
fn path_graphs_dump_path(args: &[String]) -> Option<String> {
    if let Some(given) = argument(args, "--path-graphs") {
        return Some(given);
    }
    if let Ok(from_env) = std::env::var("ASURA_PATH_GRAPHS") {
        return Some(from_env);
    }
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    for candidate in [
        std::path::PathBuf::from("data").join("path-graphs.bin"),
        repository.join("data").join("path-graphs.bin"),
    ] {
        if candidate.exists() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// Import the graphs out of a `Gw.dat`, logging what was found or why nothing was.
fn load_graphs_from_archive(archive: &str, maps: &[MapEntry]) -> asura_world::PathGraphs {
    match asura_world::PathGraphs::load(archive, maps) {
        Ok(graphs) => {
            let with_graphs = graphs.maps();
            eprintln!(
                "asura-world: path graphs from {archive}: {} of {} map(s) ({})",
                with_graphs.len(),
                maps.len(),
                with_graphs
                    .iter()
                    .map(|map_id| map_id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            graphs
        }
        Err(err) => {
            eprintln!(
                "asura-world: no path graphs, so every walk will be refused: could not open {archive}: \
                 {err}. Set ASURA_GW_DAT or pass --gwdat, or dump the graphs once with \
                 --dump-path-graphs and load them with --path-graphs."
            );
            asura_world::PathGraphs::empty()
        }
    }
}

/// Write the path graphs once, so the world can load them without a `Gw.dat`
/// (`asura-world --dump-path-graphs FILE [--dump-all]`).
///
/// Without `--dump-all` it dumps the maps this world handles (the catalogue): the twelve of the
/// reference, each by its `map_file_id`. With `--dump-all` it walks the whole archive and imports
/// every blob that is a pathing map (a container whose first four bytes are `"ffna"`), naming each
/// by its blob id - the right choice when the catalogue is not what is wanted, or when a map is not
/// in `map_config` yet.
async fn dump_path_graphs(
    pool: &asura_db::PgPool,
    catalogue: &[i32],
    archive_path: &str,
    out: &str,
    all: bool,
) -> ExitCode {
    use asura_world::graph_dump::DumpedGraph;

    let mut archive = match asura_world::gwdat::GwDat::open(archive_path) {
        Ok(archive) => archive,
        Err(err) => {
            eprintln!("asura-world: cannot open {archive_path} for the dump: {err}");
            return ExitCode::from(6);
        }
    };

    let mut graphs: Vec<DumpedGraph> = Vec::new();
    let mut failed = 0usize;
    if all {
        let ids = archive.file_ids();
        eprintln!(
            "asura-world: dumping every pathing map in {archive_path} ({} blob(s) to try)",
            ids.len()
        );
        for file_id in ids {
            match archive.file(file_id) {
                Ok(Some(content)) => {
                    if asura_world::map_data::riff_type(&content).is_none() {
                        continue;
                    }
                    match asura_world::map_data::PathStaticData::import(&content) {
                        Ok(data) => graphs.push(DumpedGraph {
                            map_id: -1,
                            map_file_id: file_id,
                            data,
                        }),
                        Err(_) => failed += 1,
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    eprintln!("asura-world: blob {file_id:#X} could not be read: {err}");
                    failed += 1;
                }
            }
        }
    } else {
        for map_id in catalogue {
            let Some(configured) = db_game::spawn_map(pool, *map_id)
                .await
                .expect("the map catalogue is in the database")
            else {
                eprintln!("asura-world: map {map_id} is not in map_config; skipped");
                continue;
            };
            let pack_file_id = asura_config::world::catalogue()
                .ok()
                .and_then(|maps| maps.get(map_id).and_then(|map| map.map_file_id));
            let file_id =
                pack_file_id.unwrap_or_else(|| u32::try_from(configured.map_file_id).unwrap_or(0));
            if file_id == 0 {
                eprintln!("asura-world: map {map_id} has no map_file_id; skipped");
                continue;
            }
            match asura_world::find_path_blob(&mut archive, file_id, *map_id) {
                Ok(Some((blob_id, content))) => {
                    match asura_world::map_data::PathStaticData::import(&content) {
                        Ok(data) => graphs.push(DumpedGraph {
                            map_id: *map_id,
                            map_file_id: blob_id,
                            data,
                        }),
                        Err(err) => {
                            eprintln!(
                                "asura-world: map {map_id}: blob {blob_id:#X} did not import: {err}"
                            );
                            failed += 1;
                        }
                    }
                }
                Ok(None) => eprintln!(
                    "asura-world: map {map_id}: blob {file_id:#X} is not in {archive_path}"
                ),
                Err(err) => eprintln!(
                    "asura-world: map {map_id}: blob {file_id:#X} could not be read: {err}"
                ),
            }
        }
    }

    let bytes = asura_world::graph_dump::encode(&graphs);
    if let Err(err) = std::fs::write(out, &bytes) {
        eprintln!("asura-world: cannot write {out}: {err}");
        return ExitCode::FAILURE;
    }
    let planes: usize = graphs.iter().map(|graph| graph.data.planes.len()).sum();
    let traps: u64 = graphs
        .iter()
        .map(|graph| u64::from(graph.data.traps_count))
        .sum();
    eprintln!(
        "asura-world: wrote {} path graph(s), {planes} plane(s), {traps} trapezoid(s) to {out} \
         ({} bytes){}",
        graphs.len(),
        bytes.len(),
        if failed > 0 {
            format!("; {failed} blob(s) did not import")
        } else {
            String::new()
        }
    );
    ExitCode::SUCCESS
}

/// The value after `name`, if the flag was given.
fn argument(args: &[String], name: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == name)?;
    args.get(index + 1).cloned()
}

fn print_help() {
    println!(
        "asura-world {} -- one ASURA world server",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("USAGE:");
    println!("    asura-world [--world ID] [--bind ADDRESS] [--public ADDRESS] [--maps LIST] [--capacity N]");
    println!();
    println!("OPTIONS:");
    println!(
        "    --world ID        the id this world registers under (default {DEFAULT_WORLD_ID})"
    );
    println!("    --bind ADDRESS    listen here (default {DEFAULT_BIND}, or ASURA_WORLD_BIND)");
    println!(
        "    --public ADDRESS  the address to advertise to clients (default: the bind address,"
    );
    println!("                      or ASURA_WORLD_PUBLIC)");
    println!(
        "    --maps LIST       comma-separated map ids, to host a subset instead of the whole"
    );
    println!(
        "                      catalogue (default: every map in map_config -- the reference starts"
    );
    println!("                      a server per configured map, so /maps lists them all and /tp reaches them)");
    println!(
        "    --on-demand       load a catalogue map this world was not started with, instead of"
    );
    println!(
        "                      refusing it: for a /tp into it, or a login whose character stands"
    );
    println!("                      in it. Off by default; the switch is the precache experiment");
    println!("    --capacity N      slots per map (default {DEFAULT_CAPACITY})");
    println!("    --gwdat PATH      the client archive the path graphs come from (default: ASURA_GW_DAT,");
    println!("                      then Gw.dat, then data\\Gw.dat in this repository)");
    println!("    --path-graphs PATH  load the path graphs from a dump instead of the archive");
    println!("                      (default: ASURA_PATH_GRAPHS, then data\\path-graphs.bin)");
    println!("    --capture PATH    record every game message both ways to PATH (plaintext JSONL,");
    println!(
        "                      or ASURA_CAPTURE); the client-side sniffer's server counterpart"
    );
    println!("    --dump-path-graphs PATH");
    println!(
        "                      write the path graphs once -- the maps in map_config -- and exit;"
    );
    println!(
        "                      a server then runs without the archive (a few MB instead of 3.6 GB)"
    );
    println!(
        "    --dump-all        with --dump-path-graphs, write every pathing map in the archive"
    );
    println!("    -V, --version     print the version and exit");
    println!("    -h, --help        print this and exit");
    println!();
    println!("ENVIRONMENT:");
    println!("    ASURA_DATABASE_URL    the PostgreSQL URL (falls back to the repository .env)");
    println!("    ASURA_GW_DAT          the client archive, for the path graphs");
    println!("    ASURA_PATH_GRAPHS     the path-graph dump to load, if one exists");
    println!("    ASURA_TRACE=1         log every message: direction, header, size");
    println!("    ASURA_MOVE_TRACE=1    log every movement: request, path with planes, echo, ticks, reports");
    println!("    ASURA_CAPTURE         the JSONL path to capture every message both ways into");
}
