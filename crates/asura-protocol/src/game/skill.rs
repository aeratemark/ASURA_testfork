//! The skill family: the bar, the learned set, a cast's lifecycle, lasting effects, damage and healing,
//! and the attribute window's requests.
//!
//! **None of these is measured on this build.** Every layout is this build's own packet table row; every
//! use and every field meaning is preforged's, which reads the 2022 client (build 37130) and numbers its
//! messages differently. The two numberings were matched by layout, message by message
//! (`docs/52-skills.md`, "The wire"): a server message here is preforged's number plus one from `0x003F`
//! on, the same number below it, and a client message is preforged's number plus two. A capture of a
//! live cast is what settles them, and until one exists each row is `Inferred` in the table.

use super::Builder;
use crate::packets::game as msg;

// ---- the agent properties a skill moves (`0x009F` / `0x00A0` / `0x00A2` / `0x00A3` kinds) -----------

/// `0x009F` kind: the skill a following damage line is attributed to (preforged `gw_combat.h`). The agent
/// is the one struck: the live server sends `[10, 31, 194]` before Flare's `[16, 31, 50, ...]` on agent 31.
pub const PROPERTY_DAMAGE_SKILL: u32 = 10;
/// `0x00EE`: one player attribute's delta. Attribute 0 is experience and attribute 10 is morale; the
/// message is two dwords, the attribute and the delta (Rurik, `toolkit/authsrv/authsrv.py:5911-5960`).
/// The client applies `+=` to the sheet, which is what makes the experience bar move on a kill or a
/// quest hand-in without re-sending the whole `0x00E9` block.
pub fn agent_kill_reward(attribute: u32, delta: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_AGENT_KILL_REWARD)
        .u32(attribute)
        .u32(delta)
        .frame()
}

/// The attribute id an experience award rides (`0x00EE`'s first dword).
pub const ATTRIBUTE_EXPERIENCE: u32 = 0;
/// The attribute id a morale change rides (`0x00EE`'s first dword).
pub const ATTRIBUTE_MORALE: u32 = 10;

/// `0x009F` kind: an ordinary melee attack finished successfully (GWCA `melee_attack_finished`; preforged
/// `gw_weapon_hit.h` `finish_2022`). Sent after the swing's damage, so the client ends the animation at
/// the hit rather than after it.
pub const PROPERTY_MELEE_FINISHED: u32 = 1;

/// `0x00A3` kind: an armor-respecting damage, as a signed fraction of maximum health.
pub const PROPERTY_DAMAGE: u32 = 16;
/// `0x00A0` kind: a body effect played on a target, with the caster beside it.
pub const PROPERTY_TARGET_BODY: u32 = 20;
/// `0x009F` kind: a body effect played on the agent itself.
pub const PROPERTY_BODY: u32 = 21;
/// `0x00A2` kind: current energy, as a fraction of the maximum.
pub const PROPERTY_ENERGY_FRACTION: u32 = 33;
/// `0x00A2` kind: current health, as a fraction of the maximum.
pub const PROPERTY_HEALTH_FRACTION: u32 = 34;
/// `0x009F` kind: maximum energy.
pub const PROPERTY_MAX_ENERGY: u32 = 41;
/// `0x009F` kind: maximum health.
pub const PROPERTY_MAX_HEALTH: u32 = 42;
/// `0x00A2` kind: energy regeneration, as a fraction of the maximum per second.
pub const PROPERTY_ENERGY_REGEN: u32 = 43;
/// `0x00A2` kind: health regeneration, as a fraction of the maximum per second.
pub const PROPERTY_HEALTH_REGEN: u32 = 44;
/// `0x00A0` kind: an ordinary attack's swing (attacker, target, 0). Measured on the live server: once per
/// swing, before the hit (`pre-searing-four-professions-2026-10-11` capture, lines 8508 and 8562).
pub const PROPERTY_ATTACK: u32 = 4;
/// `0x009F` kind: an attack skill's finish.
pub const PROPERTY_ATTACK_FINISHED: u32 = 46;
/// `0x009F` kind: an instant skill was used (it carries the skill id).
pub const PROPERTY_INSTANT: u32 = 48;
/// `0x009F` kind: an attack skill was stopped.
pub const PROPERTY_ATTACK_STOPPED: u32 = 49;
/// `0x00A0` kind: an attack skill's start (caster, target, skill).
pub const PROPERTY_ATTACK_SKILL: u32 = 50;
/// `0x00A3` kind: an armor-ignoring change of health, either way: damage is negative, healing positive.
pub const PROPERTY_HEALTH_CHANGE: u32 = 55;
/// `0x009F` kind: a cast's finish.
pub const PROPERTY_CAST_FINISHED: u32 = 58;
/// `0x009F` kind: a cast was stopped.
pub const PROPERTY_CAST_STOPPED: u32 = 59;
/// `0x00A0` kind: a cast's start (caster, target, skill).
pub const PROPERTY_CAST: u32 = 60;
/// `0x00A2` kind: a knockdown, for this many seconds.
pub const PROPERTY_KNOCKDOWN: u32 = 63;
/// `0x00A0` kind: an attack failed, with the reason in the value (`[38, target, attacker, reason]`).
/// Reason 1 is a dodge, drawn behind the shot's `0x00A7` with no damage (Rurik
/// `toolkit/authsrv/authsrv.py:17300-17378`).
pub const PROPERTY_ATTACK_FAIL: u32 = 38;
/// `PROPERTY_ATTACK_FAIL`'s dodge reason.
pub const ATTACK_FAIL_DODGE: u32 = 1;

/// `GAME_SMSG_AGENT_PROPERTY_UPDATE_INT_TARGET` (`0x00A0`): a property naming two agents.
pub fn agent_property_update_int_target(
    kind: u32,
    agent: u32,
    other: u32,
    value: u32,
) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_AGENT_PROPERTY_UPDATE_INT_TARGET)
        .u32(kind)
        .u32(agent)
        .u32(other)
        .u32(value)
        .frame()
}

/// `GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT_TARGET` (`0x00A3`): a float property naming two agents.
pub fn agent_property_update_float_target(
    kind: u32,
    agent: u32,
    other: u32,
    value: f32,
) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT_TARGET)
        .u32(kind)
        .u32(agent)
        .u32(other)
        .f32(value)
        .frame()
}

// ---- the bar and the learned set ------------------------------------------------------------

/// `GAME_SMSG_SKILLBAR_UPDATE` (`0x00DA`) with its second array named for what preforged reads it as:
/// each slot's copy index. `create` is 1 in a load and 0 in the answer to an edit.
pub fn skillbar(agent: u32, bar: &[(u32, u32); 8], create: bool) -> crate::client::Frame {
    let skills: Vec<u32> = bar.iter().map(|slot| slot.0).collect();
    let copies: Vec<u32> = bar.iter().map(|slot| slot.1).collect();
    super::skillbar_update(agent, &skills, &copies, u8::from(create))
}

/// `GAME_SMSG_SKILLBAR_UPDATE_SKILL` (`0x00D9`): one slot.
pub fn skillbar_update_skill(agent: u32, slot: u8, skill: u16, copy: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_SKILLBAR_UPDATE_SKILL)
        .u32(agent)
        .u8(slot)
        .u16(skill)
        .u32(copy)
        .frame()
}

/// `0x00DC`: a skill newly learned, with its copy count. The client opens its new-skill window, and
/// preforged notes its handler asserts a count of at least 1.
pub fn skill_learned(skill: u16, count: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_SKILL_WINDOW_COUNT)
        .u16(skill)
        .u32(count)
        .frame()
}

/// `0x00DD`: a skill's copy count, silently; 0 forgets it.
pub fn skill_learned_count(skill: u16, count: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_SKILL_LEARNED_COUNT)
        .u16(skill)
        .u32(count)
        .frame()
}

// ---- a cast ---------------------------------------------------------------------------------

fn slot_message(header: u16, agent: u32, skill: u32, copy: u32) -> crate::client::Frame {
    Builder::new(header)
        .u32(agent)
        .u16(u16::try_from(skill).unwrap_or(0))
        .u32(copy)
        .frame()
}

/// `0x00E3`: one pending use of a slot is released - a refusal's whole answer, and part of every
/// finish and stop.
pub fn skill_ack(agent: u32, skill: u32, copy: u32) -> crate::client::Frame {
    slot_message(msg::GAME_SMSG_SKILL_CANCEL, agent, skill, copy)
}

/// `GAME_SMSG_SKILL_ACTIVATE` (`0x00E4`): the slot is in use.
pub fn skill_activate(agent: u32, skill: u32, copy: u32) -> crate::client::Frame {
    slot_message(msg::GAME_SMSG_SKILL_ACTIVATE, agent, skill, copy)
}

/// `GAME_SMSG_SKILL_RECHARGE` (`0x00E5`): the slot recharges for this many seconds.
pub fn skill_recharge(agent: u32, skill: u32, copy: u32, seconds: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_SKILL_RECHARGE)
        .u32(agent)
        .u16(u16::try_from(skill).unwrap_or(0))
        .u32(copy)
        .u32(seconds)
        .frame()
}

/// `GAME_SMSG_SKILL_RECHARGED` (`0x00E6`): the slot is ready.
pub fn skill_recharged(agent: u32, skill: u32, copy: u32) -> crate::client::Frame {
    slot_message(msg::GAME_SMSG_SKILL_RECHARGED, agent, skill, copy)
}

/// `0x00E7`: the slot is spent until the next instance (Resurrection Signet).
pub fn skill_used_once(agent: u32, skill: u32, copy: u32) -> crate::client::Frame {
    slot_message(msg::GAME_SMSG_SKILL_USED_ONCE, agent, skill, copy)
}

/// `0x00D1`: a slot's adrenaline, in units (25 to a strike).
pub fn skill_adrenaline(agent: u32, skill: u32, copy: u32, units: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_SKILL_ADRENALINE)
        .u32(agent)
        .u16(u16::try_from(skill).unwrap_or(0))
        .u32(copy)
        .u32(units)
        .frame()
}

/// `0x00D2`: a slot's adrenaline was spent.
pub fn skill_adrenaline_spent(agent: u32, skill: u32, copy: u32) -> crate::client::Frame {
    slot_message(msg::GAME_SMSG_SKILL_ADRENALINE_SPENT, agent, skill, copy)
}

/// `0x0035`: an attack's timing - the interval in seconds and the speed factor applied to it.
pub fn agent_attack_timing(agent: u32, interval: f32, modifier: f32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_AGENT_ATTACK_TIMING)
        .u32(agent)
        .f32(interval)
        .f32(modifier)
        .frame()
}

/// `GAME_SMSG_AGENT_PROJECTILE_LAUNCHED` (`0x00A4`): something flies from `agent` to a point. The live
/// server's records (`packets-30352-81363781.jsonl`): the point, plane 0, the flight in seconds (about
/// 0.6), a model word (1 a River Skale's spit, 143 an arrow or a Grawl's spear, 343 a Firestarter's), a
/// counter that runs 1, 2, 3 per shooter, and a last byte (1 for arrows and spears, 0 otherwise).
pub fn agent_projectile_launched(
    agent: u32,
    to: (f32, f32),
    seconds: f32,
    model: u32,
    counter: u32,
    flag: u8,
) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_AGENT_PROJECTILE_LAUNCHED)
        .u32(agent)
        .f32(to.0)
        .f32(to.1)
        .u16(0)
        .f32(seconds)
        .u32(model)
        .u32(counter)
        .u8(flag)
        .frame()
}

// ---- lasting effects ------------------------------------------------------------------------

/// `GAME_SMSG_AGENT_PROJECTILE_ARRIVED` (`0x00A7`): the shot a launch opened, closed. The agent, the
/// `0x00A4` handle it answers, and the weapon's damage-type word. Measured at the launch's time plus
/// its flight in 311 live pairs (Rurik `schema/overrides.json` 167).
pub fn agent_projectile_arrived(agent: u32, counter: u32, kind: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_AGENT_PROJECTILE_ARRIVED)
        .u32(agent)
        .u32(counter)
        .u32(kind)
        .frame()
}

/// `GAME_SMSG_EFFECT_APPLIED` (`0x0042`): an effect on an agent, its skill and rank, the id it is
/// removed by, and its duration in seconds.
pub fn effect_applied(
    agent: u32,
    skill: u32,
    rank: u32,
    id: u32,
    seconds: f32,
) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_EFFECT_APPLIED)
        .u32(agent)
        .u16(u16::try_from(skill).unwrap_or(0))
        .u32(rank)
        .u32(id)
        .f32(seconds)
        .frame()
}

/// `GAME_SMSG_EFFECT_REMOVED` (`0x0044`).
pub fn effect_removed(agent: u32, id: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_EFFECT_REMOVED)
        .u32(agent)
        .u32(id)
        .frame()
}

/// `GAME_SMSG_EFFECT_UPKEEP_APPLIED` (`0x0041`): a maintained effect, which has no duration.
pub fn effect_upkeep_applied(
    agent: u32,
    caster: u32,
    skill: u32,
    rank: u32,
    id: u32,
) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_EFFECT_UPKEEP_APPLIED)
        .u32(agent)
        .u32(caster)
        .u16(u16::try_from(skill).unwrap_or(0))
        .u32(rank)
        .u32(id)
        .frame()
}

/// `GAME_SMSG_EFFECT_UPKEEP_REMOVED` (`0x0040`).
pub fn effect_upkeep_removed(agent: u32, id: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_EFFECT_UPKEEP_REMOVED)
        .u32(agent)
        .u32(id)
        .frame()
}

// ---- attributes -----------------------------------------------------------------------------

/// `GAME_SMSG_AGENT_UPDATE_ATTRIBUTES` (`0x003A`) as preforged fills it: the listed attribute ids, then
/// their base ranks, then their effective ranks, in one counted array.
pub fn attributes(agent: u32, listed: &[(u32, u32, u32)]) -> crate::client::Frame {
    let mut words = Vec::with_capacity(listed.len() * 3);
    words.extend(listed.iter().map(|entry| entry.0));
    words.extend(listed.iter().map(|entry| entry.1));
    words.extend(listed.iter().map(|entry| entry.2));
    super::agent_update_attributes(agent, &words)
}

/// `0x0038`: the free attribute points.
pub fn attribute_points_free(agent: u32, free: u8) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_AGENT_ATTRIBUTE_POINTS_FREE)
        .u32(agent)
        .u8(free)
        .frame()
}

/// `0x0039`: the attribute points earned in total.
pub fn attribute_points_total(agent: u32, total: u8) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_AGENT_ATTRIBUTE_POINTS_TOTAL)
        .u32(agent)
        .u8(total)
        .frame()
}

// ---- the client's requests ------------------------------------------------------------------

fn dword(body: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(body.get(at..at + 4)?.try_into().ok()?))
}

/// A skill use: `GAME_CMSG_USE_SKILL` (`0x8046`) or `GAME_CMSG_USE_ATTACK_SKILL` (`0x8027`). Both
/// carry the skill, its copy, the target and a call byte, which preforged keeps and does not interpret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UseSkill {
    /// The skill id in the slot.
    pub skill: u32,
    /// Which copy of it.
    pub copy: u32,
    /// The agent it is aimed at; 0 for none.
    pub target: u32,
    /// The call byte, kept and not interpreted.
    pub call: u8,
}

/// Read a skill use's body (13 bytes).
pub fn read_use_skill(body: &[u8]) -> Option<UseSkill> {
    if body.len() != 13 {
        return None;
    }
    Some(UseSkill {
        skill: dword(body, 0)?,
        copy: dword(body, 4)?,
        target: dword(body, 8)?,
        call: body[12],
    })
}

/// Read `GAME_CMSG_ATTACK_AGENT` (`0x8026`): the agent to attack and a byte (5 bytes). The byte is
/// unnamed; by its place in the use-skill request it is the "call target" flag.
pub fn read_attack_agent(body: &[u8]) -> Option<(u32, u8)> {
    if body.len() != 5 {
        return None;
    }
    Some((dword(body, 0)?, body[4]))
}

/// `GAME_CMSG_SKILLBAR_SKILL_SET` (`0x805C`): agent, slot, skill, copy.
pub fn read_skillbar_set(body: &[u8]) -> Option<(u32, u32, u32, u32)> {
    if body.len() != 16 {
        return None;
    }
    Some((
        dword(body, 0)?,
        dword(body, 4)?,
        dword(body, 8)?,
        dword(body, 12)?,
    ))
}

/// `GAME_CMSG_SKILLBAR_LOAD` (`0x805D`): agent, then a counted list of up to eight skill ids.
pub fn read_skillbar_load(body: &[u8]) -> Option<(u32, Vec<u32>)> {
    let agent = dword(body, 0)?;
    let count = usize::from(u16::from_le_bytes(body.get(4..6)?.try_into().ok()?));
    if count > 8 || body.len() != 6 + 4 * count {
        return None;
    }
    let ids = (0..count).filter_map(|n| dword(body, 6 + 4 * n)).collect();
    Some((agent, ids))
}

/// `GAME_CMSG_SKILLBAR_SKILL_REPLACE` (`0x805E`): agent, then two (skill, copy) pairs to exchange.
pub fn read_skillbar_swap(body: &[u8]) -> Option<(u32, (u32, u32), (u32, u32))> {
    if body.len() != 20 {
        return None;
    }
    Some((
        dword(body, 0)?,
        (dword(body, 4)?, dword(body, 8)?),
        (dword(body, 12)?, dword(body, 16)?),
    ))
}

/// `GAME_CMSG_SKILLBAR_SKILL_MOVE` (`0x805F`): agent, (skill, copy), destination slot.
pub fn read_skillbar_move(body: &[u8]) -> Option<(u32, (u32, u32), u32)> {
    if body.len() != 16 {
        return None;
    }
    Some((
        dword(body, 0)?,
        (dword(body, 4)?, dword(body, 8)?),
        dword(body, 12)?,
    ))
}

/// `GAME_CMSG_ATTRIBUTE_INCREASE` / `_DECREASE` (`0x800F` / `0x800E`): agent, a token preforged keeps
/// and does not interpret, and the attribute.
pub fn read_attribute_change(body: &[u8]) -> Option<(u32, u32)> {
    if body.len() != 12 {
        return None;
    }
    Some((dword(body, 0)?, dword(body, 8)?))
}

/// `GAME_CMSG_ATTRIBUTE_LOAD` (`0x8010`): agent, a counted list of attribute ids and a counted list of
/// their ranks, of the same length.
pub fn read_attribute_load(body: &[u8]) -> Option<(u32, Vec<(u32, u32)>)> {
    let agent = dword(body, 0)?;
    let count = usize::from(u16::from_le_bytes(body.get(4..6)?.try_into().ok()?));
    let ranks_at = 6 + 4 * count;
    let ranks = usize::from(u16::from_le_bytes(
        body.get(ranks_at..ranks_at + 2)?.try_into().ok()?,
    ));
    if count > 16 || ranks != count || body.len() != ranks_at + 2 + 4 * count {
        return None;
    }
    let pairs = (0..count)
        .filter_map(|n| Some((dword(body, 6 + 4 * n)?, dword(body, ranks_at + 2 + 4 * n)?)))
        .collect();
    Some((agent, pairs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(frame: &crate::client::Frame) -> usize {
        frame.encode().len()
    }

    #[test]
    fn a_kill_reward_is_the_attribute_and_the_delta() {
        // `0x00EE [0, 2000]`: experience 2000, the two dwords Rurik measured on the kill template.
        assert_eq!(
            agent_kill_reward(ATTRIBUTE_EXPERIENCE, 2000).encode(),
            vec![0xEE, 0x00, 0, 0, 0, 0, 0xD0, 0x07, 0, 0]
        );
        assert_eq!(wire(&agent_kill_reward(ATTRIBUTE_MORALE, 100)), 10);
    }

    #[test]
    fn an_attack_request_is_the_agent_and_a_byte() {
        assert_eq!(read_attack_agent(&[7, 0, 0, 0, 1]), Some((7, 1)));
        assert_eq!(read_attack_agent(&[7, 0, 0, 0]), None);
    }

    #[test]
    fn every_builder_is_its_table_rows_size() {
        assert_eq!(wire(&skill_ack(1, 281, 0)), 12);
        assert_eq!(wire(&skill_activate(1, 281, 0)), 12);
        assert_eq!(wire(&skill_recharge(1, 281, 0, 2)), 16);
        assert_eq!(wire(&skill_recharged(1, 281, 0)), 12);
        assert_eq!(wire(&skill_used_once(1, 2, 0)), 12);
        assert_eq!(wire(&skill_adrenaline(1, 384, 0, 25)), 16);
        assert_eq!(wire(&skill_adrenaline_spent(1, 384, 0)), 12);
        assert_eq!(wire(&agent_attack_timing(1, 1.33, 1.0)), 14);
        assert_eq!(wire(&effect_applied(1, 281, 12, 7, 8.0)), 20);
        assert_eq!(wire(&effect_removed(1, 7)), 10);
        assert_eq!(wire(&effect_upkeep_applied(1, 2, 248, 12, 7)), 20);
        assert_eq!(wire(&effect_upkeep_removed(1, 7)), 10);
        assert_eq!(wire(&agent_property_update_int_target(60, 1, 2, 281)), 18);
        assert_eq!(wire(&agent_property_update_float_target(55, 1, 2, 0.1)), 18);
        assert_eq!(wire(&skillbar_update_skill(1, 3, 281, 0)), 13);
        assert_eq!(wire(&skill_learned(281, 1)), 8);
        assert_eq!(wire(&skill_learned_count(281, 1)), 8);
        assert_eq!(wire(&attribute_points_free(1, 5)), 7);
        assert_eq!(wire(&attribute_points_total(1, 5)), 7);
        assert_eq!(
            wire(&skillbar(1, &[(281, 0); 8], true)),
            75,
            "two u16-counted arrays of eight"
        );
        assert_eq!(
            wire(&attributes(1, &[(13, 1, 1), (14, 0, 0)])),
            4 + 2 + 2 + 4 * 6
        );
    }

    #[test]
    fn the_readers_take_exactly_their_layouts() {
        let mut body = Vec::new();
        for value in [281u32, 0, 0x55] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        body.push(1);
        assert_eq!(
            read_use_skill(&body),
            Some(UseSkill {
                skill: 281,
                copy: 0,
                target: 0x55,
                call: 1
            })
        );
        assert_eq!(read_use_skill(&body[..12]), None);

        let mut load = 7u32.to_le_bytes().to_vec();
        load.extend_from_slice(&2u16.to_le_bytes());
        load.extend_from_slice(&281u32.to_le_bytes());
        load.extend_from_slice(&1u32.to_le_bytes());
        assert_eq!(read_skillbar_load(&load), Some((7, vec![281, 1])));

        let mut attributes = 7u32.to_le_bytes().to_vec();
        attributes.extend_from_slice(&1u16.to_le_bytes());
        attributes.extend_from_slice(&13u32.to_le_bytes());
        attributes.extend_from_slice(&1u16.to_le_bytes());
        attributes.extend_from_slice(&9u32.to_le_bytes());
        assert_eq!(read_attribute_load(&attributes), Some((7, vec![(13, 9)])));
    }
}
