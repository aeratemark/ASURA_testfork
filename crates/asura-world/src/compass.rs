//! What one party member shows the others: strokes drawn on the compass, and called targets.
//!
//! Both are relays. The server keeps no drawing and no target: it checks who is asking, reads the
//! message, and repeats it to the members of the asker's party standing in the same instance, with the
//! asker's player id in front. Neither message is in an ASURA capture; the handlers follow preforged
//! (`gw_compass_wire.h`, `gw_callout.h`, `party_send` in `gw_world_simulation.h`) on this build's
//! packet table, and `docs/51-compass-and-chat.md` lists what a capture still has to confirm.

use crate::*;

/// The members of a player's party standing in its instance, the player included. A player the party
/// table does not know is a party of one.
pub fn party_present(instance: &Instance, player_id: u32) -> Vec<u32> {
    instance
        .parties
        .party_of(player_id)
        .map(|party| party.present())
        .unwrap_or_else(|| vec![player_id])
}

/// Send `frames` to a player's party in its instance; `to_self` says whether the player gets them too.
pub fn party_send(instance: &Instance, player_id: u32, frames: &[Frame], to_self: bool) {
    for member in party_present(instance, player_id) {
        if to_self || member != player_id {
            instance.send_to(member, frames);
        }
    }
}

/// The player's id as the word both relays carry, or `None` (logged) for an id that does not fit.
fn player_word(player: &Player, what: &str) -> Option<u16> {
    let word = u16::try_from(player.player_id).ok();
    if word.is_none() {
        eprintln!(
            "world: {what}: player id {} does not fit the message's word; nothing sent",
            player.player_id
        );
    }
    word
}

/// `GAME_CMSG_DRAW_MAP` (`0x802B`): one stroke segment drawn on the compass.
///
/// The rest of the party is sent `0x0091` with the stroke unchanged. The drawer is not: its client has
/// already drawn the stroke it sent. (Preforged echoes to the drawer as well; which one retail does is
/// not measured.)
pub fn handle_draw_map(frame: &Frame, player: &Player, trace: bool) -> Result<bool, GameError> {
    let Some(stroke) = game::read_draw_map(frame.body()) else {
        eprintln!(
            "world: compass: {:?} sent a stroke of {} byte(s) that is not a stroke; nothing sent",
            player.name,
            frame.body().len()
        );
        return Ok(true);
    };
    let Some(drawer) = player_word(player, "compass") else {
        return Ok(true);
    };
    if trace {
        eprintln!(
            "world: compass: player {} draws {} point(s) of stroke {}",
            player.player_id,
            stroke.points.len(),
            stroke.stroke
        );
    }
    party_send(
        &player.instance,
        player.player_id,
        &[game::compass_drawing(drawer, &stroke)],
        false,
    );
    Ok(true)
}

/// `GAME_CMSG_TARGET_CALL` (`0x8023`): the player calls something to its party.
///
/// A called target (kind 10) that is an agent of this instance - a player, an NPC or an item on the
/// ground - goes to the whole party, the caller included, as `0x0034`. The other kinds announce the
/// caller's own health, energy, experience or morale; this server models none of those values, so they
/// are logged and nothing is sent.
pub fn handle_target_call(frame: &Frame, player: &Player, trace: bool) -> Result<bool, GameError> {
    let Some((kind, argument)) = game::read_target_call(frame.body()) else {
        eprintln!(
            "world: call: {:?} sent a call of {} byte(s) that is not a call; nothing sent",
            player.name,
            frame.body().len()
        );
        return Ok(true);
    };
    if kind != game::TARGET_CALL_TARGET {
        eprintln!(
            "world: call: {:?} calls kind {kind} (argument {argument}); only a called target is \
             modelled, nothing sent",
            player.name
        );
        return Ok(true);
    }
    let instance = &player.instance;
    if !instance.has_agent(argument) && instance.ground_item(argument).is_none() {
        eprintln!(
            "world: call: {:?} calls agent {argument}, which is not in this instance; nothing sent",
            player.name
        );
        return Ok(true);
    }
    let Some(caller) = player_word(player, "call") else {
        return Ok(true);
    };
    if trace {
        eprintln!(
            "world: call: player {} calls agent {argument}",
            player.player_id
        );
    }
    party_send(
        instance,
        player.player_id,
        &[game::agent_pinged(caller, argument)],
        true,
    );
    Ok(true)
}
