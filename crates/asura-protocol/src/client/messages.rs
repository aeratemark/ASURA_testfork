//! The messages the login path actually exchanges, as typed values instead of offsets.
//!
//! Every layout here is copied from GWPS `AuthMsg.h` (packed structs) and checked against the size
//! table: a body of the wrong length cannot be constructed through these functions, because
//! [`super::frame::Frame::encode_validated`] refuses it.
//!
//! Only what the login path needs is here. The rest of the 100 auth messages arrive with the code
//! that needs them, for the reason `docs/adr/0007-rust-translation.md` gives: an unused message
//! layout is a fact nobody has verified.
//!
//! **UUIDs travel in the RFC 4122 little-endian variant** - the first three fields are little
//! endian, the clock sequence and node are not (`uuid_enc_le` in GWPS `GmUuid.h`). That is what
//! `uuid::Uuid::to_bytes_le`/`from_bytes_le` do, and it is *not* the same as the canonical byte
//! order: reading it the obvious way swaps the halves of every id, so every login would report a
//! pair that does not resolve. This module keeps UUIDs as raw `[u8; 16]` and leaves the conversion
//! to the caller, so the choice is made once, where the database is read.

use super::frame::Frame;
use super::tables;

/// The status field of an `AUTH_SMSG_REQUEST_RESPONSE`: success (`GM_ERROR_NONE`).
pub const STATUS_OK: u32 = 0;
/// The status field of a refusal: the pair did not resolve (`GM_ERROR_AUTH_ERROR`, GWPS `GmErrors.h`).
pub const STATUS_AUTH_ERROR: u32 = 11;
/// `GM_ERROR_NETWORK_ERROR`: what the reference answers when it cannot place or read something.
pub const STATUS_NETWORK_ERROR: u32 = 2;
/// `GM_ERROR_INVALID_CHARNAME`: the name the client sent is not one of the account's characters
/// (`AuthSrv_HandleDeleteCharacter`).
pub const STATUS_INVALID_CHARNAME: u32 = 23;

/// Why a message body could not be read or built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageError {
    /// The frame carries a different header than this message expects.
    UnexpectedHeader {
        /// The header the frame carries.
        got: u16,
        /// The header this message must carry.
        expected: u16,
    },
    /// The body ended early: the reference's layout needs more bytes.
    Truncated {
        /// Which field ran out of bytes.
        field: &'static str,
    },
    /// A counted string or array was longer than the field that must hold it.
    TooLong {
        /// Which field overflowed.
        field: &'static str,
        /// How many units the field holds.
        capacity: usize,
        /// How many were written or read.
        got: usize,
    },
}

impl core::fmt::Display for MessageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MessageError::UnexpectedHeader { got, expected } => {
                write!(f, "message has header {got:#06X}, expected {expected:#06X}")
            }
            MessageError::Truncated { field } => write!(f, "body ends inside `{field}`"),
            MessageError::TooLong {
                field,
                capacity,
                got,
            } => write!(f, "`{field}` holds at most {capacity}, got {got}"),
        }
    }
}

impl std::error::Error for MessageError {}

/// A cursor over a message body. Every read moves forward; there is no seeking, so a layout cannot
/// be read in a different order from the one it is written in.
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, position: 0 }
    }

    fn take(&mut self, count: usize, field: &'static str) -> Result<&'a [u8], MessageError> {
        let end = self.position + count;
        if end > self.bytes.len() {
            return Err(MessageError::Truncated { field });
        }
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn u32(&mut self, field: &'static str) -> Result<u32, MessageError> {
        let bytes = self.take(4, field)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// A `u16` count followed by that many UTF-16 units - the shape the client sends for every counted
    /// string (see [`Frame::decode_client_message`]). `Ok(None)` when the bytes have not all arrived;
    /// a count larger than the field's capacity is an error, as the reference treats it.
    fn u16_utf16(
        &mut self,
        capacity: usize,
        field: &'static str,
    ) -> Result<Option<String>, MessageError> {
        let start = self.position;
        let count_bytes = match self.bytes.get(start..start + 2) {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        let count = u16::from_le_bytes([count_bytes[0], count_bytes[1]]) as usize;
        if count > capacity {
            return Err(MessageError::TooLong {
                field,
                capacity,
                got: count,
            });
        }
        let end = start + 2 + count * 2;
        if end > self.bytes.len() {
            return Ok(None);
        }
        let units: Vec<u16> = (0..count)
            .map(|index| {
                let at = start + 2 + index * 2;
                u16::from_le_bytes([self.bytes[at], self.bytes[at + 1]])
            })
            .collect();
        self.position = end;
        Ok(Some(
            char::decode_utf16(units)
                .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect(),
        ))
    }

    fn array16(&mut self, count: usize, field: &'static str) -> Result<[u8; 16], MessageError> {
        let bytes = self.take(count, field)?;
        let mut out = [0u8; 16];
        out.copy_from_slice(bytes);
        Ok(out)
    }
}

/// A message body under construction.
#[derive(Debug)]
struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Writer { bytes: Vec::new() }
    }

    fn u32(&mut self, value: u32) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn u16(&mut self, value: u16) -> &mut Self {
        self.bytes.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn array16(&mut self, value: [u8; 16]) -> &mut Self {
        self.bytes.extend_from_slice(&value);
        self
    }

    /// A counted UTF-16 string in the **client's** form: a `u16` count and the units, with no padding.
    ///
    /// The client packs this way for everything it sends (`unpack_msg` on the reference side reads a
    /// u16 count and consumes exactly that many units), which is why a client message is shorter than
    /// the table says.
    fn u16_utf16(
        &mut self,
        text: &str,
        capacity: usize,
        field: &'static str,
    ) -> Result<&mut Self, MessageError> {
        let units: Vec<u16> = text.encode_utf16().collect();
        if units.len() > capacity {
            return Err(MessageError::TooLong {
                field,
                capacity,
                got: units.len(),
            });
        }
        self.bytes
            .extend_from_slice(&(units.len() as u16).to_le_bytes());
        for unit in &units {
            self.bytes.extend_from_slice(&unit.to_le_bytes());
        }
        Ok(self)
    }

    fn into_body(self) -> Vec<u8> {
        self.bytes
    }
}

fn expect(frame: &Frame, expected: u16) -> Result<(), MessageError> {
    if frame.header() != expected {
        return Err(MessageError::UnexpectedHeader {
            got: frame.header(),
            expected,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------- client to server

/// The client's opening preamble: who the computer is, then a hash of it.
///
/// **`0x8001` is not the size the table says.** The client sends it as `header, u16 count, UTF-16
/// units, u16 count, UTF-16 units` - variable length, no padding into the 32-slot fields the
/// reference's `AuthSrv_ComputerInfo` struct implies, and the count is two bytes rather than four.
/// Measured, not guessed: a real client (build 39112) sent this on a real connection to this service,
/// and it decodes cleanly as 42 bytes of computer info followed immediately by a 22-byte
/// `AUTH_CMSG_SEND_COMPUTER_HASH` - 64 bytes in total, which is exactly what arrived:
///
/// ```text
/// 01 80 03 00 41 00 70 00 6f 00 0f 00 44 00 45 00 53 00 4b 00 54 00 4f 00 50 00
/// 2d 00 44 00 55 00 52 00 4a 00 37 00 4e 00 30 00     "Apo" (3), "DESKTOP-DURJ7N0" (15)
/// 02 80 c8 98 00 00 f5 e6 06 66 71 a9 87 45 b8 fc 3a a6 b3 c8 55 25
///                    ^^ version 0x98c8 = 39112, the client's own build, then the hash
/// ```
///
/// So the table's 138 bytes for `0x8001` describes the reference's *struct*, not this client's wire
/// format. The service consumes this message structurally (with [`decode_computer_info`]) instead of
/// by table size: waiting for 138 bytes would consume the login message that follows and frame from
/// the middle of the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerInfo {
    /// The Windows account name the client reported.
    pub username: String,
    /// The computer's name.
    pub pc_name: String,
}

/// Consume a `0x8001` computer-info message from the front of `bytes`.
///
/// `Ok(None)` means it has not fully arrived. The number returned is how many bytes it occupied,
/// which is what the caller must discard.
pub fn decode_computer_info(bytes: &[u8]) -> Result<Option<(ComputerInfo, usize)>, MessageError> {
    if bytes.len() < 2 {
        return Ok(None);
    }
    let header = u16::from_le_bytes([bytes[0], bytes[1]]);
    if header != tables::AUTH_CMSG_SEND_COMPUTER_INFO {
        return Err(MessageError::UnexpectedHeader {
            got: header,
            expected: tables::AUTH_CMSG_SEND_COMPUTER_INFO,
        });
    }

    let mut reader = Reader::new(&bytes[2..]);
    let username = match reader.u16_utf16(32, "n_username")? {
        Some(name) => name,
        None => return Ok(None),
    };
    let pc_name = match reader.u16_utf16(32, "pcname")? {
        Some(name) => name,
        None => return Ok(None),
    };
    Ok(Some((
        ComputerInfo { username, pc_name },
        2 + reader.position,
    )))
}

/// `AUTH_CMSG_PORTAL_ACCOUNT_LOGIN` (`0x8038`): the client presents its identity pair.
///
/// Layout (`AuthSrv_PortalAccountLogin`): `req_id`, `user_id[16]`, `session_id[16]`, a counted
/// character name of up to 20 UTF-16 units, and a counted `type` string of up to 32.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalAccountLogin {
    /// Echoed back in every reply to this request.
    pub request: u32,
    /// The portal user id, in the little-endian UUID variant (see the module documentation).
    pub user_id: [u8; 16],
    /// The session id, in the little-endian UUID variant.
    pub session_id: [u8; 16],
    /// The character the client last played, empty on a first login. Informational: the pair is what
    /// authorises, and the client picks a character later.
    pub character_name: String,
    /// A client-side description of the account type. Unused; carried so the trace matches.
    pub kind: String,
}

impl PortalAccountLogin {
    /// Read the message out of a received frame.
    /// Read an answer back, which the reference client does and the server never needs.
    pub fn decode(frame: &Frame) -> Result<Self, MessageError> {
        expect(frame, tables::AUTH_CMSG_PORTAL_ACCOUNT_LOGIN)?;
        let mut reader = Reader::new(frame.body());
        let request = reader.u32("req_id")?;
        let user_id = reader.array16(16, "user_id")?;
        let session_id = reader.array16(16, "session_id")?;
        let character_name = reader
            .u16_utf16(20, "charname")?
            .ok_or(MessageError::Truncated { field: "charname" })?;
        let kind = reader
            .u16_utf16(32, "type")?
            .ok_or(MessageError::Truncated { field: "type" })?;
        Ok(PortalAccountLogin {
            request,
            user_id,
            session_id,
            character_name,
            kind,
        })
    }

    /// Build the message, which is what the reference client in `super::reference_client` does.
    ///
    /// Fallible, because the character name must fit its counted field: a builder that can refuse
    /// says so in its name. `RequestResponse::frame` cannot fail and does not carry the prefix.
    pub fn try_to_frame(&self) -> Result<Frame, MessageError> {
        let mut writer = Writer::new();
        writer
            .u32(self.request)
            .array16(self.user_id)
            .array16(self.session_id)
            .u16_utf16(&self.character_name, 20, "charname")?
            .u16_utf16(&self.kind, 32, "type")?;
        Ok(Frame::new(
            tables::AUTH_CMSG_PORTAL_ACCOUNT_LOGIN,
            writer.into_body(),
        ))
    }
}

// ---------------------------------------------------------------- server to client

/// `AUTH_SMSG_REQUEST_RESPONSE` (`0x0003`): the answer to any request, success or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestResponse {
    /// The `req_id` of the request being answered.
    pub request: u32,
    /// 0 for success; the reference's nonzero codes are the client's own error dialog.
    pub status: u32,
}

impl RequestResponse {
    /// The 10-byte answer. Infallible: both fields are fixed-width dwords.
    pub fn frame(&self) -> Frame {
        let mut writer = Writer::new();
        writer.u32(self.request).u32(self.status);
        Frame::new(tables::AUTH_SMSG_REQUEST_RESPONSE, writer.into_body())
    }

    /// Read an answer back, which the reference client does and the server never needs.
    pub fn decode(frame: &Frame) -> Result<Self, MessageError> {
        expect(frame, tables::AUTH_SMSG_REQUEST_RESPONSE)?;
        let mut reader = Reader::new(frame.body());
        Ok(RequestResponse {
            request: reader.u32("req_id")?,
            status: reader.u32("status")?,
        })
    }

    /// Whether the server accepted the request.
    pub fn is_ok(&self) -> bool {
        self.status == STATUS_OK
    }
}

/// The character-settings block the reference sends inside `AUTH_SMSG_CHARACTER_INFO`.
///
/// Copied from a live response (2026-10-26): 62 bytes of the *seeded* character -- `version = 6`,
/// `last_outpost = 146`, a Warrior's appearance, level 1, campaign Prophecies, helmet shown, and five
/// armour pieces dyed the same colour.
///
/// The blob is now **built from the character row** ([`character_settings`]) instead of sent as this
/// constant, and that is what makes this constant useful: it is the reference's own answer for one
/// character, so [`character_settings`] reproducing it byte for byte from that character's fields is the
/// test that the packing is right.
pub const CHARACTER_SETTINGS_EXTENDED: [u8; 62] = [
    0x06, 0x00, 0x92, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x03, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x11, 0x40, 0x00, 0x00,
    0x05, 0x00, 0x00, 0x00, 0x00, 0x5b, 0x00, 0x0b, 0x00, 0x00, 0x5e, 0x00, 0x0b, 0x00, 0x00, 0x5d,
    0x00, 0x0b, 0x00, 0x00, 0x5a, 0x00, 0x0b, 0x00, 0x00, 0x5c, 0x00, 0x0b, 0x00, 0x00,
];

/// `CHARACTER_SETTINGS_VERSION` (`GmChar.h`): the only version the reference ever writes.
pub const CHARACTER_SETTINGS_VERSION: u16 = 6;

/// One armour piece of the settings block: `GmChar.h`'s inline `pieces[]` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArmourPiece {
    /// The piece's `file_id`, sixteen bits of it (`GameSrv.c:1903`: `item->file_id & 0xFFFF`).
    pub file_id: u16,
    /// The four dye colours, packed as the item's `dye_colors` holds them.
    pub colors: u16,
}

/// `CharacterSettings` (`GmChar.h`), as the fields a server has to fill in.
///
/// The reference's struct is a packed bitfield mix; this keeps the same values in the same widths and
/// leaves the packing to [`character_settings`]. The fields it leaves out are the ones the reference
/// leaves zero: `last_time_played`, `h001E` (uninitialised padding in the reference) and `h0021`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CharacterSettings {
    /// The map the character was last in. `last_outpost` in the reference's row.
    pub last_outpost: u16,
    /// The packed `Appearance` word: `sex` 1 bit, `height` 4, `skin_color` 5, `hair_color` 5,
    /// `face_style` 5, `primary_profession` 4, `hair_style` 6, `race` 2.
    pub appearance: u32,
    /// `Campaign`: 0 none (pvp), 1 Prophecies, 2 Factions, 3 Nightfall, 4 Eye of the North.
    pub campaign: u8,
    /// Four bits on the wire (`LEVEL_BITS`).
    pub level: u8,
    /// Whether the character was created for PvP, where the client hides PvE-only surfaces.
    pub is_pvp: bool,
    /// Four bits on the wire.
    pub secondary_profession: u8,
    /// Two bits: 0 none, 1 show, 2 hide.
    pub helm_status: u8,
    /// The armour pieces, in the reference's order: body, legs, head, boots, gloves.
    pub pieces: Vec<ArmourPiece>,
}

/// The 62-byte settings block, packed exactly as `CharacterSettings_FromDbCharacter` (`GmChar.c`) fills
/// the struct and the compiler lays it out.
///
/// The layout, in bytes: `version` 2, `last_outpost` 2, `last_time_played` 4 (always zero), `appearance`
/// 4, the guild-hall uuid 16 (zero: nothing here creates guilds), one packed word -- `campaign` 4 bits,
/// `level` 5, `is_pvp` 1, `secondary_profession` 4, `helm_status` 2, then sixteen bits of padding the
/// reference happens to write as zero -- `number_of_pieces` 1, four more bytes the reference leaves zero,
/// and five bytes per piece: `file_id` 2, two dye nibbles, a byte that is almost always zero, two more dye
/// nibbles.
///
/// **The size never changes**: the reference sends `sizeof(CharacterSettings)` -- the whole struct, all
/// five piece slots -- so a character wearing nothing still sends 62 bytes, with `number_of_pieces` zero
/// and the slots blank. A shorter block would be a client reading the pieces that are not there.
///
/// At most five pieces are carried (`pieces[5]` in the reference); a sixth is dropped rather than
/// overflowing the field.
pub fn character_settings(settings: &CharacterSettings) -> Vec<u8> {
    /// `pieces[5]`: the array is fixed, the count is not.
    const PIECE_SLOTS: usize = 5;

    let pieces = settings.pieces.len().min(PIECE_SLOTS);
    let mut blob = Vec::with_capacity(62);

    blob.extend_from_slice(&CHARACTER_SETTINGS_VERSION.to_le_bytes());
    blob.extend_from_slice(&settings.last_outpost.to_le_bytes());
    blob.extend_from_slice(&0u32.to_le_bytes()); // last_time_played: the reference zeroes it
    blob.extend_from_slice(&settings.appearance.to_le_bytes());
    blob.extend_from_slice(&[0u8; 16]); // last_guild_hall_id

    let packed = (u32::from(settings.campaign & 0xF))
        | ((u32::from(settings.level) & 0x1F) << 4)
        | (u32::from(settings.is_pvp) << 9)
        | ((u32::from(settings.secondary_profession) & 0xF) << 10)
        | ((u32::from(settings.helm_status) & 0x3) << 14);
    blob.extend_from_slice(&packed.to_le_bytes());

    blob.push(pieces as u8);
    blob.extend_from_slice(&0u32.to_le_bytes()); // h0021: uninitialised in the reference, zero here

    for index in 0..PIECE_SLOTS {
        match settings.pieces.get(index) {
            Some(piece) => {
                blob.extend_from_slice(&piece.file_id.to_le_bytes());
                blob.push((piece.colors & 0xF) as u8 | (((piece.colors >> 4) & 0xF) as u8) << 4);
                blob.push(0); // the piece's `unk`: zero for every piece the reference sends
                blob.push(
                    (((piece.colors >> 8) & 0xF) as u8)
                        | ((((piece.colors >> 12) & 0xF) as u8) << 4),
                );
            }
            // A slot past `number_of_pieces` is present in the struct and zero in the reference's copy.
            None => blob.extend_from_slice(&[0u8; 5]),
        }
    }

    blob
}

/// `AUTH_SMSG_CHARACTER_INFO` (`0x0007`): one character in the list, sent once per character.
///
/// Layout (`AuthSrv_CharacterInfo`): `req_id`, `uuid[16]`, `unk0`, a counted name, a counted
/// `extended` block of character settings.
///
/// **Both directions of this protocol are variable-length**, measured from the reference's own output:
/// its `CHARACTER_INFO` for a five-character name and a 62-byte settings block is **102 bytes**
/// (`07 00 | req_id | uuid | unk0 | 05 00 "Appok" | 3e 00 <62 bytes>`), where the table says 138. The
/// counts are `u16` and nothing is padded to the field's capacity. A padded message is one the client
/// cannot parse, which is what it reports as error 005.
pub fn character_info(
    request: u32,
    character: [u8; 16],
    name: &str,
    settings: &[u8],
) -> Result<Frame, MessageError> {
    let mut writer = Writer::new();
    writer.u32(request).array16(character).u32(0); // unk0: the reference leaves it zero
    writer.u16_utf16(name, 20, "name")?;
    if settings.len() > 64 {
        return Err(MessageError::TooLong {
            field: "extended",
            capacity: 64,
            got: settings.len(),
        });
    }
    writer.u16(settings.len() as u16);
    writer.bytes.extend_from_slice(settings);
    Ok(Frame::new(
        tables::AUTH_SMSG_CHARACTER_INFO,
        writer.into_body(),
    ))
}

/// `AUTH_SMSG_FRIEND_STREAM_END` (`0x0014`): "the friend list has finished". GWPS sends the request id
/// in both fields, and the client needs it before the login completes.
pub fn friend_stream_end(request: u32) -> Frame {
    let mut writer = Writer::new();
    writer.u32(request).u32(request);
    Frame::new(tables::AUTH_SMSG_FRIEND_STREAM_END, writer.into_body())
}

/// `AUTH_SMSG_SESSION_INFO` (`0x0001`): the server salt. GWPS sends this in answer to the *account*
/// login, not the portal one; both fields are echoed into the log for the trace.
pub fn session_info(server_salt: u32, unknown: u32) -> Frame {
    let mut writer = Writer::new();
    writer.u32(server_salt).u32(unknown);
    Frame::new(tables::AUTH_SMSG_SESSION_INFO, writer.into_body())
}

/// `AUTH_SMSG_HEARTBEAT` (`0x0000`): the answer to a client heartbeat. The reference always sends
/// `tick = 16`; it is a constant, not a clock.
pub fn heartbeat(tick: u32) -> Frame {
    let mut writer = Writer::new();
    writer.u32(tick);
    Frame::new(tables::AUTH_SMSG_HEARTBEAT, writer.into_body())
}

/// The character name out of `AUTH_CMSG_CHANGE_PLAY_CHARACTER`: `req_id`, then a counted name of up to
/// 20 UTF-16 units. The count is a `u32` on the wire, like every counted field in the table.
pub fn decode_change_character(body: &[u8]) -> Result<String, MessageError> {
    let mut reader = Reader::new(body);
    reader.u32("req_id")?;
    reader
        .u16_utf16(20, "charname")?
        .ok_or(MessageError::Truncated { field: "charname" })
}

/// The EULA version out of `AUTH_CMSG_ACCEPT_EULA` (`0x8026`, 3 bytes): one byte, the version the
/// client accepted. Measured against build 39112: `1a` = 26, and the account info the same login had
/// answered with `18` = 24, which is the lower number that made the client show the page.
pub fn decode_eula(body: &[u8]) -> Result<u8, MessageError> {
    body.first()
        .copied()
        .ok_or(MessageError::Truncated { field: "eula" })
}

/// `AUTH_SMSG_ACCOUNT_SETTINGS` (`0x0016`): the client's stored settings.
///
/// Layout (`AuthSrv_AccountSettings`): `req_id`, a **u16** `n_data`, then exactly that many bytes of
/// data. Measured from the reference's own output: `16 00 | 01 00 00 00 | 4f 02 <591 bytes>` -
/// **599 bytes**, where the table says 2058. The blob is copied from the reference's own literal
/// (`config/Server/account_settings.json`, `asura-config`); nothing here knows what most of it means, and it
/// is sent because it is what the reference sends.
pub fn account_settings(request: u32, data: &[u8]) -> Frame {
    let mut writer = Writer::new();
    writer.u32(request).u16(data.len() as u16);
    writer.bytes.extend_from_slice(data);
    Frame::new(tables::AUTH_SMSG_ACCOUNT_SETTINGS, writer.into_body())
}

/// `AUTH_SMSG_ACCOUNT_INFO` (`0x0011`): the account, the selected character, and the account's feature
/// list.
///
/// Layout and values from GWPS `AuthSrv_SendAccountInfo`. The two 8-byte blobs are its literals:
/// `h000E` (`3F 00 00 00 00 00 00 00`) and `h0016` (`80 3F 42 00 83 00 0C 02`, commented in the
/// reference as controlling which campaigns are owned). `features` is the third (`01 00 04 00 57 00
/// 01 00`), whose third `u16` is the account's number of character slots.
///
/// `character` is the session's last selection; the reference sends all zeroes when the account has
/// no characters at all. Field widths and counts follow the whole-wire rule: u16 counts, no padding.
///
/// `eula_accepted` is the version of the EULA the account has accepted, `0` when it never has
/// (`asura_db::account`). The reference hardcodes 24 here, which is why its clients are asked every
/// login; measured on this server: 24 out, `1a` = 26 back from the client on acceptance.
pub fn account_info(
    request: u32,
    account: [u8; 16],
    character: Option<[u8; 16]>,
    eula_accepted: u8,
) -> Frame {
    /// `h000E` in `AuthSrv_SendAccountInfo`.
    const H000E: [u8; 8] = [0x3F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    /// `h0016`: which campaigns the account owns, per the reference's own comment.
    const H0016: [u8; 8] = [0x80, 0x3F, 0x42, 0x00, 0x83, 0x00, 0x0C, 0x02];
    /// `features`: the third `u16` is the number of character slots (the reference notes that two are
    /// unaccounted for).
    const FEATURES: [u8; 8] = [0x01, 0x00, 0x04, 0x00, 0x57, 0x00, 0x01, 0x00];

    let mut writer = Writer::new();
    writer.u32(request);
    writer.u32(0); // h0006
    writer.u32(4); // h000A
    writer.bytes.extend_from_slice(&H000E);
    writer.bytes.extend_from_slice(&H0016);
    writer.array16(account);
    writer.array16(character.unwrap_or([0u8; 16]));
    writer.u32(8); // h003E
    writer.u16(FEATURES.len() as u16); // n_unk4
    writer.bytes.extend_from_slice(&FEATURES);
    writer.bytes.push(eula_accepted); // eula_accepted
                                      // `unk5` **is** on the wire, and getting this wrong is what desynchronised a real client:
                                      // the `03 00` after the eula byte is the first half of this dword, not the next
                                      // message's header. Measured: the reference reply is 812 bytes in total -
                                      // 10 + 102 + 599 + 10 + 81 + 10 - and this is the 81.
    writer.u32(3); // unk5
    Frame::new(tables::AUTH_SMSG_ACCOUNT_INFO, writer.into_body())
}

// ---------------------------------------------------------------- travel

/// The size of the `host` blob in `AUTH_SMSG_GAME_SERVER_INFO`.
///
/// Bigger than an IPv4 address needs, because one struct carries either family
/// (`AuthSrv_GameServerInfo.host[24]`): 2 bytes of address family, 2 of port, then the address.
pub const GAME_SERVER_HOST_LEN: usize = 24;

/// The address family values the `host` blob starts with.
///
/// These are Winsock's constants, and they are written down here rather than taken from the host
/// platform on purpose: `AF_INET6` is 23 on Windows and 10 on Linux, and the reader of this field is a
/// Windows client, so a service that ran on Linux and sent its own platform's value would send a
/// number the client does not know. The reference has the same hazard and has only ever run on
/// Windows in this rig.
pub const ADDRESS_FAMILY_INET: u16 = 2;
/// See [`ADDRESS_FAMILY_INET`].
pub const ADDRESS_FAMILY_INET6: u16 = 23;

/// The map a client asked to travel to: `AUTH_CMSG_REQUEST_GAME_INSTANCE` (`0x8029`).
///
/// Layout (`AuthSrv_RequestGameInstance`): `req_id`, `map_type`, `map_id`, `region`, `district`,
/// `language` - six dwords after the header, 26 bytes on the wire. The table says 26 and this message
/// is fixed-size in both directions, so it is one of the few where the table is the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameInstanceRequest {
    /// The `req_id` this request answers.
    pub request: u32,
    /// 0 for an outpost, 1 for an explorable area (`MapType_FromInt`).
    pub map_type: u32,
    /// The instance the client asks for.
    pub map_id: u32,
    /// The region the client names; it is a capacity hint, never a permission.
    pub region: u32,
    /// The district the client names; 0 means "any".
    pub district: u32,
    /// The client's language id, which the row stores and nothing else reads.
    pub language: u32,
}

impl GameInstanceRequest {
    /// Read the body of an `AUTH_CMSG_REQUEST_GAME_INSTANCE`.
    pub fn decode(body: &[u8]) -> Result<Self, MessageError> {
        let mut reader = Reader::new(body);
        Ok(Self {
            request: reader.u32("req_id")?,
            map_type: reader.u32("map_type")?,
            map_id: reader.u32("map_id")?,
            region: reader.u32("region")?,
            district: reader.u32("district")?,
            language: reader.u32("language")?,
        })
    }
}

/// `AUTH_SMSG_GAME_SERVER_INFO` (`0x0009`): where the client should play, and the tokens it presents
/// there.
///
/// Layout (`AuthSrv_GameServerInfo`): `req_id`, `map_token`, `map_id`, `host[24]`, `player_token` -
/// 42 bytes. The reference fills `map_token` with the world's id and `player_token` with a random
/// number per travel, and the game server is what decides whether the pair is real.
///
/// Note that the client is told about a server, never about a *map file*: the world server owns that.
pub fn game_server_info(
    request: u32,
    map_token: u32,
    map_id: u32,
    host: [u8; GAME_SERVER_HOST_LEN],
    player_token: u32,
) -> Frame {
    let mut writer = Writer::new();
    writer.u32(request);
    writer.u32(map_token);
    writer.u32(map_id);
    writer.bytes.extend_from_slice(&host);
    writer.u32(player_token);
    Frame::new(tables::AUTH_SMSG_GAME_SERVER_INFO, writer.into_body())
}

/// The `host` blob for an IPv4 game server: family, port big-endian, four address bytes, zero padding.
///
/// `le16enc` for the family and `be16enc` for the port is the reference's own asymmetry
/// (`AuthSrv_SendGameServerInfo`), and copying it is cheaper than discovering it.
pub fn host_ipv4(address: std::net::Ipv4Addr, port: u16) -> [u8; GAME_SERVER_HOST_LEN] {
    let mut host = [0u8; GAME_SERVER_HOST_LEN];
    host[0..2].copy_from_slice(&ADDRESS_FAMILY_INET.to_le_bytes());
    host[2..4].copy_from_slice(&port.to_be_bytes());
    host[4..8].copy_from_slice(&address.octets());
    host
}

/// The `host` blob for an IPv6 game server: family, port big-endian, sixteen address bytes.
pub fn host_ipv6(address: std::net::Ipv6Addr, port: u16) -> [u8; GAME_SERVER_HOST_LEN] {
    let mut host = [0u8; GAME_SERVER_HOST_LEN];
    host[0..2].copy_from_slice(&ADDRESS_FAMILY_INET6.to_le_bytes());
    host[2..4].copy_from_slice(&port.to_be_bytes());
    host[4..20].copy_from_slice(&address.octets());
    host
}

/// The `host` blob for any address the auth service may hand out.
pub fn host_for(address: std::net::SocketAddr) -> [u8; GAME_SERVER_HOST_LEN] {
    match address {
        std::net::SocketAddr::V4(v4) => host_ipv4(*v4.ip(), v4.port()),
        std::net::SocketAddr::V6(v6) => host_ipv6(*v6.ip(), v6.port()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::frame::FrameError;

    #[test]
    fn every_server_to_client_message_is_the_size_the_table_requires() {
        // The table is the authority for what *the server* sends: the reference packs those to the
        // format's full width, so their size is fixed and a body built here that disagrees cannot be
        // sent. Client messages are the other shape and are covered by the next test.
        let response = RequestResponse {
            request: 7,
            status: STATUS_OK,
        };
        assert_eq!(response.frame().len(), 10);

        // 2 + req_id 4 + uuid 16 + unk0 4 + (2 + 10 name) + (2 + 62 settings)
        let character =
            character_info(7, [0x33; 16], "Asura", &CHARACTER_SETTINGS_EXTENDED).expect("builds");
        assert_eq!(character.len(), 102);

        // Measured from a live reference reply (`tools/inspect-reply.py`): the two messages the batch
        // carries in the middle are 599 and 77 bytes, not the table's 2058 and 275.
        let settings = account_settings(7, &[0x08; 591]);
        assert_eq!(settings.len(), 599);
        let info = account_info(7, [0x44; 16], Some([0x55; 16]), 24);
        assert_eq!(info.len(), 81);

        assert_eq!(friend_stream_end(7).len(), 10);
        assert_eq!(session_info(1, 0).len(), 10);
        assert_eq!(heartbeat(16).len(), 6);

        // Every one of them frames back out of the stream by walking its own fields.
        for frame in [
            response.frame(),
            character,
            settings,
            info,
            friend_stream_end(7),
            session_info(1, 0),
            heartbeat(16),
        ] {
            let bytes = frame.encode();
            let (decoded, used) = Frame::decode_client_message(&bytes)
                .unwrap_or_else(|err| panic!("header {:#06X}: {err}", frame.header()));
            assert_eq!(decoded, frame);
            assert_eq!(used, bytes.len());
        }
    }

    #[test]
    fn a_client_message_is_shorter_than_its_table_maximum_and_walks_its_own_layout() {
        // Measured against a real client: its login is 54 bytes where the table says 150. The table's
        // number is a maximum, and treating it as a size is how this service made a client hang.
        let login = PortalAccountLogin {
            request: 7,
            user_id: [0x11; 16],
            session_id: [0x22; 16],
            character_name: String::new(),
            kind: "Portal".to_string(),
        };
        let frame = login.try_to_frame().expect("builds");
        // header 2, req_id 4, two 16-byte ids, an empty name (2), a six-unit type (2 + 12).
        assert_eq!(frame.len(), 54);
        assert!(
            !frame.size_is_expected(),
            "a client message is not the table's size, and must not be validated as if it were"
        );
        assert!(frame.len() < tables::expected_size(frame.header()).expect("known header"));

        let bytes = frame.encode();
        let (decoded, used) = Frame::decode_client_message(&bytes).expect("walks its layout");
        assert_eq!(used, bytes.len());
        assert_eq!(decoded, frame);
        assert_eq!(
            PortalAccountLogin::decode(&decoded).expect("decodes"),
            login
        );
    }

    #[test]
    fn the_account_settings_frame_a_blob_and_pad_nothing_after_it() {
        // The count is a u16 and the data is exactly that long: measured, `16 00 | req_id | 4f 02
        // <591 bytes>` - 599 bytes total, where the table's padded maximum is 2058. The blob itself is
        // configuration (`config/Server/account_settings.json`), so this test supplies its own bytes.
        let blob = [0x08u8; 591];
        let frame = account_settings(0x1234, &blob);
        let body = &frame.body();
        assert_eq!(
            u32::from_le_bytes([body[0], body[1], body[2], body[3]]),
            0x1234
        );
        let declared = u16::from_le_bytes([body[4], body[5]]) as usize;
        assert_eq!(declared, 591, "the reference's blob is 591 bytes");
        assert_eq!(&body[6..6 + declared], &blob, "the blob is copied verbatim");
        assert_eq!(body.len(), 6 + declared, "nothing is padded after the data");
    }

    #[test]
    fn the_account_info_carries_the_account_and_the_selected_character() {
        let account = [0xAAu8; 16];
        let character = [0xBBu8; 16];
        let frame = account_info(9, account, Some(character), 26);

        assert_eq!(
            u32::from_le_bytes(frame.body()[0..4].try_into().expect("4")),
            9
        );
        // req_id, h0006, h000A, then h000E and h0016 (8 each), then the two UUIDs.
        assert_eq!(&frame.body()[12..20], &[0x3F, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            &frame.body()[20..28],
            &[0x80, 0x3F, 0x42, 0x00, 0x83, 0x00, 0x0C, 0x02]
        );
        assert_eq!(&frame.body()[28..44], &account);
        assert_eq!(&frame.body()[44..60], &character);

        // With no character selected the reference sends zeroes rather than omitting the field.
        let empty = account_info(9, account, None, 26);
        assert_eq!(empty.len(), frame.len());
        assert!(empty.body()[44..60].iter().all(|byte| *byte == 0));

        // The EULA version sits between the 8 feature bytes and `unk5`: 4 + 4 + 4 + 8 + 8 + 16 + 16
        // + 4 + 2 + 8 = 74 bytes before it, and `unk5` (a dword) is all that follows.
        assert_eq!(
            frame.body()[74],
            26,
            "the accepted version is the byte we send"
        );
        assert_eq!(&frame.body()[75..79], &[3, 0, 0, 0], "unk5 after it");
        assert_eq!(
            account_info(9, account, None, 0).body()[74],
            0,
            "a fresh account asks"
        );
    }

    #[test]
    fn a_login_message_round_trips_through_the_wire() {
        let login = PortalAccountLogin {
            request: 0xDEAD_BEEF,
            user_id: core::array::from_fn(|index| index as u8),
            session_id: core::array::from_fn(|index| (index as u8).wrapping_mul(3)),
            character_name: "Presearing Hero".to_string(),
            kind: "Gw1".to_string(),
        };

        let bytes = login.try_to_frame().expect("builds").encode();
        let (frame, used) = Frame::decode_client_message(&bytes).expect("frames");
        assert_eq!(used, bytes.len());

        let decoded = PortalAccountLogin::decode(&frame).expect("decodes");
        assert_eq!(decoded, login);
    }

    #[test]
    fn a_name_that_does_not_fit_is_refused_rather_than_truncated() {
        // A truncated name is a character nobody can log in with, and the client's only symptom
        // would be a name it does not recognise.
        let long = "x".repeat(21);
        let mut writer = Writer::new();
        assert_eq!(
            writer.u16_utf16(&long, 20, "charname").map(|_| ()),
            Err(MessageError::TooLong {
                field: "charname",
                capacity: 20,
                got: 21
            })
        );

        let login = PortalAccountLogin {
            request: 1,
            user_id: [0; 16],
            session_id: [0; 16],
            character_name: long,
            kind: String::new(),
        };
        assert!(matches!(
            login.try_to_frame(),
            Err(MessageError::TooLong {
                field: "charname",
                ..
            })
        ));
    }

    #[test]
    fn a_counted_name_occupies_only_what_it_holds() {
        // The opposite of what an earlier version of this test asserted, and the measurement is the
        // reason: the reference's own `CHARACTER_INFO` for a five-character name is 102 bytes, with the
        // name as `05 00 "Appok"` and no padding. A longer name makes a longer message.
        let short = character_info(0, [0; 16], "A", &CHARACTER_SETTINGS_EXTENDED).expect("builds");
        let long = character_info(
            0,
            [0; 16],
            "AAAAAAAAAAAAAAAAAAAA",
            &CHARACTER_SETTINGS_EXTENDED,
        )
        .expect("builds");
        assert_eq!(short.len(), 2 + 4 + 16 + 4 + 2 + 2 + 2 + 62);
        assert_eq!(long.len(), short.len() + 19 * 2);
    }

    #[test]
    fn a_body_that_ends_early_is_refused() {
        let login = PortalAccountLogin {
            request: 1,
            user_id: [0; 16],
            session_id: [0; 16],
            character_name: "A".to_string(),
            kind: String::new(),
        };
        let frame = login.try_to_frame().expect("builds");
        // One message, one representation: a body that ends early is a message built eight bytes
        // short, and it is refused because the table says what its fields must add up to.
        let short = frame.body().len() - 8;
        let mut message = frame.body().to_vec();
        message.truncate(short);
        let frame = super::super::Frame::new(frame.header(), message);
        assert!(matches!(
            PortalAccountLogin::decode(&frame),
            Err(MessageError::Truncated { .. })
        ));
    }

    #[test]
    fn the_wrong_header_is_refused() {
        let frame = friend_stream_end(1);
        assert_eq!(
            RequestResponse::decode(&frame),
            Err(MessageError::UnexpectedHeader {
                got: tables::AUTH_SMSG_FRIEND_STREAM_END,
                expected: tables::AUTH_SMSG_REQUEST_RESPONSE
            })
        );
    }

    #[test]
    fn a_count_longer_than_its_field_is_refused() {
        // A client can send any count it likes; one larger than the field is a malformed message,
        // not an instruction to read past the body.
        let mut frame = PortalAccountLogin {
            request: 1,
            user_id: [0; 16],
            session_id: [0; 16],
            character_name: "A".to_string(),
            kind: String::new(),
        }
        .try_to_frame()
        .expect("builds");

        // The charname count sits after req_id, user_id and session_id.
        frame.body_mut()[36..40].copy_from_slice(&21u32.to_le_bytes());
        assert_eq!(
            PortalAccountLogin::decode(&frame),
            Err(MessageError::TooLong {
                field: "charname",
                capacity: 20,
                got: 21
            })
        );
    }

    #[test]
    fn an_empty_character_name_is_legal() {
        // A first login presents no name at all, and that is not an error.
        let login = PortalAccountLogin {
            request: 1,
            user_id: [0; 16],
            session_id: [0; 16],
            character_name: String::new(),
            kind: String::new(),
        };
        let bytes = login.try_to_frame().expect("builds").encode();
        let (frame, _) = Frame::decode_client_message(&bytes).expect("frames");
        let decoded = PortalAccountLogin::decode(&frame).expect("decodes");
        assert_eq!(decoded.character_name, "");
    }

    /// Exactly the login message a real client (build 39112) sent to this service, captured from its
    /// trace on 2026-10-26 - the message the service was waiting 96 extra bytes for, and hanging on.
    const CAPTURED_LOGIN: &[u8] = &[
        0x38, 0x80, 0x01, 0x00, 0x00, 0x00, 0xe2, 0x0e, 0x52, 0xfa, 0x19, 0x44, 0xb4, 0x4e, 0xae,
        0x49, 0x6e, 0x9a, 0xbe, 0x6e, 0xf2, 0x4f, 0x5d, 0xbf, 0xb9, 0xd8, 0xb1, 0x90, 0xbd, 0x4c,
        0x9b, 0x76, 0x88, 0xda, 0x7b, 0xe7, 0x63, 0xb6, 0x00, 0x00, 0x06, 0x00, 0x50, 0x00, 0x6f,
        0x00, 0x72, 0x00, 0x74, 0x00, 0x61, 0x00, 0x6c, 0x00,
    ];

    #[test]
    fn the_captured_login_from_a_real_client_frames_and_decodes() {
        // 54 bytes, where the table says 150. This is the exact case that made a real client hang.
        assert_eq!(CAPTURED_LOGIN.len(), 54);

        let (frame, used) = Frame::decode_client_message(CAPTURED_LOGIN).expect("frames");
        assert_eq!(used, 54, "the whole message, and no more");
        assert_eq!(frame.header(), tables::AUTH_CMSG_PORTAL_ACCOUNT_LOGIN);
        assert_eq!(frame.body().len(), 52);

        let login = PortalAccountLogin::decode(&frame).expect("decodes");
        assert_eq!(login.request, 1);
        // The rig`s portal user, fa520ee2-4419-4eb4-ae49-6e9abe6ef24f, in the wire`s little-endian
        // UUID variant. Written as bytes so the test needs no uuid dependency.
        assert_eq!(
            login.user_id,
            [
                0xe2, 0x0e, 0x52, 0xfa, 0x19, 0x44, 0xb4, 0x4e, 0xae, 0x49, 0x6e, 0x9a, 0xbe, 0x6e,
                0xf2, 0x4f
            ],
            "the portal user the rig presents"
        );
        assert_eq!(
            login.session_id,
            [
                0x5d, 0xbf, 0xb9, 0xd8, 0xb1, 0x90, 0xbd, 0x4c, 0x9b, 0x76, 0x88, 0xda, 0x7b, 0xe7,
                0x63, 0xb6
            ],
            "and its session id, d8b9bf5d-90b1-4cbd-9b76-88da7be763b6"
        );
        assert_eq!(login.character_name, "", "a first login names no character");
        assert_eq!(login.kind, "Portal");
    }

    #[test]
    fn a_client_message_whose_count_exceeds_its_field_is_refused() {
        // The reference answers ERR_BAD_USER_DATA and closes; a count of 21 in a 20-slot name field is
        // not an instruction to read past the field.
        let mut bytes = CAPTURED_LOGIN.to_vec();
        let name_count_at = 38; // after header, req_id, user_id and session_id
        bytes[name_count_at] = 21;
        assert_eq!(
            Frame::decode_client_message(&bytes),
            Err(FrameError::SizeMismatch {
                header: tables::AUTH_CMSG_PORTAL_ACCOUNT_LOGIN,
                got: 21,
                expected: 20
            })
        );
    }

    #[test]
    fn the_settings_block_built_from_a_row_is_the_references_own_for_that_character() {
        // `CHARACTER_SETTINGS_EXTENDED` is the reference's reply for the character it seeded, so building
        // the block from that character's fields has to reproduce it byte for byte. The fields are read
        // out of the blob: appearance 0x03100000 (a Warrior, hair style 3), last outpost 146, the packed
        // word 0x00004011 (Prophecies, level 1, helmet shown), and five armour pieces dyed 0x000b.
        let seeded = CharacterSettings {
            last_outpost: 0x0092,
            appearance: 0x0310_0000,
            campaign: 1,
            level: 1,
            is_pvp: false,
            secondary_profession: 0,
            helm_status: 1,
            pieces: vec![
                ArmourPiece {
                    file_id: 0x005b,
                    colors: 0x000b,
                },
                ArmourPiece {
                    file_id: 0x005e,
                    colors: 0x000b,
                },
                ArmourPiece {
                    file_id: 0x005d,
                    colors: 0x000b,
                },
                ArmourPiece {
                    file_id: 0x005a,
                    colors: 0x000b,
                },
                ArmourPiece {
                    file_id: 0x005c,
                    colors: 0x000b,
                },
            ],
        };
        assert_eq!(
            character_settings(&seeded).as_slice(),
            CHARACTER_SETTINGS_EXTENDED.as_slice(),
            "the packing reproduces the reference's own 62 bytes"
        );

        // A character wearing nothing still sends all five slots (`sizeof(CharacterSettings)`), with the
        // count zero and the slots blank -- a shorter block would be a client reading past the end.
        let bare = CharacterSettings {
            last_outpost: 0,
            ..CharacterSettings::default()
        };
        let blob = character_settings(&bare);
        assert_eq!(blob.len(), 62);
        assert_eq!(blob[32], 0, "number_of_pieces");
        assert!(
            blob[37..].iter().all(|byte| *byte == 0),
            "the slots are blank"
        );

        // Six pieces are five on the wire: `pieces[5]` is the array, and the count is one byte.
        let crowded = CharacterSettings {
            pieces: (0..6)
                .map(|index| ArmourPiece {
                    file_id: index as u16,
                    colors: 0,
                })
                .collect(),
            ..CharacterSettings::default()
        };
        let blob = character_settings(&crowded);
        assert_eq!(blob.len(), 62);
        assert_eq!(blob[32], 5, "five pieces, the sixth dropped");
    }

    #[test]
    fn both_directions_are_the_same_shape_and_the_table_is_only_a_maximum() {
        // Corrected from the opposite claim, by measurement: the reference's own `CHARACTER_INFO` is
        // 102 bytes with a u16 name count, not the table's 138 with a u32 one. So there is one rule for
        // both directions, and the table's numbers are maxima to be checked against, never sizes to be
        // trusted. `Frame::decode` (the table-sized view) therefore cannot frame this message at all.
        let frame =
            character_info(1, [0x11; 16], "Asura", &CHARACTER_SETTINGS_EXTENDED).expect("builds");
        let bytes = frame.encode();
        assert_eq!(bytes.len(), 102);
        assert!(
            bytes.len() < tables::expected_size(frame.header()).expect("known header"),
            "the wire is shorter than the table's maximum"
        );

        let (decoded, used) = Frame::decode_client_message(&bytes).expect("walks its fields");
        assert_eq!(used, bytes.len());
        assert_eq!(decoded, frame);
        assert!(
            Frame::decode(&bytes).is_err(),
            "the table-sized view would wait for 138 bytes that never come"
        );
    }

    /// Exactly what a real client (build 39112) sent after the key exchange, captured from this
    /// service's own trace on 2026-10-26: 42 bytes of computer info, then a 22-byte computer hash.
    const CAPTURED_PREAMBLE: &[u8] = &[
        0x01, 0x80, 0x03, 0x00, 0x41, 0x00, 0x70, 0x00, 0x6f, 0x00, 0x0f, 0x00, 0x44, 0x00, 0x45,
        0x00, 0x53, 0x00, 0x4b, 0x00, 0x54, 0x00, 0x4f, 0x00, 0x50, 0x00, 0x2d, 0x00, 0x44, 0x00,
        0x55, 0x00, 0x52, 0x00, 0x4a, 0x00, 0x37, 0x00, 0x4e, 0x00, 0x30, 0x00, 0x02, 0x80, 0xc8,
        0x98, 0x00, 0x00, 0xf5, 0xe6, 0x06, 0x66, 0x71, 0xa9, 0x87, 0x45, 0xb8, 0xfc, 0x3a, 0xa6,
        0xb3, 0xc8, 0x55, 0x25,
    ];

    #[test]
    fn the_captured_preamble_decodes_and_hands_over_to_the_next_message() {
        // This is the regression that matters: the client's computer-info message is 42 bytes, not
        // the table's 138. Framing it by the table would swallow the following message.
        let (info, used) = decode_computer_info(CAPTURED_PREAMBLE)
            .expect("decodes")
            .expect("complete");
        assert_eq!(used, 42, "the client's message is 42 bytes");
        assert_eq!(
            info,
            ComputerInfo {
                username: "Apo".to_string(),
                pc_name: "DESKTOP-DURJ7N0".to_string(),
            }
        );

        // Everything after it frames normally: a 22-byte computer hash.
        let rest = &CAPTURED_PREAMBLE[used..];
        assert_eq!(rest.len(), 22);
        let (hash, consumed) = Frame::decode(rest).expect("the hash frames by the table");
        assert_eq!(hash.header(), tables::AUTH_CMSG_SEND_COMPUTER_HASH);
        assert_eq!(consumed, 22);
        assert_eq!(
            u32::from_le_bytes([
                hash.body()[0],
                hash.body()[1],
                hash.body()[2],
                hash.body()[3]
            ]),
            39112,
            "the hash message carries the client's build number"
        );
        assert_eq!(used + consumed, CAPTURED_PREAMBLE.len());
    }

    #[test]
    fn a_partial_preamble_asks_for_more_rather_than_consuming_it() {
        for cut in 0..42 {
            assert_eq!(
                decode_computer_info(&CAPTURED_PREAMBLE[..cut]),
                Ok(None),
                "cut at {cut} must not be consumed"
            );
        }
        assert!(decode_computer_info(&CAPTURED_PREAMBLE[..42])
            .expect("decodes")
            .is_some());
    }

    #[test]
    fn the_preamble_decoder_refuses_a_different_header() {
        let mut bytes = CAPTURED_PREAMBLE.to_vec();
        bytes[0..2].copy_from_slice(&tables::AUTH_CMSG_PORTAL_ACCOUNT_LOGIN.to_le_bytes());
        assert_eq!(
            decode_computer_info(&bytes),
            Err(MessageError::UnexpectedHeader {
                got: tables::AUTH_CMSG_PORTAL_ACCOUNT_LOGIN,
                expected: tables::AUTH_CMSG_SEND_COMPUTER_INFO
            })
        );
    }

    #[test]
    fn the_table_is_known_to_be_wrong_about_this_one_message() {
        // Not a wish: a recorded fact, with the capture above as the evidence. If a future reader
        // "fixes" the table to 42, this test says why that would be a guess rather than a fix.
        assert_eq!(
            tables::expected_size(tables::AUTH_CMSG_SEND_COMPUTER_INFO),
            Some(138)
        );
        assert_eq!(
            decode_computer_info(CAPTURED_PREAMBLE).unwrap().unwrap().1,
            42
        );
    }

    // ------------------------------------------------------------ travel

    #[test]
    fn a_travel_request_decodes_from_the_shape_a_client_sends() {
        // `req_id, map_type, map_id, region, district, language`: the six dwords of
        // `AuthSrv_RequestGameInstance`. Map 146 is what the real client asked for after character
        // select (docs/06-reference-session.md).
        let mut body = Vec::new();
        for value in [7u32, 0, 146, 0, 0, 0] {
            body.extend_from_slice(&value.to_le_bytes());
        }
        let frame = Frame::new(tables::AUTH_CMSG_REQUEST_GAME_INSTANCE, body);
        let bytes = frame.encode();
        assert_eq!(bytes.len(), 26, "the whole message, header included");

        let (decoded, used) =
            Frame::decode_client_message(&bytes).expect("fixed size, walks by the table");
        assert_eq!(used, 26);
        assert_eq!(
            GameInstanceRequest::decode(decoded.body()).expect("decodes"),
            GameInstanceRequest {
                request: 7,
                map_type: 0,
                map_id: 146,
                region: 0,
                district: 0,
                language: 0,
            }
        );
    }

    #[test]
    fn a_truncated_travel_request_is_refused_rather_than_padded() {
        assert_eq!(
            GameInstanceRequest::decode(&[1, 2, 3]),
            Err(MessageError::Truncated { field: "req_id" })
        );
    }

    #[test]
    fn game_server_info_is_42_bytes_with_the_reference_field_order() {
        let host = host_ipv4(std::net::Ipv4Addr::new(127, 0, 0, 1), 6113);
        let frame = game_server_info(7, 8001, 146, host, 0xDEAD_BEEF);
        let bytes = frame.encode();

        assert_eq!(bytes.len(), 42, "the table says 42, and this message is 42");
        assert_eq!(frame.header(), tables::AUTH_SMSG_GAME_SERVER_INFO);
        assert_eq!(frame.body().len(), 40);

        // req_id, map_token, map_id, host, player_token - read as plain little-endian dwords either
        // side of the host blob.
        assert_eq!(u32::from_le_bytes(bytes[2..6].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(bytes[6..10].try_into().unwrap()), 8001);
        assert_eq!(u32::from_le_bytes(bytes[10..14].try_into().unwrap()), 146);
        assert_eq!(
            u32::from_le_bytes(bytes[38..42].try_into().unwrap()),
            0xDEAD_BEEF
        );
    }

    #[test]
    fn an_ipv4_host_is_family_little_endian_and_port_big_endian() {
        // `le16enc(&host[0], AF_INET)` then `be16enc(&host[2], port)` in the reference: the asymmetry
        // is the reference's, not an accident of packing.
        let host = host_ipv4(std::net::Ipv4Addr::new(192, 168, 1, 50), 6113);
        assert_eq!(&host[0..2], &[0x02, 0x00], "AF_INET, little endian");
        assert_eq!(&host[2..4], &[0x17, 0xE1], "6113 = 0x17E1, big endian");
        assert_eq!(&host[4..8], &[192, 168, 1, 50]);
        assert_eq!(&host[8..], &[0u8; 16], "the rest of the 24 is zeroed");
    }

    #[test]
    fn an_ipv6_host_fills_all_sixteen_address_bytes() {
        let address: std::net::Ipv6Addr = "::1".parse().expect("parses");
        let host = host_ipv6(address, 6113);
        assert_eq!(&host[0..2], &[23, 0], "AF_INET6 as Winsock numbers it");
        assert_eq!(&host[2..4], &[0x17, 0xE1]);
        assert_eq!(&host[4..20], &address.octets());
        assert_eq!(&host[20..], &[0u8; 4]);
    }

    #[test]
    fn a_socket_address_becomes_the_host_blob_of_its_own_family() {
        let v4: std::net::SocketAddr = "10.0.0.7:7000".parse().expect("parses");
        let host = host_for(v4);
        assert_eq!(&host[0..2], &[2, 0]);
        assert_eq!(&host[4..8], &[10, 0, 0, 7]);
        assert_eq!(&host[2..4], &7000u16.to_be_bytes());

        let v6: std::net::SocketAddr = "[::1]:7001".parse().expect("parses");
        assert_eq!(&host_for(v6)[0..2], &[23, 0]);
    }
}
