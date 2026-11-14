//! Skills: what a character has learned and put on its bar, its attributes, and what happens when it
//! uses a skill (`docs/52-skills.md`).
//!
//! The parts, and why they are apart:
//!
//! * [`model`] and [`engine`] are the mechanics, with no file, socket or database in them, so they are
//!   tested on their own (`tests.rs`);
//! * [`catalog`] reads the world pack's `skills.json`, `combat.json` and `attributes.json` into them -
//!   what a skill *does* is declared there and executed here (`docs/35-skill-mechanics-and-behavior.md`);
//! * [`wire`] turns what the engine did into messages and says who hears each;
//! * [`character`] is a character's own side: its learned set, bar and ranks, their rows, the load
//!   block's messages and the client's requests.
//!
//! The instance owns one [`Combat`]: every player and every NPC with a combat block, ticked with the
//! instance clock (`Instance::step`).

pub mod catalog;
pub mod character;
pub mod engine;
pub mod model;
pub mod wire;

#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashMap};

use crate::skill::catalog::Pack;
use crate::skill::engine::{Engine, Event, Side, Slot, Spec, ATTRIBUTES};
use crate::skill::model::Weapon;
use crate::skill::wire::To;
use crate::{db_game, db_inventory, Frame, Inventory};

pub use character::Loadout;

/// Where one batch of messages goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recipient {
    /// One player's connection.
    Player(u32),
    /// Every client in the instance that has entered it.
    Everyone,
}

/// One instance's combatants, and which player drives each agent.
pub struct Combat {
    pub engine: Engine,
    owners: HashMap<u32, u32>,
}

impl Default for Combat {
    fn default() -> Combat {
        Combat::new(0x5EED_A5A5)
    }
}

impl Combat {
    pub fn new(seed: u64) -> Combat {
        Combat {
            engine: Engine::new(seed),
            owners: HashMap::new(),
        }
    }

    /// A player's agent joins the fight.
    pub fn join_player(&mut self, agent: u32, player_id: u32, spec: Spec, position: (f32, f32)) {
        self.owners.insert(agent, player_id);
        self.engine.join(agent, spec, position);
    }

    /// An agent leaves.
    pub fn leave(&mut self, agent: u32) {
        self.owners.remove(&agent);
        self.engine.leave(agent);
    }

    /// The agent a player drives, if it is in the fight.
    pub fn owner_of(&self, agent: u32) -> Option<u32> {
        self.owners.get(&agent).copied()
    }

    /// Everything the engine did since the last call, as messages for their recipients and the agents
    /// the batch names. A message for an agent no player drives (an NPC's own energy) is dropped:
    /// there is nobody to tell.
    pub fn deliveries(&mut self, pack: &Pack) -> Vec<(Recipient, Vec<u32>, Vec<Frame>)> {
        let mut out: Vec<(Recipient, Vec<u32>, Vec<Frame>)> = Vec::new();
        for event in self.engine.drain() {
            let (to, frames) = wire::frames(&event, &pack.rules);
            let recipient = match to {
                To::All => Recipient::Everyone,
                To::Owner(agent) => match self.owners.get(&agent) {
                    Some(player_id) => Recipient::Player(*player_id),
                    None => continue,
                },
            };
            let subjects = wire::subjects(&event);
            // Consecutive messages for the same recipient and the same agents travel together, in
            // order: a client either knows every body the batch names or sees none of it.
            match out.last_mut() {
                Some((last, batch_subjects, batch))
                    if *last == recipient && *batch_subjects == subjects =>
                {
                    batch.extend(frames)
                }
                _ => out.push((recipient, subjects, frames)),
            }
        }
        out
    }

    /// The events a load block's vitals are, for one agent.
    pub fn vitals(&self, agent: u32, pack: &Pack) -> Vec<Event> {
        self.engine.vitals(agent, &pack.rules)
    }
}

/// The weapon in the equipped bag that an attack skill can be used with, and the armor rating the
/// equipped body pieces add up to (each piece's `572` word times its slot's share). A body with no armor
/// word at all has the pack's default rating.
pub fn equipment(inventory: &Inventory, pack: &Pack) -> (Option<Weapon>, i32) {
    let Some(bag) = inventory
        .bags
        .iter()
        .find(|bag| bag.bag_model_id == db_inventory::EQUIPPED_ITEMS_MODEL)
    else {
        return (None, pack.player.armor_default);
    };
    let mut weapon = None;
    let mut armor = 0.0;
    let mut armored = false;
    for item in inventory.items_in(bag.bag_id) {
        let Ok(item_type) = u8::try_from(item.item_type) else {
            continue;
        };
        let effects = item.modifier_dwords().into_iter().map(|word| {
            crate::item_modifier::ModifierWord::new(word)
                .decode()
                .effect()
        });
        if let Some(row) = pack.weapons.get(&item_type) {
            for effect in effects {
                if let crate::item_modifier::ModifierEffect::DamageRange { min, max } = effect {
                    weapon = Some(Weapon {
                        item_type,
                        attribute: row.attribute,
                        period_ms: row.period_ms,
                        kind: row.kind,
                        ranged: row.ranged,
                        min: u32::from(min),
                        max: u32::from(max).max(u32::from(min)),
                        // A player's arrow: model 143 with the byte 1, as the live server shows a
                        // ranger's shots (`packets-30352-81363781.jsonl`, `0x00A4` from agent 31).
                        projectile: row.ranged.then_some((143, 1)),
                    });
                }
            }
        } else if let Some(share) = pack.armor_slots.get(&item_type) {
            for effect in effects {
                if let crate::item_modifier::ModifierEffect::Armor { amount } = effect {
                    armor += share * f64::from(amount);
                    armored = true;
                }
            }
        }
    }
    let armor = if armored {
        armor.round() as i32
    } else {
        pack.player.armor_default
    };
    (weapon, armor)
}

/// What a player is to the engine: its professions, level and ranks, its bar, its vitals from the pack's
/// player rules, and what it is wearing.
pub fn player_spec(
    loadout: &Loadout,
    traits: &db_game::Traits,
    inventory: &Inventory,
    pack: &Pack,
) -> Spec {
    let level = u32::from(traits.level.max(1));
    let (weapon, armor) = equipment(inventory, pack);
    Spec {
        side: Side::Player,
        level,
        primary: traits.profession,
        secondary: traits.secondary_profession,
        ranks: loadout.ranks,
        bar: loadout.bar,
        max_health: pack.player.max_health(level),
        max_energy: pack.player.energy_max,
        energy_pips: pack.player.energy_regen_pips,
        armor,
        weapon,
    }
}

/// An NPC's combat block, as the engine sees it.
pub fn npc_spec(combat: &crate::npc::content::NpcCombat, level: u32) -> Spec {
    let side = if combat.hostile {
        Side::Foe
    } else {
        Side::Neutral
    };
    let mut spec = Spec::creature(side, level.max(1), combat.health, combat.armor);
    spec.max_energy = combat.energy;
    spec.energy_pips = combat.energy_regen;
    spec.primary = combat.profession;
    spec.ranks = [combat.rank; ATTRIBUTES];
    // Every skill on the bar is copy 0: a creature has one of each.
    for (slot, skill) in spec.bar.iter_mut().zip(combat.skills.iter()) {
        *slot = (*skill, 0);
    }
    spec.weapon = combat.attack.as_ref().and_then(|attack| {
        Some(Weapon {
            item_type: attack.item_type,
            // The creature's rank is the same in every attribute, so any index reads it.
            attribute: 0,
            period_ms: attack.period_ms,
            kind: crate::skill::model::DamageKind::parse(&attack.damage_type)?,
            ranged: attack.ranged,
            min: attack.min,
            max: attack.max.max(attack.min),
            projectile: attack.projectile.map(|model| (model, attack.projectile_flag)),
        })
    });
    spec
}

/// A learned set as a sorted list, the order rows and messages use.
pub fn learned_list(learned: &BTreeSet<u32>) -> Vec<u32> {
    learned.iter().copied().collect()
}

/// An empty bar.
pub const EMPTY_BAR: [Slot; 8] = [(0, 0); 8];
/// No ranks anywhere.
pub const NO_RANKS: [u8; ATTRIBUTES] = [0; ATTRIBUTES];

#[cfg(test)]
mod delivery_tests {
    use super::*;

    #[test]
    fn a_slot_message_reaches_its_owner_and_a_cast_reaches_everyone() {
        let pack = catalog::from_json(
            include_str!("../../../../config/World/Vanilla/skills.json"),
            include_str!("../../../../config/World/Vanilla/combat.json"),
            include_str!("../../../../config/World/Vanilla/attributes.json"),
        )
        .expect("the committed pack");
        let mut combat = Combat::default();
        let mut spec = Spec::creature(Side::Player, 1, 100, 60);
        spec.max_energy = 25;
        spec.bar[0] = (281, 0);
        combat.join_player(5, 1, spec, (0.0, 0.0));
        combat
            .engine
            .use_skill(5, 281, 0, 0, &pack.skills, &pack.rules)
            .expect("Orison on itself");
        let out = combat.deliveries(&pack);
        assert_eq!(
            out[0].0,
            Recipient::Player(1),
            "energy and the slot are the owner's"
        );
        assert_eq!(
            out[0].1,
            vec![5],
            "a slot message names the agent it belongs to"
        );
        assert_eq!(
            out[0].2.iter().map(Frame::header).collect::<Vec<_>>(),
            vec![
                crate::game::AGENT_PROPERTY_UPDATE_FLOAT,
                asura_protocol::packets::game::GAME_SMSG_SKILL_ACTIVATE
            ]
        );
        assert_eq!(
            out[1].0,
            Recipient::Everyone,
            "the cast is drawn by everyone"
        );
        assert_eq!(out[1].1, vec![5, 5], "the caster and its target");
        assert_eq!(
            out[1].2[0].header(),
            asura_protocol::packets::game::GAME_SMSG_AGENT_PROPERTY_UPDATE_INT_TARGET
        );
    }
}
