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
//! - **A second proof with careful feet.** Where the halving ends on a
//!   piece that cannot be halved further and the curve is not seen off
//!   the surface, the proof is made once more with every foot the nearest
//!   of the steps' and the surface's own projection (one within the
//!   surface's range before one past a pole or an apex), and each piece's
//!   end named near its start and a turn either way round the axis. By a
//!   pole or an apex the steps cross the singular point and settle on a
//!   point named past it (a cone's other nappe, the same point of a
//!   sphere named across its pole) or a turn away, and a piece between
//!   such feet is never proven however short. A curve the first proof
//!   proves is proven by it alone, as before.
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

/// How a spline surface goes on across the line between two spans: `C^k`
/// with `smooth` = k, and by how much it may jump there (zero but where
/// a full knot leaves two borders that differ by rounding).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Join {
    smooth: usize,
    jump: f64,
}

/// What the proof knows of a surface: its kind, or its Bézier patches.
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

/// Which borders of a spline's range a start is stepped off
/// (`SurfaceBound::off_singular`): none (singular points only), any, or
/// only those the surface closes across (seams).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Borders {
    No,
    Any,
    Seams,
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
        // Rows equal bit for bit: one border, as at any knot line (its
        // evaluation from either patch is held by the rounding the
        // bounds already carry), and so past the domain too.
        if dh == 0.0 && dw == 0.0 {
            return Some(0.0);
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
    /// (within a billionth of its width); with `seams_only`, only on one
    /// across which the surface closes on itself (its two borders in that
    /// direction meet at this point): a seam, where the same point has two
    /// names. A border the surface does not close across (the side of a
    /// ruled surface, which an edge may run along) is no seam (2026-11-01:
    /// an edge along it never settled off a border; ABC models).
    fn on_border(&self, (u, v): (f64, f64), seams_only: bool) -> bool {
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
        // Scale-free: against the size of what enters the evaluation
        // (`size`, the net's largest control point times the spread of
        // its weights), never an absolute unit.
        let meet =
            |a: Vec3, b: Vec3| !seams_only || a.distance(b) <= 1e-9 * self.size.max(a.length());
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
        borders: Borders,
    ) -> (f64, f64) {
        let settled = |f: (f64, f64)| {
            !self.singular(f)
                && match borders {
                    Borders::No => true,
                    Borders::Any => !self.on_border(f, false),
                    Borders::Seams => !self.on_border(f, true),
                }
        };
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
                // A sealed line's jump is bounded from its border rows by
                // the convex hull of the Bernstein forms, which holds
                // only over the domain of the other direction: past it
                // (the end patches go on as their polynomials) the
                // borders may part further, so a box that crosses a
                // sealed line and leaves the domain across it gets no
                // bound (review of 2026-11-02). A line whose rows agree
                // bit for bit has no jump anywhere.
                let outside = |(lo, hi): (f64, f64), ranges: &[(f64, f64)]| match (
                    ranges.first(),
                    ranges.last(),
                ) {
                    (Some(first), Some(last)) => lo < first.0 || hi > last.1,
                    _ => true,
                };
                if (ju > 0.0 && outside((v0, v1), vs)) || (jv > 0.0 && outside((u0, u1), us)) {
                    return None;
                }
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
    ///
    /// With careful feet a point is singular for the path's tangent where
    /// `∂u` and `∂v` are parallel to a billionth (`|∂u × ∂v|² ≤ SINGULAR
    /// |∂u|² |∂v|²`, which holds at a pole or an apex, where one of them
    /// is nought); the first proof compares `|∂u × ∂v|²` with
    /// `(|∂u|² + |∂v|²)²`, which also holds wherever one is a billion
    /// times longer than the other (a cone or a cylinder a billion times
    /// its speed along the axis from it), so that the path leaves every
    /// end straight there and a proof takes tens of thousands of pieces.
    fn piece_bound(
        &self,
        curve: &CurveBound<'_>,
        (a, b): (f64, f64),
        fa: (f64, f64),
        fb: (f64, f64),
        feet: Feet,
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
            let regular = match feet {
                Feet::Followed => det > SINGULAR * (p + r) * (p + r),
                Feet::Careful => det > SINGULAR * p * r,
            };
            if regular && t.0.is_finite() && t.1.is_finite() {
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
    ///
    /// Where the halving ends on a piece that cannot be halved further,
    /// on a surface other than a plane, and the curve is not seen off the
    /// surface ([`SurfaceBound::seen_off`]), the proof is tried once more
    /// with the feet found carefully ([`Feet::Careful`]): near a pole or an
    /// apex the feet followed by Gauss-Newton steps alone may land across
    /// the singular point, or be named a turn apart round the axis, and a
    /// piece between two such feet is never proven, however short. What
    /// the first proof proves is the answer as before, bit for bit and at
    /// the same cost; what it does not is proven only by a proof of its
    /// own, and otherwise refused at the place the first proof stopped,
    /// as before. Where the pieces allowed or the work
    /// (`crate::allowance`) run out, nothing is tried again.
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
        let stop = match self.prove(curve, range, start, breaks, target, most, Feet::Followed) {
            Ok(proven) => return Ok(proven),
            Err(stop) => stop,
        };
        let refused = NotProven { at: stop.at };
        // On a plane the steps find the foot in closed form: careful feet
        // would be the same.
        if !stop.again
            || matches!(self.surface, Surface::Plane(_))
            || self.seen_off(curve, range, stop.piece, target)
        {
            return Err(refused);
        }
        self.prove(curve, range, start, breaks, target, most, Feet::Careful)
            .map_err(|_| refused)
    }

    /// Whether a point of the curve is seen farther than `target` from the
    /// surface's point at its projection: the curve is off there, and no
    /// feet would prove it within. Looked at are the ends and the middle
    /// of the piece the proof stopped on, places on either side of it at
    /// distances that double from a 2⁻²⁴ of the range (the proof of a
    /// curve that is off stops where its distance comes to the target,
    /// beside a stretch where it is farther), and 17 places spread over
    /// the range. The projection is in closed form for the analytic kinds;
    /// for a spline it is a search, which may miss the nearest point: then
    /// a proof that could have been is not tried again, never one that
    /// could not.
    fn seen_off(
        &self,
        curve: &CurveBound<'_>,
        (lo, hi): (f64, f64),
        (a, b, _, _): Piece,
        target: f64,
    ) -> bool {
        let off = |t: f64| {
            let p = curve.eval(t);
            self.surface.project(p).is_ok_and(|f| {
                let gap = self.surface.eval(f.0, f.1).distance(p);
                gap - (curve.rounding(t) + self.rounding(f)) > target
            })
        };
        let width = hi - lo;
        let middle = 0.5 * (a + b);
        let beside = (0..24).rev().flat_map(|k| {
            let step = width * 0.5_f64.ppowi(k + 1);
            [middle - step, middle + step]
        });
        let spread = (0..=16).map(|k| lo + width * f64::from(k) / 16.0);
        [a, middle, b]
            .into_iter()
            .chain(beside)
            .chain(spread)
            .filter(|t| *t >= lo && *t <= hi)
            .any(off)
    }

    /// Whether `(u, v)` lies within the surface's parameter range in the
    /// directions it does not close round (as far as a billionth of the
    /// range's width, or of its finite end's size, past it): a foot past a
    /// pole, an apex or a spline's border names a point by the surface
    /// continued.
    fn inside(&self, (u, v): (f64, f64)) -> bool {
        let ((u0, u1), (v0, v1)) = self.surface.domain();
        let within = |x: f64, lo: f64, hi: f64, closed: bool| {
            if closed {
                return true;
            }
            let width = hi - lo;
            let slack = if width.is_finite() {
                1e-9 * width
            } else {
                let end = if lo.is_finite() { lo } else { hi };
                1e-9 * (1.0 + if end.is_finite() { end.abs() } else { 0.0 })
            };
            x >= lo - slack && x <= hi + slack
        };
        within(u, u0, u1, self.surface.period_u().is_some())
            && within(v, v0, v1, self.surface.period_v().is_some())
    }

    /// `f` named by the parameters nearest `start` round each direction
    /// the surface closes round.
    fn named_near(&self, (u, v): (f64, f64), start: (f64, f64)) -> (f64, f64) {
        let near = |x: f64, from: f64, period: Option<f64>| match period {
            Some(p) if p > 0.0 && p.is_finite() && (x - from).abs() > 0.5 * p => {
                let moved = x + ((from - x) / p).round() * p;
                if moved.is_finite() { moved } else { x }
            }
            _ => x,
        };
        (
            near(u, start.0, self.surface.period_u()),
            near(v, start.1, self.surface.period_v()),
        )
    }

    /// The foot of `p` found carefully from `start` (and `also`): the ones
    /// Gauss-Newton steps find from them ([`SurfaceBound::foot`]), the
    /// surface's own projection named near `start`, and that projection's
    /// foot by the steps; the nearest of them to `p`, and of those as near
    /// to rounding one within the surface's range (past a pole or an apex
    /// the surface continued names the same point of a sphere or of a
    /// revolved spline again, and a point of a cone's other nappe), the
    /// first of those. Steps from a start far round the axis cross a pole
    /// or an apex and settle there: on a cone at the point across the
    /// axis on the other nappe (twice the distance from the axis off), on
    /// a sphere at the same point named past the pole.
    fn careful_foot(&self, p: Vec3, start: (f64, f64), also: (f64, f64)) -> (f64, f64) {
        let followed = self.foot(p, start);
        // On the surface to rounding and within its range: none nearer
        // (and a spline's projection, a search, is dear).
        if self.inside(followed)
            && self.surface.eval(followed.0, followed.1).distance(p)
                <= 8.0 * self.rounding(followed)
        {
            return followed;
        }
        let mut candidates = vec![followed];
        if also != start {
            candidates.push(self.foot(p, also));
        }
        if let Ok(projected) = self.surface.project(p) {
            let named = self.named_near(projected, start);
            candidates.push(named);
            candidates.push(self.foot(p, named));
        }
        let scored: Vec<(f64, (f64, f64))> = candidates
            .into_iter()
            .map(|f| (self.surface.eval(f.0, f.1).distance(p), f))
            .filter(|(d, _)| d.is_finite())
            .collect();
        let Some(nearest) = scored.iter().map(|(d, _)| *d).reduce(f64::min) else {
            return followed;
        };
        let near_enough = |(d, f): &(f64, (f64, f64))| *d <= nearest + 8.0 * self.rounding(*f);
        scored
            .iter()
            .filter(|c| near_enough(c) && self.inside(c.1))
            .chain(scored.iter().filter(|c| near_enough(c)))
            .map(|(_, f)| *f)
            .next()
            .unwrap_or(followed)
    }

    /// [`SurfaceBound::piece_bound`] with the end's foot `fb` named by the
    /// parameters nearest `fa` round each direction the surface closes
    /// round, and, where that bound is over `target`, with it a turn
    /// further either way: the least of these. Any path gives a true
    /// bound; near a pole or an apex the curve goes half round the axis
    /// in a short stretch and the feet of a piece's two ends may be named
    /// a turn apart, so that the path between them goes round the axis
    /// the other way (or once more) and no halving would prove it.
    fn piece_bound_round(
        &self,
        curve: &CurveBound<'_>,
        (a, b, fa, fb): Piece,
        target: f64,
    ) -> Option<f64> {
        let named = self.named_near(fb, fa);
        let first = self.piece_bound(curve, (a, b), fa, named, Feet::Careful);
        if first.is_some_and(|x| x <= target) {
            return first;
        }
        let turns = [
            self.surface.period_u().map(|p| (p, 0.0)),
            self.surface.period_u().map(|p| (-p, 0.0)),
            self.surface.period_v().map(|p| (0.0, p)),
            self.surface.period_v().map(|p| (0.0, -p)),
        ];
        turns
            .into_iter()
            .flatten()
            .filter_map(|(du, dv)| {
                let turned = (named.0 + du, named.1 + dv);
                self.piece_bound(curve, (a, b), fa, turned, Feet::Careful)
            })
            .chain(first)
            .reduce(f64::min)
    }

    /// The proof of [`SurfaceBound::curve_within`] with the feet found as
    /// `feet` says; `Err` with where it stopped and why.
    #[allow(clippy::too_many_arguments)]
    fn prove(
        &self,
        curve: &CurveBound<'_>,
        range: (f64, f64),
        start: (f64, f64),
        breaks: &[f64],
        target: f64,
        most: usize,
        feet: Feet,
    ) -> Result<Proven, Stop> {
        let (lo, hi) = range;
        let careful = feet == Feet::Careful;
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
        let mut hint = self.off_singular(curve, range, start, Borders::No);
        let mut last = lo;
        for t in &points {
            for k in 1..FOLLOW {
                let between = last + (t - last) * k as f64 / FOLLOW as f64;
                hint = self.foot(curve.eval(between), hint);
            }
            hint = if careful {
                self.careful_foot(curve.eval(*t), hint, hint)
            } else {
                self.foot(curve.eval(*t), hint)
            };
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
            // Under a bound on the work (`crate::allowance`) that is
            // spent, no more pieces: not proven (an import refuses then,
            // naming the work).
            if crate::allowance::exhausted() {
                return Err(Stop {
                    at: middle,
                    piece: (a, b, fa, fb),
                    again: false,
                });
            }
            let bound = if careful {
                self.piece_bound_round(curve, (a, b, fa, fb), target)
            } else {
                self.piece_bound(curve, (a, b), fa, fb, Feet::Followed)
            };
            match bound {
                Some(bound) if bound <= target => {
                    proven.bound = proven.bound.max(bound);
                    proven.pieces += 1;
                    continue;
                }
                _ => {}
            }
            let narrowest = !(middle > a && middle < b);
            if proven.pieces + stack.len() + 2 > most || narrowest {
                // A piece that cannot be halved is held by a term of its
                // bound that does not fall with its width: an end's
                // distance from its foot, or a path between two feet
                // named apart (where the pieces allowed run out first,
                // nothing is tried again: the work is spent).
                return Err(Stop {
                    at: middle,
                    piece: (a, b, fa, fb),
                    again: !careful && narrowest,
                });
            }
            let p = curve.eval(middle);
            let fm = if careful {
                self.careful_foot(p, fa, fb)
            } else {
                // From both ends' feet: one poor foot would spoil every
                // halving below it.
                [fa, fb]
                    .map(|f| self.foot(p, f))
                    .into_iter()
                    .map(|f| (self.surface.eval(f.0, f.1).distance(p), f))
                    .min_by(|x, y| x.0.total_cmp(&y.0))
                    .map_or(fa, |(_, f)| f)
            };
            stack.push((middle, b, fm, fb));
            stack.push((a, middle, fa, fm));
        }
        Ok(proven)
    }
}

/// How the feet of a proof are found: followed from one to the next by
/// Gauss-Newton steps alone (every proof first), or carefully (once more,
/// where that proof stopped where the feet may be at fault): the nearest
/// of those steps and the surface's projection, one within the surface's
/// range before one past a pole or an apex
/// ([`SurfaceBound::careful_foot`]), each piece's end named near its
/// start and a turn either way ([`SurfaceBound::piece_bound_round`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Feet {
    Followed,
    Careful,
}

/// Where a proof stopped: the middle of the piece it stopped on, the
/// piece, and whether careful feet may prove what it did not.
struct Stop {
    at: f64,
    piece: Piece,
    again: bool,
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
    let prove = |start| bound.curve_within(curve, range, start, breaks, within.target, within.most);
    match start {
        Some(start) => prove(start),
        // A projection alone may name either end of a closed spline's
        // range at its seam: the one the curve goes on from. A start on
        // any border is stepped off first (as before 2026-11-01); should
        // that not prove it, a start stepped off only a border the
        // surface closes across (an edge along the side of a ruled ring,
        // whose every point is on a border, never settles off one).
        None => {
            let projected = surface.project(curve.eval(range.0)).map_err(|_| middle)?;
            let any = bound.off_singular(curve, range, projected, Borders::Any);
            match prove(any) {
                Ok(proven) => Ok(proven),
                Err(not) => {
                    let seams = bound.off_singular(curve, range, projected, Borders::Seams);
                    if seams == any { Err(not) } else { prove(seams) }
                }
            }
        }
    }
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

/// Whether the curve `b` over `range_b` and the curve `a` over `range_a`
/// are one curve within `within.target`, proven, not sampled: both run
/// between the same two ends (`same_way`: `b`'s start is `a`'s start;
/// otherwise `b`'s start is `a`'s end), and every point of either lies
/// within the bound of a point of the other.
///
/// The method is the error of interpolation, as for a curve on a surface.
/// The two parameters are paired along a correspondence `σ` from `b`'s to
/// `a`'s that runs from end to end: over a piece `[t0, t1] → [s0, s1]`
/// (`τ` from 0 to 1 across it) the difference `f(τ) = b(t(τ)) − a(σ(τ))`
/// is bounded in one of two ways, whichever is less (the Peano kernels
/// keep their sign, so both hold for vectors as for numbers):
///
/// - `σ` linear: `|f| ≤ max |f(end)| + max |f''| / 8`, `|f''|` at most
///   `b`'s second derivative over its piece plus `a`'s over its piece,
///   both in `τ`;
/// - `σ` the cubic Hermite path whose slopes at the ends take `b`'s
///   tangent's share along `a`'s:
///   `|f| ≤ max |f(end)| + ¼ max |f'(end)| + max |f⁗| / 384`,
///   where `f'` at an end is what is left of `b`'s
///   tangent off `a`'s, and `f⁗` is bounded from `b`'s fourth derivative
///   and `a`'s derivatives composed with `σ`'s (see `hermite_apart`).
///   The remainder falls with the fourth power of the piece, so curves
///   that are one are proven in few pieces.
///
/// The pieces start at `b`'s knots (paired with their feet on `a`), are
/// cut at `a`'s knots where they reach across one (paired with their feet
/// on `b`), and are halved, in a fixed order, until every bound is within
/// the target, a new pairing taken at each middle from the foot of `b`'s
/// point on `a`; at most `within.most` pieces, else not proven, with the
/// place. The feet are found by Gauss-Newton steps and
/// decide only how tight the bound is, never whether it holds: any `σ`
/// that runs from end to end gives a true bound. Since every piece's `σ`
/// passes through all of its `[s0, s1]` and the pieces' run from one end
/// of `a`'s range to the other, the bound holds both ways (every point of
/// `a` near `b` as well as every point of `b` near `a`). Rounding:
/// [`ROUNDING_ULPS`] of what enters each evaluation is added to every
/// distance measured. A range that is empty, runs backwards, or whose
/// width is not a finite number (ends far apart overflow it), and a target
/// that is not a positive number, are not proven, and never panic.
pub fn curves_within(
    a: &CurveBound<'_>,
    range_a: (f64, f64),
    b: &CurveBound<'_>,
    range_b: (f64, f64),
    same_way: bool,
    within: Within,
) -> Result<Proven, NotProven> {
    let middle = NotProven {
        at: 0.5 * range_b.0 + 0.5 * range_b.1,
    };
    // Finite ends and a finite width (ends far apart overflow it, and
    // every place between would be lost to it).
    let valid = |(lo, hi): (f64, f64)| (hi - lo).is_finite() && hi > lo;
    if !(valid(range_a) && valid(range_b) && within.target.is_finite() && within.target > 0.0) {
        return Err(middle);
    }
    let (s_start, s_end) = if same_way {
        range_a
    } else {
        (range_a.1, range_a.0)
    };
    // `a`'s parameter at `b`'s, linear between the ends: the first guess
    // of every foot.
    let linear = |t: f64| s_start + (s_end - s_start) * ((t - range_b.0) / (range_b.1 - range_b.0));
    // The pairs `(t, s)` the pieces start from: the ends and `b`'s knots
    // with their feet on `a`, kept running one way along `a` (a foot
    // that rounding puts behind the one before is moved up to it: any
    // pairing that runs from end to end gives a true bound). `a`'s knots
    // are paired where a piece reaches across one (below).
    let mut nodes: Vec<(f64, f64)> = vec![(range_b.0, s_start)];
    for t in b.breaks(range_b.0, range_b.1) {
        nodes.push((t, foot_on_curve(a, b.eval(t), linear(t), range_a)));
    }
    nodes.push((range_b.1, s_end));
    let mut previous = s_start;
    for node in &mut nodes {
        // `previous` and `s_end` are numbers in order, so no clamp here
        // can meet a NaN bound; a foot that is not a number takes the
        // place before it.
        let s = if node.1.is_nan() { previous } else { node.1 };
        node.1 = if same_way {
            s.clamp(previous, s_end)
        } else {
            s.clamp(s_end, previous)
        };
        previous = node.1;
    }
    // Pieces still to prove, the leftmost last (taken first).
    let mut stack: Vec<[(f64, f64); 2]> = nodes
        .windows(2)
        .rev()
        .filter_map(|w| match w {
            [p, q] => Some([*p, *q]),
            _ => None,
        })
        .collect();
    let mut worst = 0.0_f64;
    let mut pieces = 0_usize;
    while let Some([(t0, s0), (t1, s1)]) = stack.pop() {
        let at = NotProven {
            at: t0 + 0.5 * (t1 - t0),
        };
        let (lo, hi) = if s0 <= s1 { (s0, s1) } else { (s1, s0) };
        // A piece whose part of `a` reaches across a knot of `a` is cut
        // there first: the bounds hold within one piece of each curve. The
        // knot is paired with its foot on `b`, or, where rounding puts
        // that at an end of the piece, with the point as far along it.
        if let Some(knot) = a.breaks(lo, hi).first().copied() {
            if pieces + stack.len() + 2 > within.most {
                return Err(at);
            }
            let share = (knot - s0) / (s1 - s0);
            let even = t0 + (t1 - t0) * share;
            let foot = foot_on_curve(b, a.eval(knot), even, (t0, t1));
            let t = if foot > t0 && foot < t1 { foot } else { even };
            if !(t > t0 && t < t1) {
                return Err(at);
            }
            stack.push([(t, knot), (t1, s1)]);
            stack.push([(t0, s0), (t, knot)]);
            continue;
        }
        let Some(bound) = piece_apart(a, range_a, b, [(t0, s0), (t1, s1)]) else {
            return Err(at);
        };
        if bound <= within.target {
            worst = worst.max(bound);
            pieces += 1;
            continue;
        }
        if pieces + stack.len() + 2 > within.most {
            return Err(at);
        }
        let tm = t0 + 0.5 * (t1 - t0);
        if !(tm > t0 && tm < t1) {
            return Err(at);
        }
        let sm = foot_on_curve(a, b.eval(tm), s0 + 0.5 * (s1 - s0), (lo, hi));
        stack.push([(tm, sm), (t1, s1)]);
        stack.push([(t0, s0), (tm, sm)]);
    }
    Ok(Proven {
        bound: worst,
        pieces,
    })
}

/// A bound on how far apart the points of `b` over `[t0, t1]` and of `a`
/// paired with them from `s0` to `s1` are ([`curves_within`]): the less
/// of the bound for the linear pairing and, where it can be had, the one
/// for the cubic Hermite pairing. `None` where neither can be had.
fn piece_apart(
    a: &CurveBound<'_>,
    range_a: (f64, f64),
    b: &CurveBound<'_>,
    ends: [(f64, f64); 2],
) -> Option<f64> {
    let [(t0, s0), (t1, s1)] = ends;
    let dt = t1 - t0;
    if !(dt > 0.0) {
        return None;
    }
    let (pb0, db0) = b.derivatives(t0);
    let (pb1, db1) = b.derivatives(t1);
    let (pa0, da0) = a.derivatives(s0);
    let (pa1, da1) = a.derivatives(s1);
    // The ends, with what rounding may hide.
    let ends_apart = (pb0.distance(pa0) + b.rounding(t0) + a.rounding(s0))
        .max(pb1.distance(pa1) + b.rounding(t1) + a.rounding(s1));
    let jb = b.jets(t0, t1, pb0)?;
    // Linear: `|f''| ≤ |b''| + |a''|` in `τ` (the pairing is linear, so
    // no first derivatives enter); a piece of `a` of no length is a point.
    let ja2 = if s0 == s1 {
        0.0
    } else {
        get(&a.jets(s0.min(s1), s0.max(s1), pa0)?, 2)
    };
    let linear = ends_apart + (get(&jb, 2) + ja2) / 8.0;
    let hermite = hermite_apart(a, range_a, ends, [db0, db1], [da0, da1], dt, &jb)
        .map(|rest| ends_apart + rest);
    let bound = match hermite {
        Some(h) if h < linear => h,
        _ => linear,
    };
    bound.is_finite().then_some(bound)
}

/// The part of the cubic Hermite bound past the ends' distance: the
/// pairing `σ` runs from `s0` to `s1` as the cubic whose slopes at the ends
/// take `b`'s tangent's share along `a`'s, so `f' = b' Δt − a' σ'` there is
/// what is left of `b`'s tangent off `a`'s (nothing where the curves are
/// one); then `|f| ≤ max |f(end)| + ¼ max |f'(end)| + max |f⁗| / 384`,
/// `f⁗` bounded by `b`'s fourth derivative over its piece and by `a`'s
/// derivatives over the interval `σ` can sweep composed with the bounds
/// on `σ`'s (the majorant series, as for a path on a surface). The
/// interval `σ` sweeps is held within `¼` of the largest difference
/// between an end's slope and `s1 − s0` beyond `[s0, s1]` (what the cubic
/// Hermite basis allows), and has to lie in one piece of `a` and within
/// `range_a` (a point of `a` past its range is not a point of it). `None`
/// where that does not hold or a tangent of `a` has no length.
fn hermite_apart(
    a: &CurveBound<'_>,
    range_a: (f64, f64),
    [(_, s0), (_, s1)]: [(f64, f64); 2],
    db: [Vec3; 2],
    da: [Vec3; 2],
    dt: f64,
    jb: &Jet,
) -> Option<f64> {
    let [db0, db1] = db;
    let [da0, da1] = da;
    let slope = |d_b: Vec3, d_a: Vec3| -> Option<f64> {
        let dd = d_a.dot(d_a);
        (dd > 0.0).then(|| dt * d_b.dot(d_a) / dd)
    };
    let (m0, m1) = (slope(db0, da0)?, slope(db1, da1)?);
    let ds = s1 - s0;
    // f' at the ends, with what rounding may hide in the difference.
    let rounding = |d_b: Vec3, d_a: Vec3, m: f64| {
        ROUNDING_ULPS * f64::EPSILON * (d_b.length() * dt + d_a.length() * m.abs())
    };
    let f0 = (db0 * dt - da0 * m0).length() + rounding(db0, da0, m0);
    let f1 = (db1 * dt - da1 * m1).length() + rounding(db1, da1, m1);
    // σ = s0 + m0 τ + c2 τ² + c3 τ³.
    let c2 = 3.0 * ds - 2.0 * m0 - m1;
    let c3 = -2.0 * ds + m0 + m1;
    let first = {
        let at = |t: f64| (m0 + 2.0 * c2 * t + 3.0 * c3 * t * t).abs();
        let mut most = at(0.0).max(at(1.0));
        if c3 != 0.0 {
            let vertex = -c2 / (3.0 * c3);
            if vertex > 0.0 && vertex < 1.0 {
                most = most.max(at(vertex));
            }
        }
        most
    };
    let second = (2.0 * c2).abs().max((2.0 * c2 + 6.0 * c3).abs());
    let third = (6.0 * c3).abs();
    let sigma: Jet = [0.0, first, second, third, 0.0];
    let past = 0.25 * (m0 - ds).abs().max((m1 - ds).abs());
    let (lo, hi) = (s0.min(s1) - past, s0.max(s1) + past);
    if !(lo.is_finite() && hi.is_finite() && lo >= range_a.0 && hi <= range_a.1)
        || !a.breaks(lo, hi).is_empty()
    {
        return None;
    }
    let composed = if hi > lo {
        let jet = a.jets(lo, hi, a.eval(lo))?;
        // From `τ` over `[lo, hi]` back to `a`'s own parameter.
        let h = hi - lo;
        let mut table: Table = [[0.0; N]; N];
        for (k, row) in table.iter_mut().enumerate().skip(1) {
            if let Some(slot) = row.first_mut() {
                *slot = get(&jet, k) / h.ppowi(k as i32);
            }
        }
        compose(&table, &sigma, &[0.0; N])
    } else {
        [0.0; N]
    };
    let rest = 0.25 * f0.max(f1) + (get(jb, 4) + get(&composed, 4)) / 384.0;
    rest.is_finite().then_some(rest)
}

/// The parameter of the point of `curve` nearest `point`, by Gauss-Newton
/// steps from `guess`, kept within `range` (in either order); always a
/// number within it (the middle where the guess is not a number), so that
/// what is built on it never meets a NaN.
fn foot_on_curve(curve: &CurveBound<'_>, point: Vec3, guess: f64, range: (f64, f64)) -> f64 {
    let (lo, hi) = if range.0 <= range.1 {
        range
    } else {
        (range.1, range.0)
    };
    if !(lo.is_finite() && hi.is_finite()) {
        return guess;
    }
    let mut s = if guess.is_nan() {
        0.5 * lo + 0.5 * hi
    } else {
        guess.clamp(lo, hi)
    };
    for _ in 0..FOOT_STEPS {
        let (p, d) = curve.derivatives(s);
        let dd = d.dot(d);
        if !(dd > 0.0) {
            break;
        }
        let next = s + (point - p).dot(d) / dd;
        if !next.is_finite() {
            break;
        }
        let next = next.clamp(lo, hi);
        if next == s {
            break;
        }
        s = next;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BezierPatch, Cone, Cylinder, NurbsSurface, Plane, Sphere, Torus};
    use craft3d_math::Frame;
    use std::f64::consts::PI;

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
    pub(super) fn curve_on(
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
    pub(super) type Along = Box<dyn Fn(f64) -> (f64, f64)>;

    pub(super) fn surfaces(k: f64) -> Vec<(Surface, Along)> {
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

    /// The farthest a point of `b` over `range_b` is from `a` over
    /// `range_a`, densely, both ways: each point's distance to the nearest
    /// of many points of the other, less the spacing of those points.
    fn hausdorff_seen(
        a: &CurveBound<'_>,
        range_a: (f64, f64),
        b: &CurveBound<'_>,
        range_b: (f64, f64),
    ) -> f64 {
        let n = 2000;
        let points = |c: &CurveBound<'_>, (lo, hi): (f64, f64)| -> Vec<Vec3> {
            (0..=n)
                .map(|i| c.eval(lo + (hi - lo) * f64::from(i) / f64::from(n)))
                .collect()
        };
        let (pa, pb) = (points(a, range_a), points(b, range_b));
        let spacing = |p: &[Vec3]| {
            p.windows(2)
                .map(|w| w[0].distance(w[1]))
                .fold(0.0, f64::max)
        };
        let one_way = |from: &[Vec3], to: &[Vec3]| {
            from.iter()
                .map(|p| {
                    to.iter()
                        .map(|q| p.distance(*q))
                        .fold(f64::INFINITY, f64::min)
                })
                .fold(0.0, f64::max)
                - spacing(to)
        };
        one_way(&pa, &pb).max(one_way(&pb, &pa)).max(0.0)
    }

    /// The segment from `p` to `q` as a cubic whose speed varies (its
    /// inner control points at a tenth and at four fifths), and the
    /// circle's quarter as the exact rational quadratic.
    fn straight_cubic(p: Vec3, q: Vec3) -> NurbsCurve {
        let ctrl = [0.0, 0.1, 0.8, 1.0]
            .iter()
            .map(|s| p + (q - p) * *s)
            .collect();
        NurbsCurve::polynomial(3, vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0], ctrl).unwrap()
    }

    /// The ellipse of semi-axes `a` along `f`'s x and `b` along its y as
    /// the rational quadratic: the circle's nine control points and
    /// weights (`NurbsCurve::circle`) carried by the affine map that makes
    /// the circle the ellipse.
    fn rational_ellipse(f: &Frame, a: f64, b: f64) -> NurbsCurve {
        let unit = NurbsCurve::circle(&Frame::WORLD, 1.0).unwrap();
        let ctrl = unit
            .control_points()
            .iter()
            .map(|p| f.point(p.x * a, p.y * b, 0.0))
            .collect();
        NurbsCurve::new(2, unit.knots().to_vec(), ctrl, unit.weights().to_vec()).unwrap()
    }

    #[test]
    fn one_curve_given_twice_is_proven_within_and_the_bound_is_above_every_sample() {
        for k in [1e-3, 1.0, 1e6] {
            for (place, f) in [frame(k), far(k)].into_iter().enumerate() {
                let target = 1e-6 * k;
                let within = Within { target, most: 4096 };
                let (p, q) = (
                    f.point(1.0 * k, 2.0 * k, 0.5 * k),
                    f.point(-3.0 * k, 0.5 * k, 2.0 * k),
                );
                let line =
                    Curve3::Line(crate::Line::new(p, (q - p).normalized().unwrap()).unwrap());
                let length = p.distance(q);
                let cubic = Curve3::Nurbs(straight_cubic(p, q));
                let reversed = Curve3::Nurbs(straight_cubic(q, p));
                let circle = Curve3::Circle(crate::Circle::new(f, 2.0 * k).unwrap());
                let rational = Curve3::Nurbs(NurbsCurve::circle(&f, 2.0 * k).unwrap());
                // A cubic through 41 points of a helix: knots every fortieth,
                // given twice, so the knots of the two coincide.
                let fitted = {
                    let params: Vec<f64> = (0..=40).map(|i| f64::from(i) / 40.0).collect();
                    let values: Vec<Vec<f64>> = params
                        .iter()
                        .map(|t| {
                            let p = f.point(k * (3.0 * t).pcos(), k * (3.0 * t).psin(), k * t);
                            vec![p.x, p.y, p.z]
                        })
                        .collect();
                    let (knots, rows) = crate::refine::interpolate(&params, 3, &values).unwrap();
                    let ctrl = rows.iter().map(|r| Vec3::new(r[0], r[1], r[2])).collect();
                    Curve3::Nurbs(NurbsCurve::polynomial(3, knots, ctrl).unwrap())
                };
                let ellipse = Curve3::Ellipse(crate::Ellipse::new(f, 3.0 * k, 1.5 * k).unwrap());
                let spline_ellipse = Curve3::Nurbs(rational_ellipse(&f, 3.0 * k, 1.5 * k));
                // (a, its range, b, its range, b runs as a does)
                let pairs = [
                    (&line, (0.0, length), &cubic, (0.0, 1.0), true),
                    (&line, (0.0, length), &reversed, (0.0, 1.0), false),
                    (&cubic, (0.0, 1.0), &reversed, (0.0, 1.0), false),
                    (&circle, (0.0, 0.5 * PI), &rational, (0.0, 0.25), true),
                    (&rational, (0.25, 1.0), &circle, (0.5 * PI, 2.0 * PI), true),
                    (&fitted, (0.0, 1.0), &fitted, (0.0, 1.0), true),
                    // Closed curves, whole: both ends at one point.
                    (&circle, (0.0, 2.0 * PI), &rational, (0.0, 1.0), true),
                    (&ellipse, (0.0, 2.0 * PI), &spline_ellipse, (0.0, 1.0), true),
                    (&ellipse, (0.5 * PI, PI), &spline_ellipse, (0.25, 0.5), true),
                ];
                for (index, (a, range_a, b, range_b, same_way)) in pairs.into_iter().enumerate() {
                    let what = format!("pair {index} at {k}, place {place}");
                    let (ab, bb) = (CurveBound::new(a), CurveBound::new(b));
                    let proven = curves_within(&ab, range_a, &bb, range_b, same_way, within)
                        .unwrap_or_else(|e| panic!("{what}: {e:?}"));
                    assert!(proven.bound <= target, "{what}: {proven:?}");
                    let seen = hausdorff_seen(&ab, range_a, &bb, range_b);
                    assert!(proven.bound >= seen, "{what}: {proven:?} below {seen}");
                    // Either way round.
                    curves_within(&bb, range_b, &ab, range_a, same_way, within)
                        .unwrap_or_else(|e| panic!("{what} turned: {e:?}"));
                }
            }
        }
    }

    /// A tilted frame far from the origin, at scale `k`.
    fn far(k: f64) -> Frame {
        Frame::new(
            Vec3::new(1000.0 * k, -700.0 * k, 300.0 * k),
            Vec3::new(0.2, 0.3, 1.0),
            Vec3::new(1.0, -0.1, 0.0),
        )
        .unwrap()
    }

    #[test]
    fn two_curves_between_the_same_ends_that_part_are_not_proven_within() {
        for k in [1e-3, 1.0, 1e6] {
            let f = frame(k);
            let within = Within {
                target: 1e-6 * k,
                most: 4096,
            };
            // The two halves of a circle, between the same two points.
            let circle = Curve3::Circle(crate::Circle::new(f, 2.0 * k).unwrap());
            let cb = CurveBound::new(&circle);
            assert!(curves_within(&cb, (0.0, PI), &cb, (PI, 2.0 * PI), false, within).is_err());
            // A line and a bow over it: apart by more than the target in
            // the middle only; within it, a bow a tenth as high.
            let line = Curve3::Line(crate::Line::new(f.origin(), f.x()).unwrap());
            let lb = CurveBound::new(&line);
            let bowed = |height: f64| {
                let ctrl = vec![
                    f.origin(),
                    f.point(k / 3.0, height * k, 0.0),
                    f.point(2.0 * k / 3.0, height * k, 0.0),
                    f.point(k, 0.0, 0.0),
                ];
                Curve3::Nurbs(
                    NurbsCurve::polynomial(3, vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0], ctrl)
                        .unwrap(),
                )
            };
            let high = bowed(4e-6);
            let low = bowed(4e-8);
            let (hb, wb) = (CurveBound::new(&high), CurveBound::new(&low));
            // The bow is 3/4 of its control height at the middle.
            let not = curves_within(&lb, (0.0, k), &hb, (0.0, 1.0), true, within).unwrap_err();
            assert!(not.at > 0.0 && not.at < 1.0, "{k}: {not:?}");
            let proven = curves_within(&lb, (0.0, k), &wb, (0.0, 1.0), true, within)
                .unwrap_or_else(|e| panic!("{k}: {e:?}"));
            assert!(proven.bound >= 3e-8 * k, "{k}: {proven:?}");
            // Paired the wrong way round: the ends are a whole length apart.
            assert!(curves_within(&lb, (0.0, k), &wb, (0.0, 1.0), false, within).is_err());
        }
    }

    #[test]
    fn curves_within_takes_bad_ranges_and_budgets_without_panicking() {
        let line = Curve3::Line(crate::Line::new(Vec3::ZERO, Vec3::X).unwrap());
        let lb = CurveBound::new(&line);
        let within = Within {
            target: 1e-6,
            most: 64,
        };
        for r in [
            (1.0, 0.0),
            (0.0, f64::NAN),
            (f64::NEG_INFINITY, 0.0),
            (0.0, 0.0),
        ] {
            assert!(curves_within(&lb, r, &lb, (0.0, 1.0), true, within).is_err());
            assert!(curves_within(&lb, (0.0, 1.0), &lb, r, true, within).is_err());
        }
        for target in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let bad = Within { target, most: 64 };
            assert!(curves_within(&lb, (0.0, 1.0), &lb, (0.0, 1.0), true, bad).is_err());
        }
        // One piece allowed where the circle needs many: not proven.
        let circle = Curve3::Circle(crate::Circle::new(Frame::WORLD, 1.0).unwrap());
        let cb = CurveBound::new(&circle);
        let rational = Curve3::Nurbs(NurbsCurve::circle(&Frame::WORLD, 1.0).unwrap());
        let rb = CurveBound::new(&rational);
        let one = Within {
            target: 1e-9,
            most: 1,
        };
        assert!(curves_within(&cb, (0.0, 0.5 * PI), &rb, (0.0, 0.25), true, one).is_err());
        // Finite ranges whose width is not: a line and a cubic with a knot
        // inside, both over (−1e308, 1e308). Before 2026-11-13 the foot of
        // the knot came out NaN and a clamp on it panicked.
        let cubic = Curve3::Nurbs(
            NurbsCurve::polynomial(
                3,
                vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
                vec![
                    Vec3::ZERO,
                    Vec3::new(0.25, 0.0, 0.0),
                    Vec3::new(0.5, 0.0, 0.0),
                    Vec3::new(0.75, 0.0, 0.0),
                    Vec3::X,
                ],
            )
            .unwrap(),
        );
        let kb = CurveBound::new(&cubic);
        let huge = (-1e308, 1e308);
        for same_way in [true, false] {
            assert!(curves_within(&lb, huge, &kb, huge, same_way, within).is_err());
            assert!(curves_within(&kb, huge, &lb, huge, same_way, within).is_err());
            assert!(curves_within(&lb, huge, &kb, (0.0, 1.0), same_way, within).is_err());
            assert!(curves_within(&kb, (0.0, 1.0), &lb, huge, same_way, within).is_err());
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
                let start = sb.off_singular(&cb, range, (u, 96.0), Borders::Seams);
                assert!(start.1 < 1.0, "{u} at {k}: {start:?}");
                let proven = sb.curve_within(&cb, range, start, &[], 1e-6 * k, 1 << 14);
                assert!(
                    proven.is_ok_and(|p| p.bound <= 1e-6 * k),
                    "{u} at {k}: {proven:?}"
                );
            }
        }
    }

    /// A line sealed at a full knot is crossed with the jump its border
    /// rows bound only within the other direction's domain: past it the
    /// end patches go on as polynomials whose borders part further (here
    /// four times the rows' difference at `v = −1`, where the hull gives
    /// the difference itself), so a box that crosses the line out there
    /// gets no bound at all (review of 2026-11-02: before, it got the
    /// jump of the rows, and a curve out there could be proven nearer
    /// than it is).
    #[test]
    fn a_sealed_line_is_not_crossed_past_the_domain() {
        for k in [1e-3, 1.0, 1e3] {
            let d = 1e-6 * k;
            let mut points = Vec::new();
            for (i, x) in [0.0, 0.5, 0.5, 1.0].iter().enumerate() {
                for j in 0..3 {
                    let lift = if i == 2 && j == 1 { d } else { 0.0 };
                    points.push(Vec3::new(x * k, j as f64 * 0.5 * k, lift));
                }
            }
            let weights = vec![1.0; points.len()];
            let surface = Surface::Nurbs(
                NurbsSurface::new(
                    1,
                    2,
                    vec![0.0, 0.0, 0.5, 0.5, 1.0, 1.0],
                    vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    4,
                    3,
                    points,
                    weights,
                )
                .unwrap(),
            );
            let sb = SurfaceBound::new(&surface).unwrap();
            let Form::Patches { joins_u, .. } = &sb.form else {
                panic!("a spline surface has patches");
            };
            let jump = joins_u[0].unwrap().jump;
            assert!(jump >= d && jump < 1.01 * d, "{jump:e}");
            // Within the domain the borders part by at most the jump;
            // at v = −1 by four times the rows' difference.
            let apart = |v: f64| {
                surface
                    .eval(0.5 - 1e-12, v)
                    .distance(surface.eval(0.5 + 1e-12, v))
            };
            assert!(apart(0.5) <= jump);
            assert!(apart(-1.0) > 3.9 * d, "{:e}", apart(-1.0));
            // A box across the line within the domain is bounded, with
            // the jump; one across it past the domain is not.
            let inside = Path::new((0.45, 0.4), (0.55, 0.6), (0.1, 0.2), (0.1, 0.2));
            let (_, smooth, jumps) = sb.jets(&inside, Vec3::ZERO).unwrap();
            assert_eq!(smooth, 0);
            assert!(jumps >= jump);
            let past = Path::new((0.45, -1.1), (0.55, -0.9), (0.1, 0.2), (0.1, 0.2));
            assert!(sb.jets(&past, Vec3::ZERO).is_none());
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

#[cfg(test)]
#[path = "certify_apex_tests.rs"]
mod apex_tests;
