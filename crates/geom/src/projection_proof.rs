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
//! polynomial whose Bernstein coefficients are computed exactly (products
//! of the composed pieces, as `trim::compose` makes the edge), so its
//! bound is the hull of its coefficients plus their rounding; `J` is
//! enclosed by interval arithmetic on the hulls of the surface's Bézier
//! net and its derivative nets cut to the box (the convex hull property).
//! The edge's distance from the root is then at most
//! `r (max |S_u| + max |S_v|)` over the box, `|S_u| = |B_u| / w²`.
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
    let s = a.iter().fold(0.0_f64, |m, x| m + x.mag() * x.mag());
    // Three products and two sums, then a correctly rounded root.
    s.sqrt() * (1.0 + 4.0 * EPS)
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
    ke / (1.0 - ke)
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
/// first and second derivatives, and per patch the sizes the rounding is
/// taken of.
struct Nets {
    x: BezierNet,
    u: BezierNet,
    v: BezierNet,
    uu: BezierNet,
    uv: BezierNet,
    vv: BezierNet,
    /// Per patch: the largest `|X| + |o| |w|` (a coordinate written
    /// about `o`, with its translation's terms), the largest `|X|` (about
    /// the origin, as the stored edge is composed) and the largest `|w|`.
    sizes: Vec<(f64, f64, f64)>,
}

impl Nets {
    fn of(surface: &NurbsSurface, o: Vec3) -> Result<Nets, TrimError> {
        let abs = BezierNet::of(surface)?;
        let oo = [o.x, o.y, o.z];
        let mut sizes = Vec::with_capacity(abs.patches.len());
        for pts in &abs.patches {
            let (mut about, mut origin, mut weight) = (0.0_f64, 0.0_f64, 0.0_f64);
            for h in pts {
                for (c, oc) in h.iter().take(3).zip(oo) {
                    about = about.max(c.abs() + oc.abs() * h[3].abs());
                    origin = origin.max(c.abs());
                }
                weight = weight.max(h[3].abs());
            }
            sizes.push((about, origin, weight));
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
                        .map(|h| {
                            [
                                h[0] - o.x * h[3],
                                h[1] - o.y * h[3],
                                h[2] - o.z * h[3],
                                h[3],
                            ]
                        })
                        .collect()
                })
                .collect(),
        };
        let u = x.derivative(true)?;
        let v = x.derivative(false)?;
        let uu = u.derivative(true)?;
        let uv = u.derivative(false)?;
        let vv = v.derivative(false)?;
        Ok(Nets {
            x,
            u,
            v,
            uu,
            uv,
            vv,
            sizes,
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

/// The hull of `net`'s values over the box `[u0, u1] × [v0, v1]` of the
/// surface's parameters, written about `o2` (in the net's own frame):
/// every Bézier patch the box meets, cut to it (the end patches continued
/// past the domain where the box reaches beyond it), each coordinate's
/// hull of its coefficients, widened by the rounding of the cuts and the
/// translation. `None` where the box is not finite.
fn enclose(net: &BezierNet, bx: [f64; 4], o2: Vec3) -> Option<Encl> {
    let [u0, u1, v0, v1] = bx;
    if !bx.iter().all(|x| x.is_finite()) || u1 < u0 || v1 < v0 {
        return None;
    }
    let (p, q) = net.degrees;
    let nu = net.ku.len().checked_sub(1)?;
    let nv = net.kv.len().checked_sub(1)?;
    if nu == 0 || nv == 0 {
        return None;
    }
    let (i0, i1) = (span_of(&net.ku, u0), span_of(&net.ku, u1));
    let (j0, j1) = (span_of(&net.kv, v0), span_of(&net.kv, v1));
    let oo = [o2.x, o2.y, o2.z];
    let mut lo = [f64::INFINITY; 4];
    let mut hi = [f64::NEG_INFINITY; 4];
    let mut size = 0.0_f64;
    let mut any = false;
    for i in i0..=i1 {
        let (ka, kb) = (*net.ku.get(i)?, *net.ku.get(i + 1)?);
        let wu = kb - ka;
        if wu.is_nan() || wu <= 0.0 {
            return None;
        }
        let mut lu0 = (u0 - ka) / wu;
        let mut lu1 = (u1 - ka) / wu;
        if i > 0 {
            lu0 = lu0.max(0.0);
        }
        if i + 1 < nu {
            lu1 = lu1.min(1.0);
        }
        if lu1 < lu0 {
            continue;
        }
        for j in j0..=j1 {
            let (la, lb) = (*net.kv.get(j)?, *net.kv.get(j + 1)?);
            let wv = lb - la;
            if wv.is_nan() || wv <= 0.0 {
                return None;
            }
            let mut lv0 = (v0 - la) / wv;
            let mut lv1 = (v1 - la) / wv;
            if j > 0 {
                lv0 = lv0.max(0.0);
            }
            if j + 1 < nv {
                lv1 = lv1.min(1.0);
            }
            if lv1 < lv0 {
                continue;
            }
            let pts = net.patches.get(i * nv + j)?;
            if pts.len() != (p + 1) * (q + 1) {
                return None;
            }
            let cut = cut_patch(pts, q, (lu0, lu1), (lv0, lv1));
            let [ox, oy, oz] = oo;
            for &[x, y, z, w] in pts {
                size = size
                    .max(x.abs() + ox.abs() * w.abs())
                    .max(y.abs() + oy.abs() * w.abs())
                    .max(z.abs() + oz.abs() * w.abs())
                    .max(w.abs());
            }
            for &[x, y, z, w] in &cut {
                let t = [x - ox * w, y - oy * w, z - oz * w, w];
                for ((l, h), v) in lo.iter_mut().zip(hi.iter_mut()).zip(t) {
                    *l = l.min(v);
                    *h = h.max(v);
                    size = size.max(v.abs());
                }
            }
            any = true;
        }
    }
    if !any {
        return None;
    }
    // Each cut is `p + q` levels of convex combinations (a little past
    // them where continued), then one translation: a generous count of
    // roundings, each of the largest size met.
    let slack = gamma(8 * (p + q) + 16) * size + TINY;
    let [l0, l1, l2, l3] = lo;
    let [h0, h1, h2, h3] = hi;
    let iv = |l: f64, h: f64| Iv {
        lo: l - slack,
        hi: h + slack,
    };
    let e = Encl {
        x: [iv(l0, h0), iv(l1, h1), iv(l2, h2)],
        w: iv(l3, h3),
    };
    (e.x.iter().all(|x| x.finite()) && e.w.finite()).then_some(e)
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
    /// `c`'s coordinates.
    uv: [Vec<f64>; 2],
    /// The curve's homogeneous coordinates, about the curve's first point.
    gamma: [Vec<f64>; 4],
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

/// The rows `Q` square to the direction `d` (any two spanning that
/// plane: the condition `Q (S − γ) = 0` is `S − γ` along `d′ = q₀ × q₁`,
/// within rounding of `d`, which the bound accounts for).
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

/// A proven bound on the distance between the edge `S(c(t))` of `uv` on
/// `surface` and the projection of `curve` by `how`, at every `t` of the
/// curve's domain (`uv` is over the same parameter). Refused, with the
/// place, where the projection cannot be proven within
/// [`MAX_PROOF_PIECES`] pieces (a grazing direction, a focal point of the
/// surface, a curve whose projection turns faster than the pieces can
/// follow).
pub fn projection_bound(
    surface: &NurbsSurface,
    curve: &NurbsCurve,
    uv: &NurbsCurve,
    how: Projection,
) -> Result<ProvenGap, TrimError> {
    let (ta, _) = curve.domain();
    let o = curve.eval(ta);
    if !(o.x.is_finite() && o.y.is_finite() && o.z.is_finite()) {
        return refuse("the projected curve has no finite point".into());
    }
    let nets = Nets::of(surface, o)?;
    let (p, q) = nets.x.degrees;
    let rows = match how {
        Projection::Normal => None,
        Projection::Along(d) => Some(
            rows_square_to(d)
                .ok_or_else(|| TrimError::Refused("the direction has no length".into()))?,
        ),
    };
    // The edge and the derivatives along it, piece by piece (one set of
    // bounds: they depend only on `uv` and the knot lines).
    let (dx, bounds, px) = compose_net_pieces(&nets.x, uv, Composed::Any)?;
    let (_, bu, pu) = compose_net_pieces(&nets.u, uv, Composed::Any)?;
    let (_, bv, pv) = compose_net_pieces(&nets.v, uv, Composed::Any)?;
    if bu != bounds || bv != bounds || pu.len() != px.len() || pv.len() != px.len() {
        return refuse("the edge's pieces and its derivatives' differ; not proven".into());
    }
    let (ub, upieces) = bezier_pieces_raw(Raw::of(uv))?;
    let (gb, gpieces) = bezier_pieces_raw(Raw::of(curve))?;
    let gpieces: Vec<Vec<H>> = gpieces
        .into_iter()
        .map(|piece| {
            piece
                .into_iter()
                .map(|h| {
                    [
                        h[0] - o.x * h[3],
                        h[1] - o.y * h[3],
                        h[2] - o.z * h[3],
                        h[3],
                    ]
                })
                .collect()
        })
        .collect();
    let g_degree = curve.degree();
    let mut binom = Binomials::new();
    let mut todo: Vec<Span> = Vec::new();
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
            let x: [Vec<f64>; 4] = [
                cut(x_piece, 0),
                cut(x_piece, 1),
                cut(x_piece, 2),
                cut(x_piece, 3),
            ];
            let xu: [Vec<f64>; 4] = [
                cut(u_piece, 0),
                cut(u_piece, 1),
                cut(u_piece, 2),
                cut(u_piece, 3),
            ];
            let xv: [Vec<f64>; 4] = [
                cut(v_piece, 0),
                cut(v_piece, 1),
                cut(v_piece, 2),
                cut(v_piece, 3),
            ];
            // The curve's piece and `c`'s.
            let gi = span_of(&gb, mid);
            let (Some(&g0), Some(&g1), Some(gp)) = (gb.get(gi), gb.get(gi + 1), gpieces.get(gi))
            else {
                return refuse("the curve lost a piece; not proven".into());
            };
            let (la, lb) = ((sa_t - g0) / (g1 - g0), (sb_t - g0) / (g1 - g0));
            let gam: [Vec<f64>; 4] = [0, 1, 2, 3].map(|c| sub(&coord(gp, c), la, lb));
            let ui = span_of(&ub, mid);
            let (Some(&c0), Some(&c1), Some(cp)) = (ub.get(ui), ub.get(ui + 1), upieces.get(ui))
            else {
                return refuse("the parameter curve lost a piece; not proven".into());
            };
            let (ca, cb) = ((sa_t - c0) / (c1 - c0), (sb_t - c0) / (c1 - c0));
            let uvc: [Vec<f64>; 2] = [sub(&coord(cp, 0), ca, cb), sub(&coord(cp, 1), ca, cb)];
            // The patch it lies in, and how far its local coefficients
            // reach outside it (the composition's terms grow by that).
            let um = bern_eval(&uvc[0], 0.5);
            let vm = bern_eval(&uvc[1], 0.5);
            let (i, j) = (span_of(&nets.x.ku, um), span_of(&nets.x.kv, vm));
            let nv = nets.x.kv.len().saturating_sub(1);
            let (Some(&(m_about, m_origin, m_w)), Some(&ka), Some(&kb), Some(&la2), Some(&lb2)) = (
                nets.sizes.get(i * nv + j),
                nets.x.ku.get(i),
                nets.x.ku.get(i + 1),
                nets.x.kv.get(j),
                nets.x.kv.get(j + 1),
            ) else {
                return refuse("the edge left the surface's patches; not proven".into());
            };
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
            let m_g = max_abs(g0c).max(max_abs(g1c)).max(max_abs(g2c))
                + o.x.abs().max(o.y.abs()).max(o.z.abs()) * m_om;
            // A = ω X − w Γ, coordinate by coordinate.
            let mut big_a: Vec<Vec<f64>> = Vec::with_capacity(3);
            for (xc, gc) in [(x0, g0c), (x1, g1c), (x2, g2c)] {
                let l = binom.product(g3c, xc);
                let r = binom.product(x3, gc);
                big_a.push(poly_sub(&l, &r, &mut binom));
            }
            let m_a = m_om * m_x + m_wt * m_g;
            let (f, round, deg) = match rows {
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
                    let deg = fs[0].len().max(fs[1].len());
                    let k = 16 * (deg + p + q + g_degree + 8);
                    let rb_u = m_wt * m_xu + m_wu * m_x;
                    let rb_v = m_wt * m_xv + m_wv * m_x;
                    (
                        fs,
                        [gamma(k) * 3.0 * m_a * rb_u, gamma(k) * 3.0 * m_a * rb_v],
                        deg,
                    )
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
                        *r = (row.x.abs() + row.y.abs() + row.z.abs()) * m_a;
                    }
                    let deg = fs[0].len().max(fs[1].len());
                    let k = 16 * (deg + p + q + g_degree + 8);
                    (fs, [gamma(k) * rounds[0], gamma(k) * rounds[1]], deg)
                }
            };
            let k_edge = 16 * (dx + p + q + 8);
            todo.push(Span {
                t: (sa_t, sb_t),
                depth: 0,
                f,
                round,
                uv: uvc,
                gamma: gam,
                edge: (
                    gamma(k_edge + deg),
                    alpha(p, q) * m_origin,
                    alpha(p, q) * m_w,
                ),
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
        if done > MAX_PROOF_PIECES {
            return refuse(format!(
                "the projection is not proven within {MAX_PROOF_PIECES} pieces (near t = {})",
                span.t.0
            ));
        }
        match prove_span(&nets, &span, rows) {
            Some(d) => worst = worst.max(d),
            None => {
                if span.depth >= MAX_DEPTH {
                    return refuse(format!(
                        "the projection is not proven between t = {} and {} (the projection's condition does not contract there: a grazing direction or a focal point of the surface)",
                        span.t.0, span.t.1
                    ));
                }
                let (left, right) = halves(&span);
                todo.push(right);
                todo.push(left);
            }
        }
    }
    if !worst.is_finite() {
        return refuse("the projection's bound is not finite; not proven".into());
    }
    Ok(ProvenGap {
        bound: worst,
        pieces: done,
    })
}

/// The two halves of a piece, each polynomial cut by de Casteljau (whose
/// rounding, convex combinations, is added to the residuals' allowance).
fn halves(span: &Span) -> (Span, Span) {
    let half = |c: &[f64], first: bool| {
        let (l, r) = split(c, 0.5);
        if first { l } else { r }
    };
    let make = |first: bool| {
        let f = [half(&span.f[0], first), half(&span.f[1], first)];
        let round = [
            span.round[0] + gamma(2 * span.f[0].len() + 2) * max_abs(&span.f[0]),
            span.round[1] + gamma(2 * span.f[1].len() + 2) * max_abs(&span.f[1]),
        ];
        let m = 0.5 * span.t.0 + 0.5 * span.t.1;
        Span {
            t: if first { (span.t.0, m) } else { (m, span.t.1) },
            depth: span.depth + 1,
            f,
            round,
            uv: [half(&span.uv[0], first), half(&span.uv[1], first)],
            gamma: [
                half(&span.gamma[0], first),
                half(&span.gamma[1], first),
                half(&span.gamma[2], first),
                half(&span.gamma[3], first),
            ],
            edge: span.edge,
        }
    };
    (make(true), make(false))
}

/// The hull of a polynomial's coefficients, widened by `extra`.
fn hull(c: &[f64], extra: f64) -> Iv {
    let lo = c.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = c.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if c.iter().any(|x| !x.is_finite()) || c.is_empty() {
        return WHOLE;
    }
    let pad = extra + gamma(2 * c.len() + 2) * lo.abs().max(hi.abs()) + TINY;
    Iv {
        lo: lo - pad,
        hi: hi + pad,
    }
}

/// The Jacobian of the condition over a box, and what the distance needs
/// there (`|B_u|`, `|B_v|` upper bounds, `w`'s lower bound, `|A|`'s).
struct AtBox {
    j: [[Iv; 2]; 2],
    bu: f64,
    bv: f64,
    w_lo: f64,
    a: f64,
    om_lo: f64,
}

fn at_box(nets: &Nets, span: &Span, o2: Vec3, r: f64, rows: Option<[Vec3; 2]>) -> Option<AtBox> {
    let uh = hull(&span.uv[0], 0.0);
    let vh = hull(&span.uv[1], 0.0);
    let bx = [uh.lo - r, uh.hi + r, vh.lo - r, vh.hi + r];
    let x = enclose(&nets.x, bx, o2)?;
    let xu = enclose(&nets.u, bx, o2)?;
    let xv = enclose(&nets.v, bx, o2)?;
    // The curve about o2, over the piece.
    let om = hull(&span.gamma[3], 0.0);
    let [g0c, g1c, g2c, g3c] = &span.gamma;
    let about = |gc: &[f64], oc: f64| -> Iv {
        let coefs: Vec<f64> = gc.iter().zip(g3c).map(|(a, w)| a - oc * w).collect();
        let size = gc
            .iter()
            .zip(g3c)
            .fold(0.0_f64, |m, (a, w)| m.max(a.abs() + (oc * w).abs()));
        hull(&coefs, 2.0 * EPS * size)
    };
    let g: V3 = [about(g0c, o2.x), about(g1c, o2.y), about(g2c, o2.z)];
    let a = v_sub(v_scale(om, x.x), v_scale(x.w, g));
    let a_u = v_sub(v_scale(om, xu.x), v_scale(xu.w, g));
    let a_v = v_sub(v_scale(om, xv.x), v_scale(xv.w, g));
    let b_u = v_sub(v_scale(x.w, xu.x), v_scale(xu.w, x.x));
    let b_v = v_sub(v_scale(x.w, xv.x), v_scale(xv.w, x.x));
    let j = match rows {
        None => {
            let xuu = enclose(&nets.uu, bx, o2)?;
            let xuv = enclose(&nets.uv, bx, o2)?;
            let xvv = enclose(&nets.vv, bx, o2)?;
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
    Some(AtBox {
        j,
        bu: v_norm(b_u),
        bv: v_norm(b_v),
        w_lo: x.w.lo,
        a: v_norm(a),
        om_lo: om.lo,
    })
}

/// The bound on one piece, or `None` where Krawczyk's condition does not
/// hold there (the caller halves it).
fn prove_span(nets: &Nets, span: &Span, rows: Option<[Vec3; 2]>) -> Option<f64> {
    // The point the hulls are written about: the curve's middle.
    let wm = bern_eval(&span.gamma[3], 0.5);
    if !(wm.is_finite() && wm > 0.0) {
        return None;
    }
    let o2 = Vec3::new(
        bern_eval(&span.gamma[0], 0.5) / wm,
        bern_eval(&span.gamma[1], 0.5) / wm,
        bern_eval(&span.gamma[2], 0.5) / wm,
    );
    let first = at_box(nets, span, o2, 0.0, rows)?;
    let m = first.j.map(|row| row.map(Iv::mid));
    let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    let size = (m[0][0] * m[1][1]).abs() + (m[0][1] * m[1][0]).abs();
    if !(det.is_finite() && det.abs() > 1e-12 * size) {
        return None;
    }
    let y = [
        [m[1][1] / det, -m[0][1] / det],
        [-m[1][0] / det, m[0][0] / det],
    ];
    // |Y F| over the piece: the hull of the combined coefficients.
    if span.f[0].len() != span.f[1].len() {
        return None;
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
    let rho_max = rho[0].max(rho[1]);
    if !rho_max.is_finite() {
        return None;
    }
    // The box's radius: a little over the residual first (the contraction
    // is usually far below one), larger while the condition fails.
    let mut r = (rho_max * (1.0 + 1e-3)).max(TINY * 1e10);
    for _ in 0..6 {
        let at = at_box(nets, span, o2, r, rows)?;
        // The rows of |I − Y J| over the box; κ their largest sum.
        let mut sums = [0.0_f64; 2];
        let [j0, j1] = at.j;
        for (k, (yr, sum)) in y.iter().zip(sums.iter_mut()).enumerate() {
            for (l, (a, b)) in j0.into_iter().zip(j1).enumerate() {
                let yj = Iv::point(yr[0]).mul(a).add(Iv::point(yr[1]).mul(b));
                let id = Iv::point(if k == l { 1.0 } else { 0.0 });
                *sum += id.sub(yj).mag();
            }
            *sum *= 1.0 + 2.0 * EPS;
        }
        let kappa = sums[0].max(sums[1]);
        if !kappa.is_finite() {
            return None;
        }
        // Krawczyk: K(B) − c lies within ρ_k + Σ_l |M_kl| r in each
        // coordinate; strictly inside the box proves one root there, and
        // those are its bounds coordinate by coordinate.
        let reach = [
            (rho[0] + sums[0] * r) * (1.0 + 4.0 * EPS),
            (rho[1] + sums[1] * r) * (1.0 + 4.0 * EPS),
        ];
        if kappa < 1.0 && reach[0] < r && reach[1] < r {
            if !(at.w_lo > 0.0 && at.om_lo > 0.0) {
                return None;
            }
            let w2 = at.w_lo * at.w_lo * (1.0 - 4.0 * EPS);
            // |S(c + δ) − S(c)| ≤ |δ_u| max |S_u| + |δ_v| max |S_v|.
            let dist = (reach[0] * at.bu + reach[1] * at.bv) / w2;
            // The stored edge was composed about the origin, rounded.
            let (g, mx, mw) = span.edge;
            let edge = g * (mx / at.w_lo) * (1.0 + mw / at.w_lo);
            // Along a direction: the rows' own rounding turns `d` by a
            // few ε, moving the point by that much of its distance.
            let along = if rows.is_some() {
                8.0 * EPS * at.a / (at.om_lo * at.w_lo)
            } else {
                0.0
            };
            let total = (dist + edge + along) * (1.0 + 8.0 * EPS);
            return total.is_finite().then_some(total);
        }
        if kappa >= 0.9 {
            return None;
        }
        r = (1.1 * rho_max / (1.0 - kappa)).max(r * 1.5);
    }
    None
}
