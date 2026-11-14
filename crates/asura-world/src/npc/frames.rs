//! An NPC message as bytes: each [`Msg`] through the wire crate's builder for it.

use super::burst::Msg;
use crate::{game, Frame};

/// Several messages' frames, in order.
pub fn frames(msgs: &[Msg]) -> Vec<Frame> {
    let mut out = Vec::with_capacity(msgs.len());
    for msg in msgs {
        let frame = match msg {
            Msg::Definition(definition) => game::npc_update_properties(&game::NpcProperties {
                definition_index: definition.definition_index,
                file_id: definition.file_id,
                h000a: definition.h000a,
                visual: definition.visual,
                h0012: definition.h0012,
                flags: definition.flags,
                primary: definition.primary,
                level: definition.level,
                name: &definition.name,
            }),
            Msg::Model(definition) => {
                game::npc_update_model(definition.definition_index, &definition.model_files)
            }
            Msg::Scale { agent_id, scale } => game::agent_update_scale(*agent_id, *scale),
            Msg::Name { agent_id, units } => game::agent_update_npc_name(*agent_id, units),
            Msg::IntProperty {
                id,
                agent_id,
                value,
            } => game::agent_property_update_int(*id, *agent_id, *value),
            Msg::FloatProperty {
                id,
                agent_id,
                value,
            } => game::agent_property_update_float(*id, *agent_id, *value),
            Msg::InitialEffects { agent_id, effects } => {
                game::agent_initial_effects(*agent_id, *effects)
            }
            Msg::Status { agent_id, status } => game::agent_update_effects(*agent_id, *status),
            Msg::Flags { agent_id, flags } => game::agent_update_flags(*agent_id, *flags),
            Msg::Create(create) => game::world_create_agent(&game::CreateAgent {
                agent_id: create.agent_id,
                model_id: game::CHAR_CLASS_NPC_BASE | create.definition_index,
                position: create.position,
                plane: create.plane,
                direction: create.direction,
                speed_base: create.speed,
                player_team_token: create.team,
                h000b: create.h000b,
                h0023: create.h0023,
                ..game::CreateAgent::LIVING
            }),
            Msg::Item(item) => game::create_named_item(item),
            Msg::Weapons {
                agent_id,
                lead,
                offhand,
            } => game::npc_update_weapons(*agent_id, *lead, *offhand),
            Msg::Pace { agent_id, speed } => {
                out.push(game::agent_update_speed_base(*agent_id, *speed));
                // The 11-byte form a player's walk start sends (`crate::agent`): factor 1.0, start mode.
                game::agent_update_speed(*agent_id, 1.0, game::AGENT_SPEED_MODE_START as u8)
            }
            Msg::MoveTo {
                agent_id,
                to,
                plane,
                current_plane,
            } => game::agent_move_to_point(*agent_id, *to, *plane, *current_plane),
            Msg::Stop { agent_id } => game::agent_stop_moving(*agent_id),
            Msg::Remove { agent_id } => game::world_remove_agent(*agent_id),
        };
        out.push(frame);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::burst::entry;
    use super::super::content::tests::{definition, placed, placement};
    use super::super::content::{PlacementRecord, Property, PropertyValue};
    use super::*;
    use crate::agent::AgentState;

    #[test]
    fn a_burst_becomes_the_measured_headers_at_the_tables_sizes() {
        // Seq 246-252: agent 0x2B, definition 0x5D2, unarmed, property 11 = 5 after its weapons.
        let record = PlacementRecord {
            properties: vec![Property {
                id: 11,
                value: PropertyValue::Int(5),
            }],
            ..placement("148-05d2-1", "npc-05d2", (9647.0, 8965.0))
        };
        let body = AgentState::new(0x2B, record.position, 0, record.direction, 250.0);
        let built = frames(&entry(
            &placed(record, definition("npc-05d2", 0x5D2)),
            &body,
            false,
        ));
        let shape: Vec<(u16, usize)> = built
            .iter()
            .map(|frame| (frame.header(), frame.encode().len()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (0x56, 38),
                (0x57, 12),
                (0x9B, 16),
                (0xF0, 10),
                (0x20, 99),
                (0x6D, 14),
                (0x9F, 14)
            ]
        );
    }

    #[test]
    fn a_pace_is_the_speed_pair() {
        let built = frames(&[Msg::Pace {
            agent_id: 9,
            speed: 220.0,
        }]);
        let headers: Vec<u16> = built.iter().map(|frame| frame.header()).collect();
        assert_eq!(headers, vec![0x27, 0x2B]);
        assert_eq!(
            (built[0].encode().len(), built[1].encode().len()),
            (10, 11),
            "the packet table's sizes for 0x0027 and 0x002B"
        );
    }
}
