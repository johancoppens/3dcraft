//! Texts of a sketch in regeneration: the template evaluated, the text
//! laid out in its font, and every glyph's outline put in the sketch as
//! curves (docs/design/text.md).
//!
//! A glyph's contour is one B-spline, exactly the font's outline: a
//! TrueType contour of lines and quadratic Béziers is a spline of degree
//! 2, a CFF one of degree 3 (a line raised to the degree, which the
//! profile code makes a line again), joined at knots as often as the
//! degree, so the profile cuts it into its pieces. Overlapping contours
//! are first united by the font's nonzero fill rule ([`craft3d_font`]).
//! The map from font units to the sketch is a scale, a turn and a shift
//! (or, along a curve, one such map per glyph), so the curves stay exact.
//!
//! Each contour is named after the text entity, the glyph's place in the
//! text, its character and the contour: `e7.glyph(3,U+0041).contour(1)`.

use crate::curve_fit::{self, Spline2};
use craft3d_doc::{
    CurvePath, Document, EntityId, Geometry, Sketch, SketchText, TextAlign, TextAnchor, TextPart,
    TextSide, TextSizeOf,
};
use craft3d_expr::{Dimension, Evaluation};
use craft3d_font::{
    Align, Contour, FontFile, LayoutOptions, Metrics, Outline, STROKE, StrokeFont, layout,
};
use std::collections::BTreeMap;

/// Most curves all the texts of one sketch make.
pub const MAX_TEXT_CURVES: usize = 20_000;

/// A curve of a text: a contour of a glyph (or a stroke of the
/// single-stroke font) in sketch coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct TextCurve {
    /// The kernel's number for it ([`crate::sketch_pattern::CurveIds`]).
    pub id: u64,
    pub name: CurvePath,
    pub spline: Spline2,
    /// A closed contour (an outline), or an open stroke.
    pub closed: bool,
}

/// Where one glyph of a text was put: its origin on the baseline and the
/// direction its baseline runs (a unit vector), in sketch coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphPlace {
    /// The character's place in the text (every character counts).
    pub index: usize,
    pub ch: char,
    pub origin: [f64; 2],
    pub direction: [f64; 2],
    /// Its advance, in document units.
    pub advance: f64,
    /// How many contours (or strokes) it has.
    pub contours: usize,
}

/// A text of a sketch as regeneration laid it out.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayout {
    pub id: EntityId,
    /// The template evaluated.
    pub text: String,
    pub construction: bool,
    pub curves: Vec<TextCurve>,
    /// Every glyph laid out (spaces too), in the order of the text.
    pub glyphs: Vec<GlyphPlace>,
    /// The widest line's advance (kerning included), and the height from
    /// the first line's ascender to the last line's descender, in
    /// document units.
    pub width: f64,
    pub height: f64,
    /// The height of the capitals, in document units.
    pub cap_height: f64,
    pub lines: usize,
    /// Where the first line's baseline starts (the first glyph's origin)
    /// and the direction it runs, in sketch coordinates.
    pub baseline: [f64; 2],
    pub direction: [f64; 2],
    /// `[min x, min y, max x, max y]` of the curves' control points, which
    /// hold the curves; `None` for a text of only spaces.
    pub ink: Option<[f64; 4]>,
}

/// A number in a text: whole numbers without a point, others with up to
/// six decimals, trailing zeros dropped.
pub fn format_number(v: f64) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    if (v - v.round()).abs() <= 1e-9 * v.abs().max(1.0) && v.abs() < 1e15 {
        return format!("{}", v.round() as i64);
    }
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// The text of a template with its formulas evaluated: a length in the
/// document's unit, an angle in its angle unit, a plain number as it is.
pub fn evaluate_text(
    doc: &Document,
    variables: &Evaluation,
    id: EntityId,
    text: &SketchText,
) -> Result<String, String> {
    let mut out = String::new();
    for (k, part) in text.text.parts.iter().enumerate() {
        match part {
            TextPart::Literal(t) => out.push_str(t),
            TextPart::Formula(e) => {
                let q = variables
                    .evaluate(e.source())
                    .map_err(|err| format!("{id}.text: `{{{}}}`: {err}", e.source()))?;
                let value = if q.dimension == Dimension::LENGTH {
                    crate::to_document(q, doc).value
                } else if q.dimension == Dimension::ANGLE {
                    match doc.units.angle {
                        craft3d_doc::AngleUnit::Degree => q.value.to_degrees(),
                        craft3d_doc::AngleUnit::Radian => q.value,
                    }
                } else if q.dimension.is_dimensionless() {
                    q.value
                } else {
                    return Err(format!(
                        "{id}.text: formula {k} `{}` is {}; a text writes a number, a length or an angle",
                        e.source(),
                        q.dimension.described()
                    ));
                };
                out.push_str(&format_number(value));
            }
        }
    }
    Ok(out)
}

/// A font a text is set in.
enum Face<'a> {
    File(FontFile<'a>),
    Stroke,
}

impl Face<'_> {
    fn metrics(&self) -> &dyn Metrics {
        match self {
            Face::File(f) => f,
            Face::Stroke => &StrokeFont,
        }
    }

    fn outline(&self, glyph: u16) -> Result<Outline, String> {
        match self {
            Face::File(f) => craft3d_font::filled_outline(f, glyph).map_err(|e| e.to_string()),
            Face::Stroke => StrokeFont.outline(glyph).map_err(|e| e.to_string()),
        }
    }
}

fn face<'a>(doc: &'a Document, name: &str) -> Result<Face<'a>, String> {
    if name == STROKE {
        return Ok(Face::Stroke);
    }
    let Some(font) = doc.fonts.iter().find(|f| f.name == name) else {
        let known: Vec<&str> = doc
            .fonts
            .iter()
            .map(|f| f.name.as_str())
            .chain([STROKE])
            .collect();
        return Err(format!(
            "the document has no font `{name}` (it has {}); add it with font.add",
            known.join(", ")
        ));
    };
    let Some(bytes) = font.bytes() else {
        return Err(format!(
            "the document needs font `{name}` ({}, sha256 {}), which may not be kept in a document ({}): add it again on this machine with font.add",
            font.family, font.sha256, font.embedding
        ));
    };
    FontFile::parse(bytes.bytes())
        .map(Face::File)
        .map_err(|e| format!("font `{name}`: {e}"))
}

/// A curve a text follows, by arc length.
enum Track {
    Line {
        a: [f64; 2],
        b: [f64; 2],
    },
    Arc {
        c: [f64; 2],
        r: f64,
        a0: f64,
        sweep: f64,
    },
    Poly {
        pts: Vec<[f64; 2]>,
        cum: Vec<f64>,
    },
}

impl Track {
    fn length(&self) -> f64 {
        match self {
            Track::Line { a, b } => (b[0] - a[0]).hypot(b[1] - a[1]),
            Track::Arc { r, sweep, .. } => r * sweep.abs(),
            Track::Poly { cum, .. } => cum.last().copied().unwrap_or(0.0),
        }
    }

    fn closed(&self) -> bool {
        match self {
            Track::Arc { sweep, .. } => (sweep.abs() - std::f64::consts::TAU).abs() < 1e-12,
            Track::Poly { pts, .. } => match (pts.first(), pts.last()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
            Track::Line { .. } => false,
        }
    }

    /// The point at arc length `s` and the unit tangent there.
    fn at(&self, s: f64) -> Option<([f64; 2], [f64; 2])> {
        match self {
            Track::Line { a, b } => {
                let l = self.length();
                if !(l > 0.0) {
                    return None;
                }
                let t = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
                Some(([a[0] + t[0] * s, a[1] + t[1] * s], t))
            }
            Track::Arc { c, r, a0, sweep } => {
                let dir = sweep.signum();
                let ang = a0 + dir * s / r;
                Some((
                    [c[0] + r * ang.cos(), c[1] + r * ang.sin()],
                    [-ang.sin() * dir, ang.cos() * dir],
                ))
            }
            Track::Poly { pts, cum } => {
                let k = match cum.binary_search_by(|v| v.total_cmp(&s)) {
                    Ok(k) => k.min(pts.len().saturating_sub(2)),
                    Err(k) => k.saturating_sub(1).min(pts.len().saturating_sub(2)),
                };
                let (a, b) = (*pts.get(k)?, *pts.get(k + 1)?);
                let (s0, s1) = (*cum.get(k)?, *cum.get(k + 1)?);
                let len = s1 - s0;
                if !(len > 0.0) {
                    return None;
                }
                let u = (s - s0) / len;
                let t = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
                Some(([a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u], t))
            }
        }
    }
}

/// What a text's position needs from the solved sketch.
pub(crate) struct Solved<'a> {
    pub point: &'a dyn Fn(EntityId) -> Option<[f64; 2]>,
    pub radius: &'a dyn Fn(EntityId) -> Option<f64>,
    pub fitted: &'a dyn Fn(EntityId) -> Option<Spline2>,
}

fn track(sketch: &Sketch, id: EntityId, solved: &Solved<'_>, tol: f64) -> Result<Track, String> {
    let entity = sketch.entity(id).map_err(|e| e.to_string())?;
    let p = |e: EntityId| (solved.point)(e).ok_or_else(|| format!("{e} has no solved position"));
    let spline = |s: Spline2| -> Result<Track, String> {
        let pts = curve_fit::polyline(&s, tol);
        if pts.len() < 2 {
            return Err(format!("{id} cannot be evaluated"));
        }
        let mut cum = Vec::with_capacity(pts.len());
        let mut total = 0.0;
        let mut prev: Option<[f64; 2]> = None;
        for q in &pts {
            if let Some(a) = prev {
                total += (q[0] - a[0]).hypot(q[1] - a[1]);
            }
            cum.push(total);
            prev = Some(*q);
        }
        Ok(Track::Poly { pts, cum })
    };
    match &entity.geometry {
        &Geometry::Line { start, end } => Ok(Track::Line {
            a: p(start)?,
            b: p(end)?,
        }),
        &Geometry::Circle { center, .. } => Ok(Track::Arc {
            c: p(center)?,
            r: (solved.radius)(id).ok_or_else(|| format!("{id} has no solved radius"))?,
            a0: 0.0,
            sweep: std::f64::consts::TAU,
        }),
        &Geometry::Arc { center, start, end } => {
            let (c, s, e) = (p(center)?, p(start)?, p(end)?);
            let r = (s[0] - c[0]).hypot(s[1] - c[1]);
            let a0 = (s[1] - c[1]).atan2(s[0] - c[0]);
            let a1 = (e[1] - c[1]).atan2(e[0] - c[0]);
            let mut sweep = (a1 - a0).rem_euclid(std::f64::consts::TAU);
            if sweep <= 0.0 {
                sweep = std::f64::consts::TAU;
            }
            Ok(Track::Arc { c, r, a0, sweep })
        }
        Geometry::Spline { degree, control } => {
            let control = control
                .iter()
                .map(|e| p(*e))
                .collect::<Result<Vec<_>, _>>()?;
            spline(Spline2 {
                degree: usize::try_from(*degree).unwrap_or(usize::MAX),
                knots: craft3d_doc::clamped_uniform_knots(*degree, control.len()),
                control,
            })
        }
        Geometry::Formula { .. } | Geometry::Interpolated { .. } => {
            spline((solved.fitted)(id).ok_or_else(|| format!("{id} has no fitted curve"))?)
        }
        other => Err(format!(
            "{id} is a {}; a text follows a line, arc, circle or spline",
            other.type_name()
        )),
    }
}

/// One B-spline for a contour, every point mapped by `map` (which must
/// keep lines and Béziers: an affine map).
fn contour_spline(c: &Contour, map: &dyn Fn([f64; 2]) -> [f64; 2]) -> Result<Spline2, String> {
    let degree = c.degree().clamp(1, 3);
    let mut control: Vec<[f64; 2]> = Vec::new();
    let mut knots = vec![0.0; degree + 1];
    let n = c.segs.len();
    for (k, s) in c.segs.iter().enumerate() {
        let s = s.elevated_to(degree);
        let pts = s.points();
        let skip = usize::from(k > 0);
        control.extend(pts.iter().skip(skip).map(|q| map(*q)));
        let at = (k + 1) as f64;
        if k + 1 < n {
            knots.extend(std::iter::repeat_n(at, degree));
        } else {
            knots.extend(std::iter::repeat_n(at, degree + 1));
        }
    }
    if control.len() > craft3d_kernel::profile::MAX_SPLINE_POINTS {
        return Err(format!(
            "a contour of {} points is more than a curve holds ({})",
            control.len(),
            craft3d_kernel::profile::MAX_SPLINE_POINTS
        ));
    }
    if c.closed
        && let (Some(first), Some(last)) = (control.first().copied(), control.last_mut())
    {
        *last = first;
    }
    Ok(Spline2 {
        degree,
        knots,
        control,
    })
}

fn length_of(
    doc: &Document,
    variables: &Evaluation,
    name: &str,
    e: &craft3d_doc::Expr,
) -> Result<f64, String> {
    crate::length(doc, variables, name, e)
}

/// Lays out text `id` of `sketch`.
pub(crate) fn lay_out(
    doc: &Document,
    variables: &Evaluation,
    sketch: &Sketch,
    id: EntityId,
    text: &SketchText,
    construction: bool,
    solved: &Solved<'_>,
) -> Result<TextLayout, String> {
    let evaluated = evaluate_text(doc, variables, id, text)?;
    let size = length_of(doc, variables, &format!("{id}.size"), &text.size)?;
    if !(size.is_finite() && size > 0.0) {
        return Err(format!(
            "{id}.size: the size of a text is a positive length"
        ));
    }
    let rotation = crate::angle(doc, variables, &format!("{id}.rotation"), &text.rotation)?;
    if !rotation.is_finite() {
        return Err(format!(
            "{id}.rotation: the turn of a text is a finite angle"
        ));
    }
    let spacing = variables
        .evaluate(text.line_spacing.source())
        .map_err(|e| format!("{id}.lineSpacing: `{}`: {e}", text.line_spacing.source()))?;
    if !spacing.dimension.is_dimensionless() || !(spacing.value.is_finite() && spacing.value > 0.0)
    {
        return Err(format!(
            "{id}.lineSpacing: a positive plain number (1 is the font's own line height)"
        ));
    }
    let face = face(doc, &text.font)?;
    let m = face.metrics();
    let cap = m.cap_height();
    let scale = match text.size_of {
        TextSizeOf::Cap => size / cap,
        TextSizeOf::Em => size / m.units_per_em(),
    };
    if !(scale.is_finite() && scale > 0.0) {
        return Err(format!("{id}: the font's cap height or em is not positive"));
    }
    let width = match &text.width {
        Some(w) => {
            let w = length_of(doc, variables, &format!("{id}.width"), w)?;
            if !(w.is_finite() && w > 0.0) {
                return Err(format!("{id}.width: a positive length"));
            }
            Some(w / scale)
        }
        None => None,
    };
    let options = LayoutOptions {
        line_spacing: spacing.value,
        width,
        align: match text.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        },
    };
    let laid = layout(m, &evaluated, &options).map_err(|e| format!("{id}: {e}"))?;
    let last_baseline = laid.lines.last().map_or(0.0, |l| l.1);
    let dy = match text.anchor {
        TextAnchor::Baseline => 0.0,
        TextAnchor::Top => -cap,
        TextAnchor::Bottom => -last_baseline,
        TextAnchor::Middle => -(cap + last_baseline) / 2.0,
    };
    let at = (solved.point)(text.at)
        .filter(|p| p.iter().all(|v| v.is_finite()))
        .ok_or_else(|| format!("{} has no solved position", text.at))?;
    let (sin, cos) = rotation.sin_cos();
    let tol = doc.tolerance.linear;
    // Along a curve: the curve by arc length, where the text starts on it,
    // its distance from it and on which side.
    let path = match &text.path {
        Some(path) => {
            let track = track(sketch, path.curve, solved, tol * 1e-2)?;
            let start = length_of(doc, variables, &format!("{id}.path.start"), &path.start)?;
            let offset = length_of(doc, variables, &format!("{id}.path.offset"), &path.offset)?;
            if !(start.is_finite() && offset.is_finite()) {
                return Err(format!(
                    "{id}.path: the start and the offset along the curve are finite lengths"
                ));
            }
            Some((track, start, offset, path.side, path.reversed))
        }
        None => None,
    };
    let mut curves = Vec::new();
    let mut outlines: BTreeMap<u16, Outline> = BTreeMap::new();
    let mut ink: Option<[f64; 4]> = None;
    let mut first_origin: Option<([f64; 2], [f64; 2])> = None;
    let mut places = Vec::with_capacity(laid.glyphs.len());
    for g in &laid.glyphs {
        if !outlines.contains_key(&g.glyph) {
            let o = face
                .outline(g.glyph)
                .map_err(|e| format!("{id}: glyph {:?}: {e}", g.ch))?;
            outlines.insert(g.glyph, o);
        }
        let Some(outline) = outlines.get(&g.glyph) else {
            continue;
        };
        let advance = m.advance(g.glyph);
        // The map of this glyph from font units (relative to its origin)
        // to the sketch: a scale, a turn and a shift.
        let (origin, xdir, ydir) = match &path {
            None => {
                let ox = g.x * scale;
                let oy = (g.y + dy) * scale;
                let o = [at[0] + cos * ox - sin * oy, at[1] + sin * ox + cos * oy];
                (o, [cos, sin], [-sin, cos])
            }
            Some((track, start, offset, side, reversed)) => {
                let length = track.length();
                let centre = (g.x + advance / 2.0) * scale;
                let (sign_x, up_sign) = match side {
                    TextSide::Left => (1.0, 1.0),
                    TextSide::Right => (-1.0, -1.0),
                };
                let mut s = start + sign_x * centre;
                if *reversed {
                    s = length - s;
                }
                if track.closed() {
                    s = s.rem_euclid(length);
                } else if !(-tol..=length + tol).contains(&s) {
                    return Err(format!(
                        "{id}: the text runs off {} at glyph {} ({:?}): it needs {:.6} along it, and the curve is {:.6} long",
                        text.path.as_ref().map_or(id, |p| p.curve),
                        g.index,
                        g.ch,
                        s,
                        length
                    ));
                }
                let (p, mut t) = track
                    .at(s.clamp(0.0, length))
                    .ok_or_else(|| format!("{id}: the curve it follows has no length"))?;
                if *reversed {
                    t = [-t[0], -t[1]];
                }
                let n = [-t[1], t[0]];
                let xdir = [t[0] * sign_x, t[1] * sign_x];
                let ydir = [n[0] * up_sign, n[1] * up_sign];
                // The glyph's origin: half its advance back from the point
                // on the curve, at its line's height above the curve.
                let up = offset + (g.y + dy) * scale;
                let half = advance / 2.0 * scale;
                let o = [
                    p[0] - xdir[0] * half + ydir[0] * up,
                    p[1] - xdir[1] * half + ydir[1] * up,
                ];
                (o, xdir, ydir)
            }
        };
        if first_origin.is_none() {
            first_origin = Some((origin, xdir));
        }
        places.push(GlyphPlace {
            index: g.index,
            ch: g.ch,
            origin,
            direction: xdir,
            advance: advance * scale,
            contours: outline.contours.len(),
        });
        let map = |q: [f64; 2]| {
            let (x, y) = (q[0] * scale, q[1] * scale);
            [
                origin[0] + xdir[0] * x + ydir[0] * y,
                origin[1] + xdir[1] * x + ydir[1] * y,
            ]
        };
        let glyph_place = u32::try_from(g.index).map_err(|_| format!("{id}: text too long"))?;
        for (k, contour) in outline.contours.iter().enumerate() {
            if curves.len() >= MAX_TEXT_CURVES {
                return Err(format!(
                    "{id}: the text makes more than {MAX_TEXT_CURVES} curves"
                ));
            }
            let spline = contour_spline(contour, &map)
                .map_err(|e| format!("{id}: glyph {:?}: {e}", g.ch))?;
            for q in &spline.control {
                let b = ink.get_or_insert([q[0], q[1], q[0], q[1]]);
                *b = [
                    b[0].min(q[0]),
                    b[1].min(q[1]),
                    b[2].max(q[0]),
                    b[3].max(q[1]),
                ];
            }
            let contour_no = u32::try_from(k).map_err(|_| format!("{id}: too many contours"))?;
            if glyph_place > craft3d_doc::MAX_GLYPH_FIELD
                || contour_no > craft3d_doc::MAX_GLYPH_FIELD
            {
                return Err(format!("{id}: the text is too long to name its glyphs"));
            }
            curves.push(TextCurve {
                id: 0,
                name: CurvePath::glyph(id.0, glyph_place, g.ch, contour_no),
                spline,
                closed: contour.closed,
            });
        }
    }
    let (baseline, direction) = first_origin.unwrap_or((at, [cos, sin]));
    Ok(TextLayout {
        id,
        text: evaluated,
        construction,
        curves,
        glyphs: places,
        width: laid.width() * scale,
        height: laid.height() * scale,
        cap_height: cap * scale,
        lines: laid.lines.len(),
        baseline,
        direction,
        ink,
    })
}

/// Every text of a solved sketch, in the order of their ids, with the
/// first reason one could not be laid out.
pub(crate) fn lay_out_all(
    doc: &Document,
    variables: &Evaluation,
    sketch: &Sketch,
    solved: &Solved<'_>,
) -> (Vec<TextLayout>, Option<String>) {
    let mut out = Vec::new();
    let mut failed = None;
    let mut total = 0usize;
    for entity in &sketch.entities {
        let Geometry::Text(text) = &entity.geometry else {
            continue;
        };
        match lay_out(
            doc,
            variables,
            sketch,
            entity.id,
            text,
            entity.construction,
            solved,
        ) {
            Ok(l) => {
                total = total.saturating_add(l.curves.len());
                if total > MAX_TEXT_CURVES {
                    failed.get_or_insert(format!(
                        "the texts of the sketch make more than {MAX_TEXT_CURVES} curves"
                    ));
                    break;
                }
                out.push(l);
            }
            Err(why) => {
                failed.get_or_insert(why);
            }
        }
    }
    out.sort_by_key(|l| l.id);
    (out, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_written_plainly() {
        assert_eq!(format_number(12.0), "12");
        assert_eq!(format_number(-3.0), "-3");
        assert_eq!(format_number(2.5), "2.5");
        assert_eq!(format_number(1.0 / 3.0), "0.333333");
        assert_eq!(format_number(-0.0000001), "0");
    }
}
