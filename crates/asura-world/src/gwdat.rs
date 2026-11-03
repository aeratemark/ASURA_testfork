//! The `Gw.dat` archive: the index half of the reference's `FaArchive.c`.
//!
//! `Gw.dat` is the client's asset archive, 3.6 GB of it, and it is where a map's geometry lives: the
//! path graph a server needs to answer "can this agent walk there" is a chunk inside one of its entries.
//! The reference reads it with `FaArchive.c` (204 lines) and decompresses entries with `FaCompress.c`'s
//! `FaDecompress` (an LZ + Huffman decoder).
//!
//! This module is the **index**: the header, the file table and the hash that turns a file id into an
//! entry. That half needs no decompression, so it can be written and tested against the real file before
//! any decoder exists - and it is the half that answers the question the map catalogue already depends on,
//! "is `0x1B97D` in this archive and how big is it".
//!
//! Format, from `FaArchive.h` (little endian, packed):
//!
//! ```text
//! FaHeader   32 bytes  magic ('\x1ANA3'), header_size, block_size, checksum, mft_offset (u64), mft_size, flags
//! MFTHeader  24 bytes  at mft_offset: file_id, h0004, h0008, entry_count, h0010, h0014
//! MFTEntry   24 bytes  entry_count of them, straight after the MFTHeader:
//!                      offset (u64), size, compressed (u16), flags, stream, next_stream, checksum
//! ```
//!
//! The table of contents is itself an entry (`INDEX_TOC = 1`): its bytes are `file_id, file_index` pairs,
//! and that is the hash a client lookup goes through.
//!
//! ## The reference's own off-by-one, kept and documented
//!
//! `FaGetEntry` indexes the entry array with the TOC's `file_index` **as it is**, while `FaGetFile` uses
//! `file_index - 1`. One of the two is wrong; `FaGetFile` is the one that loads maps, so this module
//! follows it. A reader that trusted `FaGetEntry` would fetch its neighbour's bytes.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

/// `FILE_ARCHIVE_MAGIC` (`FaArchive.h:3`): `((uint32_t)'\x1ANA3')` -- a C **multi-character literal**, so
/// the four bytes `1A 4E 41 33` collapse big-endian into `0x1A4E4133`, and that is what the archive's first
/// four bytes read as a little-endian `u32`.
pub const MAGIC: u32 = 0x1A4E_4133;

/// `INDEX_TOC` (`FaArchive.h:8`): the entry whose bytes are the file-id table.
const INDEX_TOC: usize = 1;

/// The 32-byte archive header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub magic: u32,
    pub header_size: u32,
    pub block_size: u32,
    pub checksum: u32,
    pub mft_offset: u64,
    pub mft_size: u32,
    pub flags: u32,
}

/// One 24-byte file-table entry (`MFTEntry`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Where the entry's bytes start in the archive.
    pub offset: u64,
    /// How many bytes are stored there. For a compressed entry this is the **stored** size, whose last
    /// four bytes are the decompressed size.
    pub size: u32,
    /// 0 for stored, non-zero for compressed (`FaArchive.h:38`: "0=stored, 8=compress").
    pub compressed: u16,
    pub flags: u8,
    pub stream: u8,
    pub next_stream: u32,
    pub checksum: u32,
}

/// An opened `Gw.dat`: the header, the file table, and the id-to-index hash.
pub struct GwDat {
    file: File,
    path: String,
    header: Header,
    entries: Vec<Entry>,
    by_id: HashMap<u32, usize>,
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
        bytes[at + 4],
        bytes[at + 5],
        bytes[at + 6],
        bytes[at + 7],
    ])
}

impl GwDat {
    /// Open an archive and build its index.
    ///
    /// Only the header, the file table and the TOC are read - a few megabytes of the 3.6 GB - so opening
    /// is fast and leaves the entries to be fetched one at a time.
    pub fn open(path: &str) -> io::Result<GwDat> {
        let mut file = File::open(path)?;

        let mut header_bytes = [0u8; 32];
        file.read_exact(&mut header_bytes)?;
        let header = Header {
            magic: u32_at(&header_bytes, 0),
            header_size: u32_at(&header_bytes, 4),
            block_size: u32_at(&header_bytes, 8),
            checksum: u32_at(&header_bytes, 12),
            mft_offset: u64_at(&header_bytes, 16),
            mft_size: u32_at(&header_bytes, 24),
            flags: u32_at(&header_bytes, 28),
        };
        if header.magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{path} is not a Gw.dat: expected magic {MAGIC:#010X}, found {:#010X}",
                    header.magic
                ),
            ));
        }

        // The file table: the MFT header, then the entries, read sequentially from `mft_offset`.
        file.seek(SeekFrom::Start(header.mft_offset))?;
        let mut mft_header = [0u8; 24];
        file.read_exact(&mut mft_header)?;
        let entry_count = u32_at(&mft_header, 12) as usize;

        let mut table = vec![0u8; entry_count * 24];
        file.read_exact(&mut table)?;
        let entries: Vec<Entry> = (0..entry_count)
            .map(|index| {
                let at = index * 24;
                Entry {
                    offset: u64_at(&table, at),
                    size: u32_at(&table, at + 8),
                    compressed: u16_at(&table, at + 12),
                    flags: table[at + 14],
                    stream: table[at + 15],
                    next_stream: u32_at(&table, at + 16),
                    checksum: u32_at(&table, at + 20),
                }
            })
            .collect();

        // The TOC lists `(file_id, file_index)` pairs; it is the only way an id becomes an entry.
        let toc = entries
            .get(INDEX_TOC)
            .copied()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no TOC entry"))?;
        let mut toc_bytes = vec![0u8; toc.size as usize];
        file.seek(SeekFrom::Start(toc.offset))?;
        file.read_exact(&mut toc_bytes)?;

        let mut by_id = HashMap::with_capacity(toc_bytes.len() / 8);
        for pair in toc_bytes.chunks_exact(8) {
            let file_id = u32_at(pair, 0);
            let file_index = u32_at(pair, 4);
            if file_index == 0 {
                continue; // the reference skips these too (`FaGetFile`)
            }
            // `FaGetFile` decrements; see the module note.
            by_id.insert(file_id, file_index as usize - 1);
        }

        Ok(GwDat {
            file,
            path: path.to_string(),
            header,
            entries,
            by_id,
        })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// How many entries the file table holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry a file id names, or `None` if the archive does not have it.
    pub fn entry(&self, file_id: u32) -> Option<Entry> {
        self.by_id
            .get(&file_id)
            .and_then(|index| self.entries.get(*index))
            .copied()
    }

    /// Every file id the archive knows, sorted. A census, not a lookup: useful for a test that wants to
    /// know the archive is intact beyond a handful of ids.
    pub fn file_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.by_id.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// The **stored** bytes of an entry, exactly as they sit in the archive.
    ///
    /// A compressed entry's last four bytes are the decompressed size; decoding them is the other half of
    /// `FaArchive.c` (`FaGetFile` calls `FaDecompress`), and that decoder is not ported yet.
    pub fn stored(&mut self, file_id: u32) -> io::Result<Option<Vec<u8>>> {
        let Some(entry) = self.entry(file_id) else {
            return Ok(None);
        };
        let mut bytes = vec![0u8; entry.size as usize];
        self.file.seek(SeekFrom::Start(entry.offset))?;
        self.file.read_exact(&mut bytes)?;
        Ok(Some(bytes))
    }

    /// The **decompressed** bytes of an entry: what `FaGetFile` returns.
    ///
    /// A stored entry is its own content. A compressed one is decoded by `fa_compress::decompress` against
    /// the size in its last four bytes, which is the pair `FaGetFile` performs.
    pub fn file(&mut self, file_id: u32) -> io::Result<Option<Vec<u8>>> {
        let Some(entry) = self.entry(file_id) else {
            return Ok(None);
        };
        let Some(stored) = self.stored(file_id)? else {
            return Ok(None);
        };
        if entry.compressed == 0 {
            return Ok(Some(stored));
        }
        if stored.len() < 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("entry {file_id:#X} is compressed but too short to carry its size"),
            ));
        }
        let announced = u32::from_le_bytes([
            stored[stored.len() - 4],
            stored[stored.len() - 3],
            stored[stored.len() - 2],
            stored[stored.len() - 1],
        ]);
        let content =
            crate::fa_compress::decompress(&stored[..stored.len() - 4], announced as usize)
                .map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("entry {file_id:#X} did not decompress: {err}"),
                    )
                })?;
        Ok(Some(content))
    }

    /// The decompressed size an entry announces, for a compressed entry: the low `u32` of its last four
    /// bytes. `None` for a stored entry, which needs no size.
    pub fn announced_size(&mut self, file_id: u32) -> io::Result<Option<u32>> {
        let Some(entry) = self.entry(file_id) else {
            return Ok(None);
        };
        if entry.compressed == 0 || entry.size < 4 {
            return Ok(None);
        }
        let mut tail = [0u8; 4];
        self.file
            .seek(SeekFrom::Start(entry.offset + u64::from(entry.size) - 4))?;
        self.file.read_exact(&mut tail)?;
        Ok(Some(u32::from_le_bytes(tail)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A local copy of the client archive: `ASURA_GW_DAT`, then `Gw.dat`, then `data/Gw.dat`. Tests
    /// skip when it is absent, the same way the database-backed tests skip without a database, and
    /// never reach for the reference checkout: this repository stands alone.
    fn gw_dat_path() -> Option<String> {
        if let Ok(path) = std::env::var("ASURA_GW_DAT") {
            return Some(path);
        }
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()?
            .parent()?
            .to_path_buf();
        for candidate in [
            std::path::PathBuf::from("Gw.dat"),
            repo.join("data").join("Gw.dat"),
        ] {
            if candidate.exists() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
        None
    }

    #[test]
    fn a_map_blob_decompresses_to_the_containers_own_signature() {
        // The decoder's verification, against the archive rather than against my reading of the C. The
        // reference's importer accepts a blob only when its first four bytes are `RIFF_SIGNATURE` -- which
        // is `"ffna"` (`GmMapDataImport.c:3`) -- and its fifth byte is a known chunk type. A decoder that
        // is off by a bit produces the right *length* and the wrong bytes, so the signature is the check
        // that matters.
        let Some(path) = gw_dat_path() else {
            eprintln!("skipped: no local Gw.dat (set ASURA_GW_DAT or copy one into data/)");
            return;
        };
        let mut archive = GwDat::open(&path).expect("open Gw.dat");

        for (label, file_id) in [
            ("Kamadan", 0x345CC_u32),
            ("Lakeside County / Ascalon City", 0x1B97D),
            ("The Catacombs", 0x5CEE6),
        ] {
            let entry = archive.entry(file_id).expect("the entry is there");
            let content = archive
                .file(file_id)
                .unwrap_or_else(|err| panic!("{label}: {err}"))
                .expect("the entry is there");
            if entry.compressed == 0 {
                continue; // a stored entry is not this test's subject
            }
            let announced = archive
                .announced_size(file_id)
                .expect("read the tail")
                .expect("a compressed entry announces a size");
            assert_eq!(
                content.len(),
                announced as usize,
                "{label}: the decoded length is the size the entry announced"
            );
            assert_eq!(
                &content[0..4],
                b"ffna",
                "{label}: the first four bytes are the reference's `RIFF_SIGNATURE`"
            );
            assert!(
                content[4] < 16,
                "{label}: the fifth byte is a chunk type ({})",
                content[4]
            );
        }
    }

    #[test]
    fn the_archive_index_finds_the_blobs_the_map_catalogue_names() {
        let Some(path) = gw_dat_path() else {
            eprintln!("skipped: no local Gw.dat (set ASURA_GW_DAT or copy one into data/)");
            return;
        };
        let mut archive = GwDat::open(&path).expect("open Gw.dat");

        assert_eq!(archive.header().magic, MAGIC, "the archive magic");
        assert!(
            archive.len() > 1_000,
            "a file table this small is not Gw.dat: {}",
            archive.len()
        );
        assert!(
            archive.header().mft_offset > 0,
            "the MFT lives past the header"
        );

        // Every file id the map catalogue names, plus the two blob ids the reference's roadmap records as
        // verified. A catalogue entry that names an id the archive does not have is a map no client can
        // load, so this is the check that keeps the seed honest.
        let names = [
            ("Lakeside County / Ascalon City", 0x1B97D_u32),
            ("Ashford Abbey", 0x1B97D),
            ("The Catacombs", 0x5CEE6),
            ("Green Hills County / Barradin", 0x5CF29),
            ("Wizard's Folly / Foibles Fair", 0x5CEE7),
            ("Regent Valley / Fort Ranik", 0x5CF2A),
            ("The Northlands", 0x8005_EA96),
            ("Kamadan", 0x345CC),
        ];
        for (label, file_id) in names {
            let entry = archive
                .entry(file_id)
                .unwrap_or_else(|| panic!("{label}: file id {file_id:#X} is not in the archive"));
            assert!(entry.size > 0, "{label}: {file_id:#X} names an empty entry");
            assert!(
                entry.compressed != 0 || entry.size > 0,
                "{label}: {file_id:#X} is neither stored nor compressed"
            );
        }

        // A name the archive does not have is `None`, not a zero entry: the reader must not invent one.
        assert!(
            archive.entry(0xDEAD_BEEF).is_none(),
            "an id nobody put in the archive is not in the archive"
        );

        // The stored bytes of one entry, and the size it announces. Kamadan's blob is the one the
        // reference's own tool dumps (`gwdat_scan Gw.dat --dump 0x345CC`), so it is the natural first
        // candidate for a byte comparison once the decompressor exists.
        let stored = archive
            .stored(0x345CC)
            .expect("read the entry")
            .expect("the entry is there");
        let entry = archive.entry(0x345CC).expect("the entry is there");
        assert_eq!(
            stored.len(),
            entry.size as usize,
            "the stored size is the entry's size"
        );
        if entry.compressed != 0 {
            let announced = archive
                .announced_size(0x345CC)
                .expect("read the tail")
                .expect("a compressed entry announces a size");
            assert!(
                announced > entry.size,
                "a compressed entry's announced size ({announced}) is past its stored size ({})",
                entry.size
            );
        }
    }
}
