//! How two faces meet along an edge, proven: the order of continuity
//! reached (G0, G1 or G2) and the largest gap in position, in the angle
//! between the tangent planes and in normal curvature across the edge,
//! each bounded over the whole edge, not only where it was looked at.
//!
//! **Rows along the edge.** One face (`a`) is a spline the edge runs along
//! a parameter line of: its homogeneous point `X₀(u)`, its derivative
//! along the line `X₀′`, and its first and second homogeneous derivatives
//! across the line `X₁`, `X₂` are exact splines in `u` (the line cut to the
//! edge, [`super::iso_track`]). The other face (`b`) is either
//! - an *implicit* face: a plane `L·X = 0`, or a quadric `XᵀQX = 0` (a
//!   cylinder, a cone, a sphere; `Q` written about the edge's first point,
//!   so nothing cancels far from the origin), or
//! - a spline the edge runs along a parameter line of as well, with its
//!   rows `Y₀`, `Y₀′`, `Y₀″`, `Y₁`, `Y₁′`, `Y₂`, in the same parameter: the
//!   two lines, each cut to the edge and run its way over `[0, 1]`, must
//!   be one parametrisation of the seam (a network built against that
//!   face is, [`super::gordon`]); where they are not, the position gap
//!   shows it and the measure refuses, since points that do not
//!   correspond cannot be compared.
//!
//! **Residuals, not samples.** Each gap is `|R(u)| / D(u)`, `R` an exact
//! polynomial on every piece between the rows' knots (products of the
//! rows' Bernstein coefficients) that vanishes where the faces meet to
//! that order, `D` a product of positive polynomials and norms of vector
//! polynomials. The Bernstein coefficients bound a polynomial over its
//! piece (the convex hull), so `max |R| / min D` is a proven bound, and a
//! piece whose bound does not decide is halved (de Casteljau) until it
//! does, or the pieces allowed ([`MAX_SEAM_PIECES`]) are spent: then that
//! order is not reached ("undecided"), never guessed. With `S = x / w` the
//! face `a`, `V₀′ = x₀′w − x₀w₀′`, `V₁ = x₁w − x₀w₁` (`S_u`, `S_v` times
//! `w²`), `W = V₀′ × V₁` (its normal times `w⁴`):
//! - *Position*: the distance to an implicit face through the exact
//!   distance of a plane (`L·X₀ / w`), of a sphere or cylinder
//!   (`|ρ − R| = |XᵀQX| / (w (|P(x − c w)| + R w))`, `P` the projection
//!   off the axis) or of a cone (to the generatrix in the point's
//!   meridian plane, which lies on the cone; the quadric is the double
//!   cone, so near the apex the distance is to the nearer of its two
//!   nappes, and to the whole line beyond the face's own extent); to a
//!   spline face, the
//!   distance between corresponding points `|x₀ω − y₀w| / (wω)`. Every
//!   point of a face is at least as far as its nearest, so each is an
//!   upper bound on the distance to the face.
//! - *Angle*: the sine of the angle between the tangent planes, `|W × G| /
//!   (|W| |G|)`, `G` the face's normal: `(QX₀)_xyz` (the gradient, which
//!   for a sphere and a cylinder points along the normal at the nearest
//!   point; for a cone it is the normal of the quadric's level surface
//!   through the point, the cone's own on it), `n` for a plane, `W_b` for
//!   a spline face.
//! - *Second order* (G2 here): with `u` along the edge and `v` the
//!   direction the face `a` leaves it, `a`'s second derivatives seen
//!   along `b`'s normal against `b`'s second fundamental form on the
//!   edge's tangent vectors, three gaps each held to the curvature
//!   threshold: across `(n_b·S_vv − II_b(S_v, S_v)) / |S_v|²`, twist
//!   `(n_b·S_uv − II_b(S_u, S_v)) / (|S_u| |S_v|)` and along
//!   `(n_b·S_uu − II_b(S_u, S_u)) / |S_u|²`. So a cross tangent that turns
//!   about the edge within the angular tolerance but twists the seam
//!   beyond the curvature threshold is not G2, nor an edge that wanders
//!   about `b` within the tolerance but bends off it. The normals
//!   themselves are G1's: `a`'s own forms differ from these by at most
//!   `|n_a − n_b| |S_xy|`, the angle gap times `a`'s second derivatives
//!   (with their tangential, parametric part), which is not added: the
//!   angle's bound is a rounding bound far above the true angle of a held
//!   seam, and times the parametric acceleration it would hide every G2
//!   seam behind the angular tolerance. `II_b` is `b`'s form where `a`
//!   is: for an implicit face `g = 0`, that of its level surface through
//!   the point, `−Hess g(X, Y) / |∇g|`, so `n_b·S_xy − II_b(X, Y) =
//!   ∂x∂y g(S) / |∇g|`: in homogeneous terms
//!   `P_xy = H_xy w² − 2w(H_x w_y + H_y w_x) + (6 w_x w_y − 2 w w_xy) H`, `H = XᵀQX` and its
//!   derivatives, for a quadric, `L·X / w` likewise for a plane; for a
//!   spline face, its form at the corresponding point on `X` and `Y` put
//!   into its tangent plane (Cramer's rule in `U_b, V_b`), through
//!   `det[Y, Y_a, Y_t, Y_ab] = −ω⁴ (B_a × B_t)·B_ab` (Farin ch. 22).
//!
//! **Rounding.** Every coefficient carries a bound on its rounding error,
//! carried through each sum and product (running error analysis, Higham
//! ch. 3). The rows are formed from the stored net by differences first
//! (`spline_rows`): the net differentiated across and along as B-spline
//! derivative nets, the first difference each way taken of the points as
//! stored (for equal weights one rounding relative to the difference,
//! not to the points' distance from anything; for unequal weights a term
//! `(P_a − o)(w_b − w_a)` adds `γ₄ |P_a − o| |w_b − w_a|`), then combined at the line with basis values
//! that are nonnegative and sum to 1 (on a clamped border, one of them 1
//! and the others 0, exactly). So every row, including the derivatives
//! along the edge, carries the rounding of its own size, coefficient by
//! coefficient and coordinate by coordinate. The rows are cut into Bézier
//! pieces by blossoming each piece from the knot span it lies in
//! (`row_pieces`, at most `γ_{11p}` of the span's coordinates), in each
//! face's own parameter: no knot is rounded by a reparametrisation. Each
//! product of degrees `m` and `n` adds `γ_k`,
//! `k = 4 (m + n) + min(m, n) + 8`, times the product of the magnitudes
//! (`Poly::mul_gamma`). A NaN anywhere makes a bound infinite. Bounds take
//! the errors in.
//!
//! **Thresholds** come from the document: the position from the
//! tolerance, the angle from the angular tolerance, and the curvature
//! from both and the edge's length `L` (its control polygon's): a
//! difference `Δκ` turns the two faces' normals apart by `Δκ d` and opens
//! a gap of `Δκ d² / 2` over a distance `d` from the seam, so over the
//! seam's own length it is held to `min(angular / L, 2 tol / L²)`.

use super::{H, NetworkError, Raw, Track, internal, point, refuse};
use crate::refine::SAME_KNOT;
use crate::{NurbsCurve, NurbsSurface, Surface};
use craft3d_math::Portable;
use craft3d_math::Vec3;

/// Most pieces an edge is cut into to decide how its faces meet.
pub const MAX_SEAM_PIECES: usize = 4096;

/// Highest degree of the polynomials whose coefficients bound the gaps
/// (their binomials stay far inside the floating-point range).
pub const MAX_SEAM_DEGREE: usize = 384;

const EPS: f64 = f64::EPSILON;

/// The order of continuity proven along an edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Order {
    /// Not even in position within the tolerance.
    None,
    /// In position.
    G0,
    /// In position and with one tangent plane.
    G1,
    /// In position, with one tangent plane, and the face's second
    /// derivatives along the other's normal matching that face's second
    /// fundamental form on the edge's tangent vectors (normal curvature
    /// across and along, and twist).
    G2,
}

impl Order {
    pub fn name(self) -> &'static str {
        match self {
            Order::None => "none",
            Order::G0 => "G0",
            Order::G1 => "G1",
            Order::G2 => "G2",
        }
    }
}

/// Most work the measure does on one edge (in units of a product of two
/// coefficients, about): it runs inside assertions after every
/// regeneration, so past this an order not yet decided is not reached.
pub const MAX_SEAM_WORK: u64 = 1 << 32;

/// One measured gap along an edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
    /// A proven bound on the largest gap along the whole edge.
    pub bound: f64,
    /// The largest gap found where it was looked at (a lower bound on the
    /// largest), with the edge's parameter (0 to 1 along it) and the point.
    pub found: f64,
    pub param: f64,
    pub at: Vec3,
    /// The threshold it is held to.
    pub threshold: f64,
}

impl Gap {
    /// Proven within the threshold.
    pub fn within(&self) -> bool {
        self.bound <= self.threshold
    }

    /// Proven beyond it (seen beyond it somewhere).
    pub fn beyond(&self) -> bool {
        self.found > self.threshold
    }
}

/// How two faces meet along an edge. Angles are radians, curvatures per
/// unit of length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Continuity {
    pub order: Order,
    pub position: Gap,
    pub angle: Gap,
    /// The normal curvature across the edge: `n_b·S_vv − II_b(S_v)` over
    /// `|S_v|²`, `v` the direction the face `a` leaves it.
    pub curvature: Gap,
    /// The twist: `n_b·S_uv − II_b(S_u, S_v)` over `|S_u| |S_v|`, `u` along
    /// the edge, held to the curvature threshold.
    pub twist: Gap,
    /// The normal curvature along the edge: `n_b·S_uu − II_b(S_u, S_u)` over
    /// `|S_u|²`, held to the curvature threshold.
    pub along: Gap,
    /// The length the curvature threshold is derived from.
    pub length: f64,
    /// Pieces the edge was cut into.
    pub pieces: usize,
    /// The work allowed ([`MAX_SEAM_WORK`], [`MAX_SEAM_PIECES`]) was
    /// spent before every gap was decided.
    pub exhausted: bool,
}

impl Continuity {
    /// The three gaps of the second order: across, twist, along.
    pub fn second_order(&self) -> [&Gap; 3] {
        [&self.curvature, &self.twist, &self.along]
    }
}

/// The thresholds of an edge of length `length`: tolerance, angular
/// tolerance, and `min(angular / L, 2 tol / L²)` (see the module docs).
pub fn thresholds(tolerance: f64, angular: f64, length: f64) -> [f64; 3] {
    let l = length.max(f64::MIN_POSITIVE);
    [
        tolerance,
        angular,
        (angular / l).min(2.0 * tolerance / (l * l)),
    ]
}

// ---------------------------------------------------------------------------
// Polynomials in Bernstein form over a piece, with a bound on their
// rounding.

#[derive(Clone, Debug)]
struct Poly {
    c: Vec<f64>,
    /// Every coefficient is within `e` of the exact one.
    e: f64,
}

fn binomials(n: usize) -> Vec<f64> {
    let mut row = Vec::with_capacity(n + 1);
    let mut value = 1.0_f64;
    for k in 0..=n {
        row.push(value);
        value = value * (n - k) as f64 / (k + 1) as f64;
    }
    row
}

impl Poly {
    fn konst(x: f64) -> Poly {
        Poly { c: vec![x], e: 0.0 }
    }

    fn degree(&self) -> usize {
        self.c.len().saturating_sub(1)
    }

    /// A NaN anywhere (`f64::max` and `min` would pass it over): then
    /// nothing is known.
    fn unknown(&self) -> bool {
        self.e.is_nan() || self.c.iter().any(|x| x.is_nan())
    }

    fn mag(&self) -> f64 {
        if self.unknown() {
            return f64::INFINITY;
        }
        self.c.iter().fold(0.0_f64, |m, x| m.max(x.abs()))
    }

    fn lo(&self) -> f64 {
        if self.unknown() {
            return f64::NEG_INFINITY;
        }
        self.c.iter().copied().fold(f64::INFINITY, f64::min) - self.e
    }

    fn hi(&self) -> f64 {
        if self.unknown() {
            return f64::INFINITY;
        }
        self.c.iter().copied().fold(f64::NEG_INFINITY, f64::max) + self.e
    }

    /// The rounding of a product of degrees `m` and `n`, relative to the
    /// product of the factors' magnitudes. Each coefficient is
    /// `Σ C(m,i) C(n,j) / C(m+n,i+j) aᵢ bⱼ` over at most `min(m,n) + 1`
    /// terms whose weights sum to 1 (Vandermonde). The binomials come from
    /// the recurrence `C(n,k+1) = C(n,k)(n−k)/(k+1)`, two roundings a step
    /// (exact while `C(n,k)·n < 2⁵³`, about `n ≤ 56`, but not beyond), so
    /// within `2n` roundings; a term is `C(m,i)·C(n,j)/C(m+n,k)·a·b`, four
    /// more; the sum adds `min(m,n)`. So `γ_k` with
    /// `k = 4(m+n) + min(m,n) + 4` (Higham, lemma 3.1 and §3.1), taken
    /// with a margin of 4.
    fn mul_gamma(m: usize, n: usize) -> f64 {
        let k = (4 * (m + n) + m.min(n) + 8) as f64;
        let ke = k * EPS;
        ke / (1.0 - ke).max(0.5)
    }

    fn mul(&self, o: &Poly) -> Poly {
        let (m, n) = (self.degree(), o.degree());
        let (bm, bn, bmn) = (binomials(m), binomials(n), binomials(m + n));
        let mut out = vec![0.0; m + n + 1];
        for (i, (a, ba)) in self.c.iter().zip(&bm).enumerate() {
            if *a == 0.0 {
                continue;
            }
            for (j, (b, bb)) in o.c.iter().zip(&bn).enumerate() {
                if let (Some(slot), Some(d)) = (out.get_mut(i + j), bmn.get(i + j)) {
                    *slot += ba * bb / d * a * b;
                }
            }
        }
        let (ma, mb) = (self.mag(), o.mag());
        let gamma = Poly::mul_gamma(m, n);
        // What the factors carried, through weights within `γ` of summing
        // to 1, and the product's own rounding.
        Poly {
            c: out,
            e: (1.0 + gamma) * (ma * o.e + mb * self.e + self.e * o.e) + gamma * ma * mb,
        }
    }

    fn elevated(&self, degree: usize) -> Poly {
        if degree <= self.degree() {
            return self.clone();
        }
        let ones = Poly {
            c: vec![1.0; degree - self.degree() + 1],
            e: 0.0,
        };
        self.mul(&ones)
    }

    /// `fa · self + fb · o`.
    fn lin(&self, fa: f64, o: &Poly, fb: f64) -> Poly {
        let d = self.degree().max(o.degree());
        let (a, b) = (self.elevated(d), o.elevated(d));
        let c: Vec<f64> = a.c.iter().zip(&b.c).map(|(x, y)| fa * x + fb * y).collect();
        Poly {
            c,
            e: fa.abs() * a.e
                + fb.abs() * b.e
                + 2.0 * EPS * (fa.abs() * a.mag() + fb.abs() * b.mag()),
        }
    }

    fn add(&self, o: &Poly) -> Poly {
        self.lin(1.0, o, 1.0)
    }

    fn sub(&self, o: &Poly) -> Poly {
        self.lin(1.0, o, -1.0)
    }

    fn scale(&self, f: f64) -> Poly {
        Poly {
            c: self.c.iter().map(|x| x * f).collect(),
            e: f.abs() * self.e + EPS * f.abs() * self.mag(),
        }
    }

    /// The two halves (de Casteljau at ½).
    fn halves(&self) -> (Poly, Poly) {
        let mut row = self.c.clone();
        let n = row.len();
        let mut left = Vec::with_capacity(n);
        let mut right = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(f) = row.first() {
                left.push(*f);
            }
            if let Some(l) = row.last() {
                right.push(*l);
            }
            row = row
                .windows(2)
                .map(|w| match w {
                    [x, y] => 0.5 * (x + y),
                    _ => 0.0,
                })
                .collect();
        }
        right.reverse();
        let e = self.e + 2.0 * n as f64 * EPS * self.mag();
        (Poly { c: left, e }, Poly { c: right, e })
    }

    fn at(&self, t: f64) -> Poly {
        let mut row = self.c.clone();
        while row.len() > 1 {
            row = row
                .windows(2)
                .map(|w| match w {
                    [x, y] => x + t * (y - x),
                    _ => 0.0,
                })
                .collect();
        }
        Poly {
            c: vec![row.first().copied().unwrap_or(0.0)],
            e: self.e + 2.0 * self.c.len() as f64 * EPS * self.mag(),
        }
    }
}

type V3 = [Poly; 3];
type P4 = [Poly; 4];

fn dot(a: &V3, b: &V3) -> Poly {
    a[0].mul(&b[0]).add(&a[1].mul(&b[1])).add(&a[2].mul(&b[2]))
}

fn cross(a: &V3, b: &V3) -> V3 {
    [
        a[1].mul(&b[2]).sub(&a[2].mul(&b[1])),
        a[2].mul(&b[0]).sub(&a[0].mul(&b[2])),
        a[0].mul(&b[1]).sub(&a[1].mul(&b[0])),
    ]
}

fn times(a: &V3, f: &Poly) -> V3 {
    [a[0].mul(f), a[1].mul(f), a[2].mul(f)]
}

fn minus(a: &V3, b: &V3) -> V3 {
    [a[0].sub(&b[0]), a[1].sub(&b[1]), a[2].sub(&b[2])]
}

fn konst3(v: Vec3) -> V3 {
    [Poly::konst(v.x), Poly::konst(v.y), Poly::konst(v.z)]
}

fn xyz4(h: &P4) -> V3 {
    [h[0].clone(), h[1].clone(), h[2].clone()]
}

/// `Σ m_ij h_j` for a 4 × 4 matrix.
fn apply(m: &[[f64; 4]; 4], h: &P4) -> P4 {
    let row = |i: usize| {
        let r = m.get(i).copied().unwrap_or([0.0; 4]);
        h[0].scale(r[0])
            .add(&h[1].scale(r[1]))
            .add(&h[2].scale(r[2]))
            .add(&h[3].scale(r[3]))
    };
    [row(0), row(1), row(2), row(3)]
}

fn dot4(a: &P4, b: &P4) -> Poly {
    a[0].mul(&b[0])
        .add(&a[1].mul(&b[1]))
        .add(&a[2].mul(&b[2]))
        .add(&a[3].mul(&b[3]))
}

/// `det[a, b, c, d]` of four homogeneous columns (Laplace along the first
/// two).
fn det4(a: &P4, b: &P4, c: &P4, d: &P4) -> Poly {
    let zero = Poly::konst(0.0);
    let m = |x: &P4, y: &P4, i: usize, j: usize| {
        let at = |h: &P4, k: usize| h.get(k).unwrap_or(&zero).clone();
        at(x, i).mul(&at(y, j)).sub(&at(x, j).mul(&at(y, i)))
    };
    let pairs = [
        (0, 1, 2, 3, 1.0),
        (0, 2, 1, 3, -1.0),
        (0, 3, 1, 2, 1.0),
        (1, 2, 0, 3, 1.0),
        (1, 3, 0, 2, -1.0),
        (2, 3, 0, 1, 1.0),
    ];
    let mut out = Poly::konst(0.0);
    for (i, j, k, l, s) in pairs {
        out = out.lin(1.0, &m(a, b, i, j).mul(&m(c, d, k, l)), s);
    }
    out
}

/// The largest norm over the piece of a vector polynomial (an upper
/// bound), with its rounding.
fn norm_hi(v: &V3) -> f64 {
    let d = v.iter().map(Poly::degree).max().unwrap_or(0);
    let e: Vec<Poly> = v.iter().map(|p| p.elevated(d)).collect();
    let err = e.iter().map(|p| p.e * p.e).sum::<f64>().sqrt();
    let mut best = 0.0_f64;
    for k in 0..=d {
        let s: f64 = e
            .iter()
            .map(|p| p.c.get(k).copied().unwrap_or(0.0).ppowi(2))
            .sum();
        best = best.max(s.sqrt());
    }
    best + err
}

/// A lower bound on the norm of a vector polynomial over the piece.
fn norm_lo(v: &V3) -> f64 {
    dot(v, v).lo().max(0.0).sqrt()
}

/// The value of a constant vector polynomial, less its rounding.
fn norm_value_lo(v: &V3) -> f64 {
    let err = v.iter().map(|p| p.e * p.e).sum::<f64>().sqrt();
    let s: f64 = v
        .iter()
        .map(|p| p.c.first().copied().unwrap_or(0.0).ppowi(2))
        .sum();
    (s.sqrt() - err).max(0.0)
}

fn norm_value_hi(v: &V3) -> f64 {
    let err = v.iter().map(|p| p.e * p.e).sum::<f64>().sqrt();
    let s: f64 = v
        .iter()
        .map(|p| p.c.first().copied().unwrap_or(0.0).ppowi(2))
        .sum();
    s.sqrt() + err
}

fn abs_hi(p: &Poly) -> f64 {
    p.mag() + p.e
}

fn abs_value_lo(p: &Poly) -> f64 {
    (p.c.first().copied().unwrap_or(0.0).abs() - p.e).max(0.0)
}

/// A lower bound on `|p|` over the piece (zero where it may change sign).
fn abs_lo(p: &Poly) -> f64 {
    let (lo, hi) = (p.lo(), p.hi());
    if lo > 0.0 {
        lo
    } else if hi < 0.0 {
        -hi
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// The faces along the edge.

/// What the other face is, in coordinates about the edge's first point.
#[derive(Clone, Debug)]
enum Other {
    Plane {
        l: H,
    },
    /// A sphere (no axis) or a cylinder about `centre`, `radius`.
    Round {
        q: [[f64; 4]; 4],
        centre: Vec3,
        axis: Option<Vec3>,
        radius: f64,
    },
    Cone {
        q: [[f64; 4]; 4],
        apex: Vec3,
        axis: Vec3,
        tan: f64,
        cos: f64,
    },
    /// A spline face, its rows `Y₀, Y₀′, Y₀″, Y₁, Y₁′, Y₂` along the edge
    /// following the face `a`'s.
    Spline,
}

/// The quadric `xᵀAx + 2bᵀx + c` as a homogeneous 4 × 4 matrix.
fn quadric(a: [[f64; 3]; 3], b: Vec3, c: f64) -> [[f64; 4]; 4] {
    let [r0, r1, r2] = a;
    [
        [r0[0], r0[1], r0[2], b.x],
        [r1[0], r1[1], r1[2], b.y],
        [r2[0], r2[1], r2[2], b.z],
        [b.x, b.y, b.z, c],
    ]
}

/// `I − k ddᵀ` (and `j` times `ddᵀ` subtracted for a cone).
fn projector(d: Vec3, k: f64) -> [[f64; 3]; 3] {
    let v = [d.x, d.y, d.z];
    let row = |i: usize, vi: f64| {
        let mut r = [0.0; 3];
        for (j, (slot, vj)) in r.iter_mut().zip(v).enumerate() {
            *slot = if i == j { 1.0 } else { 0.0 } - k * vi * vj;
        }
        r
    };
    [row(0, d.x), row(1, d.y), row(2, d.z)]
}

fn mat3(a: &[[f64; 3]; 3], x: Vec3) -> Vec3 {
    Vec3::new(
        a[0][0] * x.x + a[0][1] * x.y + a[0][2] * x.z,
        a[1][0] * x.x + a[1][1] * x.y + a[1][2] * x.z,
        a[2][0] * x.x + a[2][1] * x.y + a[2][2] * x.z,
    )
}

/// The implicit form of an analytic face about `origin`: a plane's
/// `L = (n, −n·p)`, or a quadric's matrix; `None` for a torus or a spline.
pub(super) fn implicit_of(surface: &Surface, origin: Vec3) -> Option<ImplicitForm> {
    match surface {
        Surface::Plane(p) => {
            let n = p.frame().z();
            let o = p.frame().origin() - origin;
            Some(ImplicitForm::Plane([n.x, n.y, n.z, -n.dot(o)]))
        }
        Surface::Sphere(s) => {
            let c = s.frame().origin() - origin;
            let r = s.radius();
            let a = projector(Vec3::new(1.0, 0.0, 0.0), 0.0);
            Some(ImplicitForm::Quadric(
                quadric(a, c * -1.0, c.dot(c) - r * r),
                Round {
                    centre: c,
                    axis: None,
                    radius: r,
                },
            ))
        }
        Surface::Cylinder(cy) => {
            let o = cy.frame().origin() - origin;
            let d = cy.frame().z();
            let r = cy.radius();
            let a = projector(d, 1.0);
            let ao = mat3(&a, o);
            Some(ImplicitForm::Quadric(
                quadric(a, ao * -1.0, o.dot(ao) - r * r),
                Round {
                    centre: o,
                    axis: Some(d),
                    radius: r,
                },
            ))
        }
        Surface::Cone(co) => {
            let d = co.frame().z();
            let apex = co.frame().point(0.0, 0.0, co.apex_v()) - origin;
            let t = co.half_angle().ptan();
            let a = projector(d, 1.0 + t * t);
            let aa = mat3(&a, apex);
            Some(ImplicitForm::Cone(
                quadric(a, aa * -1.0, apex.dot(aa)),
                apex,
                d,
                t.abs(),
                co.half_angle().pcos(),
            ))
        }
        _ => None,
    }
}

/// A sphere's or cylinder's centre (a point of its axis), axis and radius.
#[derive(Clone, Copy, Debug)]
pub(super) struct Round {
    centre: Vec3,
    axis: Option<Vec3>,
    radius: f64,
}

/// The implicit form of an analytic face.
#[derive(Clone, Copy, Debug)]
pub(super) enum ImplicitForm {
    Plane(H),
    Quadric([[f64; 4]; 4], Round),
    /// Matrix, apex, axis, `|tan α|`, `cos α`.
    Cone([[f64; 4]; 4], Vec3, Vec3, f64, f64),
}

impl ImplicitForm {
    /// The covector `N` of the second-order condition `N·X₂ = r` at a point
    /// `X₀` with cross derivative `X₁` (homogeneous, about the origin the
    /// form was made about): `G(X(v)) = O(v³)` for `G = L·X` or `XᵀQX`.
    pub(super) fn second_order(&self, x0: H, x1: H) -> (H, f64) {
        match self {
            ImplicitForm::Plane(l) => (*l, 0.0),
            ImplicitForm::Quadric(q, _) | ImplicitForm::Cone(q, ..) => {
                let qx = |x: H| -> H {
                    let mut out = [0.0; 4];
                    for (i, slot) in out.iter_mut().enumerate() {
                        let r = q.get(i).copied().unwrap_or([0.0; 4]);
                        *slot = r[0] * x[0] + r[1] * x[1] + r[2] * x[2] + r[3] * x[3];
                    }
                    out
                };
                let d = |a: H, b: H| a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
                (qx(x0), -d(x1, qx(x1)))
            }
        }
    }

    fn other(&self) -> Other {
        match *self {
            ImplicitForm::Plane(l) => Other::Plane { l },
            ImplicitForm::Quadric(q, r) => Other::Round {
                q,
                centre: r.centre,
                axis: r.axis,
                radius: r.radius,
            },
            ImplicitForm::Cone(q, apex, axis, tan, cos) => Other::Cone {
                q,
                apex,
                axis,
                tan,
                cos,
            },
        }
    }
}

/// `γ_k = k ε / (1 − k ε)` (Higham, lemma 3.1): `k` roundings, each
/// within `ε` relative, compound to within `γ_k` relative.
fn gamma(k: usize) -> f64 {
    let ke = k as f64 * EPS;
    ke / (1.0 - ke).max(0.5)
}

/// A homogeneous coefficient (`x w`, `y w`, `z w`, `w`) with a bound on
/// each coordinate's rounding: the exact value is within `e[c]` of `v[c]`.
#[derive(Clone, Copy, Debug)]
struct Ep {
    v: H,
    e: [f64; 4],
}

const EP_ZERO: Ep = Ep {
    v: [0.0; 4],
    e: [0.0; 4],
};

impl Ep {
    /// A coefficient made coordinate by coordinate: `f` gives each
    /// coordinate's value and bound from its index.
    fn from_fn(f: impl Fn(usize) -> (f64, f64)) -> Ep {
        let mut out = EP_ZERO;
        for (c, (v, e)) in out.v.iter_mut().zip(out.e.iter_mut()).enumerate() {
            (*v, *e) = f(c);
        }
        out
    }

    /// Coordinate `c`'s value and bound.
    fn at(&self, c: usize) -> (f64, f64) {
        (
            self.v.get(c).copied().unwrap_or(f64::NAN),
            self.e.get(c).copied().unwrap_or(f64::INFINITY),
        )
    }

    /// `self − o`: `fl(a − b) = (a − b)(1 + δ)`, so what the two carried
    /// and `γ₁` of the result.
    fn minus(self, o: Ep) -> Ep {
        Ep::from_fn(|c| {
            let ((a, ea), (b, eb)) = (self.at(c), o.at(c));
            let v = a - b;
            (v, ea + eb + gamma(1) * v.abs())
        })
    }

    /// `g · self`, the factor within `γ_r` (relative to itself) of the
    /// exact one: `|g* x* − fl(g x)| ≤ (1 + γ_r)|g| e + γ_{r+1} |fl(g x)|`.
    fn times(self, g: f64, r: usize) -> Ep {
        Ep::from_fn(|c| {
            let (a, ea) = self.at(c);
            let v = g * a;
            (v, (1.0 + gamma(r)) * g.abs() * ea + gamma(r + 1) * v.abs())
        })
    }
}

/// The homogeneous point `((P − o) w, w)` of a control point `P` with
/// weight `w`, about `o`: two roundings a coordinate, the weight exact.
fn local_point(p: Vec3, w: f64, o: Vec3) -> Ep {
    let (pc, oc) = ([p.x, p.y, p.z], [o.x, o.y, o.z]);
    Ep::from_fn(|c| match (pc.get(c), oc.get(c)) {
        (Some(x), Some(y)) => {
            let v = (x - y) * w;
            (v, gamma(3) * v.abs())
        }
        _ => (w, 0.0),
    })
}

/// `h(b) − h(a)` of two control points in the homogeneous coordinates about
/// `o`, formed from the points as stored, not from their local
/// coordinates: `(P_b − P_a) w_b + (P_a − o)(w_b − w_a)`, each difference of
/// stored numbers one rounding relative to itself. With equal weights
/// (every polynomial face) the second term vanishes exactly and the
/// rounding is `γ₃` of the difference: a derivative formed from these
/// carries the rounding of the derivative, where one formed from rounded
/// local points would carry the rounding of the positions divided by the
/// knot span. With unequal weights the second term adds
/// `γ₄ |P_a − o| |w_b − w_a|`: of the positions' size, but times the
/// difference of the weights, which is itself a derivative's worth.
fn stored_difference(pa: Vec3, wa: f64, pb: Vec3, wb: f64, o: Vec3) -> Ep {
    let (a, b, oc) = ([pa.x, pa.y, pa.z], [pb.x, pb.y, pb.z], [o.x, o.y, o.z]);
    let dw = wb - wa;
    Ep::from_fn(|c| match (a.get(c), b.get(c), oc.get(c)) {
        (Some(a), Some(b), Some(o)) => {
            let t1 = (b - a) * wb;
            if dw == 0.0 {
                (t1, gamma(3) * t1.abs())
            } else {
                let t2 = (a - o) * dw;
                let v = t1 + t2;
                (
                    v,
                    gamma(3) * t1.abs() + gamma(4) * t2.abs() + gamma(1) * v.abs(),
                )
            }
        }
        _ => (dw, gamma(1) * dw.abs()),
    })
}

/// A face's control net as homogeneous coefficients with their rounding,
/// `pts[i][k]`, `i` across the line and `k` along it, with the knots and
/// degree each way: the net itself, or one of its derivative nets.
#[derive(Clone, Debug)]
struct Net {
    pts: Vec<Vec<Ep>>,
    across: (Vec<f64>, usize),
    along: (Vec<f64>, usize),
}

/// The derivative of a B-spline's control points in one direction, as
/// [`derivative`] forms it (`p (P_{k+1} − P_k) / (t_{k+p+1} − t_{k+1})` on
/// the knots without their first and last; a constant zero for degree 0),
/// with the differences given by `delta` (the two indices `(i, k)`).
fn net_derivative(
    net: &Net,
    along: bool,
    delta: impl Fn((usize, usize), (usize, usize)) -> Ep,
) -> Result<Net, NetworkError> {
    let (knots, p) = if along { &net.along } else { &net.across };
    let (ni, nk) = (net.pts.len(), net.pts.first().map_or(0, Vec::len));
    if *p == 0 {
        return Ok(Net {
            pts: vec![vec![EP_ZERO; nk]; ni],
            across: net.across.clone(),
            along: net.along.clone(),
        });
    }
    let factor = |k: usize| -> Result<f64, NetworkError> {
        let (Some(&lo), Some(&hi)) = (knots.get(k + 1), knots.get(k + p + 1)) else {
            return Err(internal());
        };
        Ok(if hi > lo { *p as f64 / (hi - lo) } else { 0.0 })
    };
    let (mi, mk) = if along {
        (ni, nk.saturating_sub(1))
    } else {
        (ni.saturating_sub(1), nk)
    };
    let mut pts = Vec::with_capacity(mi);
    for i in 0..mi {
        let mut row = Vec::with_capacity(mk);
        for k in 0..mk {
            let (a, b, f) = if along {
                ((i, k), (i, k + 1), factor(k)?)
            } else {
                ((i, k), (i + 1, k), factor(i)?)
            };
            // The factor: the knots' difference and the division, two
            // roundings, within `γ₂` of the exact factor and so within `γ₃`
            // relative to itself. A zero factor belongs to a basis function
            // that is zero everywhere: exactly nothing.
            row.push(if f == 0.0 {
                EP_ZERO
            } else {
                delta(a, b).times(f, 3)
            });
        }
        pts.push(row);
    }
    let trimmed = knots
        .get(1..knots.len().saturating_sub(1))
        .ok_or_else(internal)?
        .to_vec();
    let lowered = (trimmed, p - 1);
    Ok(if along {
        Net {
            pts,
            across: net.across.clone(),
            along: lowered,
        }
    } else {
        Net {
            pts,
            across: lowered,
            along: net.along.clone(),
        }
    })
}

/// The difference of two coefficients of a net already formed.
fn net_delta(net: &Net) -> impl Fn((usize, usize), (usize, usize)) -> Ep + '_ {
    move |(i, k), (j, l)| {
        let at = |i: usize, k: usize| net.pts.get(i).and_then(|r| r.get(k)).copied();
        match (at(i, k), at(j, l)) {
            (Some(a), Some(b)) => b.minus(a),
            _ => Ep {
                v: [f64::NAN; 4],
                e: [f64::INFINITY; 4],
            },
        }
    }
}

/// The net's line at `at` across: the spline along whose coefficients are
/// `Σᵢ Nᵢ(at) Pᵢₖ`, each with its rounding.
///
/// On a clamped border (`at` exactly the first or the last knot across)
/// one basis function is 1 and every other 0, exactly, so the line's
/// coefficients are the border row's own. Elsewhere the computed basis
/// values `N̂ᵢ` (Cox–de Boor: every quantity nonnegative inside the span,
/// at most six roundings a level, so `N̂ᵢ = Nᵢ (1 + θ)`, `|θ| ≤ γ_{6q}`) sum
/// with `m ≤ q + 1` terms: `|Σ Nᵢ P*ᵢ − fl(Σ N̂ᵢ P̂ᵢ)| ≤ (1 + γ_{6q}) Σ N̂ᵢ eᵢ +
/// (γ_{6q} + γ_m)(1 + γ_{6q}) Σ N̂ᵢ |P̂ᵢ|`, here taken with `γ_{7q+8}`.
fn net_line(net: &Net, at: f64) -> Result<(Raw, Vec<[f64; 4]>), NetworkError> {
    let (knots, q) = (&net.across.0, net.across.1);
    let n = net.pts.len();
    let nk = net.pts.first().map_or(0, Vec::len);
    // A border is clamped where its knot is there `q + 1` times.
    let clamped_at =
        |ks: &mut dyn Iterator<Item = &f64>| ks.take(q + 1).filter(|k| **k == at).count() == q + 1;
    let weights: Vec<(usize, f64)> = if clamped_at(&mut knots.iter()) {
        vec![(0, 1.0)]
    } else if clamped_at(&mut knots.iter().rev()) {
        vec![(n.checked_sub(1).ok_or_else(internal)?, 1.0)]
    } else {
        // The bound on the computed basis values holds where every
        // quantity of the recursion is nonnegative: `at` inside the
        // domain `[t_q, t_n]`.
        let (Some(&first), Some(&last)) = (knots.get(q), knots.get(n)) else {
            return Err(internal());
        };
        if !(first <= at && at <= last) {
            return refuse(format!(
                "the edge's line lies at {at}, outside its face's domain [{first}, {last}] across"
            ));
        }
        super::basis(knots, q, n, at)?
            .into_iter()
            .enumerate()
            .filter(|(_, b)| *b != 0.0)
            .collect()
    };
    let exact = weights.len() == 1 && weights.first().is_some_and(|(_, b)| *b == 1.0);
    let g = gamma(7 * q + 8);
    let mut points = Vec::with_capacity(nk);
    let mut errors = Vec::with_capacity(nk);
    for k in 0..nk {
        let mut v = [0.0; 4];
        let mut carried = [0.0; 4];
        let mut size = [0.0; 4];
        for &(i, b) in &weights {
            let c = net
                .pts
                .get(i)
                .and_then(|r| r.get(k))
                .copied()
                .ok_or_else(internal)?;
            for (((v, carried), size), (x, e)) in v
                .iter_mut()
                .zip(carried.iter_mut())
                .zip(size.iter_mut())
                .zip(c.v.iter().zip(&c.e))
            {
                *v += b * x;
                *carried += b * e;
                *size += b * x.abs();
            }
        }
        let mut e = carried;
        if !exact {
            for (e, size) in e.iter_mut().zip(size) {
                *e = (1.0 + g) * *e + g * size;
            }
        }
        points.push(v);
        errors.push(e);
    }
    Ok((
        Raw {
            degree: net.along.1,
            knots: net.along.0.clone(),
            points,
        },
        errors,
    ))
}

/// A row as it enters the measure: a spline along the face's own line (in
/// the face's own parameter, not cut), with the bound on each control
/// point's rounding coordinate by coordinate (`x w`, `y w`, `z w`, `w`).
type Row = (Raw, Vec<[f64; 4]>);

/// The six rows of a spline face along an edge, `X₀, X₀′, X₀″, X₁, X₁′,
/// X₂` (′ along the line, the index the order across), in the face's own
/// parameter along the line, and where on the line the edge lies.
struct Line {
    rows: [Row; 6],
    from: f64,
    to: f64,
    reversed: bool,
}

impl Line {
    /// The measure's parameter (0 to 1 along the edge) of a parameter of
    /// the line.
    fn to_edge(&self, u: f64) -> f64 {
        let width = self.to - self.from;
        if self.reversed {
            (self.to - u) / width
        } else {
            (u - self.from) / width
        }
    }

    /// The line's parameter of a parameter of the edge.
    fn of_edge(&self, s: f64) -> f64 {
        let width = self.to - self.from;
        if self.reversed {
            self.to - s * width
        } else {
            self.from + s * width
        }
    }
}

/// The sub-piece of a spline over `[x, y]`, inside its knot span `s`
/// (`t_s ≤ x < y ≤ t_{s+1}`), as Bézier coefficients: the blossom of the
/// span's `p + 1` control points at `(x, …, x, y, …, y)` (de Boor's
/// algorithm, `p` levels; Ramshaw). Every level is a combination
/// `(1 − α) a + α b` with the exact `α` in `[0, 1]` (the arguments lie in
/// the span, the knots around them), so what the points carried passes on
/// unchanged at most (a convex combination of it), and a level adds at
/// most `γ₁₀` of the largest coordinate: `α` is within `γ₃` (three
/// roundings), `1 − α` within `γ₃ + ε`, and the two products and their sum
/// add `γ₂`. Over `p` levels, with the coordinates growing by at most
/// `1 + γ₁₀` a level, that is `γ_{11p}` of the span's largest coordinate.
fn blossom_piece(
    raw: &Raw,
    err: &[[f64; 4]],
    s: usize,
    x: f64,
    y: f64,
) -> Result<(Vec<H>, [f64; 4]), NetworkError> {
    let p = raw.degree;
    let first = s.checked_sub(p).ok_or_else(internal)?;
    let span = raw.points.get(first..=s).ok_or_else(internal)?;
    let span_err = err.get(first..=s).ok_or_else(internal)?;
    let mut carried = [0.0_f64; 4];
    let mut size = [0.0_f64; 4];
    for (h, e) in span.iter().zip(span_err) {
        for (((carried, size), x), e) in carried.iter_mut().zip(size.iter_mut()).zip(h).zip(e) {
            *carried = carried.max(*e);
            *size = size.max(x.abs());
        }
    }
    let mut out = Vec::with_capacity(p + 1);
    for k in 0..=p {
        let mut d = span.to_vec();
        for level in 1..=p {
            let u = if level <= p - k { x } else { y };
            for j in (level..=p).rev() {
                let i = first + j;
                let (Some(&ti), Some(&th)) = (raw.knots.get(i), raw.knots.get(i + p + 1 - level))
                else {
                    return Err(internal());
                };
                let alpha = if th > ti { (u - ti) / (th - ti) } else { 0.0 };
                let (Some(&a), Some(&b)) = (d.get(j - 1), d.get(j)) else {
                    return Err(internal());
                };
                if let Some(slot) = d.get_mut(j) {
                    *slot = super::h_add(a, b, 1.0 - alpha, alpha);
                }
            }
        }
        out.push(d.get(p).copied().ok_or_else(internal)?);
    }
    let mut e = carried;
    for (e, size) in e.iter_mut().zip(size) {
        *e += gamma(11 * p) * size;
    }
    Ok((out, e))
}

/// A row cut into Bézier pieces between the edge's breaks (parameters of
/// the edge, 0 to 1, ascending), run the edge's way: each piece formed
/// straight from the knot span of the row it lies in ([`blossom_piece`]),
/// so its rounding is that of one blossom, however many breaks the span
/// holds. Running the edge's way reverses the pieces and their
/// coefficients, exactly; the parameter along each face is its own, up to
/// an affine change per piece (the gaps are invariant under it: each is a
/// ratio of the same order in the derivatives along on both sides, and
/// the second fundamental form is taken on the face's own derivatives),
/// so the rows are not reparametrised and no knot is rounded. The points
/// compared are those at the same local parameter of a piece on both
/// faces; how far apart they are is the position gap.
fn row_pieces(row: &Row, line: &Line, breaks: &[f64]) -> Result<Vec<P4>, NetworkError> {
    let (raw, err) = row;
    let (lo, hi) = raw.domain()?;
    let snap = 2.0 * SAME_KNOT * (hi - lo);
    let mut distinct = raw.knots.clone();
    distinct.dedup();
    let near_knot = |u: f64| {
        distinct
            .iter()
            .copied()
            .find(|k| (k - u).abs() <= snap)
            .unwrap_or(u)
    };
    let mut cuts: Vec<f64> = breaks.iter().map(|s| near_knot(line.of_edge(*s))).collect();
    if line.reversed {
        cuts.reverse();
    }
    let mut at = vec![near_knot(line.from)];
    at.extend(cuts);
    at.push(near_knot(line.to));
    let mut pieces = Vec::with_capacity(at.len().saturating_sub(1));
    for w in at.windows(2) {
        let [x, y] = w else { return Err(internal()) };
        if x.partial_cmp(y) != Some(std::cmp::Ordering::Less) {
            return Err(internal());
        }
        // The knot span `[t_s, t_{s+1})` holding `x`; it must hold `y` too.
        let s = raw
            .knots
            .iter()
            .rposition(|k| k <= x)
            .ok_or_else(internal)?
            .min(raw.points.len().saturating_sub(1));
        let end = raw.knots.get(s + 1).copied().ok_or_else(internal)?;
        // A piece that reaches past its span would leave `[end, y]` of the
        // face unmeasured if cut at `end`: that happens only where two
        // distinct knots of the face lie within rounding of each other
        // (the breaks are one there), so it is refused, not truncated.
        if *y > end {
            return refuse(format!(
                "a face has two distinct knots within {snap:e} of each other along the edge (near {end}), closer than the measure can tell apart; the seam is not measured"
            ));
        }
        let (h, e) = blossom_piece(raw, err, s, *x, *y)?;
        pieces.push((h, e));
    }
    if line.reversed {
        pieces.reverse();
        for (h, _) in &mut pieces {
            h.reverse();
        }
    }
    Ok(pieces
        .into_iter()
        .map(|(piece, e)| {
            let comp = |k: usize| Poly {
                c: piece
                    .iter()
                    .map(|h| h.get(k).copied().unwrap_or(0.0))
                    .collect(),
                e: e.get(k).copied().unwrap_or(f64::INFINITY),
            };
            [comp(0), comp(1), comp(2), comp(3)]
        })
        .collect())
}

/// One piece of the edge with every row on it.
#[derive(Clone, Debug)]
struct Piece {
    a: f64,
    b: f64,
    rows: Vec<P4>,
    depth: usize,
}

impl Piece {
    fn halves(&self) -> (Piece, Piece) {
        let mut left = Vec::with_capacity(self.rows.len());
        let mut right = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            let hs: Vec<(Poly, Poly)> = row.iter().map(Poly::halves).collect();
            let get = |k: usize, first: bool| {
                hs.get(k)
                    .map(|(l, r)| if first { l.clone() } else { r.clone() })
                    .unwrap_or_else(|| Poly::konst(0.0))
            };
            left.push([get(0, true), get(1, true), get(2, true), get(3, true)]);
            right.push([get(0, false), get(1, false), get(2, false), get(3, false)]);
        }
        let m = 0.5 * (self.a + self.b);
        (
            Piece {
                a: self.a,
                b: m,
                rows: left,
                depth: self.depth + 1,
            },
            Piece {
                a: m,
                b: self.b,
                rows: right,
                depth: self.depth + 1,
            },
        )
    }

    /// The rows at a local parameter, as constants.
    fn at(&self, t: f64) -> Vec<P4> {
        self.rows
            .iter()
            .map(|r| [r[0].at(t), r[1].at(t), r[2].at(t), r[3].at(t)])
            .collect()
    }
}

/// The gaps, in order: position, angle, and the three of the second
/// order (curvature across, twist, curvature along).
const GAPS: usize = 5;

/// Upper bounds of the gaps over rows (as polynomials of a piece), and,
/// for constant rows, lower bounds of their values.
struct Gaps {
    hi: [f64; GAPS],
    lo: [f64; GAPS],
}

fn ratio(num_hi: f64, den_lo: f64) -> f64 {
    if num_hi.is_nan() || den_lo.is_nan() {
        return f64::INFINITY;
    }
    if num_hi <= 0.0 {
        return 0.0;
    }
    if den_lo <= 0.0 {
        return f64::INFINITY;
    }
    num_hi / den_lo
}

fn add3(a: &V3, b: &V3) -> V3 {
    [a[0].add(&b[0]), a[1].add(&b[1]), a[2].add(&b[2])]
}

/// `∂x∂y (G / w)` times `w³`, `G = L·X` (from its derivatives).
#[allow(clippy::too_many_arguments)]
fn plane_second(
    gxy: &Poly,
    gx: &Poly,
    gy: &Poly,
    g: &Poly,
    w: &Poly,
    wx: &Poly,
    wy: &Poly,
    wxy: &Poly,
) -> Poly {
    gxy.mul(&w.mul(w))
        .sub(&w.mul(&gx.mul(wy).add(&gy.mul(wx))))
        .sub(&w.mul(wxy).mul(g))
        .add(&wx.mul(wy).mul(g).scale(2.0))
}

/// `∂x∂y (H / w²)` times `w⁴`, `H = XᵀQX` (from its derivatives).
#[allow(clippy::too_many_arguments)]
fn quadric_second(
    hxy: &Poly,
    hx: &Poly,
    hy: &Poly,
    h: &Poly,
    w: &Poly,
    wx: &Poly,
    wy: &Poly,
    wxy: &Poly,
) -> Poly {
    hxy.mul(&w.mul(w))
        .sub(&w.mul(&hx.mul(wy).add(&hy.mul(wx))).scale(2.0))
        .add(&wx.mul(wy).scale(6.0).sub(&w.mul(wxy).scale(2.0)).mul(h))
}

fn gaps(rows: &[P4], other: &Other) -> Result<Gaps, NetworkError> {
    let row = |k: usize| rows.get(k).cloned().ok_or_else(internal);
    let (x0, x0d, x0dd, x1, x1d, x2) = (row(0)?, row(1)?, row(2)?, row(3)?, row(4)?, row(5)?);
    let (w0, w0d, w0dd, w1, w1d, w2) = (&x0[3], &x0d[3], &x0dd[3], &x1[3], &x1d[3], &x2[3]);
    let (p0, p0d, p0dd) = (xyz4(&x0), xyz4(&x0d), xyz4(&x0dd));
    let (p1, p1d, p2) = (xyz4(&x1), xyz4(&x1d), xyz4(&x2));
    // S_u, S_v times w².
    let v0d = minus(&times(&p0d, w0), &times(&p0, w0d));
    let v1 = minus(&times(&p1, w0), &times(&p0, w1));
    let w = cross(&v0d, &v1);
    let (nu_lo, nu_hi) = (norm_lo(&v0d), norm_value_hi(&v0d));
    let (nv_lo, nv_hi) = (norm_lo(&v1), norm_value_hi(&v1));
    let (w_lo, w_hi) = (norm_lo(&w), norm_value_hi(&w));
    // A lower bound on a weight is of no use below zero.
    let (w0_lo, w0_hi) = (w0.lo().max(0.0), w0.hi());
    // `|X| |Y|` over `w⁴` for the pairs (v, v), (u, v), (u, u).
    let den_lo = [nv_lo * nv_lo, nu_lo * nv_lo, nu_lo * nu_lo];
    let den_hi = [nv_hi * nv_hi, nu_hi * nv_hi, nu_hi * nu_hi];
    let mut hi = [0.0; GAPS];
    let mut lo = [0.0; GAPS];
    // The second-order numerators and the factor of their denominators
    // beyond `|X| |Y|` (as lower and upper bounds).
    let (nums, extra_lo, extra_hi): ([Poly; 3], f64, f64) = match other {
        Other::Plane { l } => {
            let lx = |x: &P4| {
                x[0].scale(l[0])
                    .add(&x[1].scale(l[1]))
                    .add(&x[2].scale(l[2]))
                    .add(&x[3].scale(l[3]))
            };
            let (g0, g0d, g0dd) = (lx(&x0), lx(&x0d), lx(&x0dd));
            let (g1, g1d, g2) = (lx(&x1), lx(&x1d), lx(&x2));
            hi[0] = ratio(abs_hi(&g0), w0_lo);
            lo[0] = abs_value_lo(&g0) / w0_hi;
            let n = konst3(Vec3::new(l[0], l[1], l[2]));
            let z = cross(&w, &n);
            hi[1] = ratio(norm_hi(&z), w_lo);
            lo[1] = norm_value_lo(&z) / w_hi;
            let vv = plane_second(&g2, &g1, &g1, &g0, w0, w1, w1, w2);
            let uv = plane_second(&g1d, &g0d, &g1, &g0, w0, w0d, w1, w1d);
            let uu = plane_second(&g0dd, &g0d, &g0d, &g0, w0, w0d, w0d, w0dd);
            ([vv.mul(w0), uv.mul(w0), uu.mul(w0)], 1.0, 1.0)
        }
        Other::Round { .. } | Other::Cone { .. } => {
            let q = match other {
                Other::Round { q, .. } | Other::Cone { q, .. } => q,
                _ => return Err(internal()),
            };
            let (qx0, qx0d, qx1) = (apply(q, &x0), apply(q, &x0d), apply(q, &x1));
            let hh = dot4(&x0, &qx0);
            let h_u = dot4(&x0d, &qx0).scale(2.0);
            let h_v = dot4(&x1, &qx0).scale(2.0);
            let h_vv = dot4(&x1, &qx1).add(&dot4(&x2, &qx0)).scale(2.0);
            let h_uv = dot4(&x0d, &qx1).add(&dot4(&x1d, &qx0)).scale(2.0);
            let h_uu = dot4(&x0d, &qx0d).add(&dot4(&x0dd, &qx0)).scale(2.0);
            // Position.
            let (den_lo, den_hi) = match other {
                Other::Round {
                    centre,
                    axis,
                    radius,
                    ..
                } => {
                    let y = minus(&p0, &times(&konst3(*centre), w0));
                    let py = match axis {
                        Some(d) => {
                            let along = dot(&y, &konst3(*d));
                            minus(&y, &times(&konst3(*d), &along))
                        }
                        None => y,
                    };
                    (
                        w0_lo * (norm_lo(&py) + radius * w0_lo),
                        w0_hi * (norm_value_hi(&py) + radius * w0_hi),
                    )
                }
                Other::Cone {
                    apex,
                    axis,
                    tan,
                    cos,
                    ..
                } => {
                    let y = minus(&p0, &times(&konst3(*apex), w0));
                    let h = dot(&y, &konst3(*axis));
                    let py = minus(&y, &times(&konst3(*axis), &h));
                    let cos = cos.max(f64::MIN_POSITIVE);
                    (
                        w0_lo * (norm_lo(&py) + tan * abs_lo(&h)) / cos,
                        w0_hi * (norm_value_hi(&py) + tan * abs_hi(&h)) / cos,
                    )
                }
                _ => return Err(internal()),
            };
            hi[0] = ratio(abs_hi(&hh), den_lo);
            lo[0] = abs_value_lo(&hh) / den_hi;
            // Angle.
            let g = xyz4(&qx0);
            let z = cross(&w, &g);
            let (g_lo, g_hi) = (norm_lo(&g), norm_value_hi(&g));
            hi[1] = ratio(norm_hi(&z), w_lo * g_lo);
            lo[1] = norm_value_lo(&z) / (w_hi * g_hi);
            let vv = quadric_second(&h_vv, &h_v, &h_v, &hh, w0, w1, w1, w2);
            let uv = quadric_second(&h_uv, &h_u, &h_v, &hh, w0, w0d, w1, w1d);
            let uu = quadric_second(&h_uu, &h_u, &h_u, &hh, w0, w0d, w0d, w0dd);
            ([vv.mul(w0), uv.mul(w0), uu.mul(w0)], 2.0 * g_lo, 2.0 * g_hi)
        }
        Other::Spline => {
            let r = |k: usize| rows.get(6 + k).cloned().ok_or_else(internal);
            let (y0, y0d, y0dd, y1, y1d, y2) = (r(0)?, r(1)?, r(2)?, r(3)?, r(4)?, r(5)?);
            let (om, om_d, om1) = (&y0[3], &y0d[3], &y1[3]);
            let (q0, q0d, q1) = (xyz4(&y0), xyz4(&y0d), xyz4(&y1));
            // Position.
            let e0 = minus(&times(&p0, om), &times(&q0, w0));
            let (om_lo, om_hi) = (om.lo().max(0.0), om.hi());
            hi[0] = ratio(norm_hi(&e0), w0_lo * om_lo);
            lo[0] = norm_value_lo(&e0) / (w0_hi * om_hi);
            // Angle.
            let ub = minus(&times(&q0d, om), &times(&q0, om_d));
            let vb = minus(&times(&q1, om), &times(&q0, om1));
            let wb = cross(&ub, &vb);
            let z = cross(&w, &wb);
            let (wb_lo, wb_hi) = (norm_lo(&wb), norm_value_hi(&wb));
            hi[1] = ratio(norm_hi(&z), w_lo * wb_lo);
            lo[1] = norm_value_lo(&z) / (w_hi * wb_hi);
            // The second order: `n_b·S_xy − II_b(X, Y)`, `X`, `Y` put into
            // b's tangent plane by Cramer's rule in `(U_b, V_b)`.
            // S_vv, S_uv, S_uu times w³.
            let a_vv = minus(
                &times(&minus(&times(&p2, w0), &times(&p0, w2)), w0),
                &times(&v1, &w1.scale(2.0)),
            );
            let v1u = minus(
                &add3(&times(&p1d, w0), &times(&p1, w0d)),
                &add3(&times(&p0d, w1), &times(&p0, w1d)),
            );
            let a_uv = minus(&times(&v1u, w0), &times(&v1, &w0d.scale(2.0)));
            let a_uu = minus(
                &times(&minus(&times(&p0dd, w0), &times(&p0, w0dd)), w0),
                &times(&v0d, &w0d.scale(2.0)),
            );
            let m = dot(&wb, &wb);
            let coeffs = |x: &V3| (dot(&cross(x, &vb), &wb), dot(&cross(&ub, x), &wb));
            let (au, bu) = coeffs(&v0d);
            let (av, bv) = coeffs(&v1);
            let duu = det4(&y0, &y0d, &y1, &y0dd);
            let dut = det4(&y0, &y0d, &y1, &y1d);
            let dtt = det4(&y0, &y0d, &y1, &y2);
            let form = |ax: &Poly, bx: &Poly, ay: &Poly, by: &Poly| {
                ax.mul(ay)
                    .mul(&duu)
                    .add(&ax.mul(by).add(&bx.mul(ay)).mul(&dut))
                    .add(&bx.mul(by).mul(&dtt))
            };
            let om2 = om.mul(om);
            let om4 = om2.mul(&om2);
            let m2 = m.mul(&m);
            let num = |a: &V3, q: Poly| dot(&wb, a).mul(w0).mul(&m2).add(&om4.mul(&q));
            let nums = [
                num(&a_vv, form(&av, &bv, &av, &bv)),
                num(&a_uv, form(&au, &bu, &av, &bv)),
                num(&a_uu, form(&au, &bu, &au, &bu)),
            ];
            let (m_lo, m_hi) = (m.lo().max(0.0), m.hi());
            (nums, m_lo * m_lo * wb_lo, m_hi * m_hi * wb_hi)
        }
    };
    for (k, num) in nums.iter().enumerate() {
        let (geo_lo, geo_hi) = (
            den_lo.get(k).copied().unwrap_or(0.0),
            den_hi.get(k).copied().unwrap_or(f64::INFINITY),
        );
        if let (Some(h), Some(l)) = (hi.get_mut(2 + k), lo.get_mut(2 + k)) {
            *h = ratio(abs_hi(num), extra_lo * geo_lo);
            *l = abs_value_lo(num) / (extra_hi * geo_hi);
        }
    }
    for x in &mut lo {
        if !x.is_finite() {
            *x = 0.0;
        }
    }
    Ok(Gaps { hi, lo })
}

// ---------------------------------------------------------------------------
// The measure.

/// How the faces `a` and `b` meet along `edge` (see the module docs). One
/// of them must be a spline face the edge runs along a parameter line of;
/// the other a plane, cylinder, cone or sphere, or a spline face the edge
/// runs along a parameter line of in the same parametrisation.
pub fn continuity(
    a: &Surface,
    b: &Surface,
    edge: &NurbsCurve,
    tolerance: f64,
    angular: f64,
) -> Result<Continuity, NetworkError> {
    if !(tolerance.is_finite() && tolerance > 0.0 && angular.is_finite() && angular > 0.0) {
        return Err(crate::GeomError::InvalidParameter(
            "continuity is measured with positive, finite tolerances",
        )
        .into());
    }
    let spline = |s: &Surface| -> Result<Option<NurbsSurface>, NetworkError> {
        Ok(match s {
            Surface::Nurbs(n) => Some(n.clone()),
            Surface::Bezier(z) => Some(NurbsSurface::from_bezier(z)?),
            _ => None,
        })
    };
    let (sa, other_face) = match (spline(a)?, spline(b)?) {
        (Some(x), _) => (x, b),
        (None, Some(y)) => (y, a),
        (None, None) => {
            return refuse(
                "continuity is measured where one face is a spline the edge runs along a parameter line of; two analytic faces are not compared yet (owed: an analytic face written across the edge)".to_string(),
            );
        }
    };
    let track = Track::new(edge, tolerance)?;
    if track.point {
        return refuse("the edge is a point: there is no seam to measure".to_string());
    }
    // The middle of the edge's control points' box: no point of the edge
    // is farther from it than half the box's diagonal.
    let (lo, hi) = edge.control_points().iter().fold(
        (
            Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
            Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
        ),
        |(lo, hi), p| {
            (
                Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z)),
                Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z)),
            )
        },
    );
    let origin = (lo + hi) * 0.5;
    if !origin.is_finite() {
        return Err(internal());
    }
    let Some(xs) = spline_rows(&sa, edge, tolerance, origin)? else {
        return refuse(
            "the edge does not run along a parameter line of its spline face, so the face cannot be written along it exactly (owed: a measure at the feet with a Lipschitz bound)".to_string(),
        );
    };
    let (other, ys) = match other_face {
        Surface::Nurbs(_) | Surface::Bezier(_) => {
            let sb = spline(other_face)?.ok_or_else(internal)?;
            let Some(ys) = spline_rows(&sb, edge, tolerance, origin)? else {
                return refuse(
                    "the edge does not run along a parameter line of the other spline face, so the two cannot be compared at corresponding points (owed: a measure at the feet with a Lipschitz bound)".to_string(),
                );
            };
            (Other::Spline, Some(ys))
        }
        Surface::Torus(_) => {
            return refuse(
                "continuity to a torus is not measured yet (a quartic; owed: its implicit form)"
                    .to_string(),
            );
        }
        analytic => (
            implicit_of(analytic, origin).ok_or_else(internal)?.other(),
            None,
        ),
    };
    measure(xs, ys, other, origin, tolerance, angular)
}

/// The rows of a spline face along `edge` ([`Line`]): its iso-curve there
/// `X₀`, its first and second homogeneous derivatives across `X₁`, `X₂`,
/// and the derivatives along `X₀′`, `X₀″`, `X₁′`, about `origin`, each
/// control point with the bound on its rounding; `None` where the edge is
/// not a parameter line of the face.
///
/// The line is found on the face written about `origin` (the projections
/// of the edge; `iso_line` puts it onto the border where it lies within
/// `SAME_KNOT` of it). What is bounded here is the rounding of that line's
/// rows; how far the edge lies from the line is the position gap, which
/// the measure bounds and holds to the tolerance in its own right.
///
/// **Each row is formed from the stored net, by differences first**
/// (2026-10-31, the tank's long seams). A row is `Σᵢ Nᵢ⁽ʲ⁾(at) Pᵢₖ⁽ᵈ⁾`:
/// the net differentiated `j` times across and `d` times along (the
/// B-spline derivative nets, `p (P_{k+1} − P_k) / Δ`), then combined at the
/// line with the basis of degree `q − j` on the knots across trimmed `j`
/// times, whose values are nonnegative and sum to 1 ([`net_line`]). The
/// first difference each way is taken of the points as stored
/// ([`stored_difference`]): one rounding of a difference of two stored
/// numbers is relative to the difference, so with equal weights a
/// derivative carries the rounding of the derivative's own size (with
/// unequal weights, a further `γ₄ |P_a − o| |w_b − w_a|`). Formed the other way round (the
/// row first, of points rounded about `origin`, then differentiated
/// along) every derivative multiplied the rounding of the positions by
/// `2p / Δ` (`Δ` the shortest knot span along): on the tank's 0.69 m seam,
/// whose side has spans of 0.022 at degree 10 and points 330 mm from the
/// middle of the edge, `X₀″` carried 4.6e-6 against a size of 1.4e4 (a
/// relative 3e-10), and the curvature along was bounded at 1.57e-11 per
/// mm against a threshold of 1.19e-12. Every step's rounding is carried
/// coefficient by coefficient ([`Ep`]): a difference adds `γ₁` of its
/// result, a factor `p / Δ` (two roundings) `γ₃`, the combination at the
/// line `γ_{7q+8}` of `Σ N̂ᵢ |Pᵢ|` (none on a clamped border, where one basis
/// function is 1 and the others 0 exactly), and the cut into pieces its
/// own ([`row_pieces`]). Equal weights make every difference of weights
/// exactly zero, with no rounding.
fn spline_rows(
    surface: &NurbsSurface,
    edge: &NurbsCurve,
    tolerance: f64,
    origin: Vec3,
) -> Result<Option<Line>, NetworkError> {
    let local = NurbsSurface::new(
        surface.degrees().0,
        surface.degrees().1,
        surface.knots_u().to_vec(),
        surface.knots_v().to_vec(),
        surface.counts().0,
        surface.counts().1,
        surface
            .control_points()
            .iter()
            .map(|p| *p - origin)
            .collect(),
        surface.weights().to_vec(),
    )?;
    let local_edge = NurbsCurve::new(
        edge.degree(),
        edge.knots().to_vec(),
        edge.control_points().iter().map(|p| *p - origin).collect(),
        edge.weights().to_vec(),
    )?;
    let track = Track::new(&local_edge, tolerance)?;
    let Some(line) = super::iso_line(&local, &track, tolerance)? else {
        return Ok(None);
    };
    // Along v the rows across are derivatives in u.
    let along_v = line.along_v;
    let (pu, pv) = surface.degrees();
    let (nu, nv) = surface.counts();
    let (ku, kv) = (surface.knots_u().to_vec(), surface.knots_v().to_vec());
    let (ni, nk, across, along) = if along_v {
        (nu, nv, (ku, pu), (kv, pv))
    } else {
        (nv, nu, (kv, pv), (ku, pu))
    };
    let (points, weights) = (surface.control_points(), surface.weights());
    let stored = |i: usize, k: usize| -> Option<(Vec3, f64)> {
        let index = if along_v { i * nv + k } else { k * nv + i };
        Some((*points.get(index)?, *weights.get(index)?))
    };
    let mut pts = Vec::with_capacity(ni);
    for i in 0..ni {
        let mut row = Vec::with_capacity(nk);
        for k in 0..nk {
            let (p, w) = stored(i, k).ok_or_else(internal)?;
            row.push(local_point(p, w, origin));
        }
        pts.push(row);
    }
    let base = Net { pts, across, along };
    let from_stored =
        |(i, k): (usize, usize), (j, l): (usize, usize)| match (stored(i, k), stored(j, l)) {
            (Some((pa, wa)), Some((pb, wb))) => stored_difference(pa, wa, pb, wb, origin),
            _ => Ep {
                v: [f64::NAN; 4],
                e: [f64::INFINITY; 4],
            },
        };
    let d_along = net_derivative(&base, true, from_stored)?;
    let d_across = net_derivative(&base, false, from_stored)?;
    let d_along2 = net_derivative(&d_along, true, net_delta(&d_along))?;
    let d_mixed = net_derivative(&d_across, true, net_delta(&d_across))?;
    let d_across2 = net_derivative(&d_across, false, net_delta(&d_across))?;
    let at = line.at;
    Ok(Some(Line {
        rows: [
            net_line(&base, at)?,
            net_line(&d_along, at)?,
            net_line(&d_along2, at)?,
            net_line(&d_across, at)?,
            net_line(&d_mixed, at)?,
            net_line(&d_across2, at)?,
        ],
        from: line.from,
        to: line.to,
        reversed: line.reversed,
    }))
}

/// Rows written about `origin` with their allowances: the face `a`'s, and
/// a spline face `b`'s where `other` is one.
fn measure(
    xs: Line,
    ys: Option<Line>,
    other: Other,
    origin: Vec3,
    tolerance: f64,
    angular: f64,
) -> Result<Continuity, NetworkError> {
    // The length of the control polygon of the line cut to the edge.
    let cut = super::piece_raw(xs.rows[0].0.clone(), xs.from, xs.to)?;
    let length = cut
        .points
        .windows(2)
        .map(|w| match w {
            [p, q] => match (point(*p), point(*q)) {
                (Some(p), Some(q)) => p.distance(q),
                _ => f64::INFINITY,
            },
            _ => 0.0,
        })
        .sum::<f64>();
    if !(length.is_finite() && length > 0.0) {
        return refuse(
            "the edge's face has no length along it, or its weights do not stay positive"
                .to_string(),
        );
    }
    let thr = thresholds(tolerance, angular, length);
    let limit = [
        thr[0],
        angular.min(std::f64::consts::FRAC_PI_2).psin(),
        thr[2],
        thr[2],
        thr[2],
    ];
    let p = xs.rows.iter().map(|(r, _)| r.degree).max().unwrap_or(0);
    let spline_degree = ys
        .as_ref()
        .map(|y| y.rows.iter().map(|(r, _)| r.degree).max().unwrap_or(0));
    let degree = match spline_degree {
        Some(q) => 4 * p + 20 * q,
        None => 5 * p,
    };
    if degree > MAX_SEAM_DEGREE {
        return refuse(format!(
            "the faces are of too high a degree along the edge to bound how they meet (the bounds would be polynomials of degree {degree}, the most is {MAX_SEAM_DEGREE})"
        ));
    }
    // The pieces between every row's knots inside the edge, as parameters
    // of the edge (0 to 1).
    let lines: Vec<&Line> = std::iter::once(&xs).chain(ys.as_ref()).collect();
    let mut breaks: Vec<f64> = Vec::new();
    for line in &lines {
        for (raw, _) in &line.rows {
            breaks.extend(raw.interior().into_iter().map(|u| line.to_edge(u)));
        }
    }
    breaks.retain(|s| *s > SAME_KNOT && *s < 1.0 - SAME_KNOT);
    breaks.sort_by(f64::total_cmp);
    breaks.dedup_by(|x, y| (*x - *y).abs() <= SAME_KNOT);
    let mut bounds = vec![0.0];
    bounds.extend(breaks.iter().copied());
    bounds.push(1.0);
    // The work, counted before any of it is done: each bound of a piece
    // costs about the square of the polynomials' degree (their products),
    // each piece looked at in 5 or 3 places as well; cutting the rows into
    // pieces costs a blossom of `p + 1` coefficients, each `p²` levels'
    // worth, per row and piece.
    let cost = (degree as u64 + 1).saturating_pow(2);
    let pieces = bounds.len() as u64 - 1;
    let rows_count = lines.iter().map(|l| l.rows.len() as u64).sum::<u64>();
    let cutting = rows_count
        .saturating_mul(pieces)
        .saturating_mul((p.max(spline_degree.unwrap_or(0)) as u64 + 1).saturating_pow(3));
    let first = cost
        .saturating_mul(6)
        .saturating_mul(bounds.len() as u64)
        .saturating_add(cutting);
    if first > MAX_SEAM_WORK {
        return refuse(format!(
            "the edge's faces have too many knot spans along it to bound how they meet ({pieces} pieces of degree {degree})"
        ));
    }
    let mut per_row: Vec<Vec<P4>> = Vec::with_capacity(12);
    for line in &lines {
        for row in &line.rows {
            per_row.push(row_pieces(row, line, &breaks)?);
        }
    }
    let mut spent = first;
    let mut exhausted = false;
    let mut work: Vec<(Piece, [f64; GAPS])> = Vec::with_capacity(bounds.len());
    for (k, w) in bounds.windows(2).enumerate() {
        let [a, b] = w else { return Err(internal()) };
        let rows: Vec<P4> = per_row
            .iter()
            .map(|r| r.get(k).cloned().ok_or_else(internal))
            .collect::<Result<_, _>>()?;
        let piece = Piece {
            a: *a,
            b: *b,
            rows,
            depth: 0,
        };
        let hi = gaps(&piece.rows, &other)?.hi;
        work.push((piece, hi));
    }
    let mut found = [(0.0_f64, 0.0_f64, origin); GAPS];
    let look = |piece: &Piece,
                ts: &[f64],
                found: &mut [(f64, f64, Vec3); GAPS]|
     -> Result<(), NetworkError> {
        for &t in ts {
            let at = piece.at(t);
            let lo = gaps(&at, &other)?.lo;
            let place = at
                .first()
                .map(|x| {
                    let w = x[3].c.first().copied().unwrap_or(0.0);
                    let c =
                        |k: usize| x.get(k).and_then(|p| p.c.first()).copied().unwrap_or(0.0) / w;
                    Vec3::new(c(0), c(1), c(2)) + origin
                })
                .unwrap_or(origin);
            for (slot, value) in found.iter_mut().zip(lo) {
                if value > slot.0 {
                    *slot = (value, piece.a + (piece.b - piece.a) * t, place);
                }
            }
        }
        Ok(())
    };
    for (piece, _) in &work {
        look(piece, &[0.0, 0.25, 0.5, 0.75, 1.0], &mut found)?;
    }
    loop {
        let decided: Vec<bool> = (0..GAPS)
            .map(|q| {
                let lim = limit.get(q).copied().unwrap_or(0.0);
                let all_within = work
                    .iter()
                    .all(|(_, hi)| hi.get(q).is_some_and(|h| *h <= lim));
                all_within || found.get(q).is_some_and(|f| f.0 > lim)
            })
            .collect();
        if decided.iter().all(|d| *d) {
            break;
        }
        if work.len() >= MAX_SEAM_PIECES {
            exhausted = true;
            break;
        }
        let mut next: Vec<(Piece, [f64; GAPS])> = Vec::with_capacity(work.len() * 2);
        let mut split = false;
        let total = work.len();
        for (index, (piece, hi)) in work.into_iter().enumerate() {
            let open = (0..GAPS).any(|q| {
                !decided.get(q).copied().unwrap_or(true)
                    && hi.get(q).copied().unwrap_or(0.0) > limit.get(q).copied().unwrap_or(0.0)
            });
            let room = next.len() + (total - index) < MAX_SEAM_PIECES;
            let affordable = spent.saturating_add(cost.saturating_mul(8)) <= MAX_SEAM_WORK;
            if open && !affordable {
                exhausted = true;
            }
            if open && room && affordable && piece.depth < 52 {
                spent = spent.saturating_add(cost.saturating_mul(8));
                let (l, r) = piece.halves();
                for half in [l, r] {
                    look(&half, &[0.25, 0.5, 0.75], &mut found)?;
                    let h = gaps(&half.rows, &other)?.hi;
                    next.push((half, h));
                }
                split = true;
            } else {
                next.push((piece, hi));
            }
        }
        work = next;
        if !split {
            break;
        }
    }
    let bound = |q: usize| {
        work.iter()
            .map(|(_, hi)| hi.get(q).copied().unwrap_or(f64::INFINITY))
            .fold(0.0_f64, f64::max)
    };
    let gap = |q: usize, as_angle: bool| {
        let (f, param, at) = found.get(q).copied().unwrap_or((0.0, 0.0, origin));
        let b = bound(q);
        let conv = |x: f64| {
            if as_angle {
                x.clamp(0.0, 1.0).pasin()
            } else {
                x
            }
        };
        Gap {
            bound: conv(b),
            found: conv(f),
            param,
            at,
            threshold: thr.get(q.min(2)).copied().unwrap_or(0.0),
        }
    };
    let (position, angle) = (gap(0, false), gap(1, true));
    let (curvature, twist, along) = (gap(2, false), gap(3, false), gap(4, false));
    let order = if !position.within() {
        Order::None
    } else if !angle.within() {
        Order::G0
    } else if !(curvature.within() && twist.within() && along.within()) {
        Order::G1
    } else {
        Order::G2
    };
    Ok(Continuity {
        order,
        position,
        angle,
        curvature,
        twist,
        along,
        length,
        pieces: work.len(),
        exhausted,
    })
}

/// The measure in words, for a refusal.
pub fn describe(c: &Continuity) -> String {
    let part = |what: &str, g: &Gap, unit: &str| {
        let verdict = if g.within() {
            "within"
        } else if g.beyond() {
            "beyond"
        } else {
            "not proven within"
        };
        format!(
            "{what} up to {:.3e}{unit} (found {:.3e}{unit} at {:.6} along the edge), {verdict} the threshold {:.3e}{unit}",
            g.bound, g.found, g.param, g.threshold
        )
    };
    let spent = if c.exhausted {
        format!(
            "; the work allowed ({} pieces, {MAX_SEAM_WORK} units) was spent before every gap was decided, so an order not proven is not reached",
            MAX_SEAM_PIECES
        )
    } else {
        String::new()
    };
    format!(
        "{}; {}; {}; {}; {}{spent}",
        part("position", &c.position, ""),
        part("angle", &c.angle, " rad"),
        part("curvature across", &c.curvature, " per unit"),
        part("twist", &c.twist, " per unit"),
        part("curvature along", &c.along, " per unit")
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::network::iso_curve;

    // Double-double arithmetic (Dekker, Knuth): a reference for the rows
    // far more exact than the rows themselves.
    type Dd = (f64, f64);

    fn two_sum(a: f64, b: f64) -> Dd {
        let s = a + b;
        let bb = s - a;
        (s, (a - (s - bb)) + (b - bb))
    }

    fn add(x: Dd, y: Dd) -> Dd {
        let (s, e) = two_sum(x.0, y.0);
        two_sum(s, e + x.1 + y.1)
    }

    fn neg(x: Dd) -> Dd {
        (-x.0, -x.1)
    }

    fn mul(x: Dd, b: f64) -> Dd {
        let p = x.0 * b;
        let e = x.0.pmul_add(b, -p);
        two_sum(p, e + x.1 * b)
    }

    fn div(x: Dd, b: f64) -> Dd {
        let q = x.0 / b;
        let r = add(x, neg(mul((q, 0.0), b)));
        two_sum(q, r.0 / b)
    }

    /// A patch of degree 2 along u and 3 across, with an inner knot at a
    /// third (so its slopes across do not come out exact), a unit or so
    /// across, about a point a million and more from the origin.
    fn far_patch() -> (NurbsSurface, Vec3) {
        let o = Vec3::new(1_234_567.891, -2_345_678.912, 3_456_789.123);
        let mut points = Vec::new();
        for i in 0..3 {
            for j in 0..5 {
                let (i, j) = (f64::from(i), f64::from(j));
                points.push(
                    o + Vec3::new(
                        0.37 * i + 0.11 * j * j,
                        0.29 * j + 0.07 * i,
                        0.13 * i * j + 0.05 * j,
                    ),
                );
            }
        }
        let third = 1.0 / 3.0;
        let s = NurbsSurface::new(
            2,
            3,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 0.0, 0.0, third, 1.0, 1.0, 1.0, 1.0],
            3,
            5,
            points,
            vec![1.0; 15],
        )
        .unwrap();
        (s, o)
    }

    /// Every coefficient of the rows a far patch enters the measure with
    /// is within its allowance of the exact one: the rows across are
    /// differences of points a million from the origin, so their rounding
    /// is of the points' size, not of the rows' own; and the rows along
    /// (`X₀′`, `X₀″`, `X₁′`), formed from the stored points' differences,
    /// are within theirs, which are of the derivatives' size.
    #[test]
    fn the_rows_rounding_is_within_its_allowance_far_from_the_origin() {
        let (s, _) = far_patch();
        let edge = iso_curve(&s, true, 0.0).unwrap();
        let origin = edge.control_points()[0];
        let line = spline_rows(&s, &edge, 1e-6, origin).unwrap().unwrap();
        assert!(!line.reversed && line.from == 0.0 && line.to == 1.0);
        let [x0, x0d, x0dd, x1, x1d, x2] = &line.rows;
        let t = s.knots_v()[4];
        let p = s.control_points();
        let check = |row: &Row, k: usize, c: usize, exact: Dd, what: &str| {
            let exact = exact.0 + exact.1;
            let got = row.0.points[k][c];
            assert!(
                (got - exact).abs() <= row.1[k][c],
                "{what}, control point {k}, coordinate {c}: {got:e} is {:e} off {exact:e}, the allowance is {:e}",
                (got - exact).abs(),
                row.1[k][c]
            );
        };
        for c in 0..3 {
            let get = |q: Vec3| [q.x, q.y, q.z][c];
            // Differences of nearby doubles are exact (Sterbenz).
            let d = |k: usize, j1: usize, j0: usize| get(p[k * 5 + j1]) - get(p[k * 5 + j0]);
            let along = |k1: usize, k0: usize, j: usize| get(p[k1 * 5 + j]) - get(p[k0 * 5 + j]);
            // X₁ₖ = 3 (P₁ − P₀) / t; X₂ₖ = 2 / t (3 (P₂ − P₁) − 3 (P₁ − P₀) / t).
            let r1 = |k: usize| mul(div((3.0, 0.0), t), d(k, 1, 0));
            for k in 0..3 {
                let d0 = get(p[k * 5]) - get(origin);
                let inner = add(
                    mul((3.0, 0.0), d(k, 2, 1)),
                    neg(div(mul((3.0, 0.0), d(k, 1, 0)), t)),
                );
                let c2 = div((2.0, 0.0), t);
                let r2 = add(mul(c2, inner.0), mul(c2, inner.1));
                check(x0, k, c, (d0, 0.0), "X0");
                check(x1, k, c, r1(k), "X1");
                check(x2, k, c, r2, "X2");
            }
            // Along: degree 2 on [0, 0, 0, 1, 1, 1], so `X′ₖ = 2 (Xₖ₊₁ − Xₖ)`
            // and `X″ = 2 ((X₂ − X₁) − (X₁ − X₀))`.
            for k in 0..2 {
                check(x0d, k, c, mul((along(k + 1, k, 0), 0.0), 2.0), "X0'");
                let r = add(r1(k + 1), neg(r1(k)));
                check(
                    x1d,
                    k,
                    c,
                    add(mul((r.0, 0.0), 2.0), mul((r.1, 0.0), 2.0)),
                    "X1'",
                );
            }
            let second = add((along(2, 1, 0), 0.0), neg((along(1, 0, 0), 0.0)));
            check(
                x0dd,
                0,
                c,
                add(mul((second.0, 0.0), 2.0), mul((second.1, 0.0), 2.0)),
                "X0''",
            );
        }
        for (row, exact) in [
            (x0, 1.0),
            (x1, 0.0),
            (x2, 0.0),
            (x0d, 0.0),
            (x0dd, 0.0),
            (x1d, 0.0),
        ] {
            for (h, e) in row.0.points.iter().zip(&row.1) {
                assert!((h[3] - exact).abs() <= e[3]);
            }
        }
        // The rows along carry the rounding of their own size: the
        // allowance of `X₀″` is a few ulps of `X₀′`'s differences, not of
        // the positions a unit from the origin (before: `(2p / Δ)²` times
        // theirs).
        let size = x0d.0.points.iter().map(|h| h[0].abs()).fold(0.0, f64::max);
        assert!(
            x0dd.1[0][0] <= 64.0 * f64::EPSILON * size,
            "{:e}",
            x0dd.1[0][0]
        );
    }
    /// The B-spline basis function `i` of degree `p` on `knots` at `v`, and
    /// its first and second derivatives (`order` 0, 1, 2), by the
    /// recursions straight from the definition: on dyadic knots at a
    /// dyadic `v` every step is exact in `f64`.
    fn basis_exact(knots: &[f64], p: usize, i: usize, v: f64, order: usize) -> f64 {
        let ratio = |num: f64, den: f64| if den == 0.0 { 0.0 } else { num / den };
        if order == 0 {
            if p == 0 {
                return if knots[i] <= v && v < knots[i + 1] {
                    1.0
                } else {
                    0.0
                };
            }
            let a = ratio(v - knots[i], knots[i + p] - knots[i]);
            let b = ratio(knots[i + p + 1] - v, knots[i + p + 1] - knots[i + 1]);
            return a * basis_exact(knots, p - 1, i, v, 0)
                + b * basis_exact(knots, p - 1, i + 1, v, 0);
        }
        let pf = p as f64;
        ratio(pf, knots[i + p] - knots[i]) * basis_exact(knots, p - 1, i, v, order - 1)
            - ratio(pf, knots[i + p + 1] - knots[i + 1])
                * basis_exact(knots, p - 1, i + 1, v, order - 1)
    }

    /// The same as `the_rows_rounding_is_within_its_allowance_far_from_the_origin`
    /// on an inner line (M2 of the class-A review, 2026-10-31): the rows
    /// are then made of the `q + 1` rows of the net whose basis functions
    /// reach the line, not of the border's first `j + 1`, and each
    /// coefficient is still within its allowance of the exact one. The
    /// reference: the double-double sum of the basis values at the line
    /// the rows are of (dyadic knots; the values found to a few units in
    /// the last place, far inside the allowance) times the points' exact
    /// differences.
    #[test]
    fn the_rows_rounding_is_within_its_allowance_on_an_inner_line() {
        let (far, _) = far_patch();
        let knots_v = vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0];
        let s = NurbsSurface::new(
            2,
            3,
            far.knots_u().to_vec(),
            knots_v.clone(),
            3,
            5,
            far.control_points().to_vec(),
            vec![1.0; 15],
        )
        .unwrap();
        let edge = iso_curve(&s, true, 0.25).unwrap();
        let origin = edge.control_points()[0];
        let line = spline_rows(&s, &edge, 1e-6, origin).unwrap().unwrap();
        let [x0, _, _, x1, _, x2] = &line.rows;
        // The line the rows are of: the parameter `iso_track_along` found
        // from the edge's projections, a rounding off 0.25 (how far the
        // edge lies from that line is the position gap, measured apart).
        let local = NurbsSurface::new(
            2,
            3,
            s.knots_u().to_vec(),
            knots_v.clone(),
            3,
            5,
            s.control_points().iter().map(|p| *p - origin).collect(),
            vec![1.0; 15],
        )
        .unwrap();
        let local_edge = NurbsCurve::new(
            edge.degree(),
            edge.knots().to_vec(),
            edge.control_points().iter().map(|p| *p - origin).collect(),
            edge.weights().to_vec(),
        )
        .unwrap();
        let track = Track::new(&local_edge, 1e-6).unwrap();
        let at = crate::network::iso_line(&local, &track, 1e-6)
            .unwrap()
            .unwrap()
            .at;
        // The line's parameter is the mean of the edge's projections: at a
        // million from the origin they round to a few 1e-11 (5.5e-11
        // found), so the line is within that of 0.25, not exactly on it.
        assert!((at - 0.25).abs() <= 1e-10, "{at}");
        let p = s.control_points();
        for k in 0..3 {
            for c in 0..3 {
                let get = |q: Vec3| [q.x, q.y, q.z][c];
                for (order, row) in [(0, x0), (1, x1), (2, x2)] {
                    // Σ N⁽ᵒʳᵈᵉʳ⁾ⱼ (Pⱼ − o): the differences of nearby
                    // doubles exact, the multiples exact, the sum in
                    // double-double.
                    let mut sum: Dd = (0.0, 0.0);
                    for j in 0..5 {
                        let n = basis_exact(&knots_v, 3, j, at, order);
                        let d = get(p[k * 5 + j]) - get(origin);
                        sum = add(sum, mul((d, 0.0), n));
                    }
                    let exact = sum.0 + sum.1;
                    let got = row.0.points[k][c];
                    assert!(
                        (got - exact).abs() <= row.1[k][c],
                        "order {order}, control point {k}, coordinate {c}: {got:e} is {:e} off {exact:e}, the allowance is {:e}",
                        (got - exact).abs(),
                        row.1[k][c]
                    );
                    assert!(row.1[k][c] > 0.0, "an inner row has rounding to allow for");
                }
            }
        }
    }

    // ---------------------------------------------------------------------
    // A rational net: the reference rows and pieces in double-double.

    fn dd_mul(x: Dd, y: Dd) -> Dd {
        let a = mul(x, y.0);
        add(a, mul((x.0, 0.0), y.1))
    }

    fn dd_div(x: Dd, y: Dd) -> Dd {
        let q = x.0 / y.0;
        let r = add(x, neg(dd_mul((q, 0.0), y)));
        add((q, 0.0), (r.0 / y.0, 0.0))
    }

    /// `b − a` of two doubles, exactly.
    fn dd_sub(b: f64, a: f64) -> Dd {
        two_sum(b, -a)
    }

    type DdH = [Dd; 4];

    /// The B-spline derivative of a net of homogeneous points in one
    /// direction (`p (P_{k+1} − P_k) / (t_{k+p+1} − t_{k+1})`), in
    /// double-double, the knots' differences exact.
    fn dd_derivative(points: &[DdH], knots: &[f64], p: usize) -> Vec<DdH> {
        (0..points.len() - 1)
            .map(|k| {
                let d = dd_sub(knots[k + p + 1], knots[k + 1]);
                let f = dd_div((p as f64, 0.0), d);
                std::array::from_fn(|c| dd_mul(f, add(points[k + 1][c], neg(points[k][c]))))
            })
            .collect()
    }

    /// The Bézier coefficients of a spline over `[x, y]` inside its span
    /// `s`, by de Boor's blossom in double-double.
    fn dd_blossom(points: &[DdH], knots: &[f64], p: usize, s: usize, x: f64, y: f64) -> Vec<DdH> {
        (0..=p)
            .map(|k| {
                let mut d: Vec<DdH> = points[s - p..=s].to_vec();
                for level in 1..=p {
                    let u = if level <= p - k { x } else { y };
                    for j in (level..=p).rev() {
                        let i = s - p + j;
                        let alpha = dd_div(
                            dd_sub(u, knots[i]),
                            dd_sub(knots[i + p + 1 - level], knots[i]),
                        );
                        let beta = add((1.0, 0.0), neg(alpha));
                        d[j] = std::array::from_fn(|c| {
                            add(dd_mul(beta, d[j - 1][c]), dd_mul(alpha, d[j][c]))
                        });
                    }
                }
                d[p]
            })
            .collect()
    }

    /// A rational patch far from the origin, of degree 2 along u on
    /// uneven knots (three spans) and 3 across v, its weights changing both
    /// ways: the differences of homogeneous points then have unequal
    /// weights (`stored_difference`'s second term), and the pieces are
    /// blossomed from uneven spans with breaks of another face inside.
    fn rational_far_patch() -> NurbsSurface {
        let o = Vec3::new(1_234_567.891, -2_345_678.912, 3_456_789.123);
        let mut points = Vec::new();
        let mut weights = Vec::new();
        for i in 0..5 {
            for j in 0..4 {
                let (fi, fj) = (f64::from(i), f64::from(j));
                points.push(
                    o + Vec3::new(
                        0.37 * fi + 0.11 * fj * fj,
                        0.29 * fj + 0.07 * fi * fi,
                        0.13 * fi * fj + 0.05 * fj,
                    ),
                );
                weights.push(1.0 + 0.3 * (1.3 * fi).psin() + 0.17 * (0.9 * fj).pcos());
            }
        }
        NurbsSurface::new(
            2,
            3,
            vec![0.0, 0.0, 0.0, 0.3, 0.7, 1.0, 1.0, 1.0],
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            5,
            4,
            points,
            weights,
        )
        .unwrap()
    }

    /// Every row of a rational patch far from the origin, and every Bézier
    /// piece cut from it (breaks of the face's own and of another face's
    /// inside its spans), is within its allowance of the exact one,
    /// computed in double-double from the stored points: `((P − o) w, w)`,
    /// the derivative nets each way, the row at the clamped border, and
    /// de Boor's blossom of each piece.
    #[test]
    fn a_rational_patch_rows_and_pieces_are_within_their_allowance() {
        let s = rational_far_patch();
        let edge = iso_curve(&s, true, 0.0).unwrap();
        let origin = edge.control_points()[2];
        let line = spline_rows(&s, &edge, 1e-6, origin).unwrap().unwrap();
        assert!(!line.reversed && line.from == 0.0 && line.to == 1.0);
        let (ku, kv) = (s.knots_u().to_vec(), s.knots_v().to_vec());
        // The net in double-double, `[i along][j across]`.
        let net: Vec<Vec<DdH>> = (0..5)
            .map(|i| {
                (0..4)
                    .map(|j| {
                        let p = s.control_points()[i * 4 + j];
                        let w = s.weights()[i * 4 + j];
                        let d = |a: f64, b: f64| mul(dd_sub(a, b), w);
                        [
                            d(p.x, origin.x),
                            d(p.y, origin.y),
                            d(p.z, origin.z),
                            (w, 0.0),
                        ]
                    })
                    .collect()
            })
            .collect();
        // Across at the border v = 0: the first one, two and three points.
        let across = |i: usize, order: usize| -> DdH {
            let mut col: Vec<DdH> = net[i].clone();
            let mut knots = kv.clone();
            for k in 0..order {
                col = dd_derivative(&col, &knots, 3 - k);
                knots = knots[1..knots.len() - 1].to_vec();
            }
            col[0]
        };
        let row =
            |order_across: usize| -> Vec<DdH> { (0..5).map(|i| across(i, order_across)).collect() };
        let x0 = row(0);
        let x1 = row(1);
        let x2 = row(2);
        let x0d = dd_derivative(&x0, &ku, 2);
        let x0dd = dd_derivative(&x0d, &ku[1..ku.len() - 1], 1);
        let x1d = dd_derivative(&x1, &ku, 2);
        let exact = [&x0, &x0d, &x0dd, &x1, &x1d, &x2];
        let names = ["X0", "X0'", "X0''", "X1", "X1'", "X2"];
        for ((computed, reference), name) in line.rows.iter().zip(exact).zip(names) {
            assert_eq!(computed.0.points.len(), reference.len(), "{name}");
            for (k, (h, r)) in computed.0.points.iter().zip(reference).enumerate() {
                for c in 0..4 {
                    let want = r[c].0 + r[c].1;
                    let e = computed.1[k][c];
                    assert!(
                        (h[c] - want).abs() <= e,
                        "{name}, point {k}, coordinate {c}: {:e} is {:e} off {want:e}, the allowance {e:e}",
                        h[c],
                        (h[c] - want).abs()
                    );
                }
            }
        }
        // The weights differ along, so the rows along carry rounding.
        assert!(line.rows[1].1.iter().any(|e| e[3] > 0.0));
        // The pieces: the face's own knots 0.3 and 0.7, and breaks of
        // another face at 0.15 and 0.5 inside its spans.
        let breaks = [0.15, 0.3, 0.5, 0.7];
        let cuts = [0.0, 0.15, 0.3, 0.5, 0.7, 1.0];
        for ((computed, reference), name) in line.rows.iter().zip(exact).zip(names) {
            let pieces = row_pieces(computed, &line, &breaks).unwrap();
            let (knots, p) = (&computed.0.knots, computed.0.degree);
            assert_eq!(pieces.len(), 5, "{name}");
            for (n, (piece, w)) in pieces.iter().zip(cuts.windows(2)).enumerate() {
                let span = knots
                    .iter()
                    .rposition(|k| *k <= w[0])
                    .unwrap()
                    .min(reference.len() - 1);
                let want = dd_blossom(reference, knots, p, span, w[0], w[1]);
                for (c, poly) in piece.iter().enumerate() {
                    for (k, got) in poly.c.iter().enumerate() {
                        let r = want[k][c].0 + want[k][c].1;
                        assert!(
                            (got - r).abs() <= poly.e,
                            "{name}, piece {n}, coefficient {k}, coordinate {c}: {got:e} is {:e} off, the allowance {:e}",
                            (got - r).abs(),
                            poly.e
                        );
                    }
                }
            }
        }
    }
}
