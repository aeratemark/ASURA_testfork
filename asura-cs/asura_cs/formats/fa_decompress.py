"""``FaDecompress``: the Gw.dat entry decoder (LZ + Huffman), in pure Python.

A line-for-line port of ASURA's ``crates/asura-world/src/fa_compress.rs``, which is itself a port of the
reference ``FaCompress.c`` and is tested there against real archive entries. Keep the two in step: if the
Rust changes, change this.

It is slow compared to the Rust (a few MB/s at best), so callers cache what they decode
(see :class:`asura_cs.formats.gwdat.GwDat`).
"""

from __future__ import annotations

import struct

from .fa_tables import BACKTRACK, EXTRA_BITS_DISTANCE, EXTRA_BITS_LENGTH, TABLE1, TABLE2, TABLE3

M32 = 0xFFFFFFFF
NONE = M32


class DecompressError(ValueError):
    pass


class _Bits:
    __slots__ = ("words", "next", "buf1", "buf2", "available")

    def __init__(self, words):
        self.words = words
        self.next = 2
        self.buf1 = words[0] if len(words) > 0 else 0
        self.buf2 = words[1] if len(words) > 1 else 0
        self.available = 32

    def peek(self, count: int) -> int:
        return self.buf1 >> (32 - min(count, 32))

    def consume(self, count: int) -> None:
        if count == 0:
            return
        count = min(count, 32)
        self.buf1 = ((self.buf2 >> (32 - count)) | (self.buf1 << count)) & M32
        if self.available < count:
            if self.next >= len(self.words):
                self.available = 0
                self.buf2 = 0
                return
            self.buf2 = self.words[self.next]
            self.next += 1
            new_available = self.available + 32 - count
            self.buf1 |= (self.buf2 >> new_available) & M32
            self.buf2 = (self.buf2 << (count - self.available)) & M32
            self.available = new_available
        else:
            self.available -= count
            self.buf2 = (self.buf2 << count) & M32


class _Tree:
    __slots__ = ("lens", "codes", "large", "large_codes")

    def __init__(self):
        self.lens = [0] * 256
        self.codes = [0] * 256
        self.large = [(0, 0, 0)] * 24  # (first_encoding, last_index, encoding_len)
        self.large_codes: list[int] = []


def _setup_tree(bits: _Bits) -> _Tree:
    symbols_count = bits.peek(16)
    bits.consume(16)
    follow_root = [NONE] * 32
    follow = [0] * symbols_count
    total = 0

    cur = (symbols_count - 1) & M32
    while cur < symbols_count:
        index = 0
        buf1 = bits.buf1
        while index < len(TABLE1):
            if TABLE1[index][0] <= buf1:
                break
            index += 1
        if index == len(TABLE1):
            raise DecompressError("ambiguous encoding")
        bit_count = index + 3
        offset = (buf1 - TABLE1[index][0]) >> (32 - bit_count)
        bits.consume(bit_count)
        temp = TABLE2[TABLE1[index][1] - offset]
        number_of_codes = temp >> 5
        code_len = temp & 0x1F
        if code_len != 0 or symbols_count < 2:
            total += number_of_codes + 1
            for _ in range(number_of_codes + 1):
                if cur >= len(follow):
                    raise DecompressError("input exhausted")
                follow[cur] = follow_root[code_len]
                follow_root[code_len] = cur
                cur = (cur - 1) & M32
        else:
            cur = (cur - (number_of_codes + 1)) & M32

    if symbols_count != 0 and total == 0:
        if follow:
            follow[symbols_count - 1] = follow_root[0]
            follow_root[0] = symbols_count - 1
        total = 1

    tree = _Tree()
    lens, codes = tree.lens, tree.codes
    nbe = 0  # next_bit_encoding (u32)
    in_table = 0
    for enc_len in range(9):
        current = follow_root[enc_len]
        while current != NONE:
            if (1 << enc_len) <= nbe:
                raise DecompressError("ambiguous encoding")
            start = nbe << (8 - enc_len)
            for i in range(start, start + (1 << (8 - enc_len))):
                lens[i] = enc_len
                codes[i] = current
            current = follow[current] if current < len(follow) else NONE
            in_table += 1
            nbe = (nbe - 1) & M32
        nbe = ((nbe << 1) + 1) & M32

    if in_table < total:
        remaining = total - in_table
        tree.large_codes = [0] * remaining
        big = 0
        large = list(tree.large)
        for enc_len in range(9, 32):
            code = follow_root[enc_len]
            while code != NONE:
                if (1 << min(enc_len, 31)) <= nbe:
                    raise DecompressError("ambiguous encoding")
                partial = nbe >> (enc_len - 8)
                lens[partial] = NONE
                codes[partial] = 0
                if remaining <= big:
                    raise DecompressError("ambiguous encoding")
                tree.large_codes[big] = code
                big += 1
                code = follow[code] if code < len(follow) else NONE
                nbe = (nbe - 1) & M32
            first = (((nbe + 1) & M32) << (32 - enc_len)) & M32
            large[enc_len - 9] = (first, (big - 1) & M32, enc_len)
            nbe = ((nbe << 1) + 1) & M32
        tree.large = large
    return tree


def _next_code(bits: _Bits, tree: _Tree) -> int:
    nb = bits.buf1 >> 24
    enc_len = tree.lens[nb]
    code = tree.codes[nb]
    if enc_len == NONE:
        buf1 = bits.buf1
        for first, last, elen in tree.large:
            if first <= buf1 and elen != 0:
                enc_len = elen
                group = (buf1 - first) >> (32 - elen)
                index = (last - group) & M32
                if index >= len(tree.large_codes):
                    raise DecompressError("ambiguous encoding")
                code = tree.large_codes[index]
                break
        else:
            raise DecompressError("ambiguous encoding")
    if enc_len:
        bits.consume(enc_len)
    return code


def decompress(data: bytes, output_size: int) -> bytes:
    """Decode ``data`` (without the 4-byte size trailer) into exactly ``output_size`` bytes."""
    try:
        return _decompress(data, output_size)
    except IndexError as e:  # malformed stream indexing past a table: same outcome as the Rust's Err
        raise DecompressError(f"malformed stream: {e}") from None


def _decompress(data: bytes, output_size: int) -> bytes:
    pad = (-len(data)) % 4
    if pad:
        data = bytes(data) + b"\0" * pad
    words = list(struct.unpack(f"<{len(data) // 4}I", data))
    bits = _Bits(words)
    out = bytearray()

    bits.consume(4)
    first4 = bits.peek(4)
    bits.consume(4)

    while len(out) < output_size:
        literal = _setup_tree(bits)
        distance = _setup_tree(bits)
        block_size = (bits.peek(4) + 1) * 4096
        bits.consume(4)
        for _ in range(block_size):
            if len(out) >= output_size:
                break
            code = _next_code(bits, literal)
            if code < 0x100:
                out.append(code)
                continue
            index = code - 0x100
            if index >= len(EXTRA_BITS_LENGTH):
                raise DecompressError("ambiguous encoding")
            extra = EXTRA_BITS_LENGTH[index]
            code = TABLE3[index]
            if extra:
                code |= bits.peek(extra)
                bits.consume(extra)
            copy_len = first4 + code + 1
            dcode = _next_code(bits, distance)
            if dcode >= len(EXTRA_BITS_DISTANCE):
                raise DecompressError("ambiguous encoding")
            dextra = EXTRA_BITS_DISTANCE[dcode]
            back = BACKTRACK[dcode]
            if dextra:
                back |= bits.peek(dextra)
                bits.consume(dextra)
            start = len(out) - (back + 1)
            if start < 0:
                raise DecompressError("a back-reference points before the output")
            if len(out) + copy_len > output_size:
                raise DecompressError("output longer than announced")
            if back + 1 >= copy_len:
                out += out[start:start + copy_len]
            else:  # overlapping run: byte by byte, as the reference does
                for i in range(copy_len):
                    out.append(out[start + i])
    return bytes(out)
