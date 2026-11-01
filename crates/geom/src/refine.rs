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
pub(crate) const SAME_KNOT: f64 = 1e-12;

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
pub(crate) struct Raw {
    pub(crate) degree: usize,
    pub(crate) knots: Vec<f64>,
    pub(crate) points: Vec<H>,
}

impl Raw {
    pub(crate) fn of(curve: &NurbsCurve) -> Raw {
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

    pub(crate) fn curve(self) -> Result<NurbsCurve, GeomError> {
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

    pub(crate) fn domain(&self) -> Result<(f64, f64), GeomError> {
        let last = self.knots.len().saturating_sub(self.degree + 1);
        match (self.knots.get(self.degree), self.knots.get(last)) {
            (Some(&a), Some(&b)) if a < b => Ok((a, b)),
            _ => Err(internal()),
        }
    }

    /// True when the curve starts at its first control point and ends at
    /// its last: the end knots are repeated `degree + 1` times.
    pub(crate) fn clamped(&self) -> bool {
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
    pub(crate) fn multiplicity(&self, u: f64) -> usize {
        self.knots.iter().filter(|k| **k == u).count()
    }

    /// Inserts the knot `u`, strictly inside the domain, once.
    pub(crate) fn insert(&mut self, u: f64) -> Result<(), GeomError> {
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
    pub(crate) fn interior(&self) -> Vec<f64> {
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
    pub(crate) fn elevate(&mut self) -> Result<(), GeomError> {
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
        let interior = self.interior();
        for &k in &interior {
            for _ in self.multiplicity(k)..p {
                self.insert(k)?;
            }
        }
        self.raise_cut()
    }

    /// [`Raw::elevate`] with the knots inserted all at once
    /// ([`Raw::insert_all`]): the same curve, in time linear in its size
    /// (the knots inserted one by one cost the size of the curve each).
    pub(crate) fn elevate_batched(&mut self) -> Result<(), GeomError> {
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
        let missing = self.to_multiplicity(p);
        self.insert_all(&missing)?;
        self.raise_cut()
    }

    /// The knots strictly inside the domain, each once, ascending, with
    /// how often each occurs: one pass over the knots.
    pub(crate) fn knot_runs(&self) -> Vec<(f64, usize)> {
        let Ok((a, b)) = self.domain() else {
            return Vec::new();
        };
        let mut out: Vec<(f64, usize)> = Vec::new();
        for &k in &self.knots {
            if !(k > a && k < b) {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.0 == k => last.1 += 1,
                _ => out.push((k, 1)),
            }
        }
        out
    }

    /// The knots to insert (ascending) so that every knot inside the
    /// domain occurs `times` times (those that occur more often keep it).
    pub(crate) fn to_multiplicity(&self, times: usize) -> Vec<f64> {
        let mut out = Vec::new();
        for (k, m) in self.knot_runs() {
            out.extend(std::iter::repeat_n(k, times.saturating_sub(m)));
        }
        out
    }

    /// Inserts the knots `xs` (ascending, each strictly inside the domain)
    /// all at once: the knot refinement of Boehm and Prautzsch (Piegl &
    /// Tiller, *The NURBS Book*, A5.4), the same curve in time linear in
    /// the sizes of the curve and of `xs`.
    pub(crate) fn insert_all(&mut self, xs: &[f64]) -> Result<(), GeomError> {
        let (Some(&x0), Some(&xr)) = (xs.first(), xs.last()) else {
            return Ok(());
        };
        let p = self.degree;
        let count = self.points.len();
        let (a, b) = self.domain()?;
        if xs.windows(2).any(|w| matches!(w, [x, y] if y < x)) || !(x0 > a && xr < b) {
            return Err(GeomError::InvalidParameter(
                "knots are inserted in order, strictly inside the parameter domain",
            ));
        }
        let r = xs.len();
        if count.saturating_add(r) > MAX_CONTROL_POINTS {
            return Err(GeomError::InvalidParameter(
                "the curve has too many control points to refine",
            ));
        }
        let n = count.checked_sub(1).ok_or_else(internal)?;
        let m = n + p + 1;
        let u = &self.knots;
        if u.len() != m + 1 {
            return Err(internal());
        }
        let knot = |i: usize| u.get(i).copied().ok_or_else(internal);
        // The span of x: knots[s] <= x < knots[s + 1], s in [p, n].
        let span = |x: f64| -> Result<usize, GeomError> {
            let (mut lo, mut hi) = (p, n + 1);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if knot(mid)? <= x {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            Ok(lo)
        };
        let sa = span(x0)?;
        let sb = span(xr)? + 1;
        let pw = &self.points;
        let point = |i: usize| pw.get(i).copied().ok_or_else(internal);
        let mut q: Vec<H> = vec![[0.0; 4]; count + r];
        let mut ub: Vec<f64> = vec![0.0; m + 1 + r];
        let sub = |x: usize, y: usize| x.checked_sub(y).ok_or_else(internal);
        for j in 0..=sub(sa, p)? {
            *q.get_mut(j).ok_or_else(internal)? = point(j)?;
        }
        for j in sub(sb, 1)?..=n {
            *q.get_mut(j + r).ok_or_else(internal)? = point(j)?;
        }
        for j in 0..=sa {
            *ub.get_mut(j).ok_or_else(internal)? = knot(j)?;
        }
        for j in sb + p..=m {
            *ub.get_mut(j + r).ok_or_else(internal)? = knot(j)?;
        }
        let mut i = sb + p - 1;
        let mut k = sb + p + r - 1;
        for &x in xs.iter().rev() {
            while x <= knot(i)? && i > sa {
                *q.get_mut(sub(k, p + 1)?).ok_or_else(internal)? = point(sub(i, p + 1)?)?;
                *ub.get_mut(k).ok_or_else(internal)? = knot(i)?;
                k = sub(k, 1)?;
                i = sub(i, 1)?;
            }
            let next = q.get(sub(k, p)?).copied().ok_or_else(internal)?;
            *q.get_mut(sub(k, p + 1)?).ok_or_else(internal)? = next;
            for l in 1..=p {
                let ind = sub(k + l, p)?;
                let here = q.get(ind).copied().ok_or_else(internal)?;
                let before = q.get(sub(ind, 1)?).copied().ok_or_else(internal)?;
                let high = ub.get(k + l).copied().ok_or_else(internal)?;
                let alfa = high - x;
                let made = if alfa == 0.0 {
                    here
                } else {
                    let low = knot(sub(i + l, p)?)?;
                    // high > x ≥ low: the span is not empty.
                    blend(here, before, alfa / (high - low))
                };
                *q.get_mut(sub(ind, 1)?).ok_or_else(internal)? = made;
            }
            *ub.get_mut(k).ok_or_else(internal)? = x;
            k = k.saturating_sub(1);
        }
        self.points = q;
        self.knots = ub;
        Ok(())
    }

    /// One degree higher, every knot inside the domain already `degree`
    /// times there (the curve in its Bézier pieces).
    fn raise_cut(&mut self) -> Result<(), GeomError> {
        let p = self.degree;
        let (a, b) = self.domain()?;
        let interior = self.interior();
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
    pub(crate) fn normalize(&mut self) -> Result<(), GeomError> {
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

/// The binomial coefficient, as a number.
fn binomial(n: usize, k: usize) -> f64 {
    (0..k.min(n)).fold(1.0, |acc, i| acc * (n - i) as f64 / (i + 1) as f64)
}

/// The Bernstein coefficients of the product of two polynomials given by
/// theirs (of degrees `a.len() - 1` and `b.len() - 1`).
pub(crate) fn bernstein_product(a: &[f64], b: &[f64]) -> Vec<f64> {
    let (m, k) = (a.len().saturating_sub(1), b.len().saturating_sub(1));
    let mut out = vec![0.0; m + k + 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            if let Some(slot) = out.get_mut(i + j) {
                *slot += binomial(m, i) * binomial(k, j) / binomial(m + k, i + j) * x * y;
            }
        }
    }
    out
}

/// The Bernstein coefficients of the polynomial `c` (Bernstein, on
/// [0, 1]) restricted to `[lo, hi]` and run over [0, 1] there (de
/// Casteljau's subdivision at `hi`, then at `lo / hi` of what is left).
pub(crate) fn restricted(c: &[f64], lo: f64, hi: f64) -> Vec<f64> {
    let split = |c: &[f64], t: f64| -> (Vec<f64>, Vec<f64>) {
        let mut row = c.to_vec();
        let n = row.len();
        let (mut left, mut right) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for level in 0..n {
            left.push(row.first().copied().unwrap_or(0.0));
            right.push(row.last().copied().unwrap_or(0.0));
            if level + 1 < n {
                row = row
                    .windows(2)
                    .map(|w| match w {
                        [x, y] => x * (1.0 - t) + y * t,
                        _ => 0.0,
                    })
                    .collect();
            }
        }
        right.reverse();
        (left, right)
    };
    let (below, _) = split(c, hi);
    if hi > 0.0 {
        split(&below, lo / hi).1
    } else {
        below
    }
}

/// The curve as its Bézier pieces: the knots that bound them (the
/// domain's ends and every distinct knot inside it) and the homogeneous
/// points of each.
fn bezier_pieces(curve: &NurbsCurve) -> Result<(Vec<f64>, Vec<Vec<H>>), GeomError> {
    bezier_pieces_raw(Raw::of(curve))
}

/// [`bezier_pieces`] of a curve as plain data (its points any 4-vectors).
pub(crate) fn bezier_pieces_raw(mut raw: Raw) -> Result<(Vec<f64>, Vec<Vec<H>>), GeomError> {
    if !raw.clamped() {
        return Err(GeomError::Unsupported(
            "changing the parameter of a curve whose end knots are not repeated (not clamped)",
        ));
    }
    let p = raw.degree;
    let interior = raw.interior();
    for &k in &interior {
        for _ in raw.multiplicity(k)..p {
            raw.insert(k)?;
        }
    }
    cut_pieces(&raw)
}

/// [`bezier_pieces_raw`] with the knots inserted all at once
/// ([`Raw::insert_all`]): in time linear in the size of the curve.
pub(crate) fn bezier_pieces_batched(mut raw: Raw) -> Result<(Vec<f64>, Vec<Vec<H>>), GeomError> {
    if !raw.clamped() {
        return Err(GeomError::Unsupported(
            "changing the parameter of a curve whose end knots are not repeated (not clamped)",
        ));
    }
    let missing = raw.to_multiplicity(raw.degree);
    raw.insert_all(&missing)?;
    cut_pieces(&raw)
}

/// The Bézier pieces of a curve whose every knot inside the domain is
/// there `degree` times.
fn cut_pieces(raw: &Raw) -> Result<(Vec<f64>, Vec<Vec<H>>), GeomError> {
    let p = raw.degree;
    let (a, b) = raw.domain()?;
    let interior = raw.interior();
    if raw.points.len() != (interior.len() + 1) * p + 1 {
        return Err(internal());
    }
    let bounds: Vec<f64> = std::iter::once(a)
        .chain(interior)
        .chain(std::iter::once(b))
        .collect();
    let pieces = (0..bounds.len() - 1)
        .map(|i| {
            raw.points
                .get(i * p..=i * p + p)
                .map(<[H]>::to_vec)
                .ok_or_else(internal)
        })
        .collect::<Result<_, _>>()?;
    Ok((bounds, pieces))
}

/// The curve of `degree` whose Bézier pieces are `pieces`, between the
/// knots `bounds` (each inside one repeated `degree` times).
fn from_pieces(degree: usize, bounds: &[f64], pieces: &[Vec<H>]) -> Result<NurbsCurve, GeomError> {
    from_pieces_raw(degree, bounds, pieces)?.curve()
}

/// [`from_pieces`] as plain data (the points any 4-vectors).
pub(crate) fn from_pieces_raw(
    degree: usize,
    bounds: &[f64],
    pieces: &[Vec<H>],
) -> Result<Raw, GeomError> {
    if degree > MAX_DEGREE {
        return Err(GeomError::DegreeTooHigh {
            degree,
            max: MAX_DEGREE,
        });
    }
    let (Some(&a), Some(&b)) = (bounds.first(), bounds.last()) else {
        return Err(internal());
    };
    let mut knots = vec![a; degree + 1];
    for k in bounds.iter().skip(1).take(bounds.len().saturating_sub(2)) {
        knots.extend(std::iter::repeat_n(*k, degree));
    }
    knots.extend(std::iter::repeat_n(b, degree + 1));
    let mut points: Vec<H> = Vec::with_capacity(pieces.len() * degree + 1);
    for (i, piece) in pieces.iter().enumerate() {
        if piece.len() != degree + 1 {
            return Err(internal());
        }
        points.extend(piece.iter().skip(usize::from(i > 0)));
    }
    Ok(Raw {
        degree,
        knots,
        points,
    })
}

/// The same curve, each point where it was, with its parameter run
/// through the cubic φ that is increasing from the start of the domain
/// to its end and leaves the start `start` times as fast and arrives at
/// the end `end` times as fast (in the parameter scaled to [0, 1], φ is
/// Hermite's cubic from 0 to 1 with those slopes): the curve's
/// derivative at its start is `start` times what it was, at its end
/// `end` times. Three times the degree, cut into Bézier pieces (each a
/// piece composed with φ there, exactly). Refused where φ would not
/// increase all along (its slope, a quadratic, has to stay positive).
pub fn reparametrized(curve: &NurbsCurve, start: f64, end: f64) -> Result<NurbsCurve, GeomError> {
    // φ' in Bernstein form is (start, 3 − start − end, end); it is
    // positive on [0, 1] exactly when both ends are and the middle
    // coefficient is above −√(start · end).
    let middle = 3.0 - start - end;
    if !(start.is_finite() && end.is_finite() && start > 0.0 && end > 0.0)
        || middle <= -(start * end).sqrt()
    {
        return Err(GeomError::InvalidParameter(
            "a change of parameter has to keep the curve's direction all along",
        ));
    }
    composed(curve, &[0.0, start / 3.0, 1.0 - end / 3.0, 1.0])
}

/// [`reparametrized`] at one end only, by a quadratic φ: twice the degree
/// rather than three times. The derivative at that end (the start, or the
/// end when `at_end`) is `factor` times what it was, at the other end
/// `2 − factor` times (φ′ is linear); refused unless 0 < `factor` < 2.
pub fn reparametrized_at(
    curve: &NurbsCurve,
    factor: f64,
    at_end: bool,
) -> Result<NurbsCurve, GeomError> {
    if !(factor.is_finite() && factor > 0.0 && factor < 2.0) {
        return Err(GeomError::InvalidParameter(
            "a change of parameter has to keep the curve's direction all along",
        ));
    }
    let middle = if at_end {
        1.0 - factor / 2.0
    } else {
        factor / 2.0
    };
    composed(curve, &[0.0, middle, 1.0])
}

/// The curve with its parameter (scaled to [0, 1]) run through the
/// increasing polynomial φ given by its Bernstein coefficients (from 0 to
/// 1): `φ.len() − 1` times the degree, cut into Bézier pieces.
fn composed(curve: &NurbsCurve, phi: &[f64]) -> Result<NurbsCurve, GeomError> {
    composed_raw(Raw::of(curve), phi)?.curve()
}

/// [`composed`] as plain data (the points any 4-vectors).
pub(crate) fn composed_raw(raw: Raw, phi: &[f64]) -> Result<Raw, GeomError> {
    let p = raw.degree;
    let (bounds, pieces) = bezier_pieces_raw(raw)?;
    compose_pieces(p, &bounds, &pieces, phi)
}

/// [`composed_raw`] with the curve cut into its Bézier pieces by knots
/// inserted all at once (in time linear in its size), refused before
/// anything is made where the result would exceed
/// [`MAX_CONTROL_POINTS`].
pub(crate) fn composed_batched(raw: Raw, phi: &[f64]) -> Result<Raw, GeomError> {
    let p = raw.degree;
    let pieces = raw.interior().len().saturating_add(1);
    let degree = phi.len().saturating_sub(1).saturating_mul(p);
    if pieces.saturating_mul(degree).saturating_add(1) > MAX_CONTROL_POINTS {
        return Err(GeomError::InvalidParameter(
            "a curve with its parameter changed would have more control points than a curve may have",
        ));
    }
    let (bounds, pieces) = bezier_pieces_batched(raw)?;
    compose_pieces(p, &bounds, &pieces, phi)
}

/// The Bézier pieces of degree `p` between `bounds` with their parameter
/// run through φ (see [`composed`]).
fn compose_pieces(
    p: usize,
    bounds: &[f64],
    pieces: &[Vec<H>],
    phi: &[f64],
) -> Result<Raw, GeomError> {
    let at = |u: f64| {
        let mut row = phi.to_vec();
        while row.len() > 1 {
            row = row
                .windows(2)
                .map(|w| match w {
                    [x, y] => x * (1.0 - u) + y * u,
                    _ => 0.0,
                })
                .collect();
        }
        row.first().copied().unwrap_or(0.0)
    };
    let (Some(&a), Some(&b)) = (bounds.first(), bounds.last()) else {
        return Err(internal());
    };
    // Where each knot goes: φ⁻¹ of its place in the domain, by bisection
    // (φ increases).
    let inverse = |tau: f64| {
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if at(mid) < tau {
                lo = mid;
            } else {
                hi = mid;
            }
            if hi - lo <= f64::EPSILON {
                break;
            }
        }
        0.5 * (lo + hi)
    };
    let taus: Vec<f64> = bounds.iter().map(|k| (k - a) / (b - a)).collect();
    let sigmas: Vec<f64> = taus
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == 0 {
                0.0
            } else if i + 1 == taus.len() {
                1.0
            } else {
                inverse(*t)
            }
        })
        .collect();
    let degree = phi.len().saturating_sub(1) * p;
    let mut made = Vec::with_capacity(pieces.len());
    for (i, piece) in pieces.iter().enumerate() {
        let (Some(&t0), Some(&t1), Some(&s0), Some(&s1)) = (
            taus.get(i),
            taus.get(i + 1),
            sigmas.get(i),
            sigmas.get(i + 1),
        ) else {
            return Err(internal());
        };
        if !(s1 > s0 && t1 > t0) {
            return Err(internal());
        }
        // The piece's own parameter along the new span: φ there, moved
        // to run from 0 to 1 (its ends exactly).
        let mut local: Vec<f64> = restricted(phi, s0, s1)
            .iter()
            .map(|d| (d - t0) / (t1 - t0))
            .collect();
        if let Some(first) = local.first_mut() {
            *first = 0.0;
        }
        if let Some(last) = local.last_mut() {
            *last = 1.0;
        }
        let other: Vec<f64> = local.iter().map(|x| 1.0 - x).collect();
        let mut points = vec![[0.0; 4]; degree + 1];
        for (j, h) in piece.iter().enumerate() {
            let mut basis = vec![binomial(p, j)];
            for _ in 0..j {
                basis = bernstein_product(&basis, &local);
            }
            for _ in j..p {
                basis = bernstein_product(&basis, &other);
            }
            for (slot, c) in points.iter_mut().zip(&basis) {
                for (x, y) in slot.iter_mut().zip(h) {
                    *x += c * y;
                }
            }
        }
        made.push(points);
    }
    let new_bounds: Vec<f64> = sigmas.iter().map(|s| a + s * (b - a)).collect();
    from_pieces_raw(degree, &new_bounds, &made)
}

/// The same curve, every point at the same parameter, its weight level
/// at every knot: each Bézier piece's homogeneous coordinates multiplied
/// by the cubic that is 1 at the piece's ends and has there the slope
/// that cancels the weight's (−w′/w), so the weight keeps its values at
/// the knots and has slope 0 at each. Where a curve's weight turns at a
/// knot (an arc of more than a quarter turn is two rational pieces whose
/// weight falls into the knot and rises out of it), a surface laid
/// through several such curves inherits a crease there; with level
/// weights it does not. A polynomial curve comes back as it is; a
/// rational one three degrees higher. Refused where a cubic is not
/// positive all along (its Bernstein coefficients are).
pub fn level_weights(curve: &NurbsCurve) -> Result<NurbsCurve, GeomError> {
    let w = curve.weights();
    if w.iter().all(|x| Some(x) == w.first()) {
        return Ok(curve.clone());
    }
    let (bounds, pieces) = bezier_pieces(curve)?;
    let p = curve.degree();
    let mut made = Vec::with_capacity(pieces.len());
    for piece in &pieces {
        let weight = |i: usize| piece.get(i).map_or(1.0, |h| h[3]);
        let (w0, w1, wn1, wn) = (weight(0), weight(1), weight(p.saturating_sub(1)), weight(p));
        // The weight's slopes at the piece's ends, per unit of its own
        // parameter: p (w1 − w0) and p (wn − wn−1).
        let (s0, s1) = (p as f64 * (w1 - w0) / w0, p as f64 * (wn - wn1) / wn);
        let f = [1.0, 1.0 - s0 / 3.0, 1.0 + s1 / 3.0, 1.0];
        if f.iter().any(|c| !(c.is_finite() && *c > 0.0)) {
            return Err(GeomError::InvalidParameter(
                "a curve's weights are multiplied by a positive function",
            ));
        }
        let mut points = vec![[0.0; 4]; p + 4];
        for d in 0..4 {
            let column: Vec<f64> = piece
                .iter()
                .map(|h| h.get(d).copied().unwrap_or(0.0))
                .collect();
            for (slot, c) in points.iter_mut().zip(bernstein_product(&column, &f)) {
                if let Some(x) = slot.get_mut(d) {
                    *x = c;
                }
            }
        }
        made.push(points);
    }
    from_pieces(p + 3, &bounds, &made)
}

/// The piece of the curve between parameters `from < to` of its domain, as
/// a curve of its own over `[0, 1]`.
pub fn piece(curve: &NurbsCurve, from: f64, to: f64) -> Result<NurbsCurve, GeomError> {
    piece_raw(Raw::of(curve), from, to)?.curve()
}

/// [`piece`] as plain data (the points any 4-vectors), over `[0, 1]`.
pub(crate) fn piece_raw(mut raw: Raw, from: f64, to: f64) -> Result<Raw, GeomError> {
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
    Ok(out)
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
pub(crate) fn basis(
    knots: &[f64],
    degree: usize,
    count: usize,
    t: f64,
) -> Result<Vec<f64>, GeomError> {
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
    let out = solve_banded(params, degree, &knots, values)?;
    Ok((knots, out))
}

/// [`interpolate`] on a knot vector of the caller's (clamped at the first
/// and the last parameter, `params.len() + degree + 1` knots, never
/// decreasing): the values of the B-spline of `degree` on `knots` through
/// `values` at `params`. An error where a parameter does not lie in the
/// support of its own basis function within the band (the
/// Schoenberg-Whitney condition, as the averaged knots of [`interpolate`]
/// keep it), so the system has no unique solution solved this way.
pub fn interpolate_on(
    params: &[f64],
    degree: usize,
    knots: &[f64],
    values: &[Vec<f64>],
) -> Result<Vec<Vec<f64>>, GeomError> {
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
    let clamped = |at: f64, run: &[f64]| run.len() == degree + 1 && run.iter().all(|k| *k == at);
    if knots.len() != count + degree + 1
        || knots.iter().any(|k| !k.is_finite())
        || knots.windows(2).any(|w| !matches!(w, [a, b] if a <= b))
        || !params
            .first()
            .is_some_and(|f| knots.get(..=degree).is_some_and(|r| clamped(*f, r)))
        || !params
            .last()
            .is_some_and(|l| knots.get(count..).is_some_and(|r| clamped(*l, r)))
    {
        return Err(GeomError::InvalidParameter(
            "the knots of an interpolating spline are clamped at the first and the last parameter, never decreasing, as many as the points and the degree ask",
        ));
    }
    solve_banded(params, degree, knots, values)
}

/// The cubic B-spline through `values` at the increasing `params` whose
/// first derivatives at the first and the last parameter are `start` and
/// `end` (clamped end conditions: the NURBS Book, §9.2.2, the cubic case).
/// Returns its knots (the first and the last parameter four times each,
/// every inner parameter once) and its `params.len() + 2` control values.
/// The two control values beside each end are fixed by the end values and
/// derivatives (`P1 = Q0 + (u1 − u0)/3 · D0`, and likewise at the end), so
/// the curve's end tangents are the given ones to the rounding of those
/// control values; the inner ones solve a tridiagonal system (each inner
/// parameter is a knot, where only three basis functions are not zero),
/// totally positive, so eliminated without pivoting. Two points give the
/// cubic Hermite segment.
pub fn interpolate_with_end_derivatives(
    params: &[f64],
    values: &[Vec<f64>],
    start: &[f64],
    end: &[f64],
) -> Result<(Vec<f64>, Vec<Vec<f64>>), GeomError> {
    let count = params.len();
    if !(2..MAX_INTERPOLATED_BANDED).contains(&count) || values.len() != count {
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
    let width = values.first().map_or(0, Vec::len);
    if values.iter().any(|v| v.len() != width)
        || start.len() != width
        || end.len() != width
        || values
            .iter()
            .flatten()
            .chain(start)
            .chain(end)
            .any(|v| !v.is_finite())
    {
        return Err(GeomError::InvalidParameter(
            "the interpolated values and end derivatives must all be finite and have the same length",
        ));
    }
    let n = count - 1;
    let (Some(&u0), Some(&u1), Some(&ul), Some(&un)) = (
        params.first(),
        params.get(1),
        params.get(n - 1),
        params.last(),
    ) else {
        return Err(internal());
    };
    let mut knots = vec![u0; 4];
    knots.extend(params.iter().skip(1).take(n - 1).copied());
    knots.extend([un; 4]);
    let (Some(q0), Some(qn)) = (values.first(), values.last()) else {
        return Err(internal());
    };
    let along = |q: &[f64], d: &[f64], f: f64| -> Vec<f64> {
        q.iter().zip(d).map(|(q, d)| q + f * d).collect()
    };
    let p1 = along(q0, start, (u1 - u0) / 3.0);
    let pn1 = along(qn, end, -(un - ul) / 3.0);
    let mut out = Vec::with_capacity(count + 2);
    out.push(q0.clone());
    out.push(p1.clone());
    // The inner control values P2..=Pn: row i (inner parameter u_i, its span
    // i + 3) reads N_i P_i + N_{i+1} P_{i+1} + N_{i+2} P_{i+2} = Q_i, with
    // P1 and P_{n+1} known. Forward elimination (Thomas), then back.
    let inner = n - 1;
    let mut diag = Vec::with_capacity(inner);
    let mut upper = Vec::with_capacity(inner);
    let mut rhs: Vec<Vec<f64>> = Vec::with_capacity(inner);
    for i in 1..n {
        let u = params.get(i).copied().ok_or_else(internal)?;
        let local = basis_local(&knots, 3, i + 3, u)?;
        let (Some(&a), Some(&diagonal), Some(&c)) = (local.first(), local.get(1), local.get(2))
        else {
            return Err(internal());
        };
        let mut b = diagonal;
        let mut r = values.get(i).cloned().ok_or_else(internal)?;
        if i == 1 {
            for (v, p) in r.iter_mut().zip(&p1) {
                *v -= a * p;
            }
        }
        if i == n - 1 {
            for (v, p) in r.iter_mut().zip(&pn1) {
                *v -= c * p;
            }
        }
        // Eliminate the entry below the diagonal with the row before.
        if i > 1 {
            let (Some(&d), Some(&up), Some(prev)) = (diag.last(), upper.last(), rhs.last()) else {
                return Err(internal());
            };
            let factor = a / d;
            b -= factor * up;
            for (v, p) in r.iter_mut().zip(prev) {
                *v -= factor * p;
            }
        }
        if !(b.abs() > 1e-300 && b.is_finite()) {
            return Err(singular());
        }
        diag.push(b);
        upper.push(if i == n - 1 { 0.0 } else { c });
        rhs.push(r);
    }
    let mut solved = vec![vec![0.0; width]; inner];
    for k in (0..inner).rev() {
        let (Some(&d), Some(&up), Some(r)) = (diag.get(k), upper.get(k), rhs.get(k)) else {
            return Err(internal());
        };
        let mut value = r.clone();
        if let Some(next) = solved.get(k + 1) {
            for (v, x) in value.iter_mut().zip(next) {
                *v -= up * x;
            }
        }
        for v in &mut value {
            *v /= d;
        }
        *solved.get_mut(k).ok_or_else(internal)? = value;
    }
    out.extend(solved);
    out.push(pn1);
    out.push(qn.clone());
    if out.iter().flatten().any(|v| !v.is_finite()) {
        return Err(singular());
    }
    Ok((knots, out))
}

/// The banded solve of [`interpolate`] and [`interpolate_on`], on checked
/// input.
fn solve_banded(
    params: &[f64],
    degree: usize,
    knots: &[f64],
    values: &[Vec<f64>],
) -> Result<Vec<Vec<f64>>, GeomError> {
    let count = params.len();
    let width = values.first().map_or(0, Vec::len);
    // Row i holds columns i - degree ..= i + degree.
    let band = 2 * degree + 1;
    let mut rows = vec![vec![0.0; band]; count];
    // The span of each parameter: the last knot at or below it, found
    // going on from the previous one (the parameters increase).
    let mut span = degree;
    for (i, &t) in params.iter().enumerate() {
        while span + 1 < count && knots.get(span + 1).is_some_and(|x| *x <= t) {
            span += 1;
        }
        let local = basis_local(knots, degree, span, t)?;
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
    Ok(out)
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

/// The tangents of the closed cubic spline with continuous curvature
/// through `values` (each a vector of the same length: a point's
/// coordinates, weighted ones) at parameter distances `steps` (as
/// [`periodic_tangents`]): the same system, cyclic and tridiagonal,
/// solved in time and room linear in the points (the Thomas algorithm,
/// with the two corners taken out by the Sherman-Morrison formula; the
/// system is strictly diagonally dominant, so no pivoting is needed).
/// Takes up to [`MAX_INTERPOLATED_BANDED`] points.
pub fn periodic_derivatives(
    steps: &[f64],
    values: &[Vec<f64>],
) -> Result<Vec<Vec<f64>>, GeomError> {
    let n = steps.len();
    if !(3..=MAX_INTERPOLATED_BANDED).contains(&n) || values.len() != n {
        return Err(GeomError::InvalidParameter(
            "a closed spline needs at least three points, a value for each (and no more than the supported maximum)",
        ));
    }
    if steps.iter().any(|h| !(h.is_finite() && *h > 0.0)) {
        return Err(GeomError::InvalidParameter(
            "the parameter steps of a closed spline must be positive and finite",
        ));
    }
    let width = values.first().map_or(0, Vec::len);
    if values.iter().any(|v| v.len() != width) {
        return Err(GeomError::InvalidParameter(
            "the interpolated values must all have the same length",
        ));
    }
    let step = |k: usize| steps.get(k % n).copied().ok_or_else(internal);
    let value = |k: usize, d: usize| {
        values
            .get(k % n)
            .and_then(|v| v.get(d))
            .copied()
            .ok_or_else(internal)
    };
    // Row k: lower T_{k-1} + diagonal T_k + upper T_{k+1} = rhs, indices
    // taken round; the corners are row 0's lower (column n-1) and row
    // n-1's upper (column 0).
    let mut lower = vec![0.0; n];
    let mut diagonal = vec![0.0; n];
    let mut upper = vec![0.0; n];
    let mut rhs = vec![vec![0.0; width]; n];
    for k in 0..n {
        let (h_before, h_here) = (step(k + n - 1)?, step(k)?);
        let (a, b) = (3.0 * h_here / h_before, 3.0 * h_before / h_here);
        let (Some(l), Some(m), Some(u), Some(r)) = (
            lower.get_mut(k),
            diagonal.get_mut(k),
            upper.get_mut(k),
            rhs.get_mut(k),
        ) else {
            return Err(internal());
        };
        *l = h_here;
        *m = 2.0 * (h_before + h_here);
        *u = h_before;
        for (d, slot) in r.iter_mut().enumerate() {
            *slot = -a * value(k + n - 1, d)? + (a - b) * value(k, d)? + b * value(k + 1, d)?;
        }
    }
    let corner_top = *lower.first().ok_or_else(internal)?;
    let corner_bottom = *upper.last().ok_or_else(internal)?;
    let gamma = -*diagonal.first().ok_or_else(internal)?;
    if let Some(d) = diagonal.first_mut() {
        *d -= gamma;
    }
    if let Some(d) = diagonal.last_mut() {
        *d -= corner_bottom * corner_top / gamma;
    }
    // The tridiagonal solve (the Thomas algorithm), for the right-hand
    // side and for the correction vector (gamma, 0, …, 0, corner_bottom).
    let solve = |mut r: Vec<Vec<f64>>| -> Result<Vec<Vec<f64>>, GeomError> {
        let mut d = diagonal.clone();
        for k in 1..n {
            let pivot = *d.get(k - 1).ok_or_else(internal)?;
            if !(pivot.is_finite() && pivot.abs() > 0.0) {
                return Err(singular());
            }
            let w = *lower.get(k).ok_or_else(internal)? / pivot;
            let u = *upper.get(k - 1).ok_or_else(internal)?;
            *d.get_mut(k).ok_or_else(internal)? -= w * u;
            let previous = r.get(k - 1).ok_or_else(internal)?.clone();
            for (slot, p) in r.get_mut(k).ok_or_else(internal)?.iter_mut().zip(previous) {
                *slot -= w * p;
            }
        }
        for k in (0..n).rev() {
            let pivot = *d.get(k).ok_or_else(internal)?;
            if !(pivot.is_finite() && pivot.abs() > 0.0) {
                return Err(singular());
            }
            let u = *upper.get(k).ok_or_else(internal)?;
            let next = if k + 1 < n {
                r.get(k + 1).ok_or_else(internal)?.clone()
            } else {
                Vec::new()
            };
            let row = r.get_mut(k).ok_or_else(internal)?;
            for (j, slot) in row.iter_mut().enumerate() {
                let behind = next.get(j).copied().unwrap_or(0.0);
                *slot = (*slot - u * behind) / pivot;
            }
        }
        Ok(r)
    };
    let mut correction = vec![vec![0.0]; n];
    if let Some(c) = correction.first_mut() {
        *c = vec![gamma];
    }
    if let Some(c) = correction.last_mut() {
        *c = vec![corner_bottom];
    }
    let mut x = solve(rhs)?;
    let z = solve(correction)?;
    let z0 = z
        .first()
        .and_then(|v| v.first())
        .copied()
        .ok_or_else(internal)?;
    let zl = z
        .last()
        .and_then(|v| v.first())
        .copied()
        .ok_or_else(internal)?;
    let denominator = 1.0 + z0 + corner_top * zl / gamma;
    if !(denominator.is_finite() && denominator.abs() > 0.0) {
        return Err(singular());
    }
    let (x0, xl) = (
        x.first().cloned().ok_or_else(internal)?,
        x.last().cloned().ok_or_else(internal)?,
    );
    for d in 0..width {
        let (a, b) = (
            x0.get(d).copied().ok_or_else(internal)?,
            xl.get(d).copied().ok_or_else(internal)?,
        );
        let factor = (a + corner_top * b / gamma) / denominator;
        for (row, zk) in x.iter_mut().zip(&z) {
            let zk = zk.first().copied().ok_or_else(internal)?;
            if let Some(slot) = row.get_mut(d) {
                *slot -= factor * zk;
            }
        }
    }
    if x.iter().flatten().any(|v| !v.is_finite()) {
        return Err(singular());
    }
    Ok(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Frame;
    use craft3d_math::Portable;

    fn arc_and_spline() -> [NurbsCurve; 2] {
        let arc = NurbsCurve::new(
            2,
            vec![0.0, 0.0, 0.0, 2.0, 2.0, 2.0],
            vec![
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            vec![1.0, std::f64::consts::FRAC_1_SQRT_2, 1.0],
        )
        .unwrap();
        let spline = NurbsCurve::polynomial(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 0.4, 1.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 2.0, 0.0),
                Vec3::new(2.0, -1.0, 1.0),
                Vec3::new(3.0, 1.0, 0.5),
                Vec3::new(4.0, 0.0, 0.0),
            ],
        )
        .unwrap();
        [arc, spline]
    }

    /// On the knots `interpolate` lays, the same spline; on a knot vector
    /// with a knot of the degree less one, a spline through the points
    /// that is once continuously differentiable there; knots that are not
    /// clamped, too few or decreasing refused.
    #[test]
    fn an_interpolation_on_given_knots() {
        let params: Vec<f64> = (0..9).map(|i| f64::from(i) * 0.125).collect();
        let values: Vec<Vec<f64>> = params
            .iter()
            .map(|t| vec![(3.0 * t).psin(), t * t, 1.0 / (1.0 + t)])
            .collect();
        let (knots, ctrl) = interpolate(&params, 3, &values).unwrap();
        let again = interpolate_on(&params, 3, &knots, &values).unwrap();
        for (a, b) in ctrl.iter().zip(&again) {
            for (x, y) in a.iter().zip(b) {
                assert!((x - y).abs() < 1e-14);
            }
        }
        // A double knot of a cubic at 0.5, one point fewer's averages on
        // the right: square again.
        let soft = vec![
            0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.5, 0.6875, 0.8125, 1.0, 1.0, 1.0, 1.0,
        ];
        let c = interpolate_on(&params, 3, &soft, &values).unwrap();
        let points: Vec<Vec3> = c.iter().map(|r| Vec3::new(r[0], r[1], r[2])).collect();
        let curve = NurbsCurve::polynomial(3, soft.clone(), points).unwrap();
        for (t, v) in params.iter().zip(&values) {
            let p = curve.eval(*t);
            assert!(p.distance(Vec3::new(v[0], v[1], v[2])) < 1e-12, "{t}");
        }
        let (_, left) = curve.derivatives(0.5 - 1e-12);
        let (_, right) = curve.derivatives(0.5 + 1e-12);
        assert!(left.distance(right) < 1e-9, "{left:?} {right:?}");
        for bad in [
            vec![0.0; 13],
            soft[1..].to_vec(),
            vec![
                0.0, 0.0, 0.0, 0.0, 0.5, 0.25, 0.5, 0.6875, 0.8125, 1.0, 1.0, 1.0, 1.0,
            ],
            vec![
                0.0, 0.0, 0.0, 0.1, 0.25, 0.5, 0.5, 0.6875, 0.8125, 1.0, 1.0, 1.0, 1.0,
            ],
        ] {
            assert!(
                interpolate_on(&params, 3, &bad, &values).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_curve_run_through_a_cubic_is_the_same_curve() {
        for curve in arc_and_spline() {
            let (a, b) = curve.domain();
            for (start, end) in [(0.5, 2.0), (2.25, 2.25), (1.0, 0.3)] {
                let made = reparametrized(&curve, start, end).unwrap();
                assert_eq!(made.degree(), 3 * curve.degree());
                assert_eq!(made.domain(), (a, b));
                let phi = |u: f64| {
                    let v = 1.0 - u;
                    start * u * v * v + (3.0 - end) * u * u * v + u * u * u
                };
                for k in 0..=40 {
                    let u = a + (b - a) * f64::from(k) / 40.0;
                    let t = a + (b - a) * phi((u - a) / (b - a));
                    let (p, q) = (curve.eval(t), made.eval(u));
                    assert!(p.distance(q) < 1e-12, "{start} {end} at {u}: {p:?} {q:?}");
                }
                let ((_, d0), (_, e0)) = (curve.derivatives(a), made.derivatives(a));
                let ((_, d1), (_, e1)) = (curve.derivatives(b), made.derivatives(b));
                assert!((e0 - d0 * start).length() < 1e-10 * d0.length());
                assert!((e1 - d1 * end).length() < 1e-10 * d1.length());
            }
            // At one end only, by a quadratic.
            for (factor, at_end) in [(0.4, false), (1.7, true)] {
                let made = reparametrized_at(&curve, factor, at_end).unwrap();
                assert_eq!(made.degree(), 2 * curve.degree());
                let (s, e) = if at_end {
                    (2.0 - factor, factor)
                } else {
                    (factor, 2.0 - factor)
                };
                // φ in Bernstein form (0, s/2, 1): s u (1 − u) + u².
                let phi = |u: f64| s * u * (1.0 - u) + u * u;
                for k in 0..=40 {
                    let u = a + (b - a) * f64::from(k) / 40.0;
                    let t = a + (b - a) * phi((u - a) / (b - a));
                    assert!(curve.eval(t).distance(made.eval(u)) < 1e-12);
                }
                let ((_, d0), (_, e0)) = (curve.derivatives(a), made.derivatives(a));
                let ((_, d1), (_, e1)) = (curve.derivatives(b), made.derivatives(b));
                assert!((e0 - d0 * s).length() < 1e-10 * d0.length());
                assert!((e1 - d1 * e).length() < 1e-10 * d1.length());
            }
            assert!(reparametrized_at(&curve, 2.0, false).is_err());
            // Not increasing all along.
            assert!(reparametrized(&curve, 4.0, 4.0).is_err());
            assert!(reparametrized(&curve, 0.0, 1.0).is_err());
        }
    }

    #[test]
    fn a_curve_with_level_weights_is_the_same_curve() {
        // Half a turn as two rational pieces, a quarter, and the quarter
        // after a change of parameter.
        let half = NurbsCurve::new(
            2,
            vec![0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
            vec![
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(-1.0, 1.0, 0.0),
                Vec3::new(-1.0, 0.0, 0.0),
            ],
            vec![
                1.0,
                std::f64::consts::FRAC_1_SQRT_2,
                1.0,
                std::f64::consts::FRAC_1_SQRT_2,
                1.0,
            ],
        )
        .unwrap();
        let [arc, spline] = arc_and_spline();
        let moved = reparametrized(&arc, 0.6, 1.7).unwrap();
        for curve in [half, arc, moved] {
            let (a, b) = curve.domain();
            let made = level_weights(&curve).unwrap();
            assert_eq!(made.degree(), curve.degree() + 3);
            for k in 0..=40 {
                let t = a + (b - a) * f64::from(k) / 40.0;
                let ((p, d), (q, e)) = (curve.derivatives(t), made.derivatives(t));
                assert!(p.distance(q) < 1e-12);
                assert!((d - e).length() < 1e-10 * (1.0 + d.length()));
            }
            // Level at every knot: the control weights next to a knot's
            // are the knot's.
            let (n, w) = (made.degree(), made.weights());
            for i in (0..w.len()).step_by(n) {
                for j in [i.checked_sub(1), Some(i + 1)].into_iter().flatten() {
                    if j < w.len() {
                        assert!((w[j] - w[i]).abs() < 1e-12, "{w:?}");
                    }
                }
            }
        }
        // A polynomial curve is left as it is.
        let same = level_weights(&spline).unwrap();
        assert_eq!(same.degree(), spline.degree());
        assert_eq!(same.control_points(), spline.control_points());
    }

    #[test]
    fn the_linear_periodic_solve_is_the_dense_one() {
        for n in [3_usize, 4, 7, 40] {
            let steps: Vec<f64> = (0..n)
                .map(|i| 0.3 + (i as f64 * 1.7).psin().abs())
                .collect();
            let values: Vec<Vec<f64>> = (0..n)
                .map(|i| {
                    let t = i as f64;
                    vec![(0.9 * t).pcos(), t * t * 0.01, 2.0 - (0.4 * t).psin()]
                })
                .collect();
            let matrix = periodic_tangents(&steps).unwrap();
            let got = periodic_derivatives(&steps, &values).unwrap();
            for (row, g) in matrix.iter().zip(&got) {
                for d in 0..3 {
                    let want: f64 = row.iter().zip(&values).map(|(m, v)| m * v[d]).sum();
                    assert!((want - g[d]).abs() < 1e-12, "n {n}: {want} {}", g[d]);
                }
            }
        }
        // Far more points than the dense one takes: a circle, whose
        // spline's tangents are the circle's to the interpolation error.
        let many = 3000;
        let steps = vec![1.0 / many as f64; many];
        let values: Vec<Vec<f64>> = (0..many)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / many as f64;
                vec![a.pcos(), a.psin()]
            })
            .collect();
        let got = periodic_derivatives(&steps, &values).unwrap();
        for (i, g) in got.iter().enumerate() {
            let a = std::f64::consts::TAU * i as f64 / many as f64;
            let want = [
                -a.psin() * std::f64::consts::TAU,
                a.pcos() * std::f64::consts::TAU,
            ];
            assert!((g[0] - want[0]).abs() < 1e-9 && (g[1] - want[1]).abs() < 1e-9);
        }
    }

    #[test]
    fn the_banded_interpolation_is_the_dense_one() {
        let params: Vec<f64> = (0..40).map(|i| f64::from(i).ppowf(1.3) / 40.0).collect();
        let values: Vec<Vec<f64>> = params
            .iter()
            .map(|t| vec![(3.0 * t).psin(), t * t, 1.0 + 0.1 * t])
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
    fn knots_inserted_all_at_once_are_the_knots_inserted_one_by_one() {
        let circle = NurbsCurve::circle(&Frame::WORLD, 2.5).unwrap();
        for curve in [spline(), circle] {
            let raw = Raw::of(&curve);
            let (a, b) = raw.domain().unwrap();
            let at = |f: f64| a + (b - a) * f;
            // New knots, an existing one again, and one twice.
            let mut xs = vec![at(0.05), at(0.3), at(0.3), at(0.77), at(0.999)];
            xs.extend(
                raw.interior()
                    .first()
                    .copied()
                    .filter(|k| raw.multiplicity(*k) < raw.degree),
            );
            xs.sort_by(f64::total_cmp);
            let mut one = raw.clone();
            for x in &xs {
                one.insert(*x).unwrap();
            }
            let mut all = raw.clone();
            all.insert_all(&xs).unwrap();
            assert_eq!(all.knots, one.knots);
            assert_eq!(all.points.len(), one.points.len());
            for (p, q) in all.points.iter().zip(&one.points) {
                for (x, y) in p.iter().zip(q) {
                    assert!((x - y).abs() <= 1e-13 * (1.0 + x.abs()), "{p:?} {q:?}");
                }
            }
            let mut up = raw.clone();
            up.elevate().unwrap();
            let mut fast = raw.clone();
            fast.elevate_batched().unwrap();
            assert_eq!(fast.knots, up.knots);
            for (p, q) in fast.points.iter().zip(&up.points) {
                for (x, y) in p.iter().zip(q) {
                    assert!((x - y).abs() <= 1e-13 * (1.0 + x.abs()));
                }
            }
            let (bounds, pieces) = bezier_pieces_raw(raw.clone()).unwrap();
            let (bounds2, pieces2) = bezier_pieces_batched(raw.clone()).unwrap();
            assert_eq!(bounds, bounds2);
            assert_eq!(pieces.len(), pieces2.len());
            // Out of order, or on the domain's end: refused.
            assert!(raw.clone().insert_all(&[at(0.5), at(0.2)]).is_err());
            assert!(raw.clone().insert_all(&[a]).is_err());
            assert!(raw.clone().insert_all(&[]).is_ok());
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
                Vec3::new(a.pcos(), a.psin(), 0.0)
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
