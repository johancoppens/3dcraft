//! B-splines in the plane, polynomial or rational, and what the writers
//! need of them: the piece between two parameters, the same curve run the
//! other way, and its Bézier pieces by knot insertion (*The NURBS Book*,
//! Piegl and Tiller, §5.2 and §5.3).

use crate::{DrawingError, Point};

/// The highest degree a spline may have.
pub const MAX_SPLINE_DEGREE: usize = 15;

/// The most control points a spline may have, also after it is split in
/// Bézier pieces.
pub const MAX_SPLINE_CONTROL_POINTS: usize = 4096;

/// A point with its weight: `[w x, w y, w]`.
type H = [f64; 3];

fn lerp(a: H, b: H, t: f64) -> H {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

const BROKEN: DrawingError = DrawingError::InvalidSpline("its arrays do not fit together");

fn knot(knots: &[f64], i: usize) -> Result<f64, DrawingError> {
    knots.get(i).copied().ok_or(BROKEN)
}

fn point(points: &[H], i: usize) -> Result<H, DrawingError> {
    points.get(i).copied().ok_or(BROKEN)
}

/// A B-spline curve of `degree` on `control` points, clamped: it starts at
/// its first control point and ends at its last.
#[derive(Clone, Debug, PartialEq)]
pub struct Spline {
    degree: usize,
    knots: Vec<f64>,
    control: Vec<Point>,
    /// One per control point when the curve is rational.
    weights: Option<Vec<f64>>,
}

/// One Bézier piece of a spline: `degree + 1` control points, with their
/// weights when the spline is rational.
#[derive(Clone, Debug, PartialEq)]
pub struct Bezier {
    pub control: Vec<Point>,
    pub weights: Option<Vec<f64>>,
}

impl Spline {
    /// Checks the curve and, when its knots are not clamped, cuts it to
    /// its domain so that they are.
    ///
    /// `knots` has `control.len() + degree + 1` values that never
    /// decrease; no value inside the domain may occur more than `degree`
    /// times (the curve would fall apart there). `weights`, when given,
    /// are positive, one per control point.
    pub fn new(
        degree: usize,
        knots: Vec<f64>,
        control: Vec<Point>,
        weights: Option<Vec<f64>>,
    ) -> Result<Self, DrawingError> {
        let invalid = |reason| Err(DrawingError::InvalidSpline(reason));
        if degree == 0 || degree > MAX_SPLINE_DEGREE {
            return invalid("the degree must be 1 to 15");
        }
        let n = control.len();
        if n <= degree {
            return invalid("it needs more control points than its degree");
        }
        if n > MAX_SPLINE_CONTROL_POINTS {
            return invalid("it has more than 4096 control points");
        }
        if n.checked_add(degree + 1) != Some(knots.len()) {
            return invalid("it needs as many knots as control points plus the degree plus one");
        }
        if !(knots.iter().all(|k| k.is_finite())
            && control.iter().all(|p| p[0].is_finite() && p[1].is_finite()))
        {
            return invalid("a knot or a control point is not finite");
        }
        if let Some(weights) = &weights {
            if weights.len() != n {
                return invalid("it needs one weight per control point");
            }
            if !weights.iter().all(|w| w.is_finite() && *w > 0.0) {
                return invalid("a weight is not positive and finite");
            }
        }
        if knots.windows(2).any(|w| matches!(w, [a, b] if a > b)) {
            return invalid("its knots decrease");
        }
        let (start, end) = (knot(&knots, degree)?, knot(&knots, n)?);
        if end <= start {
            return invalid("its knots leave it no length");
        }
        // Inside the domain a knot may occur `degree` times at most.
        let mut run = 0_usize;
        let mut previous = f64::NAN;
        for &k in &knots {
            run = if k == previous { run + 1 } else { 1 };
            previous = k;
            if run > degree && k > start && k < end {
                return invalid("a knot inside it occurs more often than its degree");
            }
        }
        // All weights 1 is a polynomial curve.
        let weights = weights.filter(|w| w.iter().any(|w| *w != 1.0));
        let spline = Self {
            degree,
            knots,
            control,
            weights,
        };
        if spline.is_clamped() {
            Ok(spline)
        } else {
            spline.between(start, end)
        }
    }

    fn is_clamped(&self) -> bool {
        let p = self.degree;
        let n = self.control.len();
        let first = self.knots.first();
        let last = self.knots.last();
        self.knots.iter().take(p + 1).all(|k| Some(k) == first)
            && self.knots.iter().skip(n).all(|k| Some(k) == last)
    }

    pub fn degree(&self) -> usize {
        self.degree
    }

    /// The same curve moved by `d`: every control point moved, knots and
    /// weights as they are.
    pub fn translated(&self, d: Point) -> Self {
        Self {
            control: self
                .control
                .iter()
                .map(|p| [p[0] + d[0], p[1] + d[1]])
                .collect(),
            ..self.clone()
        }
    }

    pub fn knots(&self) -> &[f64] {
        &self.knots
    }

    pub fn control_points(&self) -> &[Point] {
        &self.control
    }

    /// `None` for a polynomial curve.
    pub fn weights(&self) -> Option<&[f64]> {
        self.weights.as_deref()
    }

    pub fn is_rational(&self) -> bool {
        self.weights.is_some()
    }

    /// The parameters it starts and ends at.
    pub fn domain(&self) -> (f64, f64) {
        (
            self.knots.first().copied().unwrap_or(0.0),
            self.knots.last().copied().unwrap_or(0.0),
        )
    }

    /// Where it starts: its first control point.
    pub fn start(&self) -> Point {
        self.control.first().copied().unwrap_or([0.0, 0.0])
    }

    /// Where it ends: its last control point.
    pub fn end(&self) -> Point {
        self.control.last().copied().unwrap_or([0.0, 0.0])
    }

    fn homogeneous(&self) -> Vec<H> {
        self.control
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let w = self
                    .weights
                    .as_ref()
                    .and_then(|w| w.get(i))
                    .copied()
                    .unwrap_or(1.0);
                [p[0] * w, p[1] * w, w]
            })
            .collect()
    }

    fn from_homogeneous(
        degree: usize,
        knots: Vec<f64>,
        points: &[H],
        rational: bool,
    ) -> Result<Self, DrawingError> {
        if points.iter().any(|h| h[2] <= 0.0 || !h[2].is_finite()) {
            return Err(DrawingError::InvalidSpline("a weight is not positive"));
        }
        let control = points.iter().map(|h| [h[0] / h[2], h[1] / h[2]]).collect();
        let weights = rational.then(|| points.iter().map(|h| h[2]).collect());
        Self::new(degree, knots, control, weights)
    }

    /// The same curve with its end points at exactly `start` and `end`,
    /// for a chain whose neighbours end there. The points may differ from
    /// the curve's own ends by rounding only; nothing else is moved.
    pub fn with_ends(mut self, start: Point, end: Point) -> Self {
        if let Some(first) = self.control.first_mut() {
            *first = start;
        }
        if let Some(last) = self.control.last_mut() {
            *last = end;
        }
        self
    }

    /// The same curve from its end to its start.
    pub fn reversed(&self) -> Self {
        let (a, b) = self.domain();
        Self {
            degree: self.degree,
            knots: self.knots.iter().rev().map(|k| a + (b - k)).collect(),
            control: self.control.iter().rev().copied().collect(),
            weights: self
                .weights
                .as_ref()
                .map(|w| w.iter().rev().copied().collect()),
        }
    }

    /// The piece from parameter `from` to parameter `to` (`from < to`,
    /// both in the domain; a parameter within rounding of a knot counts as
    /// that knot).
    pub fn between(&self, from: f64, to: f64) -> Result<Self, DrawingError> {
        let p = self.degree;
        let n = self.control.len();
        let (lo, hi) = (knot(&self.knots, p)?, knot(&self.knots, n)?);
        let slack = 1e-12 * (hi - lo);
        let snap = |t: f64| {
            self.knots
                .iter()
                .copied()
                .find(|k| (k - t).abs() <= slack)
                .unwrap_or(t)
        };
        if !(from.is_finite() && to.is_finite()) {
            return Err(DrawingError::InvalidSpline("a parameter is not finite"));
        }
        let (from, to) = (snap(from).clamp(lo, hi), snap(to).clamp(lo, hi));
        if to <= from {
            return Err(DrawingError::InvalidSpline(
                "a piece of it needs two parameters in its domain, the first below the second",
            ));
        }
        let mut knots = self.knots.clone();
        let mut points = self.homogeneous();
        saturate(p, &mut knots, &mut points, from, false)?;
        saturate(p, &mut knots, &mut points, to, true)?;
        // The curve is at `from` in the control point `p` before the last
        // knot that is `from`, and at `to` in the one before the first
        // knot that is `to`.
        let ka = knots
            .iter()
            .rposition(|k| *k <= from)
            .ok_or(BROKEN)?
            .min(points.len().saturating_sub(1));
        let kb = knots.iter().position(|k| *k >= to).ok_or(BROKEN)?;
        let first = ka.checked_sub(p).ok_or(BROKEN)?;
        let piece = points.get(first..kb).ok_or(BROKEN)?;
        let inner = knots.get(first + 1..kb + p).ok_or(BROKEN)?;
        let mut cut = Vec::with_capacity(inner.len() + 2);
        cut.push(from);
        cut.extend_from_slice(inner);
        cut.push(to);
        Self::from_homogeneous(p, cut, piece, self.weights.is_some())
    }

    /// Its Bézier pieces, in order: every knot inside is inserted until
    /// the curve passes through a control point there. Exact but for
    /// rounding.
    pub fn beziers(&self) -> Result<Vec<Bezier>, DrawingError> {
        let p = self.degree;
        let mut knots = self.knots.clone();
        let mut points = self.homogeneous();
        let (lo, hi) = self.domain();
        let mut inside: Vec<f64> = self
            .knots
            .iter()
            .copied()
            .filter(|k| *k > lo && *k < hi)
            .collect();
        inside.dedup();
        for k in inside {
            saturate(p, &mut knots, &mut points, k, false)?;
        }
        let rational = self.weights.is_some();
        let mut pieces = Vec::new();
        let mut first = 0_usize;
        while first + p < points.len() {
            let piece = points.get(first..=first + p).ok_or(BROKEN)?;
            if piece.iter().any(|h| h[2] <= 0.0 || !h[2].is_finite()) {
                return Err(DrawingError::InvalidSpline("a weight is not positive"));
            }
            pieces.push(Bezier {
                control: piece.iter().map(|h| [h[0] / h[2], h[1] / h[2]]).collect(),
                weights: rational.then(|| piece.iter().map(|h| h[2]).collect()),
            });
            first += p;
        }
        if pieces.is_empty() {
            return Err(BROKEN);
        }
        Ok(pieces)
    }

    /// The point at parameter `t` (clamped to the domain), by de Boor's
    /// algorithm.
    pub fn eval(&self, t: f64) -> Result<Point, DrawingError> {
        let p = self.degree;
        let n = self.control.len();
        let (lo, hi) = self.domain();
        if !t.is_finite() {
            return Err(DrawingError::InvalidSpline("a parameter is not finite"));
        }
        let t = t.clamp(lo, hi);
        let k = self
            .knots
            .iter()
            .rposition(|k| *k <= t)
            .ok_or(BROKEN)?
            .clamp(p, n.saturating_sub(1));
        let all = self.homogeneous();
        let mut d: Vec<H> = all.get(k - p..=k).ok_or(BROKEN)?.to_vec();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let left = knot(&self.knots, j + k - p)?;
                let right = knot(&self.knots, j + 1 + k - r)?;
                let span = right - left;
                let alpha = if span > 0.0 { (t - left) / span } else { 0.0 };
                let value = lerp(point(&d, j - 1)?, point(&d, j)?, alpha);
                *d.get_mut(j).ok_or(BROKEN)? = value;
            }
        }
        let h = point(&d, p)?;
        Ok([h[0] / h[2], h[1] / h[2]])
    }
}

/// Inserts the knot `u` until it occurs `degree` times (Boehm's
/// algorithm). `left` takes `u` as the end of the span before it, which is
/// what the end of the domain needs.
fn saturate(
    degree: usize,
    knots: &mut Vec<f64>,
    points: &mut Vec<H>,
    u: f64,
    left: bool,
) -> Result<(), DrawingError> {
    let present = knots.iter().filter(|k| **k == u).count();
    for _ in present..degree {
        if points.len() >= MAX_SPLINE_CONTROL_POINTS * (MAX_SPLINE_DEGREE + 1) {
            return Err(DrawingError::TooLarge("a spline with too many pieces"));
        }
        let k = if left {
            knots.iter().rposition(|k| *k < u)
        } else {
            knots.iter().rposition(|k| *k <= u)
        }
        .ok_or(BROKEN)?;
        if k < degree || k >= points.len() {
            return Err(BROKEN);
        }
        let mut next = Vec::with_capacity(points.len() + 1);
        for i in 0..=points.len() {
            next.push(if i + degree <= k {
                point(points, i)?
            } else if i > k {
                point(points, i - 1)?
            } else {
                let (lo, hi) = (knot(knots, i)?, knot(knots, i + degree)?);
                if hi <= lo {
                    return Err(BROKEN);
                }
                lerp(
                    point(points, i - 1)?,
                    point(points, i)?,
                    (u - lo) / (hi - lo),
                )
            });
        }
        *points = next;
        knots.insert(k + 1, u);
    }
    Ok(())
}

impl Bezier {
    pub fn degree(&self) -> usize {
        self.control.len().saturating_sub(1)
    }

    fn homogeneous(&self) -> Vec<H> {
        self.control
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let w = self
                    .weights
                    .as_ref()
                    .and_then(|w| w.get(i))
                    .copied()
                    .unwrap_or(1.0);
                [p[0] * w, p[1] * w, w]
            })
            .collect()
    }

    /// The point and the first derivative at `t` in `[0, 1]`, by de
    /// Casteljau's algorithm on the weighted points.
    pub fn derivatives(&self, t: f64) -> (Point, Point) {
        let mut d = self.homogeneous();
        let n = self.degree();
        // The last two points of the scheme give the derivative.
        let mut pair: Option<(H, H)> = None;
        for level in 0..n {
            if level + 1 == n {
                pair = match d.as_slice() {
                    [a, b, ..] => Some((*a, *b)),
                    _ => None,
                };
            }
            let next: Vec<H> = d
                .windows(2)
                .take(n - level)
                .filter_map(|w| match w {
                    [a, b] => Some(lerp(*a, *b, t)),
                    _ => None,
                })
                .collect();
            d = next;
        }
        let Some(h) = d.first().copied() else {
            return ([0.0, 0.0], [0.0, 0.0]);
        };
        let at = [h[0] / h[2], h[1] / h[2]];
        let Some((a, b)) = pair else {
            return (at, [0.0, 0.0]);
        };
        let k = n as f64;
        let dh = [k * (b[0] - a[0]), k * (b[1] - a[1]), k * (b[2] - a[2])];
        // (x/w)' = (x' - (x/w) w') / w.
        (
            at,
            [
                (dh[0] - at[0] * dh[2]) / h[2],
                (dh[1] - at[1] * dh[2]) / h[2],
            ],
        )
    }

    pub fn eval(&self, t: f64) -> Point {
        self.derivatives(t).0
    }

    pub fn derivative(&self, t: f64) -> Point {
        self.derivatives(t).1
    }
}
