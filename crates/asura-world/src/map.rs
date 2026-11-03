//! The map: a catalogue entry, its path graph, and the blocks that carry it to a client.
//!
//! [`Map`] is the noun and [`MapEntry`] is what a world publishes and what `/maps` lists;
//! [`PathGraphs`] is the imported `Gw.dat` geometry a
//! `MOVE_TO_COORD` is answered from (the reference's `srv->map_context`); [`spawn_block`],
//! [`load_block`] and [`manifest`] are the map's side of the instance load.
//!
//! A live instance is **not** here yet: today the map a connection is in is a `MapEntry` on its
//! `Player` record (`connection.rs`). The instance object is the missing noun, recorded in
//! `docs/HANDOFF.md` and deferred with the simulation (`docs/adr/0003-simulation-in-scope.md`).

use crate::*;

/// The map API: the questions a server answers about one map. A member takes the catalogue entry
/// instead of reading a live client: the map's state here is a `MapEntry` row from `map_config`.
#[allow(non_snake_case)]
pub struct Map;

#[allow(non_snake_case)]
impl Map {
    /// `Map.GetMapID()`.
    pub fn GetMapID(map: &MapEntry) -> i32 {
        map.map_id
    }

    /// `Map.GetMapName()`.
    pub fn GetMapName(map: &MapEntry) -> &str {
        &map.name
    }

    /// `Map.GetInstanceType()`: `MapType_*`, 0 for an outpost and 1 for an explorable.
    pub fn GetInstanceType(map: &MapEntry) -> i32 {
        map.map_type
    }

    /// `Map.IsOutpost()`.
    pub fn IsOutpost(map: &MapEntry) -> bool {
        map.map_type == MAP_TYPE_MAIN_TOWN
    }

    /// `Map.IsExplorable()`.
    pub fn IsExplorable(map: &MapEntry) -> bool {
        map.map_type == MAP_TYPE_MAIN_EXPLORABLE
    }
}

/// `MapType_CharacterCreation` (`GmMap.h`): the instance a new character is made in. The reference hosts it
/// as a map of its own, which is why `main.rs` accepts it alongside a main town.
pub const MAP_TYPE_CHARACTER_CREATION: i32 = 11;

/// MapType_MainTown (0) and MapType_MainExplorable (1), the GmMap.h names for the two kinds of place a
/// client walks around in.
pub const MAP_TYPE_MAIN_TOWN: i32 = 0;

pub const MAP_TYPE_MAIN_EXPLORABLE: i32 = 1;

/// One map a world server publishes, with what the client needs to load it.
#[derive(Debug, Clone, PartialEq)]
pub struct MapEntry {
    pub map_id: i32,
    /// The `Gw.dat` file id of the map blob (`map_config.map_file_id`). A map without one loads to a
    /// black screen, so it is passed to the client rather than guessed.
    pub map_file_id: u32,
    /// 0 for an outpost, 1 for an explorable area.
    pub map_type: i32,
    /// Where a character stands on entering. The reference's own coordinates, read back from a live
    /// client in the rig; (0, 0) is outside the walkable geometry on every map measured so far.
    pub spawn: (f32, f32),
    /// The plane inside the map blob.
    pub spawn_plane: u16,
    /// What the map is called, for `/maps` and the log. Empty is allowed: a map without a name prints
    /// its id, which is a worse answer than the reference's and not a broken one.
    pub name: String,
    /// How many players this world will place in it. `/maps` reports it because a player asking "where
    /// can I go" wants to know what is running, not only what exists.
    pub capacity: i32,
}

impl MapEntry {
    /// How the map is named in an answer or a log line: its name when it has one, its id when it does
    /// not, and never an empty string.
    pub fn label(&self) -> String {
        if self.name.is_empty() {
            format!("map {}", self.map_id)
        } else {
            format!("{} ({})", self.name, self.map_id)
        }
    }

    /// The alias `/tp` would take for this map: the reference's aliases are lower-case names with single
    /// spaces ("lakeside county"), so this is the name lower-cased. An unnamed map has no alias to type and
    /// says so, rather than inventing one.
    pub fn alias(&self) -> String {
        let alias = self.name.to_lowercase();
        let collapsed = alias.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            format!("map {}", self.map_id)
        } else {
            collapsed
        }
    }

    /// `0` and `1` are the reference's own `MapType` values; every other value it can carry is a kind of
    /// instance this world does not host.
    pub fn kind(&self) -> &'static str {
        match self.map_type {
            0 => "outpost",
            1 => "explorable",
            _ => "other",
        }
    }
}

/// The path graphs of the maps this world hosts, and one finder per map.
///
/// The reference keeps both inside its server (`srv->map_context`, `GameSrv_LoadMap`), because it runs one
/// server per map; this hosts several maps in one process, so it keeps a graph per map and one finder per
/// map, built on first use. Cheap to clone (it is shared), because `World` is cloned per connection.
///
/// A graph is keyed by the **`Gw.dat` blob id**, not by map id, because the reference's catalogue gives
/// several maps the same blob (146/148, 160/163, 161/165 and 162/166): one import serves both. A map is
/// *bound* to its blob only when the world knows both - by [`PathGraphs::bind`] at startup, or by
/// [`PathGraphs::import`] when `--on-demand` loads a map. The graphs themselves can come from a `Gw.dat`
/// ([`PathGraphs::load`]) or from a dump made once ([`PathGraphs::from_dump`]), which is what lets a
/// server run without the 3.6 GB archive.
#[derive(Clone, Default)]
pub struct PathGraphs(std::sync::Arc<PathGraphsInner>);

#[derive(Default)]
pub struct PathGraphsInner {
    /// One graph per `map_file_id`, behind a lock because a demand-loaded map (`--on-demand`) adds its
    /// graph after startup. Reads clone the `Arc` out and drop the lock, so a search never holds it.
    data:
        std::sync::Mutex<std::collections::HashMap<u32, std::sync::Arc<map_data::PathStaticData>>>,
    /// map id -> the blob id its graph is stored under, for the maps this world can path in.
    pub bound: std::sync::Mutex<std::collections::HashMap<i32, u32>>,
    /// One finder per map, behind one lock. The finder keeps scratch arrays between searches, so sharing
    /// it is the point; the searches themselves are short and a world serves a handful of players.
    pub finders: std::sync::Mutex<std::collections::HashMap<i32, pathfind::PathContext>>,
}

impl PathGraphs {
    /// Import every map's graph out of a `Gw.dat`, which is what `GameSrv_LoadMap` does with the blob its
    /// map config names.
    ///
    /// A map whose blob is missing or does not import is left out, with a line saying so: the world still
    /// serves that map (a client can load it from its own copy of the archive), it just cannot say
    /// anything about where an agent in it may walk. Only an archive that cannot be opened at all is an
    /// error, and the caller decides what to do about that.
    pub fn load(archive_path: &str, maps: &[MapEntry]) -> io::Result<PathGraphs> {
        let mut archive = gwdat::GwDat::open(archive_path)?;
        let mut data = std::collections::HashMap::new();

        for map in maps {
            if let Some(graph) = import_graph(&mut archive, archive_path, map) {
                data.entry(map.map_file_id).or_insert(graph);
            }
        }

        let graphs = PathGraphs(std::sync::Arc::new(PathGraphsInner {
            data: std::sync::Mutex::new(data),
            bound: std::sync::Mutex::new(std::collections::HashMap::new()),
            finders: std::sync::Mutex::new(std::collections::HashMap::new()),
        }));
        graphs.bind(maps);
        Ok(graphs)
    }

    /// Load the graphs from a dump written by `asura-world --dump-path-graphs`, so no `Gw.dat` is
    /// needed at runtime. Nothing is bound yet: call [`PathGraphs::bind`] with the maps this world
    /// serves, and an on-demand load binds its own map through [`PathGraphs::import`].
    pub fn from_dump(path: &str) -> io::Result<PathGraphs> {
        let bytes = std::fs::read(path)?;
        let dumped = graph_dump::decode(&bytes)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let mut data = std::collections::HashMap::new();
        for graph in dumped {
            data.insert(graph.map_file_id, std::sync::Arc::new(graph.data));
        }
        Ok(PathGraphs(std::sync::Arc::new(PathGraphsInner {
            data: std::sync::Mutex::new(data),
            bound: std::sync::Mutex::new(std::collections::HashMap::new()),
            finders: std::sync::Mutex::new(std::collections::HashMap::new()),
        })))
    }

    /// Bind every map whose blob is among the loaded graphs. Returns how many were bound.
    pub fn bind(&self, maps: &[MapEntry]) -> usize {
        let data = self.0.data.lock().unwrap_or_else(|err| err.into_inner());
        let mut bound = self.0.bound.lock().unwrap_or_else(|err| err.into_inner());
        let mut count = 0;
        for map in maps {
            if data.contains_key(&map.map_file_id) {
                bound.insert(map.map_id, map.map_file_id);
                count += 1;
            }
        }
        count
    }

    /// Make one map pathable: bind it to a graph already loaded (a dump), or import its blob from the
    /// archive. The `--on-demand` half of [`PathGraphs::load`]: an archive that will not open is the
    /// only error; a blob that is missing or does not import is logged and leaves the map without
    /// pathfinding, exactly as at startup.
    pub fn import(&self, map: &MapEntry, archive_path: &str) -> io::Result<()> {
        let already_loaded = self
            .0
            .data
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .contains_key(&map.map_file_id);
        if already_loaded {
            self.0
                .bound
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .insert(map.map_id, map.map_file_id);
            return Ok(());
        }
        let mut archive = gwdat::GwDat::open(archive_path)?;
        if let Some(graph) = import_graph(&mut archive, archive_path, map) {
            self.0
                .data
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .entry(map.map_file_id)
                .or_insert(graph);
        }
        self.0
            .bound
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(map.map_id, map.map_file_id);
        Ok(())
    }

    /// A world with no graphs: every walk is refused with [`Walk::NoGraph`].
    pub fn empty() -> PathGraphs {
        PathGraphs::default()
    }

    /// How many graphs are loaded - from an archive or a dump - whether or not a map is bound to
    /// them yet. A startup line and tests use it; `/maps` uses [`PathGraphs::maps`] instead.
    pub fn imported(&self) -> usize {
        self.0
            .data
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .len()
    }

    /// The maps this world can path in, for a startup line and for tests.
    pub fn maps(&self) -> Vec<i32> {
        let mut maps: Vec<i32> = self
            .0
            .bound
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .keys()
            .copied()
            .collect();
        maps.sort_unstable();
        maps
    }

    /// Ask the finder for a walk from `from` to `to` on `map_id`.
    pub fn walk(&self, map_id: i32, from: pathfind::Pos, to: pathfind::Pos) -> Walk {
        let file_id = self
            .0
            .bound
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(&map_id)
            .copied();
        let data = file_id.and_then(|file_id| {
            self.0
                .data
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .get(&file_id)
                .map(std::sync::Arc::clone)
        });
        let Some(data) = data else {
            return Walk::NoGraph;
        };

        let mut finders = self.0.finders.lock().unwrap_or_else(|err| err.into_inner());
        let finder = finders
            .entry(map_id)
            .or_insert_with(|| pathfind::PathContext::new(data));

        let mut waypoints = Vec::new();
        if finder.path_finding(from, to, &mut waypoints) {
            Walk::Found(waypoints)
        } else {
            Walk::NoPath
        }
    }
}

/// Read one map's blob out of an open archive and import it, or log why it is absent.
pub fn import_graph(
    archive: &mut gwdat::GwDat,
    archive_path: &str,
    map: &MapEntry,
) -> Option<std::sync::Arc<map_data::PathStaticData>> {
    match find_path_blob(archive, map.map_file_id, map.map_id) {
        Ok(Some((blob_id, content))) => {
            if blob_id != map.map_file_id {
                eprintln!(
                    "world: map {} ({}) path graph comes from blob {blob_id:#X} rather than \
                     {:#X}: the archive binds the plain logical id (the bit-31 replacement flag, \
                     docs/03-roadmap.md)",
                    map.map_id, map.name, map.map_file_id
                );
            }
            match map_data::PathStaticData::import(&content) {
                Ok(imported) => Some(std::sync::Arc::new(imported)),
                Err(err) => {
                    eprintln!(
                        "world: map {} ({}) has no path graph: blob {blob_id:#X} did not import: {err}",
                        map.map_id, map.name
                    );
                    None
                }
            }
        }
        Ok(None) => {
            eprintln!(
                "world: map {} ({}) has no path graph: blob {:#X} is not in {archive_path}",
                map.map_id, map.name, map.map_file_id
            );
            None
        }
        Err(err) => {
            eprintln!(
                "world: map {} ({}) has no path graph: blob {:#X} could not be read: {err}",
                map.map_id, map.name, map.map_file_id
            );
            None
        }
    }
}

/// The blob a map's path graph actually lives at, tolerating the archive's bit-31 replacement rule.
///
/// The reference's catalogue names some blobs with bit 31 set (`0x8005EA96` for The Northlands)
/// while a client archive may bind the plain logical id (`0x5EA96`) - the bit is the archive's
/// "replacement pending" flag, not part of the map's identity (`docs/03-roadmap.md`,
/// `docs/adr/0011-rurik-is-a-source.md`). So: the configured id, then the plain one, then the flagged one,
/// and the caller logs when a substitution happened.
///
/// Returns the id that answered and its bytes. `Ok(None)` means none of the three is in the
/// archive; the caller decides whether that is a skip or an error.
pub fn find_path_blob(
    archive: &mut gwdat::GwDat,
    wanted: u32,
    map_id: i32,
) -> io::Result<Option<(u32, Vec<u8>)>> {
    let mut candidates = vec![wanted];
    let plain = wanted & 0x7FFF_FFFF;
    if plain != wanted {
        candidates.push(plain);
    }
    let flagged = wanted | 0x8000_0000;
    if flagged != wanted {
        candidates.push(flagged);
    }
    let mut first_error = None;
    for candidate in candidates {
        match archive.file(candidate) {
            Ok(Some(content)) => {
                if candidate != wanted {
                    eprintln!(
                        "world: map {map_id} path graph found at blob {candidate:#X} \
                         (the catalogue names {wanted:#X})"
                    );
                }
                return Ok(Some((candidate, content)));
            }
            Ok(None) => {}
            Err(err) => {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
    }
    match first_error {
        Some(err) => Err(err),
        None => Ok(None),
    }
}

/// The answer to the client's spawn request: the map blob, and where to stand inside it.
pub fn spawn_block(map: &MapEntry, standing: Standing) -> Vec<Frame> {
    vec![game::instance_load_spawn_point(
        map.map_file_id,
        standing.position.0,
        standing.position.1,
        standing.plane,
    )]
}

/// The four messages a world server sends without being asked, the moment it has the connection,
/// followed by the exploration grid.
///
/// The grid's two messages come right after `INSTANCE_LOAD_INFO`, because that is where a live server
/// sends them: 0.58-0.73 s into the load, once per instance. The client's map surfaces size their
/// world-map texture from the dims they carry and assert `worldMapDims.x == mapDims.x * 4` on the
/// first M press without them (`GmMapView.cpp(1731)`, client build 39112, 2026-10-29).
pub fn load_block(map: &MapEntry, name: &str, agent_id: u32) -> Vec<Frame> {
    let mut frames = vec![
        game::instance_load_head_default(),
        game::instance_player_data_start(),
        game::instance_load_player_name(name),
        game::instance_load_info(
            agent_id,
            map.map_id as u16,
            1, // the reference hardcodes this; see the builder's documentation
            DISTRICT_NUMBER,
            0, // language: the reference's default
            0, // is_observer
        ),
    ];
    if extra_load_messages_suppressed() {
        return frames;
    }
    frames.extend(crate::cartography::init_frames(
        crate::cartography::grid_for(map.map_id),
    ));
    // The client's instance table, in the batches a live server sends it (`game::instance_table`): the
    // instances that exist, so that a destination can be resolved at all. It goes in the load block
    // beside the exploration grid because that is the phase a live server sends it in - after the
    // instance info and before the client is asked to spawn anything.
    for batch in game::instance_table::INSTANCE_TABLE {
        frames.push(game::instance_table(batch));
    }
    frames
}

/// `ASURA_LOAD_EXTRA=off`: leave the exploration grid and the instance table out of the unasked block.
///
/// A measurement switch, and it exists because those two are the only content this world sends into the
/// window where an explorable load waits 4.3 s: `docs/23-initialization-protocol.md`, "The explorable".
/// A live server sends neither pair in that window for the same map - its grid and its instance table
/// arrive later in the players block, or not at all on a second load - so "the client chews on what we
/// send early" is a hypothesis this switch can remove in one run. `off`, `0` or `none` suppresses them;
/// anything else, or no variable, keeps today's block. It is not a behaviour to keep: without the grid a
/// client's map surfaces fail their own assertion (`crates/asura-world/src/cartography.rs`).
pub fn extra_load_messages_suppressed() -> bool {
    static SUPPRESSED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPRESSED.get_or_init(|| {
        matches!(
            std::env::var("ASURA_LOAD_EXTRA").ok().as_deref(),
            Some("off" | "0" | "none")
        )
    })
}

/// The reference's `MapId_Count`: the sentinel the first manifest `DONE` carries, meaning "every map
/// up to here". Its value is 877 (`0x036D`), read off the reference's own send log
/// (`... 01 02 6D 03 00 00 00 00`).
pub const MAP_ID_COUNT: u16 = 877;

/// A zero-filled bitmap of the length the reference uses for a character with no progress: 31 words.
///
/// Copied from its trace, not chosen: `GAME_SMSG_MAP_UPDATE_UNLOCKED_LIST` there carries two lists of
/// 31 zero dwords (`02 00 00 00` twice, then 31 words of zeros, twice more). Sending the lists empty
/// instead changes the *length* of a bitmap the client indexes by map id.
pub const UNLOCKED_MAP_ZEROS: &[u32] = &[0u32; 31];

/// The instance bitmap a character's reached instances become, in the client's own addressing.
///
/// **Read from the client's world context** (`docs/22-world-model.md`): the bitmap is an array of
/// `u32` where instance `n` is bit `n % 32` of word `n / 32`. 31 words covers instances 0 to 991,
/// which contains the reference's own `MapId_Count` sentinel of 877 (`MAP_ID_COUNT`), so a character
/// can never outgrow the list the reference sends.
///
/// An id outside that range, or a negative one, is dropped rather than growing the list: the width is
/// the message's, and an instance the client cannot address is not made addressable by a longer array.
pub fn unlock_bitmap(instances: &[i32]) -> Vec<u32> {
    let mut words = UNLOCKED_MAP_ZEROS.to_vec();
    for &map_id in instances {
        let index = usize::try_from(map_id).ok().map(|id| id / 32);
        if let Some(word) = index.filter(|index| *index < words.len()) {
            words[word] |= 1 << (map_id.rem_euclid(32) as u32);
        }
    }
    words
}

/// The six-message manifest the reference sends after `READY_FOR_MAP_SPAWN`
/// (`GameSrv_SendDownloadManifest`).
///
/// The phases are the reference's own enum (`ManifestPhase_Phase1 = 0`, `Phase2 = 1`, `Done = 2`), and
/// the second `DONE` carries `Phase1` rather than `Done`: that is not a mistake in this port but the
/// reference's own sequence ("this map's manifest is complete"), and a client that is told `Done`
/// there may consider the whole download finished before it starts.
///
/// The first `DONE` carries the `MapId_Count` sentinel, the second the map itself.
pub fn manifest(map_id: u16) -> Vec<Frame> {
    vec![
        game::instance_manifest_phase(PHASE_1),
        game::instance_manifest_phase(PHASE_2),
        game::instance_manifest_done(PHASE_DONE, MAP_ID_COUNT),
        game::instance_manifest_phase(PHASE_1),
        game::instance_manifest_phase(PHASE_2),
        game::instance_manifest_done(PHASE_1, map_id),
    ]
}

/// `ManifestPhase_Phase1` in the reference's enum (`GmMap.h`).
pub const PHASE_1: u8 = 0;

/// See [`PHASE_1`].
pub const PHASE_2: u8 = 1;

/// See [`PHASE_1`].
pub const PHASE_DONE: u8 = 2;

/// The client asks where to stand. The reference answers with the agent's own position and the
/// map blob's file id.
pub async fn handle_request_spawn(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let replay = world.replay.as_ref();
    let block = match replay {
        Some(replay) if replay.pick.wants(crate::replay::BlockName::Spawn) => replay.spawn.clone(),
        _ => spawn_block(&player.instance.map, player.standing()),
    };
    send(stream, session, block, trace).await?;
    Ok(true)
}
