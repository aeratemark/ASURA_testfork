//! The client-facing protocol.
//!
//! The game client speaks this; ASURA does not design it, only reproduces it
//! (`docs/adr/0005-gwps-reference-only.md`). Every constant here is a copied fact from the GWPS
//! reference and is recorded in `docs/03-protocol.md`.
//!
//! The order of events on any connection, and therefore the order of the modules below:
//!
//! 1. `version` - the first message is not a normal frame. It is a bare `u32` "version" header
//!    that says what kind of connection this is: auth, game, or control. Get this wrong and every
//!    later decision is wrong.
//! 2. `handshake` - the key exchange: the client's seed, the server's seed, and the ARC4 key both
//!    sides derive from them (`arc4`). Everything after the server seed is scrambled.
//! 3. `frame` - after that, both directions are a `u16 header` and a body whose size the header
//!    alone fixes (`tables`). A size disagreement is what produces the client's opaque `007`
//!    error, so it is an error here, not a warning.
//!
//! Names are GWPS's, verbatim, so a Rust message can be compared against a C one by name. The
//! tables themselves are recorded in `docs/03-protocol.md`; nothing here regenerates them.
//!
//! Both families live in the one packet table ([`crate::packets`]); `tables` and `game_tables` are
//! that table's auth and game views. This module is the auth path: connection classification,
//! framing, the key exchange, and the login messages.
//!
//! ## Naming, because both directions live in this crate
//!
//! A name says what the function does to the bytes, so a reader never has to open it
//! (`docs/38-layer-decisions.md`, D15):
//!
//! * `decode_<thing>` - bytes to a struct: the client said something.
//! * `<thing>` alone - a struct to a [`Frame`]: the server says something. `instance_load_head`,
//!   `item_stream_create`. A builder carries no verb.
//! * `*_len`, `*_bytes`, `*_slots` - a size, never a value.
//! * `pack` / `unpack` - a word's bit fields, not a message.
//! * `is_*`, `has_*`, `can_*` - a predicate.
//! * no `get_` prefix: an accessor is named after what it returns.
//!
//! Every module's first line says which direction it owns, and a module that owns both says so.

pub mod arc4;
pub mod filecheck;
pub mod frame;
/// The game connection's opening message (GAME_CMSG_VERSION); not the game message builders,
/// which live in crate::game. Two modules were called game before D16.
pub mod game_version;
pub mod handshake;
pub mod messages;
/// A fake game client for tests and for the two `asura-auth` examples, never for the server
/// (`docs/38-layer-decisions.md`, D13). Compiled only when a consumer asks for `test-client`.
#[cfg(feature = "test-client")]
pub mod reference_client;
pub mod version;

/// The auth family's view of the one packet table ([`crate::packets`]): masks, counts, the named
/// headers, and the lookups. A re-export, not a table: `tables::` names where an auth caller reads
/// from, and the rows live in exactly one place.
pub use crate::packets::auth as tables;
/// The game family's view of the same table: the lookups and constants a world connection uses.
pub use crate::packets::game as game_tables;

pub use crate::packets::Packet as Entry;
pub use arc4::Arc4;
pub use frame::{Frame, FrameError};
pub use handshake::{ClientSeed, Established, HandshakeError};
pub use messages::{PortalAccountLogin, RequestResponse};
#[cfg(feature = "test-client")]
pub use reference_client::ReferenceClient;

/// Server -> client: where to go. GWPS's own struct, verbatim (`GameMsg.h:659`): 46 bytes, a
/// two-byte header, `host[24]`, then `map_token, region, player_token, is_explorable, map_id`.
/// The port target is GWPS's bytes, in GWPS's order - the captured live shape (52 bytes, map_id
/// before is_explorable) is a different client build's framing.
///
/// This is a *game* header, not an auth one, so the auth view (`tables`) has no row for it; the
/// two constants below are the recorded evidence that it must not be treated as one size. The
/// value itself lives in the one table ([`crate::packets::PACKETS`]).
pub const TRANSFER_GAME_SERVER_INFO: u16 =
    crate::packets::game::GAME_SMSG_TRANSFER_GAME_SERVER_INFO;
/// See [`TRANSFER_GAME_SERVER_INFO`]; the size GWPS sends and this server must send.
pub const TRANSFER_GAME_SERVER_INFO_LEN: usize = 46;

/// Server -> client: switch to this map now. `{header, 0, map_id, 0}`.
pub const MAP_UPDATE_CURRENT: u16 = crate::packets::game::GAME_SMSG_MAP_UPDATE_CURRENT;
/// In a travel redirect this is 12 bytes.
pub const MAP_UPDATE_CURRENT_LEN: usize = 12;
/// In the login batch it must be 5 bytes, or the client fails with error 007.
pub const MAP_UPDATE_CURRENT_LOGIN_LEN: usize = 5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_constants_match_the_recorded_protocol() {
        assert_eq!(TRANSFER_GAME_SERVER_INFO, 0x01A6);
        assert_eq!(MAP_UPDATE_CURRENT, 0x0099);
        assert_ne!(
            MAP_UPDATE_CURRENT_LEN, MAP_UPDATE_CURRENT_LOGIN_LEN,
            "the map-update size trap must stay visible"
        );
    }

    #[test]
    fn the_game_headers_are_not_in_the_auth_table() {
        // If someone adds the game table to the auth lookup, these assertions fail first.
        assert!(tables::expected_size(TRANSFER_GAME_SERVER_INFO).is_none());
        assert!(tables::expected_size(MAP_UPDATE_CURRENT).is_none());
    }
}
