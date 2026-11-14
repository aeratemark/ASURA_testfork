//! A character's side of skills: what it has learned, its bar and its ranks; the row they live in; the
//! load block's messages for them; and the client's requests that change or use them.
//!
//! Every answer to a bar or attribute request is the authoritative state, accepted or not: preforged's
//! rule (`gw_skillbar.h` `apply`, `gw_attributes.h` `propose`) is that a well-framed refused edit is
//! answered with the bar or ranks as they stand, never with silence and never with a disconnect.

use std::collections::BTreeSet;

use asura_protocol::game::skill as wire;
use asura_rules::skills::{self as rules, AttributeEdit, AttributeTable, BarEdit, SkillFacts};

use crate::skill::catalog::Pack;
use crate::skill::engine::{Refusal, Slot, ATTRIBUTES};
use crate::skill::Recipient;
use crate::*;

/// What a character has learned, has on its bar and has put its attribute points into.
#[derive(Debug, Clone, PartialEq)]
pub struct Loadout {
    pub learned: BTreeSet<u32>,
    pub bar: [Slot; 8],
    pub ranks: [u8; ATTRIBUTES],
}

impl Default for Loadout {
    fn default() -> Loadout {
        Loadout {
            learned: BTreeSet::new(),
            bar: crate::skill::EMPTY_BAR,
            ranks: crate::skill::NO_RANKS,
        }
    }
}

impl Loadout {
    /// A row as a loadout; no row is a new character's empty one.
    pub fn from_profile(profile: Option<asura_db::skills::Profile>) -> Loadout {
        let Some(profile) = profile else {
            return Loadout::default();
        };
        let mut ranks = crate::skill::NO_RANKS;
        for (attribute, rank) in profile.ranks.iter().enumerate().take(ATTRIBUTES) {
            ranks[attribute] = *rank;
        }
        Loadout {
            learned: profile.learned.into_iter().collect(),
            bar: profile.bar,
            ranks,
        }
    }

    /// The loadout as its row.
    pub fn to_profile(&self) -> asura_db::skills::Profile {
        asura_db::skills::Profile {
            learned: self.learned.iter().copied().collect(),
            bar: self.bar,
            ranks: self.ranks.to_vec(),
        }
    }
}

/// Read a character's loadout. A row that cannot be read is logged and treated as empty, so a database
/// hiccup costs the bar for one session rather than the session.
pub async fn load(world: &World, character: Uuid) -> Loadout {
    match asura_db::skills::profile(&world.pool, character).await {
        Ok(profile) => Loadout::from_profile(profile),
        Err(err) => {
            eprintln!("world: skills: could not read the skills of {character}: {err}");
            Loadout::default()
        }
    }
}

async fn save(world: &World, player: &Player) {
    if let Err(err) =
        asura_db::skills::save(&world.pool, player.character, &player.skills.to_profile()).await
    {
        eprintln!(
            "world: skills: could not save the skills of {}: {err}",
            player.character
        );
    }
}

/// Teach the character these skills, as a quest does on acceptance or at its turn-in: the learned set is
/// saved, and the client gets the new set and one learned notice per skill it did not know (the notice
/// that opens its new-skill window). A skill it already knows is skipped, so a retaken quest teaches
/// nothing twice. No skill, no frames.
pub async fn teach(world: &World, player: &mut Player, skills: &[u32]) -> Vec<Frame> {
    let new: Vec<u32> = skills
        .iter()
        .copied()
        .filter(|id| *id != 0 && *id <= u32::from(u16::MAX))
        .filter(|id| player.skills.learned.insert(*id))
        .collect();
    if new.is_empty() {
        return Vec::new();
    }
    let mut frames = vec![learned_frame(&player.skills)];
    for id in &new {
        frames.push(wire::skill_learned_count(*id as u16, 1));
    }
    save(world, player).await;
    eprintln!("world: {} learned skill(s) {new:?}", player.name);
    frames
}

fn table(pack: &Pack) -> AttributeTable<'_> {
    AttributeTable {
        professions: &pack.attributes.professions,
        rank_cost: &pack.attributes.rank_cost,
        level_points: &pack.attributes.level_points,
        max_rank: pack.attributes.max_rank,
        max_attributes: pack.attributes.max_attributes,
    }
}

fn facts(pack: &Pack) -> impl Fn(u32) -> Option<SkillFacts> + '_ {
    |id| {
        pack.skills.get(id).map(|skill| SkillFacts {
            profession: skill.profession,
            elite: skill.elite,
            pve_only: skill.pve_only,
        })
    }
}

/// Whether a bar may be changed here: in a town or an outpost, as the client itself only offers.
fn may_change(player: &Player) -> bool {
    player.instance.map.map_type == map_kinds().main_town
}

// ---- the load block ------------------------------------------------------------------------

/// The attribute points: free and total, for `AGENT_UPDATE_ATTRIBUTE_POINTS`.
pub fn attribute_points(loadout: &Loadout, level: u32, pack: &Pack) -> (u8, u8) {
    let table = table(pack);
    let total = table.points(level);
    let free = total.saturating_sub(table.spent(&loadout.ranks));
    (
        u8::try_from(free).unwrap_or(u8::MAX),
        u8::try_from(total).unwrap_or(u8::MAX),
    )
}

/// `AGENT_UPDATE_ATTRIBUTES` for the character's professions: each listed attribute's id, base rank
/// and effective rank (no equipment bonus is applied yet, so the two are equal).
pub fn attributes_frame(
    agent: u32,
    loadout: &Loadout,
    traits: &db_game::Traits,
    pack: &Pack,
) -> Frame {
    let listed: Vec<(u32, u32, u32)> = table(pack)
        .listed(traits.profession, traits.secondary_profession)
        .into_iter()
        .map(|attribute| {
            let rank = u32::from(loadout.ranks[attribute]);
            (attribute as u32, rank, rank)
        })
        .collect();
    wire::attributes(agent, &listed)
}

/// The attribute answer preforged sends after an edit: the snapshot, then free and total points.
fn attribute_frames(
    agent: u32,
    loadout: &Loadout,
    traits: &db_game::Traits,
    pack: &Pack,
) -> Vec<Frame> {
    let (free, total) = attribute_points(loadout, u32::from(traits.level.max(1)), pack);
    vec![
        attributes_frame(agent, loadout, traits, pack),
        wire::attribute_points_free(agent, free),
        wire::attribute_points_total(agent, total),
    ]
}

/// The learned set as `GAME_SMSG_UPDATE_UNLOCKED_SKILLS` carries it.
pub fn learned_frame(loadout: &Loadout) -> Frame {
    game::update_unlocked_skills(&rules::learned_bitmap(loadout.learned.iter().copied()))
}

/// The bar as the load block carries it (`create` 1).
pub fn bar_frame(agent: u32, loadout: &Loadout) -> Frame {
    wire::skillbar(agent, &loadout.bar, true)
}

/// The three vitals the load block sends where the reference sent 25, 400 and 0.0: maximum energy,
/// maximum health and the energy regeneration rate.
pub fn vitals(traits: &db_game::Traits, pack: &Pack) -> (u32, u32, f32) {
    let level = u32::from(traits.level.max(1));
    let energy = pack.player.energy_max;
    let regen = f64::from(pack.player.energy_regen_pips) * pack.rules.energy_pip_per_second
        / f64::from(energy.max(1));
    (energy, pack.player.max_health(level), regen as f32)
}

// ---- joining and leaving the fight ---------------------------------------------------------

/// Put the player's agent in its instance's fight, with its loadout, level and equipment.
pub fn enter_combat(player: &Player) {
    let Ok(pack) = crate::skill::catalog::current() else {
        return;
    };
    let spec = crate::skill::player_spec(&player.skills, &player.traits, &player.inventory, pack);
    // The position first: `standing` takes the slots' lock, which is never taken under the fight's.
    let position = player.standing().position;
    player
        .instance
        .combat()
        .join_player(player.agent_id, player.player_id, spec, position);
}

/// Send what a fight produced to its recipients. A broadcast goes only to the clients that know
/// every agent it names, so no client is asked to look up a body it was never sent.
pub fn deliver(instance: &Instance, deliveries: Vec<(Recipient, Vec<u32>, Vec<Frame>)>) {
    for (recipient, subjects, frames) in deliveries {
        match recipient {
            Recipient::Player(player_id) => instance.send_to(player_id, &frames),
            Recipient::Everyone => instance.broadcast_known(&subjects, &frames),
        }
    }
}

// ---- using a skill -------------------------------------------------------------------------

/// `GAME_CMSG_USE_SKILL` and `GAME_CMSG_USE_ATTACK_SKILL`: use a bar slot on a target.
pub async fn handle_use_skill(
    frame: &Frame,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some(request) = wire::read_use_skill(frame.body()) else {
        if trace {
            eprintln!(
                "world: skills: a use request of the wrong size ({} bytes)",
                frame.body().len()
            );
        }
        return Ok(true);
    };
    let Ok(pack) = crate::skill::catalog::current() else {
        eprintln!("world: skills: no skill pack; the request is answered with its release only");
        player.instance.send_to(
            player.player_id,
            &[wire::skill_ack(
                player.agent_id,
                request.skill,
                request.copy,
            )],
        );
        return Ok(true);
    };
    let (weapon, armor) = crate::skill::equipment(&player.inventory, pack);
    let (result, deliveries) = {
        let mut combat = player.instance.combat();
        combat.engine.set_equipment(player.agent_id, weapon, armor);
        let result = combat.engine.use_skill(
            player.agent_id,
            request.skill,
            request.copy,
            request.target,
            &pack.skills,
            &pack.rules,
        );
        (result, combat.deliveries(pack))
    };
    if let Err(refusal) = &result {
        if trace || !matches!(refusal, Refusal::Busy | Refusal::Recharging) {
            eprintln!(
                "world: skills: {} could not use skill {} on {}: {refusal:?}",
                player.name, request.skill, request.target
            );
        }
    }
    deliver(&player.instance, deliveries);
    Ok(true)
}

/// `GAME_CMSG_ATTACK_AGENT`: the player's ordinary attack goes on a foe, and the player walks into reach
/// (`crate::approach`); the fight swings once it is there (`Engine::attack`).
pub async fn handle_attack_agent(
    frame: &Frame,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some((target, _call)) = wire::read_attack_agent(frame.body()) else {
        if trace {
            eprintln!(
                "world: skills: an attack request of the wrong size ({} bytes)",
                frame.body().len()
            );
        }
        return Ok(true);
    };
    let Ok(pack) = crate::skill::catalog::current() else {
        return Ok(true);
    };
    let (weapon, armor) = crate::skill::equipment(&player.inventory, pack);
    let result = {
        let mut combat = player.instance.combat();
        combat.engine.set_equipment(player.agent_id, weapon, armor);
        combat.engine.attack(player.agent_id, target).map(|()| {
            combat
                .engine
                .attack_reach(player.agent_id, &pack.rules)
                .unwrap_or(pack.rules.melee_range)
        })
    };
    // The live server walks the attacker to the foe (`crate::approach`): the fight swings once it is
    // in reach. The fight's lock is released before the slot's is taken.
    if let Ok(reach) = &result {
        crate::approach::start(
            player,
            target,
            crate::approach::Purpose::Attack,
            *reach,
            trace,
        );
    }
    if let Err(refusal) = result {
        if trace || refusal != Refusal::BadTarget {
            eprintln!(
                "world: skills: {} could not attack {target}: {refusal:?}",
                player.name
            );
        }
        if refusal == Refusal::NeedsWeapon {
            // What the server sees in the hands: the bags by model, and each equipped item's type and
            // modifier words, so a weapon the decoder does not recognise can be named.
            let bags: Vec<(i32, i32)> = (player.inventory.bags.iter())
                .map(|bag| (bag.bag_id, bag.bag_model_id))
                .collect();
            eprintln!("world: skills: {} bags (id, model): {bags:?}", player.name);
            for bag in &player.inventory.bags {
                if bag.bag_model_id != crate::db_inventory::EQUIPPED_ITEMS_MODEL {
                    continue;
                }
                for item in player.inventory.items_in(bag.bag_id) {
                    let words: Vec<String> = (item.modifier_dwords().iter())
                        .map(|word| format!("{word:#010x}"))
                        .collect();
                    eprintln!(
                        "world: skills:   equipped item type {} modifiers [{}]",
                        item.item_type,
                        words.join(", ")
                    );
                }
            }
        }
    }
    Ok(true)
}

// ---- the bar -------------------------------------------------------------------------------

async fn answer_bar(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    agent: u32,
    edit: BarEdit,
    trace: bool,
) -> Result<bool, GameError> {
    let outcome = match crate::skill::catalog::current() {
        Err(err) => Err(format!("no skill pack: {err}")),
        Ok(_) if agent != player.agent_id => Err(format!("agent {agent} is not this player's")),
        Ok(_) if !may_change(player) => Err(format!("{:?}", rules::BarRefusal::NotHere)),
        Ok(pack) => rules::propose(&player.skills.bar, &edit)
            .and_then(|bar| {
                let learned = &player.skills.learned;
                rules::validate(
                    &bar,
                    player.traits.profession,
                    player.traits.secondary_profession,
                    &|id| learned.contains(&id),
                    &facts(pack),
                )
                .map(|()| bar)
            })
            .map_err(|refusal| format!("{refusal:?}")),
    };
    match outcome {
        Ok(bar) => {
            player.skills.bar = bar;
            player
                .instance
                .combat()
                .engine
                .set_bar(player.agent_id, bar);
            save(world, player).await;
        }
        Err(why) => eprintln!(
            "world: skills: {} bar edit {edit:?} refused: {why}",
            player.name
        ),
    }
    let answer = vec![wire::skillbar(player.agent_id, &player.skills.bar, false)];
    send(stream, session, answer, trace).await?;
    Ok(true)
}

/// `GAME_CMSG_SKILLBAR_SKILL_SET`.
pub async fn handle_bar_set(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some((agent, slot, skill, copy)) = wire::read_skillbar_set(frame.body()) else {
        return Ok(true);
    };
    answer_bar(
        stream,
        session,
        world,
        player,
        agent,
        BarEdit::Set {
            slot,
            skill: (skill, copy),
        },
        trace,
    )
    .await
}

/// `GAME_CMSG_SKILLBAR_LOAD`: a whole template.
pub async fn handle_bar_load(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some((agent, ids)) = wire::read_skillbar_load(frame.body()) else {
        return Ok(true);
    };
    answer_bar(
        stream,
        session,
        world,
        player,
        agent,
        BarEdit::Load { ids },
        trace,
    )
    .await
}

/// `GAME_CMSG_SKILLBAR_SKILL_REPLACE`: two skills on the bar exchange slots.
pub async fn handle_bar_swap(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some((agent, first, second)) = wire::read_skillbar_swap(frame.body()) else {
        return Ok(true);
    };
    answer_bar(
        stream,
        session,
        world,
        player,
        agent,
        BarEdit::Swap { first, second },
        trace,
    )
    .await
}

/// `GAME_CMSG_SKILLBAR_SKILL_MOVE`: a skill on the bar moves to an empty slot.
pub async fn handle_bar_move(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some((agent, skill, slot)) = wire::read_skillbar_move(frame.body()) else {
        return Ok(true);
    };
    answer_bar(
        stream,
        session,
        world,
        player,
        agent,
        BarEdit::Move { skill, slot },
        trace,
    )
    .await
}

// ---- attributes ----------------------------------------------------------------------------

async fn answer_attributes(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    agent: u32,
    edit: AttributeEdit,
    trace: bool,
) -> Result<bool, GameError> {
    let Ok(pack) = crate::skill::catalog::current() else {
        return Ok(true);
    };
    let outcome = if agent != player.agent_id {
        Err(format!("agent {agent} is not this player's"))
    } else {
        rules::apply_attribute_edit(
            &player.skills.ranks,
            &edit,
            player.traits.profession,
            player.traits.secondary_profession,
            u32::from(player.traits.level.max(1)),
            may_change(player),
            &table(pack),
        )
        .map_err(|refusal| format!("{refusal:?}"))
    };
    match outcome {
        Ok(ranks) => {
            player.skills.ranks.copy_from_slice(&ranks[..ATTRIBUTES]);
            player
                .instance
                .combat()
                .engine
                .set_ranks(player.agent_id, player.skills.ranks);
            save(world, player).await;
        }
        Err(why) => eprintln!(
            "world: skills: {} attribute edit {edit:?} refused: {why}",
            player.name
        ),
    }
    let answer = attribute_frames(player.agent_id, &player.skills, &player.traits, pack);
    send(stream, session, answer, trace).await?;
    Ok(true)
}

/// `GAME_CMSG_ATTRIBUTE_INCREASE`.
pub async fn handle_attribute_increase(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some((agent, attribute)) = wire::read_attribute_change(frame.body()) else {
        return Ok(true);
    };
    answer_attributes(
        stream,
        session,
        world,
        player,
        agent,
        AttributeEdit::Increase(attribute),
        trace,
    )
    .await
}

/// `GAME_CMSG_ATTRIBUTE_DECREASE`.
pub async fn handle_attribute_decrease(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some((agent, attribute)) = wire::read_attribute_change(frame.body()) else {
        return Ok(true);
    };
    answer_attributes(
        stream,
        session,
        world,
        player,
        agent,
        AttributeEdit::Decrease(attribute),
        trace,
    )
    .await
}

/// `GAME_CMSG_ATTRIBUTE_LOAD`: a whole allocation.
pub async fn handle_attribute_load(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: &Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    let Some((agent, pairs)) = wire::read_attribute_load(frame.body()) else {
        return Ok(true);
    };
    answer_attributes(
        stream,
        session,
        world,
        player,
        agent,
        AttributeEdit::Load(pairs),
        trace,
    )
    .await
}

// ---- the test command ----------------------------------------------------------------------

/// `/skill`: not the reference's - an ASURA test command, because no quest or trainer teaches skills yet.
///
/// * `/skill list` - what the character has learned, by id and name;
/// * `/skill learn <id>` - learn one pre-Searing skill of the character's professions (the client opens
///   its new-skill window);
/// * `/skill learn all` - learn every one of them, silently;
/// * `/skill forget <id>` - forget one, taking it off the bar.
pub async fn handle_skill_command(
    stream: &mut TcpStream,
    session: &mut Session,
    world: &World,
    player: &mut Player,
    argument: &str,
    trace: bool,
) -> Result<bool, GameError> {
    let Ok(pack) = crate::skill::catalog::current() else {
        send(
            stream,
            session,
            crate::chat::tell(player, "Skills are unavailable: the pack has no skills."),
            trace,
        )
        .await?;
        return Ok(true);
    };
    let mut words = argument.split_whitespace();
    let verb = words.next().unwrap_or_default();
    let what = words.next().unwrap_or_default();
    let usable = |id: u32| {
        pack.skills.get(id).is_some_and(|skill| {
            skill.profession == 0
                || skill.profession == player.traits.profession
                || skill.profession == player.traits.secondary_profession
        })
    };
    let mut frames = Vec::new();
    let line = match (verb, what) {
        ("list", _) => {
            let names: Vec<String> = player
                .skills
                .learned
                .iter()
                .map(|id| match pack.skills.get(*id) {
                    Some(skill) => format!("{id} {}", skill.name),
                    None => id.to_string(),
                })
                .collect();
            if names.is_empty() {
                "No skills learned. /skill learn <id> or /skill learn all.".to_string()
            } else {
                format!("Learned: {}", names.join(", "))
            }
        }
        ("learn", "all") => {
            let mut ids: Vec<u32> = pack
                .skills
                .by_id
                .keys()
                .copied()
                .filter(|id| usable(*id))
                .collect();
            ids.sort_unstable();
            let new: Vec<u32> = ids
                .into_iter()
                .filter(|id| player.skills.learned.insert(*id))
                .collect();
            frames.push(learned_frame(&player.skills));
            for id in &new {
                frames.push(wire::skill_learned_count(*id as u16, 1));
            }
            save(world, player).await;
            format!("Learned {} skill(s).", new.len())
        }
        ("learn", id) => match id.parse::<u32>() {
            Ok(id) if usable(id) => {
                if player.skills.learned.insert(id) {
                    frames.push(learned_frame(&player.skills));
                    frames.push(wire::skill_learned(id as u16, 1));
                    save(world, player).await;
                }
                format!(
                    "Learned {}.",
                    pack.skills
                        .get(id)
                        .map(|skill| skill.name.as_str())
                        .unwrap_or("")
                )
            }
            _ => "That is not a pre-Searing skill of your professions.".to_string(),
        },
        ("forget", id) => match id.parse::<u32>() {
            Ok(id) if player.skills.learned.remove(&id) => {
                for slot in player.skills.bar.iter_mut() {
                    if slot.0 == id {
                        *slot = (0, 0);
                    }
                }
                player
                    .instance
                    .combat()
                    .engine
                    .set_bar(player.agent_id, player.skills.bar);
                frames.push(wire::skill_learned_count(id as u16, 0));
                frames.push(learned_frame(&player.skills));
                frames.push(wire::skillbar(player.agent_id, &player.skills.bar, false));
                save(world, player).await;
                "Forgotten.".to_string()
            }
            _ => "That skill is not learned.".to_string(),
        },
        _ => "Usage: /skill list | /skill learn <id> | /skill learn all | /skill forget <id>"
            .to_string(),
    };
    frames.extend(crate::chat::tell(player, &line));
    send(stream, session, frames, trace).await?;
    Ok(true)
}
