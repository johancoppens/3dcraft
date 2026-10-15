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
//!   derivatives jump there) the bound is the Lipschitz one,
//!   `(|e(0)| + |e(1)| + max |e'|) / 2`, which holds for a continuous
//!   surface; the piece is halved until that is within the target too.
//! - **Adaptive and bounded.** The pieces start at the curve's knots and
//!   the caller's breaks; a piece whose bound is over the target is
//!   halved, in a fixed order, until every bound is within it, or the
//!   number of pieces allowed is spent and the answer is "not proven"
//!   (with the place), never a guess.
//!
//! Rounding: the distances at the ends are measured in floating point; an
//! allowance of a few units in the last place of the coordinates is added.

// `!(x > 0.0)` is deliberate: unlike `x <= 0.0` it is also true for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::{BezierSpan, Curve3, CurvePiece, NurbsCurve, RationalBezierPatch, Surface};
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
/// The narrowest box a patch is cut to, as a part of its span: a box of no
/// width (a path along an iso-line) is widened to this, which only makes
/// the bound hold over more of the patch.
const NARROWEST: f64 = 1e-9;

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
enum Form {
    Analytic,
    Patches {
        spans: Vec<BezierSpan>,
        /// The distinct spans along u and along v (the patches are these,
        /// u by u, v by v within each).
        us: Vec<(f64, f64)>,
        vs: Vec<(f64, f64)>,
        /// No interior knot of a multiplicity above the degree: the
        /// surface is continuous across its knot lines.
        continuous: bool,
    },
}

/// A surface made ready for proofs about curves near it.
pub struct SurfaceBound<'a> {
    surface: &'a Surface,
    form: Form,
}

/// A curve made ready for proofs: its Bézier pieces, or its kind.
pub struct CurveBound<'a> {
    curve: CurveRef<'a>,
    pieces: Vec<CurvePiece>,
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
        CurveBound {
            curve: CurveRef::Curve(curve),
            pieces,
        }
    }

    pub fn nurbs(curve: &'a NurbsCurve) -> CurveBound<'a> {
        CurveBound {
            curve: CurveRef::Nurbs(curve),
            pieces: curve.bezier_pieces(),
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
                *slot = r * h.abs().powi(k as i32);
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
            let slope = c.half_angle().tan().abs();
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

/// Whether every interior knot has a multiplicity of at most `degree`.
fn continuous(knots: &[f64], degree: usize) -> bool {
    let count = knots.len().saturating_sub(degree + 1);
    let (Some(&start), Some(&end)) = (knots.get(degree), knots.get(count)) else {
        return false;
    };
    let mut run = 0;
    let mut previous = f64::NAN;
    for &k in knots {
        run = if k == previous { run + 1 } else { 1 };
        previous = k;
        if k > start && k < end && run > degree {
            return false;
        }
    }
    true
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
                continuous: true,
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
                Form::Patches {
                    spans,
                    us,
                    vs,
                    continuous: continuous(s.knots_u(), du) && continuous(s.knots_v(), dv),
                }
            }
        };
        Some(SurfaceBound { surface, form })
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
            let (a, b, c) = (su.dot(su), su.dot(sv), sv.dot(sv));
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
        best.1
    }

    /// Bounds on the derivatives in `τ` (orders 0 to 4; order 0 about
    /// `origin`) of `S(σ(τ))`, and whether they hold as derivatives of a
    /// smooth function (the path's box lies in one patch); where it does
    /// not, only order 1 holds, piece by piece, as a Lipschitz bound.
    /// `None` where no bound comes out.
    fn jets(&self, path: &Path, origin: Vec3) -> Option<(Jet, bool)> {
        let range = path.range();
        let [x, y] = path.jets();
        match &self.form {
            Form::Analytic => {
                let table = analytic_table(self.surface, range)?;
                Some((compose(&table, &x, &y), true))
            }
            Form::Patches {
                spans,
                us,
                vs,
                continuous,
            } => {
                let [(u0, u1), (v0, v1)] = range;
                let (iu0, iu1) = spans_reached(us, u0, u1)?;
                let (iv0, iv1) = spans_reached(vs, v0, v1)?;
                let smooth = iu0 == iu1 && iv0 == iv1;
                if !smooth && !continuous {
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
                all.iter().all(|x| x.is_finite()).then_some((all, smooth))
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
            if det > 0.0 && t.0.is_finite() && t.1.is_finite() {
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
        let (sj, smooth) = self.jets(&path, ca)?;
        let rounding =
            32.0 * f64::EPSILON * (ca.length() + sa.length() + cb.length() + sb.length());
        let (na, nb) = (ea.length(), eb.length());
        // Lipschitz: holds for any continuous surface.
        let mut bound = 0.5 * (na + nb + get(&cj, 1) + get(&sj, 1));
        if smooth {
            // Linear interpolation, with the second derivative.
            bound = bound.min(na.max(nb) + (get(&cj, 2) + get(&sj, 2)) / 8.0);
            // Cubic Hermite interpolation, with the fourth.
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
        points.extend(breaks.iter().copied().filter(|t| *t > lo && *t < hi));
        points.sort_by(f64::total_cmp);
        points.dedup();
        let mut feet = Vec::with_capacity(points.len());
        let mut hint = start;
        for t in &points {
            hint = self.foot(curve.eval(*t), hint);
            feet.push(hint);
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
            let hint = (0.5 * (fa.0 + fb.0), 0.5 * (fa.1 + fb.1));
            let fm = self.foot(curve.eval(middle), hint);
            stack.push((middle, b, fm, fb));
            stack.push((a, middle, fa, fm));
        }
        Ok(proven)
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
        let rounding = 32.0 * f64::EPSILON * (ends[0].length() + ends[1].length());
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

/// Whether the curve `a` over `range_a` and `b` over `range_b` keep more
/// than `threshold` apart everywhere, proven: each is cut into pieces held
/// within a capsule round their chords (a radius from the bound on their
/// second derivatives), and two pieces whose capsules are more than the
/// threshold apart are apart. Two that are not are halved (the one with
/// the wider capsule or the longer chord), unless the points of the curves
/// nearest where the chords meet are themselves within the threshold:
/// then the curves come that close ([`Apart::No`]). At most `most` pairs
/// of pieces are looked at; past that the answer is [`Apart::Unknown`].
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
    let (Some(pa), Some(pb)) = (a.spans(range_a, 8), b.spans(range_b, 8)) else {
        return Apart::Unknown;
    };
    let mut stack: Vec<(Span, Span)> = Vec::with_capacity(pa.len() * pb.len());
    for x in pa.iter().rev() {
        for y in pb.iter().rev() {
            stack.push((*x, *y));
        }
    }
    let mut looked = 0_usize;
    while let Some((x, y)) = stack.pop() {
        looked += 1;
        if looked > most {
            return Apart::Unknown;
        }
        let [x0, x1] = x.ends;
        let [y0, y1] = y.ends;
        let (chords, s, t) = segments(x0, x1, y0, y1);
        if chords - x.reach - y.reach > threshold {
            continue;
        }
        let at = [
            x.range.0 + s * (x.range.1 - x.range.0),
            y.range.0 + t * (y.range.1 - y.range.0),
        ];
        let distance = a.eval(at[0]).distance(b.eval(at[1]));
        if distance <= threshold {
            return Apart::No { at, distance };
        }
        // Halve the one that is coarser.
        let size = |p: &Span| p.reach.max(p.ends[0].distance(p.ends[1]));
        let halve = |curve: &CurveBound<'_>, p: &Span| -> Option<[Span; 2]> {
            let (lo, hi) = p.range;
            let middle = 0.5 * (lo + hi);
            if !(middle > lo && middle < hi) {
                return None;
            }
            Some([curve.span(lo, middle)?, curve.span(middle, hi)?])
        };
        if size(&x) >= size(&y) {
            let Some([l, r]) = halve(a, &x) else {
                return Apart::Unknown;
            };
            stack.push((r, y));
            stack.push((l, y));
        } else {
            let Some([l, r]) = halve(b, &y) else {
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
                    * (std::f64::consts::TAU * count as f64 * t).sin().powi(2)
                    * (std::f64::consts::PI * 2.0 * count as f64 * t).sin();
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
                    ctrl.push(Vec3::new(x * k, y * k, 0.3 * k * (1.7 * x + 0.9 * y).sin()));
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
                Box::new(|t| (t, 0.5 + 0.4 * (3.0 * t).sin())),
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
                    *slot = 2.0_f64.powi(j as i32);
                }
            }
        }
        let path = Path::new((0.1, 0.2), (0.9, -0.4), (1.3, 0.2), (-0.4, 0.7));
        let [x, y] = path.jets();
        let jet = compose(&table, &x, &y);
        let f = |t: f64| {
            let u = path.c[0][0] + t * (path.c[0][1] + t * (path.c[0][2] + t * path.c[0][3]));
            let v = path.c[1][0] + t * (path.c[1][1] + t * (path.c[1][2] + t * path.c[1][3]));
            u.sin() * (2.0 * v).cos()
        };
        let h = 1e-2;
        for k in 0..=20 {
            let t = 0.05 + 0.9 * f64::from(k) / 20.0;
            let d1 = (f(t + h) - f(t - h)) / (2.0 * h);
            let d2 = (f(t + h) - 2.0 * f(t) + f(t - h)) / (h * h);
            let d4 = (f(t + 2.0 * h) - 4.0 * f(t + h) + 6.0 * f(t) - 4.0 * f(t - h)
                + f(t - 2.0 * h))
                / h.powi(4);
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
            .map(|t| vec![t * k, (gap + 4e-2 * (t - 0.507).powi(2)) * k, 0.0])
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
        let curve = bow(1e-3, 1.0);
        let cb = CurveBound::nurbs(&curve);
        assert_eq!(
            curves_apart(&cb, (0.0, 1.0), &lb, (0.0, 1.0), 1e-5, 1),
            Apart::Unknown
        );
    }
}
