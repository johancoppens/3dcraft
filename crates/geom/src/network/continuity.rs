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
//!   meridian plane, which lies on the cone); to a spline face, the
//!   distance between corresponding points `|x₀ω − y₀w| / (wω)`. Every
//!   point of a face is at least as far as its nearest, so each is an
//!   upper bound on the distance to the face.
//! - *Angle*: the sine of the angle between the tangent planes, `|W × G| /
//!   (|W| |G|)`, `G` the face's normal: `(QX₀)_xyz` (the gradient, which
//!   for a sphere and a cylinder points along the normal at the nearest
//!   point; for a cone it is the normal of the quadric's level surface
//!   through the point, the cone's own on it), `n` for a plane, `W_b` for
//!   a spline face.
//! - *Curvature*: across the edge, in the direction `S_v` the face `a`
//!   leaves it. Against an implicit face `g = 0` the second-order contact
//!   `h″ = d²/dv² g(S)`: `G2 ⇔ h = h′ = h″ = 0`, and `|h″| / (|∇g| |S_v|²)`
//!   is the difference of the normal curvatures in that direction wherever
//!   the faces share their tangent plane (exactly; in homogeneous terms
//!   `P₂ = H₂w² − 4ww₁H₁ + (6w₁² − 2ww₂)H`, `H = XᵀQX` and its
//!   derivatives across, for a quadric; `L·X` likewise for a plane).
//!   Against a spline face: `n_b·S_vv − II_b(S_v)`, `S_v` put into its
//!   tangent plane, through `det[Y, Y_a, Y_t, Y_ab] = −ω⁴ (B_a × B_t)·B_ab`
//!   (its second fundamental form, Farin ch. 22) and Cramer's rule, over
//!   `|S_v|²`.
//!
//! **Rounding.** Every coefficient carries a bound on its rounding error,
//! carried through each sum and product (running error analysis, Higham
//! ch. 3): what enters (the rows' coefficients after cutting into pieces
//! and taking derivatives) gets `16 (p + 2) ε` of the magnitudes it came
//! from, and each product of degrees `m` and `n` adds `4 (m + n + 2) ε`
//! times the product of the magnitudes. Bounds take the errors in.
//!
//! **Thresholds** come from the document: the position from the
//! tolerance, the angle from the angular tolerance, and the curvature
//! from both and the edge's length `L` (its control polygon's): a
//! difference `Δκ` turns the two faces' normals apart by `Δκ d` and opens
//! a gap of `Δκ d² / 2` over a distance `d` from the seam, so over the
//! seam's own length it is held to `min(angular / L, 2 tol / L²)`.

use super::{H, NetworkError, Raw, Track, derivative, internal, iso_track, point, refuse};
use crate::refine::{SAME_KNOT, bezier_pieces_batched};
use crate::{NurbsCurve, NurbsSurface, Surface};
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
    /// In position, with one tangent plane and the same normal curvature.
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
    pub curvature: Gap,
    /// The length the curvature threshold is derived from.
    pub length: f64,
    /// Pieces the edge was cut into.
    pub pieces: usize,
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

    fn mag(&self) -> f64 {
        self.c.iter().fold(0.0_f64, |m, x| m.max(x.abs()))
    }

    fn lo(&self) -> f64 {
        self.c.iter().copied().fold(f64::INFINITY, f64::min) - self.e
    }

    fn hi(&self) -> f64 {
        self.c.iter().copied().fold(f64::NEG_INFINITY, f64::max) + self.e
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
        let gamma = 4.0 * (m + n + 2) as f64 * EPS;
        Poly {
            c: out,
            e: ma * o.e + mb * self.e + self.e * o.e + gamma * ma * mb,
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
    let m = |x: &P4, y: &P4, i: usize, j: usize| x[i].mul(&y[j]).sub(&x[j].mul(&y[i]));
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
            .map(|p| p.c.get(k).copied().unwrap_or(0.0).powi(2))
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
        .map(|p| p.c.first().copied().unwrap_or(0.0).powi(2))
        .sum();
    (s.sqrt() - err).max(0.0)
}

fn norm_value_hi(v: &V3) -> f64 {
    let err = v.iter().map(|p| p.e * p.e).sum::<f64>().sqrt();
    let s: f64 = v
        .iter()
        .map(|p| p.c.first().copied().unwrap_or(0.0).powi(2))
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
    /// A spline face's rows `Y₀, Y₀′, Y₀″, Y₁, Y₁′, Y₂` along the edge.
    Spline([Raw; 6]),
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
            let t = co.half_angle().tan();
            let a = projector(d, 1.0 + t * t);
            let aa = mat3(&a, apex);
            Some(ImplicitForm::Cone(
                quadric(a, aa * -1.0, apex.dot(aa)),
                apex,
                d,
                t.abs(),
                co.half_angle().cos(),
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

/// `[x − o w, w]`.
fn about(raw: &Raw, o: Vec3) -> Raw {
    Raw {
        degree: raw.degree,
        knots: raw.knots.clone(),
        points: raw
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
    }
}

/// A row cut into pieces at `breaks`, with the rounding allowance of what
/// enters (`magnitude`: of what it was made from).
fn row_pieces(raw: &Raw, breaks: &[f64], magnitude: f64) -> Result<Vec<P4>, NetworkError> {
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
    let own = raw
        .points
        .iter()
        .fold(0.0_f64, |m, h| h.iter().fold(m, |m, x| m.max(x.abs())));
    let e = 16.0 * (p + 2) as f64 * EPS * own.max(magnitude);
    let (_, pieces) = bezier_pieces_batched(raw)?;
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
                e,
            };
            [comp(0), comp(1), comp(2), comp(3)]
        })
        .collect())
}

fn magnitude(raw: &Raw) -> f64 {
    raw.points
        .iter()
        .fold(0.0_f64, |m, h| h.iter().fold(m, |m, x| m.max(x.abs())))
}

/// A derivative's magnitude bound: its own, and `p / span` times its
/// source's.
fn derived(source: &Raw, d: &Raw, source_mag: f64) -> f64 {
    let p = source.degree as f64;
    let mut span = f64::INFINITY;
    for w in source.knots.windows(2) {
        if let [a, b] = w
            && b > a
        {
            span = span.min(b - a);
        }
    }
    let f = if span.is_finite() { p / span } else { 0.0 };
    magnitude(d).max(f * source_mag)
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

/// Upper bounds of the three gaps over rows (as polynomials of a piece),
/// and, for constant rows, lower bounds of their values.
struct Gaps {
    hi: [f64; 3],
    lo: [f64; 3],
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

fn gaps(rows: &[P4], other: &Other) -> Result<Gaps, NetworkError> {
    let row = |k: usize| rows.get(k).cloned().ok_or_else(internal);
    let (x0, x0d, x1, x2) = (row(0)?, row(1)?, row(2)?, row(3)?);
    let (w0, w0d, w1, w2) = (&x0[3], &x0d[3], &x1[3], &x2[3]);
    let (p0, p0d, p1, p2) = (xyz4(&x0), xyz4(&x0d), xyz4(&x1), xyz4(&x2));
    let v0d = minus(&times(&p0d, w0), &times(&p0, w0d));
    let v1 = minus(&times(&p1, w0), &times(&p0, w1));
    let w = cross(&v0d, &v1);
    let v1sq = dot(&v1, &v1);
    let (v1sq_lo, v1sq_hi) = (v1sq.lo().max(0.0), v1sq.hi());
    let (w_lo, w_hi) = (norm_lo(&w), norm_value_hi(&w));
    let (w0_lo, w0_hi) = (w0.lo(), w0.hi());
    let mut hi = [0.0; 3];
    let mut lo = [0.0; 3];
    match other {
        Other::Plane { l } => {
            let lx = |x: &P4| {
                x[0].scale(l[0])
                    .add(&x[1].scale(l[1]))
                    .add(&x[2].scale(l[2]))
                    .add(&x[3].scale(l[3]))
            };
            let (g0, g1, g2) = (lx(&x0), lx(&x1), lx(&x2));
            hi[0] = ratio(abs_hi(&g0), w0_lo);
            lo[0] = abs_value_lo(&g0) / w0_hi;
            let n = konst3(Vec3::new(l[0], l[1], l[2]));
            let z = cross(&w, &n);
            hi[1] = ratio(norm_hi(&z), w_lo);
            lo[1] = norm_value_lo(&z) / w_hi;
            let p2l = g2
                .mul(&w0.mul(w0))
                .sub(&w0.mul(w1).mul(&g1).scale(2.0))
                .sub(&w0.mul(w2).mul(&g0))
                .add(&w1.mul(w1).mul(&g0).scale(2.0));
            let num = p2l.mul(w0);
            hi[2] = ratio(abs_hi(&num), v1sq_lo);
            lo[2] = abs_value_lo(&num) / v1sq_hi;
        }
        Other::Round { .. } | Other::Cone { .. } => {
            let q = match other {
                Other::Round { q, .. } | Other::Cone { q, .. } => q,
                _ => return Err(internal()),
            };
            let qx0 = apply(q, &x0);
            let qx1 = apply(q, &x1);
            let hh = dot4(&x0, &qx0);
            let h1 = dot4(&x0, &qx1).scale(2.0);
            let h2 = dot4(&x1, &qx1).add(&dot4(&x2, &qx0)).scale(2.0);
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
            // Curvature.
            let p2 = h2
                .mul(&w0.mul(w0))
                .sub(&w0.mul(w1).mul(&h1).scale(4.0))
                .add(&w1.mul(w1).scale(6.0).sub(&w0.mul(w2).scale(2.0)).mul(&hh));
            let num = p2.mul(w0);
            hi[2] = ratio(abs_hi(&num), 2.0 * g_lo * v1sq_lo);
            lo[2] = abs_value_lo(&num) / (2.0 * g_hi * v1sq_hi);
        }
        Other::Spline(_) => {
            let r = |k: usize| rows.get(4 + k).cloned().ok_or_else(internal);
            let (y0, y0d, y0dd, y1, y1d, y2) = (r(0)?, r(1)?, r(2)?, r(3)?, r(4)?, r(5)?);
            let (om, om_d, om1) = (&y0[3], &y0d[3], &y1[3]);
            let (q0, q0d, q1) = (xyz4(&y0), xyz4(&y0d), xyz4(&y1));
            // Position.
            let e0 = minus(&times(&p0, om), &times(&q0, w0));
            let (om_lo, om_hi) = (om.lo(), om.hi());
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
            // Curvature.
            let a2 = minus(
                &times(&minus(&times(&p2, w0), &times(&p0, w2)), w0),
                &times(&v1, &w1.scale(2.0)),
            );
            let m = dot(&wb, &wb);
            let an = dot(&cross(&v1, &vb), &wb);
            let bn = dot(&cross(&ub, &v1), &wb);
            let duu = det4(&y0, &y0d, &y1, &y0dd);
            let dut = det4(&y0, &y0d, &y1, &y1d);
            let dtt = det4(&y0, &y0d, &y1, &y2);
            let q2 = an
                .mul(&an)
                .mul(&duu)
                .add(&an.mul(&bn).mul(&dut).scale(2.0))
                .add(&bn.mul(&bn).mul(&dtt));
            let om2 = om.mul(om);
            let num = dot(&wb, &a2)
                .mul(w0)
                .mul(&m.mul(&m))
                .add(&om2.mul(&om2).mul(&q2));
            let (m_lo, m_hi) = (m.lo().max(0.0), m.hi());
            hi[2] = ratio(abs_hi(&num), m_lo * m_lo * wb_lo * v1sq_lo);
            lo[2] = abs_value_lo(&num) / (m_hi * m_hi * wb_hi * v1sq_hi);
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
    let Some([x0, x1, x2]) = iso_track(&sa, &track, tolerance)? else {
        return refuse(
            "the edge does not run along a parameter line of its spline face, so the face cannot be written along it exactly (owed: a measure at the feet with a Lipschitz bound)".to_string(),
        );
    };
    let origin = x0
        .points
        .first()
        .copied()
        .and_then(point)
        .ok_or_else(internal)?;
    let other = match other_face {
        Surface::Nurbs(_) | Surface::Bezier(_) => {
            let sb = spline(other_face)?.ok_or_else(internal)?;
            let Some([y0, y1, y2]) = iso_track(&sb, &track, tolerance)? else {
                return refuse(
                    "the edge does not run along a parameter line of the other spline face, so the two cannot be compared at corresponding points (owed: a measure at the feet with a Lipschitz bound)".to_string(),
                );
            };
            let y0d = derivative(&y0)?;
            let y0dd = derivative(&y0d)?;
            let y1d = derivative(&y1)?;
            Other::Spline([y0, y0d, y0dd, y1, y1d, y2])
        }
        Surface::Torus(_) => {
            return refuse(
                "continuity to a torus is not measured yet (a quartic; owed: its implicit form)"
                    .to_string(),
            );
        }
        analytic => implicit_of(analytic, origin).ok_or_else(internal)?.other(),
    };
    measure(&x0, &x1, &x2, other, origin, tolerance, angular)
}

/// Rows written about `origin`; a spline face's rows are taken as given
/// (global) and moved there too.
fn measure(
    x0: &Raw,
    x1: &Raw,
    x2: &Raw,
    other: Other,
    origin: Vec3,
    tolerance: f64,
    angular: f64,
) -> Result<Continuity, NetworkError> {
    let length = x0
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
        angular.min(std::f64::consts::FRAC_PI_2).sin(),
        thr[2],
    ];
    // Rows about the origin, with the magnitudes they were made from.
    let mut rows: Vec<(Raw, f64)> = Vec::new();
    let push = |raw: &Raw| (about(raw, origin), magnitude(raw));
    let (a0, m0) = push(x0);
    let a0d = derivative(&a0)?;
    let m0d = derived(&a0, &a0d, m0);
    rows.push((a0, m0));
    rows.push((a0d, m0d));
    rows.push(push(x1));
    rows.push(push(x2));
    let spline_degree = if let Other::Spline(ys) = &other {
        for y in ys {
            let moved = about(y, origin);
            let mag = magnitude(y).max(magnitude(&moved));
            rows.push((moved, mag));
        }
        Some(ys.iter().map(|y| y.degree).max().unwrap_or(0))
    } else {
        None
    };
    let p = rows
        .iter()
        .take(4)
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
    for (raw, mag) in &rows {
        per_row.push(row_pieces(raw, &breaks, *mag)?);
    }
    let mut bounds = vec![0.0];
    bounds.extend(breaks.iter().copied());
    bounds.push(1.0);
    let mut work: Vec<(Piece, [f64; 3])> = Vec::with_capacity(bounds.len());
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
    let mut found = [(0.0_f64, 0.0_f64, origin); 3];
    let look = |piece: &Piece,
                ts: &[f64],
                found: &mut [(f64, f64, Vec3); 3]|
     -> Result<(), NetworkError> {
        for &t in ts {
            let at = piece.at(t);
            let lo = gaps(&at, &other)?.lo;
            let place = at
                .first()
                .map(|x| {
                    let w = x[3].c.first().copied().unwrap_or(0.0);
                    let c = |k: usize| x[k].c.first().copied().unwrap_or(0.0) / w;
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
        let decided: Vec<bool> = (0..3)
            .map(|q| {
                let lim = limit.get(q).copied().unwrap_or(0.0);
                let all_within = work
                    .iter()
                    .all(|(_, hi)| hi.get(q).is_some_and(|h| *h <= lim));
                all_within || found.get(q).is_some_and(|f| f.0 > lim)
            })
            .collect();
        if decided.iter().all(|d| *d) || work.len() >= MAX_SEAM_PIECES {
            break;
        }
        let mut next: Vec<(Piece, [f64; 3])> = Vec::with_capacity(work.len() * 2);
        let mut split = false;
        let total = work.len();
        for (index, (piece, hi)) in work.into_iter().enumerate() {
            let open = (0..3).any(|q| {
                !decided.get(q).copied().unwrap_or(true)
                    && hi.get(q).copied().unwrap_or(0.0) > limit.get(q).copied().unwrap_or(0.0)
            });
            let room = next.len() + (total - index) < MAX_SEAM_PIECES;
            if open && room && piece.depth < 52 {
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
                x.clamp(0.0, 1.0).asin()
            } else {
                x
            }
        };
        Gap {
            bound: conv(b),
            found: conv(f),
            param,
            at,
            threshold: thr.get(q).copied().unwrap_or(0.0),
        }
    };
    let (position, angle, curvature) = (gap(0, false), gap(1, true), gap(2, false));
    let order = if !position.within() {
        Order::None
    } else if !angle.within() {
        Order::G0
    } else if !curvature.within() {
        Order::G1
    } else {
        Order::G2
    };
    Ok(Continuity {
        order,
        position,
        angle,
        curvature,
        length,
        pieces: work.len(),
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
    format!(
        "{}; {}; {}",
        part("position", &c.position, ""),
        part("angle", &c.angle, " rad"),
        part("curvature", &c.curvature, " per unit")
    )
}
