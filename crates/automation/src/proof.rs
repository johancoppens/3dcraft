//! Proving knowledge of the control token without sending it (2026-11-02,
//! docs/control-protocol.md, "Threat model").
//!
//! The bridge opens with `challenge {nonce}`; the window answers with a
//! nonce of its own, its process id and `HMAC-SHA256(token, "3dcraft
//! window" | port | pid | bridge nonce | window nonce)`, which only a
//! holder of the token can make, for the port it listens on. The bridge
//! checks it with the port it connected to before it sends anything that
//! depends on the token, then authenticates with `HMAC-SHA256(token,
//! "3dcraft client" | port | pid | window nonce | bridge nonce)`. A program that took the port (another user's,
//! or one that came after a window crashed) learns nothing it could use:
//! the token never crosses the socket.
//!
//! HMAC is RFC 2104 over the workspace's own SHA-256 (FIPS 180-4,
//! `craft3d_doc::sha256`), checked against RFC 4231's test vectors.

/// The roles, so a proof made for one side never passes for the other.
pub const WINDOW: &str = "3dcraft window";
pub const CLIENT: &str = "3dcraft client";

const BLOCK: usize = 64;

/// HMAC-SHA256 of `message` under `key` (RFC 2104).
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0_u8; BLOCK];
    if key.len() > BLOCK {
        let hashed = craft3d_doc::sha256(key);
        for (slot, byte) in block.iter_mut().zip(hashed) {
            *slot = byte;
        }
    } else {
        for (slot, byte) in block.iter_mut().zip(key) {
            *slot = *byte;
        }
    }
    let mut inner = Vec::with_capacity(BLOCK + message.len());
    inner.extend(block.iter().map(|b| b ^ 0x36));
    inner.extend_from_slice(message);
    let inner = craft3d_doc::sha256(&inner);
    let mut outer = Vec::with_capacity(BLOCK + inner.len());
    outer.extend(block.iter().map(|b| b ^ 0x5c));
    outer.extend_from_slice(&inner);
    craft3d_doc::sha256(&outer)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The proof of `role` for the window listening on 127.0.0.1:`port` as
/// process `pid`, over the two nonces (lower case), as hexadecimal. The
/// port and the process are in it so that a proof the window made for a
/// program on another port (relaying a client's challenge to the real
/// window) does not pass where the client connected.
pub fn proof(token: &str, role: &str, port: u16, pid: u32, first: &str, second: &str) -> String {
    let message = format!(
        "{role}\nport {port}\npid {pid}\n{}\n{}",
        first.to_ascii_lowercase(),
        second.to_ascii_lowercase()
    );
    hex(&hmac_sha256(token.as_bytes(), message.as_bytes()))
}

/// Whether `a` and `b` are the same text, compared without stopping at the
/// first difference.
pub fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut difference = a.len() ^ b.len();
    for (index, byte) in a.iter().enumerate() {
        difference |= usize::from(byte ^ b.get(index).copied().unwrap_or(0));
    }
    difference == 0 && !a.is_empty()
}

/// Whether `text` is a nonce: 32 to 128 hexadecimal characters.
pub fn is_nonce(text: &str) -> bool {
    (32..=128).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A fresh nonce: 32 random bytes as hexadecimal.
#[cfg(not(target_arch = "wasm32"))]
pub fn nonce() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("cannot get random bytes: {e}"))?;
    Ok(hex(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    /// RFC 4231, test cases 1, 2, 4 and 6 (a key longer than a block).
    #[test]
    fn hmac_sha256_matches_rfc_4231() {
        let cases: [(Vec<u8>, Vec<u8>, &str); 4] = [
            (
                vec![0x0b; 20],
                b"Hi There".to_vec(),
                "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
            ),
            (
                b"Jefe".to_vec(),
                b"what do ya want for nothing?".to_vec(),
                "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            ),
            (
                unhex("0102030405060708090a0b0c0d0e0f10111213141516171819"),
                vec![0xcd; 50],
                "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
            ),
            (
                vec![0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First".to_vec(),
                "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
            ),
        ];
        for (key, message, expected) in cases {
            assert_eq!(hex(&hmac_sha256(&key, &message)), expected);
        }
    }

    #[test]
    fn proofs_depend_on_the_token_the_role_and_the_order_of_the_nonces() {
        let (token, a, b) = ("ab".repeat(32), "1".repeat(64), "2".repeat(64));
        let window = proof(&token, WINDOW, 7878, 42, &a, &b);
        assert_eq!(window.len(), 64);
        assert_ne!(window, proof(&token, CLIENT, 7878, 42, &a, &b));
        assert_ne!(window, proof(&token, WINDOW, 7878, 42, &b, &a));
        assert_ne!(window, proof(&"cd".repeat(32), WINDOW, 7878, 42, &a, &b));
        // Another port or process: another proof (a relay does not pass).
        assert_ne!(window, proof(&token, WINDOW, 7879, 42, &a, &b));
        assert_ne!(window, proof(&token, WINDOW, 7878, 43, &a, &b));
        // Nonces in either case are the same nonces.
        assert_eq!(
            window,
            proof(&token, WINDOW, 7878, 42, &a.to_uppercase(), &b)
        );
        assert!(same(&window, &proof(&token, WINDOW, 7878, 42, &a, &b)));
        assert!(!same(&window, &window[..63]));
        assert!(!same("", ""));
        assert!(is_nonce(&a) && !is_nonce("12") && !is_nonce(&"g".repeat(64)));
        let (one, two) = (nonce().unwrap(), nonce().unwrap());
        assert!(is_nonce(&one) && one != two);
    }
}
