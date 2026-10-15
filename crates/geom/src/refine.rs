//! Changing how a B-spline curve is described without changing the curve:
//! knot insertion, degree elevation, reversal, a new parameter range, and
//! bringing several curves to one degree and one knot vector. With those,
//! and with interpolation through a row of points, a surface can be laid
//! through section curves (skinning).
//!
//! Everything works on homogeneous control points, so rational curves
//! (circles and arcs) are handled as polynomial ones are. Reference: Piegl
//! & Tiller, *The NURBS Book*, 2nd ed., §5.2 (knot insertion), §5.5 (degree
//! elevation, here by way of the Bézier pieces), §9.2 (global
//! interpolation) and §10.3 (skinning); the periodic cubic spline is the
//! one of any text on spline interpolation (de Boor, *A Practical Guide to
//! Splines*, ch. IV).

use crate::{GeomError, MAX_CONTROL_POINTS, MAX_DEGREE, NurbsCurve};
use craft3d_math::Vec3;

/// Most points [`interpolation`] and [`periodic_tangents`] accept: their
/// matrices are dense.
pub const MAX_INTERPOLATED: usize = 256;

/// Knots closer than this, on a curve whose parameter runs over `[0, 1]`,
/// are one knot.
const SAME_KNOT: f64 = 1e-12;

type H = [f64; 4];

fn blend(a: H, b: H, t: f64) -> H {
    let mut out = [0.0; 4];
    for (slot, (x, y)) in out.iter_mut().zip(a.iter().zip(&b)) {
        *slot = x * (1.0 - t) + y * t;
    }
    out
}

fn singular() -> GeomError {
    GeomError::NoConvergence("the interpolation has no unique solution")
}

fn internal() -> GeomError {
    GeomError::InvalidParameter("a spline lost track of its control points; this is a bug")
}

/// A curve as plain data: degree, knots, homogeneous control points.
#[derive(Clone, Debug)]
struct Raw {
    degree: usize,
    knots: Vec<f64>,
    points: Vec<H>,
}

impl Raw {
    fn of(curve: &NurbsCurve) -> Raw {
        Raw {
            degree: curve.degree(),
            knots: curve.knots().to_vec(),
            points: curve
                .control_points()
                .iter()
                .zip(curve.weights())
                .map(|(p, w)| [p.x * w, p.y * w, p.z * w, *w])
                .collect(),
        }
    }

    fn curve(self) -> Result<NurbsCurve, GeomError> {
        let mut control = Vec::with_capacity(self.points.len());
        let mut weights = Vec::with_capacity(self.points.len());
        for [x, y, z, w] in self.points {
            if !(w.is_finite() && w > 0.0) {
                return Err(GeomError::InvalidParameter(
                    "weights must be positive and finite",
                ));
            }
            control.push(Vec3::new(x / w, y / w, z / w));
            weights.push(w);
        }
        NurbsCurve::new(self.degree, self.knots, control, weights)
    }

    fn domain(&self) -> Result<(f64, f64), GeomError> {
        let last = self.knots.len().saturating_sub(self.degree + 1);
        match (self.knots.get(self.degree), self.knots.get(last)) {
            (Some(&a), Some(&b)) if a < b => Ok((a, b)),
            _ => Err(internal()),
        }
    }

    /// True when the curve starts at its first control point and ends at
    /// its last: the end knots are repeated `degree + 1` times.
    fn clamped(&self) -> bool {
        let Ok((a, b)) = self.domain() else {
            return false;
        };
        let n = self.knots.len();
        self.knots.iter().take(self.degree + 1).all(|k| *k == a)
            && self
                .knots
                .iter()
                .skip(n.saturating_sub(self.degree + 1))
                .all(|k| *k == b)
    }

    /// How often `u` occurs among the knots.
    fn multiplicity(&self, u: f64) -> usize {
        self.knots.iter().filter(|k| **k == u).count()
    }

    /// Inserts the knot `u`, strictly inside the domain, once.
    fn insert(&mut self, u: f64) -> Result<(), GeomError> {
        let p = self.degree;
        let n = self.points.len();
        let (a, b) = self.domain()?;
        if !(u > a && u < b) {
            return Err(GeomError::InvalidParameter(
                "a knot is inserted strictly inside the parameter domain",
            ));
        }
        if n >= MAX_CONTROL_POINTS {
            return Err(GeomError::InvalidParameter(
                "the curve has too many control points to refine",
            ));
        }
        // The span of u: the last knot at or below it, within the domain.
        let span = (p..n)
            .rev()
            .find(|i| self.knots.get(*i).is_some_and(|k| *k <= u))
            .ok_or_else(internal)?;
        let mut points = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let point = if i + p <= span {
                *self.points.get(i).ok_or_else(internal)?
            } else if i <= span {
                let (Some(&low), Some(&high)) = (self.knots.get(i), self.knots.get(i + p)) else {
                    return Err(internal());
                };
                let (Some(&before), Some(&here)) =
                    (self.points.get(i.wrapping_sub(1)), self.points.get(i))
                else {
                    return Err(internal());
                };
                // high > u ≥ low: the span is not empty.
                blend(before, here, (u - low) / (high - low))
            } else {
                *self.points.get(i - 1).ok_or_else(internal)?
            };
            points.push(point);
        }
        self.knots.insert(span + 1, u);
        self.points = points;
        Ok(())
    }

    /// The knots strictly inside the domain, each once, ascending.
    fn interior(&self) -> Vec<f64> {
        let Ok((a, b)) = self.domain() else {
            return Vec::new();
        };
        let mut out: Vec<f64> = Vec::new();
        for &k in &self.knots {
            if k > a && k < b && out.last() != Some(&k) {
                out.push(k);
            }
        }
        out
    }

    /// One degree higher, the same curve. Every knot inside the domain
    /// ends up `degree + 1` times in the result (the curve is cut into its
    /// Bézier pieces, and they are not joined again): a description with
    /// more control points than the least possible, of the same curve.
    fn elevate(&mut self) -> Result<(), GeomError> {
        let p = self.degree;
        if p >= MAX_DEGREE {
            return Err(GeomError::DegreeTooHigh {
                degree: p + 1,
                max: MAX_DEGREE,
            });
        }
        if !self.clamped() {
            return Err(GeomError::Unsupported(
                "raising the degree of a curve whose end knots are not repeated (not clamped)",
            ));
        }
        let (a, b) = self.domain()?;
        let interior = self.interior();
        for &k in &interior {
            for _ in self.multiplicity(k)..p {
                self.insert(k)?;
            }
        }
        let pieces = interior.len() + 1;
        if self.points.len() != pieces * p + 1 {
            return Err(internal());
        }
        let mut points = Vec::with_capacity(pieces * (p + 1) + 1);
        for piece in 0..pieces {
            let local = self
                .points
                .get(piece * p..=piece * p + p)
                .ok_or_else(internal)?;
            if piece == 0 {
                points.push(*local.first().ok_or_else(internal)?);
            }
            for i in 1..=p {
                let (Some(&before), Some(&here)) = (local.get(i - 1), local.get(i)) else {
                    return Err(internal());
                };
                // Q_i = i/(p+1) · P_{i-1} + (1 − i/(p+1)) · P_i
                points.push(blend(before, here, 1.0 - i as f64 / (p + 1) as f64));
            }
            points.push(*local.last().ok_or_else(internal)?);
        }
        let mut knots = vec![a; p + 2];
        for &k in &interior {
            knots.extend(std::iter::repeat_n(k, p + 1));
        }
        knots.extend(std::iter::repeat_n(b, p + 2));
        self.degree = p + 1;
        self.knots = knots;
        self.points = points;
        Ok(())
    }

    /// The parameter moved to run over `[0, 1]`.
    fn normalize(&mut self) -> Result<(), GeomError> {
        let (a, b) = self.domain()?;
        let width = b - a;
        for k in &mut self.knots {
            *k = if *k <= a {
                0.0
            } else if *k >= b {
                1.0
            } else {
                (*k - a) / width
            };
        }
        Ok(())
    }
}

/// The same curve run the other way, over the same parameter range.
pub fn reversed(curve: &NurbsCurve) -> Result<NurbsCurve, GeomError> {
    let mut raw = Raw::of(curve);
    let (first, last) = match (raw.knots.first(), raw.knots.last()) {
        (Some(&a), Some(&b)) => (a, b),
        _ => return Err(internal()),
    };
    raw.points.reverse();
    raw.knots = raw.knots.iter().rev().map(|k| first + (last - k)).collect();
    raw.curve()
}

/// The same curve with each of `knots` (strictly inside its domain)
/// inserted once more; a value given twice is inserted twice.
pub fn with_knots(curve: &NurbsCurve, knots: &[f64]) -> Result<NurbsCurve, GeomError> {
    let mut raw = Raw::of(curve);
    for &u in knots {
        if raw.multiplicity(u) >= raw.degree {
            return Err(GeomError::InvalidParameter(
                "a knot is repeated at most as often as the degree",
            ));
        }
        raw.insert(u)?;
    }
    raw.curve()
}

/// The same curve at `degree`, which is at least its own. See
/// [`compatible`] for what the description looks like.
pub fn elevated(curve: &NurbsCurve, degree: usize) -> Result<NurbsCurve, GeomError> {
    if degree < curve.degree() {
        return Err(GeomError::InvalidParameter(
            "a curve's degree can be raised, not lowered",
        ));
    }
    if degree > MAX_DEGREE {
        return Err(GeomError::DegreeTooHigh {
            degree,
            max: MAX_DEGREE,
        });
    }
    let mut raw = Raw::of(curve);
    while raw.degree < degree {
        raw.elevate()?;
    }
    raw.curve()
}

/// The piece of the curve between parameters `from < to` of its domain, as
/// a curve of its own over `[0, 1]`.
pub fn piece(curve: &NurbsCurve, from: f64, to: f64) -> Result<NurbsCurve, GeomError> {
    let mut raw = Raw::of(curve);
    let (a, b) = raw.domain()?;
    if !(from >= a && to <= b && from < to) {
        return Err(GeomError::InvalidParameter(
            "a piece of a curve lies within its domain and has a length",
        ));
    }
    if !raw.clamped() {
        return Err(GeomError::Unsupported(
            "cutting a curve whose end knots are not repeated (not clamped)",
        ));
    }
    let p = raw.degree;
    for cut in [from, to] {
        if cut > a && cut < b {
            for _ in raw.multiplicity(cut)..p {
                raw.insert(cut)?;
            }
        }
    }
    // A knot repeated `degree` times from index i on is where control
    // point i − 1 lies on the curve.
    let first = if from > a {
        raw.knots
            .iter()
            .position(|k| *k == from)
            .and_then(|i| i.checked_sub(1))
            .ok_or_else(internal)?
    } else {
        0
    };
    let last = if to < b {
        raw.knots
            .iter()
            .position(|k| *k == to)
            .and_then(|i| i.checked_sub(1))
            .ok_or_else(internal)?
    } else {
        raw.points.len().saturating_sub(1)
    };
    let points = raw.points.get(first..=last).ok_or_else(internal)?.to_vec();
    let mut knots = vec![from; p + 1];
    knots.extend(raw.knots.iter().copied().filter(|k| *k > from && *k < to));
    knots.extend(std::iter::repeat_n(to, p + 1));
    let mut out = Raw {
        degree: p,
        knots,
        points,
    };
    out.normalize()?;
    out.curve()
}

/// The same curves, each over the parameter range `[0, 1]`, all of one
/// degree (the highest among them) and on one knot vector (every knot of
/// any of them). Their control points then correspond one to one, which
/// is what laying a surface through them needs.
///
/// The curves must be clamped. A curve whose degree is raised and that has
/// knots inside its domain comes back cut at those knots (each repeated as
/// often as the new degree): the same curve, with more control points than
/// the least possible.
pub fn compatible(curves: &[NurbsCurve]) -> Result<Vec<NurbsCurve>, GeomError> {
    let degree = curves.iter().map(NurbsCurve::degree).max().unwrap_or(1);
    let mut raws = Vec::with_capacity(curves.len());
    for curve in curves {
        let mut raw = Raw::of(curve);
        if !raw.clamped() {
            return Err(GeomError::Unsupported(
                "a section curve whose end knots are not repeated (not clamped)",
            ));
        }
        raw.normalize()?;
        while raw.degree < degree {
            raw.elevate()?;
        }
        raws.push(raw);
    }
    // Every knot of any curve, ascending; values within rounding of each
    // other are one knot, and every curve is given the same number for it.
    let mut all: Vec<f64> = raws.iter().flat_map(Raw::interior).collect();
    all.sort_by(f64::total_cmp);
    let mut distinct: Vec<f64> = Vec::new();
    for k in all {
        if distinct.last().is_none_or(|last| k - last > SAME_KNOT) {
            distinct.push(k);
        }
    }
    for raw in &mut raws {
        for k in &mut raw.knots {
            if let Some(same) = distinct.iter().find(|d| (**d - *k).abs() <= SAME_KNOT) {
                *k = *same;
            }
        }
    }
    for &k in &distinct {
        let most = raws.iter().map(|r| r.multiplicity(k)).max().unwrap_or(0);
        for raw in &mut raws {
            for _ in raw.multiplicity(k)..most.min(degree) {
                raw.insert(k)?;
            }
        }
    }
    let count = raws.first().map_or(0, |r| r.points.len());
    if raws.iter().any(|r| r.points.len() != count) {
        return Err(GeomError::Unsupported(
            "section curves with a knot repeated more often than their degree",
        ));
    }
    raws.into_iter().map(Raw::curve).collect()
}

/// The values at `t` of the `count` B-spline basis functions of `degree`
/// on `knots` (the triangular scheme of Cox and de Boor).
fn basis(knots: &[f64], degree: usize, count: usize, t: f64) -> Result<Vec<f64>, GeomError> {
    let mut row = vec![0.0; count];
    // The span of t; the last one for the end of the domain.
    let span = (degree..count)
        .rev()
        .find(|i| knots.get(*i).is_some_and(|k| *k <= t))
        .unwrap_or(degree);
    let knot = |i: usize| knots.get(i).copied().ok_or_else(internal);
    let mut values = vec![0.0; degree + 1];
    let mut left = vec![0.0; degree + 1];
    let mut right = vec![0.0; degree + 1];
    if let Some(first) = values.first_mut() {
        *first = 1.0;
    }
    for j in 1..=degree {
        let (l, r) = (t - knot(span + 1 - j)?, knot(span + j)? - t);
        if let (Some(a), Some(b)) = (left.get_mut(j), right.get_mut(j)) {
            *a = l;
            *b = r;
        }
        let mut saved = 0.0;
        for r in 0..j {
            let (Some(&right_r), Some(&left_r), Some(&value)) =
                (right.get(r + 1), left.get(j - r), values.get(r))
            else {
                return Err(internal());
            };
            let share = value / (right_r + left_r);
            if let Some(slot) = values.get_mut(r) {
                *slot = saved + right_r * share;
            }
            saved = left_r * share;
        }
        if let Some(slot) = values.get_mut(j) {
            *slot = saved;
        }
    }
    for (offset, value) in values.into_iter().enumerate() {
        if let Some(slot) = row.get_mut(span - degree + offset) {
            *slot = value;
        }
    }
    Ok(row)
}

/// The inverse of a square matrix, by elimination with the largest pivot
/// of each column; an error when it has none.
fn inverse(mut m: Vec<Vec<f64>>) -> Result<Vec<Vec<f64>>, GeomError> {
    let n = m.len();
    let singular = || GeomError::NoConvergence("the interpolation has no unique solution");
    let mut inv: Vec<Vec<f64>> = (0..n)
        .map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect())
        .collect();
    for col in 0..n {
        let at = |m: &Vec<Vec<f64>>, r: usize| m.get(r).and_then(|row| row.get(col)).copied();
        let pivot = (col..n)
            .filter_map(|r| at(&m, r).map(|v| (r, v.abs())))
            .filter(|(_, v)| v.is_finite())
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
            .ok_or_else(singular)?;
        if pivot.1 <= 1e-14 {
            return Err(singular());
        }
        m.swap(col, pivot.0);
        inv.swap(col, pivot.0);
        let scale = 1.0 / at(&m, col).ok_or_else(internal)?;
        for matrix in [&mut m, &mut inv] {
            if let Some(row) = matrix.get_mut(col) {
                for v in row.iter_mut() {
                    *v *= scale;
                }
            }
        }
        let (pivot_m, pivot_inv) = match (m.get(col), inv.get(col)) {
            (Some(a), Some(b)) => (a.clone(), b.clone()),
            _ => return Err(internal()),
        };
        for r in (0..n).filter(|r| *r != col) {
            let factor = at(&m, r).ok_or_else(internal)?;
            if factor == 0.0 {
                continue;
            }
            for (matrix, pivot_row) in [(&mut m, &pivot_m), (&mut inv, &pivot_inv)] {
                if let Some(row) = matrix.get_mut(r) {
                    for (v, p) in row.iter_mut().zip(pivot_row) {
                        *v -= factor * p;
                    }
                }
            }
        }
    }
    if inv.iter().flatten().any(|v| !v.is_finite()) {
        return Err(singular());
    }
    Ok(inv)
}

fn check_params(params: &[f64]) -> Result<(), GeomError> {
    if params.len() < 2 || params.len() > MAX_INTERPOLATED {
        return Err(GeomError::InvalidParameter(
            "interpolation needs at least two points (and no more than the supported maximum)",
        ));
    }
    if params.iter().any(|v| !v.is_finite())
        || params
            .iter()
            .zip(params.iter().skip(1))
            .any(|(a, b)| a >= b)
    {
        return Err(GeomError::InvalidParameter(
            "the parameters of interpolated points must be finite and increasing",
        ));
    }
    Ok(())
}

/// Most points [`interpolate`] accepts: its system is banded, solved in
/// time and room linear in the points.
pub const MAX_INTERPOLATED_BANDED: usize = 1 << 14;

/// The control values of the B-spline of `degree` through `values` (each
/// a vector of the same length: a point's coordinates, weighted ones)
/// at the increasing `params`, with its knot vector (clamped, the inner
/// knots the averages of `degree` consecutive parameters, as
/// [`interpolation`]). The collocation matrix is banded and totally
/// positive, so Gaussian elimination within the band needs no pivoting
/// (de Boor, *A Practical Guide to Splines*, ch. XIII).
pub fn interpolate(
    params: &[f64],
    degree: usize,
    values: &[Vec<f64>],
) -> Result<(Vec<f64>, Vec<Vec<f64>>), GeomError> {
    let count = params.len();
    if !(2..=MAX_INTERPOLATED_BANDED).contains(&count) || values.len() != count {
        return Err(GeomError::InvalidParameter(
            "interpolation needs at least two points, a value for each (and no more than the supported maximum)",
        ));
    }
    if params.iter().any(|v| !v.is_finite())
        || params.windows(2).any(|w| !matches!(w, [a, b] if a < b))
    {
        return Err(GeomError::InvalidParameter(
            "the parameters of interpolated points must be finite and increasing",
        ));
    }
    if degree == 0 || degree >= count || degree > MAX_DEGREE {
        return Err(GeomError::InvalidParameter(
            "the degree of an interpolating spline is at least 1 and below the number of points",
        ));
    }
    let width = values.first().map_or(0, Vec::len);
    if values.iter().any(|v| v.len() != width) {
        return Err(GeomError::InvalidParameter(
            "the interpolated values must all have the same length",
        ));
    }
    let (Some(&first), Some(&last)) = (params.first(), params.last()) else {
        return Err(internal());
    };
    let mut knots = vec![first; degree + 1];
    for j in 1..count - degree {
        let sum: f64 = params.iter().skip(j).take(degree).sum();
        knots.push(sum / degree as f64);
    }
    knots.extend(std::iter::repeat_n(last, degree + 1));
    // Row i holds columns i - degree ..= i + degree.
    let band = 2 * degree + 1;
    let mut rows = vec![vec![0.0; band]; count];
    for (i, &t) in params.iter().enumerate() {
        let span = (degree..count)
            .rev()
            .find(|k| knots.get(*k).is_some_and(|x| *x <= t))
            .unwrap_or(degree);
        let local = basis_local(&knots, degree, span, t)?;
        for (offset, value) in local.into_iter().enumerate() {
            let column = span - degree + offset;
            // Within the band (Schoenberg-Whitney: the averaged knots
            // keep every parameter's span next to its own index).
            let slot = (column + degree)
                .checked_sub(i)
                .and_then(|c| rows.get_mut(i).and_then(|r| r.get_mut(c)))
                .ok_or_else(singular)?;
            *slot = value;
        }
    }
    let mut rhs: Vec<Vec<f64>> = values.to_vec();
    let at = |rows: &Vec<Vec<f64>>, i: usize, c: usize| -> f64 {
        (c + degree)
            .checked_sub(i)
            .and_then(|k| rows.get(i).and_then(|r| r.get(k)))
            .copied()
            .unwrap_or(0.0)
    };
    for k in 0..count {
        let pivot = at(&rows, k, k);
        if pivot.abs().partial_cmp(&1e-300) != Some(std::cmp::Ordering::Greater)
            || !pivot.is_finite()
        {
            return Err(singular());
        }
        for i in k + 1..(k + degree + 1).min(count) {
            let factor = at(&rows, i, k) / pivot;
            if factor == 0.0 {
                continue;
            }
            for c in k..(k + degree + 1).min(count) {
                let value = at(&rows, i, c) - factor * at(&rows, k, c);
                if let Some(slot) = (c + degree)
                    .checked_sub(i)
                    .and_then(|x| rows.get_mut(i).and_then(|r| r.get_mut(x)))
                {
                    *slot = value;
                }
            }
            let source = rhs.get(k).cloned().ok_or_else(internal)?;
            let target = rhs.get_mut(i).ok_or_else(internal)?;
            for (t, s) in target.iter_mut().zip(&source) {
                *t -= factor * s;
            }
        }
    }
    let mut out = vec![vec![0.0; width]; count];
    for k in (0..count).rev() {
        let mut value = rhs.get(k).cloned().ok_or_else(internal)?;
        for c in k + 1..(k + degree + 1).min(count) {
            let a = at(&rows, k, c);
            let known = out.get(c).ok_or_else(internal)?;
            for (v, x) in value.iter_mut().zip(known) {
                *v -= a * x;
            }
        }
        let pivot = at(&rows, k, k);
        for v in &mut value {
            *v /= pivot;
        }
        *out.get_mut(k).ok_or_else(internal)? = value;
    }
    if out.iter().flatten().any(|v| !v.is_finite()) {
        return Err(singular());
    }
    Ok((knots, out))
}

/// The `degree + 1` nonzero B-spline basis values at `t` in `span`.
fn basis_local(knots: &[f64], degree: usize, span: usize, t: f64) -> Result<Vec<f64>, GeomError> {
    let knot = |i: usize| knots.get(i).copied().ok_or_else(internal);
    let mut values = vec![0.0; degree + 1];
    let mut left = vec![0.0; degree + 1];
    let mut right = vec![0.0; degree + 1];
    if let Some(first) = values.first_mut() {
        *first = 1.0;
    }
    for j in 1..=degree {
        let (l, r) = (t - knot(span + 1 - j)?, knot(span + j)? - t);
        if let (Some(a), Some(b)) = (left.get_mut(j), right.get_mut(j)) {
            *a = l;
            *b = r;
        }
        let mut saved = 0.0;
        for r in 0..j {
            let (Some(&right_r), Some(&left_r), Some(&value)) =
                (right.get(r + 1), left.get(j - r), values.get(r))
            else {
                return Err(internal());
            };
            let share = value / (right_r + left_r);
            if let Some(slot) = values.get_mut(r) {
                *slot = saved + right_r * share;
            }
            saved = left_r * share;
        }
        if let Some(slot) = values.get_mut(j) {
            *slot = saved;
        }
    }
    Ok(values)
}

/// How to lay a B-spline of `degree` through points at the parameters
/// `params` (increasing): the knot vector (clamped, its inner knots the
/// averages of `degree` consecutive parameters), and the matrix that
/// turns the points into control points: control point `i` is the sum of
/// `matrix[i][k]` times point `k`. The curve then passes through point
/// `k` at `params[k]`; it has as many control points as there are points.
pub fn interpolation(
    params: &[f64],
    degree: usize,
) -> Result<(Vec<f64>, Vec<Vec<f64>>), GeomError> {
    check_params(params)?;
    let count = params.len();
    if degree == 0 || degree >= count || degree > MAX_DEGREE {
        return Err(GeomError::InvalidParameter(
            "the degree of an interpolating spline is at least 1 and below the number of points",
        ));
    }
    let (Some(&first), Some(&last)) = (params.first(), params.last()) else {
        return Err(internal());
    };
    let mut knots = vec![first; degree + 1];
    for j in 1..count - degree {
        let sum: f64 = params.iter().skip(j).take(degree).sum();
        knots.push(sum / degree as f64);
    }
    knots.extend(std::iter::repeat_n(last, degree + 1));
    let mut rows = Vec::with_capacity(count);
    for &t in params {
        rows.push(basis(&knots, degree, count, t)?);
    }
    Ok((knots, inverse(rows)?))
}

/// The tangents of the closed cubic spline with continuous curvature
/// through points at parameter distances `steps` (`steps[k]` from point
/// `k` to the next, the last one back to point 0; all positive; at least
/// three points): the derivative at point `i` is the sum of
/// `matrix[i][k]` times point `k`.
pub fn periodic_tangents(steps: &[f64]) -> Result<Vec<Vec<f64>>, GeomError> {
    let n = steps.len();
    if !(3..=MAX_INTERPOLATED).contains(&n) {
        return Err(GeomError::InvalidParameter(
            "a closed spline needs at least three points (and no more than the supported maximum)",
        ));
    }
    if steps.iter().any(|h| !(h.is_finite() && *h > 0.0)) {
        return Err(GeomError::InvalidParameter(
            "the parameter steps of a closed spline must be positive and finite",
        ));
    }
    let step = |k: usize| steps.get(k % n).copied().ok_or_else(internal);
    let mut system = vec![vec![0.0; n]; n];
    let mut rhs = vec![vec![0.0; n]; n];
    for k in 0..n {
        let (before, after) = ((k + n - 1) % n, (k + 1) % n);
        let (h_before, h_here) = (step(before)?, step(k)?);
        let (Some(row), Some(right)) = (system.get_mut(k), rhs.get_mut(k)) else {
            return Err(internal());
        };
        // h_k T_{k-1} + 2 (h_{k-1} + h_k) T_k + h_{k-1} T_{k+1}
        //   = 3 (h_k (P_k − P_{k-1}) / h_{k-1} + h_{k-1} (P_{k+1} − P_k) / h_k)
        for (index, value) in [
            (before, h_here),
            (k, 2.0 * (h_before + h_here)),
            (after, h_before),
        ] {
            if let Some(slot) = row.get_mut(index) {
                *slot += value;
            }
        }
        let (a, b) = (3.0 * h_here / h_before, 3.0 * h_before / h_here);
        for (index, value) in [(before, -a), (k, a - b), (after, b)] {
            if let Some(slot) = right.get_mut(index) {
                *slot += value;
            }
        }
    }
    let inv = inverse(system)?;
    let mut out = vec![vec![0.0; n]; n];
    for (out_row, inv_row) in out.iter_mut().zip(&inv) {
        for (j, slot) in out_row.iter_mut().enumerate() {
            *slot = inv_row
                .iter()
                .zip(&rhs)
                .map(|(a, right)| a * right.get(j).copied().unwrap_or(0.0))
                .sum();
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Frame;

    #[test]
    fn the_banded_interpolation_is_the_dense_one() {
        let params: Vec<f64> = (0..40).map(|i| f64::from(i).powf(1.3) / 40.0).collect();
        let values: Vec<Vec<f64>> = params
            .iter()
            .map(|t| vec![(3.0 * t).sin(), t * t, 1.0 + 0.1 * t])
            .collect();
        let (knots, matrix) = interpolation(&params, 3).unwrap();
        let (banded_knots, control) = interpolate(&params, 3, &values).unwrap();
        assert_eq!(knots, banded_knots);
        for (row, got) in matrix.iter().zip(&control) {
            for d in 0..3 {
                let want: f64 = row.iter().zip(&values).map(|(m, v)| m * v[d]).sum();
                assert!((want - got[d]).abs() < 1e-12, "{want} {}", got[d]);
            }
        }
        // Far more points than the dense one takes.
        let many: Vec<f64> = (0..3000).map(|i| f64::from(i) / 2999.0).collect();
        let line: Vec<Vec<f64>> = many.iter().map(|t| vec![*t, 2.0 * t]).collect();
        let (_, c) = interpolate(&many, 3, &line).unwrap();
        assert_eq!(c.len(), 3000);
    }

    fn spline() -> NurbsCurve {
        NurbsCurve::polynomial(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 4.0, 4.0, 4.0],
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 2.0, 0.5),
                Vec3::new(3.0, -1.0, 1.0),
                Vec3::new(4.0, 4.0, 0.0),
                Vec3::new(6.0, 1.0, -2.0),
                Vec3::new(7.0, 0.0, 3.0),
            ],
        )
        .unwrap()
    }

    /// The largest distance between two curves at matching fractions of
    /// their domains.
    fn apart(a: &NurbsCurve, b: &NurbsCurve) -> f64 {
        let ((a0, a1), (b0, b1)) = (a.domain(), b.domain());
        (0..=200)
            .map(|i| {
                let s = f64::from(i) / 200.0;
                a.eval(a0 + (a1 - a0) * s)
                    .distance(b.eval(b0 + (b1 - b0) * s))
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn inserting_knots_and_raising_the_degree_leave_the_curve_where_it_is() {
        let circle = NurbsCurve::circle(&Frame::WORLD, 2.5).unwrap();
        for curve in [spline(), circle] {
            let (a, b) = curve.domain();
            let more = with_knots(&curve, &[a + (b - a) * 0.3, a + (b - a) * 0.3]).unwrap();
            assert_eq!(
                more.control_points().len(),
                curve.control_points().len() + 2
            );
            assert!(apart(&curve, &more) < 1e-14);
            for degree in curve.degree()..=curve.degree() + 3 {
                let higher = elevated(&curve, degree).unwrap();
                assert_eq!(higher.degree(), degree);
                assert!(apart(&curve, &higher) < 1e-13, "degree {degree}");
            }
        }
    }

    #[test]
    fn a_reversed_curve_runs_back_along_itself() {
        let curve = spline();
        let back = reversed(&curve).unwrap();
        let (a, b) = curve.domain();
        for i in 0..=50 {
            let s = f64::from(i) / 50.0;
            let there = curve.eval(a + (b - a) * s);
            assert!(there.distance(back.eval(b - (b - a) * s)) < 1e-14);
        }
        assert_eq!(reversed(&back).unwrap(), curve);
    }

    #[test]
    fn a_piece_is_the_curve_between_its_ends() {
        let circle = NurbsCurve::circle(&Frame::WORLD, 2.5).unwrap();
        for curve in [spline(), circle] {
            let (a, b) = curve.domain();
            for (from, to) in [(0.1, 0.35), (0.0, 0.6), (0.25, 1.0), (0.4, 0.45)] {
                let (from, to) = (a + (b - a) * from, a + (b - a) * to);
                let part = piece(&curve, from, to).unwrap();
                assert_eq!(part.domain(), (0.0, 1.0));
                for i in 0..=40 {
                    let s = f64::from(i) / 40.0;
                    let d = part.eval(s).distance(curve.eval(from + (to - from) * s));
                    assert!(d < 1e-13, "{d}");
                }
            }
        }
        assert!(piece(&spline(), 2.0, 2.0).is_err());
        assert!(piece(&spline(), -1.0, 2.0).is_err());
    }

    #[test]
    fn compatible_curves_share_degree_and_knots_and_are_the_same_curves() {
        let line = NurbsCurve::polynomial(
            1,
            vec![3.0, 3.0, 5.0, 5.0],
            vec![Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0)],
        )
        .unwrap();
        let circle = NurbsCurve::circle(&Frame::WORLD, 1.0).unwrap();
        let curves = [line, spline(), circle];
        let made = compatible(&curves).unwrap();
        for (before, after) in curves.iter().zip(&made) {
            assert_eq!(after.degree(), 3);
            assert_eq!(after.knots(), made[0].knots());
            assert_eq!(after.domain(), (0.0, 1.0));
            assert!(apart(before, after) < 1e-13);
            // The ends stay the ends, to the bit.
            assert_eq!(
                after.control_points().first(),
                before.control_points().first()
            );
            assert_eq!(
                after.control_points().last(),
                before.control_points().last()
            );
        }
        // Compatible curves stay as they are.
        assert_eq!(compatible(&made).unwrap(), made);
    }

    #[test]
    fn an_interpolating_spline_passes_through_its_points() {
        let points = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 3.0, 0.0),
            Vec3::new(4.0, 3.5, 1.0),
            Vec3::new(6.0, 0.0, 2.0),
            Vec3::new(9.0, -1.0, 0.0),
        ];
        let params = [0.0, 0.2, 0.55, 0.8, 1.0];
        for degree in 1..=4 {
            let (knots, matrix) = interpolation(&params, degree).unwrap();
            let control: Vec<Vec3> = matrix
                .iter()
                .map(|row| {
                    row.iter()
                        .zip(&points)
                        .fold(Vec3::ZERO, |sum, (w, p)| sum + *p * *w)
                })
                .collect();
            let curve = NurbsCurve::polynomial(degree, knots, control).unwrap();
            for (t, p) in params.iter().zip(&points) {
                assert!(curve.eval(*t).distance(*p) < 1e-13, "degree {degree}");
            }
        }
        assert!(interpolation(&params, 5).is_err());
        assert!(interpolation(&[0.0, 0.0, 1.0], 1).is_err());
        assert!(interpolation(&[0.0, f64::NAN], 1).is_err());
    }

    #[test]
    fn the_closed_spline_through_points_on_a_circle_has_their_tangents() {
        // Twelve points evenly on the unit circle: by symmetry the tangent
        // at each is perpendicular to its radius, and all are equally long.
        let n = 12;
        let points: Vec<Vec3> = (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * f64::from(k) / f64::from(n);
                Vec3::new(a.cos(), a.sin(), 0.0)
            })
            .collect();
        let matrix = periodic_tangents(&vec![1.0; n as usize]).unwrap();
        let tangents: Vec<Vec3> = matrix
            .iter()
            .map(|row| {
                row.iter()
                    .zip(&points)
                    .fold(Vec3::ZERO, |sum, (w, p)| sum + *p * *w)
            })
            .collect();
        let length = tangents[0].length();
        // Close to the true speed of the circle at this spacing.
        assert!((length - std::f64::consts::TAU / 12.0).abs() < 1e-3);
        for (p, t) in points.iter().zip(&tangents) {
            assert!(p.dot(*t).abs() < 1e-14);
            assert!((t.length() - length).abs() < 1e-14);
            assert!(p.cross(*t).z > 0.0);
        }
        assert!(periodic_tangents(&[1.0, 1.0]).is_err());
        assert!(periodic_tangents(&[1.0, 0.0, 1.0]).is_err());
    }
}
