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

use crate::surface::GridCache;
use crate::{BezierPatch, GeomError, MAX_DEGREE};
use craft3d_math::{Bbox3, Bits, Frame, Vec3};
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
#[derive(Clone, Debug)]
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
    /// control points at `t` (beyond the domain: the end span continued).
    /// Without `derivative`
    /// the hodograph's numbers are left at zero.
    fn new(knots: &[f64], degree: usize, count: usize, t: f64, derivative: bool) -> Self {
        let (start, end) = domain_of(knots, degree);
        // NaN compares false both ways and falls through to `start`.
        let inside = if t > start { t.min(end) } else { start };
        // Beyond the domain the end span continues as the polynomial it
        // is, as a Bézier patch continues: the span is the end one, the
        // blend factors are taken at the parameter itself (NaN: the start).
        let t = if t.is_nan() { start } else { t };
        // The knot span [span, span + 1) holding t; the last one is closed.
        let span = knots
            .partition_point(|&k| k <= inside)
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
    crate::allowance::charge(1);
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
    surface_with(s, &along_u, &along_v, derivative)
}

/// [`surface_at`] with the blend factors of both directions given.
fn surface_with<const N: usize>(
    s: &NurbsSurface,
    along_u: &Along<N>,
    along_v: &Along<N>,
    derivative: bool,
) -> (Vec3, Vec3, Vec3) {
    // One evaluation (`crate::allowance`), also with the blend factors
    // given (`derivatives_cached`).
    crate::allowance::charge(1);
    let count_v = s.count_v.max(1);
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

/// How many blend factors along each direction a [`DerivativeCache`]
/// keeps.
const CACHED_ALONG: usize = 3;

/// The blend factors of the last few parameters a NURBS surface of degree
/// three or less was evaluated at, along each direction: a walk that
/// evaluates the surface at `(u ± h, v)` and `(u, v ± h)` around each
/// step (the curving of the surface from differences of its tangents)
/// works them out for three values of each parameter, not five. For one
/// surface only (the caller's: [`NurbsSurface::derivatives_cached`]); what
/// it keeps changes no value, only the work.
#[derive(Clone, Debug, Default)]
pub struct DerivativeCache {
    u: [Option<(u64, Along<4>)>; CACHED_ALONG],
    v: [Option<(u64, Along<4>)>; CACHED_ALONG],
    next: [usize; 2],
}

impl DerivativeCache {
    /// The slot of `t`'s factors along one direction, made if not kept.
    fn slot(
        slots: &mut [Option<(u64, Along<4>)>; CACHED_ALONG],
        next: &mut usize,
        t: f64,
        make: impl FnOnce() -> Along<4>,
    ) -> usize {
        let bits = t.to_bits();
        if let Some(at) = slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|(b, _)| *b == bits))
        {
            return at;
        }
        let at = *next % CACHED_ALONG;
        *next = (at + 1) % CACHED_ALONG;
        if let Some(slot) = slots.get_mut(at) {
            *slot = Some((bits, make()));
        }
        at
    }
}

/// The denominator of a NURBS surface at `(u, v)`: `Σ N_i(u) N_j(v) w_ij`,
/// the weight of the homogeneous point (see [`NurbsSurface::weight_at`]).
fn surface_weight_at<const N: usize>(s: &NurbsSurface, u: f64, v: f64) -> f64 {
    crate::allowance::charge(1);
    let count_v = s.count_v.max(1);
    let count_u = s.homogeneous.len() / count_v;
    let along_u = Along::<N>::new(&s.knots_u, s.degree_u, count_u, u, false);
    let along_v = Along::<N>::new(&s.knots_v, s.degree_v, count_v, v, false);
    let mut rows = [ZERO; N];
    for (point, row) in rows
        .iter_mut()
        .zip(s.homogeneous.chunks(count_v).skip(along_u.first))
        .take(s.degree_u + 1)
    {
        *point = along_v.point(along_v.local(row));
    }
    along_u.point(rows).w
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

/// The rows of control points of a NURBS surface collapsed along v at one
/// `v`, every one of them: what [`surface_at`] makes of the rows that have
/// influence, for any `u` ([`NurbsSurface::at_v`]).
fn rows_at_v<const N: usize>(s: &NurbsSurface, v: f64) -> (Vec<H>, Vec<H>) {
    let count_v = s.count_v.max(1);
    let along_v = Along::<N>::new(&s.knots_v, s.degree_v, count_v, v, true);
    let rows = s.homogeneous.chunks(count_v);
    let mut points = Vec::with_capacity(rows.len());
    let mut derivs = Vec::with_capacity(rows.len());
    for row in rows {
        let d = along_v.local(row);
        derivs.push(along_v.derivative(along_v.hodograph(&d)));
        points.push(along_v.point(d));
    }
    (points, derivs)
}

/// [`surface_at`] with the rows already collapsed along v.
fn surface_at_rows<const N: usize>(
    s: &NurbsSurface,
    (points, derivs): (&[H], &[H]),
    u: f64,
) -> (Vec3, Vec3, Vec3) {
    crate::allowance::charge(1);
    let count_v = s.count_v.max(1);
    let count_u = s.homogeneous.len() / count_v;
    let along_u = Along::<N>::new(&s.knots_u, s.degree_u, count_u, u, true);
    let mut row_points = [ZERO; N];
    let mut row_derivs = [ZERO; N];
    for ((point, deriv), (p, d)) in row_points
        .iter_mut()
        .zip(row_derivs.iter_mut())
        .zip(points.iter().zip(derivs).skip(along_u.first))
        .take(s.degree_u + 1)
    {
        *deriv = *d;
        *point = *p;
    }
    let a = along_u.point(row_points);
    let point = a.p * (1.0 / a.w);
    let a_u = along_u.derivative(along_u.hodograph(&row_points));
    let a_v = along_u.point(row_derivs);
    let partial = |d: H| (d.p - point * d.w) * (1.0 / a.w);
    (point, partial(a_u), partial(a_v))
}

/// [`surface_at`] with the blend factors along u worked out already.
fn surface_at_u<const N: usize>(
    s: &NurbsSurface,
    along_u: &Along<N>,
    v: f64,
) -> (Vec3, Vec3, Vec3) {
    crate::allowance::charge(1);
    let count_v = s.count_v.max(1);
    let along_v = Along::<N>::new(&s.knots_v, s.degree_v, count_v, v, true);
    let mut row_points = [ZERO; N];
    let mut row_derivs = [ZERO; N];
    for ((point, deriv), row) in row_points
        .iter_mut()
        .zip(row_derivs.iter_mut())
        .zip(s.homogeneous.chunks(count_v).skip(along_u.first))
        .take(s.degree_u + 1)
    {
        let d = along_v.local(row);
        *deriv = along_v.derivative(along_v.hodograph(&d));
        *point = along_v.point(d);
    }
    let a = along_u.point(row_points);
    let point = a.p * (1.0 / a.w);
    let a_u = along_u.derivative(along_u.hodograph(&row_points));
    let a_v = along_u.point(row_derivs);
    let partial = |d: H| (d.p - point * d.w) * (1.0 / a.w);
    (point, partial(a_u), partial(a_v))
}

/// The blend factors along u at one `u`, in the working room for the
/// surface's degree.
// The usual degrees (to three) stay on the stack; the larger rooms are boxed.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
enum AlongAny {
    Four(Along<4>),
    Eight(Box<Along<8>>),
    Sixteen(Box<Along<16>>),
    Most(Box<Along<{ MAX_DEGREE + 1 }>>),
}

/// A NURBS surface along its line of constant `u` ([`NurbsSurface::at_u`]).
#[derive(Clone, Debug)]
pub struct AtU<'a> {
    surface: &'a NurbsSurface,
    along: AlongAny,
}

impl AtU<'_> {
    /// [`NurbsSurface::derivatives`] at `(u, v)`, bit for bit.
    pub fn derivatives(&self, v: f64) -> (Vec3, Vec3, Vec3) {
        let s = self.surface;
        match &self.along {
            AlongAny::Four(a) => surface_at_u(s, a, v),
            AlongAny::Eight(a) => surface_at_u(s, a, v),
            AlongAny::Sixteen(a) => surface_at_u(s, a, v),
            AlongAny::Most(a) => surface_at_u(s, a, v),
        }
    }
}

/// A NURBS surface along its line of constant `v` ([`NurbsSurface::at_v`]).
#[derive(Clone, Debug)]
pub struct AtV<'a> {
    surface: &'a NurbsSurface,
    points: Vec<H>,
    derivs: Vec<H>,
}

impl AtV<'_> {
    /// [`NurbsSurface::derivatives`] at `(u, v)`, bit for bit.
    pub fn derivatives(&self, u: f64) -> (Vec3, Vec3, Vec3) {
        let s = self.surface;
        let degree = s.degree_u.max(s.degree_v);
        let rows = (self.points.as_slice(), self.derivs.as_slice());
        sized_for!(degree, surface_at_rows(s, rows, u))
    }
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

    /// Point at `t`. Beyond the domain the end span continues as the
    /// polynomial (or rational function) it is, as a Bézier curve does; far
    /// out, a rational curve's weight can vanish and the point is then
    /// not finite. A non-finite `t` gives the start.
    pub fn eval(&self, t: f64) -> Vec3 {
        sized_for!(self.degree, curve_at(self, t, false)).0
    }

    /// `(point, first derivative)` at `t`, continued beyond the domain as
    /// [`NurbsCurve::eval`] is.
    pub fn derivatives(&self, t: f64) -> (Vec3, Vec3) {
        sized_for!(self.degree, curve_at(self, t, true))
    }
}

/// Upper bounds on the first and second derivatives of one rational Bézier
/// curve over `[0, 1]`, in its own parameter; `None` when a weight is not
/// positive and finite.
///
/// With `C = A / w` (`A` the weighted points, `w` the weight, both
/// polynomials in Bernstein form of degree `p`):
///
/// - `C' = (A' − C w') / w`, and `A' − C w'` has the Bernstein
///   coefficients `p (w₁ (P₁ − C) − w₀ (P₀ − C))` over neighbours, which is
///   `p (w₁ (P₁ − P₀) + (w₁ − w₀) (P₀ − C))`. `C` lies in the hull of the
///   control points, so `|P₀ − C|` is at most the reach of `P₀` across
///   their box, and `w` is at least the lightest weight.
/// - `C'' = (A'' − 2 w' C' − w'' C) / w`, and `A'' − C w''` has the
///   coefficients `p (p − 1) (w₂ (P₂ − P₁) + w₀ (P₀ − P₁) + (w₂ − 2 w₁ +
///   w₀) (P₁ − C))`; `|w'|` is at most `p` times the largest difference of
///   neighbouring weights.
///
/// For weights that are all one these are the hull bounds of a polynomial
/// Bézier curve.
fn bezier_curve_bounds(points: &[H]) -> Option<(f64, f64)> {
    let degree = points.len().checked_sub(1)? as f64;
    let (lightest, _) = weight_range(points)?;
    let bbox = Bbox3::from_points(points.iter().map(|h| h.p * (1.0 / h.w)));
    let position = |h: &H| h.p * (1.0 / h.w);
    let mut first = 0.0_f64;
    let mut weight_step = 0.0_f64;
    for (a, b) in points.iter().zip(points.iter().skip(1)) {
        let (pa, pb) = (position(a), position(b));
        let (step, dw) = (pb.distance(pa), (b.w - a.w).abs());
        first =
            first.max((b.w * step + dw * reach(&bbox, pa)).min(a.w * step + dw * reach(&bbox, pb)));
        weight_step = weight_step.max(dw);
    }
    let first = degree * first / lightest;
    let mut second = 0.0_f64;
    for ((a, b), c) in points
        .iter()
        .zip(points.iter().skip(1))
        .zip(points.iter().skip(2))
    {
        let (pa, pb, pc) = (position(a), position(b), position(c));
        let bend = ((pc - pb) * c.w + (pa - pb) * a.w).length()
            + (c.w - 2.0 * b.w + a.w).abs() * reach(&bbox, pb);
        second = second.max(bend);
    }
    let second = (degree * (degree - 1.0) * second + 2.0 * degree * weight_step * first) / lightest;
    (first.is_finite() && second.is_finite()).then_some((first, second))
}

/// The lightest and the heaviest weight; `None` unless all are positive
/// and finite.
fn weight_range(points: &[H]) -> Option<(f64, f64)> {
    let (mut lightest, mut heaviest) = (f64::INFINITY, 0.0_f64);
    for h in points {
        if !(h.w.is_finite() && h.w > 0.0) {
            return None;
        }
        lightest = lightest.min(h.w);
        heaviest = heaviest.max(h.w);
    }
    (lightest.is_finite() && heaviest > 0.0).then_some((lightest, heaviest))
}

/// The distance from `p` to the farthest corner of the box: no point of
/// the box is farther from `p`.
fn reach(bbox: &Bbox3, p: Vec3) -> f64 {
    let Some((lo, hi)) = bbox.corners() else {
        return 0.0;
    };
    Vec3::new(
        (p.x - lo.x).abs().max((hi.x - p.x).abs()),
        (p.y - lo.y).abs().max((hi.y - p.y).abs()),
        (p.z - lo.z).abs().max((hi.z - p.z).abs()),
    )
    .length()
}

/// How often a Bézier span of a curve is halved for
/// [`NurbsCurve::derivative_bounds`]: the bounds of a rational piece
/// tighten as it shortens (its weights even out).
const CURVE_BOUND_HALVINGS: usize = 3;

impl NurbsCurve {
    /// The curve as rational Bézier pieces, one per knot span of its
    /// domain (*The NURBS Book* A5.6 by repeated knot insertion): each
    /// span with its control points and weights. The first and the last
    /// piece go on past the domain as [`NurbsCurve::eval`] does.
    pub fn bezier_pieces(&self) -> Vec<CurvePiece> {
        let mut knots = self.knots.clone();
        let mut rows = vec![self.homogeneous.clone()];
        make_bezier(&mut knots, self.degree, &mut rows);
        let Some(row) = rows.first() else {
            return Vec::new();
        };
        let (start, end) = domain_of(&knots, self.degree);
        let mut pieces = Vec::new();
        for k in self.degree..row.len() {
            let (Some(&lo), Some(&hi)) = (knots.get(k), knots.get(k + 1)) else {
                continue;
            };
            if !(hi > lo && lo >= start && hi <= end) {
                continue;
            }
            let Some(span) = row.get(k - self.degree..=k) else {
                continue;
            };
            pieces.push((
                (lo, hi),
                span.iter().map(|h| (h.p * (1.0 / h.w), h.w)).collect(),
            ));
        }
        pieces
    }

    /// Upper bounds on the lengths of the first and the second derivative
    /// over the whole domain, in the curve's own parameter: proven from
    /// the control points and weights of its Bézier pieces (see
    /// `bezier_curve_bounds` in the source for the argument), also for a
    /// rational curve. `None` when no finite bound comes out.
    pub fn derivative_bounds(&self) -> Option<(f64, f64)> {
        let mut knots = self.knots.clone();
        let mut rows = vec![self.homogeneous.clone()];
        make_bezier(&mut knots, self.degree, &mut rows);
        let row = rows.first()?;
        let (start, end) = domain_of(&knots, self.degree);
        let (mut first, mut second) = (0.0_f64, 0.0_f64);
        let mut found = false;
        for k in self.degree..row.len() {
            let (Some(&lo), Some(&hi)) = (knots.get(k), knots.get(k + 1)) else {
                continue;
            };
            if !(hi > lo && lo >= start && hi <= end) {
                continue;
            }
            let span = row.get(k - self.degree..=k)?;
            let mut pieces = vec![span.to_vec()];
            for _ in 0..CURVE_BOUND_HALVINGS {
                pieces = pieces
                    .iter()
                    .flat_map(|piece| {
                        let (left, right) = split_row(piece, 0.5);
                        [left, right]
                    })
                    .collect();
            }
            let width = (hi - lo) / pieces.len() as f64;
            if width.is_nan() || width <= 0.0 {
                return None;
            }
            for piece in &pieces {
                let (l, m) = bezier_curve_bounds(piece)?;
                first = first.max(l / width);
                second = second.max(m / (width * width));
            }
            found = true;
        }
        (found && first.is_finite() && second.is_finite()).then_some((first, second))
    }
}

/// One Bézier piece of a curve: its span of the curve's parameter, and its
/// control points with their weights.
pub type CurvePiece = ((f64, f64), Vec<(Vec3, f64)>);

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
    /// The samples a projection onto the surface starts from, once made
    /// (`Surface::project_grid`). Not part of the value either.
    pub(crate) project_grid: GridCache,
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
            project_grid: GridCache::default(),
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

    /// `(point, ∂/∂u, ∂/∂v)` at `(u, v)`; beyond the domain the end spans
    /// continue as the polynomials they are, as a Bézier patch does.
    pub fn derivatives(&self, u: f64, v: f64) -> (Vec3, Vec3, Vec3) {
        let degree = self.degree_u.max(self.degree_v);
        sized_for!(degree, surface_at(self, u, v, true))
    }

    /// [`Self::derivatives`], the blend factors taken from `cache` where it
    /// has them for `u` or `v` (kept there otherwise): the same bits. Only
    /// a surface of degree three or less is cached; `cache` must serve this
    /// surface alone.
    pub fn derivatives_cached(
        &self,
        u: f64,
        v: f64,
        cache: &mut DerivativeCache,
    ) -> (Vec3, Vec3, Vec3) {
        if self.degree_u.max(self.degree_v) > 3 {
            return self.derivatives(u, v);
        }
        let count_v = self.count_v.max(1);
        let count_u = self.homogeneous.len() / count_v;
        let DerivativeCache { u: us, v: vs, next } = cache;
        let [next_u, next_v] = next;
        let at_u = DerivativeCache::slot(us, next_u, u, || {
            Along::new(&self.knots_u, self.degree_u, count_u, u, true)
        });
        let at_v = DerivativeCache::slot(vs, next_v, v, || {
            Along::new(&self.knots_v, self.degree_v, count_v, v, true)
        });
        match (
            us.get(at_u).and_then(Option::as_ref),
            vs.get(at_v).and_then(Option::as_ref),
        ) {
            (Some((_, along_u)), Some((_, along_v))) => surface_with(self, along_u, along_v, true),
            _ => self.derivatives(u, v),
        }
    }

    /// The surface along its line of constant `u`, for many points of it:
    /// the blend factors along u worked out once, so that
    /// [`AtU::derivatives`] at each `v` does the rest. Each is the very
    /// value [`Self::derivatives`] gives.
    pub fn at_u(&self, u: f64) -> AtU<'_> {
        let count_v = self.count_v.max(1);
        let count_u = self.homogeneous.len() / count_v;
        let (knots, degree) = (&self.knots_u, self.degree_u);
        let along = match self.degree_u.max(self.degree_v) {
            0..=3 => AlongAny::Four(Along::new(knots, degree, count_u, u, true)),
            4..=7 => AlongAny::Eight(Box::new(Along::new(knots, degree, count_u, u, true))),
            8..=15 => AlongAny::Sixteen(Box::new(Along::new(knots, degree, count_u, u, true))),
            _ => AlongAny::Most(Box::new(Along::new(knots, degree, count_u, u, true))),
        };
        AtU {
            surface: self,
            along,
        }
    }

    /// The surface along its line of constant `v`, for many points of it:
    /// every row of control points collapsed along v once (half of what an
    /// evaluation does), so that [`AtV::derivatives`] at each `u` does the
    /// rest. Each is the very value [`Self::derivatives`] gives.
    pub fn at_v(&self, v: f64) -> AtV<'_> {
        let degree = self.degree_u.max(self.degree_v);
        let (points, derivs) = sized_for!(degree, rows_at_v(self, v));
        AtV {
            surface: self,
            points,
            derivs,
        }
    }

    /// The surface's denominator at `(u, v)`: `Σ N_i(u) N_j(v) w_ij`, the
    /// weight of the homogeneous point the surface point is the quotient
    /// of. Positive wherever the weights are. Along a line of constant u or
    /// v and within one knot span it is a polynomial of the degree in that
    /// direction, whose Bernstein coefficients at the ends of a stretch are
    /// its values there (what a Möbius reparametrisation of the stretch
    /// needs to make them equal).
    pub fn weight_at(&self, u: f64, v: f64) -> f64 {
        let degree = self.degree_u.max(self.degree_v);
        sized_for!(degree, surface_weight_at(self, u, v))
    }
}

// ------------------------------------------------------- Bézier spans

/// `n` over `k`, as a float (the degrees are small).
fn binomial(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.0;
    }
    let k = k.min(n - k);
    let mut value = 1.0;
    for i in 0..k {
        value *= (n - i) as f64 / (i + 1) as f64;
    }
    value
}

/// De Casteljau's algorithm on one row of homogeneous Bézier points at
/// `s`: the control points of the two halves, `[0, s]` and `[s, 1]`, each
/// as long as the row.
fn split_row(points: &[H], s: f64) -> (Vec<H>, Vec<H>) {
    let mut left = vec![ZERO; points.len()];
    let mut right = vec![ZERO; points.len()];
    split_row_into(points, s, &mut left, &mut right);
    (left, right)
}

/// [`split_row`] into `left` and `right` (each as long as `points`; a
/// shorter one gets what fits). The triangle is worked in place in one
/// buffer, level by level from the left, so each point is the blend of
/// the two of the level before it, in the order and with the arithmetic
/// of a fresh row per level.
fn split_row_into(points: &[H], s: f64, left: &mut [H], right: &mut [H]) {
    // Room on the stack for the usual low degrees, so clearing it costs
    // next to nothing.
    if points.len() <= 8 {
        split_row_in::<8>(points, s, left, right);
    } else if points.len() <= MAX_DEGREE + 1 {
        split_row_in::<{ MAX_DEGREE + 1 }>(points, s, left, right);
    } else {
        let mut work = points.to_vec();
        triangle(&mut work, s, left, right);
    }
}

/// [`split_row_into`] with the triangle in an array of `N` (at least the
/// row's length).
fn split_row_in<const N: usize>(points: &[H], s: f64, left: &mut [H], right: &mut [H]) {
    let mut work = [ZERO; N];
    let n = points.len().min(N);
    for (slot, &h) in work.iter_mut().zip(points) {
        *slot = h;
    }
    triangle(work.get_mut(..n).unwrap_or_default(), s, left, right);
}

/// De Casteljau's triangle on `work` in place: level by level from the
/// left, each point the blend of the two of the level before it; the
/// first of each level goes to `left`, the last to `right` from its end.
fn triangle(work: &mut [H], s: f64, left: &mut [H], right: &mut [H]) {
    let n = work.len();
    for level in 0..n {
        let live = n - level;
        if let (Some(slot), Some(&first)) = (left.get_mut(level), work.first()) {
            *slot = first;
        }
        if let (Some(slot), Some(&last)) = (right.get_mut(n - 1 - level), work.get(live - 1)) {
            *slot = last;
        }
        for i in 0..live.saturating_sub(1) {
            if let (Some(&a), Some(&b)) = (work.get(i), work.get(i + 1))
                && let Some(slot) = work.get_mut(i)
            {
                *slot = a.lerp(b, s);
            }
        }
    }
}

/// The point of a row of homogeneous Bézier points at `s`: the last
/// point of the left half [`split_row`] makes (the same arithmetic), or
/// zero for an empty row.
fn casteljau(points: &[H], s: f64) -> H {
    if points.len() > 8 {
        let (left, _) = split_row(points, s);
        return left.last().copied().unwrap_or(ZERO);
    }
    let mut work = [ZERO; 8];
    for (slot, &h) in work.iter_mut().zip(points) {
        *slot = h;
    }
    for live in (1..points.len()).rev() {
        for i in 0..live {
            if let (Some(&a), Some(&b)) = (work.get(i), work.get(i + 1))
                && let Some(slot) = work.get_mut(i)
            {
                *slot = a.lerp(b, s);
            }
        }
    }
    if points.is_empty() {
        ZERO
    } else {
        work.first().copied().unwrap_or(ZERO)
    }
}

/// Inserts the knot `u` once into `knots` and into every curve of `rows`
/// (each a run of control points along that direction): Boehm's
/// algorithm, *The NURBS Book* A5.1, on homogeneous points. `u` must lie
/// inside the domain and have a multiplicity below the degree.
fn insert_knot(knots: &mut Vec<f64>, degree: usize, rows: &mut [Vec<H>], u: f64) {
    // The span [knot k, knot k + 1) that holds u.
    let k = knots.partition_point(|&x| x <= u).saturating_sub(1);
    let alpha = |i: usize| -> f64 {
        if i + degree <= k {
            1.0
        } else if i > k {
            0.0
        } else {
            let (Some(&lo), Some(&hi)) = (knots.get(i), knots.get(i + degree)) else {
                return 0.0;
            };
            if hi > lo { (u - lo) / (hi - lo) } else { 0.0 }
        }
    };
    for row in rows.iter_mut() {
        let old = std::mem::take(row);
        let count = old.len() + 1;
        let mut new = Vec::with_capacity(count);
        for i in 0..count {
            let a = alpha(i);
            let here = i.checked_sub(0).and_then(|i| old.get(i)).copied();
            let before = i.checked_sub(1).and_then(|i| old.get(i)).copied();
            new.push(match (here, before) {
                (Some(h), Some(b)) => b.lerp(h, a),
                (Some(h), None) => h,
                (None, Some(b)) => b,
                (None, None) => ZERO,
            });
        }
        *row = new;
    }
    knots.insert((k + 1).min(knots.len()), u);
}

/// Inserts every knot of the domain's interior, and both ends of the
/// domain, until each has the multiplicity `degree`: after that every
/// span between two different knots is one Bézier piece.
fn make_bezier(knots: &mut Vec<f64>, degree: usize, rows: &mut [Vec<H>]) {
    let (start, end) = domain_of(knots, degree);
    let mut distinct: Vec<f64> = knots
        .iter()
        .copied()
        .filter(|&x| x >= start && x <= end)
        .collect();
    distinct.dedup();
    for value in distinct {
        loop {
            let multiplicity = knots.iter().filter(|&&x| x == value).count();
            if multiplicity >= degree {
                break;
            }
            insert_knot(knots, degree, rows, value);
        }
    }
}

/// One span of a NURBS surface as a rational Bézier patch: the patch over
/// `[0, 1]²` that is the surface over `u × v`.
#[derive(Clone, Debug, PartialEq)]
pub struct BezierSpan {
    pub u: (f64, f64),
    pub v: (f64, f64),
    pub patch: RationalBezierPatch,
}

/// A rational tensor-product Bézier patch over `[0, 1] × [0, 1]`: control
/// points with positive weights, row-major like [`NurbsSurface`] (point
/// `(i, j)`, `i` along u, at `i * (degree_v + 1) + j`). The surface lies
/// in the convex hull of its control points.
#[derive(Clone, Debug, PartialEq)]
pub struct RationalBezierPatch {
    degree_u: usize,
    degree_v: usize,
    homogeneous: Vec<H>,
}

impl RationalBezierPatch {
    /// A polynomial patch, with every weight 1.
    pub fn from_bezier(patch: &BezierPatch) -> Self {
        Self {
            degree_u: patch.degree_u(),
            degree_v: patch.degree_v(),
            homogeneous: patch
                .control_points()
                .iter()
                .map(|&p| H { p, w: 1.0 })
                .collect(),
        }
    }

    pub fn degrees(&self) -> (usize, usize) {
        (self.degree_u, self.degree_v)
    }

    /// The control points (positions, not weighted).
    pub fn control_points(&self) -> Vec<Vec3> {
        self.points().collect()
    }

    /// [`Self::control_points`] one by one, without a vector for them.
    pub fn points(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.homogeneous.iter().map(|h| h.p * (1.0 / h.w))
    }

    /// The control point at `index` of [`Self::control_points`].
    pub fn control_point(&self, index: usize) -> Option<Vec3> {
        self.homogeneous.get(index).map(|h| h.p * (1.0 / h.w))
    }

    pub fn weights(&self) -> Vec<f64> {
        self.homogeneous.iter().map(|h| h.w).collect()
    }

    /// True when every weight is 1 (to rounding): the patch is polynomial
    /// and its control points are the Bézier net.
    pub fn is_polynomial(&self) -> bool {
        self.homogeneous.iter().all(|h| (h.w - 1.0).abs() <= 1e-12)
    }

    /// A box around the control points, and so around the patch.
    pub fn control_bbox(&self) -> Bbox3 {
        Bbox3::from_points(self.points())
    }

    fn row(&self, i: usize) -> &[H] {
        let width = self.degree_v + 1;
        self.homogeneous
            .get(i * width..(i + 1) * width)
            .unwrap_or_default()
    }

    fn column(&self, j: usize) -> Vec<H> {
        (0..=self.degree_u)
            .filter_map(|i| self.row(i).get(j).copied())
            .collect()
    }

    /// The point at `(s, t)` in `[0, 1]²`, by de Casteljau's algorithm.
    pub fn eval(&self, s: f64, t: f64) -> Vec3 {
        crate::allowance::charge(1);
        let mut along_u = Vec::with_capacity(self.degree_u + 1);
        for i in 0..=self.degree_u {
            along_u.push(casteljau(self.row(i), t));
        }
        let h = casteljau(&along_u, s);
        if h.w > 0.0 {
            h.p * (1.0 / h.w)
        } else {
            Vec3::new(f64::NAN, f64::NAN, f64::NAN)
        }
    }

    /// The two patches over `[0, s]` and `[s, 1]` in u (each over
    /// `[0, 1]²` of its own).
    pub fn split_u(&self, s: f64) -> (Self, Self) {
        crate::allowance::charge(1);
        let width = self.degree_v + 1;
        let height = self.degree_u + 1;
        let mut left = vec![ZERO; self.homogeneous.len()];
        let mut right = vec![ZERO; self.homogeneous.len()];
        let mut column = [ZERO; 8];
        let (mut l, mut r) = ([ZERO; 8], [ZERO; 8]);
        if height <= column.len() {
            for j in 0..width {
                let mut count = 0;
                for (slot, i) in column.iter_mut().zip(0..height) {
                    if let Some(&h) = self.row(i).get(j) {
                        *slot = h;
                        count += 1;
                    }
                }
                let (Some(points), Some(lo), Some(hi)) =
                    (column.get(..count), l.get_mut(..count), r.get_mut(..count))
                else {
                    continue;
                };
                split_row_into(points, s, lo, hi);
                for (i, (a, b)) in lo.iter().zip(hi.iter()).enumerate() {
                    if let Some(slot) = left.get_mut(i * width + j) {
                        *slot = *a;
                    }
                    if let Some(slot) = right.get_mut(i * width + j) {
                        *slot = *b;
                    }
                }
            }
        } else {
            for j in 0..width {
                let (l, r) = split_row(&self.column(j), s);
                for (i, (a, b)) in l.into_iter().zip(r).enumerate() {
                    if let Some(slot) = left.get_mut(i * width + j) {
                        *slot = a;
                    }
                    if let Some(slot) = right.get_mut(i * width + j) {
                        *slot = b;
                    }
                }
            }
        }
        let make = |homogeneous| Self {
            degree_u: self.degree_u,
            degree_v: self.degree_v,
            homogeneous,
        };
        (make(left), make(right))
    }

    /// The two patches over `[0, t]` and `[t, 1]` in v.
    pub fn split_v(&self, t: f64) -> (Self, Self) {
        crate::allowance::charge(1);
        let mut left = Vec::with_capacity(self.homogeneous.len());
        let mut right = Vec::with_capacity(self.homogeneous.len());
        for i in 0..=self.degree_u {
            let row = self.row(i);
            let (from_left, from_right) = (left.len(), right.len());
            left.resize(from_left + row.len(), ZERO);
            right.resize(from_right + row.len(), ZERO);
            if let (Some(lo), Some(hi)) = (left.get_mut(from_left..), right.get_mut(from_right..)) {
                split_row_into(row, t, lo, hi);
            }
        }
        let make = |homogeneous| Self {
            degree_u: self.degree_u,
            degree_v: self.degree_v,
            homogeneous,
        };
        (make(left), make(right))
    }

    /// For a polynomial patch: the Bézier nets of `∂/∂u` (degrees
    /// `(degree_u − 1, degree_v)`) and `∂/∂v` (degrees `(degree_u, degree_v
    /// − 1)`), as scaled differences of the control points. The
    /// derivatives lie in the convex hulls of these. `None` for a rational
    /// patch, whose derivatives are no polynomials.
    pub fn derivative_nets(&self) -> Option<(Vec<Vec3>, Vec<Vec3>)> {
        if !self.is_polynomial() {
            return None;
        }
        let points = self.control_points();
        let (m, n) = (self.degree_u, self.degree_v);
        let at = |i: usize, j: usize| points.get(i * (n + 1) + j).copied().unwrap_or(Vec3::ZERO);
        let mut along_u = Vec::with_capacity(m * (n + 1));
        for i in 0..m {
            for j in 0..=n {
                along_u.push((at(i + 1, j) - at(i, j)) * (m as f64));
            }
        }
        let mut along_v = Vec::with_capacity((m + 1) * n);
        for i in 0..=m {
            for j in 0..n {
                along_v.push((at(i, j + 1) - at(i, j)) * (n as f64));
            }
        }
        Some((along_u, along_v))
    }

    /// For a polynomial patch: the Bézier net of the normal field `∂u ×
    /// ∂v` (not unit), of degrees `(2 degree_u − 1, 2 degree_v − 1)`; the
    /// product of two Bernstein forms is a Bernstein form whose
    /// coefficients blend theirs with binomial weights. The normal lies in
    /// the convex hull of the net: a net whose hull is clear of the origin
    /// proves the patch is regular. `None` for a rational patch.
    pub fn normal_net(&self) -> Option<Vec<Vec3>> {
        let (fu, fv) = self.derivative_nets()?;
        let (m, n) = (self.degree_u, self.degree_v);
        // fu has degrees (a, b) = (m − 1, n), fv has (c, d) = (m, n − 1).
        let (a, b, c, d) = (m.saturating_sub(1), n, m, n.saturating_sub(1));
        let (ka, kb) = (a + c, b + d);
        // A cross product per pair of coefficients.
        let pairs = ((a + 1) * (b + 1)).saturating_mul((c + 1) * (d + 1)) as u64;
        crate::allowance::charge(
            pairs.saturating_mul(16) / crate::allowance::ARITHMETIC_PER_UNIT + 1,
        );
        // The binomial weights of the product, per direction, computed once
        // (the same products and quotients, in the same order, as per pair).
        let weights = |p: usize, q: usize| -> Vec<f64> {
            let mut table = Vec::with_capacity((p + 1) * (q + 1));
            for i in 0..=p {
                for i2 in 0..=q {
                    table.push(binomial(p, i) * binomial(q, i2) / binomial(p + q, i + i2));
                }
            }
            table
        };
        let (along_u, along_v) = (weights(a, c), weights(b, d));
        let mut net = vec![Vec3::ZERO; (ka + 1) * (kb + 1)];
        for i in 0..=a {
            for j in 0..=b {
                let f = fu.get(i * (b + 1) + j).copied().unwrap_or(Vec3::ZERO);
                for i2 in 0..=c {
                    let wu = along_u.get(i * (c + 1) + i2).copied().unwrap_or(0.0);
                    for j2 in 0..=d {
                        let g = fv.get(i2 * (d + 1) + j2).copied().unwrap_or(Vec3::ZERO);
                        let (k, l) = (i + i2, j + j2);
                        let weight = wu * along_v.get(j * (d + 1) + j2).copied().unwrap_or(0.0);
                        if let Some(slot) = net.get_mut(k * (kb + 1) + l) {
                            *slot += f.cross(g) * weight;
                        }
                    }
                }
            }
        }
        Some(net)
    }

    /// The lightest and the heaviest weight; `None` unless every weight is
    /// positive and finite.
    pub fn weight_range(&self) -> Option<(f64, f64)> {
        weight_range(&self.homogeneous)
    }

    /// Upper bounds on the lengths of the first and second partial
    /// derivatives over the whole patch, in its own parameters: proven
    /// from the control points and the weights, also for a rational patch.
    /// `None` when a weight is not positive and finite, or no finite bound
    /// comes out.
    ///
    /// The argument is the one for a curve (`bezier_curve_bounds` in the
    /// source), per direction: with `S = A / w`, `S_u = (A_u − S w_u) / w`
    /// and `S_uu = (A_uu − 2 w_u S_u − w_uu S) / w`, where `A_u − S w_u`
    /// and `A_uu − S w_uu` are Bernstein forms whose coefficients are
    /// differences of `w (P − S)` over neighbours, each bounded through
    /// the reach of a control point across the box of the net (`S` lies in
    /// the hull). The mixed one is `S_uv = (A_uv − w_u S_v − w_v S_u −
    /// w_uv S) / w`. For weights that are all one these are the hull
    /// bounds of a polynomial patch
    /// ([`BezierPatch::second_derivative_bounds`]).
    pub fn derivative_bounds(&self) -> Option<DerivativeBounds> {
        let (m, n) = (self.degree_u, self.degree_v);
        let (lightest, _) = self.weight_range()?;
        let width = n + 1;
        let points: Vec<(Vec3, f64)> = self
            .homogeneous
            .iter()
            .map(|h| (h.p * (1.0 / h.w), h.w))
            .collect();
        let bbox = Bbox3::from_points(points.iter().map(|p| p.0));
        let at = |i: usize, j: usize| points.get(i * width + j).copied();
        let reach = |p: Vec3| reach(&bbox, p);
        let (mut first_u, mut first_v) = (0.0_f64, 0.0_f64);
        let (mut step_u, mut step_v) = (0.0_f64, 0.0_f64);
        let (mut uu, mut uv, mut vv) = (0.0_f64, 0.0_f64, 0.0_f64);
        // The largest coefficient of `A' − S w'` between two neighbours.
        let first = |(pa, wa): (Vec3, f64), (pb, wb): (Vec3, f64)| {
            let (step, dw) = (pb.distance(pa), (wb - wa).abs());
            (
                (wb * step + dw * reach(pa)).min(wa * step + dw * reach(pb)),
                dw,
            )
        };
        // The largest coefficient of `A'' − S w''` over three in a row.
        let second = |(pa, wa): (Vec3, f64), (pb, wb): (Vec3, f64), (pc, wc): (Vec3, f64)| {
            ((pc - pb) * wc + (pa - pb) * wa).length() + (wc - 2.0 * wb + wa).abs() * reach(pb)
        };
        for i in 0..=m {
            for j in 0..=n {
                let here = at(i, j)?;
                if i < m {
                    let (bound, dw) = first(here, at(i + 1, j)?);
                    first_u = first_u.max(bound);
                    step_u = step_u.max(dw);
                }
                if j < n {
                    let (bound, dw) = first(here, at(i, j + 1)?);
                    first_v = first_v.max(bound);
                    step_v = step_v.max(dw);
                }
                if i + 2 <= m {
                    uu = uu.max(second(here, at(i + 1, j)?, at(i + 2, j)?));
                }
                if j + 2 <= n {
                    vv = vv.max(second(here, at(i, j + 1)?, at(i, j + 2)?));
                }
                if i < m && j < n {
                    let ((p00, w00), (p10, w10)) = (here, at(i + 1, j)?);
                    let ((p01, w01), (p11, w11)) = (at(i, j + 1)?, at(i + 1, j + 1)?);
                    let twist = ((p11 - p00) * w11 - (p10 - p00) * w10 - (p01 - p00) * w01)
                        .length()
                        + (w11 - w10 - w01 + w00).abs() * reach(p00);
                    uv = uv.max(twist);
                }
            }
        }
        let (m, n) = (m as f64, n as f64);
        let first_u = m * first_u / lightest;
        let first_v = n * first_v / lightest;
        let bounds = DerivativeBounds {
            first_u,
            first_v,
            second_uu: (m * (m - 1.0) * uu + 2.0 * m * step_u * first_u) / lightest,
            second_uv: (m * n * uv + m * step_u * first_v + n * step_v * first_u) / lightest,
            second_vv: (n * (n - 1.0) * vv + 2.0 * n * step_v * first_v) / lightest,
        };
        bounds.is_finite().then_some(bounds)
    }

    /// The Bézier net, of degrees `(3 degree_u − 1, 3 degree_v − 1)`, of
    /// the field `D = w³ (∂u × ∂v)`: the normal of the patch (not unit)
    /// times the cube of its weight, which is a polynomial where the
    /// normal itself is not. With `S = A / w`,
    ///
    /// `w³ (S_u × S_v) = w (A_u × A_v) + w_v (A × A_u) + w_u (A_v × A)`
    ///
    /// (from `S_u = (A_u − S w_u) / w`; the products of Bernstein forms
    /// are Bernstein forms whose coefficients blend theirs with binomial
    /// weights). `D` lies in the convex hull of the net, so the normal is
    /// at least as long as the distance from the origin to that hull over
    /// the cube of the heaviest weight: a net whose hull is clear of the
    /// origin proves the patch regular. The control points are taken from
    /// the first of them, which changes nothing in `D` and keeps the
    /// products small far from the origin. `None` when a weight is not
    /// positive and finite.
    pub fn weighted_normal_net(&self) -> Option<Vec<Vec3>> {
        let (m, n) = (self.degree_u, self.degree_v);
        self.weight_range()?;
        let first = self.homogeneous.first()?;
        let origin = first.p * (1.0 / first.w);
        let local: Vec<H> = self
            .homogeneous
            .iter()
            .map(|h| H {
                p: h.p - origin * h.w,
                w: h.w,
            })
            .collect();
        let at = |i: usize, j: usize| local.get(i * (n + 1) + j).copied().unwrap_or(ZERO);
        let mut along_u = Vec::with_capacity(m * (n + 1));
        for i in 0..m {
            for j in 0..=n {
                let (a, b) = (at(i, j), at(i + 1, j));
                along_u.push(H {
                    p: (b.p - a.p) * (m as f64),
                    w: (b.w - a.w) * (m as f64),
                });
            }
        }
        let mut along_v = Vec::with_capacity((m + 1) * n);
        for i in 0..=m {
            for j in 0..n {
                let (a, b) = (at(i, j), at(i, j + 1));
                along_v.push(H {
                    p: (b.p - a.p) * (n as f64),
                    w: (b.w - a.w) * (n as f64),
                });
            }
        }
        let whole = Grid { m, n, c: local };
        let du = Grid {
            m: m.saturating_sub(1),
            n,
            c: along_u,
        };
        let dv = Grid {
            m,
            n: n.saturating_sub(1),
            c: along_v,
        };
        let cross = |a: H, b: H| a.p.cross(b.p);
        let scaled = |weight: &Grid<H>, field: &Grid<Vec3>| {
            bernstein_product(weight, field, |h: H, v: Vec3| v * h.w)
        };
        let terms = [
            scaled(&whole, &bernstein_product(&du, &dv, cross)),
            scaled(&dv, &bernstein_product(&whole, &du, cross)),
            scaled(&du, &bernstein_product(&dv, &whole, cross)),
        ];
        let [a, b, c] = terms;
        Some(
            a.c.iter()
                .zip(&b.c)
                .zip(&c.c)
                .map(|((&a, &b), &c)| a + b + c)
                .collect(),
        )
    }
}

impl RationalBezierPatch {
    /// The same patch with its parameters exchanged: the point at
    /// `(s, t)` is this patch's at `(t, s)`. Its normal points the other
    /// way.
    pub fn transposed(&self) -> Self {
        let (m, n) = (self.degree_u, self.degree_v);
        let mut homogeneous = Vec::with_capacity(self.homogeneous.len());
        for j in 0..=n {
            for i in 0..=m {
                homogeneous.push(self.row(i).get(j).copied().unwrap_or(ZERO));
            }
        }
        Self {
            degree_u: n,
            degree_v: m,
            homogeneous,
        }
    }

    /// The Bézier nets of `w² ∂u` (degrees `(2 degree_u − 1, 2
    /// degree_v)`) and `w² ∂v` (degrees `(2 degree_u, 2 degree_v − 1)`):
    /// the partial derivatives of the patch times the square of its
    /// weight, which are polynomials where the derivatives themselves are
    /// not. With `S = A / w`,
    ///
    /// `w² S_u = A_u w − A w_u`
    ///
    /// and the products of Bernstein forms are Bernstein forms whose
    /// coefficients blend theirs with binomial weights. Each field lies
    /// in the convex hull of its net, so a derivative is no longer than
    /// the longest vector of its net over the square of the lightest
    /// weight, and a net with a row of zeros shows the derivative
    /// vanishing all along that border (a border collapsed into a point).
    /// The control points are taken from the first of them, which changes
    /// nothing in the fields and keeps the products small far from the
    /// origin. `None` when a weight is not positive and finite.
    pub fn weighted_derivative_nets(&self) -> Option<(Vec<Vec3>, Vec<Vec3>)> {
        let (m, n) = (self.degree_u, self.degree_v);
        self.weight_range()?;
        let first = self.homogeneous.first()?;
        let origin = first.p * (1.0 / first.w);
        let local: Vec<H> = self
            .homogeneous
            .iter()
            .map(|h| H {
                p: h.p - origin * h.w,
                w: h.w,
            })
            .collect();
        let at = |i: usize, j: usize| local.get(i * (n + 1) + j).copied().unwrap_or(ZERO);
        let mut along_u = Vec::with_capacity(m * (n + 1));
        for i in 0..m {
            for j in 0..=n {
                let (a, b) = (at(i, j), at(i + 1, j));
                along_u.push(H {
                    p: (b.p - a.p) * (m as f64),
                    w: (b.w - a.w) * (m as f64),
                });
            }
        }
        let mut along_v = Vec::with_capacity((m + 1) * n);
        for i in 0..=m {
            for j in 0..n {
                let (a, b) = (at(i, j), at(i, j + 1));
                along_v.push(H {
                    p: (b.p - a.p) * (n as f64),
                    w: (b.w - a.w) * (n as f64),
                });
            }
        }
        let whole = Grid { m, n, c: local };
        let du = Grid {
            m: m.saturating_sub(1),
            n,
            c: along_u,
        };
        let dv = Grid {
            m,
            n: n.saturating_sub(1),
            c: along_v,
        };
        // `A' w − A w'` from the coefficients of a derivative and of the
        // patch.
        let field = |d: H, whole: H| d.p * whole.w - whole.p * d.w;
        Some((
            bernstein_product(&du, &whole, field).c,
            bernstein_product(&dv, &whole, field).c,
        ))
    }
}

/// Upper bounds on the lengths of the first and second partial derivatives
/// of a patch over all of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DerivativeBounds {
    pub first_u: f64,
    pub first_v: f64,
    pub second_uu: f64,
    pub second_uv: f64,
    pub second_vv: f64,
}

impl DerivativeBounds {
    fn is_finite(&self) -> bool {
        [
            self.first_u,
            self.first_v,
            self.second_uu,
            self.second_uv,
            self.second_vv,
        ]
        .iter()
        .all(|x| x.is_finite())
    }

    /// The bounds of the same patch over a domain `width_u` by `width_v`
    /// in place of `[0, 1]²`.
    pub fn over(&self, width_u: f64, width_v: f64) -> DerivativeBounds {
        DerivativeBounds {
            first_u: self.first_u / width_u,
            first_v: self.first_v / width_v,
            second_uu: self.second_uu / (width_u * width_u),
            second_uv: self.second_uv / (width_u * width_v),
            second_vv: self.second_vv / (width_v * width_v),
        }
    }

    /// The larger of each bound.
    pub fn max(&self, o: &DerivativeBounds) -> DerivativeBounds {
        DerivativeBounds {
            first_u: self.first_u.max(o.first_u),
            first_v: self.first_v.max(o.first_v),
            second_uu: self.second_uu.max(o.second_uu),
            second_uv: self.second_uv.max(o.second_uv),
            second_vv: self.second_vv.max(o.second_vv),
        }
    }
}

/// A tensor-product Bernstein form of degrees `(m, n)`: coefficient
/// `(i, j)` at `i * (n + 1) + j`.
struct Grid<T> {
    m: usize,
    n: usize,
    c: Vec<T>,
}

/// `C(a, i) C(c, k) / C(a + c, i + k)` at `i * (c + 1) + k`: how the
/// coefficients of two Bernstein forms of degrees `a` and `c` blend into
/// those of their product.
fn blend_table(a: usize, c: usize) -> Vec<f64> {
    let mut table = Vec::with_capacity((a + 1) * (c + 1));
    for i in 0..=a {
        for k in 0..=c {
            table.push(binomial(a, i) * binomial(c, k) / binomial(a + c, i + k));
        }
    }
    table
}

/// The product of two Bernstein forms, with `mul` as the product of two
/// coefficients.
fn bernstein_product<A: Copy, B: Copy>(
    f: &Grid<A>,
    g: &Grid<B>,
    mul: impl Fn(A, B) -> Vec3,
) -> Grid<Vec3> {
    let (m, n) = (f.m + g.m, f.n + g.n);
    let (blend_u, blend_v) = (blend_table(f.m, g.m), blend_table(f.n, g.n));
    let mut c = vec![Vec3::ZERO; (m + 1) * (n + 1)];
    for (index, &a) in f.c.iter().enumerate() {
        let (i, j) = (index / (f.n + 1), index % (f.n + 1));
        for (other, &b) in g.c.iter().enumerate() {
            let (i2, j2) = (other / (g.n + 1), other % (g.n + 1));
            let (Some(&bu), Some(&bv)) = (
                blend_u.get(i * (g.m + 1) + i2),
                blend_v.get(j * (g.n + 1) + j2),
            ) else {
                continue;
            };
            if let Some(slot) = c.get_mut((i + i2) * (n + 1) + j + j2) {
                *slot += mul(a, b) * (bu * bv);
            }
        }
    }
    Grid { m, n, c }
}

impl NurbsSurface {
    /// The surface as rational Bézier patches, one per knot span in each
    /// direction (*The NURBS Book* A5.7 by repeated knot insertion), each
    /// with the part of the domain it covers.
    pub fn bezier_spans(&self) -> Result<Vec<BezierSpan>, GeomError> {
        let (count_u, count_v) = self.counts();
        let (du, dv) = (self.degree_u, self.degree_v);
        // Along u first: one curve per column j.
        let mut knots_u = self.knots_u.clone();
        let mut columns: Vec<Vec<H>> = (0..count_v)
            .map(|j| {
                (0..count_u)
                    .filter_map(|i| self.homogeneous.get(i * count_v + j).copied())
                    .collect()
            })
            .collect();
        make_bezier(&mut knots_u, du, &mut columns);
        let count_u = columns.first().map_or(0, Vec::len);
        // Then along v: one curve per row i of the new net.
        let mut knots_v = self.knots_v.clone();
        let mut rows: Vec<Vec<H>> = (0..count_u)
            .map(|i| columns.iter().filter_map(|c| c.get(i).copied()).collect())
            .collect();
        make_bezier(&mut knots_v, dv, &mut rows);
        let count_v = rows.first().map_or(0, Vec::len);
        let spans = |knots: &[f64], degree: usize, count: usize| -> Vec<(usize, (f64, f64))> {
            let (start, end) = domain_of(knots, degree);
            let mut found = Vec::new();
            for k in degree..count {
                let (Some(&lo), Some(&hi)) = (knots.get(k), knots.get(k + 1)) else {
                    continue;
                };
                if hi > lo && lo >= start && hi <= end {
                    found.push((k, (lo, hi)));
                }
            }
            found
        };
        let mut out = Vec::new();
        for (ku, range_u) in spans(&knots_u, du, count_u) {
            for (kv, range_v) in spans(&knots_v, dv, count_v) {
                let mut homogeneous = Vec::with_capacity((du + 1) * (dv + 1));
                for i in ku - du..=ku {
                    for j in kv - dv..=kv {
                        let h = rows.get(i).and_then(|r| r.get(j)).copied().ok_or(
                            GeomError::InvalidParameter("knot insertion lost a control point"),
                        )?;
                        homogeneous.push(h);
                    }
                }
                out.push(BezierSpan {
                    u: range_u,
                    v: range_v,
                    patch: RationalBezierPatch {
                        degree_u: du,
                        degree_v: dv,
                        homogeneous,
                    },
                });
            }
        }
        if out.is_empty() {
            return Err(GeomError::InvalidParameter(
                "a surface without a knot span cannot be split into Bézier patches",
            ));
        }
        Ok(out)
    }

    /// The same surface drawn more evenly: split into its Bézier spans (as
    /// [`Self::bezier_spans`]), and each span of u (of v) reparametrised by
    /// the Möbius map `t ↦ β t / ((1 − t) + β t)` over it that brings the
    /// weights at its two ends together (*The NURBS Book* §6.1; `β` from
    /// the geometric mean, over every row of the net across the span, of
    /// the ratio of the weights at its ends, to the power one over the
    /// degree: one map per span of u, whatever v, so that it is a change of
    /// the parameters). That is a
    /// change of weights only: the weights of the span's row `i` times
    /// `β^i`, and the whole span times the factor that makes its first row
    /// what its neighbour's last became, so that the spans still meet. The
    /// point set and the spans' ranges are the same; where the weights have
    /// a product form (every surface of revolution) the result is drawn as
    /// evenly as its weights allow. `None` where nothing would change (a
    /// polynomial surface, or spans whose end weights are equal on
    /// average), and for knots that are not clamped at both ends.
    pub fn evened(&self) -> Option<NurbsSurface> {
        let first = *self.weights.first()?;
        if self.weights.iter().all(|&w| w == first) {
            return None;
        }
        let (du, dv) = (self.degree_u, self.degree_v);
        let clamped = |knots: &[f64], degree: usize| {
            let (Some(&a), Some(&b)) = (knots.first(), knots.last()) else {
                return false;
            };
            knots.iter().take(degree + 1).all(|&k| k == a)
                && knots.iter().rev().take(degree + 1).all(|&k| k == b)
        };
        if !(clamped(&self.knots_u, du) && clamped(&self.knots_v, dv)) {
            return None;
        }
        let (count_u, count_v) = self.counts();
        let mut knots_u = self.knots_u.clone();
        let mut columns: Vec<Vec<H>> = (0..count_v)
            .map(|j| {
                (0..count_u)
                    .filter_map(|i| self.homogeneous.get(i * count_v + j).copied())
                    .collect()
            })
            .collect();
        make_bezier(&mut knots_u, du, &mut columns);
        let count_u = columns.first().map_or(0, Vec::len);
        let mut knots_v = self.knots_v.clone();
        let mut rows: Vec<Vec<H>> = (0..count_u)
            .map(|i| columns.iter().filter_map(|c| c.get(i).copied()).collect())
            .collect();
        make_bezier(&mut knots_v, dv, &mut rows);
        let count_v = rows.first().map_or(0, Vec::len);
        if knots_u.len() != count_u + du + 1 || knots_v.len() != count_v + dv + 1 {
            return None;
        }
        let weight = |i: usize, j: usize| rows.get(i).and_then(|r| r.get(j)).map(|h| h.w);
        // The factor of each row of control points along one direction.
        let factors = |knots: &[f64], degree: usize, count: usize, across: usize, along_u: bool| {
            let mut f = vec![1.0_f64; count];
            let mut changed = false;
            for k in degree..count {
                let (Some(&lo), Some(&hi)) = (knots.get(k), knots.get(k + 1)) else {
                    return None;
                };
                if hi <= lo || hi.is_nan() || lo.is_nan() {
                    continue;
                }
                let (i0, i1) = (k - degree, k);
                let mut log_ratio = 0.0;
                for j in 0..across {
                    let (a, b) = if along_u {
                        (weight(i0, j)?, weight(i1, j)?)
                    } else {
                        (weight(j, i0)?, weight(j, i1)?)
                    };
                    if !(a > 0.0 && b > 0.0) {
                        return None;
                    }
                    log_ratio += (a / b).ln();
                }
                let beta = (log_ratio / (across as f64 * degree as f64)).exp();
                if !(beta.is_finite() && beta > 0.0) {
                    return None;
                }
                changed |= (beta - 1.0).abs() > 1e-12;
                let base = *f.get(i0)?;
                let mut power = 1.0;
                for i in i0..=i1 {
                    *f.get_mut(i)? = base * power;
                    power *= beta;
                }
            }
            Some((f, changed))
        };
        let (fu, changed_u) = factors(&knots_u, du, count_u, count_v, true)?;
        let (fv, changed_v) = factors(&knots_v, dv, count_v, count_u, false)?;
        if !(changed_u || changed_v) {
            return None;
        }
        let mut ctrl = Vec::with_capacity(count_u * count_v);
        let mut weights = Vec::with_capacity(count_u * count_v);
        for (i, row) in rows.iter().enumerate() {
            for (j, h) in row.iter().enumerate() {
                if h.w.is_nan() || h.w <= 0.0 {
                    return None;
                }
                ctrl.push(h.p * (1.0 / h.w));
                weights.push(h.w * fu.get(i)? * fv.get(j)?);
            }
        }
        if !weights.iter().all(|w| w.is_finite() && *w > 0.0) {
            return None;
        }
        NurbsSurface::new(du, dv, knots_u, knots_v, count_u, count_v, ctrl, weights).ok()
    }
}

impl Bits for NurbsCurve {
    fn push_bits(&self, out: &mut Vec<u64>) {
        // `homogeneous` is made from `ctrl` and `weights`: not part of the
        // curve's value, as in `==`.
        let NurbsCurve {
            degree,
            knots,
            ctrl,
            weights,
            homogeneous: _,
        } = self;
        degree.push_bits(out);
        knots.push_bits(out);
        ctrl.push_bits(out);
        weights.push_bits(out);
    }
}

impl Bits for NurbsSurface {
    fn push_bits(&self, out: &mut Vec<u64>) {
        let NurbsSurface {
            degree_u,
            degree_v,
            knots_u,
            knots_v,
            count_v,
            ctrl,
            weights,
            homogeneous: _,
            project_grid: _,
        } = self;
        degree_u.push_bits(out);
        degree_v.push_bits(out);
        knots_u.push_bits(out);
        knots_v.push_bits(out);
        count_v.push_bits(out);
        ctrl.push_bits(out);
        weights.push_bits(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// De Casteljau's split as it was written first (a fresh row per
    /// level): what the in-place triangle must give, bit for bit.
    fn split_row_by_levels(points: &[H], s: f64) -> (Vec<H>, Vec<H>) {
        let mut work: Vec<H> = points.to_vec();
        let mut left = Vec::new();
        let mut right = Vec::new();
        while let Some((&first, _)) = work.split_first() {
            left.push(first);
            right.push(work.last().copied().unwrap_or(first));
            work = work
                .iter()
                .zip(work.iter().skip(1))
                .map(|(a, b)| a.lerp(*b, s))
                .collect();
        }
        right.reverse();
        (left, right)
    }

    fn bits(points: &[H]) -> Vec<[u64; 4]> {
        points
            .iter()
            .map(|h| {
                [
                    h.p.x.to_bits(),
                    h.p.y.to_bits(),
                    h.p.z.to_bits(),
                    h.w.to_bits(),
                ]
            })
            .collect()
    }

    #[test]
    fn splits_in_place_are_the_splits_by_levels_bit_for_bit() {
        let mut seed = 17_u64;
        let mut unit = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for n in [0, 1, 2, 3, 4, 7, 33, 34, 40] {
            let row: Vec<H> = (0..n)
                .map(|_| H {
                    p: Vec3::new(unit() - 0.5, 3.0 * unit(), 1e6 * unit()),
                    w: 0.1 + unit(),
                })
                .collect();
            for s in [0.5, 0.3, 1.0 / 3.0, 0.999] {
                let (a, b) = split_row(&row, s);
                let (c, d) = split_row_by_levels(&row, s);
                assert_eq!(bits(&a), bits(&c), "{n} at {s}");
                assert_eq!(bits(&b), bits(&d), "{n} at {s}");
                let point = casteljau(&row, s);
                assert_eq!(bits(&[point]), bits(&[c.last().copied().unwrap_or(ZERO)]));
            }
        }
        for (m, k) in [(1, 1), (3, 3), (2, 5), (5, 2)] {
            let homogeneous: Vec<H> = (0..(m + 1) * (k + 1))
                .map(|_| H {
                    p: Vec3::new(unit(), unit() - 0.5, unit() * 7.0),
                    w: 0.2 + unit(),
                })
                .collect();
            let patch = RationalBezierPatch {
                degree_u: m,
                degree_v: k,
                homogeneous,
            };
            let (lu, ru) = patch.split_u(0.5);
            let (lv, rv) = patch.split_v(0.25);
            let width = k + 1;
            for j in 0..width {
                let column = patch.column(j);
                let (l, r) = split_row_by_levels(&column, 0.5);
                assert_eq!(bits(&lu.column(j)), bits(&l));
                assert_eq!(bits(&ru.column(j)), bits(&r));
            }
            for i in 0..=m {
                let (l, r) = split_row_by_levels(patch.row(i), 0.25);
                assert_eq!(bits(lv.row(i)), bits(&l));
                assert_eq!(bits(rv.row(i)), bits(&r));
            }
        }
    }

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

    /// `weight_at` is the denominator `Σ N_i N_j w_ij`: one for every
    /// weight one, the corner weight at a clamped corner, and the bilinear
    /// blend of the four weights of a bilinear patch.
    #[test]
    fn the_weight_of_a_surface_is_its_denominator() {
        let ctrl = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 3.0, 0.0),
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::new(4.0, 3.0, 1.0),
        ];
        let knots = vec![0.0, 0.0, 1.0, 1.0];
        let weights = [1.0, 7.0, 3.0, 0.5];
        let patch =
            NurbsSurface::new(1, 1, knots.clone(), knots, 2, 2, ctrl, weights.to_vec()).unwrap();
        for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.3, 0.8), (0.9, 0.05)] {
            let want = weights[0] * (1.0 - u) * (1.0 - v)
                + weights[1] * (1.0 - u) * v
                + weights[2] * u * (1.0 - v)
                + weights[3] * u * v;
            assert!(
                (patch.weight_at(u, v) - want).abs() <= 1e-14 * want,
                "{u} {v}"
            );
        }
        let polynomial = NurbsSurface::from_bezier(
            &BezierPatch::new(
                1,
                1,
                vec![Vec3::ZERO, Vec3::Y, Vec3::X, Vec3::new(1.0, 1.0, 1.0)],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(polynomial.weight_at(0.4, 0.7), 1.0);
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
    fn evaluation_is_continuous_across_knots_and_continued_outside() {
        let curve = wiggle();
        for knot in [0.3, 0.6] {
            assert!(close(
                curve.eval(knot - 1e-12),
                curve.eval(knot + 1e-12),
                1e-9
            ));
        }
        // Beyond the domain the end spans go on as the polynomials they
        // are (2026-10-13; until then the parameter was clamped, which
        // left the marcher of the surface intersections nothing to correct
        // onto past a border): smoothly, with the end derivative.
        for (t, h) in [(0.0, -1e-6), (1.0, 1e-6)] {
            let (p, d) = curve.derivatives(t);
            let beyond = curve.eval(t + h);
            assert!(beyond != p);
            assert!(close((beyond - p) * (1.0 / h), d, 1e-4 * d.length()), "{t}");
        }
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

    /// A rational surface of degrees (2, 3) with interior knots in both
    /// directions (one of them double) and varied weights.
    /// Along a line of constant v the rows collapsed once (and along one
    /// of constant u the blend factors worked out once) give every
    /// point and derivative [`NurbsSurface::derivatives`] gives, bit for
    /// bit, at degrees that take each size of working room, inside the
    /// domain, at its ends, beyond it and at NaN; the point alone is the
    /// point of the derivatives.
    #[test]
    fn a_line_of_constant_v_evaluates_as_the_surface_does() {
        let raised = |degree_u: usize, degree_v: usize| {
            let (count_u, count_v) = (degree_u + 3, degree_v + 2);
            let mut ctrl = Vec::new();
            let mut weights = Vec::new();
            for i in 0..count_u {
                for j in 0..count_v {
                    let (x, y) = (i as f64, j as f64 * 0.7);
                    ctrl.push(Vec3::new(x, y, ((i * 5 + j * 3) % 7) as f64 * 0.2));
                    weights.push(0.5 + 0.25 * ((i * 2 + j) % 5) as f64);
                }
            }
            let clamped = |degree: usize, count: usize| {
                let mut knots = vec![0.0; degree + 1];
                for k in 1..count - degree {
                    knots.push(k as f64 / (count - degree) as f64);
                }
                knots.extend(std::iter::repeat_n(1.0, degree + 1));
                knots
            };
            NurbsSurface::new(
                degree_u,
                degree_v,
                clamped(degree_u, count_u),
                clamped(degree_v, count_v),
                count_u,
                count_v,
                ctrl,
                weights,
            )
            .unwrap()
        };
        let bits = |(p, a, b): (Vec3, Vec3, Vec3)| {
            [p, a, b].map(|q| [q.x.to_bits(), q.y.to_bits(), q.z.to_bits()])
        };
        for surface in [
            lumpy(),
            raised(1, 1),
            raised(5, 2),
            raised(3, 9),
            raised(17, 2),
        ] {
            for v in [0.0, 0.13, 0.3, 0.6, 0.999, 1.0, -0.2, 1.4, f64::NAN] {
                let line = surface.at_v(v);
                for k in 0..=40 {
                    let u = -0.25 + 1.5 * f64::from(k) / 40.0;
                    let want = surface.derivatives(u, v);
                    assert_eq!(bits(line.derivatives(u)), bits(want), "{u} {v}");
                    // The same along the line of constant u through it.
                    assert_eq!(bits(surface.at_u(u).derivatives(v)), bits(want), "{u} {v}");
                    assert_eq!(
                        bits((surface.eval(u, v), Vec3::ZERO, Vec3::ZERO)),
                        bits((want.0, Vec3::ZERO, Vec3::ZERO))
                    );
                }
                assert_eq!(
                    bits(line.derivatives(f64::NAN)),
                    bits(surface.derivatives(f64::NAN, v))
                );
                // Through a cache of blend factors, asked in a walk's
                // pattern (each place, then a little either side).
                let mut cache = DerivativeCache::default();
                for k in 0..=20 {
                    let u = -0.1 + 1.2 * f64::from(k) / 20.0;
                    for (a, b) in [
                        (u, v),
                        (u - 1e-5, v),
                        (u + 1e-5, v),
                        (u, v - 1e-5),
                        (u, v + 1e-5),
                        (u, v),
                    ] {
                        assert_eq!(
                            bits(surface.derivatives_cached(a, b, &mut cache)),
                            bits(surface.derivatives(a, b)),
                            "{a} {b}"
                        );
                    }
                }
            }
        }
    }

    fn lumpy() -> NurbsSurface {
        let (count_u, count_v) = (5, 7);
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for i in 0..count_u {
            for j in 0..count_v {
                let (x, y) = (i as f64, j as f64 * 0.8);
                let z = ((i * 7 + j * 5) % 4) as f64 * 0.4 - 0.3 * (i as f64 - 2.0).abs();
                ctrl.push(Vec3::new(x + 0.1 * y, y - 0.05 * x, z));
                weights.push(1.0 + 0.3 * ((i * 3 + j * 2) % 5) as f64);
            }
        }
        let knots_u = vec![0.0, 0.0, 0.0, 0.4, 0.7, 1.0, 1.0, 1.0];
        let knots_v = vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.6, 0.6, 1.0, 1.0, 1.0, 1.0];
        NurbsSurface::new(2, 3, knots_u, knots_v, count_u, count_v, ctrl, weights).unwrap()
    }

    /// Evaluates a Bernstein net of degrees `(m, n)` at `(s, t)`.
    fn net_at(net: &[Vec3], m: usize, n: usize, s: f64, t: f64) -> Vec3 {
        let b = |k: usize, i: usize, x: f64| {
            binomial(k, i) * x.powi(i as i32) * (1.0 - x).powi((k - i) as i32)
        };
        let mut sum = Vec3::ZERO;
        for i in 0..=m {
            for j in 0..=n {
                sum += net[i * (n + 1) + j] * (b(m, i, s) * b(n, j, t));
            }
        }
        sum
    }

    /// The homogeneous point of a patch at `(s, t)`.
    fn homogeneous_at(patch: &RationalBezierPatch, s: f64, t: f64) -> H {
        let mut along_u = Vec::new();
        for i in 0..=patch.degree_u {
            along_u.push(*split_row(patch.row(i), t).0.last().unwrap());
        }
        *split_row(&along_u, s).0.last().unwrap()
    }

    /// The largest first and second derivatives of a span seen on a
    /// lattice, in the span's own `[0, 1]²` (second ones by differences
    /// of the first).
    fn seen(surface: &NurbsSurface, span: &BezierSpan) -> DerivativeBounds {
        let (wu, wv) = (span.u.1 - span.u.0, span.v.1 - span.v.0);
        let mut b = DerivativeBounds {
            first_u: 0.0,
            first_v: 0.0,
            second_uu: 0.0,
            second_uv: 0.0,
            second_vv: 0.0,
        };
        const N: usize = 24;
        let h = 1e-5;
        for i in 0..=N {
            for j in 0..=N {
                // Kept inside the span, so the differences stay in it.
                let s = h + (1.0 - 2.0 * h) * i as f64 / N as f64;
                let t = h + (1.0 - 2.0 * h) * j as f64 / N as f64;
                let at = |s: f64, t: f64| {
                    let (_, du, dv) = surface.derivatives(span.u.0 + s * wu, span.v.0 + t * wv);
                    (du * wu, dv * wv)
                };
                let (du, dv) = at(s, t);
                let ((du_a, dv_a), (du_b, dv_b)) = (at(s - h, t), at(s + h, t));
                let ((_, dv_c), (_, dv_d)) = (at(s, t - h), at(s, t + h));
                b = b.max(&DerivativeBounds {
                    first_u: du.length(),
                    first_v: dv.length(),
                    second_uu: (du_b - du_a).length() / (2.0 * h),
                    second_uv: (dv_b - dv_a).length() / (2.0 * h),
                    second_vv: (dv_d - dv_c).length() / (2.0 * h),
                });
            }
        }
        b
    }

    fn assert_bounds(bound: &DerivativeBounds, seen: &DerivativeBounds, slack: f64, what: &str) {
        for (name, b, s) in [
            ("first u", bound.first_u, seen.first_u),
            ("first v", bound.first_v, seen.first_v),
            ("second uu", bound.second_uu, seen.second_uu),
            ("second uv", bound.second_uv, seen.second_uv),
            ("second vv", bound.second_vv, seen.second_vv),
        ] {
            assert!(
                b >= s * (1.0 - 1e-6),
                "{what}: the bound on {name} is {b}, below {s} as seen"
            );
            assert!(
                b <= slack * s + 1e-9,
                "{what}: the bound on {name} is {b}, more than {slack} times {s} as seen"
            );
        }
    }

    /// A quarter of a cylinder of radius 2 and height 3 as one rational
    /// patch of degrees (2, 1), placed by `frame`.
    fn quarter_cylinder(frame: &Frame, k: f64) -> NurbsSurface {
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let mut ctrl = Vec::new();
        let mut weights = Vec::new();
        for (x, y, w) in [(2.0, 0.0, 1.0), (2.0, 2.0, s), (0.0, 2.0, 1.0)] {
            for z in [0.0, 3.0] {
                ctrl.push(frame.point(x * k, y * k, z * k));
                weights.push(w);
            }
        }
        NurbsSurface::new(
            2,
            1,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            3,
            2,
            ctrl,
            weights,
        )
        .unwrap()
    }

    #[test]
    fn the_derivative_bounds_of_a_rational_patch_hold_and_tighten() {
        let surface = lumpy();
        for span in surface.bezier_spans().unwrap() {
            assert!(!span.patch.is_polynomial());
            let bound = span.patch.derivative_bounds().unwrap();
            let actual = seen(&surface, &span);
            // Whole spans: weights from 1 to 2.2, so the bounds are loose,
            // but bounds.
            assert_bounds(&bound, &actual, 40.0, "a span");
            // A sixteenth of the span each way: the weights even out and
            // the bounds close in on the derivatives. (Seen on the whole
            // span, so only the direction that matters is checked: no
            // piece's bound may exceed the span's by much.)
            let mut pieces = vec![span.patch.clone()];
            for _ in 0..4 {
                pieces = pieces
                    .iter()
                    .flat_map(|p| {
                        let (a, b) = p.split_u(0.5);
                        let ((a0, a1), (b0, b1)) = (a.split_v(0.5), b.split_v(0.5));
                        [a0, a1, b0, b1]
                    })
                    .collect();
            }
            let mut finest = DerivativeBounds {
                first_u: 0.0,
                first_v: 0.0,
                second_uu: 0.0,
                second_uv: 0.0,
                second_vv: 0.0,
            };
            for piece in &pieces {
                finest = finest.max(
                    &piece
                        .derivative_bounds()
                        .unwrap()
                        .over(1.0 / 16.0, 1.0 / 16.0),
                );
            }
            assert_bounds(&finest, &actual, 2.5, "the pieces of a span");
        }
    }

    #[test]
    fn the_derivative_bounds_of_a_polynomial_patch_are_its_hull_bounds() {
        let net = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                Vec3::new(
                    i as f64 + 0.1 * j as f64,
                    j as f64,
                    ((i * 5 + j * 3) % 4) as f64 * 0.3,
                )
            })
        });
        let bezier = BezierPatch::bicubic(net).unwrap();
        let bound = RationalBezierPatch::from_bezier(&bezier)
            .derivative_bounds()
            .unwrap();
        let (uu, uv, vv) = bezier.second_derivative_bounds();
        assert!((bound.second_uu - uu).abs() <= 1e-12);
        assert!((bound.second_uv - uv).abs() <= 1e-12);
        assert!((bound.second_vv - vv).abs() <= 1e-12);
    }

    #[test]
    fn the_weighted_normal_net_is_the_normal_times_the_weight_cubed() {
        // Placed as far out as the model is large, as the other tests do
        // (farther out, the control points themselves are rounded by
        // more than the comparison allows).
        let tilted = |k: f64| {
            Frame::new(
                Vec3::new(30.0, -20.0, 50.0) * k,
                Vec3::new(0.3, -0.4, 1.0),
                Vec3::new(1.0, 0.2, 0.0),
            )
            .unwrap()
        };
        let surfaces = [
            (lumpy(), 1.0),
            (quarter_cylinder(&Frame::WORLD, 1.0), 1.0),
            (quarter_cylinder(&tilted(1e-3), 1e-3), 1e-3),
            (quarter_cylinder(&tilted(1e6), 1e6), 1e6),
        ];
        for (surface, k) in surfaces {
            for span in surface.bezier_spans().unwrap() {
                let (m, n) = span.patch.degrees();
                let net = span.patch.weighted_normal_net().unwrap();
                assert_eq!(net.len(), (3 * m) * (3 * n));
                let (wu, wv) = (span.u.1 - span.u.0, span.v.1 - span.v.0);
                for (s, t) in [(0.0, 0.0), (0.3, 0.8), (0.5, 0.5), (0.9, 0.2), (1.0, 1.0)] {
                    let (_, du, dv) = surface.derivatives(span.u.0 + s * wu, span.v.0 + t * wv);
                    let w = homogeneous_at(&span.patch, s, t).w;
                    let want = (du * wu).cross(dv * wv) * (w * w * w);
                    let got = net_at(&net, 3 * m - 1, 3 * n - 1, s, t);
                    assert!(
                        close(got, want, 1e-9 * k * k),
                        "at ({s}, {t}), scale {k}: {got:?} vs {want:?}"
                    );
                }
            }
        }
        // The quarter cylinder is regular: the net of each half is clear
        // of the origin, by about the area of the patch's parallelogram.
        let (left, right) = quarter_cylinder(&Frame::WORLD, 1.0).bezier_spans().unwrap()[0]
            .patch
            .split_u(0.5);
        for half in [left, right] {
            let net = half.weighted_normal_net().unwrap();
            let clear = Bbox3::from_points(net.iter().copied());
            let (lo, hi) = clear.corners().unwrap();
            assert!(
                lo.x > 0.0 || hi.x < 0.0 || lo.y > 0.0 || hi.y < 0.0,
                "{lo:?} {hi:?}"
            );
        }
    }

    #[test]
    fn the_derivative_bounds_of_a_curve_hold_at_samples() {
        let circle = NurbsCurve::circle(&Frame::WORLD, 2.0).unwrap();
        for (curve, slack) in [(wiggle(), 6.0), (circle, 1.3)] {
            let (first, second) = curve.derivative_bounds().unwrap();
            let (start, end) = curve.domain();
            let (mut speed, mut turn) = (0.0_f64, 0.0_f64);
            const N: usize = 4000;
            let h = (end - start) / N as f64;
            for i in 0..N {
                let t = start + h * i as f64;
                let (_, d) = curve.derivatives(t + 0.25 * h);
                let (_, e) = curve.derivatives(t + 0.75 * h);
                speed = speed.max(d.length());
                // Within one stretch between two knots (the knots of both
                // curves fall on the lattice).
                turn = turn.max(e.distance(d) / (0.5 * h));
            }
            assert!(
                first >= speed && second >= turn * (1.0 - 1e-3),
                "{first} {speed} {second} {turn}"
            );
            assert!(
                first <= slack * speed && second <= slack * slack * turn,
                "loose: {first} against {speed}, {second} against {turn}"
            );
        }
        // A polynomial curve: the hull bounds.
        let line = NurbsCurve::polynomial(
            1,
            vec![0.0, 0.0, 2.0, 2.0],
            vec![Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0)],
        )
        .unwrap();
        let (first, second) = line.derivative_bounds().unwrap();
        assert!((first - 2.0).abs() < 1e-12 && second == 0.0);
    }

    #[test]
    fn the_bezier_spans_of_a_surface_are_the_surface() {
        let surface = lumpy();
        let spans = surface.bezier_spans().unwrap();
        // Three spans along u (0.4 and 0.7 inserted) times three along v.
        assert_eq!(spans.len(), 9);
        let covered: f64 = spans
            .iter()
            .map(|s| (s.u.1 - s.u.0) * (s.v.1 - s.v.0))
            .sum();
        assert!((covered - 1.0).abs() < 1e-12);
        let mut rng = 0x1234_5678_u64;
        let mut next = || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..200 {
            let (u, v) = (next(), next());
            let span = spans
                .iter()
                .find(|s| u >= s.u.0 && u <= s.u.1 && v >= s.v.0 && v <= s.v.1)
                .unwrap();
            let s = (u - span.u.0) / (span.u.1 - span.u.0);
            let t = (v - span.v.0) / (span.v.1 - span.v.0);
            let want = surface.eval(u, v);
            let got = span.patch.eval(s, t);
            assert!(close(got, want, 1e-11), "({u}, {v}): {got:?} vs {want:?}");
            assert!(span.patch.control_bbox().contains(got, 1e-9));
        }
        // The corners of a span are the surface at the corners of its range.
        for span in &spans {
            let points = span.patch.control_points();
            assert!(close(points[0], surface.eval(span.u.0, span.v.0), 1e-11));
            assert!(close(
                *points.last().unwrap(),
                surface.eval(span.u.1, span.v.1),
                1e-11
            ));
        }
    }

    #[test]
    fn an_evened_surface_is_the_same_surface_drawn_by_moebius_maps_span_by_span() {
        let surface = lumpy();
        let even = surface.evened().unwrap();
        assert_eq!(even.domain(), surface.domain());
        let spans = surface.bezier_spans().unwrap();
        // β of a span along u (along v): the geometric mean, over the rows
        // of every span with the same range of u (of v), each row of the
        // net once, of the ratio of the end weights, to one over the
        // degree. (A function of u alone, so that the map is one of the
        // parameters, not of the place.)
        let beta = |span: &BezierSpan, along_u: bool| {
            let (m, n) = (span.patch.degree_u, span.patch.degree_v);
            let (mut log, mut rows) = (0.0, 0);
            let mut others: Vec<&BezierSpan> = spans
                .iter()
                .filter(|o| {
                    if along_u {
                        o.u == span.u
                    } else {
                        o.v == span.v
                    }
                })
                .collect();
            others.sort_by(|a, b| {
                let (a, b) = if along_u {
                    (a.v.0, b.v.0)
                } else {
                    (a.u.0, b.u.0)
                };
                a.total_cmp(&b)
            });
            for (k, other) in others.iter().enumerate() {
                let w = other.patch.weights();
                let at = |i: usize, j: usize| w[i * (n + 1) + j];
                let count = if along_u { n + 1 } else { m + 1 };
                // The first row of every span but the first is the last of
                // the one before.
                for r in usize::from(k > 0)..count {
                    log += if along_u {
                        (at(0, r) / at(m, r)).ln()
                    } else {
                        (at(r, 0) / at(r, n)).ln()
                    };
                    rows += 1;
                }
            }
            let degree = if along_u { m } else { n };
            (log / (rows * degree) as f64).exp()
        };
        let moebius = |b: f64, s: f64| b * s / ((1.0 - s) + b * s);
        let mut rng = 0x5eed_u64;
        let mut next = || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..200 {
            let (u, v) = (next(), next());
            let span = spans
                .iter()
                .find(|s| u >= s.u.0 && u <= s.u.1 && v >= s.v.0 && v <= s.v.1)
                .unwrap();
            let s = (u - span.u.0) / (span.u.1 - span.u.0);
            let t = (v - span.v.0) / (span.v.1 - span.v.0);
            let want = span
                .patch
                .eval(moebius(beta(span, true), s), moebius(beta(span, false), t));
            assert!(close(even.eval(u, v), want, 1e-11), "({u}, {v})");
        }
        // Drawn evenly already: nothing changes the second time, nor for a
        // polynomial surface.
        assert!(even.evened().is_none());
        let plain = NurbsSurface::new(
            2,
            3,
            surface.knots_u().to_vec(),
            surface.knots_v().to_vec(),
            5,
            7,
            surface.control_points().to_vec(),
            vec![1.0; 35],
        )
        .unwrap();
        assert!(plain.evened().is_none());
    }

    #[test]
    fn a_single_span_surface_is_one_bezier_patch_with_its_own_points() {
        let net = [
            [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.4),
                Vec3::new(0.0, 2.0, 0.1),
                Vec3::new(0.0, 3.0, 0.0),
            ],
            [
                Vec3::new(1.0, 0.0, 0.3),
                Vec3::new(1.0, 1.0, 1.0),
                Vec3::new(1.0, 2.0, 0.7),
                Vec3::new(1.0, 3.0, 0.2),
            ],
            [
                Vec3::new(2.0, 0.0, 0.1),
                Vec3::new(2.0, 1.0, 0.6),
                Vec3::new(2.0, 2.0, 0.9),
                Vec3::new(2.0, 3.0, 0.3),
            ],
            [
                Vec3::new(3.0, 0.0, 0.0),
                Vec3::new(3.0, 1.0, 0.2),
                Vec3::new(3.0, 2.0, 0.3),
                Vec3::new(3.0, 3.0, 0.0),
            ],
        ];
        let patch = BezierPatch::bicubic(net).unwrap();
        let spans = NurbsSurface::from_bezier(&patch)
            .unwrap()
            .bezier_spans()
            .unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].u, (0.0, 1.0));
        assert_eq!(spans[0].patch.control_points(), patch.control_points());
        assert!(spans[0].patch.is_polynomial());
        assert_eq!(spans[0].patch, RationalBezierPatch::from_bezier(&patch));
    }

    #[test]
    fn splitting_a_patch_keeps_its_points_and_shrinks_its_box() {
        let surface = lumpy();
        let span = &surface.bezier_spans().unwrap()[4];
        let patch = &span.patch;
        let (left, right) = patch.split_u(0.3);
        let (low, high) = patch.split_v(0.65);
        for (s, t) in [(0.0, 0.0), (0.2, 0.9), (0.5, 0.5), (0.95, 0.1), (1.0, 1.0)] {
            let want = patch.eval(s, t);
            assert!(close(left.eval(s / 0.3, t), want, 1e-11) || s > 0.3);
            assert!(close(right.eval((s - 0.3) / 0.7, t), want, 1e-11) || s < 0.3);
            assert!(close(low.eval(s, t / 0.65), want, 1e-11) || t > 0.65);
            assert!(close(high.eval(s, (t - 0.65) / 0.35), want, 1e-11) || t < 0.65);
        }
        // Each half lies in the whole's box; the halves together are
        // tighter than the whole.
        let whole = patch.control_bbox();
        for part in [&left, &right, &low, &high] {
            let (lo, hi) = part.control_bbox().corners().unwrap();
            assert!(whole.contains(lo, 1e-9) && whole.contains(hi, 1e-9));
        }
        assert!(left.control_bbox().diagonal() < whole.diagonal());
        assert!(high.control_bbox().diagonal() < whole.diagonal());
    }

    #[test]
    fn the_derivative_and_normal_nets_of_a_polynomial_patch_are_its_derivatives() {
        let net = [
            [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.4),
                Vec3::new(0.0, 2.0, 0.1),
                Vec3::new(0.0, 3.0, 0.0),
            ],
            [
                Vec3::new(1.0, 0.0, 0.3),
                Vec3::new(1.2, 1.0, 1.0),
                Vec3::new(0.9, 2.0, 0.7),
                Vec3::new(1.0, 3.0, 0.2),
            ],
            [
                Vec3::new(2.0, 0.0, 0.1),
                Vec3::new(2.0, 1.1, 0.6),
                Vec3::new(2.1, 2.0, 0.9),
                Vec3::new(2.0, 3.0, 0.3),
            ],
            [
                Vec3::new(3.0, 0.0, 0.0),
                Vec3::new(3.0, 1.0, 0.2),
                Vec3::new(3.0, 2.0, 0.3),
                Vec3::new(3.0, 3.0, 0.0),
            ],
        ];
        let bezier = BezierPatch::bicubic(net).unwrap();
        let patch = RationalBezierPatch::from_bezier(&bezier);
        let (fu, fv) = patch.derivative_nets().unwrap();
        let normal = patch.normal_net().unwrap();
        assert_eq!(fu.len(), 3 * 4);
        assert_eq!(fv.len(), 4 * 3);
        assert_eq!(normal.len(), 6 * 6);
        for (s, t) in [(0.0, 0.0), (0.3, 0.8), (0.5, 0.5), (0.9, 0.2), (1.0, 1.0)] {
            let (_, du, dv) = bezier.derivatives(s, t);
            assert!(
                close(net_at(&fu, 2, 3, s, t), du, 1e-11),
                "du at ({s}, {t})"
            );
            assert!(
                close(net_at(&fv, 3, 2, s, t), dv, 1e-11),
                "dv at ({s}, {t})"
            );
            let n = net_at(&normal, 5, 5, s, t);
            assert!(
                close(n, du.cross(dv), 1e-10),
                "normal at ({s}, {t}): {n:?} vs {:?}",
                du.cross(dv)
            );
        }
        // A rational patch has no such nets.
        assert!(
            lumpy().bezier_spans().unwrap()[0]
                .patch
                .derivative_nets()
                .is_none()
        );
    }
}
