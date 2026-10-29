//! Pictures made by a test on a machine nobody looks at (the Strix runner
//! of the Check workflow) printed into the test's output, so that whoever
//! started the run can read them back from its log: lines `EV <name>
//! <index> <base64 chunk>`, in order (docs/development.md › Watching
//! along). Printed only when the test runs with `--nocapture` or fails.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                let i = (n >> (18 - 6 * k)) & 63;
                out.push(char::from(
                    ALPHABET.get(i as usize).copied().unwrap_or(b'A'),
                ));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Prints `bytes` (a PNG, usually) as evidence lines named `name`.
pub fn print(name: &str, bytes: &[u8]) {
    let text = base64(bytes);
    let chars: Vec<char> = text.chars().collect();
    for (i, chunk) in chars.chunks(4000).enumerate() {
        let line: String = chunk.iter().collect();
        eprintln!("EV {name} {i} {line}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_the_rfc_examples() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
