//! A reader of Radiance RGBE pictures (`.hdr`), the format the bundled
//! environments come in. Written from the format's public description
//! (Greg Ward, *Real Pixels*, Graphics Gems II, 1991, and the Radiance
//! file-format notes): a text header ending in an empty line, a resolution
//! line, then one scanline after another, each either flat (four bytes per
//! pixel) or run-length encoded per channel (the "new" encoding, which
//! starts with the bytes 2, 2 and the width). Every number is checked: a
//! malformed or hostile file is an error, never a panic or a huge
//! allocation.

use std::fmt;

/// Largest side of an environment picture this reader accepts.
pub const MAX_HDR_SIDE: usize = 16384;
/// Most pixels an environment picture may have (64 Mi).
pub const MAX_HDR_PIXELS: usize = 1 << 26;

/// Why an `.hdr` file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HdrError(pub String);

impl fmt::Display for HdrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a readable Radiance .hdr picture: {}", self.0)
    }
}

impl std::error::Error for HdrError {}

/// A decoded picture: linear RGB per pixel, row-major from the top-left.
#[derive(Clone, Debug, PartialEq)]
pub struct HdrImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<[f32; 3]>,
}

fn err(what: &str) -> HdrError {
    HdrError(what.to_string())
}

/// One RGBE pixel as linear RGB: `mantissa · 2^(e − 136)` (e = 0 is black).
fn rgbe([r, g, b, e]: [u8; 4]) -> [f32; 3] {
    if e == 0 {
        return [0.0; 3];
    }
    // 2^(e - 128) / 256, exact in f32 for every e in 1..=255.
    let scale = (f64::from(e) - 136.0).exp2() as f32;
    [
        f32::from(r) * scale,
        f32::from(g) * scale,
        f32::from(b) * scale,
    ]
}

/// Decodes a Radiance `.hdr` file.
pub fn decode_hdr(bytes: &[u8]) -> Result<HdrImage, HdrError> {
    let mut at = 0_usize;
    let mut line = || -> Result<&[u8], HdrError> {
        let rest = bytes.get(at..).ok_or_else(|| err("truncated header"))?;
        let end = rest
            .iter()
            .position(|&b| b == b'\n')
            .ok_or_else(|| err("truncated header"))?;
        let text = rest.get(..end).ok_or_else(|| err("truncated header"))?;
        at = at.saturating_add(end).saturating_add(1);
        Ok(text)
    };
    let first = line()?;
    if !(first.starts_with(b"#?RADIANCE") || first.starts_with(b"#?RGBE")) {
        return Err(err("no #?RADIANCE signature"));
    }
    let mut format_ok = true;
    let mut lines = 0;
    loop {
        lines += 1;
        if lines > 1024 {
            return Err(err("header too long"));
        }
        let text = line()?;
        if text.is_empty() {
            break;
        }
        if let Some(format) = text.strip_prefix(b"FORMAT=") {
            format_ok = format.trim_ascii() == b"32-bit_rle_rgbe";
        }
    }
    if !format_ok {
        return Err(err("only the 32-bit_rle_rgbe format is read"));
    }
    let resolution = std::str::from_utf8(line()?).map_err(|_| err("bad resolution line"))?;
    let words: Vec<&str> = resolution.split_whitespace().collect();
    let (height, width) = match words.as_slice() {
        ["-Y", h, "+X", w] => (
            h.parse::<usize>().map_err(|_| err("bad height"))?,
            w.parse::<usize>().map_err(|_| err("bad width"))?,
        ),
        _ => return Err(err("only the standard orientation -Y h +X w is read")),
    };
    if width == 0 || height == 0 || width > MAX_HDR_SIDE || height > MAX_HDR_SIDE {
        return Err(err("picture size out of range"));
    }
    let count = width
        .checked_mul(height)
        .filter(|&n| n <= MAX_HDR_PIXELS)
        .ok_or_else(|| err("picture too large"))?;
    let data = bytes.get(at..).ok_or_else(|| err("no pixel data"))?;
    // Reserved for no more pixels than there are bytes: run-length
    // encoding lets a few bytes claim many pixels, and a header alone must
    // not reserve hundreds of megabytes (an abort on WebAssembly).
    let mut pixels = Vec::with_capacity(count.min(data.len()));
    let mut pos = 0_usize;
    let mut scan = vec![[0_u8; 4]; width];
    for _ in 0..height {
        read_scanline(data, &mut pos, &mut scan)?;
        pixels.extend(scan.iter().map(|&p| rgbe(p)));
    }
    Ok(HdrImage {
        width,
        height,
        pixels,
    })
}

fn byte(data: &[u8], pos: &mut usize) -> Result<u8, HdrError> {
    let b = *data.get(*pos).ok_or_else(|| err("truncated pixel data"))?;
    *pos += 1;
    Ok(b)
}

fn read_scanline(data: &[u8], pos: &mut usize, scan: &mut [[u8; 4]]) -> Result<(), HdrError> {
    let width = scan.len();
    let head = data.get(*pos..pos.saturating_add(4));
    let rle = matches!(head, Some(&[2, 2, hi, lo]) if (8..0x8000).contains(&width)
        && (usize::from(hi) << 8 | usize::from(lo)) == width && hi & 0x80 == 0);
    if !rle {
        // Flat pixels (the old run-length scheme with repeat markers is
        // not written by any current tool and is refused).
        for px in scan.iter_mut() {
            *px = [
                byte(data, pos)?,
                byte(data, pos)?,
                byte(data, pos)?,
                byte(data, pos)?,
            ];
            if matches!(px, [1, 1, 1, _]) {
                return Err(err("old-style run-length encoding is not read"));
            }
        }
        return Ok(());
    }
    *pos += 4;
    for channel in 0..4 {
        let mut x = 0;
        while x < width {
            let count = usize::from(byte(data, pos)?);
            if count > 128 {
                let run = count - 128;
                let value = byte(data, pos)?;
                for px in scan
                    .get_mut(x..x + run)
                    .ok_or_else(|| err("run too long"))?
                {
                    if let Some(slot) = px.get_mut(channel) {
                        *slot = value;
                    }
                }
                x += run;
            } else {
                if count == 0 {
                    return Err(err("empty run"));
                }
                for px in scan
                    .get_mut(x..x + count)
                    .ok_or_else(|| err("run too long"))?
                {
                    let value = byte(data, pos)?;
                    if let Some(slot) = px.get_mut(channel) {
                        *slot = value;
                    }
                }
                x += count;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: usize, h: usize) -> Vec<u8> {
        format!("#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {h} +X {w}\n").into_bytes()
    }

    #[test]
    fn flat_pixels_decode() {
        let mut f = header(2, 1);
        f.extend([128, 64, 0, 129, 0, 0, 0, 0]);
        let img = decode_hdr(&f).unwrap();
        assert_eq!(img.pixels, vec![[1.0, 0.5, 0.0], [0.0; 3]]);
    }

    #[test]
    fn run_length_scanlines_decode() {
        let mut f = header(8, 1);
        f.extend([2, 2, 0, 8]);
        // Each channel: a run of 8 equal bytes.
        for v in [128, 128, 128, 128] {
            f.extend([128 + 8, v]);
        }
        let img = decode_hdr(&f).unwrap();
        assert_eq!(img.pixels, vec![[0.5; 3]; 8]);
    }

    #[test]
    fn hostile_files_are_errors() {
        assert!(decode_hdr(b"").is_err());
        assert!(decode_hdr(b"#?RADIANCE\n\n-Y 99999999 +X 99999999\n").is_err());
        assert!(decode_hdr(b"#?RADIANCE\n\n+X 2 -Y 2\n").is_err());
        let mut f = header(8, 1);
        f.extend([2, 2, 0, 8, 255, 1]);
        assert!(decode_hdr(&f).is_err());
        let mut f = header(2, 2);
        f.extend([1, 2, 3]);
        assert!(decode_hdr(&f).is_err());
        // The largest size allowed, with almost no pixel data: an error,
        // and no reservation for 2^26 pixels (805 MB, an abort on
        // WebAssembly) before it.
        let mut f = header(8192, 8192);
        f.extend([2, 2, 32, 0]);
        assert!(decode_hdr(&f).is_err());
        for cut in 0..40 {
            let mut f = header(8, 2);
            f.extend([2, 2, 0, 8, 136, 1, 136, 2, 136, 3, 136, 4]);
            f.truncate(cut);
            let _ = decode_hdr(&f);
        }
    }
}
