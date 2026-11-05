//! Message framing: a `u16` header and a fixed-size body that the header alone implies.
//!
//! **There is no length field.** This was verified in the reference rather than assumed:
//! `pack_msg` (`msgpack.c`) writes the format's fields and nothing else, and
//! `AuthConnection_SendMessage` sends exactly `sizeof(struct)` bytes of them, so the size of an
//! auth message is a property of its header via [`tables`] and is never transmitted. GWPS's receive
//! path reads the header, looks the format up, and lets `unpack_msg` consume the fields
//! (`AuthSrv.c`, `AuthSrv_GetMessages`).
//!
//! `docs/03-protocol.md` section 1 described a `u16 length` here. That was wrong, and it is the
//! kind of wrong that costs a session: a port that writes a length the client does not expect gets
//! nothing back but error `007`. The document now records the correction and the evidence for it.
//!
//! Consequences that shape this module:
//!
//! * **Decoding cannot see a wrong size**, because no size is on the wire. The risks are a header
//!   that is not in the table (refused, never guessed) and a body that is still arriving
//!   ([`FrameError::Incomplete`], which is a signal to read more, not a violation).
//! * **Encoding can.** [`Frame::encode_validated`] is the second half of the port's safety: a body
//!   built at the wrong size is indistinguishable from a working one until the client reports
//!   `007`, so it is refused here instead.
//! * **Buffering lives elsewhere.** [`Frame::decode`] takes bytes and returns a frame plus how many
//!   it used; the socket owner keeps the buffer.

use super::game_tables;
use super::tables;

/// The header is two bytes. Everything after it is the body, whose length the table fixes.
pub const HEADER_LEN: usize = 2;

/// One message, already validated against the table: **its bytes, header first.**
///
/// There is no "fields plus a header that gets prepended". The header is the message's first two
/// bytes, written by whoever builds the message, and no function adds it afterwards. That matters
/// beyond tidiness: a body plus an encode step is two representations of one message, so every
/// builder, capture comparison and test has to remember that one more header is coming, and the day
/// someone forgets, a two-byte difference reads as a second protocol (`docs/03-protocol.md` 5e).
/// With one representation there is nothing to remember: what a builder writes is what the wire
/// carries, and a capture compares from byte zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The whole message: `[header u16][fields...]`, exactly as it goes on the wire.
    bytes: Vec<u8>,
}

/// Why a byte stream is not (yet) a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Fewer bytes than the frame needs. Not a protocol violation: read more.
    ///
    /// `needed` is the whole frame size once the header has been read, or [`HEADER_LEN`] while
    /// even the header is incomplete (GWPS's `ERR_NOT_ENOUGH_DATA` / `ERR_WOULDBLOCK` distinction
    /// collapses into this one signal, because both mean "wait").
    Incomplete {
        /// The whole frame size once the header is read, or the header size while it is not.
        needed: usize,
    },
    /// A header that is not in the auth table. GWPS logs the bytes and closes the connection
    /// rather than guess; so does this.
    UnknownHeader {
        /// The header that is in no auth-table row.
        header: u16,
    },
    /// The body is not the size this header requires.
    ///
    /// Only [`Frame::encode_validated`] can raise this: a received frame's size came from the
    /// table, so it cannot disagree with it. On the sending side this is the `007` trap.
    SizeMismatch {
        /// The header whose size was checked.
        header: u16,
        /// The body size actually built.
        got: usize,
        /// The size the table requires for that header.
        expected: usize,
    },
}

impl core::fmt::Display for FrameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FrameError::Incomplete { needed } => write!(f, "incomplete: {needed} bytes needed"),
            FrameError::UnknownHeader { header } => {
                write!(f, "unknown auth header {header:#06X}")
            }
            FrameError::SizeMismatch {
                header,
                got,
                expected,
            } => write!(
                f,
                "header {header:#06X} built at {got} bytes, the table says {expected}"
            ),
        }
    }
}

impl std::error::Error for FrameError {}

impl Frame {
    /// A message from a header and its fields: the header is written here, once, as byte 0 and 1.
    pub fn new(header: u16, fields: Vec<u8>) -> Self {
        let mut bytes = Vec::with_capacity(HEADER_LEN + fields.len());
        bytes.extend_from_slice(&header.to_le_bytes());
        bytes.extend_from_slice(&fields);
        Frame { bytes }
    }

    /// The header, read from the message.
    pub fn header(&self) -> u16 {
        u16::from_le_bytes([self.bytes[0], self.bytes[1]])
    }

    /// The fields, read from the message: a view, never a copy that could be re-headed.
    pub fn body(&self) -> &[u8] {
        &self.bytes[HEADER_LEN..]
    }

    /// The fields for a builder to write into, inside the message itself.
    pub fn body_mut(&mut self) -> &mut [u8] {
        &mut self.bytes[HEADER_LEN..]
    }

    /// The whole message: what goes on the wire, and what a capture holds.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Take ownership of the message bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// A correctly sized, all-zero message for a header - GWPS's `AuthConnection_BuildMsg`.
    ///
    /// The header is written here, once. Every field is then written by the caller, inside the same
    /// message: starting from the table's size is what keeps a partially filled message from being
    /// the wrong length.
    pub fn zeros(header: u16) -> Result<Self, FrameError> {
        let expected = tables::expected_size(header).ok_or(FrameError::UnknownHeader { header })?;
        let mut bytes = vec![0u8; expected];
        bytes[..HEADER_LEN].copy_from_slice(&header.to_le_bytes());
        Ok(Frame { bytes })
    }

    /// The whole on-wire size of this message.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the message carries no fields at all.
    pub fn is_empty(&self) -> bool {
        self.body().is_empty()
    }

    /// Whether this header is client-to-server, which the `0x8000` direction bit decides.
    pub fn is_from_client(&self) -> bool {
        self.header() & tables::AUTH_CMSG_MASK != 0
    }

    /// The size the table requires for this header, if the header is known.
    pub fn expected_size(&self) -> Option<usize> {
        tables::expected_size(self.header())
    }

    /// Whether this message's size is the one the table requires.
    ///
    /// An unknown header is *not* expected: silence here would be how a mistyped constant reaches a
    /// client and comes back as `007`.
    pub fn size_is_expected(&self) -> bool {
        self.expected_size() == Some(self.len())
    }

    /// The bytes to put on the wire: the message itself. Nothing is added to it.
    pub fn encode(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    /// The same, refusing a message whose size the table does not expect.
    ///
    /// This is the path server code should use for the messages it constructs. The sizes in the table
    /// are not suggestions; a five-byte map update in the login batch is indistinguishable from a
    /// working one until the client reports `007` (`docs/03-protocol.md`, section 6).
    pub fn encode_validated(&self) -> Result<Vec<u8>, FrameError> {
        match self.expected_size() {
            None => Err(FrameError::UnknownHeader {
                header: self.header(),
            }),
            Some(expected) if expected != self.len() => Err(FrameError::SizeMismatch {
                header: self.header(),
                got: self.len(),
                expected,
            }),
            Some(_) => Ok(self.bytes.clone()),
        }
    }
    /// Take one message off the front of `bytes`, and say how many bytes it used.
    ///
    /// The header decides the size, so there are only three outcomes: unknown header (refuse), not
    /// enough bytes yet (wait), or a message. Note the consequence: a *fragment* that happens to start
    /// mid-stream is framed as whatever its first two bytes name, so a caller must only ever pass a
    /// buffer whose start it knows is a message boundary. The connection does that by keeping its
    /// buffer between reads.
    ///
    /// This is the **server-to-client** rule: the reference packs each message to its format's full
    /// width, so those are fixed. Client-to-server messages are not; see
    /// [`Frame::decode_client_message`].
    pub fn decode(bytes: &[u8]) -> Result<(Frame, usize), FrameError> {
        if bytes.len() < HEADER_LEN {
            return Err(FrameError::Incomplete { needed: HEADER_LEN });
        }

        let header = u16::from_le_bytes([bytes[0], bytes[1]]);
        let expected = tables::expected_size(header).ok_or(FrameError::UnknownHeader { header })?;

        if bytes.len() < expected {
            return Err(FrameError::Incomplete { needed: expected });
        }

        Ok((
            Frame {
                bytes: bytes[..expected].to_vec(),
            },
            expected,
        ))
    }

    /// Take one client-to-server message off the front of `bytes`, and say how many bytes it used.
    ///
    /// **A client message is not a fixed size.** The reference's `unpack_msg` walks the message's field
    /// list, and for every counted field (`TYPE_STRING_16` and the `ARRAY_*` kinds) it reads a **u16**
    /// count from the wire and consumes exactly that many units - so a message is as long as its own
    /// contents, and the table's `max_size` is an upper bound, not a size. Measured against a real
    /// client: its `AUTH_CMSG_PORTAL_ACCOUNT_LOGIN` is **54 bytes**, not the table's 150, because the
    /// character name is empty and the type string is six units. Waiting for 150 is exactly how this
    /// service made a real client hang at "Connecting".
    ///
    /// A count larger than its field's capacity is refused, as the reference refuses it
    /// (`ERR_BAD_USER_DATA`), rather than reading past the field.
    pub fn decode_client_message(bytes: &[u8]) -> Result<(Frame, usize), FrameError> {
        if bytes.len() < HEADER_LEN {
            return Err(FrameError::Incomplete { needed: HEADER_LEN });
        }
        let header = u16::from_le_bytes([bytes[0], bytes[1]]);
        let entry = tables::entry(header).ok_or(FrameError::UnknownHeader { header })?;
        let shapes: Vec<Shape> = entry.fields.iter().map(shape_of).collect();
        let position = walk(bytes, header, &shapes)?;
        Ok((
            Frame {
                bytes: bytes[..position].to_vec(),
            },
            position,
        ))
    }

    /// Take one game message off the front of `bytes`, and say how many bytes it used.
    ///
    /// The game family follows the same rules as the auth family and needs the same walker, but the
    /// one table is keyed by family because the two collide on header values: `0x8000 | n` is a
    /// different message in each (`GAME_CMSG_TRADE_ACKNOWLEDGE` and `AUTH_CMSG_HEARTBEAT` both arrive
    /// as `0x8000`). Hence `from_client`: the caller knows which side of the conversation it is on,
    /// and the table cannot.
    ///
    /// Like [`Frame::decode_client_message`], this is the **structural** rule rather than the table's
    /// maximum: a counted field is a `u16` count and that many units, with no padding.
    pub fn decode_game_message(
        bytes: &[u8],
        from_client: bool,
    ) -> Result<(Frame, usize), FrameError> {
        if bytes.len() < HEADER_LEN {
            return Err(FrameError::Incomplete { needed: HEADER_LEN });
        }
        let header = u16::from_le_bytes([bytes[0], bytes[1]]);
        let fields =
            game_tables::fields(header, from_client).ok_or(FrameError::UnknownHeader { header })?;
        let shapes: Vec<Shape> = fields.iter().map(shape_of).collect();
        let position = walk(bytes, header, &shapes)?;
        Ok((
            Frame {
                bytes: bytes[..position].to_vec(),
            },
            position,
        ))
    }
}

/// How one field occupies the wire, for [`walk`].
///
/// Both families describe their fields with one type ([`crate::packets::Field`]), so there is one
/// mapping from a field to its shape rather than one per table.
#[derive(Clone, Copy)]
enum Shape {
    /// The message header: already consumed before the walk starts.
    Header,
    /// A fixed number of bytes.
    Fixed(usize),
    /// A `u16` count and that many units, up to the field's capacity.
    Counted { unit: usize, capacity: u16 },
    /// A nested struct: a `u8` count and that many repeats of the fields that follow it. Measured, not
    /// assumed: the reference's `GAME_SMSG_PVP_ITEM_STREAM_ADD` with three parameters is 17 bytes
    /// (`2 header + 2 item id + 1 count + 3 * 4`), which is exactly what this shape produces.
    Nested { capacity: u8 },
}

fn shape_of(field: &crate::packets::Field) -> Shape {
    use crate::packets::FieldKind;
    match field.kind {
        FieldKind::MsgHeader => Shape::Header,
        // A four-byte game header: the walker has already consumed the first two bytes as the header
        // (HEADER_LEN), so the field covers the two that remain. Counting four here over-reads by two
        // and desynchronises the stream -- a latent trap until the first row actually used this kind.
        FieldKind::MsgHeaderGame => Shape::Fixed(HEADER_LEN),
        FieldKind::AgentId | FieldKind::Float | FieldKind::Dword => Shape::Fixed(4),
        FieldKind::Vect2 => Shape::Fixed(8),
        FieldKind::Vect3 => Shape::Fixed(12),
        FieldKind::Byte => Shape::Fixed(1),
        FieldKind::Word => Shape::Fixed(2),
        FieldKind::Blob => Shape::Fixed(field.length as usize),
        FieldKind::String16 | FieldKind::Array16 => Shape::Counted {
            unit: 2,
            capacity: field.length,
        },
        FieldKind::Array8 => Shape::Counted {
            unit: 1,
            capacity: field.length,
        },
        FieldKind::Array32 => Shape::Counted {
            unit: 4,
            capacity: field.length,
        },
        FieldKind::NestedStruct => Shape::Nested {
            capacity: field.length.min(u8::MAX as u16) as u8,
        },
    }
}

/// Walk a message's field layout and return where its body ends.
///
/// The rule this implements, for both families: a counted field carries a `u16` count and consumes
/// exactly that many units, so the table's `max_size` is an upper bound and never a length. A nested
/// struct is the same idea one level down - a `u8` count and that many repeats of the fields after it.
fn walk(bytes: &[u8], header: u16, shapes: &[Shape]) -> Result<usize, FrameError> {
    walk_from(bytes, header, HEADER_LEN, shapes)
}

fn walk_from(
    bytes: &[u8],
    header: u16,
    mut position: usize,
    shapes: &[Shape],
) -> Result<usize, FrameError> {
    let mut index = 0;
    while index < shapes.len() {
        match shapes[index] {
            Shape::Header => {}
            Shape::Fixed(width) => {
                position += width;
                if position > bytes.len() {
                    return Err(FrameError::Incomplete { needed: position });
                }
            }
            Shape::Counted { unit, capacity } => {
                if position + 2 > bytes.len() {
                    return Err(FrameError::Incomplete {
                        needed: position + 2,
                    });
                }
                let count = u16::from_le_bytes([bytes[position], bytes[position + 1]]) as usize;
                if count > capacity as usize {
                    return Err(FrameError::SizeMismatch {
                        header,
                        got: count,
                        expected: capacity as usize,
                    });
                }
                position += 2 + count * unit;
                if position > bytes.len() {
                    return Err(FrameError::Incomplete { needed: position });
                }
            }
            Shape::Nested { capacity } => {
                let Some(&count) = bytes.get(position) else {
                    return Err(FrameError::Incomplete {
                        needed: position + 1,
                    });
                };
                if count > capacity {
                    return Err(FrameError::SizeMismatch {
                        header,
                        got: count as usize,
                        expected: capacity as usize,
                    });
                }
                position += 1;
                // Every repeat is the tail of the layout, so the tail is consumed here and the loop
                // ends: there is nothing left to walk after the repeats.
                let tail = &shapes[index + 1..];
                for _ in 0..count {
                    position = walk_from(bytes, header, position, tail)?;
                }
                return Ok(position);
            }
        }
        index += 1;
    }
    Ok(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEARTBEAT_SMSG: u16 = tables::AUTH_SMSG_HEARTBEAT; // 6 bytes: header + u32 tick
    const HEARTBEAT_CMSG: u16 = tables::AUTH_CMSG_HEARTBEAT; // also 6 bytes, other direction
    const REQUEST_RESPONSE: u16 = tables::AUTH_SMSG_REQUEST_RESPONSE; // 10: header + 2 x u32

    #[test]
    fn a_frame_round_trips() {
        let frame = Frame::zeros(HEARTBEAT_SMSG).expect("known header");
        assert_eq!(frame.len(), 6);
        assert_eq!(frame.body().len(), 4);

        let bytes = frame.encode();
        assert_eq!(bytes.len(), 6, "no length field: header plus body only");
        assert_eq!(&bytes[..2], &[0x00, 0x00]);

        let (decoded, used) = Frame::decode(&bytes).expect("decodes");
        assert_eq!(decoded, frame);
        assert_eq!(used, 6);
    }

    #[test]
    fn the_wire_carries_no_length_field() {
        // A request response is header, req_id, status -- 10 bytes, with nothing between the header
        // and the payload. If a length prefix ever appears here, this test is where it shows up.
        let mut frame = Frame::zeros(REQUEST_RESPONSE).expect("known header");
        frame.body_mut()[..4].copy_from_slice(&7u32.to_le_bytes()); // req_id
        frame.body_mut()[4..].copy_from_slice(&0u32.to_le_bytes()); // status

        assert_eq!(
            frame.encode(),
            vec![0x03, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn the_direction_bit_is_the_mask() {
        assert!(Frame::new(HEARTBEAT_CMSG, vec![0; 4]).is_from_client());
        assert!(!Frame::new(HEARTBEAT_SMSG, vec![0; 4]).is_from_client());
    }

    #[test]
    fn a_short_buffer_is_incomplete_rather_than_an_error() {
        let bytes = Frame::zeros(REQUEST_RESPONSE)
            .expect("known header")
            .encode();
        assert_eq!(
            Frame::decode(&bytes[..0]),
            Err(FrameError::Incomplete { needed: HEADER_LEN })
        );
        assert_eq!(
            Frame::decode(&bytes[..1]),
            Err(FrameError::Incomplete { needed: HEADER_LEN })
        );
        for cut in HEADER_LEN..bytes.len() {
            assert_eq!(
                Frame::decode(&bytes[..cut]),
                Err(FrameError::Incomplete {
                    needed: bytes.len()
                }),
                "cut at {cut}"
            );
        }
        // The last cut is the whole frame, which must succeed.
        assert!(Frame::decode(&bytes).is_ok());
    }

    #[test]
    fn decoding_consumes_exactly_one_frame_and_leaves_the_rest() {
        let first = Frame::zeros(HEARTBEAT_SMSG).expect("known header").encode();
        let second = Frame::zeros(REQUEST_RESPONSE)
            .expect("known header")
            .encode();
        let mut stream = first.clone();
        stream.extend_from_slice(&second);

        let (decoded, used) = Frame::decode(&stream).expect("first frame");
        assert_eq!(used, first.len());
        assert_eq!(decoded.header(), HEARTBEAT_SMSG);

        let (decoded, used) = Frame::decode(&stream[used..]).expect("second frame");
        assert_eq!(used, second.len());
        assert_eq!(decoded.header(), REQUEST_RESPONSE);
    }

    #[test]
    fn an_unknown_header_is_refused() {
        // A game header is not an auth header, and pretending otherwise is how a port drifts.
        let bytes = vec![
            super::super::MAP_UPDATE_CURRENT as u8,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
        ];
        assert_eq!(
            Frame::decode(&bytes),
            Err(FrameError::UnknownHeader {
                header: super::super::MAP_UPDATE_CURRENT
            })
        );
    }

    #[test]
    fn encode_validated_refuses_a_body_of_the_wrong_size() {
        // The 007 trap, caught before a client can see it.
        let short = Frame::new(HEARTBEAT_SMSG, vec![0; 2]);
        assert_eq!(
            short.encode_validated(),
            Err(FrameError::SizeMismatch {
                header: HEARTBEAT_SMSG,
                got: 4,
                expected: 6
            })
        );

        let right = Frame::zeros(HEARTBEAT_SMSG).expect("known header");
        assert_eq!(right.encode_validated(), Ok(right.encode()));

        let unknown = Frame::new(super::super::MAP_UPDATE_CURRENT, vec![]);
        assert_eq!(
            unknown.encode_validated(),
            Err(FrameError::UnknownHeader {
                header: super::super::MAP_UPDATE_CURRENT
            })
        );
    }

    #[test]
    fn every_row_of_the_table_frames_at_exactly_its_recorded_size() {
        for entry in tables::rows() {
            let frame = Frame::zeros(entry.header).unwrap_or_else(|err| {
                panic!("{:#06X} is missing from the lookup: {err}", entry.header)
            });
            assert_eq!(frame.len(), entry.size, "{:#06X} size", entry.header);
            assert_eq!(
                frame.is_from_client(),
                entry.from_client,
                "{:#06X} direction",
                entry.header
            );
            assert_eq!(tables::expected_size(entry.header), Some(entry.size));

            let bytes = frame.encode_validated().expect("table size encodes");
            assert_eq!(bytes.len(), entry.size);
            let (decoded, used) = Frame::decode(&bytes).expect("decodes");
            assert_eq!(decoded, frame);
            assert_eq!(used, entry.size);

            // One byte short must be Incomplete, never a wrong-size acceptance.
            assert_eq!(
                Frame::decode(&bytes[..entry.size - 1]),
                Err(FrameError::Incomplete { needed: entry.size })
            );
        }
    }

    #[test]
    fn every_row_computes_to_the_size_the_reference_declares() {
        // The generated table carries each message's field layout beside its recorded size, so the
        // size is *provable* here instead of trusted: the widths come from pack_helper in
        // msgpack.c. A number that drifted, or a generator that misread a column, fails.
        for entry in tables::rows() {
            let variable = tables::VARIABLE_LENGTH_HEADERS.contains(&entry.header);
            let mut known = 0usize;
            let mut has_variable = false;
            for field in entry.fields {
                match field.wire_size() {
                    Some(width) => known += width,
                    None => has_variable = true,
                }
            }

            assert_eq!(
                has_variable, variable,
                "{:#06X}: the table and the layout disagree about variability",
                entry.header
            );

            if variable {
                // A nested struct's own length depends on a count carried on the wire, so the
                // declared maximum is carried verbatim. It must at least cover the fixed fields
                // plus the nested struct's one-byte count.
                assert!(
                    entry.size > known,
                    "{:#06X}: declared {} does not exceed its fixed fields ({known})",
                    entry.header,
                    entry.size
                );
            } else {
                assert_eq!(
                    entry.size,
                    known,
                    "{:#06X} ({}): declared size is not the sum of its fields",
                    entry.header,
                    entry.name.unwrap_or("unnamed")
                );
            }
        }
    }

    #[test]
    fn the_sizes_that_the_login_path_depends_on_are_pinned() {
        // A second, independent check on the handful of messages M3 actually sends and receives:
        // each number is the field sum written out longhand, so a generator bug cannot hide behind
        // the layout it produced. Computed from msgdefs.c's field lists, never from sizeof(struct)
        // -- see the next test for why that distinction matters.
        let cases: &[(&str, u16, usize)] = &[
            // header + server_salt, and two more dwords the struct has but the format does not
            (
                "AUTH_SMSG_SESSION_INFO",
                tables::AUTH_SMSG_SESSION_INFO,
                2 + 4 + 4,
            ),
            // header + req_id + status
            (
                "AUTH_SMSG_REQUEST_RESPONSE",
                tables::AUTH_SMSG_REQUEST_RESPONSE,
                2 + 4 + 4,
            ),
            // header + req_id + uuid + unk0 + (4 + 20*2) + (4 + 64)
            (
                "AUTH_SMSG_CHARACTER_INFO",
                tables::AUTH_SMSG_CHARACTER_INFO,
                2 + 4 + 16 + 4 + 44 + 68,
            ),
            // header + req_id + map_token + map_id + host[24] + player_token
            (
                "AUTH_SMSG_GAME_SERVER_INFO",
                tables::AUTH_SMSG_GAME_SERVER_INFO,
                2 + 4 + 4 + 4 + 24 + 4,
            ),
            // header + req_id + 3 dwords + 8 + 8 + 16 + 16 + dword + (4 + 200) + byte + dword
            (
                "AUTH_SMSG_ACCOUNT_INFO",
                tables::AUTH_SMSG_ACCOUNT_INFO,
                2 + 4 + 4 + 4 + 8 + 8 + 16 + 16 + 4 + 204 + 1 + 4,
            ),
            // header + req_id + (4 + 2048)
            (
                "AUTH_SMSG_ACCOUNT_SETTINGS",
                tables::AUTH_SMSG_ACCOUNT_SETTINGS,
                2 + 4 + 4 + 2048,
            ),
            // header + req_id + user_id + session_id + (4 + 40) + (4 + 64)
            (
                "AUTH_CMSG_PORTAL_ACCOUNT_LOGIN",
                tables::AUTH_CMSG_PORTAL_ACCOUNT_LOGIN,
                2 + 4 + 16 + 16 + 44 + 68,
            ),
            // header + value
            (
                "AUTH_CMSG_ACCEPT_EULA",
                tables::AUTH_CMSG_ACCEPT_EULA,
                2 + 1,
            ),
            // header + req_id + map_type + map_id + region + district + language
            (
                "AUTH_CMSG_REQUEST_GAME_INSTANCE",
                tables::AUTH_CMSG_REQUEST_GAME_INSTANCE,
                2 + 4 * 6,
            ),
        ];

        for (name, header, expected) in cases {
            assert_eq!(
                tables::expected_size(*header),
                Some(*expected),
                "{name} ({header:#06X})"
            );
        }
    }

    #[test]
    fn the_wire_size_comes_from_the_format_and_not_from_the_struct() {
        // The trap that this port nearly fell into: AuthSrv_SessionInfo is a 14-byte packed struct
        // (header + three dwords) but AUTH_SMSG_0001 declares only two dwords, and pack_msg writes
        // the format, not the struct. Taking sizeof(struct) as the wire size would put two extra
        // bytes on the wire and desynchronise every later message on the connection.
        let entry = tables::entry(tables::AUTH_SMSG_SESSION_INFO).expect("present");
        let dwords = entry
            .fields
            .iter()
            .filter(|field| field.kind == tables::FieldKind::Dword)
            .count();
        assert_eq!(entry.size, 2 + 4 * dwords);
        assert_eq!(
            dwords, 2,
            "the format has two dwords where the struct has three"
        );
        assert_ne!(
            entry.size, 14,
            "14 is sizeof(AuthSrv_SessionInfo); it is not what goes on the wire"
        );
    }

    #[test]
    fn the_two_heartbeats_are_different_messages_with_the_same_size() {
        // The mask is the direction, so index 0 exists in both directions and one table serves both.
        assert_eq!(HEARTBEAT_SMSG, 0x0000);
        assert_eq!(HEARTBEAT_CMSG, 0x8000);
        assert_eq!(tables::expected_size(HEARTBEAT_SMSG), Some(6));
        assert_eq!(tables::expected_size(HEARTBEAT_CMSG), Some(6));
    }
}
