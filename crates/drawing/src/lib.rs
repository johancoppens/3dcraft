//! DXF and SVG writers for flat outlines, with their own 2D model.
//!
//! Standalone: depends on no workspace crate (docs/architecture.md §3). A
//! [`Drawing`] is a length unit and layers of paths; a path is a chain of
//! lines, circular arcs, elliptic arcs and splines, or a whole circle or
//! ellipse. Both writers keep every curve the curve it is: nothing is
//! turned into a polyline. The mapping from sketches and faces lives in
//! `craft3d-io`.
//!
//! Sources: the DXF Reference (Autodesk, published for every release; the
//! group codes of `HEADER`, `TABLES`, `BLOCKS`, `LINE`, `ARC`, `CIRCLE`,
//! `ELLIPSE` and `SPLINE`) for [`write_dxf`]; the W3C recommendation
//! "Scalable Vector Graphics (SVG) 1.1 (Second Edition)", chapters 7
//! (coordinate systems, `viewBox`) and 8 (paths, with the arc
//! implementation notes of appendix F.6) for [`write_svg`]; *The NURBS
//! Book* (Piegl and Tiller), §5.2, for knot insertion.
//!
//! Coordinates are in the drawing's unit, x to the right and y up, as in
//! a sketch. Numbers are written as the shortest decimal text that reads
//! back as the same `f64`, so the same drawing gives the same bytes and a
//! reader gets our numbers bit for bit.

mod dxf;
mod spline;
mod svg;

#[cfg(test)]
mod tests;

pub use dxf::{DXF_VERSION, write_dxf};
pub use spline::{Bezier, MAX_SPLINE_CONTROL_POINTS, MAX_SPLINE_DEGREE, Spline};
pub use svg::{HAIRLINE_MM, SvgOptions, SvgReport, write_svg};

use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::fmt;

/// A point, or a vector, in the drawing's plane.
pub type Point = [f64; 2];

/// Length unit of the coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Unit {
    Micron,
    #[default]
    Millimeter,
    Centimeter,
    Meter,
    Inch,
    Foot,
}

impl Unit {
    /// The value of the DXF header variable `$INSUNITS`.
    pub fn insunits(self) -> u8 {
        match self {
            Unit::Inch => 1,
            Unit::Foot => 2,
            Unit::Millimeter => 4,
            Unit::Centimeter => 5,
            Unit::Meter => 6,
            Unit::Micron => 13,
        }
    }

    /// True for millimetre-based units (the DXF `$MEASUREMENT` flag).
    pub fn is_metric(self) -> bool {
        !matches!(self, Unit::Inch | Unit::Foot)
    }

    /// How an SVG length is written for this unit: the factor to an SVG
    /// unit and that unit's suffix. SVG has `mm`, `cm` and `in`; microns
    /// and metres are written in `mm`, feet in `in`.
    pub fn svg_length(self) -> (f64, &'static str) {
        match self {
            Unit::Micron => (0.001, "mm"),
            Unit::Millimeter => (1.0, "mm"),
            Unit::Centimeter => (1.0, "cm"),
            Unit::Meter => (1000.0, "mm"),
            Unit::Inch => (1.0, "in"),
            Unit::Foot => (12.0, "in"),
        }
    }

    /// Millimetres per unit.
    pub fn millimetres(self) -> f64 {
        match self {
            Unit::Micron => 0.001,
            Unit::Millimeter => 1.0,
            Unit::Centimeter => 10.0,
            Unit::Meter => 1000.0,
            Unit::Inch => 25.4,
            Unit::Foot => 304.8,
        }
    }
}

/// The colour of a layer: the seven colours every DXF reader knows by
/// number (AutoCAD colour index 1 to 7) and every SVG reader by value.
/// Laser software tells cutting from engraving by colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Color {
    /// AutoCAD colour 7, drawn black on a light background.
    #[default]
    Black,
    Red,
    Yellow,
    Green,
    Cyan,
    Blue,
    Magenta,
}

impl Color {
    /// The AutoCAD colour index.
    pub fn aci(self) -> u8 {
        match self {
            Color::Red => 1,
            Color::Yellow => 2,
            Color::Green => 3,
            Color::Cyan => 4,
            Color::Blue => 5,
            Color::Magenta => 6,
            Color::Black => 7,
        }
    }

    /// The colour as an SVG `stroke` value.
    pub fn svg(self) -> &'static str {
        match self {
            Color::Black => "#000000",
            Color::Red => "#ff0000",
            Color::Yellow => "#ffff00",
            Color::Green => "#00ff00",
            Color::Cyan => "#00ffff",
            Color::Blue => "#0000ff",
            Color::Magenta => "#ff00ff",
        }
    }
}

/// One piece of a chain. It starts where the piece before it ended (or at
/// the chain's start) and ends at `to`.
#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Line {
        to: Point,
    },
    /// Along the circle of `radius` about `center`, counter-clockwise when
    /// `ccw`, otherwise clockwise. Its two ends must differ: a whole
    /// circle is [`Path::Circle`].
    Arc {
        to: Point,
        center: Point,
        radius: f64,
        ccw: bool,
    },
    /// Along the ellipse about `center` whose longer semi-axis is the
    /// vector `major` and whose shorter one is `ratio` times as long, a
    /// quarter turn counter-clockwise from it.
    EllipticArc {
        to: Point,
        center: Point,
        major: Point,
        ratio: f64,
        ccw: bool,
    },
    /// A B-spline from its first control point to its last. The first is
    /// where the piece before ended.
    Spline(Spline),
}

impl Segment {
    /// Where the segment ends.
    pub fn end(&self) -> Point {
        match self {
            Segment::Line { to } | Segment::Arc { to, .. } | Segment::EllipticArc { to, .. } => *to,
            Segment::Spline(spline) => spline.end(),
        }
    }
}

/// A connected outline.
#[derive(Clone, Debug, PartialEq)]
pub enum Path {
    /// Segments head to tail from `start`. When `closed`, the last one
    /// ends at `start`.
    Chain {
        start: Point,
        segments: Vec<Segment>,
        closed: bool,
    },
    Circle {
        center: Point,
        radius: f64,
    },
    /// A whole ellipse: see [`Segment::EllipticArc`] for `major` and
    /// `ratio`.
    Ellipse {
        center: Point,
        major: Point,
        ratio: f64,
    },
}

impl Path {
    /// True for a circle, an ellipse and a closed chain.
    pub fn is_closed(&self) -> bool {
        match self {
            Path::Chain { closed, .. } => *closed,
            Path::Circle { .. } | Path::Ellipse { .. } => true,
        }
    }
}

/// Paths drawn alike. In DXF a layer of that name; in SVG one `<path>`
/// element with a subpath per path, so that loops inside loops are holes
/// under the even-odd rule.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Layer {
    /// Letters, digits, `_` and `-` only, at most 31 characters: a name
    /// every DXF release takes.
    pub name: String,
    pub color: Color,
    pub paths: Vec<Path>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Drawing {
    pub unit: Unit,
    pub layers: Vec<Layer>,
}

/// Why a drawing cannot be written. `layer` and `path` count from 0.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawingError {
    /// A coordinate, radius, knot or weight is NaN or infinite.
    NonFinite { layer: usize, path: usize },
    /// An arc, circle or ellipse that is none: see the reason.
    Degenerate {
        layer: usize,
        path: usize,
        reason: &'static str,
    },
    /// A spline that does not hold together.
    InvalidSpline(&'static str),
    /// A layer name with other characters than letters, digits, `_` and
    /// `-`, an empty or too long one, or one used twice.
    LayerName(String),
    /// There is no path at all.
    Empty,
    /// A curve could not be brought within the tolerance asked for.
    NotConverged { layer: usize, path: usize },
    /// More than the writers will take.
    TooLarge(&'static str),
    /// An option of a writer that it cannot work with.
    Option(&'static str),
}

impl fmt::Display for DrawingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrawingError::NonFinite { layer, path } => write!(
                f,
                "layer {layer}, path {path}: a number is not finite (NaN or infinity)"
            ),
            DrawingError::Degenerate {
                layer,
                path,
                reason,
            } => write!(f, "layer {layer}, path {path}: {reason}"),
            DrawingError::InvalidSpline(reason) => write!(f, "spline: {reason}"),
            DrawingError::LayerName(name) => write!(
                f,
                "`{name}` cannot be a layer name: use 1 to 31 letters, digits, `_` or `-`, each name once"
            ),
            DrawingError::Empty => write!(f, "there is nothing to draw"),
            DrawingError::NotConverged { layer, path } => write!(
                f,
                "layer {layer}, path {path}: a spline could not be approximated within the tolerance"
            ),
            DrawingError::TooLarge(what) => write!(f, "too large: {what}"),
            DrawingError::Option(what) => write!(f, "{what}"),
        }
    }
}

impl std::error::Error for DrawingError {}

/// How many of each there are in a drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Counts {
    pub paths: usize,
    /// Circles, ellipses and closed chains.
    pub closed: usize,
    pub lines: usize,
    pub arcs: usize,
    pub circles: usize,
    /// Whole ellipses and elliptic arcs.
    pub ellipses: usize,
    pub splines: usize,
}

// ------------------------------------------------------------ plane maths

pub(crate) fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

pub(crate) fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}

pub(crate) fn scale(a: Point, k: f64) -> Point {
    [a[0] * k, a[1] * k]
}

pub(crate) fn norm(a: Point) -> f64 {
    a[0].hypot(a[1])
}

/// A quarter turn counter-clockwise.
pub(crate) fn perp(a: Point) -> Point {
    [-a[1], a[0]]
}

fn finite(p: Point) -> bool {
    p[0].is_finite() && p[1].is_finite()
}

/// `a` wrapped into `(0, 2π]`: the angle run from one direction to
/// another that is not the same one.
fn turn(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w > 0.0 && w <= TAU { w } else { TAU }
}

/// A piece of a path with its ends and angles worked out: what both
/// writers and the measurements go by.
#[derive(Clone, Debug)]
pub(crate) enum Piece<'a> {
    Line {
        from: Point,
        to: Point,
    },
    /// From the angle `start` over `sweep` radians: positive is
    /// counter-clockwise, and a whole turn is `±2π` exactly.
    Arc {
        from: Point,
        to: Point,
        center: Point,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    /// As `Arc`, with `start` and `sweep` in the ellipse's own parameter
    /// (`center + major cos t + minor sin t`).
    Ellipse {
        from: Point,
        to: Point,
        center: Point,
        major: Point,
        ratio: f64,
        start: f64,
        sweep: f64,
    },
    Spline {
        spline: &'a Spline,
    },
}

/// The shorter semi-axis as a vector.
pub(crate) fn minor_axis(major: Point, ratio: f64) -> Point {
    scale(perp(major), ratio)
}

/// The point of an ellipse at parameter `t`.
pub(crate) fn ellipse_point(center: Point, major: Point, ratio: f64, t: f64) -> Point {
    let minor = minor_axis(major, ratio);
    add(center, add(scale(major, t.cos()), scale(minor, t.sin())))
}

/// The parameter of the point of an ellipse nearest in angle to `p`.
fn ellipse_param(p: Point, center: Point, major: Point, ratio: f64) -> f64 {
    let d = sub(p, center);
    let a2 = major[0] * major[0] + major[1] * major[1];
    let minor = minor_axis(major, ratio);
    let b2 = minor[0] * minor[0] + minor[1] * minor[1];
    // Components along the axes, each in units of its own semi-axis.
    let x = (d[0] * major[0] + d[1] * major[1]) / a2;
    let y = (d[0] * minor[0] + d[1] * minor[1]) / b2;
    y.atan2(x)
}

fn signed(sweep: f64, ccw: bool) -> f64 {
    if ccw { sweep } else { -sweep }
}

impl Path {
    /// The pieces of the path, in order.
    pub(crate) fn pieces(&self) -> Vec<Piece<'_>> {
        match self {
            Path::Circle { center, radius } => {
                let from = [center[0] + radius, center[1]];
                vec![Piece::Arc {
                    from,
                    to: from,
                    center: *center,
                    radius: *radius,
                    start: 0.0,
                    sweep: TAU,
                }]
            }
            Path::Ellipse {
                center,
                major,
                ratio,
            } => {
                let from = add(*center, *major);
                vec![Piece::Ellipse {
                    from,
                    to: from,
                    center: *center,
                    major: *major,
                    ratio: *ratio,
                    start: 0.0,
                    sweep: TAU,
                }]
            }
            Path::Chain {
                start, segments, ..
            } => {
                let mut from = *start;
                let mut out = Vec::with_capacity(segments.len());
                for segment in segments {
                    out.push(match segment {
                        Segment::Line { to } => Piece::Line { from, to: *to },
                        Segment::Arc {
                            to,
                            center,
                            radius,
                            ccw,
                        } => {
                            let a = sub(from, *center);
                            let b = sub(*to, *center);
                            let (a0, a1) = (a[1].atan2(a[0]), b[1].atan2(b[0]));
                            let sweep = turn(if *ccw { a1 - a0 } else { a0 - a1 });
                            Piece::Arc {
                                from,
                                to: *to,
                                center: *center,
                                radius: *radius,
                                start: a0,
                                sweep: signed(sweep, *ccw),
                            }
                        }
                        Segment::EllipticArc {
                            to,
                            center,
                            major,
                            ratio,
                            ccw,
                        } => {
                            let t0 = ellipse_param(from, *center, *major, *ratio);
                            let t1 = ellipse_param(*to, *center, *major, *ratio);
                            let sweep = turn(if *ccw { t1 - t0 } else { t0 - t1 });
                            Piece::Ellipse {
                                from,
                                to: *to,
                                center: *center,
                                major: *major,
                                ratio: *ratio,
                                start: t0,
                                sweep: signed(sweep, *ccw),
                            }
                        }
                        Segment::Spline(spline) => Piece::Spline { spline },
                    });
                    from = segment.end();
                }
                out
            }
        }
    }

    fn check(&self, layer: usize, path: usize) -> Result<(), DrawingError> {
        let non_finite = DrawingError::NonFinite { layer, path };
        let degenerate = |reason| DrawingError::Degenerate {
            layer,
            path,
            reason,
        };
        let ellipse = |major: Point, ratio: f64| {
            if !(finite(major) && ratio.is_finite()) {
                Err(non_finite.clone())
            } else if norm(major) <= 0.0 {
                Err(degenerate(
                    "an ellipse needs a longer semi-axis of some length",
                ))
            } else if !(ratio > 0.0 && ratio <= 1.0) {
                Err(degenerate(
                    "an ellipse's shorter semi-axis must be more than 0 and at most 1 times the longer",
                ))
            } else {
                Ok(())
            }
        };
        let circle = |radius: f64| {
            if !radius.is_finite() {
                Err(non_finite.clone())
            } else if radius > 0.0 {
                Ok(())
            } else {
                Err(degenerate("a radius must be positive"))
            }
        };
        match self {
            Path::Circle { center, radius } => {
                if !finite(*center) {
                    return Err(non_finite.clone());
                }
                circle(*radius)
            }
            Path::Ellipse {
                center,
                major,
                ratio,
            } => {
                if !finite(*center) {
                    return Err(non_finite.clone());
                }
                ellipse(*major, *ratio)
            }
            Path::Chain {
                start, segments, ..
            } => {
                if !finite(*start) {
                    return Err(non_finite.clone());
                }
                if segments.is_empty() {
                    return Err(degenerate("a chain needs at least one segment"));
                }
                let mut from = *start;
                for segment in segments {
                    let to = segment.end();
                    if !finite(to) {
                        return Err(non_finite.clone());
                    }
                    match segment {
                        Segment::Line { .. } => {}
                        Segment::Arc { center, radius, .. } => {
                            if !finite(*center) {
                                return Err(non_finite.clone());
                            }
                            circle(*radius)?;
                            if from == to {
                                return Err(degenerate(
                                    "an arc that ends where it starts: a whole circle is a path of its own",
                                ));
                            }
                            let off = |p: Point| (norm(sub(p, *center)) - radius).abs();
                            // Loose: it catches a wrong centre, not rounding.
                            if !(off(from) <= OFF_CURVE * radius && off(to) <= OFF_CURVE * radius) {
                                return Err(degenerate(
                                    "an arc whose ends are not at its radius from its centre",
                                ));
                            }
                        }
                        Segment::EllipticArc {
                            center,
                            major,
                            ratio,
                            ..
                        } => {
                            if !finite(*center) {
                                return Err(non_finite.clone());
                            }
                            ellipse(*major, *ratio)?;
                            if from == to {
                                return Err(degenerate(
                                    "an elliptic arc that ends where it starts: a whole ellipse is a path of its own",
                                ));
                            }
                            let off = |p: Point| {
                                let t = ellipse_param(p, *center, *major, *ratio);
                                norm(sub(p, ellipse_point(*center, *major, *ratio, t)))
                            };
                            let size = OFF_CURVE * norm(*major);
                            if !(off(from) <= size && off(to) <= size) {
                                return Err(degenerate(
                                    "an elliptic arc whose ends are not on its ellipse",
                                ));
                            }
                        }
                        // A spline is checked when it is made.
                        Segment::Spline(_) => {}
                    }
                    from = to;
                }
                Ok(())
            }
        }
    }
}

/// How far, as a part of its radius, an end of an arc may lie from the
/// arc's circle or ellipse.
const OFF_CURVE: f64 = 1e-6;

/// The largest number of paths one drawing may hold.
pub const MAX_PATHS: usize = 1_000_000;

impl Drawing {
    /// Checks everything the writers rely on: finite numbers, positive
    /// radii, layer names, at least one path.
    pub fn validate(&self) -> Result<(), DrawingError> {
        let mut total = 0_usize;
        for (index, layer) in self.layers.iter().enumerate() {
            let name = &layer.name;
            let plain = name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            let twice = self
                .layers
                .iter()
                .take(index)
                .any(|other| other.name.eq_ignore_ascii_case(name));
            if name.is_empty() || name.len() > 31 || !plain || twice {
                return Err(DrawingError::LayerName(name.chars().take(40).collect()));
            }
            for (path_index, path) in layer.paths.iter().enumerate() {
                path.check(index, path_index)?;
            }
            total = total.saturating_add(layer.paths.len());
        }
        if total == 0 {
            return Err(DrawingError::Empty);
        }
        if total > MAX_PATHS {
            return Err(DrawingError::TooLarge("more than a million paths"));
        }
        Ok(())
    }

    fn paths(&self) -> impl Iterator<Item = &Path> {
        self.layers.iter().flat_map(|layer| &layer.paths)
    }

    pub fn counts(&self) -> Counts {
        let mut counts = Counts::default();
        for path in self.paths() {
            counts.paths += 1;
            if path.is_closed() {
                counts.closed += 1;
            }
            match path {
                Path::Circle { .. } => counts.circles += 1,
                Path::Ellipse { .. } => counts.ellipses += 1,
                Path::Chain { segments, .. } => {
                    for segment in segments {
                        match segment {
                            Segment::Line { .. } => counts.lines += 1,
                            Segment::Arc { .. } => counts.arcs += 1,
                            Segment::EllipticArc { .. } => counts.ellipses += 1,
                            Segment::Spline(_) => counts.splines += 1,
                        }
                    }
                }
            }
        }
        counts
    }

    /// `[min x, min y, max x, max y]` of everything drawn; `None` for an
    /// empty drawing. Exact for lines, arcs and ellipses; a spline counts
    /// by the control points of its Bézier pieces, which hold it but may
    /// reach further than the curve does.
    pub fn bbox(&self) -> Option<[f64; 4]> {
        let mut bbox: Option<[f64; 4]> = None;
        let mut grow = |p: Point| {
            let b = bbox.get_or_insert([p[0], p[1], p[0], p[1]]);
            // Adding 0 turns a negative zero into zero.
            *b = [
                b[0].min(p[0]) + 0.0,
                b[1].min(p[1]) + 0.0,
                b[2].max(p[0]) + 0.0,
                b[3].max(p[1]) + 0.0,
            ];
        };
        for path in self.paths() {
            for piece in path.pieces() {
                match piece {
                    Piece::Line { from, to } => {
                        grow(from);
                        grow(to);
                    }
                    Piece::Arc {
                        from,
                        to,
                        center,
                        radius,
                        start,
                        sweep,
                    } => {
                        grow(from);
                        grow(to);
                        for quarter in 0..4 {
                            let angle = f64::from(quarter) * FRAC_PI_2;
                            if within(angle, start, sweep) {
                                let (x, y) = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)]
                                    .get(quarter as usize)
                                    .copied()
                                    .unwrap_or((1.0, 0.0));
                                grow([center[0] + radius * x, center[1] + radius * y]);
                            }
                        }
                    }
                    Piece::Ellipse {
                        from,
                        to,
                        center,
                        major,
                        ratio,
                        start,
                        sweep,
                    } => {
                        grow(from);
                        grow(to);
                        let minor = minor_axis(major, ratio);
                        // Where x, and where y, stops changing along
                        // `major cos t + minor sin t`.
                        for axis in 0..2 {
                            let (Some(&m), Some(&n)) = (major.get(axis), minor.get(axis)) else {
                                continue;
                            };
                            let t = n.atan2(m);
                            for t in [t, t + PI] {
                                if within(t, start, sweep) {
                                    grow(ellipse_point(center, major, ratio, t));
                                }
                            }
                        }
                    }
                    Piece::Spline { spline } => {
                        // A spline that cannot be split is refused by the
                        // writers; here its own control points stand in.
                        match spline.beziers() {
                            Ok(pieces) => pieces
                                .iter()
                                .flat_map(|piece| piece.control.iter())
                                .for_each(|p| grow(*p)),
                            Err(_) => spline.control_points().iter().for_each(|p| grow(*p)),
                        }
                    }
                }
            }
        }
        bbox
    }

    /// The length of everything drawn. Lines, arcs and circles are exact;
    /// ellipses and splines are integrated numerically: ellipses to
    /// rounding, splines to about twelve digits when they have no sharp
    /// bends.
    pub fn length(&self) -> f64 {
        let mut total = 0.0;
        for path in self.paths() {
            for piece in path.pieces() {
                total += match piece {
                    Piece::Line { from, to } => norm(sub(to, from)),
                    Piece::Arc { radius, sweep, .. } => radius * sweep.abs(),
                    Piece::Ellipse {
                        major,
                        ratio,
                        start,
                        sweep,
                        ..
                    } => {
                        let minor = minor_axis(major, ratio);
                        integrate(start, start + sweep.abs(), 512, |t| {
                            norm(add(scale(major, -t.sin()), scale(minor, t.cos())))
                        })
                    }
                    Piece::Spline { spline } => spline.beziers().map_or(0.0, |pieces| {
                        pieces
                            .iter()
                            .map(|piece| integrate(0.0, 1.0, 256, |t| norm(piece.derivative(t))))
                            .sum()
                    }),
                };
            }
        }
        total
    }
}

/// True when the direction at `angle` lies on the arc from `start` over
/// `sweep` (signed), ends included.
fn within(angle: f64, start: f64, sweep: f64) -> bool {
    if sweep.abs() >= TAU {
        return true;
    }
    let run = if sweep >= 0.0 {
        angle - start
    } else {
        start - angle
    };
    let w = run.rem_euclid(TAU);
    // The slack lets an arc that ends on an axis have its end there.
    w <= sweep.abs() + 1e-12 || TAU - w <= 1e-12
}

/// The integral of `f` over `[a, b]` by the five-point Gauss-Legendre rule
/// on each of `parts` equal pieces (Abramowitz and Stegun, table 25.4).
fn integrate(a: f64, b: f64, parts: u32, f: impl Fn(f64) -> f64) -> f64 {
    let inner = (5.0 - 2.0 * (10.0_f64 / 7.0).sqrt()).sqrt() / 3.0;
    let outer = (5.0 + 2.0 * (10.0_f64 / 7.0).sqrt()).sqrt() / 3.0;
    let w_inner = (322.0 + 13.0 * 70.0_f64.sqrt()) / 900.0;
    let w_outer = (322.0 - 13.0 * 70.0_f64.sqrt()) / 900.0;
    let rule = [
        (0.0, 128.0 / 225.0),
        (inner, w_inner),
        (-inner, w_inner),
        (outer, w_outer),
        (-outer, w_outer),
    ];
    let parts = parts.max(1);
    let h = (b - a) / f64::from(parts);
    let mut total = 0.0;
    for k in 0..parts {
        let mid = a + (f64::from(k) + 0.5) * h;
        for (x, w) in rule {
            total += w * f(mid + x * h / 2.0);
        }
    }
    total * h / 2.0
}

/// The shortest decimal text that reads back as `x`, without an exponent
/// for numbers of an everyday size. `None` for NaN and infinity.
pub(crate) fn shortest(x: f64) -> Option<String> {
    if !x.is_finite() {
        return None;
    }
    if x == 0.0 {
        // No negative zero in a file.
        return Some("0".to_string());
    }
    let size = x.abs();
    Some(if (1e-5..1e17).contains(&size) {
        format!("{x}")
    } else {
        format!("{x:E}")
    })
}
