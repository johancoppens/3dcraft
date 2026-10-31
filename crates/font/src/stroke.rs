//! The built-in single-stroke font, for engraving, CNC, laser and pen
//! plotters: every glyph is open strokes (lines and arcs, the arcs as
//! cubic Béziers), not filled outlines.
//!
//! Drawn for 3DCraft (the Hershey data's terms were not clean enough to
//! carry: docs/design/text.md). The glyphs are data in this file, on a grid
//! of 30 units to the em: baseline 0, x-height 14, capitals 21, descender
//! −7. A glyph is a path in a small language: `M x y` starts a stroke,
//! `L x y` draws a line, `A cx cy rx ry a0 a1` an elliptic arc from angle
//! `a0` to `a1` (degrees, either way round; a line first if the pen is not
//! at its start), `E cx cy rx ry` a closed ellipse of its own. Accented
//! letters are a base letter with a mark above (or a cedilla below).

use crate::bezier::{Contour, P2, Seg, dist};
use crate::layout::Metrics;
use crate::{FontError, Outline};

/// Units per em of the stroke font.
pub const STROKE_EM: f64 = 30.0;
const CAP: f64 = 21.0;
const X_HEIGHT: f64 = 14.0;
const BEARING: f64 = 2.0;
const SPACING: f64 = 4.0;
const SPACE: f64 = 10.0;

/// (character, width of the drawing, path).
const GLYPHS: &[(char, f64, &str)] = &[
    ('A', 14.0, "M0 0 L7 21 L14 0 M2.5 7.5 L11.5 7.5"),
    (
        'B',
        12.75,
        "M0 0 L0 21 L7 21 A7 15.75 5.25 5.25 90 -90 L0 10.5 M7 10.5 L7.5 10.5 A7.5 5.25 5.25 5.25 90 -90 L0 0",
    ),
    ('C', 15.0, "A8.5 10.5 8.5 10.5 40 320"),
    ('D', 13.0, "M0 0 L0 21 L4 21 A4 10.5 9 10.5 90 -90 L0 0"),
    ('E', 12.0, "M12 21 L0 21 L0 0 L12 0 M0 10.5 L8 10.5"),
    ('F', 12.0, "M12 21 L0 21 L0 0 M0 10.5 L8 10.5"),
    ('G', 17.0, "A8.5 10.5 8.5 10.5 40 360 L10 10.5"),
    ('H', 14.0, "M0 0 L0 21 M14 0 L14 21 M0 10.5 L14 10.5"),
    ('I', 0.0, "M0 0 L0 21"),
    ('J', 10.0, "M10 21 L10 5 A5 5 5 5 0 -180"),
    ('K', 14.0, "M0 0 L0 21 M14 21 L0 7 M5 12 L14 0"),
    ('L', 11.0, "M0 21 L0 0 L11 0"),
    ('M', 17.0, "M0 0 L0 21 L8.5 0 L17 21 L17 0"),
    ('N', 14.0, "M0 0 L0 21 L14 0 L14 21"),
    ('O', 17.0, "E8.5 10.5 8.5 10.5"),
    ('P', 12.5, "M0 0 L0 21 L7 21 A7 15.5 5.5 5.5 90 -90 L0 10"),
    ('Q', 17.0, "E8.5 10.5 8.5 10.5 M10 5 L17 -1"),
    (
        'R',
        13.0,
        "M0 0 L0 21 L7 21 A7 15.5 5.5 5.5 90 -90 L0 10 M6 10 L13 0",
    ),
    (
        'S',
        13.0,
        "A6.5 15.75 6.5 5.25 20 270 A6.5 5.25 6.5 5.25 90 -160",
    ),
    ('T', 14.0, "M0 21 L14 21 M7 21 L7 0"),
    ('U', 14.0, "M0 21 L0 7 A7 7 7 7 180 360 L14 21"),
    ('V', 14.0, "M0 21 L7 0 L14 21"),
    ('W', 20.0, "M0 21 L5 0 L10 21 L15 0 L20 21"),
    ('X', 14.0, "M0 21 L14 0 M14 21 L0 0"),
    ('Y', 14.0, "M0 21 L7 10.5 L14 21 M7 10.5 L7 0"),
    ('Z', 14.0, "M0 21 L14 21 L0 0 L14 0"),
    ('a', 12.0, "E6 7 6 7 M12 14 L12 0"),
    ('b', 12.0, "M0 21 L0 0 E6 7 6 7"),
    ('c', 12.0, "A6.5 7 6.5 7 40 320"),
    ('d', 12.0, "E6 7 6 7 M12 21 L12 0"),
    ('e', 12.0, "M0.5 7 L12 7 A6 7 6 7 0 320"),
    ('f', 9.0, "M10 21 L7 21 A7 18 3 3 90 180 L4 0 M0 14 L9 14"),
    ('g', 12.0, "E6 7 6 7 M12 14 L12 -2 A6 -2 6 5 0 -180"),
    ('h', 12.0, "M0 21 L0 0 M0 8 A6 8 6 6 180 0 L12 0"),
    ('i', 0.0, "M0 14 L0 0 M0 19 L0 20"),
    ('j', 4.0, "M4 14 L4 -3 A1 -3 3 4 0 -160 M4 19 L4 20"),
    ('k', 12.0, "M0 21 L0 0 M11 14 L0 5 M4 8 L12 0"),
    ('l', 0.0, "M0 21 L0 0"),
    (
        'm',
        16.0,
        "M0 14 L0 0 M0 9 A4 9 4 5 180 0 L8 0 M8 9 A12 9 4 5 180 0 L16 0",
    ),
    ('n', 12.0, "M0 14 L0 0 M0 8 A6 8 6 6 180 0 L12 0"),
    ('o', 12.0, "E6 7 6 7"),
    ('p', 12.0, "M0 14 L0 -7 E6 7 6 7"),
    ('q', 12.0, "E6 7 6 7 M12 14 L12 -7"),
    ('r', 9.0, "M0 14 L0 0 M0 8 A6 8 6 6 180 60"),
    (
        's',
        11.0,
        "A5.5 10.5 5.5 3.5 20 270 A5.5 3.5 5.5 3.5 90 -160",
    ),
    ('t', 9.0, "M3 19 L3 3 A6 3 3 3 180 270 L9 0 M0 14 L8 14"),
    ('u', 12.0, "M0 14 L0 6 A6 6 6 6 180 360 M12 14 L12 0"),
    ('v', 12.0, "M0 14 L6 0 L12 14"),
    ('w', 16.0, "M0 14 L4 0 L8 14 L12 0 L16 14"),
    ('x', 12.0, "M0 14 L12 0 M12 14 L0 0"),
    ('y', 12.0, "M0 14 L6.5 1 M12 14 L3 -7"),
    ('z', 12.0, "M0 14 L12 14 L0 0 L12 0"),
    ('ı', 0.0, "M0 14 L0 0"),
    ('0', 12.0, "E6 10.5 6 10.5"),
    ('1', 7.0, "M3 17 L7 21 L7 0"),
    ('2', 12.0, "A6 15 6 6 160 -30 L0 0 L12 0"),
    ('3', 12.0, "A6 16 5.5 5 150 -90 A6 5.5 6 5.5 90 -150"),
    ('4', 13.0, "M9 0 L9 21 L0 6 L13 6"),
    ('5', 12.0, "M11 21 L1.5 21 L0.8 11.5 A6 6.5 6 6.5 125 -150"),
    ('6', 12.0, "E6 6.5 6 6.5 A11 6.5 11 14.5 100 180"),
    ('7', 12.0, "M0 21 L12 21 L4 0"),
    ('8', 12.0, "E6 15.75 5.25 5.25 E6 5.25 6 5.25"),
    ('9', 12.0, "E6 14.5 6 6.5 A1 14.5 11 14.5 0 -80"),
    ('.', 0.0, "M0 0 L0 1"),
    (',', 1.0, "M1 1 L1 0 L0 -3"),
    (':', 0.0, "M0 14 L0 13 M0 1 L0 0"),
    (';', 1.0, "M1 14 L1 13 M1 1 L1 0 L0 -3"),
    ('!', 0.0, "M0 21 L0 6 M0 1 L0 0"),
    ('?', 10.0, "A5 16 5 5 180 -90 L5 6 M5 1 L5 0"),
    ('-', 8.0, "M0 8 L8 8"),
    ('+', 12.0, "M0 9 L12 9 M6 3 L6 15"),
    ('=', 12.0, "M0 6 L12 6 M0 12 L12 12"),
    ('/', 10.0, "M0 -2 L10 23"),
    ('\\', 10.0, "M0 23 L10 -2"),
    ('(', 3.84, "A9 9.5 9 14 125 235"),
    (')', 3.84, "A-5.16 9.5 9 14 55 -55"),
    ('[', 4.0, "M4 23 L0 23 L0 -2 L4 -2"),
    (']', 4.0, "M0 23 L4 23 L4 -2 L0 -2"),
    ('{', 5.0, "M5 23 L3 23 L2.5 12 L0 10.5 L2.5 9 L3 -2 L5 -2"),
    ('}', 5.0, "M0 23 L2 23 L2.5 12 L5 10.5 L2.5 9 L2 -2 L0 -2"),
    ('\'', 0.0, "M0 21 L0 17"),
    ('"', 3.0, "M0 21 L0 17 M3 21 L3 17"),
    ('`', 3.0, "M0 21 L3 17"),
    ('#', 13.0, "M3 0 L5 21 M8 0 L10 21 M0 7 L12 7 M1 14 L13 14"),
    ('%', 14.0, "M0 0 L14 21 E3 18 3 3 E11 3 3 3"),
    (
        '&',
        14.0,
        "M14 0 L3.5 14 A6 17 3.5 4 210 -30 L1.5 6.5 A5.5 4.5 4.5 4.5 150 270 L8 0.3 L13 7",
    ),
    ('*', 10.0, "M5 21 L5 9 M0 18 L10 12 M10 18 L0 12"),
    ('_', 14.0, "M0 -3 L14 -3"),
    ('<', 12.0, "M12 18 L0 9 L12 0"),
    ('>', 12.0, "M0 18 L12 9 L0 0"),
    (
        '@',
        17.0,
        "E8.5 9.5 3.5 4 M12 13.5 L12 7 A14.5 7 2.5 2 180 360 A8.5 9.5 8.5 10 20 300",
    ),
    (
        '$',
        13.0,
        "A6.5 15.75 6.5 5.25 20 270 A6.5 5.25 6.5 5.25 90 -160 M6.5 -2 L6.5 23",
    ),
    ('^', 10.0, "M0 15 L5 21 L10 15"),
    ('~', 12.0, "M0 9 A3 9 3 2 180 0 A9 9 3 2 180 360"),
    ('|', 0.0, "M0 -3 L0 24"),
    ('°', 6.0, "E3 18 3 3"),
    (
        '€',
        15.0,
        "A9 10.5 8 10.5 40 320 M0 8.5 L10 8.5 M0 12.5 L10 12.5",
    ),
];

/// Marks drawn above (or below) a base letter, about x = 0 at height 0.
const MARKS: &[(&str, &str)] = &[
    ("acute", "M-1.5 0 L1.5 3"),
    ("grave", "M-1.5 3 L1.5 0"),
    ("circumflex", "M-3 0 L0 3 L3 0"),
    ("diaeresis", "M-2.5 0 L-2.5 1 M2.5 0 L2.5 1"),
    ("tilde", "M-4 0.5 A-2 0.5 2 1.5 180 0 A2 0.5 2 1.5 180 360"),
    ("ring", "E0 1.5 1.5 1.5"),
    ("caron", "M-3 3 L0 0 L3 3"),
    ("cedilla", "M0 0 L0 -2 L2 -3.5 L0 -5"),
    ("slash", "M-8 -1 L8 22"),
];

/// Accented letters: (character, base, mark).
const COMPOSED: &[(char, char, &str)] = &[
    ('à', 'a', "grave"),
    ('á', 'a', "acute"),
    ('â', 'a', "circumflex"),
    ('ã', 'a', "tilde"),
    ('ä', 'a', "diaeresis"),
    ('å', 'a', "ring"),
    ('è', 'e', "grave"),
    ('é', 'e', "acute"),
    ('ê', 'e', "circumflex"),
    ('ë', 'e', "diaeresis"),
    ('ě', 'e', "caron"),
    ('ì', 'ı', "grave"),
    ('í', 'ı', "acute"),
    ('î', 'ı', "circumflex"),
    ('ï', 'ı', "diaeresis"),
    ('ò', 'o', "grave"),
    ('ó', 'o', "acute"),
    ('ô', 'o', "circumflex"),
    ('õ', 'o', "tilde"),
    ('ö', 'o', "diaeresis"),
    ('ø', 'o', "slash"),
    ('ù', 'u', "grave"),
    ('ú', 'u', "acute"),
    ('û', 'u', "circumflex"),
    ('ü', 'u', "diaeresis"),
    ('ý', 'y', "acute"),
    ('ÿ', 'y', "diaeresis"),
    ('ñ', 'n', "tilde"),
    ('ç', 'c', "cedilla"),
    ('š', 's', "caron"),
    ('ž', 'z', "caron"),
    ('č', 'c', "caron"),
    ('ř', 'r', "caron"),
    ('ň', 'n', "caron"),
    ('À', 'A', "grave"),
    ('Á', 'A', "acute"),
    ('Â', 'A', "circumflex"),
    ('Ã', 'A', "tilde"),
    ('Ä', 'A', "diaeresis"),
    ('Å', 'A', "ring"),
    ('È', 'E', "grave"),
    ('É', 'E', "acute"),
    ('Ê', 'E', "circumflex"),
    ('Ë', 'E', "diaeresis"),
    ('Ě', 'E', "caron"),
    ('Ì', 'I', "grave"),
    ('Í', 'I', "acute"),
    ('Î', 'I', "circumflex"),
    ('Ï', 'I', "diaeresis"),
    ('Ò', 'O', "grave"),
    ('Ó', 'O', "acute"),
    ('Ô', 'O', "circumflex"),
    ('Õ', 'O', "tilde"),
    ('Ö', 'O', "diaeresis"),
    ('Ø', 'O', "slash"),
    ('Ù', 'U', "grave"),
    ('Ú', 'U', "acute"),
    ('Û', 'U', "circumflex"),
    ('Ü', 'U', "diaeresis"),
    ('Ý', 'Y', "acute"),
    ('Ñ', 'N', "tilde"),
    ('Ç', 'C', "cedilla"),
    ('Š', 'S', "caron"),
    ('Ž', 'Z', "caron"),
    ('Č', 'C', "caron"),
    ('Ř', 'R', "caron"),
    ('Ň', 'N', "caron"),
];

/// The glyph numbers: 0 is the space, then [`GLYPHS`] in order, then
/// [`COMPOSED`].
fn index_of(c: char) -> Option<u16> {
    if c == ' ' || c == '\u{a0}' {
        return Some(0);
    }
    if let Some(i) = GLYPHS.iter().position(|g| g.0 == c) {
        return u16::try_from(i + 1).ok();
    }
    let i = COMPOSED.iter().position(|g| g.0 == c)?;
    u16::try_from(GLYPHS.len() + 1 + i).ok()
}

fn base_width(c: char) -> Option<f64> {
    GLYPHS.iter().find(|g| g.0 == c).map(|g| g.1)
}

/// Reads a path of the stroke language into contours, moved by `shift`.
fn parse_path(path: &str, shift: P2, out: &mut Vec<Contour>) -> Result<(), FontError> {
    let bad = || FontError::Malformed(format!("stroke path `{path}` does not read"));
    let mut tokens = path.split_whitespace().peekable();
    let mut current: Vec<Seg> = Vec::new();
    let mut pen: Option<P2> = None;
    let flush = |current: &mut Vec<Seg>, out: &mut Vec<Contour>| {
        if !current.is_empty() {
            out.push(Contour {
                segs: std::mem::take(current),
                closed: false,
            });
        }
    };
    while let Some(token) = tokens.next() {
        let (command, first) = token.split_at(token.chars().next().map_or(0, char::len_utf8));
        let mut numbers: Vec<f64> = Vec::new();
        if !first.is_empty() {
            numbers.push(first.parse().map_err(|_| bad())?);
        }
        let wanted = match command {
            "M" | "L" => 2,
            "A" => 6,
            "E" => 4,
            _ => return Err(bad()),
        };
        while numbers.len() < wanted {
            let t = tokens.next().ok_or_else(bad)?;
            numbers.push(t.parse().map_err(|_| bad())?);
        }
        let n = |i: usize| numbers.get(i).copied().unwrap_or(0.0);
        let at = |x: f64, y: f64| [x + shift[0], y + shift[1]];
        match command {
            "M" => {
                flush(&mut current, out);
                pen = Some(at(n(0), n(1)));
            }
            "L" => {
                let to = at(n(0), n(1));
                let from = pen.ok_or_else(bad)?;
                current.push(Seg::line(from, to));
                pen = Some(to);
            }
            "A" => {
                let (c, r) = (at(n(0), n(1)), [n(2), n(3)]);
                let (a0, a1) = (n(4).to_radians(), n(5).to_radians());
                let point = |a: f64| [c[0] + r[0] * libm::cos(a), c[1] + r[1] * libm::sin(a)];
                let start = point(a0);
                match pen {
                    Some(p) if dist(p, start) > 1e-9 => current.push(Seg::line(p, start)),
                    Some(_) => {}
                    None => {}
                }
                let sweep = a1 - a0;
                let parts = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0);
                let step = sweep / parts;
                let k = 4.0 / 3.0 * libm::tan(step / 4.0);
                let mut a = a0;
                for _ in 0..(parts as usize).min(16) {
                    let b = a + step;
                    let (p0, p3) = (point(a), point(b));
                    let d0 = [-r[0] * libm::sin(a), r[1] * libm::cos(a)];
                    let d1 = [-r[0] * libm::sin(b), r[1] * libm::cos(b)];
                    let p1 = [p0[0] + k * d0[0], p0[1] + k * d0[1]];
                    let p2 = [p3[0] - k * d1[0], p3[1] - k * d1[1]];
                    current.push(Seg::cubic(p0, p1, p2, p3));
                    a = b;
                }
                pen = current.last().map(Seg::end).or(Some(point(a1)));
            }
            "E" => {
                flush(&mut current, out);
                let (c, r) = (at(n(0), n(1)), [n(2), n(3)]);
                let k = 4.0 / 3.0 * libm::tan(std::f64::consts::FRAC_PI_8);
                let mut segs = Vec::new();
                for q in 0..4 {
                    let a = f64::from(q) * std::f64::consts::FRAC_PI_2;
                    let b = a + std::f64::consts::FRAC_PI_2;
                    let p0 = [c[0] + r[0] * libm::cos(a), c[1] + r[1] * libm::sin(a)];
                    let p3 = [c[0] + r[0] * libm::cos(b), c[1] + r[1] * libm::sin(b)];
                    let d0 = [-r[0] * libm::sin(a), r[1] * libm::cos(a)];
                    let d1 = [-r[0] * libm::sin(b), r[1] * libm::cos(b)];
                    segs.push(Seg::cubic(
                        p0,
                        [p0[0] + k * d0[0], p0[1] + k * d0[1]],
                        [p3[0] - k * d1[0], p3[1] - k * d1[1]],
                        p3,
                    ));
                }
                // Exactly closed.
                if let (Some(first), Some(last)) = (segs.first().map(Seg::start), segs.last_mut()) {
                    last.set_end(first);
                }
                out.push(Contour { segs, closed: true });
                pen = None;
            }
            _ => return Err(bad()),
        }
    }
    flush(&mut current, out);
    Ok(())
}

/// The built-in single-stroke font.
#[derive(Clone, Copy, Debug, Default)]
pub struct StrokeFont;

impl StrokeFont {
    /// The strokes of glyph `glyph` (open contours, and closed ones for
    /// rings: an "o", a "0"), in font units.
    pub fn outline(&self, glyph: u16) -> Result<Outline, FontError> {
        let mut contours = Vec::new();
        let g = usize::from(glyph);
        if g == 0 {
            return Ok(Outline { contours });
        }
        if let Some(&(_, _, path)) = GLYPHS.get(g - 1) {
            parse_path(path, [BEARING, 0.0], &mut contours)?;
            return Ok(Outline { contours });
        }
        let Some(&(_, base, mark)) = COMPOSED.get(g - 1 - GLYPHS.len()) else {
            return Err(FontError::Malformed(format!("no stroke glyph {glyph}")));
        };
        let base_glyph = index_of(base).ok_or(FontError::MissingGlyph(base))?;
        let mut outline = self.outline(base_glyph)?;
        let width = base_width(base).unwrap_or(0.0);
        let upper = base.is_uppercase();
        let path = MARKS
            .iter()
            .find(|m| m.0 == mark)
            .map(|m| m.1)
            .ok_or_else(|| FontError::Malformed(format!("no stroke mark {mark}")))?;
        let y = match mark {
            "cedilla" | "slash" => 0.0,
            _ if upper => CAP + 2.5,
            _ => X_HEIGHT + 3.0,
        };
        let x = BEARING + width / 2.0;
        let mut marks = Vec::new();
        let scale_slash = mark == "slash" && !upper;
        parse_path(path, [x, y], &mut marks)?;
        if scale_slash {
            // Across an "o": two thirds as tall.
            for c in &mut marks {
                *c = c.mapped(&|p| [x + (p[0] - x) * 0.75, p[1] * 0.65]);
            }
        }
        outline.contours.extend(marks);
        Ok(outline)
    }

    /// Every character the font draws.
    pub fn characters() -> Vec<char> {
        let mut out = vec![' '];
        out.extend(GLYPHS.iter().map(|g| g.0));
        out.extend(COMPOSED.iter().map(|g| g.0));
        out
    }
}

impl Metrics for StrokeFont {
    fn units_per_em(&self) -> f64 {
        STROKE_EM
    }

    fn ascender(&self) -> f64 {
        CAP + 4.0
    }

    fn descender(&self) -> f64 {
        -7.0
    }

    fn line_gap(&self) -> f64 {
        4.0
    }

    fn cap_height(&self) -> f64 {
        CAP
    }

    fn glyph(&self, c: char) -> Option<u16> {
        index_of(c)
    }

    fn advance(&self, glyph: u16) -> f64 {
        let g = usize::from(glyph);
        if g == 0 {
            return SPACE;
        }
        let width = match GLYPHS.get(g - 1) {
            Some(&(_, w, _)) => w,
            None => COMPOSED
                .get(g - 1 - GLYPHS.len())
                .and_then(|c| base_width(c.1))
                .unwrap_or(0.0),
        };
        width + 2.0 * BEARING + SPACING - 2.0
    }

    fn kerning(&self, _: u16, _: u16) -> f64 {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_reads_and_stays_in_its_box() {
        let f = StrokeFont;
        for c in StrokeFont::characters() {
            let g = f.glyph(c).unwrap();
            let o = f.outline(g).unwrap();
            if c != ' ' {
                assert!(!o.contours.is_empty(), "{c}");
            }
            for contour in &o.contours {
                let b = contour.bbox();
                assert!(b[1] >= -8.0 && b[3] <= 28.0, "{c}: {b:?}");
                assert!(b[0] >= -1.0 && b[2] <= f.advance(g) + 1.0, "{c}: {b:?}");
                // Strokes join end to end.
                for w in contour.segs.windows(2) {
                    assert!(dist(w[0].end(), w[1].start()) < 1e-9, "{c}");
                }
            }
        }
    }

    #[test]
    fn letters_digits_punctuation_and_accents_are_there() {
        let f = StrokeFont;
        for c in "AZaz09.,;:!?-+=/()[]'\"#%&*_<>@$éèêëàçñöüÅØ€".chars() {
            assert!(f.glyph(c).is_some(), "{c}");
        }
    }
}
