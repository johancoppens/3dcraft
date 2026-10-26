//! Closed profiles from the curves of a sketch: which curves join into
//! loops, which loop is the outside of a region and which are its holes.
//!
//! Version 1 accepts simple closed loops of lines, arcs and splines joined
//! end to end, and circles, where no two loops touch or cross. Everything
//! else is an error that names the curves involved; a region is never
//! guessed.
//!
//! A spline is a loop on its own when it closes on itself (its last
//! control point on its first). It is then cut in two, at the inner knot
//! nearest the middle of its domain, and is a loop of two segments with
//! the same `source` and the `piece` numbers 0 and 1: what it sweeps is
//! two faces, neither on a surface that closes on itself (for which
//! `craft3d-geom` has no period, so nothing downstream could follow its
//! seam). A spline with corners (an inner knot as often as its degree,
//! which is every inner knot of a spline of degree 1, a polyline) is cut
//! at them the same way, into as many segments as it has smooth pieces; a
//! piece of degree 1 is a `Shape::Line`. The pieces are the spline
//! exactly (knot insertion).
//!
//! How it works:
//! 1. Curve ends closer than the linear tolerance are one junction. Exactly
//!    two ends meet at a junction; one alone is an open chain, three or more
//!    a branch.
//! 2. Both curves at a junction get the same end point, the
//!    *representative*: the end of the arc when an arc meets a line or a
//!    spline (a line through two given points is exact, and so is a spline
//!    whose end control point is moved; an arc about a given centre is
//!    not), otherwise the end of the curve with the smaller id.
//! 3. Following junctions from the smallest unused curve id gives the loops.
//! 4. Every pair of curves is tested for touching: the parts of one curve
//!    within tolerance of the other may only be a stretch at an end the two
//!    share, never the whole curve. Lines, arcs and circles are compared
//!    exactly; a pair with a spline in it, and a spline with itself, by
//!    subdivision, as sharp as [`crate::spline2`] says.
//! 5. Loops then cannot cross, so one point of a loop tells whether it is
//!    inside another (winding number). Nesting is even-odd: depth 0, 2, …
//!    are outer boundaries, depth 1, 3, … holes of the loop around them.
//!
//! The pairs of steps 1, 4 and 5 that can meet at all (ends within twice
//! the tolerance in x and y, curves whose boxes grown by the tolerance
//! overlap, loops whose box holds a point of the other) are found in a
//! tree of boxes ([`BoxTree`]), and judged in the order a sweep in x over
//! every pair would judge them, so the result and the first error are the
//! same as comparing every pair (`profile_reference.rs` keeps that sweep
//! as the oracle). The winding number of a loop of many segments is summed
//! over a tree of its stretches: a stretch whose box does not hold the
//! point turns by the angle between its ends. How many pairs a step may
//! look at is bounded ([`MAX_PAIRS`]).
//!
//! The result does not depend on the order of the input or on the direction
//! lines were drawn in: curves are processed by ascending id, loops start at
//! their smallest id, outer loops run counter-clockwise and holes clockwise.

use crate::boxtree::BoxTree;
use crate::spline2::{self, Part};
pub use crate::spline2::{MAX_SPLINE_DEGREE, MAX_SPLINE_POINTS, Spline2};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::GeomError;
use craft3d_math::Tolerance;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

#[cfg(test)]
#[path = "profile_reference.rs"]
mod reference;

/// The most curves [`detect_profiles`] accepts (after splines are cut at
/// their corners). Its time grows as `n log n` with the curves of a drawn
/// or patterned sketch (the pairs it compares come from trees of boxes);
/// what bounds hostile input is [`MAX_PAIRS`]. This cap bounds memory, and
/// the time of the sheet of round holes it allows (see `docs/design/sweeps.md`).
pub const MAX_CURVES: usize = 1 << 18;

/// Coordinates and radii beyond this are refused: their products overflow.
const MAX_COORDINATE: f64 = 1e100;

/// A profile loop without segments is refused with this reason.
pub(crate) const NO_SEGMENTS: &str = "a loop without segments";
/// A profile loop of one segment that is not a circle is refused with
/// this reason.
pub(crate) const ONE_SEGMENT_OPEN: &str =
    "a loop of one segment that is not a circle is not closed";
/// A circle in a profile loop of several segments is refused with this
/// reason.
pub(crate) const CIRCLE_NOT_ALONE: &str = "a circle is a loop on its own";

type P2 = [f64; 2];

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}

fn scaled(a: P2, s: f64) -> P2 {
    [a[0] * s, a[1] * s]
}

fn dot(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn length(a: P2) -> f64 {
    a[0].hypot(a[1])
}

fn distance(a: P2, b: P2) -> f64 {
    length(sub(a, b))
}

fn angle_of(v: P2) -> f64 {
    v[1].atan2(v[0])
}

/// An angle wrapped to `[0, 2π)`.
fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    // rem_euclid can return TAU itself for tiny negative input.
    if w >= TAU { 0.0 } else { w }
}

fn in_range(p: P2) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
}

fn internal() -> KernelError {
    KernelError::Geometry(GeomError::InvalidParameter(
        "profile detection lost track of a curve; this is a bug",
    ))
}

/// A curve of a sketch in the sketch's own 2D axes.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve2 {
    Line {
        start: [f64; 2],
        end: [f64; 2],
    },
    /// Counter-clockwise from `start` to `end` about `center`.
    Arc {
        center: [f64; 2],
        start: [f64; 2],
        end: [f64; 2],
    },
    Circle {
        center: [f64; 2],
        radius: f64,
    },
    /// A polynomial B-spline from its first control point to its last.
    Spline(Spline2),
}

/// A curve with the caller's stable id for it (the sketch entity id).
#[derive(Clone, Debug, PartialEq)]
pub struct SketchCurve {
    pub id: u64,
    pub curve: Curve2,
}

/// What a [`Segment`] runs along.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Straight from `start` to `end`.
    Line,
    /// Along the circle about `center`: counter-clockwise from `start` to
    /// `end`, or clockwise when the segment is `reversed`.
    Arc { center: [f64; 2], radius: f64 },
    /// A whole circle, from its point at angle 0 (`center + (radius, 0)`)
    /// back to it: counter-clockwise, or clockwise when `reversed`.
    Circle { center: [f64; 2], radius: f64 },
    /// Along the spline, in the spline's own direction: from its first
    /// control point to its last, or back when the segment is `reversed`.
    /// The end control points are the ends of the segment exactly.
    Spline(Spline2),
}

/// The use of a curve by a loop: in its own direction, or against it.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// The id of the sketch curve.
    pub source: u64,
    /// Which piece of that curve, counted from its start: 0 unless the
    /// curve was cut into several (a spline at its corners, and a spline
    /// that closes on itself in two; see the module documentation).
    pub piece: u32,
    /// True when the loop runs against the curve's own direction (a line
    /// from its end to its start, an arc or circle clockwise).
    pub reversed: bool,
    /// Where the loop enters the segment. Ends that coincide within
    /// tolerance have been made one point (the representative, see the
    /// module documentation), so this is exactly the `end` of the segment
    /// before it.
    pub start: [f64; 2],
    /// Where the loop leaves the segment.
    pub end: [f64; 2],
    pub shape: Shape,
}

impl Segment {
    /// The start and end in the curve's own direction.
    fn own_ends(&self) -> (P2, P2) {
        if self.reversed {
            (self.end, self.start)
        } else {
            (self.start, self.end)
        }
    }

    /// For an arc or circle: the angle its counter-clockwise run starts at
    /// and the angle it sweeps, in `[0, 2π]`. `None` for a line or a spline.
    pub fn angles(&self) -> Option<(f64, f64)> {
        match self.shape {
            Shape::Line | Shape::Spline(_) => None,
            Shape::Circle { .. } => Some((0.0, TAU)),
            Shape::Arc { center, .. } => {
                let (from, to) = self.own_ends();
                let begin = angle_of(sub(from, center));
                let sweep = wrap(angle_of(sub(to, center)) - begin);
                Some((begin, sweep))
            }
        }
    }

    /// The same segment run the other way.
    fn opposite(&self) -> Segment {
        Segment {
            source: self.source,
            piece: self.piece,
            reversed: !self.reversed,
            start: self.end,
            end: self.start,
            shape: self.shape.clone(),
        }
    }

    /// The segment in its own direction as pieces that can be boxed and
    /// cut in two, for comparing it with a spline.
    fn parts(&self) -> Vec<Part> {
        let (from, to) = self.own_ends();
        match &self.shape {
            Shape::Line => vec![Part::line(from, to)],
            Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
                let (begin, sweep) = self.angles().unwrap_or((0.0, TAU));
                vec![Part::arc(*center, *radius, begin, sweep)]
            }
            Shape::Spline(spline) => spline.pieces(),
        }
    }

    /// The point at `t` in `[0, 1]` along the curve's own direction.
    fn point_at(&self, t: f64) -> P2 {
        match self.shape {
            Shape::Spline(_) => spline2::point_on(&self.parts(), t).unwrap_or([f64::NAN, f64::NAN]),
            Shape::Line => {
                let (a, b) = self.own_ends();
                add(a, scaled(sub(b, a), t))
            }
            Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
                let (begin, sweep) = self.angles().unwrap_or((0.0, TAU));
                let (s, c) = (begin + t * sweep).sin_cos();
                add(center, [radius * c, radius * s])
            }
        }
    }

    /// The distance from `p` to the nearest point of the segment; to a
    /// spline within an eighth of `tol`.
    fn distance_to(&self, p: P2, tol: f64) -> f64 {
        match self.shape {
            Shape::Spline(_) => spline2::distance_to(&self.parts(), p, tol),
            Shape::Line => {
                let (a, b) = (self.start, self.end);
                let d = sub(b, a);
                let l2 = dot(d, d);
                let t = if l2 > 0.0 {
                    (dot(sub(p, a), d) / l2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                distance(p, add(a, scaled(d, t)))
            }
            Shape::Circle { center, radius } => (distance(p, center) - radius).abs(),
            Shape::Arc { center, radius } => {
                let (begin, sweep) = self.angles().unwrap_or((0.0, 0.0));
                let v = sub(p, center);
                if wrap(angle_of(v) - begin) <= sweep {
                    (length(v) - radius).abs()
                } else {
                    distance(p, self.start).min(distance(p, self.end))
                }
            }
        }
    }

    /// `[min x, min y, max x, max y]`.
    fn bbox(&self) -> [f64; 4] {
        let mut b = [
            self.start[0].min(self.end[0]),
            self.start[1].min(self.end[1]),
            self.start[0].max(self.end[0]),
            self.start[1].max(self.end[1]),
        ];
        if let Shape::Spline(spline) = &self.shape {
            // The control points contain the curve.
            let o = spline.bbox();
            b = [
                b[0].min(o[0]),
                b[1].min(o[1]),
                b[2].max(o[2]),
                b[3].max(o[3]),
            ];
        }
        if let Shape::Arc { center, radius } | Shape::Circle { center, radius } = self.shape {
            let (begin, sweep) = self.angles().unwrap_or((0.0, TAU));
            for quarter in 0..4_u8 {
                let angle = f64::from(quarter) * FRAC_PI_2;
                if wrap(angle - begin) <= sweep {
                    let (s, c) = angle.sin_cos();
                    let p = add(center, [radius * c, radius * s]);
                    b = [
                        b[0].min(p[0]),
                        b[1].min(p[1]),
                        b[2].max(p[0]),
                        b[3].max(p[1]),
                    ];
                }
            }
        }
        b
    }

    /// The segment's share of `∮ (x dy − y dx) / 2` and of the first
    /// moments `∮ x² dy / 2` and `−∮ y² dx / 2`, with coordinates taken
    /// relative to `origin`: `(area, moment x, moment y)`.
    ///
    /// A line contributes the triangle (origin, start, end). An arc
    /// contributes the triangle of its chord plus the circular segment
    /// between chord and arc: area `r² (Δ − sin Δ) / 2` with its centroid
    /// `4 r sin³(Δ/2) / (3 (Δ − sin Δ))` from the centre along the bisector,
    /// so its moment about the centre is `(2/3) r³ sin³(Δ/2)`. The circular
    /// segment counts positive counter-clockwise, negative clockwise.
    ///
    /// A spline contributes the region swept by the line from the origin
    /// to a point running along it, integrated exactly (Gauss–Legendre on
    /// each of its polynomial pieces).
    fn moments(&self, origin: P2) -> (f64, f64, f64) {
        let sign = if self.reversed { -1.0 } else { 1.0 };
        if let Shape::Spline(spline) = &self.shape {
            let (area, mx, my) = spline2::moments(spline, origin);
            return (sign * area, sign * mx, sign * my);
        }
        if let Shape::Circle { center, radius } = self.shape {
            let area = sign * PI * radius * radius;
            let c = sub(center, origin);
            return (area, area * c[0], area * c[1]);
        }
        let (a, b) = (sub(self.start, origin), sub(self.end, origin));
        let w = cross(a, b);
        let mut sum = (w / 2.0, (a[0] + b[0]) * w / 6.0, (a[1] + b[1]) * w / 6.0);
        if let (&Shape::Arc { center, radius }, Some((begin, sweep))) = (&self.shape, self.angles())
        {
            let area = sign * radius * radius * (sweep - sweep.sin()) / 2.0;
            let c = sub(center, origin);
            let half = (sweep / 2.0).sin();
            let lever = sign * 2.0 / 3.0 * radius * radius * radius * half * half * half;
            let (s, co) = (begin + sweep / 2.0).sin_cos();
            sum.0 += area;
            sum.1 += area * c[0] + lever * co;
            sum.2 += area * c[1] + lever * s;
        }
        sum
    }

    /// The angle the direction from `p` to a point running along the
    /// segment turns through. `p` must not be on the segment.
    fn turn_about(&self, p: P2) -> f64 {
        let sign = if self.reversed { -1.0 } else { 1.0 };
        let (from, to) = self.own_ends();
        let (a, b) = (sub(from, p), sub(to, p));
        match self.shape {
            Shape::Line => cross(a, b).atan2(dot(a, b)) * sign,
            Shape::Spline(_) => spline2::turn_about(&self.parts(), p) * sign,
            Shape::Circle { center, radius } => {
                if distance(p, center) < radius {
                    sign * TAU
                } else {
                    0.0
                }
            }
            Shape::Arc { center, radius } => {
                // As seen along the chord, plus a whole turn when `p` lies
                // between the chord and the arc: the arc then passes behind
                // it. That region is inside the circle and to the right of
                // the chord, where the cross product is negative; the same
                // number decides the sign of the chord's angle, so the sum
                // is continuous when `p` is on the chord itself.
                let s = cross(a, b);
                let chord = s.atan2(dot(a, b));
                let behind = distance(p, center) < radius && s < 0.0;
                sign * (chord + if behind { TAU } else { 0.0 })
            }
        }
    }
}

/// A closed chain of segments: each starts where the one before it ends,
/// and the first where the last ends. It starts at its smallest curve id.
#[derive(Clone, Debug, PartialEq)]
pub struct Loop2 {
    pub segments: Vec<Segment>,
}

impl Loop2 {
    fn origin(&self) -> P2 {
        self.segments.first().map_or([0.0, 0.0], |s| s.start)
    }

    fn moments(&self, origin: P2) -> (f64, f64, f64) {
        let mut sum = (0.0, 0.0, 0.0);
        for segment in &self.segments {
            let m = segment.moments(origin);
            sum = (sum.0 + m.0, sum.1 + m.1, sum.2 + m.2);
        }
        sum
    }

    /// The enclosed area: positive counter-clockwise, negative clockwise.
    /// Exact for lines and arcs (Green's theorem in closed form) and for
    /// splines (the same integral by a quadrature that is exact for them).
    pub fn signed_area(&self) -> f64 {
        self.moments(self.origin()).0
    }

    /// The loop run the other way, starting at the same segment.
    fn flipped(&self) -> Loop2 {
        let mut segments = Vec::with_capacity(self.segments.len());
        if let Some(first) = self.segments.first() {
            segments.push(first.opposite());
        }
        segments.extend(self.segments.iter().skip(1).rev().map(Segment::opposite));
        Loop2 { segments }
    }

    /// The curve ids, ascending, at most `limit` of them.
    fn sources(&self, limit: usize) -> Vec<u64> {
        let mut ids: Vec<u64> = self.segments.iter().map(|s| s.source).collect();
        ids.sort_unstable();
        // A curve cut into pieces is one curve.
        ids.dedup();
        ids.truncate(limit);
        ids
    }

    /// `[min x, min y, max x, max y]` of the loop: its ends and the
    /// extremes of its arcs and circles, and the control points of its
    /// splines, so it contains the loop.
    pub(crate) fn bbox(&self) -> [f64; 4] {
        let mut b = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for s in &self.segments {
            let o = s.bbox();
            b = [
                b[0].min(o[0]),
                b[1].min(o[1]),
                b[2].max(o[2]),
                b[3].max(o[3]),
            ];
        }
        b
    }
}

/// A region of the plane: an outer boundary (counter-clockwise) and its
/// holes (clockwise).
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub outer: Loop2,
    pub holes: Vec<Loop2>,
}

impl Profile {
    fn moments(&self) -> (P2, (f64, f64, f64)) {
        let origin = self.outer.origin();
        let mut sum = self.outer.moments(origin);
        for hole in &self.holes {
            let m = hole.moments(origin);
            sum = (sum.0 + m.0, sum.1 + m.1, sum.2 + m.2);
        }
        (origin, sum)
    }

    /// The area of the region: the outer boundary less the holes. Exact for
    /// lines and arcs (Green's theorem in closed form per segment).
    pub fn area(&self) -> f64 {
        self.moments().1.0
    }

    /// The centre of area; `None` when the area is zero or not finite.
    pub fn centroid(&self) -> Option<[f64; 2]> {
        let (origin, (area, mx, my)) = self.moments();
        let c = [origin[0] + mx / area, origin[1] + my / area];
        (area != 0.0 && c.iter().all(|v| v.is_finite())).then_some(c)
    }
}

fn degenerate(id: u64, reason: &'static str) -> KernelError {
    KernelError::DegenerateCurve { id, reason }
}

/// A curve must be finite, within range and longer than the tolerance.
pub(crate) fn validate(curve: &SketchCurve, tol: f64) -> Result<(), KernelError> {
    let id = curve.id;
    let not_finite = "a coordinate is not finite or is out of range";
    match &curve.curve {
        Curve2::Spline(spline) => {
            if !spline.control.iter().all(|p| in_range(*p)) {
                return Err(degenerate(id, not_finite));
            }
            if let Some(fault) = spline.fault() {
                return Err(degenerate(id, fault));
            }
            let b = spline.bbox();
            if (b[2] - b[0]).hypot(b[3] - b[1]) <= tol {
                return Err(degenerate(id, "the spline has no length"));
            }
        }
        &Curve2::Line { start, end } => {
            if !(in_range(start) && in_range(end)) {
                return Err(degenerate(id, not_finite));
            }
            if distance(start, end) <= tol {
                return Err(degenerate(id, "the line has no length"));
            }
        }
        &Curve2::Arc { center, start, end } => {
            if !(in_range(center) && in_range(start) && in_range(end)) {
                return Err(degenerate(id, not_finite));
            }
            let (rs, re) = (distance(center, start), distance(center, end));
            if rs <= tol || re <= tol {
                return Err(degenerate(id, "the arc has no radius"));
            }
            if (rs - re).abs() > tol {
                return Err(degenerate(
                    id,
                    "the start and the end of the arc are not at the same distance from its centre",
                ));
            }
            if distance(start, end) <= tol {
                return Err(degenerate(
                    id,
                    "the start and the end of the arc coincide; use a circle for a full turn",
                ));
            }
        }
        &Curve2::Circle { center, radius } => {
            if !(in_range(center) && in_range([radius, 0.0])) {
                return Err(degenerate(id, not_finite));
            }
            if radius <= tol {
                return Err(degenerate(id, "the radius is zero or negative"));
            }
        }
    }
    Ok(())
}

/// One end of a line, an arc or a spline.
struct End {
    /// Index of the curve in id order.
    curve: usize,
    /// 0 for the start, 1 for the end.
    which: usize,
    point: P2,
    /// True for a curve that goes exactly through any end point it is
    /// given: a line or a spline, not an arc.
    line: bool,
}

/// The most pairs of curves, or of loops, that one step of
/// [`detect_profiles`] looks at because their boxes overlap (or a loop's
/// box holds a point of the other), counted in both directions and each
/// with itself: as many as [`OLD_MAX_CURVES`] curves can have, so nothing
/// accepted under that cap is refused for it. Pairs of curve ends may be
/// four times as many (two ends per curve). Drawn and patterned sketches
/// need a few per curve.
pub const MAX_PAIRS: usize = OLD_MAX_CURVES * OLD_MAX_CURVES;

/// The most control points the splines of one call to [`detect_profiles`]
/// may have in all. Deciding where a spline keeps clear of itself and of
/// the curves near it takes time in proportion to its control points
/// (25 to 90 µs each in a release build on an idle machine, measured on
/// closed cubic splines of 8 to 4095 points; see `docs/design/sweeps.md`),
/// so this bounds that work: at the bound 9 to 14 s for splines of 8 to
/// 512 points and 53 s for 64 splines of 4096 points, measured on a loaded
/// machine (about 2.4 times slower). One spline may have
/// [`MAX_SPLINE_POINTS`]. Inputs above it were accepted before this bound,
/// slowly (minutes to hours): a decision for the developer.
pub const MAX_SPLINE_POINTS_IN_ALL: usize = 1 << 18;

/// Refuses splines with more control points in all than
/// [`MAX_SPLINE_POINTS_IN_ALL`], naming those with the most.
fn spline_work(curves: &[SketchCurve]) -> Result<(), KernelError> {
    let points = |c: &SketchCurve| match &c.curve {
        Curve2::Spline(spline) => spline.control.len(),
        _ => 0,
    };
    let count = curves
        .iter()
        .map(points)
        .fold(0_usize, usize::saturating_add);
    if count <= MAX_SPLINE_POINTS_IN_ALL {
        return Ok(());
    }
    let mut largest: Vec<(usize, u64)> = curves
        .iter()
        .filter(|c| points(c) > 0)
        .map(|c| (points(c), c.id))
        .collect();
    largest.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut ids: Vec<u64> = largest
        .iter()
        .take(MAX_REPORTED)
        .map(|&(_, id)| id)
        .collect();
    ids.sort_unstable();
    Err(KernelError::TooManySplinePoints {
        count,
        limit: MAX_SPLINE_POINTS_IN_ALL,
        curves: ids,
    })
}

/// The cap on curves before the broad phase (until 2026-10-17), when every
/// step compared pairs swept in x: [`MAX_PAIRS`] is set from it.
pub const OLD_MAX_CURVES: usize = 4096;

/// The pairs a step of detection may still look at, of a limit.
struct Work {
    left: usize,
    limit: usize,
}

impl Work {
    fn new(limit: usize) -> Work {
        Work { left: limit, limit }
    }

    /// The error when the search near the curves `ids` (the one searched
    /// about first, then those found near it) ran out of pairs.
    fn crowded(&self, ids: impl IntoIterator<Item = u64>) -> KernelError {
        let mut curves: Vec<u64> = ids.into_iter().collect();
        curves.sort_unstable();
        curves.dedup();
        curves.truncate(MAX_REPORTED);
        KernelError::CrowdedCurves {
            curves,
            limit: self.limit,
        }
    }
}

/// `order` sorted by `key`, then index, and where each index is in it.
fn sweep_order(len: usize, key: impl Fn(usize) -> f64) -> (Vec<usize>, Vec<usize>) {
    let mut order: Vec<usize> = (0..len).collect();
    order.sort_by(|&i, &j| key(i).total_cmp(&key(j)).then(i.cmp(&j)));
    let mut position = vec![0; len];
    for (at, &i) in order.iter().enumerate() {
        if let Some(slot) = position.get_mut(i) {
            *slot = at;
        }
    }
    (order, position)
}

/// For every end the end it meets. Errors when an end meets none, or more
/// than one.
///
/// Two ends meet when they are within `tol`; which of them comes first is
/// the order of the ends by x (then index), as listed in errors. Candidates
/// come from a [`BoxTree`] of the end points searched within `2 tol` in x
/// and in y, which holds every end within `tol` (a distance is at least
/// either of its components, here with room for the rounding of `hypot`);
/// each candidate is then judged exactly as by comparing every pair.
fn join_ends(curves: &[SketchCurve], ends: &[End], tol: f64) -> Result<Vec<usize>, KernelError> {
    let x = |i: usize| ends.get(i).map_or(f64::NAN, |e| e.point[0]);
    let (_, position) = sweep_order(ends.len(), x);
    let tree = BoxTree::new(
        ends.iter()
            .map(|e| [e.point[0], e.point[1], e.point[0], e.point[1]])
            .collect(),
    );
    let reach = 2.0 * tol;
    let mut work = Work::new(4 * MAX_PAIRS);
    let mut found = Vec::new();
    let id_of = |end: usize| {
        ends.get(end)
            .and_then(|e| curves.get(e.curve))
            .map(|c| c.id)
    };
    // The first two ends near each end, in x order, and how many there are.
    let mut near: Vec<(usize, [usize; 2])> = Vec::with_capacity(ends.len());
    for (i, a) in ends.iter().enumerate() {
        let p = a.point;
        let within = tree.search(
            |b| {
                !(b[0] - p[0] > reach
                    || p[0] - b[2] > reach
                    || b[1] - p[1] > reach
                    || p[1] - b[3] > reach)
            },
            &mut found,
            &mut work.left,
        );
        if !within {
            return Err(work.crowded(
                std::iter::once(i)
                    .chain(found.iter().copied())
                    .filter_map(id_of),
            ));
        }
        let at = position.get(i).copied().ok_or_else(internal)?;
        let mut count = 0_usize;
        // The two nearest the start of the x order, as (place, end).
        let mut first_two = [(usize::MAX, usize::MAX); 2];
        for &j in &found {
            let (Some(b), Some(&bt)) = (ends.get(j), position.get(j)) else {
                return Err(internal());
            };
            if j == i {
                continue;
            }
            // The pair as a sweep in x meets it: the earlier end first.
            let (first, second) = if bt > at { (a, b) } else { (b, a) };
            // (The sweep stopped at an end more than `tol` further on in x.)
            let beyond = second.point[0] - first.point[0] > tol;
            if !beyond && distance(first.point, second.point) <= tol {
                count += 1;
                if bt < first_two[0].0 {
                    first_two = [(bt, j), first_two[0]];
                } else if bt < first_two[1].0 {
                    first_two[1] = (bt, j);
                }
            }
        }
        near.push((count, first_two.map(|(_, j)| j)));
    }
    let report = |mut ids: Vec<u64>| {
        ids.sort_unstable();
        ids.dedup();
        ids.truncate(MAX_REPORTED);
        ids
    };
    let mut branching = Vec::new();
    let mut open = Vec::new();
    for (i, (count, list)) in near.iter().enumerate() {
        match count {
            0 => open.extend(id_of(i)),
            1 => {}
            _ => {
                branching.extend(id_of(i));
                branching.extend(list.iter().filter_map(|&j| id_of(j)));
            }
        }
    }
    if !branching.is_empty() {
        return Err(KernelError::BranchingProfile {
            curves: report(branching),
        });
    }
    if !open.is_empty() {
        return Err(KernelError::OpenProfile {
            curves: report(open),
        });
    }
    Ok(near.iter().map(|(_, list)| list[0]).collect())
}

/// A piece of the boundary of everything within tolerance of a curve.
enum Bound {
    Line { p: P2, d: P2 },
    Circle { c: P2, r: f64 },
}

/// Curves that together contain the boundary of the region within `tol` of
/// `b`: offset lines and end circles for a line, offset circles and end
/// circles for an arc, offset circles for a circle.
fn offset_bounds(b: &Segment, tol: f64) -> Vec<Bound> {
    let mut bounds = Vec::with_capacity(4);
    match b.shape {
        Shape::Line => {
            let d = sub(b.end, b.start);
            let l = length(d);
            if l > 0.0 {
                let n = [-d[1] / l * tol, d[0] / l * tol];
                bounds.push(Bound::Line {
                    p: add(b.start, n),
                    d,
                });
                bounds.push(Bound::Line {
                    p: sub(b.start, n),
                    d,
                });
            }
        }
        // Splines are not compared through offset curves (`keeps_clear`).
        Shape::Spline(_) => {}
        Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
            bounds.push(Bound::Circle {
                c: center,
                r: radius + tol,
            });
            if radius > tol {
                bounds.push(Bound::Circle {
                    c: center,
                    r: radius - tol,
                });
            }
        }
    }
    if !matches!(b.shape, Shape::Circle { .. }) {
        bounds.push(Bound::Circle { c: b.start, r: tol });
        bounds.push(Bound::Circle { c: b.end, r: tol });
    }
    bounds
}

/// The parameters `s` at which the line `p + s d` meets the circle.
fn line_circle(p: P2, d: P2, c: P2, r: f64) -> [Option<f64>; 2] {
    let f = sub(p, c);
    let a = dot(d, d);
    // (f·d)² − |d|² (|f|² − r²), written so that a small circle far along
    // the line does not cancel.
    let k = cross(f, d);
    let disc = a * r * r - k * k;
    if !(a > 0.0 && disc >= 0.0) {
        return [None, None];
    }
    let (b, root) = (dot(f, d), disc.sqrt());
    [Some((-b - root) / a), Some((-b + root) / a)]
}

/// The points where two circles meet.
fn circle_circle(c1: P2, r1: f64, c2: P2, r2: f64) -> [Option<P2>; 2] {
    let e = sub(c2, c1);
    let d = length(e);
    if d.is_nan() || d <= 0.0 {
        return [None, None];
    }
    // The foot of the common chord is `d − k` from c1, and the half chord
    // is h with h² = r1² − (d − k)², factored so that a small circle
    // centred on a large one does not cancel.
    let k = ((d - r1) * (d + r1) + r2 * r2) / (2.0 * d);
    let h2 = (k - (d - r1)) * (r1 + d - k);
    if h2.is_nan() || h2 < 0.0 {
        return [None, None];
    }
    let (h, u) = (h2.sqrt(), scaled(e, 1.0 / d));
    let foot = add(c1, scaled(u, d - k));
    let side = [-u[1] * h, u[0] * h];
    [Some(add(foot, side)), Some(sub(foot, side))]
}

/// Adds the parameters in `[0, 1]` at which `a` meets `bound`.
fn crossings(a: &Segment, bound: &Bound, out: &mut Vec<f64>) {
    let mut push = |t: f64| {
        if (0.0..=1.0).contains(&t) {
            out.push(t);
        }
    };
    match a.shape {
        Shape::Spline(_) => {}
        Shape::Line => {
            let (p, d) = (a.start, sub(a.end, a.start));
            match *bound {
                Bound::Line { p: q, d: e } => {
                    let denominator = cross(d, e);
                    if denominator != 0.0 {
                        push(cross(sub(q, p), e) / denominator);
                    }
                }
                Bound::Circle { c, r } => {
                    line_circle(p, d, c, r).into_iter().flatten().for_each(push);
                }
            }
        }
        Shape::Arc { center, radius } | Shape::Circle { center, radius } => {
            let points = match *bound {
                Bound::Line { p: q, d: e } => {
                    line_circle(q, e, center, radius).map(|s| s.map(|s| add(q, scaled(e, s))))
                }
                Bound::Circle { c, r } => circle_circle(center, radius, c, r),
            };
            let (begin, sweep) = a.angles().unwrap_or((0.0, TAU));
            if sweep > 0.0 {
                for point in points.into_iter().flatten() {
                    push(wrap(angle_of(sub(point, center)) - begin) / sweep);
                }
            }
        }
    }
}

/// Whether `a` keeps clear of `b`. Both run in their own direction;
/// `shared[k]` says that end `k` of `a` is joined to an end of `b`.
///
/// The parts of `a` within `tol` of `b` are found exactly: they can only
/// begin and end where `a` meets the boundary of the region within `tol` of
/// `b`, so `a` is sampled at those places and halfway between them. A
/// stretch that is close is allowed only when it reaches a shared end and
/// does not reach the other end of `a`: two curves always run close for a
/// while where they join (a long while when they join tangentially), but a
/// curve that is close all along lies on the other, and one that is close
/// anywhere else touches or crosses it.
///
/// A sample at the tolerance, give or take rounding, lies on the boundary
/// of that region: every break does, and so does the middle of two breaks
/// that (nearly) coincide, as where `a` leaves the region through the
/// corner of an offset line and an end circle. Which side such a sample
/// falls on is rounding noise (a slanted rectangle was refused for it), so
/// it counts as neither close nor clear and the runs are read from the
/// samples that are clearly one or the other ([`clear_band`]).
fn keeps_clear(a: &Segment, b: &Segment, shared: [bool; 2], tol: f64) -> bool {
    if matches!(a.shape, Shape::Spline(_)) || matches!(b.shape, Shape::Spline(_)) {
        return spline2::keeps_clear(&a.parts(), &b.parts(), shared, tol);
    }
    let mut breaks = Vec::with_capacity(10);
    for bound in offset_bounds(b, tol) {
        crossings(a, &bound, &mut breaks);
    }
    let band = clear_band(a, b, tol);
    let distance = |t: f64| b.distance_to(a.point_at(t), tol);
    if matches!(a.shape, Shape::Circle { .. }) {
        // No ends: nothing of a circle may be close. It is sampled at the
        // breaks and halfway between them, around the turn.
        breaks.push(0.0);
        breaks.sort_by(f64::total_cmp);
        let first = breaks.first().copied().unwrap_or(0.0);
        let after = breaks.iter().skip(1).copied().chain([first + 1.0]);
        let mut distances = Vec::with_capacity(breaks.len() * 2);
        for (&t, next) in breaks.iter().zip(after) {
            distances.push(distance(t));
            distances.push(distance((t + next) / 2.0));
        }
        return !classify(&distances, tol, band)
            .into_iter()
            .any(|close| close);
    }
    breaks.push(0.0);
    breaks.push(1.0);
    breaks.sort_by(f64::total_cmp);
    breaks.dedup();
    let mut distances = Vec::with_capacity(breaks.len() * 2);
    for (&t, &next) in breaks.iter().zip(breaks.iter().skip(1)) {
        distances.push(distance(t));
        distances.push(distance((t + next) / 2.0));
    }
    distances.push(distance(1.0));
    let samples = classify(&distances, tol, band);
    // A run that only boundary samples separate from an end reaches it.
    let last = samples.len().saturating_sub(1);
    let mut run_start = None;
    for (i, &is_close) in samples.iter().enumerate() {
        if is_close && run_start.is_none() {
            run_start = Some(i);
        }
        let run_ends = is_close && samples.get(i + 1) != Some(&true);
        if !run_ends {
            continue;
        }
        if let Some(start) = run_start.take() {
            let at_start = start == 0;
            let at_end = i == last;
            let allowed = (at_start && shared[0] && !at_end) || (at_end && shared[1] && !at_start);
            if !allowed {
                return false;
            }
        }
    }
    true
}

/// Which of the sampled `distances` are close (`true`) and clear
/// (`false`), leaving out those within `band` of `tol`, whose side is
/// rounding noise ([`keeps_clear`]). The band is accepted: a gap in
/// `(tol - band, tol]` is taken as clear when other samples decide, so
/// two curves that come that close without touching are not refused,
/// and one that comes closer than `tol - band` is. When no sample is
/// clearly either (every one within the band: a curve that runs at the
/// tolerance from the other all along), each is taken as `d <= tol`.
fn classify(distances: &[f64], tol: f64, band: f64) -> Vec<bool> {
    let clear: Vec<bool> = distances
        .iter()
        .filter_map(|&d| {
            if d <= tol - band {
                Some(true)
            } else if d > tol + band {
                Some(false)
            } else {
                None
            }
        })
        .collect();
    if clear.is_empty() {
        distances.iter().map(|&d| d <= tol).collect()
    } else {
        clear
    }
}

/// How far from the tolerance a distance between `a` and `b` has to be
/// to say on which side of it a point lies: a thousandth of the
/// tolerance, and at least the rounding of coordinates as large as theirs
/// (a few hundred units in the last place), so that the band is not lost
/// below the rounding far from the origin; never more than half the
/// tolerance, so that a crossing is still seen where the tolerance is
/// finer than the rounding.
fn clear_band(a: &Segment, b: &Segment, tol: f64) -> f64 {
    let size = a
        .bbox()
        .into_iter()
        .chain(b.bbox())
        .fold(0.0_f64, |m, c| m.max(c.abs()));
    (tol / 1024.0)
        .max(256.0 * f64::EPSILON * size)
        .min(tol / 2.0)
}

/// Boxes of the segments grown by the tolerance: no part of a segment
/// within `tol` of another lies outside its box.
fn clear_boxes(own: &[Segment], tol: f64) -> Vec<[f64; 4]> {
    own.iter()
        .map(|s| {
            let b = s.bbox();
            [b[0] - tol, b[1] - tol, b[2] + tol, b[3] + tol]
        })
        .collect()
}

/// A spline can cross itself.
fn check_self_clear(own: &[Segment], tol: f64) -> Result<(), KernelError> {
    for segment in own {
        if matches!(segment.shape, Shape::Spline(_)) && !spline2::self_clear(&segment.parts(), tol)
        {
            return Err(KernelError::CrossingCurves {
                curves: [segment.source, segment.source],
            });
        }
    }
    Ok(())
}

/// Errors when two curves touch, cross or overlap away from a shared end.
/// `mate_curve[i][k]` is the curve joined to end `k` of curve `i`.
///
/// Only pairs whose grown boxes overlap are compared, in the order of a
/// sweep in x: the curves by the left side of their box (then index), and
/// for each the ones after it. The pairs come from a [`BoxTree`] of the
/// boxes, so the first pair that fails is the one a sweep would fail on.
fn check_clear(
    own: &[Segment],
    mate_curve: &[[Option<usize>; 2]],
    tol: f64,
) -> Result<(), KernelError> {
    let boxes = clear_boxes(own, tol);
    check_self_clear(own, tol)?;
    let min_x = |i: usize| boxes.get(i).map_or(f64::NAN, |b| b[0]);
    let (order, position) = sweep_order(own.len(), min_x);
    let tree = BoxTree::new(boxes.clone());
    let mut work = Work::new(MAX_PAIRS);
    let (mut found, mut later) = (Vec::new(), Vec::new());
    for &i in &order {
        let (Some(a), Some(box_a), Some(mates_a), Some(&at)) =
            (own.get(i), boxes.get(i), mate_curve.get(i), position.get(i))
        else {
            return Err(internal());
        };
        let within = tree.search(
            |b| !(b[0] > box_a[2] || box_a[0] > b[2] || b[1] > box_a[3] || box_a[1] > b[3]),
            &mut found,
            &mut work.left,
        );
        if !within {
            let source = |j: &usize| own.get(*j).map(|s| s.source);
            return Err(
                work.crowded(std::iter::once(a.source).chain(found.iter().filter_map(source)))
            );
        }
        later.clear();
        for &j in &found {
            let &bt = position.get(j).ok_or_else(internal)?;
            if bt > at {
                later.push((bt, j));
            }
        }
        later.sort_unstable();
        for &(_, j) in &later {
            let (Some(b), Some(mates_b)) = (own.get(j), mate_curve.get(j)) else {
                return Err(internal());
            };
            let shared_a = mates_a.map(|m| m == Some(j));
            let shared_b = mates_b.map(|m| m == Some(i));
            if !(keeps_clear(a, b, shared_a, tol) && keeps_clear(b, a, shared_b, tol)) {
                return Err(KernelError::CrossingCurves {
                    curves: [a.source.min(b.source), a.source.max(b.source)],
                });
            }
        }
    }
    Ok(())
}

/// The segments of a loop in a tree of stretches (consecutive runs, halved
/// down to a few segments), each with its box, for the winding number
/// about many points.
struct Turns {
    nodes: Vec<TurnNode>,
}

struct TurnNode {
    bbox: [f64; 4],
    first: usize,
    end: usize,
    children: Option<[usize; 2]>,
}

/// How many segments a stretch at the bottom of [`Turns`] has at most.
const TURN_LEAF: usize = 8;

impl Turns {
    fn new(path: &Loop2) -> Turns {
        let placeholder = || TurnNode {
            bbox: [0.0; 4],
            first: 0,
            end: 0,
            children: None,
        };
        let mut nodes = vec![placeholder()];
        let mut todo = vec![(0_usize, 0_usize, path.segments.len())];
        while let Some((at, first, end)) = todo.pop() {
            let mut bbox = [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            for s in path.segments.get(first..end).unwrap_or(&[]) {
                let o = s.bbox();
                bbox = [
                    bbox[0].min(o[0]),
                    bbox[1].min(o[1]),
                    bbox[2].max(o[2]),
                    bbox[3].max(o[3]),
                ];
            }
            let mut children = None;
            if end - first > TURN_LEAF {
                let mid = first + (end - first) / 2;
                let (left, right) = (nodes.len(), nodes.len() + 1);
                nodes.push(placeholder());
                nodes.push(placeholder());
                todo.push((left, first, mid));
                todo.push((right, mid, end));
                children = Some([left, right]);
            }
            if let Some(node) = nodes.get_mut(at) {
                *node = TurnNode {
                    bbox,
                    first,
                    end,
                    children,
                };
            }
        }
        Turns { nodes }
    }

    /// How many times `path` (the loop the tree was made of) goes around
    /// `p`, counter-clockwise; `p` must not be on it. A stretch whose box
    /// does not hold `p` lies in a half-plane beside `p`, so the direction
    /// from `p` to it turns by less than half a turn: by the angle between
    /// its ends. Other stretches are summed segment by segment, as
    /// [`Loop2::winding`] sums the whole loop; the sum is a whole number of
    /// turns up to rounding either way.
    fn winding(&self, path: &Loop2, p: P2) -> f64 {
        let mut turn = 0.0;
        let mut stack = vec![0_usize];
        while let Some(at) = stack.pop() {
            let Some(node) = self.nodes.get(at) else {
                continue;
            };
            let b = node.bbox;
            if p[0] < b[0] || p[1] < b[1] || p[0] > b[2] || p[1] > b[3] {
                let first = path.segments.get(node.first);
                let last = node.end.checked_sub(1).and_then(|l| path.segments.get(l));
                if let (Some(first), Some(last)) = (first, last) {
                    let (u, v) = (sub(first.start, p), sub(last.end, p));
                    turn += cross(u, v).atan2(dot(u, v));
                }
                continue;
            }
            match node.children {
                Some(below) => stack.extend(below),
                None => {
                    let stretch = path.segments.get(node.first..node.end).unwrap_or(&[]);
                    turn += stretch.iter().map(|s| s.turn_about(p)).sum::<f64>();
                }
            }
        }
        (turn / TAU).round()
    }
}

/// How deep each loop is nested and the smallest loop around it. No two
/// loops touch, so a loop is inside another when one of its points is: the
/// loops whose box holds that point come from a [`BoxTree`], in no
/// particular order, and their winding numbers about it decide; the
/// parent is chosen as a scan in index order would choose it.
fn nest(rings: &[Ring]) -> Result<Vec<(usize, Option<usize>)>, KernelError> {
    let tree = BoxTree::new(rings.iter().map(|r| r.bbox).collect());
    let turns: Vec<Turns> = rings.iter().map(|r| Turns::new(&r.path)).collect();
    let mut work = Work::new(MAX_PAIRS);
    let mut found = Vec::new();
    let mut nesting = Vec::with_capacity(rings.len());
    for (i, ring) in rings.iter().enumerate() {
        let p = ring.path.origin();
        let within = tree.search(
            |b| !(p[0] < b[0] || p[1] < b[1] || p[0] > b[2] || p[1] > b[3]),
            &mut found,
            &mut work.left,
        );
        if !within {
            // The loop searched about, and the first curve of each loop found.
            let first = |j: &usize| {
                rings
                    .get(*j)
                    .and_then(|r| r.path.sources(1).first().copied())
            };
            return Err(work.crowded(
                ring.path
                    .sources(MAX_REPORTED)
                    .into_iter()
                    .chain(found.iter().filter_map(first)),
            ));
        }
        // The found loops come in no particular order: the parent is the
        // smallest loop round the point, the first in index order of
        // equal ones, as a scan in index order keeps.
        let (mut depth, mut parent): (usize, Option<(f64, usize)>) = (0, None);
        for &j in &found {
            let (Some(around), Some(turns)) = (rings.get(j), turns.get(j)) else {
                return Err(internal());
            };
            if i == j || turns.winding(&around.path, p) == 0.0 {
                continue;
            }
            depth += 1;
            if parent.is_none_or(|(area, k)| around.area < area || (around.area == area && j < k)) {
                parent = Some((around.area, j));
            }
        }
        nesting.push((depth, parent.map(|(_, j)| j)));
    }
    Ok(nesting)
}

/// A loop, counter-clockwise, with what nesting needs.
struct Ring {
    path: Loop2,
    area: f64,
    bbox: [f64; 4],
    /// How many other loops it is inside.
    depth: usize,
    /// The smallest loop it is inside.
    parent: Option<usize>,
}

/// Finds the closed regions bounded by `curves`.
///
/// Profiles are ordered by the smallest curve id of their outer loop, the
/// holes of a profile likewise, and every loop starts at its smallest curve
/// id. The result does not depend on the order of `curves`. See the module
/// documentation for what is accepted; everything else is an error.
pub fn detect_profiles(
    curves: &[SketchCurve],
    tolerance: &Tolerance,
) -> Result<Vec<Profile>, KernelError> {
    detect(curves, tolerance, Search::Indexed)
}

/// How the pairs that may meet are found.
#[derive(Clone, Copy, Debug)]
enum Search {
    /// From trees of boxes ([`BoxTree`]).
    Indexed,
    /// By sweeps in x over every pair, as before 2026-10-17: the reference
    /// the indexed search must agree with.
    #[cfg(test)]
    AllPairs,
}

fn detect(
    curves: &[SketchCurve],
    tolerance: &Tolerance,
    search: Search,
) -> Result<Vec<Profile>, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if curves.is_empty() {
        return Err(KernelError::NoProfile);
    }
    if curves.len() > MAX_CURVES {
        return Err(KernelError::TooManyCurves {
            count: curves.len(),
            limit: MAX_CURVES,
        });
    }
    let mut sorted = curves.to_vec();
    sorted.sort_by_key(|c| c.id);
    for (a, b) in sorted.iter().zip(sorted.iter().skip(1)) {
        if a.id == b.id {
            return Err(KernelError::DuplicateCurveId { id: a.id });
        }
    }
    for curve in &sorted {
        validate(curve, tol)?;
    }
    spline_work(&sorted)?;
    // A spline is cut where its sweep would not be one smooth face that
    // ends: at its corners, and in two when it closes on itself. The
    // pieces are curves of their own from here on, with the spline's id; a
    // piece of degree 1 is a line.
    let mut pieces: Vec<u32> = Vec::with_capacity(sorted.len());
    let mut cut: Vec<SketchCurve> = Vec::with_capacity(sorted.len());
    for curve in sorted {
        let Curve2::Spline(spline) = &curve.curve else {
            pieces.push(0);
            cut.push(curve);
            continue;
        };
        let closed = matches!(
            (spline.control.first(), spline.control.last()),
            (Some(first), Some(last)) if distance(*first, *last) <= tol
        );
        for (piece, part) in spline.profile_pieces(closed).into_iter().enumerate() {
            if cut.len() >= MAX_CURVES {
                return Err(KernelError::TooManyCurves {
                    count: cut.len().saturating_add(1),
                    limit: MAX_CURVES,
                });
            }
            let straight = match (part.degree, part.control.as_slice()) {
                (1, &[start, end]) => Some(Curve2::Line { start, end }),
                _ => None,
            };
            if let Some(Curve2::Line { start, end }) = straight
                && distance(start, end) <= tol
            {
                return Err(degenerate(
                    curve.id,
                    "two control points in a row of the polyline coincide",
                ));
            }
            pieces.push(u32::try_from(piece).unwrap_or(u32::MAX));
            cut.push(SketchCurve {
                id: curve.id,
                curve: straight.unwrap_or(Curve2::Spline(part)),
            });
        }
    }
    let sorted = cut;

    // Ends: two per line or arc, consecutive.
    let mut ends = Vec::new();
    let mut first_end: Vec<Option<usize>> = Vec::with_capacity(sorted.len());
    for (index, curve) in sorted.iter().enumerate() {
        let (start, end, line) = match &curve.curve {
            Curve2::Line { start, end } => (*start, *end, true),
            Curve2::Arc { start, end, .. } => (*start, *end, false),
            Curve2::Spline(spline) => {
                let (Some(start), Some(end)) = (spline.control.first(), spline.control.last())
                else {
                    return Err(internal());
                };
                (*start, *end, true)
            }
            Curve2::Circle { .. } => {
                first_end.push(None);
                continue;
            }
        };
        first_end.push(Some(ends.len()));
        for (which, point) in [start, end].into_iter().enumerate() {
            ends.push(End {
                curve: index,
                which,
                point,
                line,
            });
        }
    }
    let mates = match search {
        Search::Indexed => join_ends(&sorted, &ends, tol)?,
        #[cfg(test)]
        Search::AllPairs => reference::join_ends(&sorted, &ends, tol)?,
    };
    let mate_of = |end: usize| mates.get(end).and_then(|&m| ends.get(m));

    // The representative of a junction: an arc's end before a line's, then
    // the smaller id (`curve` is the index in id order).
    let mut snapped = Vec::with_capacity(ends.len());
    for (i, end) in ends.iter().enumerate() {
        let other = mate_of(i).ok_or_else(internal)?;
        if other.curve == end.curve {
            // (A spline that closes on itself was cut in two above.)
            return Err(degenerate(
                sorted.get(end.curve).map_or(0, |c| c.id),
                "its two ends coincide",
            ));
        }
        let wins = (other.line, other.curve) < (end.line, end.curve);
        snapped.push(if wins { other.point } else { end.point });
    }

    // Every curve in its own direction, between the representatives.
    let mut own = Vec::with_capacity(sorted.len());
    for ((curve, first), &piece) in sorted.iter().zip(&first_end).zip(&pieces) {
        let ends_of = first.and_then(|f| Some((*snapped.get(f)?, *snapped.get(f + 1)?)));
        let (start, end, shape) = match (&curve.curve, ends_of) {
            (&Curve2::Circle { center, radius }, _) => {
                let start = [center[0] + radius, center[1]];
                (start, start, Shape::Circle { center, radius })
            }
            (Curve2::Line { .. }, Some((start, end))) => (start, end, Shape::Line),
            (
                &Curve2::Arc {
                    center,
                    start: s,
                    end: e,
                },
                Some((start, end)),
            ) => {
                let radius = (distance(center, s) + distance(center, e)) / 2.0;
                (start, end, Shape::Arc { center, radius })
            }
            (Curve2::Spline(spline), Some((start, end))) => {
                // The spline goes through the representatives: its end
                // control points are moved onto them.
                let mut spline = spline.clone();
                if let Some(first) = spline.control.first_mut() {
                    *first = start;
                }
                if let Some(last) = spline.control.last_mut() {
                    *last = end;
                }
                (start, end, Shape::Spline(spline))
            }
            _ => return Err(internal()),
        };
        if first.is_some() && distance(start, end) <= tol {
            return Err(degenerate(
                curve.id,
                "it is shorter than the tolerance once its ends are joined to its neighbours",
            ));
        }
        own.push(Segment {
            source: curve.id,
            piece,
            reversed: false,
            start,
            end,
            shape,
        });
    }

    // Loops: from the smallest unused id, through the junctions, back.
    let mut used = vec![false; own.len()];
    let mut rings = Vec::new();
    for start in 0..own.len() {
        if used.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut segments = Vec::new();
        let (mut current, mut reversed) = (start, false);
        // A loop has at most every curve once.
        for _ in 0..own.len() {
            let segment = own.get(current).ok_or_else(internal)?;
            if let Some(flag) = used.get_mut(current) {
                *flag = true;
            }
            segments.push(if reversed {
                segment.opposite()
            } else {
                segment.clone()
            });
            let Some(first) = first_end.get(current).copied().flatten() else {
                break; // A circle is a loop on its own.
            };
            let exit = first + usize::from(!reversed);
            let next = mate_of(exit).ok_or_else(internal)?;
            if next.curve == start {
                break;
            }
            (current, reversed) = (next.curve, next.which == 1);
        }
        let mut path = Loop2 { segments };
        let mut area = path.signed_area();
        // Squared tolerance: the area of the smallest square that counts.
        if area.is_nan() || area.abs() <= tol * tol {
            return Err(KernelError::ZeroAreaLoop {
                curves: path.sources(MAX_REPORTED),
            });
        }
        if area < 0.0 {
            path = path.flipped();
            area = -area;
        }
        rings.push(Ring {
            bbox: path.bbox(),
            path,
            area,
            depth: 0,
            parent: None,
        });
    }

    let mate_curve: Vec<[Option<usize>; 2]> = first_end
        .iter()
        .map(|first| match first {
            Some(f) => [
                mate_of(*f).map(|e| e.curve),
                mate_of(f + 1).map(|e| e.curve),
            ],
            None => [None, None],
        })
        .collect();
    match search {
        Search::Indexed => check_clear(&own, &mate_curve, tol)?,
        #[cfg(test)]
        Search::AllPairs => reference::check_clear(&own, &mate_curve, tol)?,
    }

    // Nesting.
    let nesting = match search {
        Search::Indexed => nest(&rings)?,
        #[cfg(test)]
        Search::AllPairs => reference::nest(&rings),
    };
    for (ring, (depth, parent)) in rings.iter_mut().zip(nesting) {
        (ring.depth, ring.parent) = (depth, parent);
    }

    // Even depth: an outer boundary. Odd: a hole of the loop around it.
    let mut holes: Vec<Vec<Loop2>> = vec![Vec::new(); rings.len()];
    for ring in &rings {
        if ring.depth % 2 == 1 {
            let parent = ring.parent.ok_or_else(internal)?;
            holes
                .get_mut(parent)
                .ok_or_else(internal)?
                .push(ring.path.flipped());
        }
    }
    // Rings are in order of their smallest id already, and so are the holes
    // pushed above.
    Ok(rings
        .into_iter()
        .zip(holes)
        .filter(|(ring, _)| ring.depth % 2 == 0)
        .map(|(ring, holes)| Profile {
            outer: ring.path,
            holes,
        })
        .collect())
}

/// The most curves (pieces of splines counted) of one open chain.
pub const MAX_CHAIN_CURVES: usize = 1024;

const MORE_THAN_ONE_CHAIN: &str = "the curves make more than one chain: an open chain is one run of curves joined end to end, with two free ends";

/// The one open chain the curves of a sketch join into: lines, arcs and
/// splines joined end to end, without branches, with two free ends (an
/// open section of a loft, which makes a sheet).
///
/// Curves are read as [`detect_profiles`] reads them: ends within the
/// tolerance are one junction with one representative point, a spline is
/// cut at its corners, and no two curves may touch or cross anywhere but
/// where they join. The chain starts at the free end of the end curve with
/// the smaller id (of a single curve, at its own start) and is walked to
/// the other free end; each [`Segment`] says whether it runs against its
/// curve. A circle, a spline that closes on itself, curves that close into
/// a loop, a branch and curves that make more than one chain are refused.
pub fn open_chain(curves: &[SketchCurve], tolerance: &Tolerance) -> Result<Loop2, KernelError> {
    let tol = tolerance.linear;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(KernelError::InvalidSize {
            name: "tolerance",
            value: tol,
        });
    }
    if curves.is_empty() {
        return Err(KernelError::NoProfile);
    }
    if curves.len() > MAX_CHAIN_CURVES {
        return Err(KernelError::TooManyCurves {
            count: curves.len(),
            limit: MAX_CHAIN_CURVES,
        });
    }
    let mut sorted = curves.to_vec();
    sorted.sort_by_key(|c| c.id);
    for (a, b) in sorted.iter().zip(sorted.iter().skip(1)) {
        if a.id == b.id {
            return Err(KernelError::DuplicateCurveId { id: a.id });
        }
    }
    for curve in &sorted {
        validate(curve, tol)?;
    }
    spline_work(&sorted)?;
    // Splines cut at their corners, as for a profile; a piece of degree 1
    // is a line.
    let mut pieces: Vec<u32> = Vec::with_capacity(sorted.len());
    let mut cut: Vec<SketchCurve> = Vec::with_capacity(sorted.len());
    let too_many = |count: usize| KernelError::TooManyCurves {
        count: count.saturating_add(1),
        limit: MAX_CHAIN_CURVES,
    };
    for curve in sorted {
        match &curve.curve {
            Curve2::Circle { .. } => {
                return Err(KernelError::InvalidProfile {
                    reason: "a circle is closed: an open chain is lines, arcs and splines joined end to end",
                });
            }
            Curve2::Spline(spline) => {
                if let (Some(first), Some(last)) = (spline.control.first(), spline.control.last())
                    && distance(*first, *last) <= tol
                {
                    return Err(KernelError::InvalidProfile {
                        reason: "a spline that closes on itself: an open chain has two free ends",
                    });
                }
                for (piece, part) in spline.profile_pieces(false).into_iter().enumerate() {
                    if cut.len() >= MAX_CHAIN_CURVES {
                        return Err(too_many(cut.len()));
                    }
                    let straight = match (part.degree, part.control.as_slice()) {
                        (1, &[start, end]) => Some(Curve2::Line { start, end }),
                        _ => None,
                    };
                    if let Some(Curve2::Line { start, end }) = straight
                        && distance(start, end) <= tol
                    {
                        return Err(degenerate(
                            curve.id,
                            "two control points in a row of the polyline coincide",
                        ));
                    }
                    pieces.push(u32::try_from(piece).unwrap_or(u32::MAX));
                    cut.push(SketchCurve {
                        id: curve.id,
                        curve: straight.unwrap_or(Curve2::Spline(part)),
                    });
                }
            }
            Curve2::Line { .. } | Curve2::Arc { .. } => {
                if cut.len() >= MAX_CHAIN_CURVES {
                    return Err(too_many(cut.len()));
                }
                pieces.push(0);
                cut.push(curve);
            }
        }
    }
    // Ends: two per curve, consecutive (end `2 k` starts curve `k`).
    let mut ends = Vec::with_capacity(2 * cut.len());
    for (index, curve) in cut.iter().enumerate() {
        let (start, end, line) = match &curve.curve {
            Curve2::Line { start, end } => (*start, *end, true),
            Curve2::Arc { start, end, .. } => (*start, *end, false),
            Curve2::Spline(spline) => {
                let (Some(start), Some(end)) = (spline.control.first(), spline.control.last())
                else {
                    return Err(internal());
                };
                (*start, *end, true)
            }
            Curve2::Circle { .. } => return Err(internal()),
        };
        for (which, point) in [start, end].into_iter().enumerate() {
            ends.push(End {
                curve: index,
                which,
                point,
                line,
            });
        }
    }
    // The end each end meets, if any: at most one. (At most 2048 ends, so
    // every pair is looked at.)
    let mut mates: Vec<Option<usize>> = Vec::with_capacity(ends.len());
    for (i, a) in ends.iter().enumerate() {
        let near: Vec<usize> = ends
            .iter()
            .enumerate()
            .filter(|(j, b)| *j != i && distance(a.point, b.point) <= tol)
            .map(|(j, _)| j)
            .collect();
        match near.as_slice() {
            [] => mates.push(None),
            [j] => {
                if ends.get(*j).is_some_and(|b| b.curve == a.curve) {
                    return Err(degenerate(
                        cut.get(a.curve).map_or(0, |c| c.id),
                        "its two ends coincide",
                    ));
                }
                mates.push(Some(*j));
            }
            _ => {
                let mut ids: Vec<u64> = std::iter::once(i)
                    .chain(near.iter().copied())
                    .filter_map(|e| ends.get(e).and_then(|e| cut.get(e.curve)).map(|c| c.id))
                    .collect();
                ids.sort_unstable();
                ids.dedup();
                ids.truncate(MAX_REPORTED);
                return Err(KernelError::BranchingProfile { curves: ids });
            }
        }
    }
    let free: Vec<usize> = (0..ends.len())
        .filter(|&i| mates.get(i).is_some_and(Option::is_none))
        .collect();
    let first_free = match free.as_slice() {
        [] => {
            return Err(KernelError::InvalidProfile {
                reason: "the curves close into a loop: an open chain has two free ends (a closed loop is a profile)",
            });
        }
        [a, _] => *a,
        _ => {
            return Err(KernelError::InvalidProfile {
                reason: MORE_THAN_ONE_CHAIN,
            });
        }
    };
    // Representatives: an arc's end before a line's, then the smaller id.
    let mut snapped = Vec::with_capacity(ends.len());
    for (i, end) in ends.iter().enumerate() {
        let point = match mates.get(i).copied().flatten().and_then(|m| ends.get(m)) {
            Some(other) if (other.line, other.curve) < (end.line, end.curve) => other.point,
            _ => end.point,
        };
        snapped.push(point);
    }
    // Every curve in its own direction, between the representatives.
    let mut own = Vec::with_capacity(cut.len());
    for (index, (curve, &piece)) in cut.iter().zip(&pieces).enumerate() {
        let (Some(&start), Some(&end)) = (snapped.get(2 * index), snapped.get(2 * index + 1))
        else {
            return Err(internal());
        };
        let shape = match &curve.curve {
            Curve2::Line { .. } => Shape::Line,
            &Curve2::Arc {
                center,
                start: s,
                end: e,
            } => Shape::Arc {
                center,
                radius: (distance(center, s) + distance(center, e)) / 2.0,
            },
            Curve2::Spline(spline) => {
                let mut spline = spline.clone();
                if let Some(first) = spline.control.first_mut() {
                    *first = start;
                }
                if let Some(last) = spline.control.last_mut() {
                    *last = end;
                }
                Shape::Spline(spline)
            }
            Curve2::Circle { .. } => return Err(internal()),
        };
        if distance(start, end) <= tol {
            return Err(degenerate(
                curve.id,
                "it is shorter than the tolerance once its ends are joined to its neighbours",
            ));
        }
        own.push(Segment {
            source: curve.id,
            piece,
            reversed: false,
            start,
            end,
            shape,
        });
    }
    // No two curves touch or cross but where they join.
    let mate_curve: Vec<[Option<usize>; 2]> = (0..cut.len())
        .map(|k| {
            [2 * k, 2 * k + 1].map(|e| {
                mates
                    .get(e)
                    .copied()
                    .flatten()
                    .and_then(|m| ends.get(m))
                    .map(|m| m.curve)
            })
        })
        .collect();
    check_clear(&own, &mate_curve, tol)?;
    // From the first free end to the other.
    let mut segments = Vec::with_capacity(own.len());
    let mut entry = first_free;
    for _ in 0..own.len() {
        let end = ends.get(entry).ok_or_else(internal)?;
        let segment = own.get(end.curve).ok_or_else(internal)?;
        let reversed = end.which == 1;
        segments.push(if reversed {
            segment.opposite()
        } else {
            segment.clone()
        });
        let exit = 2 * end.curve + usize::from(!reversed);
        match mates.get(exit).copied().flatten() {
            Some(next) => entry = next,
            None => break,
        }
    }
    if segments.len() != own.len() {
        return Err(KernelError::InvalidProfile {
            reason: MORE_THAN_ONE_CHAIN,
        });
    }
    Ok(Loop2 { segments })
}
