//! Surfaces through networks of curves: the Coons patch of four boundary
//! curves and the Gordon surface of a network of u- and v-curves that
//! cross, with each boundary free, held in position on a face, or tangent
//! (G1) to one. Every result is an exact B-spline surface: nothing here is
//! fitted or sampled into being, so there is no approximation to bound.
//!
//! **The method** (Gordon 1969; Piegl & Tiller, *The NURBS Book*, §10.5 for
//! the Coons patch and the Gordon surface as Boolean sums; Farin, *Curves
//! and Surfaces for CAGD*, ch. 22). Everything is done on homogeneous
//! control points (`[x w, y w, z w, w]`), so arcs and circles take part as
//! polynomial curves do:
//!
//! 1. *Crossings.* Every u-curve must cross every v-curve within the
//!    tolerance (closest points by Gauss-Newton from the best pair of
//!    samples); the first and last of each family are the boundary, so the
//!    others end on them. Curves are turned to run the network's way, and
//!    a family must cross the other in its order.
//! 2. *One parameter per crossing.* Each family is given common crossing
//!    parameters (the means of where its curves are crossed). A curve that
//!    is crossed elsewhere is re-parametrised exactly: cut at its crossings
//!    and each piece composed with the increasing cubic that takes the
//!    common parameters to its own (its slopes at the crossings the
//!    harmonic means of the neighbouring secants, Fritsch and Butland, so
//!    every piece increases and the curve stays once differentiable in its
//!    parameter). Three times the degree; the same points.
//! 3. *One homogeneous point per crossing.* A rational curve is the same
//!    curve with its homogeneous points multiplied by a constant; the
//!    constants that make the weights of crossing curves agree are found
//!    in closed form (logarithms, a two-way table), and a network whose
//!    weights cannot agree that way is refused. A curve that is a point (a
//!    pole) takes the weights of its neighbour in its family.
//! 4. *Boolean sum.* The u-curves skinned across v (interpolation at the
//!    crossing parameters, degree up to 3), plus the v-curves skinned
//!    across u, minus the tensor-product interpolant of the crossing
//!    points: a surface through every curve of the network, which
//!    reproduces exactly any surface that lies in the spaces of both
//!    interpolations (a bilinear patch from its four sides, a tensor
//!    B-spline from its own iso-curves at its knots' averages).
//! 5. *Tangent edges.* For an edge held tangent to a face, the curves that
//!    cross it are first made to leave it along the face (their end
//!    tangent projected into the face's tangent plane there, keeping its
//!    speed, times the edge's weight; one control point moved, with a knot
//!    inserted so nothing else of the curve changes). After the sum, the
//!    surface's cross-boundary derivative along the edge is replaced by
//!    one that lies in the face's tangent plane all along: in homogeneous
//!    terms a combination `Σ aₖ(u) Fₖ(u) + b(u) C'(u) + c(u) C(u)` of the
//!    edge curve `C`, its derivative and exact fields `Fₖ` whose
//!    projections span the face's tangent plane (constant for a plane;
//!    linear in `C` for a cylinder, cone and sphere; the face's own
//!    cross-derivative along its parameter line for a spline face), with
//!    the scalar functions interpolating, at the crossings, the
//!    coefficients of the derivative the curves there already have. The
//!    change is put on the second row of control points only, with the
//!    knots in the direction across arranged so that it reaches no other
//!    curve of the network: every curve stays on the surface, every
//!    other edge keeps its position and cross derivative.
//!
//! **What is refused, with the curves named.** Curves that do not cross,
//! cross out of order or run past the boundary, or are crossed so
//! unevenly that their speed would all but vanish at a crossing; a
//! surface that would pass farther than the tolerance from a curve (a
//! proven bound: crossing gaps, weights that no constants make agree, the
//! interpolation's amplification); a network whose surface would exceed
//! [`MAX_NETWORK_POINTS`] control points (estimated from the knots before
//! anything that large is made); a curve crossing a tangent edge square
//! to its face or along it, or curves leaving it to opposite sides; a
//! corner where two held edges ask different tangent planes or twists; a
//! tangent edge along a torus or along a spline face's curve that is not
//! one of its parameter lines (owed: a tangent ribbon fitted with a
//! proven bound); weights that would not stay positive.

use crate::refine::{
    MAX_INTERPOLATED_BANDED, Raw, SAME_KNOT, basis, bernstein_product, bezier_pieces_batched,
    composed_batched, from_pieces_raw, interpolate, piece_raw,
};
use crate::{GeomError, MAX_CONTROL_POINTS, MAX_DEGREE, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Frame, Vec3};
use std::fmt;

type H = [f64; 4];

/// Most curves in one family of a network.
pub const MAX_NETWORK_CURVES: usize = 64;

/// Most control points of a network surface, and of every net of
/// control points made on the way to it (2^20: 32 MB of homogeneous
/// points per net). Each is estimated from the knots before it is made
/// and refused above this; the work of making one (knots inserted all at
/// once) is linear in its size times its degree, so this bounds the
/// time as well as the memory.
pub const MAX_NETWORK_POINTS: usize = 1 << 20;

/// Most pairs of samples compared to find where the curves of a network
/// cross (2^28; the samples are 8 per knot span of each curve).
pub const MAX_CROSSING_PAIRS: usize = 1 << 28;

/// Why a network makes no surface.
#[derive(Clone, Debug, PartialEq)]
pub enum NetworkError {
    /// The network itself is refused; the text names the curves and why.
    Refused(String),
    Geom(GeomError),
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetworkError::Refused(why) => f.write_str(why),
            NetworkError::Geom(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for NetworkError {}

impl From<GeomError> for NetworkError {
    fn from(e: GeomError) -> Self {
        NetworkError::Geom(e)
    }
}

fn refuse<T>(why: String) -> Result<T, NetworkError> {
    Err(NetworkError::Refused(why))
}

fn internal() -> NetworkError {
    NetworkError::Geom(GeomError::InvalidParameter(
        "a curve network lost track of its control points; this is a bug",
    ))
}

/// What one boundary of a patch is held to.
#[derive(Clone, Copy, Debug)]
pub enum Condition<'a> {
    /// Nothing: the edge is where its curve is.
    Free,
    /// The edge's curve must lie on this surface (checked, proven).
    Position(&'a Surface),
    /// The patch meets this surface along the edge with the same tangent
    /// plane (G1); the curves that cross the edge leave it along the
    /// surface with their own speed times `weight` (positive).
    Tangent { surface: &'a Surface, weight: f64 },
}

impl Condition<'_> {
    fn surface(&self) -> Option<&Surface> {
        match self {
            Condition::Free => None,
            Condition::Position(s) => Some(s),
            Condition::Tangent { surface, .. } => Some(surface),
        }
    }
}

/// A network: `u` curves run along u and follow each other in v, `v`
/// curves run along v and follow each other in u. `edges` are the
/// conditions of the first u curve (v = 0), the last u curve (v = 1), the
/// first v curve (u = 0) and the last v curve (u = 1).
#[derive(Clone, Copy, Debug)]
pub struct Network<'a> {
    pub u: &'a [NurbsCurve],
    pub v: &'a [NurbsCurve],
    pub edges: [Condition<'a>; 4],
    /// Distance within which curves cross, and points coincide.
    pub tolerance: f64,
    /// Angle (radians) within which tangent planes agree.
    pub angular: f64,
}

/// The names of the edges, in the order of [`Network::edges`].
pub const EDGE_NAMES: [&str; 4] = [
    "the first u curve",
    "the last u curve",
    "the first v curve",
    "the last v curve",
];

/// A surface through a network.
#[derive(Clone, Debug)]
pub struct Patch {
    pub surface: NurbsSurface,
    /// The boundary as the surface's own iso-curves, in the order of the
    /// edges (v = 0, v = 1, u = 0, u = 1), each running with its family;
    /// `None` where the boundary is a point.
    pub boundary: [Option<NurbsCurve>; 4],
    /// A proven bound on how far the surface passes from any curve of the
    /// network (as built through it: after the curves crossing a tangent
    /// edge were bent), at least the largest gap where two curves cross;
    /// held to the tolerance.
    pub mismatch: f64,
    /// Per edge held tangent: the largest angle between the oriented
    /// tangent planes found along it (radians, 0 to π, at 32 places per
    /// knot span and more); at most the angular tolerance.
    pub tangency: [Option<f64>; 4],
    /// Which curves were run the other way to follow the network.
    pub reversed_u: Vec<bool>,
    pub reversed_v: Vec<bool>,
}

/// The Coons patch of four boundary curves: `u` along v = 0 and v = 1, `v`
/// along u = 0 and u = 1 (the Gordon surface of two curves each way).
pub fn coons(
    u: [&NurbsCurve; 2],
    v: [&NurbsCurve; 2],
    edges: [Condition<'_>; 4],
    tolerance: f64,
    angular: f64,
) -> Result<Patch, NetworkError> {
    let us = [u[0].clone(), u[1].clone()];
    let vs = [v[0].clone(), v[1].clone()];
    gordon(&Network {
        u: &us,
        v: &vs,
        edges,
        tolerance,
        angular,
    })
}

// ---------------------------------------------------------------------------
// Plain splines of 4-vectors.

fn of(curve: &NurbsCurve) -> Raw {
    Raw::of(curve)
}

fn h_add(a: H, b: H, fa: f64, fb: f64) -> H {
    [
        a[0] * fa + b[0] * fb,
        a[1] * fa + b[1] * fb,
        a[2] * fa + b[2] * fb,
        a[3] * fa + b[3] * fb,
    ]
}

fn h_of(p: Vec3, w: f64) -> H {
    [p.x * w, p.y * w, p.z * w, w]
}

fn xyz(h: H) -> Vec3 {
    Vec3::new(h[0], h[1], h[2])
}

fn point(h: H) -> Option<Vec3> {
    (h[3].is_finite() && h[3] > 0.0).then(|| xyz(h) * (1.0 / h[3]))
}

/// The spline at `t`.
fn eval(raw: &Raw, t: f64) -> Result<H, NetworkError> {
    let row = basis(&raw.knots, raw.degree, raw.points.len(), t)?;
    let mut out = [0.0; 4];
    for (n, h) in row.iter().zip(&raw.points) {
        out = h_add(out, *h, 1.0, *n);
    }
    Ok(out)
}

/// The derivative spline, one degree lower (a constant zero for degree 0).
fn derivative(raw: &Raw) -> Result<Raw, NetworkError> {
    let p = raw.degree;
    if p == 0 {
        return Ok(Raw {
            degree: 0,
            knots: raw.knots.clone(),
            points: vec![[0.0; 4]; raw.points.len()],
        });
    }
    let mut points = Vec::with_capacity(raw.points.len().saturating_sub(1));
    for (i, (a, b)) in raw.points.iter().zip(raw.points.iter().skip(1)).enumerate() {
        let (Some(&lo), Some(&hi)) = (raw.knots.get(i + 1), raw.knots.get(i + p + 1)) else {
            return Err(internal());
        };
        let f = if hi > lo { p as f64 / (hi - lo) } else { 0.0 };
        points.push(h_add(*b, *a, f, -f));
    }
    let knots = raw
        .knots
        .get(1..raw.knots.len().saturating_sub(1))
        .ok_or_else(internal)?
        .to_vec();
    Ok(Raw {
        degree: p - 1,
        knots,
        points,
    })
}

/// Point and first derivative of the curve the spline stands for.
fn point_and_tangent(raw: &Raw, d: &Raw, t: f64) -> Result<(Vec3, Vec3), NetworkError> {
    let (h, dh) = (eval(raw, t)?, eval(d, t)?);
    let p = point(h).ok_or_else(internal)?;
    Ok((p, (xyz(dh) - p * dh[3]) * (1.0 / h[3])))
}

fn reverse(raw: &Raw) -> Raw {
    let (a, b) = (
        raw.knots.first().copied().unwrap_or(0.0),
        raw.knots.last().copied().unwrap_or(1.0),
    );
    Raw {
        degree: raw.degree,
        knots: raw.knots.iter().rev().map(|k| a + (b - k)).collect(),
        points: raw.points.iter().rev().copied().collect(),
    }
}

/// Every point times `f`.
fn scaled(raw: &Raw, f: f64) -> Raw {
    Raw {
        degree: raw.degree,
        knots: raw.knots.clone(),
        points: raw.points.iter().map(|h| h_add(*h, *h, f, 0.0)).collect(),
    }
}

/// Every point mapped by `f`.
fn mapped(raw: &Raw, f: impl Fn(H) -> H) -> Raw {
    Raw {
        degree: raw.degree,
        knots: raw.knots.clone(),
        points: raw.points.iter().map(|h| f(*h)).collect(),
    }
}

/// A constant over `[0, 1]`, as a spline of degree 1.
fn constant(h: H) -> Raw {
    Raw {
        degree: 1,
        knots: vec![0.0, 0.0, 1.0, 1.0],
        points: vec![h, h],
    }
}

/// The knots strictly inside the domain with their multiplicities.
fn runs(raw: &Raw) -> Vec<(f64, usize)> {
    raw.knot_runs()
}

/// The knots inside the domain with their multiplicities once the spline
/// is raised to `degree` (each raise leaves every inner knot `degree`
/// times there, see `Raw::elevate`): found without raising it.
fn runs_at(raw: &Raw, degree: usize) -> Vec<(f64, usize)> {
    if raw.degree < degree {
        raw.interior().into_iter().map(|k| (k, degree)).collect()
    } else {
        raw.knot_runs()
    }
}

/// The common degree and inner knots (each as often as the most of any)
/// of `raws` once raised to that degree.
fn union(raws: &[&Raw]) -> Result<(usize, Vec<(f64, usize)>), NetworkError> {
    let degree = raws.iter().map(|r| r.degree).max().unwrap_or(1);
    let mut all: Vec<(f64, usize)> = Vec::new();
    for raw in raws {
        all.extend(runs_at(raw, degree));
    }
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, usize)> = Vec::new();
    for (k, m) in all {
        match out.last_mut() {
            Some(last) if k - last.0 <= SAME_KNOT => last.1 = last.1.max(m),
            _ => out.push((k, m)),
        }
    }
    for run in &mut out {
        run.1 = run.1.min(degree);
    }
    Ok((degree, out))
}

/// How many control points a spline of `degree` with these inner knots
/// has (clamped ends).
fn count(degree: usize, runs: &[(f64, usize)]) -> usize {
    runs.iter()
        .fold(degree.saturating_add(1), |n, r| n.saturating_add(r.1))
}

/// Refuses a net of `nu` × `nv` control points above the budget
/// [`MAX_NETWORK_POINTS`], before it is made.
fn within_budget(nu: usize, nv: usize) -> Result<(), NetworkError> {
    let total = nu.saturating_mul(nv);
    if total > MAX_NETWORK_POINTS || nu > MAX_CONTROL_POINTS || nv > MAX_CONTROL_POINTS {
        return refuse(format!(
            "the network's surface would need {nu} × {nv} control points, more than the {MAX_NETWORK_POINTS} a network surface may have: every curve crossed at other parameters than the rest of its family is re-parametrised and brings its own knots (owed: an approximation within half the tolerance with a proven bound); fewer curves, or curves crossed at matching parameters, keep it within"
        ));
    }
    Ok(())
}

/// The spline raised to `degree` with every knot of `target` inserted as
/// often as it is there (its own knots, within rounding of one of them,
/// moved onto it). In time linear in the sizes (knots inserted at once).
fn refine_to(raw: &mut Raw, degree: usize, target: &[(f64, usize)]) -> Result<(), NetworkError> {
    while raw.degree < degree {
        raw.elevate_batched()?;
    }
    // Both ascending: one pass.
    let mut t = 0;
    for k in &mut raw.knots {
        while target.get(t).is_some_and(|run| run.0 < *k - SAME_KNOT) {
            t += 1;
        }
        if let Some(run) = target.get(t)
            && (run.0 - *k).abs() <= SAME_KNOT
        {
            *k = run.0;
        }
    }
    let own = raw.knot_runs();
    let mut missing = Vec::new();
    let mut o = 0;
    for &(k, m) in target {
        while own.get(o).is_some_and(|run| run.0 < k) {
            o += 1;
        }
        let have = own.get(o).filter(|run| run.0 == k).map_or(0, |run| run.1);
        missing.extend(std::iter::repeat_n(k, m.saturating_sub(have)));
    }
    raw.insert_all(&missing)?;
    Ok(())
}

/// `fa · a + fb · b`, both brought to one description first.
fn combined(a: &Raw, b: &Raw, fa: f64, fb: f64) -> Result<Raw, NetworkError> {
    let (degree, target) = union(&[a, b])?;
    let (mut a, mut b) = (a.clone(), b.clone());
    refine_to(&mut a, degree, &target)?;
    refine_to(&mut b, degree, &target)?;
    if a.points.len() != b.points.len() {
        return Err(internal());
    }
    Ok(Raw {
        degree,
        knots: a.knots,
        points: a
            .points
            .iter()
            .zip(&b.points)
            .map(|(x, y)| h_add(*x, *y, fa, fb))
            .collect(),
    })
}

/// The product of the scalar spline `a` (its first coordinate) and the
/// spline `b`: a spline of the sum of their degrees, exactly (piece by
/// piece on the knots of both, Bernstein coefficients multiplied).
fn times(a: &Raw, b: &Raw) -> Result<Raw, NetworkError> {
    let (mut a, mut b) = (a.clone(), b.clone());
    let knots: Vec<f64> = {
        let mut k = a.interior();
        k.extend(b.interior());
        k.sort_by(f64::total_cmp);
        k.dedup_by(|x, y| (*x - *y).abs() <= SAME_KNOT);
        k
    };
    for raw in [&mut a, &mut b] {
        // Both ascending: one pass finds the knots it lacks.
        let mut have = raw.knots.clone();
        have.dedup();
        let mut h = 0;
        let mut missing = Vec::new();
        for &k in &knots {
            while have.get(h).is_some_and(|x| *x < k - SAME_KNOT) {
                h += 1;
            }
            if !have.get(h).is_some_and(|x| (*x - k).abs() <= SAME_KNOT) {
                missing.push(k);
            }
        }
        raw.insert_all(&missing)?;
    }
    let (pa, pb) = (a.degree, b.degree);
    let (bounds, pieces_a) = bezier_pieces_batched(a)?;
    let (_, pieces_b) = bezier_pieces_batched(b)?;
    if pieces_a.len() != pieces_b.len() {
        return Err(internal());
    }
    let mut made = Vec::with_capacity(pieces_a.len());
    for (x, y) in pieces_a.iter().zip(&pieces_b) {
        let scalar: Vec<f64> = x.iter().map(|h| h[0]).collect();
        let mut out = vec![[0.0; 4]; pa + pb + 1];
        for d in 0..4 {
            let column: Vec<f64> = y.iter().map(|h| h.get(d).copied().unwrap_or(0.0)).collect();
            for (slot, c) in out.iter_mut().zip(bernstein_product(&scalar, &column)) {
                if let Some(v) = slot.get_mut(d) {
                    *v = c;
                }
            }
        }
        made.push(out);
    }
    Ok(from_pieces_raw(pa + pb, &bounds, &made)?)
}

/// The pieces (each a spline over `[0, 1]` of one degree) joined end to
/// end, piece `k` over `[bounds[k], bounds[k + 1]]`.
fn concat(pieces: &[Raw], bounds: &[f64]) -> Result<Raw, NetworkError> {
    let degree = pieces.first().map_or(1, |p| p.degree);
    if pieces.iter().any(|p| p.degree != degree) || bounds.len() != pieces.len() + 1 {
        return Err(internal());
    }
    let mut knots = vec![bounds.first().copied().ok_or_else(internal)?; degree + 1];
    let mut points: Vec<H> = Vec::new();
    for (k, piece) in pieces.iter().enumerate() {
        let (Some(&a), Some(&b)) = (bounds.get(k), bounds.get(k + 1)) else {
            return Err(internal());
        };
        for x in piece.interior() {
            for _ in 0..piece.multiplicity(x) {
                knots.push(a + (b - a) * x);
            }
        }
        if k + 1 < pieces.len() {
            knots.extend(std::iter::repeat_n(b, degree));
        } else {
            knots.extend(std::iter::repeat_n(b, degree + 1));
        }
        points.extend(piece.points.iter().skip(usize::from(k > 0)));
    }
    if points.len() > MAX_CONTROL_POINTS {
        return Err(GeomError::InvalidParameter(
            "a curve of the network has too many control points",
        )
        .into());
    }
    Ok(Raw {
        degree,
        knots,
        points,
    })
}

// ---------------------------------------------------------------------------
// Curves of a network.

/// A curve of the network with the fields carried along with it (a face's
/// cross derivative along it, which has to follow every change of its
/// parameter).
#[derive(Clone, Debug)]
struct Track {
    raw: Raw,
    fields: Vec<Raw>,
    point: bool,
    reversed: bool,
}

impl Track {
    fn new(curve: &NurbsCurve, tol: f64) -> Result<Track, NetworkError> {
        let mut raw = of(curve);
        if !raw.clamped() {
            return Err(GeomError::Unsupported(
                "a network curve whose end knots are not repeated (not clamped)",
            )
            .into());
        }
        raw.normalize()?;
        let first = curve
            .control_points()
            .first()
            .copied()
            .ok_or_else(internal)?;
        let point = curve
            .control_points()
            .iter()
            .all(|p| p.distance(first) <= tol);
        Ok(Track {
            raw,
            fields: Vec::new(),
            point,
            reversed: false,
        })
    }

    fn reverse(&mut self) {
        self.raw = reverse(&self.raw);
        self.fields = self.fields.iter().map(reverse).collect();
        self.reversed = !self.reversed;
    }

    fn at(&self, t: f64) -> Result<Vec3, NetworkError> {
        point(eval(&self.raw, t)?).ok_or_else(internal)
    }

    /// Re-parametrised so that its own parameters `own` are reached at
    /// `common` (both increasing from 0 to 1), see the module docs.
    ///
    /// Each cubic's slopes at its ends are its Bernstein slopes `α`, `β`
    /// (at most 2) times its secant: the curve's speed at a crossing falls
    /// to `α` (or `β`) times its mean speed over the piece. A direction
    /// computed from a derivative that small carries a relative rounding
    /// error of about `ε / α`, so where `α < ε / angular` the curve's
    /// tangent there (and the surface's tangent plane) is no longer known
    /// within the angular tolerance: such crossings, spaced too unevenly
    /// against the family's, are refused.
    fn align(
        &mut self,
        own: &[f64],
        common: &[f64],
        angular: f64,
        label: &str,
    ) -> Result<(), NetworkError> {
        if own
            .iter()
            .zip(common)
            .all(|(a, b)| (a - b).abs() <= SAME_KNOT)
        {
            return Ok(());
        }
        let secants: Vec<f64> = own
            .windows(2)
            .zip(common.windows(2))
            .map(|(s, u)| match (s, u) {
                ([s0, s1], [u0, u1]) => (s1 - s0) / (u1 - u0),
                _ => 0.0,
            })
            .collect();
        if secants.iter().any(|d| !(d.is_finite() && *d > 0.0)) {
            return Err(internal());
        }
        let count = secants.len();
        let slopes: Vec<f64> = (0..=count)
            .map(|j| {
                match (
                    j.checked_sub(1).and_then(|i| secants.get(i)),
                    secants.get(j),
                ) {
                    (Some(a), Some(b)) => 2.0 * a * b / (a + b),
                    (Some(a), None) => *a,
                    (None, Some(b)) => *b,
                    (None, None) => 1.0,
                }
            })
            .collect();
        // The rounding of a point is ε times its coordinates' size, so a
        // derivative over a piece of chord c is known to about ε |P| / c
        // of its mean: the least slope scales with |P| / c (scale-free:
        // a model ten times larger, or moved, gives the same answer).
        let size = self
            .raw
            .points
            .iter()
            .filter_map(|h| point(*h))
            .fold(0.0_f64, |m, p| m.max(p.length()));
        for (j, d) in secants.iter().enumerate() {
            let (Some(m0), Some(m1), Some(&s0), Some(&s1)) =
                (slopes.get(j), slopes.get(j + 1), own.get(j), own.get(j + 1))
            else {
                return Err(internal());
            };
            let chord = self.at(s0)?.distance(self.at(s1)?);
            let least = f64::EPSILON * (size / chord).max(1.0) / angular;
            let slowest = (m0 / d).min(m1 / d);
            if slowest.is_nan() || slowest < least {
                return refuse(format!(
                    "{label}: its crossings are spaced too unevenly against the rest of its family's (between two of them its parameter would slow to {slowest:.1e} of its mean rate, below the {least:.1e} at which its direction is known within the angular tolerance)"
                ));
            }
        }
        // Its size once re-parametrised, before anything is made: each
        // piece cut at its own knots, three times the degree.
        let spans = self.raw.interior().len().saturating_add(count);
        let made = spans.saturating_mul(3 * self.raw.degree).saturating_add(1);
        if made > MAX_CONTROL_POINTS {
            return refuse(format!(
                "{label}: re-parametrised to meet its family's crossings it would have {made} control points, more than the {MAX_CONTROL_POINTS} a curve may have (fewer pieces, or crossings at matching parameters)"
            ));
        }
        let redo = |raw: &Raw| -> Result<Raw, NetworkError> {
            let mut pieces = Vec::with_capacity(count);
            for j in 0..count {
                let (Some(&s0), Some(&s1), Some(&d), Some(&m0), Some(&m1)) = (
                    own.get(j),
                    own.get(j + 1),
                    secants.get(j),
                    slopes.get(j),
                    slopes.get(j + 1),
                ) else {
                    return Err(internal());
                };
                let piece = piece_raw(raw.clone(), s0, s1)?;
                let (alpha, beta) = (m0 / d, m1 / d);
                pieces.push(composed_batched(
                    piece,
                    &[0.0, alpha / 3.0, 1.0 - beta / 3.0, 1.0],
                )?);
            }
            concat(&pieces, common)
        };
        self.raw = redo(&self.raw)?;
        self.fields = self.fields.iter().map(&redo).collect::<Result<_, _>>()?;
        Ok(())
    }
}

/// Samples of a curve's parameter: `per` in every knot span, at least
/// `least` in all.
fn samples(raw: &Raw, per: usize, least: usize) -> Vec<f64> {
    let mut bounds = vec![0.0];
    bounds.extend(raw.interior());
    bounds.push(1.0);
    let spans = bounds.len().saturating_sub(1).max(1);
    let per = per.max(least.div_ceil(spans));
    let mut out = Vec::with_capacity(spans * per + 1);
    for w in bounds.windows(2) {
        if let [a, b] = w {
            for k in 0..per {
                out.push(a + (b - a) * k as f64 / per as f64);
            }
        }
    }
    out.push(1.0);
    out
}

/// Where two curves come closest: `(s, t, distance)`.
fn closest(c: &Track, d: &Track) -> Result<(f64, f64, f64), NetworkError> {
    let (dc, dd) = (derivative(&c.raw)?, derivative(&d.raw)?);
    let (sc, sd) = (samples(&c.raw, 8, 32), samples(&d.raw, 8, 32));
    let pc: Vec<Vec3> = sc.iter().map(|t| c.at(*t)).collect::<Result<_, _>>()?;
    let pd: Vec<Vec3> = sd.iter().map(|t| d.at(*t)).collect::<Result<_, _>>()?;
    let mut best = (0.0, 0.0, f64::INFINITY);
    for (s, p) in sc.iter().zip(&pc) {
        for (t, q) in sd.iter().zip(&pd) {
            let gap = p.distance(*q);
            if gap < best.2 {
                best = (*s, *t, gap);
            }
        }
    }
    let (mut s, mut t) = (best.0, best.1);
    for _ in 0..60 {
        let (p, ps) = point_and_tangent(&c.raw, &dc, s)?;
        let (q, qt) = point_and_tangent(&d.raw, &dd, t)?;
        let r = p - q;
        // Gauss-Newton on |C(s) − D(t)|², a little damped where the
        // tangents are parallel or one curve is a point.
        let (a, b, e) = (ps.dot(ps), -ps.dot(qt), qt.dot(qt));
        let damp = 1e-14 * (a + e) + f64::MIN_POSITIVE;
        let (a, e) = (a + damp, e + damp);
        let (g0, g1) = (ps.dot(r), -qt.dot(r));
        let det = a * e - b * b;
        if !(det.is_finite() && det > 0.0) {
            break;
        }
        let ds = -(e * g0 - b * g1) / det;
        let dt = -(a * g1 - b * g0) / det;
        let (ns, nt) = ((s + ds).clamp(0.0, 1.0), (t + dt).clamp(0.0, 1.0));
        let done = (ns - s).abs() < 1e-15 && (nt - t).abs() < 1e-15;
        s = ns;
        t = nt;
        if done {
            break;
        }
    }
    let gap = c.at(s)?.distance(d.at(t)?);
    if gap <= best.2 {
        Ok((s, t, gap))
    } else {
        Ok(best)
    }
}

fn name(family: char, index: usize) -> String {
    format!("{family} curve {}", index + 1)
}

// ---------------------------------------------------------------------------
// Nets of control points.

/// A tensor-product spline of 4-vectors: `rows[i][j]`, `i` along u.
#[derive(Clone, Debug)]
struct Net {
    pu: usize,
    pv: usize,
    ku: Vec<f64>,
    kv: Vec<f64>,
    rows: Vec<Vec<H>>,
}

impl Net {
    fn count_v(&self) -> usize {
        self.rows.first().map_or(0, Vec::len)
    }

    fn transposed(&self) -> Net {
        let n = self.count_v();
        let rows = (0..n)
            .map(|j| {
                self.rows
                    .iter()
                    .map(|r| r.get(j).copied().unwrap_or([0.0; 4]))
                    .collect()
            })
            .collect();
        Net {
            pu: self.pv,
            pv: self.pu,
            ku: self.kv.clone(),
            kv: self.ku.clone(),
            rows,
        }
    }

    fn reversed_v(&self) -> Net {
        let (a, b) = (
            self.kv.first().copied().unwrap_or(0.0),
            self.kv.last().copied().unwrap_or(1.0),
        );
        Net {
            pu: self.pu,
            pv: self.pv,
            ku: self.ku.clone(),
            kv: self.kv.iter().rev().map(|k| a + (b - k)).collect(),
            rows: self
                .rows
                .iter()
                .map(|r| r.iter().rev().copied().collect())
                .collect(),
        }
    }

    /// The curve along u at control column `j`.
    fn column(&self, j: usize) -> Result<Raw, NetworkError> {
        Ok(Raw {
            degree: self.pu,
            knots: self.ku.clone(),
            points: self
                .rows
                .iter()
                .map(|r| r.get(j).copied().ok_or_else(internal))
                .collect::<Result<_, _>>()?,
        })
    }

    /// Every column brought to `degree` and `target` in u.
    fn refine_u(&mut self, degree: usize, target: &[(f64, usize)]) -> Result<(), NetworkError> {
        let n = self.count_v();
        let mut columns = Vec::with_capacity(n);
        for j in 0..n {
            let mut c = self.column(j)?;
            refine_to(&mut c, degree, target)?;
            columns.push(c);
        }
        let first = columns.first().ok_or_else(internal)?;
        let (knots, count) = (first.knots.clone(), first.points.len());
        let mut rows = vec![Vec::with_capacity(n); count];
        for c in &columns {
            if c.points.len() != count {
                return Err(internal());
            }
            for (row, h) in rows.iter_mut().zip(&c.points) {
                row.push(*h);
            }
        }
        self.pu = degree;
        self.ku = knots;
        self.rows = rows;
        Ok(())
    }

    fn refine_v(&mut self, degree: usize, target: &[(f64, usize)]) -> Result<(), NetworkError> {
        let mut t = self.transposed();
        t.refine_u(degree, target)?;
        *self = t.transposed();
        Ok(())
    }

    fn u_raw(&self) -> Raw {
        Raw {
            degree: self.pu,
            knots: self.ku.clone(),
            points: vec![[0.0; 4]; self.rows.len()],
        }
    }

    fn v_raw(&self) -> Raw {
        Raw {
            degree: self.pv,
            knots: self.kv.clone(),
            points: vec![[0.0; 4]; self.count_v()],
        }
    }

    fn surface(&self) -> Result<NurbsSurface, NetworkError> {
        let (nu, nv) = (self.rows.len(), self.count_v());
        let mut ctrl = Vec::with_capacity(nu * nv);
        let mut weights = Vec::with_capacity(nu * nv);
        for row in &self.rows {
            for h in row {
                let Some(p) = point(*h) else {
                    return refuse(
                        "the network's weights do not stay positive: its rational curves cannot be made into one surface".to_string(),
                    );
                };
                ctrl.push(p);
                weights.push(h[3]);
            }
        }
        Ok(NurbsSurface::new(
            self.pu,
            self.pv,
            self.ku.clone(),
            self.kv.clone(),
            nu,
            nv,
            ctrl,
            weights,
        )?)
    }
}

/// Brings nets to one degree and one knot vector each way.
fn compatible_nets(nets: &mut [Net]) -> Result<(), NetworkError> {
    let us: Vec<Raw> = nets.iter().map(Net::u_raw).collect();
    let vs: Vec<Raw> = nets.iter().map(Net::v_raw).collect();
    let (du, tu) = union(&us.iter().collect::<Vec<_>>())?;
    let (dv, tv) = union(&vs.iter().collect::<Vec<_>>())?;
    within_budget(count(du, &tu), count(dv, &tv))?;
    for net in nets.iter_mut() {
        net.refine_u(du, &tu)?;
        net.refine_v(dv, &tv)?;
    }
    Ok(())
}

/// The curves (one description) skinned across at `params`: the net
/// whose u runs along the curves.
fn skin(curves: &[Raw], params: &[f64], degree: usize) -> Result<Net, NetworkError> {
    let first = curves.first().ok_or_else(internal)?;
    let values: Vec<Vec<f64>> = curves
        .iter()
        .map(|c| c.points.iter().flat_map(|h| h.iter().copied()).collect())
        .collect();
    let (kv, ctrl) = interpolate(params, degree, &values)?;
    let count = first.points.len();
    let mut rows = vec![Vec::with_capacity(ctrl.len()); count];
    for across in &ctrl {
        for (i, row) in rows.iter_mut().enumerate() {
            let &[x, y, z, w] = across.get(4 * i..4 * i + 4).ok_or_else(internal)? else {
                return Err(internal());
            };
            row.push([x, y, z, w]);
        }
    }
    Ok(Net {
        pu: first.degree,
        pv: degree,
        ku: first.knots.clone(),
        kv,
        rows,
    })
}

/// One family brought to one degree and knot vector.
fn compatible_tracks(tracks: &mut [Track]) -> Result<(), NetworkError> {
    let raws: Vec<Raw> = tracks.iter().map(|t| t.raw.clone()).collect();
    let (degree, target) = union(&raws.iter().collect::<Vec<_>>())?;
    for t in tracks.iter_mut() {
        refine_to(&mut t.raw, degree, &target)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Faces as fields of tangent planes.

/// Exact fields along the edge curve `c` (homogeneous) whose projections,
/// with `c` and its derivative, span the tangent planes of `surface`
/// along it; `None` for a surface that has none of that kind here.
fn analytic_fields(surface: &Surface, c: &Raw) -> Option<Vec<Raw>> {
    let lin = |f: &dyn Fn(Vec3, f64) -> Vec3| {
        mapped(c, |h| {
            let v = f(xyz(h), h[3]);
            [v.x, v.y, v.z, 0.0]
        })
    };
    let dir = |v: Vec3| constant([v.x, v.y, v.z, 0.0]);
    match surface {
        Surface::Plane(p) => Some(vec![dir(p.frame().x()), dir(p.frame().y())]),
        Surface::Cylinder(cy) => {
            let (o, d) = (cy.frame().origin(), cy.frame().z());
            Some(vec![dir(d), lin(&|x, w| d.cross(x - o * w))])
        }
        Surface::Cone(co) => {
            let (o, d) = (co.frame().origin(), co.frame().z());
            let apex = co.frame().point(0.0, 0.0, co.apex_v());
            Some(vec![
                lin(&|x, w| x - apex * w),
                lin(&|x, w| d.cross(x - o * w)),
            ])
        }
        Surface::Sphere(s) => {
            let c0 = s.frame().origin();
            let axes = [
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ];
            Some(
                axes.iter()
                    .map(|e| lin(&|x, w| e.cross(x - c0 * w)))
                    .collect(),
            )
        }
        _ => None,
    }
}

/// The iso-curve of `surface` at `at` (along v when `along_v`, so at a
/// fixed u) as a homogeneous spline, and the surface's homogeneous
/// derivative across it there.
fn iso_rows(surface: &NurbsSurface, along_v: bool, at: f64) -> Result<(Raw, Raw), NetworkError> {
    let (pu, pv) = surface.degrees();
    let (nu, nv) = surface.counts();
    let h: Vec<H> = surface
        .control_points()
        .iter()
        .zip(surface.weights())
        .map(|(p, w)| h_of(*p, *w))
        .collect();
    let get = |i: usize, j: usize| h.get(i * nv + j).copied().unwrap_or([0.0; 4]);
    // Values and derivatives of the basis across, at `at`.
    let (knots, p, n) = if along_v {
        (surface.knots_u(), pu, nu)
    } else {
        (surface.knots_v(), pv, nv)
    };
    let values = basis(knots, p, n, at)?;
    let mut slopes = vec![0.0; n];
    if p > 0 {
        let inner = knots
            .get(1..knots.len().saturating_sub(1))
            .ok_or_else(internal)?;
        let lower = basis(inner, p - 1, n - 1, at)?;
        for (i, s) in slopes.iter_mut().enumerate() {
            let term = |k: usize, lo: usize, hi: usize| -> f64 {
                match (lower.get(k), knots.get(lo), knots.get(hi)) {
                    (Some(m), Some(a), Some(b)) if b > a => p as f64 * m / (b - a),
                    _ => 0.0,
                }
            };
            let left = i.checked_sub(1).map_or(0.0, |k| term(k, i, i + p));
            let right = if i + 1 < n {
                term(i, i + 1, i + p + 1)
            } else {
                0.0
            };
            *s = left - right;
        }
    }
    let (count, other_knots, other_degree) = if along_v {
        (nv, surface.knots_v().to_vec(), pv)
    } else {
        (nu, surface.knots_u().to_vec(), pu)
    };
    let mut curve_pts = Vec::with_capacity(count);
    let mut field_pts = Vec::with_capacity(count);
    for k in 0..count {
        let (mut a, mut b) = ([0.0; 4], [0.0; 4]);
        for i in 0..n {
            let c = if along_v { get(i, k) } else { get(k, i) };
            a = h_add(a, c, 1.0, values.get(i).copied().unwrap_or(0.0));
            b = h_add(b, c, 1.0, slopes.get(i).copied().unwrap_or(0.0));
        }
        curve_pts.push(a);
        field_pts.push(b);
    }
    let iso = Raw {
        degree: other_degree,
        knots: other_knots.clone(),
        points: curve_pts,
    };
    let field = Raw {
        degree: other_degree,
        knots: other_knots,
        points: field_pts,
    };
    Ok((iso, field))
}

/// The iso-curve of `surface` along u at `v = at` (`along_u`), or along v
/// at `u = at`: exactly the surface there.
pub fn iso_curve(
    surface: &NurbsSurface,
    along_u: bool,
    at: f64,
) -> Result<NurbsCurve, NetworkError> {
    Ok(iso_rows(surface, !along_u, at)?.0.curve()?)
}

/// For a spline face: the iso-curve of `surface` that `curve` lies on
/// (within `tol`), cut to the curve and run its way, and the face's
/// homogeneous derivative across it.
///
/// The curve is projected onto the face at 4 places per knot span (33 at
/// least, 1025 at most); it runs along a parameter line when one
/// coordinate stays put (the line there checked at every place) and the
/// other runs one way. A face closed in a direction (its first and last
/// iso-curves the same) gives a point on its seam either coordinate:
/// the fixed one is read nearest the first place's, the running one is
/// followed from place to place across the seam, and an edge that would
/// cross the seam is refused (owed: the iso-curve joined across it).
fn iso_track(
    surface: &NurbsSurface,
    curve: &Track,
    tol: f64,
) -> Result<Option<(Raw, Raw)>, NetworkError> {
    let wrapped = Surface::Nurbs(surface.clone());
    let mut ts = samples(&curve.raw, 4, 33);
    if ts.len() > 1025 {
        ts = (0..=1024).map(|k| f64::from(k) / 1024.0).collect();
    }
    let mut uv = Vec::with_capacity(ts.len());
    for t in &ts {
        let p = curve.at(*t)?;
        uv.push((p, wrapped.project(p)?));
    }
    let ((u0, u1), (v0, v1)) = surface.domain();
    // Closed in u: the iso-curves at u0 and u1 coincide (and so in v).
    let closed = |in_u: bool| {
        (0..=8).all(|k| {
            let x = f64::from(k) / 8.0;
            let (a, b) = if in_u {
                let v = v0 + (v1 - v0) * x;
                (wrapped.eval(u0, v), wrapped.eval(u1, v))
            } else {
                let u = u0 + (u1 - u0) * x;
                (wrapped.eval(u, v0), wrapped.eval(u, v1))
            };
            a.distance(b) <= tol
        })
    };
    let (closed_u, closed_v) = (closed(true), closed(false));
    for along_v in [true, false] {
        // Along v: u is fixed, v runs.
        let (fixed_range, fixed_closed, run_range, run_closed) = if along_v {
            ((u0, u1), closed_u, (v0, v1), closed_v)
        } else {
            ((v0, v1), closed_v, (u0, u1), closed_u)
        };
        let split = |(u, v): (f64, f64)| if along_v { (u, v) } else { (v, u) };
        let period = |r: (f64, f64)| r.1 - r.0;
        // The fixed coordinate, each read nearest the first.
        let first_fixed = uv.first().map(|(_, c)| split(*c).0).ok_or_else(internal)?;
        let near = |x: f64| {
            if fixed_closed {
                let w = period(fixed_range);
                if x - first_fixed > 0.5 * w {
                    x - w
                } else if first_fixed - x > 0.5 * w {
                    x + w
                } else {
                    x
                }
            } else {
                x
            }
        };
        let fixed: Vec<f64> = uv.iter().map(|(_, c)| near(split(*c).0)).collect();
        let at =
            (fixed.iter().sum::<f64>() / fixed.len() as f64).clamp(fixed_range.0, fixed_range.1);
        // The running coordinate, followed across a seam.
        let mut run: Vec<f64> = Vec::with_capacity(uv.len());
        for (_, c) in &uv {
            let mut x = split(*c).1;
            if run_closed && let Some(&last) = run.last() {
                // The whole turns between it and the place before.
                let w = period(run_range);
                let turns = ((x - last) / w).round();
                if turns.is_finite() {
                    x -= turns * w;
                }
            }
            run.push(x);
        }
        // Brought into the domain as a whole where it can be.
        if run_closed {
            let w = period(run_range);
            let slack = SAME_KNOT * w;
            let (lo, hi) = run
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
                    (a.min(*x), b.max(*x))
                });
            let shift = if hi > run_range.1 + slack {
                -w
            } else if lo < run_range.0 - slack {
                w
            } else {
                0.0
            };
            for x in &mut run {
                *x += shift;
            }
        }
        let on = uv.iter().zip(&run).all(|((p, _), r)| {
            let r = r.clamp(run_range.0, run_range.1);
            let q = if along_v {
                wrapped.eval(at, r)
            } else {
                wrapped.eval(r, at)
            };
            q.distance(*p) <= tol
        });
        if !on {
            continue;
        }
        let (Some(&s), Some(&e)) = (run.first(), run.last()) else {
            return Err(internal());
        };
        let (lo, hi) = run_range;
        let slack = SAME_KNOT * (hi - lo);
        // One way along the line, within the face.
        let rising = e > s;
        let monotone = run.windows(2).all(|w| match w {
            [a, b] => {
                if rising {
                    *b >= *a - slack
                } else {
                    *b <= *a + slack
                }
            }
            _ => true,
        });
        if run.iter().any(|x| *x < lo - slack || *x > hi + slack) {
            return refuse(
                "it runs across its face's seam; tangency along a spline face's parameter line across its seam is not supported yet (owed: the iso-curve joined across the seam)".to_string(),
            );
        }
        if !monotone {
            continue;
        }
        let (a, b) = (s.min(e).max(lo), s.max(e).min(hi));
        if b - a <= SAME_KNOT * (hi - lo) {
            continue;
        }
        let (iso, field) = iso_rows(surface, along_v, at)?;
        let (mut iso, mut field) = (piece_raw(iso, a, b)?, piece_raw(field, a, b)?);
        if s > e {
            iso = reverse(&iso);
            field = reverse(&field);
        }
        return Ok(Some((iso, field)));
    }
    Ok(None)
}

/// The coefficients of `y` in the columns `a`, in the least-squares
/// sense, and what is left of `y` (in the homogeneous coordinates given).
///
/// Scale-free: the points are first written about `centre` with the
/// weight measured in `length` (`[x − c w, y − c w, z − c w, L w]`, a
/// change of coordinates that keeps every exact combination exact), so
/// every coordinate is a length, and each column is scaled to unit
/// length. Then Householder QR with column pivoting: a column whose part
/// outside the span of those before it is `cutoff` of its length or less
/// (the sine of its angle with that span) is taken as dependent on them
/// and gets no coefficient. That is the basic solution: dependent columns
/// cannot blow the coefficients up, and no regularisation biases the rest.
fn coefficients(a: &[H], y: H, centre: Vec3, length: f64, cutoff: f64) -> (Vec<f64>, H) {
    let to = |h: &H| -> [f64; 4] {
        [
            h[0] - centre.x * h[3],
            h[1] - centre.y * h[3],
            h[2] - centre.z * h[3],
            h[3] * length,
        ]
    };
    let norm = |h: &[f64]| h.iter().map(|x| x * x).sum::<f64>().sqrt();
    let k = a.len();
    let mut cols: Vec<[f64; 4]> = a.iter().map(to).collect();
    let scales: Vec<f64> = cols
        .iter()
        .map(|c| {
            let n = norm(c);
            if n.is_finite() && n > 0.0 {
                1.0 / n
            } else {
                0.0
            }
        })
        .collect();
    for (c, f) in cols.iter_mut().zip(&scales) {
        for x in c.iter_mut() {
            *x *= f;
        }
    }
    let mut r = to(&y);
    // order[i]: the column pivoted into place i.
    let mut order: Vec<usize> = (0..k).collect();
    let mut rank = 0;
    let tail = |c: &[f64; 4], from: usize| norm(c.get(from..).unwrap_or(&[]));
    for step in 0..k.min(4) {
        let best = (step..k)
            .max_by(|x, z| {
                let nx = order
                    .get(*x)
                    .and_then(|i| cols.get(*i))
                    .map_or(0.0, |c| tail(c, step));
                let nz = order
                    .get(*z)
                    .and_then(|i| cols.get(*i))
                    .map_or(0.0, |c| tail(c, step));
                nx.total_cmp(&nz)
            })
            .unwrap_or(step);
        order.swap(step, best);
        let Some(&pivot) = order.get(step) else { break };
        let Some(col) = cols.get(pivot).copied() else {
            break;
        };
        let alpha = tail(&col, step);
        if alpha.is_nan() || alpha <= cutoff {
            break;
        }
        // The reflection taking the column's tail to −sign(x) α e_step.
        let x0 = col.get(step).copied().unwrap_or(0.0);
        let beta = if x0 >= 0.0 { -alpha } else { alpha };
        let mut v = [0.0; 4];
        for (i, slot) in v.iter_mut().enumerate().skip(step) {
            *slot = col.get(i).copied().unwrap_or(0.0);
        }
        if let Some(s0) = v.get_mut(step) {
            *s0 -= beta;
        }
        let vv: f64 = v.iter().map(|x| x * x).sum();
        if vv > 0.0 {
            let reflect = |c: &mut [f64; 4]| {
                let d: f64 = c.iter().zip(&v).map(|(x, y)| x * y).sum::<f64>() * 2.0 / vv;
                for (x, y) in c.iter_mut().zip(&v) {
                    *x -= d * y;
                }
            };
            for &j in order.iter().skip(step) {
                if let Some(c) = cols.get_mut(j) {
                    reflect(c);
                }
            }
            reflect(&mut r);
        }
        rank += 1;
    }
    // Back substitution on the first `rank` rows.
    let mut x = vec![0.0; k];
    for i in (0..rank).rev() {
        let Some(&ci) = order.get(i) else { continue };
        let mut value = r.get(i).copied().unwrap_or(0.0);
        for j in (i + 1)..rank {
            let Some(&cj) = order.get(j) else { continue };
            let rij = cols.get(cj).and_then(|c| c.get(i)).copied().unwrap_or(0.0);
            value -= rij * x.get(cj).copied().unwrap_or(0.0);
        }
        let rii = cols.get(ci).and_then(|c| c.get(i)).copied().unwrap_or(0.0);
        if let Some(slot) = x.get_mut(ci) {
            *slot = if rii != 0.0 { value / rii } else { 0.0 };
        }
    }
    let coeffs: Vec<f64> = x.iter().zip(&scales).map(|(x, s)| x * s).collect();
    let mut left = y;
    for (c, xi) in a.iter().zip(&coeffs) {
        left = h_add(left, *c, 1.0, -xi);
    }
    (coeffs, left)
}

/// Knots inserted so that the second basis function from the start of
/// `raw` ends at or before `next` (nothing to do when `next` is the end).
fn localize(raw: &mut Raw, next: f64) -> Result<(), NetworkError> {
    for _ in 0..4 {
        let p = raw.degree;
        let (Some(&first), Some(&second)) = (raw.knots.get(p + 1), raw.knots.get(p + 2)) else {
            return Err(internal());
        };
        if second <= next + SAME_KNOT || next >= 1.0 - SAME_KNOT || next <= SAME_KNOT {
            return Ok(());
        }
        let at = if first > next + SAME_KNOT {
            next
        } else if first < next - SAME_KNOT {
            0.5 * (first + next)
        } else {
            0.5 * first
        };
        raw.insert(at)?;
    }
    Err(internal())
}

/// Makes the cross derivative of `net` along v = 0 lie in the tangent
/// planes the `fields` (with the edge curve and its derivative) span,
/// matching it at the crossings `at` (in u) and changing the second row of
/// control points only, within `[0, next]` in v.
#[allow(clippy::too_many_arguments)]
fn hold_tangent(
    net: &mut Net,
    fields: &[Raw],
    at: &[f64],
    next: f64,
    angular: f64,
    tol: f64,
    edge: usize,
) -> Result<(), NetworkError> {
    let label = EDGE_NAMES.get(edge).copied().unwrap_or("an edge");
    // At least cubic across, so the change blends in smoothly.
    let mut v = net.v_raw();
    while v.degree < 3 {
        v.elevate()?;
    }
    // The second row's basis function must end at or before `next`.
    localize(&mut v, next)?;
    let q = v.degree;
    within_budget(net.rows.len(), count(q, &runs(&v)))?;
    net.refine_v(q, &runs(&v))?;
    let (Some(&t1), Some(&tq1)) = (net.kv.get(1), net.kv.get(q + 1)) else {
        return Err(internal());
    };
    let span = tq1 - t1;
    if span.is_nan() || span <= 0.0 || net.count_v() < 4 {
        return Err(internal());
    }
    let c = net.column(0)?;
    let dc = derivative(&c)?;
    let second = net.column(1)?;
    let cross = Raw {
        degree: c.degree,
        knots: c.knots.clone(),
        points: second
            .points
            .iter()
            .zip(&c.points)
            .map(|(b, a)| h_add(*b, *a, q as f64 / span, -(q as f64) / span))
            .collect(),
    };
    let mut columns: Vec<Raw> = fields.to_vec();
    columns.push(dc);
    columns.push(c.clone());
    // Lengths about the edge's first point: the extent of the first two
    // rows (what the change moves).
    let centre = c
        .points
        .first()
        .copied()
        .and_then(point)
        .ok_or_else(internal)?;
    let length = c
        .points
        .iter()
        .chain(&second.points)
        .filter_map(|h| point(*h))
        .fold(0.0_f64, |l, p| l.max(p.distance(centre)));
    if !(length.is_finite() && length > 0.0) {
        return Err(internal());
    }
    // The edge lies on its face within the tolerance, so its direction
    // may leave the face's tangent planes by about tol / extent: columns
    // within that (or the angular tolerance) of the others' span are not
    // told apart from it.
    let cutoff = angular.max(tol / length);
    let f = span / q as f64;
    // The coefficients at each crossing.
    let mut table: Vec<Vec<f64>> = vec![Vec::with_capacity(at.len()); columns.len()];
    for &u in at {
        let a: Vec<H> = columns
            .iter()
            .map(|f| eval(f, u))
            .collect::<Result<_, _>>()?;
        let y = eval(&cross, u)?;
        let (x, left) = coefficients(&a, y, centre, length, cutoff);
        // What is left of the cross derivative outside the face's planes
        // is taken off the second row there: it moves the crossing
        // curve's second control point by this much.
        let h1 = eval(&second, u)?;
        let moved = match (point(h1), point(h_add(h1, left, 1.0, -f))) {
            (Some(p), Some(q)) => p.distance(q),
            _ => f64::INFINITY,
        };
        if moved.is_nan() || moved > tol {
            return refuse(format!(
                "{label}: the surface cannot leave it along its face at u = {u:.6}: the curve crossing it there is not tangent to the face (it would be moved {moved:.3e}, the tolerance is {tol:.3e})"
            ));
        }
        for (row, xi) in table.iter_mut().zip(x) {
            row.push(xi);
        }
    }
    let degree = (at.len() - 1).min(3);
    let mut target: Option<Raw> = None;
    for (field, row) in columns.iter().zip(&table) {
        let values: Vec<Vec<f64>> = row.iter().map(|x| vec![*x, 0.0, 0.0, 0.0]).collect();
        let (knots, ctrl) = interpolate(at, degree, &values)?;
        let scalar = Raw {
            degree,
            knots,
            points: ctrl
                .iter()
                .map(|c| [c.first().copied().unwrap_or(0.0), 0.0, 0.0, 0.0])
                .collect(),
        };
        let term = times(&scalar, field)?;
        target = Some(match target {
            None => term,
            Some(t) => combined(&t, &term, 1.0, 1.0)?,
        });
    }
    let target = target.ok_or_else(internal)?;
    let mut delta = combined(&target, &cross, 1.0, -1.0)?;
    let (du, tu) = union(&[&delta, &c])?;
    within_budget(count(du, &tu), net.count_v())?;
    refine_to(&mut delta, du, &tu)?;
    net.refine_u(du, &tu)?;
    if delta.points.len() != net.rows.len() {
        return Err(internal());
    }
    // The change between the crossings is what puts the derivative into
    // the planes there; it is of the order of how far the interpolated
    // derivative drifts off them, a fraction of the ribbon between the
    // edge and the second row. Coefficients of a column kept just above
    // the cutoff could be ~1 / cutoff: a change larger than the ribbon
    // itself (with a margin of the tolerance) is that, and refused.
    for (u, (row, d)) in net.rows.iter_mut().zip(&delta.points).enumerate() {
        let h0 = row.first().copied().ok_or_else(internal)?;
        let slot = row.get_mut(1).ok_or_else(internal)?;
        let moved = h_add(*slot, *d, 1.0, f);
        let (Some(p0), Some(p1), Some(q1)) = (point(h0), point(*slot), point(moved)) else {
            return refuse(format!(
                "{label}: holding it tangent would make a weight of the surface not positive"
            ));
        };
        let change = p1.distance(q1);
        let ribbon = p1.distance(p0) + tol;
        if change.is_nan() || change > ribbon {
            return refuse(format!(
                "{label}: holding it tangent would move the surface's second row of control points by {change:.3e} at control point {u}, more than the {ribbon:.3e} between it and the edge: the face's tangent planes are not told apart well enough there to follow"
            ));
        }
        *slot = moved;
    }
    Ok(())
}

/// The largest angle between the oriented tangent planes of `patch` along
/// its edge and those of `face`, at 32 places per knot span and more.
///
/// Oriented: `along × into` (the derivative along the edge, then the
/// derivative across it into the patch) must point to the side `side` of
/// the face's normal (+1 or −1) that the curves crossing the edge leave
/// to (`bend`), so the angle runs from 0 to π: a patch whose normal turns
/// over, or that folds back over its face, is as far off as it is. Where
/// the two derivatives are within the angular tolerance of parallel (or
/// either vanishes) the patch has no tangent plane to compare, and the
/// edge is refused there; only an end of the edge where the crossing
/// family's first or last curve is a point (a pole: `pole`) is passed.
fn tangency(
    patch: &NurbsSurface,
    edge: usize,
    face: &Surface,
    side: f64,
    pole: [bool; 2],
    angular: f64,
) -> Result<f64, NetworkError> {
    let label = EDGE_NAMES.get(edge).copied().unwrap_or("an edge");
    let wrapped = Surface::Nurbs(patch.clone());
    let ((u0, u1), (v0, v1)) = patch.domain();
    let along_u = edge < 2;
    let fixed = match edge {
        0 => v0,
        1 => v1,
        2 => u0,
        _ => u1,
    };
    // Into the patch: +v from v = v0, −v from v = v1, likewise in u.
    let inward = if edge == 1 || edge == 3 { -1.0 } else { 1.0 };
    let knots = if along_u {
        patch.knots_u()
    } else {
        patch.knots_v()
    };
    let mut bounds: Vec<f64> = knots.to_vec();
    bounds.dedup();
    let spans = bounds.len().saturating_sub(1).max(1);
    let count = (32 * spans).clamp(64, 4096);
    let (lo, hi) = if along_u { (u0, u1) } else { (v0, v1) };
    let mut worst = 0.0_f64;
    for k in 0..=count {
        if (k == 0 && pole[0]) || (k == count && pole[1]) {
            continue;
        }
        let t = lo + (hi - lo) * k as f64 / count as f64;
        let (u, v) = if along_u { (t, fixed) } else { (fixed, t) };
        let (p, du, dv) = wrapped.derivatives(u, v);
        let (along, into) = if along_u {
            (du, dv * inward)
        } else {
            (dv, du * inward)
        };
        let n = along.cross(into);
        let scale = along.length() * into.length();
        // NaN fails too: no tangent plane.
        let planar = n.length() > angular * scale;
        if !planar {
            return refuse(format!(
                "{label}: the surface has no tangent plane at {t:.6} along it (its derivative across the edge vanishes there or runs along the edge), so it cannot be held tangent there"
            ));
        }
        let Some(n) = n.normalized() else {
            return Err(internal());
        };
        let (a, b) = face.project(p)?;
        let Some(m) = face.normal(a, b) else {
            return refuse(format!(
                "{label}: its face has no tangent plane at {t:.6} along it"
            ));
        };
        let m = m * side;
        worst = worst.max(n.cross(m).length().atan2(n.dot(m)));
    }
    Ok(worst)
}

// ---------------------------------------------------------------------------
// The Gordon surface.

/// The surface through a network (see the module docs).
pub fn gordon(net: &Network<'_>) -> Result<Patch, NetworkError> {
    let (m, n) = (net.u.len(), net.v.len());
    if !(2..=MAX_NETWORK_CURVES).contains(&m) || !(2..=MAX_NETWORK_CURVES).contains(&n) {
        return refuse(format!(
            "a network has 2 to {MAX_NETWORK_CURVES} curves each way; this one has {m} u curves and {n} v curves"
        ));
    }
    let tol = net.tolerance;
    if !(tol.is_finite() && tol > 0.0 && net.angular.is_finite() && net.angular > 0.0) {
        return Err(
            GeomError::InvalidParameter("a network's tolerances are positive and finite").into(),
        );
    }
    for (e, c) in net.edges.iter().enumerate() {
        if let Condition::Tangent { weight, .. } = c
            && !(weight.is_finite() && *weight > 0.0)
        {
            return refuse(format!(
                "{}: the weight of a tangent edge is positive, not {weight}",
                EDGE_NAMES.get(e).copied().unwrap_or("an edge")
            ));
        }
    }
    let mut us: Vec<Track> = net
        .u
        .iter()
        .map(|c| Track::new(c, tol))
        .collect::<Result<_, _>>()?;
    let mut vs: Vec<Track> = net
        .v
        .iter()
        .map(|c| Track::new(c, tol))
        .collect::<Result<_, _>>()?;
    if us.iter().all(|t| t.point) || vs.iter().all(|t| t.point) {
        return refuse("a network needs a curve that is not a point in each family".to_string());
    }
    for (e, track) in [us.first(), us.last(), vs.first(), vs.last()]
        .into_iter()
        .enumerate()
    {
        if track.is_some_and(|t| t.point) && !matches!(net.edges.get(e), Some(Condition::Free)) {
            return refuse(format!(
                "{} is a point: a point cannot be held to a face",
                EDGE_NAMES.get(e).copied().unwrap_or("an edge")
            ));
        }
    }
    for (i, t) in us.iter().enumerate().skip(1).take(m.saturating_sub(2)) {
        if t.point {
            return refuse(format!(
                "{} is a point; only the first and the last curve of a family may be",
                name('u', i)
            ));
        }
    }
    for (j, t) in vs.iter().enumerate().skip(1).take(n.saturating_sub(2)) {
        if t.point {
            return refuse(format!(
                "{} is a point; only the first and the last curve of a family may be",
                name('v', j)
            ));
        }
    }

    // The work of finding the crossings: every pair of samples of every
    // pair of curves (`closest`).
    let sampled = |tracks: &[Track]| {
        tracks.iter().fold(0usize, |n, t| {
            n.saturating_add(samples(&t.raw, 8, 32).len())
        })
    };
    let pairs = sampled(&us).saturating_mul(sampled(&vs));
    if pairs > MAX_CROSSING_PAIRS {
        return refuse(format!(
            "the network's curves have too many knot spans to find where they cross ({pairs} pairs of samples, the most is {MAX_CROSSING_PAIRS}): fewer curves, or curves of fewer pieces"
        ));
    }

    // 1. Crossings: s[i][j] on u curve i, t[i][j] on v curve j.
    let mut s = vec![vec![0.0; n]; m];
    let mut t = vec![vec![0.0; n]; m];
    let mut gap = 0.0_f64;
    for (i, ui) in us.iter().enumerate() {
        for (j, vj) in vs.iter().enumerate() {
            let (a, b, d) = closest(ui, vj)?;
            if d > tol {
                return refuse(format!(
                    "{} and {} do not cross: they come no closer than {d:.3e} (the tolerance is {tol:.3e})",
                    name('u', i),
                    name('v', j)
                ));
            }
            gap = gap.max(d);
            set(&mut s, i, j, a)?;
            set(&mut t, i, j, b)?;
        }
    }
    // 2. Each curve run the network's way, crossed in order, ending on
    // the boundary.
    orient(&mut us, &mut s, 'u', 'v', tol)?;
    let mut tt = transpose(&t);
    orient(&mut vs, &mut tt, 'v', 'u', tol)?;
    let common_u = means(&mut s, &us)?;
    let common_v = means(&mut tt, &vs)?;
    for (i, track) in us.iter_mut().enumerate() {
        if !track.point {
            track.align(
                s.get(i).ok_or_else(internal)?,
                &common_u,
                net.angular,
                &name('u', i),
            )?;
        }
    }
    for (j, track) in vs.iter_mut().enumerate() {
        if !track.point {
            track.align(
                tt.get(j).ok_or_else(internal)?,
                &common_v,
                net.angular,
                &name('v', j),
            )?;
        }
    }

    // Tangent edges along spline faces follow the face's own parameter
    // line: the edge curve is that line, with its cross derivative.
    for e in 0..4 {
        let Some(Condition::Tangent { surface, .. }) = net.edges.get(e) else {
            continue;
        };
        let spline = match surface {
            Surface::Nurbs(sf) => Some(sf.clone()),
            Surface::Bezier(b) => Some(NurbsSurface::from_bezier(b)?),
            Surface::Torus(_) => {
                return refuse(format!(
                    "{}: tangency to a torus is not supported yet (owed: a tangent ribbon fitted with a proven bound)",
                    EDGE_NAMES.get(e).copied().unwrap_or("an edge")
                ));
            }
            _ => None,
        };
        let Some(sf) = spline else { continue };
        let (family, index) = edge_track(e, m, n);
        let track = if family == 'u' {
            us.get_mut(index)
        } else {
            vs.get_mut(index)
        }
        .ok_or_else(internal)?;
        let crossings = if family == 'u' { &common_u } else { &common_v };
        let label = EDGE_NAMES.get(e).copied().unwrap_or("an edge");
        let found = iso_track(&sf, track, tol).map_err(|err| match err {
            NetworkError::Refused(why) => NetworkError::Refused(format!("{label}: {why}")),
            other => other,
        })?;
        let Some((iso, field)) = found else {
            return refuse(format!(
                "{}: it does not run along one of its face's parameter lines; tangency to a spline face along any other curve is not supported yet (owed: a tangent ribbon fitted with a proven bound)",
                EDGE_NAMES.get(e).copied().unwrap_or("an edge")
            ));
        };
        let mut swap = Track {
            raw: iso,
            fields: vec![field],
            point: false,
            reversed: track.reversed,
        };
        // Its parameters at the crossings, then aligned as the curve was.
        let own: Vec<f64> = crossings
            .iter()
            .map(|c| {
                let p = track.at(*c)?;
                project_on(&swap, p)
            })
            .collect::<Result<_, _>>()?;
        let mut own = own;
        if let Some(f) = own.first_mut() {
            *f = 0.0;
        }
        if let Some(l) = own.last_mut() {
            *l = 1.0;
        }
        if own.windows(2).any(|w| !matches!(w, [a, b] if b > a)) {
            return refuse(format!(
                "{}: the curves crossing it are not in order along its face's parameter line",
                EDGE_NAMES.get(e).copied().unwrap_or("an edge")
            ));
        }
        swap.align(
            &own,
            crossings,
            net.angular,
            EDGE_NAMES.get(e).copied().unwrap_or("an edge"),
        )?;
        *track = swap;
    }

    // 3. One homogeneous point per crossing.
    fill_points(&mut us)?;
    fill_points(&mut vs)?;
    let mut log = vec![vec![0.0; n]; m];
    for (i, ui) in us.iter().enumerate() {
        for (j, vj) in vs.iter().enumerate() {
            let (Some(&cu), Some(&cv)) = (common_u.get(j), common_v.get(i)) else {
                return Err(internal());
            };
            let (wa, wb) = (eval(&ui.raw, cu)?[3], eval(&vj.raw, cv)?[3]);
            if !(wa > 0.0 && wb > 0.0) {
                return Err(internal());
            }
            set(&mut log, i, j, wa.ln() - wb.ln())?;
        }
    }
    let row_mean: Vec<f64> = log
        .iter()
        .map(|r| r.iter().sum::<f64>() / n as f64)
        .collect();
    let col_mean: Vec<f64> = (0..n)
        .map(|j| {
            log.iter()
                .map(|r| r.get(j).copied().unwrap_or(0.0))
                .sum::<f64>()
                / m as f64
        })
        .collect();
    let all = row_mean.iter().sum::<f64>() / m as f64;
    // Where the table is not a sum of a row's and a column's constant,
    // no constants make every pair of crossing weights agree: the
    // homogeneous points of the u curves and of the v curves then differ
    // where they cross, and the sum moves the v curves. How far is proven
    // on the result (`off_track`, held to the tolerance), so no threshold
    // is needed here.
    // u curve i times e^(−row mean), v curve j times e^(col mean − all).
    for (track, a) in us.iter_mut().zip(&row_mean) {
        track.raw = scaled(&track.raw, (-a).exp());
    }
    for (track, b) in vs.iter_mut().zip(&col_mean) {
        track.raw = scaled(&track.raw, (b - all).exp());
    }

    // 5a. Curves crossing a tangent edge leave it along the face, to the
    // side each edge's curves all leave to.
    let mut sides = [1.0; 4];
    for (e, side) in sides.iter_mut().enumerate() {
        let Some(Condition::Tangent { surface, weight }) = net.edges.get(e) else {
            continue;
        };
        *side = bend(
            e, surface, *weight, net, &mut us, &mut vs, &common_u, &common_v,
        )?;
    }

    // 4. The Boolean sum. First its size, from the knots alone, before
    // anything of that size is made: the u curves skinned across v (as
    // many columns as there are u curves) are refined with the v curves
    // skinned across u, each way.
    {
        let raws_u: Vec<&Raw> = us.iter().map(|t| &t.raw).collect();
        let raws_v: Vec<&Raw> = vs.iter().map(|t| &t.raw).collect();
        let (du, tu) = union(&raws_u)?;
        let (dv, tv) = union(&raws_v)?;
        // A family's own knots, raised with the other's skinning (degree
        // up to 3, `count − degree − 1` inner knots).
        let size = |degree: usize, runs: &[(f64, usize)], across: usize, curves: usize| {
            let d = degree.max(across);
            let own = if degree < d {
                runs.len().saturating_mul(d)
            } else {
                runs.iter().fold(0usize, |n, r| n.saturating_add(r.1))
            };
            let other = curves.saturating_sub(across + 1).saturating_mul(d);
            own.saturating_add(other).saturating_add(d + 1)
        };
        within_budget(
            size(du, &tu, (n - 1).min(3), n),
            size(dv, &tv, (m - 1).min(3), m),
        )?;
    }
    compatible_tracks(&mut us)?;
    compatible_tracks(&mut vs)?;
    let mut mismatch = 0.0_f64;
    let mut q = vec![vec![[0.0; 4]; n]; m];
    for (i, ui) in us.iter().enumerate() {
        for (j, vj) in vs.iter().enumerate() {
            let (Some(&cu), Some(&cv)) = (common_u.get(j), common_v.get(i)) else {
                return Err(internal());
            };
            let (a, b) = (eval(&ui.raw, cu)?, eval(&vj.raw, cv)?);
            let (pa, pb) = (
                point(a).ok_or_else(internal)?,
                point(b).ok_or_else(internal)?,
            );
            mismatch = mismatch.max(pa.distance(pb));
            q.get_mut(i)
                .and_then(|r| r.get_mut(j))
                .map(|slot| *slot = b)
                .ok_or_else(internal)?;
        }
    }
    let du_across = (n - 1).min(3);
    let dv_across = (m - 1).min(3);
    let raws_u: Vec<Raw> = us.iter().map(|t| t.raw.clone()).collect();
    let raws_v: Vec<Raw> = vs.iter().map(|t| t.raw.clone()).collect();
    let a = skin(&raws_u, &common_v, dv_across)?;
    let b = skin(&raws_v, &common_u, du_across)?.transposed();
    // The tensor interpolant of the crossing points.
    let rows_t: Vec<Raw> = q
        .iter()
        .map(|row| -> Result<Raw, NetworkError> {
            let values: Vec<Vec<f64>> = row.iter().map(|h| h.to_vec()).collect();
            let (knots, ctrl) = interpolate(&common_u, du_across, &values)?;
            Ok(Raw {
                degree: du_across,
                knots,
                points: ctrl
                    .iter()
                    .map(|c| {
                        [
                            c.first().copied().unwrap_or(0.0),
                            c.get(1).copied().unwrap_or(0.0),
                            c.get(2).copied().unwrap_or(0.0),
                            c.get(3).copied().unwrap_or(0.0),
                        ]
                    })
                    .collect(),
            })
        })
        .collect::<Result<_, _>>()?;
    let tensor = skin(&rows_t, &common_v, dv_across)?;
    let mut nets = [a, b, tensor];
    compatible_nets(&mut nets)?;
    let [a, b, tensor] = nets;
    let mut sum = a.clone();
    for ((row, rb), rt) in sum.rows.iter_mut().zip(&b.rows).zip(&tensor.rows) {
        if row.len() != rb.len() || row.len() != rt.len() {
            return Err(internal());
        }
        for ((x, y), z) in row.iter_mut().zip(rb).zip(rt) {
            *x = h_add(h_add(*x, *y, 1.0, 1.0), *z, 1.0, -1.0);
        }
    }
    if a.rows.len() != b.rows.len() || a.rows.len() != tensor.rows.len() {
        return Err(internal());
    }

    // 5b. Each tangent edge's cross derivative put into the face's planes.
    for e in 0..4 {
        let Some(Condition::Tangent { surface, .. }) = net.edges.get(e) else {
            continue;
        };
        let (family, index) = edge_track(e, m, n);
        let carried = if family == 'u' {
            us.get(index)
        } else {
            vs.get(index)
        }
        .map(|t| t.fields.clone())
        .unwrap_or_default();
        let (at, next) = match e {
            0 => (common_u.clone(), common_v.get(1).copied().unwrap_or(1.0)),
            1 => (
                common_u.clone(),
                1.0 - common_v.get(m.saturating_sub(2)).copied().unwrap_or(0.0),
            ),
            2 => (common_v.clone(), common_u.get(1).copied().unwrap_or(1.0)),
            _ => (
                common_v.clone(),
                1.0 - common_u.get(n.saturating_sub(2)).copied().unwrap_or(0.0),
            ),
        };
        let mut work = match e {
            0 => sum.clone(),
            1 => sum.reversed_v(),
            2 => sum.transposed(),
            _ => sum.transposed().reversed_v(),
        };
        let fields = if carried.is_empty() {
            let c = work.column(0)?;
            analytic_fields(surface, &c).ok_or_else(internal)?
        } else {
            carried
        };
        hold_tangent(&mut work, &fields, &at, next, net.angular, tol, e)?;
        sum = match e {
            0 => work,
            1 => work.reversed_v(),
            2 => work.transposed(),
            _ => work.reversed_v().transposed(),
        };
    }

    let surface = sum.surface()?;
    let boundary = [
        boundary_curve(&sum, 0, tol)?,
        boundary_curve(&sum, 1, tol)?,
        boundary_curve(&sum, 2, tol)?,
        boundary_curve(&sum, 3, tol)?,
    ];
    // Held edges, checked on the result.
    let mut held = [None; 4];
    for (e, condition) in net.edges.iter().enumerate() {
        let label = EDGE_NAMES.get(e).copied().unwrap_or("an edge");
        if let Condition::Tangent { surface: face, .. } = condition {
            let (first, last) = if e < 2 {
                (vs.first(), vs.last())
            } else {
                (us.first(), us.last())
            };
            let pole = [
                first.is_some_and(|t| t.point),
                last.is_some_and(|t| t.point),
            ];
            let side = sides.get(e).copied().unwrap_or(1.0);
            let worst = tangency(&surface, e, face, side, pole, net.angular)?;
            // The construction is exact: what is found beyond the angular
            // tolerance is a conflict (a corner's twist), not rounding.
            if worst > net.angular {
                return refuse(format!(
                    "{label}: the surface meets its face at up to {worst:.3e} rad, beyond the angular tolerance; where two held edges meet at a corner their faces must ask the same twist there (owed: a corner blend)"
                ));
            }
            if let Some(slot) = held.get_mut(e) {
                *slot = Some(worst);
            }
        }
        if let (Some(face), Some(Some(curve))) = (condition.surface(), boundary.get(e)) {
            let worst = off_face(curve, face)?;
            if worst > tol {
                return refuse(format!(
                    "{label} lies up to {worst:.3e} off its face (the tolerance is {tol:.3e})"
                ));
            }
        }
    }
    // Every curve of the network, as the surface was built through it
    // (after the bending), against the surface's own iso-curve there:
    // a proven bound, held to the tolerance. The u curves are passed
    // through exactly but for a tangent edge along v; the v curves within
    // what the sum moves them by where the crossing points or weights
    // differ, amplified by the interpolation across.
    for (family, tracks, at) in [('u', &us, &common_v), ('v', &vs, &common_u)] {
        for (i, track) in tracks.iter().enumerate() {
            let t = at.get(i).copied().ok_or_else(internal)?;
            let off = off_track(&surface, family == 'v', t, &track.raw)?;
            if off.is_nan() || off > tol {
                return refuse(format!(
                    "{}: the surface would pass up to {off:.3e} from it (the tolerance is {tol:.3e}): where it crosses the other curves their points or their weights disagree by more than one surface can take up (rational curves whose weights cannot be made to agree by constants are not one rational surface's)",
                    name(family, i)
                ));
            }
            mismatch = mismatch.max(off);
        }
    }
    Ok(Patch {
        surface,
        boundary,
        mismatch: mismatch.max(gap),
        tangency: held,
        reversed_u: us.iter().map(|t| t.reversed).collect(),
        reversed_v: vs.iter().map(|t| t.reversed).collect(),
    })
}

fn set(table: &mut [Vec<f64>], i: usize, j: usize, x: f64) -> Result<(), NetworkError> {
    let slot = table
        .get_mut(i)
        .and_then(|r| r.get_mut(j))
        .ok_or_else(internal)?;
    *slot = x;
    Ok(())
}

fn transpose(table: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let n = table.first().map_or(0, Vec::len);
    (0..n)
        .map(|j| {
            table
                .iter()
                .map(|r| r.get(j).copied().unwrap_or(0.0))
                .collect()
        })
        .collect()
}

/// Which family and curve an edge is.
fn edge_track(edge: usize, m: usize, n: usize) -> (char, usize) {
    match edge {
        0 => ('u', 0),
        1 => ('u', m - 1),
        2 => ('v', 0),
        _ => ('v', n - 1),
    }
}

/// Turns each curve (row `i` of `own`: its parameters where the other
/// family crosses it, in that family's order) to run from the other
/// family's first curve to its last, and checks the order and the ends.
fn orient(
    tracks: &mut [Track],
    own: &mut [Vec<f64>],
    family: char,
    other: char,
    tol: f64,
) -> Result<(), NetworkError> {
    for (i, (track, row)) in tracks.iter_mut().zip(own.iter_mut()).enumerate() {
        if track.point {
            continue;
        }
        let (Some(&first), Some(&last)) = (row.first(), row.last()) else {
            return Err(internal());
        };
        if first > last {
            track.reverse();
            for x in row.iter_mut() {
                *x = 1.0 - *x;
            }
        }
        let count = row.len();
        for (j, w) in row.windows(2).enumerate() {
            if let [a, b] = w
                && b <= a
            {
                return refuse(format!(
                    "{} meets the {other} curves out of order: {} crosses it before {}",
                    name(family, i),
                    name(other, j + 1),
                    name(other, j)
                ));
            }
        }
        // The first and last of the other family are the boundary.
        for (end, j, at) in [
            (0.0, 0, row.first().copied()),
            (1.0, count - 1, row.last().copied()),
        ] {
            let at = at.ok_or_else(internal)?;
            if track.at(end)?.distance(track.at(at)?) > tol {
                return refuse(format!(
                    "{} runs past {}: the first and the last curve of each family bound the network, so the others end on them",
                    name(family, i),
                    name(other, j)
                ));
            }
        }
        if let Some(f) = row.first_mut() {
            *f = 0.0;
        }
        if let Some(l) = row.last_mut() {
            *l = 1.0;
        }
    }
    Ok(())
}

/// The common crossing parameters of a family: the means over its curves
/// that are not points (a point's are set to them).
fn means(own: &mut [Vec<f64>], tracks: &[Track]) -> Result<Vec<f64>, NetworkError> {
    let count = own.first().map_or(0, Vec::len);
    let used: Vec<&Vec<f64>> = own
        .iter()
        .zip(tracks)
        .filter(|(_, t)| !t.point)
        .map(|(r, _)| r)
        .collect();
    let mut out = Vec::with_capacity(count);
    for j in 0..count {
        let sum: f64 = used.iter().map(|r| r.get(j).copied().unwrap_or(0.0)).sum();
        out.push(sum / used.len().max(1) as f64);
    }
    if out
        .windows(2)
        .any(|w| !matches!(w, [a, b] if b - a > SAME_KNOT))
    {
        return Err(internal());
    }
    for (row, track) in own.iter_mut().zip(tracks) {
        if track.point {
            *row = out.clone();
        }
    }
    Ok(out)
}

/// A curve that is a point takes the weights of its neighbour.
fn fill_points(tracks: &mut [Track]) -> Result<(), NetworkError> {
    let count = tracks.len();
    for i in 0..count {
        if !tracks.get(i).is_some_and(|t| t.point) {
            continue;
        }
        let near = (0..count)
            .filter(|k| !tracks.get(*k).is_some_and(|t| t.point))
            .min_by_key(|k| k.abs_diff(i))
            .ok_or_else(internal)?;
        let template = tracks.get(near).ok_or_else(internal)?.raw.clone();
        let track = tracks.get_mut(i).ok_or_else(internal)?;
        let p = track.at(0.0)?;
        track.raw = mapped(&template, |h| h_of(p, h[3]));
    }
    Ok(())
}

/// The parameter of the point of `track` nearest to `p`.
fn project_on(track: &Track, p: Vec3) -> Result<f64, NetworkError> {
    let d = derivative(&track.raw)?;
    let ts = samples(&track.raw, 8, 64);
    let mut best = (0.0, f64::INFINITY);
    for x in &ts {
        let gap = track.at(*x)?.distance(p);
        if gap < best.1 {
            best = (*x, gap);
        }
    }
    let mut t = best.0;
    for _ in 0..50 {
        let (q, dq) = point_and_tangent(&track.raw, &d, t)?;
        let denom = dq.dot(dq);
        if denom.is_nan() || denom <= 0.0 {
            break;
        }
        let step = (q - p).dot(dq) / denom;
        let next = (t - step).clamp(0.0, 1.0);
        if (next - t).abs() < 1e-16 {
            break;
        }
        t = next;
    }
    Ok(t)
}

/// Makes the curves crossing tangent edge `e` leave it along `face`.
#[allow(clippy::too_many_arguments)]
fn bend(
    e: usize,
    face: &Surface,
    weight: f64,
    net: &Network<'_>,
    us: &mut [Track],
    vs: &mut [Track],
    common_u: &[f64],
    common_v: &[f64],
) -> Result<f64, NetworkError> {
    let at_start = e == 0 || e == 2;
    let label = EDGE_NAMES.get(e).copied().unwrap_or("an edge");
    let (m, n) = (us.len(), vs.len());
    let (edge_family, edge_index) = edge_track(e, m, n);
    // The edge's own direction where each curve crosses it.
    let along: Vec<Vec3> = {
        let (edge, at) = if edge_family == 'u' {
            (us.get(edge_index), common_u)
        } else {
            (vs.get(edge_index), common_v)
        };
        let edge = edge.ok_or_else(internal)?;
        let d = derivative(&edge.raw)?;
        at.iter()
            .map(|t| Ok(point_and_tangent(&edge.raw, &d, *t)?.1))
            .collect::<Result<_, NetworkError>>()?
    };
    let (crossing, family, inner): (&mut [Track], char, &[f64]) = if e < 2 {
        (vs, 'v', common_v)
    } else {
        (us, 'u', common_u)
    };
    let count = crossing.len();
    let mut side: Option<(f64, usize)> = None;
    // The parameter of the next crossing along each crossing curve.
    let next = if at_start {
        inner.get(1).copied().unwrap_or(1.0)
    } else {
        inner
            .get(inner.len().saturating_sub(2))
            .copied()
            .unwrap_or(0.0)
    };
    for (k, track) in crossing.iter_mut().enumerate() {
        if track.point {
            continue;
        }
        // Worked on at its start: a curve held at its end is turned round
        // for the change and back.
        let mut raw = if at_start {
            track.raw.clone()
        } else {
            reverse(&track.raw)
        };
        let d = derivative(&raw)?;
        let (p, tangent) = point_and_tangent(&raw, &d, 0.0)?;
        let (a, b) = face.project(p)?;
        let Some(normal) = face.normal(a, b) else {
            return refuse(format!(
                "{label}: its face has no tangent plane where {} crosses it",
                name(family, k)
            ));
        };
        let flat = tangent - normal * tangent.dot(normal);
        let speed = tangent.length();
        // The side of the edge it leaves to, along the face: the sine of
        // the angle its direction makes with the edge, measured about the
        // face's normal. Within the angular tolerance of zero its
        // direction along the face is not known (it leaves square to the
        // face, or along the edge itself).
        let edge_dir = along.get(k).and_then(|d| d.normalized()).ok_or_else(|| {
            NetworkError::Refused(format!(
                "{label}: the edge has no direction where {} crosses it",
                name(family, k)
            ))
        })?;
        let sine = edge_dir.cross(tangent).dot(normal) / speed;
        if sine.is_nan() || sine.abs() <= net.angular {
            return refuse(format!(
                "{label}: {} leaves it square to its face or along the edge itself, so it cannot be made to leave along the face",
                name(family, k)
            ));
        }
        let here = sine.signum();
        match side {
            Some((s, first)) if s != here => {
                return refuse(format!(
                    "{label}: {} and {} leave it to opposite sides along its face (the surface would fold over the edge)",
                    name(family, first),
                    name(family, k)
                ));
            }
            None => side = Some((here, k)),
            _ => {}
        }
        // A curve that is itself a held edge keeps its shape: it must
        // already leave along this face.
        let held_edge = match (family, k) {
            ('v', 0) => Some(2),
            ('v', x) if x + 1 == count => Some(3),
            ('u', 0) => Some(0),
            ('u', x) if x + 1 == count => Some(1),
            _ => None,
        };
        if let Some(other) = held_edge
            && !matches!(net.edges.get(other), Some(Condition::Free))
        {
            let sine =
                tangent.cross(flat).length() / (speed * flat.length()).max(f64::MIN_POSITIVE);
            if sine.is_nan() || sine > net.angular {
                return refuse(format!(
                    "the corner of {label} and {}: {} is held to its own face and leaves {label} at {:.3e} rad off its face; at a corner both faces must share the tangent plane",
                    EDGE_NAMES.get(other).copied().unwrap_or("an edge"),
                    name(family, k),
                    sine.asin()
                ));
            }
            continue;
        }
        let target = flat * (weight * speed / flat.length());
        while raw.degree < 3 {
            raw.elevate_batched()?;
        }
        // Only the control point beside the end moves: its basis function
        // must stop short of the next crossing.
        localize(&mut raw, if at_start { next } else { 1.0 - next })?;
        let p_deg = raw.degree;
        let (Some(&h0), Some(&h1), Some(&k1), Some(&kp1)) = (
            raw.points.first(),
            raw.points.get(1),
            raw.knots.get(1),
            raw.knots.get(p_deg + 1),
        ) else {
            return Err(internal());
        };
        let (p0, w0, w1) = (point(h0).ok_or_else(internal)?, h0[3], h1[3]);
        let p1 = p0 + target * ((kp1 - k1) * w0 / (p_deg as f64 * w1));
        if let Some(slot) = raw.points.get_mut(1) {
            *slot = h_of(p1, w1);
        }
        track.raw = if at_start { raw } else { reverse(&raw) };
    }
    side.map(|(s, _)| s).ok_or_else(|| {
        NetworkError::Refused(format!(
            "{label}: every {family} curve crossing it is a point, so no curve leaves it along the face"
        ))
    })
}

/// A proven bound on the distance between the curve `raw` (homogeneous,
/// over `[0, 1]`) and the iso-curve of `surface` at `at` (along v when
/// `along_v`, at that u) run with it: with `S = V + D` the two
/// homogeneous curves, the distance at a parameter is
/// `|D_xyz − p D_w| / w_S`, `p` the curve's point there; by the convex
/// hulls of `D`, of `V`'s points (positive weights) and of `S`'s weights,
/// that is at most `(max |D_k,xyz − c D_k,w| + R max |D_k,w|) / min w_S,k`
/// for any `c`, `R` the farthest of `V`'s points from `c`.
fn off_track(
    surface: &NurbsSurface,
    along_v: bool,
    at: f64,
    raw: &Raw,
) -> Result<f64, NetworkError> {
    let (iso, _) = iso_rows(surface, along_v, at)?;
    let lightest = iso
        .points
        .iter()
        .map(|h| h[3])
        .fold(f64::INFINITY, f64::min);
    if !(lightest.is_finite() && lightest > 0.0) {
        return refuse(
            "the network's weights do not stay positive: its rational curves cannot be made into one surface".to_string(),
        );
    }
    let mut curve = raw.clone();
    curve.normalize()?;
    let mut iso = iso;
    iso.normalize()?;
    let d = combined(&iso, &curve, 1.0, -1.0)?;
    let Some(points) = curve
        .points
        .iter()
        .map(|h| point(*h))
        .collect::<Option<Vec<Vec3>>>()
    else {
        return refuse(
            "a curve of the network has a weight that is not positive once its weights were made to agree with the others': its rational curves cannot be made into one surface".to_string(),
        );
    };
    let c = points.first().copied().ok_or_else(internal)?;
    let reach = points.iter().fold(0.0_f64, |r, p| r.max(p.distance(c)));
    let (mut lead, mut weight) = (0.0_f64, 0.0_f64);
    for h in &d.points {
        lead = lead.max((xyz(*h) - c * h[3]).length());
        weight = weight.max(h[3].abs());
    }
    Ok((lead + reach * weight) / lightest)
}

/// The boundary curve of an edge of the net, `None` where it is a point.
fn boundary_curve(net: &Net, edge: usize, tol: f64) -> Result<Option<NurbsCurve>, NetworkError> {
    let raw = match edge {
        0 => net.column(0)?,
        1 => net.column(net.count_v().saturating_sub(1))?,
        2 => net.transposed().column(0)?,
        _ => net.transposed().column(net.rows.len().saturating_sub(1))?,
    };
    let first = raw
        .points
        .first()
        .copied()
        .and_then(point)
        .ok_or_else(internal)?;
    if raw
        .points
        .iter()
        .all(|h| point(*h).is_some_and(|p| p.distance(first) <= tol))
    {
        return Ok(None);
    }
    Ok(Some(raw.curve()?))
}

/// How far `curve` strays from `face`, at 64 places per knot span and
/// more.
fn off_face(curve: &NurbsCurve, face: &Surface) -> Result<f64, NetworkError> {
    let raw = of(curve);
    let mut worst = 0.0_f64;
    let (a, b) = curve.domain();
    for x in samples(&raw, 64, 256) {
        let p = curve.eval(a + (b - a) * x);
        let (u, v) = face.project(p)?;
        worst = worst.max(face.eval(u, v).distance(p));
    }
    Ok(worst)
}

// ---------------------------------------------------------------------------
// Curves for networks.

/// An arc of the circle about `frame`'s origin in its xy plane, from
/// angle `start` over `sweep` (radians, either sign, at most a full turn),
/// as a rational quadratic in pieces of at most a quarter turn, its end
/// weights 1.
pub fn arc(frame: &Frame, radius: f64, start: f64, sweep: f64) -> Result<NurbsCurve, GeomError> {
    if !(radius.is_finite() && radius > 0.0 && start.is_finite() && sweep.is_finite())
        || sweep == 0.0
        || sweep.abs() > std::f64::consts::TAU + 1e-12
    {
        return Err(GeomError::InvalidParameter(
            "an arc has a positive radius and turns by up to a full turn",
        ));
    }
    let pieces = (sweep.abs() / std::f64::consts::FRAC_PI_2 - 1e-9)
        .ceil()
        .max(1.0) as usize;
    let step = sweep / pieces as f64;
    let half = step / 2.0;
    let w = half.cos();
    let at = |a: f64, r: f64| frame.point(r * a.cos(), r * a.sin(), 0.0);
    let mut ctrl = vec![at(start, radius)];
    let mut weights = vec![1.0];
    let mut knots = vec![0.0; 3];
    for k in 0..pieces {
        let a0 = start + step * k as f64;
        ctrl.push(at(a0 + half, radius / w));
        weights.push(w);
        ctrl.push(at(a0 + step, radius));
        weights.push(1.0);
        let x = (k + 1) as f64 / pieces as f64;
        if k + 1 < pieces {
            knots.extend([x, x]);
        }
    }
    knots.extend([1.0; 3]);
    NurbsCurve::new(2, knots, ctrl, weights)
}

/// Curves joined end to end into one (each end within `tol` of the next
/// one's start), over `[0, 1]`. Where two meet tangentially (within
/// `angular`) the parameter is made to run on at the same speed: every
/// piece gets the length of its control polygon as its share of the
/// parameter, and its end speeds are brought to that rate by an exact
/// change of its parameter (three times its degree), so the joined curve
/// is once differentiable in its parameter there, as a single spline
/// would be. Corners stay corners.
pub fn chain(curves: &[NurbsCurve], tol: f64, angular: f64) -> Result<NurbsCurve, NetworkError> {
    let Some(first) = curves.first() else {
        return refuse("a chain needs a curve".to_string());
    };
    if curves.len() == 1 {
        let mut raw = of(first);
        raw.normalize()?;
        return Ok(raw.curve()?);
    }
    if curves.len() > MAX_NETWORK_CURVES * 4 {
        return refuse(format!(
            "a chain has at most {} curves",
            MAX_NETWORK_CURVES * 4
        ));
    }
    let lengths: Vec<f64> = curves
        .iter()
        .map(|c| {
            c.control_points()
                .windows(2)
                .map(|w| match w {
                    [a, b] => a.distance(*b),
                    _ => 0.0,
                })
                .sum::<f64>()
                .max(f64::MIN_POSITIVE)
        })
        .collect();
    let mut pieces: Vec<Raw> = Vec::with_capacity(curves.len());
    for (k, c) in curves.iter().enumerate() {
        let mut raw = of(c);
        if !raw.clamped() {
            return Err(GeomError::Unsupported(
                "a chained curve whose end knots are not repeated (not clamped)",
            )
            .into());
        }
        raw.normalize()?;
        let d = derivative(&raw)?;
        let len = lengths.get(k).copied().unwrap_or(1.0);
        // Tangent within what the curves are known to: the angular
        // tolerance, and the direction a tolerance's worth of position
        // makes over the shorter piece (a joint whose curves meet within
        // the tolerance may differ in direction by that much). Taking a
        // corner for smooth only evens out the speeds there, which is
        // harmless; taking a smooth joint for a corner would leave the
        // parameter's speed jumping.
        let smooth =
            |other: Option<(&NurbsCurve, f64)>, here_start: bool| -> Result<bool, NetworkError> {
                let Some((o, other_len)) = other else {
                    return Ok(false);
                };
                let (a, b) = o.domain();
                let (_, there) = o.derivatives(if here_start { b } else { a });
                let (_, here) = point_and_tangent(&raw, &d, if here_start { 0.0 } else { 1.0 })?;
                let sine = here.cross(there).length()
                    / (here.length() * there.length()).max(f64::MIN_POSITIVE);
                let shorter = len.min(other_len);
                Ok(here.dot(there) > 0.0 && sine <= angular + tol / shorter)
            };
        let neighbour = |i: usize| curves.get(i).zip(lengths.get(i).copied());
        let before = smooth(k.checked_sub(1).and_then(neighbour), true)?;
        let after = smooth(neighbour(k + 1), false)?;
        let (_, d0) = point_and_tangent(&raw, &d, 0.0)?;
        let (_, d1) = point_and_tangent(&raw, &d, 1.0)?;
        let f0 = if before {
            len / d0.length().max(f64::MIN_POSITIVE)
        } else {
            1.0
        };
        let f1 = if after {
            len / d1.length().max(f64::MIN_POSITIVE)
        } else {
            1.0
        };
        if (f0 - 1.0).abs() > 1e-12 || (f1 - 1.0).abs() > 1e-12 {
            let made = crate::refine::reparametrized(&raw.clone().curve()?, f0, f1).map_err(|_| {
                NetworkError::Refused(format!(
                    "curve {} of the chain changes speed too much along it to be joined smoothly",
                    k + 1
                ))
            })?;
            raw = of(&made);
            raw.normalize()?;
        }
        pieces.push(raw);
    }
    // One degree, and each piece's weights scaled to meet the one before.
    let degree = pieces.iter().map(|p| p.degree).max().unwrap_or(1);
    let mut prev_end: Option<H> = None;
    for (k, piece) in pieces.iter_mut().enumerate() {
        while piece.degree < degree {
            piece.elevate()?;
        }
        let (Some(&start), Some(&end)) = (piece.points.first(), piece.points.last()) else {
            return Err(internal());
        };
        if let Some(prev) = prev_end {
            let (Some(a), Some(b)) = (point(prev), point(start)) else {
                return Err(internal());
            };
            if a.distance(b) > tol {
                return refuse(format!(
                    "curve {} of the chain does not start where curve {k} ends (they are {:.3e} apart)",
                    k + 1,
                    a.distance(b)
                ));
            }
            let f = prev[3] / start[3];
            *piece = scaled(piece, f);
            if let Some(s) = piece.points.first_mut() {
                *s = prev;
            }
            prev_end = piece.points.last().copied();
        } else {
            prev_end = Some(end);
        }
    }
    let total: f64 = lengths.iter().sum();
    let mut bounds = vec![0.0];
    let mut acc = 0.0;
    for l in &lengths {
        acc += l;
        bounds.push(acc / total);
    }
    if let Some(last) = bounds.last_mut() {
        *last = 1.0;
    }
    Ok(concat(&pieces, &bounds)?.curve()?)
}

// ---------------------------------------------------------------------------
// Seams.

/// How two surfaces meet along a curve: the largest distance between them
/// and the largest angle between their tangent planes, found at `count`
/// places along it (with where each is), and the largest change of either
/// between neighbouring places (between two places the true largest
/// exceeds the found one by about half of it at most where it changes
/// monotonically there: an estimate, not a proof).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeamGaps {
    pub position: f64,
    pub position_at: Vec3,
    pub angle: f64,
    pub angle_at: Vec3,
    pub position_step: f64,
    pub angle_step: f64,
}

/// The gaps of position (G0) and tangent plane (G1) between `a` and `b`
/// along `curve` over `range`, at `count` places (at least 2, at most
/// 2^16).
pub fn seam_gaps(
    curve: &crate::Curve3,
    range: (f64, f64),
    a: &Surface,
    b: &Surface,
    count: usize,
) -> Result<SeamGaps, GeomError> {
    if !(2..=1 << 16).contains(&count) || !(range.0.is_finite() && range.1.is_finite()) {
        return Err(GeomError::InvalidParameter(
            "a seam is measured at 2 to 65536 places over a finite range",
        ));
    }
    let mut out = SeamGaps {
        position: 0.0,
        position_at: curve.eval(range.0),
        angle: 0.0,
        angle_at: curve.eval(range.0),
        position_step: 0.0,
        angle_step: 0.0,
    };
    let mut last: Option<(f64, f64)> = None;
    for k in 0..count {
        let t = range.0 + (range.1 - range.0) * k as f64 / (count - 1) as f64;
        let p = curve.eval(t);
        let (ua, va) = a.project(p)?;
        let (ub, vb) = b.project(p)?;
        let (pa, pb) = (a.eval(ua, va), b.eval(ub, vb));
        let gap = pa.distance(pb);
        let angle = match (a.normal(ua, va), b.normal(ub, vb)) {
            (Some(x), Some(y)) => x.cross(y).length().min(1.0).asin(),
            _ => 0.0,
        };
        if gap > out.position {
            out.position = gap;
            out.position_at = p;
        }
        if angle > out.angle {
            out.angle = angle;
            out.angle_at = p;
        }
        if let Some((g, an)) = last {
            out.position_step = out.position_step.max((gap - g).abs());
            out.angle_step = out.angle_step.max((angle - an).abs());
        }
        last = Some((gap, angle));
    }
    Ok(out)
}

// Keep the cap of the banded interpolation in view: a family never has
// more curves than it can interpolate.
const _: () = assert!(MAX_NETWORK_CURVES < MAX_INTERPOLATED_BANDED && MAX_DEGREE >= 9);
