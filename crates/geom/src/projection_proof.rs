//! A proven bound on how far a projected trim's edge lies from the true
//! projection of its curve, everywhere along it, not only at samples
//! (docs/design/surfacing.md › *A projected edge, proven*).
//!
//! The edge is `S(c(t))`: `S` a (rational) tensor-product spline (a spline
//! face, or an analytic face's rational form), `c` the cubic through the
//! parameters of the projected samples. The true projection of the curve
//! `γ(t)` is a root `(u, v)` of a condition `F(t, u, v) = 0`:
//!
//! - *closest points*: `(γ − S)·S_u = (γ − S)·S_v = 0` (the foot of the
//!   perpendicular), written homogeneously as `F_u = A·B_u`, `F_v = A·B_v`
//!   with `A = ω X − w Γ` (`S = X / w`, `γ = Γ / ω`) and
//!   `B_u = w X_u − w_u X` (`w²` times `S_u`), so that `F` is a polynomial
//!   in `t` along `c`;
//! - *along a direction `d`*: `Q A = 0`, `Q` the two rows square to `d`.
//!
//! Over each piece of `t` (the composed edge's Bézier pieces, cut further
//! at the curve's own knots), the proof is Krawczyk's (Moore, Kearfott &
//! Cloud, *Introduction to Interval Analysis*, SIAM 2009, §8.2) with a
//! moving centre: for every `t` of the piece, the box `c(t) + [−r, r]²`
//! holds exactly one root of `F(t, ·)` when
//! `|Y F(t, c(t))|∞ + ‖I − Y J‖∞ r < r`, `J` the Jacobian of `F` in
//! `(u, v)` enclosed over every such box and every `t` of the piece, `Y`
//! any matrix (the inverse of `J`'s midpoint). `F(t, c(t))` is a
//! polynomial whose Bernstein coefficients are computed in floating point
//! from the composed pieces (as `trim::compose` makes the edge), each
//! within `γ_K` of the exact one times its terms' magnitudes (*Rounding
//! counts* below), so its bound is the hull of its coefficients plus that
//! allowance; `J` is
//! enclosed by interval arithmetic on the hulls of the surface's Bézier
//! net and its derivative nets cut to the box (the convex hull property).
//! The edge's distance from the root is then at most
//! `|δ_u| max |S_u| + |δ_v| max |S_v|` over the box, `|S_u| = |B_u| / w²`,
//! `δ` bounded coordinate by coordinate by Krawczyk's operator.
//!
//! A piece whose contraction `‖I − Y J‖` is not below one is halved, in a
//! fixed order, up to a depth and a number of pieces; past them the
//! answer is "not proven" with the place, never a guess.

// `!(a > b)` is meant: a comparison with a non-number fails it.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::refine::{Raw, bezier_pieces_raw};
use crate::trim::{BezierNet, Composed, Projection, TrimError, compose_net_pieces, span_of};
use crate::{NurbsCurve, NurbsSurface};
use craft3d_math::Vec3;

type H = [f64; 4];

const EPS: f64 = f64::EPSILON;

/// Below this an absolute rounding (an underflow) is folded into every
/// widened interval.
const TINY: f64 = 1e-300;

/// Most pieces one curve's proof may take (halvings included).
pub const MAX_PROOF_PIECES: usize = 1 << 15;

/// Most pieces all the proofs of one trimmed face may take together
/// (every curve, every refinement): past it the trim is refused.
pub const MAX_TRIM_PROOF_WORK: usize = 1 << 20;

/// Deepest halving of one piece.
const MAX_DEPTH: u32 = 20;

fn refuse<T>(why: String) -> Result<T, TrimError> {
    Err(TrimError::Refused(why))
}

// ---------------------------------------------------------------------
// Intervals, rounded outward.

#[derive(Clone, Copy, Debug)]
struct Iv {
    lo: f64,
    hi: f64,
}

/// The whole line: what a product with a non-number encloses.
const WHOLE: Iv = Iv {
    lo: f64::NEG_INFINITY,
    hi: f64::INFINITY,
};

impl Iv {
    /// `[lo, hi]` widened by the rounding of the operation that made the
    /// ends (half an ulp each; one `ε` of the end and an underflow taken).
    fn widened(lo: f64, hi: f64) -> Iv {
        if lo.is_nan() || hi.is_nan() {
            return WHOLE;
        }
        Iv {
            lo: lo - (lo.abs() * EPS + TINY),
            hi: hi + (hi.abs() * EPS + TINY),
        }
    }

    fn point(x: f64) -> Iv {
        Iv { lo: x, hi: x }
    }

    fn add(self, o: Iv) -> Iv {
        Iv::widened(self.lo + o.lo, self.hi + o.hi)
    }

    fn sub(self, o: Iv) -> Iv {
        Iv::widened(self.lo - o.hi, self.hi - o.lo)
    }

    fn mul(self, o: Iv) -> Iv {
        let c = [
            self.lo * o.lo,
            self.lo * o.hi,
            self.hi * o.lo,
            self.hi * o.hi,
        ];
        if c.iter().any(|x| x.is_nan()) {
            return WHOLE;
        }
        let lo = c.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = c.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Iv::widened(lo, hi)
    }

    /// The largest magnitude in the interval.
    fn mag(self) -> f64 {
        self.lo.abs().max(self.hi.abs())
    }

    fn mid(self) -> f64 {
        0.5 * self.lo + 0.5 * self.hi
    }

    fn finite(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite()
    }
}

type V3 = [Iv; 3];

fn v_sub(a: V3, b: V3) -> V3 {
    [a[0].sub(b[0]), a[1].sub(b[1]), a[2].sub(b[2])]
}

fn v_add(a: V3, b: V3) -> V3 {
    [a[0].add(b[0]), a[1].add(b[1]), a[2].add(b[2])]
}

fn v_scale(s: Iv, a: V3) -> V3 {
    [s.mul(a[0]), s.mul(a[1]), s.mul(a[2])]
}

fn v_dot(a: V3, b: V3) -> Iv {
    a[0].mul(b[0]).add(a[1].mul(b[1])).add(a[2].mul(b[2]))
}

/// An upper bound on the length of any vector in the box `a`.
fn v_norm(a: V3) -> f64 {
    // Scaled by the largest magnitude, so that squares neither underflow
    // nor overflow; then three quotients, three products, two sums, a
    // correctly rounded root and a product.
    let m = a.iter().fold(0.0_f64, |m, x| m.max(x.mag()));
    if !m.is_finite() {
        return f64::INFINITY;
    }
    if m == 0.0 {
        return 0.0;
    }
    let s = a.iter().fold(0.0_f64, |acc, x| {
        let y = x.mag() / m;
        acc + y * y
    });
    m * s.sqrt() * (1.0 + 12.0 * EPS) + TINY
}

// ---------------------------------------------------------------------
// Bernstein polynomials.

/// Binomial coefficients up to a degree, by Pascal's rule (exact below
/// 2⁵³, a rounding each above).
struct Binomials {
    rows: Vec<Vec<f64>>,
}

impl Binomials {
    fn new() -> Binomials {
        Binomials {
            rows: vec![vec![1.0]],
        }
    }

    fn grow(&mut self, n: usize) {
        while self.rows.len() <= n {
            let last = self.rows.last().cloned().unwrap_or_else(|| vec![1.0]);
            let mut next = Vec::with_capacity(last.len() + 1);
            next.push(1.0);
            for w in last.windows(2) {
                if let [a, b] = w {
                    next.push(a + b);
                }
            }
            next.push(1.0);
            self.rows.push(next);
        }
    }

    fn get(&self, n: usize, k: usize) -> f64 {
        self.rows
            .get(n)
            .and_then(|r| r.get(k))
            .copied()
            .unwrap_or(0.0)
    }

    /// The Bernstein coefficients of the product of two polynomials given
    /// by theirs.
    fn product(&mut self, a: &[f64], b: &[f64]) -> Vec<f64> {
        let (m, k) = (a.len().saturating_sub(1), b.len().saturating_sub(1));
        if a.is_empty() || b.is_empty() {
            return Vec::new();
        }
        self.grow(m + k);
        let mut out = vec![0.0; m + k + 1];
        for (i, x) in a.iter().enumerate() {
            let ci = self.get(m, i);
            for (j, y) in b.iter().enumerate() {
                let f = ci * self.get(k, j) / self.get(m + k, i + j);
                if let Some(slot) = out.get_mut(i + j) {
                    *slot += f * x * y;
                }
            }
        }
        out
    }
}

fn elevate_to(a: &[f64], n: usize, b: &mut Binomials) -> Vec<f64> {
    let m = a.len().saturating_sub(1);
    if m >= n {
        return a.to_vec();
    }
    b.product(a, &vec![1.0; n - m + 1])
}

/// `a − b` coefficientwise, raised to a common degree.
fn poly_sub(a: &[f64], c: &[f64], b: &mut Binomials) -> Vec<f64> {
    let n = a.len().max(c.len()).saturating_sub(1);
    let (a, c) = (elevate_to(a, n, b), elevate_to(c, n, b));
    a.iter().zip(&c).map(|(x, y)| x - y).collect()
}

fn poly_add(a: &[f64], c: &[f64], b: &mut Binomials) -> Vec<f64> {
    let n = a.len().max(c.len()).saturating_sub(1);
    let (a, c) = (elevate_to(a, n, b), elevate_to(c, n, b));
    a.iter().zip(&c).map(|(x, y)| x + y).collect()
}

/// De Casteljau's split of `c` at `t`: the coefficients over `[0, t]` and
/// over `[t, 1]`, each run over `[0, 1]`.
fn split(c: &[f64], t: f64) -> (Vec<f64>, Vec<f64>) {
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
}

/// The Bernstein coefficients of `c` over `[lo, hi]` (either may lie a
/// little outside `[0, 1]`: the polynomial continued), run over `[0, 1]`,
/// taken in the order that keeps the second split's parameter within
/// `[0, 1]` up to that continuation.
fn sub(c: &[f64], lo: f64, hi: f64) -> Vec<f64> {
    if hi >= 0.5 {
        let (below, _) = split(c, hi);
        split(&below, lo / hi).1
    } else {
        let (_, above) = split(c, lo);
        let rest = 1.0 - lo;
        if rest > 0.0 {
            split(&above, (hi - lo) / rest).0
        } else {
            above
        }
    }
}

fn bern_eval(c: &[f64], t: f64) -> f64 {
    let (left, _) = split(c, t);
    left.last().copied().unwrap_or(0.0)
}

fn max_abs(c: &[f64]) -> f64 {
    c.iter().fold(0.0_f64, |m, x| m.max(x.abs()))
}

fn coord(h: &[H], k: usize) -> Vec<f64> {
    h.iter().map(|p| p.get(k).copied().unwrap_or(0.0)).collect()
}

/// `x` to the power `n`, by products (no platform `powi`).
fn power(x: f64, n: usize) -> f64 {
    (0..n).fold(1.0, |acc, _| acc * x)
}

/// `γ_K = K ε / (1 − K ε)`: the relative rounding of `K` operations.
fn gamma(k: usize) -> f64 {
    let ke = k as f64 * EPS;
    if ke >= 0.5 {
        return f64::INFINITY;
    }
    ke / (1.0 - ke)
}

// ---------------------------------------------------------------------
// Rounding counts (docs/design/surfacing.md › *A projected edge, proven*,
// *The rounding*). Every coefficient below is formed by a chain of
// floating-point operations on stored data; its computed value is within
// `γ_K` (`K ε / (1 − K ε)`) of the exact one times the same expression
// on the magnitudes of its terms (Higham, *Accuracy and Stability of
// Numerical Algorithms*, 2nd ed., §3.1: a product of `K` factors
// `(1 + δᵢ)`, `|δᵢ| ≤ ε`, is within `γ_K`; sums of such terms keep the
// bound term by term). Relative errors of factors add in a product, so
// the counts of a chain add.

/// `refine::bernstein_product` for a product of degree `n`: each of its
/// three binomials is a loop of at most `n` products and `n` quotients,
/// two more operations form the factor, two the term, and at most `n + 1`
/// terms are summed into a coefficient: `7 n + 6`.
fn count_refine_product(n: usize) -> usize {
    7 * n + 6
}

/// [`Binomials::product`] for a product of degree `n`: Pascal's entries of
/// row `n` are sums of at most `n` rounded additions (exact while below
/// 2⁵³, within `γ_n` above), three of them and two operations form the
/// factor, two the term, and at most `n + 1` terms are summed: `4 n + 5`.
fn count_product(n: usize) -> usize {
    4 * n + 5
}

/// A Bézier patch of degrees `(p, q)` extracted from the spline by knot
/// insertion: a point is reached by at most `2 p` insertions in u (at the
/// two knots bounding its span) and `2 q` in v, each a convex combination
/// of five operations (its ratio's difference and quotient, two products,
/// a sum): `10 (p + q)`.
fn count_net(p: usize, q: usize) -> usize {
    10 * (p + q)
}

/// `trim::compose_piece` of a patch of degrees `(p, q)` with a curve
/// piece of degree `d`: the local parameters (four operations), the
/// powers `s^k` of degree `k d` (one product each, `k ≤ p`), the Bernstein
/// functions `B_i^p(s)` (one product and a binomial of `2 p`), their
/// products (degree `d (p + q)`), the point's product and the sum over
/// the `(p + 1)(q + 1)` points.
fn count_compose(p: usize, q: usize, d: usize) -> usize {
    let side =
        |k: usize| 4 + k * count_refine_product(k * d) + count_refine_product(k * d) + 2 * k + 1;
    side(p) + side(q) + count_refine_product(d * (p + q)) + 1 + (p + 1) * (q + 1)
}

/// A cut by [`sub`] of a polynomial with `n` coefficients: two
/// de Casteljau splits of `n` levels, three operations each.
fn count_cut(n: usize) -> usize {
    6 * n
}

// ---------------------------------------------------------------------
// The surface: its net, derivative nets, and their hulls over a box.

/// A net's homogeneous values over a box, written about a point.
#[derive(Clone, Copy, Debug)]
struct Encl {
    x: V3,
    w: Iv,
}

/// The nets of `S` about the curve's first point `o`: `S` itself, its
/// first and second derivatives, per patch the sizes the rounding is
/// taken of, and the continuity across each knot line.
struct Nets {
    x: BezierNet,
    u: BezierNet,
    v: BezierNet,
    uu: BezierNet,
    uv: BezierNet,
    vv: BezierNet,
    /// The surface's degrees.
    degrees: (usize, usize),
    /// Per patch: the largest `|X| + |o| |w|` (a coordinate written
    /// about `o`, with its translation's terms), the largest `|X|` (about
    /// the origin, as the stored edge is composed) and the largest `|w|`.
    /// Lengths and weights are kept apart: they are of different units.
    sizes: Vec<(f64, f64, f64)>,
    /// Per patch: bounds on how far each translated point of `x` is from
    /// the exact one (its extraction and translation), its coordinates
    /// and its weight, which a derivative net's difference carries times
    /// `2 p / Δ` per order.
    base_err: Vec<(f64, f64)>,
    /// The multiplicity in the surface's knots of each of `x.ku` and
    /// `x.kv` (a line inside the domain is `C^(p − m)`).
    mult_u: Vec<usize>,
    mult_v: Vec<usize>,
}

impl Nets {
    fn of(surface: &NurbsSurface, o: Vec3) -> Result<Nets, TrimError> {
        let abs = BezierNet::of(surface)?;
        let (p, q) = abs.degrees;
        let oo = [o.x, o.y, o.z];
        let mut sizes = Vec::with_capacity(abs.patches.len());
        let mut base_err = Vec::with_capacity(abs.patches.len());
        for pts in &abs.patches {
            let (mut about, mut origin, mut weight) = (0.0_f64, 0.0_f64, 0.0_f64);
            for &[x, y, z, w] in pts {
                for (c, oc) in [x, y, z].into_iter().zip(oo) {
                    about = about.max(c.abs() + oc.abs() * w.abs());
                    origin = origin.max(c.abs());
                }
                weight = weight.max(w.abs());
            }
            sizes.push((about, origin, weight));
            // Extraction relative to the point, translation (two
            // operations) relative to `|X| + |o| |w|`.
            let g = gamma(count_net(p, q) + 2);
            base_err.push((g * about + TINY, g * weight + TINY));
        }
        let x = BezierNet {
            degrees: abs.degrees,
            ku: abs.ku.clone(),
            kv: abs.kv.clone(),
            patches: abs
                .patches
                .iter()
                .map(|pts| {
                    pts.iter()
                        .map(|&[x, y, z, w]| [x - o.x * w, y - o.y * w, z - o.z * w, w])
                        .collect()
                })
                .collect(),
        };
        let u = x.derivative(true)?;
        let v = x.derivative(false)?;
        let uu = u.derivative(true)?;
        let uv = u.derivative(false)?;
        let vv = v.derivative(false)?;
        let count = |knots: &[f64], k: f64| knots.iter().filter(|x| **x == k).count();
        let mult_u = x.ku.iter().map(|k| count(surface.knots_u(), *k)).collect();
        let mult_v = x.kv.iter().map(|k| count(surface.knots_v(), *k)).collect();
        Ok(Nets {
            x,
            u,
            v,
            uu,
            uv,
            vv,
            degrees: (p, q),
            sizes,
            base_err,
            mult_u,
            mult_v,
        })
    }
}

/// A Bézier patch (`(p + 1) × (q + 1)` homogeneous points, row-major
/// with `a` along u) cut to `[lu0, lu1] × [lv0, lv1]` of its own square.
fn cut_patch(pts: &[H], q: usize, lu: (f64, f64), lv: (f64, f64)) -> Vec<H> {
    let mut cut: Vec<H> = pts.to_vec();
    let stride = q + 1;
    for b in 0..stride {
        for c in 0..4 {
            let col: Vec<f64> = cut
                .iter()
                .skip(b)
                .step_by(stride)
                .map(|h| h.get(c).copied().unwrap_or(0.0))
                .collect();
            let new = sub(&col, lu.0, lu.1);
            for (h, x) in cut.iter_mut().skip(b).step_by(stride).zip(new) {
                if let Some(slot) = h.get_mut(c) {
                    *slot = x;
                }
            }
        }
    }
    for row in cut.chunks_mut(stride) {
        for c in 0..4 {
            let vals: Vec<f64> = row
                .iter()
                .map(|h| h.get(c).copied().unwrap_or(0.0))
                .collect();
            let new = sub(&vals, lv.0, lv.1);
            for (h, x) in row.iter_mut().zip(new) {
                if let Some(slot) = h.get_mut(c) {
                    *slot = x;
                }
            }
        }
    }
    cut
}

/// How much a de Casteljau cut to `[lo, hi]` can grow a coefficient's
/// error per level: 1 inside `[0, 1]`, `|t| + |1 − t|` at a parameter `t`
/// outside it (a continuation).
fn reach_factor(lo: f64, hi: f64) -> f64 {
    let f = |t: f64| t.abs() + (1.0 - t).abs();
    f(lo).max(f(hi)).max(1.0)
}

/// The hull of the net of derivative order `orders` (`net`) over the box
/// `[u0, u1] × [v0, v1]` of the surface's parameters, written about `o2`
/// (in the nets' frame): every Bézier patch the box meets, each continued
/// as its polynomial over the *whole* box (so the hull holds both the
/// surface and any patch's continuation past its knot lines inside the
/// box), each coordinate's hull of its coefficients, widened by the
/// rounding of the cuts and the translation, and by the error the net's
/// own points carry (`base_err` times `2 p / Δ` per order of derivative),
/// both grown by a continuation's reach. `None` where the box is not
/// finite.
fn enclose(
    nets: &Nets,
    net: &BezierNet,
    orders: (usize, usize),
    bx: [f64; 4],
    o2: Vec3,
) -> Option<Encl> {
    let [u0, u1, v0, v1] = bx;
    if !bx.iter().all(|x| x.is_finite()) || u1 < u0 || v1 < v0 {
        return None;
    }
    let (p, q) = net.degrees;
    let (p0, q0) = nets.degrees;
    let nu = net.ku.len().checked_sub(1)?;
    let nv = net.kv.len().checked_sub(1)?;
    if nu == 0 || nv == 0 {
        return None;
    }
    let (i0, i1) = (span_of(&net.ku, u0), span_of(&net.ku, u1));
    let (j0, j1) = (span_of(&net.kv, v0), span_of(&net.kv, v1));
    let [ox, oy, oz] = [o2.x, o2.y, o2.z];
    let mut lo = [f64::INFINITY; 4];
    let mut hi = [f64::NEG_INFINITY; 4];
    // Slack of the coordinates (lengths) and of the weight, apart.
    let mut slack = 0.0_f64;
    let mut slack_w = 0.0_f64;
    let mut any = false;
    for i in i0..=i1 {
        let (ka, kb) = (*net.ku.get(i)?, *net.ku.get(i + 1)?);
        let wu = kb - ka;
        if wu.is_nan() || wu <= 0.0 {
            return None;
        }
        let (lu0, lu1) = ((u0 - ka) / wu, (u1 - ka) / wu);
        for j in j0..=j1 {
            let (la, lb) = (*net.kv.get(j)?, *net.kv.get(j + 1)?);
            let wv = lb - la;
            if wv.is_nan() || wv <= 0.0 {
                return None;
            }
            let (lv0, lv1) = ((v0 - la) / wv, (v1 - la) / wv);
            let k = i * nv + j;
            let pts = net.patches.get(k)?;
            if pts.len() != (p + 1) * (q + 1) {
                return None;
            }
            let cut = cut_patch(pts, q, (lu0, lu1), (lv0, lv1));
            let (mut size, mut size_w) = (0.0_f64, 0.0_f64);
            for &[x, y, z, w] in pts {
                size = size
                    .max(x.abs() + ox.abs() * w.abs())
                    .max(y.abs() + oy.abs() * w.abs())
                    .max(z.abs() + oz.abs() * w.abs());
                size_w = size_w.max(w.abs());
            }
            for &[x, y, z, w] in &cut {
                let t = [x - ox * w, y - oy * w, z - oz * w, w];
                for ((l, h), v) in lo.iter_mut().zip(hi.iter_mut()).zip(t) {
                    *l = l.min(v);
                    *h = h.max(v);
                }
                size = size
                    .max(x.abs() + ox.abs() * w.abs())
                    .max(y.abs() + oy.abs() * w.abs())
                    .max(z.abs() + oz.abs() * w.abs());
                size_w = size_w.max(w.abs());
            }
            // The points' own error, through the derivative's differences.
            let mut factor = 1.0;
            for a in 0..orders.0 {
                factor *= 2.0 * p0.saturating_sub(a) as f64 / wu;
            }
            for b in 0..orders.1 {
                factor *= 2.0 * q0.saturating_sub(b) as f64 / wv;
            }
            let (carried, carried_w) = nets
                .base_err
                .get(k)
                .copied()
                .unwrap_or((f64::INFINITY, f64::INFINITY));
            // Two splits of `p` and `q` levels each way, and the
            // translation; a continuation grows them per level.
            let grow = power(reach_factor(lu0, lu1), 2 * p) * power(reach_factor(lv0, lv1), 2 * q);
            let g = gamma(count_cut(p + 1) + count_cut(q + 1) + 4);
            slack = slack.max(grow * (g * size + carried * factor));
            slack_w = slack_w.max(grow * (g * size_w + carried_w * factor));
            any = true;
        }
    }
    if !any || !slack.is_finite() || !slack_w.is_finite() {
        return None;
    }
    let slack = slack * (1.0 + 4.0 * EPS) + TINY;
    let slack_w = slack_w * (1.0 + 4.0 * EPS) + TINY;
    let [l0, l1, l2, l3] = lo;
    let [h0, h1, h2, h3] = hi;
    let iv = |l: f64, h: f64, pad: f64| Iv {
        lo: l - pad,
        hi: h + pad,
    };
    let e = Encl {
        x: [iv(l0, h0, slack), iv(l1, h1, slack), iv(l2, h2, slack)],
        w: iv(l3, h3, slack_w),
    };
    (e.x.iter().all(|x| x.finite()) && e.w.finite()).then_some(e)
}

/// A bound on how far the polynomial `g` (Bernstein coefficients on
/// `[0, 1]`) rises above zero, by branch and bound on its subdivisions:
/// a part whose coefficients are all at most zero is left, a part whose
/// largest coefficient is within a thousandth of its largest end value
/// (a value of `g`) is taken at that coefficient, others are halved to a
/// depth of 48 and at most 64 parts alive; the hull of what is left then
/// bounds the rest. Widened by the rounding of the splits.
fn rise(g: &[f64]) -> f64 {
    let scale = max_abs(g);
    if !scale.is_finite() {
        return f64::INFINITY;
    }
    let pad = gamma(count_cut(g.len()) * 49 + 4) * scale + TINY;
    if g.iter().all(|x| *x <= -pad) {
        return 0.0;
    }
    let mut best = 0.0_f64;
    let mut live: Vec<(Vec<f64>, u32)> = vec![(g.to_vec(), 0)];
    while let Some((c, depth)) = live.pop() {
        let top = c.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if top <= best {
            continue;
        }
        let ends = c
            .first()
            .copied()
            .unwrap_or(0.0)
            .max(c.last().copied().unwrap_or(0.0));
        best = best.max(ends);
        if top - ends <= 1e-3 * top.abs() || depth >= 48 || live.len() >= 64 {
            best = best.max(top);
            continue;
        }
        let (l, r) = split(&c, 0.5);
        live.push((r, depth + 1));
        live.push((l, depth + 1));
    }
    (best.max(0.0) + pad) * (1.0 + 4.0 * EPS)
}

// ---------------------------------------------------------------------
// One piece of the proof.

/// A piece of `t` with everything its proof needs, each polynomial in the
/// piece's own parameter `s ∈ [0, 1]`.
struct Span {
    /// The piece's ends in the curve's parameter (for reports).
    t: (f64, f64),
    depth: u32,
    /// The residuals `F₀(s)`, `F₁(s)` and bounds on their rounding.
    f: [Vec<f64>; 2],
    round: [f64; 2],
    /// `c`'s coordinates, and bounds on their coefficients' rounding.
    uv: [Vec<f64>; 2],
    uv_round: [f64; 2],
    /// The curve's homogeneous coordinates, about the curve's first
    /// point, and bounds on their coefficients' rounding.
    gamma: [Vec<f64>; 4],
    gamma_round: [f64; 4],
    /// How far `c` reaches past the knot lines of its patch inside the
    /// domain, in u and in v (where its residual is the patch's
    /// continuation, not the surface).
    past: [f64; 2],
    /// For the stored edge's rounding: the relative rounding, the largest
    /// `|X|` about the origin and the largest `|w|` of the composed terms.
    edge: (f64, f64, f64),
}

/// What [`projection_bound`] proved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProvenGap {
    /// The largest distance, over the whole curve, between the edge
    /// `S(c(t))` and the projection of `γ(t)` (the root of the
    /// projection's condition within the proven box about `c(t)`).
    pub bound: f64,
    /// Pieces the proof took (halvings included).
    pub pieces: usize,
}

/// What a proof came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Proof {
    /// Proven, with this bound.
    Proven(ProvenGap),
    /// Not proven, and why (with the place): more samples may help.
    NotProven(String),
    /// Not provable this way whatever the samples (the curve's projection
    /// crosses a crease of the face), and why.
    Unprovable(String),
}

/// The rows `Q` square to the direction `d`: unit, within a few `ε` of an
/// exact orthonormal pair square to `d` (the bound accounts for the
/// difference, [`ROWS_OFF`]).
fn rows_square_to(d: Vec3) -> Option<[Vec3; 2]> {
    let a = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    let pick = a
        .iter()
        .copied()
        .min_by(|x, y| x.dot(d).abs().total_cmp(&y.dot(d).abs()))?;
    let q0 = d.cross(pick).normalized()?;
    let q1 = d.cross(q0).normalized()?;
    Some([q0, q1])
}

/// How far each computed row of `Q` may be from an exact unit row square
/// to the (unit) direction, in units of `ε`: a cross product (two
/// products and a difference per coordinate) and a normalisation (three
/// products, two sums, a root and a quotient) for `q₀`, the same again
/// for `q₁` from `q₀`, and `d`'s own distance from unit length (taken
/// within 4 ε by the caller's normalisation): `2 (3 + 7) + 4`, doubled.
const ROWS_OFF: f64 = 48.0;

/// A proven bound on the distance between the edge `S(c(t))` of `uv` on
/// `surface` and the projection of `curve` by `how`, at every `t` of the
/// curve's domain (`uv` is over the same parameter). `smooth_lines` says
/// the surface is `C¹` across every knot line inside its domain whatever
/// the knots' multiplicity (an analytic face's rational form, whose
/// quarter turns join `C¹` by construction); otherwise a line of
/// multiplicity at least the degree is a crease, which closest points are
/// not proven across. `budget` is the number of pieces the caller allows
/// all its proofs together; spending it is an error (refused), while a
/// piece that cannot be proven within [`MAX_PROOF_PIECES`] for this curve
/// is [`Proof::NotProven`] with its place.
pub fn projection_bound(
    surface: &NurbsSurface,
    curve: &NurbsCurve,
    uv: &NurbsCurve,
    how: Projection,
    smooth_lines: bool,
    budget: &mut usize,
) -> Result<Proof, TrimError> {
    let (ta, _) = curve.domain();
    let o = curve.eval(ta);
    if !(o.x.is_finite() && o.y.is_finite() && o.z.is_finite()) {
        return refuse("the projected curve has no finite point".into());
    }
    let nets = Nets::of(surface, o)?;
    let (p, q) = nets.degrees;
    let rows = match how {
        Projection::Normal => None,
        Projection::Along(d) => Some(
            rows_square_to(d)
                .ok_or_else(|| TrimError::Refused("the direction has no length".into()))?,
        ),
    };
    // The edge and the derivatives along it, piece by piece (one set of
    // bounds: they depend only on `uv` and the knot lines).
    let (_, bounds, px) = compose_net_pieces(&nets.x, uv, Composed::Any)?;
    let (_, bu, pu) = compose_net_pieces(&nets.u, uv, Composed::Any)?;
    let (_, bv, pv) = compose_net_pieces(&nets.v, uv, Composed::Any)?;
    if bu != bounds || bv != bounds || pu.len() != px.len() || pv.len() != px.len() {
        return refuse("the edge's pieces and its derivatives' differ; not proven".into());
    }
    let d = uv.degree();
    let g_degree = curve.degree();
    let (ub, upieces) = bezier_pieces_raw(Raw::of(uv))?;
    let (gb, gpieces) = bezier_pieces_raw(Raw::of(curve))?;
    let o_size = o.x.abs().max(o.y.abs()).max(o.z.abs());
    let gpieces: Vec<Vec<H>> = gpieces
        .into_iter()
        .map(|piece| {
            piece
                .into_iter()
                .map(|[x, y, z, w]| [x - o.x * w, y - o.y * w, z - o.z * w, w])
                .collect()
        })
        .collect();
    // The counts of the chains (see *Rounding counts* above).
    let dd = d * (p + q);
    let du = d * (p.saturating_sub(1) + q);
    let err_x = count_net(p, q) + 2 + count_compose(p, q, d) + count_cut(dd + 1);
    let err_xu = count_net(p, q) + 4 + count_compose(p.max(1), q.max(1), d) + count_cut(dd + 1);
    let err_g = count_net(g_degree, 0) + 2 + count_cut(g_degree + 1);
    let err_uv = count_net(d, 0) + count_cut(d + 1);
    let k_normal = 2 * err_x
        + err_xu
        + err_g
        + count_product(dd + g_degree)
        + count_product(dd + du)
        + count_product(2 * dd + du + g_degree)
        + 4;
    let k_along = err_x + err_g + count_product(dd + g_degree) + 4;
    // The stored edge: the composition about the origin, its pieces
    // subdivided (at most 12 deep) while weights are not positive, each
    // point divided by its weight.
    let k_edge = count_net(p, q) + count_compose(p, q, d) + 12 * count_cut(d + 1) + 2;
    let mut binom = Binomials::new();
    let mut todo: Vec<Span> = Vec::new();
    let nu = nets.x.ku.len().saturating_sub(1);
    let nv = nets.x.kv.len().saturating_sub(1);
    for (k, x_piece) in px.iter().enumerate() {
        let (Some(&t0), Some(&t1), Some(u_piece), Some(v_piece)) =
            (bounds.get(k), bounds.get(k + 1), pu.get(k), pv.get(k))
        else {
            return refuse("the edge lost a piece; not proven".into());
        };
        if !(t1 > t0) {
            continue;
        }
        // Cut further at the curve's own knots.
        let mut cuts = vec![t0];
        cuts.extend(gb.iter().copied().filter(|g| *g > t0 && *g < t1));
        cuts.push(t1);
        for w in cuts.windows(2) {
            let [sa_t, sb_t] = [
                w.first().copied().unwrap_or(t0),
                w.get(1).copied().unwrap_or(t1),
            ];
            if !(sb_t > sa_t) {
                continue;
            }
            let (sa, sb) = ((sa_t - t0) / (t1 - t0), (sb_t - t0) / (t1 - t0));
            let mid = 0.5 * (sa_t + sb_t);
            let cut = |piece: &[H], c: usize| sub(&coord(piece, c), sa, sb);
            let x: [Vec<f64>; 4] = [0, 1, 2, 3].map(|c| cut(x_piece, c));
            let xu: [Vec<f64>; 4] = [0, 1, 2, 3].map(|c| cut(u_piece, c));
            let xv: [Vec<f64>; 4] = [0, 1, 2, 3].map(|c| cut(v_piece, c));
            // The curve's piece and `c`'s.
            let gi = span_of(&gb, mid);
            let (Some(&g0), Some(&g1), Some(gp)) = (gb.get(gi), gb.get(gi + 1), gpieces.get(gi))
            else {
                return refuse("the curve lost a piece; not proven".into());
            };
            let (la, lb) = ((sa_t - g0) / (g1 - g0), (sb_t - g0) / (g1 - g0));
            let gam: [Vec<f64>; 4] = [0, 1, 2, 3].map(|c| sub(&coord(gp, c), la, lb));
            let gam_size = max_abs(&gam[3]) * o_size;
            let gamma_round: [f64; 4] = [0, 1, 2, 3].map(|c| {
                let own = gam.get(c).map_or(0.0, |v| max_abs(v));
                gamma(err_g) * (own + if c < 3 { gam_size } else { 0.0 })
            });
            let ui = span_of(&ub, mid);
            let (Some(&c0), Some(&c1), Some(cp)) = (ub.get(ui), ub.get(ui + 1), upieces.get(ui))
            else {
                return refuse("the parameter curve lost a piece; not proven".into());
            };
            let (ca, cb) = ((sa_t - c0) / (c1 - c0), (sb_t - c0) / (c1 - c0));
            let uvc: [Vec<f64>; 2] = [sub(&coord(cp, 0), ca, cb), sub(&coord(cp, 1), ca, cb)];
            let uv_round = [
                gamma(err_uv) * max_abs(&uvc[0]),
                gamma(err_uv) * max_abs(&uvc[1]),
            ];
            // The patch it lies in, and how far its local coefficients
            // reach outside it (the composition's terms grow by that).
            let um = bern_eval(&uvc[0], 0.5);
            let vm = bern_eval(&uvc[1], 0.5);
            let (i, j) = (span_of(&nets.x.ku, um), span_of(&nets.x.kv, vm));
            let (Some(&(m_about, m_origin, m_w)), Some(&ka), Some(&kb), Some(&la2), Some(&lb2)) = (
                nets.sizes.get(i * nv + j),
                nets.x.ku.get(i),
                nets.x.ku.get(i + 1),
                nets.x.kv.get(j),
                nets.x.kv.get(j + 1),
            ) else {
                return refuse("the edge left the surface's patches; not proven".into());
            };
            // How far `c` goes past the patch's knot lines inside the
            // domain (crossings within a rounding of a piece's end are
            // merged by the composition, and a curve along a line within
            // its floor is not cut): the residual there is the patch's
            // continuation.
            let past_line = |coef: &[f64], line: f64, above: bool, pad: f64| -> f64 {
                let g: Vec<f64> = coef
                    .iter()
                    .map(|x| if above { x - line } else { line - x })
                    .collect();
                rise(&g) + pad
            };
            let mut past = [0.0_f64; 2];
            if i > 0 {
                past[0] = past[0].max(past_line(&uvc[0], ka, false, uv_round[0]));
            }
            if i + 1 < nu {
                past[0] = past[0].max(past_line(&uvc[0], kb, true, uv_round[0]));
            }
            if j > 0 {
                past[1] = past[1].max(past_line(&uvc[1], la2, false, uv_round[1]));
            }
            if j + 1 < nv {
                past[1] = past[1].max(past_line(&uvc[1], lb2, true, uv_round[1]));
            }
            let reach = |c: &[f64], a: f64, b: f64| {
                c.iter().fold(1.0_f64, |m, x| {
                    let l = (x - a) / (b - a);
                    m.max(l.abs() + (1.0 - l).abs())
                })
            };
            let (au, av) = (reach(&uvc[0], ka, kb), reach(&uvc[1], la2, lb2));
            let alpha = |pp: usize, qq: usize| power(au, pp) * power(av, qq);
            let m_x = alpha(p, q) * m_about;
            let m_wt = alpha(p, q) * m_w;
            let fu = if p > 0 {
                2.0 * p as f64 / (kb - ka)
            } else {
                0.0
            };
            let fv = if q > 0 {
                2.0 * q as f64 / (lb2 - la2)
            } else {
                0.0
            };
            let a_u = alpha(p.saturating_sub(1), q);
            let a_v = alpha(p, q.saturating_sub(1));
            let (m_xu, m_wu) = (a_u * fu * m_about, a_u * fu * m_w);
            let (m_xv, m_wv) = (a_v * fv * m_about, a_v * fv * m_w);
            let [g0c, g1c, g2c, g3c] = &gam;
            let [x0, x1, x2, x3] = &x;
            let m_om = max_abs(g3c);
            let m_g = max_abs(g0c).max(max_abs(g1c)).max(max_abs(g2c)) + o_size * m_om;
            // A = ω X − w Γ, coordinate by coordinate.
            let mut big_a: Vec<Vec<f64>> = Vec::with_capacity(3);
            for (xc, gc) in [(x0, g0c), (x1, g1c), (x2, g2c)] {
                let l = binom.product(g3c, xc);
                let r = binom.product(x3, gc);
                big_a.push(poly_sub(&l, &r, &mut binom));
            }
            let m_a = m_om * m_x + m_wt * m_g;
            let (f, round) = match rows {
                None => {
                    // B_u = w X_u − w_u X, B_v the same in v; F = A·B.
                    let mut fs: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
                    for (slot, d) in fs.iter_mut().zip([&xu, &xv]) {
                        let mut acc: Vec<f64> = Vec::new();
                        let [d0, d1, d2, d3] = d;
                        for ((xc, dc), ac) in [x0, x1, x2].into_iter().zip([d0, d1, d2]).zip(&big_a)
                        {
                            let b = poly_sub(
                                &binom.product(x3, dc),
                                &binom.product(d3, xc),
                                &mut binom,
                            );
                            let term = binom.product(ac, &b);
                            acc = if acc.is_empty() {
                                term
                            } else {
                                poly_add(&acc, &term, &mut binom)
                            };
                        }
                        *slot = acc;
                    }
                    let rb_u = m_wt * m_xu + m_wu * m_x;
                    let rb_v = m_wt * m_xv + m_wv * m_x;
                    let g = gamma(k_normal);
                    (fs, [g * 3.0 * m_a * rb_u, g * 3.0 * m_a * rb_v])
                }
                Some(qr) => {
                    let mut fs: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
                    let mut rounds = [0.0; 2];
                    for ((slot, r), row) in fs.iter_mut().zip(rounds.iter_mut()).zip(qr) {
                        let qc = [row.x, row.y, row.z];
                        let mut acc: Vec<f64> = Vec::new();
                        for (a, qk) in big_a.iter().zip(qc) {
                            let term: Vec<f64> = a.iter().map(|y| y * qk).collect();
                            acc = if acc.is_empty() {
                                term
                            } else {
                                poly_add(&acc, &term, &mut binom)
                            };
                        }
                        *slot = acc;
                        *r = gamma(k_along) * (row.x.abs() + row.y.abs() + row.z.abs()) * m_a;
                    }
                    (fs, rounds)
                }
            };
            todo.push(Span {
                t: (sa_t, sb_t),
                depth: 0,
                f,
                round,
                uv: uvc,
                uv_round,
                gamma: gam,
                gamma_round,
                past,
                edge: (gamma(k_edge), alpha(p, q) * m_origin, alpha(p, q) * m_w),
            });
        }
    }
    // Pieces in order: the first is proven first, a halved piece's first
    // half before its second.
    todo.reverse();
    let mut worst = 0.0_f64;
    let mut done = 0usize;
    while let Some(span) = todo.pop() {
        done += 1;
        if *budget == 0 {
            return refuse(format!(
                "the proofs of this trim spent their work allowance (near t = {} of a curve)",
                span.t.0
            ));
        }
        *budget -= 1;
        if done > MAX_PROOF_PIECES {
            return Ok(Proof::NotProven(format!(
                "not proven within {MAX_PROOF_PIECES} pieces (near t = {})",
                span.t.0
            )));
        }
        match prove_span(&nets, &span, rows, smooth_lines) {
            Verdict::Bound(d) => worst = worst.max(d),
            Verdict::Refuse(why) => return Ok(Proof::Unprovable(why)),
            Verdict::Halve => {
                if span.depth >= MAX_DEPTH {
                    return Ok(Proof::NotProven(format!(
                        "not proven between t = {} and {} (the projection's condition does not contract there: a grazing direction or a focal point of the surface)",
                        span.t.0, span.t.1
                    )));
                }
                let (left, right) = halves(&span);
                todo.push(right);
                todo.push(left);
            }
        }
    }
    if !worst.is_finite() {
        return Ok(Proof::NotProven("the bound is not finite".into()));
    }
    Ok(Proof::Proven(ProvenGap {
        bound: worst,
        pieces: done,
    }))
}

/// The two halves of a piece, each polynomial cut by de Casteljau, whose
/// rounding (one split of `n` levels, three operations each, relative to
/// the largest coefficient) is added to each polynomial's allowance.
fn halves(span: &Span) -> (Span, Span) {
    let half = |c: &[f64], first: bool| {
        let (l, r) = split(c, 0.5);
        if first { l } else { r }
    };
    let grown = |c: &[f64], round: f64| round + gamma(3 * c.len() + 2) * (max_abs(c) + round);
    let make = |first: bool| {
        let m = 0.5 * span.t.0 + 0.5 * span.t.1;
        Span {
            t: if first { (span.t.0, m) } else { (m, span.t.1) },
            depth: span.depth + 1,
            f: [half(&span.f[0], first), half(&span.f[1], first)],
            round: [
                grown(&span.f[0], span.round[0]),
                grown(&span.f[1], span.round[1]),
            ],
            uv: [half(&span.uv[0], first), half(&span.uv[1], first)],
            uv_round: [
                grown(&span.uv[0], span.uv_round[0]),
                grown(&span.uv[1], span.uv_round[1]),
            ],
            gamma: [0, 1, 2, 3]
                .map(|c| span.gamma.get(c).map_or_else(Vec::new, |g| half(g, first))),
            gamma_round: [0, 1, 2, 3].map(|c| match (span.gamma.get(c), span.gamma_round.get(c)) {
                (Some(g), Some(r)) => grown(g, *r),
                _ => f64::INFINITY,
            }),
            past: span.past,
            edge: span.edge,
        }
    };
    (make(true), make(false))
}

/// The hull of a polynomial's coefficients, widened by `extra` (the
/// coefficients' own error).
fn hull(c: &[f64], extra: f64) -> Iv {
    if c.iter().any(|x| !x.is_finite()) || c.is_empty() || !extra.is_finite() {
        return WHOLE;
    }
    let lo = c.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = c.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let pad = (extra + TINY) * (1.0 + 2.0 * EPS);
    Iv::widened(lo - pad, hi + pad)
}

/// The Jacobian of the condition over a box, and what the distance needs
/// there (`|B_u|`, `|B_v|` upper bounds, `w`'s lower bound, `|A|`'s and
/// its derivatives').
struct AtBox {
    j: [[Iv; 2]; 2],
    bu: f64,
    bv: f64,
    w_lo: f64,
    a: f64,
    a_u: f64,
    a_v: f64,
    om_lo: f64,
}

/// Why a box gave no answer.
enum Fail {
    /// The piece may yet be proven in halves.
    Halve,
    /// Not provable here at all, and why.
    Refuse(String),
}

/// What a piece's proof came to.
enum Verdict {
    Bound(f64),
    Halve,
    Refuse(String),
}

/// The knot lines strictly inside `(lo, hi)` of `knots` (a net's `ku`
/// or `kv`) with their multiplicities, the domain's ends left out.
fn lines_inside<'a>(
    knots: &'a [f64],
    mult: &'a [usize],
    lo: f64,
    hi: f64,
) -> impl Iterator<Item = (f64, usize)> + 'a {
    let n = knots.len();
    knots
        .iter()
        .zip(mult)
        .enumerate()
        .filter(move |(i, (k, _))| *i > 0 && i + 1 < n && **k > lo && **k < hi)
        .map(|(_, (k, m))| (*k, *m))
}

fn at_box(
    nets: &Nets,
    span: &Span,
    o2: Vec3,
    r: f64,
    rows: Option<[Vec3; 2]>,
    smooth_lines: bool,
) -> Result<AtBox, Fail> {
    let uh = hull(&span.uv[0], span.uv_round[0]);
    let vh = hull(&span.uv[1], span.uv_round[1]);
    let bx = [uh.lo - r, uh.hi + r, vh.lo - r, vh.hi + r];
    // The condition must be continuous across the box: the surface's
    // position (both ways of projecting) and, for closest points, its
    // first derivatives.
    let (p0, q0) = nets.degrees;
    let need = if rows.is_some() || smooth_lines { 0 } else { 1 };
    for (k, m, p, along_u) in lines_inside(&nets.x.ku, &nets.mult_u, bx[0], bx[1])
        .map(|(k, m)| (k, m, p0, true))
        .chain(lines_inside(&nets.x.kv, &nets.mult_v, bx[2], bx[3]).map(|(k, m)| (k, m, q0, false)))
    {
        let continuity = p as i64 - m as i64;
        let required = if smooth_lines && m <= p { 0 } else { need };
        if continuity < required {
            return Err(Fail::Refuse(format!(
                "the curve's projection crosses the knot line {} = {k} of the face, where it is only C{} ({}): not proven there{}",
                if along_u { "u" } else { "v" },
                continuity.max(0),
                if continuity < 0 { "a gap" } else { "a crease" },
                if rows.is_none() {
                    " (project it along a direction instead)"
                } else {
                    ""
                }
            )));
        }
    }
    let x = enclose(nets, &nets.x, (0, 0), bx, o2).ok_or(Fail::Halve)?;
    let xu = enclose(nets, &nets.u, (1, 0), bx, o2).ok_or(Fail::Halve)?;
    let xv = enclose(nets, &nets.v, (0, 1), bx, o2).ok_or(Fail::Halve)?;
    // The curve about o2, over the piece.
    let [g0c, g1c, g2c, g3c] = &span.gamma;
    let [r0, r1, r2, r3] = span.gamma_round;
    let om = hull(g3c, r3);
    let about = |gc: &[f64], rc: f64, oc: f64| -> Iv {
        let coefs: Vec<f64> = gc.iter().zip(g3c).map(|(a, w)| a - oc * w).collect();
        let size = gc
            .iter()
            .zip(g3c)
            .fold(0.0_f64, |m, (a, w)| m.max(a.abs() + (oc * w).abs()));
        hull(&coefs, 2.0 * EPS * size + rc + oc.abs() * r3)
    };
    let g: V3 = [
        about(g0c, r0, o2.x),
        about(g1c, r1, o2.y),
        about(g2c, r2, o2.z),
    ];
    let a = v_sub(v_scale(om, x.x), v_scale(x.w, g));
    let a_u = v_sub(v_scale(om, xu.x), v_scale(xu.w, g));
    let a_v = v_sub(v_scale(om, xv.x), v_scale(xv.w, g));
    let b_u = v_sub(v_scale(x.w, xu.x), v_scale(xu.w, x.x));
    let b_v = v_sub(v_scale(x.w, xv.x), v_scale(xv.w, x.x));
    let j = match rows {
        None => {
            let xuu = enclose(nets, &nets.uu, (2, 0), bx, o2).ok_or(Fail::Halve)?;
            let xuv = enclose(nets, &nets.uv, (1, 1), bx, o2).ok_or(Fail::Halve)?;
            let xvv = enclose(nets, &nets.vv, (0, 2), bx, o2).ok_or(Fail::Halve)?;
            let b_uu = v_sub(v_scale(x.w, xuu.x), v_scale(xuu.w, x.x));
            let b_vv = v_sub(v_scale(x.w, xvv.x), v_scale(xvv.w, x.x));
            // ∂B_u/∂v and ∂B_v/∂u.
            let b_uv = v_sub(
                v_add(v_scale(xv.w, xu.x), v_scale(x.w, xuv.x)),
                v_add(v_scale(xuv.w, x.x), v_scale(xu.w, xv.x)),
            );
            let b_vu = v_sub(
                v_add(v_scale(xu.w, xv.x), v_scale(x.w, xuv.x)),
                v_add(v_scale(xuv.w, x.x), v_scale(xv.w, xu.x)),
            );
            [
                [
                    v_dot(a_u, b_u).add(v_dot(a, b_uu)),
                    v_dot(a_v, b_u).add(v_dot(a, b_uv)),
                ],
                [
                    v_dot(a_u, b_v).add(v_dot(a, b_vu)),
                    v_dot(a_v, b_v).add(v_dot(a, b_vv)),
                ],
            ]
        }
        Some([q0, q1]) => {
            let qv = |q: Vec3| [Iv::point(q.x), Iv::point(q.y), Iv::point(q.z)];
            [
                [v_dot(qv(q0), a_u), v_dot(qv(q0), a_v)],
                [v_dot(qv(q1), a_u), v_dot(qv(q1), a_v)],
            ]
        }
    };
    Ok(AtBox {
        j,
        bu: v_norm(b_u),
        bv: v_norm(b_v),
        w_lo: x.w.lo,
        a: v_norm(a),
        a_u: v_norm(a_u),
        a_v: v_norm(a_v),
        om_lo: om.lo,
    })
}

/// The bound on one piece, or why there is none.
fn prove_span(nets: &Nets, span: &Span, rows: Option<[Vec3; 2]>, smooth_lines: bool) -> Verdict {
    let fail = |f: Fail| match f {
        Fail::Halve => Verdict::Halve,
        Fail::Refuse(why) => Verdict::Refuse(why),
    };
    // The point the hulls are written about: the curve's middle (any
    // point serves; this one keeps the products small).
    let wm = bern_eval(&span.gamma[3], 0.5);
    if !(wm.is_finite() && wm > 0.0) {
        return Verdict::Halve;
    }
    let o2 = Vec3::new(
        bern_eval(&span.gamma[0], 0.5) / wm,
        bern_eval(&span.gamma[1], 0.5) / wm,
        bern_eval(&span.gamma[2], 0.5) / wm,
    );
    let first = match at_box(nets, span, o2, 0.0, rows, smooth_lines) {
        Ok(a) => a,
        Err(f) => return fail(f),
    };
    let m = first.j.map(|row| row.map(Iv::mid));
    let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    let size = (m[0][0] * m[1][1]).abs() + (m[0][1] * m[1][0]).abs();
    if !(det.is_finite() && det.abs() > 1e-12 * size) {
        return Verdict::Halve;
    }
    let y = [
        [m[1][1] / det, -m[0][1] / det],
        [-m[1][0] / det, m[0][0] / det],
    ];
    if span.f[0].len() != span.f[1].len() {
        return Verdict::Halve;
    }
    // `|(Y F)_k|` over the piece, per row: the hull of the combined
    // coefficients, their rounding, and the residuals' own allowance.
    let mut rho = [0.0_f64; 2];
    for (slot, row) in rho.iter_mut().zip(y) {
        let mut lead = 0.0_f64;
        let mut size = 0.0_f64;
        for (fa, fb) in span.f[0].iter().zip(&span.f[1]) {
            let (a, b) = (fa * row[0], fb * row[1]);
            lead = lead.max((a + b).abs());
            size = size.max(a.abs() + b.abs());
        }
        let bound =
            lead + 3.0 * EPS * size + row[0].abs() * span.round[0] + row[1].abs() * span.round[1];
        *slot = bound * (1.0 + 4.0 * EPS);
    }
    if !(rho[0].is_finite() && rho[1].is_finite()) {
        return Verdict::Halve;
    }
    // The box's radius: a little over the residual first (the contraction
    // is usually far below one), larger while the condition fails.
    let mut r = (rho[0].max(rho[1]) * (1.0 + 1e-3)).max(TINY * 1e10);
    for _ in 0..6 {
        let at = match at_box(nets, span, o2, r, rows, smooth_lines) {
            Ok(a) => a,
            Err(f) => return fail(f),
        };
        let [j0, j1] = at.j;
        // What the residual misses, per row of Y:
        // - where `c` is past a knot line of its patch, the true surface's
        //   condition differs from the continuation's by at most the
        //   overshoot times both sides' derivatives (each in the box's
        //   enclosure, which holds the continuation too);
        // - along a direction, the true rows differ from `Q` by
        //   `ROWS_OFF ε` each, which moves `F` by that much of `|A|` and
        //   `J` by that much of `|A_u|`, `|A_v|`.
        let [past_u, past_v] = span.past;
        let off = ROWS_OFF * EPS;
        let mut extra = [0.0_f64; 2];
        let mut widen = [0.0_f64; 2];
        for ((e, wd), yr) in extra.iter_mut().zip(widen.iter_mut()).zip(y) {
            for (yl, (ju, jv)) in yr.iter().zip([(j0[0], j0[1]), (j1[0], j1[1])]) {
                *e += yl.abs() * 2.0 * (past_u * ju.mag() + past_v * jv.mag());
            }
            if rows.is_some() {
                let ysum = yr[0].abs() + yr[1].abs();
                *e += ysum * off * at.a;
                *wd += ysum * off * (at.a_u + at.a_v);
            }
        }
        let rho_k = [
            (rho[0] + extra[0]) * (1.0 + 4.0 * EPS),
            (rho[1] + extra[1]) * (1.0 + 4.0 * EPS),
        ];
        // The rows of |I − Y J| over the box (widened as above); κ their
        // largest sum.
        let mut sums = [0.0_f64; 2];
        for (k, ((yr, sum), wd)) in y.iter().zip(sums.iter_mut()).zip(widen).enumerate() {
            for (l, (a, b)) in j0.into_iter().zip(j1).enumerate() {
                let yj = Iv::point(yr[0]).mul(a).add(Iv::point(yr[1]).mul(b));
                let id = Iv::point(if k == l { 1.0 } else { 0.0 });
                *sum += id.sub(yj).mag();
            }
            *sum = (*sum + wd) * (1.0 + 2.0 * EPS);
        }
        let kappa = sums[0].max(sums[1]);
        if !kappa.is_finite() || !rho_k.iter().all(|x| x.is_finite()) {
            return Verdict::Halve;
        }
        // Krawczyk: K(B) − c lies within ρ_k + Σ_l |M_kl| r in each
        // coordinate; strictly inside the box proves one root there, and
        // those are its bounds coordinate by coordinate.
        let reach = [
            (rho_k[0] + sums[0] * r) * (1.0 + 4.0 * EPS),
            (rho_k[1] + sums[1] * r) * (1.0 + 4.0 * EPS),
        ];
        if kappa < 1.0 && reach[0] < r && reach[1] < r {
            if !(at.w_lo > 0.0 && at.om_lo > 0.0) {
                return Verdict::Halve;
            }
            let w2 = at.w_lo * at.w_lo * (1.0 - 4.0 * EPS);
            // |S(c + δ) − S(c)| ≤ |δ_u| max |S_u| + |δ_v| max |S_v|; and
            // where `c` is past a knot line, the stored edge (on the
            // continuation) is that far from the surface at `c` times
            // both sides' derivatives.
            let dist = ((reach[0] + 2.0 * past_u) * at.bu + (reach[1] + 2.0 * past_v) * at.bv) / w2;
            // The stored edge was composed about the origin, rounded.
            let (g, mx, mw) = span.edge;
            let edge = g * (mx / at.w_lo) * (1.0 + mw / at.w_lo);
            let total = (dist + edge) * (1.0 + 8.0 * EPS);
            return if total.is_finite() {
                Verdict::Bound(total)
            } else {
                Verdict::Halve
            };
        }
        if kappa >= 0.9 {
            return Verdict::Halve;
        }
        r = (1.1 * rho_k[0].max(rho_k[1]) / (1.0 - kappa)).max(r * 1.5);
    }
    Verdict::Halve
}
