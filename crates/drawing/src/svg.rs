//! The SVG writer: SVG 1.1, one `<path>` per layer.
//!
//! The size is in real units (`width="60mm"`) with a `viewBox` of the same
//! extent in drawing units, so one user unit is one drawing unit and a
//! laser cutter's software gets the part at its true size. SVG's y axis
//! points down: every y is written negated, so the picture is the drawing
//! as it is, not its mirror image, and there is no `transform` for a
//! reader to forget.
//!
//! A layer is one `<path>` with a closed subpath per loop and the even-odd
//! fill rule: a loop inside a loop is a hole whichever way the loops run,
//! in a program that fills, and laser software that only follows strokes
//! sees the same loops either way. Lines are `L`, circular and elliptic
//! arcs are `A` (true arcs: end points, radii and flags), splines are `Q`
//! and `C` Bézier pieces. A path element has no circle of its own: a
//! whole circle is four quarter arcs.

use crate::spline::Bezier;
use crate::{Drawing, DrawingError, Path, Piece, Point, add, norm, scale, shortest, sub};
use std::f64::consts::PI;

/// The stroke width, in millimetres, that laser software takes for a line
/// to cut rather than to engrave ("hairline").
pub const HAIRLINE_MM: f64 = 0.01;

/// The longest arc written as one `A` command: a quarter turn, and a
/// rounding error more so that a quarter stays one. An SVG arc is given
/// by its end points and its radii, from which a reader works out the
/// centre; for a half turn that is as badly conditioned as it gets (an
/// error of 1e-16 in a radius moves the centre by 1e-8), for a quarter
/// turn it is not at all. So longer arcs are written in equal parts.
const MAX_SWEEP: f64 = PI / 2.0 * (1.0 + 1e-12);

/// A whole turn in quarters.
const MAX_PARTS: u32 = 4;

/// How deep a spline piece may be halved to get within the tolerance.
const MAX_DEPTH: u32 = 24;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgOptions {
    /// How far a cubic Bézier may lie from the spline it stands in for,
    /// in drawing units. Only splines of a degree above 3 and rational
    /// ones need it: everything else is written exactly.
    pub tolerance: f64,
    /// The stroke width in millimetres.
    pub stroke_mm: f64,
}

impl Default for SvgOptions {
    fn default() -> Self {
        Self {
            tolerance: 1e-6,
            stroke_mm: HAIRLINE_MM,
        }
    }
}

/// What [`write_svg`] did besides writing.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SvgReport {
    /// `[min x, min y, width, height]` of the `viewBox`.
    pub view_box: [f64; 4],
    /// How many splines were approximated by cubic Béziers (a degree
    /// above 3, or rational).
    pub approximated: usize,
    /// The largest distance measured between such a spline and its
    /// Béziers; 0 when there are none.
    pub deviation: f64,
}

struct Out {
    d: String,
    tolerance: f64,
    report: SvgReport,
}

impl Out {
    fn number(&mut self, x: f64) -> Result<(), DrawingError> {
        let text = shortest(x).ok_or(DrawingError::NonFinite { layer: 0, path: 0 })?;
        self.d.push_str(&text);
        Ok(())
    }

    /// `x,y` with y negated.
    fn point(&mut self, p: Point) -> Result<(), DrawingError> {
        self.number(p[0])?;
        self.d.push(',');
        self.number(-p[1])
    }

    fn command(&mut self, letter: char) {
        if !self.d.is_empty() {
            self.d.push(' ');
        }
        self.d.push(letter);
    }

    fn to(&mut self, letter: char, points: &[Point]) -> Result<(), DrawingError> {
        self.command(letter);
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                self.d.push(' ');
            }
            self.point(*p)?;
        }
        Ok(())
    }

    /// An arc of the ellipse with semi-axes `rx` and `ry`, the first at
    /// `rotation` radians counter-clockwise in the drawing, to `to`.
    fn arc(
        &mut self,
        rx: f64,
        ry: f64,
        rotation: f64,
        large: bool,
        ccw: bool,
        to: Point,
    ) -> Result<(), DrawingError> {
        self.command('A');
        self.number(rx)?;
        self.d.push(',');
        self.number(ry)?;
        self.d.push(' ');
        // With y negated the axis turns the other way.
        let degrees = (-rotation).to_degrees();
        self.number(if degrees == 0.0 { 0.0 } else { degrees })?;
        self.d.push(' ');
        self.d.push(if large { '1' } else { '0' });
        self.d.push(' ');
        // SVG's positive direction is clockwise on the screen; ours,
        // with y up, is counter-clockwise.
        self.d.push(if ccw { '0' } else { '1' });
        self.d.push(' ');
        self.point(to)
    }

    /// An arc over `sweep` (signed) from parameter `start`, where
    /// `at(t)` is the point at parameter `t` and `to` the exact end: in
    /// parts of a quarter turn at most.
    #[allow(clippy::too_many_arguments)]
    fn swept(
        &mut self,
        rx: f64,
        ry: f64,
        rotation: f64,
        start: f64,
        sweep: f64,
        to: Point,
        at: impl Fn(f64) -> Point,
    ) -> Result<(), DrawingError> {
        let ccw = sweep >= 0.0;
        let parts = (1..=MAX_PARTS)
            .find(|n| sweep.abs() <= f64::from(*n) * MAX_SWEEP)
            .unwrap_or(MAX_PARTS);
        for part in 1..parts {
            let t = start + sweep * f64::from(part) / f64::from(parts);
            self.arc(rx, ry, rotation, false, ccw, at(t))?;
        }
        self.arc(rx, ry, rotation, false, ccw, to)
    }

    fn bezier(&mut self, piece: &Bezier) -> Result<bool, DrawingError> {
        let rest = piece.control.get(1..).unwrap_or(&[]);
        match (piece.degree(), &piece.weights) {
            (1, None) => self.to('L', rest).map(|()| false),
            (2, None) => self.to('Q', rest).map(|()| false),
            (3, None) => self.to('C', rest).map(|()| false),
            _ => {
                self.cubics(piece, 0.0, 1.0, 0)?;
                Ok(true)
            }
        }
    }

    /// Cubic Béziers for the piece between `t0` and `t1`: the cubic with
    /// the piece's ends and end tangents, halved until it lies within the
    /// tolerance at nine places along it.
    fn cubics(&mut self, piece: &Bezier, t0: f64, t1: f64, depth: u32) -> Result<(), DrawingError> {
        let (p0, d0) = piece.derivatives(t0);
        let (p3, d1) = piece.derivatives(t1);
        let third = (t1 - t0) / 3.0;
        let cubic = Bezier {
            control: vec![p0, add(p0, scale(d0, third)), sub(p3, scale(d1, third)), p3],
            weights: None,
        };
        let mut worst = 0.0_f64;
        for i in 1..10 {
            let s = f64::from(i) / 10.0;
            let off = norm(sub(piece.eval(t0 + (t1 - t0) * s), cubic.eval(s)));
            if !off.is_finite() {
                return Err(DrawingError::NonFinite { layer: 0, path: 0 });
            }
            worst = worst.max(off);
        }
        if worst <= self.tolerance {
            self.report.deviation = self.report.deviation.max(worst);
            return self.to('C', cubic.control.get(1..).unwrap_or(&[]));
        }
        if depth >= MAX_DEPTH {
            return Err(DrawingError::NotConverged { layer: 0, path: 0 });
        }
        let mid = (t0 + t1) / 2.0;
        self.cubics(piece, t0, mid, depth + 1)?;
        self.cubics(piece, mid, t1, depth + 1)
    }

    fn path(&mut self, path: &Path) -> Result<(), DrawingError> {
        let pieces = path.pieces();
        let Some(first) = pieces.first() else {
            return Ok(());
        };
        let start = match first {
            Piece::Line { from, .. } | Piece::Arc { from, .. } | Piece::Ellipse { from, .. } => {
                *from
            }
            Piece::Spline { spline } => spline.start(),
        };
        self.to('M', &[start])?;
        for piece in &pieces {
            match piece {
                Piece::Line { to, .. } => self.to('L', &[*to])?,
                Piece::Arc {
                    to,
                    center,
                    radius,
                    start,
                    sweep,
                    ..
                } => {
                    let (c, r) = (*center, *radius);
                    self.swept(r, r, 0.0, *start, *sweep, *to, |t| {
                        [c[0] + r * t.cos(), c[1] + r * t.sin()]
                    })?;
                }
                Piece::Ellipse {
                    to,
                    center,
                    major,
                    ratio,
                    start,
                    sweep,
                    ..
                } => {
                    let rx = norm(*major);
                    let rotation = major[1].atan2(major[0]);
                    let (c, m, k) = (*center, *major, *ratio);
                    self.swept(rx, rx * k, rotation, *start, *sweep, *to, |t| {
                        crate::ellipse_point(c, m, k, t)
                    })?;
                }
                Piece::Spline { spline } => {
                    let mut approximated = false;
                    for bezier in spline.beziers()? {
                        approximated |= self.bezier(&bezier)?;
                    }
                    if approximated {
                        self.report.approximated += 1;
                    }
                }
            }
        }
        if path.is_closed() {
            self.command('Z');
        }
        Ok(())
    }
}

/// Writes the drawing as an SVG 1.1 document, and says what was
/// approximated (see [`SvgOptions::tolerance`]).
pub fn write_svg(
    drawing: &Drawing,
    options: &SvgOptions,
) -> Result<(String, SvgReport), DrawingError> {
    drawing.validate()?;
    if !(options.tolerance.is_finite() && options.tolerance > 0.0) {
        return Err(DrawingError::Option(
            "the tolerance must be positive and finite",
        ));
    }
    if !(options.stroke_mm.is_finite() && options.stroke_mm > 0.0) {
        return Err(DrawingError::Option(
            "the stroke width must be positive and finite",
        ));
    }
    let stroke = options.stroke_mm / drawing.unit.millimetres();
    let bbox = drawing.bbox().ok_or(DrawingError::Empty)?;
    // A drawing without width or height (one straight line) still needs
    // a box with both.
    let (mut x, mut y) = (bbox[0], -bbox[3]);
    let (mut width, mut height) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
    if width <= 0.0 {
        x -= stroke;
        width = 2.0 * stroke;
    }
    if height <= 0.0 {
        y -= stroke;
        height = 2.0 * stroke;
    }
    let mut report = SvgReport {
        view_box: [x, y, width, height],
        ..SvgReport::default()
    };
    let text = |v: f64| shortest(v).ok_or(DrawingError::NonFinite { layer: 0, path: 0 });
    let (factor, suffix) = drawing.unit.svg_length();
    let mut svg = String::new();
    svg.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" width=\"{}{suffix}\" height=\"{}{suffix}\" viewBox=\"{} {} {} {}\">\n",
        text(width * factor)?,
        text(height * factor)?,
        text(x)?,
        text(y)?,
        text(width)?,
        text(height)?,
    ));
    for (index, layer) in drawing.layers.iter().enumerate() {
        if layer.paths.is_empty() {
            continue;
        }
        let mut out = Out {
            d: String::new(),
            tolerance: options.tolerance,
            report,
        };
        for (path_index, path) in layer.paths.iter().enumerate() {
            out.path(path).map_err(|e| match e {
                DrawingError::NonFinite { .. } => DrawingError::NonFinite {
                    layer: index,
                    path: path_index,
                },
                DrawingError::NotConverged { .. } => DrawingError::NotConverged {
                    layer: index,
                    path: path_index,
                },
                other => other,
            })?;
        }
        report = out.report;
        // A dashed layer: dashes of its length, gaps half as long.
        let dashes = match layer.dash {
            Some(dash) => format!(
                " stroke-dasharray=\"{} {}\"",
                text(dash)?,
                text(dash * 0.5)?
            ),
            None => String::new(),
        };
        svg.push_str(&format!(
            "<path id=\"layer-{}\" d=\"{}\" fill=\"none\" fill-rule=\"evenodd\" stroke=\"{}\" stroke-width=\"{}\"{dashes}/>\n",
            layer.name,
            out.d,
            layer.color.svg(),
            text(stroke)?,
        ));
    }
    svg.push_str("</svg>\n");
    Ok((svg, report))
}
