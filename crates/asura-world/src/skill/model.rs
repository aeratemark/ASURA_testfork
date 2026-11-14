//! What a skill is, as the engine executes it: the definitions `skills.json` declares, already parsed.
//!
//! Nothing here reads a file or knows a message. `catalog.rs` turns the pack's JSON into these types;
//! `engine.rs` executes them; `wire.rs` turns what the engine did into messages. Keeping the three apart
//! is what lets the engine be tested with no pack, no socket and no database.

use std::collections::HashMap;

/// Milliseconds on the instance's own clock.
pub type Ms = u64;

/// The client's skill type, by what the server does differently for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillType {
    Stance,
    Hex,
    Spell,
    Enchantment,
    Signet,
    Skill,
    Glyph,
    Attack,
    Preparation,
}

impl SkillType {
    /// The pack's word for it.
    pub fn parse(word: &str) -> Option<SkillType> {
        Some(match word {
            "stance" => SkillType::Stance,
            "hex" => SkillType::Hex,
            "spell" => SkillType::Spell,
            "enchantment" => SkillType::Enchantment,
            "signet" => SkillType::Signet,
            "skill" => SkillType::Skill,
            "glyph" => SkillType::Glyph,
            "attack" => SkillType::Attack,
            "preparation" => SkillType::Preparation,
            _ => return None,
        })
    }

    /// Spells are what Backfire, Aura of Restoration and a glyph's discount react to: hexes,
    /// enchantments and plain spells. Signets, glyphs, skills, stances and attacks are not.
    pub fn is_spell(self) -> bool {
        matches!(
            self,
            SkillType::Spell | SkillType::Hex | SkillType::Enchantment
        )
    }
}

/// Who a skill may be aimed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRule {
    /// The caster; the request's target is ignored.
    Caster,
    /// A living ally, the caster included.
    Ally,
    /// A living foe.
    Foe,
    /// A dead ally.
    DeadAlly,
    /// An animal (Charm Animal); no animal exists in this world yet.
    Animal,
}

impl TargetRule {
    /// The pack's word for it.
    pub fn parse(word: &str) -> Option<TargetRule> {
        Some(match word {
            "self" => TargetRule::Caster,
            "ally" => TargetRule::Ally,
            "foe" => TargetRule::Foe,
            "dead_ally" => TargetRule::DeadAlly,
            "animal" => TargetRule::Animal,
            _ => return None,
        })
    }
}

/// A number a declaration names: one of the skill's three ranges at the caster's rank, explicit
/// endpoints, a constant, or the negative of another.
#[derive(Debug, Clone, PartialEq)]
pub enum Amount {
    Scale,
    Bonus,
    Duration,
    Range(i32, i32),
    Fixed(f64),
    Negate(Box<Amount>),
}

/// Who an operation reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Caster,
    Target,
    TargetAndAdjacentFoes,
    AdjacentFoesOfCaster,
    FoesInField,
}

impl Area {
    pub fn parse(word: &str) -> Option<Area> {
        Some(match word {
            "self" => Area::Caster,
            "target" => Area::Target,
            "target_and_adjacent_foes" => Area::TargetAndAdjacentFoes,
            "adjacent_foes_of_self" => Area::AdjacentFoesOfCaster,
            "foes_in_field" => Area::FoesInField,
            _ => return None,
        })
    }
}

/// A damage type. `Untyped` is damage the game names without a type, which armor does not reduce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageKind {
    Holy,
    Fire,
    Cold,
    Lightning,
    Earth,
    Untyped,
    Slashing,
    Piercing,
    Blunt,
}

impl DamageKind {
    pub fn parse(word: &str) -> Option<DamageKind> {
        Some(match word {
            "holy" => DamageKind::Holy,
            "fire" => DamageKind::Fire,
            "cold" => DamageKind::Cold,
            "lightning" => DamageKind::Lightning,
            "earth" => DamageKind::Earth,
            "untyped" => DamageKind::Untyped,
            "slashing" => DamageKind::Slashing,
            "piercing" => DamageKind::Piercing,
            "blunt" => DamageKind::Blunt,
            _ => return None,
        })
    }

    /// Whether armor reduces it.
    pub fn respects_armor(self) -> bool {
        self != DamageKind::Untyped
    }

    /// The client's damage-type word: item modifier 587's argument, which `0x00A7`'s third field
    /// repeats (0 blunt, 1 piercing, 2 slashing, 3 cold, 4 lightning, 5 fire, 8 holy, 11 earth,
    /// 12 generic; Rurik `content/world.toml` `[damage_type.table]`).
    pub fn wire_word(self) -> u32 {
        match self {
            DamageKind::Blunt => 0,
            DamageKind::Piercing => 1,
            DamageKind::Slashing => 2,
            DamageKind::Cold => 3,
            DamageKind::Lightning => 4,
            DamageKind::Fire => 5,
            DamageKind::Holy => 8,
            DamageKind::Earth => 11,
            DamageKind::Untyped => 12,
        }
    }
}

/// The nine conditions a pre-Searing skill or creature can cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Condition {
    Bleeding,
    Blind,
    Burning,
    Crippled,
    DeepWound,
    Disease,
    Poison,
    Dazed,
    Weakness,
}

impl Condition {
    pub const ALL: [Condition; 9] = [
        Condition::Bleeding,
        Condition::Blind,
        Condition::Burning,
        Condition::Crippled,
        Condition::DeepWound,
        Condition::Disease,
        Condition::Poison,
        Condition::Dazed,
        Condition::Weakness,
    ];

    pub fn parse(word: &str) -> Option<Condition> {
        Some(match word {
            "bleeding" => Condition::Bleeding,
            "blind" => Condition::Blind,
            "burning" => Condition::Burning,
            "crippled" => Condition::Crippled,
            "deep_wound" => Condition::DeepWound,
            "disease" => Condition::Disease,
            "poison" => Condition::Poison,
            "dazed" => Condition::Dazed,
            "weakness" => Condition::Weakness,
            _ => return None,
        })
    }

    pub fn word(self) -> &'static str {
        match self {
            Condition::Bleeding => "bleeding",
            Condition::Blind => "blind",
            Condition::Burning => "burning",
            Condition::Crippled => "crippled",
            Condition::DeepWound => "deep_wound",
            Condition::Disease => "disease",
            Condition::Poison => "poison",
            Condition::Dazed => "dazed",
            Condition::Weakness => "weakness",
        }
    }

    /// The bit the agent's effect word shows it by, where the client has one of its own. Every
    /// condition also sets the shared "conditioned" bit.
    pub fn flag(self) -> u32 {
        match self {
            Condition::Bleeding => 0x01,
            Condition::Crippled => 0x08,
            Condition::DeepWound => 0x20,
            Condition::Poison => 0x40,
            _ => 0,
        }
    }
}

/// A condition on the target that gates an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guard {
    /// The target is bleeding (Gash).
    Bleeding,
    /// The target is using an attack skill (Bane Signet, Lightning Javelin).
    Attacking,
}

impl Guard {
    pub fn parse(word: &str) -> Option<Guard> {
        Some(match word {
            "bleeding" => Guard::Bleeding,
            "attacking" => Guard::Attacking,
            _ => return None,
        })
    }
}

/// What a lasting effect changes while it lasts. Every field is optional; a skill sets the ones its
/// description names.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mods {
    /// Multiplies the time an attack takes (Frenzy 0.67, Faintheartedness 1.5).
    pub attack_period: Option<f64>,
    /// Multiplies damage taken (Frenzy 2.0).
    pub damage_taken: Option<f64>,
    /// Health regeneration pips on the bearer; negative is degeneration.
    pub health_regen: Option<Amount>,
    /// Health regeneration pips on the caster while the effect lasts (Life Siphon).
    pub caster_health_regen: Option<Amount>,
    /// Flat reduction of each damage the bearer takes (Shielding Hands).
    pub damage_reduction: Option<Amount>,
    /// The next damage, up to this much, heals instead; the effect then ends (Reversal of Fortune).
    pub heal_instead_of_damage: Option<Amount>,
    /// An attacker that damages the bearer with an attack takes this much (Retribution).
    pub attacker_damage: Option<Amount>,
    /// The bearer takes this much when it casts a spell (Backfire).
    pub damage_on_spell: Option<Amount>,
    /// The bearer takes this much when it attacks (Empathy).
    pub damage_on_attack: Option<Amount>,
    /// The bearer takes this much when an enchantment or a hex is applied to it (Soul Barbs).
    pub damage_on_enchant_or_hex: Option<Amount>,
    /// The bearer is healed this much when the effect ends on its own (Blood Renewal).
    pub heal_on_end: Option<Amount>,
    /// The bearer is healed this percentage of each spell's energy cost when it casts one (Aura of
    /// Restoration).
    pub spell_heal_percent_of_cost: Option<Amount>,
    /// The bearer's next spells cost this much less energy (Glyph of Lesser Energy).
    pub spell_energy_discount: Option<Amount>,
    /// How many spells the discount covers.
    pub charges: Option<Amount>,
    /// Multiplies movement speed (Imagined Burden 0.5).
    pub move_speed: Option<f64>,
    /// Extra damage on each arrow (Read the Wind).
    pub arrow_damage: Option<Amount>,
    /// Each arrow explodes for this much damage on the target and its neighbours (Ignite Arrows).
    pub arrow_explosion: Option<Amount>,
    /// The explosion's damage type.
    pub arrow_explosion_kind: Option<DamageKind>,
}

/// One thing a skill does when it completes.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// A weapon attack: the weapon's damage plus `bonus`, `strikes` times, each multiplied.
    Attack {
        to: Area,
        bonus: Amount,
        strikes: u32,
        multiplier: f64,
        on_hit: Vec<Op>,
    },
    Damage {
        to: Area,
        kind: DamageKind,
        amount: Amount,
    },
    Heal {
        to: Area,
        amount: Amount,
        divine_favor: bool,
    },
    /// The target loses health and the caster gains the same; armor does not reduce it.
    LifeSteal {
        amount: Amount,
    },
    Condition {
        to: Area,
        condition: Condition,
        duration: Amount,
        guard: Option<Guard>,
    },
    Knockdown {
        to: Area,
        seconds: Amount,
        guard: Option<Guard>,
    },
    Interrupt {
        to: Area,
        guard: Option<Guard>,
    },
    /// A lasting effect. `duration` none is a maintained enchantment: it lasts until removed.
    Effect {
        to: Area,
        duration: Option<Amount>,
        mods: Mods,
    },
    /// An area that pulses `pulse` every `every_ms` while it lasts, at the caster or the target.
    Field {
        at_target: bool,
        duration: Amount,
        every_ms: Ms,
        radius: f32,
        pulse: Vec<Op>,
    },
    /// `hit` on the target, then on the nearest other foe of the last victim, `strikes` times.
    Chain {
        strikes: u32,
        every_ms: Ms,
        radius: f32,
        hit: Vec<Op>,
    },
    DrainEnergy {
        amount: Amount,
        caster_heal_per_point: Amount,
    },
    /// Remove the target's newest hex of this profession; when one goes, run `then`.
    RemoveHex {
        profession: u8,
        then: Vec<Op>,
    },
    Resurrect {
        health_percent: Amount,
        energy_percent: Amount,
        once_per_instance: bool,
    },
    LoseAdrenaline,
    /// A skill whose mechanics this world does not have; casting it is refused with this reason.
    Unsupported(String),
}

/// One skill, as the pack declares it.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillDef {
    pub id: u32,
    pub name: String,
    pub kind: SkillType,
    pub profession: u8,
    pub attribute: u8,
    pub elite: bool,
    pub pve_only: bool,
    pub energy: u32,
    /// Adrenaline in the client's units: 25 is one strike.
    pub adrenaline: u32,
    /// Percent of maximum health sacrificed on use.
    pub health_sacrifice: u32,
    pub activation_ms: Ms,
    pub aftercast_ms: Ms,
    pub recharge_ms: Ms,
    pub target: TargetRule,
    /// The item type the skill requires in the weapon slot.
    pub weapon: Option<u8>,
    pub scale: (i32, i32),
    pub bonus: (i32, i32),
    /// Seconds.
    pub duration: (i32, i32),
    pub aoe: f32,
    pub caster_body: Option<u32>,
    pub target_body: Option<u32>,
    /// Armor added to the caster while the skill is being used (Healing Signet -40).
    pub while_casting_armor: i32,
    /// A touch skill: its reach is the touch range, not the cast range (Vampiric Touch).
    pub touch: bool,
    pub effects: Vec<Op>,
}

impl SkillDef {
    /// The reason casting it is refused outright, when its declaration needs a mechanic this world has
    /// not got.
    pub fn unsupported(&self) -> Option<&str> {
        self.effects.iter().find_map(|op| match op {
            Op::Unsupported(why) => Some(why.as_str()),
            _ => None,
        })
    }
}

/// A weapon an attack is made with: the item's own damage range and the pack's row for its type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weapon {
    pub item_type: u8,
    pub attribute: u8,
    pub period_ms: Ms,
    pub kind: DamageKind,
    pub ranged: bool,
    pub min: u32,
    pub max: u32,
    /// What a ranged swing shows flying: `0x00A4`'s model word and its last byte. `None` for a swing
    /// with nothing to see.
    pub projectile: Option<(u32, u8)>,
}

/// The numbers `combat.json` holds, already read.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    pub spell_range: f32,
    pub touch_range: f32,
    pub adjacent_range: f32,
    pub melee_range: f32,
    pub bow_range: f32,
    /// How fast a ranged weapon's shot flies, in map units per second; the flight is the distance
    /// from the shooter to the aim over this.
    pub projectile_speed: f32,
    /// How far a projectile's target may stand from the aim across the line of fire and still be
    /// hit; outside, the shot is dodged (`combat.json`).
    pub dodge_tolerance: f32,
    /// How far short of or past the aim a projectile's target may stand and still be hit.
    pub dodge_along: f32,
    pub range_tolerance: f32,
    pub health_pip_per_second: f64,
    pub energy_pip_per_second: f64,
    pub regen_pip_cap: i32,
    pub spell_strike_per_level: f64,
    pub weapon_strike_per_rank: f64,
    pub armor_divisor: f64,
    pub adrenaline_per_hit: u32,
    pub divine_favor_attribute: u8,
    pub divine_favor_heal_per_rank: f64,
    pub monk_profession: u8,
    pub condition_skills: HashMap<Condition, u32>,
    pub condition_degeneration: HashMap<Condition, i32>,
    pub knockdown_ms: Ms,
}

impl Default for Rules {
    /// The values `config/World/Vanilla/combat.json` ships with, for tests that have no pack.
    fn default() -> Rules {
        Rules {
            spell_range: 1248.0,
            touch_range: 144.0,
            adjacent_range: 166.0,
            melee_range: 144.0,
            bow_range: 1248.0,
            projectile_speed: 1600.0,
            dodge_tolerance: 54.0,
            dodge_along: 114.0,
            range_tolerance: 1.25,
            health_pip_per_second: 2.0,
            energy_pip_per_second: 1.0 / 3.0,
            regen_pip_cap: 10,
            spell_strike_per_level: 3.0,
            weapon_strike_per_rank: 5.0,
            armor_divisor: 40.0,
            adrenaline_per_hit: 25,
            divine_favor_attribute: 16,
            divine_favor_heal_per_rank: 3.2,
            monk_profession: 3,
            condition_skills: Condition::ALL
                .iter()
                .enumerate()
                .map(|(index, condition)| (*condition, 478 + index as u32))
                .collect(),
            condition_degeneration: [
                (Condition::Bleeding, 3),
                (Condition::Burning, 7),
                (Condition::Disease, 4),
                (Condition::Poison, 4),
            ]
            .into_iter()
            .collect(),
            knockdown_ms: 2000,
        }
    }
}

/// Every skill the pack knows, by id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skills {
    pub by_id: HashMap<u32, SkillDef>,
}

impl Skills {
    pub fn get(&self, id: u32) -> Option<&SkillDef> {
        self.by_id.get(&id)
    }
}

/// `round(low + (high - low) * rank / 15)`: how every skill number scales with its attribute.
pub fn interpolate(low: i32, high: i32, rank: u8) -> i32 {
    let rank = f64::from(rank.min(20));
    (f64::from(low) + f64::from(high - low) * rank / 15.0).round() as i32
}
