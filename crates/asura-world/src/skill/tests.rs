//! The engine, driven a millisecond at a time with skills built in the test: no pack, no socket.

use super::engine::*;
use super::model::*;

const ME: u32 = 10;
const FRIEND: u32 = 11;
const FOE: u32 = 20;
const FOE2: u32 = 21;
const TOWNSFOLK: u32 = 30;

fn skill(id: u32, kind: SkillType, target: TargetRule, effects: Vec<Op>) -> SkillDef {
    SkillDef {
        id,
        name: format!("skill {id}"),
        kind,
        profession: 3,
        attribute: 13,
        elite: false,
        pve_only: false,
        energy: 5,
        adrenaline: 0,
        health_sacrifice: 0,
        activation_ms: 1000,
        aftercast_ms: 750,
        recharge_ms: 2000,
        target,
        weapon: None,
        scale: (20, 70),
        bonus: (0, 0),
        duration: (8, 8),
        aoe: 156.0,
        caster_body: None,
        target_body: Some(500),
        while_casting_armor: 0,
        touch: false,
        effects,
    }
}

fn player(bar: [Slot; 8]) -> Spec {
    let mut ranks = [0u8; ATTRIBUTES];
    ranks[13] = 15;
    Spec {
        side: Side::Player,
        level: 20,
        primary: 3,
        secondary: 0,
        ranks,
        bar,
        max_health: 480,
        max_energy: 25,
        energy_pips: 4,
        armor: 60,
        weapon: None,
    }
}

fn bar(skills: &[u32]) -> [Slot; 8] {
    let mut bar = [(0, 0); 8];
    for (slot, id) in skills.iter().enumerate() {
        bar[slot] = (*id, 0);
    }
    bar
}

fn bow() -> Weapon {
    Weapon {
        item_type: 5,
        attribute: 0,
        period_ms: 1750,
        kind: DamageKind::Piercing,
        ranged: true,
        min: 10,
        max: 10,
        projectile: Some((143, 1)),
    }
}

fn world(defs: Vec<SkillDef>) -> (Engine, Skills, Rules) {
    let skills = Skills {
        by_id: defs.into_iter().map(|def| (def.id, def)).collect(),
    };
    let ids: Vec<u32> = skills.by_id.keys().copied().collect();
    let mut engine = Engine::new(7);
    engine.join(ME, player(bar(&ids)), (0.0, 0.0));
    engine.join(FRIEND, player(bar(&[])), (100.0, 0.0));
    engine.join(FOE, Spec::creature(Side::Foe, 20, 480, 60), (300.0, 0.0));
    engine.join(FOE2, Spec::creature(Side::Foe, 20, 480, 60), (380.0, 0.0));
    engine.join(
        TOWNSFOLK,
        Spec::creature(Side::Neutral, 5, 100, 0),
        (50.0, 0.0),
    );
    (engine, skills, Rules::default())
}

fn heal_skill() -> SkillDef {
    skill(
        281,
        SkillType::Spell,
        TargetRule::Ally,
        vec![Op::Heal {
            to: Area::Target,
            amount: Amount::Scale,
            divine_favor: true,
        }],
    )
}

fn damage_skill(id: u32, kind: DamageKind) -> SkillDef {
    skill(
        id,
        SkillType::Spell,
        TargetRule::Foe,
        vec![Op::Damage {
            to: Area::Target,
            kind,
            amount: Amount::Scale,
        }],
    )
}

#[test]
fn a_cast_publishes_start_then_finish_ack_and_recharge_in_preforged_order() {
    let (mut engine, skills, rules) = world(vec![heal_skill()]);
    engine
        .use_skill(ME, 281, 0, FRIEND, &skills, &rules)
        .expect("admitted");
    let start = engine.drain();
    assert_eq!(
        start,
        vec![
            Event::Energy {
                agent: ME,
                fraction: 20.0 / 25.0
            },
            Event::SkillActivate {
                agent: ME,
                skill: 281,
                copy: 0
            },
            Event::CastStart {
                caster: ME,
                target: FRIEND,
                skill: 281,
                attack: false
            },
        ]
    );
    engine.tick(999, &skills, &rules);
    assert!(!engine
        .drain()
        .iter()
        .any(|e| matches!(e, Event::CastFinish { .. })));
    engine.tick(1, &skills, &rules);
    let done = engine.drain();
    let finish = done
        .iter()
        .position(|e| matches!(e, Event::CastFinish { attack: false, .. }))
        .expect("finish");
    let body = done
        .iter()
        .position(|e| matches!(e, Event::TargetBody { resource: 500, .. }))
        .expect("body");
    let heal = done
        .iter()
        .position(|e| matches!(e, Event::Heal { .. }))
        .expect("heal");
    let ack = done
        .iter()
        .position(|e| matches!(e, Event::SkillAck { .. }))
        .expect("ack");
    let recharge = done
        .iter()
        .position(|e| matches!(e, Event::SkillRecharge { seconds: 2, .. }))
        .expect("recharge");
    assert!(
        finish < body && body < heal && heal < ack && ack < recharge,
        "{done:?}"
    );
    engine.tick(2000, &skills, &rules);
    assert!(engine.drain().contains(&Event::SkillRecharged {
        agent: ME,
        skill: 281,
        copy: 0
    }));
}

#[test]
fn a_heal_scales_with_rank_and_divine_favor() {
    let (mut engine, skills, rules) = world(vec![heal_skill()]);
    // Healing rank 15 gives 70; Divine Favor 0 adds nothing.
    engine
        .use_skill(ME, 281, 0, FRIEND, &skills, &rules)
        .unwrap();
    engine.tick(1000, &skills, &rules);
    let healed = engine
        .drain()
        .into_iter()
        .find_map(|e| match e {
            Event::Heal {
                amount,
                target: FRIEND,
                ..
            } => Some(amount),
            _ => None,
        })
        .unwrap();
    assert_eq!(healed, 70.0);

    let mut ranks = [0u8; ATTRIBUTES];
    ranks[13] = 15;
    ranks[16] = 10;
    engine.set_ranks(ME, ranks);
    engine.tick(5000, &skills, &rules);
    engine.drain();
    engine
        .use_skill(ME, 281, 0, FRIEND, &skills, &rules)
        .unwrap();
    engine.tick(1000, &skills, &rules);
    let healed = engine
        .drain()
        .into_iter()
        .find_map(|e| match e {
            Event::Heal {
                amount,
                target: FRIEND,
                ..
            } => Some(amount),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        healed,
        70.0 + 32.0,
        "3.2 per rank of Divine Favor, for a primary Monk"
    );
}

#[test]
fn refusals_are_answered_with_the_acknowledgement_and_spend_nothing() {
    let (mut engine, skills, rules) = world(vec![damage_skill(194, DamageKind::Fire)]);
    assert_eq!(
        engine.use_skill(ME, 194, 0, FRIEND, &skills, &rules),
        Err(Refusal::BadTarget),
        "a foe spell on an ally"
    );
    assert_eq!(
        engine.use_skill(ME, 194, 0, TOWNSFOLK, &skills, &rules),
        Err(Refusal::BadTarget),
        "a townsperson is nobody's foe"
    );
    assert_eq!(
        engine.drain(),
        vec![
            Event::SkillAck {
                agent: ME,
                skill: 194,
                copy: 0
            },
            Event::SkillAck {
                agent: ME,
                skill: 194,
                copy: 0
            }
        ]
    );
    assert_eq!(engine.actor(ME).unwrap().energy, 25.0);
    assert_eq!(
        engine.use_skill(ME, 5, 0, FOE, &skills, &rules),
        Err(Refusal::NotOnBar)
    );
    engine.join(99, Spec::creature(Side::Foe, 1, 10, 0), (5000.0, 0.0));
    assert_eq!(
        engine.use_skill(ME, 194, 0, 99, &skills, &rules),
        Err(Refusal::OutOfRange)
    );
}

#[test]
fn damage_follows_the_armor_formula_and_untyped_damage_ignores_armor() {
    let (mut engine, skills, rules) = world(vec![
        damage_skill(194, DamageKind::Fire),
        damage_skill(27, DamageKind::Untyped),
    ]);
    engine.use_skill(ME, 194, 0, FOE, &skills, &rules).unwrap();
    engine.tick(1000, &skills, &rules);
    let fire = engine
        .drain()
        .into_iter()
        .find_map(|e| match e {
            Event::Damage {
                amount,
                armor_ignoring: false,
                ..
            } => Some(amount),
            _ => None,
        })
        .unwrap();
    // Level 20: strike 60 against armor 60 is the base.
    assert!((fire - 70.0).abs() < 1e-9, "{fire}");
    engine.tick(2000, &skills, &rules);
    engine.drain();
    engine.use_skill(ME, 27, 0, FOE, &skills, &rules).unwrap();
    engine.tick(1000, &skills, &rules);
    assert!(engine.drain().iter().any(
        |e| matches!(e, Event::Damage { armor_ignoring: true, amount, .. } if *amount == 70.0)
    ));
}

#[test]
fn a_condition_is_an_effect_with_its_own_skill_id_and_degenerates() {
    let mut blind = skill(
        220,
        SkillType::Spell,
        TargetRule::Foe,
        vec![Op::Condition {
            to: Area::Target,
            condition: Condition::Bleeding,
            duration: Amount::Duration,
            guard: None,
        }],
    );
    blind.target_body = None;
    let (mut engine, skills, rules) = world(vec![blind]);
    engine.use_skill(ME, 220, 0, FOE, &skills, &rules).unwrap();
    engine.tick(1000, &skills, &rules);
    let events = engine.drain();
    assert!(events.iter().any(|e| matches!(e, Event::EffectAdd { bearer: FOE, skill: 478, seconds, .. } if *seconds == 8.0)));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Flags { agent: FOE, flags } if *flags == 0x03)));
    assert!(events.contains(&Event::HealthRegen {
        agent: FOE,
        pips: -3,
        max: 480
    }));
    engine.tick(1000, &skills, &rules);
    let left = engine.actor(FOE).unwrap().health;
    assert!(left < 480.0 && left > 470.0, "{left}");
    engine.tick(8000, &skills, &rules);
    let events = engine.drain();
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::EffectRemove { bearer: FOE, .. })));
    assert!(events.contains(&Event::Flags {
        agent: FOE,
        flags: 0
    }));
}

#[test]
fn backfire_hurts_its_bearer_when_it_casts_a_spell() {
    let backfire = skill(
        28,
        SkillType::Hex,
        TargetRule::Foe,
        vec![Op::Effect {
            to: Area::Target,
            duration: Some(Amount::Duration),
            mods: Mods {
                damage_on_spell: Some(Amount::Fixed(50.0)),
                ..Mods::default()
            },
        }],
    );
    let mut flare = damage_skill(194, DamageKind::Fire);
    flare.energy = 0;
    let (mut engine, skills, rules) = world(vec![backfire, flare]);
    // A foe that casts: FOE again, with a bar.
    let mut caster = Spec::creature(Side::Foe, 20, 480, 60);
    caster.bar = bar(&[194]);
    engine.leave(FOE);
    engine.join(FOE, caster, (300.0, 0.0));
    engine.use_skill(ME, 28, 0, FOE, &skills, &rules).unwrap();
    engine.tick(1000, &skills, &rules);
    let events = engine.drain();
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Flags { agent: FOE, flags } if *flags == flags::HEXED)));
    engine.use_skill(FOE, 194, 0, ME, &skills, &rules).unwrap();
    engine.tick(1000, &skills, &rules);
    let events = engine.drain();
    let backlash = events
        .iter()
        .position(|e| matches!(e, Event::Damage { target: FOE, source: ME, skill: 28, amount, .. } if *amount == 50.0))
        .expect("the hexed caster takes 50");
    let spell = events
        .iter()
        .position(|e| {
            matches!(
                e,
                Event::Damage {
                    target: ME,
                    skill: 194,
                    ..
                }
            )
        })
        .expect("its spell still lands");
    assert!(
        backlash < spell,
        "Backfire comes before what the spell does"
    );
}

#[test]
fn reversal_of_fortune_turns_the_next_damage_into_healing_up_to_its_limit() {
    let reversal = skill(
        307,
        SkillType::Enchantment,
        TargetRule::Ally,
        vec![Op::Effect {
            to: Area::Target,
            duration: Some(Amount::Duration),
            mods: Mods {
                heal_instead_of_damage: Some(Amount::Fixed(30.0)),
                ..Mods::default()
            },
        }],
    );
    let mut foe_spell = damage_skill(194, DamageKind::Untyped);
    foe_spell.energy = 0;
    foe_spell.effects = vec![Op::Damage {
        to: Area::Target,
        kind: DamageKind::Untyped,
        amount: Amount::Fixed(70.0),
    }];
    let (mut engine, skills, rules) = world(vec![reversal, foe_spell]);
    engine
        .use_skill(ME, 307, 0, FRIEND, &skills, &rules)
        .unwrap();
    engine.tick(1000, &skills, &rules);
    let events = engine.drain();
    assert!(events.iter().any(|e| matches!(
        e,
        Event::EffectAdd {
            bearer: FRIEND,
            skill: 307,
            ..
        }
    )));
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Flags { agent: FRIEND, flags } if *flags == flags::ENCHANTED)));
    let mut caster = Spec::creature(Side::Foe, 20, 480, 60);
    caster.bar = bar(&[194]);
    engine.join(50, caster, (200.0, 0.0));
    engine
        .use_skill(50, 194, 0, FRIEND, &skills, &rules)
        .unwrap();
    engine.tick(1000, &skills, &rules);
    let events = engine.drain();
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::EffectRemove { bearer: FRIEND, .. })));
    let damage = events
        .iter()
        .find_map(|e| match e {
            Event::Damage {
                target: FRIEND,
                amount,
                ..
            } => Some(*amount),
            _ => None,
        })
        .unwrap();
    assert_eq!(damage, 40.0, "70 damage, 30 of it turned into healing");
}

#[test]
fn an_attack_needs_its_weapon_and_hits_with_weapon_damage_plus_the_bonus() {
    let mut gash = skill(
        384,
        SkillType::Attack,
        TargetRule::Foe,
        vec![Op::Attack {
            to: Area::Target,
            bonus: Amount::Fixed(20.0),
            strikes: 1,
            multiplier: 1.0,
            on_hit: vec![Op::Condition {
                to: Area::Target,
                condition: Condition::DeepWound,
                duration: Amount::Fixed(10.0),
                guard: Some(Guard::Bleeding),
            }],
        }],
    );
    gash.weapon = Some(27);
    gash.energy = 0;
    gash.activation_ms = 0;
    gash.aftercast_ms = 0;
    gash.recharge_ms = 0;
    gash.target_body = None;
    let (mut engine, skills, rules) = world(vec![gash]);
    assert_eq!(
        engine.use_skill(ME, 384, 0, FOE, &skills, &rules),
        Err(Refusal::NeedsWeapon)
    );
    engine.set_equipment(
        ME,
        Some(Weapon {
            item_type: 27,
            attribute: 20,
            period_ms: 1330,
            kind: DamageKind::Slashing,
            ranged: false,
            min: 10,
            max: 10,
            projectile: None,
        }),
        60,
    );
    let mut ranks = [0u8; ATTRIBUTES];
    ranks[20] = 12;
    engine.set_ranks(ME, ranks);
    engine.set_position(FOE, (100.0, 0.0));
    engine.drain();
    engine.use_skill(ME, 384, 0, FOE, &skills, &rules).unwrap();
    let start = engine.drain();
    assert!(start.contains(&Event::CastStart {
        caster: ME,
        target: FOE,
        skill: 384,
        attack: true
    }));
    assert!(start
        .iter()
        .any(|e| matches!(e, Event::AttackTiming { .. })));
    engine.tick(1400, &skills, &rules);
    let done = engine.drain();
    assert!(done.contains(&Event::CastFinish {
        caster: ME,
        attack: true
    }));
    let damage = done
        .iter()
        .find_map(|e| match e {
            Event::Damage {
                target: FOE,
                amount,
                ..
            } => Some(*amount),
            _ => None,
        })
        .unwrap();
    assert!(
        (damage - 30.0).abs() < 1e-9,
        "rank 12 sword against armor 60: 10 + 20"
    );
    assert!(
        !done
            .iter()
            .any(|e| matches!(e, Event::EffectAdd { bearer: FOE, .. })),
        "no Deep Wound unless the foe bleeds"
    );
}

#[test]
fn a_ranged_swing_closes_its_shot_with_the_handle_and_damage_type() {
    let (mut engine, skills, rules) = world(Vec::new());
    engine.set_equipment(ME, Some(bow()), 60);
    engine.set_position(FOE, (900.0, 0.0));
    engine.drain();
    engine.attack(ME, FOE).expect("a bowman against a foe");
    // The swing opens at the first step, its windup is 775 ms, then 900 units at 1,600 u/s.
    engine.tick(900, &skills, &rules);
    let launched = engine.drain();
    let counter = launched
        .iter()
        .find_map(|event| match event {
            Event::Projectile {
                agent: ME,
                counter,
                model: 143,
                ..
            } => Some(*counter),
            _ => None,
        })
        .expect("the shot leaves at the windup");
    engine.tick(600, &skills, &rules);
    let landed = engine.drain();
    let arrival = landed
        .iter()
        .position(|event| {
            matches!(
                event,
                Event::ProjectileArrived {
                    agent: ME,
                    counter: c,
                    kind: 1
                } if *c == counter
            )
        })
        .expect("the shot is closed with its handle and damage type");
    let hit = landed
        .iter()
        .position(|event| matches!(event, Event::AttackFinished { attacker: ME }))
        .expect("the swing lands");
    assert!(arrival < hit, "0x00A7 precedes the hit's word: {landed:?}");
}

#[test]
fn a_shot_from_close_lands_sooner_than_one_from_far() {
    let land_ms = |distance: f32| -> u32 {
        let (mut engine, skills, rules) = world(Vec::new());
        engine.set_equipment(ME, Some(bow()), 60);
        engine.set_position(FOE, (distance, 0.0));
        engine.drain();
        engine.attack(ME, FOE).expect("a bowman against a foe");
        for elapsed in (10..=3_000).step_by(10) {
            engine.tick(10, &skills, &rules);
            if engine
                .drain()
                .iter()
                .any(|event| matches!(event, Event::AttackFinished { attacker: ME }))
            {
                return elapsed;
            }
        }
        panic!("no hit at {distance} units");
    };
    let close = land_ms(100.0);
    let far = land_ms(1_000.0);
    assert!(
        close + 400 < far,
        "a shot at 100 units lands sooner than one at 1,000: {close} ms against {far} ms"
    );
}

#[test]
fn a_shot_at_a_target_that_steps_aside_is_dodged() {
    let (mut engine, skills, rules) = world(Vec::new());
    engine.set_equipment(ME, Some(bow()), 60);
    engine.set_position(FOE, (900.0, 0.0));
    engine.drain();
    engine.attack(ME, FOE).expect("a bowman against a foe");
    engine.tick(900, &skills, &rules);
    assert!(engine
        .drain()
        .iter()
        .any(|event| matches!(event, Event::Projectile { agent: ME, .. })));
    // 300 units across the line of fire: outside the 54 u band.
    engine.set_position(FOE, (900.0, 300.0));
    engine.tick(600, &skills, &rules);
    let landed = engine.drain();
    assert!(
        landed
            .iter()
            .any(|event| matches!(event, Event::AttackFailed { attacker: ME, target: FOE })),
        "the shot is dodged: {landed:?}"
    );
    assert!(
        !landed
            .iter()
            .any(|event| matches!(event, Event::Damage { target: FOE, .. })),
        "a dodge deals no damage"
    );
}

#[test]
fn a_shot_leads_a_target_that_keeps_moving() {
    let (mut engine, skills, rules) = world(Vec::new());
    engine.set_equipment(ME, Some(bow()), 60);
    engine.set_position(FOE, (600.0, 0.0));
    engine.tick(100, &skills, &rules);
    engine.set_position(FOE, (600.0, 100.0));
    engine.drain();
    engine.attack(ME, FOE).expect("a bowman against a foe");
    let mut y = 100.0;
    let mut hit = false;
    for _ in 0..300 {
        // 200 u/s: below a player's 288 u/s run, so the shot's one refinement converges.
        y += 2.0;
        engine.set_position(FOE, (600.0, y));
        engine.tick(10, &skills, &rules);
        if engine
            .drain()
            .iter()
            .any(|event| matches!(event, Event::AttackFinished { attacker: ME }))
        {
            hit = true;
            break;
        }
    }
    assert!(hit, "the aim leads a target moving at 200 u/s");
}

#[test]
fn a_field_pulses_on_the_foes_inside_it_until_it_ends() {
    let storm = skill(
        197,
        SkillType::Spell,
        TargetRule::Foe,
        vec![Op::Field {
            at_target: true,
            duration: Amount::Fixed(3.0),
            every_ms: 1000,
            radius: 156.0,
            pulse: vec![Op::Damage {
                to: Area::FoesInField,
                kind: DamageKind::Untyped,
                amount: Amount::Fixed(10.0),
            }],
        }],
    );
    let (mut engine, skills, rules) = world(vec![storm]);
    engine.use_skill(ME, 197, 0, FOE, &skills, &rules).unwrap();
    engine.tick(1000, &skills, &rules);
    engine.drain();
    engine.tick(5000, &skills, &rules);
    let hits: Vec<u32> = engine
        .drain()
        .into_iter()
        .filter_map(|e| match e {
            Event::Damage { target, .. } => Some(target),
            _ => None,
        })
        .collect();
    assert_eq!(hits.iter().filter(|t| **t == FOE).count(), 3, "{hits:?}");
    assert_eq!(
        hits.iter().filter(|t| **t == FOE2).count(),
        3,
        "80 units away, inside 156"
    );
}

#[test]
fn death_ends_effects_and_resurrection_brings_back_a_dead_ally() {
    let res = SkillDef {
        target_body: None,
        activation_ms: 3000,
        recharge_ms: 0,
        energy: 0,
        ..skill(
            2,
            SkillType::Signet,
            TargetRule::DeadAlly,
            vec![Op::Resurrect {
                health_percent: Amount::Fixed(100.0),
                energy_percent: Amount::Fixed(25.0),
                once_per_instance: true,
            }],
        )
    };
    let (mut engine, skills, rules) = world(vec![res]);
    assert_eq!(
        engine.use_skill(ME, 2, 0, FRIEND, &skills, &rules),
        Err(Refusal::BadTarget),
        "a living ally"
    );
    engine.kill(FRIEND, &skills);
    assert!(engine.drain().contains(&Event::Death {
        agent: FRIEND,
        player: true
    }));
    engine.use_skill(ME, 2, 0, FRIEND, &skills, &rules).unwrap();
    engine.tick(3000, &skills, &rules);
    let events = engine.drain();
    assert!(events.contains(&Event::Resurrected {
        agent: FRIEND,
        player: true
    }));
    assert!(events.contains(&Event::SkillSpent {
        agent: ME,
        skill: 2,
        copy: 0
    }));
    assert_eq!(engine.actor(FRIEND).unwrap().health, 480.0);
    engine.tick(5000, &skills, &rules);
    engine.kill(FRIEND, &skills);
    engine.drain();
    assert_eq!(
        engine.use_skill(ME, 2, 0, FRIEND, &skills, &rules),
        Err(Refusal::UsedThisInstance)
    );
}

#[test]
fn a_maintained_enchantment_costs_its_caster_a_pip_and_ends_with_it() {
    let retribution = skill(
        248,
        SkillType::Enchantment,
        TargetRule::Ally,
        vec![Op::Effect {
            to: Area::Target,
            duration: None,
            mods: Mods {
                attacker_damage: Some(Amount::Range(5, 20)),
                ..Mods::default()
            },
        }],
    );
    let (mut engine, skills, rules) = world(vec![retribution]);
    engine
        .use_skill(ME, 248, 0, FRIEND, &skills, &rules)
        .unwrap();
    engine.tick(1100, &skills, &rules);
    let events = engine.drain();
    assert!(events.iter().any(|e| matches!(
        e,
        Event::UpkeepAdd {
            bearer: FRIEND,
            caster: ME,
            skill: 248,
            ..
        }
    )));
    assert!(events.contains(&Event::EnergyRegen {
        agent: ME,
        pips: 3,
        max: 25
    }));
    engine.leave(ME);
    assert!(engine
        .drain()
        .iter()
        .any(|e| matches!(e, Event::UpkeepRemove { bearer: FRIEND, .. })));
}

#[test]
fn interpolation_matches_the_client_rule() {
    assert_eq!(interpolate(20, 70, 0), 20);
    assert_eq!(interpolate(20, 70, 15), 70);
    assert_eq!(interpolate(20, 70, 12), 60);
    assert_eq!(interpolate(3, 18, 7), 10);
}

fn axe() -> Weapon {
    Weapon {
        item_type: 2,
        attribute: 18,
        period_ms: 1330,
        kind: DamageKind::Slashing,
        ranged: false,
        min: 6,
        max: 6,
        projectile: None,
    }
}

#[test]
fn an_ordinary_attack_swings_lands_and_repeats_while_in_reach() {
    let (mut engine, skills, rules) = world(vec![]);
    let mut grawl = Spec::creature(Side::Foe, 2, 120, 26);
    grawl.weapon = Some(axe());
    engine.join(40, grawl, (100.0, 50.0));
    engine.drain();
    engine.attack(40, ME).expect("a foe in reach");
    engine.tick(100, &skills, &rules);
    let started = engine.drain();
    assert!(started.contains(&Event::AttackStart {
        attacker: 40,
        target: ME
    }));
    assert!(matches!(started[0], Event::AttackTiming { agent: 40, .. }));
    let before = engine.actor(ME).unwrap().health;
    engine.tick(1330, &skills, &rules);
    let landed = engine.drain();
    assert!(engine.actor(ME).unwrap().health < before, "{landed:?}");
    let damage_at = landed
        .iter()
        .position(|event| matches!(event, Event::Damage { target: ME, .. }))
        .expect("the hit");
    let finished_at = landed
        .iter()
        .position(|event| matches!(event, Event::AttackFinished { attacker: 40 }))
        .expect("the swing's finish");
    assert!(
        finished_at < damage_at,
        "retail opens the hit batch with the melee finish, then the word: {landed:?}"
    );
    assert!(
        landed
            .iter()
            .filter(|event| matches!(event, Event::AttackStart { .. }))
            .count()
            >= 1,
        "the next swing starts: {landed:?}"
    );
}

#[test]
fn an_ordinary_swing_lands_at_its_windup_not_at_the_end_of_its_interval() {
    // A 1.33 s weapon contacts at 0.565 s: `interval / 2 - 0.1 s` (Rurik's derived law). The client
    // draws the swing over the declared interval, so a hit sent at the interval's end appears after
    // the animation has finished.
    let (mut engine, skills, rules) = world(vec![]);
    engine.set_equipment(ME, Some(axe()), 60);
    engine.set_position(FOE, (100.0, 0.0));
    engine.attack(ME, FOE).expect("a foe in reach");
    engine.tick(1, &skills, &rules);
    let full = engine.actor(FOE).unwrap().health;
    engine.tick(500, &skills, &rules);
    assert_eq!(
        engine.actor(FOE).unwrap().health,
        full,
        "no damage 0.5 s into a 1.33 s swing"
    );
    engine.tick(100, &skills, &rules);
    assert!(
        engine.actor(FOE).unwrap().health < full,
        "the contact is at 0.565 s, not at 1.33 s"
    );
}

#[test]
fn the_windup_law_is_the_measured_one() {
    assert_eq!(swing_windup(1330), 565);
    assert_eq!(swing_windup(1750), 775);
    assert_eq!(swing_windup(2000), 900);
    assert_eq!(swing_windup(2475), 1137);
    assert_eq!(swing_windup(100), 1);
}

#[test]
fn an_attack_out_of_reach_waits_and_a_dead_target_ends_it() {
    let (mut engine, skills, rules) = world(vec![]);
    let mut grawl = Spec::creature(Side::Foe, 2, 120, 26);
    grawl.weapon = Some(axe());
    engine.join(40, grawl, (900.0, 0.0));
    engine.drain();
    engine.attack(40, ME).unwrap();
    engine.tick(2000, &skills, &rules);
    assert!(!engine
        .drain()
        .iter()
        .any(|event| matches!(event, Event::AttackStart { .. })));
    engine.set_position(40, (60.0, 0.0));
    engine.kill(ME, &skills);
    engine.tick(2000, &skills, &rules);
    assert_eq!(engine.actor(40).unwrap().attack_target, None);
}

#[test]
fn attacks_need_a_weapon_and_a_foe() {
    let (mut engine, _skills, _rules) = world(vec![]);
    assert_eq!(engine.attack(ME, FOE), Err(Refusal::NeedsWeapon));
    let mut grawl = Spec::creature(Side::Foe, 2, 120, 26);
    grawl.weapon = Some(axe());
    engine.join(40, grawl, (100.0, 50.0));
    assert_eq!(engine.attack(40, FOE), Err(Refusal::BadTarget));
}

#[test]
fn restore_heals_a_creature_to_full_and_revive_brings_it_back() {
    let (mut engine, skills, rules) = world(vec![]);
    engine.kill(FOE, &skills);
    engine.revive(FOE, (1.0, 2.0), &rules);
    let foe = engine.actor(FOE).unwrap();
    assert!(foe.alive);
    assert_eq!(foe.health, 480.0);
    assert_eq!(foe.position, (1.0, 2.0));
}
