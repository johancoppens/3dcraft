//! Faces on sheets drawn wildly, as a review of the graded grid attacked
//! it (`tess/src/graded.rs`; `docs/design/tessellator.md` › *A grid
//! graded to the shape*): a block with tools through and onto its sides,
//! all analytic, each side then put on one rational sheet with weights of
//! no form, a heavy corner or a Möbius map in every span, of degree 1 to
//! 5, in one to three spans, larger than its face or not; and blocks
//! whose sides carry weights from the ends of what a number holds, or
//! ninety decades apart.
//!
//! The first graded grid measured a triangle of such a face, where the
//! few steps from its mean parameters left it off the surface, by a
//! search of the whole surface for the nearest point, without a count: a
//! block on sheets with weights from 5e-324 to 1e300 took half a minute
//! to most of an hour to refuse. A graded face's measure is now of its
//! grid first, and counted, and weights the numbers do not hold get no
//! graded grid. Each body here ends within a count of work
//! (`geom::allowance` units, the same on every machine: no clock), meshed
//! right by its closed form or refused, never wrong.
//!
//! The shapes, the sheets and the order of their draws are the review's,
//! taken over so that its cases are these cases.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

mod boolean_common;

use boolean_common::Rng;
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_kernel::boolean::BooleanOp::{self, Intersect, Subtract, Union};
use craft3d_kernel::boolean::boolean;
use craft3d_kernel::primitives;
use craft3d_math::Portable;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_tess::{Mesh, TessError, TessParams, graded_grids, tessellate_body_with_threads};
use craft3d_topo::{Body, check, mass_properties};

// ---------------------------------------------------------------- shapes

#[derive(Clone, Copy, Debug)]
enum Prim {
    Box {
        lo: Vec3,
        hi: Vec3,
    },
    /// A capped cylinder from `base` along the unit `axis`.
    Rod {
        base: Vec3,
        axis: Vec3,
        r: f64,
        h: f64,
    },
    Ball {
        c: Vec3,
        r: f64,
    },
    /// Upright, radius `r` at `base`, apex `h` above.
    Cone {
        base: Vec3,
        r: f64,
        h: f64,
    },
    /// About the vertical through `c`.
    Torus {
        c: Vec3,
        big: f64,
        small: f64,
    },
}

fn box_sdf(p: Vec3, lo: Vec3, hi: Vec3) -> f64 {
    let (c, h) = ((lo + hi) * 0.5, (hi - lo) * 0.5);
    let q = Vec3::new(
        (p.x - c.x).abs() - h.x,
        (p.y - c.y).abs() - h.y,
        (p.z - c.z).abs() - h.z,
    );
    let outside = Vec3::new(q.x.max(0.0), q.y.max(0.0), q.z.max(0.0)).length();
    outside + q.x.max(q.y).max(q.z).min(0.0)
}

impl Prim {
    fn body(&self) -> Body {
        match *self {
            Prim::Box { lo, hi } => primitives::block(
                &Frame::WORLD.at(lo).unwrap(),
                hi.x - lo.x,
                hi.y - lo.y,
                hi.z - lo.z,
            )
            .unwrap(),
            Prim::Rod { base, axis, r, h } => {
                let hint = if axis.x.abs() > 0.9 { Vec3::Y } else { Vec3::X };
                primitives::cylinder(&Frame::new(base, axis, hint).unwrap(), r, h).unwrap()
            }
            Prim::Ball { c, r } => primitives::sphere(&Frame::WORLD.at(c).unwrap(), r).unwrap(),
            Prim::Cone { base, r, h } => {
                primitives::cone(&Frame::WORLD.at(base).unwrap(), r, h).unwrap()
            }
            Prim::Torus { c, big, small } => {
                primitives::torus(&Frame::WORLD.at(c).unwrap(), big, small).unwrap()
            }
        }
    }

    /// Signed distance, exact for each primitive (negative inside).
    fn sdf(&self, p: Vec3) -> f64 {
        match *self {
            Prim::Box { lo, hi } => box_sdf(p, lo, hi),
            Prim::Rod { base, axis, r, h } => {
                let q = p - base;
                let t = q.dot(axis);
                let radial = (q - axis * t).length();
                let (d0, d1) = (radial - r, (t - 0.5 * h).abs() - 0.5 * h);
                d0.max(d1).min(0.0) + d0.max(0.0).phypot(d1.max(0.0))
            }
            Prim::Ball { c, r } => (p - c).length() - r,
            Prim::Cone { base, r, h } => {
                // A capped cone, bottom radius r, top radius 0, about its
                // middle (Quilez's exact form).
                let hh = 0.5 * h;
                let q = ((p.x - base.x).phypot(p.y - base.y), p.z - (base.z + hh));
                let (r1, r2) = (r, 0.0);
                let k1 = (r2, hh);
                let k2 = (r2 - r1, 2.0 * hh);
                let ca = (
                    q.0 - q.0.min(if q.1 < 0.0 { r1 } else { r2 }),
                    q.1.abs() - hh,
                );
                let dot = |a: (f64, f64), b: (f64, f64)| a.0 * b.0 + a.1 * b.1;
                let s = (dot((k1.0 - q.0, k1.1 - q.1), k2) / dot(k2, k2)).clamp(0.0, 1.0);
                let cb = (q.0 - k1.0 + k2.0 * s, q.1 - k1.1 + k2.1 * s);
                let sign = if cb.0 < 0.0 && ca.1 < 0.0 { -1.0 } else { 1.0 };
                sign * dot(ca, ca).min(dot(cb, cb)).sqrt()
            }
            Prim::Torus { c, big, small } => {
                let q = p - c;
                (q.x.phypot(q.y) - big).phypot(q.z) - small
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Csg {
    base: Prim,
    ops: Vec<(BooleanOp, Prim)>,
}

impl Csg {
    fn sdf(&self, p: Vec3) -> f64 {
        let mut d = self.base.sdf(p);
        for (op, prim) in &self.ops {
            let s = prim.sdf(p);
            d = match op {
                Union => d.min(s),
                Intersect => d.max(s),
                Subtract => d.max(-s),
            };
        }
        d
    }

    /// The analytic body, by the kernel's booleans on analytic surfaces;
    /// `None` where one of them is refused (not this review's matter).
    fn body(&self, tol: &Tolerance) -> Option<Body> {
        let mut acc = self.base.body();
        for (op, prim) in &self.ops {
            acc = boolean(&acc, &prim.body(), *op, tol).ok()?.body;
        }
        Some(acc)
    }
}

// ---------------------------------------------------------------- sheets

/// How the weights of a sheet are drawn.
#[derive(Clone, Copy, Debug)]
enum Weights {
    /// `10^(decades (x − 1/2))` per control point, `x` uniform: no form.
    Random(f64),
    /// `ratio^(i j / ((nu − 1)(nv − 1)))`: one heavy corner.
    Corner(f64),
    /// A Möbius map in every span, the end weights `ratio` apart, the
    /// other way round in every other span, along `u`, `v` or both.
    Spans(f64, bool, bool),
    /// Weights from the ends of what a weight may be.
    Hostile,
    /// Weights of no form, as far apart as a graded grid takes them.
    Far,
}

fn clamped_knots(
    degree: usize,
    spans: usize,
    full: bool,
    rng: &mut Rng,
    uniform: bool,
) -> Vec<f64> {
    let mut knots = vec![0.0; degree + 1];
    let mut inner: Vec<f64> = (1..spans)
        .map(|k| {
            if uniform {
                k as f64 / spans as f64
            } else {
                (k as f64 + rng.between(-0.35, 0.35)) / spans as f64
            }
        })
        .collect();
    inner.sort_by(f64::total_cmp);
    for k in inner {
        for _ in 0..(if full { degree } else { 1 }) {
            knots.push(k);
        }
    }
    knots.extend(vec![1.0; degree + 1]);
    knots
}

/// The Greville abscissae of a knot vector, in `[0, 1]`.
fn greville(knots: &[f64], degree: usize) -> Vec<f64> {
    let n = knots.len() - degree - 1;
    (0..n)
        .map(|i| {
            if degree == 0 {
                knots[i]
            } else {
                knots[i + 1..=i + degree].iter().sum::<f64>() / degree as f64
            }
        })
        .collect()
}

/// The log of the Möbius factor a span gives every step of control points
/// along one parameter, cumulated: the weight factor of index `i`.
fn span_logs(n: usize, degree: usize, ratio: f64, alternate: bool) -> Vec<f64> {
    let mut out = vec![0.0; n];
    let step = ratio.pln() / degree.max(1) as f64;
    for i in 1..n {
        let span = (i - 1) / degree.max(1);
        let sign = if alternate && span % 2 == 1 {
            -1.0
        } else {
            1.0
        };
        out[i] = out[i - 1] + sign * step;
    }
    out
}

fn weights_for(
    mode: Weights,
    (nu, nv): (usize, usize),
    (du, dv): (usize, usize),
    rng: &mut Rng,
) -> Vec<f64> {
    let mut logs = vec![0.0_f64; nu * nv];
    match mode {
        Weights::Random(decades) => {
            for l in &mut logs {
                *l = decades * (rng.unit() - 0.5) * std::f64::consts::LN_10;
            }
        }
        Weights::Corner(ratio) => {
            let d = ((nu - 1) * (nv - 1)).max(1) as f64;
            for i in 0..nu {
                for j in 0..nv {
                    logs[i * nv + j] = ratio.pln() * (i * j) as f64 / d;
                }
            }
        }
        Weights::Spans(ratio, along_u, along_v) => {
            let lu = span_logs(nu, du, if along_u { ratio } else { 1.0 }, true);
            let lv = span_logs(nv, dv, if along_v { ratio } else { 1.0 }, true);
            for i in 0..nu {
                for j in 0..nv {
                    logs[i * nv + j] = lu[i] + lv[j];
                }
            }
        }
        Weights::Hostile => {
            let choices = [5e-324, 1e-310, 1e-300, 1e-150, 1.0, 1e150, 1e300];
            return (0..nu * nv)
                .map(|_| choices[rng.below(choices.len() as u64) as usize])
                .collect();
        }
        Weights::Far => {
            let choices = [1e-45, 1e-30, 1e-15, 1.0, 1e15, 1e30, 1e45];
            return (0..nu * nv)
                .map(|_| choices[rng.below(choices.len() as u64) as usize])
                .collect();
        }
    }
    let mean = logs.iter().sum::<f64>() / logs.len() as f64;
    logs.iter().map(|l| (l - mean).pexp()).collect()
}

#[derive(Clone, Copy, Debug)]
struct SheetSpec {
    degree: (usize, usize),
    spans: (usize, usize),
    full: bool,
    uniform: bool,
    weights: Weights,
}

/// A sheet in the plane z = 0 of `frame` over `[0, a] × [0, b]`.
fn plane_sheet(frame: &Frame, (a, b): (f64, f64), spec: &SheetSpec, rng: &mut Rng) -> NurbsSurface {
    let (du, dv) = spec.degree;
    let full_u = spec.full || du == 1;
    let full_v = spec.full || dv == 1;
    let ku = clamped_knots(du, spec.spans.0, full_u, rng, spec.uniform);
    let kv = clamped_knots(dv, spec.spans.1, full_v, rng, spec.uniform);
    let (gu, gv) = (greville(&ku, du), greville(&kv, dv));
    let (nu, nv) = (gu.len(), gv.len());
    let mut ctrl = Vec::with_capacity(nu * nv);
    for &x in &gu {
        for &y in &gv {
            ctrl.push(frame.point(a * x, b * y, 0.0));
        }
    }
    let weights = weights_for(spec.weights, (nu, nv), (du, dv), rng);
    NurbsSurface::new(du, dv, ku, kv, nu, nv, ctrl, weights).unwrap()
}

/// Whether the sheet's map onto its plane keeps its orientation
/// everywhere sampled (else it folds: no valid face).
fn regular(sheet: &NurbsSurface, frame: &Frame) -> bool {
    let ((u0, u1), (v0, v1)) = sheet.domain();
    let mut knots_u: Vec<f64> = sheet.knots_u().to_vec();
    knots_u.dedup();
    let mut knots_v: Vec<f64> = sheet.knots_v().to_vec();
    knots_v.dedup();
    let samples = [
        1e-9,
        1e-6,
        1e-3,
        0.02,
        0.25,
        0.5,
        0.75,
        0.98,
        0.999,
        1.0 - 1e-6,
        1.0 - 1e-9,
    ];
    let local = |u: f64, v: f64| {
        let (x, y, _) = frame.local(sheet.eval(u, v));
        (x, y)
    };
    let mut sign = 0.0_f64;
    for wu in knots_u.windows(2) {
        for wv in knots_v.windows(2) {
            let (wa, wb) = (wu[1] - wu[0], wv[1] - wv[0]);
            if wa <= 0.0 || wb <= 0.0 {
                continue;
            }
            for &s in &samples {
                for &t in &samples {
                    let (u, v) = (wu[0] + wa * s, wv[0] + wb * t);
                    let (hu, hv) = (1e-10 * wa, 1e-10 * wb);
                    if u - hu < u0 || u + hu > u1 || v - hv < v0 || v + hv > v1 {
                        continue;
                    }
                    let (a, b) = (local(u + hu, v), local(u - hu, v));
                    let (c, d) = (local(u, v + hv), local(u, v - hv));
                    let (xu, yu) = (a.0 - b.0, a.1 - b.1);
                    let (xv, yv) = (c.0 - d.0, c.1 - d.1);
                    let det = xu * yv - xv * yu;
                    if !det.is_finite() {
                        return false;
                    }
                    if det == 0.0 {
                        continue;
                    }
                    if sign == 0.0 {
                        sign = det.signum();
                    } else if det.signum() != sign {
                        return false;
                    }
                }
            }
        }
    }
    sign > 0.0
}

/// The body with every planar face that lies on a side of the box
/// `lo..hi` put on one sheet per side (drawn by `spec`, the side's
/// rectangle grown by `pad` of its size on every side), the face keeping
/// its outside. `None` when a sheet folds.
fn sheeted(
    body: &Body,
    lo: Vec3,
    hi: Vec3,
    pad: f64,
    spec: &SheetSpec,
    rng: &mut Rng,
) -> Option<(Body, usize)> {
    let mut twin = body.clone();
    let size = (hi - lo).length();
    let mut sides: Vec<(usize, f64, Surface)> = Vec::new();
    let mut replaced = 0;
    let corners: Vec<Vec3> = (0..8)
        .map(|k| {
            Vec3::new(
                if k & 1 == 0 { lo.x } else { hi.x },
                if k & 2 == 0 { lo.y } else { hi.y },
                if k & 4 == 0 { lo.z } else { hi.z },
            )
        })
        .collect();
    for index in 0..twin.faces.len() {
        let Surface::Plane(plane) = &twin.faces[index].surface else {
            continue;
        };
        let f = *plane.frame();
        let n = f.z();
        let Some(axis) = [n.x.abs(), n.y.abs(), n.z.abs()]
            .iter()
            .position(|&c| c > 1.0 - 1e-12)
        else {
            continue;
        };
        let at = [f.origin().x, f.origin().y, f.origin().z][axis];
        let (l, h) = ([lo.x, lo.y, lo.z][axis], [hi.x, hi.y, hi.z][axis]);
        let side = if (at - l).abs() <= 1e-9 * size {
            l
        } else if (at - h).abs() <= 1e-9 * size {
            h
        } else {
            continue;
        };
        let surface = if let Some((_, _, s)) =
            sides.iter().find(|(a, s, _)| *a == axis && *s == side)
        {
            s.clone()
        } else {
            let on: Vec<(f64, f64)> = corners
                .iter()
                .filter(|c| ([c.x, c.y, c.z][axis] - side).abs() <= 1e-12 * size)
                .map(|&c| {
                    let (x, y, _) = f.local(c);
                    (x, y)
                })
                .collect();
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for (x, y) in on {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
            let (w, d) = (x1 - x0, y1 - y0);
            let (x0, y0) = (x0 - pad * w, y0 - pad * d);
            let frame = Frame::new(f.point(x0, y0, 0.0), f.z(), f.x()).unwrap();
            let sheet = plane_sheet(
                &frame,
                (w * (1.0 + 2.0 * pad), d * (1.0 + 2.0 * pad)),
                spec,
                rng,
            );
            if !matches!(spec.weights, Weights::Hostile | Weights::Far) && !regular(&sheet, &frame)
            {
                return None;
            }
            let s = Surface::Nurbs(sheet);
            sides.push((axis, side, s.clone()));
            s
        };
        // Keep the face's outside: the sheet's normal against the plane's.
        let ((u0, u1), (v0, v1)) = surface.domain();
        let normal = surface.normal(0.5 * (u0 + u1), 0.5 * (v0 + v1));
        let same = normal.is_none_or(|nn| nn.dot(n) > 0.0);
        let was = twin.faces[index].reversed;
        twin.faces[index].surface = surface;
        twin.faces[index].reversed = if same { was } else { !was };
        replaced += 1;
    }
    Some((twin, replaced))
}

// ---------------------------------------------------------------- the judge

/// What a body's mesh is, by its closed form.
#[derive(Debug)]
enum Verdict {
    /// Its triangles.
    Right(usize),
    /// More triangles than this file makes.
    Over,
    Refused(String),
    Wrong(String),
}

fn same(a: &Mesh, b: &Mesh) -> bool {
    let bits = |p: &Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    a.triangles == b.triangles
        && a.face_ids == b.face_ids
        && a.positions.len() == b.positions.len()
        && a.positions
            .iter()
            .zip(&b.positions)
            .all(|(p, q)| bits(p) == bits(q))
}

/// The mesh of `body` on one thread and the work it took, in
/// `geom::allowance` units: a count, the same on every machine.
fn meshed(body: &Body, params: &TessParams) -> (Result<Mesh, TessError>, u64) {
    craft3d_geom::allowance::measured(|| tessellate_body_with_threads(body, params, 1))
}

/// `mesh`, the mesh of `body` on one thread, judged by the closed form:
/// the same on two threads and on all; closed; 28 points of every
/// triangle (its corners, the thirds and sixths of its sides, and inside)
/// on the boundary of the solid, by the signed distance `sdf` of its
/// shapes taken at the point brought `back` to where they stand: the
/// seven points the tessellator measures (corners, side middles, middle)
/// within the tolerance, the others within one and a half (a triangle
/// across a hole is off by the hole); and the volume, summed about a
/// point of the mesh, within the tolerance times the area.
fn judge(
    body: &Body,
    mesh: &Mesh,
    sdf: &dyn Fn(Vec3) -> f64,
    back: &Transform,
    params: &TessParams,
    linear: f64,
    (volume, area): (f64, f64),
) -> Verdict {
    for threads in [2, 0] {
        match tessellate_body_with_threads(body, params, threads) {
            Ok(again) if same(mesh, &again) => {}
            _ => return Verdict::Wrong(format!("differs on {threads} threads")),
        }
    }
    let Ok(welded) = mesh.weld() else {
        return Verdict::Wrong("a triangle with a corner that does not exist".into());
    };
    if !welded.is_closed() {
        return Verdict::Wrong("the mesh is not watertight".into());
    }
    let tolerance = params.chordal_tolerance;
    let about = mesh.positions.first().copied().unwrap_or(Vec3::ZERO);
    let (mut worst, mut worst_measured, mut six_volumes) = (0.0_f64, 0.0_f64, 0.0_f64);
    for corners in &mesh.triangles {
        let [a, b, c] = corners.map(|i| mesh.positions[i as usize]);
        six_volumes += (a - about).dot((b - about).cross(c - about));
        for i in 0..=6_u32 {
            for j in 0..=(6 - i) {
                let k = 6 - i - j;
                let point = (a * f64::from(i) + b * f64::from(j) + c * f64::from(k)) * (1.0 / 6.0);
                let off = sdf(back.point(point)).abs();
                let off = if off.is_nan() { f64::INFINITY } else { off };
                worst = worst.max(off);
                let measured = [i, j, k].iter().all(|n| n % 3 == 0) || (i, j, k) == (2, 2, 2);
                if measured {
                    worst_measured = worst_measured.max(off);
                }
            }
        }
    }
    if worst_measured > tolerance + linear {
        return Verdict::Wrong(format!(
            "a measured point {:.3} tolerances from the solid's boundary",
            worst_measured / tolerance
        ));
    }
    if worst > 1.5 * tolerance + linear {
        return Verdict::Wrong(format!(
            "a point {:.3} tolerances from the solid's boundary",
            worst / tolerance
        ));
    }
    let off = (six_volumes / 6.0 - volume).abs();
    if off.is_nan() || off > area * tolerance {
        return Verdict::Wrong(format!("mesh volume {} for {volume}", six_volumes / 6.0));
    }
    Verdict::Right(mesh.triangles.len())
}

fn tilt(k: f64) -> Transform {
    Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
        .unwrap()
        .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
}

/// One test of this file at a time: each reads the count of graded grids
/// made (`tess::graded_grids`, one count for the process) before and
/// after its bodies.
fn alone() -> std::sync::MutexGuard<'static, ()> {
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ONE.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn pick<T: Copy>(rng: &mut Rng, items: &[T]) -> T {
    items[rng.below(items.len() as u64) as usize]
}

/// The most work the mesh of one body of this file may take, in
/// `geom::allowance` units: ten times the most one takes (measured
/// 2026-11-16: 3 433 401 for a plane case; 20 576 083 for a block on
/// sheets with weights the numbers do not hold, which is what refusing
/// it took before there was a graded grid; 155 459 092 for one on sheets
/// ninety decades apart, refused after 88 graded grids: the rounds of
/// finer chords make a face's grid again each time).
const MOST_WORK: u64 = 1_600_000_000;

/// How many plane cases are drawn: all of the review's.
const CASES: u64 = 220;

/// The most triangles a body of the plane cases is made with: a side
/// that keeps its even grid on a sheet larger than itself has hundreds
/// of thousands, which are counted, not made (and not this file's
/// matter: `docs/design/tessellator.md`, owed).
const MOST_TRIANGLES: usize = 20_000;

// ---------------------------------------------------------------- planes

/// A block with a tool through or onto its sides, each side then on a
/// sheet drawn wildly, as the review drew them. Each ends within
/// [`MOST_WORK`]; each is right by the closed form, refused, or over
/// [`MOST_TRIANGLES`]; none is wrong. On 2026-11-16, of 220: 84 right
/// (73 of them with a face on a graded grid), 71 over the triangles, 10
/// refused as their even grids were, 55 not bodies (a tool the kernel
/// refuses, a sheet that folds, a twin the check refuses). `WILD_ONLY`,
/// by hand: the one case to mesh.
#[test]
fn planes_with_holes_and_neighbours_on_wild_sheets_end_and_are_right() {
    let _alone = alone();
    let mut rng = Rng(0x5eed_2026_1116);
    let (mut right, mut graded, mut skipped, mut over, mut most) = (0, 0, 0, 0, 0_u64);
    let mut triangles = 0;
    let (mut refused, mut wrong): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    for case in 0..CASES {
        let k = pick(&mut rng, &[1e-3, 1.0, 1e6]);
        let size = pick(
            &mut rng,
            &[Vec3::new(4.0, 3.0, 2.0), Vec3::new(30.0, 900.0, 20.0)],
        ) * k;
        let (lo, hi) = (Vec3::ZERO, size);
        let m = size.x.min(size.y);
        let at = |rng: &mut Rng, a: f64, b: f64| -> (f64, f64) {
            (size.x * rng.between(a, b), size.y * rng.between(a, b))
        };
        let shape = rng.below(10);
        let rod_z = |x: f64, y: f64, r: f64| Prim::Rod {
            base: Vec3::new(x, y, -0.5 * size.z),
            axis: Vec3::Z,
            r,
            h: 2.0 * size.z,
        };
        let mut ops: Vec<(BooleanOp, Prim)> = Vec::new();
        let shape_name = match shape {
            0 => {
                let (x, y) = at(&mut rng, 0.2, 0.8);
                ops.push((Subtract, rod_z(x, y, m * rng.between(0.05, 0.15))));
                "hole"
            }
            1 => {
                // A thin wall between a hole and the side.
                let r = m * rng.between(0.05, 0.12);
                let wall = m * pick(&mut rng, &[0.002, 0.01, 0.03]);
                let y = size.y * rng.between(0.3, 0.7);
                ops.push((Subtract, rod_z(r + wall, y, r)));
                "hole by a thin wall"
            }
            2 => {
                // Near a corner, where a heavy corner races.
                let r = m * 0.04;
                let corner = pick(&mut rng, &[(0.08, 0.08), (0.92, 0.92), (0.08, 0.92)]);
                ops.push((Subtract, rod_z(size.x * corner.0, size.y * corner.1, r)));
                "hole near a corner"
            }
            3 => {
                let (x, y) = at(&mut rng, 0.1, 0.6);
                let (w, d) = (
                    size.x * rng.between(0.1, 0.3),
                    size.y * rng.between(0.1, 0.3),
                );
                ops.push((
                    Subtract,
                    Prim::Box {
                        lo: Vec3::new(x, y, 0.5 * size.z),
                        hi: Vec3::new(x + w, y + d, 1.5 * size.z),
                    },
                ));
                "pocket"
            }
            4 => {
                let (x, y) = at(&mut rng, 0.3, 0.7);
                ops.push((
                    Union,
                    Prim::Ball {
                        c: Vec3::new(x, y, size.z),
                        r: m * rng.between(0.08, 0.25),
                    },
                ));
                "dome"
            }
            5 => {
                let (x, y) = at(&mut rng, 0.3, 0.7);
                ops.push((
                    Union,
                    Prim::Cone {
                        base: Vec3::new(x, y, size.z - 0.1 * size.z),
                        r: m * rng.between(0.08, 0.2),
                        h: size.z * 0.6,
                    },
                ));
                "cone on top"
            }
            6 => {
                ops.push((
                    Subtract,
                    Prim::Torus {
                        c: Vec3::new(0.5 * size.x, 0.5 * size.y, size.z),
                        big: 0.25 * m,
                        small: 0.05 * m,
                    },
                ));
                "groove"
            }
            7 => {
                let (y, z) = (
                    size.y * rng.between(0.2, 0.8),
                    size.z * rng.between(0.3, 0.7),
                );
                ops.push((
                    Subtract,
                    Prim::Rod {
                        base: Vec3::new(-0.5 * size.x, y, z),
                        axis: Vec3::X,
                        r: size.z.min(size.y) * 0.1,
                        h: 2.0 * size.x,
                    },
                ));
                "side hole"
            }
            8 => {
                for _ in 0..3 {
                    let (x, y) = at(&mut rng, 0.15, 0.85);
                    ops.push((Subtract, rod_z(x, y, m * 0.04)));
                }
                "three holes"
            }
            _ => "plain",
        };
        let csg = Csg {
            base: Prim::Box { lo, hi },
            ops,
        };
        let tol = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let degree = (1 + rng.below(5) as usize, 1 + rng.below(5) as usize);
        let spans = (1 + rng.below(3) as usize, 1 + rng.below(3) as usize);
        let weights = match rng.below(6) {
            0 => Weights::Random(4.0),
            1 => Weights::Random(9.0),
            2 => Weights::Corner(pick(&mut rng, &[1e4, 1e6, 1e9])),
            3 => Weights::Spans(pick(&mut rng, &[1e4, 1e9]), true, false),
            4 => Weights::Spans(pick(&mut rng, &[1e4, 1e9]), false, true),
            _ => Weights::Spans(pick(&mut rng, &[1e4, 1e9]), true, true),
        };
        let spec = SheetSpec {
            degree,
            spans,
            full: rng.below(2) == 0,
            uniform: rng.below(3) != 0,
            weights,
        };
        // A pad of 1 with three uniform spans: the face is the middle span,
        // its borders on knot lines.
        let pad = pick(&mut rng, &[0.0, 0.0, 0.37, 1.0, 3.0]);
        let place = rng.below(3);
        let rel = pick(&mut rng, &[1e-2, 1e-3]);
        let what = format!(
            "case {case}: {shape_name}, k {k}, size {:?}, {spec:?}, pad {pad}, place {place}, chord {rel}",
            size * (1.0 / k)
        );
        let Some(plain) = csg.body(&tol) else {
            skipped += 1;
            continue;
        };
        let Some((twin, _)) = sheeted(&plain, lo, hi, pad, &spec, &mut rng) else {
            skipped += 1;
            continue;
        };
        let transform = match place {
            0 => Transform::translation(Vec3::ZERO),
            1 => tilt(k),
            _ => tilt(k).then(&Transform::translation(
                Vec3::new(1.0, 2.0, 3.0) * (1e4 * size.length()),
            )),
        };
        let back = transform.inverse().unwrap();
        let twin = twin.transformed(&transform).unwrap();
        let plain = plain.transformed(&transform).unwrap();
        // (A twin the check refuses is no body: the nearest point on such
        // a sheet, not the mesh.)
        if !check(&twin, &tol).is_empty() {
            skipped += 1;
            continue;
        }
        // (`WILD_ONLY`: the one case to mesh, the others drawn alone.)
        if std::env::var("WILD_ONLY").is_ok_and(|only| only != case.to_string()) {
            continue;
        }
        let mass = mass_properties(&plain).unwrap();
        let params = TessParams {
            chordal_tolerance: rel * size.length(),
            max_triangles: Some(MOST_TRIANGLES),
            ..TessParams::default()
        };
        let before = graded_grids();
        let started = std::time::Instant::now();
        let (made, units) = meshed(&twin, &params);
        let took = started.elapsed();
        most = most.max(units);
        let was_graded = graded_grids() > before;
        graded += usize::from(was_graded);
        let verdict = match &made {
            Ok(mesh) => judge(
                &twin,
                mesh,
                &|p: Vec3| csg.sdf(p),
                &back,
                &params,
                tol.linear,
                (mass.volume, mass.area),
            ),
            Err(TessError::OverBudget { .. }) => Verdict::Over,
            Err(why) => Verdict::Refused(why.to_string()),
        };
        println!("planes {what}: {verdict:?}, graded {was_graded}, {units} units ({took:.2?})");
        assert!(units <= MOST_WORK, "{what}: {units} units of work");
        match verdict {
            Verdict::Right(count) => {
                right += 1;
                triangles += count;
            }
            Verdict::Over => over += 1,
            Verdict::Refused(why) => refused.push(format!("{what}: {why}")),
            Verdict::Wrong(why) => wrong.push(format!("{what}: {why}")),
        }
    }
    println!(
        "planes: {right} right ({triangles} triangles), {over} over {MOST_TRIANGLES} triangles, {} refused, {} wrong ({graded} bodies with a graded face), {skipped} skipped (a tool the kernel refuses, a sheet that folds, a twin the check refuses); the most work {most} units",
        refused.len(),
        wrong.len()
    );
    for line in &refused {
        println!("planes, refused: {line}");
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    // (What is right today stays right: a floor, with room for bodies the
    // kernel's booleans come to make otherwise.)
    assert!(
        std::env::var_os("WILD_ONLY").is_some() || right >= 80,
        "{right} right"
    );
}

// ---------------------------------------------------------------- hostile weights

/// Blocks whose sides are sheets of degree 1 to 5 in one to three spans
/// with every weight drawn from 5e-324, 1e-310, 1e-300, 1e-150, 1, 1e150
/// and 1e300, which a surface may carry and a file may hold: 24 of them,
/// as the review drew them (the first graded grid measured their faces
/// without a bound: 33 s to more than 53 minutes each). No graded grid is
/// made on such weights, and each ends within [`MOST_WORK`], without a
/// panic, meshed (then closed, with every point on the block) or refused
/// as it was before.
#[test]
fn sheets_with_weights_from_the_ends_of_what_a_number_holds_end() {
    let _alone = alone();
    let began = graded_grids();
    let (made, refused) = blocks_on_sheets(0xbad_5eed, Weights::Hostile, "hostile");
    assert_eq!(made + refused, 24);
    // (No graded grid on weights the numbers do not hold: each face is
    // refused as it was before there was one, at the cost it had.)
    assert_eq!(graded_grids(), began, "a graded grid on weights not held");
}

/// The same blocks with every weight drawn from 1e-45, 1e-30, 1e-15, 1,
/// 1e15, 1e30 and 1e45: of no form, and as far apart as a graded grid
/// takes them. Each ends within [`MOST_WORK`], without a panic, meshed
/// (then the block's) or refused.
#[test]
fn sheets_with_weights_ninety_decades_apart_end() {
    let _alone = alone();
    let began = graded_grids();
    let (made, refused) = blocks_on_sheets(0xfa5_5eed, Weights::Far, "far");
    assert_eq!(made + refused, 24);
    assert!(graded_grids() > began, "no graded grid among them");
}

/// 24 blocks 4 by 3 by 2 with each side on a sheet of weights drawn as
/// `weights` says (the sheets are not asked to keep their orientation:
/// not every one is a valid face): how many are meshed and how many
/// refused, each within [`MOST_WORK`] and without a panic, a mesh closed
/// and with every point on the block.
fn blocks_on_sheets(seed: u64, weights: Weights, name: &str) -> (usize, usize) {
    let mut rng = Rng(seed);
    let size = Vec3::new(4.0, 3.0, 2.0);
    let (mut made, mut refused, mut most) = (0, 0, 0_u64);
    let mut reasons: Vec<String> = Vec::new();
    for case in 0..24 {
        let spec = SheetSpec {
            degree: (1 + rng.below(5) as usize, 1 + rng.below(5) as usize),
            spans: (1 + rng.below(3) as usize, 1 + rng.below(3) as usize),
            full: rng.below(2) == 0,
            uniform: true,
            weights,
        };
        let plain = primitives::block(&Frame::WORLD, size.x, size.y, size.z).unwrap();
        let (twin, _) = sheeted(&plain, Vec3::ZERO, size, 0.0, &spec, &mut rng).unwrap();
        let params = TessParams {
            chordal_tolerance: 1e-3 * size.length(),
            ..TessParams::default()
        };
        let before = graded_grids();
        let started = std::time::Instant::now();
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| meshed(&twin, &params)));
        let Ok((result, units)) = outcome else {
            panic!("{name} case {case} {spec:?}: a panic");
        };
        most = most.max(units);
        let line = match &result {
            Err(why) => {
                refused += 1;
                reasons.push(why.to_string());
                format!("refused: {why}")
            }
            Ok(mesh) => {
                made += 1;
                let worst = mesh
                    .positions
                    .iter()
                    .map(|&p| box_sdf(p, Vec3::ZERO, size).abs())
                    .fold(0.0_f64, f64::max);
                assert!(
                    mesh.weld().is_ok_and(|w| w.is_closed()) && worst <= 1e-9 * size.length(),
                    "{name} case {case}: a mesh that is not the block's: {worst:e} off"
                );
                format!("{} triangles", mesh.triangles.len())
            }
        };
        println!(
            "{name} case {case}: {line}; graded grids {}, {units} units ({:.2?})",
            graded_grids() - before,
            started.elapsed()
        );
        assert!(
            units <= MOST_WORK,
            "{name} case {case}: {units} units of work"
        );
    }
    reasons.sort();
    reasons.dedup();
    println!(
        "{name}: {made} meshed, {refused} refused; the most work {most} units; the reasons: {reasons:#?}"
    );
    (made, refused)
}
