//! The first message on every connection: what kind of connection is this?
//!
//! Before any framing, before the key exchange, the client sends a bare little-endian `u32` that
//! identifies the connection. It is **not** a [`super::frame::Frame`]: it has no two-byte header and
//! no table size, and decoding it as one would read `0x0400` as a message header. GWPS decides on
//! exactly this value and routes the socket (`AuthSrv.c`, `Connection_ReadVersion`); ASURA must
//! make the same decision or nothing later can be right.
//!
//! Source of truth for the numbers: GWPS `AuthSrv.h` (the three `*_VERSION_HEADER` macros) and
//! `proto.h` (the structs, packed). Recorded in `docs/03-protocol.md` section 2.
//!
//! One fact worth keeping: GWPS stores `version`, `h0008` and `h000C` from the auth version message
//! and **never compares them to anything**. They are not a compatibility gate. Reproducing a gate
//! the reference does not have would be inventing protocol, so this module exposes the fields for
//! the packet trace and refuses to draw conclusions from them.

/// `AUTH_CMSG_VERSION_HEADER`: the login connection.
pub const AUTH_CMSG_VERSION_HEADER: u32 = 0x000C_0400;
/// `GAME_CMSG_VERSION_HEADER`: a gameplay connection to a world server.
pub const GAME_CMSG_VERSION_HEADER: u32 = 0x000C_0500;
/// `CTRL_CMSG_VERSION_HEADER`: a server-to-server control connection.
pub const CTRL_CMSG_VERSION_HEADER: u32 = 0x000C_0FFF;

/// `sizeof(AUTH_CMSG_VERSION)`: header, version, h0008, h000C.
pub const AUTH_CMSG_VERSION_LEN: usize = 4 * 4;
/// `sizeof(GAME_CMSG_VERSION)`: the auth shape plus map/player tokens and both UUIDs.
pub const GAME_CMSG_VERSION_LEN: usize = 6 * 4 + 16 + 16 + 2 * 4;
/// `sizeof(CTRL_CMSG_VERSION)`: the header alone.
pub const CTRL_CMSG_VERSION_LEN: usize = 4;

/// What kind of connection the client opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    /// Client -> auth service: key exchange, login, character list.
    Auth,
    /// Client -> world server: a gameplay connection. Recognised, not handled here.
    Game,
    /// Server -> auth service: the control link. Recognised, not handled here.
    Ctrl,
}

impl ConnectionKind {
    /// How many bytes this kind's version message occupies on the wire, for a caller that must
    /// consume the whole thing before the key exchange starts.
    pub fn version_len(self) -> usize {
        match self {
            ConnectionKind::Auth => AUTH_CMSG_VERSION_LEN,
            ConnectionKind::Game => GAME_CMSG_VERSION_LEN,
            ConnectionKind::Ctrl => CTRL_CMSG_VERSION_LEN,
        }
    }
}

/// Why the first bytes are not (yet) a usable version message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionError {
    /// Fewer than four bytes: read more. This is normal on a fresh TCP read.
    Incomplete { needed: usize },
    /// Four bytes that name no connection type. GWPS logs the bytes and closes, because guessing
    /// what a client meant is how a port drifts.
    Unknown { header: u32 },
    /// The right header, but the message has not fully arrived yet.
    BodyIncomplete { needed: usize },
}

impl core::fmt::Display for VersionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            VersionError::Incomplete { needed } => {
                write!(f, "incomplete: {needed} bytes needed for a version header")
            }
            VersionError::Unknown { header } => {
                write!(f, "unknown connection version header {header:#010X}")
            }
            VersionError::BodyIncomplete { needed } => {
                write!(
                    f,
                    "incomplete: {needed} bytes needed for the version message"
                )
            }
        }
    }
}

impl std::error::Error for VersionError {}

/// Why a `GAME_CMSG_VERSION` could not be read.
///
/// Separate from [`VersionError`] because the two failures mean different things: an unknown header is
/// "this socket is not mine", while a short body is "keep reading". A world server has to tell them
/// apart to log the first and wait for the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameVersionError {
    /// The message has not fully arrived yet.
    Incomplete { needed: usize },
    /// The header names a connection that is not a game connection.
    NotAGameVersion { header: u32 },
}

impl core::fmt::Display for GameVersionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GameVersionError::Incomplete { needed } => {
                write!(f, "incomplete: {needed} more bytes needed")
            }
            GameVersionError::NotAGameVersion { header } => {
                write!(f, "{header:#010X} is not a game version header")
            }
        }
    }
}

impl std::error::Error for GameVersionError {}

/// Classify a connection from its first four bytes.
///
/// Returns [`VersionError::Incomplete`] until four bytes have arrived; the caller keeps reading.
pub fn classify(bytes: &[u8]) -> Result<ConnectionKind, VersionError> {
    if bytes.len() < 4 {
        return Err(VersionError::Incomplete { needed: 4 });
    }
    let header = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    match header {
        AUTH_CMSG_VERSION_HEADER => Ok(ConnectionKind::Auth),
        GAME_CMSG_VERSION_HEADER => Ok(ConnectionKind::Game),
        CTRL_CMSG_VERSION_HEADER => Ok(ConnectionKind::Ctrl),
        other => Err(VersionError::Unknown { header: other }),
    }
}

/// The auth version message, kept for the packet trace and for nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthVersion {
    pub version: u32,
    /// `h0008` in the reference. Observed as 1; never validated there, and not validated here.
    pub h0008: u32,
    /// `h000C` in the reference. Observed as 4; same caveat.
    pub h000c: u32,
}

impl AuthVersion {
    /// Take the auth version message off the front of `bytes`, and say how many bytes it used.
    pub fn decode(bytes: &[u8]) -> Result<(AuthVersion, usize), VersionError> {
        match classify(bytes)? {
            ConnectionKind::Auth => {}
            // Not an auth connection: the caller is talking to the wrong kind of client.
            _ => {
                return Err(VersionError::Unknown {
                    header: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                })
            }
        }
        if bytes.len() < AUTH_CMSG_VERSION_LEN {
            return Err(VersionError::BodyIncomplete {
                needed: AUTH_CMSG_VERSION_LEN,
            });
        }
        Ok((
            AuthVersion {
                version: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
                h0008: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
                h000c: u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
            },
            AUTH_CMSG_VERSION_LEN,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tables;

    fn le(value: u32) -> [u8; 4] {
        value.to_le_bytes()
    }

    #[test]
    fn the_three_version_headers_classify() {
        assert_eq!(
            classify(&le(AUTH_CMSG_VERSION_HEADER)),
            Ok(ConnectionKind::Auth)
        );
        assert_eq!(
            classify(&le(GAME_CMSG_VERSION_HEADER)),
            Ok(ConnectionKind::Game)
        );
        assert_eq!(
            classify(&le(CTRL_CMSG_VERSION_HEADER)),
            Ok(ConnectionKind::Ctrl)
        );
    }

    #[test]
    fn fewer_than_four_bytes_is_incomplete() {
        for cut in 0..4 {
            assert_eq!(
                classify(&AUTH_CMSG_VERSION_HEADER.to_le_bytes()[..cut]),
                Err(VersionError::Incomplete { needed: 4 }),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn anything_else_is_unknown_and_never_guessed() {
        assert_eq!(
            classify(&le(0x0000_0000)),
            Err(VersionError::Unknown { header: 0 })
        );
        assert_eq!(
            classify(&le(0xDEAD_BEEF)),
            Err(VersionError::Unknown {
                header: 0xDEAD_BEEF
            })
        );

        // A normal framed message must not classify: an all-zero-body request response starts
        // 03 00 00 00, which as a u32 is 3 -- not a version header. This is why classification has
        // to happen before framing, and why it cannot be inferred from a failed frame decode.
        let frame = super::super::Frame::zeros(tables::AUTH_SMSG_REQUEST_RESPONSE)
            .expect("known header")
            .encode();
        assert_eq!(
            classify(&frame[..4]),
            Err(VersionError::Unknown {
                header: 0x0000_0003
            })
        );
    }

    #[test]
    fn the_auth_version_message_decodes_its_three_fields() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&le(AUTH_CMSG_VERSION_HEADER));
        bytes.extend_from_slice(&le(34591)); // a plausible client build number
        bytes.extend_from_slice(&le(1));
        bytes.extend_from_slice(&le(4));

        let (version, used) = AuthVersion::decode(&bytes).expect("decodes");
        assert_eq!(used, AUTH_CMSG_VERSION_LEN);
        assert_eq!(version.version, 34591);
        assert_eq!(version.h0008, 1);
        assert_eq!(version.h000c, 4);
    }

    #[test]
    fn a_partial_auth_version_message_is_incomplete() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&le(AUTH_CMSG_VERSION_HEADER));
        bytes.extend_from_slice(&le(1));
        assert_eq!(
            AuthVersion::decode(&bytes),
            Err(VersionError::BodyIncomplete {
                needed: AUTH_CMSG_VERSION_LEN
            })
        );
    }

    #[test]
    fn decoding_a_game_version_as_an_auth_version_is_refused() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&le(GAME_CMSG_VERSION_HEADER));
        bytes.resize(GAME_CMSG_VERSION_LEN, 0);
        assert_eq!(
            AuthVersion::decode(&bytes),
            Err(VersionError::Unknown {
                header: GAME_CMSG_VERSION_HEADER
            })
        );
    }

    #[test]
    fn the_version_lengths_match_the_packed_reference_structs() {
        // proto.h, pragma pack 1.
        assert_eq!(AUTH_CMSG_VERSION_LEN, 16);
        assert_eq!(GAME_CMSG_VERSION_LEN, 64);
        assert_eq!(CTRL_CMSG_VERSION_LEN, 4);
        assert_eq!(ConnectionKind::Auth.version_len(), 16);
        assert_eq!(ConnectionKind::Game.version_len(), 64);
        assert_eq!(ConnectionKind::Ctrl.version_len(), 4);
    }
}
