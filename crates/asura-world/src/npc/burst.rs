//! What a client is told about an NPC, as data: the messages in the order the live server sends them.
//!
//! [`Msg`] is one message with its fields; `crate::npc::frames` turns it into a frame through the wire
//! crate's builders. Keeping the order here, apart from the bytes, is what lets the order be tested
//! without a socket.
//!
//! **The entry burst** is measured on all 18 NPC creates of
//! `recordings/client/feature/maps/pre-searing-load-3.jsonl` (seq 229-298, 384-395, 512-519, 573-586):
//!
//! ```text
//! [0x0056 [0x0057]]  the definition, the first time this connection needs it; 0x0057 only with model files
//! [0x009A]           the agent's scale word, when the placement has one
//! [0x009B]           the agent's own name, when the placement has one
//! [0x009F 36]        the agent's level, when the placement has one
//! 0x00F0             initial effects
//! 0x0020             the create
//! [0x0161 ...]       the items the body holds, when it is armed
//! 0x006D             weapons (no weapon: both item ids 0)
//! [0x009F | 0x00A2]  the placement's properties, in order
//! ```
//!
//! A second body of a definition the connection already has starts at `0x009A`/`0x009B` (seq 265,
//! 320, 380, 581). The definitions are not sent as a block before the agents: each one goes out
//! immediately in front of the first agent that needs it, in the agent array's own order.

use super::content::{NpcDefinition, NpcPlacement, PropertyValue, HELD_ITEMS};
use crate::agent::AgentState;
use crate::game::{self, NamedItem};
use std::sync::Arc;

/// The first item id a held item may take. The client indexes its item table by the id (`ctx+0xB8`,
/// bound `ctx+0xC0`), so it must stay small: the live load's ids reach 13404. This base sits above the
/// ids this server gives a character's own items.
const HELD_ITEM_BASE: u32 = 0x1000;

/// The item id one held item is given on the wire: a slot of [`HELD_ITEMS`] ids per agent id, so two
/// bodies never share one and a body gets the same ids for every viewer.
pub fn held_item_id(agent_id: u32, index: usize) -> u32 {
    HELD_ITEM_BASE + (agent_id & 0x7FF) * HELD_ITEMS as u32 + index as u32
}

/// The create of one NPC body, field for field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Create {
    /// The agent id.
    pub agent_id: u32,
    /// The definition's index; the wire's model word is the NPC class over it.
    pub definition_index: u32,
    /// The definition's `h000b`.
    pub h000b: u8,
    /// Where the body stands now.
    pub position: (f32, f32),
    /// Its plane.
    pub plane: u16,
    /// Its facing.
    pub direction: (f32, f32),
    /// The definition's speed.
    pub speed: f32,
    /// The placement's `h0023`.
    pub h0023: f32,
    /// The definition's team code.
    pub team: u32,
}

/// One message about an NPC.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// `0x0056` `NPC_UPDATE_PROPERTIES`.
    Definition(Arc<NpcDefinition>),
    /// `0x0057` `NPC_UPDATE_MODEL`.
    Model(Arc<NpcDefinition>),
    /// `0x009A` `AGENT_UPDATE_SCALE`.
    Scale { agent_id: u32, scale: u32 },
    /// `0x009B` `AGENT_UPDATE_NPC_NAME`.
    Name { agent_id: u32, units: Vec<u16> },
    /// `0x009F` `AGENT_PROPERTY_UPDATE_INT`.
    IntProperty { id: u32, agent_id: u32, value: u32 },
    /// `0x00A2` `AGENT_PROPERTY_UPDATE_FLOAT`.
    FloatProperty { id: u32, agent_id: u32, value: f32 },
    /// `0x00F0` `AGENT_INITIAL_EFFECTS`.
    InitialEffects { agent_id: u32, effects: u32 },
    /// `0x00F1` `AGENT_UPDATE_EFFECTS`: the status word, 0 surfaced, 0x1000 in a transition.
    Status { agent_id: u32, status: u32 },
    /// `0x0026` `AGENT_UPDATE_FLAGS`: the agent's flags byte (9 on a burrow re-create).
    Flags { agent_id: u32, flags: u8 },
    /// `0x0020` `WORLD_CREATE_AGENT`.
    Create(Create),
    /// `0x0161` `CREATE_NAMED_ITEM`: one item the body holds, between the create and `0x006D`.
    Item(NamedItem),
    /// `0x006D` `NPC_UPDATE_WEAPONS`.
    Weapons {
        agent_id: u32,
        lead: u32,
        offhand: u32,
    },
    /// `0x0027` `AGENT_UPDATE_SPEED_BASE` then `0x002B` `AGENT_UPDATE_SPEED`: the speed a body with a
    /// `walk_speed` walks at, in front of each of its legs.
    Pace { agent_id: u32, speed: f32 },
    /// `0x0029` `AGENT_MOVE_TO_POINT`: walk to a point (the client stops there by itself).
    MoveTo {
        agent_id: u32,
        to: (f32, f32),
        plane: u16,
        current_plane: u16,
    },
    /// `0x0028` `AGENT_STOP_MOVING`: a walk interrupted before its point.
    Stop { agent_id: u32 },
    /// `0x0021` `WORLD_REMOVE_AGENT`: the body leaves this client's view.
    Remove { agent_id: u32 },
}

/// The entry burst for one body, in the measured order. `definition_known` is whether this connection
/// has already been sent the body's definition.
/// The body is created where it stands now, not where its placement put it.
pub fn entry(placement: &NpcPlacement, body: &AgentState, definition_known: bool) -> Vec<Msg> {
    entry_with_status(placement, body, definition_known, placement.effects)
}

/// [`entry`] with a chosen initial status word: a burrowing body is created already in its transition
/// (`BURROW_TRANSITION`), not with the placement's own effects.
pub fn entry_with_status(
    placement: &NpcPlacement,
    body: &AgentState,
    definition_known: bool,
    status: u32,
) -> Vec<Msg> {
    let agent_id = body.agent_id;
    let definition = &placement.definition;
    let mut burst = Vec::with_capacity(8 + placement.properties.len());
    if !definition_known {
        burst.push(Msg::Definition(Arc::clone(definition)));
        if !definition.model_files.is_empty() {
            burst.push(Msg::Model(Arc::clone(definition)));
        }
    }
    if let Some(scale) = placement.scale {
        burst.push(Msg::Scale { agent_id, scale });
    }
    if let Some(units) = &placement.name {
        burst.push(Msg::Name {
            agent_id,
            units: units.clone(),
        });
    }
    if let Some(level) = placement.level {
        burst.push(Msg::IntProperty {
            id: game::AGENT_PROPERTY_PUBLIC_LEVEL,
            agent_id,
            value: level,
        });
    }
    burst.push(Msg::InitialEffects {
        agent_id,
        effects: status,
    });
    burst.push(Msg::Create(Create {
        agent_id,
        definition_index: definition.definition_index,
        h000b: definition.h000b,
        position: body.position,
        plane: body.plane,
        direction: body.facing,
        speed: definition.speed,
        h0023: placement.h0023,
        team: definition.team.0,
    }));
    for (index, item) in placement.items.iter().enumerate() {
        burst.push(Msg::Item(NamedItem {
            id: held_item_id(agent_id, index),
            ..item.clone()
        }));
    }
    let weapon = |hand: usize| {
        placement.weapons[hand]
            .map(|index| held_item_id(agent_id, index))
            .unwrap_or(0)
    };
    burst.push(Msg::Weapons {
        agent_id,
        lead: weapon(0),
        offhand: weapon(1),
    });
    for property in &placement.properties {
        burst.push(match property.value {
            PropertyValue::Int(value) => Msg::IntProperty {
                id: property.id,
                agent_id,
                value,
            },
            PropertyValue::Float(value) => Msg::FloatProperty {
                id: property.id,
                agent_id,
                value,
            },
        });
    }
    // The client computes a damage number from the damage fraction it is sent, multiplied by the
    // target's maximum health (`docs/36` E182). A creature whose maximum was never sent shows every
    // hit as 0, so a body with a combat block gets its 42 and a full 34 with its burst.
    if let Some(combat) = &definition.combat {
        burst.push(Msg::IntProperty {
            id: game::AGENT_PROPERTY_HEALTH,
            agent_id,
            value: combat.health,
        });
        burst.push(Msg::FloatProperty {
            id: asura_protocol::game::skill::PROPERTY_HEALTH_FRACTION,
            agent_id,
            value: 1.0,
        });
    }
    burst
}

/// The status bit a burrow's emerge and submerge animations carry (`0x00F0`/`0x00F1`), measured at
/// exactly 2.00 s over 373 intervals in Rurik's live corpus (`docs/59-plague-worm-burrow.md`).
pub const BURROW_TRANSITION: u32 = 0x1000;

/// A burrowing body's re-create, the five-message burst Rurik measured 140 times: `0x009F[66, agent,
/// 0]`, `0x00F0[agent, 0x1000]`, `0x0020`, `0x006D`, `0x0026[agent, 9]`. No definition, name, scale or
/// level: the client kept them across the remove.
pub fn burrow_recreate(placement: &NpcPlacement, body: &AgentState) -> Vec<Msg> {
    let agent_id = body.agent_id;
    let definition = &placement.definition;
    let weapon = |hand: usize| {
        placement.weapons[hand]
            .map(|index| held_item_id(agent_id, index))
            .unwrap_or(0)
    };
    vec![
        Msg::IntProperty {
            id: 66,
            agent_id,
            value: 0,
        },
        Msg::InitialEffects {
            agent_id,
            effects: BURROW_TRANSITION,
        },
        Msg::Create(Create {
            agent_id,
            definition_index: definition.definition_index,
            h000b: definition.h000b,
            position: body.position,
            plane: body.plane,
            direction: body.facing,
            speed: definition.speed,
            h0023: placement.h0023,
            team: definition.team.0,
        }),
        Msg::Weapons {
            agent_id,
            lead: weapon(0),
            offhand: weapon(1),
        },
        Msg::Flags {
            agent_id,
            flags: 9,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::super::content::tests::{definition, placed, placement};
    use super::super::content::{NpcCombat, PlacementRecord, Property};
    use super::*;
    fn person() -> PlacementRecord {
        PlacementRecord {
            plane: 26,
            properties: vec![Property {
                id: 11,
                value: PropertyValue::Int(5),
            }],
            ..placement("a", "guard", (11715.0, 3517.0))
        }
    }

    /// The burst for `record` as a body of `definition` standing where it was placed.
    fn burst(agent_id: u32, record: PlacementRecord, of: NpcDefinition, known: bool) -> Vec<Msg> {
        let body = AgentState::new(
            agent_id,
            record.position,
            record.plane,
            record.direction,
            of.speed,
        );
        entry(&placed(record, of), &body, known)
    }

    fn guard() -> NpcDefinition {
        definition("guard", 0x5B3)
    }

    fn headers(burst: &[Msg]) -> Vec<u16> {
        crate::npc::frames::frames(burst)
            .iter()
            .map(|frame| frame.header())
            .collect()
    }

    #[test]
    fn an_armed_body_declares_its_items_before_its_weapons() {
        let mut body = person();
        body.items = vec![
            NamedItem {
                file_id: 9451,
                item_type: 27,
                flags: 0x2200_0140,
                model_id: 415,
                quantity: 1,
                ..NamedItem::default()
            },
            NamedItem {
                file_id: 14372,
                item_type: 24,
                flags: 0x2000_0200,
                model_id: 343,
                quantity: 1,
                ..NamedItem::default()
            },
        ];
        body.weapons = [Some(0), Some(1)];
        let burst = burst(0x29, body, guard(), true);
        assert_eq!(
            headers(&burst),
            vec![0x9B, 0xF0, 0x20, 0x161, 0x161, 0x6D, 0x9F],
            "the items sit between the create and 0x006D"
        );
        let ids: Vec<u32> = burst
            .iter()
            .filter_map(|msg| match msg {
                Msg::Item(item) => Some(item.id),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec![held_item_id(0x29, 0), held_item_id(0x29, 1)]);
        assert_eq!(
            burst[5],
            Msg::Weapons {
                agent_id: 0x29,
                lead: ids[0],
                offhand: ids[1]
            }
        );
    }

    #[test]
    fn a_foe_publishes_its_maximum_health_and_full_fraction() {
        let mut foe = guard();
        foe.combat = Some(NpcCombat {
            hostile: true,
            stationary: false,
            aggression: crate::npc::content::Aggression::Always,
            health: 480,
            armor: 20,
            energy: 0,
            energy_regen: 0,
            rank: 0,
            profession: 1,
            attack: None,
            skills: Vec::new(),
        });
        let burst = burst(0x29, person(), foe, true);
        assert_eq!(
            headers(&burst),
            vec![0x9B, 0xF0, 0x20, 0x6D, 0x9F, 0x9F, 0xA2],
            "the maximum then the full fraction, after the placement's own properties"
        );
        let max = burst.iter().find_map(|msg| match msg {
            Msg::IntProperty {
                id: 42, value, ..
            } => Some(*value),
            _ => None,
        });
        let fraction = burst.iter().find_map(|msg| match msg {
            Msg::FloatProperty {
                id: 34, value, ..
            } => Some(*value),
            _ => None,
        });
        assert_eq!(max, Some(480));
        assert_eq!(fraction, Some(1.0));
    }

    #[test]
    fn a_person_enters_as_seq_246_to_252() {
        let burst = burst(0x2B, person(), guard(), false);
        assert_eq!(
            headers(&burst),
            vec![0x56, 0x57, 0x9B, 0xF0, 0x20, 0x6D, 0x9F]
        );
        let Msg::Create(create) = burst[4] else {
            panic!("the fifth message is the create")
        };
        assert_eq!(create.definition_index, 0x5B3);
        assert_eq!(create.team, 0x6E6F_6E63);
        assert_eq!((create.h000b, create.h0023, create.speed), (9, 1.0, 250.0));
    }

    #[test]
    fn a_second_body_of_a_known_definition_starts_at_its_name_as_seq_265() {
        let burst = burst(0x34, person(), guard(), true);
        assert_eq!(headers(&burst), vec![0x9B, 0xF0, 0x20, 0x6D, 0x9F]);
    }

    #[test]
    fn an_animal_enters_as_seq_573_to_580() {
        let mut body = person();
        body.name = None;
        body.scale = Some(0x2800_0000);
        body.level = Some(1);
        body.properties = vec![Property {
            id: 44,
            value: PropertyValue::Float(0.2),
        }];
        let burst = burst(0x196, body, guard(), false);
        assert_eq!(
            headers(&burst),
            vec![0x56, 0x57, 0x9A, 0x9F, 0xF0, 0x20, 0x6D, 0xA2]
        );
        assert_eq!(
            burst[3],
            Msg::IntProperty {
                id: 36,
                agent_id: 0x196,
                value: 1
            }
        );
    }

    #[test]
    fn a_definition_without_model_files_sends_no_model_as_seq_294() {
        let mut bare = guard();
        bare.model_files.clear();
        let burst = burst(0x3C, person(), bare, false);
        assert_eq!(headers(&burst)[..2], [0x56, 0x9B]);
    }
}
