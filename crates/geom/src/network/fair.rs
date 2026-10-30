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
//! must change sign twice (on an open reach, where only the far point
//! is kept, only the second holds, and once suffices). The curvature along a line from the seam goes
//! from the face's to the sheet's own with one swing past it, of least
//! variation: for a straight curve (the quintic `t²(1 − t)³` in the
//! limit of fine spans) the swing is at most about 0.38 of the change at
//! the seam, and back by 0.22.

use super::{NetworkError, Raw, internal, refuse, second_at_start};
#[cfg(test)]
use super::{derivative, eval};
use crate::refine::SAME_KNOT;
use craft3d_math::quadrature::gauss_legendre;

/// Equal spans the reach of a curvature edge is cut into.
pub(super) const FAIR_SPANS: usize = 8;

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
    /// `G(0) = 0`, `G′(0) = 1`, `G″(0) = 0`: the derivative across, and
    /// not the second derivative.
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
///
/// The minimum is sought among the splines on the reach's own grid
/// (`end · j / FAIR_SPANS`, each taken as the curve's knot within
/// `SAME_KNOT` of it, which [`localize`] put there) and then written on the
/// curve's knots by knot refinement (exact: the grid is a subsequence of
/// the curve's knots). A curve with many knots of its own would make the
/// least-variation system as ill-conditioned as its spans are many
/// (`∫ G‴²` grows as the fifth power of the inverse span, so 2¹⁶ spans
/// read as singular); on the grid it is always the same small, well
/// conditioned system, and the work stays linear in the curve's points.
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
    // The grid, as the curve's own knots.
    let last = if open(end) {
        FAIR_SPANS - 1
    } else {
        FAIR_SPANS
    };
    let mut grid = Vec::with_capacity(last);
    for j in 1..=last {
        let t = end * j as f64 / FAIR_SPANS as f64;
        if t >= 1.0 - SAME_KNOT {
            continue;
        }
        let k = knots
            .iter()
            .copied()
            .filter(|k| (k - t).abs() <= SAME_KNOT)
            .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
            .ok_or_else(internal)?;
        if grid.last().is_none_or(|g| k > *g) {
            grid.push(k);
        }
    }
    let (Some(&a), Some(&b)) = (knots.first(), knots.last()) else {
        return Err(internal());
    };
    let mut coarse = vec![a; q + 1];
    coarse.extend_from_slice(&grid);
    coarse.extend(vec![b; q + 1]);
    let values = profile_on(&coarse, q, end, order, far)?;
    // Written on the curve's knots: insert those the grid lacks.
    let mut missing = Vec::new();
    let mut g = grid.iter().peekable();
    for k in knots.get(q + 1..count).ok_or_else(internal)? {
        if g.peek().is_some_and(|x| **x == *k) {
            g.next();
        } else {
            missing.push(*k);
        }
    }
    let mut raw = Raw {
        degree: q,
        knots: coarse,
        points: values.iter().map(|c| [*c, 0.0, 0.0, 0.0]).collect(),
    };
    raw.insert_all(&missing)?;
    if raw.points.len() != count {
        return Err(internal());
    }
    Ok(raw.points.iter().map(|h| h[0]).collect())
}

/// [`profile`] on the knots given, solved there.
fn profile_on(
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
    let [_, beta, gamma] = second_at_start(q, knots).ok_or_else(internal)?;
    let mut c = vec![0.0; count];
    match order {
        Order::Tangent => {
            let c1 = 1.0 / a1;
            if let Some(s) = c.get_mut(1) {
                *s = c1;
            }
            if let Some(s) = c.get_mut(2) {
                *s = -beta * c1 / gamma;
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
    // The Gram matrix of the basis functions' third derivatives over the
    // reach, exactly (Gauss–Legendre with q − 2 nodes per span integrates
    // their products, of degree 2q − 6) and banded: on a span only q + 1
    // of them are not zero, so it is kept as `band[i][o] = M[i][i + o]`,
    // o ≤ q, and the work and room are linear in the control points.
    let nodes_count = q - 2;
    if nodes_count > craft3d_math::quadrature::MAX_NODES {
        return Err(internal());
    }
    let nodes = gauss_legendre(nodes_count);
    let mut band = vec![vec![0.0; q + 1]; inside];
    for s in q..count {
        let (Some(&a), Some(&b)) = (knots.get(s), knots.get(s + 1)) else {
            break;
        };
        if !(b - a > SAME_KNOT && b <= end + SAME_KNOT) {
            continue;
        }
        let (mid, half) = (0.5 * (a + b), 0.5 * (b - a));
        for (x, weight) in &nodes {
            let t = mid + half * x;
            let values = thirds_at(knots, q, s, t);
            let first = s - q;
            for (i, vi) in values.iter().enumerate() {
                let row = first + i;
                if row >= inside {
                    continue;
                }
                for (j, vj) in values.iter().enumerate().skip(i) {
                    if first + j >= inside {
                        continue;
                    }
                    if let Some(slot) = band.get_mut(row).and_then(|r| r.get_mut(j - i)) {
                        *slot += weight * half * vi * vj;
                    }
                }
            }
        }
    }
    // The free coefficients (3 and on): Σ_g M_fg x_g = −Σ_k M_fk c_k, the
    // fixed ones being 1 and 2 (0 is zero).
    let entry = |i: usize, j: usize| -> f64 {
        let (lo, hi) = (i.min(j), i.max(j));
        band.get(lo)
            .and_then(|r| r.get(hi - lo))
            .copied()
            .unwrap_or(0.0)
    };
    let rhs: Vec<f64> = (3..inside)
        .map(|f| {
            -(1..3)
                .map(|k| entry(f, k) * c.get(k).copied().unwrap_or(0.0))
                .sum::<f64>()
        })
        .collect();
    let system: Vec<Vec<f64>> = (3..inside)
        .map(|f| (0..=q).map(|o| entry(f, f + o)).collect())
        .collect();
    let x = solve_banded_spd(&system, &rhs, q)?;
    for (slot, value) in c.iter_mut().skip(3).zip(x) {
        *slot = value;
    }
    Ok(c)
}

/// `(q − 2)/(t_{i+q+1} − t_{i+3})`-style factors of the derivative's
/// recurrence: `p / (knots[i + p + 1 + lift] − knots[i + 1 + lift])`, zero
/// on an empty span (as `derivative` takes them).
fn factor(knots: &[f64], p: usize, i: usize, lift: usize) -> f64 {
    match (knots.get(i + q_plus(p, lift)), knots.get(i + 1 + lift)) {
        (Some(hi), Some(lo)) if hi > lo => p as f64 / (hi - lo),
        _ => 0.0,
    }
}

fn q_plus(p: usize, lift: usize) -> usize {
    p + 1 + lift
}

/// The coefficients of `c_k … c_{k+3}` in the `k`-th control value of
/// the third derivative of `Σ c_j N_j` (degree `q`): three differences,
/// each with its factor (Piegl & Tiller, eq. 3.4 applied thrice).
fn third_row(knots: &[f64], q: usize, k: usize) -> [f64; 4] {
    // c'_i = a1_i (c_{i+1} − c_i), on degree q.
    let a1 = |i: usize| factor(knots, q, i, 0);
    // c''_i = a2_i (c'_{i+1} − c'_i), on degree q − 1 (knots shifted by one).
    let a2 = |i: usize| factor(knots, q - 1, i, 1);
    let a3 = |i: usize| factor(knots, q - 2, i, 2);
    let first = |i: usize| [-a1(i), a1(i)];
    let second = |i: usize| {
        let (lo, hi) = (first(i), first(i + 1));
        let f = a2(i);
        [-f * lo[0], f * (hi[0] - lo[1]), f * hi[1]]
    };
    let (lo, hi) = (second(k), second(k + 1));
    let f = a3(k);
    [
        -f * lo[0],
        f * (hi[0] - lo[1]),
        f * (hi[1] - lo[2]),
        f * hi[2],
    ]
}

/// The third derivatives at `t` (in the span `[knots[s], knots[s + 1])`)
/// of the `q + 1` basis functions of degree `q` not zero there,
/// `N_{s−q} … N_s`: the degree `q − 3` basis there (Cox–de Boor, Piegl
/// & Tiller A2.2) combined by [`third_row`].
fn thirds_at(knots: &[f64], q: usize, s: usize, t: f64) -> Vec<f64> {
    let r = q - 3;
    // Degree r on the knots without their first and last three:
    // knots3[x] = knots[x + 3]; t lies in its span s − 3.
    let k3 = |x: usize| knots.get(x + 3).copied().unwrap_or(0.0);
    let sp = s - 3;
    let mut n = vec![0.0; r + 1];
    if let Some(n0) = n.get_mut(0) {
        *n0 = 1.0;
    }
    let mut left = vec![0.0; r + 1];
    let mut right = vec![0.0; r + 1];
    for j in 1..=r {
        if let Some(l) = left.get_mut(j) {
            *l = t - k3(sp + 1 - j);
        }
        if let Some(rt) = right.get_mut(j) {
            *rt = k3(sp + j) - t;
        }
        let mut saved = 0.0;
        for i in 0..j {
            let ri = right.get(i + 1).copied().unwrap_or(0.0);
            let lj = left.get(j - i).copied().unwrap_or(0.0);
            let denom = ri + lj;
            let temp = if denom != 0.0 {
                n.get(i).copied().unwrap_or(0.0) / denom
            } else {
                0.0
            };
            if let Some(slot) = n.get_mut(i) {
                *slot = saved + ri * temp;
            }
            saved = lj * temp;
        }
        if let Some(slot) = n.get_mut(j) {
            *slot = saved;
        }
    }
    // M_{sp − r + i} = n[i]; M_k's control value of the third derivative
    // is Σ_m row(k)[m] c_{k+m}, and M_k of knots3 index k pairs with the
    // original coefficients from k on (the derivative drops one knot each
    // side, so indices of control values stay aligned).
    let mut out = vec![0.0; q + 1];
    let first_j = s - q;
    for (i, mi) in n.iter().enumerate() {
        let k = sp - r + i;
        let row = third_row(knots, q, k);
        for (m, lm) in row.iter().enumerate() {
            if let Some(slot) = (k + m).checked_sub(first_j).and_then(|o| out.get_mut(o)) {
                *slot += mi * lm;
            }
        }
    }
    out
}

/// Solves the symmetric positive definite banded system (`rows[i][o]` is
/// the entry at `(i, i + o)`, `o ≤ width`) by LDLᵀ, after scaling it to
/// unit diagonal (Jacobi), so that the test for a singular system is
/// relative: a pivot at or below 1e-12 of its unit diagonal is refused.
fn solve_banded_spd(
    rows: &[Vec<f64>],
    rhs: &[f64],
    width: usize,
) -> Result<Vec<f64>, NetworkError> {
    let n = rows.len();
    let at = |i: usize, o: usize| rows.get(i).and_then(|r| r.get(o)).copied().unwrap_or(0.0);
    let singular = || {
        refuse::<Vec<f64>>(
            "the change across a curvature edge could not be spread over its reach (its fairness system is singular)"
                .to_string(),
        )
    };
    let mut scale = Vec::with_capacity(n);
    for i in 0..n {
        let d = at(i, 0);
        if !(d.is_finite() && d > 0.0) {
            return singular();
        }
        scale.push(1.0 / d.sqrt());
    }
    let sc = |i: usize| scale.get(i).copied().unwrap_or(0.0);
    // lower[j][o] = L[j][j − o], o in 1..=width; diag[i] = D[i].
    let mut lower = vec![vec![0.0; width + 1]; n];
    let mut diag = vec![0.0; n];
    let l = |lower: &Vec<Vec<f64>>, j: usize, i: usize| -> f64 {
        j.checked_sub(i)
            .filter(|o| *o >= 1 && *o <= width)
            .and_then(|o| lower.get(j).and_then(|r| r.get(o)))
            .copied()
            .unwrap_or(0.0)
    };
    for i in 0..n {
        let from = i.saturating_sub(width);
        let mut d = at(i, 0) * sc(i) * sc(i);
        for k in from..i {
            let lik = l(&lower, i, k);
            d -= lik * lik * diag.get(k).copied().unwrap_or(0.0);
        }
        if !(d.is_finite() && d > 1e-12) {
            return singular();
        }
        if let Some(slot) = diag.get_mut(i) {
            *slot = d;
        }
        for j in (i + 1)..n.min(i + width + 1) {
            let mut v = at(i, j - i) * sc(i) * sc(j);
            for k in j.saturating_sub(width)..i {
                v -= l(&lower, i, k) * l(&lower, j, k) * diag.get(k).copied().unwrap_or(0.0);
            }
            if let Some(slot) = lower.get_mut(j).and_then(|r| r.get_mut(j - i)) {
                *slot = v / d;
            }
        }
    }
    // L y = S b, D z = y, Lᵀ x̂ = z, x = S x̂.
    let mut y: Vec<f64> = (0..n)
        .map(|i| rhs.get(i).copied().unwrap_or(0.0) * sc(i))
        .collect();
    for i in 0..n {
        let mut v = y.get(i).copied().unwrap_or(0.0);
        for k in i.saturating_sub(width)..i {
            v -= l(&lower, i, k) * y.get(k).copied().unwrap_or(0.0);
        }
        if let Some(slot) = y.get_mut(i) {
            *slot = v;
        }
    }
    for (v, d) in y.iter_mut().zip(&diag) {
        *v /= d;
    }
    for i in (0..n).rev() {
        let mut v = y.get(i).copied().unwrap_or(0.0);
        for j in (i + 1)..n.min(i + width + 1) {
            v -= l(&lower, j, i) * y.get(j).copied().unwrap_or(0.0);
        }
        if let Some(slot) = y.get_mut(i) {
            *slot = v;
        }
    }
    let x: Vec<f64> = y.iter().enumerate().map(|(i, v)| v * sc(i)).collect();
    if x.iter().any(|v| !v.is_finite()) {
        return Err(internal());
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
                let want = match order {
                    Order::Tangent => (0.0, 1.0, 0.0),
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
            assert!(
                (eval(&g2, 0.0).unwrap()[0] - f64::from(u8::from(order == Order::Curvature))).abs()
                    < 1e-9
            );
        }
    }

    /// A dense curve (nearly 2^16 control points, the most a spline may have)
    /// with an open reach: the whole curve takes part, and the banded
    /// solve stays linear in it (the dense one held a 34 GB Gram matrix).
    #[test]
    fn a_profile_over_a_dense_curve_is_linear_in_its_points() {
        let q = 3;
        // Spans a multiple of the grid's, so the grid is among the knots
        // (as `localize` makes it).
        let m = (crate::MAX_CONTROL_POINTS - q) / FAIR_SPANS * FAIR_SPANS;
        let count = m + q;
        let inner: Vec<f64> = (1..m).map(|j| j as f64 / m as f64).collect();
        let k = knots(q, &inner);
        let started = std::time::Instant::now();
        for order in [Order::Tangent, Order::Curvature] {
            let c = profile(&k, q, 1.0, order, End::Point).unwrap();
            assert_eq!(c.len(), count);
            assert!(c.iter().all(|x| x.is_finite()));
        }
        let took = started.elapsed();
        assert!(took.as_secs_f64() < 5.0, "{took:?}");
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
