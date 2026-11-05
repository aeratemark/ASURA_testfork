//! The dispatch table: one header in, one domain's handler out.
//!
//! The reference's `GameSrv.c` is one switch over headers and so is this; a client cannot be told
//! about a command seam it does not speak. What this file is *not*: a home for domain logic. Each
//! arm validates, then calls into `item`, `chat`, `travel`, `creation`, `player` or `map`; the
//! only state it touches is the one the message is about.
//!
//! A header with no arm is framed and ignored - named in the trace, never guessed at.

use asura_protocol::packets::game as msg;

use crate::*;

/// Answer one client message. `Ok(false)` finishes the session.
pub async fn answer(
    stream: &mut TcpStream,
    session: &mut Session,
    frame: Frame,
    world: &World,
    player: &mut Player,
    trace: bool,
) -> Result<bool, GameError> {
    match frame.header() {
        msg::GAME_CMSG_INSTANCE_LOAD_REQUEST_ITEMS => {
            item::handle_request_items(stream, session, world, player, trace).await
        }

        msg::GAME_CMSG_INSTANCE_LOAD_REQUEST_SPAWN => {
            // The client asking to spawn is the sign its load - the exploration init included - was
            // accepted; a load that never gets here is treated as a refused init (`cartography`).
            player.walker.loaded = true;
            cartography::note_init_accepted(player.character.as_u128());
            agent::handle_request_spawn(stream, session, world, player, trace).await
        }

        msg::GAME_CMSG_INSTANCE_LOAD_REQUEST_PLAYERS => {
            player::handle_request_players(stream, session, world, player, trace).await
        }

        msg::GAME_CMSG_PING_REQUEST => {
            connection::handle_ping_request(stream, session, trace).await
        }

        msg::GAME_CMSG_MOVE_TO_COORD => {
            agent::handle_move_to_coord(stream, session, &frame, world, player, trace).await
        }

        msg::GAME_CMSG_POSITION_UPDATE => {
            let answer =
                agent::handle_position_update(stream, session, &frame, world, player, trace).await;

            // The step the report moved the agent along: the doorway test is the wall the step crosses
            // (`travel::portal_crossed`, `config/World/<pack>/.../portals.json`).
            if let Some((from, to)) = player.last_step.take() {
                travel::portal_crossed(stream, session, world, player, from, to, trace).await?;
            }
            player::walked(world, player, trace).await;
            answer
        }

        msg::GAME_CMSG_CANCEL_MOVEMENT => {
            agent::handle_cancel_movement(stream, session, &frame, world, player, trace).await
        }

        msg::GAME_CMSG_LAST_POS_BEFORE_MOVE_CANCELED => {
            let answer = agent::handle_last_position(stream, session, &frame, player, trace).await;
            // A correction the server accepted is a movement the reports lagged behind; it crosses a
            // doorway the same way a walk does.
            if let Some((from, to)) = player.last_step.take() {
                travel::portal_crossed(stream, session, world, player, from, to, trace).await?;
            }
            player::walked(world, player, trace).await;
            answer
        }

        // The client's own travel announcement: the player touched a travel object. Named rather than
        // left to the fallthrough, because the line it prints is the measurement a portal table is
        // built from - see `travel::handle_travel_init` for what it carries and what is still missing.
        msg::GAME_CMSG_TRAVEL_INIT => travel::handle_travel_init(&frame, player, trace),

        // The world map's request. The client has already decided which instance it wants; this carries
        // the decision out with the same cycle `/tp` uses.
        msg::GAME_CMSG_PARTY_TRAVEL => {
            travel::handle_party_travel(stream, session, &frame, world, player, trace).await
        }

        msg::GAME_CMSG_SEND_CHAT_MESSAGE => {
            chat::handle_chat_message(stream, session, &frame, world, player, trace).await
        }

        // The client is going away.
        msg::GAME_CMSG_DISCONNECT => Ok(false),

        msg::GAME_CMSG_CHAR_CREATION_REQUEST_PLAYER => {
            creation::handle_request_player(stream, session, player, trace).await
        }

        msg::GAME_CMSG_CHAR_CREATION_REQUEST_ARMORS => {
            creation::handle_request_armors(stream, session, trace).await
        }

        msg::GAME_CMSG_CHAR_CREATION_CHANGE_PROF => {
            creation::handle_change_profession(stream, session, &frame, player, trace).await
        }

        msg::GAME_CMSG_CHAR_CREATION_CONFIRM => {
            creation::handle_confirm(stream, session, &frame, world, player, trace).await
        }

        msg::GAME_CMSG_DROP_ITEM => item::handle_drop_item(&frame, world, player).await,

        msg::GAME_CMSG_ITEM_MOVE => {
            item::handle_item_move(stream, session, &frame, world, player, trace).await
        }

        msg::GAME_CMSG_EQUIP_ITEM => {
            item::handle_equip_item(stream, session, &frame, world, player, trace).await
        }

        msg::GAME_CMSG_UNEQUIP_ITEM => item::handle_unequip_item(),

        // The Hero panel's title checkbox: show this title, or show none (`crate::titles`).
        msg::GAME_CMSG_TITLE_DISPLAY => {
            titles::handle_display(stream, session, &frame, world, player, trace).await
        }
        msg::GAME_CMSG_TITLE_HIDE => titles::handle_hide(world, player).await,

        // The client answered one of our pings (`GAME_CMSG_PING_REPLY`, `0x8009`, 10 bytes). Nothing to
        // do: the reference discards it. Named rather than ignored, because a trace that says "ignored"
        // twenty-nine times reads like a bug.
        msg::GAME_CMSG_PING_REPLY => Ok(true),

        // `GAME_CMSG_HEARTBEAT` (`0x800C`, 2 bytes). The other keep-alive, likewise discarded.
        msg::GAME_CMSG_HEARTBEAT => Ok(true),

        // A message this world can frame but does not act on. Framing already happened, so ignoring it
        // is safe and is what the reference does for most of them. The name is in the trace when the
        // generated table has one: "0x8072 ignored" is a riddle, "GAME_CMSG_ITEM_MOVE ignored" is a
        // to-do.
        other => {
            if trace {
                let name = asura_protocol::client::game_tables::lookup(other, true)
                    .and_then(|entry| entry.name)
                    .unwrap_or("unnamed");
                eprintln!("world: {other:#06X} {name} ignored");
            }
            Ok(true)
        }
    }
}
#[cfg(test)]
mod tests {
    use asura_protocol::packets::{game as msg, lookup, Conn, Status};

    /// A rename is only safe while the constant and the catalogue row still name the same message at
    /// the same header. Every arm of [`answer`] is checked here against the row it claims to be, so a
    /// constant that drifts from the catalogue fails a test instead of quietly dispatching the wrong
    /// message.
    #[test]
    fn every_arm_names_the_message_the_catalogue_names() {
        let arms: &[(u16, &str)] = &[
            (
                msg::GAME_CMSG_INSTANCE_LOAD_REQUEST_ITEMS,
                "GAME_CMSG_INSTANCE_LOAD_REQUEST_ITEMS",
            ),
            (
                msg::GAME_CMSG_INSTANCE_LOAD_REQUEST_SPAWN,
                "GAME_CMSG_INSTANCE_LOAD_REQUEST_SPAWN",
            ),
            (
                msg::GAME_CMSG_INSTANCE_LOAD_REQUEST_PLAYERS,
                "GAME_CMSG_INSTANCE_LOAD_REQUEST_PLAYERS",
            ),
            (msg::GAME_CMSG_PING_REQUEST, "GAME_CMSG_PING_REQUEST"),
            (msg::GAME_CMSG_MOVE_TO_COORD, "GAME_CMSG_MOVE_TO_COORD"),
            (msg::GAME_CMSG_POSITION_UPDATE, "GAME_CMSG_POSITION_UPDATE"),
            (msg::GAME_CMSG_CANCEL_MOVEMENT, "GAME_CMSG_CANCEL_MOVEMENT"),
            (
                msg::GAME_CMSG_LAST_POS_BEFORE_MOVE_CANCELED,
                "GAME_CMSG_LAST_POS_BEFORE_MOVE_CANCELED",
            ),
            (msg::GAME_CMSG_TRAVEL_INIT, "GAME_CMSG_TRAVEL_INIT"),
            (msg::GAME_CMSG_PARTY_TRAVEL, "GAME_CMSG_PARTY_TRAVEL"),
            (
                msg::GAME_CMSG_SEND_CHAT_MESSAGE,
                "GAME_CMSG_SEND_CHAT_MESSAGE",
            ),
            (msg::GAME_CMSG_DISCONNECT, "GAME_CMSG_DISCONNECT"),
            (
                msg::GAME_CMSG_CHAR_CREATION_REQUEST_PLAYER,
                "GAME_CMSG_CHAR_CREATION_REQUEST_PLAYER",
            ),
            (
                msg::GAME_CMSG_CHAR_CREATION_REQUEST_ARMORS,
                "GAME_CMSG_CHAR_CREATION_REQUEST_ARMORS",
            ),
            (
                msg::GAME_CMSG_CHAR_CREATION_CHANGE_PROF,
                "GAME_CMSG_CHAR_CREATION_CHANGE_PROF",
            ),
            (
                msg::GAME_CMSG_CHAR_CREATION_CONFIRM,
                "GAME_CMSG_CHAR_CREATION_CONFIRM",
            ),
            (msg::GAME_CMSG_DROP_ITEM, "GAME_CMSG_DROP_ITEM"),
            (msg::GAME_CMSG_ITEM_MOVE, "GAME_CMSG_ITEM_MOVE"),
            (msg::GAME_CMSG_EQUIP_ITEM, "GAME_CMSG_EQUIP_ITEM"),
            (msg::GAME_CMSG_UNEQUIP_ITEM, "GAME_CMSG_UNEQUIP_ITEM"),
            (msg::GAME_CMSG_TITLE_DISPLAY, "GAME_CMSG_TITLE_DISPLAY"),
            (msg::GAME_CMSG_TITLE_HIDE, "GAME_CMSG_TITLE_HIDE"),
            (msg::GAME_CMSG_PING_REPLY, "GAME_CMSG_PING_REPLY"),
            (msg::GAME_CMSG_HEARTBEAT, "GAME_CMSG_HEARTBEAT"),
        ];
        assert_eq!(
            arms.len(),
            24,
            "an arm was added or removed without this table"
        );
        for (header, name) in arms {
            let row = lookup(Conn::Game, *header, true).unwrap_or_else(|| {
                panic!("{name} ({header:#06X}) is not a game client-to-server row")
            });
            assert_eq!(
                row.name,
                Some(*name),
                "{header:#06X} is named {:?} in the catalogue, not {name}",
                row.name
            );
            assert_ne!(row.status, Status::Unnamed, "{name} must be a named row");
        }
    }
}
