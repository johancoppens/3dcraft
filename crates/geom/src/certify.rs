//! Proven bounds: how far a curve can stray from a surface over an
//! interval of its parameter, decided by proof, not at samples.
//!
//! A fitted curve that passes through points found exactly and is held to
//! the tolerance at the middles between them can still leave its surface
//! between those middles. [`SurfaceBound::curve_within`] proves a bound on
//! the distance instead, for any curve and any surface the kernel knows:
//!
//! - **A comparison path.** Over a piece `[a, b]` of the curve's parameter
//!   (`τ` from 0 to 1 across it) the points of the curve are compared with
//!   the points `S(σ(τ))` of the surface along a path `σ` in its
//!   parameters: the cubic Hermite path from the foot of the curve's point
//!   at `a` to the foot of its point at `b`, leaving each along the
//!   projection of the curve's tangent into the surface's tangent plane.
//!   Every point of the surface is as far from the curve's point as its
//!   foot at most, so `|e(τ)| = |C(τ) − S(σ(τ))|` bounds the distance from
//!   above whatever `σ` is: the feet are found by Gauss-Newton steps, and
//!   how well they converged changes how tight the bound is, never whether
//!   it holds.
//! - **The error of cubic Hermite interpolation.** Where `e` is four times
//!   differentiable (the curve's piece is one Bézier piece, the box `σ`
//!   sweeps lies in one Bézier patch of the surface),
//!   `|e(τ)| ≤ max |e(end)| + ¼ max |e'(end)| + max |e⁗| / 384` (the Peano
//!   kernel of cubic Hermite interpolation keeps its sign, so the bound
//!   holds for vectors as for numbers, with `τ² (1 − τ)² / 24 ≤ 1 / 384`).
//!   `e'` at the ends is what is left of the curve's tangent off the
//!   surface's tangent plane, and `e⁗` is bounded by majorants: the
//!   partial derivatives of the surface over the box (in closed form for
//!   the analytic kinds; from the differences of the Bézier net cut to the
//!   box, for a rational patch through its homogeneous form `A / w` with
//!   `|G⁽ᵏ⁾| ≤ (|A⁽ᵏ⁾| + Σ C(k, j) |w⁽ʲ⁾| |G⁽ᵏ⁻ʲ⁾|) / min w`) composed with
//!   the bounds on the path's derivatives (Faà di Bruno with every term
//!   replaced by its bound, which is the composition of the majorant
//!   series). The remainder falls with the fourth power of the piece, as
//!   the error of the fitted curve does, so a few halvings prove a curve
//!   that holds.
//! - **Where the box crosses a knot line** of the surface (whose
//!   derivatives may jump there) the derivatives are bounded patch by
//!   patch, and a bound of order `j` holds over the whole piece where the
//!   surface is `C^(j−1)` across the lines crossed (degree less the
//!   knot's multiplicity): the cubic Hermite bound across `C³` lines, the
//!   linear one, `max |e(end)| + max |e''| / 8`, across `C¹` lines (a
//!   cubic surface's simple knots), and otherwise the Lipschitz one,
//!   `(|e(0)| + |e(1)| + max |e'|) / 2`, which holds for a continuous
//!   surface; the piece is halved until that is within the target too.
//! - **Adaptive and bounded.** The pieces start at the curve's knots and
//!   the caller's breaks; a piece whose bound is over the target is
//!   halved, in a fixed order, until every bound is within it, or the
//!   number of pieces allowed is spent and the answer is "not proven"
//!   (with the place), never a guess.
//!
//! **The surface continued.** Distances are to the surface as it
//! evaluates, continued past its domain: the analytic kinds are unbounded,
//! and a spline's end patches go on as the polynomials they are (as
//! `eval` does, so a blend continued past a lofted face's edge is held to
//! the face it would be). A proof says the curve is near that surface, not
//! that it stays within a face's bounds or the spline's domain: a line
//! five units outside a Bézier square, in the plane the square continues
//! into, is proven on it.
//!
//! Rounding: the distances at the ends are measured in floating point; an
//! allowance of [`ROUNDING_ULPS`] units in the last place of what enters
//! the evaluations (the control net times the spread of its weights; the
//! frame's origin, the radii and the parameters of an analytic kind) is
//! added. For two curves, the closest points of nearly parallel chords are
//! found through a difference that cancels, and up to `√ε` of their
//! lengths is allowed.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::{BezierSpan, Curve3, CurvePiece, NurbsCurve, RationalBezierPatch, Surface};
use craft3d_math::Portable;
use craft3d_math::Vec3;

/// Orders of derivative carried: 0 to 4, what the error of cubic Hermite
/// interpolation needs.
const N: usize = 5;
/// Bounds on the derivatives of orders 0 to 4 of a function of `τ`.
type Jet = [f64; N];
/// Bounds on the partial derivatives `∂uⁱ ∂vʲ` of a function of `(u, v)`,
/// at `[i][j]`, for `i + j < N`.
type Table = [[f64; N]; N];
/// A homogeneous point, translated: `((p − origin) w, w)`.
type Hom = [f64; 4];

/// Gauss-Newton steps for a foot point.
const FOOT_STEPS: usize = 32;
/// Steps by which the foot of one starting point is followed to the next.
const FOLLOW: usize = 8;
/// The narrowest box a patch is cut to, as a part of its span: a box of no
/// width (a path along an iso-line) is widened to this, which only makes
/// the bound hold over more of the patch.
const NARROWEST: f64 = 1e-9;
/// Where `|∂u × ∂v|²` is below this share of `(|∂u|² + |∂v|²)²` the
/// parametrisation is singular for a foot's steps.
const SINGULAR: f64 = 1e-18;

fn get(jet: &Jet, k: usize) -> f64 {
    jet.get(k).copied().unwrap_or(0.0)
}

fn entry(table: &Table, i: usize, j: usize) -> f64 {
    table
        .get(i)
        .and_then(|row| row.get(j))
        .copied()
        .unwrap_or(0.0)
}

fn factorial(k: usize) -> f64 {
    (1..=k).map(|x| x as f64).product()
}

fn binomial(n: usize, k: usize) -> f64 {
    factorial(n) / (factorial(k) * factorial(n.saturating_sub(k)))
}

/// `m (m − 1) … (m − i + 1)`: what the `i`-th derivative of a Bernstein
/// form of degree `m` scales its `i`-th differences by.
fn falling(m: usize, i: usize) -> f64 {
    if i > m {
        return 0.0;
    }
    (0..i).map(|k| (m - k) as f64).product()
}

/// The product of two truncated power series.
fn series_mul(a: &Jet, b: &Jet) -> Jet {
    let mut out = [0.0; N];
    for (n, slot) in out.iter_mut().enumerate() {
        *slot = (0..=n).map(|k| get(a, k) * get(b, n - k)).sum();
    }
    out
}

/// Bounds on the derivatives of `F(x(τ), y(τ))`, from bounds `table` on
/// the partial derivatives of `F` over a box that holds the path, and
/// bounds `x`, `y` on the derivatives (orders 1 to 4) of the path: the
/// coefficients of the composition of the majorant series. Order 0 is
/// `table[0][0]`.
fn compose(table: &Table, x: &Jet, y: &Jet) -> Jet {
    let series = |d: &Jet| -> Jet {
        let mut s = [0.0; N];
        for (k, slot) in s.iter_mut().enumerate().skip(1) {
            *slot = get(d, k) / factorial(k);
        }
        s
    };
    let (xs, ys) = (series(x), series(y));
    let mut one = [0.0; N];
    if let Some(first) = one.first_mut() {
        *first = 1.0;
    }
    let powers = |s: &Jet| -> Vec<Jet> {
        let mut out = vec![one];
        for _ in 1..N {
            let last = out.last().copied().unwrap_or(one);
            out.push(series_mul(&last, s));
        }
        out
    };
    let (px, py) = (powers(&xs), powers(&ys));
    let mut total = [0.0; N];
    for (i, pi) in px.iter().enumerate() {
        for (j, pj) in py.iter().enumerate() {
            if i + j >= N {
                continue;
            }
            let m = entry(table, i, j);
            if !(m > 0.0) {
                continue;
            }
            let term = series_mul(pi, pj);
            let scale = m / (factorial(i) * factorial(j));
            for (slot, t) in total.iter_mut().zip(term) {
                *slot += scale * t;
            }
        }
    }
    let mut out = [0.0; N];
    for (k, (slot, t)) in out.iter_mut().zip(total).enumerate() {
        *slot = t * factorial(k);
    }
    if let Some(first) = out.first_mut() {
        *first = entry(table, 0, 0);
    }
    out
}

/// Bounds on the derivatives of `G = A / w` from those of `A` and `w`,
/// `|G|` at most `g0` and `w` at least `lightest`: from `A = w G`,
/// `w G⁽ᵏ⁾ = A⁽ᵏ⁾ − Σⱼ₌₁ᵏ C(k, j) w⁽ʲ⁾ G⁽ᵏ⁻ʲ⁾`.
fn quotient(a: &Jet, w: &Jet, g0: f64, lightest: f64) -> Jet {
    let mut g = [0.0; N];
    for k in 0..N {
        let value = if k == 0 {
            g0
        } else {
            let mut sum = get(a, k);
            for j in 1..=k {
                sum += binomial(k, j) * get(w, j) * get(&g, k - j);
            }
            sum / lightest
        };
        if let Some(slot) = g.get_mut(k) {
            *slot = value;
        }
    }
    g
}

/// A Bernstein row reparametrised to `[a, b]` (any two values, also
/// outside `[0, 1]`): coefficient `k` of the new row is the blossom at
/// `a` taken `p − k` times and `b` taken `k` times.
fn reparametrise(row: &[Hom], a: f64, b: f64) -> Vec<Hom> {
    let p = row.len().saturating_sub(1);
    (0..=p)
        .map(|k| {
            let mut work = row.to_vec();
            for level in 0..p {
                let t = if level < k { b } else { a };
                for i in 0..p - level {
                    let (Some(&x), Some(&y)) = (work.get(i), work.get(i + 1)) else {
                        continue;
                    };
                    if let Some(slot) = work.get_mut(i) {
                        for (s, (u, v)) in slot.iter_mut().zip(x.iter().zip(y.iter())) {
                            *s = (1.0 - t) * u + t * v;
                        }
                    }
                }
            }
            work.first().copied().unwrap_or([0.0; 4])
        })
        .collect()
}

/// A homogeneous Bézier net over `[0, 1]²` of degrees `(m, n)` (a curve:
/// `n = 0`), coefficient `(i, j)` at `i (n + 1) + j`.
struct Net {
    m: usize,
    n: usize,
    c: Vec<Hom>,
}

impl Net {
    fn from_points(m: usize, n: usize, points: &[(Vec3, f64)], origin: Vec3) -> Option<Net> {
        if points.len() != (m + 1).checked_mul(n + 1)? {
            return None;
        }
        let c = points
            .iter()
            .map(|(p, w)| {
                let d = (*p - origin) * *w;
                [d.x, d.y, d.z, *w]
            })
            .collect();
        Some(Net { m, n, c })
    }

    /// The net of the same polynomial over `[u0, u1] × [v0, v1]`.
    fn cut(&self, (u0, u1): (f64, f64), (v0, v1): (f64, f64)) -> Net {
        let width = self.n + 1;
        let mut c = self.c.clone();
        for j in 0..width {
            let column: Vec<Hom> = (0..=self.m)
                .filter_map(|i| self.c.get(i * width + j).copied())
                .collect();
            for (i, h) in reparametrise(&column, u0, u1).into_iter().enumerate() {
                if let Some(slot) = c.get_mut(i * width + j) {
                    *slot = h;
                }
            }
        }
        if self.n > 0 {
            for i in 0..=self.m {
                let Some(row) = c.get(i * width..(i + 1) * width) else {
                    continue;
                };
                let new = reparametrise(row, v0, v1);
                for (j, h) in new.into_iter().enumerate() {
                    if let Some(slot) = c.get_mut(i * width + j) {
                        *slot = h;
                    }
                }
            }
        }
        Net {
            m: self.m,
            n: self.n,
            c,
        }
    }

    /// Bounds on the partial derivatives over `[0, 1]²` of the weighted
    /// points and of the weight (the hull bounds of the differences of the
    /// net), how far the patch is from the origin at most, and the least
    /// weight; `None` unless every weight is positive and finite.
    fn partials(&self) -> Option<(Table, Table, f64, f64)> {
        let mut lightest = f64::INFINITY;
        let mut reach = 0.0_f64;
        for h in &self.c {
            let [x, y, z, w] = *h;
            if !(w.is_finite() && w > 0.0) {
                return None;
            }
            lightest = lightest.min(w);
            reach = reach.max(Vec3::new(x, y, z).length() / w);
        }
        let (mut points, mut weights): (Table, Table) = ([[0.0; N]; N], [[0.0; N]; N]);
        let width = self.n + 1;
        // Differences along u, i times; then along v, j times.
        let mut along_u: Vec<Hom> = self.c.clone();
        let mut rows = self.m + 1;
        for i in 0..N.min(self.m + 1) {
            if i > 0 {
                along_u = (0..rows - 1)
                    .flat_map(|r| {
                        let here = &along_u;
                        (0..width).map(move |j| {
                            let a = here.get(r * width + j).copied().unwrap_or([0.0; 4]);
                            let b = here.get((r + 1) * width + j).copied().unwrap_or([0.0; 4]);
                            [b[0] - a[0], b[1] - a[1], b[2] - a[2], b[3] - a[3]]
                        })
                    })
                    .collect();
                rows -= 1;
            }
            let mut grid = along_u.clone();
            let mut cols = width;
            for j in 0..(N - i).min(self.n + 1) {
                if j > 0 {
                    grid = (0..rows)
                        .flat_map(|r| {
                            let here = &grid;
                            (0..cols - 1).map(move |k| {
                                let a = here.get(r * cols + k).copied().unwrap_or([0.0; 4]);
                                let b = here.get(r * cols + k + 1).copied().unwrap_or([0.0; 4]);
                                [b[0] - a[0], b[1] - a[1], b[2] - a[2], b[3] - a[3]]
                            })
                        })
                        .collect();
                    cols -= 1;
                }
                let scale = falling(self.m, i) * falling(self.n, j);
                let (mut p, mut w) = (0.0_f64, 0.0_f64);
                for h in &grid {
                    p = p.max(Vec3::new(h[0], h[1], h[2]).length());
                    w = w.max(h[3].abs());
                }
                if let Some(slot) = points.get_mut(i).and_then(|r| r.get_mut(j)) {
                    *slot = scale * p;
                }
                if let Some(slot) = weights.get_mut(i).and_then(|r| r.get_mut(j)) {
                    *slot = scale * w;
                }
            }
        }
        (lightest.is_finite() && reach.is_finite()).then_some((points, weights, reach, lightest))
    }
}

/// The cubic Hermite path `σ` in a surface's parameters over `τ ∈ [0, 1]`:
/// per coordinate the coefficients of `c0 + c1 τ + c2 τ² + c3 τ³`.
struct Path {
    c: [[f64; 4]; 2],
}

impl Path {
    fn new(a: (f64, f64), b: (f64, f64), ta: (f64, f64), tb: (f64, f64)) -> Path {
        let coefficients = |pa: f64, pb: f64, da: f64, db: f64| {
            [
                pa,
                da,
                3.0 * (pb - pa) - 2.0 * da - db,
                2.0 * (pa - pb) + da + db,
            ]
        };
        Path {
            c: [
                coefficients(a.0, b.0, ta.0, tb.0),
                coefficients(a.1, b.1, ta.1, tb.1),
            ],
        }
    }

    /// The range of each coordinate over `[0, 1]`: within the hull of its
    /// Bernstein coefficients.
    fn range(&self) -> [(f64, f64); 2] {
        self.c.map(|[c0, c1, c2, c3]| {
            let b = [
                c0,
                c0 + c1 / 3.0,
                c0 + 2.0 * c1 / 3.0 + c2 / 3.0,
                c0 + c1 + c2 + c3,
            ];
            (
                b.iter().copied().fold(f64::INFINITY, f64::min),
                b.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            )
        })
    }

    /// Bounds on the derivatives (orders 1 to 4) of each coordinate over
    /// `[0, 1]`.
    fn jets(&self) -> [Jet; 2] {
        self.c.map(|[_, c1, c2, c3]| {
            let first = |t: f64| (c1 + 2.0 * c2 * t + 3.0 * c3 * t * t).abs();
            let mut d1 = first(0.0).max(first(1.0));
            if c3 != 0.0 {
                let t = -c2 / (3.0 * c3);
                if t > 0.0 && t < 1.0 {
                    d1 = d1.max(first(t));
                }
            }
            let d2 = (2.0 * c2).abs().max((2.0 * c2 + 6.0 * c3).abs());
            [0.0, d1, d2, (6.0 * c3).abs(), 0.0]
        })
    }
}

/// A piece still to prove: its ends on the curve and their feet.
type Piece = (f64, f64, (f64, f64), (f64, f64));

/// A box in a surface's parameters, as `(u range, v range)`.
type Box2 = [(f64, f64); 2];

/// What the proof knows of a surface: its kind, or its Bézier patches.
/// How a spline surface goes on across the line between two spans: `C^k`
/// with `smooth` = k, and by how much it may jump there (zero but where
/// a full knot leaves two borders that differ by rounding).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Join {
    smooth: usize,
    jump: f64,
}

enum Form {
    Analytic,
    Patches {
        spans: Vec<BezierSpan>,
        /// The distinct spans along u and along v (the patches are these,
        /// u by u, v by v within each).
        us: Vec<(f64, f64)>,
        vs: Vec<(f64, f64)>,
        /// How smooth the surface is across the line between span `i`
        /// and span `i + 1` along u (and along v): the degree less the
        /// knot's multiplicity, so `Some(k)` for `C^k`; `None` where the
        /// surface may jump (a multiplicity above the degree).
        joins_u: Vec<Option<Join>>,
        joins_v: Vec<Option<Join>>,
    },
}

/// A surface made ready for proofs about curves near it.
pub struct SurfaceBound<'a> {
    surface: &'a Surface,
    form: Form,
    /// For a spline surface, the size of what enters its evaluation: the
    /// largest control point times the spread of the weights.
    size: f64,
}

/// A curve made ready for proofs: its Bézier pieces, or its kind.
pub struct CurveBound<'a> {
    curve: CurveRef<'a>,
    pieces: Vec<CurvePiece>,
    /// For a spline curve, the size of what enters its evaluation (see
    /// [`SurfaceBound`]).
    size: f64,
}

/// Units in the last place allowed for the rounding of one evaluation, of
/// the size of what enters it: de Boor's and de Casteljau's algorithms
/// are convex combinations (each step loses at most an ulp or two of the
/// largest point), the analytic kinds a few products and a sine.
const ROUNDING_ULPS: f64 = 64.0;

/// The size of what enters the evaluation of a spline: the largest
/// control point times the heaviest over the lightest weight.
fn net_size(points: &[Vec3], weights: &[f64]) -> f64 {
    let largest = points.iter().map(|p| p.length()).fold(0.0, f64::max);
    let heaviest = weights.iter().copied().fold(0.0, f64::max);
    let lightest = weights.iter().copied().fold(f64::INFINITY, f64::min);
    if lightest > 0.0 {
        largest * heaviest / lightest
    } else {
        f64::INFINITY
    }
}

#[derive(Clone, Copy)]
enum CurveRef<'a> {
    Curve(&'a Curve3),
    Nurbs(&'a NurbsCurve),
}

/// A bound proven: every point of the curve over the range lies within
/// `bound` of the surface; `pieces` is how many pieces the proof took.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Proven {
    pub bound: f64,
    pub pieces: usize,
}

/// No bound within the target could be proven in the pieces allowed:
/// `at` is the curve's parameter in the middle of the piece where the
/// proof stopped (where the curve is, or may be, too far from the
/// surface).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NotProven {
    pub at: f64,
}

impl<'a> CurveBound<'a> {
    pub fn new(curve: &'a Curve3) -> CurveBound<'a> {
        let pieces = match curve {
            Curve3::Nurbs(c) => c.bezier_pieces(),
            Curve3::Line(_) | Curve3::Circle(_) | Curve3::Ellipse(_) => Vec::new(),
        };
        let size = match curve {
            Curve3::Nurbs(c) => net_size(c.control_points(), c.weights()),
            Curve3::Line(_) | Curve3::Circle(_) | Curve3::Ellipse(_) => 0.0,
        };
        CurveBound {
            curve: CurveRef::Curve(curve),
            pieces,
            size,
        }
    }

    pub fn nurbs(curve: &'a NurbsCurve) -> CurveBound<'a> {
        CurveBound {
            curve: CurveRef::Nurbs(curve),
            pieces: curve.bezier_pieces(),
            size: net_size(curve.control_points(), curve.weights()),
        }
    }

    fn derivatives(&self, t: f64) -> (Vec3, Vec3) {
        match self.curve {
            CurveRef::Curve(c) => c.derivatives(t),
            CurveRef::Nurbs(c) => c.derivatives(t),
        }
    }

    fn eval(&self, t: f64) -> Vec3 {
        self.derivatives(t).0
    }

    /// How far a point of the curve near `t` may be off by rounding.
    fn rounding(&self, t: f64) -> f64 {
        let size = match self.curve {
            CurveRef::Curve(Curve3::Line(l)) => {
                l.origin().length() + t.abs() * l.direction().length()
            }
            CurveRef::Curve(Curve3::Circle(c)) => c.frame().origin().length() + c.radius(),
            CurveRef::Curve(Curve3::Ellipse(e)) => {
                let (x, y) = e.semi_axes();
                e.frame().origin().length() + x.abs().max(y.abs())
            }
            CurveRef::Curve(Curve3::Nurbs(_)) | CurveRef::Nurbs(_) => self.size,
        };
        ROUNDING_ULPS * f64::EPSILON * size
    }

    /// The parameters where the curve's derivatives may jump (its knots),
    /// strictly inside `(lo, hi)`.
    fn breaks(&self, lo: f64, hi: f64) -> Vec<f64> {
        self.pieces
            .iter()
            .skip(1)
            .map(|((start, _), _)| *start)
            .filter(|t| *t > lo && *t < hi)
            .collect()
    }

    /// Bounds on the derivatives in `τ` (orders 0 to 4; order 0 about
    /// `origin`) of the curve over `[a, b]`, which lies in one piece.
    fn jets(&self, a: f64, b: f64, origin: Vec3) -> Option<Jet> {
        let h = b - a;
        let powers = |r: f64| -> Jet {
            let mut out = [0.0; N];
            for (k, slot) in out.iter_mut().enumerate().skip(1) {
                *slot = r * h.abs().ppowi(k as i32);
            }
            out
        };
        match self.curve {
            CurveRef::Curve(Curve3::Line(l)) => {
                let mut out = [0.0; N];
                if let Some(slot) = out.get_mut(1) {
                    *slot = l.direction().length() * h.abs();
                }
                return Some(out);
            }
            CurveRef::Curve(Curve3::Circle(c)) => return Some(powers(c.radius())),
            CurveRef::Curve(Curve3::Ellipse(e)) => {
                let (x, y) = e.semi_axes();
                return Some(powers(x.abs().max(y.abs())));
            }
            CurveRef::Curve(Curve3::Nurbs(_)) | CurveRef::Nurbs(_) => {}
        }
        let middle = 0.5 * (a + b);
        let count = self.pieces.len();
        let index = self
            .pieces
            .iter()
            .position(|((_, hi), _)| middle <= *hi)
            .unwrap_or(count.saturating_sub(1));
        let ((lo, hi), points) = self.pieces.get(index)?;
        let width = hi - lo;
        if !(width > 0.0) {
            return None;
        }
        let degree = points.len().checked_sub(1)?;
        let net = Net::from_points(degree, 0, points, origin)?
            .cut(((a - lo) / width, (b - lo) / width), (0.0, 1.0));
        let (p, w, reach, lightest) = net.partials()?;
        let column = |t: &Table| -> Jet {
            let mut out = [0.0; N];
            for (k, slot) in out.iter_mut().enumerate() {
                *slot = entry(t, k, 0);
            }
            out
        };
        Some(quotient(&column(&p), &column(&w), reach, lightest))
    }
}

/// Bounds on the partial derivatives of an analytic surface over a box
/// (order 0 is not needed: no quotient is taken).
fn analytic_table(surface: &Surface, [_, (v0, v1)]: Box2) -> Option<Table> {
    let mut t: Table = [[0.0; N]; N];
    let mut set = |i: usize, j: usize, x: f64| {
        if let Some(slot) = t.get_mut(i).and_then(|r| r.get_mut(j)) {
            *slot = x;
        }
    };
    match surface {
        // S = o + u x + v y.
        Surface::Plane(_) => {
            set(1, 0, 1.0);
            set(0, 1, 1.0);
        }
        // S = o + r (cos u x + sin u y) + v z.
        Surface::Cylinder(c) => {
            for i in 1..N {
                set(i, 0, c.radius());
            }
            set(0, 1, 1.0);
        }
        // S = o + ρ(v) (cos u x + sin u y) + v z, ρ = R + v tan α.
        Surface::Cone(c) => {
            let slope = c.half_angle().ptan().abs();
            let widest = c.radius_at(v0).abs().max(c.radius_at(v1).abs());
            for i in 1..N {
                set(i, 0, widest);
                set(i, 1, slope);
            }
            set(0, 1, (1.0 + slope * slope).sqrt());
        }
        // S = c + r (cos v cos u, cos v sin u, sin v): every partial of
        // order one or more is at most r long.
        Surface::Sphere(s) => {
            for i in 0..N {
                for j in 0..N - i {
                    if i + j > 0 {
                        set(i, j, s.radius());
                    }
                }
            }
        }
        // S = c + (R + r cos v)(cos u, sin u, 0) + r sin v z.
        Surface::Torus(tor) => {
            let (major, minor) = tor.radii();
            for i in 0..N {
                for j in 0..N - i {
                    if i + j > 0 {
                        set(i, j, if j == 0 { major.abs() + minor } else { minor });
                    }
                }
            }
        }
        Surface::Bezier(_) | Surface::Nurbs(_) => return None,
    }
    Some(t)
}

/// The indices of the spans of `ranges` (in order, the first and the last
/// going on past the ends) that `[lo, hi]` reaches.
fn spans_reached(ranges: &[(f64, f64)], lo: f64, hi: f64) -> Option<(usize, usize)> {
    let last = ranges.len().checked_sub(1)?;
    let first = ranges.iter().position(|(_, end)| lo < *end).unwrap_or(last);
    let until = ranges
        .iter()
        .position(|(_, end)| hi <= *end)
        .unwrap_or(last);
    Some((first, until.max(first)))
}

/// How smooth a spline is across the line between each span of `spans`
/// and the next: `Some(degree − multiplicity)` of the knot there (`C^k`),
/// `None` where the multiplicity is above the degree (the spline may
/// jump) or the knot is not found.
fn joins(spans: &[(f64, f64)], knots: &[f64], degree: usize) -> Vec<Option<Join>> {
    spans
        .windows(2)
        .map(|pair| {
            let [(_, end), _] = pair else {
                return None;
            };
            let multiplicity = knots.iter().filter(|k| **k == *end).count();
            if multiplicity == 0 {
                return None;
            }
            degree
                .checked_sub(multiplicity)
                .map(|smooth| Join { smooth, jump: 0.0 })
        })
        .collect()
}

/// Lines between spans where a knot of a multiplicity above the degree
/// lets the surface jump, but the patches on both sides end in the same
/// border curve up to rounding (a writer that ends every span with a full
/// knot, as some write rational surfaces): continuous there, `C⁰`, with
/// the jump bounded from the two border rows. Where the rows are rational
/// curves `A / a` and `B / b` (homogeneous control points), the borders
/// differ by at most `(max |A − B| + max |B| / min b · max |a − b|) /
/// min a` (Bernstein forms are convex combinations); any jump is added to
/// the bounds across that line, so a border that is not shared is never
/// taken as one.
fn seal(
    spans: &[BezierSpan],
    count_v: usize,
    joins_u: &mut [Option<Join>],
    joins_v: &mut [Option<Join>],
) {
    let patch = |iu: usize, iv: usize| {
        iu.checked_mul(count_v)
            .and_then(|k| k.checked_add(iv))
            .and_then(|k| spans.get(k))
            .map(|s| &s.patch)
    };
    // The jump between the rows `ia` of `a` and `ib` of `b` (indices of
    // control points), `None` when it cannot be bounded.
    let jump = |a: &RationalBezierPatch, ia: &[usize], b: &RationalBezierPatch, ib: &[usize]| {
        let (wa, wb) = (a.weights(), b.weights());
        let (mut dh, mut dw, mut big, mut light_a, mut light_b) =
            (0.0_f64, 0.0_f64, 0.0_f64, f64::INFINITY, f64::INFINITY);
        if ia.len() != ib.len() || ia.is_empty() {
            return None;
        }
        for (&i, &j) in ia.iter().zip(ib) {
            let (pa, pb) = (a.control_point(i)?, b.control_point(j)?);
            let (xa, xb) = (*wa.get(i)?, *wb.get(j)?);
            dh = dh.max((pa * xa - pb * xb).length());
            dw = dw.max((xa - xb).abs());
            big = big.max((pb * xb).length());
            light_a = light_a.min(xa);
            light_b = light_b.min(xb);
        }
        if !(light_a > 0.0 && light_b > 0.0) {
            return None;
        }
        let d = (dh + big / light_b * dw) / light_a;
        // Plus the rounding of both evaluations.
        let d = d + ROUNDING_ULPS * f64::EPSILON * big / light_b;
        d.is_finite().then_some(d)
    };
    let count_u = spans.len().checked_div(count_v).unwrap_or(0);
    for (iu, join) in joins_u.iter_mut().enumerate() {
        if join.is_some() {
            continue;
        }
        let mut worst = Some(0.0_f64);
        for iv in 0..count_v {
            let d = match (patch(iu, iv), patch(iu + 1, iv)) {
                (Some(a), Some(b)) if a.degrees().1 == b.degrees().1 => {
                    let (m, n) = a.degrees();
                    let ia: Vec<usize> = (0..=n).map(|j| m * (n + 1) + j).collect();
                    let ib: Vec<usize> = (0..=n).collect();
                    jump(a, &ia, b, &ib)
                }
                _ => None,
            };
            worst = worst.zip(d).map(|(w, d)| w.max(d));
        }
        *join = worst.map(|jump| Join { smooth: 0, jump });
    }
    for (iv, join) in joins_v.iter_mut().enumerate() {
        if join.is_some() {
            continue;
        }
        let mut worst = Some(0.0_f64);
        for iu in 0..count_u {
            let d = match (patch(iu, iv), patch(iu, iv + 1)) {
                (Some(a), Some(b)) if a.degrees().0 == b.degrees().0 => {
                    let (m, n) = a.degrees();
                    let n2 = b.degrees().1;
                    let ia: Vec<usize> = (0..=m).map(|i| i * (n + 1) + n).collect();
                    let ib: Vec<usize> = (0..=m).map(|i| i * (n2 + 1)).collect();
                    jump(a, &ia, b, &ib)
                }
                _ => None,
            };
            worst = worst.zip(d).map(|(w, d)| w.max(d));
        }
        *join = worst.map(|jump| Join { smooth: 0, jump });
    }
}

/// The least smoothness across the lines between spans `first` to
/// `last` (`usize::MAX` within one span), and the sum of their jumps;
/// `None` if any may jump by more than can be bounded.
fn smoothness(joins: &[Option<Join>], first: usize, last: usize) -> Option<(usize, f64)> {
    let mut least = usize::MAX;
    let mut jumps = 0.0;
    for i in first..last {
        let join = (*joins.get(i)?)?;
        least = least.min(join.smooth);
        jumps += join.jump;
    }
    Some((least, jumps))
}

impl<'a> SurfaceBound<'a> {
    /// `None` for a spline surface that cannot be split into Bézier
    /// patches.
    pub fn new(surface: &'a Surface) -> Option<SurfaceBound<'a>> {
        let form = match surface {
            Surface::Plane(_)
            | Surface::Cylinder(_)
            | Surface::Cone(_)
            | Surface::Sphere(_)
            | Surface::Torus(_) => Form::Analytic,
            Surface::Bezier(b) => Form::Patches {
                spans: vec![BezierSpan {
                    u: (0.0, 1.0),
                    v: (0.0, 1.0),
                    patch: RationalBezierPatch::from_bezier(b),
                }],
                us: vec![(0.0, 1.0)],
                vs: vec![(0.0, 1.0)],
                joins_u: Vec::new(),
                joins_v: Vec::new(),
            },
            Surface::Nurbs(s) => {
                let spans = s.bezier_spans().ok()?;
                let mut us: Vec<(f64, f64)> = Vec::new();
                let mut vs: Vec<(f64, f64)> = Vec::new();
                for span in &spans {
                    if us.last() != Some(&span.u) {
                        us.push(span.u);
                    }
                    if us.len() == 1 {
                        vs.push(span.v);
                    }
                }
                if us.len().checked_mul(vs.len())? != spans.len() {
                    return None;
                }
                let (du, dv) = s.degrees();
                let mut joins_u = joins(&us, s.knots_u(), du);
                let mut joins_v = joins(&vs, s.knots_v(), dv);
                seal(&spans, vs.len(), &mut joins_u, &mut joins_v);
                Form::Patches {
                    spans,
                    us,
                    vs,
                    joins_u,
                    joins_v,
                }
            }
        };
        let size = match surface {
            Surface::Bezier(b) => {
                net_size(b.control_points(), &vec![1.0; b.control_points().len()])
            }
            Surface::Nurbs(n) => net_size(n.control_points(), n.weights()),
            Surface::Plane(_)
            | Surface::Cylinder(_)
            | Surface::Cone(_)
            | Surface::Sphere(_)
            | Surface::Torus(_) => 0.0,
        };
        Some(SurfaceBound {
            surface,
            form,
            size,
        })
    }

    /// How far a point of the surface near `(u, v)` may be off by rounding.
    fn rounding(&self, (u, v): (f64, f64)) -> f64 {
        let size = match self.surface {
            Surface::Plane(p) => p.frame().origin().length() + u.abs() + v.abs(),
            Surface::Cylinder(c) => c.frame().origin().length() + c.radius() + v.abs(),
            Surface::Cone(c) => c.frame().origin().length() + c.radius_at(v).abs() + v.abs(),
            Surface::Sphere(s) => s.frame().origin().length() + s.radius(),
            Surface::Torus(t) => {
                let (major, minor) = t.radii();
                t.frame().origin().length() + major.abs() + minor
            }
            Surface::Bezier(_) | Surface::Nurbs(_) => self.size,
        };
        ROUNDING_ULPS * f64::EPSILON * size
    }

    /// The parameters of the foot of `p` on the surface: Gauss-Newton
    /// steps from `start`, which keep the parameters continuous (round a
    /// seam they go on past it). Where the steps do not settle, the best
    /// found: any parameters make a sound bound, a poor foot only a loose
    /// one.
    fn foot(&self, p: Vec3, start: (f64, f64)) -> (f64, f64) {
        let (mut u, mut v) = start;
        let mut best = (self.surface.eval(u, v).distance(p), (u, v));
        if !best.0.is_finite() {
            return start;
        }
        for _ in 0..FOOT_STEPS {
            let (s, su, sv) = self.surface.derivatives(u, v);
            let r = p - s;
            let (mut a, b, mut c) = (su.dot(su), su.dot(sv), sv.dot(sv));
            // At a singular point of the parametrisation (a pole, an
            // apex) the steps have no direction along the singular
            // border and would fly off along it: damped there
            // (Levenberg-Marquardt), so the step goes where the surface
            // moves and the parameter along the border stays.
            if a * c - b * b <= SINGULAR * (a + c) * (a + c) {
                let damping = SINGULAR.sqrt() * (a + c);
                (a, c) = (a + damping, c + damping);
            }
            let det = a * c - b * b;
            if !(det > 0.0) || !det.is_finite() {
                break;
            }
            let (ru, rv) = (su.dot(r), sv.dot(r));
            let (du, dv) = ((c * ru - b * rv) / det, (a * rv - b * ru) / det);
            let (nu, nv) = (u + du, v + dv);
            if !(nu.is_finite() && nv.is_finite()) {
                break;
            }
            (u, v) = (nu, nv);
            let d = self.surface.eval(u, v).distance(p);
            if d < best.0 {
                best = (d, (u, v));
            }
            if (su * du + sv * dv).length() <= 1e-15 * (1.0 + s.length()) {
                break;
            }
        }
        // The same point, named by the parameters nearest the start round
        // a periodic direction: the path between two feet must not go
        // round the surface (or round a pole) to get from one to the
        // other.
        let (mut u, mut v) = best.1;
        let near = |x: f64, from: f64, period: Option<f64>| match period {
            Some(p) if p > 0.0 && p.is_finite() && (x - from).abs() > 0.5 * p => {
                let turns = ((from - x) / p).round();
                let moved = x + turns * p;
                if moved.is_finite() { moved } else { x }
            }
            _ => x,
        };
        u = near(u, start.0, self.surface.period_u());
        v = near(v, start.1, self.surface.period_v());
        (u, v)
    }

    /// Whether `(u, v)` lies on a border of a spline's parameter range
    /// (within a billionth of its width) across which the surface closes
    /// on itself (its two borders in that direction meet at this point):
    /// a seam, where the same point has two names. A border the surface
    /// does not close across (the side of a ruled surface, which an edge
    /// may run along) is no seam (2026-11-01: an edge along it was taken
    /// for one and its start never settled; ABC models).
    fn on_border(&self, (u, v): (f64, f64)) -> bool {
        let Form::Patches { us, vs, .. } = &self.form else {
            return false;
        };
        let ends = |ranges: &[(f64, f64)]| match (ranges.first(), ranges.last()) {
            (Some(&(lo, _)), Some(&(_, hi))) => Some((lo, hi)),
            _ => None,
        };
        let near = |x: f64, (lo, hi): (f64, f64)| {
            let width = (hi - lo).abs();
            (x - lo).abs() <= 1e-9 * width || (x - hi).abs() <= 1e-9 * width
        };
        let meet = |a: Vec3, b: Vec3| a.distance(b) <= 1e-9 * (1.0 + a.length());
        let across_u = ends(us).is_some_and(|e| {
            near(u, e) && meet(self.surface.eval(e.0, v), self.surface.eval(e.1, v))
        });
        let across_v = ends(vs).is_some_and(|e| {
            near(v, e) && meet(self.surface.eval(u, e.0), self.surface.eval(u, e.1))
        });
        across_u || across_v
    }

    /// Whether the parametrisation is singular at `(u, v)`: `∂u × ∂v` has
    /// no length there (a sphere's pole, a cone's apex, a spline's border
    /// that is one point), so a foot there has no direction to step in
    /// and any `u` (or `v`) along the singular border is as good as any
    /// other.
    fn singular(&self, (u, v): (f64, f64)) -> bool {
        let (_, su, sv) = self.surface.derivatives(u, v);
        let scale = su.length() + sv.length();
        let cross = su.cross(sv).length();
        !(cross > 1e-9 * scale * scale) || !cross.is_finite()
    }

    /// The parameters to follow the feet from when the curve starts at a
    /// singular point of the surface: there the foot has no longitude,
    /// and feet followed from an arbitrary one stay stuck in it (the
    /// steps have no direction to go), so every bound would carry the
    /// whole jump in `u`. The foot of a point a little along the curve,
    /// where the parametrisation is regular, is taken instead and the
    /// start's foot found from it: at the singular point that changes
    /// only which of the equal parameters names it, and keeps the path
    /// continuous. `start` when the curve starts at a regular point, or
    /// no regular foot is found near the start.
    ///
    /// With `borders`, a start on a border of a spline's parameter range
    /// is taken the same way: a spline surface closed round (a revolved
    /// curve, an extruded circle) has its seam at both ends of the range,
    /// and the projection of a point on the seam may name the end the
    /// curve does not go on from; past it the surface goes on as the
    /// polynomial of its last piece, away from the curve.
    fn off_singular(
        &self,
        curve: &CurveBound<'_>,
        (lo, hi): (f64, f64),
        start: (f64, f64),
        borders: bool,
    ) -> (f64, f64) {
        let settled = |f: (f64, f64)| !self.singular(f) && !(borders && self.on_border(f));
        if settled(start) {
            return start;
        }
        for share in [1e-6, 1e-4, 1e-2, 0.125] {
            let t = lo + (hi - lo) * share;
            let Ok(near) = self.surface.project(curve.eval(t)) else {
                continue;
            };
            if !settled(near) {
                continue;
            }
            // Back to the start through points in between, so the foot
            // stays on the branch of the regular one.
            let mut hint = near;
            for k in (0..FOLLOW).rev() {
                let between = lo + (t - lo) * k as f64 / FOLLOW as f64;
                hint = self.foot(curve.eval(between), hint);
            }
            return hint;
        }
        start
    }

    /// Bounds on the derivatives in `τ` (orders 0 to 4; order 0 about
    /// `origin`) of `S(σ(τ))`, patch by patch where the path's box
    /// crosses knot lines, and how smooth `S(σ(τ))` is across them: `k`
    /// for `C^k` (`usize::MAX` within one patch). A bound on the
    /// derivative of order `j ≤ k + 1` taken patch by patch holds for the
    /// whole path (the derivative of order `j − 1` is continuous and
    /// piecewise differentiable); and the sum of the jumps of the lines
    /// crossed (`seal`). `None` where no bound comes out or the surface
    /// may jump within the box by more than can be bounded.
    fn jets(&self, path: &Path, origin: Vec3) -> Option<(Jet, usize, f64)> {
        let range = path.range();
        let [x, y] = path.jets();
        match &self.form {
            Form::Analytic => {
                let table = analytic_table(self.surface, range)?;
                Some((compose(&table, &x, &y), usize::MAX, 0.0))
            }
            Form::Patches {
                spans,
                us,
                vs,
                joins_u,
                joins_v,
            } => {
                let [(u0, u1), (v0, v1)] = range;
                let (iu0, iu1) = spans_reached(us, u0, u1)?;
                let (iv0, iv1) = spans_reached(vs, v0, v1)?;
                let (su, ju) = smoothness(joins_u, iu0, iu1)?;
                let (sv, jv) = smoothness(joins_v, iv0, iv1)?;
                let smooth = su.min(sv);
                let jumps = ju + jv;
                let last_u = us.len().checked_sub(1)?;
                let last_v = vs.len().checked_sub(1)?;
                let mut all = [0.0_f64; N];
                for iu in iu0..=iu1 {
                    for iv in iv0..=iv1 {
                        let span = spans.get(iu.checked_mul(vs.len())?.checked_add(iv)?)?;
                        // The part of the box in this patch (the first and
                        // the last go on past the domain), in its own
                        // parameters.
                        let local = |(lo, hi): (f64, f64),
                                     (b0, b1): (f64, f64),
                                     first: bool,
                                     last: bool|
                         -> Option<(f64, f64, f64)> {
                            let width = hi - lo;
                            if !(width > 0.0) {
                                return None;
                            }
                            let a = if first { b0 } else { b0.max(lo) };
                            let b = if last { b1 } else { b1.min(hi) };
                            let (mut a, mut b) = ((a - lo) / width, (b - lo) / width);
                            if !(b - a >= NARROWEST) {
                                let middle = 0.5 * (a + b);
                                a = middle - 0.5 * NARROWEST;
                                b = middle + 0.5 * NARROWEST;
                            }
                            Some((a, b, width))
                        };
                        let (a_u, b_u, wu) = local(span.u, (u0, u1), iu == 0, iu == last_u)?;
                        let (a_v, b_v, wv) = local(span.v, (v0, v1), iv == 0, iv == last_v)?;
                        let (m, n) = span.patch.degrees();
                        let points: Vec<(Vec3, f64)> = span
                            .patch
                            .control_points()
                            .into_iter()
                            .zip(span.patch.weights())
                            .collect();
                        let net =
                            Net::from_points(m, n, &points, origin)?.cut((a_u, b_u), (a_v, b_v));
                        let (p, w, reach, lightest) = net.partials()?;
                        // The path in the cut patch's own parameters.
                        let (su, sv) = ((b_u - a_u) * wu, (b_v - a_v) * wv);
                        let scaled = |d: &Jet, s: f64| -> Jet { d.map(|x| x / s) };
                        let (xl, yl) = (scaled(&x, su), scaled(&y, sv));
                        let jet = quotient(
                            &compose(&p, &xl, &yl),
                            &compose(&w, &xl, &yl),
                            reach,
                            lightest,
                        );
                        for (slot, value) in all.iter_mut().zip(jet) {
                            *slot = slot.max(value);
                        }
                    }
                }
                (all.iter().all(|x| x.is_finite()) && jumps.is_finite())
                    .then_some((all, smooth, jumps))
            }
        }
    }

    /// A bound on the distance from the surface of the curve over `[a, b]`
    /// (one piece of it), from the feet `fa` and `fb` of its ends.
    fn piece_bound(
        &self,
        curve: &CurveBound<'_>,
        a: f64,
        b: f64,
        fa: (f64, f64),
        fb: (f64, f64),
    ) -> Option<f64> {
        let h = b - a;
        let (ca, da) = curve.derivatives(a);
        let (cb, db) = curve.derivatives(b);
        let (sa, sua, sva) = self.surface.derivatives(fa.0, fa.1);
        let (sb, sub, svb) = self.surface.derivatives(fb.0, fb.1);
        let (ea, eb) = (ca - sa, cb - sb);
        // The path leaves each end along the curve's tangent projected
        // into the surface's tangent plane (in `τ`).
        let straight = (fb.0 - fa.0, fb.1 - fa.1);
        let along = |su: Vec3, sv: Vec3, d: Vec3| -> (f64, f64) {
            let (p, q, r) = (su.dot(su), su.dot(sv), sv.dot(sv));
            let det = p * r - q * q;
            let (x, y) = (su.dot(d), sv.dot(d));
            let t = ((r * x - q * y) / det, (p * y - q * x) / det);
            // At a singular point (a pole) the tangent plane's
            // parameters are not unique and the solution is all rounding:
            // the path goes straight from that end.
            if det > SINGULAR * (p + r) * (p + r) && t.0.is_finite() && t.1.is_finite() {
                t
            } else {
                straight
            }
        };
        let ta = along(sua, sva, da * h);
        let tb = along(sub, svb, db * h);
        let off_a = da * h - (sua * ta.0 + sva * ta.1);
        let off_b = db * h - (sub * tb.0 + svb * tb.1);
        let path = Path::new(fa, fb, ta, tb);
        let cj = curve.jets(a, b, ca)?;
        let (sj, smooth, jumps) = self.jets(&path, ca)?;
        // The curve is one Bézier piece over `[a, b]`; `S(σ(τ))` is
        // `C^smooth` across the knot lines the path crosses.
        // Each distance at an end is off by the rounding of both points.
        let rounding =
            (curve.rounding(a) + self.rounding(fa)).max(curve.rounding(b) + self.rounding(fb));
        let (na, nb) = (ea.length(), eb.length());
        // Lipschitz: holds for any continuous surface.
        // Where the surface may jump across a line the path crosses (a
        // cubic path crosses one at most three times), by the jumps.
        let mut bound = 0.5 * (na + nb + get(&cj, 1) + get(&sj, 1)) + 3.0 * jumps;
        if smooth >= 1 {
            // Linear interpolation, with the second derivative: its
            // Peano kernel needs `e` in `C¹` with `e''` bounded piece by
            // piece (2026-11-01: across the simple knots of a cubic
            // surface, where only the Lipschitz bound was used before).
            bound = bound.min(na.max(nb) + (get(&cj, 2) + get(&sj, 2)) / 8.0);
        }
        if smooth >= 3 {
            // Cubic Hermite interpolation, with the fourth (`e` in `C³`).
            bound = bound.min(
                na.max(nb)
                    + 0.25 * off_a.length().max(off_b.length())
                    + (get(&cj, 4) + get(&sj, 4)) / 384.0,
            );
        }
        let bound = bound + rounding;
        bound.is_finite().then_some(bound)
    }

    /// Proves that `curve` over `range` lies within `target` of the
    /// surface: the pieces start at the curve's knots and at `breaks`
    /// (where the caller knows the curve meets the surface, such as the
    /// points it was fitted through), and are halved where their bound is
    /// over the target, at most `most` pieces in all. `start` is the
    /// parameters of the surface near the curve's point at `range.0`.
    pub fn curve_within(
        &self,
        curve: &CurveBound<'_>,
        range: (f64, f64),
        start: (f64, f64),
        breaks: &[f64],
        target: f64,
        most: usize,
    ) -> Result<Proven, NotProven> {
        let (lo, hi) = range;
        if !(lo.is_finite() && hi.is_finite() && hi > lo && target.is_finite()) {
            return Err(NotProven { at: lo });
        }
        let mut points: Vec<f64> = vec![lo, hi];
        points.extend(curve.breaks(lo, hi));
        // Analytic curves have no knots: eight pieces to start from, so
        // that no foot is far from the one it starts from.
        if curve.pieces.is_empty() {
            points.extend((1..8).map(|k| lo + (hi - lo) * f64::from(k) / 8.0));
        }
        points.extend(breaks.iter().copied().filter(|t| *t > lo && *t < hi));
        points.sort_by(f64::total_cmp);
        points.dedup();
        // The feet, each followed from the one before through points in
        // between (a foot found from far away may be another one: on a
        // torus, the inside of the tube).
        let mut feet = Vec::with_capacity(points.len());
        let mut hint = self.off_singular(curve, range, start, false);
        let mut last = lo;
        for t in &points {
            for k in 1..FOLLOW {
                let between = last + (t - last) * k as f64 / FOLLOW as f64;
                hint = self.foot(curve.eval(between), hint);
            }
            hint = self.foot(curve.eval(*t), hint);
            feet.push(hint);
            last = *t;
        }
        let mut stack: Vec<Piece> = points
            .windows(2)
            .zip(feet.windows(2))
            .rev()
            .filter_map(|(t, f)| match (t, f) {
                ([a, b], [fa, fb]) => Some((*a, *b, *fa, *fb)),
                _ => None,
            })
            .collect();
        let mut proven = Proven {
            bound: 0.0,
            pieces: 0,
        };
        while let Some((a, b, fa, fb)) = stack.pop() {
            let middle = 0.5 * (a + b);
            match self.piece_bound(curve, a, b, fa, fb) {
                Some(bound) if bound <= target => {
                    proven.bound = proven.bound.max(bound);
                    proven.pieces += 1;
                    continue;
                }
                _ => {}
            }
            if proven.pieces + stack.len() + 2 > most || !(middle > a && middle < b) {
                return Err(NotProven { at: middle });
            }
            // From both ends' feet: one poor foot would spoil every
            // halving below it.
            let p = curve.eval(middle);
            let fm = [fa, fb]
                .map(|f| self.foot(p, f))
                .into_iter()
                .map(|f| (self.surface.eval(f.0, f.1).distance(p), f))
                .min_by(|x, y| x.0.total_cmp(&y.0))
                .map_or(fa, |(_, f)| f);
            stack.push((middle, b, fm, fb));
            stack.push((a, middle, fa, fm));
        }
        Ok(proven)
    }
}

/// How a proof that a curve lies on a surface is held: every point within
/// `target` of the surface, in at most `most` pieces
/// ([`curve_on_surface`]). The multiple of the tolerance is the caller's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Within {
    pub target: f64,
    pub most: usize,
}

/// Whether `curve` over `range` lies within `within.target` of `surface`,
/// proven, not sampled: the one answer to "this curve lies on this
/// surface" in the kernel ([`SurfaceBound::curve_within`] for the method,
/// and for a surface made ready once for many proofs).
///
/// `start` is the surface's parameters near the curve's point at
/// `range.0`; `None` takes the projection of that point. `breaks` are
/// where the caller knows the curve meets the surface (the points it was
/// fitted through). A surface that cannot be made ready for proofs (a
/// spline not split into Bézier patches), or a start that cannot be
/// found, is not proven, at the middle of the range.
pub fn curve_on_surface(
    curve: &CurveBound<'_>,
    range: (f64, f64),
    surface: &Surface,
    start: Option<(f64, f64)>,
    breaks: &[f64],
    within: Within,
) -> Result<Proven, NotProven> {
    let middle = NotProven {
        at: 0.5 * (range.0 + range.1),
    };
    let bound = SurfaceBound::new(surface).ok_or(middle)?;
    let start = match start {
        Some(start) => start,
        // A projection alone may name either end of a closed spline's
        // range at its seam: the one the curve goes on from.
        None => {
            let projected = surface.project(curve.eval(range.0)).map_err(|_| middle)?;
            bound.off_singular(curve, range, projected, true)
        }
    };
    bound.curve_within(curve, range, start, breaks, within.target, within.most)
}

/// How two curves lie, as far as [`curves_apart`] could tell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Apart {
    /// Proven: no point of one is within the threshold of the other.
    Yes,
    /// Seen: the points at these parameters are within the threshold.
    No { at: [f64; 2], distance: f64 },
    /// Neither, in the pairs of pieces allowed.
    Unknown,
}

/// The distance between the segments from `a0` to `a1` and from `b0` to
/// `b1`, with the parameters (0 to 1) of the nearest points.
fn segments(a0: Vec3, a1: Vec3, b0: Vec3, b1: Vec3) -> (f64, f64, f64) {
    let (d1, d2, r) = (a1 - a0, b1 - b0, a0 - b0);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (c, b) = (d1.dot(r), d1.dot(d2));
    let denominator = a * e - b * b;
    let mut s = if denominator > 0.0 {
        ((b * f - c * e) / denominator).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut t = if e > 0.0 { (b * s + f) / e } else { 0.0 };
    if t < 0.0 {
        t = 0.0;
        s = if a > 0.0 {
            (-c / a).clamp(0.0, 1.0)
        } else {
            0.0
        };
    } else if t > 1.0 {
        t = 1.0;
        s = if a > 0.0 {
            ((b - c) / a).clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    ((a0 + d1 * s).distance(b0 + d2 * t), s, t)
}

/// A piece of a curve for [`curves_apart`]: its range, its ends, and how
/// far it can be from the chord between them.
#[derive(Clone, Copy)]
struct Span {
    range: (f64, f64),
    ends: [Vec3; 2],
    reach: f64,
}

impl CurveBound<'_> {
    /// The piece over `[a, b]` (inside one Bézier piece): every point lies
    /// within `reach` of its chord, by the error of linear interpolation
    /// (`max |C''| / 8`), or `max |C'|` where that is less.
    fn span(&self, a: f64, b: f64) -> Option<Span> {
        let ends = [self.eval(a), self.eval(b)];
        let jet = self.jets(a, b, ends[0])?;
        let reach = (get(&jet, 2) / 8.0).min(get(&jet, 1));
        let rounding = self.rounding(a).max(self.rounding(b));
        let reach = reach + rounding;
        reach.is_finite().then_some(Span {
            range: (a, b),
            ends,
            reach,
        })
    }

    /// The pieces of `range` between its knots, each cut into `cuts`.
    fn spans(&self, (lo, hi): (f64, f64), cuts: usize) -> Option<Vec<Span>> {
        let mut points = vec![lo];
        points.extend(self.breaks(lo, hi));
        points.push(hi);
        let mut out = Vec::new();
        for pair in points.windows(2) {
            let [a, b] = pair else { continue };
            for k in 0..cuts {
                let x = a + (b - a) * k as f64 / cuts as f64;
                let y = a + (b - a) * (k + 1) as f64 / cuts as f64;
                out.push(self.span(x, y)?);
            }
        }
        Some(out)
    }
}

/// The box round a piece's capsule: its chord's ends, widened by its
/// reach.
fn capsule_box(span: &Span) -> (Vec3, Vec3) {
    let [p, q] = span.ends;
    let r = Vec3::new(span.reach, span.reach, span.reach);
    (
        Vec3::new(p.x.min(q.x), p.y.min(q.y), p.z.min(q.z)) - r,
        Vec3::new(p.x.max(q.x), p.y.max(q.y), p.z.max(q.z)) + r,
    )
}

/// The distance between two boxes (0 where they overlap).
fn box_distance((a0, a1): (Vec3, Vec3), (b0, b1): (Vec3, Vec3)) -> f64 {
    let gap = |lo_a: f64, hi_a: f64, lo_b: f64, hi_b: f64| (lo_b - hi_a).max(lo_a - hi_b).max(0.0);
    Vec3::new(
        gap(a0.x, a1.x, b0.x, b1.x),
        gap(a0.y, a1.y, b0.y, b1.y),
        gap(a0.z, a1.z, b0.z, b1.z),
    )
    .length()
}

/// A node of a [`Tree`]: the pieces it holds, its box, its children.
type TreeNode = (usize, usize, (Vec3, Vec3), Option<[usize; 2]>);

/// A tree of boxes over the pieces of a curve: node `k` holds pieces
/// `lo..hi` and the box round all their capsules; its children are the
/// two halves (none for one piece).
struct Tree {
    pieces: Vec<Span>,
    nodes: Vec<TreeNode>,
}

impl Tree {
    fn new(pieces: Vec<Span>) -> Option<Tree> {
        let mut tree = Tree {
            pieces,
            nodes: Vec::new(),
        };
        let count = tree.pieces.len();
        if count == 0 {
            return None;
        }
        tree.build(0, count)?;
        Some(tree)
    }

    /// Builds the node over `lo..hi` (and those below it); its index.
    /// The depth is the logarithm of the number of pieces.
    fn build(&mut self, lo: usize, hi: usize) -> Option<usize> {
        let index = self.nodes.len();
        self.nodes.push((lo, hi, (Vec3::ZERO, Vec3::ZERO), None));
        let (bbox, children) = if hi - lo == 1 {
            (capsule_box(self.pieces.get(lo)?), None)
        } else {
            let middle = lo + (hi - lo) / 2;
            let left = self.build(lo, middle)?;
            let right = self.build(middle, hi)?;
            let (l, r) = (self.nodes.get(left)?.2, self.nodes.get(right)?.2);
            (
                (
                    Vec3::new(l.0.x.min(r.0.x), l.0.y.min(r.0.y), l.0.z.min(r.0.z)),
                    Vec3::new(l.1.x.max(r.1.x), l.1.y.max(r.1.y), l.1.z.max(r.1.z)),
                ),
                Some([left, right]),
            )
        };
        let slot = self.nodes.get_mut(index)?;
        slot.2 = bbox;
        slot.3 = children;
        Some(index)
    }
}

/// One side of a pair looked at by [`curves_apart`]: a node of the tree,
/// or a piece made by halving a piece.
#[derive(Clone, Copy)]
enum Part {
    Node(usize),
    Piece(Span),
}

/// Whether the curve `a` over `range_a` and `b` over `range_b` keep more
/// than `threshold` apart everywhere, proven. Each is cut into pieces at
/// its knots, held within capsules round their chords (a radius from the
/// bound on their second derivatives), with a tree of boxes over them.
/// Pairs are looked at from the roots down: two whose boxes (or, for two
/// pieces, whose capsules) are more than the threshold apart are apart;
/// otherwise the larger side is split (a node into its halves, a piece
/// halved), unless the points of the curves nearest where two pieces'
/// chords meet are themselves within the threshold: then the curves come
/// that close ([`Apart::No`]). At most `most` pairs are looked at; past
/// that the answer is [`Apart::Unknown`].
pub fn curves_apart(
    a: &CurveBound<'_>,
    range_a: (f64, f64),
    b: &CurveBound<'_>,
    range_b: (f64, f64),
    threshold: f64,
    most: usize,
) -> Apart {
    let valid = |(lo, hi): (f64, f64)| lo.is_finite() && hi.is_finite() && hi > lo;
    if !(valid(range_a) && valid(range_b) && threshold.is_finite()) {
        return Apart::Unknown;
    }
    // Analytic curves have no knots: eight pieces to start from.
    let cuts = |c: &CurveBound<'_>| if c.pieces.is_empty() { 8 } else { 1 };
    let (Some(ta), Some(tb)) = (
        a.spans(range_a, cuts(a)).and_then(Tree::new),
        b.spans(range_b, cuts(b)).and_then(Tree::new),
    ) else {
        return Apart::Unknown;
    };
    // A part's box, its size (what is split first), and its children.
    let bbox = |tree: &Tree, part: &Part| -> Option<(Vec3, Vec3)> {
        match part {
            Part::Node(k) => tree.nodes.get(*k).map(|n| n.2),
            Part::Piece(span) => Some(capsule_box(span)),
        }
    };
    let as_piece = |tree: &Tree, part: &Part| -> Option<Span> {
        match part {
            Part::Node(k) => {
                let (lo, hi, _, children) = tree.nodes.get(*k)?;
                if children.is_some() || hi - lo != 1 {
                    return None;
                }
                tree.pieces.get(*lo).copied()
            }
            Part::Piece(span) => Some(*span),
        }
    };
    let split = |curve: &CurveBound<'_>, tree: &Tree, part: &Part| -> Option<[Part; 2]> {
        if let Part::Node(k) = part
            && let Some([l, r]) = tree.nodes.get(*k)?.3
        {
            return Some([Part::Node(l), Part::Node(r)]);
        }
        let span = as_piece(tree, part)?;
        let (lo, hi) = span.range;
        let middle = 0.5 * (lo + hi);
        if !(middle > lo && middle < hi) {
            return None;
        }
        Some([
            Part::Piece(curve.span(lo, middle)?),
            Part::Piece(curve.span(middle, hi)?),
        ])
    };
    let size = |b: (Vec3, Vec3)| (b.1 - b.0).length();
    let mut stack = vec![(Part::Node(0), Part::Node(0))];
    let mut looked = 0_usize;
    while let Some((x, y)) = stack.pop() {
        looked += 1;
        if looked > most {
            return Apart::Unknown;
        }
        let (Some(bx), Some(by)) = (bbox(&ta, &x), bbox(&tb, &y)) else {
            return Apart::Unknown;
        };
        if box_distance(bx, by) > threshold {
            continue;
        }
        if let (Some(px), Some(py)) = (as_piece(&ta, &x), as_piece(&tb, &y)) {
            let [x0, x1] = px.ends;
            let [y0, y1] = py.ends;
            let (chords, s, t) = segments(x0, x1, y0, y1);
            // For nearly parallel chords the closest points are found
            // through `a e − b²`, which cancels: the distance is taken as
            // up to √ε of the chords' lengths less than computed.
            let cancel = f64::EPSILON.sqrt() * (x0.distance(x1) + y0.distance(y1));
            if chords - cancel - px.reach - py.reach > threshold {
                continue;
            }
            let at = [
                px.range.0 + s * (px.range.1 - px.range.0),
                py.range.0 + t * (py.range.1 - py.range.0),
            ];
            let distance = a.eval(at[0]).distance(b.eval(at[1]));
            if distance <= threshold {
                return Apart::No { at, distance };
            }
        }
        // Split the larger side.
        if size(bx) >= size(by) {
            let Some([l, r]) = split(a, &ta, &x) else {
                return Apart::Unknown;
            };
            stack.push((r, y));
            stack.push((l, y));
        } else {
            let Some([l, r]) = split(b, &tb, &y) else {
                return Apart::Unknown;
            };
            stack.push((x, r));
            stack.push((x, l));
        }
    }
    Apart::Yes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BezierPatch, Cone, Cylinder, NurbsSurface, Plane, Sphere, Torus};
    use craft3d_math::Frame;

    fn frame(k: f64) -> Frame {
        Frame::new(
            Vec3::new(0.3 * k, -0.2 * k, 0.1 * k),
            Vec3::new(0.2, 0.3, 1.0),
            Vec3::new(1.0, -0.1, 0.0),
        )
        .unwrap()
    }

    /// The farthest a curve is from the surface at many points, each by
    /// its foot from the previous one.
    fn sampled(
        sb: &SurfaceBound<'_>,
        cb: &CurveBound<'_>,
        range: (f64, f64),
        start: (f64, f64),
    ) -> f64 {
        let mut hint = start;
        let mut worst = 0.0_f64;
        for k in 0..=4000 {
            let t = range.0 + (range.1 - range.0) * f64::from(k) / 4000.0;
            let p = cb.eval(t);
            hint = sb.foot(p, hint);
            worst = worst.max(sb.surface.eval(hint.0, hint.1).distance(p));
        }
        worst
    }

    /// A cubic through points of `surface` at `params`, lifted off it by
    /// `bump` times a bump that is zero at every fit point and at every
    /// middle between them.
    fn curve_on(
        surface: &Surface,
        path: impl Fn(f64) -> (f64, f64),
        count: usize,
        bump: f64,
        k: f64,
    ) -> (NurbsCurve, Vec<f64>) {
        let params: Vec<f64> = (0..=count).map(|i| i as f64 / count as f64).collect();
        let fine: Vec<f64> = (0..=8 * count)
            .map(|i| i as f64 / (8 * count) as f64)
            .collect();
        let points: Vec<Vec<f64>> = fine
            .iter()
            .map(|t| {
                let (u, v) = path(*t);
                let (p, su, sv) = surface.derivatives(u, v);
                let n = su.cross(sv).normalized().unwrap();
                // Zero at every multiple of 1 / (2 count): the fit points
                // and the middles between them.
                let lift = bump
                    * k
                    * (std::f64::consts::TAU * count as f64 * t).psin().ppowi(2)
                    * (std::f64::consts::PI * 2.0 * count as f64 * t).psin();
                let q = p + n * lift;
                vec![q.x, q.y, q.z]
            })
            .collect();
        let (knots, rows) = crate::refine::interpolate(&fine, 3, &points).unwrap();
        let ctrl = rows.iter().map(|r| Vec3::new(r[0], r[1], r[2])).collect();
        (NurbsCurve::polynomial(3, knots, ctrl).unwrap(), params)
    }

    /// A path in a surface's parameters, over `t` from 0 to 1.
    type Along = Box<dyn Fn(f64) -> (f64, f64)>;

    fn surfaces(k: f64) -> Vec<(Surface, Along)> {
        let f = frame(k);
        let wavy = {
            let mut ctrl = Vec::new();
            for i in 0..4 {
                for j in 0..4 {
                    let (x, y) = (f64::from(i), f64::from(j));
                    ctrl.push(Vec3::new(
                        x * k,
                        y * k,
                        0.3 * k * (1.7 * x + 0.9 * y).psin(),
                    ));
                }
            }
            BezierPatch::new(3, 3, ctrl).unwrap()
        };
        // A rational spline surface: a quarter of a cylinder in v (a
        // rational quadratic), three cubic spans along u.
        let rational = {
            let w = std::f64::consts::FRAC_1_SQRT_2;
            let mut ctrl = Vec::new();
            let mut weights = Vec::new();
            for i in 0..6 {
                let x = f64::from(i) * k;
                let r = (1.0 + 0.1 * f64::from(i * i)) * k;
                for (p, wt) in [((r, 0.0), 1.0), ((r, r), w), ((0.0, r), 1.0)] {
                    ctrl.push(Vec3::new(x, p.0, p.1 + 0.05 * x));
                    weights.push(wt);
                }
            }
            NurbsSurface::new(
                3,
                2,
                vec![0.0, 0.0, 0.0, 0.0, 0.3, 0.6, 1.0, 1.0, 1.0, 1.0],
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                6,
                3,
                ctrl,
                weights,
            )
            .unwrap()
        };
        vec![
            (
                Surface::Plane(Plane::new(f)),
                Box::new(move |t| (k * t, 0.4 * k * t * t)),
            ),
            (
                Surface::Cylinder(Cylinder::new(f, 2.0 * k).unwrap()),
                Box::new(move |t| (5.0 * t, k * (1.0 - t * t))),
            ),
            (
                Surface::Cone(Cone::new(f, 2.0 * k, 0.4).unwrap()),
                Box::new(move |t| (4.0 * t + 1.0, k * (0.5 + t))),
            ),
            (
                Surface::Sphere(Sphere::new(f, 3.0 * k).unwrap()),
                Box::new(|t| (5.0 * t, 1.2 * t - 0.6)),
            ),
            (
                Surface::Torus(Torus::new(f, 4.0 * k, k).unwrap()),
                Box::new(|t| (3.0 * t, 6.0 * t)),
            ),
            (
                Surface::Bezier(wavy),
                Box::new(|t| (t, 0.5 + 0.4 * (3.0 * t).psin())),
            ),
            (
                Surface::Nurbs(rational),
                Box::new(|t| (t, 0.1 + 0.8 * t * t)),
            ),
        ]
    }

    #[test]
    fn a_fitted_curve_on_a_surface_is_proven_near_it_and_the_bound_is_above_every_sample() {
        for k in [1e-3, 1.0, 1e6] {
            for (surface, path) in surfaces(k) {
                let (curve, params) = curve_on(&surface, &path, 16, 0.0, k);
                let sb = SurfaceBound::new(&surface).unwrap();
                let cb = CurveBound::nurbs(&curve);
                let target = 1e-6 * k;
                let proven = sb
                    .curve_within(&cb, (0.0, 1.0), path(0.0), &params, target, 4096)
                    .unwrap_or_else(|e| panic!("{} at {k}: {e:?}", surface.kind()));
                let seen = sampled(&sb, &cb, (0.0, 1.0), path(0.0));
                assert!(
                    proven.bound >= seen,
                    "{}: {proven:?} below {seen}",
                    surface.kind()
                );
                assert!(proven.bound <= target);
            }
        }
    }

    #[test]
    fn a_curve_that_leaves_the_surface_between_its_middles_is_not_proven_near_it() {
        for k in [1e-3, 1.0, 1e6] {
            for (surface, path) in surfaces(k) {
                // Lifted by up to 1e-5 between the samples, exactly on
                // the surface at the fit points and the middles.
                let (curve, params) = curve_on(&surface, &path, 16, 1e-5, k);
                let sb = SurfaceBound::new(&surface).unwrap();
                let cb = CurveBound::nurbs(&curve);
                // At the middles the sampled check saw nothing.
                for pair in params.windows(2) {
                    let t = 0.5 * (pair[0] + pair[1]);
                    let p = cb.eval(t);
                    let foot = sb.foot(p, path(t));
                    assert!(sb.surface.eval(foot.0, foot.1).distance(p) <= 1e-9 * k);
                }
                let seen = sampled(&sb, &cb, (0.0, 1.0), path(0.0));
                assert!(seen > 2e-6 * k, "{}: {seen}", surface.kind());
                let refused = sb.curve_within(&cb, (0.0, 1.0), path(0.0), &params, 1e-6 * k, 4096);
                assert!(refused.is_err(), "{}: {refused:?}", surface.kind());
                // A target above how far it goes is proven, and the bound
                // is above what is seen.
                let proven = sb
                    .curve_within(&cb, (0.0, 1.0), path(0.0), &params, 2e-5 * k, 4096)
                    .unwrap_or_else(|e| panic!("{} at {k}: {e:?}", surface.kind()));
                assert!(proven.bound >= seen);
            }
        }
    }

    #[test]
    fn the_bounds_hold_for_curves_off_the_surface_by_any_amount() {
        // Bound above the sampled distance for curves lifted by large and
        // small bumps, with few pieces allowed or many.
        for (surface, path) in surfaces(1.0) {
            for bump in [1e-9, 1e-4, 1e-2, 0.2] {
                let (curve, params) = curve_on(&surface, &path, 5, bump, 1.0);
                let sb = SurfaceBound::new(&surface).unwrap();
                let cb = CurveBound::nurbs(&curve);
                let seen = sampled(&sb, &cb, (0.0, 1.0), path(0.0));
                if let Ok(p) = sb.curve_within(&cb, (0.0, 1.0), path(0.0), &params, 1.0, 64) {
                    assert!(
                        p.bound >= seen,
                        "{} {bump}: {p:?} below {seen}",
                        surface.kind()
                    );
                }
            }
        }
    }

    #[test]
    fn a_bad_range_or_target_is_not_proven_and_does_not_panic() {
        let surface = Surface::Plane(Plane::new(frame(1.0)));
        let line = Curve3::Line(crate::Line::new(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)).unwrap());
        let sb = SurfaceBound::new(&surface).unwrap();
        let cb = CurveBound::new(&line);
        for (range, target) in [
            ((1.0, 0.0), 1.0),
            ((0.0, f64::NAN), 1.0),
            ((0.0, 1.0), f64::NAN),
            ((0.0, f64::INFINITY), 1.0),
        ] {
            assert!(
                sb.curve_within(&cb, range, (0.0, 0.0), &[], target, 64)
                    .is_err()
            );
        }
        assert!(
            sb.curve_within(&cb, (0.0, 1.0), (0.0, 0.0), &[], 1e-9, 0)
                .is_err()
        );
    }

    #[test]
    fn the_jets_of_a_composition_bound_the_derivatives_seen() {
        // F(u, v) = sin(u) cos(2 v) along a cubic path: the majorant bounds
        // are above the derivatives by finite differences.
        let mut table: Table = [[0.0; N]; N];
        for (i, row) in table.iter_mut().enumerate() {
            for (j, slot) in row.iter_mut().enumerate() {
                if i + j < N {
                    *slot = 2.0_f64.ppowi(j as i32);
                }
            }
        }
        let path = Path::new((0.1, 0.2), (0.9, -0.4), (1.3, 0.2), (-0.4, 0.7));
        let [x, y] = path.jets();
        let jet = compose(&table, &x, &y);
        let f = |t: f64| {
            let u = path.c[0][0] + t * (path.c[0][1] + t * (path.c[0][2] + t * path.c[0][3]));
            let v = path.c[1][0] + t * (path.c[1][1] + t * (path.c[1][2] + t * path.c[1][3]));
            u.psin() * (2.0 * v).pcos()
        };
        let h = 1e-2;
        for k in 0..=20 {
            let t = 0.05 + 0.9 * f64::from(k) / 20.0;
            let d1 = (f(t + h) - f(t - h)) / (2.0 * h);
            let d2 = (f(t + h) - 2.0 * f(t) + f(t - h)) / (h * h);
            let d4 = (f(t + 2.0 * h) - 4.0 * f(t + h) + 6.0 * f(t) - 4.0 * f(t - h)
                + f(t - 2.0 * h))
                / h.ppowi(4);
            assert!(d1.abs() <= jet[1] * 1.01, "{d1} {}", jet[1]);
            assert!(d2.abs() <= jet[2] * 1.01, "{d2} {}", jet[2]);
            assert!(d4.abs() <= jet[4] * 1.05, "{d4} {}", jet[4]);
        }
    }

    /// A cubic that bows down towards a line, its lowest point `gap`
    /// above it, between the 64 chords a sampled check would take.
    fn bow(gap: f64, k: f64) -> NurbsCurve {
        let params: Vec<f64> = (0..=40).map(|i| f64::from(i) / 40.0).collect();
        let values: Vec<Vec<f64>> = params
            .iter()
            .map(|t| vec![t * k, (gap + 4e-2 * (t - 0.507).ppowi(2)) * k, 0.0])
            .collect();
        let (knots, rows) = crate::refine::interpolate(&params, 3, &values).unwrap();
        let ctrl = rows.iter().map(|r| Vec3::new(r[0], r[1], r[2])).collect();
        NurbsCurve::polynomial(3, knots, ctrl).unwrap()
    }

    #[test]
    fn two_curves_are_proven_apart_or_seen_close_and_never_called_apart_when_close() {
        for k in [1e-3, 1.0, 1e6] {
            let line = Curve3::Line(crate::Line::new(Vec3::ZERO, Vec3::X).unwrap());
            let lb = CurveBound::new(&line);
            let threshold = 1e-5 * k;
            for (gap, expect_apart) in [(5e-6, false), (9e-6, false), (2e-5, true), (1e-3, true)] {
                let curve = bow(gap, k);
                let cb = CurveBound::nurbs(&curve);
                let found = curves_apart(&cb, (0.0, 1.0), &lb, (0.0, k), threshold, 1 << 16);
                // The least distance, densely.
                let least = (0..=100_000)
                    .map(|i| cb.eval(f64::from(i) / 100_000.0).y.abs())
                    .fold(f64::INFINITY, f64::min);
                match found {
                    Apart::Yes => {
                        assert!(expect_apart, "{k} {gap}");
                        assert!(least > threshold);
                    }
                    Apart::No { distance, .. } => {
                        assert!(!expect_apart, "{k} {gap}");
                        assert!(distance <= threshold);
                    }
                    Apart::Unknown => panic!("{k} {gap}: unknown"),
                }
            }
        }
    }

    #[test]
    fn curves_apart_takes_bad_ranges_without_panicking() {
        let line = Curve3::Line(crate::Line::new(Vec3::ZERO, Vec3::X).unwrap());
        let lb = CurveBound::new(&line);
        for r in [(1.0, 0.0), (0.0, f64::NAN), (f64::NEG_INFINITY, 0.0)] {
            assert_eq!(
                curves_apart(&lb, r, &lb, (0.0, 1.0), 1.0, 64),
                Apart::Unknown
            );
        }
        // One pair allowed of the many to look at: unknown.
        let curve = bow(9e-6, 1.0);
        let cb = CurveBound::nurbs(&curve);
        assert_eq!(
            curves_apart(&cb, (0.0, 1.0), &lb, (0.0, 1.0), 1e-5, 1),
            Apart::Unknown
        );
    }

    /// Curves a known distance `d` from a surface everywhere: lines,
    /// circles, an ellipse and rational spline circles, on every analytic
    /// kind.
    fn at_distance(d: f64) -> Vec<(&'static str, Surface, Curve3, (f64, f64))> {
        let f = frame(1.0);
        let around = |height: f64| Frame::new(f.point(0.0, 0.0, height), f.z(), f.x()).unwrap();
        let circle = |height: f64, radius: f64| {
            Curve3::Circle(crate::Circle::new(around(height), radius).unwrap())
        };
        let spline = |height: f64, radius: f64| {
            let c = NurbsCurve::circle(&around(height), radius).unwrap();
            let range = c.domain();
            (Curve3::Nurbs(c), range)
        };
        let full = (0.0, std::f64::consts::TAU);
        let (slope, cone_v) = (0.4_f64, 1.0);
        let cone_r = 2.0 + cone_v * slope.ptan();
        let lat = 0.4_f64;
        let (rs, rs_range) = spline(0.0, 2.0 + d);
        let (rt, rt_range) = spline(1.0 + d, 4.0);
        vec![
            (
                "line over a plane",
                Surface::Plane(Plane::new(f)),
                Curve3::Line(crate::Line::new(f.point(-1.0, 0.5, d), f.x()).unwrap()),
                (0.0, 3.0),
            ),
            (
                "circle over a plane",
                Surface::Plane(Plane::new(f)),
                circle(d, 2.0),
                full,
            ),
            (
                "ellipse over a plane",
                Surface::Plane(Plane::new(f)),
                Curve3::Ellipse(crate::Ellipse::new(around(d), 3.0, 1.0).unwrap()),
                full,
            ),
            (
                "circle round a cylinder",
                Surface::Cylinder(Cylinder::new(f, 2.0).unwrap()),
                circle(1.0, 2.0 + d),
                full,
            ),
            (
                "line along a cylinder",
                Surface::Cylinder(Cylinder::new(f, 2.0).unwrap()),
                Curve3::Line(crate::Line::new(f.point(2.0 + d, 0.0, -1.0), f.z()).unwrap()),
                (0.0, 3.0),
            ),
            (
                "spline circle round a cylinder",
                Surface::Cylinder(Cylinder::new(f, 2.0).unwrap()),
                rs,
                rs_range,
            ),
            (
                "circle round a cone",
                Surface::Cone(Cone::new(f, 2.0, slope).unwrap()),
                circle(cone_v, cone_r + d / slope.pcos()),
                full,
            ),
            (
                "circle at a latitude of a sphere",
                Surface::Sphere(Sphere::new(f, 3.0).unwrap()),
                circle((3.0 + d) * lat.psin(), (3.0 + d) * lat.pcos()),
                (0.3, 2.9),
            ),
            (
                "circle round a torus",
                Surface::Torus(Torus::new(f, 4.0, 1.0).unwrap()),
                circle(0.0, 5.0 + d),
                full,
            ),
            (
                "spline circle over a torus",
                Surface::Torus(Torus::new(f, 4.0, 1.0).unwrap()),
                rt,
                rt_range,
            ),
        ]
    }

    /// The one entry gives what the method on a surface made ready gives,
    /// bit for bit, with the start its projection when none is given.
    #[test]
    fn curve_on_surface_is_the_method() {
        for d in [1e-2, 1e-5] {
            for (name, surface, curve, range) in at_distance(d) {
                let sb = SurfaceBound::new(&surface).unwrap();
                let cb = CurveBound::new(&curve);
                let start = surface.project(curve.eval(range.0)).unwrap();
                for target in [1.5 * d, 0.95 * d] {
                    let within = Within {
                        target,
                        most: 1 << 12,
                    };
                    let method = sb.curve_within(&cb, range, start, &[], target, 1 << 12);
                    for given in [Some(start), None] {
                        let entry = curve_on_surface(&cb, range, &surface, given, &[], within);
                        assert_eq!(
                            format!("{entry:?}"),
                            format!("{method:?}"),
                            "{name} at {d}, target {target}"
                        );
                    }
                }
            }
        }
    }

    /// A bicubic surface of 16 x 16 spans whose interior knots have
    /// multiplicity `m` (so it is `C^(3 − m)` across them), wavy.
    fn many_spans(k: f64, m: usize) -> Surface {
        let spans = 16;
        let mut knots = vec![0.0; 4];
        for i in 1..spans {
            knots.extend(std::iter::repeat_n(i as f64 / spans as f64, m));
        }
        knots.extend([1.0; 4]);
        let count = knots.len() - 4;
        let mut ctrl = Vec::new();
        for i in 0..count {
            for j in 0..count {
                let (x, y) = (i as f64 / count as f64, j as f64 / count as f64);
                ctrl.push(Vec3::new(
                    x * k,
                    y * k,
                    0.1 * k * (7.0 * x + 3.0 * y).psin() * (5.0 * y).pcos(),
                ));
            }
        }
        let weights = vec![1.0; ctrl.len()];
        Surface::Nurbs(
            NurbsSurface::new(3, 3, knots.clone(), knots, count, count, ctrl, weights).unwrap(),
        )
    }

    /// Curves that cross many knot lines of a spline surface, or run
    /// along one, are proven with bounds of the second (across `C¹` and
    /// `C²` lines) or fourth order (2026-11-01: only the Lipschitz bound,
    /// first order, was used across a knot line, so a curve along one
    /// needed pieces shorter than the target; ABC models from Parasolid
    /// were refused for it), and the bounds stay above what is seen.
    #[test]
    fn curves_across_and_along_knot_lines_are_proven() {
        let paths: Vec<(&str, Along)> = vec![
            ("across", Box::new(|t| (0.05 + 0.9 * t, 0.1 + 0.8 * t * t))),
            ("along", Box::new(|t| (0.5, 0.05 + 0.9 * t))),
            ("near", Box::new(|t| (0.5 + 1e-9 * t, 0.05 + 0.9 * t))),
        ];
        for k in [1e-3, 1.0, 1e6] {
            for m in [1, 2] {
                let surface = many_spans(k, m);
                let sb = SurfaceBound::new(&surface).unwrap();
                for (name, path) in &paths {
                    let (curve, params) = curve_on(&surface, path, 64, 0.0, k);
                    let cb = CurveBound::nurbs(&curve);
                    let seen = sampled(&sb, &cb, (0.0, 1.0), path(0.0));
                    let proven = sb
                        .curve_within(&cb, (0.0, 1.0), path(0.0), &[], 1e-6 * k, 1 << 12)
                        .unwrap_or_else(|e| panic!("{name}, multiplicity {m}, at {k}: {e:?}"));
                    // The Lipschitz bound alone took 1700 to 3200 pieces
                    // along the line at a tenfold target.
                    assert!(proven.pieces <= 2048, "{name}, {m}: {proven:?}");
                    assert!(proven.bound >= seen, "{name}: {proven:?} below {seen}");
                    // Lifted between the fit points: never proven below
                    // what is seen, and above it the bound holds it.
                    let (lifted, _) = curve_on(&surface, path, 64, 1e-4, k);
                    let lb = CurveBound::nurbs(&lifted);
                    let seen = sampled(&sb, &lb, (0.0, 1.0), path(0.0));
                    assert!(seen > 2e-5 * k, "{name}: {seen}");
                    let refused =
                        sb.curve_within(&lb, (0.0, 1.0), path(0.0), &params, 0.9 * seen, 1 << 12);
                    assert!(refused.is_err(), "{name}, {m}: {refused:?} below {seen}");
                    if let Ok(p) =
                        sb.curve_within(&lb, (0.0, 1.0), path(0.0), &params, 2.0 * seen, 1 << 14)
                    {
                        assert!(p.bound >= seen, "{name}: {p:?} below {seen}");
                    }
                }
            }
        }
    }

    /// A ruled surface between two circles, each of `n` rational quadratic
    /// arcs ending in a knot of multiplicity `m` (3: a full knot, so the
    /// surface could jump at every arc's end), radius `10 k`, the second
    /// half of the arcs moved out by `jump`; and the line on it at `u`.
    fn arcs(n: usize, m: usize, k: f64, jump: f64, u: f64) -> (Surface, Curve3) {
        let half = std::f64::consts::PI / n as f64;
        let w = half.pcos();
        let mut ring: Vec<(f64, f64, f64, f64)> = Vec::new();
        for i in 0..n {
            let r = if i >= n / 2 {
                10.0 * k + jump
            } else {
                10.0 * k
            };
            let a0 = 2.0 * half * i as f64;
            let am = a0 + half;
            // Each arc's start from its own angle: at a full knot it
            // differs from the end of the arc before by rounding.
            if i == 0 || m == 3 {
                ring.push((r * a0.pcos(), r * a0.psin(), 1.0, 1.0));
            }
            ring.push((r * am.pcos() / w, r * am.psin() / w, w, 0.0));
            let a1 = a0 + 2.0 * half;
            ring.push((r * a1.pcos(), r * a1.psin(), 1.0, 1.0));
        }
        let mut knots = vec![0.0; 3];
        for i in 1..n {
            knots.extend(std::iter::repeat_n(i as f64, m));
        }
        knots.extend([n as f64; 3]);
        let count = ring.len();
        let mut points = Vec::new();
        let mut weights = Vec::new();
        for z in [0.0, 5.0 * k] {
            for (x, y, wt, _) in &ring {
                points.push(Vec3::new(*x, *y, z));
                weights.push(*wt);
            }
        }
        let surface = Surface::Nurbs(
            NurbsSurface::new(
                1,
                2,
                vec![0.0, 0.0, 1.0, 1.0],
                knots.clone(),
                2,
                count,
                points,
                weights,
            )
            .unwrap(),
        );
        let line: Vec<Vec3> = ring
            .iter()
            .map(|(x, y, _, _)| Vec3::new(*x, *y, u * 5.0 * k))
            .collect();
        let lw = ring.iter().map(|p| p.2).collect();
        (
            surface,
            Curve3::Nurbs(NurbsCurve::new(2, knots, line, lw).unwrap()),
        )
    }

    /// A curve on a spline surface whose spans each end in a knot of
    /// multiplicity above the degree is proven when the spans share their
    /// borders up to rounding (2026-11-01: never proven before, the
    /// surface "may jump" at every such knot; ABC models from Parasolid
    /// write rational surfaces so), and a real jump is never taken for a
    /// shared border.
    #[test]
    fn full_knots_with_shared_borders_are_crossed() {
        for k in [1e-3, 1.0, 1e3] {
            for m in [2, 3] {
                let (surface, line) = arcs(96, m, k, 0.0, 0.43);
                let range = (0.1, 95.7);
                let proven = curve_on_surface(
                    &CurveBound::new(&line),
                    range,
                    &surface,
                    None,
                    &[],
                    Within {
                        target: 1e-6 * k,
                        most: 1 << 14,
                    },
                );
                assert!(
                    proven.is_ok_and(|p| p.bound <= 1e-6 * k),
                    "{m} at {k}: {proven:?}"
                );
            }
            // The second half moved out by 1e-4: the line of the first
            // half's radius is that far off it, and a bound below that
            // is never proven, whatever the joins say.
            let (surface, _) = arcs(96, 3, k, 1e-4 * k, 0.43);
            let (_, line) = arcs(96, 3, k, 0.0, 0.43);
            for target in [1e-6 * k, 0.9e-4 * k] {
                let refused = curve_on_surface(
                    &CurveBound::new(&line),
                    (0.1, 95.7),
                    &surface,
                    None,
                    &[],
                    Within {
                        target,
                        most: 1 << 14,
                    },
                );
                assert!(refused.is_err(), "{target:e} at {k}: {refused:?}");
            }
        }
    }

    /// An edge along the side of a ruled ring (a border of the spline's
    /// range across which it does not close), starting on the ring's
    /// seam, is proven: only a border the surface closes across is a seam
    /// whose start must be stepped off (2026-11-01: the side was taken
    /// for one, the start never settled and may have named the far end of
    /// the seam; ABC models on rational rings).
    #[test]
    fn an_edge_along_the_side_of_a_ring_from_its_seam_is_proven() {
        for k in [1e-3, 1.0, 1e3] {
            for u in [0.0, 1.0] {
                let (surface, side) = arcs(96, 2, k, 0.0, u);
                let sb = SurfaceBound::new(&surface).unwrap();
                let cb = CurveBound::new(&side);
                let range = (0.0, 95.7);
                // The start's foot named by the far end of the seam, as a
                // projection may name it.
                let start = sb.off_singular(&cb, range, (u, 96.0), true);
                assert!(start.1 < 1.0, "{u} at {k}: {start:?}");
                let proven = sb.curve_within(&cb, range, start, &[], 1e-6 * k, 1 << 14);
                assert!(
                    proven.is_ok_and(|p| p.bound <= 1e-6 * k),
                    "{u} at {k}: {proven:?}"
                );
            }
        }
    }

    /// Every bound is at least the exact distance, and a target just
    /// under it (0.95 of it) is never proven.
    #[test]
    fn bounds_are_never_below_the_exact_distance() {
        for d in [1e-2, 1e-5] {
            for (name, surface, curve, range) in at_distance(d) {
                let sb = SurfaceBound::new(&surface).unwrap();
                let cb = CurveBound::new(&curve);
                let start = surface.project(curve.eval(range.0)).unwrap();
                let proven = sb
                    .curve_within(&cb, range, start, &[], 1.5 * d, 1 << 16)
                    .unwrap_or_else(|e| panic!("{name} at {d}: {e:?}"));
                assert!(
                    proven.bound >= d * (1.0 - 1e-9),
                    "{name}: {proven:?} below {d}"
                );
                assert!(
                    sb.curve_within(&cb, range, start, &[], 0.95 * d, 1 << 16)
                        .is_err(),
                    "{name}: 0.95 of {d} proven"
                );
            }
        }
    }

    /// Curves that start, end or pass at a singular point of the
    /// parametrisation (a sphere's pole, a cone's apex, a revolved
    /// spline's pole) are proven like any other: the feet keep one
    /// longitude through the point (2026-10-21: a meridian starting at a
    /// pole was not proven at all, its feet stuck at the pole).
    #[test]
    fn curves_through_poles_and_apexes_are_proven() {
        let h = std::f64::consts::FRAC_PI_2;
        let at = Vec3::new(1.0, 2.0, 3.0);
        let frame = Frame::new(at, Vec3::Z, Vec3::X).unwrap();
        let tilted = Frame::new(at, Vec3::new(0.3, 1.0, 0.0), Vec3::new(1.0, -0.3, 0.0)).unwrap();
        let sphere = Surface::Sphere(Sphere::new(frame, 10.0).unwrap());
        let meridian = Curve3::Circle(crate::Circle::new(tilted, 10.0).unwrap());
        let cone = Surface::Cone(Cone::new(frame, 0.0, 0.5).unwrap());
        let up = Vec3::new(0.5f64.ptan() * 0.6, 0.5f64.ptan() * 0.8, 1.0);
        let ruling = Curve3::Line(crate::Line::new(at, up).unwrap());
        // Running down to the apex on the same nappe.
        let falling = Curve3::Line(crate::Line::new(at, -up).unwrap());
        // A dome: a quarter circle from the axis turned about it, as a
        // rational spline whose first row of control points is one point.
        let dome = {
            let s = std::f64::consts::FRAC_1_SQRT_2;
            let ring = [
                (1.0, 0.0, 1.0),
                (1.0, 1.0, s),
                (0.0, 1.0, 1.0),
                (-1.0, 1.0, s),
                (-1.0, 0.0, 1.0),
                (-1.0, -1.0, s),
                (0.0, -1.0, 1.0),
                (1.0, -1.0, s),
                (1.0, 0.0, 1.0),
            ];
            let arc = [(0.0, 10.0, 1.0), (10.0, 10.0, s), (10.0, 0.0, 1.0)];
            let mut points = Vec::new();
            let mut weights = Vec::new();
            for (x, y, w) in ring {
                for (rho, z, own) in arc {
                    points.push(at + Vec3::new(rho * x, rho * y, z));
                    weights.push(w * own);
                }
            }
            Surface::Nurbs(
                NurbsSurface::new(
                    2,
                    2,
                    vec![
                        0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
                    ],
                    vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    9,
                    3,
                    points,
                    weights,
                )
                .unwrap(),
            )
        };
        let upright = Frame::new(at, Vec3::new(0.6, -0.8, 0.0), Vec3::new(0.8, 0.6, 0.0)).unwrap();
        let over_the_top = Curve3::Circle(crate::Circle::new(upright, 10.0).unwrap());
        let cases: Vec<(&str, &Surface, &Curve3, (f64, f64))> = vec![
            ("meridian from the pole", &sphere, &meridian, (h, 3.0)),
            ("meridian to the pole", &sphere, &meridian, (0.0, h)),
            ("meridian over the pole", &sphere, &meridian, (0.2, 2.5)),
            ("ruling from the apex", &cone, &ruling, (0.0, 5.0)),
            ("ruling to the apex", &cone, &falling, (-3.0, 0.0)),
            ("arc over a dome's pole", &dome, &over_the_top, (0.3, 2.8)),
            ("arc from a dome's pole", &dome, &over_the_top, (h, 2.8)),
        ];
        for (name, surface, curve, range) in cases {
            let proven = curve_on_surface(
                &CurveBound::new(curve),
                range,
                surface,
                None,
                &[],
                Within {
                    target: 1e-6,
                    most: 1 << 10,
                },
            );
            assert!(proven.is_ok_and(|p| p.bound <= 1e-6), "{name}: {proven:?}");
        }
        // Off the surface by more than the target near the pole: never
        // proven within it.
        let off = Curve3::Circle(crate::Circle::new(tilted, 10.01).unwrap());
        let proven = curve_on_surface(
            &CurveBound::new(&off),
            (h, 3.0),
            &sphere,
            None,
            &[],
            Within {
                target: 1e-3,
                most: 1 << 10,
            },
        );
        assert!(proven.is_err(), "{proven:?}");
    }
}
