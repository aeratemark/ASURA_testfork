//! The exploration grid a character's map surfaces draw from.
//!
//! The client keeps one bit per 32-unit cell, **per continent**, and it has no source for those
//! dimensions except the server: `GAME_SMSG_MAP_INIT_START` (`0x008B`) announces
//! `{dims.x, dims.y, compressed byte count}`, the `GAME_SMSG_CARTOGRAPHY_DATA` (`0x008A`) messages
//! that follow carry the compressed bits, and the client decompresses once it has the announced
//! number of bytes. It asserts `dims.x % 32 == 0` on the way in, and its map view asserts the grid
//! against the continent's own map art the first time a player opens a map
//! (`Assertion: worldMapDims.x == mapDims.x * DXT_BLOCK_SIZE`, client build 39112,
//! `GmMapView.cpp(1731)`, measured 2026-10-29 - the assertion this module answers).
//!
//! The compressed form is the client's own, and it is a band format rather than a general
//! compressor: the grid is cut into bands of 16 scanlines, each band is a `u16` byte length followed
//! by a two-state run-length encoding, and runs alternate polarity starting with **zeros**
//! (unexplored). A run's length is the sum of consecutive `0xFF` bytes up to and including the first
//! byte that is not `0xFF`, and every run after the first is one longer than its bytes say.
//!
//! **A band with no runs needs no bytes at all**: its length is `0` and the client fills the band
//! with unexplored cells (`CharCli` zeroes the grid before it decodes and stops a band when its
//! bytes end). A live client's own message shows exactly that shape - eleven empty bands beside one
//! full one (`recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`,
//! seq 1055/1056).
//!
//! What this module sends today is a grid with **nothing explored**: every band is empty, so the
//! whole blob is `dims.y / 16` pairs of zero bytes. The bits are a character's progress and this
//! schema keeps none, so "nothing" is the honest value and it is also what a fresh character has.

use asura_protocol::client::Frame;
use asura_protocol::game;

/// The scanlines one band of the compressed grid covers. The client's own constant.
const BAND_ROWS: u32 = 16;

/// An empty band: a `u16` length of zero, and nothing after it.
const EMPTY_BAND: [u8; 2] = [0, 0];

/// Tyria's grid, in cells: `64 x 128`, measured from a live client's own `MapInitStart` while it
/// stood in a Tyria outpost (a 26-byte blob, eight bands of 16 scanlines at width 64).
///
/// A grid is per continent, not per map: `MapInitStart` replaces the client's whole grid, and a map's
/// own area is a rectangle inside it. The width is a multiple of 32 because the client asserts that,
/// and the height a multiple of 16 because the band format has no partial band. The world map's
/// assertion compares `dims.x * 4` with the continent's own art, so the width is the number that
/// matters: 64 cells is a 256-texel world map, and 256 cells would be 1024. Another continent's
/// measurement (a character-creation instance) was 256 x 192 - a wider grid for a wider world.
pub const TYRIA: (u32, u32) = (64, 128);

/// The grid of the continent `map_id` belongs to.
///
/// Only Tyria's dimensions are assumed, so every map gets them: a map on another continent then
/// fails the same map-view assertion it fails today, and none that works today can start failing.
/// The other continents' grids are a measurement, not a choice, and they are named in
/// `docs/03-protocol.md` as an open question.
pub fn grid_for(_map_id: i32) -> (u32, u32) {
    TYRIA
}

/// The compressed form of `dims` with every cell unexplored.
pub fn unexplored_blob(dims: (u32, u32)) -> Vec<u8> {
    let (width, height) = dims;
    debug_assert!(
        width % 32 == 0,
        "the client asserts the grid width is a multiple of 32"
    );
    debug_assert!(
        height % BAND_ROWS == 0,
        "the band format has no partial band"
    );
    let mut blob = Vec::with_capacity((height / BAND_ROWS) as usize * EMPTY_BAND.len());
    for _ in 0..height / BAND_ROWS {
        blob.extend_from_slice(&EMPTY_BAND);
    }
    blob
}

/// The frames that give the client its grid: the announcement, then the blob in `0x008A` slices.
pub fn init_frames(dims: (u32, u32)) -> Vec<Frame> {
    let blob = unexplored_blob(dims);
    let mut frames = vec![game::map_init_start(dims.0, dims.1, blob.len() as u32)];
    for slice in blob.chunks(game::CARTOGRAPHY_DATA_BYTES) {
        frames.push(game::cartography_data(slice));
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run byte that continues a run rather than ending it: the decoder below reads it, and no
    /// grid this module sends today carries a run at all.
    const RUN_CONTINUES: u8 = 0xFF;

    /// The client's decompressor, written here only to read back what [`unexplored_blob`] writes: the
    /// band structure, the run length rule, the polarity, and the zero fill of an empty or short band.
    fn decode(blob: &[u8], dims: (u32, u32)) -> Vec<u8> {
        let mut bits = Vec::new();
        let mut at = 0usize;
        for _ in 0..dims.1 / BAND_ROWS {
            let length = u16::from_le_bytes([blob[at], blob[at + 1]]) as usize;
            at += 2;
            let band = &blob[at..at + length];
            at += length;

            let target = (dims.0 * BAND_ROWS) as usize;
            let mut produced = 0usize;
            let mut explored = false;
            let mut index = 0usize;
            while produced < target && index < band.len() {
                let mut run = 0usize;
                loop {
                    let byte = band[index];
                    index += 1;
                    run += usize::from(byte);
                    if byte != RUN_CONTINUES || index == band.len() {
                        break;
                    }
                }
                if produced > 0 {
                    run += 1;
                }
                for _ in 0..run.min(target - produced) {
                    bits.push(u8::from(explored));
                }
                produced += run.min(target - produced);
                explored = !explored;
            }
            bits.resize(bits.len() + (target - produced), 0);
        }
        bits
    }

    #[test]
    fn the_blob_is_the_band_form_the_client_decompresses() {
        let blob = unexplored_blob(TYRIA);
        // 8 bands of 16 scanlines, every one of them empty: two zero bytes each.
        assert_eq!(blob.len(), 8 * 2);
        assert!(blob.iter().all(|byte| *byte == 0), "empty bands are zeros");
        let bits = decode(&blob, TYRIA);
        assert_eq!(bits.len(), (TYRIA.0 * TYRIA.1) as usize, "one bit per cell");
        assert!(bits.iter().all(|bit| *bit == 0), "nothing is explored");
    }

    /// The decoder above is checked against the live client's own compressed band, which a byte
    /// forwarded from a capture pins: this is the one test that would catch it being wrong.
    #[test]
    fn the_decoder_reads_the_clients_own_band() {
        // `character_creation-live-necro_factions.jsonl` seq 1056, band 5: 20 bytes at width 256.
        let band = [
            0xffu8, 0xff, 0xff, 0xff, 0x35, 0x02, 0xfc, 0x02, 0xfc, 0x02, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xd4,
        ];
        let mut blob = Vec::new();
        blob.extend_from_slice(&(band.len() as u16).to_le_bytes());
        blob.extend_from_slice(&band);
        let bits = decode(&blob, (256, 16));
        assert_eq!(bits.len(), 256 * 16, "one band, filled to the grid");
        assert!(bits.contains(&1), "the captured band has explored cells");
    }

    #[test]
    fn the_frames_announce_the_blob_they_carry() {
        let frames = init_frames(TYRIA);
        let blob = unexplored_blob(TYRIA);

        let start = &frames[0];
        assert_eq!(start.header(), game::MAP_INIT_START);
        assert_eq!(start.len(), 14, "three dwords after the header");
        assert_eq!(
            u32::from_le_bytes([start.body()[0], start.body()[1], start.body()[2], start.body()[3]]),
            TYRIA.0
        );
        assert_eq!(
            u32::from_le_bytes([start.body()[4], start.body()[5], start.body()[6], start.body()[7]]),
            TYRIA.1
        );
        assert_eq!(
            u32::from_le_bytes([start.body()[8], start.body()[9], start.body()[10], start.body()[11]]),
            blob.len() as u32,
            "the byte count the client waits for"
        );

        // 16 bytes is one slice of the 260-byte message: a u16 dword count, then the fixed 64 slots.
        assert_eq!(frames.len(), 2, "the announcement and one slice");
        let slice = &frames[1];
        assert_eq!(slice.header(), game::CARTOGRAPHY_DATA);
        assert_eq!(slice.len(), 260, "a u16 count and the fixed 64-slot array");
        let count = u16::from_le_bytes([slice.body()[0], slice.body()[1]]);
        assert_eq!(
            count,
            blob.len().div_ceil(4) as u16,
            "the dwords that count"
        );
        assert_eq!(
            &slice.body()[2..2 + blob.len()],
            &blob[..],
            "the blob, then padding"
        );
        assert!(
            slice.body()[2 + blob.len()..].iter().all(|byte| *byte == 0),
            "the rest of the fixed payload is zero"
        );
    }
}
