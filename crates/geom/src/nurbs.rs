//! NURBS curves and surfaces.
//!
//! Evaluation is de Boor's algorithm on homogeneous control points;
//! derivatives come from the hodograph (the derivative of a B-spline is a
//! B-spline of one degree less) and the quotient rule. Reference: Piegl &
//! Tiller, *The NURBS Book*, 2nd ed., ch. 2–4 and §7.5 (circles).

use crate::{BezierPatch, GeomError, MAX_DEGREE};
use craft3d_math::{Frame, Vec3};

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

/// De Boor's algorithm: the point at `t` of the B-spline with the given
/// knots, degree and homogeneous control points. `t` is clamped to the
/// domain. The inputs satisfy `check_knots`.
fn de_boor(knots: &[f64], degree: usize, ctrl: &[H], t: f64) -> H {
    let zero = H {
        p: Vec3::ZERO,
        w: 0.0,
    };
    let knot = |i: usize| knots.get(i).copied().unwrap_or(0.0);
    let (start, end) = domain_of(knots, degree);
    // NaN compares false both ways and falls through to `start`.
    let t = if t > start { t.min(end) } else { start };
    // The knot span [span, span + 1) holding t; the last one is closed.
    let span = knots
        .partition_point(|&k| k <= t)
        .saturating_sub(1)
        .clamp(degree, ctrl.len().saturating_sub(1));
    let first = span - degree;
    let mut d: Vec<H> = ctrl.iter().skip(first).take(degree + 1).copied().collect();
    for r in 1..=degree {
        d = d
            .iter()
            .zip(d.iter().skip(1))
            .enumerate()
            .map(|(j, (&a, &b))| {
                let (lo, hi) = (knot(first + r + j), knot(span + 1 + j));
                // lo < hi inside a non-empty span; guard the division anyway.
                let alpha = if hi > lo { (t - lo) / (hi - lo) } else { 0.0 };
                a.lerp(b, alpha)
            })
            .collect();
    }
    d.first().copied().unwrap_or(zero)
}

/// Knots and control points of the derivative spline (degree − 1).
fn hodograph(knots: &[f64], degree: usize, ctrl: &[H]) -> (Vec<f64>, Vec<H>) {
    let knot = |i: usize| knots.get(i).copied().unwrap_or(0.0);
    let points = ctrl
        .iter()
        .zip(ctrl.iter().skip(1))
        .enumerate()
        .map(|(i, (a, b))| {
            let span = knot(i + degree + 1) - knot(i + 1);
            // A zero span means the two points have no influence here.
            let scale = if span > 0.0 {
                degree as f64 / span
            } else {
                0.0
            };
            H {
                p: (b.p - a.p) * scale,
                w: (b.w - a.w) * scale,
            }
        })
        .collect();
    let inner = knots
        .iter()
        .skip(1)
        .take(knots.len().saturating_sub(2))
        .copied()
        .collect();
    (inner, points)
}

/// The homogeneous derivative at `t`; zero for degree 1 splines' missing
/// lower degree is handled by the caller never passing degree 0.
fn de_boor_derivative(knots: &[f64], degree: usize, ctrl: &[H], t: f64) -> H {
    let (inner, points) = hodograph(knots, degree, ctrl);
    if degree == 1 {
        // Degree 0: piecewise constant; pick the span's single value.
        let (start, end) = domain_of(knots, degree);
        let t = if t > start { t.min(end) } else { start };
        let span = knots
            .partition_point(|&k| k <= t)
            .saturating_sub(1)
            .clamp(1, ctrl.len().saturating_sub(1));
        return points.get(span - 1).copied().unwrap_or(H {
            p: Vec3::ZERO,
            w: 0.0,
        });
    }
    de_boor(&inner, degree - 1, &points, t)
}

fn homogeneous(ctrl: &[Vec3], weights: &[f64]) -> Vec<H> {
    ctrl.iter()
        .zip(weights)
        .map(|(&p, &w)| H { p: p * w, w })
        .collect()
}

/// A non-uniform rational B-spline curve.
#[derive(Clone, Debug, PartialEq)]
pub struct NurbsCurve {
    degree: usize,
    knots: Vec<f64>,
    ctrl: Vec<Vec3>,
    weights: Vec<f64>,
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
        Ok(Self {
            degree,
            knots,
            ctrl,
            weights,
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
        self.derivatives(t).0
    }

    /// `(point, first derivative)` at `t`, clamped to the domain.
    pub fn derivatives(&self, t: f64) -> (Vec3, Vec3) {
        let h = homogeneous(&self.ctrl, &self.weights);
        let a = de_boor(&self.knots, self.degree, &h, t);
        let da = de_boor_derivative(&self.knots, self.degree, &h, t);
        // Weights are positive, so a.w > 0 (convex combination).
        let point = a.p * (1.0 / a.w);
        (point, (da.p - point * da.w) * (1.0 / a.w))
    }
}

/// A non-uniform rational B-spline surface. Control points are row-major:
/// point `(i, j)` (`i` along u) is at index `i * count_v + j`.
#[derive(Clone, Debug, PartialEq)]
pub struct NurbsSurface {
    degree_u: usize,
    degree_v: usize,
    knots_u: Vec<f64>,
    knots_v: Vec<f64>,
    count_v: usize,
    ctrl: Vec<Vec3>,
    weights: Vec<f64>,
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
        Ok(Self {
            degree_u,
            degree_v,
            knots_u,
            knots_v,
            count_v,
            ctrl,
            weights,
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
        self.derivatives(u, v).0
    }

    /// `(point, ∂/∂u, ∂/∂v)` at `(u, v)`, clamped to the domain.
    pub fn derivatives(&self, u: f64, v: f64) -> (Vec3, Vec3, Vec3) {
        let h = homogeneous(&self.ctrl, &self.weights);
        // Collapse every row along v, then work along u.
        let mut row_points = Vec::new();
        let mut row_derivs = Vec::new();
        for row in h.chunks(self.count_v.max(1)) {
            row_points.push(de_boor(&self.knots_v, self.degree_v, row, v));
            row_derivs.push(de_boor_derivative(&self.knots_v, self.degree_v, row, v));
        }
        let a = de_boor(&self.knots_u, self.degree_u, &row_points, u);
        let a_u = de_boor_derivative(&self.knots_u, self.degree_u, &row_points, u);
        let a_v = de_boor(&self.knots_u, self.degree_u, &row_derivs, u);
        let point = a.p * (1.0 / a.w);
        let partial = |d: H| (d.p - point * d.w) * (1.0 / a.w);
        (point, partial(a_u), partial(a_v))
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
