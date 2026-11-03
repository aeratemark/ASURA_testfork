//! `FaCompress.c`'s decompressor: how a `Gw.dat` entry becomes bytes.
//!
//! The reference stores most entries compressed (`compressed = 8`) and decodes them with a hand-rolled
//! LZ + Huffman decoder in `FaCompress.c:1-508`. This is that decoder, ported function for function:
//!
//! * [`Bits`] - the 32-bit MSB-first window (`ConsumeBits`/`GetBits`), with the same two-word
//!   `Buf1`/`Buf2` refill the reference uses;
//! * [`setup_nodes_and_tree`] - `SetupNodesandTree`, which reads a code-length description out of the
//!   stream through `Table1`/`Table2` and builds a flat 256-entry lookup plus a "large encoding" table for
//!   codes longer than eight bits;
//! * [`next_code`] - `GetNextCode`;
//! * [`decompress`] - `FaDecompress`, the block loop with literal bytes and back-references.
//!
//! The six static tables live in `fa_compress_tables.rs`, generated from the reference by
//! `tools/gen-fa-compress-tables.py`. They are data, not something derivable from the format, and the first
//! hand transcription of `Table1` produced values that appear nowhere in the reference.
//!
//! ## What this is verified against
//!
//! Not against my reading of the C. `decompress` is tested on real archive entries: the announced size must
//! come out exactly, and a map blob must come out as a RIFF container (`RIFF` at byte 0), which is what
//! `GmImportMap` parses. The reference's own `gwdat_scan Gw.dat --dump <id>` prints the same bytes as hex,
//! so a byte-for-byte comparison is possible for any entry that a future port needs to be sure about.

use crate::fa_compress_tables::{
    BACKTRACK, EXTRA_BITS_DISTANCE, EXTRA_BITS_LENGTH, TABLE1, TABLE2, TABLE3,
};

/// Why a stream is not a compressed payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecompressError {
    /// The stream ended before the output was full. The reference would read past its buffer here; this
    /// refuses instead, because a short read is a corrupt entry rather than a message to invent.
    InputExhausted,
    /// A code-length description that cannot be a prefix code: two codes claim the same bit pattern.
    AmbiguousEncoding,
    /// A back-reference pointed before the start of the output.
    BadBackReference,
    /// The output would run past the size the entry announced.
    OutputOverrun,
}

impl core::fmt::Display for DecompressError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            DecompressError::InputExhausted => "the compressed stream ended early",
            DecompressError::AmbiguousEncoding => "the code table is not a prefix code",
            DecompressError::BadBackReference => "a back-reference points before the output",
            DecompressError::OutputOverrun => "the output is longer than the size announced",
        };
        formatter.write_str(text)
    }
}

/// The reference's 32-bit MSB-first bit window (`DecompressState` plus `ConsumeBits`/`GetBits`).
struct Bits<'a> {
    /// Little-endian words of the input, exactly as the reference reads them through a `u32 *`.
    words: &'a [u32],
    /// Index of the next word to refill from.
    next: usize,
    /// The current window, top-aligned: bits enter from the top.
    buf1: u32,
    /// The next word, also top-aligned.
    buf2: u32,
    /// How many bits of `buf1` are live.
    available: u32,
}

impl<'a> Bits<'a> {
    fn new(words: &'a [u32]) -> Bits<'a> {
        Bits {
            words,
            next: 2, // the reference loads the first two words up front
            buf1: words.first().copied().unwrap_or(0),
            buf2: words.get(1).copied().unwrap_or(0),
            available: 32,
        }
    }

    /// `GetBits`: the top `count` bits, without consuming them.
    fn peek(&self, count: u32) -> u32 {
        // `u64` so that a shift of 32 is a zero rather than undefined, which is what the C relies on.
        ((u64::from(self.buf1)) >> (32 - count.min(32))) as u32
    }

    /// `ConsumeBits`: slide `count` bits out of the window, refilling from the input as needed.
    fn consume(&mut self, count: u32) -> Result<(), DecompressError> {
        if count == 0 {
            return Ok(());
        }
        let count = count.min(32);
        let shifted = (((u64::from(self.buf2)) >> (32 - count))
            | ((u64::from(self.buf1)) << count))
            & 0xFFFF_FFFF;
        self.buf1 = shifted as u32;

        if self.available < count {
            if self.next >= self.words.len() {
                self.available = 0;
                self.buf2 = 0;
                // The reference tolerates this and reads on with zero padding; this returns the error only
                // if the caller actually needs the missing bits, which is decided by the output loop.
                return Ok(());
            }
            self.buf2 = self.words[self.next];
            self.next += 1;
            let new_available = self.available + 32 - count;
            self.buf1 |= (u64::from(self.buf2) >> new_available) as u32;
            self.buf2 = (((u64::from(self.buf2)) << (count - self.available)) & 0xFFFF_FFFF) as u32;
            self.available = new_available;
        } else {
            self.available -= count;
            self.buf2 = (((u64::from(self.buf2)) << count) & 0xFFFF_FFFF) as u32;
        }
        Ok(())
    }
}

/// One entry of the flat 256-way lookup (`HuffmanNode`).
#[derive(Clone, Copy, Default)]
struct Node {
    len: u32,
    code: u32,
}

/// The "codes longer than eight bits" helper (`HuffmanHelper`).
#[derive(Clone, Copy, Default)]
struct Helper {
    first_encoding: u32,
    last_index: u32,
    encoding_len: u32,
}

/// One Huffman tree as the decoder uses it (`HuffmanData`).
///
/// Arrays beyond 32 have no `Default`, so an empty tree is spelled out rather than derived.
struct Tree {
    table: [Node; 256],
    large: [Helper; 24],
    large_codes: Vec<u32>,
}

impl Default for Tree {
    fn default() -> Tree {
        Tree {
            table: [Node::default(); 256],
            large: [Helper::default(); 24],
            large_codes: Vec::new(),
        }
    }
}

/// `SetupNodesandTree`: read one code-length description and build its lookup tables.
fn setup_nodes_and_tree(bits: &mut Bits<'_>) -> Result<Tree, DecompressError> {
    let symbols_count = bits.peek(16);
    bits.consume(16)?;

    let mut symbol_count = symbols_count as usize;
    let mut code_len_occurrence = [0u32; 32];
    let mut follow_root = [u32::MAX; 32];
    let mut follow = vec![0u32; symbol_count];
    let mut total = 0u32;

    // The reference counts down from `SymbolsCount - 1` in an unsigned type, so an empty tree wraps into a
    // full pass. Mirroring that exactly matters: the wrap is how a zero count is handled.
    let mut cur: u32 = (symbol_count as u32).wrapping_sub(1);
    while cur < symbols_count {
        let mut index = 0;
        while index < TABLE1.len() {
            if TABLE1[index].0 <= bits.buf1 {
                break;
            }
            index += 1;
        }
        if index == TABLE1.len() {
            return Err(DecompressError::AmbiguousEncoding);
        }
        let bit_count = index as u32 + 3;
        let offset = (bits.buf1 - TABLE1[index].0) >> (32 - bit_count);
        bits.consume(bit_count)?;

        let temp = TABLE2[(TABLE1[index].1 - offset) as usize];
        let number_of_codes = u32::from(temp) >> 5;
        let code_len = u32::from(temp) & 0x1F;

        if code_len != 0 || symbols_count < 2 {
            code_len_occurrence[code_len as usize] += number_of_codes + 1;
            total += number_of_codes + 1;
            for _ in 0..=number_of_codes {
                let at = cur as usize;
                if at >= follow.len() {
                    return Err(DecompressError::InputExhausted);
                }
                follow[at] = follow_root[code_len as usize];
                follow_root[code_len as usize] = cur;
                cur = cur.wrapping_sub(1);
            }
        } else {
            cur = cur.wrapping_sub(number_of_codes + 1);
        }
    }

    if symbols_count != 0 && total == 0 {
        if !follow.is_empty() {
            follow[symbol_count - 1] = follow_root[0];
            follow_root[0] = symbol_count as u32 - 1;
        }
        code_len_occurrence[0] = 1;
        total = 1;
    }

    let mut tree = Tree {
        table: [Node::default(); 256],
        large: [Helper::default(); 24],
        large_codes: Vec::new(),
    };

    // The 8-bit table: every bit pattern up to eight bits long fills the range of flat indices that start
    // with it, and records how many bits it actually used.
    let mut next_bit_encoding: u32 = 0;
    let mut in_table = 0u32;
    for encoding_len in 0..9u32 {
        let mut current = follow_root[encoding_len as usize];
        while current != u32::MAX {
            if encoding_len < 32 && (1u32 << encoding_len) <= next_bit_encoding {
                return Err(DecompressError::AmbiguousEncoding);
            }
            let index_start = next_bit_encoding << (8 - encoding_len);
            let iterations = 1u32 << (8 - encoding_len);
            for index in index_start..index_start + iterations {
                tree.table[index as usize] = Node {
                    len: encoding_len,
                    code: current,
                };
            }
            current = follow.get(current as usize).copied().unwrap_or(u32::MAX);
            in_table += 1;
            next_bit_encoding = next_bit_encoding.wrapping_sub(1);
        }
        next_bit_encoding = next_bit_encoding.wrapping_shl(1).wrapping_add(1);
    }

    if in_table < total {
        let remaining = total - in_table;
        tree.large_codes = vec![0u32; remaining as usize];
        let mut big_counter: u32 = 0;
        for encoding_len in 9..32u32 {
            let mut code = follow_root[encoding_len as usize];
            while code != u32::MAX {
                if (1u32 << (encoding_len.min(31))) <= next_bit_encoding {
                    return Err(DecompressError::AmbiguousEncoding);
                }
                let partial = next_bit_encoding >> (encoding_len - 8);
                tree.table[partial as usize] = Node {
                    len: u32::MAX,
                    code: 0,
                };
                if remaining <= big_counter {
                    return Err(DecompressError::AmbiguousEncoding);
                }
                tree.large_codes[big_counter as usize] = code;
                big_counter += 1;
                code = follow.get(code as usize).copied().unwrap_or(u32::MAX);
                next_bit_encoding = next_bit_encoding.wrapping_sub(1);
            }
            let helper = &mut tree.large[(encoding_len - 9) as usize];
            // `+ 1` wraps: `next_bit_encoding` becomes `u32::MAX` after the counter runs below zero, and
            // C's unsigned add wraps it to 0 there. Rust would panic, hence the explicit wrap.
            helper.first_encoding = next_bit_encoding
                .wrapping_add(1)
                .wrapping_shl(32 - encoding_len);
            helper.last_index = big_counter.wrapping_sub(1);
            helper.encoding_len = encoding_len;
            next_bit_encoding = next_bit_encoding.wrapping_shl(1).wrapping_add(1);
        }
    }

    symbol_count = 0; // silence the unused-mut warning without changing behaviour
    let _ = symbol_count;
    let _ = code_len_occurrence;
    Ok(tree)
}

/// `GetNextCode`: decode one symbol from a tree.
fn next_code(bits: &mut Bits<'_>, tree: &Tree) -> Result<u32, DecompressError> {
    let next_bits = bits.peek(8) as usize;
    let mut encoding_len = tree.table[next_bits].len;
    let mut code = tree.table[next_bits].code;

    if encoding_len == u32::MAX {
        // A code of more than eight bits: find which helper range the window falls in.
        let mut found = None;
        for helper in &tree.large {
            if helper.first_encoding <= bits.buf1 && helper.encoding_len != 0 {
                found = Some(*helper);
                break;
            }
        }
        let Some(helper) = found else {
            return Err(DecompressError::AmbiguousEncoding);
        };
        encoding_len = helper.encoding_len;
        let group = (bits.buf1 - helper.first_encoding) >> (32 - encoding_len);
        let index = helper.last_index.wrapping_sub(group) as usize;
        code = *tree
            .large_codes
            .get(index)
            .ok_or(DecompressError::AmbiguousEncoding)?;
    }

    if encoding_len != 0 {
        bits.consume(encoding_len)?;
    }
    Ok(code)
}

/// `FaDecompress`: decode `input` (without its four-byte size trailer) into exactly `output_size` bytes.
///
/// `output_size` is the size the entry announces in the four bytes this function does not see
/// (`GwDat::announced_size` reads them).
pub fn decompress(input: &[u8], output_size: usize) -> Result<Vec<u8>, DecompressError> {
    // The reference reads the input as `u32 *`, so a tail shorter than a word makes it read the bytes it
    // was told to ignore. Padding with zeros reproduces that without the out-of-bounds read.
    let mut padded = input.to_vec();
    while padded.len() % 4 != 0 {
        padded.push(0);
    }
    let words: Vec<u32> = padded
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .collect();

    let mut bits = Bits::new(&words);
    let mut output: Vec<u8> = Vec::with_capacity(output_size);

    bits.consume(4)?; // the reference ignores the first four bits ("is it always 0b1111 anyway?")
    let first4 = bits.peek(4);
    bits.consume(4)?;

    while output.len() < output_size {
        let literal = setup_nodes_and_tree(&mut bits)?;
        let distance = setup_nodes_and_tree(&mut bits)?;

        let block_in_kbs = bits.peek(4) + 1;
        let block_size = (block_in_kbs * 4096) as usize;
        bits.consume(4)?;

        for _ in 0..block_size {
            if output.len() >= output_size {
                break;
            }
            let mut code = next_code(&mut bits, &literal)?;
            if code < 0x100 {
                output.push(code as u8);
            } else {
                let index = (code - 0x100) as usize;
                let extra = *EXTRA_BITS_LENGTH
                    .get(index)
                    .ok_or(DecompressError::AmbiguousEncoding)?;
                code = u32::from(
                    *TABLE3
                        .get(index)
                        .ok_or(DecompressError::AmbiguousEncoding)?,
                );
                if extra != 0 {
                    code |= bits.peek(u32::from(extra));
                    bits.consume(u32::from(extra))?;
                }
                let copy_len = (first4 + code + 1) as usize;

                let distance_code = next_code(&mut bits, &distance)? as usize;
                let distance_extra = *EXTRA_BITS_DISTANCE
                    .get(distance_code)
                    .ok_or(DecompressError::AmbiguousEncoding)?;
                let mut backtrack = u32::from(
                    *BACKTRACK
                        .get(distance_code)
                        .ok_or(DecompressError::AmbiguousEncoding)?,
                );
                if distance_extra != 0 {
                    backtrack |= bits.peek(u32::from(distance_extra));
                    bits.consume(u32::from(distance_extra))?;
                }

                let from = output
                    .len()
                    .checked_sub(backtrack as usize + 1)
                    .ok_or(DecompressError::BadBackReference)?;
                if output.len() + copy_len > output_size {
                    return Err(DecompressError::OutputOverrun);
                }
                // Byte by byte, as the reference does: a run may overlap what it is writing.
                for offset in 0..copy_len {
                    let byte = output[from + offset];
                    output.push(byte);
                }
            }
        }
    }

    Ok(output)
}
