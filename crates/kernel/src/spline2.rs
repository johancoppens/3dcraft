//! Plane B-splines for profiles: validation, exact area and winding, and
//! the test that tells whether a spline keeps clear of another curve and
//! of itself.
//!
//! Everything works on the Bézier pieces of the spline, one per knot span
//! (found by blossoming, exact up to rounding). A Bézier piece lies inside
//! the box of its control points and is cut in two by de Casteljau's
//! construction, which is all the subdivision below needs.
//!
//! **What the clearance test guarantees.** Curves are compared as chords of
//! pieces that are flat within an eighth of the tolerance. So, away from an
//! end the two share: curves that come within three quarters of the
//! tolerance of each other are always reported; curves that stay more than
//! one and a quarter of it apart never are; in between either may happen.
//! A spline is tested against itself the same way, piece against piece,
//! where a piece whose control polygon turns through less than half a turn
//! cannot meet itself.

use std::f64::consts::{PI, TAU};

type P2 = [f64; 2];

/// The highest degree a profile spline may have.
pub const MAX_SPLINE_DEGREE: usize = craft3d_geom::MAX_DEGREE;
/// The most control points a profile spline may have.
pub const MAX_SPLINE_POINTS: usize = 4096;

/// The work one comparison of two curves may take, counted in pairs of
/// pieces looked at. Only curves that run alongside each other within the
/// tolerance over a great length get near it.
const WORK_LIMIT: usize = 5_000_000;

/// How far apart, in the parameter of a curve (0 to 1), two stretches may
/// be and still count as one.
pub(crate) const RUN_GAP: f64 = 1e-9;

/// A control polygon that turns through less than this (radians) bounds a
/// piece that cannot meet itself; half a turn would do, the margin is for
/// rounding.
const SAFE_TURN: f64 = 3.0;

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn lerp(a: P2, b: P2, t: f64) -> P2 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn dot(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn distance(a: P2, b: P2) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// An angle wrapped to `[0, 2π)`.
fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

/// The distance from `p` to the segment from `a` to `b`.
fn point_segment(p: P2, a: P2, b: P2) -> f64 {
    let d = sub(b, a);
    let l2 = dot(d, d);
    let t = if l2 > 0.0 {
        (dot(sub(p, a), d) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance(p, lerp(a, b, t))
}

/// A polynomial B-spline in the plane.
#[derive(Clone, Debug, PartialEq)]
pub struct Spline2 {
    pub degree: usize,
    /// `control.len() + degree + 1` knots, clamped at both ends: the curve
    /// starts at the first control point and ends at the last.
    pub knots: Vec<f64>,
    pub control: Vec<[f64; 2]>,
}

impl Spline2 {
    /// Why the spline cannot be used, if it cannot. Coordinates are checked
    /// by the caller.
    pub(crate) fn fault(&self) -> Option<&'static str> {
        let (p, n) = (self.degree, self.control.len());
        if p == 0 || p > MAX_SPLINE_DEGREE {
            return Some("the degree of the spline is zero or too high");
        }
        if n <= p {
            return Some("the spline has fewer control points than its degree plus one");
        }
        if n > MAX_SPLINE_POINTS {
            return Some("the spline has too many control points");
        }
        if self.knots.len() != n + p + 1 {
            return Some(
                "the spline does not have one knot per control point plus degree plus one",
            );
        }
        if !self.knots.iter().all(|k| k.is_finite()) {
            return Some("a knot of the spline is not finite");
        }
        if self.knots.windows(2).any(|w| matches!(w, [a, b] if a > b)) {
            return Some("the knots of the spline go down");
        }
        let (Some(first), Some(start), Some(end), Some(last)) = (
            self.knots.first(),
            self.knots.get(p),
            self.knots.get(n),
            self.knots.last(),
        ) else {
            return Some("the spline has too few knots");
        };
        if first != start || end != last {
            return Some("the spline is not clamped: it does not end on its end control points");
        }
        if start >= end {
            return Some("the knots of the spline span no length");
        }
        // An inner knot repeated `degree` times is a corner, where a
        // profile cuts the spline in two ([`Spline2::profile_pieces`]);
        // repeated more often it tears the curve apart.
        let inner = self.knots.get(p + 1..n).unwrap_or(&[]);
        let mut run = 0_usize;
        for (i, knot) in inner.iter().enumerate() {
            run = match i.checked_sub(1).and_then(|before| inner.get(before)) {
                Some(before) if before == knot => run + 1,
                _ => 1,
            };
            if run > p {
                return Some(
                    "an inner knot of the spline is repeated more often than its degree, which tears the curve apart there",
                );
            }
        }
        None
    }

    /// The spline cut in two at the knot value `u`, strictly inside its
    /// domain: the same curve as two clamped splines, the first ending and
    /// the second starting on the point at `u`. Exact up to rounding (the
    /// knot is inserted until it is there `degree` times, Boehm's
    /// algorithm). `None` for a spline with a [`Spline2::fault`] and for a
    /// `u` that is not inside.
    fn cut(&self, u: f64) -> Option<(Spline2, Spline2)> {
        let p = self.degree;
        let (start, end) = (*self.knots.get(p)?, *self.knots.get(self.control.len())?);
        if self.fault().is_some() || !(u > start && u < end) {
            return None;
        }
        let (mut knots, mut control) = (self.knots.clone(), self.control.clone());
        for _ in 0..=p {
            let times = knots.iter().filter(|k| **k == u).count();
            if times >= p {
                break;
            }
            // The span the knot goes into: the last knot not above it.
            let k = knots.iter().rposition(|knot| *knot <= u)?;
            let mut next = Vec::with_capacity(control.len() + 1);
            for i in 0..=control.len() {
                next.push(if i + p <= k {
                    *control.get(i)?
                } else if i > k {
                    *control.get(i - 1)?
                } else {
                    let (low, high) = (*knots.get(i)?, *knots.get(i + p)?);
                    let alpha = (u - low) / (high - low);
                    lerp(*control.get(i - 1)?, *control.get(i)?, alpha)
                });
            }
            knots.insert(k + 1, u);
            control = next;
        }
        // The knot is there `degree` times from `first` on: the control
        // point before it is the point of the curve.
        let first = knots.iter().position(|knot| *knot == u)?;
        let mut left_knots = knots.get(..first + p)?.to_vec();
        left_knots.push(u);
        let mut right_knots = vec![u];
        right_knots.extend_from_slice(knots.get(first..)?);
        Some((
            Spline2 {
                degree: p,
                knots: left_knots,
                control: control.get(..first)?.to_vec(),
            },
            Spline2 {
                degree: p,
                knots: right_knots,
                control: control.get(first.checked_sub(1)?..)?.to_vec(),
            },
        ))
    }

    /// The pieces a profile makes of the spline, in order: cut at every
    /// corner (an inner knot repeated `degree` times, which is every inner
    /// knot of a spline of degree 1), and a spline that closes on itself
    /// (`closed`) without a corner in two, at the inner knot nearest the
    /// middle of its domain or at the middle itself. Each piece sweeps a
    /// smooth surface that does not close on itself. One piece, the spline
    /// as it is, when there is nothing to cut.
    pub(crate) fn profile_pieces(&self, closed: bool) -> Vec<Spline2> {
        let (p, n) = (self.degree, self.control.len());
        let whole = || vec![self.clone()];
        if self.fault().is_some() {
            return whole();
        }
        let (Some(&start), Some(&end)) = (self.knots.get(p), self.knots.get(n)) else {
            return whole();
        };
        let inner = self.knots.get(p + 1..n).unwrap_or(&[]);
        let mut cuts: Vec<f64> = Vec::new();
        for &knot in inner {
            let times = inner.iter().filter(|k| **k == knot).count();
            if times >= p && knot > start && knot < end && cuts.last() != Some(&knot) {
                cuts.push(knot);
            }
        }
        if cuts.is_empty() && closed {
            let middle = start + (end - start) / 2.0;
            let nearest = inner
                .iter()
                .copied()
                .filter(|knot| *knot > start && *knot < end)
                .min_by(|a, b| (a - middle).abs().total_cmp(&(b - middle).abs()));
            cuts.push(nearest.unwrap_or(middle));
        }
        let mut pieces = Vec::with_capacity(cuts.len() + 1);
        let mut rest = self.clone();
        for u in cuts {
            let Some((left, right)) = rest.cut(u) else {
                return whole();
            };
            pieces.push(left);
            rest = right;
        }
        pieces.push(rest);
        pieces
    }

    /// The blossom of the span that starts at knot `k`, at the arguments
    /// `(a × (degree − j), b × j)`: Bézier point `j` of that span.
    fn bezier_point(&self, k: usize, j: usize, a: f64, b: f64) -> Option<P2> {
        let p = self.degree;
        let mut d: Vec<P2> = self.control.get(k.checked_sub(p)?..=k)?.to_vec();
        for r in 1..=p {
            // Arguments in order: the first `p − j` are `a`, the rest `b`.
            let u = if r <= p - j { a } else { b };
            for i in (r..=p).rev() {
                let low = *self.knots.get(k + i - p)?;
                let high = *self.knots.get(k + i + 1 - r)?;
                let alpha = (u - low) / (high - low);
                let (before, here) = (*d.get(i - 1)?, *d.get(i)?);
                *d.get_mut(i)? = lerp(before, here, alpha);
            }
        }
        d.last().copied()
    }

    /// The Bézier pieces of the spline in order, each with the stretch of
    /// the curve's parameter (0 at the start, 1 at the end) it covers.
    /// Empty for a spline with a [`Spline2::fault`].
    pub(crate) fn pieces(&self) -> Vec<Part> {
        if self.fault().is_some() {
            return Vec::new();
        }
        let (p, n) = (self.degree, self.control.len());
        let (Some(&start), Some(&end)) = (self.knots.get(p), self.knots.get(n)) else {
            return Vec::new();
        };
        let mut parts = Vec::new();
        for k in p..n {
            let (Some(&a), Some(&b)) = (self.knots.get(k), self.knots.get(k + 1)) else {
                continue;
            };
            if a >= b {
                continue;
            }
            let points: Option<Vec<P2>> = (0..=p).map(|j| self.bezier_point(k, j, a, b)).collect();
            let Some(points) = points else { continue };
            parts.push(Part {
                piece: Piece::Bezier(points),
                t0: (a - start) / (end - start),
                t1: (b - start) / (end - start),
            });
        }
        // The ends are the end control points exactly, whatever rounding
        // the blossoms had.
        if let (Some(first), Some(&point)) = (parts.first_mut(), self.control.first()) {
            first.t0 = 0.0;
            if let Piece::Bezier(points) = &mut first.piece
                && let Some(slot) = points.first_mut()
            {
                *slot = point;
            }
        }
        if let (Some(last), Some(&point)) = (parts.last_mut(), self.control.last()) {
            last.t1 = 1.0;
            if let Piece::Bezier(points) = &mut last.piece
                && let Some(slot) = points.last_mut()
            {
                *slot = point;
            }
        }
        parts
    }

    /// The least and the greatest of `(p − origin) · normal` over the
    /// points `p` of the curve: how far it reaches either way across the
    /// line through `origin` at right angles to `normal`. Found on the
    /// Bézier pieces, cut in two where their control points still allow
    /// more than their ends reach, to about 1e-14 of the size; not finite
    /// for a spline with a [`Spline2::fault`].
    pub(crate) fn extent(&self, origin: P2, normal: P2) -> (f64, f64) {
        let height = |p: &P2| dot(sub(*p, origin), normal);
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut stack: Vec<(Vec<f64>, u32)> = Vec::new();
        for part in self.pieces() {
            if let Piece::Bezier(points) = &part.piece {
                stack.push((points.iter().map(height).collect(), 0));
            }
        }
        if stack.is_empty() {
            return (f64::NAN, f64::NAN);
        }
        let mut work = 0_usize;
        while let Some((heights, depth)) = stack.pop() {
            let (Some(&first), Some(&last)) = (heights.first(), heights.last()) else {
                continue;
            };
            // The ends are points of the curve.
            (low, high) = (low.min(first.min(last)), high.max(first.max(last)));
            let least = heights.iter().copied().fold(f64::INFINITY, f64::min);
            let most = heights.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let slack = 1e-14 * low.abs().max(high.abs());
            work = work.saturating_add(1);
            if (least >= low - slack && most <= high + slack) || depth >= 60 || work > WORK_LIMIT {
                continue;
            }
            // De Casteljau on the heights alone.
            let mut row = heights;
            let (mut left, mut right) = (Vec::new(), Vec::new());
            while let (Some(&a), Some(&b)) = (row.first(), row.last()) {
                left.push(a);
                right.push(b);
                row = row
                    .iter()
                    .zip(row.iter().skip(1))
                    .map(|(a, b)| (a + b) / 2.0)
                    .collect();
            }
            right.reverse();
            stack.push((left, depth + 1));
            stack.push((right, depth + 1));
        }
        (low, high)
    }

    /// `[min x, min y, max x, max y]` of the control points, which contain
    /// the curve.
    pub(crate) fn bbox(&self) -> [f64; 4] {
        bbox_of(&self.control)
    }
}

fn bbox_of(points: &[P2]) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in points {
        b = [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ];
    }
    b
}

/// De Casteljau: the point at `s`, and the control points of the parts
/// before and after it.
fn casteljau(points: &[P2], s: f64) -> (Vec<P2>, Vec<P2>) {
    let mut work = points.to_vec();
    let n = work.len();
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for level in 0..n {
        if let (Some(&first), Some(&last)) = (work.first(), work.get(n - 1 - level)) {
            left.push(first);
            right.push(last);
        }
        for i in 0..n - 1 - level {
            if let (Some(&a), Some(&b)) = (work.get(i), work.get(i + 1))
                && let Some(slot) = work.get_mut(i)
            {
                *slot = lerp(a, b, s);
            }
        }
    }
    right.reverse();
    (left, right)
}

/// The point of a Bézier curve at `s`, and its derivative there.
fn bezier_at(points: &[P2], s: f64) -> (P2, P2) {
    let mut work = points.to_vec();
    let n = work.len();
    let mut tangent = [0.0, 0.0];
    for level in 0..n.saturating_sub(1) {
        if level + 2 == n
            && let (Some(&a), Some(&b)) = (work.first(), work.get(1))
        {
            // Counts are at most the degree, exact as f64.
            #[allow(clippy::cast_precision_loss)]
            let degree = (n - 1) as f64;
            tangent = [(b[0] - a[0]) * degree, (b[1] - a[1]) * degree];
        }
        for i in 0..n - 1 - level {
            if let (Some(&a), Some(&b)) = (work.get(i), work.get(i + 1))
                && let Some(slot) = work.get_mut(i)
            {
                *slot = lerp(a, b, s);
            }
        }
    }
    (work.first().copied().unwrap_or([f64::NAN; 2]), tangent)
}

/// What a [`Part`] runs along.
#[derive(Clone, Debug)]
pub(crate) enum Piece {
    Bezier(Vec<P2>),
    Line(P2, P2),
    /// Counter-clockwise from the angle `begin` through `sweep`.
    Arc {
        center: P2,
        radius: f64,
        begin: f64,
        sweep: f64,
    },
}

/// A stretch of a curve that can be boxed and cut in two: from parameter
/// `t0` to `t1` of the curve it is part of (0 at its start, 1 at its end).
#[derive(Clone, Debug)]
pub(crate) struct Part {
    pub piece: Piece,
    pub t0: f64,
    pub t1: f64,
}

impl Part {
    pub(crate) fn line(a: P2, b: P2) -> Part {
        Part {
            piece: Piece::Line(a, b),
            t0: 0.0,
            t1: 1.0,
        }
    }

    pub(crate) fn arc(center: P2, radius: f64, begin: f64, sweep: f64) -> Part {
        Part {
            piece: Piece::Arc {
                center,
                radius,
                begin,
                sweep,
            },
            t0: 0.0,
            t1: 1.0,
        }
    }

    /// The point at `s` in `[0, 1]` along the part.
    fn point(&self, s: f64) -> P2 {
        match &self.piece {
            Piece::Bezier(points) => bezier_at(points, s).0,
            Piece::Line(a, b) => lerp(*a, *b, s),
            Piece::Arc {
                center,
                radius,
                begin,
                sweep,
            } => {
                let (sin, cos) = (begin + sweep * s).sin_cos();
                [center[0] + radius * cos, center[1] + radius * sin]
            }
        }
    }

    fn ends(&self) -> (P2, P2) {
        match &self.piece {
            Piece::Bezier(points) => (
                points.first().copied().unwrap_or([f64::NAN; 2]),
                points.last().copied().unwrap_or([f64::NAN; 2]),
            ),
            Piece::Line(a, b) => (*a, *b),
            Piece::Arc { .. } => (self.point(0.0), self.point(1.0)),
        }
    }

    fn bbox(&self) -> [f64; 4] {
        match &self.piece {
            Piece::Bezier(points) => bbox_of(points),
            Piece::Line(a, b) => bbox_of(&[*a, *b]),
            Piece::Arc {
                center,
                radius,
                begin,
                sweep,
            } => {
                let (a, b) = self.ends();
                let mut points = vec![a, b];
                for quarter in 0..4_u8 {
                    let angle = f64::from(quarter) * std::f64::consts::FRAC_PI_2;
                    if wrap(angle - begin) <= *sweep {
                        let (sin, cos) = angle.sin_cos();
                        points.push([center[0] + radius * cos, center[1] + radius * sin]);
                    }
                }
                bbox_of(&points)
            }
        }
    }

    /// True when the part stays within `eps` of its chord (and the chord
    /// within `eps` of the part), or is too short in parameter to cut.
    fn is_flat(&self, eps: f64) -> bool {
        if self.t1 - self.t0 < 1e-13 {
            return true;
        }
        match &self.piece {
            Piece::Line(..) => true,
            Piece::Bezier(points) => {
                let (a, b) = self.ends();
                points.iter().all(|p| point_segment(*p, a, b) <= eps)
            }
            Piece::Arc { radius, sweep, .. } => radius * (1.0 - (sweep / 2.0).cos()) <= eps,
        }
    }

    fn halves(&self) -> (Part, Part) {
        let middle = self.t0 + (self.t1 - self.t0) / 2.0;
        let (first, second) = match &self.piece {
            Piece::Bezier(points) => {
                let (left, right) = casteljau(points, 0.5);
                (Piece::Bezier(left), Piece::Bezier(right))
            }
            Piece::Line(a, b) => {
                let m = lerp(*a, *b, 0.5);
                (Piece::Line(*a, m), Piece::Line(m, *b))
            }
            Piece::Arc {
                center,
                radius,
                begin,
                sweep,
            } => {
                let half = |begin: f64| Piece::Arc {
                    center: *center,
                    radius: *radius,
                    begin,
                    sweep: sweep / 2.0,
                };
                (half(*begin), half(begin + sweep / 2.0))
            }
        };
        (
            Part {
                piece: first,
                t0: self.t0,
                t1: middle,
            },
            Part {
                piece: second,
                t0: middle,
                t1: self.t1,
            },
        )
    }

    fn size(&self) -> f64 {
        let b = self.bbox();
        (b[2] - b[0]).hypot(b[3] - b[1])
    }

    /// How far the control polygon of a Bézier piece turns in all, legs
    /// shorter than `tiny` left out; zero for the other kinds.
    fn turning(&self, tiny: f64) -> f64 {
        let Piece::Bezier(points) = &self.piece else {
            return 0.0;
        };
        let mut turned = 0.0;
        let mut before: Option<P2> = None;
        for pair in points.windows(2) {
            let [a, b] = pair else { continue };
            let leg = sub(*b, *a);
            if leg[0].hypot(leg[1]) <= tiny {
                continue;
            }
            if let Some(last) = before {
                turned += cross(last, leg).atan2(dot(last, leg)).abs();
            }
            before = Some(leg);
        }
        turned
    }
}

/// The point of a curve given as parts at parameter `t` of the curve.
pub(crate) fn point_on(parts: &[Part], t: f64) -> Option<P2> {
    let part = parts
        .iter()
        .find(|part| t <= part.t1)
        .or_else(|| parts.last())?;
    let width = part.t1 - part.t0;
    let s = if width > 0.0 {
        ((t - part.t0) / width).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Some(part.point(s))
}

/// The distance between two boxes; zero when they overlap.
fn box_gap(a: [f64; 4], b: [f64; 4]) -> f64 {
    let dx = (b[0] - a[2]).max(a[0] - b[2]).max(0.0);
    let dy = (b[1] - a[3]).max(a[1] - b[3]).max(0.0);
    dx.hypot(dy)
}

/// The stretch `[lo, hi]` of the segment `a` (0 at its start, 1 at its
/// end) that is within `reach` of the segment `b`, if any. The distance to
/// a segment is a convex function along a line, so the stretch is one
/// interval around its minimum.
fn chord_run(a: (P2, P2), b: (P2, P2), reach: f64) -> Option<(f64, f64)> {
    let far = |s: f64| point_segment(lerp(a.0, a.1, s), b.0, b.1);
    // The minimum, by ternary search (the function may be flat around it).
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for _ in 0..64 {
        let (first, second) = (low + (high - low) / 3.0, high - (high - low) / 3.0);
        if far(first) <= far(second) {
            high = second;
        } else {
            low = first;
        }
    }
    let best = [0.0, (low + high) / 2.0, 1.0]
        .into_iter()
        .min_by(|x, y| far(*x).total_cmp(&far(*y)))?;
    if far(best).is_nan() || far(best) > reach {
        return None;
    }
    // From the minimum outwards the distance only grows: bisect for where
    // it passes `reach`.
    let edge = |mut inside: f64, mut outside: f64| {
        if far(outside) <= reach {
            return outside;
        }
        for _ in 0..60 {
            let middle = (inside + outside) / 2.0;
            if far(middle) <= reach {
                inside = middle;
            } else {
                outside = middle;
            }
        }
        inside
    };
    Some((edge(best, 0.0), edge(best, 1.0)))
}

/// The stretches of curve `a`, in its parameter, that are within `tol` of
/// curve `b` (see the module documentation for how sharp that is), merged
/// and ascending. `None` when the comparison takes more work than a pair
/// of curves that keep clear of each other can need.
pub(crate) fn close_runs(a: &[Part], b: &[Part], tol: f64) -> Option<Vec<(f64, f64)>> {
    let eps = tol / 8.0;
    let boxes_b: Vec<[f64; 4]> = b.iter().map(Part::bbox).collect();
    let mut stack: Vec<(Part, Part)> = Vec::new();
    let mut work = 0_usize;
    for part in a {
        let own = part.bbox();
        for (other, other_box) in b.iter().zip(&boxes_b) {
            work = work.saturating_add(1);
            if box_gap(own, *other_box) <= tol {
                stack.push((part.clone(), other.clone()));
            }
        }
    }
    let mut runs: Vec<(f64, f64)> = Vec::new();
    while let Some((pa, pb)) = stack.pop() {
        work = work.saturating_add(1);
        if work > WORK_LIMIT {
            return None;
        }
        // NaN (a broken curve) is not "apart": it goes on to the chords,
        // which then report nothing.
        if box_gap(pa.bbox(), pb.bbox()) > tol {
            continue;
        }
        let (flat_a, flat_b) = (pa.is_flat(eps), pb.is_flat(eps));
        if flat_a && flat_b {
            if let Some((lo, hi)) = chord_run(pa.ends(), pb.ends(), tol) {
                let width = pa.t1 - pa.t0;
                let at = |s: f64| {
                    if s >= 1.0 { pa.t1 } else { pa.t0 + width * s }
                };
                runs.push((at(lo), at(hi)));
            }
            continue;
        }
        if !flat_a && (flat_b || pa.size() >= pb.size()) {
            let (first, second) = pa.halves();
            stack.push((first, pb.clone()));
            stack.push((second, pb));
        } else {
            let (first, second) = pb.halves();
            stack.push((pa.clone(), first));
            stack.push((pa, second));
        }
    }
    runs.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (lo, hi) in runs {
        match merged.last_mut() {
            Some(last) if lo <= last.1 + RUN_GAP => last.1 = last.1.max(hi),
            _ => merged.push((lo, hi)),
        }
    }
    Some(merged)
}

/// Whether curve `a` keeps clear of curve `b`, two different curves of a
/// sketch; `shared[k]` says that end `k` of `a` is joined to an end of
/// `b`. A stretch of `a` near `b` is allowed only when it reaches a shared
/// end and does not reach the other end of `a`.
pub(crate) fn keeps_clear(a: &[Part], b: &[Part], shared: [bool; 2], tol: f64) -> bool {
    let Some(runs) = close_runs(a, b, tol) else {
        return false;
    };
    runs.iter().all(|&(lo, hi)| {
        let (at_start, at_end) = (lo <= RUN_GAP, hi >= 1.0 - RUN_GAP);
        (at_start && shared[0] && !at_end) || (at_end && shared[1] && !at_start)
    })
}

/// Whether the stretch `left` of a curve keeps clear of the stretch
/// `right` that follows it: the two may only be near each other where one
/// goes over into the other, within twice the tolerance of that point.
fn follows_clear(left: &[Part], right: &[Part], tol: f64) -> bool {
    let (Some(last), Some(first)) = (left.last(), right.first()) else {
        return true;
    };
    let joint = last.ends().1;
    let (joint_t, after_t) = (last.t1, first.t0);
    let near_joint = |parts: &[Part], t: f64| {
        point_on(parts, t).is_some_and(|p| distance(p, joint) <= 2.0 * tol)
    };
    let (Some(on_left), Some(on_right)) =
        (close_runs(left, right, tol), close_runs(right, left, tol))
    else {
        return false;
    };
    on_left
        .iter()
        .all(|&(lo, hi)| hi >= joint_t - RUN_GAP && near_joint(left, lo))
        && on_right
            .iter()
            .all(|&(lo, hi)| lo <= after_t + RUN_GAP && near_joint(right, hi))
}

/// Whether one piece keeps clear of itself.
fn piece_clear(part: &Part, tol: f64) -> bool {
    let mut stack = vec![part.clone()];
    let mut work = 0_usize;
    while let Some(part) = stack.pop() {
        work = work.saturating_add(1);
        if work > WORK_LIMIT {
            return false;
        }
        if part.turning(tol * 1e-6) < SAFE_TURN {
            continue;
        }
        // Still turning back on itself at a size below the tolerance: a
        // cusp, or a loop too small to see.
        if part.size() <= tol / 4.0 || part.t1 - part.t0 < 1e-13 {
            return false;
        }
        let (first, second) = part.halves();
        if !follows_clear(
            std::slice::from_ref(&first),
            std::slice::from_ref(&second),
            tol,
        ) {
            return false;
        }
        stack.push(first);
        stack.push(second);
    }
    true
}

/// Whether a curve, given as its pieces in order, keeps clear of itself:
/// it neither crosses nor touches itself, and has no cusp.
pub(crate) fn self_clear(parts: &[Part], tol: f64) -> bool {
    // Halves of halves: each stretch against the one that follows it.
    let mut stack: Vec<&[Part]> = vec![parts];
    while let Some(parts) = stack.pop() {
        match parts {
            [] => {}
            [one] => {
                if !piece_clear(one, tol) {
                    return false;
                }
            }
            _ => {
                let (left, right) = parts.split_at(parts.len() / 2);
                if !follows_clear(left, right, tol) {
                    return false;
                }
                stack.push(left);
                stack.push(right);
            }
        }
    }
    true
}

/// The distance from `p` to the curve given as parts.
pub(crate) fn distance_to(parts: &[Part], p: P2, tol: f64) -> f64 {
    let eps = tol / 8.0;
    let mut best = f64::INFINITY;
    let mut stack: Vec<Part> = parts.to_vec();
    let mut work = 0_usize;
    while let Some(part) = stack.pop() {
        work = work.saturating_add(1);
        let bound = box_gap(part.bbox(), [p[0], p[1], p[0], p[1]]);
        if bound >= best {
            continue;
        }
        if part.is_flat(eps) || work > WORK_LIMIT {
            let (a, b) = part.ends();
            best = best.min(point_segment(p, a, b));
            continue;
        }
        let (first, second) = part.halves();
        stack.push(first);
        stack.push(second);
    }
    best
}

/// The angle the direction from `p` to a point running along the curve
/// turns through, from the start of the curve to its end. `p` must not be
/// on the curve.
///
/// Exact: a piece whose box does not hold `p` cannot pass around it, so it
/// turns the direction as much as its chord does; other pieces are cut in
/// two until that is so.
pub(crate) fn turn_about(parts: &[Part], p: P2) -> f64 {
    let mut total = 0.0;
    let mut stack: Vec<(Part, u32)> = parts.iter().map(|part| (part.clone(), 0)).collect();
    while let Some((part, depth)) = stack.pop() {
        let b = part.bbox();
        let outside = p[0] < b[0] || p[1] < b[1] || p[0] > b[2] || p[1] > b[3];
        // 200 halvings leave nothing of any length to cut.
        if outside || matches!(part.piece, Piece::Line(..)) || depth >= 200 {
            let (from, to) = part.ends();
            let (a, b) = (sub(from, p), sub(to, p));
            total += cross(a, b).atan2(dot(a, b));
            continue;
        }
        let (first, second) = part.halves();
        stack.push((first, depth + 1));
        stack.push((second, depth + 1));
    }
    total
}

/// Gauss–Legendre nodes and weights on `[0, 1]`: exact for polynomials up
/// to degree `2 count − 1`.
fn gauss_legendre(count: usize) -> Vec<(f64, f64)> {
    let mut rule = Vec::with_capacity(count);
    // Counts are at most a few dozen, exact as f64.
    #[allow(clippy::cast_precision_loss)]
    let n = count as f64;
    for i in 0..count {
        #[allow(clippy::cast_precision_loss)]
        let mut x = (PI * (i as f64 + 0.75) / (n + 0.5)).cos();
        let mut slope = 1.0;
        for _ in 0..100 {
            // Legendre's recurrence up to P_count, and its derivative.
            let (mut before, mut here) = (1.0, x);
            for k in 2..=count {
                #[allow(clippy::cast_precision_loss)]
                let k = k as f64;
                let next = ((2.0 * k - 1.0) * x * here - (k - 1.0) * before) / k;
                (before, here) = (here, next);
            }
            slope = n * (x * here - before) / (x * x - 1.0);
            let step = here / slope;
            x -= step;
            if step.abs() <= 1e-16 {
                break;
            }
        }
        let weight = 2.0 / ((1.0 - x * x) * slope * slope);
        rule.push(((x + 1.0) / 2.0, weight / 2.0));
    }
    rule
}

/// The spline's share of a loop's area and first moments, as
/// `profile::Segment::moments` defines them: over the region swept by the
/// line from `origin` to a point running along the curve from its start to
/// its end. `(area, moment x, moment y)`, moments about `origin`.
///
/// Exact up to rounding: the integrands are polynomials of degree
/// `3 degree − 1` on each Bézier piece, and the Gauss–Legendre rule used
/// is exact for them.
pub(crate) fn moments(spline: &Spline2, origin: P2) -> (f64, f64, f64) {
    let rule = gauss_legendre(3 * spline.degree / 2 + 1);
    let mut sum = (0.0, 0.0, 0.0);
    for part in spline.pieces() {
        let Piece::Bezier(points) = &part.piece else {
            continue;
        };
        for &(s, weight) in &rule {
            let (point, tangent) = bezier_at(points, s);
            let at = sub(point, origin);
            // Twice the area swept per unit of the parameter.
            let swept = cross(at, tangent) * weight;
            sum.0 += swept / 2.0;
            sum.1 += at[0] * swept / 3.0;
            sum.2 += at[1] * swept / 3.0;
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clamped(degree: usize, control: &[P2]) -> Spline2 {
        let n = control.len();
        let spans = n - degree;
        let mut knots = vec![0.0; degree + 1];
        knots.extend((1..spans).map(|i| i as f64 / spans as f64));
        knots.extend(vec![1.0; degree + 1]);
        Spline2 {
            degree,
            knots,
            control: control.to_vec(),
        }
    }

    /// The point at `t`, by de Boor's algorithm on the knots: independent
    /// of the Bézier pieces.
    fn de_boor(spline: &Spline2, t: f64) -> P2 {
        let (p, n) = (spline.degree, spline.control.len());
        let u = spline.knots[p] + t * (spline.knots[n] - spline.knots[p]);
        let k = (p..n)
            .rev()
            .find(|&k| spline.knots[k] <= u)
            .unwrap_or(p)
            .min(n - 1);
        let mut d: Vec<P2> = spline.control[k - p..=k].to_vec();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = k - p + j;
                let alpha = (u - spline.knots[i]) / (spline.knots[i + p + 1 - r] - spline.knots[i]);
                d[j] = lerp(d[j - 1], d[j], alpha);
            }
        }
        d[p]
    }

    const WAVE: [P2; 7] = [
        [0.0, 0.0],
        [1.0, 2.0],
        [2.5, -1.0],
        [4.0, 3.0],
        [5.0, 0.5],
        [6.5, 1.0],
        [8.0, 0.0],
    ];

    #[test]
    fn the_bezier_pieces_are_the_spline() {
        for degree in 2..=6 {
            let spline = clamped(degree, &WAVE);
            let parts = spline.pieces();
            assert_eq!(parts.len(), WAVE.len() - degree, "degree {degree}");
            assert_eq!(parts[0].t0, 0.0);
            assert_eq!(parts.last().unwrap().t1, 1.0);
            assert_eq!(parts[0].ends().0, WAVE[0]);
            assert_eq!(parts.last().unwrap().ends().1, WAVE[6]);
            for pair in parts.windows(2) {
                assert_eq!(pair[0].t1, pair[1].t0);
                assert!(distance(pair[0].ends().1, pair[1].ends().0) < 1e-14);
            }
            for i in 0..=400 {
                let t = f64::from(i) / 400.0;
                let (ours, theirs) = (point_on(&parts, t).unwrap(), de_boor(&spline, t));
                assert!(
                    distance(ours, theirs) < 1e-13,
                    "degree {degree} t {t}: {ours:?} {theirs:?}"
                );
            }
        }
    }

    #[test]
    fn the_pieces_of_a_spline_are_the_spline() {
        // A cubic with a corner in it, a polyline, and smooth splines cut
        // in the middle because they close.
        let mut cornered = clamped(3, &WAVE);
        cornered.knots = vec![0.0, 0.0, 0.0, 0.0, 0.4, 0.4, 0.4, 1.0, 1.0, 1.0, 1.0];
        let mut polyline = clamped(1, &WAVE);
        polyline.knots = vec![0.0, 0.0, 0.1, 0.3, 0.5, 0.6, 0.8, 1.0, 1.0];
        let cases = [
            (cornered, false, 2),
            (polyline, false, 6),
            (clamped(3, &WAVE), false, 1),
            (clamped(3, &WAVE), true, 2),
            (clamped(2, &WAVE), true, 2),
            (clamped(5, &WAVE), true, 2),
            (clamped(6, &WAVE), true, 2),
            (clamped(3, &WAVE[..4]), true, 2),
        ];
        for (number, (spline, closed, count)) in cases.into_iter().enumerate() {
            let pieces = spline.profile_pieces(closed);
            assert_eq!(pieces.len(), count, "case {number}");
            let (p, n) = (spline.degree, spline.control.len());
            let (start, end) = (spline.knots[p], spline.knots[n]);
            assert_eq!(pieces[0].control[0], spline.control[0]);
            assert_eq!(pieces.last().unwrap().control.last(), spline.control.last());
            for pair in pieces.windows(2) {
                assert_eq!(pair[0].control.last(), pair[1].control.first());
            }
            for piece in &pieces {
                assert_eq!(piece.fault(), None, "case {number}");
                assert_eq!(piece.profile_pieces(false).len(), 1, "case {number}");
                let (q, m) = (piece.degree, piece.control.len());
                let (from, to) = (piece.knots[q], piece.knots[m]);
                for i in 0..=200 {
                    let s = f64::from(i) / 200.0;
                    let u = from + (to - from) * s;
                    // Just inside at a corner, where the two sides differ
                    // only in which piece they belong to.
                    let ours = de_boor(piece, s);
                    let theirs = de_boor(&spline, (u - start) / (end - start));
                    assert!(
                        distance(ours, theirs) < 1e-13,
                        "case {number} at {u}: {ours:?} {theirs:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn halves_are_the_same_curve() {
        let part = clamped(3, &WAVE[..4]).pieces().remove(0);
        let (first, second) = part.halves();
        for i in 0..=50 {
            let s = f64::from(i) / 50.0;
            assert!(distance(first.point(s), part.point(s / 2.0)) < 1e-14);
            assert!(distance(second.point(s), part.point(0.5 + s / 2.0)) < 1e-14);
        }
        let arc = Part::arc([1.0, 2.0], 3.0, 0.5, 4.0);
        let (first, second) = arc.halves();
        assert!(distance(first.point(1.0), arc.point(0.5)) < 1e-14);
        assert!(distance(second.point(1.0), arc.point(1.0)) < 1e-14);
        let b = arc.bbox();
        assert!(
            (b[0] - -2.0).abs() < 1e-14 && (b[3] - 5.0).abs() < 1e-14,
            "{b:?}"
        );
    }

    #[test]
    fn gauss_legendre_is_exact_to_its_degree() {
        for count in 1..=40 {
            let rule = gauss_legendre(count);
            assert_eq!(rule.len(), count);
            for power in [0, 1, 2 * count - 1] {
                let sum: f64 = rule.iter().map(|(x, w)| w * x.powi(power as i32)).sum();
                let exact = 1.0 / (power as f64 + 1.0);
                assert!(
                    (sum - exact).abs() < 1e-13,
                    "{count} nodes, x^{power}: {sum} {exact}"
                );
            }
        }
    }

    #[test]
    fn the_moments_of_a_spline_are_those_of_a_fine_polygon() {
        for degree in 2..=6 {
            let spline = clamped(degree, &WAVE);
            let origin = [0.3, -0.7];
            let (area, mx, my) = moments(&spline, origin);
            // The same integrals over a polygon of 200 000 chords.
            let steps = 200_000;
            let mut sum = (0.0, 0.0, 0.0);
            let mut before = sub(de_boor(&spline, 0.0), origin);
            for i in 1..=steps {
                let here = sub(de_boor(&spline, f64::from(i) / f64::from(steps)), origin);
                let w = cross(before, here);
                sum.0 += w / 2.0;
                sum.1 += (before[0] + here[0]) * w / 6.0;
                sum.2 += (before[1] + here[1]) * w / 6.0;
                before = here;
            }
            for (ours, theirs) in [(area, sum.0), (mx, sum.1), (my, sum.2)] {
                assert!(
                    (ours - theirs).abs() < 1e-8 * theirs.abs().max(1.0),
                    "degree {degree}: {ours} {theirs}"
                );
            }
        }
    }

    #[test]
    fn a_spline_that_crosses_itself_or_has_a_cusp_is_not_clear() {
        let tol = 1e-6;
        for scale in [1e-3, 1.0, 1e3] {
            let tol = tol * scale;
            let at = |points: &[P2]| -> Vec<P2> {
                points
                    .iter()
                    .map(|p| [p[0] * scale, p[1] * scale])
                    .collect()
            };
            // A wave, an arch and a tight but open hook are clear.
            for good in [
                &WAVE[..],
                &[[0.0, 0.0], [0.0, 4.0], [4.0, 4.0], [4.0, 0.0]],
                &[[0.0, 0.0], [5.0, 0.0], [5.0, 1.0], [0.0, 1.0]],
                &[[0.0, 0.0], [3.0, 0.0], [3.0, 2.0], [1.0, 2.0], [1.0, 1.0]],
            ] {
                for degree in 2..=3 {
                    let parts = clamped(degree, &at(good)).pieces();
                    assert!(!parts.is_empty());
                    assert!(self_clear(&parts, tol), "{good:?} degree {degree} x{scale}");
                }
            }
            // A loop (the curve crosses itself), in one span and in many.
            let fish = [[0.0, 0.0], [4.0, 3.0], [4.0, -3.0], [0.0, 0.5]];
            assert!(!self_clear(&clamped(3, &at(&fish)).pieces(), tol));
            let knot = [
                [0.0, 0.0],
                [2.0, 0.0],
                [4.0, 0.0],
                [4.0, 2.0],
                [2.0, 2.0],
                [2.0, -1.0],
                [2.0, -2.0],
            ];
            for degree in 2..=3 {
                assert!(
                    !self_clear(&clamped(degree, &at(&knot)).pieces(), tol),
                    "degree {degree} x{scale}"
                );
            }
            // A cusp: the curve stops and turns back.
            let cusp = [[0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0]];
            assert!(!self_clear(&clamped(3, &at(&cusp)).pieces(), tol));
            // Back along itself.
            let back = [[0.0, 0.0], [2.0, 0.0], [1.0, 0.0]];
            assert!(!self_clear(&clamped(2, &at(&back)).pieces(), tol));
            // Coming back to end across its own first stretch.
            let touch = [[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [2.0, 2.0], [2.0, 0.0]];
            assert!(!self_clear(&clamped(2, &at(&touch)).pieces(), tol));
            // Touching itself without crossing: two stretches of one
            // curve, the last ending on the first.
            let first = Part {
                piece: Piece::Line(at(&[[0.0, 0.0]])[0], at(&[[4.0, 0.0]])[0]),
                t0: 0.0,
                t1: 0.25,
            };
            let rest = clamped(2, &at(&[[4.0, 0.0], [4.0, 3.0], [2.0, 3.0], [2.0, 0.0]])).pieces();
            let mut whole = vec![first];
            whole.extend(rest.into_iter().map(|part| Part {
                t0: 0.25 + part.t0 * 0.75,
                t1: 0.25 + part.t1 * 0.75,
                piece: part.piece,
            }));
            assert!(!self_clear(&whole, tol));
            // Ending just short of it is clear.
            if let Some(Part {
                piece: Piece::Bezier(points),
                ..
            }) = whole.last_mut()
            {
                points[2][1] = 3.0 * tol;
            }
            assert!(self_clear(&whole, tol));
        }
    }

    #[test]
    fn close_stretches_are_found_where_curves_meet() {
        let tol = 1e-6;
        let arch = clamped(3, &[[0.0, 0.0], [0.0, 4.0], [4.0, 4.0], [4.0, 0.0]]).pieces();
        // A line through both ends: near at both ends only.
        let base = [Part::line([0.0, 0.0], [4.0, 0.0])];
        let runs = close_runs(&arch, &base, tol).unwrap();
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert!(runs[0].0 == 0.0 && runs[0].1 < 1e-5);
        assert!(runs[1].1 == 1.0 && runs[1].0 > 1.0 - 1e-5);
        assert!(keeps_clear(&arch, &base, [true, true], tol));
        assert!(keeps_clear(&base, &arch, [true, true], tol));
        assert!(!keeps_clear(&arch, &base, [true, false], tol));
        // A line across the arch: near in the middle, twice.
        let across = [Part::line([-1.0, 2.0], [5.0, 2.0])];
        let runs = close_runs(&arch, &across, tol).unwrap();
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert!(!keeps_clear(&arch, &across, [false, false], tol));
        assert!(!keeps_clear(&across, &arch, [false, false], tol));
        // The top of the arch is at y = 3: a line just above is clear, one
        // just touching is not.
        let above = [Part::line([-1.0, 3.0 + 2e-6], [5.0, 3.0 + 2e-6])];
        assert!(keeps_clear(&arch, &above, [false, false], tol));
        let touching = [Part::line([-1.0, 3.0 + 5e-7], [5.0, 3.0 + 5e-7])];
        assert!(!keeps_clear(&arch, &touching, [false, false], tol));
        // A circle about the top of the arch crosses it; a far one does not.
        let circle = [Part::arc([2.0, 3.0], 0.5, 0.0, TAU)];
        assert!(!keeps_clear(&circle, &arch, [false, false], tol));
        let far = [Part::arc([2.0, 1.0], 0.5, 0.0, TAU)];
        assert!(keeps_clear(&far, &arch, [false, false], tol));
        assert!(keeps_clear(&arch, &far, [false, false], tol));
        // The same curve twice lies on itself all along.
        assert!(!keeps_clear(&arch, &arch, [true, true], tol));
    }

    #[test]
    fn winding_and_distance_follow_the_curve_not_its_chord() {
        let arch = clamped(3, &[[0.0, 0.0], [0.0, 4.0], [4.0, 4.0], [4.0, 0.0]]).pieces();
        // Closed by the chord from (4,0) back to (0,0), the arch goes
        // clockwise around points under it.
        let chord = |p: P2| {
            let (a, b) = (sub([4.0, 0.0], p), sub([0.0, 0.0], p));
            cross(a, b).atan2(dot(a, b))
        };
        for (p, inside) in [
            ([2.0, 1.0], true),
            ([2.0, 2.99], true),
            ([2.0, 3.01], false),
            ([0.1, 0.5], true),
            ([-0.1, 0.5], false),
            ([2.0, -1.0], false),
            ([3.9, 3.5], false),
        ] {
            let turns = (turn_about(&arch, p) + chord(p)) / TAU;
            let expected = if inside { -1.0 } else { 0.0 };
            assert!((turns - expected).abs() < 1e-9, "{p:?}: {turns}");
        }
        assert!((distance_to(&arch, [2.0, 5.0], 1e-6) - 2.0).abs() < 1e-6);
        assert!((distance_to(&arch, [2.0, 2.5], 1e-6) - 0.5).abs() < 1e-6);
        assert!(distance_to(&arch, [0.0, 0.0], 1e-6) < 1e-12);
    }

    #[test]
    fn a_spline_with_a_fault_says_which() {
        let good = clamped(3, &WAVE);
        assert_eq!(good.fault(), None);
        type Damage = fn(&mut Spline2);
        let broken: [Damage; 9] = [
            |s| s.degree = 0,
            |s| s.degree = MAX_SPLINE_DEGREE + 1,
            |s| s.control.truncate(3),
            |s| s.knots.truncate(5),
            |s| s.knots[5] = f64::NAN,
            |s| s.knots.swap(4, 5),
            |s| s.knots[0] = -1.0,
            |s| s.knots = vec![0.0; 11],
            // A knot four times in a cubic tears it apart.
            |s| {
                s.control.push([9.0, 1.0]);
                s.knots = vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0];
            },
        ];
        for (i, damage) in broken.into_iter().enumerate() {
            let mut spline = good.clone();
            damage(&mut spline);
            assert!(spline.fault().is_some(), "damage {i}");
            assert!(spline.pieces().is_empty(), "damage {i}");
        }
        // A triple inner knot is a corner in a cubic, and every inner knot
        // of a polyline is one: no fault, a profile cuts the spline there.
        let mut corner = good.clone();
        corner.knots = vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0];
        assert_eq!(corner.fault(), None);
        let mut polyline = good.clone();
        polyline.degree = 1;
        polyline.knots = vec![0.0, 0.0, 0.2, 0.4, 0.5, 0.6, 0.8, 1.0, 1.0];
        assert_eq!(polyline.fault(), None);
        assert_eq!(polyline.profile_pieces(false).len(), 6);
        // A double inner knot leaves a cubic smooth, and a spline of
        // degree 1 on two points is a line.
        let mut double = good.clone();
        double.knots = vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.75, 1.0, 1.0, 1.0, 1.0];
        assert_eq!(double.fault(), None);
        assert_eq!(clamped(1, &WAVE[..2]).fault(), None);
    }
}
