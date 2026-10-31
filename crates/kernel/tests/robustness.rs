//! Seeded runs against independent oracles, and hostile input: profile
//! detection and extrusion return `Ok` or `Err`, never panic and never
//! hang, and every `Ok` is right.
//!
//! - Random polygons on an integer grid: whether a polygon is simple is
//!   decided here with exact integer arithmetic, and must agree with
//!   detection; the area must be the integer shoelace sum.
//! - Random circles on an integer grid: whether two meet, and how deeply
//!   each is nested, is decided with integers; the regions and their total
//!   area (even-odd) must agree.
//! - Garbage, and good profiles with one thing spoiled.
//! - Sizes up to the cap, with timings: every layout up to the old cap
//!   of 4096 curves (where all pairs were compared), and layouts that
//!   drawn and patterned sketches make up to the cap itself.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::{P, Rng, arc, catalogue, circle, line, polygon};
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::profile::{
    Curve2, MAX_CURVES, MAX_PAIRS, OLD_MAX_CURVES, SketchCurve, detect_profiles,
};
use craft3d_math::{Frame, Tolerance};
use craft3d_testkit::cost::WorkClock;
use craft3d_topo::{check, mass_properties};
use std::f64::consts::PI;

type I = [i64; 2];

fn icross(o: I, a: I, b: I) -> i64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// `p` is on the closed segment `a b`, given that the three are collinear.
fn within(a: I, b: I, p: I) -> bool {
    p[0] >= a[0].min(b[0])
        && p[0] <= a[0].max(b[0])
        && p[1] >= a[1].min(b[1])
        && p[1] <= a[1].max(b[1])
}

/// The closed segments `a b` and `c d` have a point in common.
fn segments_meet(a: I, b: I, c: I, d: I) -> bool {
    let (d1, d2) = (icross(c, d, a), icross(c, d, b));
    let (d3, d4) = (icross(a, b, c), icross(a, b, d));
    if ((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0)) {
        return true;
    }
    (d1 == 0 && within(c, d, a))
        || (d2 == 0 && within(c, d, b))
        || (d3 == 0 && within(a, b, c))
        || (d4 == 0 && within(a, b, d))
}

/// Whether the closed polygon through distinct `points` is simple: sides
/// meet only at the corner they share.
fn is_simple(points: &[I]) -> bool {
    let n = points.len();
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (points[i], points[(i + 1) % n]);
            let (c, d) = (points[j], points[(j + 1) % n]);
            let meet = if b == c || d == a {
                // Neighbours: only folding back onto each other is wrong.
                // (In a triangle every pair is neighbours once.)
                let (shared, p, q) = if b == c { (b, a, d) } else { (a, b, c) };
                let d = [p[0] - shared[0], p[1] - shared[1]];
                let e = [q[0] - shared[0], q[1] - shared[1]];
                icross(shared, p, q) == 0 && d[0] * e[0] + d[1] * e[1] > 0
            } else {
                segments_meet(a, b, c, d)
            };
            if meet {
                return false;
            }
        }
    }
    true
}

fn shoelace2(points: &[I]) -> i64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a[0] * b[1] - a[1] * b[0]
        })
        .sum()
}

#[test]
fn random_polygons_agree_with_exact_integer_geometry() {
    let tol = Tolerance::default();
    let mut rng = Rng(0xC0FFEE);
    let (mut simple, mut crossed) = (0, 0);
    for round in 0..6000 {
        let n = 3 + rng.below(5) as usize;
        let grid = 3 + rng.below(6);
        let mut points: Vec<I> = Vec::new();
        while points.len() < n {
            let p = [rng.below(grid) as i64, rng.below(grid) as i64];
            if !points.contains(&p) {
                points.push(p);
            }
        }
        let scale = [1e-3, 1.0, 1e3][round % 3];
        let floats: Vec<P> = points
            .iter()
            .map(|p| [p[0] as f64 * scale, p[1] as f64 * scale])
            .collect();
        let mut curves = polygon(1, 1, &floats);
        if round % 2 == 1 {
            curves = common::lines_reversed(&curves);
            rng.shuffle(&mut curves);
        }
        let result = detect_profiles(&curves, &tol);
        if is_simple(&points) {
            simple += 1;
            let profiles = result.unwrap_or_else(|e| panic!("{points:?} is simple: {e}"));
            assert_eq!(profiles.len(), 1, "{points:?}");
            let expected = shoelace2(&points).abs() as f64 / 2.0 * scale * scale;
            // Integers times a power of ten: only the rounding of the sum.
            assert!(
                (profiles[0].area() - expected).abs() <= 1e-12 * expected,
                "{points:?}: {} vs {expected}",
                profiles[0].area()
            );
            let body = extrude(&profiles, &Frame::WORLD, -scale, scale, &tol)
                .unwrap_or_else(|e| panic!("{points:?}: {e}"))
                .body;
            assert_eq!(check(&body, &tol), vec![]);
            assert_eq!(body.faces.len(), n + 2);
            let volume = mass_properties(&body).unwrap().volume;
            assert!((volume - expected * 2.0 * scale).abs() <= 1e-12 * volume);
        } else {
            crossed += 1;
            assert!(
                matches!(
                    result,
                    Err(KernelError::CrossingCurves { .. } | KernelError::ZeroAreaLoop { .. })
                ),
                "{points:?} is not simple: {result:?}"
            );
        }
    }
    // Both kinds were exercised.
    assert!(
        simple > 500 && crossed > 500,
        "{simple} simple, {crossed} not"
    );
}

#[test]
fn random_circles_agree_with_exact_integer_nesting() {
    let tol = Tolerance::default();
    let mut rng = Rng(0xD15C5);
    let (mut clear, mut touching, mut nested) = (0, 0, 0);
    for _ in 0..4000 {
        let n = 1 + rng.below(6) as usize;
        let discs: Vec<(I, i64)> = (0..n)
            .map(|_| {
                (
                    [rng.below(24) as i64, rng.below(24) as i64],
                    1 + rng.below(9) as i64,
                )
            })
            .collect();
        let mut curves: Vec<SketchCurve> = discs
            .iter()
            .enumerate()
            .map(|(i, (c, r))| circle(i as u64 * 3 + 1, [c[0] as f64, c[1] as f64], *r as f64))
            .collect();
        rng.shuffle(&mut curves);
        let d2 = |a: I, b: I| (a[0] - b[0]).pow(2) + (a[1] - b[1]).pow(2);
        // Two circles have a point in common when the distance of their
        // centres is between the difference and the sum of their radii.
        let meet = (0..n).any(|i| {
            (0..i).any(|j| {
                let d = d2(discs[i].0, discs[j].0);
                d >= (discs[i].1 - discs[j].1).pow(2) && d <= (discs[i].1 + discs[j].1).pow(2)
            })
        });
        let result = detect_profiles(&curves, &tol);
        if meet {
            touching += 1;
            assert!(
                matches!(result, Err(KernelError::CrossingCurves { .. })),
                "{discs:?}: {result:?}"
            );
            continue;
        }
        clear += 1;
        // Disc i is inside disc j when j is larger and their centres are
        // closer than the difference of the radii.
        let depth = |i: usize| {
            (0..n)
                .filter(|&j| {
                    discs[j].1 > discs[i].1
                        && d2(discs[i].0, discs[j].0) < (discs[j].1 - discs[i].1).pow(2)
                })
                .count()
        };
        let outers = (0..n).filter(|&i| depth(i) % 2 == 0).count();
        if outers < n {
            nested += 1;
        }
        // Even-odd: a disc at even depth adds its area, at odd depth
        // takes it away.
        let area: f64 = (0..n)
            .map(|i| {
                let sign = if depth(i) % 2 == 0 { 1.0 } else { -1.0 };
                sign * PI * (discs[i].1 * discs[i].1) as f64
            })
            .sum();
        let profiles = result.unwrap_or_else(|e| panic!("{discs:?}: {e}"));
        assert_eq!(profiles.len(), outers, "{discs:?}");
        let holes: usize = profiles.iter().map(|p| p.holes.len()).sum();
        assert_eq!(holes, n - outers, "{discs:?}");
        let total: f64 = profiles.iter().map(|p| p.area()).sum();
        assert!(
            (total - area).abs() <= 1e-12 * area,
            "{discs:?}: {total} vs {area}"
        );
        assert!(profiles.iter().all(|p| p.area() > 0.0));
        let body = extrude(&profiles, &Frame::WORLD, 0.0, 2.0, &tol)
            .unwrap()
            .body;
        assert_eq!(check(&body, &tol), vec![]);
        let volume = mass_properties(&body).unwrap().volume;
        assert!((volume - 2.0 * area).abs() <= 1e-12 * volume, "{discs:?}");
    }
    assert!(
        clear > 300 && touching > 300 && nested > 50,
        "{clear} {touching} {nested}"
    );
}

const HOSTILE: [f64; 20] = [
    0.0,
    -0.0,
    1.0,
    -1.0,
    2.0,
    3.0,
    0.5,
    1e-7,
    5e-7,
    1e-6,
    1e-300,
    f64::MIN_POSITIVE,
    1e100,
    -1e100,
    1e300,
    f64::MAX,
    f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
    1e16,
];

fn hostile(rng: &mut Rng) -> f64 {
    match rng.below(4) {
        0 => HOSTILE[rng.below(HOSTILE.len() as u64) as usize],
        1 => rng.below(4) as f64,
        2 => rng.unit() * 4.0,
        _ => rng.below(4) as f64 + (rng.unit() - 0.5) * 3e-6,
    }
}

fn hostile_curve(rng: &mut Rng, id: u64) -> SketchCurve {
    let kind = rng.below(3);
    let mut p = || [hostile(rng), hostile(rng)];
    match kind {
        0 => line(id, p(), p()),
        1 => arc(id, p(), p(), p()),
        _ => circle(id, p(), p()[0]),
    }
}

/// Runs both operations and checks whatever comes back. Returns whether
/// detection and extrusion succeeded.
fn survive(curves: &[SketchCurve], tol: &Tolerance) -> (bool, bool) {
    let Ok(profiles) = detect_profiles(curves, tol) else {
        return (false, false);
    };
    assert!(!profiles.is_empty());
    for p in &profiles {
        assert!(p.area() > 0.0 && p.area().is_finite(), "{curves:?}");
    }
    match extrude(&profiles, &Frame::WORLD, -1.0, 2.0, tol) {
        Ok(result) => {
            assert_eq!(check(&result.body, tol), vec![], "{curves:?}");
            (true, true)
        }
        Err(_) => (true, false),
    }
}

#[test]
fn garbage_never_panics() {
    let tol = Tolerance::default();
    let mut rng = Rng(0x0BAD_5EED);
    let mut detected = 0;
    for _ in 0..30_000 {
        let n = rng.below(10) as usize;
        let curves: Vec<SketchCurve> = (0..n)
            .map(|i| {
                // Sometimes a repeated id.
                let id = if rng.below(20) == 0 { 0 } else { i as u64 };
                hostile_curve(&mut rng, id)
            })
            .collect();
        detected += usize::from(survive(&curves, &tol).0);
    }
    // Random circles mostly; enough that the Ok path is exercised.
    assert!(detected > 100, "{detected}");
}

/// Good profiles with one thing spoiled: a coordinate replaced by a hostile
/// number or nudged by about the tolerance, a curve dropped or doubled.
#[test]
fn spoiled_profiles_never_panic() {
    let mut rng = Rng(0x0590_11ED);
    let (mut ok, mut detect_only, mut refused) = (0, 0, 0);
    for round in 0..20_000 {
        let k = [1e-3, 1.0, 1e6][round % 3];
        let tol = Tolerance::default();
        let cases = catalogue(k);
        let case = &cases[rng.below(cases.len() as u64) as usize];
        let mut curves = case.curves.clone();
        let pick = rng.below(curves.len() as u64) as usize;
        match rng.below(5) {
            0 => {
                curves.remove(pick);
            }
            1 => {
                let mut copy = curves[pick].clone();
                copy.id = 1000;
                curves.push(copy);
            }
            kind => {
                let value = |old: f64, rng: &mut Rng| match kind {
                    2 => hostile(rng),
                    3 => old + (rng.unit() - 0.5) * 4e-6,
                    _ => old + (rng.unit() - 0.5) * k,
                };
                let which = rng.below(6) as usize;
                let edit = |p: &mut P, rng: &mut Rng| p[which % 2] = value(p[which % 2], rng);
                match &mut curves[pick].curve {
                    Curve2::Line { start, end } => {
                        edit(if which < 3 { start } else { end }, &mut rng)
                    }
                    Curve2::Arc { center, start, end } => edit(
                        match which / 2 {
                            0 => center,
                            1 => start,
                            _ => end,
                        },
                        &mut rng,
                    ),
                    Curve2::Spline(spline) => {
                        let count = spline.control.len();
                        edit(&mut spline.control[which % count], &mut rng)
                    }
                    Curve2::Circle { center, radius } => {
                        if which < 4 {
                            edit(center, &mut rng)
                        } else {
                            *radius = value(*radius, &mut rng)
                        }
                    }
                }
            }
        }
        match survive(&curves, &tol) {
            (true, true) => ok += 1,
            (true, false) => detect_only += 1,
            _ => refused += 1,
        }
    }
    println!(
        "spoiled profiles: {ok} extruded, {detect_only} detected but not extruded, {refused} refused"
    );
    assert!(ok > 1000 && refused > 1000, "{ok} {refused}");
}

/// A regular polygon with `n` sides inscribed in the circle of radius `r`.
fn ngon(n: usize, r: f64, first_id: u64) -> Vec<SketchCurve> {
    let points: Vec<P> = (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * 2.0 * PI;
            [r * a.pcos(), r * a.psin()]
        })
        .collect();
    polygon(first_id, 1, &points)
}

/// A gear-like outline of `teeth` lines and `teeth` arcs: around a circle
/// of radius `r`, every other step is the chord and every other the arc.
fn scalloped(teeth: usize, r: f64) -> Vec<SketchCurve> {
    let n = teeth * 2;
    let at = |i: usize| {
        let a = (i % n) as f64 / n as f64 * 2.0 * PI;
        [r * a.pcos(), r * a.psin()]
    };
    (0..n)
        .map(|i| {
            if i % 2 == 0 {
                line(i as u64, at(i), at(i + 1))
            } else {
                arc(i as u64, [0.0, 0.0], at(i), at(i + 1))
            }
        })
        .collect()
}

fn timed(name: &str, curves: &[SketchCurve], expect_area: Option<f64>) {
    let tol = Tolerance::default();
    // On the work of this thread, which a busy machine does not stretch.
    let start = WorkClock::start();
    let result = detect_profiles(curves, &tol);
    let detect = start.elapsed();
    let start = WorkClock::start();
    let extruded = result
        .as_ref()
        .ok()
        .map(|p| extrude(p, &Frame::WORLD, 0.0, 1.0, &tol));
    let sweep = start.elapsed();
    println!(
        "{name}: {} curves, detect {detect:.2?} ({}), extrude and check {sweep:.2?}",
        curves.len(),
        if result.is_ok() { "ok" } else { "refused" },
    );
    // Never hang: generous for an unoptimised build on a loaded machine.
    assert!(detect.as_secs() < 60 && sweep.as_secs() < 60, "{name}");
    if let Some(area) = expect_area {
        let profiles = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        let total: f64 = profiles.iter().map(|p| p.area()).sum();
        assert!(
            (total - area).abs() <= 1e-9 * area,
            "{name}: {total} vs {area}"
        );
        let body = extruded
            .unwrap()
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .body;
        assert_eq!(check(&body, &tol), vec![]);
    }
}

#[test]
fn large_and_adversarial_inputs_finish() {
    // A regular n-gon inscribed in radius r has area n r² sin(2π/n) / 2.
    let ngon_area = |n: usize, r: f64| n as f64 * r * r * (2.0 * PI / n as f64).psin() / 2.0;
    timed("300-gon", &ngon(300, 100.0, 0), Some(ngon_area(300, 100.0)));
    // Chords and arcs alternate: half the sectors are whole (r² θ / 2 each),
    // half are triangles (r² sin θ / 2), with θ = 2π / 400.
    let theta = 2.0 * PI / 400.0;
    let area = 200.0 * 50.0 * 50.0 * (theta + theta.psin()) / 2.0;
    timed(
        "scalloped, 200 lines and 200 arcs",
        &scalloped(200, 50.0),
        Some(area),
    );
    timed(
        "the old cap as one polygon",
        &ngon(OLD_MAX_CURVES, 1000.0, 0),
        Some(ngon_area(OLD_MAX_CURVES, 1000.0)),
    );

    // The old cap as discs in a grid, every one with a hole.
    let mut curves = Vec::new();
    for i in 0..OLD_MAX_CURVES / 2 {
        let c = [(i % 64) as f64 * 5.0, (i / 64) as f64 * 5.0];
        curves.push(circle(2 * i as u64, c, 2.0));
        curves.push(circle(2 * i as u64 + 1, c, 1.0));
    }
    // Each ring has area π (2² − 1²).
    timed(
        "rings in a grid",
        &curves,
        Some(OLD_MAX_CURVES as f64 / 2.0 * 3.0 * PI),
    );

    // The old cap as concentric circles of radius 1, 2, …: every box overlaps
    // every other. Even-odd: rings between radius 2k−1 and 2k, each of
    // area π (4k − 1), k = 1 … n/2; summed, π (n/2) (n + 1).
    let n = OLD_MAX_CURVES;
    let curves: Vec<SketchCurve> = (0..n)
        .map(|i| circle(i as u64, [0.0, 0.0], (i + 1) as f64))
        .collect();
    let area = PI * (n / 2) as f64 * (n + 1) as f64;
    timed("concentric circles", &curves, Some(area));

    // The old cap as the same line over and over, as lines fanning out of one
    // point, as crossing lines, and as tiny loops on top of each other.
    let same: Vec<SketchCurve> = (0..n as u64)
        .map(|i| line(i, [0.0, 0.0], [1.0, 1.0]))
        .collect();
    timed("one line many times", &same, None);
    let fan: Vec<SketchCurve> = (0..n as u64)
        .map(|i| line(i, [0.0, 0.0], [(i as f64).pcos(), (i as f64).psin()]))
        .collect();
    timed("a fan of lines", &fan, None);
    let mut mesh = Vec::new();
    for i in 0..n / 8 {
        // Squares that all cross each other.
        let o = i as f64 * 1e-3;
        mesh.extend(common::rectangle(
            i as u64 * 4,
            1,
            (o, o + 10.0),
            (-o, 10.0 - o),
        ));
    }
    timed("crossing squares", &mesh, None);
    // Concentric circles 3e-6 apart: close, but not touching.
    let stacked: Vec<SketchCurve> = (0..n as u64 / 2)
        .map(|i| circle(i, [0.0, 0.0], 1.0 + i as f64 * 3e-6))
        .collect();
    timed("circles a hair apart", &stacked, None);
    let column: Vec<SketchCurve> = (0..n as u64)
        .map(|i| line(i, [0.0, i as f64], [0.0, i as f64 + 0.5]))
        .collect();
    timed("ends in a column", &column, None);
}

/// At the cap, in every test run: a polygon is accepted, layouts whose
/// curves are all near one another are refused by the bound on pairs
/// (naming curves), and one quadratic in nesting but under the bound is
/// accepted. Every layout: `the_cap_finishes_with_every_layout`.
#[test]
fn the_cap_finishes() {
    let ngon_area = |n: usize, r: f64| n as f64 * r * r * (2.0 * PI / n as f64).psin() / 2.0;
    timed(
        "the cap as one polygon",
        &ngon(MAX_CURVES, 1e5, 0),
        Some(ngon_area(MAX_CURVES, 1e5)),
    );
    let same: Vec<SketchCurve> = (0..MAX_CURVES as u64)
        .map(|i| line(i, [0.0, 0.0], [1.0, 1.0]))
        .collect();
    crowded("one line many times", &same, 4 * MAX_PAIRS);
    let mut gons = Vec::new();
    let mut area = 0.0;
    for i in 0..50_u64 {
        let r = 10.0 + i as f64;
        gons.extend(ngon(1000, r, i * 1000));
        let a = ngon_area(1000, r);
        area += if (49 - i) % 2 == 0 { a } else { -a };
    }
    timed("50 nested 1000-gons", &gons, Some(area));
}

/// At the cap, every adversarial layout (run in release: `cargo test
/// --release -p craft3d-kernel --test robustness -- --ignored --nocapture`;
/// the times are in docs/design/sweeps.md). Layouts that sketches make are accepted, and layouts whose
/// curves are all near one another are refused by the bound on pairs,
/// quickly, instead of being compared pair by pair.
#[test]
#[ignore = "seconds per layout; run in release with --ignored --nocapture"]
fn the_cap_finishes_with_every_layout() {
    let ngon_area = |n: usize, r: f64| n as f64 * r * r * (2.0 * PI / n as f64).psin() / 2.0;
    timed(
        "the cap as one polygon",
        &ngon(MAX_CURVES, 1e5, 0),
        Some(ngon_area(MAX_CURVES, 1e5)),
    );
    let mut rings = Vec::new();
    for i in 0..MAX_CURVES / 2 {
        let c = [(i % 512) as f64 * 5.0, (i / 512) as f64 * 5.0];
        rings.push(circle(2 * i as u64, c, 2.0));
        rings.push(circle(2 * i as u64 + 1, c, 1.0));
    }
    timed(
        "rings in a grid",
        &rings,
        Some(MAX_CURVES as f64 / 2.0 * 3.0 * PI),
    );
    // Every adversarial layout of the old cap, at the cap. Those whose
    // curves are all near one another are refused by the bound on pairs,
    // naming curves, after the bounded work and no more.
    let n = MAX_CURVES;
    let theta = 2.0 * PI / n as f64;
    timed(
        "scalloped",
        &scalloped(n / 2, 1e5),
        Some((n / 2) as f64 * 1e10 * (theta + theta.psin()) / 2.0),
    );
    let concentric: Vec<SketchCurve> = (0..n)
        .map(|i| circle(i as u64, [0.0, 0.0], (i + 1) as f64))
        .collect();
    crowded("concentric circles", &concentric, MAX_PAIRS);
    let same: Vec<SketchCurve> = (0..n as u64)
        .map(|i| line(i, [0.0, 0.0], [1.0, 1.0]))
        .collect();
    crowded("one line many times", &same, 4 * MAX_PAIRS);
    let fan: Vec<SketchCurve> = (0..n as u64)
        .map(|i| line(i, [0.0, 0.0], [(i as f64).pcos(), (i as f64).psin()]))
        .collect();
    crowded("a fan of lines", &fan, 4 * MAX_PAIRS);
    let stacked: Vec<SketchCurve> = (0..n as u64 / 2)
        .map(|i| circle(i, [0.0, 0.0], 1.0 + i as f64 * 3e-6))
        .collect();
    crowded("circles a hair apart", &stacked, MAX_PAIRS);
    let mut nested = Vec::new();
    for i in 0..n / 4 {
        let d = i as f64;
        nested.extend(common::rectangle(
            i as u64 * 4,
            1,
            (-1.0 - d, 1.0 + d),
            (-1.0 - d, 1.0 + d),
        ));
    }
    crowded("nested squares", &nested, MAX_PAIRS);
    let mut mesh = Vec::new();
    for i in 0..n / 8 {
        let o = i as f64 * 1e-3;
        mesh.extend(common::rectangle(
            i as u64 * 4,
            1,
            (o, o + 10.0),
            (-o, 10.0 - o),
        ));
    }
    timed("crossing squares", &mesh, None);
    // Under the bound but quadratic in nesting: every loop holds the
    // point of every smaller one. Accepted, with the even-odd area.
    let mut nested = Vec::new();
    let mut area = 0.0;
    for i in 0..5000_usize {
        let d = i as f64;
        nested.extend(common::rectangle(
            i as u64 * 4,
            1,
            (-1.0 - d, 1.0 + d),
            (-1.0 - d, 1.0 + d),
        ));
        // Square i is at depth 4999 - i: even depths add, odd take away.
        let side = 2.0 + 2.0 * d;
        area += if (4999 - i) % 2 == 0 {
            side * side
        } else {
            -side * side
        };
    }
    timed("5000 nested squares", &nested, Some(area));
    // Loops of many segments round each other: winding numbers through
    // the tree of stretches.
    let mut gons = Vec::new();
    let mut area = 0.0;
    for i in 0..200_u64 {
        let r = 10.0 + i as f64;
        gons.extend(ngon(1000, r, i * 1000));
        let a = ngon_area(1000, r);
        area += if (199 - i) % 2 == 0 { a } else { -a };
    }
    timed("200 nested 1000-gons", &gons, Some(area));
    let column: Vec<SketchCurve> = (0..n as u64)
        .map(|i| line(i, [0.0, i as f64], [0.0, i as f64 + 0.5]))
        .collect();
    timed("ends in a column", &column, None);
}

/// `curves` are refused by the bound on pairs (`limit`), naming curves,
/// within the time `timed` allows.
fn crowded(name: &str, curves: &[SketchCurve], limit: usize) {
    let started = WorkClock::start();
    let result = detect_profiles(curves, &Tolerance::default());
    let took = started.elapsed();
    println!("{name}: {} curves, refused in {took:.2?}", curves.len());
    match result {
        Err(KernelError::CrowdedCurves { curves, limit: l }) => {
            assert_eq!(l, limit, "{name}");
            assert!(!curves.is_empty(), "{name}");
        }
        other => panic!("{name}: {other:?}"),
    }
    assert!(took.as_secs() < 60, "{name}");
}
