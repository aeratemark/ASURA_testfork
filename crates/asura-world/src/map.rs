//! The instance load: the blocks that carry a map to a client.
//!
//! [`spawn_block`] and [`load_block`] are the map's side of the instance load, and [`manifest`] is the
//! three-phase handshake that precedes it. The catalogue entry they take is
//! [`crate::catalogue::MapEntry`]; the graphs a walk is answered from are `crate::pathfind`.
//!
//! A live instance is **not** here yet: today the map a connection is in is a `MapEntry` on its
//! `Player` record (`connection.rs`). The instance object is the missing noun, recorded in
//! `docs/HANDOFF.md` and deferred with the simulation (`docs/adr/0003-simulation-in-scope.md`).

use crate::*;

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
pub fn load_block(
    map: &MapEntry,
    name: &str,
    agent_id: u32,
    explored: &crate::cartography::Grid,
) -> Vec<Frame> {
    let mut frames = vec![
        game::instance_load_head_default(),
        game::instance_player_data_start(),
        game::instance_load_player_name(name),
        game::instance_load_info(
            agent_id,
            map.map_id as u16,
            load_info_explorable(map.map_type),
            DISTRICT_NUMBER,
            0, // language: the reference's default
            0, // is_observer
        ),
    ];
    if extra_load_messages_suppressed() {
        return frames;
    }
    frames.extend(crate::cartography::init_frames_for(
        crate::cartography::grid_for(map.map_id),
        explored,
    ));
    // The client's instance table, in the batches a live server sends it: the instances that exist, so
    // that a destination can be resolved at all. It goes in the load block beside the exploration grid
    // because that is the phase a live server sends it in - after the instance info and before the client
    // is asked to spawn anything. The pairs are the pack's map folders (`docs/38` D36), built here and
    // framed by the wire layer.
    let (table, notes) = crate::instance_table::table_for(&crate::instance_table::pack());
    for note in &notes {
        eprintln!("world: instance table: {note}");
    }
    frames.extend(table.frames());
    frames
}

/// `INSTANCE_LOAD_INFO`'s `is_explorable` for a map of this type: 0 for a town or outpost, 1 for
/// everything else.
///
/// The reference hardcodes 1 (`// what to put here?`); the live server does not. Every
/// `INSTANCE_LOAD_INFO` in `recordings/client/feature/` carries 0 for an outpost (148 Ascalon City in
/// `login-to-portal-20261031-003720.jsonl`, 163-166 in `doors-20261031-095754.jsonl`) and 1 for an
/// explorable (146, 160-162). With 1 for Ascalon City the client treated the town as an explorable and
/// drew no title under a name, which it does only in towns (owner's live client, 2026-11-05). Only the
/// outpost value changes: an explorable already sent 1.
pub fn load_info_explorable(map_type: i32) -> u8 {
    u8::from(map_type != map_kinds().main_town)
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
