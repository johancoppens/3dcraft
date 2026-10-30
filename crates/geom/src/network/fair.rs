//! Fair changes across an edge held in curvature (G2).
//!
//! Holding an edge in curvature changes the curves crossing it, and then
//! the surface's rows beside the edge, by a change that must vanish at the
//! edge (position), may change the derivative across it only as asked
//! (tangency) and its second derivative only as asked (curvature), and
//! must be gone by the next curve of the network. Moving one control
//! point for each condition (the second for tangency, the third for
//! curvature) squeezes the change into the first knot spans: the curvature
//! across then overshoots to several times the face's and swings back
//! within a narrow band beside the seam (measured on the guide's fin, a
//! post of radius 20: from 1/20 at the seam to −0.35 a few millimetres
//! out, four changes of sign).
//!
//! Here the change is `d(t) = D · G(t)`, `D` the change asked at the edge
//! and `G = Σ c_j N_j` a scalar spline on the curve's knots that meets the
//! conditions at `t = 0` and is the one of least curvature variation
//! `∫ G‴²` among those that vanish, with every derivative the spline has,
//! from the reach on (Farin, *Curves and Surfaces for CAGD*, ch. 23, the
//! energy `∫ ‖x‴‖²` of fairing; constrained by the end conditions, it is a
//! linear system in the free coefficients). The conditions are linear in
//! the coefficients, so the change is exact: the seam gets precisely what
//! the one-point change gave it, and the measure (`continuity`) proves it
//! as before. The reach is cut into [`FAIR_SPANS`] equal spans so that
//! there are coefficients to spread the change over.
//!
//! Where the next curve is the sheet's far edge and that edge is free,
//! the change runs over the whole curve and keeps only its last point
//! (as a tangent hold's change of the second control point always did):
//! nothing then asks the curvature to swing back. The surface's own
//! corrections between the crossings (the tangent and curvature holds
//! along the edge) keep the far edge's position, tangent plane and
//! curvature even then ([`End::Kept`]): they are not asked for by any
//! curve, and spread to a free far edge they bent it between the curves
//! (measured: a dome's far edge from −1.4 to −9.6 between two of its
//! crossings).
//!
//! What no such change can do: one that ends with the curve's own position
//! and tangent has `∫ Δκ ds ≈ 0` and `∫ (L − s) Δκ ds ≈ 0` over its reach
//! `L` (to first order in the change), so the change of curvature `Δκ`
//! must change sign twice. The curvature along a line from the seam goes
//! from the face's to the sheet's own with one swing past it, of least
//! variation: for a straight curve (the quintic `t²(1 − t)³` in the
//! limit of fine spans) the swing is at most about 0.38 of the change at
//! the seam, and back by 0.22.

use super::{NetworkError, Raw, derivative, eval, internal, refuse, second_at_start};
use crate::refine::SAME_KNOT;
use craft3d_math::quadrature::gauss_legendre;

/// Equal spans the reach of a curvature edge is cut into.
pub(super) const FAIR_SPANS: usize = 16;

/// How far (in the crossing curve's parameter, from the edge at 0) a
/// change for a curvature edge may reach: to the next curve crossing it
/// (where it ends with every derivative the spline has); where that is
/// the opposite edge, over the whole curve with only its last point kept
/// (1: the sheet's far edge is free, its points stay on its curve), or
/// half way when the opposite edge is held tangent or in curvature itself
/// (its own rows take the other half).
pub(super) fn limit(next: f64, far_held: bool) -> f64 {
    if next >= 1.0 - SAME_KNOT {
        if far_held { 0.5 } else { 1.0 }
    } else {
        next
    }
}

/// What a change over a reach to the curve's far end keeps there (an
/// inner reach always ends with every derivative the spline has).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum End {
    /// Only the last point: a curve crossing the edge, bent (its far end
    /// stays on the far edge, which is free).
    Point,
    /// The far edge's position, tangent plane and curvature (the last
    /// `degree` control points): the surface's corrections between the
    /// crossings, which must not disturb the far edge where no curve
    /// asks it.
    Kept,
}

/// Whether a reach runs to the curve's far end.
fn open(end: f64) -> bool {
    end >= 1.0 - SAME_KNOT
}

/// What a profile changes at the edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Order {
    /// `G(0) = 0`, `G′(0) = 1` and `G″(0)` what moving the second control
    /// point alone gives (the first three coefficients are that move's):
    /// the derivative across, the second derivative as the one-point
    /// change left it, so the curvature step that follows starts from the
    /// same second derivative at the edge as before, and only the rest is
    /// spread.
    Tangent,
    /// `G(0) = G′(0) = 0`, `G″(0) = 1`: the second derivative only.
    Curvature,
}

/// Knots inserted at every `end · j / FAIR_SPANS` (`end` from [`limit`])
/// that `raw` does not have yet, so that its first three basis functions
/// end within the reach and there are coefficients to spread a change
/// over.
pub(super) fn localize(raw: &mut Raw, end: f64) -> Result<(), NetworkError> {
    if end <= SAME_KNOT {
        return Ok(());
    }
    for j in 1..=FAIR_SPANS {
        let t = end * j as f64 / FAIR_SPANS as f64;
        if t >= 1.0 - SAME_KNOT || raw.knots.iter().any(|k| (k - t).abs() <= SAME_KNOT) {
            continue;
        }
        raw.insert(t)?;
    }
    let p = raw.degree;
    match raw.knots.get(p + 3) {
        Some(&last) if last <= end + SAME_KNOT => Ok(()),
        _ => Err(internal()),
    }
}

/// The coefficients `c_j` (one per control point) of the scalar spline of
/// `degree` on `knots` that meets `order` at 0, vanishes from `end` on
/// (only basis functions that end by then take part; for an open reach,
/// `end` = 1, all but those `far` keeps) and has the least `∫ G‴²` among
/// those.
pub(super) fn profile(
    knots: &[f64],
    degree: usize,
    end: f64,
    order: Order,
    far: End,
) -> Result<Vec<f64>, NetworkError> {
    let q = degree;
    let count = knots
        .len()
        .checked_sub(q + 1)
        .filter(|c| *c > 0)
        .ok_or_else(internal)?;
    let inside = if open(end) {
        count.saturating_sub(match far {
            End::Point => 1,
            End::Kept => q,
        })
    } else {
        (0..count)
            .take_while(|j| knots.get(j + q + 1).is_some_and(|k| *k <= end + SAME_KNOT))
            .count()
    };
    if q < 3 || inside < 3 {
        return Err(internal());
    }
    let (Some(&k1), Some(&kq1)) = (knots.get(1), knots.get(q + 1)) else {
        return Err(internal());
    };
    let a1 = q as f64 / (kq1 - k1);
    let [_, _, gamma] = second_at_start(q, knots).ok_or_else(internal)?;
    let mut c = vec![0.0; count];
    match order {
        Order::Tangent => {
            let c1 = 1.0 / a1;
            if let Some(s) = c.get_mut(1) {
                *s = c1;
            }
        }
        Order::Curvature => {
            if let Some(s) = c.get_mut(2) {
                *s = 1.0 / gamma;
            }
        }
    }
    if c.iter().any(|x| !x.is_finite()) {
        return Err(internal());
    }
    let free = inside - 3;
    if free == 0 {
        return Ok(c);
    }
    // The third derivatives of the basis functions that take part.
    let thirds: Vec<Raw> = (0..inside)
        .map(|j| {
            let mut points = vec![[0.0; 4]; count];
            if let Some(slot) = points.get_mut(j) {
                *slot = [1.0, 0.0, 0.0, 0.0];
            }
            let unit = Raw {
                degree: q,
                knots: knots.to_vec(),
                points,
            };
            derivative(&derivative(&derivative(&unit)?)?)
        })
        .collect::<Result<_, _>>()?;
    // Their Gram matrix over the reach, exactly: Gauss–Legendre with
    // q − 2 nodes per span integrates the products (degree 2q − 6).
    let nodes = gauss_legendre(q - 2);
    let mut gram = vec![vec![0.0; inside]; inside];
    for w in knots.windows(2) {
        let [a, b] = w else { continue };
        if !(b - a > SAME_KNOT && *b <= end + SAME_KNOT) {
            continue;
        }
        let (mid, half) = (0.5 * (a + b), 0.5 * (b - a));
        for (x, weight) in &nodes {
            let t = mid + half * x;
            let values: Vec<f64> = thirds
                .iter()
                .map(|d| eval(d, t).map(|h| h[0]))
                .collect::<Result<_, _>>()?;
            for (row, vi) in gram.iter_mut().zip(&values) {
                for (slot, vj) in row.iter_mut().zip(&values) {
                    *slot += weight * half * vi * vj;
                }
            }
        }
    }
    // The free coefficients (3 and on): Σ_g M_fg x_g = −Σ_k M_fk c_k.
    let mut system: Vec<Vec<f64>> = (3..inside)
        .map(|f| {
            let row = gram.get(f).ok_or_else(internal)?;
            let mut out: Vec<f64> = row.get(3..inside).ok_or_else(internal)?.to_vec();
            let rhs: f64 = (0..3)
                .map(|k| row.get(k).copied().unwrap_or(0.0) * c.get(k).copied().unwrap_or(0.0))
                .sum();
            out.push(-rhs);
            Ok(out)
        })
        .collect::<Result<_, NetworkError>>()?;
    let x = solve(&mut system, free)?;
    for (slot, value) in c.iter_mut().skip(3).zip(x) {
        *slot = value;
    }
    Ok(c)
}

/// Gaussian elimination with partial pivoting on the augmented rows of a
/// symmetric positive definite system of `n` unknowns.
fn solve(rows: &mut [Vec<f64>], n: usize) -> Result<Vec<f64>, NetworkError> {
    let scale = (0..n)
        .filter_map(|i| rows.get(i).and_then(|r| r.get(i)))
        .fold(0.0_f64, |m, x| m.max(x.abs()));
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|a, b| {
                let size = |r: usize| {
                    rows.get(r)
                        .and_then(|row| row.get(col))
                        .map_or(0.0, |v| v.abs())
                };
                size(*a).total_cmp(&size(*b))
            })
            .ok_or_else(internal)?;
        rows.swap(col, pivot);
        let (top, rest) = rows.split_at_mut(col + 1);
        let lead = top.last().ok_or_else(internal)?;
        let diagonal = lead.get(col).copied().unwrap_or(0.0);
        if !(diagonal.is_finite() && diagonal.abs() > 1e-13 * scale) {
            return refuse(
                "the change across a curvature edge could not be spread over its reach (its fairness system is singular)"
                    .to_string(),
            );
        }
        for row in rest.iter_mut().take(n - col - 1) {
            let factor = row.get(col).copied().unwrap_or(0.0) / diagonal;
            if factor == 0.0 {
                continue;
            }
            for (slot, value) in row.iter_mut().zip(lead).skip(col) {
                *slot -= factor * value;
            }
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let row = rows.get(i).ok_or_else(internal)?;
        let mut value = row.get(n).copied().unwrap_or(0.0);
        for (a, xj) in row.iter().zip(&x).skip(i + 1).take(n - i - 1) {
            value -= a * xj;
        }
        let diagonal = row.get(i).copied().unwrap_or(0.0);
        let solved = value / diagonal;
        if !solved.is_finite() {
            return Err(internal());
        }
        if let Some(slot) = x.get_mut(i) {
            *slot = solved;
        }
    }
    Ok(x)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn knots(q: usize, inner: &[f64]) -> Vec<f64> {
        let mut k = vec![0.0; q + 1];
        k.extend_from_slice(inner);
        k.extend(vec![1.0; q + 1]);
        k
    }

    fn scalar(q: usize, k: &[f64], c: &[f64]) -> Raw {
        Raw {
            degree: q,
            knots: k.to_vec(),
            points: c.iter().map(|x| [*x, 0.0, 0.0, 0.0]).collect(),
        }
    }

    #[test]
    fn profiles_meet_their_conditions_and_vanish_past_the_reach() {
        for q in [3, 4, 5] {
            let inner: Vec<f64> = (1..=8).map(|j| 0.5 * j as f64 / 8.0).collect();
            let k = knots(q, &inner);
            for order in [Order::Tangent, Order::Curvature] {
                let c = profile(&k, q, 0.5, order, End::Point).unwrap();
                let g = scalar(q, &k, &c);
                let g1 = derivative(&g).unwrap();
                let g2 = derivative(&g1).unwrap();
                let at = |r: &Raw, t: f64| eval(r, t).unwrap()[0];
                let [_, beta, _] = second_at_start(q, &k).unwrap();
                let want = match order {
                    Order::Tangent => (0.0, 1.0, beta * k[q + 1] / q as f64),
                    Order::Curvature => (0.0, 0.0, 1.0),
                };
                assert!((at(&g, 0.0) - want.0).abs() < 1e-12, "{q} {order:?}");
                assert!((at(&g1, 0.0) - want.1).abs() < 1e-9, "{q} {order:?}");
                assert!((at(&g2, 0.0) - want.2).abs() < 1e-9, "{q} {order:?}");
                for t in [0.5, 0.6, 0.9, 1.0] {
                    assert!(at(&g, t).abs() < 1e-15, "{q} {order:?} at {t}");
                }
            }
        }
    }

    #[test]
    fn an_open_profile_keeps_only_the_last_point() {
        let q = 3;
        let inner: Vec<f64> = (1..8).map(|j| j as f64 / 8.0).collect();
        let k = knots(q, &inner);
        for order in [Order::Tangent, Order::Curvature] {
            let c = profile(&k, q, 1.0, order, End::Point).unwrap();
            assert_eq!(*c.last().unwrap(), 0.0);
            assert!(
                c.iter().rev().skip(1).take(3).any(|x| x.abs() > 1e-6),
                "{c:?}"
            );
            let g = scalar(q, &k, &c);
            let g2 = derivative(&derivative(&g).unwrap()).unwrap();
            let [_, beta, _] = second_at_start(q, &k).unwrap();
            let want = match order {
                Order::Tangent => beta * k[q + 1] / q as f64,
                Order::Curvature => 1.0,
            };
            assert!((eval(&g2, 0.0).unwrap()[0] - want).abs() < 1e-9);
        }
    }

    /// The curvature profile tends to the quintic's: `G″ = (1 − s)(20s² −
    /// 16s + 2)/2` (s = t / reach), one swing below of about −0.375 and one
    /// above of about 0.215.
    #[test]
    fn the_curvature_profile_swings_once_each_way_by_the_quintics_amounts() {
        let q = 3;
        let inner: Vec<f64> = (1..=8).map(|j| 0.5 * j as f64 / 8.0).collect();
        let k = knots(q, &inner);
        let c = profile(&k, q, 0.5, Order::Curvature, End::Kept).unwrap();
        let g2 = derivative(&derivative(&scalar(q, &k, &c)).unwrap()).unwrap();
        let values: Vec<f64> = (0..500)
            .map(|i| eval(&g2, 0.5 * i as f64 / 500.0).unwrap()[0])
            .collect();
        let signs = values.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        let low = values.iter().fold(f64::MAX, |m, x| m.min(*x));
        let high = values.iter().skip(250).fold(f64::MIN, |m, x| m.max(*x));
        assert_eq!(signs, 2, "{values:?}");
        assert!((-0.45..=-0.3).contains(&low), "{low}");
        assert!((0.15..=0.3).contains(&high), "{high}");
    }
}
