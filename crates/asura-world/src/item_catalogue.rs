//! Static item definitions, and the owned items that name them.
//!
//! An owned item is a row in `t_item` (`asura_db::inventory::Item`): it has an id, a bag and slot,
//! a quantity, dye, flags and the modifier words the wire carries. A definition is content: the
//! model, file id, item type, default name, inherent modifiers, stack behaviour and the equipment
//! positions the item may occupy. Definitions live in the world pack
//! (`config/World/Vanilla/items.json`, `docs/38-layer-decisions.md` D20-D24), never in a table;
//! `t_item.definition_id` is a reference to this catalogue and nothing else.
//!
//! The boundary is deliberately one-way. A definition never holds an item id, a bag, a slot, a
//! quantity, a dye, an identification flag or an instance's actual modifier list; those belong to
//! the row. A row keeps every wire field it already had, so a database written before this module
//! existed still rebuilds `CREATE_NAMED_ITEM` byte for byte with `definition_id` null.
//!
//! [`ItemDefinition::inherent_modifiers`] is the default list a new instance starts with. The
//! row's `modifiers` is the list the client is actually sent, including upgrades and runes, and
//! [`crate::item_modifier`] interprets that list. Do not recompute a row's wire fields from a
//! definition: the row is the evidence of what this instance is.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The file inside a world pack that holds the item definitions.
pub const FILE: &str = "items.json";
/// The file inside a world pack that holds the upgrades (insignias, runes, weapon mods). Optional: a
/// pack without it has no upgrade definitions, and then no item in it may list a fitted upgrade.
pub const UPGRADES_FILE: &str = "upgrades.json";

/// The first word of a name the client *composed* for display (`0xa3f(...)`, `0xa40(...)`, and the
/// `0xa30`/`0xa31`/`0xa33` base-plus-upgrade templates inside them). The live server never sends one:
/// across 584 live `CREATE_NAMED_ITEM` records no name starts with these. Sending one for an item with
/// upgrades made the client add the upgrades' names a second time ("Knight's Knight's Templar Helm of
/// Vitae of Vitae"), so a definition with upgrades must carry its bare base name.
pub const COMPOSED_NAME_WORDS: [u16; 8] = [0xA30, 0xA31, 0xA33, 0xA3D, 0xA3F, 0xA40, 0xA42, 0xA43];

/// The most name units and modifier words `CREATE_NAMED_ITEM` can carry (both counted fields are
/// bounded by the wire: 64 units and a one-byte count whose practical limit is 64 dwords).
pub const MAX_NAME_UNITS: usize = 64;
/// See [`MAX_NAME_UNITS`].
pub const MAX_MODIFIERS: usize = 64;

/// One static item definition.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDefinition {
    /// The stable key an owned item writes into `t_item.definition_id`.
    pub id: String,
    /// True when the client's own item probe has shown this exact wire identity. Only these
    /// definitions are safe to hand the client: an unverified file id can be missing its UI texture.
    #[serde(default)]
    pub live: bool,
    /// What this definition is, for the person opening the file.
    pub doc: String,
    /// The `Gw.dat` entry the model comes from, as `CREATE_NAMED_ITEM.file_id`.
    pub file_id: u32,
    /// The model the client draws, as `CREATE_NAMED_ITEM.model_id`.
    pub model_id: u32,
    /// The client's `ItemType` (`GmItem.h`).
    pub item_type: u8,
    /// The default name, as the UTF-16 code units the wire carries. Empty for an item whose name
    /// the client resolves from the model.
    pub name: Vec<u16>,
    /// A readable description for authors. Never sent: the client's item name is `name`.
    pub description: String,
    /// The item's merchant value, as `CREATE_NAMED_ITEM.value`.
    pub value: u32,
    /// Crafting materials the item carries or needs, as `CREATE_NAMED_ITEM.materials`.
    pub materials: u16,
    /// The reference's own unverified byte, copied from the row that defines it.
    pub unk1: u8,
    /// The interaction flags a new instance starts with. Bit 0 is the client's
    /// `GetIsIdentified`; without it the inventory draws the hourglass icon.
    pub flags: u32,
    /// The profession an item was made for, or 0 for `Profession_None`.
    pub profession: u8,
    /// The item's inherent modifier words, as the client's own dwords.
    pub modifiers: Vec<u32>,
    /// Whether more than one can sit in a stack.
    pub stackable: bool,
    /// The largest stack this definition allows. `null` means no source has measured it; a caller
    /// that would merge into it refuses rather than inventing a limit.
    pub max_stack: Option<u32>,
    /// The equipped-items bag slots this item may occupy (`EquippedItemSlot`): 0 weapon, 1 off
    /// hand, 2 body, 3 legs, 4 head, 5 boots, 6 gloves, 7 costume body, 8 costume head.
    pub equip_positions: Vec<u8>,
    /// Named restrictions. Empty means nothing beyond the normal item rules applies.
    pub restrictions: Vec<String>,
    /// The base this definition is a copy of: two definitions with the same `base` are the same item
    /// with different upgrades or rolls ("Wyvern Boots" plain, and "Knight's Wyvern Boots of Superior
    /// Absorption"). `None` for a hand-written definition that does not say.
    #[serde(default)]
    pub base: Option<String>,
    /// The upgrades fitted to this copy, by id (`upgrades.json`), in the order its modifier words list
    /// them. The words themselves stay in `modifiers`: an upgrade's effect value is rolled per item.
    #[serde(default)]
    pub upgrades: Vec<u16>,
}

/// One upgrade: an insignia, a rune or a weapon mod. Its identity is the `0x2408XXXX` word it puts in
/// an item's modifier list; its words' values live in the item, because they are rolled.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upgrade {
    /// The id in the upgrade's `0x2408XXXX` word.
    pub id: u16,
    /// `prefix` (an insignia, a weapon prefix), `suffix` (a rune, a weapon suffix) or `unknown` (the
    /// client does not name it: an inscription, a built-in mod).
    pub role: String,
    /// The fragment the client composes into the item's display name. Never sent by the server.
    pub name: Vec<u16>,
    /// The top 16 bits of each word it contributes, in order: the id word, then a pair word and an
    /// effect word per effect.
    pub shape: Vec<u16>,
    /// The probed items it was read from, for the reader.
    #[serde(default)]
    pub seen_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UpgradeFile {
    #[serde(default)]
    note: Option<String>,
    upgrades: Vec<Upgrade>,
}

impl ItemDefinition {
    /// The largest stack, or `None` when the definition did not measure one. A non-stackable item
    /// is a stack of one by definition, so it answers `Some(1)` even when `max_stack` is null.
    pub fn stack_limit(&self) -> Option<u32> {
        if self.stackable {
            self.max_stack
        } else {
            Some(1)
        }
    }

    /// Whether this definition may occupy an equipped-items bag slot.
    pub fn can_equip_in(&self, slot: u8) -> bool {
        self.equip_positions.contains(&slot)
    }

    /// Whether this definition can be put in a stack of `quantity`. One copy is always legal;
    /// anything more needs a measured maximum, so an unmeasured stackable definition refuses a
    /// merge instead of choosing a limit.
    pub fn accepts_quantity(&self, quantity: u32) -> bool {
        match self.stack_limit() {
            Some(max) => quantity > 0 && quantity <= max,
            None => quantity == 1,
        }
    }
}

/// The file's shape: a note for the reader and the definitions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionFile {
    #[serde(default)]
    note: Option<String>,
    definitions: Vec<ItemDefinition>,
}

/// A loaded catalogue, indexed by key and by the wire fields a row carries.
#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    definitions: Vec<ItemDefinition>,
    by_id: BTreeMap<String, usize>,
    by_wire: BTreeMap<(u32, u8, u32), usize>,
    upgrades: BTreeMap<u16, Upgrade>,
}

impl Catalogue {
    /// Parse and validate a catalogue file. A malformed or ambiguous definition is an error, not a
    /// definition silently dropped: the world refuses to start rather than serving half a pack.
    pub fn from_json(text: &str) -> Result<Catalogue, String> {
        let file: DefinitionFile =
            serde_json::from_str(text).map_err(|err| format!("items.json: {err}"))?;
        let definitions = file.definitions;
        if definitions.is_empty() {
            return Err("items.json: definitions is empty".to_string());
        }

        let mut by_id = BTreeMap::new();
        let mut by_wire = BTreeMap::new();
        for (index, definition) in definitions.iter().enumerate() {
            validate(definition)?;
            if by_id.insert(definition.id.clone(), index).is_some() {
                return Err(format!(
                    "items.json: duplicate definition id {:?}",
                    definition.id
                ));
            }
            by_wire
                .entry((
                    definition.file_id,
                    definition.item_type,
                    definition.model_id,
                ))
                .or_insert(index);
        }

        Ok(Catalogue {
            definitions,
            by_id,
            by_wire,
            upgrades: BTreeMap::new(),
        })
    }

    /// Add the pack's upgrades and check every definition against them: each upgrade a definition
    /// lists must be defined here.
    pub fn with_upgrades_json(mut self, text: &str) -> Result<Catalogue, String> {
        let file: UpgradeFile =
            serde_json::from_str(text).map_err(|err| format!("{UPGRADES_FILE}: {err}"))?;
        for upgrade in file.upgrades {
            if upgrade.shape.first() != Some(&0x2408) {
                return Err(format!(
                    "{UPGRADES_FILE}: upgrade {:#x} does not start with its 0x2408 id word",
                    upgrade.id
                ));
            }
            if !matches!(upgrade.role.as_str(), "prefix" | "suffix" | "unknown") {
                return Err(format!(
                    "{UPGRADES_FILE}: upgrade {:#x} has role {:?}",
                    upgrade.id, upgrade.role
                ));
            }
            let id = upgrade.id;
            if self.upgrades.insert(id, upgrade).is_some() {
                return Err(format!("{UPGRADES_FILE}: upgrade {id:#x} is defined twice"));
            }
        }
        for definition in &self.definitions {
            if let Some(missing) = definition
                .upgrades
                .iter()
                .find(|id| !self.upgrades.contains_key(id))
            {
                return Err(format!(
                    "{FILE}: {} lists upgrade {missing:#x}, which {UPGRADES_FILE} does not define",
                    definition.id
                ));
            }
        }
        Ok(self)
    }

    /// The upgrade with this id.
    pub fn upgrade(&self, id: u16) -> Option<&Upgrade> {
        self.upgrades.get(&id)
    }

    /// Every upgrade, by id.
    pub fn upgrades(&self) -> impl Iterator<Item = &Upgrade> {
        self.upgrades.values()
    }

    /// Every definition, in file order.
    pub fn definitions(&self) -> &[ItemDefinition] {
        &self.definitions
    }

    /// The definition with this stable key.
    pub fn definition(&self, id: &str) -> Option<&ItemDefinition> {
        self.by_id.get(id).map(|index| &self.definitions[*index])
    }

    /// The first definition with these wire fields, for a row written before `definition_id`
    /// existed. A row with an explicit id must use [`Catalogue::definition`] instead: the id is
    /// the authoritative link even when two definitions share a model.
    pub fn by_wire(&self, file_id: u32, item_type: u8, model_id: u32) -> Option<&ItemDefinition> {
        self.by_wire
            .get(&(file_id, item_type, model_id))
            .map(|index| &self.definitions[*index])
    }

    /// The definition an owned row names, falling back to the row's wire identity for a row that
    /// predates the column. `None` means this pack does not define the item; the row is still
    /// valid and still rebuilds its message.
    pub fn for_item(&self, item: &asura_db::inventory::Item) -> Option<&ItemDefinition> {
        match item.definition_id.as_deref() {
            Some(id) => self.definition(id),
            None => self.by_wire(
                u32::try_from(item.file_id).unwrap_or(u32::MAX),
                u8::try_from(item.item_type).unwrap_or(u8::MAX),
                u32::try_from(item.model_id).unwrap_or(u32::MAX),
            ),
        }
    }
}

fn validate(definition: &ItemDefinition) -> Result<(), String> {
    if definition.id.trim().is_empty() {
        return Err("items.json: a definition has an empty id".to_string());
    }
    if definition.doc.trim().is_empty() {
        return Err(format!("items.json: {} has no doc", definition.id));
    }
    if definition.description.trim().is_empty() {
        return Err(format!("items.json: {} has no description", definition.id));
    }
    if definition.name.len() > MAX_NAME_UNITS {
        return Err(format!(
            "items.json: {} has {} name units; the wire allows {MAX_NAME_UNITS}",
            definition.id,
            definition.name.len()
        ));
    }
    if definition.modifiers.len() > MAX_MODIFIERS {
        return Err(format!(
            "items.json: {} has {} modifier words; the wire allows {MAX_MODIFIERS}",
            definition.id,
            definition.modifiers.len()
        ));
    }
    let positions: BTreeSet<u8> = definition.equip_positions.iter().copied().collect();
    if positions.len() != definition.equip_positions.len() {
        return Err(format!(
            "items.json: {} lists an equipment position twice",
            definition.id
        ));
    }
    if let Some(slot) = positions.iter().find(|slot| **slot > 8) {
        return Err(format!(
            "items.json: {} names equipped slot {slot}; the client has 0..8",
            definition.id
        ));
    }
    // The upgrades a definition lists are exactly the ones its words carry, in their order: the list
    // is an index into `modifiers`, never a second source of truth.
    let fitted = crate::item_modifier::fitted_upgrades(&definition.modifiers);
    if fitted != definition.upgrades {
        return Err(format!(
            "items.json: {} lists upgrades {:x?}, and its modifier words carry {:x?}",
            definition.id, definition.upgrades, fitted
        ));
    }
    if !definition.upgrades.is_empty()
        && definition
            .name
            .first()
            .is_some_and(|word| COMPOSED_NAME_WORDS.contains(word))
    {
        return Err(format!(
            "items.json: {} has upgrades and a composed display name ({:#x}...); it must carry its bare \
             base name, or the client names its upgrades twice",
            definition.id, definition.name[0]
        ));
    }
    if definition.stackable {
        if definition.max_stack == Some(0) {
            return Err(format!(
                "items.json: {} is stackable with a maximum of zero",
                definition.id
            ));
        }
    } else if definition.max_stack.is_some_and(|max| max != 1) {
        return Err(format!(
            "items.json: {} is not stackable and its max_stack is not 1",
            definition.id
        ));
    }
    Ok(())
}

/// Read `<pack>/items.json`, and `<pack>/upgrades.json` when the pack has one.
pub fn load(pack: &Path) -> Result<Catalogue, String> {
    let path = pack.join(FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("{} is not readable: {err}", path.display()))?;
    let catalogue = Catalogue::from_json(&text)
        .map_err(|err| format!("{} is not an item catalogue: {err}", path.display()))?;
    let upgrades = pack.join(UPGRADES_FILE);
    let text = match std::fs::read_to_string(&upgrades) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // No upgrades file: an item may still load, but not one that lists an upgrade.
            return catalogue.with_upgrades_json(r#"{"upgrades":[]}"#);
        }
        Err(err) => return Err(format!("{} is not readable: {err}", upgrades.display())),
    };
    catalogue.with_upgrades_json(&text)
}

/// The pack in force, for a caller that only needs the path.
pub fn path() -> PathBuf {
    crate::spawns::pack().join(FILE)
}

/// The catalogue this process serves, read once and held.
pub fn current() -> Result<&'static Catalogue, String> {
    static IN_FORCE: OnceLock<Result<Catalogue, String>> = OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let pack = crate::spawns::pack();
            load(&pack).map(|catalogue| {
                eprintln!(
                    "asura: {} item definition(s) and {} upgrade(s) from {}",
                    catalogue.definitions.len(),
                    catalogue.upgrades.len(),
                    pack.display()
                );
                catalogue
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl ItemDefinition {
    /// The default name as the wire writes it.
    pub fn name_units(&self) -> Vec<u16> {
        self.name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed pack, read the way the world reads it.
    const VANILLA: &str = include_str!("../../../config/World/Vanilla/items.json");
    const VANILLA_UPGRADES: &str = include_str!("../../../config/World/Vanilla/upgrades.json");

    fn vanilla() -> Catalogue {
        Catalogue::from_json(VANILLA).expect("the committed catalogue parses")
    }

    #[test]
    fn the_committed_catalogue_loads() {
        let catalogue = vanilla();
        assert!(
            catalogue.definitions().len() >= 58,
            "the committed catalogue must keep at least the 58 reference-seed and curated definitions"
        );
        let backpack = catalogue.definition("backpack").expect("backpack");
        assert_eq!(backpack.item_type, 3);
        assert_eq!(backpack.model_id, 32);
        assert_eq!(backpack.name, vec![0x21A8, 0xD157, 0xB58F, 0x166F]);
        assert_eq!(backpack.modifiers, vec![0x2448_1400]);
        assert_eq!(backpack.flags, 0x2000_1000);
        assert_eq!(backpack.stack_limit(), Some(1));
    }

    #[test]
    fn every_reference_seed_item_names_a_definition() {
        let catalogue = vanilla();
        for seed in asura_db::inventory::seed::ITEMS {
            let definition = catalogue.definition(seed.definition_id).unwrap_or_else(|| {
                panic!(
                    "seed item {} names definition {:?}, which the pack does not define",
                    seed.id, seed.definition_id
                )
            });
            assert_eq!(
                i64::from(definition.file_id),
                seed.file_id,
                "{} file id",
                definition.id
            );
            assert_eq!(
                i32::from(definition.item_type),
                seed.item_type,
                "{} item type",
                definition.id
            );
            assert_eq!(
                i64::from(definition.model_id),
                i64::from(seed.model_id),
                "{} model id",
                definition.id
            );
            // The definition carries the item's richer default name and
            // inherent modifiers; the row keeps the exact snapshot the wire
            // capture sent. The byte test in lib.rs guards that snapshot.
            assert_eq!(
                i32::from(definition.profession),
                seed.profession,
                "{} profession",
                definition.id
            );
        }
    }

    #[test]
    fn a_row_with_an_id_resolves_even_when_two_definitions_share_wire_fields() {
        let catalogue = vanilla();
        let seed = &asura_db::inventory::seed::ITEMS[0];
        let item = seed.item();
        assert_eq!(
            catalogue
                .for_item(&item)
                .map(|definition| definition.id.as_str()),
            Some("backpack")
        );

        // The two cupcake rows are one definition: identity is the definition, not the item id.
        let cupcake = &asura_db::inventory::seed::ITEMS[1];
        let other = &asura_db::inventory::seed::ITEMS[2];
        assert_eq!(cupcake.definition_id, other.definition_id);
    }

    #[test]
    fn stackability_and_equipment_positions_are_definition_data() {
        let catalogue = vanilla();
        let cupcake = catalogue
            .definition("birthday_cupcake")
            .expect("Birthday Cupcake");
        assert!(cupcake.stackable);
        assert_eq!(
            cupcake.stack_limit(),
            Some(250),
            "Guild Wars stacks hold up to 250"
        );
        assert!(cupcake.accepts_quantity(100));
        assert!(!cupcake.accepts_quantity(251));
        assert!(!cupcake.accepts_quantity(0));

        let hammer = catalogue
            .definition("warrior_starter_hammer")
            .expect("hammer");
        assert!(!hammer.stackable);
        assert_eq!(hammer.stack_limit(), Some(1));
        assert!(hammer.can_equip_in(0));
        assert!(!hammer.can_equip_in(1), "a hammer is not an off hand");
        assert!(
            !catalogue
                .definition("backpack")
                .expect("backpack")
                .can_equip_in(0),
            "a bag's own item is not worn"
        );
    }

    #[test]
    fn a_duplicate_id_or_an_unknown_key_is_refused() {
        let duplicate = r#"{"definitions":[
            {"id":"a","doc":"d","file_id":1,"model_id":1,"item_type":1,"name":[],
             "description":"d","value":0,"materials":0,"unk1":0,"flags":0,"profession":0,
             "modifiers":[],"stackable":false,"max_stack":1,"equip_positions":[],
             "restrictions":[]},
            {"id":"a","doc":"d","file_id":2,"model_id":2,"item_type":1,"name":[],
             "description":"d","value":0,"materials":0,"unk1":0,"flags":0,"profession":0,
             "modifiers":[],"stackable":false,"max_stack":1,"equip_positions":[],
             "restrictions":[]}
        ]}"#;
        let err = Catalogue::from_json(duplicate).expect_err("duplicate refused");
        assert!(err.contains("duplicate definition id"), "said: {err}");

        let unknown = r#"{"definitions":[{"id":"a","doc":"d","file_id":1,"model_id":1,
            "item_type":1,"name":[],"description":"d","value":0,"materials":0,"unk1":0,
            "flags":0,"profession":0,"modifiers":[],"stackable":false,"max_stack":1,
            "equip_positions":[],"restrictions":[],"invented":true}]}"#;
        assert!(Catalogue::from_json(unknown).is_err());
    }

    /// One definition with the given name, words and listed upgrades.
    fn one(name: &str, words: &str, upgrades: &str) -> String {
        format!(
            r#"{{"definitions":[{{"id":"a","doc":"d","file_id":1,"model_id":1,"item_type":7,
            "name":{name},"description":"d","value":0,"materials":0,"unk1":0,"flags":0,
            "profession":0,"modifiers":{words},"stackable":false,"max_stack":1,
            "equip_positions":[2],"restrictions":[],"upgrades":{upgrades}}}]}}"#
        )
    }

    #[test]
    fn the_committed_upgrades_load_and_every_listed_upgrade_is_defined() {
        let catalogue = vanilla()
            .with_upgrades_json(VANILLA_UPGRADES)
            .expect("the committed upgrades load");
        assert!(catalogue.upgrades().count() > 100);
        // Vitae (0x212) is a rune: a suffix, Health +10.
        let vitae = catalogue.upgrade(0x212).expect("Vitae");
        assert_eq!(vitae.role, "suffix");
        assert_eq!(vitae.shape, vec![0x2408, 0x2530, 0x2348]);
    }

    #[test]
    fn an_upgraded_item_is_its_base_name_plus_its_upgrades() {
        let catalogue = vanilla()
            .with_upgrades_json(VANILLA_UPGRADES)
            .expect("the committed upgrades load");
        // Knight's Templar Helm of Vitae: the base name alone, and the two upgrades it carries.
        let helm = catalogue.definition("live_49_23244_16").expect("the helm");
        assert_eq!(helm.name, vec![0x8102, 0x0A19]);
        assert_eq!(helm.upgrades, vec![0x212, 0x1F9]);
        assert_eq!(helm.base.as_deref(), Some("t16_8102_a19"));
        for definition in catalogue.definitions() {
            if !definition.upgrades.is_empty() {
                assert!(
                    !COMPOSED_NAME_WORDS.contains(&definition.name[0]),
                    "{} carries a composed name",
                    definition.id
                );
            }
        }
    }

    #[test]
    fn a_composed_name_on_an_upgraded_item_is_refused() {
        // The doubled-name bug, as data: 0xa3f(0x8102 0x95b) with Knight's fitted.
        let composed = one(
            "[2623,266,33026,2395,1,0]",
            "[604504569,2747813888]",
            "[505]",
        );
        let err = Catalogue::from_json(&composed).expect_err("refused");
        assert!(err.contains("composed display name"), "said: {err}");
        let bare = one("[33026,2395]", "[604504569,2747813888]", "[505]");
        assert!(Catalogue::from_json(&bare).is_ok());
    }

    #[test]
    fn listed_upgrades_must_match_the_words_and_be_defined() {
        // Words carry Knight's (0x1f9); the list says nothing.
        let unlisted = one("[33026,2395]", "[604504569]", "[]");
        assert!(Catalogue::from_json(&unlisted)
            .expect_err("refused")
            .contains("lists upgrades"));
        // Listed and carried, but the upgrades file does not define it.
        let undefined = one("[33026,2395]", "[604504569]", "[505]");
        let err = Catalogue::from_json(&undefined)
            .expect("the items parse")
            .with_upgrades_json(r#"{"upgrades":[]}"#)
            .expect_err("refused");
        assert!(err.contains("does not define"), "said: {err}");
    }

    #[test]
    fn a_missing_file_is_an_error_not_an_empty_catalogue() {
        let err = load(Path::new("no/such/pack")).expect_err("refused");
        assert!(err.contains("items.json"), "said: {err}");
    }
}
