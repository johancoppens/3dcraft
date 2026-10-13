//! NURBS curves and surfaces.
//!
//! Evaluation is de Boor's algorithm on homogeneous control points;
//! derivatives come from the hodograph (the derivative of a B-spline is a
//! B-spline of one degree less) and the quotient rule. Reference: Piegl &
//! Tiller, *The NURBS Book*, 2nd ed., ch. 2–4 and §7.5 (circles).
//!
//! Evaluation is local and does not allocate: the homogeneous net is made
//! once when the spline is made, and a call touches only the `degree + 1`
//! control points (per direction) of the knot span it lands in, on the
//! stack. The blend factors depend on the knots and the parameter alone, so
//! a surface works them out once per direction and not once per row. The
//! arithmetic on the control points is the same, in the same order, as
//! reducing every row in full, so the results are the same bit for bit
//! (`tests/spline_eval.rs` keeps that older algorithm to compare with).

use crate::{BezierPatch, GeomError, MAX_DEGREE};
use craft3d_math::{Frame, Vec3};
use std::fmt;

/// Largest accepted number of control points per direction.
pub const MAX_CONTROL_POINTS: usize = 1 << 16;

/// A homogeneous point: `p` is the position times the weight `w`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct H {
    p: Vec3,
    w: f64,
}

impl H {
    fn lerp(self, o: H, t: f64) -> H {
        H {
            p: self.p.lerp(o.p, t),
            w: self.w * (1.0 - t) + o.w * t,
        }
    }
}

/// Checks a knot vector for `count` control points of `degree`, and that
/// its parameter domain is not empty.
fn check_knots(knots: &[f64], degree: usize, count: usize) -> Result<(), GeomError> {
    if degree == 0 || degree > MAX_DEGREE {
        return Err(GeomError::InvalidParameter(
            "degree must be between 1 and the supported maximum",
        ));
    }
    if count <= degree || count > MAX_CONTROL_POINTS {
        return Err(GeomError::InvalidParameter(
            "a spline needs more control points than its degree (and no more than the supported maximum)",
        ));
    }
    if knots.len() != count + degree + 1 {
        return Err(GeomError::InvalidParameter(
            "a knot vector needs (control points + degree + 1) knots",
        ));
    }
    if knots.iter().any(|k| !k.is_finite())
        || knots.iter().zip(knots.iter().skip(1)).any(|(a, b)| a > b)
    {
        return Err(GeomError::InvalidParameter(
            "knots must be finite and non-decreasing",
        ));
    }
    match (knots.get(degree), knots.get(count)) {
        (Some(a), Some(b)) if a < b => Ok(()),
        _ => Err(GeomError::InvalidParameter(
            "the knot vector leaves an empty parameter domain",
        )),
    }
}

fn check_points(ctrl: &[Vec3], weights: &[f64]) -> Result<(), GeomError> {
    if weights.len() != ctrl.len() {
        return Err(GeomError::InvalidParameter(
            "one weight per control point is required",
        ));
    }
    if let Some(index) = ctrl.iter().position(|p| !p.is_finite()) {
        return Err(GeomError::NonFiniteControlPoint { index });
    }
    if weights.iter().any(|w| !(w.is_finite() && *w > 0.0)) {
        return Err(GeomError::InvalidParameter(
            "weights must be positive and finite",
        ));
    }
    Ok(())
}

/// `(start, end)` of the parameter domain of a checked knot vector.
fn domain_of(knots: &[f64], degree: usize) -> (f64, f64) {
    let count = knots.len().saturating_sub(degree + 1);
    match (knots.get(degree), knots.get(count)) {
        (Some(&a), Some(&b)) => (a, b),
        _ => (0.0, 1.0),
    }
}

const ZERO: H = H {
    p: Vec3::ZERO,
    w: 0.0,
};

/// What evaluating a spline at one parameter needs from its knot vector, and
/// nothing from its control points: the knot span and the blend factors of
/// de Boor's algorithm, for the spline and for its hodograph (the derivative
/// spline, one degree lower, on the same knots without the first and last).
/// A surface works these out once per direction and uses them for every row.
///
/// `N` is the room per level and must be above the degree; a shorter array
/// only drops terms, it cannot panic.
struct Along<const N: usize> {
    degree: usize,
    /// Index of the first of the `degree + 1` control points with influence.
    first: usize,
    /// `alpha[r][j]`: the factor of level `r` (from 1) between the points
    /// `j` and `j + 1` of the level before.
    alpha: [[f64; N]; N],
    /// The same for the hodograph, of degree `degree − 1`.
    hodograph_alpha: [[f64; N]; N],
    /// `degree / (knot difference)` per pair of neighbouring control points.
    hodograph_scale: [f64; N],
}

impl<const N: usize> Along<N> {
    /// For the spline with the given (checked) knots, degree and number of
    /// control points at `t`, clamped to the domain. Without `derivative`
    /// the hodograph's numbers are left at zero.
    fn new(knots: &[f64], degree: usize, count: usize, t: f64, derivative: bool) -> Self {
        let (start, end) = domain_of(knots, degree);
        // NaN compares false both ways and falls through to `start`.
        let t = if t > start { t.min(end) } else { start };
        // The knot span [span, span + 1) holding t; the last one is closed.
        let span = knots
            .partition_point(|&k| k <= t)
            .saturating_sub(1)
            .min(count.saturating_sub(1))
            .max(degree);
        let first = span - degree;
        let mut along = Self {
            degree,
            first,
            alpha: [[0.0; N]; N],
            hodograph_alpha: [[0.0; N]; N],
            hodograph_scale: [0.0; N],
        };
        // The only knots that matter here: `first + 1 ..= span + degree`,
        // `degree` of them up to the span and `degree` after it.
        let (below, above) = knots
            .get(first + 1..)
            .and_then(|k| k.split_at_checked(degree))
            .unwrap_or_default();
        // lo < hi inside a non-empty span; the division is guarded anyway.
        let factor = |lo: f64, hi: f64| if hi > lo { (t - lo) / (hi - lo) } else { 0.0 };
        // Level r blends with the knots `first + r + j` and `span + 1 + j`.
        for (r, level) in along.alpha.iter_mut().enumerate().skip(1) {
            let Some(lo) = below.get(r - 1..) else { break };
            for ((alpha, &lo), &hi) in level.iter_mut().zip(lo).zip(above) {
                *alpha = factor(lo, hi);
            }
        }
        if derivative {
            for ((scale, &lo), &hi) in along.hodograph_scale.iter_mut().zip(below).zip(above) {
                let width = hi - lo;
                // A zero width means the two points have no influence here.
                *scale = if width > 0.0 {
                    degree as f64 / width
                } else {
                    0.0
                };
            }
            // The hodograph's knots are these knots without the first and
            // the last, its span is `span − 1` and its first point is
            // `first`: both domains are [knot(degree), knot(count)], and
            // dropping the first knot moves every index down by one.
            // (Dropping the last knot matters only at the end of a clamped
            // domain, where the span is the last one either way.) So its
            // level r blends with the knots `first + r + j + 1` and
            // `span + 1 + j` of this spline.
            for (r, level) in along.hodograph_alpha.iter_mut().enumerate().skip(1) {
                let Some(lo) = below.get(r..) else { break };
                for ((alpha, &lo), &hi) in level.iter_mut().zip(lo).zip(above) {
                    *alpha = factor(lo, hi);
                }
            }
        }
        along
    }

    /// The `degree + 1` control points with influence, out of one row.
    #[inline]
    fn local(&self, row: &[H]) -> [H; N] {
        let mut d = [ZERO; N];
        let local = row.get(self.first..).unwrap_or_default();
        for (slot, &h) in d.iter_mut().zip(local).take(self.degree + 1) {
            *slot = h;
        }
        d
    }

    /// The hodograph's control points of the local points `d`: `degree` of
    /// them (the derivative of a B-spline is a B-spline of one degree less).
    #[inline]
    fn hodograph(&self, d: &[H; N]) -> [H; N] {
        let mut e = [ZERO; N];
        let pairs = d.iter().zip(d.iter().skip(1)).take(self.degree);
        for ((slot, (a, b)), &scale) in e.iter_mut().zip(pairs).zip(&self.hodograph_scale) {
            *slot = H {
                p: (b.p - a.p) * scale,
                w: (b.w - a.w) * scale,
            };
        }
        e
    }

    /// De Boor's algorithm on the local points: the homogeneous point.
    #[inline]
    fn point(&self, d: [H; N]) -> H {
        reduce(d, &self.alpha, self.degree)
    }

    /// The homogeneous derivative, from the local points' hodograph. For a
    /// degree 1 spline that is the span's single value.
    #[inline]
    fn derivative(&self, e: [H; N]) -> H {
        reduce(e, &self.hodograph_alpha, self.degree.saturating_sub(1))
    }
}

/// De Boor's triangle in place: level `r` replaces the points `r..=degree`
/// by blends of neighbours of the level before; the point is the last one.
#[inline]
fn reduce<const N: usize>(mut d: [H; N], alpha: &[[f64; N]; N], degree: usize) -> H {
    let Some(points) = d.get_mut(..=degree) else {
        return ZERO;
    };
    for (r, level) in alpha.iter().enumerate().skip(1) {
        let Some((before, rest)) = points.get_mut(r - 1..).and_then(<[H]>::split_first_mut) else {
            break;
        };
        let mut before = *before;
        for (slot, &alpha) in rest.iter_mut().zip(level) {
            let here = *slot;
            *slot = before.lerp(here, alpha);
            before = here;
        }
    }
    points.last().copied().unwrap_or(ZERO)
}

fn homogeneous(ctrl: &[Vec3], weights: &[f64]) -> Vec<H> {
    ctrl.iter()
        .zip(weights)
        .map(|(&p, &w)| H { p: p * w, w })
        .collect()
}

fn curve_at<const N: usize>(c: &NurbsCurve, t: f64, derivative: bool) -> (Vec3, Vec3) {
    let along = Along::<N>::new(&c.knots, c.degree, c.homogeneous.len(), t, derivative);
    let d = along.local(&c.homogeneous);
    let da = if derivative {
        along.derivative(along.hodograph(&d))
    } else {
        ZERO
    };
    let a = along.point(d);
    // Weights are positive, so a.w > 0 (convex combination).
    let point = a.p * (1.0 / a.w);
    if !derivative {
        return (point, Vec3::ZERO);
    }
    (point, (da.p - point * da.w) * (1.0 / a.w))
}

fn surface_at<const N: usize>(
    s: &NurbsSurface,
    u: f64,
    v: f64,
    derivative: bool,
) -> (Vec3, Vec3, Vec3) {
    let count_v = s.count_v.max(1);
    let count_u = s.homogeneous.len() / count_v;
    let along_u = Along::<N>::new(&s.knots_u, s.degree_u, count_u, u, derivative);
    let along_v = Along::<N>::new(&s.knots_v, s.degree_v, count_v, v, derivative);
    // Collapse the rows with influence along v, then work along u.
    let mut row_points = [ZERO; N];
    let mut row_derivs = [ZERO; N];
    for ((point, deriv), row) in row_points
        .iter_mut()
        .zip(row_derivs.iter_mut())
        .zip(s.homogeneous.chunks(count_v).skip(along_u.first))
        .take(s.degree_u + 1)
    {
        let d = along_v.local(row);
        if derivative {
            *deriv = along_v.derivative(along_v.hodograph(&d));
        }
        *point = along_v.point(d);
    }
    // The rows are already the local ones: no offset along u any more.
    let a = along_u.point(row_points);
    let point = a.p * (1.0 / a.w);
    if !derivative {
        return (point, Vec3::ZERO, Vec3::ZERO);
    }
    let a_u = along_u.derivative(along_u.hodograph(&row_points));
    let a_v = along_u.point(row_derivs);
    let partial = |d: H| (d.p - point * d.w) * (1.0 / a.w);
    (point, partial(a_u), partial(a_v))
}

/// Picks the working room for a degree: small arrays for the usual low
/// degrees, so clearing them costs next to nothing.
macro_rules! sized_for {
    ($degree:expr, $call:ident($($arg:expr),*)) => {
        match $degree {
            0..=3 => $call::<4>($($arg),*),
            4..=7 => $call::<8>($($arg),*),
            8..=15 => $call::<16>($($arg),*),
            _ => $call::<{ MAX_DEGREE + 1 }>($($arg),*),
        }
    };
}

/// A non-uniform rational B-spline curve.
#[derive(Clone)]
pub struct NurbsCurve {
    degree: usize,
    knots: Vec<f64>,
    ctrl: Vec<Vec3>,
    weights: Vec<f64>,
    /// `ctrl` times `weights`, made once in [`NurbsCurve::new`] (the only
    /// place a curve is made) so evaluation does not rebuild it. Not part of
    /// the curve's value: left out of `==` and of the debug text.
    homogeneous: Vec<H>,
}

impl PartialEq for NurbsCurve {
    fn eq(&self, o: &Self) -> bool {
        self.degree == o.degree
            && self.knots == o.knots
            && self.ctrl == o.ctrl
            && self.weights == o.weights
    }
}

impl fmt::Debug for NurbsCurve {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NurbsCurve")
            .field("degree", &self.degree)
            .field("knots", &self.knots)
            .field("ctrl", &self.ctrl)
            .field("weights", &self.weights)
            .finish()
    }
}

impl NurbsCurve {
    pub fn new(
        degree: usize,
        knots: Vec<f64>,
        ctrl: Vec<Vec3>,
        weights: Vec<f64>,
    ) -> Result<Self, GeomError> {
        check_knots(&knots, degree, ctrl.len())?;
        check_points(&ctrl, &weights)?;
        let homogeneous = homogeneous(&ctrl, &weights);
        Ok(Self {
            degree,
            knots,
            ctrl,
            weights,
            homogeneous,
        })
    }

    /// A polynomial (all weights 1) curve.
    pub fn polynomial(degree: usize, knots: Vec<f64>, ctrl: Vec<Vec3>) -> Result<Self, GeomError> {
        let weights = vec![1.0; ctrl.len()];
        Self::new(degree, knots, ctrl, weights)
    }

    /// The full circle of `radius` in the xy plane of `frame`, starting on
    /// its x axis: the exact rational quadratic with nine control points
    /// (*The NURBS Book* §7.5). Its parameter runs over `[0, 1]` and is not
    /// the angle.
    pub fn circle(frame: &Frame, radius: f64) -> Result<Self, GeomError> {
        if !(radius.is_finite() && radius > 0.0) {
            return Err(GeomError::InvalidParameter(
                "radius must be positive and finite",
            ));
        }
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let square = [
            (1.0, 0.0),
            (1.0, 1.0),
            (0.0, 1.0),
            (-1.0, 1.0),
            (-1.0, 0.0),
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (1.0, 0.0),
        ];
        let ctrl = square
            .iter()
            .map(|&(x, y)| frame.point(x * radius, y * radius, 0.0))
            .collect();
        let weights = vec![1.0, s, 1.0, s, 1.0, s, 1.0, s, 1.0];
        let knots = vec![
            0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
        ];
        Self::new(2, knots, ctrl, weights)
    }

    pub fn degree(&self) -> usize {
        self.degree
    }

    pub fn knots(&self) -> &[f64] {
        &self.knots
    }

    pub fn control_points(&self) -> &[Vec3] {
        &self.ctrl
    }

    pub fn weights(&self) -> &[f64] {
        &self.weights
    }

    /// `(start, end)` of the parameter domain.
    pub fn domain(&self) -> (f64, f64) {
        domain_of(&self.knots, self.degree)
    }

    /// Point at `t`, clamped to the domain.
    pub fn eval(&self, t: f64) -> Vec3 {
        sized_for!(self.degree, curve_at(self, t, false)).0
    }

    /// `(point, first derivative)` at `t`, clamped to the domain.
    pub fn derivatives(&self, t: f64) -> (Vec3, Vec3) {
        sized_for!(self.degree, curve_at(self, t, true))
    }
}

/// A non-uniform rational B-spline surface. Control points are row-major:
/// point `(i, j)` (`i` along u) is at index `i * count_v + j`.
#[derive(Clone)]
pub struct NurbsSurface {
    degree_u: usize,
    degree_v: usize,
    knots_u: Vec<f64>,
    knots_v: Vec<f64>,
    count_v: usize,
    ctrl: Vec<Vec3>,
    weights: Vec<f64>,
    /// `ctrl` times `weights`, made once in [`NurbsSurface::new`] (the only
    /// place a surface is made) so evaluation does not rebuild it. Not part
    /// of the surface's value: left out of `==` and of the debug text.
    homogeneous: Vec<H>,
}

impl PartialEq for NurbsSurface {
    fn eq(&self, o: &Self) -> bool {
        self.degree_u == o.degree_u
            && self.degree_v == o.degree_v
            && self.knots_u == o.knots_u
            && self.knots_v == o.knots_v
            && self.count_v == o.count_v
            && self.ctrl == o.ctrl
            && self.weights == o.weights
    }
}

impl fmt::Debug for NurbsSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NurbsSurface")
            .field("degree_u", &self.degree_u)
            .field("degree_v", &self.degree_v)
            .field("knots_u", &self.knots_u)
            .field("knots_v", &self.knots_v)
            .field("count_v", &self.count_v)
            .field("ctrl", &self.ctrl)
            .field("weights", &self.weights)
            .finish()
    }
}

impl NurbsSurface {
    /// `count_u` rows of `count_v` control points each.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        degree_u: usize,
        degree_v: usize,
        knots_u: Vec<f64>,
        knots_v: Vec<f64>,
        count_u: usize,
        count_v: usize,
        ctrl: Vec<Vec3>,
        weights: Vec<f64>,
    ) -> Result<Self, GeomError> {
        check_knots(&knots_u, degree_u, count_u)?;
        check_knots(&knots_v, degree_v, count_v)?;
        // Both counts are bounded by check_knots, so the product is small.
        if ctrl.len() != count_u * count_v {
            return Err(GeomError::ControlPointCount {
                expected: count_u * count_v,
                actual: ctrl.len(),
            });
        }
        check_points(&ctrl, &weights)?;
        let homogeneous = homogeneous(&ctrl, &weights);
        Ok(Self {
            degree_u,
            degree_v,
            knots_u,
            knots_v,
            count_v,
            ctrl,
            weights,
            homogeneous,
        })
    }

    /// The same surface as a Bézier patch: one knot span per direction.
    pub fn from_bezier(patch: &BezierPatch) -> Result<Self, GeomError> {
        let clamped = |degree: usize| {
            let mut knots = vec![0.0; degree + 1];
            knots.extend(std::iter::repeat_n(1.0, degree + 1));
            knots
        };
        let (du, dv) = (patch.degree_u(), patch.degree_v());
        let ctrl = patch.control_points().to_vec();
        let weights = vec![1.0; ctrl.len()];
        Self::new(
            du,
            dv,
            clamped(du),
            clamped(dv),
            du + 1,
            dv + 1,
            ctrl,
            weights,
        )
    }

    pub fn degrees(&self) -> (usize, usize) {
        (self.degree_u, self.degree_v)
    }

    pub fn knots_u(&self) -> &[f64] {
        &self.knots_u
    }

    pub fn knots_v(&self) -> &[f64] {
        &self.knots_v
    }

    /// `(count_u, count_v)`.
    pub fn counts(&self) -> (usize, usize) {
        (self.ctrl.len() / self.count_v.max(1), self.count_v)
    }

    pub fn control_points(&self) -> &[Vec3] {
        &self.ctrl
    }

    pub fn weights(&self) -> &[f64] {
        &self.weights
    }

    /// `((u start, u end), (v start, v end))`.
    pub fn domain(&self) -> ((f64, f64), (f64, f64)) {
        (
            domain_of(&self.knots_u, self.degree_u),
            domain_of(&self.knots_v, self.degree_v),
        )
    }

    pub fn eval(&self, u: f64, v: f64) -> Vec3 {
        let degree = self.degree_u.max(self.degree_v);
        sized_for!(degree, surface_at(self, u, v, false)).0
    }

    /// `(point, ∂/∂u, ∂/∂v)` at `(u, v)`, clamped to the domain.
    pub fn derivatives(&self, u: f64, v: f64) -> (Vec3, Vec3, Vec3) {
        let degree = self.degree_u.max(self.degree_v);
        sized_for!(degree, surface_at(self, u, v, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3, tol: f64) -> bool {
        a.distance(b) <= tol
    }

    fn wiggle() -> NurbsCurve {
        // Cubic, non-uniform knots, an interior double knot, varied weights.
        let ctrl = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 2.0, 0.5),
            Vec3::new(2.5, -1.0, 1.0),
            Vec3::new(4.0, 0.5, -0.5),
            Vec3::new(5.0, 3.0, 0.0),
            Vec3::new(6.5, 1.0, 2.0),
            Vec3::new(8.0, 0.0, 1.0),
        ];
        let knots = vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.6, 0.6, 1.0, 1.0, 1.0, 1.0];
        let weights = vec![1.0, 2.0, 0.5, 1.0, 3.0, 1.0, 0.7];
        NurbsCurve::new(3, knots, ctrl, weights).unwrap()
    }

    #[test]
    fn a_single_span_spline_is_a_bezier_curve() {
        let p = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(3.0, 2.0, 1.0),
            Vec3::new(4.0, 0.0, 2.0),
        ];
        let curve =
            NurbsCurve::polynomial(3, vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0], p.to_vec())
                .unwrap();
        for t in [0.0, 0.1, 0.5, 0.77, 1.0] {
            let s = 1.0 - t;
            let want = p[0] * (s * s * s)
                + p[1] * (3.0 * s * s * t)
                + p[2] * (3.0 * s * t * t)
                + p[3] * (t * t * t);
            let slope = (p[1] - p[0]) * (3.0 * s * s)
                + (p[2] - p[1]) * (6.0 * s * t)
                + (p[3] - p[2]) * (3.0 * t * t);
            let (point, d) = curve.derivatives(t);
            assert!(close(point, want, 1e-14), "t = {t}");
            assert!(close(d, slope, 1e-13), "t = {t}: {d:?} vs {slope:?}");
        }
        assert_eq!(curve.eval(0.0), p[0]);
        assert_eq!(curve.eval(1.0), p[3]);
    }

    #[test]
    fn the_rational_circle_is_exactly_round() {
        let frame =
            Frame::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 1.0, 1.0), Vec3::X).unwrap();
        for radius in [1e-3, 2.5, 1e6] {
            let circle = NurbsCurve::circle(&frame, radius).unwrap();
            for i in 0..=200 {
                let t = f64::from(i) / 200.0;
                let (p, d) = circle.derivatives(t);
                let (x, y, z) = frame.local(p);
                // Rounding is relative to the coordinates, which include
                // the frame's origin (about 4 units from the world origin).
                let eps = 1e-14 * (radius + 4.0);
                assert!((x.hypot(y) - radius).abs() <= eps, "t = {t}");
                assert!(z.abs() <= eps);
                // Tangent is perpendicular to the radius and never vanishes.
                assert!(d.dot(p - frame.origin()).abs() <= 1e-12 * (radius + 4.0) * d.length());
                assert!(d.length() > radius);
            }
            // Quarter points land on the axes, counter-clockwise.
            let eps = 1e-14 * (radius + 4.0);
            assert!(close(circle.eval(0.0), frame.point(radius, 0.0, 0.0), eps));
            assert!(close(circle.eval(0.25), frame.point(0.0, radius, 0.0), eps));
            assert!(close(circle.eval(0.5), frame.point(-radius, 0.0, 0.0), eps));
        }
    }

    #[test]
    fn derivatives_match_central_differences() {
        let curve = wiggle();
        let h = 1e-6;
        for t in [0.05, 0.2, 0.31, 0.45, 0.59, 0.61, 0.8, 0.95] {
            let (p, d) = curve.derivatives(t);
            assert_eq!(p, curve.eval(t));
            let fd = (curve.eval(t + h) - curve.eval(t - h)) * (0.5 / h);
            assert!(close(d, fd, 1e-7), "t = {t}: {d:?} vs {fd:?}");
        }
        // One-sided at the start of the domain, with the second-order
        // forward difference (-3 f(0) + 4 f(h) - f(2h)) / 2h.
        let (p0, d0) = curve.derivatives(0.0);
        // Its error is h² times a third of the third derivative, which for
        // this curve is in the tens of thousands near the start: h is small.
        let h = 1e-6;
        let forward = (curve.eval(h) * 4.0 - p0 * 3.0 - curve.eval(2.0 * h)) * (0.5 / h);
        assert!(close(d0, forward, 1e-6), "{d0:?} vs {forward:?}");
    }

    #[test]
    fn evaluation_is_continuous_across_knots_and_clamped_outside() {
        let curve = wiggle();
        for knot in [0.3, 0.6] {
            assert!(close(
                curve.eval(knot - 1e-12),
                curve.eval(knot + 1e-12),
                1e-9
            ));
        }
        assert_eq!(curve.eval(-5.0), curve.eval(0.0));
        assert_eq!(curve.eval(7.0), curve.eval(1.0));
        assert_eq!(curve.eval(f64::NAN), curve.eval(0.0));
        assert_eq!(curve.eval(1.0), *curve.control_points().last().unwrap());
        assert_eq!(curve.domain(), (0.0, 1.0));
    }

    #[test]
    fn affine_maps_of_the_control_points_map_the_curve() {
        // Partition of unity: translating and scaling the net does the same
        // to every point of the curve.
        let curve = wiggle();
        let map = |p: Vec3| p * 3.0 + Vec3::new(10.0, -20.0, 5.0);
        let moved = NurbsCurve::new(
            3,
            curve.knots().to_vec(),
            curve.control_points().iter().map(|&p| map(p)).collect(),
            curve.weights().to_vec(),
        )
        .unwrap();
        for i in 0..=50 {
            let t = f64::from(i) / 50.0;
            assert!(close(moved.eval(t), map(curve.eval(t)), 1e-12));
        }
    }

    #[test]
    fn degree_one_is_a_polyline() {
        let pts = vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 2.0, 0.0)];
        let line = NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 3.0, 3.0], pts).unwrap();
        assert!(close(line.eval(0.5), Vec3::new(0.5, 0.0, 0.0), 1e-15));
        assert!(close(line.eval(2.0), Vec3::new(1.0, 1.0, 0.0), 1e-15));
        assert!(close(line.derivatives(0.5).1, Vec3::X, 1e-15));
        assert!(close(line.derivatives(2.0).1, Vec3::Y, 1e-15));
    }

    #[test]
    fn a_surface_from_a_bezier_patch_is_the_same_surface() {
        let mut net = [[Vec3::ZERO; 4]; 4];
        for (i, row) in net.iter_mut().enumerate() {
            for (j, p) in row.iter_mut().enumerate() {
                *p = Vec3::new(i as f64, j as f64 * 1.5, ((i * 7 + j * 3) % 5) as f64 * 0.4);
            }
        }
        let patch = BezierPatch::bicubic(net).unwrap();
        let surface = NurbsSurface::from_bezier(&patch).unwrap();
        assert_eq!(surface.counts(), (4, 4));
        assert_eq!(surface.domain(), ((0.0, 1.0), (0.0, 1.0)));
        for (u, v) in [(0.0, 0.0), (0.3, 0.9), (0.5, 0.5), (1.0, 0.2), (1.0, 1.0)] {
            let (p, du, dv) = surface.derivatives(u, v);
            let (bp, bdu, bdv) = patch.derivatives(u, v);
            assert!(
                close(p, bp, 1e-13) && close(du, bdu, 1e-12) && close(dv, bdv, 1e-12),
                "({u}, {v})"
            );
        }
    }

    #[test]
    fn a_rational_surface_reproduces_a_cylinder() {
        // The NURBS circle swept along z: degree 2 × 1, 9 × 2 control points.
        let circle = NurbsCurve::circle(&Frame::WORLD, 2.0).unwrap();
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for (&p, &w) in circle.control_points().iter().zip(circle.weights()) {
            ctrl.extend([p, p + Vec3::Z * 5.0]);
            weights.extend([w, w]);
        }
        let surface = NurbsSurface::new(
            2,
            1,
            circle.knots().to_vec(),
            vec![0.0, 0.0, 1.0, 1.0],
            9,
            2,
            ctrl,
            weights,
        )
        .unwrap();
        let h = 1e-6;
        for i in 0..=20 {
            for j in 0..=4 {
                let (u, v) = (f64::from(i) / 20.0, f64::from(j) / 4.0);
                let (p, du, dv) = surface.derivatives(u, v);
                assert!((p.x.hypot(p.y) - 2.0).abs() < 1e-14 && (p.z - 5.0 * v).abs() < 1e-14);
                assert!(close(dv, Vec3::Z * 5.0, 1e-13));
                // Outward normal: du × dv points away from the axis.
                assert!(du.cross(dv).dot(Vec3::new(p.x, p.y, 0.0)) > 0.0);
                // Not at the knots 0.25, 0.5, 0.75: the circle is only C1
                // there, and a difference across a knot is first order.
                if i % 5 != 0 {
                    let fd = (surface.eval(u + h, v) - surface.eval(u - h, v)) * (0.5 / h);
                    assert!(close(du, fd, 1e-6), "({u}, {v})");
                }
            }
        }
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        let pts = |n: usize| vec![Vec3::X; n];
        let bad = |degree, knots: Vec<f64>, ctrl: Vec<Vec3>, weights: Vec<f64>| {
            matches!(
                NurbsCurve::new(degree, knots, ctrl, weights),
                Err(GeomError::InvalidParameter(_))
            )
        };
        assert!(bad(0, vec![0.0, 1.0], pts(1), vec![1.0]), "degree 0");
        assert!(
            bad(MAX_DEGREE + 1, vec![], pts(2), vec![1.0; 2]),
            "degree too high"
        );
        assert!(
            bad(2, vec![0.0, 0.0, 1.0, 1.0], pts(2), vec![1.0; 2]),
            "too few points"
        );
        assert!(
            bad(1, vec![0.0, 0.0, 1.0], pts(2), vec![1.0; 2]),
            "knot count"
        );
        assert!(
            bad(1, vec![0.0, 1.0, 0.5, 2.0], pts(2), vec![1.0; 2]),
            "decreasing knots"
        );
        assert!(
            bad(1, vec![0.0, f64::NAN, 1.0, 1.0], pts(2), vec![1.0; 2]),
            "NaN knot"
        );
        assert!(
            bad(1, vec![0.0, 1.0, 1.0, 2.0], pts(2), vec![1.0; 2]),
            "empty domain"
        );
        assert!(
            bad(1, vec![0.0, 0.0, 1.0, 1.0], pts(2), vec![1.0]),
            "weight count"
        );
        assert!(
            bad(1, vec![0.0, 0.0, 1.0, 1.0], pts(2), vec![1.0, 0.0]),
            "zero weight"
        );
        assert!(
            bad(1, vec![0.0, 0.0, 1.0, 1.0], pts(2), vec![1.0, -2.0]),
            "negative weight"
        );
        assert!(matches!(
            NurbsCurve::polynomial(
                1,
                vec![0.0, 0.0, 1.0, 1.0],
                vec![Vec3::X, Vec3::new(f64::INFINITY, 0.0, 0.0)]
            ),
            Err(GeomError::NonFiniteControlPoint { index: 1 })
        ));
        assert!(NurbsCurve::circle(&Frame::WORLD, 0.0).is_err());
        assert!(NurbsCurve::circle(&Frame::WORLD, f64::NAN).is_err());
        assert!(matches!(
            NurbsSurface::new(
                1,
                1,
                vec![0.0, 0.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                2,
                2,
                pts(3),
                vec![1.0; 3]
            ),
            Err(GeomError::ControlPointCount {
                expected: 4,
                actual: 3
            })
        ));
    }
}
