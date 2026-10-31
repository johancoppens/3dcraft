//! Fonts for 3DCraft's text: font files read into exact outlines, laid
//! out, and the built-in single-stroke font.
//!
//! - [`FontFile`] reads an OpenType or TrueType file with `ttf-parser`:
//!   names, metrics, the embedding permission (`OS/2 fsType`), glyph
//!   outlines exactly as the font has them (TrueType lines and quadratic
//!   Béziers, CFF lines and cubic Béziers) and kerning (GPOS pair
//!   adjustment of the `kern` feature, else the `kern` table).
//! - [`layout`] places a text's glyphs: advances, kerning, lines,
//!   alignment, wrapping to a width.
//! - [`fill_nonzero`] resolves the font's fill rule: overlapping or
//!   self-crossing contours become the simple boundary of the region
//!   they fill.
//! - [`StrokeFont`] is the single-stroke font, drawn for 3DCraft, as data.
//!
//! Font bytes are hostile input: every count is capped and nothing
//! panics (docs/design/text.md). Standalone: no workspace dependencies.

mod bezier;
mod builtin;
mod face;
mod fill;
mod layout;
mod stroke;

pub use bezier::{Contour, P2, Seg};
pub use builtin::{BUILTIN, BuiltinFont, STROKE, UI_FONTS, builtin, shipped, shipped_font};
pub use face::{Embedding, FontFile, FontInfo, OutlineKind};
pub use fill::{MAX_FILL_SEGMENTS, fill_nonzero};
pub use layout::{Align, Layout, LayoutOptions, Metrics, Placed, advance_width, layout};
pub use stroke::{STROKE_EM, StrokeFont};

use std::fmt;

/// The largest font file read.
pub const MAX_FONT_BYTES: usize = 32 << 20;
/// Most contours of one glyph.
pub const MAX_CONTOURS: usize = 2_000;
/// Most segments of one glyph.
pub const MAX_GLYPH_SEGMENTS: usize = 20_000;
/// Most characters of one text.
pub const MAX_TEXT_CHARS: usize = 10_000;

/// Why a font or a text could not be read or laid out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontError {
    TooLarge(usize),
    Malformed(String),
    TooComplex(String),
    MissingGlyph(char),
    Unsupported(String),
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FontError::TooLarge(n) => write!(
                f,
                "the font file has {n} bytes, more than the {MAX_FONT_BYTES} read"
            ),
            FontError::Malformed(why) => write!(f, "the font is malformed: {why}"),
            FontError::TooComplex(why) => write!(f, "too complex: {why}"),
            FontError::MissingGlyph(c) => write!(
                f,
                "the font has no glyph for {c:?} (U+{:04X})",
                u32::from(*c)
            ),
            FontError::Unsupported(why) => write!(f, "not supported yet: {why}"),
        }
    }
}

impl std::error::Error for FontError {}

/// A glyph's contours in font units, y up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    pub contours: Vec<Contour>,
}

impl Outline {
    /// The filled area (nonzero rule, for outlines that do not overlap:
    /// the sum of the contours' signed areas).
    pub fn area(&self) -> f64 {
        self.contours.iter().map(Contour::signed_area).sum()
    }
}

/// The outline of `glyph` with the font's fill rule resolved
/// ([`fill_nonzero`]): simple closed contours, outer ones
/// counter-clockwise, holes clockwise.
pub fn filled_outline(font: &FontFile<'_>, glyph: u16) -> Result<Outline, FontError> {
    let raw = font.outline(glyph)?;
    let eps = font.units_per_em() * 1e-7;
    Ok(Outline {
        contours: fill_nonzero(&raw.contours, eps)?,
    })
}
