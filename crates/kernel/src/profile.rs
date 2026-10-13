//! Closed profiles from the curves of a sketch: which curves join into
//! loops, which loop is the outside of a region and which are its holes.
//!
//! Version 1 accepts simple closed loops of lines, arcs and splines joined
//! end to end, and circles, where no two loops touch or cross. Everything
//! else is an error that names the curves involved; a region is never
//! guessed. (A spline that closes on itself is not a loop yet: its wall
//! would be a surface that closes on itself, which the tessellator does
//! not follow. Two splines that share both ends are.)
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
//! The result does not depend on the order of the input or on the direction
//! lines were drawn in: curves are processed by ascending id, loops start at
//! their smallest id, outer loops run counter-clockwise and holes clockwise.

use crate::spline2::{self, Part};
pub use crate::spline2::{MAX_SPLINE_DEGREE, MAX_SPLINE_POINTS, Spline2};
use crate::{KernelError, MAX_REPORTED};
use craft3d_geom::GeomError;
use craft3d_math::Tolerance;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// The most curves [`detect_profiles`] accepts. Touch detection compares
/// pairs of curves whose bounding boxes overlap, which is quadratic in the
/// worst case.
pub const MAX_CURVES: usize = 4096;

/// Coordinates and radii beyond this are refused: their products overflow.
const MAX_COORDINATE: f64 = 1e100;

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
        ids.truncate(limit);
        ids
    }

    /// How many times the loop goes around `p`, counter-clockwise.
    fn winding(&self, p: P2) -> f64 {
        let turn: f64 = self.segments.iter().map(|s| s.turn_about(p)).sum();
        (turn / TAU).round()
    }

    fn bbox(&self) -> [f64; 4] {
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
fn validate(curve: &SketchCurve, tol: f64) -> Result<(), KernelError> {
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
            if let (Some(first), Some(last)) = (spline.control.first(), spline.control.last())
                && distance(*first, *last) <= tol
            {
                return Err(degenerate(
                    id,
                    "the spline closes on itself, and a loop of one spline is not supported yet: draw the loop as two splines that share their end points",
                ));
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

/// For every end the end it meets. Errors when an end meets none, or more
/// than one.
fn join_ends(curves: &[SketchCurve], ends: &[End], tol: f64) -> Result<Vec<usize>, KernelError> {
    let mut order: Vec<usize> = (0..ends.len()).collect();
    let x = |i: usize| ends.get(i).map_or(f64::NAN, |e| e.point[0]);
    order.sort_by(|&i, &j| x(i).total_cmp(&x(j)).then(i.cmp(&j)));
    // The first few ends near each end, and how many there are in all.
    let mut near: Vec<(usize, [usize; 2])> = vec![(0, [usize::MAX; 2]); ends.len()];
    let mut record = |i: usize, j: usize| {
        if let Some((count, list)) = near.get_mut(i) {
            if let Some(slot) = list.get_mut(*count) {
                *slot = j;
            }
            *count = count.saturating_add(1);
        }
    };
    for (position, &i) in order.iter().enumerate() {
        let Some(a) = ends.get(i) else { continue };
        for &j in order.iter().skip(position + 1) {
            let Some(b) = ends.get(j) else { continue };
            if b.point[0] - a.point[0] > tol {
                break;
            }
            if distance(a.point, b.point) <= tol {
                record(i, j);
                record(j, i);
            }
        }
    }
    let id_of = |end: usize| {
        ends.get(end)
            .and_then(|e| curves.get(e.curve))
            .map(|c| c.id)
    };
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
fn keeps_clear(a: &Segment, b: &Segment, shared: [bool; 2], tol: f64) -> bool {
    if matches!(a.shape, Shape::Spline(_)) || matches!(b.shape, Shape::Spline(_)) {
        return spline2::keeps_clear(&a.parts(), &b.parts(), shared, tol);
    }
    let mut breaks = Vec::with_capacity(10);
    for bound in offset_bounds(b, tol) {
        crossings(a, &bound, &mut breaks);
    }
    let close = |t: f64| b.distance_to(a.point_at(t), tol) <= tol;
    if matches!(a.shape, Shape::Circle { .. }) {
        // No ends: nothing of a circle may be close. It is sampled at the
        // breaks and halfway between them, around the turn.
        breaks.push(0.0);
        breaks.sort_by(f64::total_cmp);
        let first = breaks.first().copied().unwrap_or(0.0);
        let after = breaks.iter().skip(1).copied().chain([first + 1.0]);
        return !breaks
            .iter()
            .zip(after)
            .any(|(&t, next)| close(t) || close((t + next) / 2.0));
    }
    breaks.push(0.0);
    breaks.push(1.0);
    breaks.sort_by(f64::total_cmp);
    breaks.dedup();
    let mut samples = Vec::with_capacity(breaks.len() * 2);
    for (&t, &next) in breaks.iter().zip(breaks.iter().skip(1)) {
        samples.push(close(t));
        samples.push(close((t + next) / 2.0));
    }
    samples.push(close(1.0));
    let last = samples.len() - 1;
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

/// Errors when two curves touch, cross or overlap away from a shared end.
/// `mate_curve[i][k]` is the curve joined to end `k` of curve `i`.
fn check_clear(
    own: &[Segment],
    mate_curve: &[[Option<usize>; 2]],
    tol: f64,
) -> Result<(), KernelError> {
    // Boxes grown by the tolerance, swept in x: only pairs whose boxes
    // overlap are compared.
    let boxes: Vec<[f64; 4]> = own
        .iter()
        .map(|s| {
            let b = s.bbox();
            [b[0] - tol, b[1] - tol, b[2] + tol, b[3] + tol]
        })
        .collect();
    // A spline can cross itself.
    for segment in own {
        if matches!(segment.shape, Shape::Spline(_)) && !spline2::self_clear(&segment.parts(), tol)
        {
            return Err(KernelError::CrossingCurves {
                curves: [segment.source, segment.source],
            });
        }
    }
    let mut order: Vec<usize> = (0..own.len()).collect();
    let min_x = |i: usize| boxes.get(i).map_or(f64::NAN, |b| b[0]);
    order.sort_by(|&i, &j| min_x(i).total_cmp(&min_x(j)).then(i.cmp(&j)));
    for (position, &i) in order.iter().enumerate() {
        let (Some(a), Some(box_a), Some(mates_a)) = (own.get(i), boxes.get(i), mate_curve.get(i))
        else {
            return Err(internal());
        };
        for &j in order.iter().skip(position + 1) {
            let (Some(b), Some(box_b), Some(mates_b)) =
                (own.get(j), boxes.get(j), mate_curve.get(j))
            else {
                return Err(internal());
            };
            if box_b[0] > box_a[2] {
                break;
            }
            if box_b[1] > box_a[3] || box_a[1] > box_b[3] {
                continue;
            }
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
    let mates = join_ends(&sorted, &ends, tol)?;
    let mate_of = |end: usize| mates.get(end).and_then(|&m| ends.get(m));

    // The representative of a junction: an arc's end before a line's, then
    // the smaller id (`curve` is the index in id order).
    let mut snapped = Vec::with_capacity(ends.len());
    for (i, end) in ends.iter().enumerate() {
        let other = mate_of(i).ok_or_else(internal)?;
        if other.curve == end.curve {
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
    for (curve, first) in sorted.iter().zip(&first_end) {
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
    check_clear(&own, &mate_curve, tol)?;

    // Nesting. No two loops touch, so a loop is inside another when one of
    // its points is.
    let mut nesting = Vec::with_capacity(rings.len());
    for (i, ring) in rings.iter().enumerate() {
        let p = ring.path.origin();
        let (mut depth, mut parent): (usize, Option<(f64, usize)>) = (0, None);
        for (j, around) in rings.iter().enumerate() {
            let b = around.bbox;
            let outside_box = p[0] < b[0] || p[1] < b[1] || p[0] > b[2] || p[1] > b[3];
            if i == j || outside_box || around.path.winding(p) == 0.0 {
                continue;
            }
            depth += 1;
            if parent.is_none_or(|(area, _)| around.area < area) {
                parent = Some((around.area, j));
            }
        }
        nesting.push((depth, parent.map(|(_, j)| j)));
    }
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
