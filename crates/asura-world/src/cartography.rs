//! The exploration grid a character's map surfaces draw from.
//!
//! The client keeps one bit per block, **per continent**, and it has no source for the grid's
//! dimensions except the server: `GAME_SMSG_MAP_INIT_START` (`0x008B`, which the client's own
//! disassembly calls the exploration init's *begin*) announces `{dims.x, dims.y, byte count}`, the
//! `GAME_SMSG_CARTOGRAPHY_DATA` (`0x008A`) messages that follow carry the RLE stream, and the client
//! expands it once it has the announced number of bytes. It asserts `dims.x % 32 == 0` on the way in
//! (`ChCliApi:77`), and its map view asserts the grid against the continent's own map art the first
//! time a player opens a map (`Assertion: worldMapDims.x == mapDims.x * DXT_BLOCK_SIZE`, client build
//! 39112, `GmMapView.cpp(1731)`, measured 2026-10-29 - the assertion this module answers).
//!
//! # The format, as retail's own bytes settle it
//!
//! **This module's reading changed on 2026-11-04, and the two rules that changed are the two that
//! only matter once a grid is not empty** - which is why a server that sent nothing but empty bands
//! could hold the wrong reading for as long as it did. The referee is ArenaNet's own 38-byte stream
//! (`Rurik probes.FOG_INIT_PAYLOAD`, continent 1, dims `64 x 128`, band chain
//! `(0, 22, 0, 0, 0, 0, 0, 0)` closing exactly at its declared 38 bytes) together with two blocks
//! the client itself proved, by what marking them did to the screen: block `(30, 24)` is **set** and
//! block `(26, 22)` is **clear** (`Rurik studies/minimap/FINDINGS.md` 6f.4, the 0 px and 1,059 px
//! mark runs). Of the four candidate readings, exactly two reproduce both blocks, and the third
//! fixture - 947 bits set, the "large revealed region" the client draws on the M press (6f.2) -
//! separates them:
//!
//! | run length | first run's colour | `(30,24)` | `(26,22)` | bits set |
//! |---|---|---|---|---|
//! | **byte sum** | **revealed** | **set** | **clear** | **947** |
//! | byte sum | fogged | clear | set | 45 |
//! | byte sum **+1** after the first | revealed | clear | set | 969 |
//! | byte sum +1 after the first | fogged | set | clear | 55 |
//!
//! So:
//!
//! - the grid is cut into **bands of 16 block-rows** (`shr eax, 4`), each a little-endian `u16` byte
//!   length followed by that many run bytes, and the next band's header starts right after;
//! - a run's length is the **plain sum** of its bytes, `0xFF` meaning "add 255 and keep reading".
//!   There is no "one longer than its bytes say" rule: the reading that had one is the third row
//!   above, and it inverts both of the client's own blocks;
//! - runs **alternate colour starting with revealed**, zero-length runs included, so a band that
//!   starts fogged opens with a bare `0x00`. (This polarity is behaviour-corroborated rather than
//!   read off the instructions - one static trace of the expander reads fogged-first. The arbiter is
//!   one screenshot: if `ASURA_CARTOGRAPHY=all` ever opens a fully *fogged* map, flip
//!   [`REVEALED_FIRST`] and nothing else changes.)
//! - bits go in LSB-first and are written **only on a full-dword flush**, so a band's trailing
//!   partial dword is dropped and never lands. [`encode_grid`] pads the bits it emits up to a multiple of
//!   32 for exactly this reason; `dims.x % 32 == 0` makes a whole band a whole number of dwords, so
//!   nothing else can be lost.
//!
//! **An empty band needs no bytes at all**, and means all-fogged under every reading above: the
//! expander memsets the bitmap to zero before it starts, so a stream of empty bands is a valid
//! all-fogged grid whoever is right about the rest. That is what this server sent before it could
//! send anything else, and it is what a fresh character still gets.
//!
//! # What this module does, and what is still missing
//!
//! [`encode_grid`] and [`decode_grid`] are the two halves of the format and they round-trip, ArenaNet's own
//! stream included, so this module can now send **any** grid. [`startup_blob`] chooses which:
//!
//! | `ASURA_CARTOGRAPHY` | the grid every load sends |
//! |---|---|
//! | unset, `none`, `off` | nothing explored - what a fresh character has |
//! | `all` | the known-good retail region - the one non-empty grid a live client accepted |
//! | `<hex>` | that stream verbatim, for replaying a captured one |
//!
//! What is **not** here is the other half of the feature: a character's progress has nowhere to live
//! (no column keeps it) and nothing reveals a block as a player walks. The reveal is a third message
//! this server does not send yet - `0x008C`, 11 bytes, `{x, y, span}` in continent blocks, which the
//! client ORs into this same grid and which feeds the compass and both maps - and it is inert until
//! the `0x008B`+`0x008A` pair above has run, which this server already sends. See
//! `docs/03-protocol.md`, the exploration grid, for the three things it needs that are not measured
//! here: where a map sits in its continent's blocks, the client's per-map "explorable" mask (which
//! can refuse a mark outright), and a place in the schema to keep the bits.

use asura_protocol::client::Frame;
use asura_protocol::game;

/// The block-rows one band of the stream covers. The client's own constant (`shr eax, 4`).
pub const BAND_ROWS: u32 = 16;

/// The bits one dword of the bitmap holds, and the granularity the client flushes at: a band's
/// trailing partial dword is accumulated and then dropped.
const BITS_PER_FLUSH: usize = 32;

/// A byte that continues a run rather than ending it: a run's length is the sum of its bytes, and
/// `0xFF` means "add 255 and keep reading".
const RUN_CONTINUES: u8 = 0xFF;

/// Whether the first run of a band is the explored colour.
///
/// `true` is the behaviour-corroborated reading and the one the module header's table settles; a
/// static trace of the client's expander reads `false`. Both agree on an all-fogged stream, so this
/// only decides what a *revealed* grid looks like - and the arbiter is one screenshot, named in the
/// header.
pub const REVEALED_FIRST: bool = true;

/// An empty band: a `u16` length of zero, and nothing after it. All-fogged under every reading.
const EMPTY_BAND: [u8; 2] = [0, 0];

/// Tyria's grid, in blocks: `64 x 128`, measured from a live client's own `MapInitStart` while it
/// stood in a Tyria outpost, and the same value ArenaNet's own stream carries on 8 of 8 captures.
///
/// A grid is per continent, not per map: the init pair replaces the client's whole grid, and a map's
/// own area is a rectangle inside it. The width is a multiple of 32 because the client asserts that,
/// and the height a multiple of 16 because the band format has no partial band. The world map's
/// assertion compares `dims.x * 4` with the continent's own art, so the width is the number that
/// matters: 64 blocks is a 256-texel world map, and 256 blocks would be 1024. Another continent's
/// measurement (a character-creation instance) was 256 x 192 - a wider grid for a wider world.
pub const TYRIA: (u32, u32) = (64, 128);

/// ArenaNet's own exploration stream for continent 1, trimmed to its declared 38 bytes.
///
/// This is the only non-empty stream the live client has accepted here. Its revealed region is the
/// known-valid part of the pre-searing strip; synthetic full bands are refused with `007`. The
/// `all` override and `/explore all` use [`Grid::known_good_revealed`], which decodes these bytes.
pub const RETAIL_STREAM: [u8; 38] = [
    0x00, 0x00, 0x16, 0x00, 0xff, 0x21, 0x02, 0x3a, 0x04, 0x3a, 0x04, 0x39, 0x05, 0x38, 0x06, 0x35,
    0x09, 0x35, 0x06, 0x38, 0x05, 0x3a, 0x02, 0x3c, 0x02, 0xa2, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// The grid of the continent `map_id` belongs to.
///
/// Only Tyria's dimensions are assumed, so every map gets them: a map on another continent then
/// fails the same map-view assertion it fails today, and none that works today can start failing.
/// The other continents' grids are a measurement, not a choice, and they are named in
/// `docs/03-protocol.md` as an open question.
pub fn grid_for(_map_id: i32) -> (u32, u32) {
    TYRIA
}

/// The continent `map_id` belongs to: the key a character's stored exploration is filed under.
///
/// 1 is Tyria, and every map this server serves is in pre-searing Tyria, so every map answers 1. It
/// is a function rather than a constant because the thing it stands for is per map and a second
/// continent would need a table here - and because a stored row carries the continent it was saved
/// under, a wrong answer here would file one character's progress under two keys rather than
/// corrupt either.
pub fn continent_for(_map_id: i32) -> i16 {
    TYRIA_CONTINENT
}

/// Tyria's continent number, as the client's own map rows number continents.
pub const TYRIA_CONTINENT: i16 = 1;

/// One continent's blocks: one byte per block, `1` explored, row-major, `dims.0` wide.
///
/// A byte per block rather than a bit, because a continent's grid is small - Tyria's is 8 KiB - and
/// every edge in the stream format is about runs rather than words. [`encode_grid`] is the only thing
/// that packs.
#[derive(Clone, PartialEq, Eq)]
pub struct Grid {
    dims: (u32, u32),
    blocks: Vec<u8>,
}

/// Dimensions and a count, because a failed assertion that printed 8,192 blocks would bury the one
/// number that says what went wrong.
impl std::fmt::Debug for Grid {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            out,
            "Grid({} x {}, {} of {} explored)",
            self.dims.0,
            self.dims.1,
            self.explored_count(),
            self.blocks.len()
        )
    }
}

impl Grid {
    /// A grid with nothing explored.
    pub fn empty(dims: (u32, u32)) -> Self {
        debug_assert!(
            dims.0 % 32 == 0,
            "the client asserts the grid width is a multiple of 32 (ChCliApi:77)"
        );
        debug_assert!(
            dims.1 % BAND_ROWS == 0,
            "the band loop floors dims.y / 16, stranding the remainder rows"
        );
        Self {
            dims,
            blocks: vec![0; (dims.0 * dims.1) as usize],
        }
    }

    /// A grid with every block explored: the codec's own maximum, not what `all` sends.
    ///
    /// A synthetic full grid is rejected by the live client with `007`; see
    /// [`Self::known_good_revealed`] for the grid `all` actually sends.
    pub fn all_explored(dims: (u32, u32)) -> Self {
        let mut grid = Self::empty(dims);
        grid.blocks.fill(1);
        grid
    }

    /// The known-valid explored region: ArenaNet's own stream, decoded.
    ///
    /// This is what `all` sends. A synthetic fill of band 1, or of the whole 64 x 128 grid, is
    /// refused with `007`; the retail stream loads and draws the pre-searing region. It is not
    /// every block, and that is deliberate: the client's map art does not accept an arbitrary
    /// block set, and this is the one non-empty set a live client has confirmed.
    pub fn known_good_revealed(dims: (u32, u32)) -> Self {
        decode_grid(&RETAIL_STREAM, dims)
    }

    pub fn dims(&self) -> (u32, u32) {
        self.dims
    }

    pub fn blocks(&self) -> &[u8] {
        &self.blocks
    }

    /// Whether the block at `(x, y)` is explored. Out of bounds is unexplored.
    pub fn is_explored(&self, x: u32, y: u32) -> bool {
        self.index(x, y).is_some_and(|at| self.blocks[at] == 1)
    }

    /// Mark the block at `(x, y)` explored, and answer whether that changed anything. Out of bounds
    /// is ignored rather than a panic: a reveal that runs off the edge of a continent is a clamp,
    /// not a bug.
    pub fn explore(&mut self, x: u32, y: u32) -> bool {
        match self.index(x, y) {
            Some(at) if self.blocks[at] == 0 => {
                self.blocks[at] = 1;
                true
            }
            _ => false,
        }
    }

    /// Mark a horizontal span of `span` blocks from `(x, y)` explored, and answer how many changed.
    ///
    /// A span is the shape the client's own reveal message carries (`0x008C`: `{x, y, span}`), so
    /// this is the unit a live reveal would accumulate in and send.
    pub fn explore_span(&mut self, x: u32, y: u32, span: u32) -> usize {
        (x..x.saturating_add(span))
            .filter(|at| self.explore(*at, y))
            .count()
    }

    /// Mark every block within `radius` of `(x, y)` explored, and answer how many changed: the
    /// brush a walking player would paint with, once something measures where a player stands in
    /// continent blocks.
    pub fn explore_disc(&mut self, x: i64, y: i64, radius: u32) -> usize {
        let radius = radius as i64;
        let mut painted = 0;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy > radius * radius || x + dx < 0 || y + dy < 0 {
                    continue;
                }
                painted += usize::from(self.explore((x + dx) as u32, (y + dy) as u32));
            }
        }
        painted
    }

    /// How many blocks are explored: the number a cartographer percentage would come from.
    pub fn explored_count(&self) -> usize {
        self.blocks.iter().filter(|block| **block == 1).count()
    }

    /// The blocks as one bit each: `dims.0 * dims.1 / 8` bytes, row-major, LSB first within a byte.
    ///
    /// This is the *stored* form, not the wire form. It says one thing only, which is why a database
    /// row holds it rather than the compressed stream: no polarity, no runs, no bands - bit
    /// `y * dims.0 + x` of a flat array. See migration 0027 for why that matters.
    pub fn packed(&self) -> Vec<u8> {
        let mut packed = vec![0u8; self.blocks.len().div_ceil(8)];
        for (at, block) in self.blocks.iter().enumerate() {
            if *block == 1 {
                packed[at / 8] |= 1 << (at % 8);
            }
        }
        packed
    }

    /// The grid `packed` describes, or `None` when it is not `dims`-shaped.
    ///
    /// `None` rather than a best effort: a blob of the wrong length against these dims is a stored row
    /// from before a dimension changed, and drawing it would reveal a character's map in the wrong
    /// places with nothing to say that is what happened.
    pub fn from_packed(dims: (u32, u32), packed: &[u8]) -> Option<Self> {
        let mut grid = Self::empty(dims);
        if packed.len() != grid.blocks.len().div_ceil(8) {
            return None;
        }
        for (at, block) in grid.blocks.iter_mut().enumerate() {
            *block = u8::from(packed[at / 8] & (1 << (at % 8)) != 0);
        }
        Some(grid)
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        (x < self.dims.0 && y < self.dims.1).then(|| (y * self.dims.0 + x) as usize)
    }
}

/// One run of `bits` bits: `0xFF` continuation bytes that sum to it.
///
/// Every byte before the last is `0xFF` ("add 255, keep reading"), and any byte below `0xFF` ends
/// the run - so a multiple of 255 needs its explicit `0x00` terminator, and zero is a bare `0x00`.
fn run_bytes(bits: usize) -> Vec<u8> {
    let mut out = vec![RUN_CONTINUES; bits / RUN_CONTINUES as usize];
    out.push((bits % RUN_CONTINUES as usize) as u8);
    out
}

/// The stream `grid` compresses to.
///
/// Three of the client's rules shape what comes out, and each one is a line below: runs alternate
/// from [`REVEALED_FIRST`] so a band that starts fogged opens with a zero-length run; a band whose
/// blocks are all unexplored is written as an empty band, which is all-fogged under every reading;
/// and the bits a band emits are padded up to a whole dword, because the client drops a trailing
/// partial one and the last explored blocks would go with it.
///
/// A run is written as one natural sequence of `0xFF` bytes and a terminator, the way ArenaNet's
/// own streams are: retail's longest is 288 bits and gw-interface's is 594. Splitting long runs at
/// an arbitrary chunk size inserts zero-length runs into the middle of a run, which no captured
/// stream does, and is the shape the stored-init 007 runs all had.
pub fn encode_grid(grid: &Grid) -> Vec<u8> {
    let (width, height) = grid.dims;
    let capacity = (width * BAND_ROWS) as usize;
    let mut stream = Vec::new();

    for band in 0..(height / BAND_ROWS) as usize {
        let blocks = &grid.blocks[band * capacity..(band + 1) * capacity];

        // Nothing explored in this band: no bytes at all.
        let Some(last) = blocks.iter().rposition(|block| *block == 1) else {
            stream.extend_from_slice(&EMPTY_BAND);
            continue;
        };
        // Up to the end of the dword the last explored block sits in, so that it flushes.
        let covered = (last / BITS_PER_FLUSH + 1) * BITS_PER_FLUSH;

        let mut body = Vec::new();
        let mut colour = REVEALED_FIRST;
        let mut run = 0usize;
        for block in &blocks[..covered] {
            if (*block == 1) == colour {
                run += 1;
            } else {
                body.extend_from_slice(&run_bytes(run));
                colour = !colour;
                run = 1;
            }
        }
        body.extend_from_slice(&run_bytes(run));

        stream.extend_from_slice(&(body.len() as u16).to_le_bytes());
        stream.extend_from_slice(&body);
    }
    stream
}

/// The grid `stream` expands to: the client's expander, reproduced.
///
/// Faithful to its edges, because they are what a server has to encode around: a band length clamped
/// to the declared total, the loop stopping once the next header no longer fits, a run truncated
/// mid-continuation still emitting what it summed, writes stopping at a band's capacity, and a
/// band's trailing partial dword dropped rather than written. `stream` must already be trimmed to
/// the byte count `0x008B` declared, the way the client trims it.
///
/// This is not only a test fixture: it is the referee [`encode_grid`] is checked against, and it is how a
/// captured stream is read back.
pub fn decode_grid(stream: &[u8], dims: (u32, u32)) -> Grid {
    let mut grid = Grid::empty(dims);
    let capacity = (dims.0 * BAND_ROWS) as usize;
    if stream.len() < 2 {
        return grid; // an empty stream is all-fogged
    }

    let mut at = 0usize;
    for band in 0..(dims.1 / BAND_ROWS) as usize {
        let Some(header) = stream.get(at..at + 2) else {
            break;
        };
        let length = usize::from(u16::from_le_bytes([header[0], header[1]]));
        let start = at + 2;
        let end = (start + length).min(stream.len());
        at = end;

        let base = band * capacity;
        let mut emitted = 0usize;
        let mut colour = REVEALED_FIRST;
        let mut cursor = start;
        while cursor < end {
            let mut run = 0usize;
            while cursor < end {
                let byte = stream[cursor];
                cursor += 1;
                run += usize::from(byte);
                if byte != RUN_CONTINUES {
                    break;
                }
            }
            if colour {
                for at in emitted..(emitted + run).min(capacity) {
                    grid.blocks[base + at] = 1;
                }
            }
            emitted += run;
            colour = !colour;
        }

        // The partial-dword drop: bits past the last full dword were only ever in the accumulator.
        let emitted = emitted.min(capacity);
        for at in (emitted / BITS_PER_FLUSH * BITS_PER_FLUSH)..emitted {
            grid.blocks[base + at] = 0;
        }

        if at + 2 > stream.len() {
            break; // the next header does not fit
        }
    }
    grid
}

/// The stream for a grid with nothing explored: one empty band per 16 rows.
///
/// Kept under its own name because it is the one stream whose meaning no open question touches, and
/// because it is what [`startup_blob`] answers with by default.
pub fn unexplored_blob(dims: (u32, u32)) -> Vec<u8> {
    encode_grid(&Grid::empty(dims))
}

/// The stream every load sends, and the one knob this module has: `ASURA_CARTOGRAPHY`.
///
/// `none`/`off`/unset keeps the empty grid. `all` sends the known-good retail region - the one
/// non-empty grid a live client accepted; a synthetic full band is refused with `007`, so `all`
/// is not every block. A hex string is sent verbatim, so a captured stream can be replayed byte for
/// byte. Anything unrecognised is the default, because a typo in an environment variable should not
/// desynchronise a client.
pub fn startup_blob(dims: (u32, u32)) -> Vec<u8> {
    override_blob(dims).unwrap_or_else(|| unexplored_blob(dims))
}

/// What `ASURA_CARTOGRAPHY` asks for, if it asks for anything.
///
/// `None` means "send the character its own grid", which is the ordinary path; `Some` is the
/// measurement switch overriding it. Unset, `none` and `off` are all `None`, and so is a value this
/// cannot parse - a typo in an environment variable should not desynchronise a client, and it should
/// not quietly wipe what a character has explored either.
pub fn override_blob(dims: (u32, u32)) -> Option<Vec<u8>> {
    match std::env::var("ASURA_CARTOGRAPHY").ok().as_deref() {
        Some("all" | "full") => Some(encode_grid(&Grid::known_good_revealed(dims))),
        Some(hex) if hex.len() >= 2 && hex.len() % 2 == 0 => (0..hex.len() / 2)
            .map(|at| u8::from_str_radix(&hex[at * 2..at * 2 + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .ok(),
        _ => None,
    }
}

/// The stream one load sends for a character: the empty grid, unless the measurement switch or
/// `ASURA_CARTOGRAPHY_INIT=stored` asks for more.
///
/// The override wins deliberately. It exists to put a known grid on the wire and read the client's
/// answer, and a run that silently got a character's real progress instead would be a measurement of
/// nothing. It changes only what is *sent*: the character's stored grid is untouched by it, so a run
/// with `ASURA_CARTOGRAPHY=all` cannot write its override into anybody's save.
pub fn stream_for(dims: (u32, u32), explored: &Grid) -> Vec<u8> {
    if let Some(blob) = override_blob(dims) {
        return blob;
    }
    if init_carries_stored() {
        encode_grid(&init_safe(explored))
    } else {
        unexplored_blob(dims)
    }
}

/// Whether the init stream carries the character's saved grid. Off unless
/// `ASURA_CARTOGRAPHY_INIT=stored` (or `on`, `1`, `true`).
///
/// The ordinary path is the empty init: a character's saved blocks are replayed through the paced
/// `0x008C` marks the walker already sends on its first position update, across every pre-searing
/// map. That is the path a live client accepts for arbitrary saved grids. A stored grid in the init
/// is the measurement switch: the client refused every sparse stored grid measured on 2026-11-05
/// (9 and 18 blocks) with `007` before it asked to spawn, while the same blocks arrived as marks and
/// the empty init loaded. The grid goes through [`init_safe`] when this is on, and a character whose
/// load fails anyway falls back to the empty grid by itself - see [`note_init_refused`] - so a
/// refused stream costs one `007`, not a lockout.
pub fn init_carries_stored() -> bool {
    match std::env::var("ASURA_CARTOGRAPHY_INIT").ok().as_deref() {
        Some("stored" | "on" | "1" | "true") => true,
        Some("empty" | "off" | "0" | "none") => false,
        _ => asura_config::cartography().stored_init(),
    }
}

/// Characters whose last load with a saved-grid init ended before the client asked to spawn - which
/// is what a refused init stream looks like from the server. Keyed by the character's UUID as a
/// `u128`. In memory only: a world restart tries the saved grid again, once.
fn init_refused_set() -> &'static std::sync::Mutex<std::collections::HashSet<u128>> {
    static REFUSED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<u128>>> =
        std::sync::OnceLock::new();
    REFUSED.get_or_init(Default::default)
}

/// Whether this character's saved grid was refused as an init last time: if so, its next load sends
/// the empty grid and its progress comes back as marks instead.
pub fn init_refused(character: u128) -> bool {
    init_refused_set()
        .lock()
        .map(|set| set.contains(&character))
        .unwrap_or(false)
}

/// The self-healing half of carrying the saved grid: a load that was sent it and never got as far
/// as asking to spawn is treated as a refusal, and the character's next load is sent the empty grid.
/// A client closed mid-load is counted the same way, which costs nothing - it only means one load
/// with the old behaviour.
pub fn note_init_refused(character: u128) {
    if let Ok(mut set) = init_refused_set().lock() {
        set.insert(character);
    }
}

/// The client spawned, so whatever init it was sent was accepted: forget any earlier refusal.
pub fn note_init_accepted(character: u128) {
    if let Ok(mut set) = init_refused_set().lock() {
        set.remove(&character);
    }
}

/// The blocks of `grid` that [`init_safe`] leaves out of the init stream: each band's last 32. When
/// the init carries the saved grid, these are all a load still has to replay as marks.
pub fn left_out_of_init(grid: &Grid) -> Grid {
    let safe = init_safe(grid);
    let mut left = grid.clone();
    for (block, kept) in left.blocks.iter_mut().zip(safe.blocks.iter()) {
        if *kept == 1 {
            *block = 0;
        }
    }
    left
}

/// `grid` as the init stream can carry it: each band's last 32 blocks cleared.
///
/// **Every init stream the client has refused had a band whose runs add up to exactly 1,024 bits -
/// the whole band - and every one it accepted stopped short** (retail at 1,004 bits, gw-interface's
/// captured stream at 1,010). [`encode_grid`] ends a band at the next 32-bit boundary past its last
/// explored block, so whenever that block is in the band's final dword the band runs to 1,024. The
/// decisive case: ArenaNet's own retail stream, re-encoded that way, reaches 1,024 with the same
/// pixels. And it is not long runs or zero-length runs, which is what this encoder used to guard
/// against: gw-interface's accepted stream has a 594-bit run and a zero-length one mid-band.
///
/// Clearing the last dword keeps every band at 992 bits at most. Those blocks are row 15 of a band,
/// x 32 to 63; the character keeps them in its grid, and on the map they belong to they come back as
/// marks, like everything else did before the init carried anything.
pub fn init_safe(grid: &Grid) -> Grid {
    let mut safe = grid.clone();
    let (width, height) = grid.dims();
    let capacity = (width * BAND_ROWS) as usize;
    for band in 0..(height / BAND_ROWS) as usize {
        let end = (band + 1) * capacity;
        for block in &mut safe.blocks[end - BITS_PER_FLUSH..end] {
            *block = 0;
        }
    }
    safe
}

/// The frames that give the client its grid: the announcement, then the stream in `0x008A` slices.
///
/// The grid with nothing explored, for a caller that has no character to speak for - a creation
/// instance, or a test.
pub fn init_frames(dims: (u32, u32)) -> Vec<Frame> {
    frames_for(dims, &startup_blob(dims))
}

/// The same pair for a character's own grid: what every ordinary map load sends.
pub fn init_frames_for(dims: (u32, u32), explored: &Grid) -> Vec<Frame> {
    frames_for(dims, &stream_for(dims, explored))
}

/// The same pair for a stream already in hand: what a stored per-character grid would go out as.
pub fn frames_for(dims: (u32, u32), stream: &[u8]) -> Vec<Frame> {
    let mut frames = vec![game::map_init_start(dims.0, dims.1, stream.len() as u32)];
    for slice in stream.chunks(game::CARTOGRAPHY_DATA_BYTES) {
        frames.push(game::cartography_data(slice));
    }
    frames
}

/// The `0x008C` marks that reveal every explored block of `grid`, in 3 x 3 windows.
///
/// The mark is the incremental path: the client validates one small window at a time and ORs it
/// into the bitmap the init pair already built. It does not replace the init message, so a block
/// the client will not draw cannot desynchronise the whole stream. One mark covers a 3 x 3 block
/// square (`half_span = 1`, Rurik's live mark), so windows step by three.
pub fn mark_frames(grid: &Grid) -> Vec<Frame> {
    let (width, height) = grid.dims();
    let mut frames = Vec::new();
    let mut y = 0;
    while y < height {
        let mut x = 0;
        while x < width {
            let mut any = false;
            for at_y in y..(y + 3).min(height) {
                for at_x in x..(x + 3).min(width) {
                    any |= grid.is_explored(at_x, at_y);
                }
            }
            if any {
                frames.push(game::map_exploration_mark(x, y, 1));
            }
            x += 3;
        }
        y += 3;
    }
    frames
}

/// The block rectangle a map occupies on the world map, as `x0..x1, y0..y1` with **both upper
/// bounds exclusive** - the client's own rectangle, not an estimate.
///
/// The source is GWToolbox's `tools/bake_cartography/placed_maps.txt`: "the world-map rectangles
/// extracted from the client's AreaInfo table", as `map:continent:x0:y0:x1:y1` in world units. The
/// rows here are those, divided by 32 - a block is 32 world units - and every one is a whole number
/// of blocks.
///
/// **This is what decides whether a mark is safe, and the upper bounds are exclusive.** Lakeside is
/// `768..1184` world units, which is blocks `24` up to but not including `37`. A mark that touches
/// column 37 or row 32 is outside the map, and a mark outside the map is an immediate `007`
/// (measured): that is what ended the Ashford Abbey run of 2026-11-05, where a replayed window
/// reached the far edge. The checks:
///
/// * every pre-searing doorway (migration 0025), put through [`block_at`]'s transform, lands inside
///   its own map's rectangle here - 27 of 27;
/// * the x ranges agree with the shifts solved independently from those doorways, which is what
///   showed `tools/presearing-cartography.py`'s table to be mirrored in x for every map but Lakeside;
/// * **the Catacombs (145) are not in the table at all**: they are not on the world map, so they earn
///   no exploration and are never marked - WorldCompletion calls these non-completion zones.
///
/// An outpost has the rectangle of the explorable around it: Ascalon City and Ashford Abbey are
/// Lakeside's exactly. Rows are per map, not per file, because that is how the client keys them.
pub fn map_rect(map_id: i32) -> Option<(u32, u32, u32, u32)> {
    // (x0, y0, x1, y1) in world units, verbatim from placed_maps.txt.
    let (x0, y0, x1, y1) = match map_id {
        146 | 148 | 164 => (768, 512, 1184, 1024), // Lakeside County, Ascalon City, Ashford Abbey
        147 => (768, 192, 1312, 672),              // The Northlands
        160 | 163 => (288, 576, 864, 1024),        // Green Hills County, Barradin Estate
        161 | 165 => (448, 928, 960, 1440),        // Wizard's Folly, Foible's Fair
        162 | 166 => (864, 928, 1536, 1344),       // Regent Valley, Fort Ranik
        _ => return None,
    };
    Some((x0 / 32, x1 / 32, y0 / 32, y1 / 32))
}

/// One map from each distinct pre-searing rectangle: Lakeside (with Ascalon City and Ashford Abbey),
/// the Northlands, Green Hills (with Barradin), Wizard's Folly (with Foible's Fair) and Regent
/// Valley (with Fort Ranik). Between them they cover every place in pre-searing that is on the world
/// map; the Catacombs are not, and have no rectangle.
pub const PRE_SEARING_MAPS: [i32; 5] = [146, 147, 160, 161, 162];

/// Every block of every pre-searing rectangle: what "all of pre-searing explored" is.
pub fn pre_searing_union(dims: (u32, u32)) -> Grid {
    let mut grid = Grid::empty(dims);
    for map in PRE_SEARING_MAPS {
        let (x0, x1, y0, y1) = map_rect(map).expect("every pre-searing map has a rectangle");
        for y in y0..y1 {
            for x in x0..x1 {
                grid.explore(x, y);
            }
        }
    }
    grid
}

/// The marks that reveal what `grid` holds across **all** of pre-searing, not just the map the player
/// is on: [`replay_centres`] for each rectangle, in [`PRE_SEARING_MAPS`] order, without repeats.
///
/// Every mark is inside its own map's rectangle and inside the envelope, the same as a walking mark.
/// What is not established yet is whether the client accepts a mark for a map other than the one it
/// is on: the run that said it did not used the mirrored rectangles, so it put those marks outside
/// every map. This is the measurement.
pub fn reveal_centres(grid: &Grid) -> Vec<(u32, u32)> {
    let mut seen = std::collections::HashSet::new();
    PRE_SEARING_MAPS
        .iter()
        .flat_map(|map| replay_centres(grid, map_rect(*map).expect("a pre-searing rectangle")))
        .filter(|centre| seen.insert(*centre))
        .collect()
}

/// The block rectangle a pre-searing map *file* occupies, as `x0..x1, y0..y1` with the upper bounds
/// exclusive - what `/reveal` covers. Every map in one file shares one rectangle, so this is
/// [`map_rect`] for any map in the file. The Catacombs' file has none (see [`map_rect`]).
pub fn pre_searing_footprint(file_id: u32) -> Option<(u32, u32, u32, u32)> {
    let map_id = match file_id {
        113021 => 146,
        116025 => 147,
        113190 => 160,
        113355 => 161,
        113437 => 162,
        // The Ascalon City arena, as `tools/presearing-cartography.py` measured it; not in the
        // client's table, so it keeps that measurement.
        380710 => return Some((25, 35, 16, 33)),
        _ => return None,
    };
    map_rect(map_id)
}

/// How far a map's own game coordinates sit from Lakeside's, in game units.
///
/// **Keyed by map, not by file.** It was keyed by file, which only worked for Lakeside, Ascalon City
/// and Ashford Abbey: the server's own `map_config` names the other maps by the blob ids its path
/// graphs use - Regent Valley and Fort Ranik `380714`, Wizard's Folly and Foible's Fair `380647`,
/// Green Hills and Barradin `380713`, the Northlands `2147871382` - so the `113xxx` ids found
/// nothing and walking there revealed nothing (measured 2026-11-05: exploration stopped at
/// Lakeside's edge). The maps of one file share one frame, so they share one shift.
///
/// Solved from the doorways two maps share: a doorway is one physical arch, so in the shared frame
/// both ends coincide, and `shift(B) - shift(A) = position in A - position in B`. Every pair agrees,
/// and every shift is a whole number of 3,072-unit tiles. They are exactly the negation of the
/// offsets in `tools/presearing-cartography.py`, and the client's rectangles in [`map_rect`] agree
/// with these, not with those.
pub fn map_shift(map_id: i32) -> Option<(f64, f64)> {
    Some(match map_id {
        146 | 148 | 164 => (0.0, 0.0), // Lakeside, Ascalon City, Ashford Abbey: the frame
        147 => (6144.0, 30720.0),      // the Northlands, north
        160 | 163 => (-36864.0, -3072.0), // Green Hills, Barradin: west
        161 | 165 => (-24576.0, -39936.0), // Wizard's Folly, Foible's Fair: south-west
        162 | 166 => (21504.0, -36864.0), // Regent Valley, Fort Ranik: south-east
        145 => (-30720.0, -9216.0),    // the Catacombs: west (no rectangle; never marked)
        _ => return None,
    })
}

/// Game units in one world-map unit (WorldCompletion's `kGwinchesPerWorldUnit`).
pub const UNITS_PER_WORLD_UNIT: f64 = 96.0;

/// World-map units in one block (WorldCompletion's `kWorldUnitsPerCell`): a block is 3,072 game
/// units square.
pub const WORLD_UNITS_PER_BLOCK: f64 = 32.0;

/// Lakeside's world-map anchor, from map 148 as the client itself measured it (Rurik
/// `studies/minimap/FINDINGS.md` 6c.4): its world-map rectangle is `(768, 512, 1184, 1024)` and its
/// file's game rectangle is `(-18432, -24576, 21504, 24576)`. WorldCompletion's anchor is
/// `rect.min_x - start.x / 96` and `rect.min_y + end.y / 96 + 1`.
pub const LAKESIDE_ANCHOR: (f64, f64) = (768.0 + 18432.0 / 96.0, 512.0 + 24576.0 / 96.0 + 1.0);

/// The continent block a player stands in, or `None` when the map has no rectangle on the world map
/// ([`map_rect`]) or the position falls outside it.
///
/// WorldCompletion's transform, verbatim (`plugins/WorldCompletion/WorldCompletion.cpp`):
///
/// ```text
/// world.x =  game.x / 96 + anchor.x          block.x = floor(world.x / 32)
/// world.y = -game.y / 96 + anchor.y          block.y = ceil(world.y / 32) - 1
/// ```
///
/// Two details matter and neither is guessable. **Y is flipped**: game `+y` is north (Lakeside's
/// door to the Northlands is at `y = +13494`, its southern exits near `-20000`), and the world map
/// draws north at the top, at the low block rows. And the y block is `ceil - 1`, not `floor`. The
/// footprint rectangles cannot check either - Lakeside's game rectangle is symmetric about `y = 0`,
/// so it maps onto the same rows either way up - which is how a formula without the flip produced
/// the right rectangle and would have put every player upside down inside it.
pub fn block_at(map_id: i32, position: (f32, f32)) -> Option<(u32, u32)> {
    let shift = map_shift(map_id)?;
    let (x0, x1, y0, y1) = map_rect(map_id)?;
    let game_x = f64::from(position.0) + shift.0;
    let game_y = f64::from(position.1) + shift.1;
    let world_x = game_x / UNITS_PER_WORLD_UNIT + LAKESIDE_ANCHOR.0;
    let world_y = -game_y / UNITS_PER_WORLD_UNIT + LAKESIDE_ANCHOR.1;
    let x = (world_x / WORLD_UNITS_PER_BLOCK).floor();
    let y = (world_y / WORLD_UNITS_PER_BLOCK).ceil() - 1.0;
    // Exclusive upper bounds: the client's rectangle ends *before* x1 and y1.
    let inside = x >= f64::from(x0) && x < f64::from(x1) && y >= f64::from(y0) && y < f64::from(y1);
    inside.then_some((x as u32, y as u32))
}

/// The position to mark for a player at `block`: the block itself, pulled inside the envelope that
/// live marks are proven safe in.
///
/// **One mark outside the map's rectangle is an immediate `007`**, so this does not reason about
/// where a mark's window reaches - which is not settled: the builder calls the third field a half
/// span, Rurik's assert reads it as a span starting at `x`. Instead it keeps every mark at a position
/// `/reveal` has already sent live without harm. `/reveal`'s windows over Lakeside sent positions
/// from the rectangle's first block to two short of its exclusive end (`x0..=x1 - 2`), and one
/// further block in from each side keeps a mark inside the rectangle under *both* readings: as a
/// centre (`p - 1..=p + 1`) and as a corner (`p..=p + 2`). So positions are `x0 + 1..=x1 - 3`, and
/// the same in y - a strict subset of what has been seen to work.
///
/// The cost is the outermost column and row on the far sides, which a mark never reaches. That is
/// the price of never sending one the client has not already accepted.
fn mark_centre(block: (u32, u32), rect: (u32, u32, u32, u32)) -> Option<(u32, u32)> {
    let (x0, x1, y0, y1) = rect;
    if x1 < x0 + 4 || y1 < y0 + 4 {
        return None; // too small to hold one proven-safe mark
    }
    Some((block.0.clamp(x0 + 1, x1 - 3), block.1.clamp(y0 + 1, y1 - 3)))
}

/// How long the marker waits between two marks: `config/Server/intervals.json`'s
/// `cartography_mark_interval_ms`, or `ASURA_MARK_INTERVAL_MS` for a one-off experiment.
///
/// The file sets the pace. 100 ms was the measured-safe pace, 2 ms loaded a 15-mark replay without
/// a crash on 2026-11-05, and 1 ms is the next experiment. A true burst still crashed the old client.
pub fn mark_interval() -> std::time::Duration {
    static INTERVAL: std::sync::OnceLock<std::time::Duration> = std::sync::OnceLock::new();
    *INTERVAL.get_or_init(|| {
        std::env::var("ASURA_MARK_INTERVAL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|millis| *millis > 0)
            .map(std::time::Duration::from_millis)
            .unwrap_or_else(|| asura_config::intervals().cartography_mark_interval())
    })
}

/// How long after `INSTANCE_LOAD_FINISH` the replay of a character's stored exploration starts:
/// `config/Server/intervals.json`'s `cartography_replay_delay_ms`.
pub fn replay_start_delay() -> std::time::Duration {
    asura_config::intervals().cartography_replay_delay()
}

/// One player's walking reveal: the block it last marked, and the queue its marks go out through.
///
/// Retail reveals as a character walks by sending `0x008C` for the block it has just entered - the
/// live captures step `x` by one with a half-span of 1 (`[140, 32, 24, 1]`, `[140, 33, 24, 1]`).
/// This does the same: [`Walker::advance`] decides which block to mark and records the window it
/// reveals in the character's grid, which the connection already saves at logout; the marks
/// themselves go through [`spawn_marker`]'s task, one every [`mark_interval`], so walking and the
/// replay of a stored map share one pace and neither blocks the session.
#[derive(Debug, Default)]
pub struct Walker {
    last: Option<(u32, u32)>,
    replayed: bool,
    marks: Option<Marker>,
    /// Whether this connection's init carried the saved grid. If it did, the first step replays only
    /// what [`init_safe`] left out; if not, it replays everything.
    pub init_carried: bool,
    /// Whether the client has asked to spawn - the sign that the load, init included, was accepted.
    pub loaded: bool,
}

/// The handle on one connection's marker task: the queue it reads, and the switch that stops it.
///
/// The switch exists because a queued mark can outlive the map it was meant for. A travel sends the
/// client its redirect straight away, and a mark still waiting its turn would reach the client after
/// that, while it is leaving the map - a message out of place there is a `007`. So the moment a
/// player is moved, [`Walker::halt`] throws the switch and everything still queued is dropped.
#[derive(Debug)]
pub struct Marker {
    queue: tokio::sync::mpsc::UnboundedSender<(u32, u32)>,
    halted: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Walker {
    /// A walker whose marks go out through `marks` (from [`spawn_marker`]). `None` decides and
    /// records without sending anything, which is what a test wants.
    pub fn new(marks: Option<Marker>) -> Self {
        Self {
            last: None,
            replayed: false,
            marks,
            init_carried: false,
            loaded: false,
        }
    }

    /// Queue the replay of everything `grid` holds, across every pre-searing map - the character's
    /// saved progress, so the world map shows all of it. Returns the centres queued, in order.
    ///
    /// Called when the load block ends (`INSTANCE_LOAD_FINISH`) and, as a fallback, on the first
    /// position update, so a client that never sends one still gets the replay. The first caller
    /// wins; later calls queue nothing.
    ///
    /// The whole grid, not just the current map: a mark for a map the player is not on is accepted
    /// (measured 2026-11-05: `/reveal all` revealed every map with no `007`), so this is how the
    /// world map shows everything the character has explored without the init stream carrying it.
    pub fn replay(&mut self, grid: &Grid) -> Vec<(u32, u32)> {
        if self.replayed {
            return Vec::new();
        }
        self.replayed = true;
        let queued = if self.init_carried {
            reveal_centres(&left_out_of_init(grid))
        } else {
            reveal_centres(grid)
        };
        self.queue(&queued);
        queued
    }

    /// The player now stands at `position` in `file_id`: record what that reveals in `grid` and
    /// queue the marks it needs. Returns the centres queued, in order.
    ///
    /// The first call on a map also queues the replay of everything `grid` holds, if the load did
    /// not already do it (`Self::replay`).
    pub fn advance(
        &mut self,
        grid: &mut Grid,
        map_id: i32,
        position: (f32, f32),
    ) -> Vec<(u32, u32)> {
        let Some(rect) = map_rect(map_id) else {
            return Vec::new();
        };
        let mut queued = self.replay(grid);
        if let Some(centre) = block_at(map_id, position).and_then(|block| mark_centre(block, rect))
        {
            if self.last != Some(centre) {
                self.last = Some(centre);
                for y in centre.1 - 1..=centre.1 + 1 {
                    for x in centre.0 - 1..=centre.0 + 1 {
                        grid.explore(x, y);
                    }
                }
                queued.push(centre);
                self.queue(&[centre]);
            }
        }
        queued
    }

    /// Queue marks through this walker's marker, behind whatever is already queued: the paced path
    /// every mark takes, so a command's marks and a walk's never arrive in a burst together.
    pub fn queue(&mut self, centres: &[(u32, u32)]) {
        if let Some(marks) = &self.marks {
            for centre in centres {
                let _ = marks.queue.send(*centre);
            }
        }
    }

    /// Forget which block was marked last, so the next step marks again - after the grid was cleared.
    pub fn forget_position(&mut self) {
        self.last = None;
    }

    /// Stop marking for good on this connection: drop whatever is still queued, unsent.
    ///
    /// For a player who is being moved to another map. What was revealed is already in the grid and
    /// is saved with it; only the marks that would have reached a client that is leaving are lost.
    pub fn halt(&mut self) {
        if let Some(marks) = self.marks.take() {
            marks
                .halted
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// The mark positions that re-reveal what `grid` holds inside `rect`, each window covering at
/// least one stored block that no earlier window has.
///
/// Greedy and row-major: the first stored block not yet covered gets a window whose top-left it is
/// (pulled inside the footprint), and that window's nine blocks are covered. A walked map is a union
/// of exactly such windows, so this rebuilds it with about one mark per window walked.
pub fn replay_centres(grid: &Grid, rect: (u32, u32, u32, u32)) -> Vec<(u32, u32)> {
    let (x0, x1, y0, y1) = rect;
    let mut covered = std::collections::HashSet::new();
    let mut centres = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            if !grid.is_explored(x, y) || covered.contains(&(x, y)) {
                continue;
            }
            let Some(centre) = mark_centre((x + 1, y + 1), rect) else {
                return centres;
            };
            let mut adds = false;
            for at_y in centre.1 - 1..=centre.1 + 1 {
                for at_x in centre.0 - 1..=centre.0 + 1 {
                    adds |= covered.insert((at_x, at_y));
                }
            }
            // A stored block in the far column or row no envelope mark reaches: covering it is not
            // possible, so it is skipped rather than marked again and again.
            if adds {
                centres.push(centre);
            }
            covered.insert((x, y));
        }
    }
    centres
}

/// The task that turns queued centres into `0x008C` marks on the player's own outbox, one every
/// [`mark_interval`].
///
/// The outbox is the channel the session loop already forwards to the client (`connection::run`),
/// so the marks interleave with everything else the client is sent without the session ever
/// waiting on them. The task ends by itself when either side closes: the walker dropping its
/// sender, or the connection dropping the outbox's receiver.
pub fn spawn_marker(outbox: tokio::sync::mpsc::UnboundedSender<Vec<Frame>>) -> Marker {
    let (queue, mut queued) = tokio::sync::mpsc::unbounded_channel::<(u32, u32)>();
    let halted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = halted.clone();
    tokio::spawn(async move {
        while let Some((x, y)) = queued.recv().await {
            // Checked before every send, not once: a halt between two marks must stop the second.
            if stop.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            if outbox
                .send(vec![game::map_exploration_mark(x, y, 1)])
                .is_err()
            {
                break;
            }
            tokio::time::sleep(mark_interval()).await;
        }
    });
    Marker { queue, halted }
}

/// The 3 x 3 window origins that cover one map's footprint, staying inside the rectangle.
fn footprint_positions(file_id: u32, dims: (u32, u32)) -> Vec<(u32, u32)> {
    let Some((x0, x1, y0, y1)) = pre_searing_footprint(file_id) else {
        return Vec::new();
    };
    let x1 = x1.min(dims.0);
    let y1 = y1.min(dims.1);
    if x1 < x0 + 2 || y1 < y0 + 2 {
        return Vec::new();
    }
    let mut xs = Vec::new();
    let mut x = x0;
    while x + 2 < x1 {
        xs.push(x);
        x += 3;
    }
    xs.push(x1 - 2);
    xs.dedup();
    let mut ys = Vec::new();
    let mut y = y0;
    while y + 2 < y1 {
        ys.push(y);
        y += 3;
    }
    ys.push(y1 - 2);
    ys.dedup();

    let mut positions = Vec::new();
    for y in ys {
        for x in &xs {
            positions.push((*x, y));
        }
    }
    positions
}

/// The marks that cover one map's footprint in 3 x 3 windows, staying inside the rectangle.
pub fn footprint_marks(file_id: u32, dims: (u32, u32)) -> Vec<Frame> {
    footprint_positions(file_id, dims)
        .into_iter()
        .map(|(x, y)| game::map_exploration_mark(x, y, 1))
        .collect()
}

/// The marks that cover every pre-searing map footprint, deduplicated where rectangles overlap.
pub fn union_footprint_marks(dims: (u32, u32)) -> Vec<Frame> {
    let mut seen = std::collections::HashSet::new();
    let mut frames = Vec::new();
    for file_id in [113021, 116016, 116025, 113190, 113355, 113437, 380710] {
        for (x, y) in footprint_positions(file_id, dims) {
            if seen.insert((x, y)) {
                frames.push(game::map_exploration_mark(x, y, 1));
            }
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every pre-searing doorway (migration 0025): `(file, map, x, y)` in its own map's game units.
    const DOORWAYS: &[(u32, u32, f32, f32)] = &[
        (113021, 146, 7378.0, 5429.0),
        (113021, 146, 4384.0, -19844.0),
        (113021, 146, -13897.0, -20108.0),
        (113021, 146, -5488.0, 13494.0),
        (113021, 146, -14540.0, 10054.0),
        (113021, 146, -11309.0, -6226.0),
        (113021, 148, 7378.0, 5429.0),
        (113021, 164, -11309.0, -6226.0),
        (113021, 164, -14157.0, -7083.0),
        (113021, 164, -15986.0, -7065.0),
        (116025, 147, -11632.0, -17226.0),
        (116025, 147, 26113.0, 4767.0),
        (113190, 160, 22324.0, 13126.0),
        (113190, 160, -7361.0, 1435.0),
        (113190, 160, -6793.0, -16845.0),
        (113190, 160, -3250.0, 9209.0),
        (113190, 163, -7361.0, 1435.0),
        (113355, 161, 10679.0, 19828.0),
        (113355, 161, -19081.0, 20019.0),
        (113355, 161, 426.0, 7688.0),
        (113355, 161, 20040.0, -10113.0),
        (113355, 161, -5466.0, 18901.0),
        (113355, 165, 426.0, 7688.0),
        (113437, 162, -17120.0, 17020.0),
        (113437, 162, -26040.0, -13185.0),
        (113437, 162, 22595.0, 7257.0),
        (113437, 166, 22595.0, 7257.0),
        (116016, 145, 14904.0, 2095.0),
        (116016, 145, 16563.0, 2133.0),
        (116016, 145, -9648.0, 15525.0),
        (116016, 145, 5961.0, -22656.0),
    ];

    /// **The check the rectangles rest on.** A map's doorways stand on its own ground, so each must
    /// land inside its own map's rectangle - the client's, with exclusive upper bounds. All 27 that are
    /// on the world map do. The Catacombs' four are not on the world map and land nowhere.
    #[test]
    fn every_doorway_lands_inside_its_own_maps_rectangle() {
        assert_eq!(DOORWAYS.len(), 31);
        let mut inside = 0;
        for &(_file, map, x, y) in DOORWAYS {
            let found = block_at(map as i32, (x, y));
            if map == 145 {
                assert_eq!(found, None, "the Catacombs are not on the world map");
            } else {
                assert!(
                    found.is_some(),
                    "map {map}'s doorway at ({x}, {y}) is outside its rectangle"
                );
                inside += 1;
            }
        }
        assert_eq!(inside, 27);
    }

    /// `/reveal all` is every pre-searing rectangle, and its marks stay inside the envelope of
    /// whichever rectangle they belong to.
    #[test]
    fn all_of_pre_searing_is_every_rectangle() {
        let all = pre_searing_union(TYRIA);
        for map in [146, 147, 148, 160, 161, 162, 163, 164, 165, 166] {
            let (x0, x1, y0, y1) = map_rect(map).unwrap();
            assert!(
                all.is_explored(x0, y0) && all.is_explored(x1 - 1, y1 - 1),
                "map {map}"
            );
        }
        assert!(
            !all.is_explored(0, 0) && !all.is_explored(63, 127),
            "nothing off the maps"
        );
        let centres = reveal_centres(&all);
        assert!(
            !centres.is_empty() && centres.len() < 300,
            "{} marks",
            centres.len()
        );
        for &(cx, cy) in &centres {
            let home = PRE_SEARING_MAPS.iter().any(|map| {
                let (x0, x1, y0, y1) = map_rect(*map).unwrap();
                cx >= x0 + 1 && cx + 3 <= x1 && cy >= y0 + 1 && cy + 3 <= y1
            });
            assert!(home, "({cx}, {cy}) is in no rectangle's envelope");
        }
        assert!(
            reveal_centres(&Grid::empty(TYRIA)).is_empty(),
            "nothing explored, nothing marked"
        );
    }

    /// A refused init falls back by itself, and a spawn clears the refusal.
    #[test]
    fn a_refused_init_falls_back_and_a_spawn_clears_it() {
        let character = 0x1234_5678_9abc_def0_1234_5678_9abc_def0u128;
        assert!(!init_refused(character));
        note_init_refused(character);
        assert!(
            init_refused(character),
            "the next load sends the empty grid"
        );
        note_init_accepted(character);
        assert!(
            !init_refused(character),
            "and a spawn puts the saved grid back"
        );
    }

    /// With the saved grid in the init, a load replays only what the init had to leave out.
    #[test]
    fn with_the_grid_in_the_init_only_the_left_out_blocks_are_replayed() {
        let mut grid = Grid::empty(TYRIA);
        for y in 16..32 {
            for x in 24..37 {
                grid.explore(x, y);
            }
        }
        let left = left_out_of_init(&grid);
        // Band 1's last row is row 31; its last 32 blocks are x 32..63.
        assert_eq!(left.explored_count(), 5, "row 31, x 32..36");
        assert!(left.is_explored(32, 31) && !left.is_explored(31, 31) && !left.is_explored(32, 30));
        let mut walker = Walker::new(None);
        walker.init_carried = true;
        let queued = walker.advance(&mut grid, 146, (9826.0, 8077.0));
        assert_eq!(queued.len(), reveal_centres(&left).len() + 1);
    }

    /// **The init rule.** The client refused every init stream with a band whose runs reach 1,024 bits,
    /// and ArenaNet's own stream re-encoded by [`encode_grid`] does exactly that. [`init_safe`] never
    /// lets a band reach it, for any grid, and leaves retail's own picture untouched.
    #[test]
    fn the_init_never_fills_a_band() {
        fn band_sums(stream: &[u8]) -> Vec<usize> {
            let mut sums = Vec::new();
            let mut at = 0;
            while at + 2 <= stream.len() {
                let length = usize::from(u16::from_le_bytes([stream[at], stream[at + 1]]));
                let body = &stream[at + 2..at + 2 + length];
                at += 2 + length;
                let (mut sum, mut cursor) = (0, 0);
                while cursor < body.len() {
                    loop {
                        let byte = body[cursor];
                        cursor += 1;
                        sum += usize::from(byte);
                        if byte != 0xFF || cursor == body.len() {
                            break;
                        }
                    }
                }
                sums.push(sum);
            }
            sums
        }
        let retail = decode_grid(&RETAIL_STREAM, TYRIA);
        assert!(
            band_sums(&RETAIL_STREAM).iter().all(|s| *s < 1024),
            "retail stops short"
        );
        let mut full = Grid::empty(TYRIA);
        for y in 0..TYRIA.1 {
            for x in 0..TYRIA.0 {
                full.explore(x, y);
            }
        }
        assert!(
            band_sums(&encode_grid(&full)).contains(&1024),
            "the old path fills a band"
        );
        let mut walked = retail.clone();
        for y in 16..33 {
            for x in 24..38 {
                walked.explore(x, y);
            }
        }
        for grid in [&retail, &full, &walked] {
            let sums = band_sums(&encode_grid(&init_safe(grid)));
            assert!(sums.iter().all(|s| *s <= 992), "{sums:?}");
        }
        assert_eq!(
            init_safe(&retail),
            retail,
            "retail's own picture is untouched"
        );
    }

    /// Every pre-searing spawn point the server uses (`tools/map-config.sql`) lands inside its own map's
    /// rectangle, looked up by **map id**. The server names most of these maps by path-graph blob ids
    /// (`380714`, `380647`, ...), not the `113xxx` files, which is why a lookup by file found nothing
    /// outside Lakeside and walking there revealed nothing.
    #[test]
    fn every_spawn_lands_inside_its_maps_rectangle() {
        let spawns: &[(i32, f32, f32)] = &[
            (146, 6602.0, 4485.0),         // Lakeside County
            (147, -11679.181, -16305.796), // The Northlands
            (148, 9541.873, 4796.456),     // Ascalon City
            (160, -7492.531, 1454.671),    // Green Hills County
            (161, 633.0, 7270.0),          // Wizard's Folly
            (162, 22614.0, 6849.0),        // Regent Valley
            (163, -6479.301, 1795.212),    // The Barradin Estate
            (164, -12078.813, -8452.183),  // Ashford Abbey
            (165, 12.172, 9910.412),       // Foible's Fair
            (166, 22697.885, 10275.67),    // Fort Ranik
        ];
        for &(map, x, y) in spawns {
            assert!(
                block_at(map, (x, y)).is_some(),
                "map {map}'s spawn ({x}, {y}) is off its map"
            );
        }
        assert_eq!(
            block_at(145, (14442.0, 2135.0)),
            None,
            "the Catacombs have no rectangle"
        );
    }

    /// The client's rectangle ends **before** its upper bound. Lakeside's southernmost game row
    /// computes to block row 32, which is outside `16..32` - and a mark there is a `007`.
    #[test]
    fn the_rectangles_upper_edge_is_exclusive() {
        assert_eq!(map_rect(146), Some((24, 37, 16, 32)));
        assert_eq!(
            block_at(146, (-18432.0, 24575.0)),
            Some((24, 16)),
            "north-west corner"
        );
        assert_eq!(
            block_at(146, (21503.0, -24575.0)),
            None,
            "row 32 is outside"
        );
        assert_eq!(
            block_at(146, (21503.0, -24000.0)),
            Some((36, 31)),
            "the last row inside"
        );
    }

    /// The x ranges are the client's, and they are the doorway-solved ones: the old table had them
    /// mirrored about Lakeside for every other map.
    #[test]
    fn the_client_rectangles_are_the_corrected_ones() {
        assert_eq!(map_rect(160), Some((9, 27, 18, 32)), "Green Hills, west");
        assert_eq!(
            map_rect(161),
            Some((14, 30, 29, 45)),
            "Wizard's Folly, south-west"
        );
        assert_eq!(
            map_rect(162),
            Some((27, 48, 29, 42)),
            "Regent Valley, south-east"
        );
        assert_eq!(
            map_rect(147),
            Some((24, 41, 6, 21)),
            "the Northlands, north"
        );
        assert_eq!(map_rect(145), None, "the Catacombs");
        assert_eq!(
            map_rect(164),
            map_rect(146),
            "Ashford Abbey shares Lakeside's rectangle"
        );
    }

    /// North is up: Lakeside's door to the Northlands sits above its door to Regent Valley, and both
    /// ends of that northern doorway are one block.
    #[test]
    fn north_is_the_low_rows() {
        let north = block_at(146, (-5488.0, 13494.0)).expect("the Northlands door");
        let south = block_at(146, (4384.0, -19844.0)).expect("the Regent Valley door");
        assert!(north.1 < south.1, "{north:?} should be above {south:?}");
        let northlands = block_at(147, (-11632.0, -17226.0)).expect("its other end");
        assert_eq!(northlands, north);
    }

    /// The envelope: every position a mark can take is one `/reveal` has already sent live, and its
    /// window stays inside the rectangle whether the client reads the position as a centre or a corner.
    #[test]
    fn every_mark_position_is_inside_the_proven_envelope() {
        for map in [146, 147, 160, 161, 162] {
            let (x0, x1, y0, y1) = map_rect(map).unwrap();
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = mark_centre((x, y), (x0, x1, y0, y1)).unwrap();
                    // As /reveal sent them: x0..=x1-2.
                    assert!(px >= x0 && px + 2 <= x1 && py >= y0 && py + 2 <= y1);
                    // As a centre: p-1..=p+1 inside.
                    assert!(px - 1 >= x0 && px + 1 < x1 && py - 1 >= y0 && py + 1 < y1);
                    // As a corner: p..=p+2 inside.
                    assert!(px + 2 < x1 && py + 2 < y1, "map {map}: ({px}, {py})");
                }
            }
        }
    }

    /// Walking marks once per new position, records what it marks, and does nothing on a map with no
    /// rectangle or off the map.
    #[test]
    fn walking_marks_new_positions_and_records_them() {
        let mut grid = Grid::empty(TYRIA);
        let mut walker = Walker::new(None);
        let first = walker.advance(&mut grid, 146, (9826.0, 8077.0));
        assert_eq!(
            first,
            vec![(33, 21)],
            "the spawn's own block, nothing to replay"
        );
        assert_eq!(grid.explored_count(), 9, "a 3 x 3 window");
        assert!(
            walker.advance(&mut grid, 146, (9900.0, 8100.0)).is_empty(),
            "same block"
        );
        let next = walker.advance(&mut grid, 146, (9826.0 + 3072.0, 8077.0));
        assert_eq!(next, vec![(34, 21)], "one block east");
        // At the far edge the position is held inside the envelope.
        let edge = walker.advance(&mut grid, 146, (21503.0, -24000.0));
        assert_eq!(edge, vec![(34, 29)]);
        assert!(
            walker.advance(&mut grid, 146, (90000.0, 0.0)).is_empty(),
            "off the map"
        );
        assert!(
            walker.advance(&mut grid, 145, (0.0, 0.0)).is_empty(),
            "the Catacombs"
        );
    }

    /// The replay of a stored map stays inside the envelope, covers everything an envelope mark can
    /// reach, and goes out once, ahead of the first walking mark.
    #[test]
    fn a_stored_map_is_replayed_inside_the_envelope() {
        let rect = map_rect(146).unwrap();
        let (x0, x1, y0, y1) = rect;
        let mut grid = Grid::empty(TYRIA);
        for y in y0..y1 {
            for x in x0..x1 {
                grid.explore(x, y);
            }
        }
        grid.explore(5, 5); // outside the rectangle: stored, never replayed here
        let centres = replay_centres(&grid, rect);
        let mut covered = std::collections::HashSet::new();
        for &(cx, cy) in &centres {
            assert!(cx >= x0 + 1 && cx + 3 <= x1 && cy >= y0 + 1 && cy + 3 <= y1);
            for y in cy - 1..=cy + 1 {
                for x in cx - 1..=cx + 1 {
                    covered.insert((x, y));
                }
            }
        }
        for y in y0..y1 - 1 {
            for x in x0..x1 - 1 {
                assert!(covered.contains(&(x, y)), "({x}, {y}) was not replayed");
            }
        }
        assert!(centres.len() <= 40, "{} marks for one map", centres.len());

        let mut walker = Walker::new(None);
        let queued = walker.advance(&mut grid, 146, (9826.0, 8077.0));
        assert_eq!(
            queued.len(),
            reveal_centres(&grid).len() + 1,
            "the replay of every map, then the walking mark"
        );
        let again = walker.advance(&mut grid, 146, (9826.0 + 6144.0, 8077.0));
        assert_eq!(again.len(), 1, "replayed once");
    }

    /// **The referee.** Retail's own stream, read by this module, against three things the client
    /// itself proved: two blocks whose state was measured by what marking them did to the screen,
    /// and the size of the revealed region it draws on the M press. Of the four candidate readings
    /// of the format only one reproduces all three, and it is the one [`decode_grid`] implements - so
    /// this test is what would catch the run-length rule or the polarity being wrong again.
    #[test]
    fn retails_own_stream_decodes_to_the_blocks_the_client_proved() {
        let grid = decode_grid(&RETAIL_STREAM, TYRIA);

        // Marking (30, 24) changed 0 of 2,013,440 px: it was already explored.
        assert!(grid.is_explored(30, 24), "block (30,24) is set");
        // Marking (26, 22) revealed 1,059 px: it was not.
        assert!(!grid.is_explored(26, 22), "block (26,22) is clear");
        // The "large revealed region" on the M press, counted.
        assert_eq!(grid.explored_count(), 947, "947 blocks explored");
    }

    /// The band chain of that same stream closes exactly at the byte count it declared - the
    /// structural half of the referee, and what rules out the rival band heights and header widths.
    #[test]
    fn retails_own_band_chain_closes_at_its_declared_count() {
        let mut at = 0usize;
        let mut chain = Vec::new();
        while at + 2 <= RETAIL_STREAM.len() {
            let length = u16::from_le_bytes([RETAIL_STREAM[at], RETAIL_STREAM[at + 1]]);
            chain.push(length);
            at += 2 + usize::from(length);
        }
        assert_eq!(
            chain,
            vec![0, 22, 0, 0, 0, 0, 0, 0],
            "eight bands, one of them 22 bytes"
        );
        assert_eq!(at, 38, "the chain closes at exactly the declared 38 bytes");
    }

    /// And this module writes what the client wrote: retail's stream, decoded and re-encoded, is the
    /// same grid again. An encoder that disagreed with the client about a run length or the polarity
    /// could not survive a round trip through ArenaNet's own bytes.
    #[test]
    fn retails_own_stream_re_encodes_to_the_same_grid() {
        let grid = decode_grid(&RETAIL_STREAM, TYRIA);
        let ours = encode_grid(&grid);
        assert_eq!(
            decode_grid(&ours, TYRIA),
            grid,
            "re-encoding keeps every block"
        );
        assert!(
            ours.len() <= RETAIL_STREAM.len(),
            "and is no longer than retail's own {} bytes, not {}",
            RETAIL_STREAM.len(),
            ours.len()
        );
    }

    /// Every grid below goes out through [`encode_grid`] and comes back through [`decode_grid`].
    fn round_trips(grid: &Grid) -> Vec<u8> {
        let stream = encode_grid(grid);
        assert_eq!(
            &decode_grid(&stream, grid.dims()),
            grid,
            "encode and decode disagree"
        );
        stream
    }

    #[test]
    fn an_empty_grid_is_one_empty_band_a_row_of_sixteen() {
        let stream = round_trips(&Grid::empty(TYRIA));
        assert_eq!(stream, unexplored_blob(TYRIA));
        assert_eq!(stream.len(), 8 * 2, "eight empty bands");
        assert!(stream.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn a_full_grid_is_one_natural_run_a_band() {
        let grid = Grid::all_explored(TYRIA);
        let stream = round_trips(&grid);
        // Per band: one run of 1,024 bits, four 0xFF continuations and a 0x04 terminator.
        assert_eq!(stream.len(), 8 * (2 + 5), "five body bytes a band");
        assert_eq!(&stream[..2], &[5, 0], "band 0's declared length");
        assert_eq!(&stream[2..7], &[0xFF, 0xFF, 0xFF, 0xFF, 0x04]);
        assert_eq!(grid.explored_count(), (TYRIA.0 * TYRIA.1) as usize);
    }

    /// `all` sends only what the client can write: band 1's first 992 bits, not a full 1,024-bit
    /// band and not bands this pre-searing server cannot draw.
    #[test]
    fn known_good_is_the_retail_region() {
        let grid = Grid::known_good_revealed(TYRIA);
        assert_eq!(grid.explored_count(), 947);
        assert!(grid.is_explored(30, 24), "the block the client proved set");
        assert!(
            !grid.is_explored(26, 22),
            "the block the client proved clear"
        );

        let stream = round_trips(&grid);
        assert!(
            stream.len() <= RETAIL_STREAM.len(),
            "the re-encoded retail region is no longer than its own {} bytes",
            RETAIL_STREAM.len()
        );
    }

    #[test]
    fn reveal_marks_are_3x3_windows() {
        let mut grid = Grid::empty(TYRIA);
        assert!(grid.explore(30, 24));
        let frames = mark_frames(&grid);
        assert_eq!(frames.len(), 1, "one window covers the block");
        assert_eq!(frames[0].header(), game::MAP_EXPLORATION_MARK);
        assert_eq!(frames[0].len(), 11, "header, x, y, half-span");
    }

    #[test]
    fn the_lakeside_footprint_is_thirty_marks() {
        let frames = footprint_marks(113021, TYRIA);
        assert_eq!(
            frames.len(),
            30,
            "five columns by six rows of 3 x 3 windows"
        );
        assert!(frames
            .iter()
            .all(|frame| frame.header() == game::MAP_EXPLORATION_MARK));
        assert!(frames.iter().all(|frame| frame.len() == 11));
    }

    #[test]
    fn the_union_footprint_marks_cover_all_maps() {
        let frames = union_footprint_marks(TYRIA);
        assert!(frames.len() > 100, "the six footprints need many windows");
        assert!(frames
            .iter()
            .all(|frame| frame.header() == game::MAP_EXPLORATION_MARK));
        assert!(frames.iter().all(|frame| frame.len() == 11));
    }

    #[test]
    fn a_single_block_survives_the_round_trip() {
        for (x, y) in [
            (0, 0),
            (63, 0),
            (0, 127),
            (63, 127),
            (31, 64),
            (32, 15),
            (30, 24),
        ] {
            let mut grid = Grid::empty(TYRIA);
            assert!(grid.explore(x, y), "a fresh block changes");
            assert!(
                !grid.explore(x, y),
                "an explored block does not change again"
            );
            round_trips(&grid);
            assert_eq!(grid.explored_count(), 1, "({x},{y}) and nothing else");
        }
    }

    /// The partial-dword drop is the edge an encoder loses blocks to, so it gets its own test: a
    /// single explored block in every one of a band's 32-bit positions, including the last.
    #[test]
    fn a_block_in_any_dword_position_survives_the_round_trip() {
        for at in 0..BITS_PER_FLUSH as u32 * 3 {
            let mut grid = Grid::empty(TYRIA);
            grid.explore(at % TYRIA.0, at / TYRIA.0);
            round_trips(&grid);
        }
        // And the very last block of a band, whose dword is the one the client would drop.
        let mut last = Grid::empty(TYRIA);
        last.explore(TYRIA.0 - 1, BAND_ROWS - 1);
        round_trips(&last);
        assert!(last.is_explored(TYRIA.0 - 1, BAND_ROWS - 1));
    }

    #[test]
    fn a_span_and_a_disc_paint_what_they_say() {
        let mut grid = Grid::empty(TYRIA);
        assert_eq!(grid.explore_span(10, 5, 4), 4);
        assert_eq!(grid.explore_span(10, 5, 4), 0, "again changes nothing");
        assert!(grid.is_explored(13, 5) && !grid.is_explored(14, 5));
        round_trips(&grid);

        let mut disc = Grid::empty(TYRIA);
        assert_eq!(disc.explore_disc(10, 10, 2), 13, "a radius-2 disc");
        assert!(disc.is_explored(10, 12) && !disc.is_explored(12, 12));
        round_trips(&disc);

        let mut corner = Grid::empty(TYRIA);
        corner.explore_disc(0, 0, 3);
        corner.explore_span(TYRIA.0 - 2, 0, 9);
        assert!(
            corner.explored_count() > 0,
            "the edges clamp rather than panic"
        );
        round_trips(&corner);
    }

    /// The runs that exercise the length rules: alternating single blocks, a run longer than 255,
    /// and a band whose very first block is explored against one whose is not.
    #[test]
    fn the_awkward_runs_survive_the_round_trip() {
        let mut stripes = Grid::empty(TYRIA);
        for at in (0..TYRIA.0 * TYRIA.1).step_by(2) {
            stripes.explore(at % TYRIA.0, at / TYRIA.0);
        }
        round_trips(&stripes);

        let mut long = Grid::empty(TYRIA);
        for at in 0..600 {
            long.explore(at % TYRIA.0, at / TYRIA.0);
        }
        round_trips(&long);

        let mut starts_explored = Grid::empty(TYRIA);
        let mut starts_fogged = Grid::empty(TYRIA);
        for band in 0..TYRIA.1 / BAND_ROWS {
            starts_explored.explore(0, band * BAND_ROWS);
            starts_fogged.explore(1, band * BAND_ROWS);
        }
        round_trips(&starts_explored);
        round_trips(&starts_fogged);
    }

    /// A deterministic sweep, because the format's edges are about where runs land rather than what
    /// they contain: dense grids and the sparse, streaky ones a walking player actually produces.
    #[test]
    fn pseudo_random_grids_survive_the_round_trip() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..64 {
            let mut dense = Grid::empty(TYRIA);
            let mut sparse = Grid::empty(TYRIA);
            for y in 0..TYRIA.1 {
                for x in 0..TYRIA.0 {
                    if next() % 2 == 0 {
                        dense.explore(x, y);
                    }
                }
            }
            for _ in 0..40 {
                let at = (next() % u64::from(TYRIA.0 * TYRIA.1)) as u32;
                let span = (next() % 70) as u32 + 1;
                sparse.explore_span(at % TYRIA.0, at / TYRIA.0, span);
            }
            round_trips(&dense);
            round_trips(&sparse);
        }
    }

    /// The other continents' dimensions are not measured, but the format does not care: bands of 16
    /// rows at a width that is a multiple of 32, whatever the numbers are.
    #[test]
    fn other_dimensions_survive_the_round_trip() {
        for dims in [(32, 16), (256, 192), (96, 48), (1024, 16)] {
            round_trips(&Grid::all_explored(dims));
            round_trips(&Grid::empty(dims));
            let mut sparse = Grid::empty(dims);
            sparse.explore_disc((dims.0 / 2) as i64, (dims.1 / 2) as i64, 5);
            round_trips(&sparse);
        }
    }

    /// The stored form round-trips, which is the other half of keeping a character's progress: the
    /// wire codec above puts a grid on the wire, and this puts the same grid in a database row.
    #[test]
    fn the_packed_form_round_trips() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut grids = vec![
            Grid::empty(TYRIA),
            Grid::all_explored(TYRIA),
            decode_grid(&RETAIL_STREAM, TYRIA),
        ];
        for _ in 0..16 {
            let mut grid = Grid::empty(TYRIA);
            for _ in 0..50 {
                let at = (next() % u64::from(TYRIA.0 * TYRIA.1)) as u32;
                grid.explore_span(at % TYRIA.0, at / TYRIA.0, (next() % 40) as u32 + 1);
            }
            grids.push(grid);
        }
        for grid in grids {
            let packed = grid.packed();
            assert_eq!(
                packed.len(),
                64 * 128 / 8,
                "one bit a block: 1,024 bytes for Tyria"
            );
            assert_eq!(
                Grid::from_packed(TYRIA, &packed).as_ref(),
                Some(&grid),
                "the stored form lost a block"
            );
        }
    }

    /// A single block in each of the first sixteen positions, to pin the bit order rather than only
    /// the round trip: a packer that agreed with its own unpacker about the wrong order would pass
    /// the test above and store something no other reader could make sense of.
    #[test]
    fn the_packed_bit_order_is_lsb_first_and_row_major() {
        for at in 0..16u32 {
            let mut grid = Grid::empty(TYRIA);
            grid.explore(at % TYRIA.0, at / TYRIA.0);
            let packed = grid.packed();
            assert_eq!(packed[at as usize / 8], 1 << (at % 8), "block {at}");
            assert!(
                packed
                    .iter()
                    .enumerate()
                    .all(|(byte, value)| byte == at as usize / 8 || *value == 0),
                "block {at} set a bit in another byte"
            );
        }
        // And the first block of the second row is the width'th bit, which is what "row-major" means.
        let mut second_row = Grid::empty(TYRIA);
        second_row.explore(0, 1);
        assert_eq!(second_row.packed()[TYRIA.0 as usize / 8], 1);
    }

    /// A blob of the wrong length is refused rather than reshaped.
    #[test]
    fn a_packed_blob_of_the_wrong_shape_is_refused() {
        let packed = Grid::all_explored(TYRIA).packed();
        assert!(Grid::from_packed(TYRIA, &packed).is_some());
        assert!(Grid::from_packed(TYRIA, &packed[..packed.len() - 1]).is_none());
        assert!(Grid::from_packed((256, 192), &packed).is_none());
        assert!(Grid::from_packed(TYRIA, &[]).is_none());
    }

    /// The switch overrides what is sent and nothing else, which is the property that keeps a
    /// measurement run from writing a fully explored map into a character's save: `stream_for` takes
    /// the grid by reference and hands back bytes, so there is no path from the override to the grid.
    #[test]
    fn the_override_changes_the_stream_and_not_the_grid() {
        let grid = Grid::empty(TYRIA);
        // With no switch set, which is the state every test here runs in, the stream is the grid's.
        assert_eq!(stream_for(TYRIA, &grid), unexplored_blob(TYRIA));
        assert_eq!(grid.explored_count(), 0, "the grid is untouched either way");
    }

    #[test]
    fn every_map_this_server_serves_is_on_tyria() {
        for map_id in [0, 146, 147, 148, 156, 168, 214, 248] {
            assert_eq!(continent_for(map_id), TYRIA_CONTINENT);
        }
    }

    /// A short or truncated stream is a grid, not a panic: the client clamps and so does this.
    #[test]
    fn a_truncated_stream_decodes_to_what_it_carried() {
        assert_eq!(decode_grid(&[], TYRIA).explored_count(), 0);
        assert_eq!(decode_grid(&[0], TYRIA).explored_count(), 0);
        for cut in 0..RETAIL_STREAM.len() {
            let grid = decode_grid(&RETAIL_STREAM[..cut], TYRIA);
            assert!(
                grid.explored_count() <= 947,
                "a prefix explores no more than the whole"
            );
        }
    }

    #[test]
    fn the_frames_announce_the_stream_they_carry() {
        let stream = unexplored_blob(TYRIA);
        let frames = frames_for(TYRIA, &stream);

        let start = &frames[0];
        assert_eq!(start.header(), game::MAP_INIT_START);
        assert_eq!(start.len(), 14, "three dwords after the header");
        for (at, expected) in [(0, TYRIA.0), (4, TYRIA.1), (8, stream.len() as u32)] {
            assert_eq!(
                u32::from_le_bytes([
                    start.body()[at],
                    start.body()[at + 1],
                    start.body()[at + 2],
                    start.body()[at + 3],
                ]),
                expected
            );
        }

        assert_eq!(frames.len(), 2, "the announcement and one slice");
        let slice = &frames[1];
        assert_eq!(slice.header(), game::CARTOGRAPHY_DATA);
        assert_eq!(slice.len(), 260, "a u32 count, 63 dwords and two pad bytes");
        let count = u32::from_le_bytes([
            slice.body()[0],
            slice.body()[1],
            slice.body()[2],
            slice.body()[3],
        ]);
        assert_eq!(
            count as usize,
            stream.len().div_ceil(4),
            "the dwords that count"
        );
        assert_eq!(
            &slice.body()[4..4 + stream.len()],
            &stream[..],
            "the stream, then padding"
        );
        assert!(
            slice.body()[4 + stream.len()..]
                .iter()
                .all(|byte| *byte == 0),
            "the rest of the fixed payload is zero"
        );
    }

    /// **The frame this client loads.** A fixed 260-byte `0x008A`: a `u32` count at byte 2, the
    /// stream at byte 6, 63 dwords and two pad bytes. A 262-byte frame and a variable frame both
    /// returned `007`; a u16-count frame loaded but shifted the stream one band, so `all` landed in
    /// band 0 and drew nothing. The shape is asserted over a grid of each kind.
    #[test]
    fn every_grid_is_framed_at_the_wire_size_this_client_reads() {
        let mut band_zero_carries_something = Grid::empty(TYRIA);
        band_zero_carries_something.explore(0, 0);
        for grid in [
            Grid::empty(TYRIA),
            Grid::all_explored(TYRIA),
            Grid::known_good_revealed(TYRIA),
            band_zero_carries_something,
            decode_grid(&RETAIL_STREAM, TYRIA),
        ] {
            let stream = encode_grid(&grid);
            for frame in frames_for(TYRIA, &stream) {
                let expected = match frame.header() {
                    game::MAP_INIT_START => 14,
                    game::CARTOGRAPHY_DATA => 260,
                    other => panic!("a grid frame this test does not know: {other:#06X}"),
                };
                assert_eq!(
                    frame.len(),
                    expected,
                    "{:#06X} was built at the wrong wire size",
                    frame.header()
                );
                if frame.header() == game::CARTOGRAPHY_DATA {
                    assert_eq!(
                        u32::from_le_bytes([
                            frame.body()[0],
                            frame.body()[1],
                            frame.body()[2],
                            frame.body()[3],
                        ]) as usize,
                        stream.len().div_ceil(4),
                        "the slice's count"
                    );
                }
            }
        }
    }

    /// Band 0 carries content, so its length must sit after the four-byte count, not inside it.
    /// The count must be 14 (`56 / 4`) and the first band's `05 00` must sit at byte 6.
    #[test]
    fn an_all_explored_grid_puts_band_zero_after_the_count() {
        let stream = encode_grid(&Grid::all_explored(TYRIA));
        assert_eq!(&stream[..2], &[5, 0], "band 0 is a five-byte natural run");
        assert_eq!(&stream[2..7], &[0xFF, 0xFF, 0xFF, 0xFF, 0x04]);
        assert_eq!(stream.len(), 56, "eight bands of two-byte length plus five");

        let frame = &frames_for(TYRIA, &stream)[1];
        assert_eq!(frame.len(), 260, "the fixed frame");
        assert_eq!(&frame.body()[..4], &14u32.to_le_bytes(), "the count");
        assert_eq!(&frame.body()[4..6], &[5, 0], "band 0 after the count");
        assert_eq!(&frame.body()[4..4 + stream.len()], &stream[..]);
    }

    /// A fully explored Tyria still fits one `0x008A`, so the switch changes the stream and not the
    /// shape of the load block. A grid that needed two slices would be a different measurement.
    #[test]
    fn a_full_grid_still_fits_one_slice() {
        let frames = frames_for(TYRIA, &encode_grid(&Grid::all_explored(TYRIA)));
        assert_eq!(frames.len(), 2, "the announcement and one slice");
    }
}
