//! Text in sketches, and the fonts a document keeps.
//!
//! A text is an entity of a sketch ([`crate::Geometry::Text`]): a template
//! with formulas in braces (`"Part {partNo}"`), a font, a size and the
//! point it is anchored at. It has no unknowns of its own: regeneration
//! evaluates the template, lays it out and puts the glyphs' outlines in
//! the sketch as closed profiles (or, in the single-stroke font, open
//! strokes), named `e7.glyph(3:U+0041).contour(1)` (docs/design/text.md).
//!
//! A document keeps the bytes of every font its texts use
//! ([`DocFont`]), so it opens the same everywhere; a font whose
//! `OS/2 fsType` forbids embedding is not kept, and the document records
//! that it needs that font by name and hash.

use crate::{Blob, EntityId, Expr};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Longest template of a text, in bytes.
pub const MAX_TEXT_TEMPLATE: usize = 10_000;
/// Most formulas in one template.
pub const MAX_TEXT_FORMULAS: usize = 64;
/// Most fonts one document keeps.
pub const MAX_DOC_FONTS: usize = 64;
/// Longest name of a font.
pub const MAX_FONT_NAME: usize = 200;

/// A piece of a template: text as it is, or a formula whose value is
/// written in its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextPart {
    Literal(String),
    Formula(Expr),
}

/// The text of a [`SketchText`]: literal text with formulas in braces,
/// `{{` and `}}` for a brace itself. Written to JSON as that one string.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TextTemplate {
    pub parts: Vec<TextPart>,
}

impl TextTemplate {
    /// Reads `Part {partNo} of {count}`.
    pub fn parse(source: &str) -> Result<Self, String> {
        if source.len() > MAX_TEXT_TEMPLATE {
            return Err(format!(
                "a text is at most {MAX_TEXT_TEMPLATE} bytes, this one {}",
                source.len()
            ));
        }
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = source.chars().peekable();
        let mut formulas = 0usize;
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    literal.push('}');
                }
                '}' => {
                    return Err(
                        "a `}` without its `{`: write `}}` for a brace in the text".to_string()
                    );
                }
                '{' => {
                    let mut formula = String::new();
                    let mut closed = false;
                    for d in chars.by_ref() {
                        if d == '}' {
                            closed = true;
                            break;
                        }
                        if d == '{' {
                            return Err("a formula in a text has no `{` inside it".to_string());
                        }
                        formula.push(d);
                    }
                    if !closed {
                        return Err(
                            "a `{` without its `}`: write `{{` for a brace in the text".to_string()
                        );
                    }
                    if formula.trim().is_empty() {
                        return Err("`{}` holds no formula".to_string());
                    }
                    formulas += 1;
                    if formulas > MAX_TEXT_FORMULAS {
                        return Err(format!("a text holds at most {MAX_TEXT_FORMULAS} formulas"));
                    }
                    if !literal.is_empty() {
                        parts.push(TextPart::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(TextPart::Formula(Expr::new(formula.trim())));
                }
                c => literal.push(c),
            }
        }
        if !literal.is_empty() {
            parts.push(TextPart::Literal(literal));
        }
        Ok(Self { parts })
    }

    /// The formulas, in order.
    pub fn formulas(&self) -> impl Iterator<Item = &Expr> {
        self.parts.iter().filter_map(|p| match p {
            TextPart::Formula(e) => Some(e),
            TextPart::Literal(_) => None,
        })
    }

    pub(crate) fn formulas_mut(&mut self) -> impl Iterator<Item = &mut Expr> {
        self.parts.iter_mut().filter_map(|p| match p {
            TextPart::Formula(e) => Some(e),
            TextPart::Literal(_) => None,
        })
    }
}

impl fmt::Display for TextTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for part in &self.parts {
            match part {
                TextPart::Literal(t) => f.write_str(&t.replace('{', "{{").replace('}', "}}"))?,
                TextPart::Formula(e) => write!(f, "{{{}}}", e.source())?,
            }
        }
        Ok(())
    }
}

impl Serialize for TextTemplate {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for TextTemplate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        TextTemplate::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// What a text's size measures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextSizeOf {
    /// The height of the capitals (an "H"): what a drawing dimensions.
    #[default]
    Cap,
    /// The em of the font, as a word processor's point size.
    Em,
}

/// Which line of the text's box sits on its point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAnchor {
    /// The baseline of the first line.
    #[default]
    Baseline,
    /// The top of the capitals of the first line.
    Top,
    /// Halfway between the top of the first line's capitals and the
    /// baseline of the last line.
    Middle,
    /// The baseline of the last line.
    Bottom,
}

/// How the lines sit against the point: its left, centre or right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Which side of a curve a text along it stands on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextSide {
    /// On the left of the curve, looking along it (outside a circle drawn
    /// counter-clockwise).
    #[default]
    Left,
    /// On the right, the glyphs turned over so they still read along the
    /// curve.
    Right,
}

/// A text set along a curve of its sketch instead of a straight
/// baseline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPath {
    /// The curve: a line, arc, circle or spline of the same sketch.
    pub curve: EntityId,
    /// Along the curve from its start (a circle's start is at angle 0) to
    /// where the text starts (for `align` center, its middle; right, its
    /// end): a length.
    #[serde(default = "zero")]
    pub start: Expr,
    /// From the curve to the baseline, square to it: a length.
    #[serde(default = "zero")]
    pub offset: Expr,
    #[serde(default)]
    pub side: TextSide,
    /// Run the curve the other way.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reversed: bool,
}

fn zero() -> Expr {
    Expr::new("0")
}

fn one() -> Expr {
    Expr::new("1")
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(e: &Expr) -> bool {
    e.source().trim() == "0"
}

fn is_one(e: &Expr) -> bool {
    e.source().trim() == "1"
}

/// A text in a sketch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SketchText {
    pub text: TextTemplate,
    /// A font of the document ([`DocFont::name`]), or `stroke`, the
    /// built-in single-stroke font.
    pub font: String,
    /// A length: the height of the capitals, or the em ([`TextSizeOf`]).
    pub size: Expr,
    #[serde(default, skip_serializing_if = "is_default_size_of")]
    pub size_of: TextSizeOf,
    /// The point the text is anchored at (a point entity of the sketch,
    /// so constraints place the text). For a text along a curve, unused
    /// for its position.
    pub at: EntityId,
    #[serde(default)]
    pub anchor: TextAnchor,
    #[serde(default)]
    pub align: TextAlign,
    /// The turn of the baseline from the sketch's x axis: an angle.
    #[serde(default = "zero", skip_serializing_if = "is_zero")]
    pub rotation: Expr,
    /// The distance between baselines, as a multiple of the font's line
    /// height: a plain number.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub line_spacing: Expr,
    /// Lines are broken at spaces to fit this length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Expr>,
    /// Set along a curve of the sketch instead of a straight line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<TextPath>,
}

fn is_default_size_of(s: &TextSizeOf) -> bool {
    *s == TextSizeOf::Cap
}

impl SketchText {
    /// Every expression with a name that says where it is (`e7.size`,
    /// `e7.text.0`).
    pub fn exprs(&self, id: EntityId) -> Vec<(String, &Expr)> {
        let mut out = vec![
            (format!("{id}.size"), &self.size),
            (format!("{id}.rotation"), &self.rotation),
            (format!("{id}.lineSpacing"), &self.line_spacing),
        ];
        if let Some(w) = &self.width {
            out.push((format!("{id}.width"), w));
        }
        if let Some(path) = &self.path {
            out.push((format!("{id}.path.start"), &path.start));
            out.push((format!("{id}.path.offset"), &path.offset));
        }
        for (k, e) in self.text.formulas().enumerate() {
            out.push((format!("{id}.text.{k}"), e));
        }
        out
    }

    pub(crate) fn exprs_mut(&mut self) -> Vec<&mut Expr> {
        let mut out = vec![&mut self.size, &mut self.rotation, &mut self.line_spacing];
        if let Some(w) = &mut self.width {
            out.push(w);
        }
        if let Some(path) = &mut self.path {
            out.push(&mut path.start);
            out.push(&mut path.offset);
        }
        out.extend(self.text.formulas_mut());
        out
    }

    /// The point entities it uses: its anchor point.
    pub fn points(&self) -> Vec<EntityId> {
        vec![self.at]
    }

    /// Why it cannot be, apart from its points (checked by the sketch).
    pub fn fault(&self) -> Option<String> {
        if self.font.trim().is_empty() || self.font.len() > MAX_FONT_NAME {
            return Some(format!(
                "a text needs a font name of 1 to {MAX_FONT_NAME} bytes"
            ));
        }
        if self.text.to_string().len() > MAX_TEXT_TEMPLATE {
            return Some(format!("a text is at most {MAX_TEXT_TEMPLATE} bytes"));
        }
        for (what, e) in [
            ("size", &self.size),
            ("rotation", &self.rotation),
            ("lineSpacing", &self.line_spacing),
        ] {
            if e.source().trim().is_empty() {
                return Some(format!("the text has no `{what}`"));
            }
        }
        None
    }
}

/// A font a document keeps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocFont {
    /// What texts call it (`font: "sans"`): unique in the document.
    pub name: String,
    /// The family and style the font file names itself.
    pub family: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub style: String,
    /// SHA-256 of the font file, lowercase hexadecimal.
    pub sha256: String,
    /// The font's embedding permission (`installable`, `editable`,
    /// `previewAndPrint`, `restricted`, `bitmapOnly`).
    pub embedding: String,
    /// The font file's bytes, when the font allows a document to keep
    /// them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<Blob>,
    /// The bytes of a font that may not be kept, for this session only:
    /// never saved. A document reopened without them says it needs the
    /// font (`font.add` it again).
    #[serde(skip)]
    pub session: Option<Blob>,
}

impl DocFont {
    /// The bytes to read the font from, if there are any.
    pub fn bytes(&self) -> Option<&Blob> {
        self.blob.as_ref().or(self.session.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_read_and_write_back() {
        let t = TextTemplate::parse("Part {partNo} of {{n}} {a + 1}").unwrap();
        assert_eq!(t.parts.len(), 4);
        assert_eq!(t.formulas().count(), 2);
        assert_eq!(t.to_string(), "Part {partNo} of {{n}} {a + 1}");
        assert_eq!(TextTemplate::parse(&t.to_string()).unwrap(), t);
    }

    #[test]
    fn bad_templates_are_errors() {
        for bad in ["{", "}", "a {b", "{}", "{a{b}}", "x}"] {
            assert!(TextTemplate::parse(bad).is_err(), "{bad}");
        }
        assert!(TextTemplate::parse(&"x".repeat(MAX_TEXT_TEMPLATE + 1)).is_err());
    }
}
