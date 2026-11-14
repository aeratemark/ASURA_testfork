//! What an NPC is, where it stands and where it may walk: the pack's NPC content as the world holds it.
//!
//! Three records, kept apart (`docs/49-npc-framework.md` section 3.1):
//!
//! - [`NpcDefinition`] - the type. Everything `NPC_UPDATE_PROPERTIES` (`0x0056`) and
//!   `NPC_UPDATE_MODEL` (`0x0057`) carry, plus the create fields that every agent of the type shares in
//!   the map 148 capture. One per `id` in the pack, shared by every map.
//! - [`NpcPlacement`] - one body on one map: where it stands and the per-agent messages of its entry
//!   burst (`0x009A`, `0x009B`, `0x009F`, `0x00F0`, the create's own fields, the properties after
//!   `0x006D`). Two placements of one definition carry different names and different `h0023` values in
//!   the capture, which is why those are placement fields.
//! - [`Route`] - points on one map that a behavior may walk.
//!
//! Nothing here is mutable at runtime and nothing here knows an agent id: an agent id is an instance's
//! handle, allocated when the instance is created (`crate::npc::table`). This module is the pure half
//! of the loader: [`NpcContent::assemble`] takes parsed records and refuses the whole pack, listing every
//! problem it found. The files are read by `crate::npc::file`.

use crate::game::NamedItem;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// The most UTF-16 units `0x0056` carries for a definition's name (its `String16` capacity, 8).
pub const DEFINITION_NAME_UNITS: usize = 8;
/// The most model files `0x0057` carries (its `Array32` capacity, 8).
pub const MODEL_FILES: usize = 8;
/// The most UTF-16 units `0x009B` carries for an agent's name (its `String16` capacity, 32).
pub const AGENT_NAME_UNITS: usize = 32;
/// The most held items one body may declare. The item id carries two index bits.
pub const HELD_ITEMS: usize = 4;
/// The most UTF-16 units a held item's name may carry (`0x0161`'s `String16` capacity, 64).
pub const ITEM_NAME_UNITS: usize = 64;
/// The most modifier dwords a held item may carry (`0x0161`'s nested-struct capacity, 64).
pub const ITEM_MODIFIERS: usize = 64;
/// The create's model word is the NPC class nibble `0x2` over the definition index
/// (`pre-searing-load-3.jsonl` seq 233: `0x200005B3` for definition `0x5B3`), so an index must fit
/// below the nibble or it would change the class the client reads.
pub const DEFINITION_INDEX_LIMIT: u32 = 0x1000_0000;

/// The client's four-character team code, as the create's team dword carries it: the NPCs of map 148
/// carry `nonc`, the players `play` (seq 233 and 221). Stored as the dword; written in a file as the
/// four characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FourCc(pub u32);

impl FourCc {
    /// Four ASCII characters, first character in the high byte - how the client's constant reads.
    pub fn parse(text: &str) -> Option<FourCc> {
        let bytes: [u8; 4] = text.as_bytes().try_into().ok()?;
        bytes
            .iter()
            .all(|byte| byte.is_ascii_graphic())
            .then(|| FourCc(u32::from_be_bytes(bytes)))
    }

    /// The four characters, for a log line.
    pub fn text(self) -> String {
        self.0
            .to_be_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect()
    }
}

impl<'de> serde::Deserialize<'de> for FourCc {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<FourCc, D::Error> {
        let text = String::deserialize(deserializer)?;
        FourCc::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "team {text:?} is not four printable ASCII characters"
            ))
        })
    }
}

/// One NPC type: the definition the client is told once per connection before the first agent that
/// uses it.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcDefinition {
    /// The stable pack key a placement names. Never sent.
    pub id: String,
    /// The client's NPC table index: `0x0056`/`0x0057` field 1 and the low bits of the create's model.
    pub definition_index: u32,
    /// `0x0056` field 2: the model file the body is drawn from.
    pub file_id: u32,
    /// `0x0056` field 3 (body offset `0x0A`). Zero in all 14 captured definitions; unnamed.
    pub h000a: u32,
    /// `0x0056` field 4: the visual word. Its top byte is the scale in percent - the same word goes out
    /// in `0x009A` for the animals (seq 512 and 514: `0x1B000000` in both).
    pub visual: u32,
    /// `0x0056` field 5 (body offset `0x12`). Zero in all 14 captured definitions; unnamed.
    pub h0012: u32,
    /// `0x0056` field 6: a flags word (`0x20C`, `0x200`, `0x22C`, `0x18204` in the capture).
    pub flags: u32,
    /// `0x0056` byte 1: the primary profession.
    pub primary: u8,
    /// `0x0056` byte 2: the level the definition declares.
    pub level: u8,
    /// `0x0056`'s name: the encoded-string units the client resolves, 1 to 8 of them.
    pub name: Vec<u16>,
    /// `0x0057`'s model files. Empty means no `0x0057` is sent (seq 294: definition `0x22A6`).
    pub model_files: Vec<u32>,
    /// The create's byte at body offset `0x0B`: 9 for every person and 8 for every animal in the capture.
    pub h000b: u8,
    /// The create's speed, in units per second.
    pub speed: f32,
    /// The create's team code.
    pub team: FourCc,
    /// What the body is in a fight (`docs/52-skills.md`). Absent for a body nobody fights: every
    /// townsperson the captures hold. Not a client field; nothing of it is sent in the entry burst.
    #[serde(default)]
    pub combat: Option<NpcCombat>,
}

/// An NPC type's combat block: whether players fight it, what it can take and how it fights back
/// (`docs/52-skills.md`, "Mobs"). The numbers of a creature from the captures are written by
/// `tools/mobs-to-pack.py`, which names the recording each one came from.
/// Whether a creature starts a fight on its own, or only once it is attacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggression {
    /// It notices a player inside `aggro_range` and fights, as every hostile body did before.
    #[default]
    Always,
    /// It stands aside until a player damages it, then fights that player. A provoked body that
    /// returns home forgets the provocation and stands aside again.
    WhenProvoked,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcCombat {
    /// Whether it is a foe of every player. False is a combatant that stands aside: healable, not
    /// damageable.
    pub hostile: bool,
    /// Whether it stays on the spot it was placed on: it never chases and never walks home. A Plague
    /// Worm spits from its hole (owner's report, 2026-11-14).
    #[serde(default)]
    pub stationary: bool,
    /// Whether it starts the fight or waits to be hit (`Aggression`).
    #[serde(default)]
    pub aggression: Aggression,
    /// Its maximum health.
    pub health: u32,
    /// Its armor rating against every damage type.
    pub armor: i32,
    /// Its maximum energy, which Ether Feast drains and its spells cost. 0 for a creature with none.
    #[serde(default)]
    pub energy: u32,
    /// Its energy regeneration in pips; 0 for none.
    #[serde(default)]
    pub energy_regen: i32,
    /// The rank it has in every attribute: what its weapon strike and its skills scale with.
    #[serde(default)]
    pub rank: u8,
    /// The profession its skills count as theirs (divine favor and the like).
    #[serde(default)]
    pub profession: u8,
    /// Its ordinary attack; `None` for a creature that never swings.
    #[serde(default)]
    pub attack: Option<NpcAttack>,
    /// The skills it uses, in the order its brain tries them. A skill the catalogue does not hold is
    /// skipped.
    #[serde(default)]
    pub skills: Vec<u32>,
}

/// A creature's ordinary attack: what `crate::skill::engine::Weapon` is for a player's weapon item.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcAttack {
    /// The weapon item type its attack skills ask for (2 axe, 5 bow, 15 hammer, 27 sword); 0 for
    /// claws, spears and staves no skill asks for.
    #[serde(default)]
    pub item_type: u8,
    /// Milliseconds between two swings.
    pub period_ms: u64,
    /// The damage type, as `combat.json` spells it.
    pub damage_type: String,
    /// Whether it strikes from range.
    pub ranged: bool,
    /// The damage range of one swing before armor.
    pub min: u32,
    /// See `min`.
    pub max: u32,
    /// The model word of what its ranged swing shows flying (`0x00A4`), as captured; `None` for none.
    #[serde(default)]
    pub projectile: Option<u32>,
    /// `0x00A4`'s last byte for it, as captured.
    #[serde(default)]
    pub projectile_flag: u8,
}

/// An agent property sent after `0x006D` in the entry burst, by its client property id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropertyValue {
    /// `0x009F` `AGENT_PROPERTY_UPDATE_INT`.
    Int(u32),
    /// `0x00A2` `AGENT_PROPERTY_UPDATE_FLOAT`.
    Float(f32),
}

/// One property of an entry burst's tail (seq 245: property 11 = 5; seq 580: property 44 = 0.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Property {
    /// The client's property id.
    pub id: u32,
    /// The value, with the message that carries it.
    pub value: PropertyValue,
}

/// One body on one map: its record with the definition resolved. The record's fields read through it.
#[derive(Debug, Clone, PartialEq)]
pub struct NpcPlacement {
    /// The definition it is a body of.
    pub definition: Arc<NpcDefinition>,
    record: PlacementRecord,
}

impl std::ops::Deref for NpcPlacement {
    type Target = PlacementRecord;
    fn deref(&self) -> &PlacementRecord {
        &self.record
    }
}

/// One point of a route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoutePoint {
    /// Where the leg ends.
    pub position: (f32, f32),
    /// The plane the point lies on.
    pub plane: u16,
    /// How long the body waits here before the next leg, on the tree clock.
    pub pause_ms: u32,
    /// The facing the body takes on arriving, in radians, when the point states one.
    pub angle: Option<f32>,
}

/// What a route does at its last point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteMode {
    /// Walk back to the first point and keep going.
    Loop,
    /// Stop at the last point; the walk succeeds.
    Once,
    /// Pick the next point at random from the route, never the one just left.
    Random,
}

/// Points on one map a behavior may walk.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// The map-local key a placement or a behavior names.
    pub id: String,
    /// What happens at the end.
    pub mode: RouteMode,
    /// The points, in walking order.
    pub points: Vec<RoutePoint>,
}

/// One map's NPC content.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapNpcs {
    /// The map these belong to.
    pub map_id: i32,
    /// The bodies, in the order the file lists them - which is the order their agent ids are handed out.
    pub placements: Vec<NpcPlacement>,
    /// The routes, by id.
    pub routes: BTreeMap<String, Route>,
}

/// The world-wide values the NPC runtime reads.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SightRules {
    /// The compass range: a body is created for a client inside it, removed outside it, and ticked only
    /// while some client is inside it (`docs/36` R34).
    pub compass_range: f32,
}

impl SightRules {
    /// Whether a client at `viewer` is within range of a body at `body`. The distance is flat: the
    /// live server creates a body on plane 26 for a client on plane 0 (seq 233 against seq 221).
    pub fn sees(&self, viewer: (f32, f32), body: (f32, f32)) -> bool {
        crate::agent::distance_between(viewer, body) <= self.compass_range
    }
}

/// A placement as a file states it: the definition by id.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementRecord {
    /// The map-local key a script or a log names. Never an agent id.
    pub id: String,
    /// The definition's pack id.
    pub definition: String,
    /// Where it is created.
    pub position: (f32, f32),
    /// The pathing plane, not a height.
    pub plane: u16,
    /// The create's direction, as a unit vector: the live server's own bits where the row was copied
    /// from a capture.
    pub direction: (f32, f32),
    /// `0x009A`, sent before the create when present.
    pub scale: Option<u32>,
    /// `0x009B`: the agent's own name, when present.
    pub name: Option<Vec<u16>>,
    /// `0x009F` property 36 before the create, when present (the animals' only level message).
    pub level: Option<u32>,
    /// `0x00F0`'s effect word.
    pub effects: u32,
    /// The create's float at body offset `0x23`: 1.0 for most bodies, 0.32 and 0.167 for others.
    pub h0023: f32,
    /// The properties after `0x006D`, in order.
    pub properties: Vec<Property>,
    /// The items the body holds, declared by `0x0161` before `0x006D`. Their `id` is 0 here: the world
    /// gives each one an id per body when it builds the burst.
    pub items: Vec<NamedItem>,
    /// The lead and offhand item, by index into `items`; `None` is an empty hand.
    pub weapons: [Option<usize>; 2],
    /// The route a behavior's `Patrol` walks when it names none of its own.
    pub route: Option<String>,
    /// The speed this body walks at, when it is not the definition's create speed. The create keeps
    /// the captured value; `0x0027` carries this one before a leg (cmonmeow's `movement_base_speed`).
    pub walk_speed: Option<f32>,
    /// The behavior that drives it; `None` is a body that stands where it was created.
    pub behavior: Option<String>,
}

/// One map's file, as parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct MapRecord {
    /// The map the file is about.
    pub map_id: i32,
    /// Its placements.
    pub placements: Vec<PlacementRecord>,
    /// Its routes.
    pub routes: Vec<Route>,
}

/// The pack's whole NPC content, checked.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NpcContent {
    /// Every definition, by pack id.
    pub definitions: BTreeMap<String, Arc<NpcDefinition>>,
    /// Every map that has NPC content, by map id.
    pub maps: HashMap<i32, Arc<MapNpcs>>,
    /// The world-wide values.
    pub sight: SightRules,
}

impl NpcContent {
    /// One map's content, if the pack has any.
    pub fn map(&self, map_id: i32) -> Option<Arc<MapNpcs>> {
        self.maps.get(&map_id).cloned()
    }

    /// How many placements and routes the pack holds, for the one start-up line.
    pub fn counts(&self) -> (usize, usize, usize) {
        let placements = self.maps.values().map(|map| map.placements.len()).sum();
        let routes = self.maps.values().map(|map| map.routes.len()).sum();
        (self.definitions.len(), placements, routes)
    }

    /// Check every record and resolve every reference. Every problem is collected, each naming its
    /// file-level key and field, and any problem refuses the whole pack: a partial pack is a second
    /// source of truth.
    pub fn assemble(
        sight: SightRules,
        definitions: Vec<NpcDefinition>,
        maps: Vec<MapRecord>,
    ) -> Result<NpcContent, Vec<String>> {
        let mut problems = Vec::new();
        if !(sight.compass_range.is_finite() && 0.0 < sight.compass_range) {
            problems.push(format!(
                "compass_range {} is not a positive distance",
                sight.compass_range
            ));
        }

        let mut by_id: BTreeMap<String, Arc<NpcDefinition>> = BTreeMap::new();
        let mut indices: HashMap<u32, String> = HashMap::new();
        for definition in definitions {
            check_definition(&definition, &mut problems);
            if let Some(other) = indices.insert(definition.definition_index, definition.id.clone())
            {
                problems.push(format!(
                    "definition {}: definition_index {:#x} is also {other}'s",
                    definition.id, definition.definition_index
                ));
            }
            let id = definition.id.clone();
            if by_id.insert(id.clone(), Arc::new(definition)).is_some() {
                problems.push(format!("definition {id}: the id is used twice"));
            }
        }

        let mut assembled: HashMap<i32, Arc<MapNpcs>> = HashMap::new();
        for map in maps {
            let map_id = map.map_id;
            if let Some(map) = assemble_map(map, &by_id, &mut problems) {
                if assembled.insert(map_id, Arc::new(map)).is_some() {
                    problems.push(format!("map {map_id}: two files describe it"));
                }
            }
        }

        if problems.is_empty() {
            Ok(NpcContent {
                definitions: by_id,
                maps: assembled,
                sight,
            })
        } else {
            Err(problems)
        }
    }
}

fn check_definition(definition: &NpcDefinition, problems: &mut Vec<String>) {
    let id = &definition.id;
    if id.is_empty() {
        problems.push("a definition has an empty id".to_string());
    }
    if DEFINITION_INDEX_LIMIT <= definition.definition_index {
        problems.push(format!(
            "definition {id}: definition_index {:#x} does not fit below the create's class nibble",
            definition.definition_index
        ));
    }
    if definition.name.is_empty() || DEFINITION_NAME_UNITS < definition.name.len() {
        problems.push(format!(
            "definition {id}: name has {} unit(s); 0x0056 carries 1 to {DEFINITION_NAME_UNITS}",
            definition.name.len()
        ));
    }
    if definition.name.contains(&0) {
        problems.push(format!("definition {id}: name contains a zero unit"));
    }
    if MODEL_FILES < definition.model_files.len() {
        problems.push(format!(
            "definition {id}: {} model file(s); 0x0057 carries at most {MODEL_FILES}",
            definition.model_files.len()
        ));
    }
    if let Some(attack) = definition.combat.as_ref().and_then(|combat| combat.attack.as_ref()) {
        if crate::skill::model::DamageKind::parse(&attack.damage_type).is_none() {
            problems.push(format!(
                "definition {id}: combat.attack.damage_type {:?} is not a damage type",
                attack.damage_type
            ));
        }
        if attack.period_ms == 0 || attack.max < attack.min {
            problems.push(format!(
                "definition {id}: combat.attack needs a period and min <= max"
            ));
        }
    }
    if definition.combat.as_ref().is_some_and(|combat| 8 < combat.skills.len()) {
        problems.push(format!("definition {id}: combat.skills holds more than a bar's 8"));
    }
    if !(definition.speed.is_finite() && 0.0 <= definition.speed) {
        problems.push(format!(
            "definition {id}: speed {} is not a speed",
            definition.speed
        ));
    }
}

fn assemble_map(
    map: MapRecord,
    definitions: &BTreeMap<String, Arc<NpcDefinition>>,
    problems: &mut Vec<String>,
) -> Option<MapNpcs> {
    let map_id = map.map_id;
    let before = problems.len();

    let mut routes: BTreeMap<String, Route> = BTreeMap::new();
    for route in map.routes {
        let id = route.id.clone();
        if route.points.is_empty() {
            problems.push(format!("map {map_id} route {id}: it has no points"));
        }
        for (index, point) in route.points.iter().enumerate() {
            let (x, y) = point.position;
            if !(x.is_finite() && y.is_finite()) || point.angle.is_some_and(|a| !a.is_finite()) {
                problems.push(format!(
                    "map {map_id} route {id}: point {index} is not a finite position and angle"
                ));
            }
        }
        if routes.insert(id.clone(), route).is_some() {
            problems.push(format!("map {map_id} route {id}: the id is used twice"));
        }
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut placements = Vec::new();
    for record in map.placements {
        let id = record.id.clone();
        let at = format!("map {map_id} placement {id}");
        if !seen.insert(id.clone()) {
            problems.push(format!("{at}: the id is used twice"));
        }
        let (x, y) = record.position;
        if !(x.is_finite() && y.is_finite()) {
            problems.push(format!("{at}: position is not finite"));
        }
        let (dx, dy) = record.direction;
        if !(dx.is_finite() && dy.is_finite()) {
            problems.push(format!("{at}: direction is not finite"));
        }
        if !record.h0023.is_finite() {
            problems.push(format!("{at}: h0023 is not finite"));
        }
        if let Some(name) = &record.name {
            if name.is_empty() || AGENT_NAME_UNITS < name.len() || name.contains(&0) {
                problems.push(format!(
                    "{at}: name has {} unit(s); 0x009B carries 1 to {AGENT_NAME_UNITS}, none zero",
                    name.len()
                ));
            }
        }
        for property in &record.properties {
            if let PropertyValue::Float(value) = property.value {
                if !value.is_finite() {
                    problems.push(format!("{at}: property {} is not finite", property.id));
                }
            }
        }
        if let Some(route) = &record.route {
            if !routes.contains_key(route) {
                problems.push(format!("{at}: route {route} is not on this map"));
            }
        }
        if let Some(speed) = record.walk_speed {
            if !speed.is_finite() || speed <= 0.0 {
                problems.push(format!("{at}: walk_speed is not a positive finite number"));
            }
        }
        if HELD_ITEMS < record.items.len() {
            problems.push(format!(
                "{at}: {} held item(s); 0x0161 ids carry at most {HELD_ITEMS}",
                record.items.len()
            ));
        }
        for (index, item) in record.items.iter().enumerate() {
            if ITEM_NAME_UNITS < item.name.len() || item.name.contains(&0) {
                problems.push(format!(
                    "{at}: item {index}'s name has {} unit(s); 0x0161 carries 0 to {ITEM_NAME_UNITS}, \
                     none zero",
                    item.name.len()
                ));
            }
            if ITEM_MODIFIERS < item.modifiers.len() {
                problems.push(format!(
                    "{at}: item {index} has {} modifier(s); 0x0161 carries at most {ITEM_MODIFIERS}",
                    item.modifiers.len()
                ));
            }
        }
        for (hand, weapon) in record.weapons.iter().enumerate() {
            if let Some(index) = weapon {
                if record.items.get(*index).is_none() {
                    let name = if hand == 0 { "lead" } else { "offhand" };
                    problems.push(format!(
                        "{at}: the {name} names item {index}, which the placement does not declare"
                    ));
                }
            }
        }
        let Some(definition) = definitions.get(&record.definition) else {
            problems.push(format!(
                "{at}: definition {} is not in the pack",
                record.definition
            ));
            continue;
        };
        placements.push(NpcPlacement {
            definition: Arc::clone(definition),
            record,
        });
    }

    (problems.len() == before).then_some(MapNpcs {
        map_id,
        placements,
        routes,
    })
}

/// Check every placement and route point against the maps' walkable surfaces. A point off the surface
/// refuses the pack; a map this world has no graph for is reported once and passes, because a body that
/// only stands needs no graph (a walk on such a map is refused when it is asked for). `on_ground` is
/// `None` for a map without a graph.
pub fn check_ground(
    content: &NpcContent,
    on_ground: impl Fn(i32, (f32, f32), u16) -> Option<bool>,
) -> (Vec<String>, Vec<String>) {
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    let mut maps: Vec<&Arc<MapNpcs>> = content.maps.values().collect();
    maps.sort_by_key(|map| map.map_id);
    for map in maps {
        let map_id = map.map_id;
        // `true` when the point has to be on the surface to work: a placement that a route or a
        // behavior can move, and every route point. A placement with neither only stands where the
        // live server put it, so an off-surface position there is a note - the same rule as a map
        // without a graph.
        let mut points: Vec<(String, (f32, f32), u16, bool)> = map
            .placements
            .iter()
            .map(|p| {
                (
                    format!("placement {}", p.id),
                    p.position,
                    p.plane,
                    p.route.is_some() || p.behavior.is_some(),
                )
            })
            .collect();
        for route in map.routes.values() {
            for (index, point) in route.points.iter().enumerate() {
                points.push((
                    format!("route {} point {index}", route.id),
                    point.position,
                    point.plane,
                    true,
                ));
            }
        }
        for (what, position, plane, moves) in points {
            match on_ground(map_id, position, plane) {
                Some(true) => {}
                Some(false) if moves => problems.push(format!(
                    "map {map_id} {what}: ({:.1}, {:.1}) plane {plane} is not on the walkable surface",
                    position.0, position.1
                )),
                Some(false) => notes.push(format!(
                    "map {map_id} {what}: ({:.1}, {:.1}) plane {plane} is not on the walkable \
                     surface; it has no route or behavior, so it only stands there",
                    position.0, position.1
                )),
                None => {
                    notes.push(format!(
                        "map {map_id}: no path graph, so its NPC positions are not checked and its \
                         routes cannot be walked"
                    ));
                    break;
                }
            }
        }
    }
    (problems, notes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sight() -> SightRules {
        SightRules {
            compass_range: 5000.0,
        }
    }

    pub(crate) fn definition(id: &str, index: u32) -> NpcDefinition {
        NpcDefinition {
            id: id.to_string(),
            definition_index: index,
            file_id: 0x1C603,
            h000a: 0,
            visual: 0x6400_0000,
            h0012: 0,
            flags: 0x20C,
            primary: 1,
            level: 2,
            name: vec![0x0F4B, 0x8B51, 0xE2DF, 0x498E],
            model_files: vec![0x1FB99],
            h000b: 9,
            speed: 250.0,
            team: FourCc::parse("nonc").expect("four letters"),
            combat: None,
        }
    }

    pub(crate) fn placement(id: &str, definition: &str, at: (f32, f32)) -> PlacementRecord {
        PlacementRecord {
            id: id.to_string(),
            definition: definition.to_string(),
            position: at,
            plane: 0,
            direction: (1.0, 0.0),
            scale: None,
            name: Some(vec![0x3382, 0x95BE, 0xE31B, 0x6F31]),
            level: None,
            effects: 0,
            h0023: 1.0,
            properties: Vec::new(),
            items: Vec::new(),
            weapons: [None, None],
            route: None,
            walk_speed: None,
            behavior: None,
        }
    }

    /// A placement of `definition`, for the tests that build a body without a pack.
    pub(crate) fn placed(record: PlacementRecord, definition: NpcDefinition) -> NpcPlacement {
        NpcPlacement {
            definition: Arc::new(definition),
            record,
        }
    }

    pub(crate) fn map(map_id: i32, placements: Vec<PlacementRecord>) -> MapRecord {
        MapRecord {
            map_id,
            placements,
            routes: Vec::new(),
        }
    }

    #[test]
    fn the_team_code_reads_as_the_client_writes_it() {
        // seq 233 carries the bytes 63 6e 6f 6e: the dword 0x6E6F6E63, the constant 'nonc'.
        let code = FourCc::parse("nonc").expect("four letters");
        assert_eq!(code.0, 0x6E6F_6E63);
        assert_eq!(code.text(), "nonc");
        assert_eq!(FourCc::parse("non"), None);
        assert_eq!(FourCc::parse("no c"), None);
    }

    #[test]
    fn a_pack_that_resolves_assembles() {
        let content = NpcContent::assemble(
            sight(),
            vec![definition("guard", 0x5B3)],
            vec![map(148, vec![placement("gate", "guard", (1.0, 2.0))])],
        )
        .expect("assembles");
        let map = content.map(148).expect("map 148");
        assert_eq!(map.placements[0].definition.definition_index, 0x5B3);
        assert_eq!(map.placements[0].direction, (1.0, 0.0));
        assert_eq!(content.counts(), (1, 1, 0));
    }

    #[test]
    fn every_problem_is_named_and_the_pack_is_refused() {
        let mut bad = definition("guard", 0x5B3);
        bad.name = vec![1; 9];
        let mut twin = definition("twin", 0x5B3);
        twin.model_files = vec![1; 9];
        let mut placements = vec![
            placement("gate", "guard", (1.0, 2.0)),
            placement("gate", "nobody", (f32::NAN, 2.0)),
        ];
        placements[0].route = Some("nowhere".to_string());
        let problems = NpcContent::assemble(sight(), vec![bad, twin], vec![map(148, placements)])
            .expect_err("refused");
        let text = problems.join("\n");
        for expected in [
            "definition guard: name has 9 unit(s)",
            "definition twin: 9 model file(s)",
            "definition twin: definition_index 0x5b3 is also guard's",
            "placement gate: the id is used twice",
            "placement gate: position is not finite",
            "placement gate: route nowhere is not on this map",
            "placement gate: definition nobody is not in the pack",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
    }

    #[test]
    fn an_index_that_would_change_the_class_nibble_is_refused() {
        let problems = NpcContent::assemble(
            sight(),
            vec![definition("big", DEFINITION_INDEX_LIMIT)],
            Vec::new(),
        )
        .expect_err("refused");
        assert!(problems[0].contains("class nibble"), "{problems:?}");
    }

    #[test]
    fn a_route_on_the_map_passes() {
        let mut record = placement("walker", "guard", (1.0, 2.0));
        record.route = Some("square".to_string());
        record.behavior = Some("patrol".to_string());
        let mut file = map(148, vec![record]);
        file.routes.push(Route {
            id: "square".to_string(),
            mode: RouteMode::Loop,
            points: vec![RoutePoint {
                position: (5.0, 5.0),
                plane: 0,
                pause_ms: 0,
                angle: None,
            }],
        });
        NpcContent::assemble(sight(), vec![definition("guard", 1)], vec![file]).expect("assembles");
    }

    #[test]
    fn a_standing_point_off_the_surface_is_a_note_and_a_moving_one_is_a_problem() {
        let mut walker = placement("walker", "guard", (-1.0, 1.0));
        walker.behavior = Some("patrol".to_string());
        let content = NpcContent::assemble(
            sight(),
            vec![definition("guard", 1)],
            vec![
                map(148, vec![placement("in", "guard", (1.0, 1.0))]),
                map(146, vec![placement("out", "guard", (-1.0, 1.0)), walker]),
                map(150, vec![placement("blind", "guard", (1.0, 1.0))]),
            ],
        )
        .expect("assembles");
        let (problems, notes) = check_ground(&content, |map_id, (x, _), _| match map_id {
            150 => None,
            _ => Some(0.0 < x),
        });
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("map 146 placement walker"));
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes
            .iter()
            .any(|note| note.starts_with("map 146 placement out")));
        assert!(notes.iter().any(|note| note.starts_with("map 150")));
    }
}
