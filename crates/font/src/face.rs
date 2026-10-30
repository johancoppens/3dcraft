//! A font file read with `ttf-parser`: its names, metrics, embedding
//! permission, glyph outlines and kerning.

use crate::bezier::{Contour, P2, Seg};
use crate::layout::Metrics;
use crate::{FontError, MAX_CONTOURS, MAX_FONT_BYTES, MAX_GLYPH_SEGMENTS, Outline};
use ttf_parser::{GlyphId, OutlineBuilder, Tag};

/// What the font's `OS/2 fsType` allows a document to do with the font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Embedding {
    /// Installable embedding (no restriction): may be stored.
    Installable,
    /// Editable embedding: may be stored.
    Editable,
    /// Preview and print only: the document must not be edited with it,
    /// so a CAD document does not store it.
    PreviewAndPrint,
    /// Restricted licence embedding: must not be stored.
    Restricted,
    /// Only bitmaps may be embedded (fsType bit 9): the outlines must not
    /// be stored.
    BitmapOnly,
}

impl Embedding {
    /// Whether a document may keep the font's bytes.
    pub fn allows_storing(self) -> bool {
        matches!(self, Embedding::Installable | Embedding::Editable)
    }

    pub fn name(self) -> &'static str {
        match self {
            Embedding::Installable => "installable",
            Embedding::Editable => "editable",
            Embedding::PreviewAndPrint => "previewAndPrint",
            Embedding::Restricted => "restricted",
            Embedding::BitmapOnly => "bitmapOnly",
        }
    }
}

/// Whether the outlines are TrueType quadratics or CFF cubics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineKind {
    Quadratic,
    Cubic,
}

/// What a font file says about itself.
#[derive(Clone, Debug, PartialEq)]
pub struct FontInfo {
    pub family: String,
    pub subfamily: String,
    pub full_name: String,
    pub units_per_em: f64,
    pub ascender: f64,
    pub descender: f64,
    pub line_gap: f64,
    /// The height of the capitals above the baseline: the font's own
    /// (`OS/2 sCapHeight`), or the top of its "H", or 0.7 em.
    pub cap_height: f64,
    pub embedding: Embedding,
    pub outlines: OutlineKind,
    pub glyphs: u16,
}

/// A parsed font file. Borrowed from its bytes; cheap to make again.
pub struct FontFile<'a> {
    face: ttf_parser::Face<'a>,
    cap_height: f64,
}

struct Builder {
    contours: Vec<Contour>,
    current: Vec<Seg>,
    at: P2,
    start: P2,
    segments: usize,
    overflow: bool,
}

impl Builder {
    fn push(&mut self, seg: Seg) {
        self.segments += 1;
        if self.segments > MAX_GLYPH_SEGMENTS {
            self.overflow = true;
            return;
        }
        self.at = seg.end();
        self.current.push(seg);
    }

    fn finish(&mut self) {
        if self.current.is_empty() {
            return;
        }
        if self.contours.len() >= MAX_CONTOURS {
            self.overflow = true;
            self.current.clear();
            return;
        }
        let segs = std::mem::take(&mut self.current);
        self.contours.push(Contour { segs, closed: true });
    }
}

fn p(x: f32, y: f32) -> P2 {
    [f64::from(x), f64::from(y)]
}

impl OutlineBuilder for Builder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish();
        self.at = p(x, y);
        self.start = self.at;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.push(Seg::line(self.at, p(x, y)));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.push(Seg::quad(self.at, p(x1, y1), p(x, y)));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.push(Seg::cubic(self.at, p(x1, y1), p(x2, y2), p(x, y)));
    }

    fn close(&mut self) {
        if self.current.last().is_some_and(|s| s.end() != self.start) {
            self.push(Seg::line(self.at, self.start));
        }
        self.finish();
    }
}

impl<'a> FontFile<'a> {
    /// Reads the first face of a font file (TrueType, OpenType, or a
    /// collection's first face).
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FontError> {
        if bytes.len() > MAX_FONT_BYTES {
            return Err(FontError::TooLarge(bytes.len()));
        }
        let face = ttf_parser::Face::parse(bytes, 0)
            .map_err(|e| FontError::Malformed(format!("not a font file this reads: {e}")))?;
        if face.units_per_em() == 0 {
            return Err(FontError::Malformed("its units per em are 0".into()));
        }
        let upem = f64::from(face.units_per_em());
        let cap = face
            .capital_height()
            .filter(|h| *h > 0)
            .map(f64::from)
            .or_else(|| {
                let h = face.glyph_index('H')?;
                let rect = face.glyph_bounding_box(h)?;
                (rect.y_max > 0).then(|| f64::from(rect.y_max))
            })
            .unwrap_or(0.7 * upem);
        Ok(Self {
            face,
            cap_height: cap,
        })
    }

    fn name(&self, ids: &[u16]) -> String {
        for id in ids {
            for record in self.face.names() {
                if record.name_id == *id
                    && record.is_unicode()
                    && let Some(text) = record.to_string()
                    && !text.trim().is_empty()
                {
                    return text.chars().take(200).collect();
                }
            }
        }
        String::new()
    }

    pub fn info(&self) -> FontInfo {
        use ttf_parser::name_id;
        let embedding = if self.face.tables().os2.is_none() {
            Embedding::Installable
        } else {
            match self.face.permissions() {
                Some(ttf_parser::Permissions::Restricted) | None => Embedding::Restricted,
                Some(_) if !self.face.is_outline_embedding_allowed() => Embedding::BitmapOnly,
                Some(ttf_parser::Permissions::PreviewAndPrint) => Embedding::PreviewAndPrint,
                Some(ttf_parser::Permissions::Editable) => Embedding::Editable,
                Some(ttf_parser::Permissions::Installable) => Embedding::Installable,
            }
        };
        let tables = self.face.tables();
        let cubic = tables.cff.is_some();
        FontInfo {
            family: self.name(&[name_id::TYPOGRAPHIC_FAMILY, name_id::FAMILY]),
            subfamily: self.name(&[name_id::TYPOGRAPHIC_SUBFAMILY, name_id::SUBFAMILY]),
            full_name: self.name(&[name_id::FULL_NAME]),
            units_per_em: f64::from(self.face.units_per_em()),
            ascender: f64::from(self.face.ascender()),
            descender: f64::from(self.face.descender()),
            line_gap: f64::from(self.face.line_gap()),
            cap_height: self.cap_height,
            embedding,
            outlines: if cubic {
                OutlineKind::Cubic
            } else {
                OutlineKind::Quadratic
            },
            glyphs: self.face.number_of_glyphs(),
        }
    }

    /// The outline of glyph `glyph`, exactly as the font draws it (lines
    /// and quadratic or cubic Béziers in font units, y up), contours not
    /// yet united.
    pub fn outline(&self, glyph: u16) -> Result<Outline, FontError> {
        let mut b = Builder {
            contours: Vec::new(),
            current: Vec::new(),
            at: [0.0; 2],
            start: [0.0; 2],
            segments: 0,
            overflow: false,
        };
        // No outline (a space) is an empty outline.
        let _ = self.face.outline_glyph(GlyphId(glyph), &mut b);
        b.finish();
        if b.overflow {
            return Err(FontError::TooComplex(format!(
                "glyph {glyph} has more than {MAX_CONTOURS} contours or {MAX_GLYPH_SEGMENTS} segments"
            )));
        }
        Ok(Outline {
            contours: b.contours,
        })
    }

    /// Kerning between two glyphs from the GPOS `kern` feature's pair
    /// adjustments, or, without one, from the `kern` table.
    fn pair_kerning(&self, left: u16, right: u16) -> f64 {
        let (l, r) = (GlyphId(left), GlyphId(right));
        if let Some(gpos) = self.face.tables().gpos
            && let Some(feature) = gpos.features.find(Tag::from_bytes(b"kern"))
        {
            let mut total = 0.0;
            let mut seen = 0usize;
            for index in feature.lookup_indices {
                seen += 1;
                if seen > 256 {
                    break;
                }
                let Some(lookup) = gpos.lookups.get(index) else {
                    continue;
                };
                for subtable in lookup
                    .subtables
                    .into_iter::<ttf_parser::gpos::PositioningSubtable>()
                    .take(256)
                {
                    let ttf_parser::gpos::PositioningSubtable::Pair(pair) = subtable else {
                        continue;
                    };
                    let Some(coverage) = pair.coverage().get(l) else {
                        continue;
                    };
                    let value = match pair {
                        ttf_parser::gpos::PairAdjustment::Format1 { sets, .. } => sets
                            .get(coverage)
                            .and_then(|set| set.get(r))
                            .map(|(a, _)| a.x_advance),
                        ttf_parser::gpos::PairAdjustment::Format2 {
                            classes, matrix, ..
                        } => matrix
                            .get((classes.0.get(l), classes.1.get(r)))
                            .map(|(a, _)| a.x_advance),
                    };
                    if let Some(v) = value {
                        total += f64::from(v);
                        // The first subtable of a lookup that holds the
                        // pair is the one that applies.
                        break;
                    }
                }
            }
            return total;
        }
        if let Some(kern) = self.face.tables().kern {
            for subtable in kern.subtables.into_iter().take(64) {
                if subtable.horizontal
                    && !subtable.variable
                    && !subtable.has_cross_stream
                    && let Some(v) = subtable.glyphs_kerning(l, r)
                {
                    return f64::from(v);
                }
            }
        }
        0.0
    }
}

impl Metrics for FontFile<'_> {
    fn units_per_em(&self) -> f64 {
        f64::from(self.face.units_per_em())
    }

    fn ascender(&self) -> f64 {
        f64::from(self.face.ascender())
    }

    fn descender(&self) -> f64 {
        f64::from(self.face.descender())
    }

    fn line_gap(&self) -> f64 {
        f64::from(self.face.line_gap())
    }

    fn cap_height(&self) -> f64 {
        self.cap_height
    }

    fn glyph(&self, c: char) -> Option<u16> {
        self.face.glyph_index(c).map(|g| g.0)
    }

    fn advance(&self, glyph: u16) -> f64 {
        self.face
            .glyph_hor_advance(GlyphId(glyph))
            .map_or(0.0, f64::from)
    }

    fn kerning(&self, left: u16, right: u16) -> f64 {
        self.pair_kerning(left, right)
    }
}
