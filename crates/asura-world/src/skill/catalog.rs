//! The skill pack: `skills.json`, `combat.json` and `attributes.json` in the world pack, read into the
//! engine's types.
//!
//! Every key is required and every word is checked: a skill that names an operation, an area or a
//! condition this world does not know is a start-up error that names the skill, not a skill that
//! silently does less (`docs/38-layer-decisions.md`, D35).

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use super::model::*;

/// The files inside a pack this module reads.
pub const SKILLS_FILE: &str = "skills.json";
/// See [`SKILLS_FILE`].
pub const COMBAT_FILE: &str = "combat.json";
/// See [`SKILLS_FILE`].
pub const ATTRIBUTES_FILE: &str = "attributes.json";

/// One weapon type's row in `combat.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponRow {
    pub attribute: u8,
    pub period_ms: Ms,
    pub kind: DamageKind,
    pub ranged: bool,
}

/// What a player is made of, before skills: `combat.json`'s player keys.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerRules {
    pub health_base: u32,
    pub health_per_level: u32,
    pub energy_max: u32,
    pub energy_regen_pips: i32,
    pub armor_default: i32,
}

impl PlayerRules {
    pub fn max_health(&self, level: u32) -> u32 {
        self.health_base + self.health_per_level * level
    }
}

/// `attributes.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeRules {
    /// Per attribute index: its profession and whether only a primary may raise it.
    pub professions: Vec<(u8, bool)>,
    pub rank_cost: Vec<u32>,
    pub level_points: Vec<u32>,
    pub max_rank: u8,
    pub max_attributes: usize,
}

/// The whole skill pack.
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    pub skills: Skills,
    pub rules: Rules,
    pub player: PlayerRules,
    /// By item type.
    pub weapons: HashMap<u8, WeaponRow>,
    /// Body-armor item type to its share of the rating.
    pub armor_slots: HashMap<u8, f64>,
    pub attributes: AttributeRules,
}

/// Read the three files out of a pack directory.
pub fn load(pack: &Path) -> Result<Pack, String> {
    let read = |file: &str| -> Result<String, String> {
        let path = pack.join(file);
        std::fs::read_to_string(&path)
            .map_err(|err| format!("{} is not readable: {err}", path.display()))
    };
    let combat = read(COMBAT_FILE)?;
    let skills = read(SKILLS_FILE)?;
    let attributes = read(ATTRIBUTES_FILE)?;
    from_json(&skills, &combat, &attributes).map_err(|err| format!("{}: {err}", pack.display()))
}

/// Parse the three files' text.
pub fn from_json(skills: &str, combat: &str, attributes: &str) -> Result<Pack, String> {
    let combat: Value =
        serde_json::from_str(combat).map_err(|err| format!("{COMBAT_FILE}: {err}"))?;
    let skills: Value =
        serde_json::from_str(skills).map_err(|err| format!("{SKILLS_FILE}: {err}"))?;
    let attributes: Value =
        serde_json::from_str(attributes).map_err(|err| format!("{ATTRIBUTES_FILE}: {err}"))?;

    let (rules, player, weapons, armor_slots, requirements) =
        read_combat(&combat).map_err(|err| format!("{COMBAT_FILE}: {err}"))?;
    let attributes =
        read_attributes(&attributes).map_err(|err| format!("{ATTRIBUTES_FILE}: {err}"))?;
    let mut by_id = HashMap::new();
    let list = skills
        .get("skills")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{SKILLS_FILE}: no `skills` list"))?;
    for row in list {
        let def = read_skill(row, &requirements).map_err(|err| {
            let id = row.get("id").and_then(Value::as_u64).unwrap_or(0);
            format!("{SKILLS_FILE}: skill {id}: {err}")
        })?;
        if by_id.insert(def.id, def).is_some() {
            return Err(format!("{SKILLS_FILE}: a skill id appears twice"));
        }
    }
    Ok(Pack {
        skills: Skills { by_id },
        rules,
        player,
        weapons,
        armor_slots,
        attributes,
    })
}

/// The pack in force, read once per process from the world pack (`crate::spawns::pack`).
pub fn current() -> Result<&'static Pack, String> {
    static IN_FORCE: std::sync::OnceLock<Result<Pack, String>> = std::sync::OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let path = crate::spawns::pack();
            load(&path).map(|pack| {
                eprintln!(
                    "asura: {} skills from {}",
                    pack.skills.by_id.len(),
                    path.display()
                );
                pack
            })
        })
        .as_ref()
        .map_err(|err| err.clone())
}

// ---- combat.json ---------------------------------------------------------------------------

type Combat = (
    Rules,
    PlayerRules,
    HashMap<u8, WeaponRow>,
    HashMap<u8, f64>,
    HashMap<String, u8>,
);

/// A documented value's `value`.
fn value<'a>(file: &'a Value, key: &str) -> Result<&'a Value, String> {
    file.get(key)
        .and_then(|entry| entry.get("value"))
        .ok_or_else(|| format!("`{key}` is missing or has no value"))
}

fn number(file: &Value, key: &str) -> Result<f64, String> {
    value(file, key)?
        .as_f64()
        .ok_or_else(|| format!("`{key}` is not a number"))
}

fn whole(file: &Value, key: &str) -> Result<u64, String> {
    value(file, key)?
        .as_u64()
        .ok_or_else(|| format!("`{key}` is not a whole number"))
}

fn read_combat(file: &Value) -> Result<Combat, String> {
    let mut condition_skills = HashMap::new();
    for (word, id) in value(file, "condition_skills")?
        .as_object()
        .ok_or("`condition_skills` is not a table")?
    {
        let condition =
            Condition::parse(word).ok_or_else(|| format!("unknown condition `{word}`"))?;
        let id = id
            .as_u64()
            .ok_or("a condition skill is not a whole number")?;
        condition_skills.insert(condition, id as u32);
    }
    let mut condition_degeneration = HashMap::new();
    for (word, pips) in value(file, "condition_degeneration")?
        .as_object()
        .ok_or("`condition_degeneration` is not a table")?
    {
        let condition =
            Condition::parse(word).ok_or_else(|| format!("unknown condition `{word}`"))?;
        let pips = pips
            .as_i64()
            .ok_or("a degeneration is not a whole number")?;
        condition_degeneration.insert(condition, pips as i32);
    }
    let rules = Rules {
        spell_range: number(file, "spell_range")? as f32,
        touch_range: number(file, "touch_range")? as f32,
        adjacent_range: number(file, "adjacent_range")? as f32,
        melee_range: number(file, "melee_range")? as f32,
        bow_range: number(file, "bow_range")? as f32,
        projectile_speed: number(file, "projectile_speed")? as f32,
        dodge_tolerance: number(file, "dodge_tolerance")? as f32,
        dodge_along: number(file, "dodge_along")? as f32,
        range_tolerance: number(file, "range_tolerance")? as f32,
        health_pip_per_second: number(file, "health_pip_per_second")?,
        energy_pip_per_second: number(file, "energy_pip_per_second")?,
        regen_pip_cap: whole(file, "regen_pip_cap")? as i32,
        spell_strike_per_level: number(file, "spell_strike_per_level")?,
        weapon_strike_per_rank: number(file, "weapon_strike_per_rank")?,
        armor_divisor: number(file, "armor_divisor")?,
        adrenaline_per_hit: whole(file, "adrenaline_per_hit")? as u32,
        divine_favor_attribute: whole(file, "divine_favor_attribute")? as u8,
        divine_favor_heal_per_rank: number(file, "divine_favor_heal_per_rank")?,
        monk_profession: whole(file, "monk_profession")? as u8,
        condition_skills,
        condition_degeneration,
        knockdown_ms: whole(file, "knockdown_ms")?,
    };
    if !(rules.projectile_speed.is_finite() && 0.0 < rules.projectile_speed) {
        return Err("`projectile_speed` is not a positive speed".to_string());
    }
    if !(rules.dodge_tolerance.is_finite() && 0.0 < rules.dodge_tolerance) {
        return Err("`dodge_tolerance` is not a positive distance".to_string());
    }
    if !(rules.dodge_along.is_finite() && 0.0 < rules.dodge_along) {
        return Err("`dodge_along` is not a positive distance".to_string());
    }
    let player = PlayerRules {
        health_base: whole(file, "health_base")? as u32,
        health_per_level: whole(file, "health_per_level")? as u32,
        energy_max: whole(file, "energy_max")? as u32,
        energy_regen_pips: whole(file, "energy_regen_pips")? as i32,
        armor_default: whole(file, "player_armor_default")? as i32,
    };
    let mut weapons = HashMap::new();
    for (item_type, row) in value(file, "weapons")?
        .as_object()
        .ok_or("`weapons` is not a table")?
    {
        let item_type: u8 = item_type
            .parse()
            .map_err(|_| format!("weapon key `{item_type}` is not an item type"))?;
        let field = |key: &str| {
            row.get(key)
                .ok_or_else(|| format!("weapon {item_type} has no `{key}`"))
        };
        let kind = field("damage_type")?
            .as_str()
            .and_then(DamageKind::parse)
            .ok_or_else(|| format!("weapon {item_type} has an unknown damage type"))?;
        weapons.insert(
            item_type,
            WeaponRow {
                attribute: field("attribute")?.as_u64().ok_or("bad attribute")? as u8,
                period_ms: field("period_ms")?.as_u64().ok_or("bad period")?,
                kind,
                ranged: field("ranged")?.as_bool().ok_or("bad ranged")?,
            },
        );
    }
    let mut requirements = HashMap::new();
    for (word, item_type) in value(file, "weapon_requirements")?
        .as_object()
        .ok_or("`weapon_requirements` is not a table")?
    {
        let item_type = item_type
            .as_u64()
            .ok_or("a weapon requirement is not a whole number")? as u8;
        if !weapons.contains_key(&item_type) {
            return Err(format!("weapon requirement `{word}` names item type {item_type}, which `weapons` has no row for"));
        }
        requirements.insert(word.clone(), item_type);
    }
    let mut armor_slots = HashMap::new();
    for (item_type, share) in value(file, "armor_slots")?
        .as_object()
        .ok_or("`armor_slots` is not a table")?
    {
        let item_type: u8 = item_type
            .parse()
            .map_err(|_| format!("armor key `{item_type}` is not an item type"))?;
        armor_slots.insert(
            item_type,
            share.as_f64().ok_or("an armor share is not a number")?,
        );
    }
    Ok((rules, player, weapons, armor_slots, requirements))
}

// ---- attributes.json -----------------------------------------------------------------------

fn read_attributes(file: &Value) -> Result<AttributeRules, String> {
    let numbers = |key: &str| -> Result<Vec<u32>, String> {
        value(file, key)?
            .as_array()
            .ok_or_else(|| format!("`{key}` is not a list"))?
            .iter()
            .map(|entry| {
                entry
                    .as_u64()
                    .map(|n| n as u32)
                    .ok_or_else(|| format!("`{key}` holds a value that is not a whole number"))
            })
            .collect()
    };
    let professions = value(file, "professions")?
        .as_array()
        .ok_or("`professions` is not a list")?
        .iter()
        .map(|pair| {
            let pair = pair.as_array().filter(|pair| pair.len() == 2);
            match pair {
                Some(pair) => match (pair[0].as_u64(), pair[1].as_bool()) {
                    (Some(profession), Some(primary_only)) => Ok((profession as u8, primary_only)),
                    _ => Err("a `professions` pair is not [profession, primary_only]".to_string()),
                },
                None => Err("a `professions` entry is not a pair".to_string()),
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let rank_cost = numbers("rank_cost")?;
    let max_rank = whole(file, "max_rank")? as u8;
    if rank_cost.len() != usize::from(max_rank) + 1 {
        return Err("`rank_cost` must have one entry per rank 0..max_rank".to_string());
    }
    let level_points = numbers("level_points")?;
    if level_points.len() != 20 {
        return Err("`level_points` must have one entry per level 1..20".to_string());
    }
    Ok(AttributeRules {
        professions,
        rank_cost,
        level_points,
        max_rank,
        max_attributes: whole(file, "max_attributes")? as usize,
    })
}

// ---- skills.json ---------------------------------------------------------------------------

fn field<'a>(row: &'a Value, key: &str) -> Result<&'a Value, String> {
    row.get(key).ok_or_else(|| format!("no `{key}`"))
}

fn pair(row: &Value, key: &str) -> Result<(i32, i32), String> {
    let list = field(row, key)?
        .as_array()
        .filter(|list| list.len() == 2)
        .ok_or_else(|| format!("`{key}` is not a pair"))?;
    match (list[0].as_i64(), list[1].as_i64()) {
        (Some(low), Some(high)) => Ok((low as i32, high as i32)),
        _ => Err(format!("`{key}` is not a pair of whole numbers")),
    }
}

fn seconds_to_ms(row: &Value, key: &str) -> Result<Ms, String> {
    let seconds = field(row, key)?
        .as_f64()
        .filter(|seconds| *seconds >= 0.0)
        .ok_or_else(|| format!("`{key}` is not a number of seconds"))?;
    Ok((seconds * 1000.0).round() as Ms)
}

fn optional_resource(row: &Value, key: &str) -> Result<Option<u32>, String> {
    let entry = field(row, key)?;
    if entry.is_null() {
        return Ok(None);
    }
    entry
        .as_u64()
        .map(|id| Some(id as u32))
        .ok_or_else(|| format!("`{key}` is neither null nor a resource id"))
}

fn read_skill(row: &Value, requirements: &HashMap<String, u8>) -> Result<SkillDef, String> {
    let whole = |key: &str| -> Result<u64, String> {
        field(row, key)?
            .as_u64()
            .ok_or_else(|| format!("`{key}` is not a whole number"))
    };
    let flag = |key: &str| -> Result<bool, String> {
        field(row, key)?
            .as_bool()
            .ok_or_else(|| format!("`{key}` is not true or false"))
    };
    let kind_word = field(row, "type")?.as_str().unwrap_or_default();
    let kind = SkillType::parse(kind_word).ok_or_else(|| format!("unknown type `{kind_word}`"))?;
    let target_word = field(row, "target")?.as_str().unwrap_or_default();
    let target =
        TargetRule::parse(target_word).ok_or_else(|| format!("unknown target `{target_word}`"))?;
    let weapon = match field(row, "weapon")? {
        Value::Null => None,
        entry => {
            let word = entry
                .as_str()
                .ok_or("`weapon` is neither null nor a word")?;
            Some(*requirements.get(word).ok_or_else(|| {
                format!("weapon `{word}` is not in combat.json's weapon_requirements")
            })?)
        }
    };
    let effects = field(row, "effects")?
        .as_array()
        .ok_or("`effects` is not a list")?
        .iter()
        .map(read_op)
        .collect::<Result<Vec<_>, String>>()?;
    let while_casting_armor = match row.get("while_casting") {
        None => 0,
        Some(entry) => entry
            .get("armor")
            .and_then(Value::as_i64)
            .ok_or("`while_casting` has no whole `armor`")? as i32,
    };
    let touch = effects.iter().any(|op| matches!(op, Op::LifeSteal { .. }))
        && field(row, "effects")?.as_array().is_some_and(|ops| {
            ops.iter()
                .any(|op| op.get("touch").and_then(Value::as_bool) == Some(true))
        });
    Ok(SkillDef {
        id: whole("id")? as u32,
        name: field(row, "name")?
            .as_str()
            .ok_or("`name` is not text")?
            .to_string(),
        kind,
        profession: whole("profession")? as u8,
        attribute: whole("attribute")? as u8,
        elite: flag("elite")?,
        pve_only: flag("pve_only")?,
        energy: whole("energy")? as u32,
        adrenaline: whole("adrenaline")? as u32,
        health_sacrifice: whole("health_sacrifice")? as u32,
        activation_ms: seconds_to_ms(row, "activation")?,
        aftercast_ms: seconds_to_ms(row, "aftercast")?,
        recharge_ms: whole("recharge")? * 1000,
        target,
        weapon,
        scale: pair(row, "scale")?,
        bonus: pair(row, "bonus")?,
        duration: pair(row, "duration")?,
        aoe: field(row, "aoe")?.as_f64().ok_or("`aoe` is not a number")? as f32,
        caster_body: optional_resource(row, "caster_body")?,
        target_body: optional_resource(row, "target_body")?,
        while_casting_armor,
        touch,
        effects,
    })
}

fn read_amount(entry: &Value) -> Result<Amount, String> {
    match entry {
        Value::String(word) => match word.as_str() {
            "scale" => Ok(Amount::Scale),
            "bonus" => Ok(Amount::Bonus),
            "duration" => Ok(Amount::Duration),
            other => Err(format!("unknown amount `{other}`")),
        },
        Value::Number(_) => Ok(Amount::Fixed(entry.as_f64().ok_or("bad number")?)),
        Value::Object(_) => {
            if let Some(inner) = entry.get("negate") {
                return Ok(Amount::Negate(Box::new(read_amount(inner)?)));
            }
            match (
                entry.get("low").and_then(Value::as_i64),
                entry.get("high").and_then(Value::as_i64),
            ) {
                (Some(low), Some(high)) => Ok(Amount::Range(low as i32, high as i32)),
                _ => Err("an amount table is neither {low, high} nor {negate}".to_string()),
            }
        }
        _ => Err("an amount is neither a word, a number nor a table".to_string()),
    }
}

fn read_area(op: &Value, default: Area) -> Result<Area, String> {
    match op.get("to") {
        None => Ok(default),
        Some(entry) => {
            let word = entry.as_str().ok_or("`to` is not a word")?;
            Area::parse(word).ok_or_else(|| format!("unknown area `{word}`"))
        }
    }
}

fn read_guard(op: &Value) -> Result<Option<Guard>, String> {
    match op.get("if_target") {
        None => Ok(None),
        Some(entry) => {
            let word = entry.as_str().ok_or("`if_target` is not a word")?;
            Guard::parse(word)
                .map(Some)
                .ok_or_else(|| format!("unknown guard `{word}`"))
        }
    }
}

fn read_ops(op: &Value, key: &str) -> Result<Vec<Op>, String> {
    match op.get(key) {
        None => Ok(Vec::new()),
        Some(list) => list
            .as_array()
            .ok_or_else(|| format!("`{key}` is not a list"))?
            .iter()
            .map(read_op)
            .collect(),
    }
}

fn amount_of(op: &Value, key: &str) -> Result<Amount, String> {
    read_amount(op.get(key).ok_or_else(|| format!("no `{key}`"))?)
}

/// A radius: a number, or "aoe" for the skill's own area value (0 here, resolved by the engine).
fn radius_of(op: &Value) -> Result<f32, String> {
    match op.get("radius") {
        Some(Value::String(word)) if word == "aoe" => Ok(0.0),
        Some(entry) => entry
            .as_f64()
            .map(|radius| radius as f32)
            .ok_or_else(|| "`radius` is neither \"aoe\" nor a number".to_string()),
        None => Err("no `radius`".to_string()),
    }
}

fn read_mods(entry: &Value) -> Result<Mods, String> {
    let table = entry.as_object().ok_or("`mods` is not a table")?;
    let mut mods = Mods::default();
    for (key, value) in table {
        let factor = || {
            value
                .as_f64()
                .ok_or_else(|| format!("`{key}` is not a number"))
        };
        match key.as_str() {
            "attack_period" => mods.attack_period = Some(factor()?),
            "damage_taken" => mods.damage_taken = Some(factor()?),
            "move_speed" => mods.move_speed = Some(factor()?),
            "health_regen" => mods.health_regen = Some(read_amount(value)?),
            "caster_health_regen" => mods.caster_health_regen = Some(read_amount(value)?),
            "damage_reduction" => mods.damage_reduction = Some(read_amount(value)?),
            "heal_instead_of_damage" => mods.heal_instead_of_damage = Some(read_amount(value)?),
            "attacker_damage" => mods.attacker_damage = Some(read_amount(value)?),
            "damage_on_spell" => mods.damage_on_spell = Some(read_amount(value)?),
            "damage_on_attack" => mods.damage_on_attack = Some(read_amount(value)?),
            "damage_on_enchant_or_hex" => mods.damage_on_enchant_or_hex = Some(read_amount(value)?),
            "heal_on_end" => mods.heal_on_end = Some(read_amount(value)?),
            "spell_heal_percent_of_cost" => {
                mods.spell_heal_percent_of_cost = Some(read_amount(value)?)
            }
            "spell_energy_discount" => mods.spell_energy_discount = Some(read_amount(value)?),
            "charges" => mods.charges = Some(read_amount(value)?),
            "arrow_damage" => mods.arrow_damage = Some(read_amount(value)?),
            "arrow_explosion" => mods.arrow_explosion = Some(read_amount(value)?),
            "arrow_explosion_kind" => {
                let word = value
                    .as_str()
                    .ok_or("`arrow_explosion_kind` is not a word")?;
                mods.arrow_explosion_kind = Some(
                    DamageKind::parse(word)
                        .ok_or_else(|| format!("unknown damage type `{word}`"))?,
                );
            }
            other => return Err(format!("unknown modifier `{other}`")),
        }
    }
    Ok(mods)
}

fn read_op(op: &Value) -> Result<Op, String> {
    let word = op
        .get("op")
        .and_then(Value::as_str)
        .ok_or("an effect has no `op`")?;
    Ok(match word {
        "attack" => Op::Attack {
            to: read_area(op, Area::Target)?,
            bonus: amount_of(op, "bonus")?,
            strikes: op.get("strikes").and_then(Value::as_u64).unwrap_or(1) as u32,
            multiplier: op
                .get("damage_multiplier")
                .and_then(Value::as_f64)
                .unwrap_or(1.0),
            on_hit: read_ops(op, "on_hit")?,
        },
        "damage" => {
            let kind_word = op.get("kind").and_then(Value::as_str).ok_or("no `kind`")?;
            Op::Damage {
                to: read_area(op, Area::Target)?,
                kind: DamageKind::parse(kind_word)
                    .ok_or_else(|| format!("unknown damage type `{kind_word}`"))?,
                amount: amount_of(op, "amount")?,
            }
        }
        "heal" => Op::Heal {
            to: read_area(op, Area::Target)?,
            amount: amount_of(op, "amount")?,
            divine_favor: op
                .get("divine_favor")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        "life_steal" => Op::LifeSteal {
            amount: amount_of(op, "amount")?,
        },
        "condition" => {
            let name = op
                .get("condition")
                .and_then(Value::as_str)
                .ok_or("no `condition`")?;
            Op::Condition {
                to: read_area(op, Area::Target)?,
                condition: Condition::parse(name)
                    .ok_or_else(|| format!("unknown condition `{name}`"))?,
                duration: amount_of(op, "duration")?,
                guard: read_guard(op)?,
            }
        }
        "knockdown" => Op::Knockdown {
            to: read_area(op, Area::Target)?,
            seconds: amount_of(op, "seconds")?,
            guard: read_guard(op)?,
        },
        "interrupt" => Op::Interrupt {
            to: read_area(op, Area::Target)?,
            guard: read_guard(op)?,
        },
        "effect" => {
            let maintained = op
                .get("maintained")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Op::Effect {
                to: read_area(op, Area::Target)?,
                duration: if maintained {
                    None
                } else {
                    Some(amount_of(op, "duration")?)
                },
                mods: read_mods(op.get("mods").ok_or("no `mods`")?)?,
            }
        }
        "field" => {
            let at = op.get("at").and_then(Value::as_str).ok_or("no `at`")?;
            Op::Field {
                at_target: match at {
                    "self" => false,
                    "target" => true,
                    other => return Err(format!("unknown field anchor `{other}`")),
                },
                duration: amount_of(op, "duration")?,
                every_ms: op
                    .get("every_ms")
                    .and_then(Value::as_u64)
                    .ok_or("no `every_ms`")?,
                radius: radius_of(op)?,
                pulse: read_ops(op, "pulse")?,
            }
        }
        "chain" => Op::Chain {
            strikes: op
                .get("strikes")
                .and_then(Value::as_u64)
                .ok_or("no `strikes`")? as u32,
            every_ms: op
                .get("every_ms")
                .and_then(Value::as_u64)
                .ok_or("no `every_ms`")?,
            radius: radius_of(op)?,
            hit: read_ops(op, "hit")?,
        },
        "drain_energy" => Op::DrainEnergy {
            amount: amount_of(op, "amount")?,
            caster_heal_per_point: amount_of(op, "caster_heal_per_point")?,
        },
        "remove_hex" => Op::RemoveHex {
            profession: op
                .get("profession")
                .and_then(Value::as_u64)
                .ok_or("no `profession`")? as u8,
            then: read_ops(op, "then")?,
        },
        "resurrect" => Op::Resurrect {
            health_percent: amount_of(op, "health_percent")?,
            energy_percent: amount_of(op, "energy_percent")?,
            once_per_instance: op
                .get("once_per_instance")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        "lose_adrenaline" => Op::LoseAdrenaline,
        "unsupported" => Op::Unsupported(
            op.get("needs")
                .and_then(Value::as_str)
                .unwrap_or("a mechanic this world does not have")
                .to_string(),
        ),
        other => return Err(format!("unknown operation `{other}`")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILLS: &str = include_str!("../../../../config/World/Vanilla/skills.json");
    const COMBAT: &str = include_str!("../../../../config/World/Vanilla/combat.json");
    const ATTRIBUTES: &str = include_str!("../../../../config/World/Vanilla/attributes.json");

    #[test]
    fn the_committed_pack_declares_every_pre_searing_skill() {
        let pack = from_json(SKILLS, COMBAT, ATTRIBUTES).unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(pack.skills.by_id.len(), 44);
        let orison = pack.skills.get(281).expect("Orison of Healing");
        assert_eq!(
            (orison.energy, orison.activation_ms, orison.recharge_ms),
            (5, 1000, 2000)
        );
        assert_eq!(orison.target, TargetRule::Ally);
        assert_eq!(pack.skills.get(384).expect("Gash").weapon, Some(27));
        assert!(pack.skills.get(156).expect("Vampiric Touch").touch);
        assert!(!pack.skills.get(153).expect("Vampiric Gaze").touch);
        assert_eq!(
            pack.skills
                .get(1)
                .expect("Healing Signet")
                .while_casting_armor,
            -40
        );
        assert_eq!(
            pack.skills.get(411).expect("Charm Animal").unsupported(),
            Some("pets")
        );
        assert!(matches!(
            pack.skills.get(248).expect("Retribution").effects[0],
            Op::Effect { duration: None, .. }
        ));
        assert_eq!(pack.attributes.professions.len(), 51);
        assert_eq!(pack.rules.condition_skills[&Condition::Bleeding], 478);
        assert_eq!(pack.weapons[&5].period_ms, 2000);
    }

    #[test]
    fn an_unknown_word_is_an_error_that_names_the_skill() {
        let broken = SKILLS.replacen("\"op\": \"heal\"", "\"op\": \"teleport\"", 1);
        let err = from_json(&broken, COMBAT, ATTRIBUTES).expect_err("refused");
        assert!(
            err.contains("skill 1") && err.contains("teleport"),
            "said: {err}"
        );
    }
}
