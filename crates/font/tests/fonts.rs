//! The built-in fonts read into outlines, checked against independent
//! oracles. The fonts are fetched by `cargo xtask fonts` (never committed):
//! `$CRAFT3D_FONTS`, or `assets/fonts/` of the repository.

use craft3d_font::{
    Embedding, FontFile, LayoutOptions, Metrics, OutlineKind, Seg, StrokeFont, advance_width,
    filled_outline, layout,
};
use std::path::PathBuf;

fn font_bytes(file: &str) -> Vec<u8> {
    let dir = match std::env::var_os("CRAFT3D_FONTS") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
    };
    let path = dir.join(file);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; fetch the built-in fonts with `cargo xtask fonts`",
            path.display()
        )
    })
}

fn sans() -> Vec<u8> {
    font_bytes("NotoSans-Regular.otf")
}

fn mono() -> Vec<u8> {
    font_bytes("JetBrainsMono-Regular.ttf")
}

/// The area of contours by the shoelace formula over a dense sampling of
/// each segment: independent of the Gauss quadrature the crate uses.
fn sampled_area(contours: &[craft3d_font::Contour]) -> f64 {
    let mut a = 0.0;
    for c in contours {
        let mut pts = Vec::new();
        for s in &c.segs {
            let n = if s.degree == 1 { 1 } else { 2000 };
            for k in 0..n {
                pts.push(s.eval(k as f64 / n as f64));
            }
        }
        for k in 0..pts.len() {
            let p = pts[k];
            let q = pts[(k + 1) % pts.len()];
            a += p[0] * q[1] - p[1] * q[0];
        }
    }
    a / 2.0
}

#[test]
fn the_built_in_fonts_read_with_their_outline_kinds() {
    let bytes = sans();
    let f = FontFile::parse(&bytes).unwrap();
    let info = f.info();
    assert_eq!(info.outlines, OutlineKind::Cubic);
    assert_eq!(info.family, "Noto Sans");
    assert!(info.embedding.allows_storing());
    assert!(
        info.cap_height > 600.0 && info.cap_height < 800.0,
        "{}",
        info.cap_height
    );
    let bytes = mono();
    let f = FontFile::parse(&bytes).unwrap();
    assert_eq!(f.info().outlines, OutlineKind::Quadratic);
    assert_eq!(f.info().family, "JetBrains Mono");
}

#[test]
fn quadratic_and_cubic_segments_come_out_exactly() {
    let bytes = mono();
    let f = FontFile::parse(&bytes).unwrap();
    let o = f.outline(f.glyph('O').unwrap()).unwrap();
    let degrees: Vec<usize> = o
        .contours
        .iter()
        .flat_map(|c| c.segs.iter().map(|s| s.degree))
        .collect();
    assert!(degrees.iter().all(|d| *d <= 2) && degrees.contains(&2));
    let bytes = sans();
    let f = FontFile::parse(&bytes).unwrap();
    let o = f.outline(f.glyph('O').unwrap()).unwrap();
    let degrees: Vec<usize> = o
        .contours
        .iter()
        .flat_map(|c| c.segs.iter().map(|s| s.degree))
        .collect();
    assert!(degrees.contains(&3) && !degrees.contains(&2));
}

#[test]
fn glyph_areas_match_the_outline_integrated_independently() {
    for bytes in [sans(), mono()] {
        let f = FontFile::parse(&bytes).unwrap();
        for c in "ABOQRabegos08&@%".chars() {
            let g = f.glyph(c).unwrap();
            let raw = f.outline(g).unwrap();
            let filled = filled_outline(&f, g).unwrap();
            let independent = sampled_area(&raw.contours).abs();
            let ours = filled.area();
            assert!(ours > 0.0, "{c}");
            assert!(
                (ours - independent).abs() <= 1e-5 * independent,
                "{c}: {ours} vs {independent}"
            );
        }
    }
}

#[test]
fn an_o_has_a_hole_and_glyphs_have_their_regions() {
    for bytes in [sans(), mono()] {
        let f = FontFile::parse(&bytes).unwrap();
        // (character, outer loops, holes)
        for (c, outer, holes) in [
            ('O', 1, 1),
            ('B', 1, 2),
            ('8', 1, 2),
            ('i', 2, 0),
            ('l', 1, 0),
            ('%', 3, 2),
            (':', 2, 0),
        ] {
            let o = filled_outline(&f, f.glyph(c).unwrap()).unwrap();
            let pos = o.contours.iter().filter(|k| k.signed_area() > 0.0).count();
            let neg = o.contours.iter().filter(|k| k.signed_area() < 0.0).count();
            assert_eq!((pos, neg), (outer, holes), "{c}");
        }
    }
}

#[test]
fn a_string_advances_by_the_sum_of_advances_and_kerning() {
    let bytes = sans();
    let f = FontFile::parse(&bytes).unwrap();
    let text = "AVATAR Type";
    let glyphs: Vec<u16> = text.chars().map(|c| f.glyph(c).unwrap()).collect();
    let advances: f64 = glyphs.iter().map(|g| f.advance(*g)).sum();
    let kerning: f64 = glyphs.windows(2).map(|w| f.kerning(w[0], w[1])).sum();
    // The GPOS kern feature is read: "AV" and "Ty" are kerned in.
    assert!(f.kerning(glyphs[0], glyphs[1]) < 0.0);
    assert!(kerning < 0.0);
    let w = advance_width(&f, text).unwrap();
    assert!((w - (advances + kerning)).abs() < 1e-9);
    let l = layout(&f, text, &LayoutOptions::default()).unwrap();
    assert!((l.width() - w).abs() < 1e-9);
    let last = l.glyphs.last().unwrap();
    assert!((last.x + f.advance(last.glyph) - w).abs() < 1e-9);
}

#[test]
fn the_mono_font_has_one_advance() {
    let bytes = mono();
    let f = FontFile::parse(&bytes).unwrap();
    let a = f.advance(f.glyph('i').unwrap());
    assert_eq!(a, f.advance(f.glyph('W').unwrap()));
    assert!((advance_width(&f, "iWiW").unwrap() - 4.0 * a).abs() < 1e-9);
}

/// The byte offset of table `tag` in an OpenType file.
fn table_offset(bytes: &[u8], tag: &[u8; 4]) -> Option<usize> {
    let n = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    (0..n).find_map(|i| {
        let rec = 12 + 16 * i;
        (&bytes[rec..rec + 4] == tag).then(|| {
            u32::from_be_bytes([
                bytes[rec + 8],
                bytes[rec + 9],
                bytes[rec + 10],
                bytes[rec + 11],
            ]) as usize
        })
    })
}

/// The font with `OS/2 fsType` set to `value`.
pub fn with_fs_type(bytes: &[u8], value: u16) -> Vec<u8> {
    let mut b = bytes.to_vec();
    let os2 = table_offset(&b, b"OS/2").unwrap();
    b[os2 + 8..os2 + 10].copy_from_slice(&value.to_be_bytes());
    b
}

#[test]
fn the_embedding_permission_is_read_from_fs_type() {
    let bytes = sans();
    for (value, expected) in [
        (0x0000, Embedding::Installable),
        (0x0002, Embedding::Restricted),
        (0x0004, Embedding::PreviewAndPrint),
        (0x0008, Embedding::Editable),
        (0x0208, Embedding::BitmapOnly),
    ] {
        let b = with_fs_type(&bytes, value);
        let f = FontFile::parse(&b).unwrap();
        assert_eq!(f.info().embedding, expected, "{value:#06x}");
    }
    assert!(!Embedding::Restricted.allows_storing());
    assert!(!Embedding::PreviewAndPrint.allows_storing());
}

/// A tiny seeded generator (no dependency): xorshift64*.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

#[test]
fn hostile_font_bytes_never_panic() {
    for (seed, base) in [(1u64, sans()), (2, mono())] {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ seed);
        for round in 0..120 {
            let mut b = base.clone();
            match round % 3 {
                0 => {
                    let cut = (rng.next() as usize) % b.len();
                    b.truncate(cut);
                }
                _ => {
                    for _ in 0..(1 + rng.next() % 64) {
                        let at = (rng.next() as usize) % b.len();
                        b[at] = rng.next() as u8;
                    }
                }
            }
            let Ok(f) = FontFile::parse(&b) else { continue };
            let _ = f.info();
            let _ = advance_width(&f, "Hello, wörld!");
            for c in "AaO8%@é".chars() {
                if let Some(g) = f.glyph(c) {
                    let _ = filled_outline(&f, g);
                }
            }
            for g in 0..f.info().glyphs.min(40) {
                let _ = f.outline(g);
            }
        }
    }
    assert!(FontFile::parse(&[]).is_err());
    assert!(FontFile::parse(&[0u8; 12]).is_err());
}

#[test]
fn the_stroke_font_is_open_strokes() {
    let f = StrokeFont;
    let o = f.outline(f.glyph('T').unwrap()).unwrap();
    assert_eq!(o.contours.len(), 2);
    assert!(o.contours.iter().all(|c| !c.closed));
    let o = f.outline(f.glyph('o').unwrap()).unwrap();
    assert!(o.contours.iter().all(|c| c.closed));
    let _: &[Seg] = &o.contours[0].segs;
}
