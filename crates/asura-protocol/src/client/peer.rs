//! A test client: the same protocol from the other end.
//!
//! This exists so the auth service can be tested without the game. It speaks what
//! `docs/03-protocol.md` says a client speaks - the version message, the client seed, the ARC4
//! scrambler, and the framed messages - so a bug in the server's ordering, framing or encryption
//! shows up as a failed assertion rather than as `007` in front of a player.
//!
//! **What it does not prove.** It computes the shared secret with the same
//! [`super::handshake::modpow`] the server uses, because it knows the fixed private exponent. So it
//! agrees with the server *by construction*: the Diffie-Hellman arithmetic itself is pinned
//! separately, by the modpow padding test. What this does prove is everything around it - message
//! order, sizes, UTF-16 names, UUID byte order, the cipher offsets on both directions.
//!
//! It is a test client, not a production client: it does not retry, does not interpret more of the
//! login batch than its caller asks for, and holds no state beyond one connection.

use super::arc4::Arc4;
use super::frame::{Frame, FrameError};
use super::handshake::{self, MASTER_SECRET_LEN, PUBLIC_LEN, SERVER_SEED_LEN};
use super::messages::{PortalAccountLogin, RequestResponse};
use super::version;

/// Why a reference client could not continue.
#[derive(Debug)]
pub enum ClientError {
    /// The server's reply did not look like a server seed.
    BadServerSeed(handshake::HandshakeError),
    /// A frame in the stream was malformed.
    Framing(FrameError),
    /// The message could not be read.
    Message(super::messages::MessageError),
}

impl core::fmt::Display for ClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ClientError::BadServerSeed(err) => write!(f, "server seed: {err}"),
            ClientError::Framing(err) => write!(f, "framing: {err}"),
            ClientError::Message(err) => write!(f, "message: {err}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// One connection, seen from the client's side.
pub struct ReferenceClient {
    /// The client's public value, as sent.
    public: [u8; PUBLIC_LEN],
    /// Set once the server's seed has arrived.
    scrambler_out: Option<Arc4>,
    scrambler_in: Option<Arc4>,
    /// Decrypted bytes not yet consumed as frames.
    plaintext: Vec<u8>,
}

impl ReferenceClient {
    /// A client with the given public value.
    pub fn new(public: [u8; PUBLIC_LEN]) -> Self {
        ReferenceClient {
            public,
            scrambler_out: None,
            scrambler_in: None,
            plaintext: Vec::new(),
        }
    }

    /// A client with a public value derived from a seed, so a test can be reproducible.
    pub fn with_seed(seed: u64) -> Self {
        // Not key material: the value only has to be a plausible public value, and this keeps a test
        // deterministic. A real client derives it from its own private random value.
        let mut public = [0u8; PUBLIC_LEN];
        let mut state = seed | 1;
        for byte in public.iter_mut() {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            *byte = (state >> 33) as u8;
        }
        // Keep it well below the modulus so the exchange is a normal one: the top byte is cleared,
        // which also keeps the value from ever being congruent to zero.
        public[PUBLIC_LEN - 1] = 0;
        ReferenceClient::new(public)
    }

    /// `AUTH_CMSG_VERSION` (`0x000C0400`): the first message on the connection.
    pub fn version_message(&self, build: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(version::AUTH_CMSG_VERSION_LEN);
        bytes.extend_from_slice(&version::AUTH_CMSG_VERSION_HEADER.to_le_bytes());
        bytes.extend_from_slice(&build.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes()); // h0008, as the reference sets it
        bytes.extend_from_slice(&4u32.to_le_bytes()); // h000C, as the reference sets it
        bytes
    }

    /// `MSG_CLIENT_SEED` (`0x4200`): the client's half of the key exchange, 66 bytes, plaintext.
    pub fn client_seed_message(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(handshake::CLIENT_SEED_LEN);
        bytes.extend_from_slice(&handshake::CLIENT_SEED_HEADER.to_le_bytes());
        bytes.extend_from_slice(&self.public);
        bytes
    }

    /// Consume the server's seed and set up both scramblers, as the client does.
    pub fn accept_server_seed(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        if bytes.len() < SERVER_SEED_LEN {
            return Err(ClientError::BadServerSeed(
                handshake::HandshakeError::Incomplete {
                    needed: SERVER_SEED_LEN,
                },
            ));
        }
        let header = u16::from_le_bytes([bytes[0], bytes[1]]);
        if header != handshake::SERVER_SEED_HEADER {
            return Err(ClientError::BadServerSeed(
                handshake::HandshakeError::UnexpectedHeader {
                    got: header,
                    expected: handshake::SERVER_SEED_HEADER,
                },
            ));
        }

        let shared = handshake::modpow(&self.public);
        let mut seed = [0u8; MASTER_SECRET_LEN];
        seed.copy_from_slice(&bytes[2..SERVER_SEED_LEN]);
        let master = handshake::xor_master_secret(&seed, &shared);
        let key = handshake::derive_key(&master);

        let outgoing = Arc4::new(&key);
        self.scrambler_in = Some(outgoing.fork());
        self.scrambler_out = Some(outgoing);
        Ok(())
    }

    /// Whether the exchange has completed, so messages can be sent.
    pub fn is_established(&self) -> bool {
        self.scrambler_out.is_some()
    }

    /// Scramble and serialise a frame for the wire.
    pub fn seal(&mut self, frame: &Frame) -> Result<Vec<u8>, ClientError> {
        let mut bytes = frame.encode_validated().map_err(ClientError::Framing)?;
        self.seal_raw(&mut bytes);
        Ok(bytes)
    }

    /// Scramble bytes that are already in wire form.
    ///
    /// Needed for the messages the table cannot size - the computer info (`0x8001`, variable length)
    /// and anything pipelined behind it.
    pub fn seal_raw(&mut self, bytes: &mut [u8]) {
        let scrambler = self
            .scrambler_out
            .as_mut()
            .expect("seal is only valid after accept_server_seed");
        scrambler.apply(bytes);
    }

    /// The preamble a real client sends straight after the key exchange: its computer info (`0x8001`
    /// in the variable-length form that client actually uses) and its computer hash (`0x8002`, which
    /// carries the build number).
    ///
    /// The service answers the hash with `AUTH_SMSG_SESSION_INFO`, and the client waits for that before
    /// it will send a login - so a client double that skips this is not faithful to the wire.
    pub fn computer_info_preamble(&self, username: &str, pc_name: &str, build: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        // 0x8001: header, u16 count + UTF-16 units, u16 count + UTF-16 units.
        bytes.extend_from_slice(&super::tables::AUTH_CMSG_SEND_COMPUTER_INFO.to_le_bytes());
        let user: Vec<u16> = username.encode_utf16().collect();
        bytes.extend_from_slice(&(user.len() as u16).to_le_bytes());
        for unit in &user {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let pc: Vec<u16> = pc_name.encode_utf16().collect();
        bytes.extend_from_slice(&(pc.len() as u16).to_le_bytes());
        for unit in &pc {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }

        // 0x8002: header, the build number, then a 16-byte hash. The service does not interpret the
        // hash, and the real client's own value is its business.
        bytes.extend_from_slice(&super::tables::AUTH_CMSG_SEND_COMPUTER_HASH.to_le_bytes());
        bytes.extend_from_slice(&build.to_le_bytes());
        bytes.extend_from_slice(&[0xA5u8; 16]);
        bytes
    }

    /// Take everything that has arrived from the server and return the frames it completed.
    ///
    /// Bytes are unscrambled as they arrive, then framed from the plaintext buffer, so a message that
    /// spans two TCP reads is reassembled exactly as a client would reassemble it.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, ClientError> {
        let mut incoming = bytes.to_vec();
        let scrambler = self
            .scrambler_in
            .as_mut()
            .expect("receive is only valid after accept_server_seed");
        scrambler.apply(&mut incoming);
        self.plaintext.extend_from_slice(&incoming);

        let mut frames = Vec::new();
        loop {
            match Frame::decode_client_message(&self.plaintext) {
                Ok((frame, used)) => {
                    self.plaintext.drain(..used);
                    frames.push(frame);
                }
                Err(FrameError::Incomplete { .. }) => break,
                Err(err) => return Err(ClientError::Framing(err)),
            }
        }
        Ok(frames)
    }

    /// Take everything that has arrived from a **world server** and return the game messages it
    /// completed.
    ///
    /// The same scrambling and reassembly as [`ReferenceClient::receive`], framed by the game table
    /// instead of the auth one: the two families collide on header values, so a client double that
    /// framed a world server's stream with the auth table would misread the first message it did not
    /// recognise.
    pub fn receive_game(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, ClientError> {
        let mut incoming = bytes.to_vec();
        let scrambler = self
            .scrambler_in
            .as_mut()
            .expect("receive is only valid after accept_server_seed");
        scrambler.apply(&mut incoming);
        self.plaintext.extend_from_slice(&incoming);

        let mut frames = Vec::new();
        loop {
            match Frame::decode_game_message(&self.plaintext, false) {
                Ok((frame, used)) => {
                    self.plaintext.drain(..used);
                    frames.push(frame);
                }
                Err(FrameError::Incomplete { .. }) => break,
                Err(err) => return Err(ClientError::Framing(err)),
            }
        }
        Ok(frames)
    }

    /// Decrypt a chunk in place, without framing it.
    ///
    /// For a caller that needs the raw byte stream rather than messages - when two servers disagree
    /// about where a message ends, the bytes are the only evidence.
    pub fn decrypt(&mut self, bytes: &mut [u8]) {
        let scrambler = self
            .scrambler_in
            .as_mut()
            .expect("decrypt is only valid after accept_server_seed");
        scrambler.apply(bytes);
    }

    /// The decrypted bytes that have arrived but not yet been consumed as frames.
    ///
    /// Exposed so a caller can show what actually came down the wire when framing fails: the bytes are
    /// the evidence, and a framing error otherwise throws them away.
    pub fn pending_plaintext(&self) -> &[u8] {
        &self.plaintext
    }

    /// The login message for an identity pair.
    pub fn login_message(
        &self,
        request: u32,
        user_id: [u8; 16],
        session_id: [u8; 16],
    ) -> Result<Frame, ClientError> {
        PortalAccountLogin {
            request,
            user_id,
            session_id,
            character_name: String::new(),
            kind: "Gw1".to_string(),
        }
        .frame()
        .map_err(ClientError::Message)
    }
}

/// Read a request response out of a frame, for a caller that expects one.
pub fn as_request_response(frame: &Frame) -> Result<RequestResponse, ClientError> {
    RequestResponse::decode(frame).map_err(ClientError::Message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_and_seed_messages_have_the_recorded_sizes() {
        let client = ReferenceClient::with_seed(1);
        assert_eq!(client.version_message(34591).len(), 16);
        assert_eq!(client.client_seed_message().len(), 66);
        assert_eq!(
            &client.client_seed_message()[..2],
            &handshake::CLIENT_SEED_HEADER.to_le_bytes()
        );
    }

    #[test]
    fn a_client_seeded_from_a_seed_is_deterministic_and_never_degenerate() {
        let a = ReferenceClient::with_seed(42);
        let b = ReferenceClient::with_seed(42);
        assert_eq!(a.client_seed_message(), b.client_seed_message());
        assert_ne!(
            ReferenceClient::with_seed(7).client_seed_message(),
            a.client_seed_message()
        );
        // Never zero, and never above the modulus: the top byte is cleared on purpose.
        assert_eq!(a.client_seed_message()[65], 0);
        assert!(a.client_seed_message()[2..].iter().any(|byte| *byte != 0));
    }

    #[test]
    fn the_client_and_the_server_agree_on_the_scrambler() {
        // The server accepts the seed; the client consumes the reply; both then unscramble each
        // other's bytes. This is the property the whole connection depends on.
        let mut client = ReferenceClient::with_seed(1234);
        let master = [0x42u8; MASTER_SECRET_LEN];
        let established =
            handshake::accept(&client.client_seed_message(), &master).expect("server accepts");

        client
            .accept_server_seed(&established.reply)
            .expect("client accepts the reply");
        assert!(client.is_established());

        // The client seals a frame with its own scrambler; the server opens it with its incoming one.
        // The server frames client messages by walking their layout, not by the table's maximum.
        let frame = client.login_message(9, [1; 16], [2; 16]).expect("builds");
        let mut bytes = frame.encode();
        client.seal_raw(&mut bytes);

        let mut server_incoming = established.incoming;
        let mut received = bytes;
        server_incoming.apply(&mut received);
        let (decoded, used) = Frame::decode_client_message(&received).expect("frames");
        assert_eq!(used, received.len());
        assert_eq!(decoded, frame);

        // And a server frame reaches the client the same way.
        let mut server_outgoing = established.outgoing;
        let mut reply = super::super::messages::RequestResponse {
            request: 9,
            status: super::super::messages::STATUS_OK,
        }
        .frame()
        .encode();
        server_outgoing.apply(&mut reply);
        let frames = client.receive(&reply).expect("client receives");
        assert_eq!(frames.len(), 1);
        assert!(as_request_response(&frames[0]).expect("response").is_ok());
    }

    #[test]
    fn a_frame_split_across_two_reads_is_reassembled() {
        let mut client = ReferenceClient::with_seed(99);
        let master = [0x01u8; MASTER_SECRET_LEN];
        let established =
            handshake::accept(&client.client_seed_message(), &master).expect("server accepts");
        client
            .accept_server_seed(&established.reply)
            .expect("client accepts");

        let mut server_outgoing = established.outgoing;
        let first = super::super::messages::character_info(
            1,
            [7; 16],
            "Split",
            &super::super::messages::CHARACTER_SETTINGS_EXTENDED,
        )
        .expect("builds")
        .encode();
        let second = super::super::messages::friend_stream_end(1).encode();
        let mut stream = first.clone();
        stream.extend_from_slice(&second);
        server_outgoing.apply(&mut stream);

        // Half of the first message, then the rest.
        let split = 5;
        assert!(client
            .receive(&stream[..split])
            .expect("first half")
            .is_empty());
        let frames = client.receive(&stream[split..]).expect("second half");
        assert_eq!(
            frames.len(),
            2,
            "both messages complete once the bytes arrive"
        );
        assert_eq!(
            frames[0].header(),
            super::super::tables::AUTH_SMSG_CHARACTER_INFO
        );
        assert_eq!(
            frames[1].header(),
            super::super::tables::AUTH_SMSG_FRIEND_STREAM_END
        );
    }

    #[test]
    fn a_reply_that_is_not_a_server_seed_is_refused() {
        let mut client = ReferenceClient::with_seed(5);
        let short = [0u8; 3];
        assert!(matches!(
            client.accept_server_seed(&short),
            Err(ClientError::BadServerSeed(_))
        ));

        let mut wrong = vec![0u8; SERVER_SEED_LEN];
        wrong[0..2].copy_from_slice(&handshake::CLIENT_SEED_HEADER.to_le_bytes());
        assert!(matches!(
            client.accept_server_seed(&wrong),
            Err(ClientError::BadServerSeed(_))
        ));
    }
}
