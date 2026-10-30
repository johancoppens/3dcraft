//! Laying out a text: glyphs left to right with their advances and the
//! kerning between them, lines broken at newlines and, with a width, at
//! spaces, each line aligned. All in font units, y up, the baseline of the
//! first line at y = 0. Complex scripts (shaping) are not done here.

use crate::{FontError, MAX_TEXT_CHARS};

/// What layout needs from a font.
pub trait Metrics {
    fn units_per_em(&self) -> f64;
    fn ascender(&self) -> f64;
    /// Below the baseline: negative.
    fn descender(&self) -> f64;
    fn line_gap(&self) -> f64;
    fn cap_height(&self) -> f64;
    /// The glyph for a character, if the font has one.
    fn glyph(&self, c: char) -> Option<u16>;
    fn advance(&self, glyph: u16) -> f64;
    /// The change in advance between two glyphs that follow each other.
    fn kerning(&self, left: u16, right: u16) -> f64;
}

/// How the lines sit against the anchor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutOptions {
    /// The distance between baselines as a multiple of the font's line
    /// height (ascender − descender + line gap).
    pub line_spacing: f64,
    /// Lines are broken at spaces to fit this width (font units).
    pub width: Option<f64>,
    pub align: Align,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            line_spacing: 1.0,
            width: None,
            align: Align::Left,
        }
    }
}

/// One glyph placed: `index` is the character's position in the text
/// (counting every character, spaces and newlines too).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub index: usize,
    pub ch: char,
    pub glyph: u16,
    /// The glyph's origin on the baseline.
    pub x: f64,
    pub y: f64,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub glyphs: Vec<Placed>,
    /// Per line: its advance width (kerning included) and its baseline y.
    pub lines: Vec<(f64, f64)>,
    pub ascender: f64,
    pub descender: f64,
}

impl Layout {
    /// The widest line.
    pub fn width(&self) -> f64 {
        self.lines.iter().map(|l| l.0).fold(0.0, f64::max)
    }

    /// From the ascender of the first line to the descender of the last.
    pub fn height(&self) -> f64 {
        let last = self.lines.last().map_or(0.0, |l| l.1);
        self.ascender - (last + self.descender)
    }
}

/// The advance width of one line of `text` (no breaking): the sum of the
/// advances and the kerning between neighbours.
pub fn advance_width(m: &dyn Metrics, text: &str) -> Result<f64, FontError> {
    let mut total = 0.0;
    let mut prev: Option<u16> = None;
    for c in text.chars() {
        let g = glyph_of(m, c)?;
        if let Some(p) = prev {
            total += m.kerning(p, g);
        }
        total += m.advance(g);
        prev = Some(g);
    }
    Ok(total)
}

fn glyph_of(m: &dyn Metrics, c: char) -> Result<u16, FontError> {
    let c = if c == '\t' { ' ' } else { c };
    m.glyph(c).ok_or(FontError::MissingGlyph(c))
}

/// A word or a run of spaces of a line, as characters with their indices.
fn tokens(line: &[(usize, char)]) -> Vec<&[(usize, char)]> {
    let mut out = Vec::new();
    let mut start = 0;
    for k in 1..=line.len() {
        let space = |i: usize| line.get(i).is_some_and(|(_, c)| c.is_whitespace());
        if k == line.len() || space(k) != space(k - 1) {
            if let Some(t) = line.get(start..k) {
                out.push(t);
            }
            start = k;
        }
    }
    out
}

/// Lays out `text`. A character the font has no glyph for is an error.
pub fn layout(m: &dyn Metrics, text: &str, options: &LayoutOptions) -> Result<Layout, FontError> {
    let count = text.chars().count();
    if count > MAX_TEXT_CHARS {
        return Err(FontError::TooComplex(format!(
            "the text has {count} characters, more than {MAX_TEXT_CHARS}"
        )));
    }
    if !(options.line_spacing.is_finite() && options.line_spacing > 0.0) {
        return Err(FontError::Malformed(
            "the line spacing is not positive".into(),
        ));
    }
    if let Some(w) = options.width
        && !(w.is_finite() && w > 0.0)
    {
        return Err(FontError::Malformed("the width is not positive".into()));
    }
    let line_height = (m.ascender() - m.descender() + m.line_gap()).max(m.units_per_em() * 0.5)
        * options.line_spacing;
    let chars: Vec<(usize, char)> = text
        .chars()
        .enumerate()
        .filter(|(_, c)| *c != '\r')
        .collect();
    // Hard lines.
    let mut hard: Vec<Vec<(usize, char)>> = vec![Vec::new()];
    for &(i, c) in &chars {
        if c == '\n' {
            hard.push(Vec::new());
        } else if let Some(line) = hard.last_mut() {
            line.push((i, c));
        }
    }
    // Soft lines: broken at spaces to fit the width.
    let width_of =
        |run: &[(usize, char)], before: Option<u16>| -> Result<(f64, Option<u16>), FontError> {
            let mut w = 0.0;
            let mut prev = before;
            for &(_, c) in run {
                let g = glyph_of(m, c)?;
                if let Some(p) = prev {
                    w += m.kerning(p, g);
                }
                w += m.advance(g);
                prev = Some(g);
            }
            Ok((w, prev))
        };
    let mut lines: Vec<Vec<(usize, char)>> = Vec::new();
    for line in hard {
        let Some(limit) = options.width else {
            lines.push(line);
            continue;
        };
        let mut current: Vec<(usize, char)> = Vec::new();
        for token in tokens(&line) {
            let is_space = token.first().is_some_and(|(_, c)| c.is_whitespace());
            let mut tried = current.clone();
            tried.extend_from_slice(token);
            let (w, _) = width_of(&tried, None)?;
            if w <= limit || current.is_empty() && is_space {
                current = tried;
                continue;
            }
            if is_space {
                // The spaces at a break are dropped.
                lines.push(std::mem::take(&mut current));
                continue;
            }
            if !current.is_empty() {
                // Trailing spaces of the line are dropped.
                while current.last().is_some_and(|(_, c)| c.is_whitespace()) {
                    current.pop();
                }
                lines.push(std::mem::take(&mut current));
            }
            // A word wider than the line is broken between characters.
            for &ch in token {
                let mut tried = current.clone();
                tried.push(ch);
                let (w, _) = width_of(&tried, None)?;
                if w > limit && !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                    current.push(ch);
                } else {
                    current = tried;
                }
            }
        }
        while current.last().is_some_and(|(_, c)| c.is_whitespace()) && options.width.is_some() {
            current.pop();
        }
        lines.push(current);
    }
    let mut glyphs = Vec::new();
    let mut boxes = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let y = -(n as f64) * line_height;
        let (w, _) = width_of(line, None)?;
        let shift = match options.align {
            Align::Left => 0.0,
            Align::Center => -w / 2.0,
            Align::Right => -w,
        };
        let mut x = shift;
        let mut prev: Option<u16> = None;
        for &(index, ch) in line {
            let g = glyph_of(m, ch)?;
            if let Some(p) = prev {
                x += m.kerning(p, g);
            }
            glyphs.push(Placed {
                index,
                ch,
                glyph: g,
                x,
                y,
                line: n,
            });
            x += m.advance(g);
            prev = Some(g);
        }
        boxes.push((w, y));
    }
    Ok(Layout {
        glyphs,
        lines: boxes,
        ascender: m.ascender(),
        descender: m.descender(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph 10 wide; "AV" kerned by −3.
    struct Fixed;

    impl Metrics for Fixed {
        fn units_per_em(&self) -> f64 {
            100.0
        }
        fn ascender(&self) -> f64 {
            80.0
        }
        fn descender(&self) -> f64 {
            -20.0
        }
        fn line_gap(&self) -> f64 {
            0.0
        }
        fn cap_height(&self) -> f64 {
            70.0
        }
        fn glyph(&self, c: char) -> Option<u16> {
            (c.is_ascii_graphic() || c == ' ').then_some(c as u16)
        }
        fn advance(&self, _: u16) -> f64 {
            10.0
        }
        fn kerning(&self, l: u16, r: u16) -> f64 {
            if (l, r) == ('A' as u16, 'V' as u16) {
                -3.0
            } else {
                0.0
            }
        }
    }

    #[test]
    fn advances_and_kerning_add_up() {
        assert_eq!(advance_width(&Fixed, "AVA").unwrap(), 27.0);
        let l = layout(&Fixed, "AV", &LayoutOptions::default()).unwrap();
        assert_eq!(l.glyphs[1].x, 7.0);
    }

    #[test]
    fn lines_wrap_at_spaces_and_align() {
        let o = LayoutOptions {
            width: Some(75.0),
            align: Align::Right,
            ..LayoutOptions::default()
        };
        let l = layout(&Fixed, "abc def ghi", &o).unwrap();
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].0, 70.0); // "abc def"
        assert_eq!(l.lines[1].1, -100.0);
        // Right aligned: the last glyph of each line ends at x = 0.
        let last = l.glyphs.iter().filter(|g| g.line == 1).last().unwrap();
        assert_eq!(last.x + 10.0, 0.0);
        // Indices are positions in the text.
        assert_eq!(last.index, 10);
    }

    #[test]
    fn a_word_wider_than_the_line_breaks_between_characters() {
        let o = LayoutOptions {
            width: Some(25.0),
            ..LayoutOptions::default()
        };
        let l = layout(&Fixed, "abcdef", &o).unwrap();
        assert_eq!(l.lines.len(), 3);
    }

    #[test]
    fn a_missing_glyph_is_named() {
        let e = layout(&Fixed, "aé", &LayoutOptions::default()).unwrap_err();
        assert_eq!(e, FontError::MissingGlyph('é'));
    }
}
