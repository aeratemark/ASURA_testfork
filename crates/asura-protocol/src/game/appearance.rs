//! A character's appearance: the client's own 32-bit bitfield, and the word it packs to.
//!
//! Moved out of game/mod.rs because it is a codec for one word, not a message builder
//! (`docs/38-layer-decisions.md`, D16). `Appearance::pack`/`unpack` are the bit fields;
//! the message that carries the word is built in `super`.

/// A character's appearance, as the client's own 32-bit bitfield (`GmChar.h`, `Appearance`).
///
/// This is not decoration: the client builds the character model from these bits, and an all-zero
/// appearance says `Profession_None` with hair style 0 -- a model it has no data for. The reference's
/// own character sends `0x03100000` in `GAME_SMSG_PLAYER_CREATE` (its own trace, read by
/// `tools/compare-load.py`), which decodes as profession 1 and hair style 3.
///
/// The *default* the server sends for a character with no bits of its own is not here: that is a value,
/// and it is `config/World/Vanilla/character_defaults.json` (`appearance`). This type only packs and
/// unpacks a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Appearance {
    /// 1 bit.
    pub sex: u8,
    /// 4 bits.
    pub height: u8,
    /// 5 bits.
    pub skin_color: u8,
    /// 5 bits.
    pub hair_color: u8,
    /// 5 bits.
    pub face_style: u8,
    /// 4 bits.
    pub primary_profession: u8,
    /// 6 bits.
    pub hair_style: u8,
    /// 2 bits: 0 Prophecies, 1 Factions, 2 Nightfall.
    pub race: u8,
}

impl Appearance {
    /// Pack into the wire's 32-bit word, in the client's field order.
    pub fn pack(&self) -> u32 {
        (u32::from(self.sex) & 0x1)
            | ((u32::from(self.height) & 0xF) << 1)
            | ((u32::from(self.skin_color) & 0x1F) << 5)
            | ((u32::from(self.hair_color) & 0x1F) << 10)
            | ((u32::from(self.face_style) & 0x1F) << 15)
            | ((u32::from(self.primary_profession) & 0xF) << 20)
            | ((u32::from(self.hair_style) & 0x3F) << 24)
            | ((u32::from(self.race) & 0x3) << 30)
    }

    /// Unpack a word from the wire, for tests and for reading one back.
    pub fn unpack(word: u32) -> Self {
        Appearance {
            sex: (word & 0x1) as u8,
            height: ((word >> 1) & 0xF) as u8,
            skin_color: ((word >> 5) & 0x1F) as u8,
            hair_color: ((word >> 10) & 0x1F) as u8,
            face_style: ((word >> 15) & 0x1F) as u8,
            primary_profession: ((word >> 20) & 0xF) as u8,
            hair_style: ((word >> 24) & 0x3F) as u8,
            race: ((word >> 30) & 0x3) as u8,
        }
    }
}
