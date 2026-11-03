//! Message layouts and framing.
//!
//! Two protocol families live here:
//!
//! * the **client protocol** - what the game client speaks. Every constant in this file was observed
//!   in GWPS and is recorded in `docs/03-protocol.md`. It is not ours to design, only to reproduce.
//! * the **internal protocol** - ours, versioned, and small by policy: state and placement go through
//!   the database, not through messages (`docs/02-architecture.md`).
//!
//! AzerothCore parallel: their `Packet`/`WorldPacket` layer with `ByteBuffer` readers. Same shape:
//! explicit little-endian writers and readers, and a fixed layout per header.

/// Frame header for the internal (service-to-service) protocol.
///
/// ```text
/// u32 magic          "ASUR"
/// u16 version        protocol version; a mismatch is fatal, never a warning
/// u16 kind           message kind
/// u32 length         body length
/// u8  body[length]
/// ```
pub const INTERNAL_MAGIC: [u8; 4] = *b"ASUR";
pub const INTERNAL_VERSION: u16 = 1;
pub const INTERNAL_HEADER_LEN: usize = 12;

/// Message kinds this project defines. Deliberately short: placement and state go through the
/// database, so only events that must not wait for a heartbeat belong here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Kind {
    /// A world server announcing itself: id, address, port, build, capacity.
    WorldHello = 1,
    /// A world server reporting liveness and load.
    Heartbeat = 2,
    /// A world server reporting that an instance is released and back in its pool.
    InstanceReleased = 3,
    /// Ask a world server to flush state and exit.
    Shutdown = 4,
}

impl Kind {
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(Kind::WorldHello),
            2 => Some(Kind::Heartbeat),
            3 => Some(Kind::InstanceReleased),
            4 => Some(Kind::Shutdown),
            _ => None,
        }
    }
}

/// A framed internal message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: Kind,
    pub body: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    TooShort,
    BadMagic,
    VersionMismatch { got: u16, expected: u16 },
    UnknownKind(u16),
    LengthMismatch { declared: usize, actual: usize },
}

impl Frame {
    pub fn new(kind: Kind, body: Vec<u8>) -> Self {
        Frame { kind, body }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(INTERNAL_HEADER_LEN + self.body.len());
        out.extend_from_slice(&INTERNAL_MAGIC);
        out.extend_from_slice(&INTERNAL_VERSION.to_le_bytes());
        out.extend_from_slice(&(self.kind as u16).to_le_bytes());
        out.extend_from_slice(&(self.body.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.body);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<(Frame, usize), FrameError> {
        if bytes.len() < INTERNAL_HEADER_LEN {
            return Err(FrameError::TooShort);
        }
        if bytes[0..4] != INTERNAL_MAGIC {
            return Err(FrameError::BadMagic);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != INTERNAL_VERSION {
            return Err(FrameError::VersionMismatch {
                got: version,
                expected: INTERNAL_VERSION,
            });
        }
        let kind_raw = u16::from_le_bytes([bytes[6], bytes[7]]);
        let kind = Kind::from_u16(kind_raw).ok_or(FrameError::UnknownKind(kind_raw))?;
        let declared = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        if bytes.len() < INTERNAL_HEADER_LEN + declared {
            return Err(FrameError::LengthMismatch {
                declared,
                actual: bytes.len() - INTERNAL_HEADER_LEN,
            });
        }
        let body = bytes[INTERNAL_HEADER_LEN..INTERNAL_HEADER_LEN + declared].to_vec();
        Ok((Frame { kind, body }, INTERNAL_HEADER_LEN + declared))
    }
}

/// The client-facing protocol: connection classification, framing, and the auth message table.
///
/// See the module documentation for the order a connection follows and for what is deliberately
/// not here yet (the game/world family).
pub mod client;

/// The game protocol: what a world server sends a client that has just been placed in a map, and the
/// table of client messages it must read. Separate from [`client`] because the two families collide on
/// header values: `0x8000 | n` names a different message in each.
pub mod game;

/// **Every packet, one table.** Both protocol families, one row per header, with the fact columns
/// (header, index, size, name, layout) and the two columns a session maintains (status, evidence).
/// The single source for what a message is: nothing else in the tree lists a header.
pub mod packets;

/// The per-message trace switch both services read (`ASURA_TRACE`).
pub mod trace;

pub use trace::Trace;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trips() {
        let frame = Frame::new(Kind::Heartbeat, vec![1, 2, 3, 4]);
        let bytes = frame.encode();
        assert_eq!(bytes.len(), INTERNAL_HEADER_LEN + 4);
        let (decoded, consumed) = Frame::decode(&bytes).expect("decodes");
        assert_eq!(decoded, frame);
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn frame_rejects_a_version_mismatch_loudly() {
        let mut bytes = Frame::new(Kind::Shutdown, vec![]).encode();
        bytes[4] = 0xFF;
        bytes[5] = 0xFF;
        assert_eq!(
            Frame::decode(&bytes),
            Err(FrameError::VersionMismatch {
                got: 0xFFFF,
                expected: INTERNAL_VERSION
            })
        );
    }

    #[test]
    fn frame_rejects_truncated_bodies() {
        let bytes = Frame::new(Kind::Heartbeat, vec![1, 2, 3]).encode();
        let truncated = &bytes[..bytes.len() - 1];
        assert!(matches!(
            Frame::decode(truncated),
            Err(FrameError::LengthMismatch { .. })
        ));
    }
}
