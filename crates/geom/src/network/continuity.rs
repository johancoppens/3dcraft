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
//! ch. 3). The faces and the edge are first written about a point of the
//! edge, so the rows are formed from local points and nothing cancels far
//! from the origin; a row across, `Σ cᵢ Pᵢ`, enters with `(4p + 6q + 24) ε`
//! times `Σ |cᵢ|` (bounded from the knots) times, per homogeneous
//! coordinate, the largest local value of that coordinate over the net's
//! rows the row is made from (`spline_rows`: on a clamped border the
//! first `j + 1` rows for the row `j`, every other basis function and its
//! first two derivatives being exactly zero there; on an inner line the
//! `q + 1` rows whose basis functions reach it); derivatives along the edge and the cut into
//! pieces add their own; each product of degrees `m` and `n` adds `γ_k`,
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

use super::{H, NetworkError, Raw, Track, derivative, internal, iso_track_along, point, refuse};
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

/// A row as it enters the measure, with the bound on its coefficients'
/// rounding, coordinate by coordinate (`x w`, `y w`, `z w`, `w`): each is
/// formed from that coordinate alone, so its rounding is of that
/// coordinate's size (an edge along a plane has its coordinate across the
/// plane at zero, and its rounding there with it: 2026-10-31, the
/// class-A mouse's seams).
type Row = (Raw, [f64; 4]);

/// A row cut into pieces at `breaks`: its coefficients carry what it came
/// in with (`allow`) and the cutting's own rounding (convex combinations,
/// `16 (p + 2) ε` of its magnitude).
fn row_pieces(raw: &Raw, breaks: &[f64], allow: [f64; 4]) -> Result<Vec<P4>, NetworkError> {
    let own = self::magnitude(raw);
    // Each coordinate and the weight by their own magnitudes.
    let allowance = |factor: f64| -> [f64; 4] {
        let mut out = allow;
        for (a, m) in out.iter_mut().zip(own) {
            *a += factor * EPS * m;
        }
        out
    };
    let mut raw = raw.clone();
    let mut have = raw.knots.clone();
    have.dedup();
    let mut missing = Vec::new();
    let mut h = 0;
    for &k in breaks {
        while have.get(h).is_some_and(|x| *x < k - SAME_KNOT) {
            h += 1;
        }
        if !have.get(h).is_some_and(|x| (*x - k).abs() <= SAME_KNOT) {
            missing.push(k);
        }
    }
    raw.insert_all(&missing)?;
    let p = raw.degree;
    let es = allowance(16.0 * (p + 2) as f64);
    let (_, pieces) = super::pieces_any(&raw)?;
    if pieces.len() != breaks.len() + 1 {
        return Err(internal());
    }
    Ok(pieces
        .into_iter()
        .map(|piece| {
            let comp = |k: usize| Poly {
                c: piece
                    .iter()
                    .map(|h| h.get(k).copied().unwrap_or(0.0))
                    .collect(),
                e: es.get(k).copied().unwrap_or(0.0),
            };
            [comp(0), comp(1), comp(2), comp(3)]
        })
        .collect())
}

/// The largest of each homogeneous coordinate of a row (`x w`, `y w`,
/// `z w`, `w`).
fn magnitude(raw: &Raw) -> [f64; 4] {
    raw.points.iter().fold([0.0_f64; 4], |mut m, h| {
        for (m, x) in m.iter_mut().zip(h) {
            *m = m.max(x.abs());
        }
        m
    })
}

/// The derivative of a row along the edge, with its allowance: each
/// coefficient is `fᵢ (b − a)`, `fᵢ = p / (t_{i+p+1} − t_{i+1})` (at most
/// `f`, the largest), so it carries `2 f` times what its source carried,
/// and the rounding of the two products and their sum, `4 ε f` times the
/// source's magnitude, doubled for the rounding of `fᵢ` itself.
fn derived(row: &Row) -> Result<Row, NetworkError> {
    let (source, allow) = row;
    let d = derivative(source)?;
    let p = source.degree;
    let span = source
        .knots
        .iter()
        .skip(1)
        .zip(source.knots.iter().skip(p + 1))
        .map(|(a, b)| b - a)
        .filter(|d| *d > 0.0)
        .fold(f64::INFINITY, f64::min);
    let f = if p == 0 {
        0.0
    } else if span.is_finite() {
        p as f64 / span
    } else {
        f64::INFINITY
    };
    let mag = magnitude(source);
    let a = |k: usize| {
        2.0 * f * allow.get(k).copied().unwrap_or(f64::INFINITY)
            + 8.0 * EPS * f * mag.get(k).copied().unwrap_or(f64::INFINITY)
    };
    Ok((d, [a(0), a(1), a(2), a(3)]))
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

/// The rows `X₀, X₁, X₂` of a spline face along `edge` (its iso-curve
/// there and its first and second homogeneous derivatives across), about
/// `origin`, each with the bound on its coefficients' rounding; `None`
/// where the edge is not a parameter line of the face.
///
/// The face and the edge are first written about `origin` (a point of the
/// edge), so the rows are formed from local points and nothing cancels
/// far from the origin: `x − o` is exact where the two are within a factor
/// two of each other (Sterbenz) and otherwise off by `ε` of the local
/// value, as is `(x − o) w`, so the local net is the face to within `2 ε`
/// of each local coefficient. A row across is `Σ cᵢ Pᵢ`, the `cᵢ` the
/// basis functions' values, first or second derivatives across at the
/// line, at most `p + 1` of them nonzero; `Σ |cᵢ|` is 1, at most
/// `S₁ = 2p / D_p` and at most `S₂ = S₁ · 2(p − 1) / D_{p−1}`
/// (`N′ᵢ = p (N_{i,p−1} / Δᵢ − N_{i+1,p−1} / Δᵢ₊₁)` and the partition of
/// unity; `D_k` the shortest positive `t_{i+k} − t_i`, in the direction
/// across). So each coefficient is within `γ S M_c` of the exact one,
/// `γ = (4p + 6q + 24) ε` with `p` the degree along and `q` across, and
/// `M_c` for each homogeneous coordinate `c` on its own (x, y, z times the
/// weight, and the weight): the largest local value of that coordinate
/// over the net's rows the row is a combination of. A coordinate of the
/// row is a sum of that coordinate of those points only, so its rounding
/// is of their size, not of the largest coordinate of the whole net (on a
/// long seam far longer than the patch is wide, the rows along the far
/// side and the other coordinates no longer set the allowance). Which
/// rows: on a clamped border (`at` exactly the first or the last knot)
/// the row `j` (`j` = 0 the line, 1 and 2 its derivatives across) is made
/// of the first `j + 1` rows of the net from that border, every other
/// basis function and its first and second derivatives being exactly zero
/// there (the clamped knot's multiplicity `q + 1`: `Nᵢ` with `i > j`
/// vanishes to order `i` at the border, so its derivatives up to `j < i`
/// are zero, exactly, in the recursions as computed: they are sums of
/// products with a factor that is exactly 0); on an inner line, the
/// `q + 1` rows whose basis functions reach the knot span of `at` (both
/// spans where `at` is a knot). The line is the face's parameter line at
/// `at` (`iso_track_along` puts `at` onto the border where the edge's
/// projections lie within `SAME_KNOT` of it): what is bounded here is
/// that line's rounding, and how far the edge lies from that line is the
/// position gap, which the measure bounds and holds to the tolerance in
/// its own right; the snap moves nothing the claim depends on. The
/// factor `γ` is the sum of: the net's rounding
/// (`2 ε`), the `cᵢ`'s (Cox–de Boor, `3 ε` a level, so `3q ε`; the
/// divided differences of the second row, `8 ε` each, and its de Boor
/// evaluation, `3q ε`), the sum (`q` more) and the cut to the edge (up to
/// `p` knot insertions a coefficient, convex combinations of two, `3 ε`
/// each, and the reparametrisation, `p ε`), with room.
fn spline_rows(
    surface: &NurbsSurface,
    edge: &NurbsCurve,
    tolerance: f64,
    origin: Vec3,
) -> Result<Option<[Row; 3]>, NetworkError> {
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
    let Some((rows, along_v, at)) = iso_track_along(&local, &track, tolerance)? else {
        return Ok(None);
    };
    // Along v the rows across are derivatives in u.
    let (pu, pv) = local.degrees();
    let (nu, nv) = local.counts();
    let (knots, q, n) = if along_v {
        (local.knots_u(), pu, nu)
    } else {
        (local.knots_v(), pv, nv)
    };
    // The net's rows across that row `j` (0 the line, 1 and 2 the
    // derivatives across) is made from: on a clamped border only the first
    // `j + 1` from it (every other basis function and its first two
    // derivatives are exactly zero there, so the other rows add exactly
    // nothing); elsewhere the `q + 1` whose basis functions reach the line.
    let (first, last) = (knots.first().copied(), knots.last().copied());
    let span_rows = || {
        let s = knots
            .iter()
            .rposition(|k| *k <= at)
            .unwrap_or(0)
            .min(n.saturating_sub(1).max(q));
        (s.saturating_sub(q), s.min(n.saturating_sub(1)))
    };
    let touched = |j: usize| -> (usize, usize) {
        if Some(at) == first {
            (0, j.min(n.saturating_sub(1)))
        } else if Some(at) == last {
            (n.saturating_sub(1 + j), n.saturating_sub(1))
        } else {
            span_rows()
        }
    };
    // Each homogeneous coordinate's largest local value over those rows: a
    // row's coordinate is a sum of that coordinate of the net's points.
    let magnitude_of = |(lo, hi): (usize, usize)| -> [f64; 4] {
        let points = local.control_points();
        let weights = local.weights();
        let count_along = if along_v { nv } else { nu };
        let mut m = [0.0_f64; 4];
        for i in lo..=hi {
            for k in 0..count_along {
                let index = if along_v { i * nv + k } else { k * nv + i };
                let (Some(p), Some(w)) = (points.get(index), weights.get(index)) else {
                    continue;
                };
                let w = w.abs();
                m = [
                    m[0].max(p.x.abs() * w),
                    m[1].max(p.y.abs() * w),
                    m[2].max(p.z.abs() * w),
                    m[3].max(w),
                ];
            }
        }
        m
    };
    let s = across_sums(knots, q);
    let sums = [1.0, s[0], s[1]];
    let mut out = Vec::with_capacity(3);
    for (j, (raw, sum)) in rows.into_iter().zip(sums).enumerate() {
        let gamma = (4 * raw.degree + 6 * q + 24) as f64 * EPS;
        let m = magnitude_of(touched(j));
        out.push((raw, m.map(|m| gamma * sum * m)));
    }
    let [x0, x1, x2]: [Row; 3] = out.try_into().map_err(|_| internal())?;
    Ok(Some([x0, x1, x2]))
}

/// `[S₁, S₂]`: bounds on `Σ |N′ᵢ|` and `Σ |N″ᵢ|` of the basis of degree `p`
/// on `knots` anywhere (see [`spline_rows`]); infinite where a span the
/// bound divides by is missing.
fn across_sums(knots: &[f64], p: usize) -> [f64; 2] {
    let shortest = |k: usize| {
        knots
            .iter()
            .zip(knots.iter().skip(k))
            .map(|(a, b)| b - a)
            .filter(|d| *d > 0.0)
            .fold(f64::INFINITY, f64::min)
    };
    if p == 0 {
        return [0.0, 0.0];
    }
    // With no positive span there is no bound.
    let over = |x: f64, d: f64| if d.is_finite() { x / d } else { f64::INFINITY };
    let s1 = over(2.0 * p as f64, shortest(p));
    let s2 = if p == 1 {
        0.0
    } else {
        s1 * over(2.0 * (p - 1) as f64, shortest(p - 1))
    };
    [s1, s2]
}

/// Rows written about `origin` with their allowances: the face `a`'s, and
/// a spline face `b`'s where `other` is one.
fn measure(
    xs: [Row; 3],
    ys: Option<[Row; 3]>,
    other: Other,
    origin: Vec3,
    tolerance: f64,
    angular: f64,
) -> Result<Continuity, NetworkError> {
    let [x0, x1, x2] = xs;
    let length =
        x0.0.points
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
    let x0d = derived(&x0)?;
    let x0dd = derived(&x0d)?;
    let x1d = derived(&x1)?;
    let mut rows: Vec<Row> = vec![x0, x0d, x0dd, x1, x1d, x2];
    let spline_degree = match ys {
        Some([y0, y1, y2]) => {
            let q = y0.0.degree.max(y1.0.degree).max(y2.0.degree);
            let y0d = derived(&y0)?;
            let y0dd = derived(&y0d)?;
            let y1d = derived(&y1)?;
            rows.extend([y0, y0d, y0dd, y1, y1d, y2]);
            Some(q)
        }
        None => None,
    };
    let p = rows
        .iter()
        .take(6)
        .map(|(r, _)| r.degree)
        .max()
        .unwrap_or(0);
    let degree = match spline_degree {
        Some(q) => 4 * p + 20 * q,
        None => 5 * p,
    };
    if degree > MAX_SEAM_DEGREE {
        return refuse(format!(
            "the faces are of too high a degree along the edge to bound how they meet (the bounds would be polynomials of degree {degree}, the most is {MAX_SEAM_DEGREE})"
        ));
    }
    // The pieces between every row's knots.
    let mut breaks: Vec<f64> = rows.iter().flat_map(|(r, _)| r.interior()).collect();
    breaks.sort_by(f64::total_cmp);
    breaks.dedup_by(|x, y| (*x - *y).abs() <= SAME_KNOT);
    let mut per_row: Vec<Vec<P4>> = Vec::with_capacity(rows.len());
    for (raw, allow) in &rows {
        per_row.push(row_pieces(raw, &breaks, *allow)?);
    }
    let mut bounds = vec![0.0];
    bounds.extend(breaks.iter().copied());
    bounds.push(1.0);
    // The work: each bound of a piece costs about the square of the
    // polynomials' degree (their products), each piece looked at in 5 or
    // 3 places as well.
    let cost = (degree as u64 + 1).saturating_pow(2);
    let first = cost.saturating_mul(6).saturating_mul(bounds.len() as u64);
    if first > MAX_SEAM_WORK {
        return refuse(format!(
            "the edge's faces have too many knot spans along it to bound how they meet ({} pieces of degree {degree})",
            bounds.len() - 1
        ));
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
    /// is of the points' size, not of the rows' own.
    #[test]
    fn the_rows_rounding_is_within_its_allowance_far_from_the_origin() {
        let (s, _) = far_patch();
        let edge = iso_curve(&s, true, 0.0).unwrap();
        let origin = edge.control_points()[0];
        let [x0, x1, x2] = spline_rows(&s, &edge, 1e-6, origin).unwrap().unwrap();
        let t = s.knots_v()[4];
        let p = s.control_points();
        for k in 0..3 {
            let at = |j: usize| p[k * 5 + j];
            for c in 0..3 {
                let get = |q: Vec3| [q.x, q.y, q.z][c];
                // Differences of nearby doubles are exact (Sterbenz).
                let d0 = get(at(0)) - get(origin);
                let d10 = get(at(1)) - get(at(0));
                let d21 = get(at(2)) - get(at(1));
                // X₁ = 3 (P₁ − P₀) / t; X₂ = 2 / t (3 (P₂ − P₁) − 3 (P₁ − P₀) / t).
                let r1 = mul(div((3.0, 0.0), t), d10);
                let inner = add(mul((3.0, 0.0), d21), neg(div(mul((3.0, 0.0), d10), t)));
                let c2 = div((2.0, 0.0), t);
                let r2 = add(mul(c2, inner.0), mul(c2, inner.1));
                for (row, exact) in [(&x0, d0), (&x1, r1.0 + r1.1), (&x2, r2.0 + r2.1)] {
                    let got = row.0.points[k][c];
                    assert!(
                        (got - exact).abs() <= row.1[c],
                        "control point {k}, coordinate {c}: {got:e} is {:e} off {exact:e}, the allowance is {:e}",
                        (got - exact).abs(),
                        row.1[c]
                    );
                }
            }
            for (row, exact) in [(&x0, 1.0), (&x1, 0.0), (&x2, 0.0)] {
                assert!((row.0.points[k][3] - exact).abs() <= row.1[3]);
            }
        }
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
        let [x0, x1, x2] = spline_rows(&s, &edge, 1e-6, origin).unwrap().unwrap();
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
        let (_, _, at) = iso_track_along(&local, &track, 1e-6).unwrap().unwrap();
        // The line's parameter is the mean of the edge's projections: at a
        // million from the origin they round to a few 1e-11 (5.5e-11
        // found), so the line is within that of 0.25, not exactly on it.
        assert!((at - 0.25).abs() <= 1e-10, "{at}");
        let p = s.control_points();
        for k in 0..3 {
            for c in 0..3 {
                let get = |q: Vec3| [q.x, q.y, q.z][c];
                for (order, row) in [(0, &x0), (1, &x1), (2, &x2)] {
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
                        (got - exact).abs() <= row.1[c],
                        "order {order}, control point {k}, coordinate {c}: {got:e} is {:e} off {exact:e}, the allowance is {:e}",
                        (got - exact).abs(),
                        row.1[c]
                    );
                    assert!(row.1[c] > 0.0, "an inner row has rounding to allow for");
                }
            }
        }
    }
}
