//! The six rows of a spline face along an edge, with their rounding
//! bounded term by term (docs/design/surfacing.md › *The rows' rounding,
//! differentiated first*).
//!
//! The continuity measure needs, along the edge, the face's point `X₀`,
//! its first and second derivatives along the edge `X₀′`, `X₀″`, the
//! derivative across `X₁` and its derivative along `X₁′`, and the second
//! derivative across `X₂`. Formed from the face's control points written
//! about a point of the edge, then differentiated as splines, every
//! derivative carries the rounding of the *points* (`ε` of their distance
//! from that point) times the derivative's factor `p / Δ` once per order:
//! on a long seam of many short spans that floor reached the curvature
//! threshold (the tank's Finding 1, and a cylinder drum cut by an
//! ellipse here). So the derivatives are taken *first*, on the stored
//! control points, each difference formed as
//! `w_{i+1}(x_{i+1} − x_i) + (x_i − o)(w_{i+1} − w_i)` (the homogeneous
//! difference about `o`, exactly), whose rounding is a few `ε` of its own
//! terms, not of the points; every later step (the combination across at
//! the line, the cut to the edge, a spline face's Bézier patches and
//! their composition with a curve on it, products with that curve's
//! derivatives) is linear in the nets with known coefficient sums, and
//! carries each coefficient's bound alongside it.

use super::{H, NetworkError, Raw, derivative, internal, reverse};
use crate::NurbsCurve;
use crate::NurbsSurface;
use crate::refine::{basis, bernstein_product, piece_raw, restricted};
use crate::trim::{BezierNet, Composed, TrimError, compose_net_pieces};
use craft3d_math::Vec3;

const EPS: f64 = f64::EPSILON;

/// A row with the bound on its coefficients' rounding, coordinate by
/// coordinate (`x w`, `y w`, `z w`, `w`).
pub(super) type Row = (Raw, [f64; 4]);

fn from_trim(e: TrimError) -> NetworkError {
    match e {
        TrimError::Refused(why) => NetworkError::Refused(why),
        TrimError::Geom(g) => NetworkError::Geom(g),
    }
}

/// A tensor-product net of homogeneous values about a local origin, each
/// coefficient with a bound on its rounding (`e`, nonnegative, per
/// coordinate).
#[derive(Clone, Debug)]
pub(super) struct TNet {
    degrees: (usize, usize),
    ku: Vec<f64>,
    kv: Vec<f64>,
    nu: usize,
    nv: usize,
    v: Vec<H>,
    e: Vec<H>,
}

fn h_abs(h: H) -> H {
    h.map(f64::abs)
}

impl TNet {
    /// The surface's net about `o`: `((x − o) w, w)`, the subtraction and
    /// the product each rounded once (`2 ε` of the value), the weight
    /// exact.
    pub(super) fn of(s: &NurbsSurface, o: Vec3) -> TNet {
        let (nu, nv) = s.counts();
        let mut v = Vec::with_capacity(nu * nv);
        let mut e = Vec::with_capacity(nu * nv);
        for (p, w) in s.control_points().iter().zip(s.weights()) {
            let h = [(p.x - o.x) * w, (p.y - o.y) * w, (p.z - o.z) * w, *w];
            v.push(h);
            e.push([
                2.0 * EPS * h[0].abs(),
                2.0 * EPS * h[1].abs(),
                2.0 * EPS * h[2].abs(),
                0.0,
            ]);
        }
        TNet {
            degrees: s.degrees(),
            ku: s.knots_u().to_vec(),
            kv: s.knots_v().to_vec(),
            nu,
            nv,
            v,
            e,
        }
    }

    fn at(&self, i: usize, j: usize) -> Option<(H, H)> {
        let k = i.checked_mul(self.nv)?.checked_add(j)?;
        Some((*self.v.get(k)?, *self.e.get(k)?))
    }

    /// The shape of the derivative in u (`in_u`) or v: degree, knots and
    /// counts, and the factor `p / (t_{i+p+1} − t_{i+1})` of difference `i`.
    fn derived_shape(&self, in_u: bool) -> (usize, usize, Vec<f64>, Vec<f64>, usize, usize) {
        let (p, q) = self.degrees;
        let trim = |k: &[f64]| k.get(1..k.len().saturating_sub(1)).unwrap_or(&[]).to_vec();
        if in_u {
            (
                p.saturating_sub(1),
                q,
                trim(&self.ku),
                self.kv.clone(),
                self.nu.saturating_sub(1),
                self.nv,
            )
        } else {
            (
                p,
                q.saturating_sub(1),
                self.ku.clone(),
                trim(&self.kv),
                self.nu,
                self.nv.saturating_sub(1),
            )
        }
    }

    fn factor(&self, in_u: bool, i: usize) -> f64 {
        let (p, q) = self.degrees;
        let (deg, knots) = if in_u { (p, &self.ku) } else { (q, &self.kv) };
        match (knots.get(i + 1), knots.get(i + deg + 1)) {
            (Some(a), Some(b)) if b > a => deg as f64 / (b - a),
            _ => 0.0,
        }
    }

    /// The first derivative in u or v of the surface's net about `o`,
    /// from its stored points: each homogeneous difference
    /// `w_{i+1}(x_{i+1} − x_i) + (x_i − o)(w_{i+1} − w_i)` (exactly the
    /// difference of the local values) times the knot factor. Six
    /// roundings of its two terms (the two differences, the two products,
    /// the sum, the factor) and the factor's own: within
    /// `f (6 ε (|t₁| + |t₂|)) + 2 ε |value|`; the weight's within
    /// `3 ε |value|`.
    pub(super) fn first(s: &NurbsSurface, o: Vec3, in_u: bool) -> Result<TNet, NetworkError> {
        let base = TNet::of(s, o);
        let along = if in_u { base.degrees.0 } else { base.degrees.1 };
        if along == 0 {
            return Ok(base.zero_like());
        }
        let (dp, dq, ku, kv, nu, nv) = base.derived_shape(in_u);
        let points = s.control_points();
        let weights = s.weights();
        let (snu, snv) = s.counts();
        let get = |i: usize, j: usize| -> Option<(Vec3, f64)> {
            let k = i.checked_mul(snv)?.checked_add(j)?;
            Some((*points.get(k)?, *weights.get(k)?))
        };
        let mut v = Vec::with_capacity(nu * nv);
        let mut e = Vec::with_capacity(nu * nv);
        for i in 0..nu {
            for j in 0..nv {
                let (lo, hi, index) = if in_u {
                    ((i, j), (i + 1, j), i)
                } else {
                    ((i, j), (i, j + 1), j)
                };
                if hi.0 >= snu || hi.1 >= snv {
                    return Err(internal());
                }
                let (Some((pa, wa)), Some((pb, wb))) = (get(lo.0, lo.1), get(hi.0, hi.1)) else {
                    return Err(internal());
                };
                let f = base.factor(in_u, index);
                let dw = wb - wa;
                let mut h = [0.0; 4];
                let mut err = [0.0; 4];
                for c in 0..3 {
                    let (xa, xb, oc) = match c {
                        0 => (pa.x, pb.x, o.x),
                        1 => (pa.y, pb.y, o.y),
                        _ => (pa.z, pb.z, o.z),
                    };
                    let t1 = wb * (xb - xa);
                    let t2 = (xa - oc) * dw;
                    let value = f * (t1 + t2);
                    h[c] = value;
                    err[c] = f * 6.0 * EPS * (t1.abs() + t2.abs()) + 2.0 * EPS * value.abs();
                }
                h[3] = f * dw;
                err[3] = 3.0 * EPS * h[3].abs();
                v.push(h);
                e.push(err);
            }
        }
        Ok(TNet {
            degrees: (dp, dq),
            ku,
            kv,
            nu,
            nv,
            v,
            e,
        })
    }

    /// A net of zeros of the same shape (the derivative along a direction
    /// of degree 0).
    fn zero_like(&self) -> TNet {
        TNet {
            degrees: self.degrees,
            ku: self.ku.clone(),
            kv: self.kv.clone(),
            nu: self.nu,
            nv: self.nv,
            v: vec![[0.0; 4]; self.v.len()],
            e: vec![[0.0; 4]; self.e.len()],
        }
    }

    /// The derivative in u or v of this net: `f (v_{i+1} − v_i)`, within
    /// `f (e_i + e_{i+1}) + 3 ε |value|`.
    pub(super) fn diff(&self, in_u: bool) -> Result<TNet, NetworkError> {
        let along = if in_u { self.degrees.0 } else { self.degrees.1 };
        if along == 0 {
            return Ok(self.zero_like());
        }
        let (dp, dq, ku, kv, nu, nv) = self.derived_shape(in_u);
        let mut v = Vec::with_capacity(nu * nv);
        let mut e = Vec::with_capacity(nu * nv);
        for i in 0..nu {
            for j in 0..nv {
                let (hi, index) = if in_u {
                    ((i + 1, j), i)
                } else {
                    ((i, j + 1), j)
                };
                let (Some((va, ea)), Some((vb, eb))) = (self.at(i, j), self.at(hi.0, hi.1)) else {
                    return Err(internal());
                };
                let f = self.factor(in_u, index);
                let mut h = [0.0; 4];
                let mut err = [0.0; 4];
                for c in 0..4 {
                    h[c] = f * (vb[c] - va[c]);
                    err[c] = f * (ea[c] + eb[c]) + 3.0 * EPS * h[c].abs();
                }
                v.push(h);
                e.push(err);
            }
        }
        Ok(TNet {
            degrees: (dp, dq),
            ku,
            kv,
            nu,
            nv,
            v,
            e,
        })
    }

    /// The net's parameter line at `at` across (along v when `along_v`,
    /// so u is fixed): `Σ Nᵢ(at) vᵢ` with the basis values (nonnegative,
    /// summing to one), each coefficient within `Σ Nᵢ eᵢ` plus the
    /// rounding of the values (Cox–de Boor, `3 ε` a level) and of the sum:
    /// `(3 q + q + 4) ε Σ Nᵢ |vᵢ|`. On a clamped border the values are
    /// exactly one and zeros, and nothing is added.
    fn iso(&self, along_v: bool, at: f64) -> Result<(Raw, Vec<H>), NetworkError> {
        let (p, q) = self.degrees;
        let (knots, deg, n, count, other_knots, other_degree) = if along_v {
            (&self.ku, p, self.nu, self.nv, &self.kv, q)
        } else {
            (&self.kv, q, self.nv, self.nu, &self.ku, p)
        };
        let values = basis(knots, deg, n, at)?;
        let gamma = (4 * deg + 4) as f64 * EPS;
        let mut points = Vec::with_capacity(count);
        let mut errs = Vec::with_capacity(count);
        for k in 0..count {
            let (mut sum, mut err, mut mag) = ([0.0; 4], [0.0; 4], [0.0; 4]);
            for (i, b) in values.iter().enumerate() {
                if *b == 0.0 {
                    continue;
                }
                let (vi, ei) = if along_v {
                    self.at(i, k)
                } else {
                    self.at(k, i)
                }
                .ok_or_else(internal)?;
                for c in 0..4 {
                    sum[c] += b * vi[c];
                    err[c] += b.abs() * ei[c];
                    mag[c] += b.abs() * vi[c].abs();
                }
            }
            let exact = values.iter().filter(|b| **b != 0.0).count() == 1
                && values.iter().any(|b| *b == 1.0);
            for c in 0..4 {
                if !exact {
                    err[c] += gamma * mag[c];
                }
            }
            points.push(sum);
            errs.push(err);
        }
        Ok((
            Raw {
                degree: other_degree,
                knots: other_knots.clone(),
                points,
            },
            errs,
        ))
    }

    /// The net's Bézier patches, with the patches of its bounds and of its
    /// values' magnitudes beside them (the same extraction).
    fn patches(&self) -> Result<[BezierNet; 3], NetworkError> {
        let make = |pts: Vec<H>| {
            BezierNet::of_net(self.degrees, &self.ku, &self.kv, self.nu, self.nv, &pts)
                .map_err(from_trim)
        };
        Ok([
            make(self.v.clone())?,
            make(self.e.clone())?,
            make(self.v.iter().map(|h| h_abs(*h)).collect())?,
        ])
    }
}

/// The largest coefficient of each coordinate.
fn magnitude(raw: &Raw) -> [f64; 4] {
    raw.points.iter().fold([0.0; 4], |mut m, h| {
        for (m, x) in m.iter_mut().zip(h) {
            *m = m.max(x.abs());
        }
        m
    })
}

fn max4(errs: &[H]) -> [f64; 4] {
    errs.iter().fold([0.0; 4], |mut m, h| {
        for (m, x) in m.iter_mut().zip(h) {
            *m = m.max(*x);
        }
        m
    })
}

/// The line of a net cut to `[from, to]` of its running parameter (over
/// `[0, 1]`, reversed when asked) and its derivatives along brought to
/// that parameter (`order` of them: times `±(to − from)` each): the cut's
/// knot insertions are convex combinations, `16 (p + 2) ε` of the row's
/// magnitude where a cut is made; the scaling one rounding.
fn cut(
    net: &TNet,
    along_v: bool,
    at: f64,
    range: (f64, f64),
    reversed: bool,
    order: i32,
) -> Result<Row, NetworkError> {
    let (raw, errs) = net.iso(along_v, at)?;
    let (lo, hi) = raw.domain()?;
    let mut allow = max4(&errs);
    let (from, to) = range;
    let whole = from <= lo && to >= hi;
    let mut raw = if whole {
        let mut r = raw;
        r.normalize()?;
        r
    } else {
        let mag = magnitude(&raw);
        for (a, m) in allow.iter_mut().zip(mag) {
            *a += 16.0 * (raw.degree + 2) as f64 * EPS * m;
        }
        piece_raw(raw, from.max(lo), to.min(hi))?
    };
    if reversed {
        raw = reverse(&raw);
    }
    let sign = if reversed { -1.0 } else { 1.0 };
    let scale = (sign * (to - from)).powi(order);
    if order != 0 {
        for h in &mut raw.points {
            for x in h.iter_mut() {
                *x *= scale;
            }
        }
        let mag = magnitude(&raw);
        for (a, m) in allow.iter_mut().zip(mag) {
            *a = *a * scale.abs() + 2.0 * EPS * m;
        }
    }
    Ok((raw, allow))
}

/// The six rows of a spline face along its parameter line (`along_v`,
/// `at`, the edge's range on it and its direction), about `o`:
/// `[X₀, X₀′, X₀″, X₁, X₁′, X₂]`.
pub(super) fn line_rows(
    s: &NurbsSurface,
    o: Vec3,
    along_v: bool,
    at: f64,
    range: (f64, f64),
    reversed: bool,
) -> Result<[Row; 6], NetworkError> {
    // Along v: the running direction is v, across is u.
    let along_u = !along_v;
    let base = TNet::of(s, o);
    let d_along = TNet::first(s, o, along_u)?;
    let d_across = TNet::first(s, o, along_v)?;
    let r = |net: &TNet, order: i32| cut(net, along_v, at, range, reversed, order);
    Ok([
        r(&base, 0)?,
        r(&d_along, 1)?,
        r(&d_along.diff(along_u)?, 2)?,
        r(&d_across, 0)?,
        r(&d_across.diff(along_u)?, 1)?,
        r(&d_across.diff(along_v)?, 0)?,
    ])
}

/// A row as Bézier pieces over shared bounds (each piece its own: rows
/// of a rational face may jump where its homogeneous derivatives do),
/// with the bound on its coefficients' rounding.
#[derive(Clone, Debug)]
struct Pieces {
    degree: usize,
    bounds: Vec<f64>,
    pts: Vec<Vec<H>>,
    allow: [f64; 4],
}

impl Pieces {
    fn magnitude(&self) -> [f64; 4] {
        self.pts.iter().flatten().fold([0.0; 4], |mut m, h| {
            for (m, x) in m.iter_mut().zip(h) {
                *m = m.max(x.abs());
            }
            m
        })
    }

    /// Raised to `degree` piece by piece (convex combinations).
    fn at_degree(&self, degree: usize) -> Pieces {
        let mut pts = self.pts.clone();
        for piece in &mut pts {
            while piece.len() < degree + 1 {
                let n = piece.len() - 1;
                let mut next = Vec::with_capacity(n + 2);
                for i in 0..=n + 1 {
                    let a = i as f64 / (n + 1) as f64;
                    let lo = if i > 0 {
                        piece.get(i - 1).copied()
                    } else {
                        None
                    };
                    let hi = piece.get(i).copied();
                    let mut h = [0.0; 4];
                    for (c, slot) in h.iter_mut().enumerate() {
                        *slot = a * lo.map_or(0.0, |x| x[c]) + (1.0 - a) * hi.map_or(0.0, |x| x[c]);
                    }
                    next.push(h);
                }
                *piece = next;
            }
        }
        Pieces {
            degree: degree.max(self.degree),
            bounds: self.bounds.clone(),
            pts,
            allow: self.allow,
        }
    }

    /// The row as a spline whose every inner knot is there once more than
    /// its degree (each piece's points kept, joined or not).
    fn into_row(self) -> Result<Row, NetworkError> {
        let d = self.degree;
        let (Some(&a), Some(&b)) = (self.bounds.first(), self.bounds.last()) else {
            return Err(internal());
        };
        let mut knots = vec![a; d + 1];
        for k in self
            .bounds
            .iter()
            .skip(1)
            .take(self.bounds.len().saturating_sub(2))
        {
            knots.extend(std::iter::repeat_n(*k, d + 1));
        }
        knots.extend(std::iter::repeat_n(b, d + 1));
        let points: Vec<H> = self.pts.into_iter().flatten().collect();
        if points.len() + d + 1 != knots.len() {
            return Err(internal());
        }
        Ok((
            Raw {
                degree: d,
                knots,
                points,
            },
            self.allow,
        ))
    }
}

/// A net composed with the curve `uv` in its parameters, piece by piece,
/// with its allowance: the bounds' net composed as magnitudes (what the
/// net's own rounding becomes) plus `(4 D + 6 (p + q) + 24) ε` times the
/// values' magnitudes composed (the Bézier extraction's knot insertions
/// and the composition's products and sums, `D` the composed degree).
fn composed(net: &TNet, uv: &NurbsCurve) -> Result<Pieces, NetworkError> {
    let [values, bounds, mags] = net.patches()?;
    let (degree, cuts, pts) = compose_net_pieces(&values, uv, Composed::Any).map_err(from_trim)?;
    let (_, _, propagated) =
        compose_net_pieces(&bounds, uv, Composed::Magnitude).map_err(from_trim)?;
    let (_, _, magnitudes) =
        compose_net_pieces(&mags, uv, Composed::Magnitude).map_err(from_trim)?;
    let (p, q) = net.degrees;
    let gamma = (4 * degree + 6 * (p + q) + 24) as f64 * EPS;
    let largest = |pieces: &[Vec<H>]| -> [f64; 4] {
        pieces.iter().flatten().fold([0.0; 4], |mut m, h| {
            for (m, x) in m.iter_mut().zip(h) {
                *m = m.max(x.abs());
            }
            m
        })
    };
    let (pm, mm) = (largest(&propagated), largest(&magnitudes));
    let mut allow = [0.0; 4];
    for c in 0..4 {
        allow[c] = pm[c] * (1.0 + 64.0 * EPS) + gamma * mm[c];
    }
    Ok(Pieces {
        degree,
        bounds: cuts,
        pts,
        allow,
    })
}

/// A scalar polynomial piece by piece on the rows' bounds, with a bound
/// on its coefficients' rounding relative to its largest.
struct Scalar {
    degree: usize,
    pts: Vec<Vec<f64>>,
    relative: f64,
}

impl Scalar {
    fn max(&self) -> f64 {
        self.pts
            .iter()
            .flatten()
            .fold(0.0_f64, |m, x| m.max(x.abs()))
    }

    fn times(&self, other: &Scalar) -> Scalar {
        let degree = self.degree + other.degree;
        Scalar {
            degree,
            pts: self
                .pts
                .iter()
                .zip(&other.pts)
                .map(|(a, b)| bernstein_product(a, b))
                .collect(),
            relative: self.relative + other.relative + 4.0 * (degree + 2) as f64 * EPS,
        }
    }
}

/// Coordinate `k` of the derivative of order `order` of the parameter
/// curve `uv`, on the pieces `bounds` (each inside one of `uv`'s pieces:
/// that piece's derivative restricted to it, de Casteljau).
fn scalar(uv: &NurbsCurve, k: usize, order: usize, bounds: &[f64]) -> Result<Scalar, NetworkError> {
    let mut raw = Raw::of(uv);
    for _ in 0..order {
        raw = derivative(&raw)?;
    }
    let (own, pieces) = super::pieces_any(&raw)?;
    let mut pts = Vec::with_capacity(bounds.len().saturating_sub(1));
    for w in bounds.windows(2) {
        let [b0, b1] = w else { continue };
        let mid = 0.5 * (b0 + b1);
        let parent = own
            .windows(2)
            .position(|o| matches!(o, [x, y] if *x <= mid && mid <= *y))
            .ok_or_else(internal)?;
        let (Some(&t0), Some(&t1), Some(piece)) =
            (own.get(parent), own.get(parent + 1), pieces.get(parent))
        else {
            return Err(internal());
        };
        if !(t1 > t0) {
            return Err(internal());
        }
        let coefficients: Vec<f64> = piece.iter().map(|h| h[k]).collect();
        let (lo, hi) = (
            ((b0 - t0) / (t1 - t0)).clamp(0.0, 1.0),
            ((b1 - t0) / (t1 - t0)).clamp(0.0, 1.0),
        );
        pts.push(restricted(&coefficients, lo, hi));
    }
    let degree = raw.degree;
    Ok(Scalar {
        degree,
        pts,
        // The derivative's differences and factor (3 ε a level) and the
        // restriction's convex combinations (`2 ε` a level of de
        // Casteljau, twice).
        relative: (3 * order + 4 * degree + 4) as f64 * EPS,
    })
}

/// `s · r` piece by piece with its allowance: `max|s|` times `r`'s, plus
/// `s`'s relative bound and the product's rounding (`4 (m + n + 2) ε`)
/// times `max|s|` and `r`'s magnitude (each Bernstein coefficient of a
/// product is a convex combination of products of coefficients).
fn scaled(s: &Scalar, r: &Pieces) -> Result<Pieces, NetworkError> {
    if s.pts.len() != r.pts.len() {
        return Err(internal());
    }
    let degree = s.degree + r.degree;
    let mut pts = Vec::with_capacity(r.pts.len());
    for (a, piece) in s.pts.iter().zip(&r.pts) {
        let mut out = vec![[0.0; 4]; degree + 1];
        for c in 0..4 {
            let column: Vec<f64> = piece.iter().map(|h| h[c]).collect();
            for (slot, x) in out.iter_mut().zip(bernstein_product(a, &column)) {
                slot[c] = x;
            }
        }
        pts.push(out);
    }
    let (ms, mr) = (s.max(), r.magnitude());
    let k = 4.0 * (degree + 2) as f64 * EPS;
    let mut allow = [0.0; 4];
    for c in 0..4 {
        allow[c] = ms * r.allow[c] + (s.relative + k) * ms * mr[c];
    }
    Ok(Pieces {
        degree,
        bounds: r.bounds.clone(),
        pts,
        allow,
    })
}

/// `a + f b` piece by piece with its allowance: the two allowances, and
/// the degree raising and the sum, `(2 D + 4) ε` of the magnitudes.
fn sum(a: &Pieces, b: &Pieces, f: f64) -> Result<Pieces, NetworkError> {
    if a.bounds != b.bounds || a.pts.len() != b.pts.len() {
        return Err(internal());
    }
    let degree = a.degree.max(b.degree);
    let (a, b) = (a.at_degree(degree), b.at_degree(degree));
    let pts = a
        .pts
        .iter()
        .zip(&b.pts)
        .map(|(x, y)| {
            x.iter()
                .zip(y)
                .map(|(p, q)| {
                    [
                        p[0] + f * q[0],
                        p[1] + f * q[1],
                        p[2] + f * q[2],
                        p[3] + f * q[3],
                    ]
                })
                .collect()
        })
        .collect();
    let (ma, mb) = (a.magnitude(), b.magnitude());
    let k = (2 * degree + 4) as f64 * EPS;
    let mut allow = [0.0; 4];
    for c in 0..4 {
        allow[c] = a.allow[c] + f.abs() * b.allow[c] + k * (ma[c] + f.abs() * mb[c]);
    }
    Ok(Pieces {
        degree,
        bounds: a.bounds.clone(),
        pts,
        allow,
    })
}

/// The six rows of a spline face written across the curve `uv` in its
/// parameters (the ribbon `Φ(c(t) + ρ e)`, `e` along u when `across_u`),
/// about `o`, over `uv`'s parameter: `Y₀ = Φ∘c`,
/// `Y₀′ = Φ_u∘c u′ + Φ_v∘c v′`,
/// `Y₀″ = Φ_uu∘c u′² + 2 Φ_uv∘c u′v′ + Φ_vv∘c v′² + Φ_u∘c u″ + Φ_v∘c v″`,
/// `Y₁ = Φ_e∘c`, `Y₁′ = Φ_eu∘c u′ + Φ_ev∘c v′`, `Y₂ = Φ_ee∘c`. Each a
/// spline of pieces that need not meet (a rational face's homogeneous
/// derivatives jump across its knot lines of full multiplicity).
pub(super) fn ribbon_rows(
    s: &NurbsSurface,
    o: Vec3,
    uv: &NurbsCurve,
    across_u: bool,
) -> Result<[Row; 6], NetworkError> {
    let base = TNet::of(s, o);
    let du = TNet::first(s, o, true)?;
    let dv = TNet::first(s, o, false)?;
    let duu = du.diff(true)?;
    let duv = du.diff(false)?;
    let dvv = dv.diff(false)?;
    let y0 = composed(&base, uv)?;
    let bounds = y0.bounds.clone();
    let (cu, cv) = (composed(&du, uv)?, composed(&dv, uv)?);
    let (cuu, cuv, cvv) = (
        composed(&duu, uv)?,
        composed(&duv, uv)?,
        composed(&dvv, uv)?,
    );
    let u1 = scalar(uv, 0, 1, &bounds)?;
    let v1 = scalar(uv, 1, 1, &bounds)?;
    let u2 = scalar(uv, 0, 2, &bounds)?;
    let v2 = scalar(uv, 1, 2, &bounds)?;
    let y0d = sum(&scaled(&u1, &cu)?, &scaled(&v1, &cv)?, 1.0)?;
    let y0dd = {
        let a = scaled(&u1.times(&u1), &cuu)?;
        let b = scaled(&u1.times(&v1), &cuv)?;
        let c2 = scaled(&v1.times(&v1), &cvv)?;
        let d = scaled(&u2, &cu)?;
        let e = scaled(&v2, &cv)?;
        sum(&sum(&sum(&a, &b, 2.0)?, &c2, 1.0)?, &sum(&d, &e, 1.0)?, 1.0)?
    };
    let (y1, y1d, y2) = if across_u {
        let y1d = sum(&scaled(&u1, &cuu)?, &scaled(&v1, &cuv)?, 1.0)?;
        (cu, y1d, cuu)
    } else {
        let y1d = sum(&scaled(&u1, &cuv)?, &scaled(&v1, &cvv)?, 1.0)?;
        (cv, y1d, cvv)
    };
    Ok([
        y0.into_row()?,
        y0d.into_row()?,
        y0dd.into_row()?,
        y1.into_row()?,
        y1d.into_row()?,
        y2.into_row()?,
    ])
}
