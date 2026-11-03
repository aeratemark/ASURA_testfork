//! The file check: a fixed 21-byte question and a fixed 32-byte answer, on the auth port.
//!
//! This is not a message with a header in the table - it is a bare request that arrives **before** the
//! version message, in the reference's `ConnState_AwaitHello` state
//! (`AuthSrv_TryHandleFileCheck` in GWPS `AuthSrv.c`). The client compares the reply against what it
//! expects; the reference answers and then keeps waiting, because "the client repeats the check, and
//! anything that follows it is a normal handshake".
//!
//! Both byte strings below are copied verbatim from that function, including the comment's meaning:
//! the six ids in the reply are `Gw.dat` file ids in tight consecutive order with a zero among them,
//! which is a check of *which* files the client should have, not a transfer.
//!
//! Refusing this request costs the whole login, which is why it is here: a service that classifies the
//! first four bytes as an unknown connection type - as this one did before the check existed - closes a
//! socket the client is waiting on, and the client sits at "Connecting" with nothing to report.

/// The request, exactly as the reference compares it (`memcmp` against all 21 bytes).
pub const FILE_CHECK_REQUEST: [u8; 21] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0xF1, 0x00, 0x10, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00,
];

/// The reply: `F1 02 20 00` (header 0x02F1 and a 0x2000 length) then six file ids and a zero -
/// 391083, 391092, 344988, 391094, 391095, 0, 391093 as little-endian words.
///
/// **These are the reference's ids, and they are not the ones ArenaNet sends.** A capture of a real
/// file check (`PreSearing/docs/09-client-connections.md`) shows 391339, 391348, 344988, 391350,
/// 391351, 0, 391349 - every id that differs is exactly `0x100` higher, so the reference's literal is
/// an approximation of the real answer. The reference's own value is used here because it is what the
/// rig demonstrably accepts (and GWPS is the floor, `docs/adr/0010-gwps-is-the-floor.md`); the captured
/// ids are the alternative if the client ever rejects this reply.
pub const FILE_CHECK_RESPONSE: [u8; 32] = [
    0xF1, 0x02, 0x20, 0x00, 0xAB, 0xF7, 0x05, 0x00, 0xB4, 0xF7, 0x05, 0x00, 0x9C, 0x43, 0x05, 0x00,
    0xB6, 0xF7, 0x05, 0x00, 0xB7, 0xF7, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0xB5, 0xF7, 0x05, 0x00,
];

/// Whether the buffer starts with a complete file-check request.
///
/// A partial request is *not* consumed: the reference waits for all 21 bytes before comparing, so a
/// caller that gets `false` reads more and asks again.
pub fn starts_with_request(bytes: &[u8]) -> bool {
    bytes.len() >= FILE_CHECK_REQUEST.len()
        && bytes[..FILE_CHECK_REQUEST.len()] == FILE_CHECK_REQUEST
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_and_reply_are_what_the_reference_has() {
        // Transcribed from `AuthSrv_TryHandleFileCheck`. The ids in the reply are checked as numbers
        // here, so a mistyped byte cannot hide: they are the ids the reference's literal spells out.
        assert_eq!(FILE_CHECK_REQUEST.len(), 21);
        assert_eq!(FILE_CHECK_RESPONSE.len(), 32);
        assert_eq!(&FILE_CHECK_RESPONSE[..4], &[0xF1, 0x02, 0x20, 0x00]);

        let ids: Vec<u32> = FILE_CHECK_RESPONSE[4..]
            .chunks(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        assert_eq!(ids, vec![391083, 391092, 344988, 391094, 391095, 0, 391093]);

        // And the capture says other ids: every one that differs is 0x100 higher. Recorded so that a
        // reader who finds the capture first does not "fix" these, and so that the alternative is on
        // the record if the client rejects the reference's answer.
        let captured = [391339, 391348, 344988, 391350, 391351, 0, 391349];
        for (index, (mine, theirs)) in ids.iter().zip(captured.iter()).enumerate() {
            // The two ids that do not differ are the one shared value (344988) and the zero slot.
            let expected_difference = if *mine == 344988 || *mine == 0 {
                0
            } else {
                0x100
            };
            assert_eq!(
                *theirs - *mine,
                expected_difference,
                "id {index} differs from the capture by {:?}",
                *theirs - *mine
            );
        }
    }

    #[test]
    fn only_the_exact_request_counts() {
        assert!(starts_with_request(&FILE_CHECK_REQUEST));
        // Trailing bytes are fine: the client may pipeline the version message behind it.
        let mut pipelined = FILE_CHECK_REQUEST.to_vec();
        pipelined.extend_from_slice(&[0x00, 0x04, 0x0C, 0x00]);
        assert!(starts_with_request(&pipelined));

        // A partial request is not a match, so the caller reads more instead of guessing.
        for cut in 0..FILE_CHECK_REQUEST.len() {
            assert!(
                !starts_with_request(&FILE_CHECK_REQUEST[..cut]),
                "cut at {cut}"
            );
        }

        // One byte different is a different request.
        let mut wrong = FILE_CHECK_REQUEST;
        wrong[20] = 0x01;
        assert!(!starts_with_request(&wrong));
    }
}
