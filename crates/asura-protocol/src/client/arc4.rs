//! ARC4, the stream cipher the client protocol scrambles everything with after the key exchange.
//!
//! **This is a port, not a design.** Source: GWPS `arc4.c` (30 lines: `arc4_setup`, `arc4_crypt`,
//! `arc4_crypt_inplace`), which is textbook RC4 - the key schedule and the byte generator below are
//! the standard ones, and the known-answer tests pin that down against published vectors rather than
//! against my reading of the C.
//!
//! RC4 is broken as a cipher, and nobody should use it for anything new. It is here because the
//! client speaks it and there is no negotiating: `docs/03-protocol.md` section 3. That also means a
//! *crate* would not help - the requirement is byte-exactness with one specific implementation, and
//! the honest way to get that is to write the 20 lines and test them (`docs/adr/0009-crypto-
//! primitives.md`).
//!
//! Two instances per connection, one per direction, keyed identically: GWPS copies the encryption
//! context into the decryption one (`cipher_dec = cipher_enc` in `AuthSrv_DoPostHandshake`). They are
//! independent *states*, so each direction has its own keystream offset - which is why [`Arc4`] is
//! `Clone` and why the handshake clones rather than shares.

/// The RC4 keystream state: a 256-byte permutation and two indices.
///
/// The C version keeps `x` and `y` as `int` and masks on every use; here they are `u8`, and the
/// masking is implicit. The arithmetic is identical, because every use in the reference applies
/// `& 0xFF` to a value that is already reduced.
#[derive(Clone)]
pub struct Arc4 {
    x: u8,
    y: u8,
    m: [u8; 256],
}

impl Arc4 {
    /// The key schedule (`arc4_setup`).
    ///
    /// Any key length is accepted, and a key longer than 256 bytes would simply wrap: the reference
    /// cycles the key with `k = 0` when `k >= keylen`.
    pub fn new(key: &[u8]) -> Self {
        assert!(
            !key.is_empty(),
            "RC4 needs a key; the handshake's is 20 bytes"
        );

        let mut m = [0u8; 256];
        for (index, byte) in m.iter_mut().enumerate() {
            *byte = index as u8;
        }

        let mut j: u8 = 0;
        let mut k = 0usize;
        for index in 0..256usize {
            if k >= key.len() {
                k = 0;
            }
            let a = m[index];
            j = j.wrapping_add(a).wrapping_add(key[k]);
            m[index] = m[j as usize];
            m[j as usize] = a;
            k += 1;
        }

        Arc4 { x: 0, y: 0, m }
    }

    /// One keystream byte, advancing the state (`arc4_crypt`'s loop body).
    pub fn keystream_byte(&mut self) -> u8 {
        self.x = self.x.wrapping_add(1);
        let a = self.m[self.x as usize];

        self.y = self.y.wrapping_add(a);
        let b = self.m[self.y as usize];

        self.m[self.x as usize] = b;
        self.m[self.y as usize] = a;

        self.m[a.wrapping_add(b) as usize]
    }

    /// Scramble or unscramble a buffer in place.
    ///
    /// Encryption and decryption are the same operation for a stream cipher, so there is one method
    /// and no way to reach for the wrong one.
    pub fn apply(&mut self, buffer: &mut [u8]) {
        for byte in buffer.iter_mut() {
            *byte ^= self.keystream_byte();
        }
    }

    /// A decrypting or encrypting copy at the current position - what GWPS's `cipher_dec =
    /// cipher_enc` does.
    pub fn fork(&self) -> Self {
        self.clone()
    }
}

impl Drop for Arc4 {
    /// Overwrite the state on drop.
    ///
    /// Not load-bearing for correctness - a leaked RC4 state only exposes a dead connection's
    /// keystream - but a login path should not scatter key material into freed memory, and this costs
    /// nothing. `write_volatile` keeps the writes from being optimised away.
    fn drop(&mut self) {
        for byte in self.m.iter_mut() {
            // SAFETY: `byte` is a valid, aligned, exclusively-borrowed u8 for the duration of the
            // write, and `write_volatile` has no other precondition.
            unsafe { core::ptr::write_volatile(byte, 0) };
        }
        self.x = 0;
        self.y = 0;
    }
}

impl core::fmt::Debug for Arc4 {
    /// Never print the state: it is the keystream position, and the trace logs everything.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Arc4(<state hidden>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keystream(key: &[u8], length: usize) -> Vec<u8> {
        let mut cipher = Arc4::new(key);
        (0..length).map(|_| cipher.keystream_byte()).collect()
    }

    #[test]
    fn the_published_rc4_vectors_hold() {
        // The two vectors everybody knows, from the original RC4 description. They are the cheapest
        // independent evidence that this is standard RC4 and not a plausible variant of it.
        let mut cipher = Arc4::new(b"Key");
        let mut buffer = *b"Plaintext";
        cipher.apply(&mut buffer);
        assert_eq!(
            buffer,
            [0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]
        );

        let mut cipher = Arc4::new(b"Wiki");
        let mut buffer = *b"pedia";
        cipher.apply(&mut buffer);
        assert_eq!(buffer, [0x10, 0x21, 0xBF, 0x04, 0x20]);
    }

    #[test]
    fn encryption_and_decryption_are_the_same_operation() {
        let key = [7u8; 20];
        let plaintext: Vec<u8> = (0..=255u8).collect();

        let mut encrypting = Arc4::new(&key);
        let mut scrambled = plaintext.clone();
        encrypting.apply(&mut scrambled);
        assert_ne!(scrambled, plaintext, "the keystream must change the bytes");

        let mut decrypting = Arc4::new(&key);
        decrypting.apply(&mut scrambled);
        assert_eq!(scrambled, plaintext);
    }

    #[test]
    fn a_forked_cipher_shares_the_key_and_not_the_position() {
        // This is the property the handshake depends on: both directions use the same key from
        // position zero, and advancing one must not advance the other.
        let mut outgoing = Arc4::new(&[3u8; 20]);
        let mut incoming = outgoing.fork();

        let expected = keystream(&[3u8; 20], 4);
        let first: Vec<u8> = (0..4).map(|_| outgoing.keystream_byte()).collect();
        let second: Vec<u8> = (0..4).map(|_| incoming.keystream_byte()).collect();
        assert_eq!(first, expected);
        assert_eq!(second, expected, "each direction starts at the same offset");
    }

    #[test]
    fn a_byte_at_a_time_matches_a_buffer_at_a_time() {
        // The reference has both forms (`arc4_crypt` and `arc4_crypt_inplace`) and they must agree,
        // otherwise a stream that arrives in different chunks would decrypt differently.
        let key = b"a twenty byte key!!!";
        let plaintext: Vec<u8> = (0..64u8).map(|i| i.wrapping_mul(7)).collect();

        let mut whole = Arc4::new(key);
        let mut buffer = plaintext.clone();
        whole.apply(&mut buffer);

        let mut bytewise = Arc4::new(key);
        let mut other = Vec::new();
        for byte in &plaintext {
            let mut one = [*byte];
            bytewise.apply(&mut one);
            other.push(one[0]);
        }

        assert_eq!(buffer, other);
    }

    #[test]
    fn the_key_cycles_when_it_is_shorter_than_the_state() {
        // The reference wraps k with `k >= keylen`; a 20-byte key therefore repeats over 256 steps.
        // Checked through behaviour rather than internals: a short key and its repeated form are the
        // same cipher only if the wrap is implemented, so the two must differ.
        let short = keystream(&[1, 2, 3, 4], 16);
        let long = keystream([1, 2, 3, 4].repeat(64).as_slice(), 16);
        assert_eq!(short, long, "wrapping the key is the same as repeating it");

        let two = keystream(&[1, 2], 16);
        let three = keystream(&[1, 2, 3], 16);
        assert_ne!(two, three);
    }

    #[test]
    fn the_debug_form_does_not_print_the_state() {
        let text = format!("{:?}", Arc4::new(&[9u8; 20]));
        assert_eq!(text, "Arc4(<state hidden>)");
    }
}
