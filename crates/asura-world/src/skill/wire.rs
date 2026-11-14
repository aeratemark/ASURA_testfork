//! What the engine did, as messages, and who hears them.
//!
//! A slot's lifecycle, adrenaline and energy are the caster's own business: only its connection hears
//! them (preforged keeps them on the owner's outbox, `gw_world_simulation.h` `owner_vitals`). Everything
//! another client draws - casts, bodies, health, effects, death - goes to the whole instance.

use crate::skill::engine::Event;
use crate::skill::model::Rules;
use crate::{game, Frame};
use asura_protocol::game::skill as wire;

/// Who a message is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum To {
    /// The connection that drives this agent.
    Owner(u32),
    /// Every client in the instance.
    All,
}

/// The agents one event's messages name. A client that was never sent a body cannot look it up, so
/// the instance sends an event only to the clients that know every one of these (`docs/49` 2.1).
pub fn subjects(event: &Event) -> Vec<u32> {
    match *event {
        Event::SkillActivate { agent, .. }
        | Event::SkillAck { agent, .. }
        | Event::SkillRecharge { agent, .. }
        | Event::SkillRecharged { agent, .. }
        | Event::SkillSpent { agent, .. }
        | Event::Adrenaline { agent, .. }
        | Event::AdrenalineSpent { agent, .. }
        | Event::Projectile { agent, .. }
        | Event::ProjectileArrived { agent, .. }
        | Event::AttackTiming { agent, .. }
        | Event::AttackFinished { attacker: agent }
        | Event::Instant { caster: agent, .. }
        | Event::CastFinish { caster: agent, .. }
        | Event::CastCancel { caster: agent, .. }
        | Event::CasterBody { agent, .. }
        | Event::Health { agent, .. }
        | Event::Energy { agent, .. }
        | Event::MaxHealth { agent, .. }
        | Event::MaxEnergy { agent, .. }
        | Event::HealthRegen { agent, .. }
        | Event::EnergyRegen { agent, .. }
        | Event::Flags { agent, .. }
        | Event::Knockdown { agent, .. }
        | Event::Speed { agent, .. }
        | Event::Death { agent, .. }
        | Event::Resurrected { agent, .. } => vec![agent],
        Event::AttackStart { attacker, target }
        | Event::AttackFailed { attacker, target }
        | Event::CastStart {
            caster: attacker,
            target,
            ..
        }
        | Event::TargetBody {
            target,
            caster: attacker,
            ..
        }
        | Event::Damage {
            target,
            source: attacker,
            ..
        }
        | Event::Heal {
            target,
            source: attacker,
            ..
        } => vec![attacker, target],
        Event::EffectAdd { bearer, .. } | Event::EffectRemove { bearer, .. } => vec![bearer],
        Event::UpkeepAdd { bearer, caster, .. } => vec![bearer, caster],
        Event::UpkeepRemove { bearer, .. } => vec![bearer],
    }
}

/// One event's messages.
pub fn frames(event: &Event, rules: &Rules) -> (To, Vec<Frame>) {
    let int = game::agent_property_update_int;
    let float = game::agent_property_update_float;
    match *event {
        Event::SkillActivate { agent, skill, copy } => (
            To::Owner(agent),
            vec![wire::skill_activate(agent, skill, copy)],
        ),
        Event::SkillAck { agent, skill, copy } => {
            (To::Owner(agent), vec![wire::skill_ack(agent, skill, copy)])
        }
        Event::SkillRecharge {
            agent,
            skill,
            copy,
            seconds,
        } => (
            To::Owner(agent),
            vec![wire::skill_recharge(agent, skill, copy, seconds)],
        ),
        Event::SkillRecharged { agent, skill, copy } => (
            To::Owner(agent),
            vec![wire::skill_recharged(agent, skill, copy)],
        ),
        Event::SkillSpent { agent, skill, copy } => (
            To::Owner(agent),
            vec![wire::skill_used_once(agent, skill, copy)],
        ),
        Event::Adrenaline {
            agent,
            skill,
            copy,
            units,
        } => (
            To::Owner(agent),
            vec![wire::skill_adrenaline(agent, skill, copy, units)],
        ),
        Event::AdrenalineSpent { agent, skill, copy } => (
            To::Owner(agent),
            vec![wire::skill_adrenaline_spent(agent, skill, copy)],
        ),
        // Seen once per swing on the live server, the attacker first (`packets-30352-81363781.jsonl`
        // lines 8508, 8562, 8866: Bandit Firestarter 52 on the player 31, each before a hit).
        Event::AttackStart { attacker, target } => (
            To::All,
            vec![wire::agent_property_update_int_target(
                wire::PROPERTY_ATTACK,
                attacker,
                target,
                0,
            )],
        ),
        Event::Projectile {
            agent,
            to,
            seconds,
            model,
            counter,
            flag,
        } => (
            To::All,
            vec![wire::agent_projectile_launched(
                agent, to, seconds, model, counter, flag,
            )],
        ),
        Event::ProjectileArrived {
            agent,
            counter,
            kind,
        } => (
            To::All,
            vec![wire::agent_projectile_arrived(agent, counter, kind)],
        ),
        Event::AttackFailed { attacker, target } => (
            To::All,
            vec![wire::agent_property_update_int_target(
                wire::PROPERTY_ATTACK_FAIL,
                target,
                attacker,
                wire::ATTACK_FAIL_DODGE,
            )],
        ),
        Event::AttackTiming {
            agent,
            interval,
            modifier,
        } => (
            To::All,
            vec![wire::agent_attack_timing(agent, interval, modifier)],
        ),
        Event::AttackFinished { attacker } => (
            To::All,
            vec![int(wire::PROPERTY_MELEE_FINISHED, attacker, 0)],
        ),
        Event::CastStart {
            caster,
            target,
            skill,
            attack,
        } => (
            To::All,
            vec![wire::agent_property_update_int_target(
                if attack {
                    wire::PROPERTY_ATTACK_SKILL
                } else {
                    wire::PROPERTY_CAST
                },
                caster,
                target,
                skill,
            )],
        ),
        Event::Instant { caster, skill } => {
            (To::All, vec![int(wire::PROPERTY_INSTANT, caster, skill)])
        }
        Event::CastFinish { caster, attack } => (
            To::All,
            vec![int(
                if attack {
                    wire::PROPERTY_ATTACK_FINISHED
                } else {
                    wire::PROPERTY_CAST_FINISHED
                },
                caster,
                0,
            )],
        ),
        Event::CastCancel { caster, attack } => (
            To::All,
            vec![int(
                if attack {
                    wire::PROPERTY_ATTACK_STOPPED
                } else {
                    wire::PROPERTY_CAST_STOPPED
                },
                caster,
                0,
            )],
        ),
        Event::CasterBody { agent, resource } => {
            (To::All, vec![int(wire::PROPERTY_BODY, agent, resource)])
        }
        Event::TargetBody {
            target,
            caster,
            resource,
        } => (
            To::All,
            vec![wire::agent_property_update_int_target(
                wire::PROPERTY_TARGET_BODY,
                target,
                caster,
                resource,
            )],
        ),
        Event::Damage {
            target,
            source,
            skill,
            amount,
            max,
            armor_ignoring,
            health,
        } => {
            let max = f64::from(max.max(1));
            (
                To::All,
                vec![
                    // The skill first, on the target: the client keeps it for the damage line that
                    // follows. The live server names the struck agent here, not the source
                    // (`packets-30352-81363781.jsonl` lines 7318-7319: `[10, 31, 194]` then
                    // `[16, 31, 50, ...]`, Flare from 50 on 31).
                    int(wire::PROPERTY_DAMAGE_SKILL, target, skill),
                    wire::agent_property_update_float_target(
                        if armor_ignoring {
                            wire::PROPERTY_HEALTH_CHANGE
                        } else {
                            wire::PROPERTY_DAMAGE
                        },
                        target,
                        source,
                        (-amount / max) as f32,
                    ),
                    float(
                        wire::PROPERTY_HEALTH_FRACTION,
                        target,
                        (health / max) as f32,
                    ),
                ],
            )
        }
        Event::Heal {
            target,
            source,
            amount,
            max,
            health,
        } => {
            let max = f64::from(max.max(1));
            (
                To::All,
                vec![
                    wire::agent_property_update_float_target(
                        wire::PROPERTY_HEALTH_CHANGE,
                        target,
                        source,
                        (amount / max) as f32,
                    ),
                    float(
                        wire::PROPERTY_HEALTH_FRACTION,
                        target,
                        (health / max) as f32,
                    ),
                ],
            )
        }
        Event::Health { agent, fraction } => (
            To::All,
            vec![float(wire::PROPERTY_HEALTH_FRACTION, agent, fraction)],
        ),
        Event::Energy { agent, fraction } => (
            To::Owner(agent),
            vec![float(wire::PROPERTY_ENERGY_FRACTION, agent, fraction)],
        ),
        Event::MaxHealth { agent, max } => {
            (To::All, vec![int(wire::PROPERTY_MAX_HEALTH, agent, max)])
        }
        Event::MaxEnergy { agent, max } => (
            To::Owner(agent),
            vec![int(wire::PROPERTY_MAX_ENERGY, agent, max)],
        ),
        Event::HealthRegen { agent, pips, max } => (
            To::All,
            vec![float(
                wire::PROPERTY_HEALTH_REGEN,
                agent,
                (f64::from(pips) * rules.health_pip_per_second / f64::from(max.max(1))) as f32,
            )],
        ),
        Event::EnergyRegen { agent, pips, max } => (
            To::Owner(agent),
            vec![float(
                wire::PROPERTY_ENERGY_REGEN,
                agent,
                (f64::from(pips) * rules.energy_pip_per_second / f64::from(max.max(1))) as f32,
            )],
        ),
        Event::EffectAdd {
            bearer,
            skill,
            rank,
            id,
            seconds,
        } => (
            To::All,
            vec![wire::effect_applied(bearer, skill, rank, id, seconds)],
        ),
        Event::EffectRemove { bearer, id } => (To::All, vec![wire::effect_removed(bearer, id)]),
        Event::UpkeepAdd {
            bearer,
            caster,
            skill,
            rank,
            id,
        } => (
            To::All,
            vec![wire::effect_upkeep_applied(bearer, caster, skill, rank, id)],
        ),
        Event::UpkeepRemove { bearer, id } => {
            (To::All, vec![wire::effect_upkeep_removed(bearer, id)])
        }
        Event::Flags { agent, flags } => (To::All, vec![game::agent_update_effects(agent, flags)]),
        Event::Knockdown { agent, seconds } => (
            To::All,
            vec![float(wire::PROPERTY_KNOCKDOWN, agent, seconds)],
        ),
        Event::Speed { agent, multiplier } => (
            To::All,
            vec![game::agent_update_speed(agent, multiplier, 0)],
        ),
        // A player's death is the live pair (`multiplay-live-20261106-200638.jsonl` seq 52-55); a
        // creature's is preforged's effect word with the dead bit (`gw_combat.h` `health_packets_2022`).
        Event::Death {
            agent,
            player: true,
        } => (
            To::All,
            vec![
                game::agent_player_die(agent),
                game::agent_update_flags(agent, 4),
            ],
        ),
        Event::Death {
            agent,
            player: false,
        } => (
            To::All,
            vec![game::agent_update_effects(
                agent,
                crate::skill::engine::flags::DEAD,
            )],
        ),
        // Not measured: the flags word back to 0. The vitals and the effect word follow as their own
        // events.
        Event::Resurrected { agent, .. } => (To::All, vec![game::agent_update_flags(agent, 0)]),
    }
}
