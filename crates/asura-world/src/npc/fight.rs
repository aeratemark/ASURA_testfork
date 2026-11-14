//! What a creature does in a fight: notice a player, call its group, chase, attack, use its skills, give
//! up and walk home, and lie dead until it respawns (`docs/52-skills.md`, "Mobs").
//!
//! The brain decides and nothing else. It reads where every combatant is and whether it lives, and it
//! answers with [`Order`]s: the fight's half (attack, skills, restore, revive) is carried out on
//! `crate::skill::engine::Engine`, the walking half on the NPC table (`crate::npc::table`). The
//! instance runs both, under its locks, once per tick (`crate::instance::Instance::step`).
//!
//! The numbers are `creatures.json`'s. None of them is measured: the live server's aggro, leash and
//! call-for-help rules are not in any capture this tree has, so each value says what it was chosen to
//! do. A creature's own numbers (health, armor, attack, skills) are its definition's combat block.

use std::collections::BTreeMap;

use super::content::Aggression;

/// The file `creatures.json` in the world pack.
pub const RULES_FILE: &str = "creatures.json";

/// The brain's numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FightRules {
    /// A creature notices a living player within this distance of it.
    pub aggro_range: f32,
    /// When a creature notices a player, every idle creature within this distance of it joins in.
    pub assist_range: f32,
    /// A creature that is this far from where it stands gives up and walks back.
    pub leash_range: f32,
    /// How often a chase asks the path finder again, in milliseconds.
    pub repath_ms: u64,
    /// How far the chased agent must have moved before a chase asks again sooner.
    pub repath_distance: f32,
    /// How often a creature tries its skills, in milliseconds.
    pub skill_ms: u64,
    /// A creature walks to within this share of its reach before it stops to attack.
    pub approach: f32,
    /// How long a dead creature lies before it stands up where it was placed; 0 is never, as on the
    /// live server, where a cleared area stays cleared until the instance ends.
    pub respawn_ms: u64,
    /// A creature this close to its home is home.
    pub home_radius: f32,
}

impl Default for FightRules {
    fn default() -> FightRules {
        FightRules {
            aggro_range: 1000.0,
            assist_range: 600.0,
            leash_range: 3000.0,
            repath_ms: 700,
            repath_distance: 120.0,
            skill_ms: 1000,
            approach: 0.8,
            respawn_ms: 0,
            home_radius: 100.0,
        }
    }
}

/// The reach of each way of fighting, from `combat.json`: what the brain walks into before it stops.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    /// A melee swing.
    pub melee: f32,
    /// A ranged swing.
    pub ranged: f32,
    /// A spell.
    pub spell: f32,
}

/// How a creature fights, which decides how close it walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// It swings in melee.
    Melee,
    /// It swings from range.
    Ranged,
    /// It has no swing, only skills.
    Caster,
}

/// One combatant as the brain sees it this tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seen {
    /// Its agent id.
    pub agent: u32,
    /// Where it stands.
    pub position: (f32, f32),
    /// Whether it lives.
    pub alive: bool,
}

/// What a creature does now. The executor ignores an order the fight refuses: a skill still
/// recharging, a target that died in between.
#[derive(Debug, Clone, PartialEq)]
pub enum Order {
    /// Put its ordinary attack on this agent.
    Attack {
        /// The player.
        target: u32,
    },
    /// Try these skills on this agent, in order, until one starts.
    UseSkills {
        /// The skills, in the order its block lists them.
        skills: Vec<u32>,
        /// The player.
        target: u32,
    },
    /// Stop its ordinary attack.
    StopAttack,
    /// Walk to this point.
    Walk {
        /// Where to.
        to: (f32, f32),
    },
    /// Stop where it stands.
    Halt,
    /// It died: a client that sees it from now on sees a corpse.
    Fall,
    /// Back to full health with nothing on it: it walked home.
    Restore,
    /// Stand up alive at its home.
    Revive {
        /// Its home.
        at: (f32, f32),
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    /// Standing, looking for players.
    Idle,
    /// Fighting this agent.
    Engaged { target: u32 },
    /// Walking back home after giving up.
    Returning,
    /// Dead since this time.
    Dead { since: u64 },
}

/// One creature's mind.
#[derive(Debug, Clone, PartialEq)]
struct Brain {
    home: (f32, f32),
    style: Style,
    skills: Vec<u32>,
    /// It stays on its placement point: no chase and no walk home.
    stationary: bool,
    aggression: Aggression,
    /// A player that damaged it while it stood aside; consumed by the next idle thought.
    provoked: Option<u32>,
    state: State,
    /// The last walk it was given: where to, and when.
    walk: Option<((f32, f32), u64)>,
    next_skill: u64,
}

/// Every creature's mind in one instance.
#[derive(Debug, Clone, Default)]
pub struct Fight {
    brains: BTreeMap<u32, Brain>,
    now: u64,
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

impl Fight {
    /// A hostile creature joins: it stands at `home`, fights in `style` and uses `skills`. `aggression`
    /// decides whether it notices a player on its own or waits to be damaged.
    pub fn add(
        &mut self,
        agent: u32,
        home: (f32, f32),
        style: Style,
        skills: Vec<u32>,
        aggression: Aggression,
    ) {
        self.brains.insert(
            agent,
            Brain {
                home,
                style,
                skills,
                stationary: false,
                aggression,
                provoked: None,
                state: State::Idle,
                walk: None,
                next_skill: 0,
            },
        );
    }

    /// A creature that never moves: it fights from where it was placed, so a chase and a walk home
    /// would both be invented. It still notices a player in `aggro_range` and still attacks in reach.
    pub fn pin(&mut self, agent: u32) {
        if let Some(brain) = self.brains.get_mut(&agent) {
            brain.stationary = true;
        }
    }

    /// A player damaged a creature: it remembers who, and the next idle thought engages that player.
    /// A body that fights on sight needs no reminder for a player in range, but it does for one that
    /// hit it from beyond `aggro_range`; a body that only fights when provoked has no other way in.
    pub fn provoke(&mut self, agent: u32, player: u32) {
        if let Some(brain) = self.brains.get_mut(&agent) {
            if brain.state == State::Idle {
                brain.provoked = Some(player);
            }
        }
    }

    /// Whether the brain drives this body now, so its own behavior must wait: a fight, a walk home or
    /// death.
    pub fn drives(&self, agent: u32) -> bool {
        self.brains
            .get(&agent)
            .is_some_and(|brain| brain.state != State::Idle)
    }

    /// The agent it fights, if it fights.
    pub fn target_of(&self, agent: u32) -> Option<u32> {
        match self.brains.get(&agent)?.state {
            State::Engaged { target } => Some(target),
            _ => None,
        }
    }

    /// The creatures the brain has, by agent id.
    pub fn agents(&self) -> Vec<u32> {
        self.brains.keys().copied().collect()
    }

    /// One tick of thought. `bodies` is every creature with a brain as the fight sees it; `players` is
    /// every player in the instance. Returns each creature's orders, creature by creature.
    pub fn think(
        &mut self,
        elapsed_ms: u64,
        rules: &FightRules,
        reach: Reach,
        bodies: &BTreeMap<u32, Seen>,
        players: &[Seen],
    ) -> Vec<(u32, Order)> {
        self.now += elapsed_ms;
        let now = self.now;
        let mut orders = Vec::new();
        let mut noticed: Vec<(u32, u32)> = Vec::new();

        let nearest = |from: (f32, f32), within: f32| -> Option<u32> {
            players
                .iter()
                .filter(|player| player.alive && distance(player.position, from) <= within)
                .min_by(|a, b| {
                    distance(a.position, from)
                        .partial_cmp(&distance(b.position, from))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|player| player.agent)
        };
        let player = |agent: u32| players.iter().find(|player| player.agent == agent);

        for (agent, brain) in self.brains.iter_mut() {
            let Some(me) = bodies.get(agent) else {
                continue;
            };
            let mut say = |order: Order| orders.push((*agent, order));

            if !me.alive {
                match brain.state {
                    State::Dead { since } => {
                        if 0 < rules.respawn_ms && rules.respawn_ms <= now - since {
                            brain.state = State::Idle;
                            brain.walk = None;
                            brain.provoked = None;
                            say(Order::Revive { at: brain.home });
                        }
                    }
                    _ => {
                        brain.state = State::Dead { since: now };
                        brain.walk = None;
                        say(Order::StopAttack);
                        say(Order::Halt);
                        say(Order::Fall);
                    }
                }
                continue;
            }
            if matches!(brain.state, State::Dead { .. }) {
                // Brought back by something else: a fresh start where it stands.
                brain.state = State::Idle;
            }

            match brain.state {
                State::Idle => {
                    // A hit is noticed wherever it came from; only the on-sight search needs a range.
                    let provoked = brain
                        .provoked
                        .take()
                        .filter(|target| player(*target).is_some_and(|them| them.alive));
                    let target = match brain.aggression {
                        Aggression::Always => {
                            provoked.or_else(|| nearest(me.position, rules.aggro_range))
                        }
                        Aggression::WhenProvoked => provoked,
                    };
                    if let Some(target) = target {
                        brain.state = State::Engaged { target };
                        noticed.push((*agent, target));
                        say(Order::Attack { target });
                    }
                }
                State::Engaged { target } => {
                    let far_from_home = rules.leash_range < distance(me.position, brain.home);
                    let attack_reach = match brain.style {
                        Style::Melee => reach.melee,
                        Style::Ranged => reach.ranged,
                        Style::Caster => reach.spell,
                    };
                    // A body that cannot walk cannot follow: it lets a player who left its reach go,
                    // rather than staying in a fight it can no longer join. The reach is the farther
                    // of its notice range and its swing, so a spit at 1,200 is answered.
                    let left_reach = brain.stationary
                        && player(target).is_some_and(|them| {
                            rules.aggro_range.max(attack_reach)
                                < distance(me.position, them.position)
                        });
                    let target = match player(target) {
                        Some(them) if them.alive && !far_from_home && !left_reach => Some(target),
                        _ if far_from_home => None,
                        // Its foe died or left: an always-hostile body takes the next player it can
                        // see; a provoked one has done its job and goes home.
                        _ => match brain.aggression {
                            Aggression::Always => nearest(me.position, rules.aggro_range),
                            Aggression::WhenProvoked => None,
                        },
                    };
                    let Some(target) = target else {
                        brain.state = if brain.stationary {
                            brain.walk = None;
                            brain.provoked = None;
                            say(Order::StopAttack);
                            say(Order::Halt);
                            say(Order::Restore);
                            State::Idle
                        } else {
                            brain.walk = None;
                            say(Order::StopAttack);
                            say(Order::Walk { to: brain.home });
                            brain.walk = Some((brain.home, now));
                            State::Returning
                        };
                        continue;
                    };
                    if brain.state != (State::Engaged { target }) {
                        brain.state = State::Engaged { target };
                        say(Order::Attack { target });
                    }
                    let there = player(target).map(|them| them.position).unwrap_or(me.position);
                    let reach = attack_reach * rules.approach;
                    if reach < distance(me.position, there) {
                        if !brain.stationary {
                            let stale = match brain.walk {
                                None => true,
                                Some((to, when)) => {
                                    rules.repath_ms <= now - when
                                        || rules.repath_distance < distance(to, there)
                                }
                            };
                            if stale {
                                brain.walk = Some((there, now));
                                say(Order::Walk { to: there });
                            }
                        }
                    } else {
                        if brain.walk.take().is_some() {
                            say(Order::Halt);
                        }
                        if !brain.skills.is_empty() && brain.next_skill <= now {
                            brain.next_skill = now + rules.skill_ms;
                            say(Order::UseSkills {
                                skills: brain.skills.clone(),
                                target,
                            });
                        }
                    }
                }
                State::Returning => {
                    if distance(me.position, brain.home) <= rules.home_radius {
                        brain.state = State::Idle;
                        brain.walk = None;
                        brain.provoked = None;
                        say(Order::Halt);
                        say(Order::Restore);
                    } else if !brain.stationary
                        && brain
                            .walk
                            .is_none_or(|(_, when)| rules.repath_ms * 4 <= now - when)
                    {
                        brain.walk = Some((brain.home, now));
                        say(Order::Walk { to: brain.home });
                    }
                }
                State::Dead { .. } => {}
            }
        }

        // The call for help: an idle creature near one that noticed a player fights the same player.
        for (caller, target) in noticed {
            let Some(at) = bodies.get(&caller).map(|body| body.position) else {
                continue;
            };
            for (agent, brain) in self.brains.iter_mut() {
                let Some(body) = bodies.get(agent) else {
                    continue;
                };
                if brain.aggression == Aggression::Always
                    && brain.state == State::Idle
                    && body.alive
                    && distance(body.position, at) <= rules.assist_range
                {
                    brain.state = State::Engaged { target };
                    orders.push((*agent, Order::Attack { target }));
                }
            }
        }
        orders
    }
}

/// `creatures.json`'s text, read.
pub fn rules_from_json(text: &str) -> Result<FightRules, String> {
    let file: serde_json::Value =
        serde_json::from_str(text).map_err(|err| format!("{RULES_FILE}: {err}"))?;
    let number = |key: &str| -> Result<f64, String> {
        file.get(key)
            .and_then(|entry| entry.get("value"))
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && 0.0 <= *value)
            .ok_or_else(|| format!("{RULES_FILE}: `{key}` is not a documented number"))
    };
    Ok(FightRules {
        aggro_range: number("aggro_range")? as f32,
        assist_range: number("assist_range")? as f32,
        leash_range: number("leash_range")? as f32,
        repath_ms: number("repath_ms")? as u64,
        repath_distance: number("repath_distance")? as f32,
        skill_ms: number("skill_ms")? as u64,
        approach: number("approach")? as f32,
        respawn_ms: (number("respawn_seconds")? * 1000.0) as u64,
        home_radius: number("home_radius")? as f32,
    })
}

/// The rules in force, read once per process from the world pack (`crate::spawns::pack`).
pub fn rules() -> Result<&'static FightRules, String> {
    static IN_FORCE: std::sync::OnceLock<Result<FightRules, String>> = std::sync::OnceLock::new();
    IN_FORCE
        .get_or_init(|| {
            let path = crate::spawns::pack().join(RULES_FILE);
            let text = std::fs::read_to_string(&path)
                .map_err(|err| format!("{} is not readable: {err}", path.display()))?;
            rules_from_json(&text)
        })
        .as_ref()
        .map_err(|err| err.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAWL: u32 = 40;
    const SHAMAN: u32 = 41;
    const FAR: u32 = 42;
    const PLAYER: u32 = 10;

    fn reach() -> Reach {
        Reach {
            melee: 144.0,
            ranged: 1248.0,
            spell: 1248.0,
        }
    }

    fn body(agent: u32, position: (f32, f32)) -> (u32, Seen) {
        (
            agent,
            Seen {
                agent,
                position,
                alive: true,
            },
        )
    }

    fn pack() -> (Fight, BTreeMap<u32, Seen>) {
        let mut fight = Fight::default();
        fight.add(GRAWL, (0.0, 0.0), Style::Melee, vec![], Aggression::Always);
        fight.add(SHAMAN, (300.0, 0.0), Style::Caster, vec![292], Aggression::Always);
        fight.add(FAR, (5000.0, 0.0), Style::Melee, vec![], Aggression::Always);
        let bodies = [
            body(GRAWL, (0.0, 0.0)),
            body(SHAMAN, (300.0, 0.0)),
            body(FAR, (5000.0, 0.0)),
        ]
        .into_iter()
        .collect();
        (fight, bodies)
    }

    fn player(position: (f32, f32)) -> Vec<Seen> {
        vec![Seen {
            agent: PLAYER,
            position,
            alive: true,
        }]
    }

    #[test]
    fn a_player_in_range_is_noticed_and_the_group_joins_but_not_a_far_creature() {
        let (mut fight, bodies) = pack();
        let rules = FightRules::default();
        let orders = fight.think(100, &rules, reach(), &bodies, &player((-800.0, 0.0)));
        assert!(orders.contains(&(GRAWL, Order::Attack { target: PLAYER })));
        assert!(orders.contains(&(SHAMAN, Order::Attack { target: PLAYER })));
        assert_eq!(fight.target_of(FAR), None);
        assert!(fight.drives(GRAWL));
    }

    #[test]
    fn a_provoked_body_ignores_a_player_until_a_hit_wakes_it() {
        let (mut fight, bodies) = pack();
        fight.add(
            GRAWL,
            (0.0, 0.0),
            Style::Melee,
            vec![],
            Aggression::WhenProvoked,
        );
        let rules = FightRules::default();
        let idle = fight.think(100, &rules, reach(), &bodies, &player((100.0, 0.0)));
        assert!(
            !idle
                .iter()
                .any(|(agent, order)| *agent == GRAWL && matches!(order, Order::Attack { .. })),
            "a body that only fights when provoked does not open on a player in range: {idle:?}"
        );
        fight.provoke(GRAWL, PLAYER);
        let woken = fight.think(200, &rules, reach(), &bodies, &player((100.0, 0.0)));
        assert!(
            woken.contains(&(GRAWL, Order::Attack { target: PLAYER })),
            "the hit that provoked it is answered: {woken:?}"
        );
    }

    #[test]
    fn a_pinned_creature_never_walks_and_lets_a_player_out_of_its_reach_go() {
        let (mut fight, bodies) = pack();
        fight.pin(GRAWL);
        let rules = FightRules::default();
        // Inside aggro but outside melee reach: it notices the player and does not chase.
        let orders = fight.think(100, &rules, reach(), &bodies, &player((-800.0, 0.0)));
        assert!(orders.contains(&(GRAWL, Order::Attack { target: PLAYER })));
        assert!(
            !orders
                .iter()
                .any(|(agent, order)| *agent == GRAWL && matches!(order, Order::Walk { .. })),
            "a pinned creature stays on its placement point: {orders:?}"
        );
        // The player leaves its aggro range: the fight is over and it stands at home.
        let orders = fight.think(100, &rules, reach(), &bodies, &player((-1500.0, 0.0)));
        assert!(orders.contains(&(GRAWL, Order::StopAttack)));
        assert!(orders.contains(&(GRAWL, Order::Restore)));
        assert!(!fight.drives(GRAWL));
    }

    #[test]
    fn a_hit_from_beyond_aggro_range_starts_the_fight() {
        let (mut fight, bodies) = pack();
        let rules = FightRules::default();
        fight.provoke(GRAWL, PLAYER);
        let orders = fight.think(100, &rules, reach(), &bodies, &player((-1500.0, 0.0)));
        assert!(
            orders.contains(&(GRAWL, Order::Attack { target: PLAYER })),
            "a body that was hit knows its attacker: {orders:?}"
        );
    }

    #[test]
    fn a_creature_chases_until_in_reach_then_stops_and_uses_its_skills() {
        let (mut fight, mut bodies) = pack();
        let rules = FightRules::default();
        fight.think(100, &rules, reach(), &bodies, &player((-600.0, 0.0)));
        let orders = fight.think(100, &rules, reach(), &bodies, &player((-600.0, 0.0)));
        assert!(orders.contains(&(GRAWL, Order::Walk { to: (-600.0, 0.0) })));
        // The shaman casts from where it stands: 900 is inside its spell reach.
        assert!(orders.contains(&(
            SHAMAN,
            Order::UseSkills {
                skills: vec![292],
                target: PLAYER
            }
        )));
        bodies.get_mut(&GRAWL).unwrap().position = (-500.0, 0.0);
        let orders = fight.think(100, &rules, reach(), &bodies, &player((-600.0, 0.0)));
        assert!(orders.contains(&(GRAWL, Order::Halt)));
    }

    #[test]
    fn a_creature_dragged_past_its_leash_walks_home_and_is_restored() {
        let (mut fight, mut bodies) = pack();
        let rules = FightRules::default();
        fight.think(100, &rules, reach(), &bodies, &player((-800.0, 0.0)));
        bodies.get_mut(&GRAWL).unwrap().position = (-3500.0, 0.0);
        let orders = fight.think(100, &rules, reach(), &bodies, &player((-3600.0, 0.0)));
        assert!(orders.contains(&(GRAWL, Order::StopAttack)));
        assert!(orders.contains(&(GRAWL, Order::Walk { to: (0.0, 0.0) })));
        bodies.get_mut(&GRAWL).unwrap().position = (10.0, 0.0);
        let orders = fight.think(100, &rules, reach(), &bodies, &player((-3600.0, 0.0)));
        assert!(orders.contains(&(GRAWL, Order::Restore)));
        assert!(!fight.drives(GRAWL));
    }

    #[test]
    fn a_dead_creature_stops_and_stands_up_after_the_respawn_time() {
        let (mut fight, mut bodies) = pack();
        let rules = FightRules {
            respawn_ms: 1000,
            ..FightRules::default()
        };
        fight.think(100, &rules, reach(), &bodies, &player((-800.0, 0.0)));
        bodies.get_mut(&GRAWL).unwrap().alive = false;
        let orders = fight.think(100, &rules, reach(), &bodies, &player((-800.0, 0.0)));
        assert!(orders.contains(&(GRAWL, Order::StopAttack)));
        let orders = fight.think(1000, &rules, reach(), &bodies, &player((-800.0, 0.0)));
        assert!(orders.contains(&(GRAWL, Order::Revive { at: (0.0, 0.0) })));
    }

    #[test]
    fn a_dead_player_is_dropped_for_the_next_one_or_home() {
        let (mut fight, bodies) = pack();
        let rules = FightRules::default();
        fight.think(100, &rules, reach(), &bodies, &player((-800.0, 0.0)));
        let dead = vec![Seen {
            agent: PLAYER,
            position: (-800.0, 0.0),
            alive: false,
        }];
        let orders = fight.think(100, &rules, reach(), &bodies, &dead);
        assert!(orders.contains(&(GRAWL, Order::StopAttack)));
        assert!(orders.contains(&(GRAWL, Order::Walk { to: (0.0, 0.0) })));
    }

    #[test]
    fn the_rules_file_reads_every_documented_number() {
        let text = include_str!("../../../../config/World/Vanilla/creatures.json");
        let rules = rules_from_json(text).expect("the committed creatures.json reads");
        assert!(0.0 < rules.aggro_range);
        assert!(rules.assist_range <= rules.aggro_range);
    }
}
