//! Curves and surfaces for 3DCraft: the [`Curve3`] and [`Surface`] enums,
//! their analytic kinds, NURBS, and (in this file) the tensor-product Bézier
//! patch.
//! Reference: Piegl & Tiller, *The NURBS Book*, 2nd ed., ch. 1 (Bernstein
//! polynomials, Bézier surfaces).

mod curve;
pub mod intersect;
mod nurbs;
mod surface;
mod transform;

pub use curve::{Circle, Curve3, Ellipse, Line};
pub use nurbs::{
    BezierSpan, DerivativeBounds, MAX_CONTROL_POINTS, NurbsCurve, NurbsSurface, RationalBezierPatch,
};
pub use surface::{Cone, Cylinder, Plane, Sphere, Surface, Torus};
pub use transform::{ISOMETRY_EPSILON, MirrorRule};

use craft3d_math::{Bbox3, Vec3};
use std::fmt;

/// Highest polynomial degree accepted per direction. Caps the allocation an
/// input-derived degree can cause.
pub const MAX_DEGREE: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub enum GeomError {
    DegreeTooHigh {
        degree: usize,
        max: usize,
    },
    ControlPointCount {
        expected: usize,
        actual: usize,
    },
    NonFiniteControlPoint {
        index: usize,
    },
    /// A dimension, direction, knot vector or weight that cannot describe a
    /// curve or surface; the text says which rule was broken.
    InvalidParameter(&'static str),
    /// Valid input that this kind of geometry cannot handle yet.
    Unsupported(&'static str),
    /// A numeric method ran into one of its caps (iterations, points,
    /// subdivisions) before it reached the tolerance; the text says which.
    /// Nothing partial is returned.
    NoConvergence(&'static str),
}

impl fmt::Display for GeomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeomError::DegreeTooHigh { degree, max } => {
                write!(f, "degree {degree} is above the supported maximum {max}")
            }
            GeomError::ControlPointCount { expected, actual } => write!(
                f,
                "a patch of this degree needs {expected} control points, got {actual}"
            ),
            GeomError::NonFiniteControlPoint { index } => {
                write!(f, "control point {index} is not finite (NaN or infinity)")
            }
            GeomError::InvalidParameter(rule) => write!(f, "invalid geometry: {rule}"),
            GeomError::Unsupported(what) => write!(f, "not supported yet: {what}"),
            GeomError::NoConvergence(what) => write!(f, "did not converge: {what}"),
        }
    }
}

impl std::error::Error for GeomError {}

/// The values of a basis of degree at most [`MAX_DEGREE`], on the stack:
/// a slice of `degree + 1` numbers.
struct Basis {
    values: [f64; MAX_DEGREE + 1],
    len: usize,
}

impl std::ops::Deref for Basis {
    type Target = [f64];

    fn deref(&self) -> &[f64] {
        self.values.get(..self.len).unwrap_or(&self.values)
    }
}

/// Bernstein basis of degree `n` (at most [`MAX_DEGREE`]) at `t`: `n + 1`
/// values that sum to 1.
fn bernstein(n: usize, t: f64) -> Basis {
    let n = n.min(MAX_DEGREE);
    let mut b = [0.0; MAX_DEGREE + 1];
    if let Some(first) = b.first_mut() {
        *first = 1.0;
    }
    // Raise the degree in place: value k of degree m + 1 blends the values
    // k − 1 and k of degree m, and the new last one has only a left parent.
    for m in 0..n {
        let mut prev = 0.0;
        let mut values = b.iter_mut();
        for slot in values.by_ref().take(m + 1) {
            let c = *slot;
            *slot = t * prev + (1.0 - t) * c;
            prev = c;
        }
        if let Some(last) = values.next() {
            *last = t * prev;
        }
    }
    Basis {
        values: b,
        len: n + 1,
    }
}

/// First derivative of the Bernstein basis of degree `n` at `t`: `n + 1`
/// values.
fn bernstein_derivative(n: usize, t: f64) -> Basis {
    let n = n.min(MAX_DEGREE);
    let mut d = [0.0; MAX_DEGREE + 1];
    if let Some(lower_degree) = n.checked_sub(1) {
        let lower = bernstein(lower_degree, t);
        let scale = n as f64;
        let mut prev = 0.0;
        let mut values = d.iter_mut();
        for (&c, slot) in lower.iter().zip(values.by_ref()) {
            *slot = scale * (prev - c);
            prev = c;
        }
        if let Some(last) = values.next() {
            *last = scale * prev;
        }
    }
    Basis {
        values: d,
        len: n + 1,
    }
}

/// A polynomial tensor-product Bézier patch over `[0, 1] × [0, 1]`.
///
/// Control points are stored row-major: point `(i, j)` (`i` along u, `j`
/// along v) is at index `i * (degree_v + 1) + j`.
#[derive(Clone, Debug, PartialEq)]
pub struct BezierPatch {
    degree_u: usize,
    degree_v: usize,
    ctrl: Vec<Vec3>,
}

impl BezierPatch {
    pub fn new(degree_u: usize, degree_v: usize, ctrl: Vec<Vec3>) -> Result<Self, GeomError> {
        for degree in [degree_u, degree_v] {
            if degree > MAX_DEGREE {
                return Err(GeomError::DegreeTooHigh {
                    degree,
                    max: MAX_DEGREE,
                });
            }
        }
        let expected = (degree_u + 1) * (degree_v + 1);
        if ctrl.len() != expected {
            return Err(GeomError::ControlPointCount {
                expected,
                actual: ctrl.len(),
            });
        }
        if let Some(index) = ctrl.iter().position(|p| !p.is_finite()) {
            return Err(GeomError::NonFiniteControlPoint { index });
        }
        Ok(Self {
            degree_u,
            degree_v,
            ctrl,
        })
    }

    /// A bicubic patch from a 4 × 4 net, `net[i][j]` with `i` along u.
    pub fn bicubic(net: [[Vec3; 4]; 4]) -> Result<Self, GeomError> {
        Self::new(3, 3, net.into_iter().flatten().collect())
    }

    pub fn degree_u(&self) -> usize {
        self.degree_u
    }

    pub fn degree_v(&self) -> usize {
        self.degree_v
    }

    pub fn control_points(&self) -> &[Vec3] {
        &self.ctrl
    }

    pub fn control_point(&self, i: usize, j: usize) -> Option<Vec3> {
        if i > self.degree_u || j > self.degree_v {
            return None;
        }
        self.ctrl.get(i * (self.degree_v + 1) + j).copied()
    }

    /// Bounding box of the control net. The surface lies inside it (convex
    /// hull property).
    pub fn control_bbox(&self) -> Bbox3 {
        Bbox3::from_points(self.ctrl.iter().copied())
    }

    /// Point at `(u, v)`. Parameters outside `[0, 1]` extrapolate the
    /// polynomial; non-finite parameters give a non-finite point.
    pub fn eval(&self, u: f64, v: f64) -> Vec3 {
        let bu = bernstein(self.degree_u, u);
        let bv = bernstein(self.degree_v, v);
        // Sum of `bu[i] * bv[j] * P(i, j)`, row by row.
        let mut sum = Vec3::ZERO;
        for (row, &a) in self.ctrl.chunks(self.degree_v + 1).zip(bu.iter()) {
            for (&p, &b) in row.iter().zip(bv.iter()) {
                sum += p * (a * b);
            }
        }
        sum
    }

    /// `(point, ∂/∂u, ∂/∂v)` at `(u, v)`.
    pub fn derivatives(&self, u: f64, v: f64) -> (Vec3, Vec3, Vec3) {
        let bu = bernstein(self.degree_u, u);
        let bv = bernstein(self.degree_v, v);
        let du = bernstein_derivative(self.degree_u, u);
        let dv = bernstein_derivative(self.degree_v, v);
        // Three sums over the net in one pass; each adds its terms in the
        // order it would have on its own.
        let (mut point, mut along_u, mut along_v) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
        for (row, (&a, &da)) in self
            .ctrl
            .chunks(self.degree_v + 1)
            .zip(bu.iter().zip(du.iter()))
        {
            for (&p, (&b, &db)) in row.iter().zip(bv.iter().zip(dv.iter())) {
                point += p * (a * b);
                along_u += p * (da * b);
                along_v += p * (a * db);
            }
        }
        (point, along_u, along_v)
    }

    /// Unit normal `∂u × ∂v` at `(u, v)`.
    ///
    /// Where the patch is degenerate (a collapsed edge, as at the top of the
    /// teapot lid) the cross product vanishes; the normal is then taken a
    /// little towards the middle of the patch, which is its limit there.
    /// `None` when no normal exists nearby (a patch collapsed to a curve).
    pub fn normal(&self, u: f64, v: f64) -> Option<Vec3> {
        // Where an edge is collapsed, the tangent along it is rounding noise
        // rather than exactly zero, and normalising noise gives a garbage
        // direction. Tangents are of the order of the control net's size, so
        // a cross product far below size² is treated as zero.
        let size = self.control_bbox().diagonal();
        let floor = 1e-9 * size * size;
        let at = |u: f64, v: f64| {
            let (_, du, dv) = self.derivatives(u, v);
            let cross = du.cross(dv);
            if cross.length() > floor {
                cross.normalized()
            } else {
                None
            }
        };
        if let Some(n) = at(u, v) {
            return Some(n);
        }
        [1e-6, 1e-4, 1e-2]
            .into_iter()
            .find_map(|eps| at(u + (0.5 - u) * eps, v + (0.5 - v) * eps))
    }

    /// The same surface with u and v exchanged. Flips the normal.
    pub fn transposed(&self) -> Self {
        let mut ctrl = Vec::with_capacity(self.ctrl.len());
        for j in 0..=self.degree_v {
            for i in 0..=self.degree_u {
                ctrl.extend(self.control_point(i, j));
            }
        }
        Self {
            degree_u: self.degree_v,
            degree_v: self.degree_u,
            ctrl,
        }
    }

    /// The same surface with u running the other way. Flips the normal.
    pub fn reversed_u(&self) -> Self {
        let ctrl = self
            .ctrl
            .chunks(self.degree_v + 1)
            .rev()
            .flatten()
            .copied()
            .collect();
        Self {
            degree_u: self.degree_u,
            degree_v: self.degree_v,
            ctrl,
        }
    }

    /// The patch with every control point mapped by `f` (exact for affine
    /// maps). Fails when `f` produces a non-finite point.
    pub fn map_points(&self, f: impl Fn(Vec3) -> Vec3) -> Result<Self, GeomError> {
        Self::new(
            self.degree_u,
            self.degree_v,
            self.ctrl.iter().map(|&p| f(p)).collect(),
        )
    }

    /// Upper bounds `(Muu, Muv, Mvv)` on the lengths of the second partial
    /// derivatives over the whole patch, from differences of the control net:
    /// each derivative of a Bézier patch is a Bézier patch whose control
    /// points are scaled differences, and it lies in their convex hull.
    pub fn second_derivative_bounds(&self) -> (f64, f64, f64) {
        let p = |i: usize, j: usize| self.control_point(i, j).unwrap_or(Vec3::ZERO);
        let (m, n) = (self.degree_u, self.degree_v);
        let mut uu = 0.0_f64;
        let mut uv = 0.0_f64;
        let mut vv = 0.0_f64;
        for i in 0..=m {
            for j in 0..=n {
                if i + 2 <= m {
                    uu = uu.max((p(i + 2, j) - p(i + 1, j) * 2.0 + p(i, j)).length());
                }
                if j + 2 <= n {
                    vv = vv.max((p(i, j + 2) - p(i, j + 1) * 2.0 + p(i, j)).length());
                }
                if i < m && j < n {
                    uv = uv.max((p(i + 1, j + 1) - p(i + 1, j) - p(i, j + 1) + p(i, j)).length());
                }
            }
        }
        let (mf, nf) = (m as f64, n as f64);
        (
            mf * (mf - 1.0).max(0.0) * uu,
            mf * nf * uv,
            nf * (nf - 1.0).max(0.0) * vv,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3, tol: f64) -> bool {
        a.distance(b) <= tol
    }

    /// A bicubic patch with a bump: not planar, no symmetry to hide mistakes.
    fn bumpy() -> BezierPatch {
        let mut net = [[Vec3::ZERO; 4]; 4];
        for (i, row) in net.iter_mut().enumerate() {
            for (j, p) in row.iter_mut().enumerate() {
                let (x, y) = (i as f64, j as f64 * 1.5);
                let z = ((i * 7 + j * 3) % 5) as f64 * 0.4 - 0.3 * x;
                *p = Vec3::new(x + 0.1 * y, y, z);
            }
        }
        BezierPatch::bicubic(net).unwrap()
    }

    #[test]
    fn bernstein_sums_to_one_and_matches_cubic_formula() {
        for n in 0..8 {
            for t in [0.0, 0.2, 0.5, 0.9, 1.0] {
                let b = bernstein(n, t);
                assert_eq!(b.len(), n + 1);
                assert!((b.iter().sum::<f64>() - 1.0).abs() < 1e-14);
                // Derivatives of a partition of unity sum to zero.
                assert!(bernstein_derivative(n, t).iter().sum::<f64>().abs() < 1e-12);
            }
        }
        let t: f64 = 0.3;
        let s = 1.0 - t;
        let expected = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
        for (got, want) in bernstein(3, t).iter().zip(expected) {
            assert!((got - want).abs() < 1e-15);
        }
    }

    #[test]
    fn corners_interpolate_the_corner_control_points() {
        let p = bumpy();
        assert_eq!(p.eval(0.0, 0.0), p.control_point(0, 0).unwrap());
        assert_eq!(p.eval(1.0, 0.0), p.control_point(3, 0).unwrap());
        assert_eq!(p.eval(0.0, 1.0), p.control_point(0, 3).unwrap());
        assert_eq!(p.eval(1.0, 1.0), p.control_point(3, 3).unwrap());
    }

    #[test]
    fn a_planar_net_reproduces_the_plane_exactly() {
        // Control points on a regular grid of the plane z = 2x - y + 1:
        // linear precision means S(u, v) = (3u, 3v, 6u - 3v + 1).
        let mut net = [[Vec3::ZERO; 4]; 4];
        for (i, row) in net.iter_mut().enumerate() {
            for (j, p) in row.iter_mut().enumerate() {
                let (x, y) = (i as f64, j as f64);
                *p = Vec3::new(x, y, 2.0 * x - y + 1.0);
            }
        }
        let patch = BezierPatch::bicubic(net).unwrap();
        for (u, v) in [(0.0, 0.0), (0.25, 0.7), (0.5, 0.5), (1.0, 0.1)] {
            let want = Vec3::new(3.0 * u, 3.0 * v, 6.0 * u - 3.0 * v + 1.0);
            assert!(close(patch.eval(u, v), want, 1e-13));
            let n = patch.normal(u, v).unwrap();
            let plane_normal = Vec3::new(-2.0, 1.0, 1.0).normalized().unwrap();
            assert!(close(n, plane_normal, 1e-13));
        }
        assert_eq!(patch.second_derivative_bounds(), (0.0, 0.0, 0.0));
    }

    #[test]
    fn derivatives_match_central_differences() {
        let p = bumpy();
        let h = 1e-6;
        for (u, v) in [(0.1, 0.2), (0.5, 0.5), (0.83, 0.37)] {
            let (pt, du, dv) = p.derivatives(u, v);
            assert_eq!(pt, p.eval(u, v));
            let fd_u = (p.eval(u + h, v) - p.eval(u - h, v)) * (0.5 / h);
            let fd_v = (p.eval(u, v + h) - p.eval(u, v - h)) * (0.5 / h);
            assert!(close(du, fd_u, 1e-8), "du {du:?} vs {fd_u:?}");
            assert!(close(dv, fd_v, 1e-8), "dv {dv:?} vs {fd_v:?}");
        }
    }

    #[test]
    fn second_derivative_bounds_hold_at_samples() {
        let p = bumpy();
        let (muu, muv, mvv) = p.second_derivative_bounds();
        let h = 1e-4;
        for a in 1..10 {
            for b in 1..10 {
                let (u, v) = (a as f64 / 10.0, b as f64 / 10.0);
                let suu =
                    (p.eval(u + h, v) - p.eval(u, v) * 2.0 + p.eval(u - h, v)) * (1.0 / h / h);
                let svv =
                    (p.eval(u, v + h) - p.eval(u, v) * 2.0 + p.eval(u, v - h)) * (1.0 / h / h);
                let suv = (p.eval(u + h, v + h) - p.eval(u + h, v - h) - p.eval(u - h, v + h)
                    + p.eval(u - h, v - h))
                    * (0.25 / h / h);
                assert!(suu.length() <= muu + 1e-4);
                assert!(svv.length() <= mvv + 1e-4);
                assert!(suv.length() <= muv + 1e-4);
            }
        }
    }

    #[test]
    fn transposing_and_reversing_keep_the_surface_and_flip_the_normal() {
        let p = bumpy();
        let t = p.transposed();
        let r = p.reversed_u();
        for (u, v) in [(0.0, 1.0), (0.3, 0.8), (0.6, 0.1)] {
            assert!(close(t.eval(v, u), p.eval(u, v), 1e-13));
            assert!(close(r.eval(1.0 - u, v), p.eval(u, v), 1e-13));
            let n = p.normal(u, v).unwrap();
            assert!(close(t.normal(v, u).unwrap(), -n, 1e-12));
            assert!(close(r.normal(1.0 - u, v).unwrap(), -n, 1e-12));
        }
        assert_eq!(t.transposed(), p);
        assert_eq!(r.reversed_u(), p);
    }

    #[test]
    fn a_collapsed_edge_still_has_a_normal() {
        // A cone-like patch: the whole u = 0 edge is the apex.
        let apex = Vec3::new(0.0, 0.0, 1.0);
        let mut net = [[apex; 4]; 4];
        for (i, row) in net.iter_mut().enumerate().skip(1) {
            for (j, p) in row.iter_mut().enumerate() {
                let r = i as f64;
                *p = Vec3::new(r, j as f64 - 1.5, 1.0 - r);
            }
        }
        let patch = BezierPatch::bicubic(net).unwrap();
        let (_, du, dv) = patch.derivatives(0.0, 0.5);
        assert!(
            du.cross(dv).normalized().is_none(),
            "expected a degenerate point"
        );
        let n = patch.normal(0.0, 0.5).unwrap();
        assert!((n.length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert_eq!(
            BezierPatch::new(3, 3, vec![Vec3::ZERO; 15]),
            Err(GeomError::ControlPointCount {
                expected: 16,
                actual: 15
            })
        );
        assert!(matches!(
            BezierPatch::new(usize::MAX, 1, vec![]),
            Err(GeomError::DegreeTooHigh { .. })
        ));
        assert_eq!(
            BezierPatch::new(0, 0, vec![Vec3::new(0.0, f64::NAN, 0.0)]),
            Err(GeomError::NonFiniteControlPoint { index: 0 })
        );
        // A degree-0 patch is a point: no normal anywhere, and no panic.
        let point = BezierPatch::new(0, 0, vec![Vec3::X]).unwrap();
        assert_eq!(point.eval(0.4, 0.9), Vec3::X);
        assert!(point.normal(0.5, 0.5).is_none());
        // Non-finite parameters do not panic.
        assert!(!bumpy().eval(f64::NAN, 0.5).is_finite());
    }
}
