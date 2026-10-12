//! Standard base64 (RFC 4648) with padding, for returning images in JSON.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let symbol = |index: u32| {
        ALPHABET
            .get((index & 63) as usize)
            .map_or('=', |&b| char::from(b))
    };
    for chunk in bytes.chunks(3) {
        let mut group = 0_u32;
        for (i, &byte) in chunk.iter().enumerate() {
            group |= u32::from(byte) << (16 - 8 * i);
        }
        out.push(symbol(group >> 18));
        out.push(symbol(group >> 12));
        out.push(if chunk.len() > 1 {
            symbol(group >> 6)
        } else {
            '='
        });
        out.push(if chunk.len() > 2 { symbol(group) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::encode;

    #[test]
    fn matches_the_rfc_4648_test_vectors() {
        let vectors = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (input, expected) in vectors {
            assert_eq!(encode(input.as_bytes()), expected);
        }
        assert_eq!(encode(&[0xfb, 0xff, 0xfe]), "+//+");
    }
}
