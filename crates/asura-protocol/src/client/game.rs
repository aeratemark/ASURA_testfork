//! The game connection: what the client says when it connects to a world server.
//!
//! The client is sent to a world by `AUTH_SMSG_GAME_SERVER_INFO`, and its next connection opens with
//! `GAME_CMSG_VERSION` (`0xC0500`, **64 bytes**): the auth version's shape, then the tokens the auth
//! service just issued and both identities, so a world server can decide whether this connection is a
//! travel it knows about.
//!
//! ```text
//! offset  size  field              what it carries
//!      0     4  header             0xC0500 -- this is also how the connection is classified
//!      4     4  version            the client's build number
//!      8     4  h0008              observed 1, never compared by the reference
//!     12     4  map_token          the world id from GAME_SERVER_INFO
//!     16     4  map_id             the map from GAME_SERVER_INFO
//!     20     4  player_token       the random token the auth service issued for this travel
//!     24    16  account_uuid       the account, RFC 4122 little-endian
//!     40    16  character_uuid     the character, same encoding
//!     56     4  h0038              observed 0
//!     60     4  h003C              observed 0
//! ```
//!
//! Source: GWPS `proto.h` (`_GAME_CMSG_VERSION`, packed) and the reader in `AuthSrv.c`
//! (`Connection_ReadVersion`), which takes the same fields in the same order.
//!
//! **The token pair is the credential.** A world server that accepts a connection without matching
//! `map_token`/`player_token` to a transfer it was told about will happily serve anyone. GWPS compares
//! both, plus the character id, and drops the connection on a mismatch (`AuthSrv_DoPostHandshake`).
//!
//! Like the auth version message, this one is *not* a [`super::frame::Frame`]: it has a `u32` header
//! rather than a `u16`, and it arrives before the key exchange, so it is read as a bare structure.

use super::version::{GameVersionError, GAME_CMSG_VERSION_LEN};

/// How the reference pads and orders the UUID block: sixteen bytes each, no length fields.
pub const UUID_LEN: usize = 16;

/// The fields of `GAME_CMSG_VERSION`, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameVersion {
    /// The client's build number.
    pub version: u32,
    /// `h0008`. Observed as 1; the reference does not compare it, and neither does this.
    pub h0008: u32,
    /// The world the client was told to connect to, as the auth service numbered it.
    pub map_token: u32,
    /// The map it was told about. The world server's own configuration is what decides whether it can
    /// serve it.
    pub map_id: u32,
    /// The token the auth service issued for this travel.
    pub player_token: u32,
    /// The account, in the wire's little-endian UUID variant.
    pub account: [u8; UUID_LEN],
    /// The character, in the same variant.
    pub character: [u8; UUID_LEN],
    /// `h0038`. Observed as 0.
    pub h0038: u32,
    /// `h003C`. Observed as 0.
    pub h003c: u32,
}

impl GameVersion {
    /// Take a `GAME_CMSG_VERSION` off the front of `bytes`, and say how many bytes it used.
    ///
    /// `Err(BodyIncomplete)` until all 64 bytes are there, like the auth version message: the caller
    /// keeps reading rather than guessing where the key exchange begins.
    pub fn decode(bytes: &[u8]) -> Result<(GameVersion, usize), GameVersionError> {
        if bytes.len() < GAME_CMSG_VERSION_LEN {
            return Err(GameVersionError::Incomplete {
                needed: GAME_CMSG_VERSION_LEN - bytes.len(),
            });
        }
        let dword = |offset: usize| {
            u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ])
        };
        let header = dword(0);
        if header != super::version::GAME_CMSG_VERSION_HEADER {
            return Err(GameVersionError::NotAGameVersion { header });
        }

        let mut account = [0u8; UUID_LEN];
        account.copy_from_slice(&bytes[24..40]);
        let mut character = [0u8; UUID_LEN];
        character.copy_from_slice(&bytes[40..56]);

        Ok((
            GameVersion {
                version: dword(4),
                h0008: dword(8),
                map_token: dword(12),
                map_id: dword(16),
                player_token: dword(20),
                account,
                character,
                h0038: dword(56),
                h003c: dword(60),
            },
            GAME_CMSG_VERSION_LEN,
        ))
    }

    /// The 64 bytes a client sends. Only tests and the reference client need this; the service reads.
    pub fn encode(&self) -> [u8; GAME_CMSG_VERSION_LEN] {
        let mut bytes = [0u8; GAME_CMSG_VERSION_LEN];
        let words = [
            (0, super::version::GAME_CMSG_VERSION_HEADER),
            (4, self.version),
            (8, self.h0008),
            (12, self.map_token),
            (16, self.map_id),
            (20, self.player_token),
            (56, self.h0038),
            (60, self.h003c),
        ];
        for (offset, value) in words {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[24..40].copy_from_slice(&self.account);
        bytes[40..56].copy_from_slice(&self.character);
        bytes
    }

    /// Whether this connection presents the tokens a travel was given.
    ///
    /// Deliberately the whole comparison and not a helper that could be skipped: a world server that
    /// checks only the map token accepts any client that knows a world id.
    pub fn presents(&self, map_token: u32, player_token: u32) -> bool {
        self.map_token == map_token && self.player_token == player_token
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> GameVersion {
        GameVersion {
            version: 39112,
            h0008: 1,
            map_token: 8001,
            map_id: 146,
            player_token: 0xDEAD_BEEF,
            account: [0xAA; UUID_LEN],
            character: [0xBB; UUID_LEN],
            h0038: 0,
            h003c: 0,
        }
    }

    #[test]
    fn the_game_version_message_is_64_bytes_and_round_trips() {
        let version = sample();
        let bytes = version.encode();
        assert_eq!(bytes.len(), 64, "sizeof(GAME_CMSG_VERSION)");

        let (decoded, used) = GameVersion::decode(&bytes).expect("decodes");
        assert_eq!(used, 64);
        assert_eq!(decoded, version);
    }

    #[test]
    fn a_short_message_asks_for_more_rather_than_reading_garbage() {
        let bytes = sample().encode();
        for cut in [0usize, 4, 20, 63] {
            assert_eq!(
                GameVersion::decode(&bytes[..cut]),
                Err(GameVersionError::Incomplete { needed: 64 - cut }),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn an_auth_version_message_is_not_a_game_version_message() {
        // Both arrive on the same shape of socket; a world server that accepted either would serve a
        // login connection as gameplay.
        let mut bytes = [0u8; 64];
        bytes[0..4].copy_from_slice(&super::super::version::AUTH_CMSG_VERSION_HEADER.to_le_bytes());
        assert_eq!(
            GameVersion::decode(&bytes),
            Err(GameVersionError::NotAGameVersion {
                header: super::super::version::AUTH_CMSG_VERSION_HEADER
            })
        );
    }

    #[test]
    fn the_tokens_are_compared_as_a_pair() {
        let version = sample();
        assert!(version.presents(8001, 0xDEAD_BEEF));
        assert!(!version.presents(8001, 0xDEAD_BEEE), "the token matters");
        assert!(!version.presents(8002, 0xDEAD_BEEF), "the world matters");
    }

    #[test]
    fn the_field_offsets_are_the_ones_the_reference_reads() {
        // Spelled out here so a reordering mistake fails a test rather than a client. The reference
        // reads: header, version, h0008, map_token, map_id, player_token, uuid, uuid, h0038, h003C.
        let bytes = sample().encode();
        assert_eq!(&bytes[0..4], &0xC0500u32.to_le_bytes());
        assert_eq!(&bytes[4..8], &39112u32.to_le_bytes());
        assert_eq!(&bytes[12..16], &8001u32.to_le_bytes(), "map_token at 12");
        assert_eq!(&bytes[16..20], &146u32.to_le_bytes(), "map_id at 16");
        assert_eq!(
            &bytes[20..24],
            &0xDEAD_BEEFu32.to_le_bytes(),
            "player_token at 20"
        );
        assert_eq!(&bytes[24..40], &[0xAA; 16], "account uuid at 24");
        assert_eq!(&bytes[40..56], &[0xBB; 16], "character uuid at 40");
        assert_eq!(&bytes[56..64], &[0u8; 8], "h0038 and h003C at 56");
    }
}
