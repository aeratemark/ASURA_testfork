//! The combat engine: every combatant in one instance, the skills they are using, and what happens
//! when time passes.
//!
//! It holds state and decides; it sends nothing. Each call leaves [`Event`]s behind, and the instance
//! turns them into messages (`wire.rs`) and delivers them. The engine reads no clock: the instance
//! tells it how much time passed (`tick`), so a test can drive it a millisecond at a time.
//!
//! What a skill does is the pack's declaration (`model::Op`); what the engine owns is the mechanics
//! every declaration shares (`docs/35-skill-mechanics-and-behavior.md`): paying for a skill, the cast
//! and its recharge, damage against armor, healing, conditions, lasting effects and their reactions,
//! areas that pulse, death and resurrection.
//!
//! The order of a cast's messages is preforged's (`gw_skill_casting.h`, the `begin_packets` and
//! `finish_packets` blocks): activate, then the cast start; at completion the finish, the body effects,
//! what the skill did, the acknowledgement and the recharge. None of it is measured against a live
//! server yet (`docs/52-skills.md`).

use std::collections::{BTreeMap, HashMap, HashSet};

use super::model::*;

/// The contact instant of an ordinary swing is the attack interval's windup, not its end: retail's
/// law is `interval / 2 - 0.1 s` (Rurik, `toolkit/authsrv/authsrv.py:20165-20172`, derived over
/// 1.33/1.75/2.0/2.475/3.0 s weapons; a 1.75 s hammer contacts at 0.775 s, a 2.475 s bow at 1.1375 s).
/// The damage word, the melee finish and adrenaline land here; the next swing still opens one full
/// interval after this one started.
pub(crate) fn swing_windup(interval: Ms) -> Ms {
    (interval / 2).saturating_sub(100).max(1)
}

/// The attribute index meaning "no attribute".
pub const NO_ATTRIBUTE: u8 = 51;
/// How many attributes a rank array holds.
pub const ATTRIBUTES: usize = 51;

/// Which side of a fight an agent is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// A player character: every player is every other player's ally in PvE.
    Player,
    /// A creature that fights players.
    Foe,
    /// A body nobody fights: a townsperson.
    Neutral,
}

/// One slot of a skill bar: the skill and which copy of it.
pub type Slot = (u32, u32);

/// What a combatant is when it joins.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub side: Side,
    pub level: u32,
    pub primary: u8,
    pub secondary: u8,
    pub ranks: [u8; ATTRIBUTES],
    pub bar: [Slot; 8],
    pub max_health: u32,
    pub max_energy: u32,
    pub energy_pips: i32,
    pub armor: i32,
    pub weapon: Option<Weapon>,
}

impl Spec {
    /// A creature with no skills: what an NPC's combat block describes.
    pub fn creature(side: Side, level: u32, max_health: u32, armor: i32) -> Spec {
        Spec {
            side,
            level,
            primary: 0,
            secondary: 0,
            ranks: [0; ATTRIBUTES],
            bar: [(0, 0); 8],
            max_health,
            max_energy: 0,
            energy_pips: 0,
            armor,
            weapon: None,
        }
    }
}

/// Why a skill request was refused. The connection logs it; the client hears the acknowledgement.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    NoCaster,
    Dead,
    Busy,
    KnockedDown,
    NotOnBar,
    UnknownSkill,
    Unsupported(String),
    Recharging,
    UsedThisInstance,
    BadTarget,
    OutOfRange,
    NeedsWeapon,
    Energy,
    Adrenaline,
}

/// What happened, for the wire to publish. `agent` fields are agent ids.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    SkillActivate {
        agent: u32,
        skill: u32,
        copy: u32,
    },
    SkillAck {
        agent: u32,
        skill: u32,
        copy: u32,
    },
    SkillRecharge {
        agent: u32,
        skill: u32,
        copy: u32,
        seconds: u32,
    },
    SkillRecharged {
        agent: u32,
        skill: u32,
        copy: u32,
    },
    SkillSpent {
        agent: u32,
        skill: u32,
        copy: u32,
    },
    /// An ordinary attack's swing begins (`0x00A0` kind 4: attacker, target).
    AttackStart {
        attacker: u32,
        target: u32,
    },
    /// An ordinary melee swing landed (`0x009F` kind 1: the attacker, 0).
    AttackFinished {
        attacker: u32,
    },
    /// A shot reached its aim and the target was outside the dodge bands: `0x00A0` kind 38, reason 1.
    AttackFailed {
        attacker: u32,
        target: u32,
    },
    /// A ranged swing's missile leaves (`0x00A4`).
    Projectile {
        agent: u32,
        to: (f32, f32),
        seconds: f32,
        model: u32,
        counter: u32,
        flag: u8,
    },
    /// The shot arrives (`0x00A7`): the agent, the `0x00A4` counter it answers, and the weapon's
    /// damage-type word. Every launch is closed at launch time plus its flight, hit or miss.
    ProjectileArrived {
        agent: u32,
        counter: u32,
        kind: u32,
    },
    AttackTiming {
        agent: u32,
        interval: f32,
        modifier: f32,
    },
    CastStart {
        caster: u32,
        target: u32,
        skill: u32,
        attack: bool,
    },
    Instant {
        caster: u32,
        skill: u32,
    },
    CastFinish {
        caster: u32,
        attack: bool,
    },
    CastCancel {
        caster: u32,
        attack: bool,
    },
    CasterBody {
        agent: u32,
        resource: u32,
    },
    TargetBody {
        target: u32,
        caster: u32,
        resource: u32,
    },
    Damage {
        target: u32,
        source: u32,
        skill: u32,
        amount: f64,
        max: u32,
        armor_ignoring: bool,
        health: f64,
    },
    Heal {
        target: u32,
        source: u32,
        amount: f64,
        max: u32,
        health: f64,
    },
    Health {
        agent: u32,
        fraction: f32,
    },
    Energy {
        agent: u32,
        fraction: f32,
    },
    MaxHealth {
        agent: u32,
        max: u32,
    },
    MaxEnergy {
        agent: u32,
        max: u32,
    },
    HealthRegen {
        agent: u32,
        pips: i32,
        max: u32,
    },
    EnergyRegen {
        agent: u32,
        pips: i32,
        max: u32,
    },
    EffectAdd {
        bearer: u32,
        skill: u32,
        rank: u32,
        id: u32,
        seconds: f32,
    },
    EffectRemove {
        bearer: u32,
        id: u32,
    },
    UpkeepAdd {
        bearer: u32,
        caster: u32,
        skill: u32,
        rank: u32,
        id: u32,
    },
    UpkeepRemove {
        bearer: u32,
        id: u32,
    },
    Flags {
        agent: u32,
        flags: u32,
    },
    Knockdown {
        agent: u32,
        seconds: f32,
    },
    Speed {
        agent: u32,
        multiplier: f32,
    },
    Adrenaline {
        agent: u32,
        skill: u32,
        copy: u32,
        units: u32,
    },
    AdrenalineSpent {
        agent: u32,
        skill: u32,
        copy: u32,
    },
    Death {
        agent: u32,
        player: bool,
    },
    Resurrected {
        agent: u32,
        player: bool,
    },
}

/// The effect-word bits the engine sets (the client's agent effect word).
pub mod flags {
    pub const CONDITIONED: u32 = 0x0002;
    pub const DEAD: u32 = 0x0010;
    pub const ENCHANTED: u32 = 0x0080;
    pub const DEGEN_HEXED: u32 = 0x0400;
    pub const HEXED: u32 = 0x0800;
}

/// An effect's modifiers, resolved to numbers at the caster's rank when it was applied.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    pub attack_period: Option<f64>,
    pub damage_taken: Option<f64>,
    pub health_regen: i32,
    pub caster_health_regen: i32,
    pub damage_reduction: f64,
    pub heal_instead_of_damage: f64,
    pub attacker_damage: f64,
    pub damage_on_spell: f64,
    pub damage_on_attack: f64,
    pub damage_on_enchant_or_hex: f64,
    pub heal_on_end: f64,
    pub spell_heal_percent_of_cost: f64,
    pub spell_energy_discount: f64,
    pub charges: u32,
    pub move_speed: Option<f64>,
    pub arrow_damage: f64,
    pub arrow_explosion: f64,
    pub arrow_explosion_kind: Option<DamageKind>,
}

/// A lasting effect on one bearer.
#[derive(Debug, Clone, PartialEq)]
pub struct Active {
    pub id: u32,
    pub skill: u32,
    pub rank: u8,
    pub kind: SkillType,
    pub profession: u8,
    pub caster: u32,
    /// `None` for a maintained enchantment.
    pub expires: Option<Ms>,
    pub mods: Resolved,
}

#[derive(Debug, Clone, PartialEq)]
struct Cast {
    skill: u32,
    copy: u32,
    target: u32,
    completes: Ms,
    attack: bool,
}

/// One ordinary swing: its target, when the missile leaves, when it lands, and whether it has left.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Swing {
    target: u32,
    /// The shooter's point when the shot opened, and the point it is aimed at (the target led by
    /// its velocity over the flight). The arrival's dodge test is measured against that line.
    origin: (f32, f32),
    aim: (f32, f32),
    launch: Ms,
    lands: Ms,
    launched: bool,
}

/// One combatant.
#[derive(Debug, Clone, PartialEq)]
pub struct Actor {
    pub agent: u32,
    pub spec: Spec,
    pub position: (f32, f32),
    /// Its speed and direction from the last two position samples, in map units per second: what a
    /// shot at it leads by. Held between samples.
    velocity: (f32, f32),
    /// When the last position sample landed.
    sampled_at: Ms,
    pub alive: bool,
    pub health: f64,
    pub energy: f64,
    pub effects: Vec<Active>,
    /// Each condition's expiry and the effect id it was published with.
    pub conditions: BTreeMap<Condition, (Ms, u32)>,
    recharge: HashMap<Slot, Ms>,
    adrenaline: HashMap<Slot, u32>,
    used_once: HashSet<Slot>,
    cast: Option<Cast>,
    /// The foe its ordinary attack is on, kept through skills and knockdowns until it is stopped.
    pub attack_target: Option<u32>,
    /// The swing in progress.
    swing: Option<Swing>,
    /// When the next ordinary swing may open: one full interval after the swing that is running (or
    /// last landed) started. The contact is earlier (`swing_windup`); this is the cadence.
    swing_restart: Option<Ms>,
    /// How many missiles it has shot, for `0x00A4`'s counter.
    shots: u32,
    /// A missile on its way: when it arrives, the shot counter it answers, and the damage type its
    /// `0x00A7` names. Kept apart from the swing so a shot already in the air still arrives when the
    /// swing is lost.
    in_flight: Option<(Ms, u32, DamageKind)>,
    aftercast_until: Ms,
    knocked_until: Ms,
    casting_armor: i32,
    shown_flags: u32,
    shown_health_pips: i32,
    shown_energy_pips: i32,
    shown_speed: f64,
}

impl Actor {
    fn new(agent: u32, spec: Spec, position: (f32, f32)) -> Actor {
        let health = f64::from(spec.max_health);
        let energy = f64::from(spec.max_energy);
        let energy_pips = spec.energy_pips;
        Actor {
            agent,
            spec,
            position,
            velocity: (0.0, 0.0),
            sampled_at: 0,
            alive: true,
            health,
            energy,
            effects: Vec::new(),
            conditions: BTreeMap::new(),
            recharge: HashMap::new(),
            adrenaline: HashMap::new(),
            used_once: HashSet::new(),
            cast: None,
            attack_target: None,
            swing: None,
            swing_restart: None,
            shots: 0,
            in_flight: None,
            aftercast_until: 0,
            knocked_until: 0,
            casting_armor: 0,
            shown_flags: 0,
            shown_health_pips: 0,
            shown_energy_pips: energy_pips,
            shown_speed: 1.0,
        }
    }

    /// The rank the actor has in an attribute; 0 for none.
    pub fn rank(&self, attribute: u8) -> u8 {
        self.spec
            .ranks
            .get(usize::from(attribute))
            .copied()
            .unwrap_or(0)
    }

    fn has(&self, condition: Condition, now: Ms) -> bool {
        self.conditions
            .get(&condition)
            .is_some_and(|(expires, _)| *expires > now)
    }

    fn attacking(&self) -> bool {
        self.cast.as_ref().is_some_and(|cast| cast.attack)
    }

    fn product(&self, pick: impl Fn(&Resolved) -> Option<f64>) -> f64 {
        self.effects
            .iter()
            .filter_map(|effect| pick(&effect.mods))
            .product()
    }

    fn sum(&self, pick: impl Fn(&Resolved) -> f64) -> f64 {
        self.effects.iter().map(|effect| pick(&effect.mods)).sum()
    }
}

/// An area that pulses.
#[derive(Debug, Clone, PartialEq)]
struct Field {
    caster: u32,
    skill: u32,
    rank: u8,
    position: (f32, f32),
    radius: f32,
    every: Ms,
    next: Ms,
    ends: Ms,
    pulse: Vec<Op>,
}

/// A strike that jumps from foe to foe.
#[derive(Debug, Clone, PartialEq)]
struct Chain {
    caster: u32,
    skill: u32,
    rank: u8,
    last: u32,
    struck: Vec<u32>,
    left: u32,
    every: Ms,
    next: Ms,
    radius: f32,
    hit: Vec<Op>,
}

/// Who an operation is being run for.
#[derive(Debug, Clone, Copy)]
struct Context {
    caster: u32,
    skill: u32,
    rank: u8,
    target: u32,
    field: Option<(f32, f32)>,
    field_radius: f32,
}

/// Every combatant in one instance.
#[derive(Debug, Clone)]
pub struct Engine {
    now: Ms,
    actors: BTreeMap<u32, Actor>,
    fields: Vec<Field>,
    chains: Vec<Chain>,
    next_effect: u32,
    rng: u64,
    events: Vec<Event>,
    /// Creatures a player damaged since the last [`Engine::take_provoked`]: the mob and its attacker.
    provoked: Vec<(u32, u32)>,
}

impl Engine {
    /// An empty instance. `seed` drives weapon rolls and blindness misses.
    pub fn new(seed: u64) -> Engine {
        Engine {
            now: 0,
            actors: BTreeMap::new(),
            fields: Vec::new(),
            chains: Vec::new(),
            next_effect: 1,
            rng: seed | 1,
            events: Vec::new(),
            provoked: Vec::new(),
        }
    }

    /// The instance clock.
    pub fn now(&self) -> Ms {
        self.now
    }

    /// The events left since the last call.
    pub fn drain(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The creatures a player has damaged since the last call, for the brain (`npc::Fight::provoke`).
    /// Every damaged body is reported: a hit wakes a body that waits to be hit and tells an on-sight
    /// body who attacked it even when the attacker stood beyond `aggro_range`.
    pub fn take_provoked(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.provoked)
    }

    pub fn actor(&self, agent: u32) -> Option<&Actor> {
        self.actors.get(&agent)
    }

    pub fn holds(&self, agent: u32) -> bool {
        self.actors.contains_key(&agent)
    }

    // ---- membership --------------------------------------------------------------------------

    /// A combatant enters, at full health and energy. Nothing is published: its own load block or
    /// introduction carries what a client needs.
    pub fn join(&mut self, agent: u32, spec: Spec, position: (f32, f32)) {
        self.actors.insert(agent, Actor::new(agent, spec, position));
    }

    /// A combatant leaves. Its maintained enchantments on others end with it.
    pub fn leave(&mut self, agent: u32) {
        self.actors.remove(&agent);
        let maintained: Vec<(u32, u32)> = self
            .actors
            .values()
            .flat_map(|actor| {
                actor
                    .effects
                    .iter()
                    .filter(|effect| effect.caster == agent && effect.expires.is_none())
                    .map(move |effect| (actor.agent, effect.id))
            })
            .collect();
        for (bearer, id) in maintained {
            self.remove_effect(bearer, id, false);
        }
        self.chains.retain(|chain| chain.caster != agent);
    }

    pub fn set_position(&mut self, agent: u32, position: (f32, f32)) {
        let now = self.now;
        if let Some(actor) = self.actors.get_mut(&agent) {
            // The speed a shot leads by is the last two samples' difference. A second call in one
            // instant (the tick sets positions twice) leaves the estimate alone.
            if actor.sampled_at == 0 {
                actor.sampled_at = now.max(1);
            } else if now > actor.sampled_at {
                let seconds = (now - actor.sampled_at) as f32 / 1000.0;
                actor.velocity = (
                    (position.0 - actor.position.0) / seconds,
                    (position.1 - actor.position.1) / seconds,
                );
                actor.sampled_at = now;
            }
            actor.position = position;
        }
    }

    /// Replace the bar, keeping recharges and adrenaline of the slots that stay.
    pub fn set_bar(&mut self, agent: u32, bar: [Slot; 8]) {
        if let Some(actor) = self.actors.get_mut(&agent) {
            actor.spec.bar = bar;
            actor.adrenaline.retain(|slot, _| bar.contains(slot));
        }
    }

    pub fn set_ranks(&mut self, agent: u32, ranks: [u8; ATTRIBUTES]) {
        if let Some(actor) = self.actors.get_mut(&agent) {
            actor.spec.ranks = ranks;
        }
    }

    pub fn set_equipment(&mut self, agent: u32, weapon: Option<Weapon>, armor: i32) {
        if let Some(actor) = self.actors.get_mut(&agent) {
            actor.spec.weapon = weapon;
            actor.spec.armor = armor;
        }
    }

    /// The vitals a client is told when it takes control of an agent: maxima first, then the
    /// fractions and the rates, preforged's order (`gw_vitals.h` `packets_2022`).
    pub fn vitals(&self, agent: u32, rules: &Rules) -> Vec<Event> {
        let Some(actor) = self.actors.get(&agent) else {
            return Vec::new();
        };
        let mut events = vec![
            Event::MaxEnergy {
                agent,
                max: actor.spec.max_energy,
            },
            Event::MaxHealth {
                agent,
                max: actor.spec.max_health,
            },
        ];
        if actor.spec.max_energy > 0 {
            events.push(Event::Energy {
                agent,
                fraction: (actor.energy / f64::from(actor.spec.max_energy)) as f32,
            });
        }
        if actor.spec.max_health > 0 {
            events.push(Event::Health {
                agent,
                fraction: (actor.health / f64::from(actor.spec.max_health)) as f32,
            });
        }
        events.push(Event::EnergyRegen {
            agent,
            pips: self.energy_pips(agent, rules),
            max: actor.spec.max_energy,
        });
        events
    }

    // ---- using a skill -----------------------------------------------------------------------

    /// A request to use the skill in a bar slot on a target. On refusal the client is answered with
    /// the slot's acknowledgement and nothing is spent.
    pub fn use_skill(
        &mut self,
        caster: u32,
        skill: u32,
        copy: u32,
        target: u32,
        skills: &Skills,
        rules: &Rules,
    ) -> Result<(), Refusal> {
        let result = self.admit(caster, skill, copy, target, skills, rules);
        if result.is_err() && self.actors.contains_key(&caster) {
            self.events.push(Event::SkillAck {
                agent: caster,
                skill,
                copy,
            });
        }
        result
    }

    fn admit(
        &mut self,
        caster: u32,
        skill: u32,
        copy: u32,
        requested: u32,
        skills: &Skills,
        rules: &Rules,
    ) -> Result<(), Refusal> {
        let now = self.now;
        let actor = self.actors.get(&caster).ok_or(Refusal::NoCaster)?;
        if !actor.alive {
            return Err(Refusal::Dead);
        }
        if actor.cast.is_some() || actor.aftercast_until > now {
            return Err(Refusal::Busy);
        }
        if actor.knocked_until > now {
            return Err(Refusal::KnockedDown);
        }
        let slot = (skill, copy);
        if skill == 0 || !actor.spec.bar.contains(&slot) {
            return Err(Refusal::NotOnBar);
        }
        let def = skills.get(skill).ok_or(Refusal::UnknownSkill)?;
        if let Some(why) = def.unsupported() {
            return Err(Refusal::Unsupported(why.to_string()));
        }
        if actor.recharge.get(&slot).is_some_and(|ready| *ready > now) {
            return Err(Refusal::Recharging);
        }
        if actor.used_once.contains(&slot) {
            return Err(Refusal::UsedThisInstance);
        }
        let weapon_ok = match def.weapon {
            None => true,
            Some(required) => actor
                .spec
                .weapon
                .is_some_and(|weapon| weapon.item_type == required),
        };
        if !weapon_ok || (def.kind == SkillType::Attack && actor.spec.weapon.is_none()) {
            return Err(Refusal::NeedsWeapon);
        }
        let target = self.resolve_target(caster, requested, def.target)?;
        if target != caster {
            let reach = if def.kind == SkillType::Attack {
                match actor.spec.weapon {
                    Some(weapon) if weapon.ranged => rules.bow_range,
                    _ => rules.melee_range,
                }
            } else if def.touch {
                rules.touch_range
            } else {
                rules.spell_range
            };
            let there = self.actors[&target].position;
            if distance(actor.position, there) > reach * rules.range_tolerance {
                return Err(Refusal::OutOfRange);
            }
        }
        let discount = if def.kind.is_spell() {
            actor
                .effects
                .iter()
                .filter(|effect| effect.mods.charges > 0)
                .map(|effect| effect.mods.spell_energy_discount)
                .fold(0.0, f64::max)
        } else {
            0.0
        };
        let cost = (f64::from(def.energy) - discount).max(0.0);
        if actor.energy + 1e-9 < cost {
            return Err(Refusal::Energy);
        }
        if def.adrenaline > 0 && actor.adrenaline.get(&slot).copied().unwrap_or(0) < def.adrenaline
        {
            return Err(Refusal::Adrenaline);
        }

        // Admitted: pay, then publish the start.
        let attack = def.kind == SkillType::Attack;
        let period = actor
            .spec
            .weapon
            .map(|weapon| weapon.period_ms)
            .unwrap_or(0);
        let speed = actor.product(|mods| mods.attack_period);
        let max_energy = actor.spec.max_energy;
        let max_health = actor.spec.max_health;
        let actor = self.actors.get_mut(&caster).expect("checked above");
        if cost > 0.0 {
            actor.energy -= cost;
            if discount > 0.0 {
                if let Some(glyph) = actor
                    .effects
                    .iter_mut()
                    .find(|effect| effect.mods.charges > 0)
                {
                    glyph.mods.charges -= 1;
                }
            }
        }
        if def.adrenaline > 0 {
            actor.adrenaline.insert(slot, 0);
        }
        let mut sacrificed = 0.0;
        if def.health_sacrifice > 0 {
            sacrificed = f64::from(max_health) * f64::from(def.health_sacrifice) / 100.0;
            actor.health = (actor.health - sacrificed).max(1.0);
        }
        actor.casting_armor = def.while_casting_armor;
        // A skill takes the hands: the swing in progress is lost, the attack resumes after it.
        actor.swing = None;
        actor.swing_restart = None;
        let completes = if attack {
            now + ((period as f64) * speed).round() as Ms
        } else {
            now + def.activation_ms
        };
        actor.cast = Some(Cast {
            skill,
            copy,
            target,
            completes,
            attack,
        });
        let energy = actor.energy;
        let health = actor.health;
        if cost > 0.0 && max_energy > 0 {
            self.events.push(Event::Energy {
                agent: caster,
                fraction: (energy / f64::from(max_energy)) as f32,
            });
        }
        if def.adrenaline > 0 {
            self.events.push(Event::AdrenalineSpent {
                agent: caster,
                skill,
                copy,
            });
        }
        if sacrificed > 0.0 {
            self.events.push(Event::Health {
                agent: caster,
                fraction: (health / f64::from(max_health)) as f32,
            });
        }
        if attack {
            self.events.push(Event::AttackTiming {
                agent: caster,
                interval: (period as f64 * speed / 1000.0) as f32,
                modifier: speed as f32,
            });
        }
        self.events.push(Event::SkillActivate {
            agent: caster,
            skill,
            copy,
        });
        if def.activation_ms == 0 && !attack {
            self.events.push(Event::Instant { caster, skill });
            self.complete(caster, skills, rules);
        } else {
            self.events.push(Event::CastStart {
                caster,
                target,
                skill,
                attack,
            });
        }
        Ok(())
    }

    /// The agent a request lands on under the skill's target rule.
    fn resolve_target(
        &self,
        caster: u32,
        requested: u32,
        rule: TargetRule,
    ) -> Result<u32, Refusal> {
        let me = &self.actors[&caster];
        let them = self.actors.get(&requested);
        match rule {
            TargetRule::Caster => Ok(caster),
            TargetRule::Ally => match them {
                // An ally spell with no target, or a foe targeted, lands on the caster as the client
                // itself does.
                None => Ok(caster),
                Some(other) if other.alive && allied(me.spec.side, other.spec.side) => {
                    Ok(requested)
                }
                Some(_) => Ok(caster),
            },
            TargetRule::Foe => match them {
                Some(other) if other.alive && hostile(me.spec.side, other.spec.side) => {
                    Ok(requested)
                }
                _ => Err(Refusal::BadTarget),
            },
            TargetRule::DeadAlly => match them {
                Some(other)
                    if !other.alive
                        && requested != caster
                        && allied(me.spec.side, other.spec.side) =>
                {
                    Ok(requested)
                }
                _ => Err(Refusal::BadTarget),
            },
            TargetRule::Animal => Err(Refusal::BadTarget),
        }
    }

    /// Stop the caster's skill, as a cancel or a knockdown does. An interrupt also starts the recharge.
    pub fn cancel(&mut self, agent: u32, recharge: bool, skills: &Skills) {
        let now = self.now;
        let Some(actor) = self.actors.get_mut(&agent) else {
            return;
        };
        let Some(cast) = actor.cast.take() else {
            return;
        };
        actor.casting_armor = 0;
        self.events.push(Event::SkillAck {
            agent,
            skill: cast.skill,
            copy: cast.copy,
        });
        self.events.push(Event::CastCancel {
            caster: agent,
            attack: cast.attack,
        });
        if recharge {
            if let Some(def) = skills.get(cast.skill) {
                if def.recharge_ms > 0 {
                    actor
                        .recharge
                        .insert((cast.skill, cast.copy), now + def.recharge_ms);
                    self.events.push(Event::SkillRecharge {
                        agent,
                        skill: cast.skill,
                        copy: cast.copy,
                        seconds: (def.recharge_ms / 1000) as u32,
                    });
                }
            }
        }
    }

    fn complete(&mut self, caster: u32, skills: &Skills, rules: &Rules) {
        let now = self.now;
        let Some(actor) = self.actors.get_mut(&caster) else {
            return;
        };
        let Some(cast) = actor.cast.take() else {
            return;
        };
        actor.casting_armor = 0;
        let Some(def) = skills.get(cast.skill) else {
            return;
        };
        let rank = if def.attribute < NO_ATTRIBUTE {
            actor.rank(def.attribute)
        } else {
            0
        };
        let ranged = actor.spec.weapon.is_some_and(|weapon| weapon.ranged);

        // The target must still be what the rule asked for.
        let target_ok = match self.actors.get(&cast.target) {
            None => false,
            Some(target) => match def.target {
                TargetRule::DeadAlly => !target.alive,
                _ => target.alive,
            },
        };
        if !target_ok {
            self.events.push(Event::SkillAck {
                agent: caster,
                skill: cast.skill,
                copy: cast.copy,
            });
            self.events.push(Event::CastCancel {
                caster,
                attack: cast.attack,
            });
            return;
        }

        // The finish: 58 for a cast, 46 for a melee attack; an instant skill and a bow attack send
        // none (preforged `finish_packets`).
        let instant = def.activation_ms == 0 && !cast.attack;
        if !instant && !(cast.attack && ranged) {
            self.events.push(Event::CastFinish {
                caster,
                attack: cast.attack,
            });
        }
        if let Some(resource) = def.caster_body {
            self.events.push(Event::CasterBody {
                agent: caster,
                resource,
            });
        }
        if let Some(resource) = def.target_body {
            self.events.push(Event::TargetBody {
                target: cast.target,
                caster,
                resource,
            });
        }

        // A spell's reactions on its caster come before what the spell does.
        if def.kind.is_spell() {
            self.on_spell_cast(caster, def, rules);
        }

        let context = Context {
            caster,
            skill: def.id,
            rank,
            target: cast.target,
            field: None,
            field_radius: 0.0,
        };
        for op in &def.effects {
            self.run(op, context, def, skills, rules);
        }

        let slot = (cast.skill, cast.copy);
        self.events.push(Event::SkillAck {
            agent: caster,
            skill: cast.skill,
            copy: cast.copy,
        });
        let once = def.effects.iter().any(|op| {
            matches!(
                op,
                Op::Resurrect {
                    once_per_instance: true,
                    ..
                }
            )
        });
        if let Some(actor) = self.actors.get_mut(&caster) {
            actor.aftercast_until = now + def.aftercast_ms;
            if once {
                actor.used_once.insert(slot);
                self.events.push(Event::SkillSpent {
                    agent: caster,
                    skill: cast.skill,
                    copy: cast.copy,
                });
            } else if def.recharge_ms > 0 {
                actor.recharge.insert(slot, now + def.recharge_ms);
                self.events.push(Event::SkillRecharge {
                    agent: caster,
                    skill: cast.skill,
                    copy: cast.copy,
                    seconds: (def.recharge_ms / 1000) as u32,
                });
            }
        }
    }

    /// Backfire and Aura of Restoration: what casting any spell does to its caster.
    fn on_spell_cast(&mut self, caster: u32, def: &SkillDef, rules: &Rules) {
        let Some(actor) = self.actors.get(&caster) else {
            return;
        };
        let backfires: Vec<(u32, u32, f64)> = actor
            .effects
            .iter()
            .filter(|effect| effect.mods.damage_on_spell > 0.0)
            .map(|effect| (effect.caster, effect.skill, effect.mods.damage_on_spell))
            .collect();
        let aura = actor.sum(|mods| mods.spell_heal_percent_of_cost);
        for (source, skill, amount) in backfires {
            self.hurt(
                source,
                caster,
                skill,
                DamageKind::Untyped,
                amount,
                0.0,
                false,
                rules,
            );
        }
        if aura > 0.0 {
            let heal = (f64::from(def.energy) * aura / 100.0).round();
            self.heal(caster, caster, heal);
        }
    }

    // ---- executing a declaration -------------------------------------------------------------

    fn amount(&self, amount: &Amount, def: &SkillDef, rank: u8) -> f64 {
        match amount {
            Amount::Scale => f64::from(interpolate(def.scale.0, def.scale.1, rank)),
            Amount::Bonus => f64::from(interpolate(def.bonus.0, def.bonus.1, rank)),
            Amount::Duration => f64::from(interpolate(def.duration.0, def.duration.1, rank)),
            Amount::Range(low, high) => f64::from(interpolate(*low, *high, rank)),
            Amount::Fixed(value) => *value,
            Amount::Negate(inner) => -self.amount(inner, def, rank),
        }
    }

    fn recipients(&self, area: Area, context: Context, rules: &Rules) -> Vec<u32> {
        let Some(caster) = self.actors.get(&context.caster) else {
            return Vec::new();
        };
        let side = caster.spec.side;
        let foes_near = |centre: (f32, f32), radius: f32, except: Option<u32>| -> Vec<u32> {
            self.actors
                .values()
                .filter(|actor| {
                    actor.alive
                        && hostile(side, actor.spec.side)
                        && Some(actor.agent) != except
                        && distance(actor.position, centre) <= radius
                })
                .map(|actor| actor.agent)
                .collect()
        };
        match area {
            Area::Caster => vec![context.caster],
            Area::Target => vec![context.target],
            Area::TargetAndAdjacentFoes => {
                let mut list = vec![context.target];
                if let Some(target) = self.actors.get(&context.target) {
                    list.extend(foes_near(
                        target.position,
                        rules.adjacent_range,
                        Some(context.target),
                    ));
                }
                list
            }
            Area::AdjacentFoesOfCaster => foes_near(caster.position, rules.adjacent_range, None),
            Area::FoesInField => match context.field {
                Some(centre) => foes_near(centre, context.field_radius, None),
                None => Vec::new(),
            },
        }
    }

    fn guard_holds(&self, guard: Option<Guard>, target: u32) -> bool {
        let Some(guard) = guard else {
            return true;
        };
        let Some(actor) = self.actors.get(&target) else {
            return false;
        };
        match guard {
            Guard::Bleeding => actor.has(Condition::Bleeding, self.now),
            Guard::Attacking => actor.attacking(),
        }
    }

    fn run(&mut self, op: &Op, context: Context, def: &SkillDef, skills: &Skills, rules: &Rules) {
        let rank = context.rank;
        match op {
            Op::Attack {
                to,
                bonus,
                strikes,
                multiplier,
                on_hit,
            } => {
                let bonus = self.amount(bonus, def, rank);
                // Empathy: attacking hurts the attacker, once per attack.
                let empathy: Vec<(u32, u32, f64)> = self
                    .actors
                    .get(&context.caster)
                    .map(|actor| {
                        actor
                            .effects
                            .iter()
                            .filter(|effect| effect.mods.damage_on_attack > 0.0)
                            .map(|effect| {
                                (effect.caster, effect.skill, effect.mods.damage_on_attack)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                for (source, skill, amount) in empathy {
                    self.hurt(
                        source,
                        context.caster,
                        skill,
                        DamageKind::Untyped,
                        amount,
                        0.0,
                        false,
                        rules,
                    );
                }
                let mut any_hit = false;
                for target in self.recipients(*to, context, rules) {
                    for _ in 0..(*strikes).max(1) {
                        if self.strike(context, target, bonus, *multiplier, rules) {
                            any_hit = true;
                            let hit_context = Context { target, ..context };
                            for follow in on_hit {
                                self.run(follow, hit_context, def, skills, rules);
                            }
                        }
                    }
                }
                if any_hit {
                    self.gain_adrenaline(context.caster, skills, rules);
                }
            }
            Op::Damage { to, kind, amount } => {
                let amount = self.amount(amount, def, rank);
                let strike = self.spell_strike(context.caster, rules);
                for target in self.recipients(*to, context, rules) {
                    self.hurt(
                        context.caster,
                        target,
                        context.skill,
                        *kind,
                        amount,
                        strike,
                        false,
                        rules,
                    );
                }
            }
            Op::Heal {
                to,
                amount,
                divine_favor,
            } => {
                let mut amount = self.amount(amount, def, rank);
                if *divine_favor {
                    if let Some(caster) = self.actors.get(&context.caster) {
                        if caster.spec.primary == rules.monk_profession {
                            amount += (rules.divine_favor_heal_per_rank
                                * f64::from(caster.rank(rules.divine_favor_attribute)))
                            .round();
                        }
                    }
                }
                for target in self.recipients(*to, context, rules) {
                    self.heal(context.caster, target, amount);
                }
            }
            Op::LifeSteal { amount } => {
                let amount = self.amount(amount, def, rank);
                let stolen = self
                    .actors
                    .get(&context.target)
                    .map(|target| amount.min(target.health))
                    .unwrap_or(0.0);
                if stolen > 0.0
                    && self
                        .hurt(
                            context.caster,
                            context.target,
                            context.skill,
                            DamageKind::Untyped,
                            stolen,
                            0.0,
                            false,
                            rules,
                        )
                        .is_some()
                {
                    self.heal(context.caster, context.caster, stolen);
                }
            }
            Op::Condition {
                to,
                condition,
                duration,
                guard,
            } => {
                let seconds = self.amount(duration, def, rank);
                for target in self.recipients(*to, context, rules) {
                    if self.guard_holds(*guard, target) {
                        self.inflict(target, *condition, (seconds * 1000.0) as Ms, rules);
                    }
                }
            }
            Op::Knockdown { to, seconds, guard } => {
                let seconds = self.amount(seconds, def, rank);
                let ms = if seconds > 0.0 {
                    (seconds * 1000.0) as Ms
                } else {
                    rules.knockdown_ms
                };
                for target in self.recipients(*to, context, rules) {
                    if self.guard_holds(*guard, target) {
                        self.knock_down(target, ms, skills);
                    }
                }
            }
            Op::Interrupt { to, guard } => {
                for target in self.recipients(*to, context, rules) {
                    if self.guard_holds(*guard, target) {
                        self.cancel(target, true, skills);
                    }
                }
            }
            Op::Effect { to, duration, mods } => {
                let ms = duration
                    .as_ref()
                    .map(|duration| (self.amount(duration, def, rank) * 1000.0) as Ms);
                let resolved = self.resolve(mods, def, rank);
                for target in self.recipients(*to, context, rules) {
                    self.apply_effect(
                        context.caster,
                        target,
                        def,
                        rank,
                        ms,
                        resolved.clone(),
                        rules,
                    );
                }
            }
            Op::Field {
                at_target,
                duration,
                every_ms,
                radius,
                pulse,
            } => {
                let centre = if *at_target {
                    context.target
                } else {
                    context.caster
                };
                let Some(position) = self.actors.get(&centre).map(|actor| actor.position) else {
                    return;
                };
                let seconds = self.amount(duration, def, rank);
                let every = (*every_ms).max(1);
                self.fields.push(Field {
                    caster: context.caster,
                    skill: def.id,
                    rank,
                    position,
                    radius: if *radius > 0.0 { *radius } else { def.aoe },
                    every,
                    next: self.now + every,
                    ends: self.now + (seconds * 1000.0) as Ms,
                    pulse: pulse.clone(),
                });
            }
            Op::Chain {
                strikes,
                every_ms,
                radius,
                hit,
            } => {
                for follow in hit {
                    self.run(follow, context, def, skills, rules);
                }
                if *strikes > 1 {
                    self.chains.push(Chain {
                        caster: context.caster,
                        skill: def.id,
                        rank,
                        last: context.target,
                        struck: vec![context.target],
                        left: strikes - 1,
                        every: (*every_ms).max(1),
                        next: self.now + (*every_ms).max(1),
                        radius: if *radius > 0.0 { *radius } else { def.aoe },
                        hit: hit.clone(),
                    });
                }
            }
            Op::DrainEnergy {
                amount,
                caster_heal_per_point,
            } => {
                let wanted = self.amount(amount, def, rank);
                let per_point = self.amount(caster_heal_per_point, def, rank);
                let lost = match self.actors.get_mut(&context.target) {
                    Some(target) => {
                        let lost = wanted.min(target.energy).max(0.0).floor();
                        target.energy -= lost;
                        let (energy, max) = (target.energy, target.spec.max_energy);
                        if lost > 0.0 && max > 0 {
                            self.events.push(Event::Energy {
                                agent: context.target,
                                fraction: (energy / f64::from(max)) as f32,
                            });
                        }
                        lost
                    }
                    None => 0.0,
                };
                if lost > 0.0 {
                    self.heal(context.caster, context.caster, lost * per_point);
                }
            }
            Op::RemoveHex { profession, then } => {
                let removed = self.actors.get(&context.target).and_then(|target| {
                    target
                        .effects
                        .iter()
                        .rev()
                        .find(|effect| {
                            effect.kind == SkillType::Hex && effect.profession == *profession
                        })
                        .map(|effect| effect.id)
                });
                if let Some(id) = removed {
                    self.remove_effect(context.target, id, false);
                    for follow in then {
                        self.run(follow, context, def, skills, rules);
                    }
                }
            }
            Op::Resurrect {
                health_percent,
                energy_percent,
                ..
            } => {
                let health = self.amount(health_percent, def, rank);
                let energy = self.amount(energy_percent, def, rank);
                self.resurrect(context.target, health, energy, rules);
            }
            Op::LoseAdrenaline => {
                if let Some(actor) = self.actors.get_mut(&context.caster) {
                    let slots: Vec<Slot> = actor.adrenaline.keys().copied().collect();
                    for slot in slots {
                        actor.adrenaline.insert(slot, 0);
                        self.events.push(Event::Adrenaline {
                            agent: context.caster,
                            skill: slot.0,
                            copy: slot.1,
                            units: 0,
                        });
                    }
                }
            }
            Op::Unsupported(_) => {}
        }
    }

    fn resolve(&self, mods: &Mods, def: &SkillDef, rank: u8) -> Resolved {
        let value = |amount: &Option<Amount>| {
            amount
                .as_ref()
                .map(|amount| self.amount(amount, def, rank))
                .unwrap_or(0.0)
        };
        Resolved {
            attack_period: mods.attack_period,
            damage_taken: mods.damage_taken,
            health_regen: value(&mods.health_regen) as i32,
            caster_health_regen: value(&mods.caster_health_regen) as i32,
            damage_reduction: value(&mods.damage_reduction),
            heal_instead_of_damage: value(&mods.heal_instead_of_damage),
            attacker_damage: value(&mods.attacker_damage),
            damage_on_spell: value(&mods.damage_on_spell),
            damage_on_attack: value(&mods.damage_on_attack),
            damage_on_enchant_or_hex: value(&mods.damage_on_enchant_or_hex),
            heal_on_end: value(&mods.heal_on_end),
            spell_heal_percent_of_cost: value(&mods.spell_heal_percent_of_cost),
            spell_energy_discount: value(&mods.spell_energy_discount),
            charges: value(&mods.charges).max(0.0) as u32,
            move_speed: mods.move_speed,
            arrow_damage: value(&mods.arrow_damage),
            arrow_explosion: value(&mods.arrow_explosion),
            arrow_explosion_kind: mods.arrow_explosion_kind,
        }
    }

    fn spell_strike(&self, caster: u32, rules: &Rules) -> f64 {
        self.actors
            .get(&caster)
            .map(|actor| rules.spell_strike_per_level * f64::from(actor.spec.level))
            .unwrap_or(0.0)
    }

    /// One weapon strike. Returns whether it hit.
    fn strike(
        &mut self,
        context: Context,
        target: u32,
        bonus: f64,
        multiplier: f64,
        rules: &Rules,
    ) -> bool {
        let Some(attacker) = self.actors.get(&context.caster) else {
            return false;
        };
        let Some(weapon) = attacker.spec.weapon else {
            return false;
        };
        let blind = attacker.has(Condition::Blind, self.now);
        let strike = rules.weapon_strike_per_rank * f64::from(attacker.rank(weapon.attribute));
        let arrow = if weapon.ranged {
            attacker.sum(|mods| mods.arrow_damage)
        } else {
            0.0
        };
        let explosion = if weapon.ranged {
            attacker
                .effects
                .iter()
                .find(|effect| effect.mods.arrow_explosion > 0.0)
                .map(|effect| {
                    (
                        effect.skill,
                        effect.mods.arrow_explosion,
                        effect.mods.arrow_explosion_kind.unwrap_or(DamageKind::Fire),
                    )
                })
        } else {
            None
        };
        let level_strike = rules.spell_strike_per_level * f64::from(attacker.spec.level);
        if blind && self.roll(100) < 90 {
            return false;
        }
        let spread = weapon.max.saturating_sub(weapon.min);
        let rolled = f64::from(weapon.min + self.roll(spread + 1));
        let base = (rolled + bonus + arrow) * multiplier;
        let hit = self
            .hurt(
                context.caster,
                target,
                context.skill,
                weapon.kind,
                base,
                strike,
                true,
                rules,
            )
            .is_some();
        if let Some((skill, amount, kind)) = explosion {
            let around = Context { target, ..context };
            for victim in self.recipients(Area::TargetAndAdjacentFoes, around, rules) {
                self.hurt(
                    context.caster,
                    victim,
                    skill,
                    kind,
                    amount,
                    level_strike,
                    false,
                    rules,
                );
            }
        }
        hit
    }

    fn gain_adrenaline(&mut self, agent: u32, skills: &Skills, rules: &Rules) {
        let Some(actor) = self.actors.get_mut(&agent) else {
            return;
        };
        for slot in actor.spec.bar {
            let Some(def) = skills.get(slot.0) else {
                continue;
            };
            if def.adrenaline == 0 {
                continue;
            }
            let units = actor.adrenaline.entry(slot).or_insert(0);
            let next = (*units + rules.adrenaline_per_hit).min(def.adrenaline);
            if next != *units {
                *units = next;
                self.events.push(Event::Adrenaline {
                    agent,
                    skill: slot.0,
                    copy: slot.1,
                    units: next,
                });
            }
        }
    }

    // ---- health ------------------------------------------------------------------------------

    /// Damage from `source` to `target`. Returns what was applied, or `None` when the target could not
    /// be damaged.
    #[allow(clippy::too_many_arguments)]
    fn hurt(
        &mut self,
        source: u32,
        target: u32,
        skill: u32,
        kind: DamageKind,
        base: f64,
        strike: f64,
        attack: bool,
        rules: &Rules,
    ) -> Option<f64> {
        let source_side = self.actors.get(&source).map(|actor| actor.spec.side);
        let victim = self.actors.get(&target)?;
        if !victim.alive || base <= 0.0 {
            return None;
        }
        // Only foes are damaged; a hex's own reaction (Backfire, Empathy) is the foe's caster hurting it.
        if let Some(side) = source_side {
            if source != target && !hostile(side, victim.spec.side) {
                return None;
            }
        }
        let mut amount = if kind.respects_armor() {
            let armor = f64::from(victim.spec.armor + victim.casting_armor);
            base * ((strike - armor) / rules.armor_divisor).exp2()
        } else {
            base
        };
        amount *= victim.product(|mods| mods.damage_taken);
        amount = (amount - victim.sum(|mods| mods.damage_reduction)).max(0.0);
        // Health is whole points on the live client: a fractional hit would leave a body at a sliver
        // of health that never reaches zero.
        amount = amount.round();

        // Reversal of Fortune: the first damage heals instead, up to its limit.
        let reversal = victim
            .effects
            .iter()
            .find(|effect| effect.mods.heal_instead_of_damage > 0.0)
            .map(|effect| (effect.id, effect.mods.heal_instead_of_damage));
        let retribution: Vec<(u32, u32, f64)> = if attack {
            victim
                .effects
                .iter()
                .filter(|effect| effect.mods.attacker_damage > 0.0)
                .map(|effect| (effect.caster, effect.skill, effect.mods.attacker_damage))
                .collect()
        } else {
            Vec::new()
        };
        if let Some((id, limit)) = reversal {
            if amount > 0.0 {
                let converted = amount.min(limit);
                self.remove_effect(target, id, false);
                self.heal(target, target, converted);
                amount -= converted;
            }
        }
        if amount <= 0.0 {
            return Some(0.0);
        }
        let victim = self.actors.get_mut(&target)?;
        victim.health = (victim.health - amount).max(0.0);
        if victim.health < 1.0 {
            victim.health = 0.0;
        }
        let (health, max) = (victim.health, victim.spec.max_health);
        self.events.push(Event::Damage {
            target,
            source,
            skill,
            amount,
            max,
            armor_ignoring: !kind.respects_armor(),
            health,
        });
        if amount > 0.0
            && source != target
            && self
                .actors
                .get(&source)
                .is_some_and(|attacker| attacker.spec.side == Side::Player)
        {
            self.provoked.push((target, source));
        }
        if health <= 0.0 {
            self.die(target, skills_none());
        } else {
            // Retribution: the attacker takes damage from the enchantment's caster.
            for (caster, skill, damage) in retribution {
                self.hurt(
                    caster,
                    source,
                    skill,
                    DamageKind::Untyped,
                    damage,
                    0.0,
                    false,
                    rules,
                );
            }
        }
        Some(amount)
    }

    fn heal(&mut self, source: u32, target: u32, amount: f64) {
        let Some(actor) = self.actors.get_mut(&target) else {
            return;
        };
        if !actor.alive || amount <= 0.0 {
            return;
        }
        let max = actor.spec.max_health;
        actor.health = (actor.health + amount).min(f64::from(max));
        let health = actor.health;
        self.events.push(Event::Heal {
            target,
            source,
            amount,
            max,
            health,
        });
    }

    fn die(&mut self, agent: u32, skills: &Skills) {
        let Some(actor) = self.actors.get_mut(&agent) else {
            return;
        };
        if !actor.alive {
            return;
        }
        let player = actor.spec.side == Side::Player;
        actor.alive = false;
        actor.health = 0.0;
        actor.attack_target = None;
        actor.swing = None;
        actor.swing_restart = None;
        if actor.cast.is_some() {
            self.cancel(agent, false, skills);
        }
        let actor = self.actors.get_mut(&agent).expect("present");
        let effects: Vec<u32> = actor.effects.iter().map(|effect| effect.id).collect();
        let conditions: Vec<u32> = actor.conditions.values().map(|(_, id)| *id).collect();
        actor.conditions.clear();
        for id in effects {
            self.remove_effect(agent, id, false);
        }
        for id in conditions {
            self.events.push(Event::EffectRemove { bearer: agent, id });
        }
        // The enchantments it maintains on others end too.
        let maintained: Vec<(u32, u32)> = self
            .actors
            .values()
            .flat_map(|other| {
                other
                    .effects
                    .iter()
                    .filter(|effect| effect.caster == agent && effect.expires.is_none())
                    .map(move |effect| (other.agent, effect.id))
            })
            .collect();
        for (bearer, id) in maintained {
            self.remove_effect(bearer, id, false);
        }
        self.events.push(Event::Death { agent, player });
        if let Some(actor) = self.actors.get_mut(&agent) {
            actor.shown_flags = flags::DEAD;
        }
    }

    fn resurrect(&mut self, agent: u32, health_percent: f64, energy_percent: f64, rules: &Rules) {
        let Some(actor) = self.actors.get_mut(&agent) else {
            return;
        };
        if actor.alive {
            return;
        }
        actor.alive = true;
        actor.health = (f64::from(actor.spec.max_health) * health_percent / 100.0).max(1.0);
        actor.energy = f64::from(actor.spec.max_energy) * energy_percent / 100.0;
        actor.shown_flags = 0;
        let player = actor.spec.side == Side::Player;
        self.events.push(Event::Resurrected { agent, player });
        let vitals = self.vitals(agent, rules);
        self.events.extend(vitals);
        self.events.push(Event::Flags { agent, flags: 0 });
    }

    /// Kill an agent outright, as `/resign` does.
    pub fn kill(&mut self, agent: u32, skills: &Skills) {
        self.die(agent, skills);
    }

    /// Put an agent's ordinary attack on `target`: it swings whenever it is free, in reach and not
    /// knocked down, until [`Engine::stop_attack`], its death or the target's. The client walks a
    /// player into reach itself; a creature is walked there by its brain (`crate::npc::fight`).
    pub fn attack(&mut self, agent: u32, target: u32) -> Result<(), Refusal> {
        let me = self.actors.get(&agent).ok_or(Refusal::NoCaster)?;
        if !me.alive {
            return Err(Refusal::Dead);
        }
        if me.spec.weapon.is_none() {
            return Err(Refusal::NeedsWeapon);
        }
        let them = self.actors.get(&target).ok_or(Refusal::BadTarget)?;
        if !them.alive || !hostile(me.spec.side, them.spec.side) {
            return Err(Refusal::BadTarget);
        }
        let actor = self.actors.get_mut(&agent).expect("checked above");
        if actor.attack_target != Some(target) {
            actor.swing = None;
            actor.swing_restart = None;
        }
        actor.attack_target = Some(target);
        Ok(())
    }

    /// How near an agent must stand to a foe to swing at it with what it holds: the reach `swing`
    /// tests, without its tolerance. `None` for an agent with no weapon or not in the fight.
    pub fn attack_reach(&self, agent: u32, rules: &Rules) -> Option<f32> {
        let weapon = self.actors.get(&agent)?.spec.weapon?;
        Some(if weapon.ranged {
            rules.bow_range
        } else {
            rules.melee_range
        })
    }

    /// Stop an agent's ordinary attack; a swing in progress does not land.
    pub fn stop_attack(&mut self, agent: u32) {
        if let Some(actor) = self.actors.get_mut(&agent) {
            actor.attack_target = None;
            actor.swing = None;
            actor.swing_restart = None;
        }
    }

    /// Bring a living agent back to full health and energy with nothing on it: a creature that gave
    /// up a chase and walked home.
    pub fn restore(&mut self, agent: u32, rules: &Rules) {
        let Some(actor) = self.actors.get(&agent) else {
            return;
        };
        if !actor.alive {
            return;
        }
        let effects: Vec<u32> = actor.effects.iter().map(|effect| effect.id).collect();
        let conditions: Vec<u32> = actor.conditions.values().map(|(_, id)| *id).collect();
        for id in effects {
            self.remove_effect(agent, id, false);
        }
        let actor = self.actors.get_mut(&agent).expect("present");
        actor.conditions.clear();
        actor.health = f64::from(actor.spec.max_health);
        actor.energy = f64::from(actor.spec.max_energy);
        for id in conditions {
            self.events.push(Event::EffectRemove { bearer: agent, id });
        }
        self.events.push(Event::Health {
            agent,
            fraction: 1.0,
        });
        self.publish_flags(agent);
        let _ = rules;
    }

    /// Bring a dead agent back at full health and energy: a creature's respawn.
    pub fn revive(&mut self, agent: u32, position: (f32, f32), rules: &Rules) {
        if let Some(actor) = self.actors.get_mut(&agent) {
            actor.position = position;
        }
        self.resurrect(agent, 100.0, 100.0, rules);
    }

    /// Start, land and repeat ordinary attacks.
    fn swing(&mut self, skills: &Skills, rules: &Rules) {
        let now = self.now;
        let agents: Vec<u32> = self
            .actors
            .values()
            .filter(|actor| actor.attack_target.is_some())
            .map(|actor| actor.agent)
            .collect();
        for agent in agents {
            let Some(actor) = self.actors.get(&agent) else {
                continue;
            };
            let Some(target) = actor.attack_target else {
                continue;
            };
            let Some(weapon) = actor.spec.weapon else {
                continue;
            };
            let target_alive = self.actors.get(&target).is_some_and(|them| them.alive);
            if !actor.alive || !target_alive {
                let actor = self.actors.get_mut(&agent).expect("present");
                actor.attack_target = None;
                actor.swing = None;
                actor.swing_restart = None;
                continue;
            }
            let reach = if weapon.ranged {
                rules.bow_range
            } else {
                rules.melee_range
            } * rules.range_tolerance;
            let there = self.actors[&target].position;
            let in_reach = distance(actor.position, there) <= reach;
            if let Some(mut state) = actor.swing {
                // The missile leaves at the windup, so it arrives at `lands`. Every swing's flight is
                // its own: the distance over the pack's projectile speed, so a shot at 100 units
                // takes 0.06 s and one at 1,000 takes 0.63 s (`combat.json` `projectile_speed`).
                if !state.launched && state.launch <= now {
                    if let Some((model, flag)) = weapon.projectile {
                        // Fire: the aim leads the target's position and velocity now, by this
                        // shot's flight, and the flight is the distance to that aim.
                        let origin = self.actors[&agent].position;
                        let at = self.actors[&target].position;
                        let velocity = self.actors[&target].velocity;
                        let (aim, flight) = aim_point(origin, at, velocity, rules.projectile_speed);
                        let lands = now + flight;
                        let actor = self.actors.get_mut(&agent).expect("present");
                        actor.shots = actor.shots % 255 + 1;
                        let counter = actor.shots;
                        if let Some(swing) = actor.swing.as_mut() {
                            swing.launched = true;
                            swing.origin = origin;
                            swing.aim = aim;
                            swing.lands = lands;
                        }
                        if lands > now {
                            actor.in_flight = Some((lands, counter, weapon.kind));
                        }
                        self.events.push(Event::Projectile {
                            agent,
                            to: aim,
                            seconds: flight as f32 / 1000.0,
                            model,
                            counter,
                            flag,
                        });
                        // A flight that fits inside this step (a shot at the shooter's feet) is
                        // closed now, before the hit's word.
                        if lands <= now {
                            self.events.push(Event::ProjectileArrived {
                                agent,
                                counter,
                                kind: weapon.kind.wire_word(),
                            });
                        }
                        state.launched = true;
                        state.origin = origin;
                        state.aim = aim;
                        state.lands = lands;
                    } else {
                        let actor = self.actors.get_mut(&agent).expect("present");
                        if let Some(swing) = actor.swing.as_mut() {
                            swing.launched = true;
                        }
                        state.launched = true;
                    }
                }
                if state.launched && state.lands <= now {
                    let actor = self.actors.get_mut(&agent).expect("present");
                    actor.swing = None;
                    let shot = weapon.ranged && weapon.projectile.is_some();
                    let connects_now = if shot {
                        let at = self.actors[&target].position;
                        connects(
                            state.origin,
                            state.aim,
                            at,
                            rules.dodge_tolerance,
                            rules.dodge_along,
                        )
                    } else {
                        in_reach
                    };
                    if state.target == target && connects_now {
                        let context = Context {
                            caster: agent,
                            skill: 0,
                            rank: 0,
                            target,
                            field: None,
                            field_radius: 0.0,
                        };
                        // Retail's hit batch opens with property 1 (the melee finish), then the damage
                        // word: `[1, me, 0]` before the word on 1,376 of 1,376 live hits (Rurik,
                        // `toolkit/authsrv/authsrv.py:20206-20218`). The client latches the attacker on
                        // property 1 and splices the damage into that agent's pending animation; with it
                        // after the word the number waits for the next animation (the owner's delayed,
                        // then double hit). So insert the finish ahead of whatever the strike emits.
                        let before = self.events.len();
                        if self.strike(context, target, 0.0, 1.0, rules) {
                            self.events
                                .insert(before, Event::AttackFinished { attacker: agent });
                            self.gain_adrenaline(agent, skills, rules);
                        }
                    } else if state.target == target && shot {
                        self.events.push(Event::AttackFailed {
                            attacker: agent,
                            target,
                        });
                    }
                }
                if state.lands > now {
                    continue;
                }
            }
            // The next swing starts as soon as the last one lands: an attacker is never idle between
            // two swings at the same foe.
            let Some(actor) = self.actors.get(&agent) else {
                continue;
            };
            let target_alive = self.actors.get(&target).is_some_and(|them| them.alive);
            let busy = !actor.alive
                || actor.cast.is_some()
                || actor.aftercast_until > now
                || actor.knocked_until > now;
            if busy || !in_reach || !target_alive || actor.attack_target != Some(target) {
                continue;
            }
            // The contact is at the windup, but the next swing opens a full interval after this one
            // started: an attacker is never idle between two contacts, and never faster than its
            // declared cadence.
            if actor.swing.is_none() && actor.swing_restart.is_some_and(|restart| now < restart) {
                continue;
            }
            let speed = actor.product(|mods| mods.attack_period);
            let period = (weapon.period_ms as f64 * speed).round() as Ms;
            let windup = swing_windup(period);
            let launch = now + windup;
            let actor = self.actors.get_mut(&agent).expect("present");
            // The aim is fixed when the missile leaves, not when the swing opens: the target moves
            // through the windup. Until then this swing lands where it started.
            actor.swing = Some(Swing {
                target,
                origin: actor.position,
                aim: there,
                launch,
                lands: launch,
                launched: false,
            });
            actor.swing_restart = Some(now + period.max(1));
            self.events.push(Event::AttackTiming {
                agent,
                interval: (period as f64 / 1000.0) as f32,
                modifier: speed as f32,
            });
            self.events.push(Event::AttackStart {
                attacker: agent,
                target,
            });
        }
    }

    // ---- conditions and effects --------------------------------------------------------------

    fn inflict(&mut self, target: u32, condition: Condition, ms: Ms, rules: &Rules) {
        let now = self.now;
        let id = self.next_effect;
        let Some(actor) = self.actors.get_mut(&target) else {
            return;
        };
        if !actor.alive || ms == 0 {
            return;
        }
        let expires = now + ms;
        let previous = actor.conditions.get(&condition).copied();
        // Reapplication never shortens what is already running.
        if let Some((old, _)) = previous {
            if old >= expires {
                return;
            }
        }
        self.next_effect += 1;
        actor.conditions.insert(condition, (expires, id));
        if let Some((_, old_id)) = previous {
            self.events.push(Event::EffectRemove {
                bearer: target,
                id: old_id,
            });
        }
        let skill = rules.condition_skills.get(&condition).copied().unwrap_or(0);
        self.events.push(Event::EffectAdd {
            bearer: target,
            skill,
            rank: 0,
            id,
            seconds: ms as f32 / 1000.0,
        });
        self.publish_flags(target);
    }

    fn knock_down(&mut self, target: u32, ms: Ms, skills: &Skills) {
        let now = self.now;
        let Some(actor) = self.actors.get_mut(&target) else {
            return;
        };
        if !actor.alive {
            return;
        }
        actor.knocked_until = actor.knocked_until.max(now + ms);
        actor.swing = None;
        actor.swing_restart = None;
        self.events.push(Event::Knockdown {
            agent: target,
            seconds: ms as f32 / 1000.0,
        });
        self.cancel(target, true, skills);
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_effect(
        &mut self,
        caster: u32,
        bearer: u32,
        def: &SkillDef,
        rank: u8,
        ms: Option<Ms>,
        mods: Resolved,
        rules: &Rules,
    ) {
        let now = self.now;
        let Some(actor) = self.actors.get(&bearer) else {
            return;
        };
        if !actor.alive {
            return;
        }
        // One effect of a skill per bearer: a recast replaces it.
        let replaced = actor
            .effects
            .iter()
            .find(|effect| effect.skill == def.id)
            .map(|effect| effect.id);
        // Soul Barbs: an enchantment or a hex arriving hurts, once per barbs that was already there.
        let barbs: Vec<(u32, u32, f64)> =
            if matches!(def.kind, SkillType::Enchantment | SkillType::Hex) {
                actor
                    .effects
                    .iter()
                    .filter(|effect| effect.mods.damage_on_enchant_or_hex > 0.0)
                    .map(|effect| {
                        (
                            effect.caster,
                            effect.skill,
                            effect.mods.damage_on_enchant_or_hex,
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
        if let Some(id) = replaced {
            self.remove_effect(bearer, id, false);
        }
        let id = self.next_effect;
        self.next_effect += 1;
        let active = Active {
            id,
            skill: def.id,
            rank,
            kind: def.kind,
            profession: def.profession,
            caster,
            expires: ms.map(|ms| now + ms),
            mods,
        };
        let speed_changes = active.mods.move_speed.is_some();
        if let Some(actor) = self.actors.get_mut(&bearer) {
            actor.effects.push(active);
        }
        match ms {
            Some(ms) => self.events.push(Event::EffectAdd {
                bearer,
                skill: def.id,
                rank: u32::from(rank),
                id,
                seconds: ms as f32 / 1000.0,
            }),
            None => self.events.push(Event::UpkeepAdd {
                bearer,
                caster,
                skill: def.id,
                rank: u32::from(rank),
                id,
            }),
        }
        self.publish_flags(bearer);
        if speed_changes {
            self.publish_speed(bearer);
        }
        for (source, skill, amount) in barbs {
            self.hurt(
                source,
                bearer,
                skill,
                DamageKind::Untyped,
                amount,
                0.0,
                false,
                rules,
            );
        }
    }

    /// End an effect. `expired` is an effect running out on its own, which is when Blood Renewal heals.
    fn remove_effect(&mut self, bearer: u32, id: u32, expired: bool) {
        let Some(actor) = self.actors.get_mut(&bearer) else {
            return;
        };
        let Some(index) = actor.effects.iter().position(|effect| effect.id == id) else {
            return;
        };
        let effect = actor.effects.remove(index);
        let speed = effect.mods.move_speed.is_some();
        if effect.expires.is_some() {
            self.events.push(Event::EffectRemove { bearer, id });
        } else {
            self.events.push(Event::UpkeepRemove { bearer, id });
        }
        if expired && effect.mods.heal_on_end > 0.0 {
            self.heal(effect.caster, bearer, effect.mods.heal_on_end);
        }
        self.publish_flags(bearer);
        if speed {
            self.publish_speed(bearer);
        }
    }

    fn effect_flags(&self, agent: u32) -> u32 {
        let Some(actor) = self.actors.get(&agent) else {
            return 0;
        };
        let now = self.now;
        let mut word = 0;
        for condition in Condition::ALL {
            if actor.has(condition, now) {
                word |= flags::CONDITIONED | condition.flag();
            }
        }
        for effect in &actor.effects {
            match effect.kind {
                SkillType::Enchantment => word |= flags::ENCHANTED,
                SkillType::Hex => {
                    word |= flags::HEXED;
                    if effect.mods.health_regen < 0 {
                        word |= flags::DEGEN_HEXED;
                    }
                }
                _ => {}
            }
        }
        if !actor.alive {
            word |= flags::DEAD;
        }
        word
    }

    fn publish_flags(&mut self, agent: u32) {
        let word = self.effect_flags(agent);
        if let Some(actor) = self.actors.get_mut(&agent) {
            if actor.shown_flags != word {
                actor.shown_flags = word;
                self.events.push(Event::Flags { agent, flags: word });
            }
        }
    }

    fn publish_speed(&mut self, agent: u32) {
        let Some(actor) = self.actors.get(&agent) else {
            return;
        };
        let speed = actor.product(|mods| mods.move_speed).clamp(0.01, 1.0);
        if (speed - actor.shown_speed).abs() > 1e-6 {
            if let Some(actor) = self.actors.get_mut(&agent) {
                actor.shown_speed = speed;
            }
            self.events.push(Event::Speed {
                agent,
                multiplier: speed as f32,
            });
        }
    }

    /// Health regeneration pips on an agent now: its effects, the effects it maintains as a caster that
    /// feed it (Life Siphon), and its conditions, capped.
    pub fn health_pips(&self, agent: u32, rules: &Rules) -> i32 {
        let Some(actor) = self.actors.get(&agent) else {
            return 0;
        };
        if !actor.alive {
            return 0;
        }
        let own: i32 = actor
            .effects
            .iter()
            .map(|effect| effect.mods.health_regen)
            .sum();
        let fed: i32 = self
            .actors
            .values()
            .flat_map(|other| other.effects.iter())
            .filter(|effect| effect.caster == agent)
            .map(|effect| effect.mods.caster_health_regen)
            .sum();
        let conditions: i32 = Condition::ALL
            .iter()
            .filter(|condition| actor.has(**condition, self.now))
            .map(|condition| {
                rules
                    .condition_degeneration
                    .get(condition)
                    .copied()
                    .unwrap_or(0)
            })
            .sum();
        (own + fed - conditions).clamp(-rules.regen_pip_cap, rules.regen_pip_cap)
    }

    /// Energy regeneration pips: the agent's own, less one for each enchantment it maintains.
    pub fn energy_pips(&self, agent: u32, rules: &Rules) -> i32 {
        let Some(actor) = self.actors.get(&agent) else {
            return 0;
        };
        let upkeep = self
            .actors
            .values()
            .flat_map(|other| other.effects.iter())
            .filter(|effect| effect.caster == agent && effect.expires.is_none())
            .count() as i32;
        (actor.spec.energy_pips - upkeep).clamp(-rules.regen_pip_cap, rules.regen_pip_cap)
    }

    // ---- time --------------------------------------------------------------------------------

    /// Let `elapsed` milliseconds pass, in steps no longer than 100 ms so a cast or a pulse lands near
    /// its own moment.
    pub fn tick(&mut self, elapsed: Ms, skills: &Skills, rules: &Rules) {
        let mut left = elapsed;
        while left > 0 {
            let step = left.min(100);
            left -= step;
            self.step(step, skills, rules);
        }
    }

    fn step(&mut self, step: Ms, skills: &Skills, rules: &Rules) {
        self.now += step;
        let now = self.now;

        // Missiles that arrive now. The live server closes every launch with `0x00A7` at launch time
        // plus its flight, hit or miss, and the hit's word follows in the same frame (Rurik
        // `schema/overrides.json` 167).
        let arrived: Vec<(u32, u32, u32)> = self
            .actors
            .values_mut()
            .filter_map(|actor| match actor.in_flight {
                Some((lands, counter, kind)) if lands <= now => {
                    actor.in_flight = None;
                    Some((actor.agent, counter, kind.wire_word()))
                }
                _ => None,
            })
            .collect();
        for (agent, counter, kind) in arrived {
            self.events.push(Event::ProjectileArrived {
                agent,
                counter,
                kind,
            });
        }

        // Casts that finish now.
        let due: Vec<u32> = self
            .actors
            .values()
            .filter(|actor| {
                actor
                    .cast
                    .as_ref()
                    .is_some_and(|cast| cast.completes <= now)
            })
            .map(|actor| actor.agent)
            .collect();
        for agent in due {
            self.complete(agent, skills, rules);
        }

        // Ordinary attacks: swings that land now, and new ones.
        self.swing(skills, rules);

        // Recharges that end now.
        let mut ready = Vec::new();
        for actor in self.actors.values_mut() {
            let done: Vec<Slot> = actor
                .recharge
                .iter()
                .filter(|(_, until)| **until <= now)
                .map(|(slot, _)| *slot)
                .collect();
            for slot in done {
                actor.recharge.remove(&slot);
                ready.push((actor.agent, slot));
            }
        }
        for (agent, (skill, copy)) in ready {
            self.events
                .push(Event::SkillRecharged { agent, skill, copy });
        }

        // Effects and conditions that run out now, and glyphs whose charges are spent.
        let mut expired = Vec::new();
        let mut ended_conditions = Vec::new();
        for actor in self.actors.values_mut() {
            for effect in &actor.effects {
                let spent = effect.mods.spell_energy_discount > 0.0 && effect.mods.charges == 0;
                if effect.expires.is_some_and(|expires| expires <= now) || spent {
                    expired.push((actor.agent, effect.id));
                }
            }
            let gone: Vec<(Condition, u32)> = actor
                .conditions
                .iter()
                .filter(|(_, (expires, _))| *expires <= now)
                .map(|(condition, (_, id))| (*condition, *id))
                .collect();
            for (condition, id) in gone {
                actor.conditions.remove(&condition);
                ended_conditions.push((actor.agent, id));
            }
        }
        for (agent, id) in expired {
            self.remove_effect(agent, id, true);
        }
        for (agent, id) in ended_conditions {
            self.events.push(Event::EffectRemove { bearer: agent, id });
            self.publish_flags(agent);
        }

        // Fields that pulse now.
        let mut pulses = Vec::new();
        for field in &mut self.fields {
            while field.next <= now && field.next <= field.ends {
                pulses.push(field.clone());
                field.next += field.every;
            }
        }
        self.fields.retain(|field| field.next <= field.ends);
        for field in pulses {
            if let Some(def) = skills.get(field.skill) {
                let context = Context {
                    caster: field.caster,
                    skill: field.skill,
                    rank: field.rank,
                    target: field.caster,
                    field: Some(field.position),
                    field_radius: field.radius,
                };
                for op in &field.pulse {
                    self.run(op, context, def, skills, rules);
                }
            }
        }

        // Chains that jump now.
        let mut jumps = Vec::new();
        for chain in &mut self.chains {
            if chain.next <= now && chain.left > 0 {
                jumps.push(chain.clone());
                chain.left -= 1;
                chain.next += chain.every;
            }
        }
        for chain in jumps {
            let Some(last) = self.actors.get(&chain.last).map(|actor| actor.position) else {
                continue;
            };
            let side = self.actors.get(&chain.caster).map(|actor| actor.spec.side);
            let next = self
                .actors
                .values()
                .filter(|actor| {
                    actor.alive
                        && side.is_some_and(|side| hostile(side, actor.spec.side))
                        && !chain.struck.contains(&actor.agent)
                        && distance(actor.position, last) <= chain.radius
                })
                .min_by(|a, b| {
                    distance(a.position, last)
                        .partial_cmp(&distance(b.position, last))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|actor| actor.agent);
            let Some(victim) = next else {
                if let Some(live) = self
                    .chains
                    .iter_mut()
                    .find(|live| live.caster == chain.caster && live.skill == chain.skill)
                {
                    live.left = 0;
                }
                continue;
            };
            if let Some(live) = self
                .chains
                .iter_mut()
                .find(|live| live.caster == chain.caster && live.skill == chain.skill)
            {
                live.last = victim;
                live.struck.push(victim);
            }
            if let Some(def) = skills.get(chain.skill) {
                let context = Context {
                    caster: chain.caster,
                    skill: chain.skill,
                    rank: chain.rank,
                    target: victim,
                    field: None,
                    field_radius: 0.0,
                };
                for op in &chain.hit {
                    self.run(op, context, def, skills, rules);
                }
            }
        }
        self.chains.retain(|chain| chain.left > 0);

        // Regeneration: integrate, publish a changed rate, and let degeneration kill.
        let agents: Vec<u32> = self.actors.keys().copied().collect();
        for agent in agents {
            let health_pips = self.health_pips(agent, rules);
            let energy_pips = self.energy_pips(agent, rules);
            let seconds = step as f64 / 1000.0;
            let Some(actor) = self.actors.get_mut(&agent) else {
                continue;
            };
            if !actor.alive {
                continue;
            }
            let max_health = f64::from(actor.spec.max_health);
            let max_energy = f64::from(actor.spec.max_energy);
            actor.health = (actor.health
                + f64::from(health_pips) * rules.health_pip_per_second * seconds)
                .min(max_health);
            actor.energy = (actor.energy
                + f64::from(energy_pips) * rules.energy_pip_per_second * seconds)
                .clamp(0.0, max_energy);
            let mut out = Vec::new();
            if health_pips != actor.shown_health_pips {
                actor.shown_health_pips = health_pips;
                out.push(Event::HealthRegen {
                    agent,
                    pips: health_pips,
                    max: actor.spec.max_health,
                });
            }
            if energy_pips != actor.shown_energy_pips && actor.spec.max_energy > 0 {
                actor.shown_energy_pips = energy_pips;
                out.push(Event::EnergyRegen {
                    agent,
                    pips: energy_pips,
                    max: actor.spec.max_energy,
                });
            }
            let dead = actor.health < 1.0 && health_pips < 0;
            if dead {
                actor.health = 0.0;
                out.push(Event::Health {
                    agent,
                    fraction: 0.0,
                });
            }
            self.events.extend(out);
            if dead {
                self.die(agent, skills);
            }
        }
    }

    fn roll(&mut self, below: u32) -> u32 {
        if below == 0 {
            return 0;
        }
        // xorshift64*.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let value = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((value >> 32) % u64::from(below)) as u32
    }
}

/// A shared empty catalogue, for the death paths that cancel a cast without needing its recharge.
fn skills_none() -> &'static Skills {
    static EMPTY: std::sync::OnceLock<Skills> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Skills::default)
}

/// Whether `a` fights `b`.
pub fn hostile(a: Side, b: Side) -> bool {
    matches!(
        (a, b),
        (Side::Player, Side::Foe) | (Side::Foe, Side::Player)
    )
}

/// Whether `a` helps `b`.
pub fn allied(a: Side, b: Side) -> bool {
    a == b && a != Side::Neutral
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// A shot's aim point and flight. The aim leads the target by its velocity over the flight, and the
/// flight is refined once against that led point (Rurik `toolkit/authsrv/authsrv.py:17300-17339`;
/// GWW "Projectile": the trajectory is calculated from the target's location and velocity at fire).
fn aim_point(
    origin: (f32, f32),
    target: (f32, f32),
    velocity: (f32, f32),
    speed: f32,
) -> ((f32, f32), Ms) {
    let first = distance(origin, target) / speed * 1000.0;
    let aim = (
        target.0 + velocity.0 * first / 1000.0,
        target.1 + velocity.1 * first / 1000.0,
    );
    let flight = (distance(origin, aim) / speed * 1000.0).round().max(0.0) as Ms;
    (aim, flight)
}

/// Whether a shot that reached `aim` connects with a target now at `target`: within the tolerance
/// ACROSS the line of fire (`origin` to `aim`) and within `along` of the aim, short or long. A shot
/// with no line of fire on record falls back to a disc of the across tolerance (Rurik
/// `toolkit/authsrv/authsrv.py:17340-17375`).
fn connects(
    origin: (f32, f32),
    aim: (f32, f32),
    target: (f32, f32),
    tolerance: f32,
    along: f32,
) -> bool {
    let dx = target.0 - aim.0;
    let dy = target.1 - aim.1;
    let fx = aim.0 - origin.0;
    let fy = aim.1 - origin.1;
    let span = (fx * fx + fy * fy).sqrt();
    if span <= f32::EPSILON {
        return (dx * dx + dy * dy).sqrt() <= tolerance;
    }
    let ux = fx / span;
    let uy = fy / span;
    (dx * uy - dy * ux).abs() <= tolerance && (dx * ux + dy * uy).abs() <= along
}
