//! Bytes a document holds as they were given: the file an `Import`
//! feature was read from.
//!
//! A blob is shared, not copied, between the copies of a document (every
//! undo step is a whole document, docs/decisions.md): cloning it clones an
//! `Arc`. Two blobs are equal when their bytes are; the same `Arc` is
//! equal without looking, so the regeneration cache's comparison of a
//! feature with its last definition costs nothing. A blob is named by the
//! SHA-256 of its bytes (FIPS 180-4), which is also how `craft3d-format`
//! stores it (`blobs/<sha256>.step`), once per content.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

/// Immutable bytes with their SHA-256.
#[derive(Clone)]
pub struct Blob {
    bytes: Arc<[u8]>,
    hash: [u8; 32],
}

impl Blob {
    pub fn new(bytes: Vec<u8>) -> Self {
        let hash = sha256(&bytes);
        Self {
            bytes: bytes.into(),
            hash,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The SHA-256 of the bytes, as 64 lowercase hexadecimal digits.
    pub fn sha256(&self) -> String {
        hex(&self.hash)
    }

    /// Whether both are the same shared bytes (not only equal ones).
    pub fn same(&self, other: &Blob) -> bool {
        Arc::ptr_eq(&self.bytes, &other.bytes)
    }
}

impl PartialEq for Blob {
    fn eq(&self, other: &Self) -> bool {
        self.same(other) || (self.hash == other.hash && self.bytes == other.bytes)
    }
}

impl Eq for Blob {}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Blob({} bytes, sha256 {})", self.len(), self.sha256())
    }
}

/// What a blob looks like in JSON: its size and hash, never its bytes
/// (a feature's parameters in `document.inspect` would otherwise carry
/// the whole file). The native format stores the bytes beside the
/// manifest (`craft3d-format`).
#[derive(Serialize, Deserialize)]
struct Summary {
    bytes: usize,
    sha256: String,
}

impl Serialize for Blob {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Summary {
            bytes: self.len(),
            sha256: self.sha256(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Blob {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let summary = Summary::deserialize(deserializer)?;
        Err(serde::de::Error::custom(format!(
            "the {} bytes of blob {} are not in JSON; a .3dcraft file holds them beside its manifest",
            summary.bytes, summary.sha256
        )))
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        for nibble in [b >> 4, b & 15] {
            out.push(char::from(
                DIGITS.get(usize::from(nibble)).copied().unwrap_or(b'0'),
            ));
        }
    }
    out
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 of `data` (FIPS 180-4, section 6.2).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    // The message, a one bit, zeros to 56 mod 64 bytes, and its length in
    // bits as a big-endian 64-bit number.
    let bits = (data.len() as u64).wrapping_mul(8);
    let mut tail: Vec<u8> = data.chunks_exact(64).remainder().to_vec();
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&bits.to_be_bytes());
    for block in data.chunks_exact(64).chain(tail.chunks_exact(64)) {
        let mut w = [0u32; 64];
        for (word, bytes) in w.iter_mut().zip(block.chunks_exact(4)) {
            let mut four = [0u8; 4];
            four.copy_from_slice(bytes);
            *word = u32::from_be_bytes(four);
        }
        for t in 16..64 {
            let at = |i: usize| w.get(i).copied().unwrap_or(0);
            let (w15, w2) = (at(t - 15), at(t - 2));
            let s0 = w15.rotate_right(7) ^ w15.rotate_right(18) ^ (w15 >> 3);
            let s1 = w2.rotate_right(17) ^ w2.rotate_right(19) ^ (w2 >> 10);
            let value = at(t - 16)
                .wrapping_add(s0)
                .wrapping_add(at(t - 7))
                .wrapping_add(s1);
            if let Some(slot) = w.get_mut(t) {
                *slot = value;
            }
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for (k, wt) in K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(*k)
                .wrapping_add(*wt);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, add) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(add);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, word) in out.chunks_exact_mut(4).zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_of_the_standards_examples() {
        // FIPS 180-4 examples (and the empty message).
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million = vec![b'a'; 1_000_000];
        assert_eq!(
            hex(&sha256(&million)),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        // Lengths around the padding boundary all hash (55, 56, 63, 64).
        for n in [55, 56, 63, 64, 65, 119, 120] {
            assert_eq!(sha256(&vec![7u8; n]).len(), 32);
        }
    }

    #[test]
    fn blobs_compare_by_content_and_share_on_clone() {
        let a = Blob::new(b"ISO-10303-21;".to_vec());
        let b = a.clone();
        assert!(a.same(&b));
        let c = Blob::new(b"ISO-10303-21;".to_vec());
        assert!(!a.same(&c));
        assert_eq!(a, c);
        assert_ne!(a, Blob::new(b"other".to_vec()));
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(json["bytes"], 13);
        assert!(serde_json::from_value::<Blob>(json).is_err());
    }
}
