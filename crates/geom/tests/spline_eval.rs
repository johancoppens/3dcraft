//! Evaluation of NURBS curves, NURBS surfaces and Bézier patches against the
//! algorithm they had before evaluation was made local.
//!
//! The reference in `old` is that algorithm, kept here unchanged: it builds
//! the homogeneous net on every call, reduces every row of a surface and
//! allocates as it goes. The library must give the same numbers **bit for
//! bit** (it does the same arithmetic in the same order on the control
//! points of the knot span only), on seeded random curves and surfaces:
//! rational and not, degrees 1 to 32, clamped and unclamped knot vectors
//! with interior knots of every multiplicity, at the ends and corners of the
//! domain, on every knot, just beside every knot, outside the domain and at
//! NaN, at scales 1e-3, 1 and 1e6.
//!
//! The reference reads a surface through its public accessors only, so a
//! surface whose cached net does not belong to its control points (after a
//! transform, a clone) is caught as well.
//!
//! `timings` (ignored) prints the cost per call:
//! `cargo test --release -p craft3d-geom --test spline_eval -- --ignored --nocapture`.

// Helpers outside `#[test]` functions unwrap and index, as tests may.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use craft3d_geom::{BezierPatch, Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Transform, Vec3};

/// The evaluation as it was before the change. Do not "improve" this: it is
/// the yardstick.
mod old {
    use craft3d_geom::{BezierPatch, NurbsCurve, NurbsSurface};
    use craft3d_math::Vec3;

    #[derive(Clone, Copy)]
    struct H {
        p: Vec3,
        w: f64,
    }

    impl H {
        fn lerp(self, o: H, t: f64) -> H {
            H {
                p: self.p.lerp(o.p, t),
                w: self.w * (1.0 - t) + o.w * t,
            }
        }
    }

    fn domain_of(knots: &[f64], degree: usize) -> (f64, f64) {
        let count = knots.len().saturating_sub(degree + 1);
        match (knots.get(degree), knots.get(count)) {
            (Some(&a), Some(&b)) => (a, b),
            _ => (0.0, 1.0),
        }
    }

    fn de_boor(knots: &[f64], degree: usize, ctrl: &[H], t: f64) -> H {
        let zero = H {
            p: Vec3::ZERO,
            w: 0.0,
        };
        let knot = |i: usize| knots.get(i).copied().unwrap_or(0.0);
        let (start, end) = domain_of(knots, degree);
        // The end span continues beyond the domain (as the crate does since
        // 2026-10-13; before, the parameter was clamped).
        let inside = if t > start { t.min(end) } else { start };
        let t = if t.is_nan() { start } else { t };
        let span = knots
            .partition_point(|&k| k <= inside)
            .saturating_sub(1)
            .clamp(degree, ctrl.len().saturating_sub(1));
        let first = span - degree;
        let mut d: Vec<H> = ctrl.iter().skip(first).take(degree + 1).copied().collect();
        for r in 1..=degree {
            d = d
                .iter()
                .zip(d.iter().skip(1))
                .enumerate()
                .map(|(j, (&a, &b))| {
                    let (lo, hi) = (knot(first + r + j), knot(span + 1 + j));
                    let alpha = if hi > lo { (t - lo) / (hi - lo) } else { 0.0 };
                    a.lerp(b, alpha)
                })
                .collect();
        }
        d.first().copied().unwrap_or(zero)
    }

    fn hodograph(knots: &[f64], degree: usize, ctrl: &[H]) -> (Vec<f64>, Vec<H>) {
        let knot = |i: usize| knots.get(i).copied().unwrap_or(0.0);
        let points = ctrl
            .iter()
            .zip(ctrl.iter().skip(1))
            .enumerate()
            .map(|(i, (a, b))| {
                let span = knot(i + degree + 1) - knot(i + 1);
                let scale = if span > 0.0 {
                    degree as f64 / span
                } else {
                    0.0
                };
                H {
                    p: (b.p - a.p) * scale,
                    w: (b.w - a.w) * scale,
                }
            })
            .collect();
        let inner = knots
            .iter()
            .skip(1)
            .take(knots.len().saturating_sub(2))
            .copied()
            .collect();
        (inner, points)
    }

    fn de_boor_derivative(knots: &[f64], degree: usize, ctrl: &[H], t: f64) -> H {
        let (inner, points) = hodograph(knots, degree, ctrl);
        if degree == 1 {
            let (start, end) = domain_of(knots, degree);
            let inside = if t > start { t.min(end) } else { start };
            let span = knots
                .partition_point(|&k| k <= inside)
                .saturating_sub(1)
                .clamp(1, ctrl.len().saturating_sub(1));
            return points.get(span - 1).copied().unwrap_or(H {
                p: Vec3::ZERO,
                w: 0.0,
            });
        }
        de_boor(&inner, degree - 1, &points, t)
    }

    fn homogeneous(ctrl: &[Vec3], weights: &[f64]) -> Vec<H> {
        ctrl.iter()
            .zip(weights)
            .map(|(&p, &w)| H { p: p * w, w })
            .collect()
    }

    pub fn curve(c: &NurbsCurve, t: f64) -> (Vec3, Vec3) {
        let h = homogeneous(c.control_points(), c.weights());
        let a = de_boor(c.knots(), c.degree(), &h, t);
        let da = de_boor_derivative(c.knots(), c.degree(), &h, t);
        let point = a.p * (1.0 / a.w);
        (point, (da.p - point * da.w) * (1.0 / a.w))
    }

    pub fn surface(s: &NurbsSurface, u: f64, v: f64) -> (Vec3, Vec3, Vec3) {
        let (degree_u, degree_v) = s.degrees();
        let (_, count_v) = s.counts();
        let h = homogeneous(s.control_points(), s.weights());
        let mut row_points = Vec::new();
        let mut row_derivs = Vec::new();
        for row in h.chunks(count_v.max(1)) {
            row_points.push(de_boor(s.knots_v(), degree_v, row, v));
            row_derivs.push(de_boor_derivative(s.knots_v(), degree_v, row, v));
        }
        let a = de_boor(s.knots_u(), degree_u, &row_points, u);
        let a_u = de_boor_derivative(s.knots_u(), degree_u, &row_points, u);
        let a_v = de_boor(s.knots_u(), degree_u, &row_derivs, u);
        let point = a.p * (1.0 / a.w);
        let partial = |d: H| (d.p - point * d.w) * (1.0 / a.w);
        (point, partial(a_u), partial(a_v))
    }

    fn bernstein(n: usize, t: f64) -> Vec<f64> {
        let mut b = Vec::with_capacity(n + 1);
        b.push(1.0);
        for _ in 0..n {
            let mut next = Vec::with_capacity(n + 1);
            let mut prev = 0.0;
            for &c in &b {
                next.push(t * prev + (1.0 - t) * c);
                prev = c;
            }
            next.push(t * prev);
            b = next;
        }
        b
    }

    fn bernstein_derivative(n: usize, t: f64) -> Vec<f64> {
        let Some(lower_degree) = n.checked_sub(1) else {
            return vec![0.0];
        };
        let lower = bernstein(lower_degree, t);
        let scale = n as f64;
        let mut d = Vec::with_capacity(n + 1);
        let mut prev = 0.0;
        for &c in &lower {
            d.push(scale * (prev - c));
            prev = c;
        }
        d.push(scale * prev);
        d
    }

    fn blend(b: &BezierPatch, wu: &[f64], wv: &[f64]) -> Vec3 {
        let mut sum = Vec3::ZERO;
        for (row, &a) in b.control_points().chunks(b.degree_v() + 1).zip(wu) {
            for (&p, &c) in row.iter().zip(wv) {
                sum += p * (a * c);
            }
        }
        sum
    }

    pub fn patch(b: &BezierPatch, u: f64, v: f64) -> (Vec3, Vec3, Vec3) {
        let bu = bernstein(b.degree_u(), u);
        let bv = bernstein(b.degree_v(), v);
        let du = bernstein_derivative(b.degree_u(), u);
        let dv = bernstein_derivative(b.degree_v(), v);
        (blend(b, &bu, &bv), blend(b, &du, &bv), blend(b, &bu, &dv))
    }
}

/// SplitMix64: no dependency, the same numbers everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[a, b)`.
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * ((self.next() >> 11) as f64 / (1u64 << 53) as f64)
    }

    /// Uniform in `0..n`.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn pick<T: Copy>(&mut self, of: &[T]) -> T {
        of[self.below(of.len())]
    }

    fn vec(&mut self, half: f64) -> Vec3 {
        Vec3::new(
            self.range(-half, half),
            self.range(-half, half),
            self.range(-half, half),
        )
    }
}

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
/// Parameter domains: the unit interval, one that straddles zero, one far
/// from zero and short (few bits left between its knots), one long.
const DOMAINS: [(f64, f64); 4] = [(0.0, 1.0), (-3.5, 2.25), (1000.0, 1000.001), (0.0, 1e5)];

/// The numbers of a vector, with every NaN made the same NaN: equal bits are
/// asked for, but which NaN comes out of an operation is not part of the
/// result.
fn bits(v: Vec3) -> [u64; 3] {
    [v.x, v.y, v.z].map(|c| if c.is_nan() { u64::MAX } else { c.to_bits() })
}

/// A clamped knot vector for `count` control points of `degree` over
/// `domain`: the interior knots come in runs of 1 to `degree + 1` equal
/// values (a run of `degree` is a corner, of `degree + 1` a jump).
fn clamped_knots(rng: &mut Rng, degree: usize, count: usize, domain: (f64, f64)) -> Vec<f64> {
    let (a, b) = domain;
    let interior = count - degree - 1;
    let mut inner = Vec::new();
    while inner.len() < interior {
        let value = rng.range(a, b);
        let run = 1 + rng.below(degree + 1);
        for _ in 0..run.min(interior - inner.len()) {
            inner.push(value);
        }
    }
    inner.sort_by(f64::total_cmp);
    let mut knots = vec![a; degree + 1];
    knots.extend(inner);
    knots.extend(std::iter::repeat_n(b, degree + 1));
    knots
}

/// Any valid knot vector: non-decreasing with repeats, the ends not clamped,
/// so the domain starts and ends inside the vector.
fn loose_knots(rng: &mut Rng, degree: usize, count: usize, domain: (f64, f64)) -> Vec<f64> {
    let (a, b) = domain;
    loop {
        let mut knots = Vec::new();
        while knots.len() < count + degree + 1 {
            let value = rng.range(a, b);
            for _ in 0..1 + rng.below(3) {
                knots.push(value);
            }
        }
        knots.truncate(count + degree + 1);
        knots.sort_by(f64::total_cmp);
        if knots[degree] < knots[count] {
            return knots;
        }
    }
}

fn knots(rng: &mut Rng, degree: usize, count: usize) -> Vec<f64> {
    let domain = rng.pick(&DOMAINS);
    if rng.below(4) == 0 {
        loose_knots(rng, degree, count, domain)
    } else {
        clamped_knots(rng, degree, count, domain)
    }
}

/// Mostly low degrees, now and then a high one up to the supported maximum.
fn degree(rng: &mut Rng) -> usize {
    match rng.below(10) {
        0 => rng.pick(&[6, 7, 8, 9, 15, 16, 17, 31, 32]),
        _ => 1 + rng.below(5),
    }
}

fn count(rng: &mut Rng, degree: usize) -> usize {
    match rng.below(4) {
        // A single span: a Bézier curve.
        0 => degree + 1,
        _ => degree + 1 + rng.below(12),
    }
}

fn weights(rng: &mut Rng, n: usize, rational: bool) -> Vec<f64> {
    (0..n)
        .map(|_| if rational { rng.range(0.2, 5.0) } else { 1.0 })
        .collect()
}

/// Where to evaluate along one direction: both ends of the domain exactly,
/// every knot, a few units in the last place either side of every knot,
/// outside the domain, not a number, and random places inside.
fn parameters(rng: &mut Rng, knots: &[f64], degree: usize, random: usize) -> Vec<f64> {
    let count = knots.len() - degree - 1;
    let (start, end) = (knots[degree], knots[count]);
    let mut at = vec![
        start,
        end,
        start - 1.0,
        end + 1.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    let step = (end - start).abs().max(start.abs()).max(end.abs()) * 4.0 * f64::EPSILON;
    for &k in knots {
        at.extend([k, k - step, k + step]);
    }
    at.extend((0..random).map(|_| rng.range(start, end)));
    at
}

fn random_curve(rng: &mut Rng, rational: bool) -> NurbsCurve {
    let degree = degree(rng);
    let count = count(rng, degree);
    let scale = rng.pick(&SCALES);
    let ctrl = (0..count).map(|_| rng.vec(10.0) * scale).collect();
    NurbsCurve::new(
        degree,
        knots(rng, degree, count),
        ctrl,
        weights(rng, count, rational),
    )
    .unwrap()
}

fn random_surface(rng: &mut Rng, rational: bool) -> NurbsSurface {
    let (degree_u, degree_v) = (degree(rng), degree(rng));
    let (count_u, count_v) = (count(rng, degree_u), count(rng, degree_v));
    let scale = rng.pick(&SCALES);
    let ctrl = (0..count_u * count_v)
        .map(|_| rng.vec(10.0) * scale)
        .collect();
    NurbsSurface::new(
        degree_u,
        degree_v,
        knots(rng, degree_u, count_u),
        knots(rng, degree_v, count_v),
        count_u,
        count_v,
        ctrl,
        weights(rng, count_u * count_v, rational),
    )
    .unwrap()
}

/// Holds a curve to the reference at `t`; returns whether anything finite
/// came out, so a test can say it compared numbers and not only NaNs.
fn check_curve(c: &NurbsCurve, t: f64, what: &str) -> bool {
    let (p, d) = c.derivatives(t);
    let (old_p, old_d) = old::curve(c, t);
    assert_eq!(
        (bits(p), bits(d)),
        (bits(old_p), bits(old_d)),
        "{what} at t = {t:?}: {p:?} {d:?} against the reference {old_p:?} {old_d:?}"
    );
    assert_eq!(bits(c.eval(t)), bits(old_p), "{what}: eval at t = {t:?}");
    p.is_finite() && d.is_finite()
}

fn check_surface(s: &NurbsSurface, u: f64, v: f64, what: &str) -> bool {
    let (p, du, dv) = s.derivatives(u, v);
    let (old_p, old_du, old_dv) = old::surface(s, u, v);
    assert_eq!(
        (bits(p), bits(du), bits(dv)),
        (bits(old_p), bits(old_du), bits(old_dv)),
        "{what} at ({u:?}, {v:?}): {p:?} {du:?} {dv:?} against the reference \
         {old_p:?} {old_du:?} {old_dv:?}"
    );
    assert_eq!(
        bits(s.eval(u, v)),
        bits(old_p),
        "{what}: eval at ({u:?}, {v:?})"
    );
    p.is_finite() && du.is_finite() && dv.is_finite()
}

fn check_patch(b: &BezierPatch, u: f64, v: f64, what: &str) -> bool {
    let (p, du, dv) = b.derivatives(u, v);
    let (old_p, old_du, old_dv) = old::patch(b, u, v);
    assert_eq!(
        (bits(p), bits(du), bits(dv)),
        (bits(old_p), bits(old_du), bits(old_dv)),
        "{what} at ({u:?}, {v:?})"
    );
    assert_eq!(
        bits(b.eval(u, v)),
        bits(old_p),
        "{what}: eval at ({u:?}, {v:?})"
    );
    p.is_finite() && du.is_finite() && dv.is_finite()
}

#[test]
fn nurbs_curves_evaluate_as_before() {
    let mut rng = Rng(0x00C0_FFEE);
    let mut finite = 0;
    for case in 0..600 {
        let rational = case % 2 == 0;
        let c = random_curve(&mut rng, rational);
        let what = format!(
            "curve {case} (degree {}, {} points)",
            c.degree(),
            c.control_points().len()
        );
        for t in parameters(&mut rng, c.knots(), c.degree(), 12) {
            finite += usize::from(check_curve(&c, t, &what));
        }
    }
    // Everything but the NaN parameter gives numbers.
    assert!(finite > 600 * 20, "{finite} finite comparisons");
}

#[test]
fn nurbs_surfaces_evaluate_as_before() {
    let mut rng = Rng(0x5EED_0001);
    let mut finite = 0;
    for case in 0..240 {
        let rational = case % 2 == 0;
        let s = random_surface(&mut rng, rational);
        let what = format!(
            "surface {case} (degrees {:?}, counts {:?})",
            s.degrees(),
            s.counts()
        );
        let (du, dv) = s.degrees();
        let us = parameters(&mut rng, s.knots_u(), du, 4);
        let vs = parameters(&mut rng, s.knots_v(), dv, 4);
        // The ends of the domain against everything (which holds the four
        // corners and the four edges at every knot of the other direction);
        // elsewhere each u with a few of the v, a different few every time.
        let ((u0, u1), (v0, v1)) = s.domain();
        for &u in &us {
            for &v in &[v0, v1] {
                finite += usize::from(check_surface(&s, u, v, &what));
            }
        }
        for &v in &vs {
            for &u in &[u0, u1] {
                finite += usize::from(check_surface(&s, u, v, &what));
            }
        }
        for &u in &us {
            for _ in 0..6 {
                let v = rng.pick(&vs);
                finite += usize::from(check_surface(&s, u, v, &what));
            }
        }
    }
    assert!(finite > 240 * 100, "{finite} finite comparisons");
}

#[test]
fn the_ends_and_corners_of_a_clamped_surface_are_its_net() {
    // Independent of the reference: a clamped surface goes through the
    // corners of its net and its edges are the curves of the border rows.
    let mut rng = Rng(0xC0DE_0003);
    for case in 0..60 {
        let (degree_u, degree_v) = (1 + rng.below(4), 1 + rng.below(4));
        let (count_u, count_v) = (count(&mut rng, degree_u), count(&mut rng, degree_v));
        let scale = rng.pick(&SCALES);
        let ctrl: Vec<Vec3> = (0..count_u * count_v)
            .map(|_| rng.vec(10.0) * scale)
            .collect();
        let w = weights(&mut rng, count_u * count_v, case % 2 == 0);
        let ku = clamped_knots(&mut rng, degree_u, count_u, (0.0, 1.0));
        let kv = clamped_knots(&mut rng, degree_v, count_v, (-3.5, 2.25));
        let s = NurbsSurface::new(
            degree_u,
            degree_v,
            ku.clone(),
            kv.clone(),
            count_u,
            count_v,
            ctrl.clone(),
            w.clone(),
        )
        .unwrap();
        let ((u0, u1), (v0, v1)) = s.domain();
        // Rounding is relative to the coordinates: p * w / w is within a few
        // units in the last place of p.
        let eps = 1e-14 * 10.0 * scale;
        let at = |i: usize, j: usize| ctrl[i * count_v + j];
        for (u, v, corner) in [
            (u0, v0, at(0, 0)),
            (u0, v1, at(0, count_v - 1)),
            (u1, v0, at(count_u - 1, 0)),
            (u1, v1, at(count_u - 1, count_v - 1)),
        ] {
            let p = s.eval(u, v);
            assert!(p.distance(corner) <= eps, "case {case}: corner ({u}, {v})");
        }
        // The edge v = v1 is the curve of the last column; u = u0 that of
        // the first row.
        let column = |j: usize| -> (Vec<Vec3>, Vec<f64>) {
            (0..count_u).map(|i| (at(i, j), w[i * count_v + j])).unzip()
        };
        let (points, ws) = column(count_v - 1);
        let edge = NurbsCurve::new(degree_u, ku.clone(), points, ws).unwrap();
        let row: Vec<Vec3> = ctrl[..count_v].to_vec();
        let side = NurbsCurve::new(degree_v, kv.clone(), row, w[..count_v].to_vec()).unwrap();
        for k in 0..=16 {
            let f = f64::from(k) / 16.0;
            let u = u0 + (u1 - u0) * f;
            let (p, du, _) = s.derivatives(u, v1);
            let (q, dq) = edge.derivatives(u);
            assert!(
                p.distance(q) <= eps,
                "case {case}: edge v1 at u = {u}: {p:?} against {q:?}"
            );
            // Derivatives are differences of control points over knot spans:
            // relative to their own size.
            assert!(
                du.distance(dq) <= 1e-9 * (1.0 + dq.length()),
                "case {case}: tangent along v1 at u = {u}: {du:?} against {dq:?}"
            );
            let v = v0 + (v1 - v0) * f;
            let (p, _, dv) = s.derivatives(u0, v);
            let (q, dq) = side.derivatives(v);
            assert!(p.distance(q) <= eps, "case {case}: edge u0 at v = {v}");
            assert!(
                dv.distance(dq) <= 1e-9 * (1.0 + dq.length()),
                "case {case}: tangent along u0 at v = {v}: {dv:?} against {dq:?}"
            );
        }
    }
}

#[test]
fn derivatives_are_the_slope_of_the_surface() {
    // Independent of the reference: central differences of `eval`, away from
    // the knots (where a spline has fewer derivatives than its degree).
    let mut rng = Rng(0xD1FF_0004);
    let mut compared = 0;
    for case in 0..80 {
        let (degree_u, degree_v) = (2 + rng.below(3), 2 + rng.below(3));
        let (count_u, count_v) = (degree_u + 1 + rng.below(5), degree_v + 1 + rng.below(5));
        let ctrl: Vec<Vec3> = (0..count_u * count_v).map(|_| rng.vec(10.0)).collect();
        let uniform = |degree: usize, count: usize| -> Vec<f64> {
            let spans = count - degree;
            let mut k = vec![0.0; degree];
            k.extend((0..=spans).map(|i| i as f64));
            k.extend(std::iter::repeat_n(spans as f64, degree));
            k
        };
        let s = NurbsSurface::new(
            degree_u,
            degree_v,
            uniform(degree_u, count_u),
            uniform(degree_v, count_v),
            count_u,
            count_v,
            ctrl,
            weights(&mut rng, count_u * count_v, case % 2 == 0),
        )
        .unwrap();
        let ((_, u1), (_, v1)) = s.domain();
        let h = 1e-5;
        for _ in 0..20 {
            // Middle of a knot span, so the stencil stays inside it.
            let u = (rng.below(u1 as usize) as f64) + rng.range(0.1, 0.9);
            let v = (rng.below(v1 as usize) as f64) + rng.range(0.1, 0.9);
            let (p, du, dv) = s.derivatives(u, v);
            assert_eq!(bits(p), bits(s.eval(u, v)));
            let fd_u = (s.eval(u + h, v) - s.eval(u - h, v)) * (0.5 / h);
            let fd_v = (s.eval(u, v + h) - s.eval(u, v - h)) * (0.5 / h);
            // The error of the difference is h² times a sixth of the third
            // derivative; with weights between 0.2 and 5 on a net of size 10
            // that derivative is large, so the bound is relative and loose.
            // A wrong term of the quotient rule is off by the size of the
            // derivative itself.
            let tol = 1e-4 * (1.0 + du.length() + dv.length());
            assert!(
                du.distance(fd_u) <= tol && dv.distance(fd_v) <= tol,
                "case {case} at ({u}, {v}): {du:?} against {fd_u:?}, {dv:?} against {fd_v:?}"
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 1600);
}

#[test]
fn bezier_patches_evaluate_as_before() {
    let mut rng = Rng(0xBE21_E400);
    let mut finite = 0;
    for case in 0..300 {
        let degrees = |rng: &mut Rng| match rng.below(12) {
            0 => rng.pick(&[0, 7, 12, 31, 32]),
            _ => rng.below(6),
        };
        let (degree_u, degree_v) = (degrees(&mut rng), degrees(&mut rng));
        let scale = rng.pick(&SCALES);
        let ctrl = (0..(degree_u + 1) * (degree_v + 1))
            .map(|_| rng.vec(10.0) * scale)
            .collect();
        let b = BezierPatch::new(degree_u, degree_v, ctrl).unwrap();
        let what = format!("patch {case} (degrees {degree_u}, {degree_v})");
        // Ends, corners, the middle, outside (a patch extrapolates), not a
        // number, and random places.
        let mut at = vec![0.0, 1.0, 0.5, -0.25, 1.75, f64::NAN, f64::INFINITY];
        at.extend((0..5).map(|_| rng.range(0.0, 1.0)));
        for &u in &at {
            for &v in &at {
                finite += usize::from(check_patch(&b, u, v, &what));
            }
        }
        // The same patch as a NURBS surface goes through the spline code.
        let s = NurbsSurface::from_bezier(&b);
        if degree_u == 0 || degree_v == 0 {
            assert!(s.is_err(), "{what}: a spline has no degree 0");
        } else {
            let s = s.unwrap();
            for &u in &at {
                for &v in &at {
                    check_surface(&s, u, v, &what);
                }
            }
        }
    }
    assert!(finite > 300 * 80, "{finite} finite comparisons");
}

#[test]
fn a_transformed_or_cloned_spline_evaluates_from_its_own_net() {
    // The reference reads the control points and weights through the public
    // accessors; a copy that kept another surface's cached net would differ.
    let mut rng = Rng(0x7AA5_F0A7);
    for case in 0..60 {
        let turn = Transform::rotation_about(rng.vec(5.0), Vec3::new(0.3, -0.4, 1.0), 1.1)
            .unwrap()
            .then(&Transform::translation(rng.vec(50.0)));
        let flip = Transform::mirror(rng.vec(5.0), Vec3::new(1.0, -3.0, 2.0)).unwrap();
        let s = random_surface(&mut rng, case % 2 == 0);
        let c = random_curve(&mut rng, case % 2 == 0);
        let (du, dv) = s.degrees();
        let us = parameters(&mut rng, s.knots_u(), du, 3);
        let vs = parameters(&mut rng, s.knots_v(), dv, 3);
        let ts = parameters(&mut rng, c.knots(), c.degree(), 6);
        for t in [turn, flip, turn.then(&flip)] {
            let moved = s.transformed(&t).unwrap();
            assert_ne!(moved, s, "case {case}: the surface was moved");
            for &u in &us {
                let v = rng.pick(&vs);
                check_surface(&moved, u, v, "a transformed surface");
                // And through the enum, as the rest of the kernel calls it.
                let wrapped = Surface::Nurbs(s.clone()).transformed(&t).unwrap();
                let (p, a, b) = wrapped.derivatives(u, v);
                let (old_p, old_a, old_b) = old::surface(&moved, u, v);
                assert_eq!(
                    (bits(p), bits(a), bits(b)),
                    (bits(old_p), bits(old_a), bits(old_b))
                );
                // The image of the point, within rounding of the map.
                let direct = t.point(s.eval(u, v));
                if direct.is_finite() {
                    assert!(
                        p.distance(direct) <= 1e-9 * (1.0 + direct.length()),
                        "case {case} at ({u}, {v}): {p:?} against {direct:?}"
                    );
                }
            }
            let moved_curve = c.transformed(&t).unwrap();
            for &at in &ts {
                check_curve(&moved_curve, at, "a transformed curve");
                let wrapped = Curve3::Nurbs(c.clone()).transformed(&t).unwrap();
                let (p, d) = wrapped.derivatives(at);
                let (old_p, old_d) = old::curve(&moved_curve, at);
                assert_eq!((bits(p), bits(d)), (bits(old_p), bits(old_d)));
            }
        }
        let copy = s.clone();
        assert_eq!(copy, s);
        for &u in &us {
            check_surface(&copy, u, rng.pick(&vs), "a cloned surface");
        }
        let patch = BezierPatch::new(2, 3, (0..12).map(|_| rng.vec(10.0)).collect()).unwrap();
        for t in [turn, flip] {
            let moved = patch.transformed(&t).unwrap();
            for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.3, 0.8), (1.0, 0.0)] {
                check_patch(&moved, u, v, "a transformed patch");
                check_patch(&moved.transposed(), v, u, "a transposed patch");
                check_patch(&moved.reversed_u(), u, v, "a reversed patch");
            }
        }
    }
}

#[test]
fn equality_and_the_debug_text_are_those_of_the_defining_data() {
    // What a surface stores to evaluate quickly is not part of its value:
    // two splines are equal exactly when degrees, knots, control points and
    // weights are, and the debug text shows those and nothing else.
    let ctrl = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 1.0)];
    let knots = vec![0.0, 0.0, 1.0, 1.0];
    let surface = |ctrl: Vec<Vec3>, weights: Vec<f64>| {
        NurbsSurface::new(1, 1, knots.clone(), knots.clone(), 2, 2, ctrl, weights).unwrap()
    };
    let a = surface(ctrl.clone(), vec![1.0, 2.0, 3.0, 4.0]);
    assert_eq!(a, surface(ctrl.clone(), vec![1.0, 2.0, 3.0, 4.0]));
    assert_eq!(a, a.clone());
    assert_ne!(a, surface(ctrl.clone(), vec![1.0, 2.0, 3.0, 5.0]));
    let mut other = ctrl.clone();
    other[3] = Vec3::new(1.0, 1.0, 2.0);
    assert_ne!(a, surface(other, vec![1.0, 2.0, 3.0, 4.0]));
    let text = format!("{a:?}");
    assert_eq!(
        text,
        format!(
            "NurbsSurface {{ degree_u: 1, degree_v: 1, knots_u: {knots:?}, knots_v: {knots:?}, \
             count_v: 2, ctrl: {ctrl:?}, weights: [1.0, 2.0, 3.0, 4.0] }}"
        )
    );

    let curve = |weights: Vec<f64>| {
        NurbsCurve::new(
            1,
            vec![0.0, 0.0, 0.5, 1.0, 1.0],
            ctrl[..3].to_vec(),
            weights,
        )
        .unwrap()
    };
    let c = curve(vec![1.0, 2.0, 3.0]);
    assert_eq!(c, curve(vec![1.0, 2.0, 3.0]));
    assert_eq!(c, c.clone());
    assert_ne!(c, curve(vec![1.0, 2.0, 4.0]));
    assert_eq!(
        format!("{c:?}"),
        format!(
            "NurbsCurve {{ degree: 1, knots: [0.0, 0.0, 0.5, 1.0, 1.0], ctrl: {:?}, \
             weights: [1.0, 2.0, 3.0] }}",
            &ctrl[..3]
        )
    );
}

/// Best of three runs of `calls` calls each, in nanoseconds per call. Other
/// work on the machine only ever makes a run slower, so the least is the
/// closest to the cost of the code.
fn ns_per_call(calls: usize, mut f: impl FnMut(usize) -> Vec3) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..3 {
        let start = std::time::Instant::now();
        let mut sum = Vec3::ZERO;
        for i in 0..calls {
            sum += std::hint::black_box(f(std::hint::black_box(i)));
        }
        std::hint::black_box(sum);
        best = best.min(start.elapsed().as_nanos() as f64 / calls as f64);
    }
    best
}

fn timing_surface(rng: &mut Rng, degree: usize, count: usize, rational: bool) -> NurbsSurface {
    let ctrl = (0..count * count).map(|_| rng.vec(10.0)).collect();
    let uniform = || -> Vec<f64> {
        let spans = count - degree;
        let mut k = vec![0.0; degree];
        k.extend((0..=spans).map(|i| i as f64 / spans as f64));
        k.extend(std::iter::repeat_n(1.0, degree));
        k
    };
    NurbsSurface::new(
        degree,
        degree,
        uniform(),
        uniform(),
        count,
        count,
        ctrl,
        weights(rng, count * count, rational),
    )
    .unwrap()
}

#[test]
#[ignore = "timing; run in release with --ignored --nocapture"]
fn timings() {
    let mut rng = Rng(0x71ED_0005);
    // Parameters that walk over the whole domain, the same for every run.
    let at = |i: usize| {
        (
            (i * 37 % 1009) as f64 / 1009.0,
            (i * 91 % 997) as f64 / 997.0,
        )
    };
    eprintln!("ns per call, best of 3                  eval  derivatives");
    for (degree, count) in [(3, 8), (3, 20), (5, 12)] {
        for rational in [false, true] {
            let s = timing_surface(&mut rng, degree, count, rational);
            let calls = 200_000;
            let eval = ns_per_call(calls, |i| {
                let (u, v) = at(i);
                s.eval(u, v)
            });
            let derivatives = ns_per_call(calls, |i| {
                let (u, v) = at(i);
                let (p, du, dv) = s.derivatives(u, v);
                p + du + dv
            });
            let kind = if rational { "rational" } else { "polynomial" };
            eprintln!(
                "{:<38}{eval:>6.0}  {derivatives:>11.0}",
                format!("nurbs surface deg {degree} {count}x{count} {kind}")
            );
        }
    }
    for degree in [3, 5] {
        let ctrl = (0..(degree + 1) * (degree + 1))
            .map(|_| rng.vec(10.0))
            .collect();
        let b = BezierPatch::new(degree, degree, ctrl).unwrap();
        let calls = 1_000_000;
        let eval = ns_per_call(calls, |i| {
            let (u, v) = at(i);
            b.eval(u, v)
        });
        let derivatives = ns_per_call(calls, |i| {
            let (u, v) = at(i);
            let (p, du, dv) = b.derivatives(u, v);
            p + du + dv
        });
        eprintln!(
            "{:<38}{eval:>6.0}  {derivatives:>11.0}",
            format!("bezier patch deg {degree}")
        );
    }
    for (degree, count, rational) in [(3, 8, false), (3, 20, true), (3, 200, true)] {
        let ctrl = (0..count).map(|_| rng.vec(10.0)).collect();
        let c = NurbsCurve::new(
            degree,
            clamped_knots(&mut rng, degree, count, (0.0, 1.0)),
            ctrl,
            weights(&mut rng, count, rational),
        )
        .unwrap();
        let calls = 1_000_000;
        let eval = ns_per_call(calls, |i| c.eval(at(i).0));
        let derivatives = ns_per_call(calls, |i| {
            let (p, d) = c.derivatives(at(i).0);
            p + d
        });
        let kind = if rational { "rational" } else { "polynomial" };
        eprintln!(
            "{:<38}{eval:>6.0}  {derivatives:>11.0}",
            format!("nurbs curve deg {degree} {count} pts {kind}")
        );
    }
}
