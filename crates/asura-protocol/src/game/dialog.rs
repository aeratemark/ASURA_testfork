//! An NPC's dialog: the text it says, who says it, and the choices under it, and the client's choice.
//!
//! Measured on a live server (`recordings/client/feature/movement/movement-live-20261109-200703.jsonl`,
//! seq 5171-5185 and 5260-5270): a talk is answered with one `0x0080` body, `0x0081` naming the NPC, one
//! `0x007E` per choice, then `0x009F {12, npc, 0}` (`super::vendor::talked_to`). The client answers a
//! choice with `0x803B {dialog id}` (seq 5245, 5277). The body and the choice labels are coded strings:
//! string ids and markers, not text (`docs/51-compass-and-chat.md`).

use super::Builder;
use crate::packets::game as msg;

/// The units one `0x0080` carries: its declared 122 minus one, as a body longer than that is split over
/// several (preforged `gw_npc_dialogue.h`, `open_encoded`: the client appends each part to its buffer).
pub const DIALOG_BODY_UNITS: usize = 121;
/// The units a choice's label may hold (`0x007E`'s string field).
pub const DIALOG_LABEL_UNITS: usize = 128;
/// The skill field of a choice that names no skill, as preforged sends it. The live server sent `0` on
/// one choice and `0xFFFFFFFF` on the rest of the same dialog (seq 5178, 5179); a choice in the pack says
/// which it carries.
pub const DIALOG_NO_SKILL: u32 = 0xFFFF_FFFF;

/// `GAME_SMSG_DIALOG_BODY` (`0x0080`): the text, split into as many messages as it takes. A unit that
/// would end a part as the first half of a surrogate pair moves to the next part.
pub fn dialog_body(units: &[u16]) -> Vec<crate::client::Frame> {
    let mut frames = Vec::new();
    let mut at = 0;
    while at < units.len() {
        let mut count = DIALOG_BODY_UNITS.min(units.len() - at);
        if at + count < units.len() && (0xD800..=0xDBFF).contains(&units[at + count - 1]) {
            count -= 1;
        }
        frames.push(
            Builder::new(msg::GAME_SMSG_DIALOG_BODY)
                .units(&units[at..at + count], DIALOG_BODY_UNITS + 1)
                .frame(),
        );
        at += count;
    }
    frames
}

/// `GAME_SMSG_DIALOG_SENDER` (`0x0081`): the agent saying the body, which shows it.
pub fn dialog_sender(agent_id: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_DIALOG_SENDER)
        .u32(agent_id)
        .frame()
}

/// `GAME_SMSG_DIALOG_BUTTON` (`0x007E`): one choice: its icon, its label, the id the client sends back,
/// and a skill.
pub fn dialog_button(icon: u8, label: &[u16], dialog_id: u32, skill: u32) -> crate::client::Frame {
    Builder::new(msg::GAME_SMSG_DIALOG_BUTTON)
        .u8(icon)
        .units(label, DIALOG_LABEL_UNITS)
        .u32(dialog_id)
        .u32(skill)
        .frame()
}

/// Read a `0x803B` (`GAME_CMSG_SEND_DIALOG`) body: the chosen dialog id.
pub fn read_send_dialog(body: &[u8]) -> Option<u32> {
    let bytes: [u8; 4] = body.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_body_is_one_message() {
        // seq 5176: [0x8102, 0x3E7E, 0xD7B5, 0xDBE4, 0x49F2].
        let frames = dialog_body(&[0x8102, 0x3E7E, 0xD7B5, 0xDBE4, 0x49F2]);
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].encode(),
            vec![0x80, 0x00, 5, 0, 0x02, 0x81, 0x7E, 0x3E, 0xB5, 0xD7, 0xE4, 0xDB, 0xF2, 0x49]
        );
    }

    #[test]
    fn a_long_body_is_split_and_never_inside_a_pair() {
        let mut units = vec![0x41u16; 120];
        units.push(0xD800);
        units.push(0xDC00);
        units.extend(std::iter::repeat(0x42).take(10));
        let frames = dialog_body(&units);
        assert_eq!(frames.len(), 2);
        assert_eq!(
            u16::from_le_bytes([frames[0].body()[0], frames[0].body()[1]]),
            120
        );
        assert_eq!(
            u16::from_le_bytes([frames[1].body()[0], frames[1].body()[1]]),
            12
        );
    }

    #[test]
    fn a_choice_is_icon_label_id_skill() {
        let frame = dialog_button(0x0B, &[0x8102, 0x3EB5], 0x87, DIALOG_NO_SKILL);
        assert_eq!(
            frame.encode(),
            vec![
                0x7E, 0x00, 0x0B, 2, 0, 0x02, 0x81, 0xB5, 0x3E, 0x87, 0, 0, 0, 0xFF, 0xFF, 0xFF,
                0xFF
            ]
        );
        assert_eq!(read_send_dialog(&[0x8B, 0, 0, 0]), Some(0x8B));
        assert_eq!(read_send_dialog(&[0x8B, 0, 0]), None);
    }
}
