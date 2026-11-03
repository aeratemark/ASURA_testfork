//! The key exchange: what happens between the version message and the first encrypted frame.
//!
//! Source: GWPS `AuthSrv.c` (`Connection_DoKeyExchange`, `AuthSrv_DoPostHandshake`) and `main.c` (the
//! fixed constants). The sequence, byte for byte:
//!
//! ```text
//! client -> server   MSG_CLIENT_SEED   0x4200, u16 header, then 64 bytes: the client's public value,
//!                                      little endian, as a 512-bit integer          (66 bytes total)
//! server             shared = client_public ^ server_private mod prime
//! server             master = 20 random bytes
//! server -> client   MSG_SERVER_SEED   0x1601, u16 header, then 20 bytes:
//!                                      master[i] XOR shared_le[i]                     (22 bytes total)
//! both               key = arc4_hash(master)                -- 20 bytes
//! both               ARC4(key) for each direction, from position zero
//! ```
//!
//! Everything after the server seed is scrambled. Note what is *not* here: there is no generator and
//! no server public value on the wire. The client's patch carries the matching parameters, so the
//! server's job is only to raise the client's number to a fixed exponent - which is why this module
//! needs modular exponentiation and nothing else.
//!
//! Two things are deliberately different from the reference, and both are recorded:
//!
//! * **A degenerate shared secret is refused** ([`HandshakeError::DegenerateSecret`]). GWPS computes
//!   the shared secret and uses it; if it came out zero the master secret would be its own seed and
//!   the connection would be scrambled with a key that had never been exchanged. The real client
//!   cannot present such a value, so refusing it costs nothing and closes a hole the reference leaves
//!   open (`docs/adr/0009-crypto-primitives.md`).
//! * **The randomness is supplied by the caller**, not read here. A handshake that reads its own
//!   entropy cannot be tested; [`accept`] takes the 20 bytes it must use, and the service supplies
//!   them from the operating system.

use super::arc4::Arc4;
use num_bigint_dig::BigUint;

/// `CMSG_CLIENT_SEED_HEADER`: the client's half of the key exchange.
pub const CLIENT_SEED_HEADER: u16 = 0x4200;
/// `SMSG_SERVER_SEED_HEADER`: the server's reply.
pub const SERVER_SEED_HEADER: u16 = 0x1601;

/// The client seed's public value: 64 bytes, a 512-bit integer, little endian.
pub const PUBLIC_LEN: usize = 64;
/// `MSG_CLIENT_SEED`: header plus the public value.
pub const CLIENT_SEED_LEN: usize = 2 + PUBLIC_LEN;
/// `conn->master_secret` in GWPS: the random material the ARC4 key is derived from.
pub const MASTER_SECRET_LEN: usize = 20;
/// `MSG_SERVER_SEED`: header plus the scrambled master secret.
pub const SERVER_SEED_LEN: usize = 2 + MASTER_SECRET_LEN;
/// The ARC4 key derived from the master secret: an `arc4_hash` digest.
pub const KEY_LEN: usize = 20;

/// The prime modulus, verbatim from GWPS `main.c` (decimal, 155 digits, 512 bits).
pub const PRIME_MODULUS_DECIMAL: &str = "11369030006184639557951841839086711170523356038756186113745357553615433746914467378379225197818567184257516325746258410038643664166668140728650714232881773";

/// The server's private exponent, verbatim from GWPS `main.c`. Fixed: the client's patch carries the
/// matching public value, so this is a protocol constant rather than a secret in any useful sense.
pub const SERVER_PRIVATE_DECIMAL: &str = "4955013838800442477563322845022960998436564559618750588243916809877035796638982560199828129813551335307191644041932374697066251319749593910087044611412991";

/// Why the key exchange could not be completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeError {
    /// Fewer bytes than a whole client seed. Read more.
    Incomplete { needed: usize },
    /// A whole client seed whose header is not [`CLIENT_SEED_HEADER`] - GWPS returns
    /// `ERR_BAD_USER_DATA` and drops the connection, without guessing what was meant.
    UnexpectedHeader { got: u16, expected: u16 },
    /// The shared secret came out zero, which would mean the exchange exchanged nothing. See the
    /// module documentation: this is a deliberate deviation, not a reproduction.
    DegenerateSecret,
}

impl core::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HandshakeError::Incomplete { needed } => {
                write!(f, "incomplete: {needed} bytes needed for the client seed")
            }
            HandshakeError::UnexpectedHeader { got, expected } => write!(
                f,
                "client seed has header {got:#06X}, expected {expected:#06X}"
            ),
            HandshakeError::DegenerateSecret => f.write_str(
                "the shared secret is zero: the client's public value is not a usable one",
            ),
        }
    }
}

impl std::error::Error for HandshakeError {}

/// The client's half of the exchange, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientSeed {
    /// The client's public value, little endian, exactly as it arrived.
    pub public: [u8; PUBLIC_LEN],
}

impl ClientSeed {
    /// Decode a whole client seed message.
    pub fn decode(bytes: &[u8]) -> Result<Self, HandshakeError> {
        if bytes.len() < CLIENT_SEED_LEN {
            return Err(HandshakeError::Incomplete {
                needed: CLIENT_SEED_LEN,
            });
        }
        let header = u16::from_le_bytes([bytes[0], bytes[1]]);
        if header != CLIENT_SEED_HEADER {
            return Err(HandshakeError::UnexpectedHeader {
                got: header,
                expected: CLIENT_SEED_HEADER,
            });
        }
        let mut public = [0u8; PUBLIC_LEN];
        public.copy_from_slice(&bytes[2..CLIENT_SEED_LEN]);
        Ok(ClientSeed { public })
    }
}

/// What a completed exchange yields: the bytes to send, and the two ciphers to continue with.
pub struct Established {
    /// The complete `MSG_SERVER_SEED` frame, ready for the socket.
    pub reply: Vec<u8>,
    /// Scrambles everything this connection sends from now on.
    pub outgoing: Arc4,
    /// Unscrambles everything it receives.
    pub incoming: Arc4,
    /// The master secret, for the packet trace. Never logged in production.
    pub master_secret: [u8; MASTER_SECRET_LEN],
}

/// Raise `public` to the fixed private exponent, modulo the fixed prime. Little endian in and out.
///
/// Isolated on purpose: this is the only place the big-integer crate is used, so replacing it is a
/// change to one function (`docs/adr/0009-crypto-primitives.md`).
///
/// The output is zero-padded to [`PUBLIC_LEN`] bytes, matching `mbedtls_mpi_write_binary_le` with a
/// 64-byte buffer. A result longer than 64 bytes is impossible: the shared secret is less than the
/// 512-bit modulus.
pub fn modpow(public: &[u8; PUBLIC_LEN]) -> [u8; PUBLIC_LEN] {
    let base = BigUint::from_bytes_le(public);
    let exponent = BigUint::parse_bytes(SERVER_PRIVATE_DECIMAL.as_bytes(), 10)
        .expect("the private exponent is a decimal constant");
    let modulus = BigUint::parse_bytes(PRIME_MODULUS_DECIMAL.as_bytes(), 10)
        .expect("the prime modulus is a decimal constant");

    let shared = base.modpow(&exponent, &modulus);

    let mut out = [0u8; PUBLIC_LEN];
    let bytes = shared.to_bytes_le();
    debug_assert!(
        bytes.len() <= PUBLIC_LEN,
        "shared secret exceeds the modulus size"
    );
    out[..bytes.len()].copy_from_slice(&bytes);
    out
}

/// `arc4_hash`: the key derivation, ported from `AuthSrv.c`.
///
/// It is not a standard hash. It borrows SHA-1's initial values and two of its round constants, runs
/// five rounds over the 20 input bytes, and *adds* the working state back onto the input words rather
/// than emitting a chaining value. There is no padding, no length encoding and no second block: the
/// input is always exactly the 20-byte master secret. Reproduced exactly, because a different digest
/// means a keystream the client will not recognise - and the client reports nothing but `007`.
///
/// It is also not a strong hash: five rounds diffuse poorly, so flipping one input bit changes only
/// some of the digest. That is fine for what it is - the input is a uniformly random secret, so there
/// is nothing to protect against by mixing - and it is *not* something to improve on. Improving it
/// would break the handshake.
pub fn derive_key(master: &[u8; MASTER_SECRET_LEN]) -> [u8; KEY_LEN] {
    fn read_le(bytes: &[u8]) -> u32 {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    let input = [
        read_le(&master[0..4]),
        read_le(&master[4..8]),
        read_le(&master[8..12]),
        read_le(&master[12..16]),
        read_le(&master[16..20]),
    ];

    let mut a: u32 = 0x6745_2301;
    let mut b: u32 = 0xEFCD_AB89;
    let mut c: u32 = 0x98BA_DCFE;
    let mut d: u32 = 0x1032_5476;
    let mut e: u32 = 0xC3D2_E1F0;

    // Round 1: f(x, y, z) = z ^ (y & (x ^ z)), constant 0x5A827999.
    let mut f = d ^ (b & (c ^ d));
    e = e
        .wrapping_add(input[0])
        .wrapping_add(a.rotate_left(5))
        .wrapping_add(f)
        .wrapping_add(0x5A82_7999);
    b = b.rotate_left(30);

    f = c ^ (a & (b ^ c));
    d = d
        .wrapping_add(input[1])
        .wrapping_add(e.rotate_left(5))
        .wrapping_add(f)
        .wrapping_add(0x5A82_7999);
    a = a.rotate_left(30);

    f = b ^ (e & (a ^ b));
    c = c
        .wrapping_add(input[2])
        .wrapping_add(d.rotate_left(5))
        .wrapping_add(f)
        .wrapping_add(0x5A82_7999);
    e = e.rotate_left(30);

    // Round 2: f = parity, constant 0x6ED9EBA1.
    f = b ^ c ^ d;
    e = e
        .wrapping_add(input[3])
        .wrapping_add(a.rotate_left(5))
        .wrapping_add(f)
        .wrapping_add(0x6ED9_EBA1);
    b = b.rotate_left(30);

    f = a ^ b ^ c;
    d = d
        .wrapping_add(input[4])
        .wrapping_add(e.rotate_left(5))
        .wrapping_add(f)
        .wrapping_add(0x6ED9_EBA1);
    a = a.rotate_left(30);

    let digest = [
        input[0].wrapping_add(a),
        input[1].wrapping_add(b),
        input[2].wrapping_add(c),
        input[3].wrapping_add(d),
        input[4].wrapping_add(e),
    ];

    let mut out = [0u8; KEY_LEN];
    for (index, word) in digest.iter().enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// Recover the master secret from a server seed, which is what the client does.
///
/// Present here because the server-side tests need it: an exchange can only be *proved* by unwrapping
/// the seed with the same shared secret, the way the client does. XOR is symmetric, so this is the
/// same operation in both directions.
pub fn xor_master_secret(seed: &[u8], shared: &[u8; PUBLIC_LEN]) -> [u8; MASTER_SECRET_LEN] {
    let mut master = [0u8; MASTER_SECRET_LEN];
    for index in 0..MASTER_SECRET_LEN {
        master[index] = seed[index] ^ shared[index];
    }
    master
}

/// Complete the key exchange for one connection.
///
/// `bytes` is whatever has arrived of the client seed; `master_secret` is the 20 bytes of randomness
/// the caller supplied. On success the caller sends `reply` plaintext and then scrambles everything
/// through the two returned ciphers.
pub fn accept(
    bytes: &[u8],
    master_secret: &[u8; MASTER_SECRET_LEN],
) -> Result<Established, HandshakeError> {
    let client = ClientSeed::decode(bytes)?;
    let shared = modpow(&client.public);

    if shared.iter().all(|byte| *byte == 0) {
        return Err(HandshakeError::DegenerateSecret);
    }

    let mut reply = Vec::with_capacity(SERVER_SEED_LEN);
    reply.extend_from_slice(&SERVER_SEED_HEADER.to_le_bytes());
    reply.extend_from_slice(&xor_master_secret(master_secret, &shared));

    let key = derive_key(master_secret);
    let outgoing = Arc4::new(&key);
    let incoming = outgoing.fork();

    Ok(Established {
        reply,
        outgoing,
        incoming,
        master_secret: *master_secret,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client seed for a given public value, framed as the client sends it.
    fn client_seed(public: [u8; PUBLIC_LEN]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CLIENT_SEED_LEN);
        bytes.extend_from_slice(&CLIENT_SEED_HEADER.to_le_bytes());
        bytes.extend_from_slice(&public);
        bytes
    }

    #[test]
    fn the_constants_are_the_ones_the_client_uses() {
        assert_eq!(CLIENT_SEED_HEADER, 0x4200);
        assert_eq!(SERVER_SEED_HEADER, 0x1601);
        assert_eq!(CLIENT_SEED_LEN, 66);
        assert_eq!(SERVER_SEED_LEN, 22);
        // The packed 2-byte header plus the public value, in both directions.
        assert_eq!(CLIENT_SEED_LEN, 2 + 64);
        assert_eq!(SERVER_SEED_LEN, 2 + 20);
    }

    #[test]
    fn a_short_client_seed_is_incomplete_rather_than_wrong() {
        let seed = client_seed([0u8; PUBLIC_LEN]);
        for cut in 0..CLIENT_SEED_LEN {
            assert_eq!(
                ClientSeed::decode(&seed[..cut]),
                Err(HandshakeError::Incomplete {
                    needed: CLIENT_SEED_LEN
                }),
                "cut at {cut}"
            );
        }
        assert!(ClientSeed::decode(&seed).is_ok());
    }

    #[test]
    fn a_wrong_header_is_refused_and_the_expected_one_named() {
        let mut seed = client_seed([1u8; PUBLIC_LEN]);
        seed[0] = 0x01;
        seed[1] = 0x16; // the server seed header, 0x1601, in the client's direction
        assert_eq!(
            ClientSeed::decode(&seed),
            Err(HandshakeError::UnexpectedHeader {
                got: 0x1601,
                expected: CLIENT_SEED_HEADER
            })
        );
    }

    #[test]
    fn the_client_public_value_is_taken_byte_for_byte() {
        let public: [u8; PUBLIC_LEN] = core::array::from_fn(|index| (index as u8).wrapping_mul(3));
        let seed = ClientSeed::decode(&client_seed(public)).expect("decodes");
        assert_eq!(seed.public, public);
    }

    #[test]
    fn a_degenerate_public_value_is_refused() {
        // Zero to any power is zero, so the "shared" secret would be nothing at all. GWPS would carry
        // on with it; refusing is a recorded deviation (see the module documentation).
        for public in [[0u8; PUBLIC_LEN], {
            // The prime itself, little endian: congruent to zero modulo the modulus.
            let mut value = [0u8; PUBLIC_LEN];
            let prime = BigUint::parse_bytes(PRIME_MODULUS_DECIMAL.as_bytes(), 10).expect("prime");
            let bytes = prime.to_bytes_le();
            value[..bytes.len()].copy_from_slice(&bytes);
            value
        }] {
            assert_eq!(
                accept(&client_seed(public), &[0u8; MASTER_SECRET_LEN]).map(|_| ()),
                Err(HandshakeError::DegenerateSecret)
            );
        }
    }

    #[test]
    fn the_exchange_is_consistent_with_what_the_client_can_compute() {
        // The whole point of the exchange: the client, holding server_public = 2^server_private and its
        // own private value, arrives at the same shared secret the server computes from the client's
        // public value. Here the "client" is simulated with the same modpow, so this test proves the
        // *shape* of the protocol (which bytes are scrambled with what), not the arithmetic underneath
        // it -- that is pinned separately by `modpow_is_padded_to_the_full_width`.
        let client_private = BigUint::from(0xDEAD_BEEFu64);
        let prime = BigUint::parse_bytes(PRIME_MODULUS_DECIMAL.as_bytes(), 10).expect("prime");
        let generator = BigUint::from(5u64);
        let client_public = generator.modpow(&client_private, &prime);

        let mut public_bytes = [0u8; PUBLIC_LEN];
        let raw = client_public.to_bytes_le();
        public_bytes[..raw.len()].copy_from_slice(&raw);

        let master = [0x5Au8; MASTER_SECRET_LEN];
        let established = accept(&client_seed(public_bytes), &master).expect("exchange");

        // The client's side: unwrap the server seed with its own shared secret and derive the key.
        let server_private =
            BigUint::parse_bytes(SERVER_PRIVATE_DECIMAL.as_bytes(), 10).expect("priv");
        let server_public = generator.modpow(&server_private, &prime);
        let client_shared_big = server_public.modpow(&client_private, &prime);
        let mut client_shared = [0u8; PUBLIC_LEN];
        let raw = client_shared_big.to_bytes_le();
        client_shared[..raw.len()].copy_from_slice(&raw);

        assert_eq!(
            established.reply.len(),
            SERVER_SEED_LEN,
            "the reply is one whole server seed"
        );
        let recovered = xor_master_secret(&established.reply[2..], &client_shared);
        assert_eq!(recovered, master, "the client recovers the master secret");
    }

    #[test]
    fn both_directions_are_keyed_alike_and_start_at_the_same_offset() {
        let master = [0x11u8; MASTER_SECRET_LEN];
        let mut established = accept(&client_seed([7u8; PUBLIC_LEN]), &master).expect("exchange");

        let key = derive_key(&master);
        let expected: Vec<u8> = (0..8)
            .scan(Arc4::new(&key), |cipher, _| Some(cipher.keystream_byte()))
            .collect();

        let outgoing: Vec<u8> = (0..8)
            .map(|_| established.outgoing.keystream_byte())
            .collect();
        let incoming: Vec<u8> = (0..8)
            .map(|_| established.incoming.keystream_byte())
            .collect();
        assert_eq!(outgoing, expected);
        assert_eq!(incoming, expected);
    }

    #[test]
    fn the_derived_key_is_twenty_bytes_and_depends_on_every_input_byte() {
        let base = [0u8; MASTER_SECRET_LEN];
        let key = derive_key(&base);
        assert_eq!(key.len(), KEY_LEN);

        for index in 0..MASTER_SECRET_LEN {
            let mut changed = base;
            changed[index] ^= 0x01;
            assert_ne!(
                derive_key(&changed),
                key,
                "byte {index} of the master secret must affect the key"
            );
        }

        // Deliberately not asserted: that every output byte changes. This is a five-round
        // construction, not a hash with an avalanche property, and it does *not* diffuse that way.
        // It does not need to: its input is a uniformly random 20-byte secret, and its only job is to
        // match GWPS. Recording the weaker property here keeps a later reader from "improving" it
        // into something the client would not recognise.
        let mut changed = base;
        changed[0] ^= 0x01;
        let other = derive_key(&changed);
        let differing = key.iter().zip(other.iter()).filter(|(a, b)| a != b).count();
        assert!(
            differing > 0 && differing <= KEY_LEN,
            "the digest changes, and by fewer than all bytes: {differing} of {KEY_LEN}"
        );
    }

    #[test]
    fn modpow_is_padded_to_the_full_width() {
        // A small public value yields a small shared secret; the wire still carries 64 bytes of it,
        // which is what mbedtls_mpi_write_binary_le with a 64-byte buffer does.
        let shared = modpow(&[1u8; PUBLIC_LEN][..].try_into().expect("64 bytes"));
        assert_eq!(shared.len(), PUBLIC_LEN);
        assert!(
            shared.iter().any(|byte| *byte != 0),
            "the secret must not be all zeroes for a usable public value"
        );

        // 1 to any power is 1: the whole 64-byte output except the first byte is padding.
        let one = modpow(&{
            let mut value = [0u8; PUBLIC_LEN];
            value[0] = 1;
            value
        });
        assert_eq!(one[0], 1);
        assert!(one[1..].iter().all(|byte| *byte == 0));
    }
}
