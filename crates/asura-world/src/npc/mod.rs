//! NPCs: content-defined bodies that an instance owns, shows to each client in range, and drives.
//!
//! The framework is `docs/49-npc-framework.md`; this is its map onto the code.
//!
//! | layer | module | depends on |
//! |---|---|---|
//! | content: types, placements, routes, checked at start-up | [`content`], [`file`] | nothing / serde |
//! | wire order: the measured entry burst, as data | [`burst`] | [`content`] |
//! | wire bytes: each message through its builder | [`frames`] | `asura_protocol::game` |
//! | behavior: the tree runtime and its leaves | [`behavior`] | the agent and path types |
//! | runtime: one instance's bodies, sight and tick | [`table`] | all of the above |
//!
//! The instance (`crate::instance::Instance`) seeds an [`NpcTable`] when it is created, hands each
//! loading client the bodies in its range inside the load's agent array, ticks the table on its own
//! clock, and drops it with itself. Nothing about an NPC is in the database.
//!
//! Behavior trees come from a [`BehaviorSource`]. A pack with `npcs/behaviors.lua` uses its trees
//! through [`lua`]; a pack without one runs [`PackBehaviors`]. A placement that names a behavior
//! neither source can build refuses the pack at start-up rather than standing silently.

pub mod behavior;
pub mod burst;
pub mod content;
pub mod fight;
pub mod file;
pub mod frames;
pub mod lua;
pub mod table;

pub use content::{MapNpcs, NpcContent, NpcDefinition, NpcPlacement, SightRules};
pub use table::{BehaviorSource, NpcTable, PackBehaviors, Viewer};

use crate::agent::Walk;
use crate::pathfind::Pos;
use crate::PathGraphs;
use std::sync::{Arc, OnceLock};

impl behavior::Pathing for PathGraphs {
    fn leg(&self, map_id: i32, from: Pos, to: Pos) -> Walk {
        PathGraphs::walk(self, map_id, from, to)
    }
}

/// What a world's instances need to run their NPCs: the content, the graphs a walk asks, and the
/// behavior source. Cloned into every instance; everything in it is shared and immutable.
#[derive(Clone)]
pub struct NpcWorld {
    /// The pack's NPC content.
    pub content: Arc<NpcContent>,
    /// The maps' path graphs.
    pub graphs: PathGraphs,
    /// Where behavior trees come from.
    pub behaviors: Arc<dyn BehaviorSource + Send + Sync>,
}

impl NpcWorld {
    /// The world's NPCs with the pack's own behaviors: the one place the behavior source is chosen.
    /// A pack with `npcs/behaviors.lua` uses its trees; a pack without one uses the built-in
    /// `patrol`. A script that does not load is an error, not a silent fallback.
    pub fn new(content: Arc<NpcContent>, graphs: PathGraphs) -> Result<NpcWorld, String> {
        let pack = crate::spawns::pack();
        let behaviors: Arc<dyn BehaviorSource + Send + Sync> = match lua::source(&pack)? {
            Some(source) => Arc::new(source),
            None => Arc::new(PackBehaviors),
        };
        Ok(NpcWorld {
            content,
            graphs,
            behaviors,
        })
    }
}

/// Read and check a pack's NPC content, trees included: every problem, or the content.
pub fn load(pack: &std::path::Path) -> Result<NpcContent, Vec<String>> {
    let content = file::load(pack)?;
    let source: Box<dyn BehaviorSource + Send + Sync> = match lua::source(pack) {
        Ok(Some(source)) => Box::new(source),
        Ok(None) => Box::new(PackBehaviors),
        Err(err) => return Err(vec![err]),
    };
    let trees = table::check_trees(&content, source.as_ref());
    if trees.is_empty() {
        Ok(content)
    } else {
        Err(trees)
    }
}

/// The pack's NPC content, read once and held. An error lists every problem; the world refuses to
/// start on it (`main.rs`), as it does for the item definitions.
pub fn current() -> Result<Arc<NpcContent>, String> {
    static IN_FORCE: OnceLock<Result<Arc<NpcContent>, String>> = OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let pack = crate::spawns::pack();
            let content = load(&pack).map_err(|problems| {
                format!(
                    "{} NPC problem(s) in {}:\n  {}",
                    problems.len(),
                    pack.display(),
                    problems.join("\n  ")
                )
            })?;
            let (definitions, placements, routes) = content.counts();
            eprintln!(
                "asura: {definitions} NPC definition(s), {placements} placement(s) and {routes} \
                 route(s) on {} map(s) from {}",
                content.maps.len(),
                pack.display()
            );
            Ok(Arc::new(content))
        })
        .clone()
}

/// Check every NPC position against the graphs this world loaded. Problems refuse the start; a map
/// without a graph is noted and passes.
pub fn check_ground(npcs: &NpcContent, graphs: &PathGraphs) -> Result<(), String> {
    let (problems, notes) = content::check_ground(npcs, |map_id, (x, y), plane| {
        graphs.on_ground(map_id, Pos { x, y, plane })
    });
    for note in notes {
        eprintln!("asura: npc: {note}");
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} NPC position(s) off the walkable surface:\n  {}",
            problems.len(),
            problems.join("\n  ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("a crate directory")
            .parent()
            .expect("a repository directory")
            .to_path_buf()
    }

    #[test]
    fn the_committed_pack_stands_and_patrols_on_the_bundled_graphs() {
        let content = load(&crate::spawns::pack()).expect("the pack's NPCs load");
        let dump = repository().join("data").join("path-graphs.bin");
        let graphs =
            PathGraphs::from_dump(dump.to_str().expect("a path")).expect("the bundled dump loads");
        let map = crate::MapEntry {
            map_id: 148,
            map_file_id: 113021,
            map_type: 0,
            spawn: (0.0, 0.0),
            spawn_plane: 0,
            spawns: Vec::new(),
            name: "Ascalon City (pre-Searing)".to_string(),
            capacity: 4,
        };
        assert_eq!(graphs.bind(&[map]), 1, "map 148's blob is in the dump");
        check_ground(&content, &graphs).expect("every placement and route point is walkable");
    }
}
