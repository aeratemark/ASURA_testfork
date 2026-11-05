//! The file check: a fixed 21-byte question and a fixed 32-byte answer, on the auth port.
//!
//! This is not a message with a header in the table - it is a bare request that arrives **before** the
//! version message, in the reference's `ConnState_AwaitHello` state
//! (`AuthSrv_TryHandleFileCheck` in GWPS `AuthSrv.c`). The client compares the reply against what it
//! expects; the reference answers and then keeps waiting, because "the client repeats the check, and
//! anything that follows it is a normal handshake".
//!
//! The two byte strings are in `config/Server/file_check.json`, copied verbatim from that function -
//! the six ids in the reply are `Gw.dat` file ids in tight consecutive order with a zero among them,
//! which is a check of *which* files the client should have, not a transfer.
//!
//! Refusing this request costs the whole login, which is why it is here: a service that classifies the
//! first four bytes as an unknown connection type - as this one did before the check existed - closes a
//! socket the client is waiting on, and the client sits at "Connecting" with nothing to report.

/// Whether the buffer starts with a complete file-check request.
///
/// The bytes themselves are configuration (`config/Server/file_check.json`, `asura-config`): they are the
/// client's own payload, they change when its files change, and a crate that writers build messages in is
/// not where a changing byte string belongs. This function is the codec - the comparison - and it takes
/// the question as a parameter.
///
/// A partial request is *not* consumed: the reference waits for all of it before comparing, so a caller
/// that gets `false` reads more and asks again.
pub fn starts_with_request(bytes: &[u8], request: &[u8]) -> bool {
    !request.is_empty() && bytes.len() >= request.len() && bytes[..request.len()] == *request
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The question the client asks, as `config/Server/file_check.json` states it.
    const REQUEST: [u8; 21] = [
        0x01, 0x00, 0x00, 0x00, 0x00, 0xF1, 0x00, 0x10, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn only_the_exact_request_counts() {
        assert!(starts_with_request(&REQUEST, &REQUEST));

        // Trailing bytes are fine: the client may pipeline the version message behind it.
        let mut pipelined = REQUEST.to_vec();
        pipelined.extend_from_slice(&[0x00, 0x04, 0x0C, 0x00]);
        assert!(starts_with_request(&pipelined, &REQUEST));

        // A partial request is not a match, so the caller reads more instead of guessing.
        for cut in 0..REQUEST.len() {
            assert!(
                !starts_with_request(&REQUEST[..cut], &REQUEST),
                "cut at {cut}"
            );
        }

        // One byte different is a different request.
        let mut wrong = REQUEST;
        wrong[20] = 0x01;
        assert!(!starts_with_request(&wrong, &REQUEST));

        // An empty expectation matches nothing rather than everything.
        assert!(!starts_with_request(&REQUEST, &[]));
    }
}
