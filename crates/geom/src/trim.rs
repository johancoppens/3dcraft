//! Curves on a surface, made exactly (docs/design/surfacing.md › *A face on
//! a surface, bounded by curves on it*).
//!
//! A boundary of a trimmed face is a curve `c(t) = (u(t), v(t))` in the
//! surface's parameters, held as a [`NurbsCurve`] whose `x` and `y` are `u`
//! and `v` (`z` is 0). Its edge in space is `S(c(t))`, which for a
//! (rational) tensor-product B-spline `S` of degrees `(p, q)` and a
//! (rational) spline `c` of degree `d` is a (rational) B-spline of degree
//! `d (p + q)`: [`compose`] makes it exactly by functional composition
//! (DeRose, Goldman, Hagen & Mann, *Functional composition algorithms via
//! blossoming*, ACM TOG 12(2), 1993), here by Bernstein products. `c` is
//! cut where it crosses a knot line of `S` (roots isolated by subdividing
//! Bernstein forms, never sampled), and on every piece the Bézier patch it
//! lies in is composed with it on homogeneous points:
//! `S(c) = Σᵢⱼ Hᵢⱼ (p choose i)(q choose j) Uˢⁱ (W − Uˢ)ᵖ⁻ⁱ Vˢʲ (W − Vˢ)^(q−j)`,
//! `Uˢ = (U − u₀ W) / Δu` the patch's local parameter times the curve's
//! weight `W` (the common factor `W^(p+q)` is a homogeneous scale and is
//! left out).
//!
//! [`project_points`] and [`uv_through`] make such a `c` for a curve in
//! space: its points taken to the surface (closest points, or along a
//! direction), the cubic through their parameters, sampled more densely
//! until the curve on the surface lies within the asked distance of the
//! projection at the midpoints between samples.

use crate::refine::restricted;
use crate::refine::{Raw, bernstein_product, bezier_pieces_raw, from_pieces_raw, interpolate};
use crate::{GeomError, MAX_DEGREE, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Frame, Portable, Vec3};
use std::fmt;

type H = [f64; 4];

/// Why no curve on the surface was made.
#[derive(Clone, Debug, PartialEq)]
pub enum TrimError {
    /// Refused; the text says why, with the numbers.
    Refused(String),
    Geom(GeomError),
}

impl fmt::Display for TrimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrimError::Refused(why) => f.write_str(why),
            TrimError::Geom(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TrimError {}

impl From<GeomError> for TrimError {
    fn from(e: GeomError) -> Self {
        TrimError::Geom(e)
    }
}

fn refuse<T>(why: String) -> Result<T, TrimError> {
    Err(TrimError::Refused(why))
}

fn internal() -> TrimError {
    TrimError::Geom(GeomError::InvalidParameter(
        "a curve on a surface lost track of its pieces; this is a bug",
    ))
}

/// Most pieces a composed curve is cut into (crossings of knot lines and
/// subdivisions for positive weights together).
pub const MAX_COMPOSED_PIECES: usize = 1 << 14;

/// Crossings of knot lines closer than this (in a piece's own parameter,
/// 0 to 1) to its ends or to each other are taken as one ([`compose_net`]).
const CUT_MERGE: f64 = 1e-10;

/// Most samples taken along one curve projected onto a surface.
pub const MAX_PROJECTED_SAMPLES: usize = 4096;

/// The degree of `S(c)` for a surface of `degrees` and a curve of degree
/// `d`.
pub fn composed_degree(degrees: (usize, usize), d: usize) -> usize {
    d.saturating_mul(degrees.0.saturating_add(degrees.1))
}

/// The value at `t` of the Bernstein polynomial with coefficients `c`
/// (de Casteljau).
fn bern_eval(c: &[f64], t: f64) -> f64 {
    let mut row = c.to_vec();
    while row.len() > 1 {
        row = row
            .windows(2)
            .map(|w| match w {
                [a, b] => a * (1.0 - t) + b * t,
                _ => 0.0,
            })
            .collect();
    }
    row.first().copied().unwrap_or(0.0)
}

/// The places in `(0, 1)` where the Bernstein polynomial `c` may change
/// sign: every root, isolated by subdivision (the polynomial lies in the
/// hull of its coefficients, so where they share one strict sign there is
/// none), and refined by bisection where the ends differ in sign. A root
/// that only touches zero is listed too (an extra cut is harmless). At
/// most `cap` places.
///
/// `floor` is the absolute size below which a coefficient is rounding:
/// for `U − k W` it is of the size of `k` and the domain times the
/// weights, not of the polynomial's own coefficients, which along a
/// knot line are nothing but that rounding (a threshold relative to them
/// would chase the noise to the depth limit). A polynomial all within
/// it is zero: the curve runs along the line and crosses nothing.
fn sign_changes(c: &[f64], floor: f64, cap: usize) -> Result<Vec<f64>, TrimError> {
    let scale = c.iter().fold(0.0_f64, |m, x| m.max(x.abs()));
    if !scale.is_finite() {
        return refuse("a curve's coefficients are not finite".into());
    }
    if scale <= floor.max(0.0) {
        return Ok(Vec::new());
    }
    // Coefficients within this of zero count as zero.
    let tiny = (scale * 1e-14).max(floor);
    let mut out: Vec<f64> = Vec::new();
    // (lo, hi, coefficients over [lo, hi], depth)
    let mut stack: Vec<(f64, f64, Vec<f64>, u32)> = vec![(0.0, 1.0, c.to_vec(), 0)];
    let mut work = 0usize;
    while let Some((lo, hi, coef, depth)) = stack.pop() {
        work += 1;
        if work > 1 << 16 {
            return Err(TrimError::Geom(GeomError::NoConvergence(
                "isolating where a curve crosses a surface's knot lines",
            )));
        }
        let all_pos = coef.iter().all(|x| *x > tiny);
        let all_neg = coef.iter().all(|x| *x < -tiny);
        if all_pos || all_neg {
            continue;
        }
        let width = hi - lo;
        // Zero all over the piece within rounding: the curve touches or
        // runs along the line here; one cut is enough.
        if coef.iter().all(|x| x.abs() <= tiny) {
            let mid = 0.5 * (lo + hi);
            if mid > 0.0 && mid < 1.0 {
                out.push(mid);
            }
            if out.len() > cap {
                return refuse(format!(
                    "a curve crosses a surface's knot lines more than {cap} times"
                ));
            }
            continue;
        }
        if depth >= 48 || width < 1e-13 {
            let mid = 0.5 * (lo + hi);
            if mid > 0.0 && mid < 1.0 {
                out.push(mid);
            }
            if out.len() > cap {
                return refuse(format!(
                    "a curve crosses a surface's knot lines more than {cap} times"
                ));
            }
            continue;
        }
        // One sign change between the ends and a monotone hull (one change
        // of sign in the coefficients): bisect on values directly.
        let changes = coef
            .windows(2)
            .filter(
                |w| matches!(w, [a, b] if (*a > tiny && *b < -tiny) || (*a < -tiny && *b > tiny)),
            )
            .count();
        let zeros = coef.iter().filter(|x| x.abs() <= tiny).count();
        if changes == 1 && zeros == 0 {
            let (Some(&f0), Some(&f1)) = (coef.first(), coef.last()) else {
                return Err(internal());
            };
            let (mut a, mut b, mut fa) = (0.0_f64, 1.0_f64, f0);
            if f0.signum() != f1.signum() {
                for _ in 0..80 {
                    let m = 0.5 * (a + b);
                    let fm = bern_eval(&coef, m);
                    if fm == 0.0 {
                        a = m;
                        b = m;
                        break;
                    }
                    if fm.signum() == fa.signum() {
                        a = m;
                        fa = fm;
                    } else {
                        b = m;
                    }
                }
                let root = lo + 0.5 * (a + b) * width;
                if root > 0.0 && root < 1.0 {
                    out.push(root);
                }
                if out.len() > cap {
                    return refuse(format!(
                        "a curve crosses a surface's knot lines more than {cap} times"
                    ));
                }
                continue;
            }
        }
        let left = restricted(&coef, 0.0, 0.5);
        let right = restricted(&coef, 0.5, 1.0);
        let mid = lo + 0.5 * width;
        stack.push((mid, hi, right, depth + 1));
        stack.push((lo, mid, left, depth + 1));
    }
    out.sort_by(f64::total_cmp);
    // A root on a subdivision's midpoint is found from both sides: places
    // closer than this are one (a cut that far from a crossing reaches
    // past it by nothing the patch's extension could show).
    let mut merged: Vec<f64> = Vec::with_capacity(out.len());
    for x in out {
        match merged.last_mut() {
            Some(last) if x - *last <= 1e-9 => *last = 0.5 * (*last + x),
            _ => merged.push(x),
        }
    }
    Ok(merged)
}

/// The homogeneous control points of a rational Bézier patch, row-major
/// (`i` along u).
fn patch_points(patch: &crate::RationalBezierPatch) -> Vec<H> {
    patch
        .control_points()
        .iter()
        .zip(patch.weights())
        .map(|(p, w)| [p.x * w, p.y * w, p.z * w, w])
        .collect()
}

/// Bernstein coefficients of `a^k` (degree `k · deg a`).
fn powers(a: &[f64], k: usize) -> Vec<Vec<f64>> {
    let mut out = Vec::with_capacity(k + 1);
    out.push(vec![1.0]);
    for i in 1..=k {
        let next = out
            .get(i - 1)
            .map(|prev| bernstein_product(prev, a))
            .unwrap_or_default();
        out.push(next);
    }
    out
}

fn binomial(n: usize, k: usize) -> f64 {
    (0..k.min(n)).fold(1.0, |acc, i| acc * (n - i) as f64 / (i + 1) as f64)
}

/// The patch composed with the curve piece `(U, V, W)` (Bernstein
/// coefficients, the curve's homogeneous coordinates), the patch over
/// `u × v` of the surface's parameters: the homogeneous Bernstein points
/// of the curve on the surface, degree `d (p + q)`.
#[allow(clippy::too_many_arguments)]
fn compose_piece(
    points: &[H],
    degrees: (usize, usize),
    u: (f64, f64),
    v: (f64, f64),
    big_u: &[f64],
    big_v: &[f64],
    big_w: &[f64],
    magnitude: bool,
) -> Result<Vec<H>, TrimError> {
    let (p, q) = degrees;
    let (du, dv) = (u.1 - u.0, v.1 - v.0);
    if !(du > 0.0 && dv > 0.0) {
        return Err(internal());
    }
    let us: Vec<f64> = big_u
        .iter()
        .zip(big_w)
        .map(|(a, w)| (a - u.0 * w) / du)
        .collect();
    let vs: Vec<f64> = big_v
        .iter()
        .zip(big_w)
        .map(|(a, w)| (a - v.0 * w) / dv)
        .collect();
    let um: Vec<f64> = big_w.iter().zip(&us).map(|(w, a)| w - a).collect();
    let vm: Vec<f64> = big_w.iter().zip(&vs).map(|(w, a)| w - a).collect();
    let mag = |x: Vec<f64>| -> Vec<f64> {
        if magnitude {
            x.into_iter().map(f64::abs).collect()
        } else {
            x
        }
    };
    let (us, um, vs, vm) = (mag(us), mag(um), mag(vs), mag(vm));
    let (pu, pum) = (powers(&us, p), powers(&um, p));
    let (pv, pvm) = (powers(&vs, q), powers(&vm, q));
    let mut bu: Vec<Vec<f64>> = Vec::with_capacity(p + 1);
    for i in 0..=p {
        let (Some(a), Some(b)) = (pu.get(i), pum.get(p - i)) else {
            return Err(internal());
        };
        let c = binomial(p, i);
        bu.push(bernstein_product(a, b).into_iter().map(|x| x * c).collect());
    }
    let mut bv: Vec<Vec<f64>> = Vec::with_capacity(q + 1);
    for j in 0..=q {
        let (Some(a), Some(b)) = (pv.get(j), pvm.get(q - j)) else {
            return Err(internal());
        };
        let c = binomial(q, j);
        bv.push(bernstein_product(a, b).into_iter().map(|x| x * c).collect());
    }
    let d = big_w.len().saturating_sub(1);
    let degree = composed_degree(degrees, d);
    let mut out: Vec<H> = vec![[0.0; 4]; degree + 1];
    for (i, a) in bu.iter().enumerate() {
        for (j, b) in bv.iter().enumerate() {
            let Some(h) = points.get(i * (q + 1) + j) else {
                return Err(internal());
            };
            let prod = bernstein_product(a, b);
            if prod.len() != degree + 1 {
                return Err(internal());
            }
            for (slot, x) in out.iter_mut().zip(&prod) {
                for (s, c) in slot.iter_mut().zip(h) {
                    *s += if magnitude { c.abs() * x } else { c * x };
                }
            }
        }
    }
    Ok(out)
}

/// The sorted distinct values of `knots` strictly inside `(lo, hi)`.
fn inner_breaks(knots: &[f64], lo: f64, hi: f64) -> Vec<f64> {
    let mut out: Vec<f64> = knots
        .iter()
        .copied()
        .filter(|k| *k > lo && *k < hi)
        .collect();
    out.dedup();
    out
}

/// The index of the span `[breaks[i], breaks[i + 1]]` holding `x`
/// (clamped to the first and the last).
fn span_of(breaks: &[f64], x: f64) -> usize {
    let n = breaks.len().saturating_sub(1);
    let mut i = breaks.partition_point(|b| *b <= x).saturating_sub(1);
    if i >= n {
        i = n.saturating_sub(1);
    }
    i
}

/// A tensor-product spline as its Bézier patches on homogeneous points,
/// each over its own `[0, 1]²` (the patch `(i, j)` over the knot spans
/// `[ku[i], ku[i + 1]] × [kv[j], kv[j + 1]]` of the surface's parameters).
/// Derivatives of the surface are nets of the same kind ([`BezierNet::derivative`]),
/// whose homogeneous points need not have positive weights.
#[derive(Clone, Debug)]
pub(crate) struct BezierNet {
    pub(crate) degrees: (usize, usize),
    pub(crate) ku: Vec<f64>,
    pub(crate) kv: Vec<f64>,
    /// Row-major over the spans (`i` along u), each patch's points
    /// row-major (`a` along u) at `a * (q + 1) + b`.
    pub(crate) patches: Vec<Vec<H>>,
}

impl BezierNet {
    /// The surface's Bézier patches.
    pub(crate) fn of(surface: &NurbsSurface) -> Result<BezierNet, TrimError> {
        let ((u0, u1), (v0, v1)) = surface.domain();
        let ku = {
            let mut b = vec![u0];
            b.extend(inner_breaks(surface.knots_u(), u0, u1));
            b.push(u1);
            b
        };
        let kv = {
            let mut b = vec![v0];
            b.extend(inner_breaks(surface.knots_v(), v0, v1));
            b.push(v1);
            b
        };
        let spans = surface.bezier_spans()?;
        let columns = kv.len().saturating_sub(1);
        let rows = ku.len().saturating_sub(1);
        let mut patches: Vec<Option<Vec<H>>> = vec![None; rows * columns];
        for span in &spans {
            let i = span_of(&ku, 0.5 * (span.u.0 + span.u.1));
            let j = span_of(&kv, 0.5 * (span.v.0 + span.v.1));
            if let Some(slot) = patches.get_mut(i * columns + j) {
                *slot = Some(patch_points(&span.patch));
            }
        }
        let patches = patches
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(internal)?;
        Ok(BezierNet {
            degrees: surface.degrees(),
            ku,
            kv,
            patches,
        })
    }

    /// The Bézier patches of a tensor-product spline given as plain data:
    /// degrees, clamped knot vectors, `nu × nv` homogeneous points
    /// row-major (`i` along u, at `i * nv + j`), any 4-vectors (a
    /// derivative of a surface, or bounds carried beside one: the
    /// extraction is knot insertion, convex combinations, so a net of
    /// nonnegative bounds stays a bound of what it bounded, up to its own
    /// rounding).
    pub(crate) fn of_net(
        degrees: (usize, usize),
        knots_u: &[f64],
        knots_v: &[f64],
        nu: usize,
        nv: usize,
        points: &[H],
    ) -> Result<BezierNet, TrimError> {
        let (p, q) = degrees;
        if points.len() != nu.saturating_mul(nv) {
            return Err(internal());
        }
        // Along u first: each column j as a curve in u.
        let mut bounds_u: Vec<f64> = Vec::new();
        let mut columns: Vec<Vec<Vec<H>>> = Vec::with_capacity(nv);
        for j in 0..nv {
            let raw = Raw {
                degree: p,
                knots: knots_u.to_vec(),
                points: (0..nu)
                    .map(|i| points.get(i * nv + j).copied().ok_or_else(internal))
                    .collect::<Result<_, _>>()?,
            };
            let (b, pieces) = pieces_of(raw)?;
            bounds_u = b;
            columns.push(pieces);
        }
        let rows = bounds_u.len().saturating_sub(1);
        // Then each piece's rows along v.
        let mut bounds_v: Vec<f64> = Vec::new();
        // by_piece[k][a] = the pieces along v of local row a of u piece k.
        let mut by_piece: Vec<Vec<Vec<Vec<H>>>> = Vec::with_capacity(rows);
        for k in 0..rows {
            let mut locals = Vec::with_capacity(p + 1);
            for a in 0..=p {
                let raw = Raw {
                    degree: q,
                    knots: knots_v.to_vec(),
                    points: columns
                        .iter()
                        .map(|c| {
                            c.get(k)
                                .and_then(|piece| piece.get(a))
                                .copied()
                                .ok_or_else(internal)
                        })
                        .collect::<Result<_, _>>()?,
                };
                let (b, pieces) = pieces_of(raw)?;
                bounds_v = b;
                locals.push(pieces);
            }
            by_piece.push(locals);
        }
        let cols = bounds_v.len().saturating_sub(1);
        let mut patches = Vec::with_capacity(rows * cols);
        for locals in &by_piece {
            for l in 0..cols {
                let mut pts = Vec::with_capacity((p + 1) * (q + 1));
                for local in locals {
                    let piece = local.get(l).ok_or_else(internal)?;
                    pts.extend(piece.iter().copied());
                }
                if pts.len() != (p + 1) * (q + 1) {
                    return Err(internal());
                }
                patches.push(pts);
            }
        }
        Ok(BezierNet {
            degrees,
            ku: bounds_u,
            kv: bounds_v,
            patches,
        })
    }

    /// The homogeneous partial derivative in u (`in_u`) or v, patch by
    /// patch in the surface's own parameters: `p (H_{a+1,b} − H_{a,b}) / Δu`.
    /// With `magnitude`, the bound `p (|H_{a+1,b}| + |H_{a,b}|) / Δu` on
    /// each coordinate instead (for rounding allowances).
    pub(crate) fn derivative(&self, in_u: bool, magnitude: bool) -> Result<BezierNet, TrimError> {
        let (p, q) = self.degrees;
        let columns = self.kv.len().saturating_sub(1);
        let along = if in_u { p } else { q };
        if along == 0 {
            return Ok(BezierNet {
                degrees: self.degrees,
                ku: self.ku.clone(),
                kv: self.kv.clone(),
                patches: self
                    .patches
                    .iter()
                    .map(|x| vec![[0.0; 4]; x.len()])
                    .collect(),
            });
        }
        let (np, nq) = if in_u { (p - 1, q) } else { (p, q - 1) };
        let mut patches = Vec::with_capacity(self.patches.len());
        for (k, pts) in self.patches.iter().enumerate() {
            let (i, j) = (k / columns.max(1), k % columns.max(1));
            let width = if in_u {
                self.ku.get(i + 1).zip(self.ku.get(i)).map(|(b, a)| b - a)
            } else {
                self.kv.get(j + 1).zip(self.kv.get(j)).map(|(b, a)| b - a)
            }
            .ok_or_else(internal)?;
            if !(width > 0.0) {
                return Err(internal());
            }
            let f = along as f64 / width;
            let mut out = Vec::with_capacity((np + 1) * (nq + 1));
            for a in 0..=np {
                for b in 0..=nq {
                    let (lo, hi) = if in_u {
                        (a * (q + 1) + b, (a + 1) * (q + 1) + b)
                    } else {
                        (a * (q + 1) + b, a * (q + 1) + b + 1)
                    };
                    let (Some(x), Some(y)) = (pts.get(lo), pts.get(hi)) else {
                        return Err(internal());
                    };
                    let mut h = [0.0; 4];
                    for (c, slot) in h.iter_mut().enumerate() {
                        *slot = if magnitude {
                            f * (x[c].abs() + y[c].abs())
                        } else {
                            f * (y[c] - x[c])
                        };
                    }
                    out.push(h);
                }
            }
            patches.push(out);
        }
        Ok(BezierNet {
            degrees: (np, nq),
            ku: self.ku.clone(),
            kv: self.kv.clone(),
            patches,
        })
    }

    fn patch(&self, i: usize, j: usize) -> Option<&Vec<H>> {
        self.patches.get(i * self.kv.len().saturating_sub(1) + j)
    }
}

/// The Bézier pieces of a spline given as plain data and the knots that
/// bound them; degree 0 (a derivative of a degree-1 direction) taken span
/// by span.
fn pieces_of(mut raw: Raw) -> Result<(Vec<f64>, Vec<Vec<H>>), TrimError> {
    if raw.degree == 0 {
        let mut bounds: Vec<f64> = Vec::new();
        let mut pieces: Vec<Vec<H>> = Vec::new();
        for (i, h) in raw.points.iter().enumerate() {
            let (Some(&a), Some(&b)) = (raw.knots.get(i), raw.knots.get(i + 1)) else {
                return Err(internal());
            };
            if b > a {
                if bounds.is_empty() {
                    bounds.push(a);
                }
                bounds.push(b);
                pieces.push(vec![*h]);
            }
        }
        return Ok((bounds, pieces));
    }
    // Every knot raised to the degree, then each piece's points from where
    // the last ended, as far on as that knot's multiplicity (a derivative
    // of a spline joined only in position has knots once more than its
    // degree, and its pieces do not meet).
    let p = raw.degree;
    let missing = raw.to_multiplicity(p);
    raw.insert_all(&missing)?;
    let (a, b) = raw.domain()?;
    let runs = raw.knot_runs();
    let mut bounds = vec![a];
    bounds.extend(runs.iter().map(|r| r.0));
    bounds.push(b);
    let mut pieces = Vec::with_capacity(runs.len() + 1);
    let mut start = 0usize;
    for k in 0..=runs.len() {
        let piece = raw
            .points
            .get(start..=start + p)
            .ok_or_else(internal)?
            .to_vec();
        pieces.push(piece);
        if let Some(run) = runs.get(k) {
            start += run.1;
        }
    }
    if start + p + 1 != raw.points.len() {
        return Err(internal());
    }
    Ok((bounds, pieces))
}

/// The curve `S(c(t))` on `surface` of the parameter-space curve `uv`
/// (its `x` and `y` are `u` and `v`; `z` is ignored), exactly, as a
/// (rational) B-spline over `uv`'s own parameter, of degree
/// `deg(uv) · (p + q)`. Refused where that degree is above
/// [`MAX_DEGREE`], where `uv` leaves the surface's domain (by more than
/// rounding), or where it would take more than [`MAX_COMPOSED_PIECES`]
/// pieces.
pub fn compose(surface: &NurbsSurface, uv: &NurbsCurve) -> Result<NurbsCurve, TrimError> {
    let net = BezierNet::of(surface)?;
    Ok(compose_net(&net, uv, Composed::Positive)?.curve()?)
}

/// What [`compose_net`] makes of each piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Composed {
    /// The curve on the surface, its weights kept positive (pieces
    /// subdivided where they are not).
    Positive,
    /// Any homogeneous net (a derivative of the surface): no condition on
    /// the weights.
    Any,
    /// A bound on the magnitude of every Bernstein coefficient's terms:
    /// the net's coordinates and the curve's local coefficients taken by
    /// their magnitudes (every product then has nonnegative terms that
    /// dominate the true ones), for rounding allowances.
    Magnitude,
}

/// [`compose`] on a net of Bézier patches, as plain homogeneous data
/// (pieces joined in position: each piece's first point is the last
/// one's end).
pub(crate) fn compose_net(
    net: &BezierNet,
    uv: &NurbsCurve,
    how: Composed,
) -> Result<Raw, TrimError> {
    let (degree, bounds, pieces) = compose_net_pieces(net, uv, how)?;
    Ok(from_pieces_raw(degree, &bounds, &pieces)?)
}

/// [`compose_net`] as its Bézier pieces, each its own (a derivative net
/// joined only in position across a knot line gives pieces that do not
/// meet there): the degree, the pieces' bounds in `uv`'s parameter, and
/// their homogeneous points. The bounds depend only on `uv` and the net's
/// knot lines (and, for [`Composed::Positive`], the subdivisions for
/// positive weights), so nets of one surface and its derivatives
/// composed with one curve (not `Positive`) share them.
pub(crate) fn compose_net_pieces(
    net: &BezierNet,
    uv: &NurbsCurve,
    how: Composed,
) -> Result<(usize, Vec<f64>, Vec<Vec<H>>), TrimError> {
    let degrees = net.degrees;
    let d = uv.degree();
    let degree = composed_degree(degrees, d);
    if degree > MAX_DEGREE {
        return refuse(format!(
            "the curve on the surface would have degree {degree} ({d} × ({} + {})), above the supported {MAX_DEGREE} (owed: a lower-degree fit with a proven bound)",
            degrees.0, degrees.1
        ));
    }
    let (ku, kv) = (&net.ku, &net.kv);
    let (Some(&u0), Some(&u1), Some(&v0), Some(&v1)) =
        (ku.first(), ku.last(), kv.first(), kv.last())
    else {
        return Err(internal());
    };
    let inner_u: Vec<f64> = ku
        .get(1..ku.len().saturating_sub(1))
        .unwrap_or(&[])
        .to_vec();
    let inner_v: Vec<f64> = kv
        .get(1..kv.len().saturating_sub(1))
        .unwrap_or(&[])
        .to_vec();
    let reach = (u1 - u0).abs().max(v1 - v0).max(1.0) * 1e-9;
    let (bounds, pieces) = bezier_pieces_raw(Raw::of(uv))?;
    let mut out_bounds: Vec<f64> = Vec::new();
    let mut out_pieces: Vec<Vec<H>> = Vec::new();
    for (k, piece) in pieces.iter().enumerate() {
        let (Some(&t0), Some(&t1)) = (bounds.get(k), bounds.get(k + 1)) else {
            return Err(internal());
        };
        let big_u: Vec<f64> = piece.iter().map(|h| h[0]).collect();
        let big_v: Vec<f64> = piece.iter().map(|h| h[1]).collect();
        let big_w: Vec<f64> = piece.iter().map(|h| h[3]).collect();
        if big_w.iter().any(|w| !(w.is_finite() && *w > 0.0)) {
            return refuse("a parameter-space curve has a weight that is not positive".into());
        }
        // The piece's reach in u and v (hull of its points).
        let us: Vec<f64> = big_u.iter().zip(&big_w).map(|(a, w)| a / w).collect();
        let vs: Vec<f64> = big_v.iter().zip(&big_w).map(|(a, w)| a / w).collect();
        let lo_u = us.iter().copied().fold(f64::INFINITY, f64::min);
        let hi_u = us.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let lo_v = vs.iter().copied().fold(f64::INFINITY, f64::min);
        let hi_v = vs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        // The curve itself must stay inside the domain (its hull may
        // stray): each of U − (u0 − reach) W, (u1 + reach) W − U and the
        // same in v, in Bernstein form, is positive where all its
        // coefficients are; otherwise it is positive at the start and
        // changes sign nowhere, or the curve leaves the domain there.
        let wmax = big_w.iter().fold(0.0_f64, |m, w| m.max(w.abs()));
        let floor_u = 1e-12 * (u0.abs().max(u1.abs()) + (u1 - u0)) * wmax;
        let floor_v = 1e-12 * (v0.abs().max(v1.abs()) + (v1 - v0)) * wmax;
        let sides: [(&[f64], f64, f64, f64); 4] = [
            (&big_u, 1.0, u0 - reach, floor_u),
            (&big_u, -1.0, u1 + reach, floor_u),
            (&big_v, 1.0, v0 - reach, floor_v),
            (&big_v, -1.0, v1 + reach, floor_v),
        ];
        for (coord, sign, edge, floor) in sides {
            let c: Vec<f64> = coord
                .iter()
                .zip(&big_w)
                .map(|(a, w)| sign * (a - edge * w))
                .collect();
            if c.iter().all(|x| *x > 0.0) {
                continue;
            }
            let start = c.first().copied().unwrap_or(f64::NAN);
            let roots = sign_changes(&c, floor, MAX_COMPOSED_PIECES)?;
            let out = if start.is_nan() || start <= 0.0 {
                Some(0.0)
            } else {
                roots.first().copied()
            };
            if let Some(x) = out {
                let pt = uv.eval(t0 + x * (t1 - t0));
                return refuse(format!(
                    "the curve leaves the surface's parameter domain [{u0}, {u1}] × [{v0}, {v1}] at ({}, {})",
                    pt.x, pt.y
                ));
            }
        }
        let mut cuts: Vec<f64> = vec![0.0, 1.0];
        for &k in inner_u.iter().filter(|k| **k > lo_u && **k < hi_u) {
            let c: Vec<f64> = big_u.iter().zip(&big_w).map(|(a, w)| a - k * w).collect();
            let floor = 1e-12 * (k.abs() + (u1 - u0)) * wmax;
            cuts.extend(sign_changes(&c, floor, MAX_COMPOSED_PIECES)?);
        }
        for &k in inner_v.iter().filter(|k| **k > lo_v && **k < hi_v) {
            let c: Vec<f64> = big_v.iter().zip(&big_w).map(|(a, w)| a - k * w).collect();
            let floor = 1e-12 * (k.abs() + (v1 - v0)) * wmax;
            cuts.extend(sign_changes(&c, floor, MAX_COMPOSED_PIECES)?);
        }
        cuts.sort_by(f64::total_cmp);
        // A crossing within `CUT_MERGE` of the piece's end or of another
        // crossing is that one (a curve recovered from an edge that was
        // already cut there finds its crossings again a rounding inside
        // its ends): the piece then reaches past the knot line by that
        // much of its parameter, where the neighbouring patch differs from
        // its own by the jump of a derivative times that width to its
        // order, far below rounding, instead of leaving a sliver whose
        // knots lie within rounding of each other.
        cuts.retain(|c| *c <= 0.0 || *c >= 1.0 || (*c > CUT_MERGE && *c < 1.0 - CUT_MERGE));
        cuts.dedup_by(|a, b| (*a - *b).abs() <= CUT_MERGE);
        // Sub-pieces, each subdivided further while the composed weights
        // are not all positive.
        let mut todo: Vec<(f64, f64, u32)> = cuts
            .windows(2)
            .filter_map(|w| match w {
                [a, b] if b > a => Some((*a, *b, 0)),
                _ => None,
            })
            .rev()
            .collect();
        while let Some((a, b, depth)) = todo.pop() {
            if out_pieces.len() >= MAX_COMPOSED_PIECES {
                return refuse(format!(
                    "the curve on the surface would take more than {MAX_COMPOSED_PIECES} pieces"
                ));
            }
            let su = restricted(&big_u, a, b);
            let sv = restricted(&big_v, a, b);
            let sw = restricted(&big_w, a, b);
            let m = 0.5 * (a + b);
            let wm = bern_eval(&big_w, m);
            let (um, vm) = (bern_eval(&big_u, m) / wm, bern_eval(&big_v, m) / wm);
            let (i, j) = (span_of(ku, um), span_of(kv, vm));
            let (Some(points), Some(&pu0), Some(&pu1), Some(&pv0), Some(&pv1)) = (
                net.patch(i, j),
                ku.get(i),
                ku.get(i + 1),
                kv.get(j),
                kv.get(j + 1),
            ) else {
                return Err(internal());
            };
            let composed = compose_piece(
                points,
                degrees,
                (pu0, pu1),
                (pv0, pv1),
                &su,
                &sv,
                &sw,
                how == Composed::Magnitude,
            )?;
            if how == Composed::Positive {
                let wmax = composed.iter().fold(0.0_f64, |m, h| m.max(h[3].abs()));
                let positive = composed.iter().all(|h| h[3] > wmax * 1e-12);
                if !positive {
                    if depth >= 12 {
                        return refuse(
                            "the curve on the surface keeps weights that are not positive (a surface whose weights vary too much along it)".into(),
                        );
                    }
                    todo.push((m, b, depth + 1));
                    todo.push((a, m, depth + 1));
                    continue;
                }
            }
            if out_bounds.is_empty() {
                out_bounds.push(t0 + a * (t1 - t0));
            }
            out_bounds.push(t0 + b * (t1 - t0));
            out_pieces.push(composed);
        }
    }
    // Pieces that rounding made shorter than nothing would give repeated
    // knots beyond the degree: drop them, keeping the curve joined.
    let mut bounds: Vec<f64> = Vec::with_capacity(out_bounds.len());
    let mut pieces: Vec<Vec<H>> = Vec::with_capacity(out_pieces.len());
    if let Some(&first) = out_bounds.first() {
        bounds.push(first);
    }
    for (k, piece) in out_pieces.into_iter().enumerate() {
        let Some(&end) = out_bounds.get(k + 1) else {
            return Err(internal());
        };
        if bounds.last().is_some_and(|last| end > *last) {
            bounds.push(end);
            pieces.push(piece);
        }
    }
    if pieces.is_empty() {
        return refuse("the parameter-space curve has no length".into());
    }
    Ok((degree, bounds, pieces))
}

/// The cubic (or lower, for fewer points) parameter-space curve through
/// `uv` at the increasing `params`.
pub fn uv_through(params: &[f64], uv: &[(f64, f64)]) -> Result<NurbsCurve, TrimError> {
    let n = params.len();
    if n < 2 || uv.len() != n {
        return refuse("a curve on a surface needs at least two points".into());
    }
    let degree = 3.min(n - 1);
    let values: Vec<Vec<f64>> = uv.iter().map(|(u, v)| vec![*u, *v]).collect();
    let (knots, ctrl) = interpolate(params, degree, &values)?;
    let points: Vec<Vec3> = ctrl
        .iter()
        .map(|c| {
            Vec3::new(
                c.first().copied().unwrap_or(0.0),
                c.get(1).copied().unwrap_or(0.0),
                0.0,
            )
        })
        .collect();
    Ok(NurbsCurve::polynomial(degree, knots, points)?)
}

/// How a curve in space is taken onto a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Each point to its closest point on the surface.
    Normal,
    /// Each point along this direction (unit) to where its line meets the
    /// surface, from the closest point.
    Along(Vec3),
}

/// The parameters of `p` on `surface`, by `how`, starting from `start`
/// (the previous point's) when given.
pub fn project_point(
    surface: &Surface,
    p: Vec3,
    how: Projection,
    start: Option<(f64, f64)>,
    tolerance: f64,
) -> Result<(f64, f64), TrimError> {
    let closest = match start {
        Some(s) => surface.project_from(p, s)?,
        None => surface.project(p)?,
    };
    let Projection::Along(d) = how else {
        return Ok(closest);
    };
    let ((u0, u1), (v0, v1)) = surface.domain();
    let (mut u, mut v) = closest;
    // λ: the distance along d from p.
    let mut lambda = (surface.eval(u, v) - p).dot(d);
    for _ in 0..64 {
        let s = surface.eval(u, v);
        let f = s - p - d * lambda;
        if f.length() <= tolerance * 1e-3 {
            let normal = surface.normal(u, v).unwrap_or(d);
            if normal.dot(d).abs() < 0.05 {
                return refuse(format!(
                    "the line along the direction through ({}, {}, {}) meets the surface at a grazing angle",
                    p.x, p.y, p.z
                ));
            }
            return Ok((u, v));
        }
        let (_, su, sv) = surface.derivatives(u, v);
        let m = [su, sv, -d];
        let det = su.dot(sv.cross(-d));
        let scale = su.length() * sv.length();
        if !(det.is_finite() && det.abs() > scale * 1e-12) {
            break;
        }
        // Cramer's rule for m · (du, dv, dλ) = −f.
        let rhs = -f;
        let du = rhs.dot(m[1].cross(m[2])) / det;
        let dv = m[0].dot(rhs.cross(m[2])) / det;
        let dl = m[0].dot(m[1].cross(rhs)) / det;
        u = (u + du).clamp(u0, u1);
        v = (v + dv).clamp(v0, v1);
        lambda += dl;
        if !(u.is_finite() && v.is_finite() && lambda.is_finite()) {
            break;
        }
    }
    refuse(format!(
        "the line along the direction through ({}, {}, {}) does not meet the surface",
        p.x, p.y, p.z
    ))
}

/// The parameter-space curve of `curve` over `range` taken onto
/// `surface` by `how`: the cubic through the parameters of its points at
/// its own parameters, with as many samples (from 16, doubled, at most
/// [`MAX_PROJECTED_SAMPLES`]) as make `S(c(t))` lie within `within` of the
/// projection of the curve's point at every midpoint between samples.
/// Returns the curve in parameter space over `range` and the largest
/// distance found at the midpoints.
pub fn project_points(
    surface: &Surface,
    curve: &crate::Curve3,
    range: (f64, f64),
    how: Projection,
    within: f64,
    tolerance: f64,
) -> Result<(NurbsCurve, f64), TrimError> {
    let (a, b) = range;
    if !(a.is_finite() && b.is_finite() && b > a) {
        return refuse("a curve projected onto a surface needs an increasing range".into());
    }
    let mut count = 16usize;
    loop {
        let mut params = Vec::with_capacity(count + 1);
        let mut uv: Vec<(f64, f64)> = Vec::with_capacity(count + 1);
        let mut last: Option<(f64, f64)> = None;
        for i in 0..=count {
            let t = a + (b - a) * i as f64 / count as f64;
            let at = project_point(surface, curve.eval(t), how, last, tolerance)?;
            params.push(t);
            uv.push(at);
            last = Some(at);
        }
        let c = uv_through(&params, &uv)?;
        // Midpoints: the curve on the surface against the projection.
        let mut worst = 0.0_f64;
        for i in 0..count {
            let t = a + (b - a) * (i as f64 + 0.5) / count as f64;
            let start = uv.get(i).copied();
            let target = project_point(surface, curve.eval(t), how, start, tolerance)?;
            let q = c.eval(t);
            let gap = surface
                .eval(q.x, q.y)
                .distance(surface.eval(target.0, target.1));
            worst = worst.max(gap);
        }
        if worst <= within {
            return Ok((c, worst));
        }
        if count * 2 > MAX_PROJECTED_SAMPLES {
            return refuse(format!(
                "a curve projected onto the surface is not followed within {within:e} by {MAX_PROJECTED_SAMPLES} samples (off by {worst:e})"
            ));
        }
        count *= 2;
    }
}

/// The surface of revolution about `axis` (its z), a full turn from the
/// angle `start`, of the meridian `meridian` drawn in the plane of the
/// axis frame's x (the distance from the axis) and z (Piegl & Tiller,
/// *The NURBS Book*, §8.5): control points `o + r_j (c_i,x x + c_i,y y) +
/// z_j z`, weights `w_i m_j`, `c_i, w_i` the unit circle's rational
/// quadratic pieces. Exact.
fn revolution(axis: &Frame, start: f64, meridian: &NurbsCurve) -> Result<NurbsSurface, TrimError> {
    let unit = Frame::new(
        Vec3::ZERO,
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(1.0, 0.0, 0.0),
    )
    .ok_or_else(internal)?;
    let circle = crate::network::arc(&unit, 1.0, start, std::f64::consts::TAU)?;
    let (cu, cw) = (circle.control_points(), circle.weights());
    let (mp, mw) = (meridian.control_points(), meridian.weights());
    let mut ctrl = Vec::with_capacity(cu.len() * mp.len());
    let mut weights = Vec::with_capacity(cu.len() * mp.len());
    for (c, w) in cu.iter().zip(cw) {
        for (m, wm) in mp.iter().zip(mw) {
            ctrl.push(axis.point(m.x * c.x, m.x * c.y, m.z));
            weights.push(w * wm);
        }
    }
    Ok(NurbsSurface::new(
        2,
        meridian.degree(),
        circle.knots().to_vec(),
        meridian.knots().to_vec(),
        cu.len(),
        mp.len(),
        ctrl,
        weights,
    )?)
}

/// The angle about `frame`'s z of the mean of `points`' directions from
/// its axis, and whether every point keeps clear of the angle opposite
/// it (where a full turn from there starts and ends) by more than
/// `clear` radians.
fn mean_angle(frame: &Frame, points: &[Vec3], clear: f64) -> Option<f64> {
    let (mut sx, mut sy) = (0.0, 0.0);
    for p in points {
        let (x, y, _) = frame.local(*p);
        let r = (x * x + y * y).sqrt();
        if r > 0.0 {
            sx += x / r;
            sy += y / r;
        }
    }
    if sx == 0.0 && sy == 0.0 {
        return None;
    }
    let mean = sy.patan2(sx);
    let seam = mean + std::f64::consts::PI;
    for p in points {
        let (x, y, _) = frame.local(*p);
        let a = y.patan2(x);
        let mut d = (a - seam) % std::f64::consts::TAU;
        if d > std::f64::consts::PI {
            d -= std::f64::consts::TAU;
        }
        if d < -std::f64::consts::PI {
            d += std::f64::consts::TAU;
        }
        if d.abs() <= clear {
            return None;
        }
    }
    Some(mean)
}

/// A rational spline form of an analytic surface (cylinder, cone, sphere,
/// torus) that is exactly the same surface, its parameters running round
/// from the side opposite `points` (so a region about them is inside its
/// domain, away from its seam and its poles), for curves on it made exact
/// by [`compose`]. Refused where the points go round the axis (a loop that
/// encircles it), reach a cone's apex, or come within 10° of the poles of
/// the sphere's frame chosen square to their mean direction.
pub fn rational_form(surface: &Surface, points: &[Vec3]) -> Result<NurbsSurface, TrimError> {
    let clear = 10.0_f64.to_radians();
    let around = |frame: &Frame| {
        mean_angle(frame, points, clear).ok_or_else(|| {
            TrimError::Refused(
                "the curves go round the surface's axis; a loop that encircles it is not trimmed yet".into(),
            )
        })
    };
    let line = |a: Vec3, b: Vec3| NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![a, b]);
    let heights = |frame: &Frame| {
        let zs: Vec<f64> = points.iter().map(|p| frame.local(*p).2).collect();
        let lo = zs.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = zs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let margin = 0.25 * (hi - lo) + 1e-6 * (lo.abs().max(hi.abs()).max(1.0));
        (lo - margin, hi + margin)
    };
    let meridian_plane = Frame::new(
        Vec3::ZERO,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
    )
    .ok_or_else(internal)?;
    match surface {
        Surface::Cylinder(c) => {
            let frame = *c.frame();
            let start = around(&frame)? + std::f64::consts::PI;
            let (z0, z1) = heights(&frame);
            let r = c.radius();
            revolution(
                &frame,
                start,
                &line(Vec3::new(r, 0.0, z0), Vec3::new(r, 0.0, z1))?,
            )
        }
        Surface::Cone(c) => {
            let frame = *c.frame();
            let start = around(&frame)? + std::f64::consts::PI;
            let (z0, z1) = heights(&frame);
            let apex = c.apex_v();
            let (r0, r1) = (
                c.radius() + z0 * c.half_angle().ptan(),
                c.radius() + z1 * c.half_angle().ptan(),
            );
            if (apex - z0) * (apex - z1) <= 0.0 || r0 <= 0.0 || r1 <= 0.0 {
                return refuse(
                    "the curves reach the cone's apex; a trim about it is not supported yet".into(),
                );
            }
            revolution(
                &frame,
                start,
                &line(Vec3::new(r0, 0.0, z0), Vec3::new(r1, 0.0, z1))?,
            )
        }
        Surface::Sphere(s) => {
            let centre = s.frame().origin();
            let mut mean = Vec3::ZERO;
            for p in points {
                if let Some(d) = (*p - centre).normalized() {
                    mean += d;
                }
            }
            let mean = mean.normalized().ok_or_else(|| {
                TrimError::Refused("the curves go all round the sphere; not trimmed yet".into())
            })?;
            let helper = if mean.x.abs() < 0.9 {
                Vec3::new(1.0, 0.0, 0.0)
            } else {
                Vec3::new(0.0, 1.0, 0.0)
            };
            let axis = mean.cross(helper).normalized().ok_or_else(internal)?;
            let frame = Frame::new(centre, axis, mean).ok_or_else(internal)?;
            // Every point at least 10° from the poles of this frame.
            for p in points {
                let (x, y, z) = frame.local(*p);
                if z.patan2((x * x + y * y).sqrt()).abs() > std::f64::consts::FRAC_PI_2 - clear {
                    return refuse("the curves reach too far round the sphere (within 10° of a pole of any frame about their middle); not trimmed yet".into());
                }
            }
            let start = around(&frame)? + std::f64::consts::PI;
            let half = crate::network::arc(
                &meridian_plane,
                s.radius(),
                -std::f64::consts::FRAC_PI_2,
                std::f64::consts::PI,
            )?;
            revolution(&frame, start, &half)
        }
        Surface::Torus(t) => {
            let frame = *t.frame();
            let start = around(&frame)? + std::f64::consts::PI;
            let (major, minor) = t.radii();
            // The tube's angle of every point, about the tube's centre in
            // its meridian plane.
            let tube: Vec<Vec3> = points
                .iter()
                .map(|p| {
                    let (x, y, z) = frame.local(*p);
                    Vec3::new((x * x + y * y).sqrt() - major, z, 0.0)
                })
                .collect();
            let flat = Frame::WORLD;
            let tube_mean = mean_angle(&flat, &tube, clear).ok_or_else(|| {
                TrimError::Refused("the curves go round the torus's tube; not trimmed yet".into())
            })?;
            let tube_frame = Frame::new(
                Vec3::new(major, 0.0, 0.0),
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
            )
            .ok_or_else(internal)?;
            let circle = crate::network::arc(
                &tube_frame,
                minor,
                tube_mean + std::f64::consts::PI,
                std::f64::consts::TAU,
            )?;
            revolution(&frame, start, &circle)
        }
        Surface::Nurbs(n) => Ok(n.clone()),
        Surface::Bezier(b) => Ok(NurbsSurface::from_bezier(b)?),
        Surface::Plane(_) => {
            refuse("a plane takes its curves by an affine map, not a rational form".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NurbsSurface;

    fn bilinear_plane() -> NurbsSurface {
        NurbsSurface::new(
            1,
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![0.0, 0.0, 1.0, 1.0],
            2,
            2,
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 2.0, 0.0),
                Vec3::new(3.0, 0.0, 0.0),
                Vec3::new(3.0, 2.0, 0.0),
            ],
            vec![1.0; 4],
        )
        .unwrap()
    }

    fn wavy() -> NurbsSurface {
        // A bicubic with two spans each way, bumpy and rational.
        let n = 5;
        let mut pts = Vec::new();
        let mut w = Vec::new();
        for i in 0..n {
            for j in 0..n {
                let z = ((i * 7 + j * 3) % 5) as f64 * 0.3;
                pts.push(Vec3::new(i as f64, j as f64, z));
                w.push(1.0 + 0.2 * ((i + 2 * j) % 3) as f64);
            }
        }
        let k = vec![0.0, 0.0, 0.0, 0.0, 0.4, 1.0, 1.0, 1.0, 1.0];
        NurbsSurface::new(3, 3, k.clone(), k, n, n, pts, w).unwrap()
    }

    #[test]
    fn composition_is_the_surface_along_the_curve() {
        let s = wavy();
        // A cubic in uv crossing both inner knot lines several times.
        let uv = NurbsCurve::polynomial(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::new(0.1, 0.1, 0.0),
                Vec3::new(0.9, 0.2, 0.0),
                Vec3::new(0.1, 0.6, 0.0),
                Vec3::new(0.8, 0.9, 0.0),
                Vec3::new(0.3, 0.95, 0.0),
            ],
        )
        .unwrap();
        let c = compose(&s, &uv).unwrap();
        assert_eq!(c.degree(), 18);
        for i in 0..=400 {
            let t = i as f64 / 400.0;
            let q = uv.eval(t);
            let want = s.eval(q.x, q.y);
            let got = c.eval(t);
            assert!(got.distance(want) < 1e-12, "t {t}: {got:?} vs {want:?}");
        }
    }

    #[test]
    fn a_line_on_a_bilinear_plane_stays_a_line() {
        let s = bilinear_plane();
        let uv = NurbsCurve::polynomial(
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![Vec3::new(0.0, 0.5, 0.0), Vec3::new(1.0, 0.5, 0.0)],
        )
        .unwrap();
        let c = compose(&s, &uv).unwrap();
        for i in 0..=10 {
            let t = i as f64 / 10.0;
            assert!(c.eval(t).distance(Vec3::new(3.0 * t, 1.0, 0.0)) < 1e-14);
        }
    }

    #[test]
    fn too_high_a_degree_and_leaving_the_domain_are_refused() {
        let s = wavy();
        let quintic = NurbsCurve::polynomial(
            6,
            vec![0.0; 7].into_iter().chain(vec![1.0; 7]).collect(),
            (0..7)
                .map(|i| Vec3::new(0.1 * i as f64, 0.5, 0.0))
                .collect(),
        )
        .unwrap();
        assert!(
            compose(&s, &quintic)
                .unwrap_err()
                .to_string()
                .contains("degree 36")
        );
        let out = NurbsCurve::polynomial(
            1,
            vec![0.0, 0.0, 1.0, 1.0],
            vec![Vec3::new(0.5, 0.5, 0.0), Vec3::new(1.5, 0.5, 0.0)],
        )
        .unwrap();
        assert!(
            compose(&s, &out)
                .unwrap_err()
                .to_string()
                .contains("leaves")
        );
    }

    #[test]
    fn a_curve_along_a_knot_line_is_composed() {
        // u = 0.4 is the wavy surface's inner knot; the curve runs along it
        // in v, rational and cut at an inner knot of its own, so U − 0.4 W
        // is rounding noise, not zero: it must count as zero, not be
        // isolated root by root.
        let s = wavy();
        let mut found_noise = false;
        for cut in [0.45, 0.3, 0.7] {
            let uv = NurbsCurve::new(
                3,
                vec![0.0, 0.0, 0.0, 0.0, cut, 1.0, 1.0, 1.0, 1.0],
                (0..5)
                    .map(|i| Vec3::new(0.4, 0.1 + 0.2 * i as f64, 0.0))
                    .collect(),
                vec![1.0, 1.7, 0.6, 1.3, 1.0],
            )
            .unwrap();
            let (_, pieces) = bezier_pieces_raw(Raw::of(&uv)).unwrap();
            // Noise of both signs in one piece: no sign is shared.
            found_noise |= pieces.iter().any(|p| {
                let c: Vec<f64> = p.iter().map(|h| h[0] - 0.4 * h[3]).collect();
                c.iter().any(|x| *x > 0.0) && c.iter().any(|x| *x < 0.0)
            });
            let c = compose(&s, &uv).unwrap();
            for i in 0..=50 {
                let t = i as f64 / 50.0;
                let q = uv.eval(t);
                assert!(c.eval(t).distance(s.eval(q.x, q.y)) < 1e-12);
            }
        }
        assert!(
            found_noise,
            "the test must carry rounding of both signs off the line"
        );
    }

    #[test]
    fn a_curve_that_leaves_the_domain_between_its_samples_is_refused() {
        // Ends and middle inside [0, 1]^2; v reaches 1.06 near t = 0.25
        // and -0.06 near t = 0.75.
        let s = wavy();
        let uv = NurbsCurve::polynomial(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::new(0.2, 0.5, 0.0),
                Vec3::new(0.4, 2.5, 0.0),
                Vec3::new(0.6, -1.5, 0.0),
                Vec3::new(0.8, 0.5, 0.0),
            ],
        )
        .unwrap();
        for t in [0.0, 0.5, 1.0] {
            let q = uv.eval(t);
            assert!(q.y > 0.0 && q.y < 1.0);
        }
        let e = compose(&s, &uv).unwrap_err().to_string();
        assert!(e.contains("leaves the surface's parameter domain"), "{e}");
        // Touching the border from inside is not leaving it.
        let touching = NurbsCurve::polynomial(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![
                Vec3::new(0.2, 0.5, 0.0),
                Vec3::new(0.5, 1.5, 0.0),
                Vec3::new(0.8, 0.5, 0.0),
            ],
        )
        .unwrap();
        assert!(compose(&s, &touching).is_ok());
    }

    #[test]
    fn sign_changes_finds_every_root() {
        // (t - 0.2)(t - 0.5)(t - 0.7) in power form, to Bernstein.
        let f = |t: f64| (t - 0.2) * (t - 0.5) * (t - 0.7);
        // Bernstein coefficients by interpolation at 4 points is overkill;
        // build from the power basis: c_k = Σ_{i<=k} C(k,i)/C(3,i) a_i.
        let a = [-0.07, 0.59, -1.4, 1.0];
        let c: Vec<f64> = (0..4)
            .map(|k| {
                (0..=k)
                    .map(|i| binomial(k, i) / binomial(3, i) * a[i])
                    .sum()
            })
            .collect();
        for t in [0.0, 0.3, 1.0] {
            assert!((bern_eval(&c, t) - f(t)).abs() < 1e-14);
        }
        let r = sign_changes(&c, 1e-15, 10).unwrap();
        assert_eq!(r.len(), 3, "{r:?}");
        for (x, want) in r.iter().zip([0.2, 0.5, 0.7]) {
            assert!((x - want).abs() < 1e-12);
        }
    }

    #[test]
    fn rational_forms_are_the_analytic_surfaces() {
        use crate::{Cone, Cylinder, Sphere, Torus};
        let frame = Frame::new(
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(0.2, 0.3, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
        )
        .unwrap();
        let surfaces = [
            Surface::Cylinder(Cylinder::new(frame, 4.0).unwrap()),
            Surface::Cone(Cone::new(frame, 4.0, 0.3).unwrap()),
            Surface::Sphere(Sphere::new(frame, 4.0).unwrap()),
            Surface::Torus(Torus::new(frame, 6.0, 2.0).unwrap()),
        ];
        for s in &surfaces {
            // A few points on the surface, about one place.
            let ((u0, _), (v0, v1)) = s.domain();
            let vm = if v0.is_finite() && v1.is_finite() {
                0.5 * (v0 + v1)
            } else {
                1.0
            };
            let pts: Vec<Vec3> = (0..5)
                .map(|i| s.eval(u0 + 0.3 + 0.1 * i as f64, vm + 0.05 * i as f64))
                .collect();
            let n = rational_form(s, &pts).unwrap();
            let form = Surface::Nurbs(n.clone());
            let ((a0, a1), (b0, b1)) = n.domain();
            for i in 0..=20 {
                for j in 0..=20 {
                    let p = n.eval(
                        a0 + (a1 - a0) * i as f64 / 20.0,
                        b0 + (b1 - b0) * j as f64 / 20.0,
                    );
                    let (u, v) = s.project(p).unwrap();
                    assert!(
                        s.eval(u, v).distance(p) < 1e-12 * 10.0,
                        "{} off at {i},{j}: {}",
                        s.kind(),
                        s.eval(u, v).distance(p)
                    );
                }
            }
            for p in &pts {
                let (u, v) = form.project(*p).unwrap();
                assert!(
                    n.eval(u, v).distance(*p) < 1e-9,
                    "{} misses a point",
                    s.kind()
                );
            }
        }
    }
}
