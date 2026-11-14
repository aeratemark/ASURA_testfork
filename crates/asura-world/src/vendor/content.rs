//! The pack's trade windows and Xunlai storage NPCs: `config/World/<pack>/vendors.json`, read once.
//!
//! What the file holds is content (`AGENTS.md` 4a): which NPC, by map and name, opens which window, the
//! full item row of everything a window offers, and the limits a trade runs on. The file is written by
//! `tools/vendors-from-preforged.py`; why each row is what it is lives in `docs/53-npc-services.md`.
//!
//! The loader refuses a file whose rows do not hold together (a merchant selling an item the file does
//! not define, a collector with more offers than a window lists), naming the row, so a broken pack is
//! a start-up line rather than a window a client cannot draw.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::character_defaults::Value;

/// The file inside a pack.
pub const FILE: &str = "vendors.json";

/// The most ids one `0x0084` lists, and so the most rows one window offers (the row's `Array32` is 16
/// long). A collector's window spends one of them on its cost row.
pub const WINDOW_ROWS: usize = 16;

/// One item row: every field `CREATE_NAMED_ITEM` carries but the id and the quantity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemRow {
    /// The key merchants and collectors name it by.
    pub id: String,
    /// The `Gw.dat` file id, bit 31 included, as the live server sends it.
    pub file_id: u32,
    /// `item_type`.
    pub item_type: u8,
    /// `dye_tint`.
    pub dye_tint: u8,
    /// `dye_colors`.
    pub dye_colors: u16,
    /// `materials`.
    pub materials: u16,
    /// `unk1`.
    pub unk1: u8,
    /// `flags`.
    pub flags: u32,
    /// `value`: what a merchant pays for one, and half of what one costs.
    pub value: u32,
    /// `model_id`.
    pub model_id: u32,
    /// The coded name.
    pub name: Vec<u16>,
    /// The modifier dwords.
    pub modifiers: Vec<u32>,
    /// The `items.json` definition this row is, written into an owned copy's `definition_id`.
    #[serde(default)]
    pub definition: Option<String>,
}

impl ItemRow {
    /// The row as one `CREATE_NAMED_ITEM`, under `id` and `quantity`.
    pub fn named(&self, id: u32, quantity: u32) -> asura_protocol::game::NamedItem {
        asura_protocol::game::NamedItem {
            id,
            file_id: self.file_id,
            item_type: self.item_type,
            dye_tint: self.dye_tint,
            dye_colors: self.dye_colors,
            materials: self.materials,
            unk1: self.unk1,
            flags: self.flags,
            value: self.value,
            model_id: self.model_id,
            quantity,
            name: self.name.clone(),
            modifiers: self.modifiers.clone(),
        }
    }

    /// The row as an owned item: the database row a purchase or an exchange writes. Its place is the
    /// caller's to set.
    pub fn owned(&self, id: i32, quantity: i32, profession: u8) -> asura_db::inventory::Item {
        asura_db::inventory::Item {
            id,
            definition_id: self.definition.clone(),
            bag_id: None,
            slot: None,
            file_id: i64::from(self.file_id),
            item_type: i32::from(self.item_type),
            model_id: i32::try_from(self.model_id).unwrap_or(i32::MAX),
            dye_tint: i32::from(self.dye_tint),
            dye_colors: i32::from(self.dye_colors),
            materials: i32::from(self.materials),
            unk1: i32::from(self.unk1),
            flags: i64::from(self.flags),
            value: i32::try_from(self.value).unwrap_or(i32::MAX),
            quantity,
            profession: i32::from(profession),
            name: self.name.iter().map(|unit| i32::from(*unit)).collect(),
            modifiers: self.modifiers.iter().map(|word| i64::from(*word)).collect(),
        }
    }
}

/// A merchant: where, who, and what it sells.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Merchant {
    /// Its map.
    pub map: i32,
    /// Its name, without a bracketed title.
    pub npc: String,
    /// The item keys it sells, in window order.
    pub stock: Vec<String>,
}

/// One collector offer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    /// The reward's item key.
    pub reward: String,
    /// The primary profession it is for; 0 for any.
    pub profession: u8,
}

/// A collector: where, who, its trophy and its offers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collector {
    /// Its map.
    pub map: i32,
    /// Its name, without a bracketed title.
    pub npc: String,
    /// The trophy's item key.
    pub trophy: String,
    /// How many trophies one offer costs.
    pub cost: u32,
    /// The coded line it says over its offers (`0x00C5`).
    pub heading: Vec<u16>,
    /// Its offers, in window order.
    pub offers: Vec<Offer>,
}

/// An NPC that opens Xunlai storage.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Storage {
    /// Its map; `None` for every map.
    pub map: Option<i32>,
    /// Its name, without a bracketed title.
    pub npc: String,
}

/// The limits a trade runs on.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    /// How far a player may stand from the NPC whose window is open.
    pub service_range: Value<f32>,
    /// A merchant's price scale.
    pub buy_scale: Value<f32>,
    /// The most gold a character carries.
    pub carried_gold_max: Value<u32>,
    /// The most gold storage holds.
    pub stored_gold_max: Value<u32>,
    /// The largest stack a purchase may make.
    pub stack_max: Value<u32>,
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vendors {
    /// Free text for whoever opens the file.
    #[serde(default)]
    pub note: Option<String>,
    /// What each section means.
    #[serde(default)]
    pub fields: Option<BTreeMap<String, String>>,
    /// The limits.
    pub rules: Rules,
    /// Models a merchant does not buy.
    #[serde(default)]
    pub unsellable_models: Vec<u32>,
    /// Every item row.
    pub items: Vec<ItemRow>,
    /// The merchants.
    #[serde(default)]
    pub merchants: Vec<Merchant>,
    /// The collectors.
    #[serde(default)]
    pub collectors: Vec<Collector>,
    /// The Xunlai storage NPCs.
    #[serde(default)]
    pub storage: Vec<Storage>,
}

/// What an NPC does when talked to, if the pack gives it a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service<'a> {
    /// A merchant's window.
    Merchant(&'a Merchant),
    /// A collector's window.
    Collector(&'a Collector),
    /// The Xunlai storage window.
    Storage,
}

/// A name as the pack compares it: without a bracketed title (`"Sanura [Merchant]"` is `"Sanura"`), and
/// without surrounding spaces.
pub fn base_name(name: &str) -> &str {
    let trimmed = name.trim();
    match trimmed.rfind(" [") {
        Some(at) if trimmed.ends_with(']') => trimmed[..at].trim_end(),
        _ => trimmed,
    }
}

impl Vendors {
    /// Parse and check the file's text.
    pub fn from_json(text: &str) -> Result<Vendors, String> {
        let vendors: Vendors = serde_json::from_str(text).map_err(|err| err.to_string())?;
        vendors.check()?;
        Ok(vendors)
    }

    /// No windows at all: the answer for a pack without the file.
    pub fn empty() -> Vendors {
        Vendors {
            note: None,
            fields: None,
            rules: Rules {
                service_range: Value {
                    value: 0.0,
                    doc: None,
                },
                buy_scale: Value {
                    value: 1.0,
                    doc: None,
                },
                carried_gold_max: Value {
                    value: 0,
                    doc: None,
                },
                stored_gold_max: Value {
                    value: 0,
                    doc: None,
                },
                stack_max: Value {
                    value: 1,
                    doc: None,
                },
            },
            unsellable_models: Vec::new(),
            items: Vec::new(),
            merchants: Vec::new(),
            collectors: Vec::new(),
            storage: Vec::new(),
        }
    }

    fn check(&self) -> Result<(), String> {
        let mut keys = std::collections::BTreeSet::new();
        for item in &self.items {
            if !keys.insert(item.id.as_str()) {
                return Err(format!("item {} is defined twice", item.id));
            }
            if item.file_id == 0 {
                return Err(format!(
                    "item {} has file id 0, which the client asserts on",
                    item.id
                ));
            }
            if item.name.len() > 64 || item.modifiers.len() > 64 {
                return Err(format!(
                    "item {} has more name units or modifiers than 0x0161 carries",
                    item.id
                ));
            }
        }
        let known = |key: &str, whose: &str| {
            if keys.contains(key) {
                Ok(())
            } else {
                Err(format!(
                    "{whose} names item {key}, which the file does not define"
                ))
            }
        };
        for merchant in &self.merchants {
            if merchant.stock.is_empty() || WINDOW_ROWS < merchant.stock.len() {
                return Err(format!(
                    "merchant {} on map {} sells {} items; a window lists 1 to {WINDOW_ROWS}",
                    merchant.npc,
                    merchant.map,
                    merchant.stock.len()
                ));
            }
            for key in &merchant.stock {
                known(key, &format!("merchant {}", merchant.npc))?;
            }
        }
        for collector in &self.collectors {
            if collector.offers.is_empty() || WINDOW_ROWS <= collector.offers.len() {
                return Err(format!(
                    "collector {} on map {} has {} offers; a window lists 1 to {}",
                    collector.npc,
                    collector.map,
                    collector.offers.len(),
                    WINDOW_ROWS - 1
                ));
            }
            if collector.cost == 0 || collector.cost > self.rules.stack_max.value {
                return Err(format!(
                    "collector {} costs {} trophies",
                    collector.npc, collector.cost
                ));
            }
            if collector.heading.len() > 122 {
                return Err(format!(
                    "collector {}'s heading is longer than 0x00C5 carries",
                    collector.npc
                ));
            }
            known(&collector.trophy, &format!("collector {}", collector.npc))?;
            for offer in &collector.offers {
                known(&offer.reward, &format!("collector {}", collector.npc))?;
            }
        }
        let range = self.rules.service_range.value;
        if range.is_nan() || range <= 0.0 {
            return Err("rules.service_range must be above 0".to_string());
        }
        let scale = self.rules.buy_scale.value;
        if !scale.is_finite() || scale <= 0.0 {
            return Err("rules.buy_scale must be a finite number above 0".to_string());
        }
        Ok(())
    }

    /// An item row by key.
    pub fn item(&self, key: &str) -> Option<&ItemRow> {
        self.items.iter().find(|item| item.id == key)
    }

    /// What the NPC called `name` on `map_id` does, if anything. A merchant or a collector of this map
    /// comes first; a storage NPC may be named for every map.
    pub fn service(&self, map_id: i32, name: &str) -> Option<Service<'_>> {
        let name = base_name(name);
        if let Some(merchant) = self
            .merchants
            .iter()
            .find(|merchant| merchant.map == map_id && base_name(&merchant.npc) == name)
        {
            return Some(Service::Merchant(merchant));
        }
        if let Some(collector) = self
            .collectors
            .iter()
            .find(|collector| collector.map == map_id && base_name(&collector.npc) == name)
        {
            return Some(Service::Collector(collector));
        }
        self.storage
            .iter()
            .any(|storage| {
                storage.map.is_none_or(|map| map == map_id) && base_name(&storage.npc) == name
            })
            .then_some(Service::Storage)
    }

    /// The limits the pure rules take.
    pub fn limits(&self) -> super::rules::Limits {
        super::rules::Limits {
            buy_scale: self.rules.buy_scale.value,
            carried_max: self.rules.carried_gold_max.value,
            stored_max: self.rules.stored_gold_max.value,
            stack_max: self.rules.stack_max.value,
        }
    }
}

/// Read `<pack>/vendors.json`. A pack without the file has no windows.
pub fn load(pack: &std::path::Path) -> Result<Vendors, String> {
    let path = pack.join(FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vendors::empty()),
        Err(err) => return Err(format!("{} is not readable: {err}", path.display())),
    };
    Vendors::from_json(&text)
        .map_err(|err| format!("{} is not a vendors file: {err}", path.display()))
}

/// The windows in force, read once per process. A broken file is logged once and answers `None`: every
/// NPC then answers a talk with nothing, as an NPC with no window does.
pub fn current() -> Option<&'static Vendors> {
    static IN_FORCE: OnceLock<Option<Vendors>> = OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let pack = crate::spawns::pack();
            match load(&pack) {
                Ok(vendors) => {
                    eprintln!(
                        "asura: {} merchant(s), {} collector(s) and {} storage NPC name(s) from {}",
                        vendors.merchants.len(),
                        vendors.collectors.len(),
                        vendors.storage.len(),
                        pack.display()
                    );
                    Some(vendors)
                }
                Err(err) => {
                    eprintln!("asura: {err}; no NPC opens a trade or storage window");
                    None
                }
            }
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack() -> Vendors {
        load(&crate::spawns::pack()).expect("the pack's vendors file loads")
    }

    #[test]
    fn the_vanilla_pack_holds_together() {
        let vendors = pack();
        assert_eq!(vendors.merchants.len(), 6);
        assert_eq!(vendors.collectors.len(), 8);
        let sanura = vendors
            .merchants
            .iter()
            .find(|m| m.npc == "Sanura")
            .expect("Sanura");
        assert_eq!(sanura.map, 148);
        assert_eq!(sanura.stock.len(), 7);
        // Every live file id carries bit 31.
        assert!(vendors
            .items
            .iter()
            .all(|item| item.file_id & 0x8000_0000 != 0));
    }

    #[test]
    fn a_name_is_found_with_or_without_its_title() {
        let vendors = pack();
        assert!(matches!(
            vendors.service(148, "Sanura [Merchant]"),
            Some(Service::Merchant(_))
        ));
        assert!(matches!(
            vendors.service(148, "Sanura"),
            Some(Service::Merchant(_))
        ));
        assert!(vendors.service(163, "Sanura").is_none());
        assert!(matches!(
            vendors.service(146, "Brownlow [Collector]"),
            Some(Service::Collector(_))
        ));
        assert!(matches!(
            vendors.service(500, "Xunlai Agent [Storage]"),
            Some(Service::Storage)
        ));
        assert_eq!(base_name("  Gram  "), "Gram");
    }

    #[test]
    fn a_merchant_selling_an_unknown_item_is_refused_at_load() {
        let text = std::fs::read_to_string(crate::spawns::pack().join(FILE)).expect("the file");
        let mut doc: serde_json::Value = serde_json::from_str(&text).expect("json");
        doc["merchants"][0]["stock"][0] = serde_json::Value::from("no_such_item");
        let err = Vendors::from_json(&doc.to_string()).expect_err("refused");
        assert!(err.contains("no_such_item"), "said: {err}");
    }
}
