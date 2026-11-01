//! A spline face written across any curve on it (docs/design/surfacing.md
//! › *Holds and the measure along any curve on a spline face*).
//!
//! A spline face `Φ(u, v)` and a curve on it `c(t) = (u(t), v(t))` in its
//! parameters give the *ribbon* `R(t, ρ) = Φ(c(t) + ρ e)`, `e` the unit
//! vector of the parameter axis that stays furthest from the curve's
//! direction: an exact local parametrisation of the face in which the
//! curve is the line `ρ = 0`. Its rows across, `R = Φ∘c`,
//! `R_ρ = Φ_e∘c` and `R_ρρ = Φ_ee∘c`, are exact (rational) splines in `t`:
//! the face's homogeneous derivative nets composed with `c`, patch by
//! patch, by the same Bernstein products as a trimmed face's edges
//! ([`crate::trim`]). They are what a parameter line of the face gives
//! (its iso-curve and the derivatives across it), so the holds (G1, G2)
//! and the continuity measure take them unchanged.
//!
//! Where `c` comes from: an edge that is itself a curve on the face
//! (a trimmed face's edge, a patch's edge built here) is `Φ∘c` for a
//! piecewise polynomial `c` in the edge's own parameter; `c` is recovered
//! piece by piece from the parameters of points of the edge (a polynomial
//! of degree `k` through `k + 1` of them) and accepted only where the
//! distance between `Φ∘c` and the edge, bounded by the convex hull of
//! their difference, is within a hundredth of the tolerance
//! ([`Ribbon::exact`]). Otherwise, for holds only, `c` is the cubic
//! through the projections of the edge's points (as a trimmed face's
//! boundary projected onto it), followed within half the tolerance at the
//! midpoints between samples; the measure takes only recovered curves.

use super::{
    H, NetworkError, Raw, Track, eval, internal, pieces_any, point, refine_to, refuse, union,
};
use crate::refine::bernstein_product;
use crate::trim::{self, BezierNet, Composed, TrimError, compose_net};
use crate::{Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Portable, Vec3};

/// Most pieces of an edge a curve in parameter space is recovered over.
const MAX_RECOVERED_PIECES: usize = 1 << 13;

/// The least sine between the curve's direction in parameter space and
/// the axis the ribbon runs across along: below it the ribbon's two
/// directions are too close to be a parametrisation the rows can be
/// trusted in.
const LEAST_ACROSS_SINE: f64 = 0.05;

/// How close `Φ∘c` must lie to the edge, as a share of the tolerance,
/// for `c` to count as the edge's own curve in parameter space.
const RECOVERED_WITHIN: f64 = 1e-2;

fn from_trim(e: TrimError) -> NetworkError {
    match e {
        TrimError::Refused(why) => NetworkError::Refused(why),
        TrimError::Geom(g) => NetworkError::Geom(g),
    }
}

/// A spline face written across a curve on it.
#[derive(Clone, Debug)]
pub(super) struct Ribbon {
    /// The curve in the face's parameters (`x` = u, `y` = v), over the
    /// edge's parameter range.
    pub(super) uv: NurbsCurve,
    /// Whether the ribbon runs across along u (else along v).
    pub(super) across_u: bool,
    /// `Φ∘c`, `Φ_e∘c`, `Φ_ee∘c` (homogeneous).
    pub(super) rows: [Raw; 3],
    /// Whether `c` was recovered as the edge's own curve on the face
    /// (else projected, sampled).
    pub(super) exact: bool,
}

/// Bernstein basis value `B_j^k(s)`.
fn bernstein(k: usize, j: usize, s: f64) -> f64 {
    let binom = (0..j.min(k)).fold(1.0, |acc, i| acc * (k - i) as f64 / (i + 1) as f64);
    let (j32, rest) = (
        i32::try_from(j).unwrap_or(i32::MAX),
        i32::try_from(k.saturating_sub(j)).unwrap_or(i32::MAX),
    );
    binom * s.ppowi(j32) * (1.0 - s).ppowi(rest)
}

/// The Bernstein coefficients of the polynomial of degree `k` through
/// `values` at `s = i / k` (Gaussian elimination with partial pivoting on
/// the small collocation matrix; `None` where it is singular).
fn through(k: usize, values: &[(f64, f64)]) -> Option<Vec<(f64, f64)>> {
    let n = k + 1;
    if values.len() != n {
        return None;
    }
    let mut a: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            let s = i as f64 / k as f64;
            let mut row: Vec<f64> = (0..n).map(|j| bernstein(k, j, s)).collect();
            let (u, v) = values.get(i).copied().unwrap_or((0.0, 0.0));
            row.push(u);
            row.push(v);
            row
        })
        .collect();
    for col in 0..n {
        let pivot = (col..n).max_by(|x, y| {
            let fx = a.get(*x).and_then(|r| r.get(col)).map_or(0.0, |v| v.abs());
            let fy = a.get(*y).and_then(|r| r.get(col)).map_or(0.0, |v| v.abs());
            fx.total_cmp(&fy)
        })?;
        a.swap(col, pivot);
        let lead = a.get(col)?.get(col).copied()?;
        if lead.abs() < 1e-12 {
            return None;
        }
        let row = a.get(col)?.clone();
        for (r, other) in a.iter_mut().enumerate() {
            if r == col {
                continue;
            }
            let f = other.get(col).copied().unwrap_or(0.0) / lead;
            for (x, y) in other.iter_mut().zip(&row) {
                *x -= f * y;
            }
        }
    }
    (0..n)
        .map(|i| {
            let r = a.get(i)?;
            let lead = r.get(i)?;
            Some((r.get(n)? / lead, r.get(n + 1)? / lead))
        })
        .collect()
}

/// The parameters of `p`'s closest point on `surface` from `start` (a
/// closest point found to the projection's own tolerance), polished by
/// Gauss–Newton steps on `|Φ(u, v) − p|²` to the rounding of the point:
/// a point of the edge lies on the face, so the residual vanishes and the
/// steps converge quadratically; a step that does not shrink the
/// residual ends it.
fn polished(surface: &Surface, p: Vec3, start: (f64, f64)) -> (f64, f64) {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let (mut u, mut v) = start;
    let mut gap = surface.eval(u, v).distance(p);
    for _ in 0..8 {
        let (q, su, sv) = surface.derivatives(u, v);
        let r = p - q;
        let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
        let (x, y) = (su.dot(r), sv.dot(r));
        let det = a * c - b * b;
        if !(det.is_finite() && det > 0.0) {
            break;
        }
        let (du, dv) = ((x * c - y * b) / det, (y * a - x * b) / det);
        let (nu, nv) = ((u + du).clamp(u0, u1), (v + dv).clamp(v0, v1));
        let next = surface.eval(nu, nv).distance(p);
        if next.is_nan() || next >= gap {
            break;
        }
        (u, v, gap) = (nu, nv, next);
    }
    (u, v)
}

/// The curve in `surface`'s parameters whose image is the edge `raw`,
/// piecewise polynomial of degree `k` between the edge's knots, from the
/// parameters of `k + 1` points of each piece (shared at the ends, so the
/// pieces join). `None` where a projection fails or the system is
/// singular.
fn recovered_of_degree(
    wrapped: &Surface,
    raw: &Raw,
    bounds: &[f64],
    k: usize,
) -> Result<Option<NurbsCurve>, NetworkError> {
    let at = |t: f64| -> Result<Vec3, NetworkError> { point(eval(raw, t)?).ok_or_else(internal) };
    let mut last: Option<(f64, f64)> = None;
    let mut project = |p: Vec3| -> Option<(f64, f64)> {
        let found = match last {
            Some(s) => wrapped.project_from(p, s).ok(),
            None => wrapped.project(p).ok(),
        };
        let found = found.map(|f| polished(wrapped, p, f));
        if found.is_some() {
            last = found;
        }
        found
    };
    let mut ctrl: Vec<Vec3> = Vec::new();
    let mut knots: Vec<f64> = Vec::new();
    let mut previous_end: Option<(f64, f64)> = None;
    let pieces = bounds.len().saturating_sub(1);
    for piece in 0..pieces {
        let (Some(&t0), Some(&t1)) = (bounds.get(piece), bounds.get(piece + 1)) else {
            return Err(internal());
        };
        let mut values = Vec::with_capacity(k + 1);
        for i in 0..=k {
            if i == 0
                && let Some(v) = previous_end
            {
                values.push(v);
                continue;
            }
            let t = t0 + (t1 - t0) * i as f64 / k as f64;
            let Some(v) = project(at(t)?) else {
                return Ok(None);
            };
            values.push(v);
        }
        let Some(coefficients) = through(k, &values) else {
            return Ok(None);
        };
        previous_end = values.last().copied();
        if piece == 0 {
            knots.extend(std::iter::repeat_n(t0, k + 1));
        } else {
            knots.extend(std::iter::repeat_n(t0, k));
        }
        for (j, (u, v)) in coefficients.into_iter().enumerate() {
            if piece > 0 && j == 0 {
                continue;
            }
            ctrl.push(Vec3::new(u, v, 0.0));
        }
        if piece + 1 == pieces {
            knots.extend(std::iter::repeat_n(t1, k + 1));
        }
    }
    Ok(NurbsCurve::polynomial(k, knots, ctrl).ok())
}

/// A proven bound on the distance between the curves `a` and `b` over one
/// parameter (homogeneous, positive weights): the numerator
/// `a_xyz b_w − b_xyz a_w` about the point `o`, over `a_w b_w`, each by the
/// convex hull of its Bernstein coefficients. Infinite where the weights'
/// product is not proven positive.
fn distance_bound(a: &Raw, b: &Raw, o: Vec3) -> Result<f64, NetworkError> {
    let local = |r: &Raw| Raw {
        degree: r.degree,
        knots: r.knots.clone(),
        points: r
            .points
            .iter()
            .map(|h| {
                [
                    h[0] - o.x * h[3],
                    h[1] - o.y * h[3],
                    h[2] - o.z * h[3],
                    h[3],
                ]
            })
            .collect(),
    };
    let (mut a, mut b) = (local(a), local(b));
    // One description for both (knots within rounding of each other are
    // one knot), so that their products are taken piece by piece.
    let (degree, target) = union(&[&a, &b])?;
    refine_to(&mut a, degree, &target)?;
    refine_to(&mut b, degree, &target)?;
    // Piece by piece (no spline of the products' degree is made: it may
    // pass the degree a spline may have).
    let (_, pa) = pieces_any(&a)?;
    let (_, pb) = pieces_any(&b)?;
    if pa.len() != pb.len() {
        return Err(internal());
    }
    let (mut lead, mut lightest) = (0.0_f64, f64::INFINITY);
    for (x, y) in pa.iter().zip(&pb) {
        let coord = |h: &[H], c: usize| -> Vec<f64> {
            h.iter().map(|p| p.get(c).copied().unwrap_or(0.0)).collect()
        };
        let (xw, yw) = (coord(x, 3), coord(y, 3));
        let mut n = Vec::with_capacity(3);
        for c in 0..3 {
            let ab = bernstein_product(&coord(x, c), &yw);
            let ba = bernstein_product(&coord(y, c), &xw);
            n.push(ab.iter().zip(&ba).map(|(p, q)| p - q).collect::<Vec<f64>>());
        }
        if let [n0, n1, n2] = n.as_slice() {
            for ((p, q), r) in n0.iter().zip(n1).zip(n2) {
                lead = lead.max((p * p + q * q + r * r).sqrt());
            }
        }
        for w in bernstein_product(&xw, &yw) {
            lightest = lightest.min(w);
        }
    }
    if !(lightest.is_finite() && lightest > 0.0) {
        return Ok(f64::INFINITY);
    }
    Ok(lead / lightest)
}

/// The edge's own curve in `surface`'s parameters, where it is one:
/// cubic pieces first, then pieces of the degree the edge's degree asks
/// (`deg / (p + q)`, at most 5). With the proven distance between the edge
/// and its composition.
fn recovered(
    surface: &NurbsSurface,
    net: &BezierNet,
    raw: &Raw,
    tol: f64,
) -> Result<Option<(NurbsCurve, f64)>, NetworkError> {
    let (bounds, _) = pieces_any(raw)?;
    if bounds.len() > MAX_RECOVERED_PIECES {
        return Ok(None);
    }
    let wrapped = Surface::Nurbs(surface.clone());
    let o = raw
        .points
        .first()
        .and_then(|h| point(*h))
        .ok_or_else(internal)?;
    let (p, q) = surface.degrees();
    // The lowest degree that reproduces the edge: a quadratic fitted as
    // a cubic is the same curve of a higher degree (and its composition
    // too).
    let mut degrees = vec![1usize, 2, 3];
    if p + q > 0 && raw.degree.is_multiple_of(p + q) {
        let k = raw.degree / (p + q);
        if k > 3 && k <= 5 {
            degrees.push(k);
        }
    }
    for k in degrees {
        let Some(uv) = recovered_of_degree(&wrapped, raw, &bounds, k)? else {
            continue;
        };
        let composed = match compose_net(net, &uv, Composed::Positive) {
            Ok(c) => c,
            Err(TrimError::Refused(_)) => continue,
            Err(e) => return Err(from_trim(e)),
        };
        let gap = distance_bound(&composed, raw, o)?;
        if gap <= RECOVERED_WITHIN * tol {
            return Ok(Some((uv, gap)));
        }
    }
    Ok(None)
}

/// Whether the ribbon runs across along u: the axis whose least sine with
/// the curve's direction (sampled at 16 places per piece) is the larger,
/// refused below [`LEAST_ACROSS_SINE`].
fn across_axis(uv: &NurbsCurve) -> Result<bool, NetworkError> {
    let raw = Raw::of(uv);
    let d = super::derivative(&raw)?;
    let (bounds, _) = pieces_any(&raw)?;
    let (mut least_u, mut least_v) = (f64::INFINITY, f64::INFINITY);
    for w in bounds.windows(2) {
        let [a, b] = w else { continue };
        for i in 0..=16 {
            let t = a + (b - a) * f64::from(i) / 16.0;
            let h = eval(&d, t)?;
            let len = h[0].phypot(h[1]);
            if len.is_nan() || len <= 0.0 {
                return refuse(
                    "the curve stands still in its face's parameters, so the face cannot be written across it".to_string(),
                );
            }
            // Across along u: the sine with the u axis is |v'| / |c'|.
            least_u = least_u.min(h[1].abs() / len);
            least_v = least_v.min(h[0].abs() / len);
        }
    }
    let across_u = least_u > least_v;
    if least_u.max(least_v) < LEAST_ACROSS_SINE {
        return refuse(format!(
            "the curve turns through both parameter directions of its face (it comes within {:.3} of each axis), so the face is not written across it yet (owed: the ribbon across along the curve's normal in parameter space)",
            least_u.max(least_v)
        ));
    }
    Ok(across_u)
}

/// The rows of the ribbon across `uv`: `Φ∘c`, `Φ_e∘c`, `Φ_ee∘c`.
pub(super) fn rows(
    net: &BezierNet,
    uv: &NurbsCurve,
    across_u: bool,
) -> Result<[Raw; 3], NetworkError> {
    let first = net.derivative(across_u).map_err(from_trim)?;
    let second = first.derivative(across_u).map_err(from_trim)?;
    Ok([
        compose_net(net, uv, Composed::Positive).map_err(from_trim)?,
        compose_net(&first, uv, Composed::Any).map_err(from_trim)?,
        compose_net(&second, uv, Composed::Any).map_err(from_trim)?,
    ])
}

/// The face written across the edge `track`, when the edge lies on it:
/// the edge's own curve in the face's parameters where it is one, else
/// (with `sampled`) the projection of its points. `None` where the edge
/// does not lie on the face within the tolerance (or, without `sampled`,
/// is not recovered as a curve of it).
pub(super) fn ribbon(
    surface: &NurbsSurface,
    track: &Track,
    tol: f64,
    sampled: bool,
) -> Result<Option<Ribbon>, NetworkError> {
    let net = BezierNet::of(surface).map_err(from_trim)?;
    let raw = &track.raw;
    let (uv, exact) = match recovered(surface, &net, raw, tol)? {
        Some((uv, _)) => (uv, true),
        None if sampled => {
            let wrapped = Surface::Nurbs(surface.clone());
            let curve = raw.clone().curve()?;
            // The edge must lie on the face: its points within the
            // tolerance of their closest points.
            let (a, b) = raw.domain()?;
            let mut last = None;
            for i in 0..=64 {
                let t = a + (b - a) * f64::from(i) / 64.0;
                let p = curve.eval(t);
                let found = match last {
                    Some(s) => wrapped.project_from(p, s)?,
                    None => wrapped.project(p)?,
                };
                last = Some(found);
                if wrapped.eval(found.0, found.1).distance(p) > tol {
                    return Ok(None);
                }
            }
            match trim::project_points(
                &wrapped,
                &Curve3::Nurbs(curve),
                (a, b),
                trim::Projection::Normal,
                0.5 * tol,
                tol,
            ) {
                Ok((uv, _)) => (uv, false),
                Err(TrimError::Refused(_)) => return Ok(None),
                Err(e) => return Err(from_trim(e)),
            }
        }
        None => return Ok(None),
    };
    let across_u = across_axis(&uv)?;
    let rows = rows(&net, &uv, across_u)?;
    Ok(Some(Ribbon {
        uv,
        across_u,
        rows,
        exact,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::trim::{compose, uv_through};
    use craft3d_math::Frame;

    #[test]
    fn a_curve_composed_on_a_rational_drum_is_recovered_exactly_as_a_cubic() {
        let r = 2.0;
        let circle = super::super::arc(&Frame::WORLD, r, -1.0, 2.0).unwrap();
        let n = circle.control_points().len();
        let mut points = Vec::new();
        let mut weights = Vec::new();
        for (p, w) in circle.control_points().iter().zip(circle.weights()) {
            points.push(*p + Vec3::new(0.0, 0.0, -r));
            points.push(*p + Vec3::new(0.0, 0.0, 2.0 * r));
            weights.push(*w);
            weights.push(*w);
        }
        let drum = NurbsSurface::new(
            2,
            1,
            circle.knots().to_vec(),
            vec![0.0, 0.0, 1.0, 1.0],
            n,
            2,
            points,
            weights,
        )
        .unwrap();
        let c = uv_through(
            &[0.0, 0.25, 0.5, 0.75, 1.0],
            &[(0.1, 0.3), (0.3, 0.35), (0.5, 0.4), (0.7, 0.35), (0.9, 0.3)],
        )
        .unwrap();
        let edge = compose(&drum, &c).unwrap();
        let track = Track::new(&edge, 1e-6).unwrap();
        let net = BezierNet::of(&drum).unwrap();
        let (bounds, _) = pieces_any(&track.raw).unwrap();
        let wrapped = Surface::Nurbs(drum.clone());
        let o = point(track.raw.points[0]).unwrap();
        for k in 1..=3 {
            let uv = recovered_of_degree(&wrapped, &track.raw, &bounds, k).unwrap();
            let uv = uv.unwrap();
            let composed = compose_net(&net, &uv, Composed::Positive).unwrap();
            let gap = distance_bound(&composed, &track.raw, o).unwrap();
            if k == 3 {
                assert!(gap <= 1e-13, "{gap:e}");
            } else {
                // A line or a parabola does not follow the cubic.
                assert!(gap > 1e-6, "{gap:e}");
            }
        }
    }
}
