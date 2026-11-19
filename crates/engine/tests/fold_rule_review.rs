//! The review of the fold rule (`craft3d_topo::folds`, check/fold-check at
//! c94ccc7d; docs/design/verification.md › *A face folded over itself*),
//! by its own means: the cache key (the tolerance, a vertex's own
//! tolerance), the pair cap of a search and what it says on valid
//! notches and on the seventh fade (an instrumented copy of the rule),
//! the cost of a face with suspects and many loops, slots and a pleat at
//! three scales and two tolerances, trimmed slots, a relief with noise
//! near the tolerance, hostile surfaces, spans of too high a degree, the
//! words agents read, and a census of the repository's documents with the
//! switch `FOLDS_REFUSED` as it is built (run once with it on, on the
//! reviewer's branch only). Lines `REVIEW …` (with `-- --nocapture`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::type_complexity,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::approx_constant,
    clippy::manual_range_contains,
    clippy::excessive_precision
)]

mod common;
mod fold_common;

use craft3d_geom::{Circle, Curve3, NurbsCurve, NurbsSurface, Surface};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, Coedge, Defect, Fold, FoldKind, check, folds, forget_folds};
use fold_common::{narrow, notch_bodies, read_step, right_angle, roof_block, script};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Instant;

const SEVEN: [&str; 7] = [
    "fade-3-6-0.33",
    "fade-3-6-0.5",
    "fade-4-8-0.33",
    "fade-4-8-0.5",
    "fade-5-10-1",
    "fade-3-3-2",
    "fade-4-8-1",
];

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fold_probe")
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A tolerance of any size (the helper's `Tolerance::new` may refuse the
/// extremes).
fn tol(linear: f64) -> Tolerance {
    Tolerance {
        linear,
        angular: 1e-9,
    }
}

fn fade(name: &str) -> Body {
    read_step(&fixtures().join(format!("fades/{name}.step")), 0.02)
        .unwrap_or_else(|| panic!("{name} not read"))
}

/// The places said, without the tolerance they were asked at (which a
/// store that left the tolerance out of its key would still fill in from
/// the call).
fn places(found: &[Fold]) -> Vec<(u32, Vec3, FoldKind)> {
    found
        .iter()
        .map(|f| (f.face.0, f.at, f.kind.clone()))
        .collect()
}

fn shown(found: &[Fold]) -> usize {
    found.iter().filter(|f| f.is_fold()).count()
}

fn secs(start: Instant) -> f64 {
    start.elapsed().as_secs_f64()
}

fn clamped(n: usize, degree: usize) -> Vec<f64> {
    let inner = n - degree - 1;
    let mut k = vec![0.0; degree + 1];
    for i in 1..=inner {
        k.push(i as f64 / (inner + 1) as f64);
    }
    k.extend(vec![1.0; degree + 1]);
    k
}

fn segment(a: Vec3, b: Vec3) -> Curve3 {
    Curve3::Nurbs(NurbsCurve::polynomial(1, vec![0.0, 0.0, 1.0, 1.0], vec![a, b]).unwrap())
}

/// A sheet of one face: a cubic B-spline profile (control points in x, y,
/// uniform knots) drawn along z by `depth`, trimmed to the profile's
/// parameters `trim`, its sense as the surface has it. Its edges are the
/// profile at z = 0 and z = depth and two segments along z, all on the
/// surface.
fn sheet(profile: &[[f64; 2]], depth: f64, trim: (f64, f64)) -> Body {
    let n = profile.len();
    let knots = clamped(n, 3);
    let mut net = Vec::with_capacity(2 * n);
    for p in profile {
        net.push(Vec3::new(p[0], p[1], 0.0));
        net.push(Vec3::new(p[0], p[1], depth));
    }
    let surface = NurbsSurface::new(
        3,
        1,
        knots.clone(),
        vec![0.0, 0.0, 1.0, 1.0],
        n,
        2,
        net,
        vec![1.0; 2 * n],
    )
    .unwrap();
    let row = |z: f64| -> Curve3 {
        Curve3::Nurbs(
            NurbsCurve::polynomial(
                3,
                knots.clone(),
                profile.iter().map(|p| Vec3::new(p[0], p[1], z)).collect(),
            )
            .unwrap(),
        )
    };
    let s = Surface::Nurbs(surface);
    let (a, b) = trim;
    let mut body = Body::new(BodyKind::Sheet);
    let a0 = body.add_vertex(s.eval(a, 0.0));
    let b0 = body.add_vertex(s.eval(b, 0.0));
    let a1 = body.add_vertex(s.eval(a, 1.0));
    let b1 = body.add_vertex(s.eval(b, 1.0));
    let low = body.add_edge(row(0.0), (a, b), a0, b0);
    let high = body.add_edge(row(depth), (a, b), a1, b1);
    let end = body.add_edge(segment(s.eval(b, 0.0), s.eval(b, 1.0)), (0.0, 1.0), b0, b1);
    let start = body.add_edge(segment(s.eval(a, 0.0), s.eval(a, 1.0)), (0.0, 1.0), a0, a1);
    let lp = body.add_loop(vec![
        Coedge {
            edge: low,
            reversed: false,
        },
        Coedge {
            edge: end,
            reversed: false,
        },
        Coedge {
            edge: high,
            reversed: true,
        },
        Coedge {
            edge: start,
            reversed: true,
        },
    ]);
    let face = body.add_face(s, false, vec![lp]);
    body.add_shell(vec![face]);
    body
}

/// Points on the circle of radius `r` about the origin, at 202.5°, 225°,
/// …, 337.5° (the lower half, without its ends), as (cos, sin).
const LOWER: [(f64, f64); 7] = [
    (-0.923_879_532_511_286_7, -0.382_683_432_365_089_8),
    (-0.707_106_781_186_547_6, -0.707_106_781_186_547_6),
    (-0.382_683_432_365_089_8, -0.923_879_532_511_286_7),
    (0.0, -1.0),
    (0.382_683_432_365_089_8, -0.923_879_532_511_286_7),
    (0.707_106_781_186_547_6, -0.707_106_781_186_547_6),
    (0.923_879_532_511_286_7, -0.382_683_432_365_089_8),
];

/// A slot drawn as one spline: two walls `2 r` apart and `l` long, a round
/// bottom of radius about `r` (control points on its circle).
fn slot(r: f64, l: f64) -> Vec<[f64; 2]> {
    let mut p = vec![[-r, l], [-r, 2.0 * l / 3.0], [-r, l / 3.0], [-r, 0.0]];
    for (c, s) in LOWER {
        p.push([r * c, r * s]);
    }
    p.extend([[r, 0.0], [r, l / 3.0], [r, 2.0 * l / 3.0], [r, l]]);
    p
}

/// A pleat drawn as one spline: three layers `2 r` apart, turned by two
/// round bends of radius about `r` (a Z in section).
fn pleat(r: f64, l: f64) -> Vec<[f64; 2]> {
    let mut p = vec![
        [0.0, 2.0 * r],
        [l / 3.0, 2.0 * r],
        [2.0 * l / 3.0, 2.0 * r],
        [l, 2.0 * r],
    ];
    // The right bend about (l, r), from the top down: 67.5°, 45°, …,
    // -67.5° about its centre.
    for (c, s) in LOWER {
        p.push([l - s * r, r - c * r]);
    }
    p.extend([[l, 0.0], [2.0 * l / 3.0, 0.0], [l / 3.0, 0.0], [0.0, 0.0]]);
    // The left bend about (0, -r): the left half, from the top down.
    for (c, s) in LOWER {
        p.push([s * r, -r - c * r]);
    }
    p.extend([
        [0.0, -2.0 * r],
        [l / 3.0, -2.0 * r],
        [2.0 * l / 3.0, -2.0 * r],
        [l, -2.0 * r],
    ]);
    p
}

/// The sheet's sense set so that its outward normal at the profile's
/// parameter `probe` points along `towards`.
fn facing(mut body: Body, probe: f64, towards: Vec3) -> Body {
    let (_, su, sv) = body.faces[0].surface.derivatives(probe, 0.5);
    body.faces[0].reversed = su.cross(sv).dot(towards) < 0.0;
    body
}

/// The sheet's loop run the other way (so that it runs anticlockwise
/// about the outward normal of a face whose sense `facing` turned, as
/// the validity check wants; the fold rule reads either way alike).
fn turned(mut body: Body) -> Body {
    let coedges = &mut body.loops[0].coedges;
    coedges.reverse();
    for c in coedges.iter_mut() {
        c.reversed = !c.reversed;
    }
    body
}

/// The rule with its bound on pairs of pieces a parameter
/// (`craft3d_topo::folds_examined`, without the store of faces), and what
/// its search did: what the review reads the bound and the in-face test by.
/// (At c94ccc7d this was a copy of the rule as it was then, its pair cap a
/// parameter; the rule's search was rebuilt after the review, so the copy
/// is now the rule itself, asked with its bound.)
mod mirror {
    #![allow(dead_code, clippy::new_without_default)]

    use craft3d_math::{Tolerance, Vec3};
    use craft3d_topo::{Body, FoldKind};

    /// What the searches met: faces searched, pairs of pieces compared
    /// (in all and on the face that took most), faces whose search
    /// reached the bound, and the pairs meeting the definition by how
    /// their points are told.
    #[derive(Clone, Debug)]
    pub struct Stats {
        pub searches: u64,
        pub pairs: u64,
        pub most_pairs: u64,
        pub capped: u64,
        pub in_in: u64,
        pub near: u64,
        pub not_told: u64,
        pub out: u64,
        pub least_in_in: f64,
        pub least_doubtful: f64,
        /// Not kept by the rule's own search (its pieces are not boxes in
        /// parameters about a place).
        pub boxes: Vec<(f64, f64, usize)>,
    }

    impl Stats {
        pub fn new() -> Self {
            Stats {
                searches: 0,
                pairs: 0,
                most_pairs: 0,
                capped: 0,
                in_in: 0,
                near: 0,
                not_told: 0,
                out: 0,
                least_in_in: f64::INFINITY,
                least_doubtful: f64::INFINITY,
                boxes: Vec::new(),
            }
        }
    }

    /// `craft3d_topo::folds` without its store, its bound `cap` pairs of
    /// pieces a face.
    pub fn folds(body: &Body, t: f64, cap: u64, stats: &mut Stats) -> Vec<(u32, Vec3, FoldKind)> {
        let tol = Tolerance {
            linear: t,
            angular: 1e-9,
        };
        let (found, work) = craft3d_topo::folds_examined(body, &tol, cap);
        stats.searches += work.faces;
        stats.pairs += work.pairs;
        stats.most_pairs = stats.most_pairs.max(work.most_pairs);
        stats.capped += work.spent;
        stats.in_in += work.in_in;
        stats.near += work.near;
        stats.not_told += work.not_told;
        stats.out += work.out;
        stats.least_in_in = stats.least_in_in.min(work.least_in_in);
        stats.least_doubtful = stats.least_doubtful.min(work.least_doubtful);
        found
            .iter()
            .map(|f| (f.face.0, f.at, f.kind.clone()))
            .collect()
    }
}

// ---------------------------------------------------------------------
// 4. Determinism: the store's key.

/// The store must not answer one tolerance with another's places, nor a
/// face whose vertex's own tolerance changed with the places of before:
/// each answer from a warm store equals the cold one, and the two
/// tolerances (and the two vertex tolerances) give different places, so
/// that a key without them would fail here.
#[test]
fn review_the_store_is_keyed_by_the_tolerance_and_the_vertex_tolerances() {
    let body = fade("fade-3-6-0.5");
    let (a, b) = (tol(0.02), tol(0.01));
    forget_folds();
    let cold_a = places(&folds(&body, &a));
    forget_folds();
    let cold_b = places(&folds(&body, &b));
    println!("REVIEW key t 0.02: {cold_a:?}");
    println!("REVIEW key t 0.01: {cold_b:?}");
    assert_ne!(cold_a, cold_b, "teeth: two tolerances, two answers");
    forget_folds();
    let _ = folds(&body, &a);
    let warm_b = places(&folds(&body, &b));
    forget_folds();
    let _ = folds(&body, &b);
    let warm_a = places(&folds(&body, &a));
    assert_eq!(warm_b, cold_b, "0.01 answered from 0.02's store");
    assert_eq!(warm_a, cold_a, "0.02 answered from 0.01's store");

    // A vertex of the folded face given a tolerance of its own whose tip
    // ball holds every place shown at 0.02.
    let at: Vec<Vec3> = cold_a
        .iter()
        .filter(|(_, _, k)| !matches!(k, FoldKind::NotProven { .. }))
        .map(|(_, p, _)| *p)
        .collect();
    assert!(!at.is_empty(), "teeth: a fold shown at 0.02");
    let face = cold_a[0].0 as usize;
    let mut vertices = Vec::new();
    for id in &body.faces[face].loops {
        for coedge in &body.loop_(*id).unwrap().coedges {
            let edge = body.edge(coedge.edge).unwrap();
            vertices.push(edge.start);
            vertices.push(edge.end);
        }
    }
    let reach = |v: craft3d_topo::VertexId| -> f64 {
        let q = body.vertex(v).unwrap().point;
        at.iter().map(|p| p.distance(q)).fold(0.0_f64, f64::max)
    };
    let nearest = *vertices
        .iter()
        .min_by(|x, y| reach(**x).total_cmp(&reach(**y)))
        .unwrap();
    let own = 0.3 * reach(nearest) + 0.02;
    let mut loose = body.clone();
    loose.set_vertex_tolerance(nearest, own).unwrap();
    forget_folds();
    let _ = folds(&body, &a);
    let warm_loose = places(&folds(&loose, &a));
    forget_folds();
    let cold_loose = places(&folds(&loose, &a));
    println!(
        "REVIEW key vertex {nearest:?} own tolerance {own}: {cold_loose:?} (reach {})",
        reach(nearest)
    );
    assert_ne!(
        cold_loose, cold_a,
        "teeth: the vertex's tolerance changes the answer"
    );
    assert_eq!(
        warm_loose, cold_loose,
        "a vertex's own tolerance answered from the store"
    );
}

// ---------------------------------------------------------------------
// 2, 3 and 5. The pair cap: valid notches, the seventh fade.

/// The rule's search stops at 4 million candidate pairs and then says
/// "not proven ... near an edge, or where its loops cannot be followed".
/// The copy of the rule (`mirror`) is checked against the rule on every
/// case, then asked without the cap: how many pairs a search meets, and
/// whether the cap or the in-face test makes a place "not proven".
#[test]
fn review_the_pair_cap_on_valid_notches_and_on_the_seventh_fade() {
    let mut failures: Vec<String> = Vec::new();
    let mut cases: Vec<(String, Body, f64)> = Vec::new();
    for name in SEVEN {
        cases.push((name.to_string(), fade(name), 0.02));
    }
    for (tag, control) in [
        ("notch 90 root 0.05", right_angle(0.05)),
        ("notch 90 root 0.02", right_angle(0.02)),
        ("notch 30", narrow(0.05)),
    ] {
        for b in notch_bodies(
            &format!("rev-{}", tag.replace(' ', "-")),
            0.02,
            &control,
            1.0,
        ) {
            cases.push((tag.to_string(), (*b).clone(), 0.02));
        }
    }
    assert!(cases.len() >= 10, "{} cases", cases.len());
    for (name, body, t) in &cases {
        forget_folds();
        let start = Instant::now();
        let rule = places(&folds(body, &tol(*t)));
        let rule_s = secs(start);
        let mut capped = mirror::Stats::new();
        let mine = mirror::folds(body, *t, 4_000_000, &mut capped);
        let mut free = mirror::Stats::new();
        let start = Instant::now();
        let open = mirror::folds(body, *t, u64::MAX, &mut free);
        let open_s = secs(start);
        println!("REVIEW cap {name}: the rule ({rule_s:.2} s): {rule:?}");
        println!("REVIEW cap {name}: capped at 4e6: {capped:?}");
        println!("REVIEW cap {name}: no cap ({open_s:.2} s): {free:?}");
        println!("REVIEW cap {name}: no cap says: {open:?}");
        if mine != rule {
            failures.push(format!(
                "{name}: the copy is not the rule: {mine:?} against {rule:?}"
            ));
        }
        if name.starts_with("notch") && !rule.is_empty() {
            failures.push(format!(
                "{name}: a valid notch is said not proven ({} places; the cap cut {} of {} searches, most pairs {}): {rule:?}",
                rule.len(),
                capped.capped,
                capped.searches,
                capped.most_pairs
            ));
        }
        if name == "fade-4-8-1" {
            let shown_free = open
                .iter()
                .filter(|(_, _, k)| matches!(k, FoldKind::Layers { .. }))
                .count();
            println!(
                "REVIEW cap fade-4-8-1: shown without the cap: {shown_free}; pairs in the face {} (least {}), near {} not told {} (least {})",
                free.in_in, free.least_in_in, free.near, free.not_told, free.least_doubtful
            );
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------------------------------------------------------------------
// 6. Cost: a face with suspects and many loops.

/// The walk follows every edge of a face in 1024 steps and tells every
/// point of every search against every chord of every loop: the work of
/// a face is bounded in suspects, searches and pairs, not in its edges.
/// The valid notch (two suspects at its root) with 0, 16 and 64 small
/// round holes in its spline face, away from the root.
#[test]
fn review_a_face_with_suspects_and_many_loops() {
    let made = notch_bodies("rev-holes", 0.02, &right_angle(0.05), 1.0);
    let base: Body = (**made.first().expect("the notch is built")).clone();
    let fi = base
        .faces
        .iter()
        .position(|f| matches!(f.surface, Surface::Nurbs(_)))
        .unwrap();
    let surface = base.faces[fi].surface.clone();
    let ((u0, u1), (v0, v1)) = surface.domain();
    // The profile runs along the parameter whose ends lie farther apart
    // (40 mm against the extrusion's 10); its root is at its middle.
    let (um, vm) = (0.5 * (u0 + u1), 0.5 * (v0 + v1));
    let profile_is_u = surface.eval(u0, vm).distance(surface.eval(u1, vm))
        > surface.eval(um, v0).distance(surface.eval(um, v1));
    let mut centres: Vec<(f64, f64)> = Vec::new();
    let m = 16;
    for i in 1..=m {
        for j in 1..=m {
            let (fa, fb) = (i as f64 / (m + 1) as f64, j as f64 / (m + 1) as f64);
            if (fa - 0.5).abs() < 0.15 || fb < 0.08 || fb > 0.92 {
                continue;
            }
            centres.push(if profile_is_u {
                (u0 + (u1 - u0) * fa, v0 + (v1 - v0) * fb)
            } else {
                (u0 + (u1 - u0) * fb, v0 + (v1 - v0) * fa)
            });
        }
    }
    println!("REVIEW loops: {} places for holes", centres.len());
    let mut times: Vec<(usize, f64)> = Vec::new();
    for n in [0_usize, 16, 64] {
        let mut body = base.clone();
        for &(u, v) in centres.iter().take(n) {
            let (p, su, sv) = surface.derivatives(u, v);
            let normal = su.cross(sv).normalized().unwrap();
            let frame = Frame::new(p, normal, su).unwrap();
            let circle = Curve3::Circle(Circle::new(frame, 0.1).unwrap());
            let first = circle.eval(0.0);
            let vx = body.add_vertex(first);
            let e = body.add_edge(circle, (0.0, std::f64::consts::TAU), vx, vx);
            let lp = body.add_loop(vec![Coedge {
                edge: e,
                reversed: false,
            }]);
            body.faces[fi].loops.push(lp);
        }
        let holes = body.faces[fi].loops.len() - 1;
        forget_folds();
        let (spans0, searches0) = craft3d_topo::fold_work();
        let start = Instant::now();
        let found = folds(&body, &tol(0.02));
        let cold = secs(start);
        let (spans1, searches1) = craft3d_topo::fold_work();
        let start = Instant::now();
        let again = folds(&body, &tol(0.02));
        let warm = secs(start);
        println!(
            "REVIEW loops: {holes} holes: cold {cold:.2} s, warm {warm:.4} s, spans {}, searches {}, {} places: {found:?}",
            spans1 - spans0,
            searches1 - searches0,
            found.len()
        );
        assert_eq!(places(&found), places(&again));
        times.push((holes, cold));
    }
    let (holes, last) = times[times.len() - 1];
    assert!(
        last < 10.0,
        "the rule took {last:.1} s on one valid face with {holes} holes ({times:?}): its work is not bounded by the face's bound"
    );
}

// ---------------------------------------------------------------------
// 2 and 3. Slots and a pleat, three scales, two tolerances; trims.

/// A slot drawn as one spline, its air between the walls: walls 0.5 t
/// and 0.9 t apart are two layers facing each other through air (shown);
/// 1.1 t and 2 t apart, with a round bottom of radius about half the gap,
/// are valid and must pass without any line. At sizes 1e-3, 1 and 1e6 and
/// at tolerances of 1e-2 and 1e-6 of the size.
#[test]
fn review_slots_at_three_scales_and_two_tolerances() {
    let mut failures: Vec<String> = Vec::new();
    for k in [1e-3, 1.0, 1e6] {
        for rel in [1e-2, 1e-6] {
            let t = rel * k;
            let l = 10.0 * k;
            for (what, r, fold) in [
                ("0.5 t", 0.25 * t, true),
                ("0.9 t", 0.45 * t, true),
                // By the definition a fold: drawn with control points on a
                // circle of 0.55 t, its walls narrow at the root to within
                // t (0.998 t at 1e-2 of the size, 0.981 t at 1e-6), facing
                // each other through air (taken in by fold-check, 2026-11-19; it was
                // `false`, "valid").
                ("1.1 t", 0.55 * t, true),
                ("2 t", t, false),
            ] {
                let body = facing(sheet(&slot(r, l), l, (0.0, 1.0)), 0.05, Vec3::X);
                forget_folds();
                let start = Instant::now();
                let found = folds(&body, &tol(t));
                let spent = secs(start);
                println!(
                    "REVIEW slot {what} x{k} t {t:e}: {} shown, {} places, {spent:.2} s; first: {}",
                    shown(&found),
                    found.len(),
                    found.first().map(ToString::to_string).unwrap_or_default()
                );
                if fold && shown(&found) == 0 {
                    failures.push(format!("slot {what} x{k} t {t:e}: missed: {found:?}"));
                }
                if !fold && !found.is_empty() {
                    failures.push(format!(
                        "slot {what} x{k} t {t:e}: a valid slot is said: {found:?}"
                    ));
                }
            }
        }
    }
    // Trims of the slot of 0.5 t at size 1: one wall left out (no fold
    // in the face), both walls in part (a fold).
    let t = 0.01;
    for (trim, fold) in [((0.0, 0.5), false), ((0.5, 1.0), false), ((0.2, 0.8), true)] {
        let probe = 0.5 * (trim.0 + trim.1);
        let body = sheet(&slot(0.25 * t, 10.0), 10.0, trim);
        // Outward into the slot: towards the slot's middle line x = 0 from
        // the side the probe lies on (at the bottom's middle, upwards).
        let p = body.faces[0].surface.eval(probe, 0.5);
        let towards = if p.x.abs() < 0.1 * t {
            Vec3::new(0.0, 1.0, 0.0)
        } else {
            Vec3::new(-p.x, 0.0, 0.0)
        };
        let body = facing(body, probe, towards);
        forget_folds();
        let found = folds(&body, &tol(t));
        println!(
            "REVIEW slot trimmed to {trim:?}: {} shown, {} places: {found:?}",
            shown(&found),
            found.len()
        );
        if fold && shown(&found) == 0 {
            failures.push(format!("slot trimmed to {trim:?}: missed: {found:?}"));
        }
        if !fold && shown(&found) > 0 {
            failures.push(format!(
                "slot trimmed to {trim:?}: a fold said where one layer is trimmed away: {found:?}"
            ));
        }
    }
    // A pleat (three layers 0.5 t apart): whichever its sense, one pair of
    // layers faces each other through air.
    for reversed in [false, true] {
        let mut body = sheet(&pleat(0.25 * t, 10.0), 10.0, (0.0, 1.0));
        body.faces[0].reversed = reversed;
        forget_folds();
        let found = folds(&body, &tol(t));
        println!(
            "REVIEW pleat reversed {reversed}: {} shown, {} places: {found:?}",
            shown(&found),
            found.len()
        );
        if shown(&found) == 0 {
            failures.push(format!("pleat reversed {reversed}: missed: {found:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------------------------------------------------------------------
// 2 and 5. A relief with noise near the tolerance.

/// A grid surface through 30 x 30 points 5 t apart, heights noise of one
/// and of three tolerances: a height field (no two normals more than a
/// right angle apart), valid; it must give no fold, and what it says as
/// not proven is printed.
#[test]
fn review_a_relief_with_noise_near_the_tolerance() {
    let t = 0.01;
    let mut failures: Vec<String> = Vec::new();
    for amplitude in [1.0, 3.0] {
        let step = 5.0 * t;
        let mut seed: u64 = 12345;
        let mut rows: Vec<Value> = Vec::new();
        for j in 0..30 {
            let mut row: Vec<Value> = Vec::new();
            for i in 0..30 {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let noise = ((seed >> 33) as f64 / (1_u64 << 31) as f64) * 2.0 - 1.0;
                row.push(json!([
                    i as f64 * step,
                    j as f64 * step,
                    amplitude * t * noise
                ]));
            }
            rows.push(Value::Array(row));
        }
        let bodies = script(
            &format!("rev-relief-{amplitude}"),
            t,
            &[(
                "feature.surface.grid",
                json!({"points": rows, "name": "Relief"}),
            )],
        );
        if bodies.is_empty() {
            failures.push(format!("relief {amplitude}: not built"));
        }
        for body in &bodies {
            forget_folds();
            let start = Instant::now();
            let found = folds(body, &tol(t));
            let spent = secs(start);
            println!(
                "REVIEW relief noise {amplitude} t: {} shown, {} places, {spent:.2} s; {:?}",
                shown(&found),
                found.len(),
                found.iter().map(ToString::to_string).collect::<Vec<_>>()
            );
            if shown(&found) > 0 {
                failures.push(format!(
                    "relief {amplitude}: a fold said on a height field: {found:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------------------------------------------------------------------
// 1. Hostile surfaces.

fn grid_surface(n: usize, degree: usize, at: impl Fn(usize, usize) -> Vec3) -> Surface {
    let mut net = Vec::with_capacity(n * n);
    for i in 0..n {
        for j in 0..n {
            net.push(at(i, j));
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(
            degree,
            degree,
            clamped(n, degree),
            clamped(n, degree),
            n,
            n,
            net,
            vec![1.0; n * n],
        )
        .unwrap(),
    )
}

/// The roof block with its spline top swapped for `surface` (its loops
/// then lie off it: the walk projects them).
fn swapped(surface: Surface) -> Body {
    let mut body = roof_block(18.0, 1.0, false, 3);
    body.faces[5].surface = surface;
    body
}

/// Never a panic, and each answered within a bound: a face drawn into one
/// point, the slot at 1e200 (derivatives' products overflow) and 1e-170
/// (they underflow), a surface of degree one of 100 489 spans (past the
/// rule's 65 536), one of 62 500 zigzag spans whose every knot line is a
/// crease, a surface with a pole inside (a row drawn into a point at a
/// knot of full multiplicity), and a surface closed both ways.
#[test]
fn review_hostile_surfaces_are_answered_within_a_bound() {
    let mut said: Vec<(String, usize, usize, f64)> = Vec::new();
    let mut run = |name: &str, body: &Body, t: f64| {
        forget_folds();
        let start = Instant::now();
        let found = folds(body, &tol(t));
        let spent = secs(start);
        println!(
            "REVIEW hostile {name}: {} places, {} shown, {spent:.2} s; first: {:?}",
            found.len(),
            shown(&found),
            found.first()
        );
        said.push((name.to_string(), found.len(), shown(&found), spent));
    };
    let point = Vec3::new(1.0, 1.0, 1.0);
    run(
        "one point",
        &swapped(grid_surface(4, 3, |_, _| point)),
        1e-3,
    );
    for k in [1e200, 1e-170] {
        let body = facing(
            sheet(&slot(0.25 * k, 10.0 * k), 10.0 * k, (0.0, 1.0)),
            0.05,
            Vec3::X,
        );
        run(&format!("slot x{k:e}"), &body, k);
    }
    let zig = |n: usize| {
        let s = 18.0 / (n - 1) as f64;
        move |i: usize, j: usize| {
            Vec3::new(
                i as f64 * s,
                j as f64 * s,
                6.0 + if i % 2 == 1 { 2.0 * s } else { 0.0 },
            )
        }
    };
    run(
        "degree 1, 100 489 spans",
        &swapped(grid_surface(318, 1, zig(318))),
        1e-3,
    );
    run(
        "degree 1, 62 500 creased spans",
        &swapped(grid_surface(251, 1, zig(251))),
        1e-3,
    );
    // A pole inside: knots of full multiplicity at 0.5, the middle row a point.
    let mut net = Vec::new();
    for i in 0..7 {
        for j in 0..7 {
            net.push(if i == 3 {
                Vec3::new(9.0, 9.0, 8.0)
            } else {
                Vec3::new(3.0 * i as f64, 3.0 * j as f64, 6.0)
            });
        }
    }
    let knots = vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0];
    let inside = Surface::Nurbs(
        NurbsSurface::new(3, 3, knots.clone(), knots, 7, 7, net, vec![1.0; 49]).unwrap(),
    );
    run("a pole inside", &swapped(inside), 1e-3);
    // Closed both ways: a torus of degree one, 8 x 8 panels.
    const C: [f64; 9] = [
        1.0,
        0.707_106_781_186_547_6,
        0.0,
        -0.707_106_781_186_547_6,
        -1.0,
        -0.707_106_781_186_547_6,
        0.0,
        0.707_106_781_186_547_6,
        1.0,
    ];
    const S: [f64; 9] = [
        0.0,
        0.707_106_781_186_547_6,
        1.0,
        0.707_106_781_186_547_6,
        0.0,
        -0.707_106_781_186_547_6,
        -1.0,
        -0.707_106_781_186_547_6,
        0.0,
    ];
    let torus = grid_surface(9, 1, |i, j| {
        let ring = 10.0 + 3.0 * C[j];
        Vec3::new(ring * C[i], ring * S[i], 3.0 * S[j])
    });
    run("a torus closed both ways", &swapped(torus), 1e-3);
    let mut failures = Vec::new();
    for (name, _, shown_n, spent) in &said {
        if *spent > 60.0 {
            failures.push(format!("{name}: {spent:.1} s"));
        }
        if (name == "a pole inside" || name == "a torus closed both ways") && *shown_n > 0 {
            failures.push(format!("{name}: a fold said"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A span of degree 16 or more is said "not proven" at its middle, one
/// line a span, with no test that the place is in the face: a surface of
/// degree 16 with 14 x 14 spans gives up to 196 lines for one face.
#[test]
fn review_spans_of_too_high_a_degree_say_one_line_each() {
    let n = 30;
    let body = swapped(grid_surface(n, 16, |i, j| {
        Vec3::new(i as f64, j as f64, 6.0 + ((i * 7 + j * 3) % 5) as f64 * 0.1)
    }));
    forget_folds();
    let found = folds(&body, &tol(1e-3));
    println!(
        "REVIEW degree 16, {} spans: {} places, {} shown; first: {:?}",
        (n - 16) * (n - 16),
        found.len(),
        shown(&found),
        found.first().map(ToString::to_string)
    );
    assert_eq!(shown(&found), 0);
    assert!(
        found.len() <= 1,
        "one face of degree 16 gives {} lines of foldsNotProven",
        found.len()
    );
}

// ---------------------------------------------------------------------
// 5. The words.

/// What an agent reads of `document.check` (its summary, the command's
/// description over MCP) names every warning kind and list it returns;
/// the guide's troubleshooting page says what to do with each. And each
/// place said of a shown fold lies on its face within half the tolerance.
#[test]
fn review_the_words_agents_read() {
    let mut failures: Vec<String> = Vec::new();
    let spec = craft3d_engine::Session::command("document.check").unwrap();
    for word in ["fold", "foldsNotProven"] {
        if !spec.summary.contains(word) {
            failures.push(format!(
                "document.check's summary does not name `{word}` (it names every other warning kind and `loose`)"
            ));
        }
    }
    let guide =
        std::fs::read_to_string(root().join("docs/guide/troubleshooting.md")).unwrap_or_default();
    if !guide.contains("folds over itself") && !guide.contains("foldsNotProven") {
        failures.push(
            "docs/guide/troubleshooting.md does not say what a fold warning means or what to do"
                .into(),
        );
    }
    for name in SEVEN {
        let body = fade(name);
        forget_folds();
        for f in folds(&body, &tol(0.02)) {
            let surface = &body.faces[f.face.0 as usize].surface;
            let off = surface
                .project(f.at)
                .map(|(u, v)| surface.eval(u, v).distance(f.at))
                .unwrap_or(f64::INFINITY);
            println!("REVIEW words {name}: {f} | {off:e} off its surface");
            if f.is_fold() && off > 0.5 * 0.02 + 1e-9 {
                failures.push(format!(
                    "{name}: the place said is {off} off its face's surface: {f}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------------------------------------------------------------------
// 2 and 3. The slots and the pleat without the pair cap.

/// The slots and the pleat of `review_slots_at_three_scales_and_two_tolerances`
/// (size 1, t = 0.01) asked through the copy of the rule with and without
/// its pair cap: without it, the slots 0.5 t and 0.9 t and the pleat are
/// shown and the slots 1.1 t and 2 t pass without a line, so that what
/// the rule says of them is the cap's.
#[test]
fn review_slots_without_the_pair_cap() {
    let t = 0.01;
    let mut failures: Vec<String> = Vec::new();
    let mut bodies: Vec<(String, Body, bool)> = Vec::new();
    for (what, r, fold) in [
        ("slot 0.5 t", 0.25 * t, true),
        ("slot 0.9 t", 0.45 * t, true),
        // By the definition a fold (see the test above).
        ("slot 1.1 t", 0.55 * t, true),
        ("slot 2 t", t, false),
    ] {
        let body = facing(sheet(&slot(r, 10.0), 10.0, (0.0, 1.0)), 0.05, Vec3::X);
        bodies.push((what.to_string(), body, fold));
    }
    for reversed in [false, true] {
        let mut body = sheet(&pleat(0.25 * t, 10.0), 10.0, (0.0, 1.0));
        body.faces[0].reversed = reversed;
        bodies.push((format!("pleat reversed {reversed}"), body, true));
    }
    for (what, body, fold) in &bodies {
        // The construction is a fold by the design's definition: two
        // points of the face at mid-depth, one on each wall (each outer
        // layer of the pleat), within t, their outward normals more than
        // 154° apart and air between them, taken the way the rule takes
        // them.
        let face = &body.faces[0];
        let sign = if face.reversed { -1.0 } else { 1.0 };
        let mut best: Option<(f64, f64, f64, f64, f64)> = None;
        for i in 0..=200 {
            for j in 0..=200 {
                let (a, b) = (f64::from(i) / 200.0, f64::from(j) / 200.0);
                if (a - b).abs() < 0.2 {
                    continue;
                }
                let (p, pu, pv) = face.surface.derivatives(a, 0.5);
                let (q, qu, qv) = face.surface.derivatives(b, 0.5);
                let (Some(m), Some(n)) = (pu.cross(pv).normalized(), qu.cross(qv).normalized())
                else {
                    continue;
                };
                let (m, n) = (m * sign, n * sign);
                let d = q - p;
                if d.length() < t && m.dot(n) < -0.9 && d.dot(m) > 0.0 && -d.dot(n) > 0.0 {
                    let gap = d.length();
                    if best.is_none_or(|x| gap < x.0) {
                        best = Some((gap, a, b, m.dot(n), d.dot(m)));
                    }
                }
            }
        }
        println!(
            "REVIEW uncapped {what}: by the definition, the closest facing pair through air at mid-depth (gap, u, u', cos, along): {best:?}"
        );
        if *fold && best.is_none() {
            failures.push(format!(
                "{what}: the construction is no fold by the definition"
            ));
        }
        let mut capped = mirror::Stats::new();
        let with = mirror::folds(body, t, 4_000_000, &mut capped);
        let mut free = mirror::Stats::new();
        let start = Instant::now();
        let without = mirror::folds(body, t, u64::MAX, &mut free);
        let spent = secs(start);
        let shown_free = without
            .iter()
            .filter(|(_, _, k)| !matches!(k, FoldKind::NotProven { .. }))
            .count();
        println!(
            "REVIEW uncapped {what}: capped {capped:?} says {} places",
            with.len()
        );
        println!(
            "REVIEW uncapped {what}: no cap ({spent:.2} s) {free:?} says {shown_free} shown of {}: {without:?}",
            without.len()
        );
        if *fold && shown_free == 0 {
            failures.push(format!(
                "{what}: not shown even without the cap: {without:?}"
            ));
        }
        if !*fold && !without.is_empty() {
            failures.push(format!("{what}: said even without the cap: {without:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------------------------------------------------------------------
// 7. The switch.

/// With `FOLDS_REFUSED` as built (off) this counts what fails; on the
/// reviewer's branch with it on (never merged), what `check` and the
/// regeneration of the repository's documents would refuse for a fold.
#[test]
fn review_switch_census() {
    let on = craft3d_topo::FOLDS_REFUSED;
    let mut refused_fades = 0;
    for name in SEVEN {
        // Read as `read_step` reads, the refusal kept: with the switch on
        // the reader's own check may refuse the fold.
        let bytes = std::fs::read(fixtures().join(format!("fades/{name}.step"))).unwrap();
        let options = craft3d_stepread::Options {
            metres_per_unit: 1e-3,
            tolerance: tol(0.02),
            loosest: 1000.0 * 0.02,
        };
        let body = match craft3d_stepread::parse(&bytes).and_then(|p| p.build(0, &options)) {
            Ok(read) => read.body,
            Err(e) => {
                let said = format!("{e:?}");
                println!(
                    "REVIEW switch {on} {name}: refused as it is read: {}",
                    said.chars().take(400).collect::<String>()
                );
                refused_fades += 1;
                continue;
            }
        };
        let defects = check(&body, &tol(0.02));
        let folded = defects
            .iter()
            .filter(|d| matches!(d, Defect::FaceFolds { .. }))
            .count();
        println!(
            "REVIEW switch {on} {name}: {} defects, {folded} folds: {}",
            defects.len(),
            defects.first().map(ToString::to_string).unwrap_or_default()
        );
        if folded > 0 {
            refused_fades += 1;
        }
    }
    let t = 0.01;
    for (what, r) in [("slot 0.5 t", 0.25 * t), ("slot 2 t", t)] {
        let body = facing(sheet(&slot(r, 10.0), 10.0, (0.0, 1.0)), 0.05, Vec3::X);
        let body = if body.faces[0].reversed {
            turned(body)
        } else {
            body
        };
        let defects = check(&body, &tol(t));
        println!(
            "REVIEW switch {on} {what}: {:?}",
            defects.iter().map(ToString::to_string).collect::<Vec<_>>()
        );
    }
    let folders = [
        "docs/examples/published",
        "docs/showcase",
        "crates/engine/tests/fillet_fade",
        "crates/engine/tests/stale_sketches",
        "crates/format/tests/fixtures/versions",
        "crates/format/tests/fixtures/migrations",
    ];
    let (mut documents, mut failed, mut for_folds) = (0, 0, 0);
    let start = Instant::now();
    for folder in folders {
        let Ok(list) = std::fs::read_dir(root().join(folder)) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = list
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "3dcraft"))
            .collect();
        paths.sort();
        for path in paths {
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(doc) = craft3d_format::from_bytes(&bytes) else {
                continue;
            };
            documents += 1;
            let regen = craft3d_regen::regenerate(&doc);
            for f in &regen.features {
                if let craft3d_regen::Status::Failed(why) = &f.status {
                    failed += 1;
                    if why.contains("folds over itself") {
                        for_folds += 1;
                        println!("REVIEW switch {on} {}: {why}", path.display());
                    }
                }
            }
        }
    }
    println!(
        "REVIEW switch {on}: {documents} documents in {:.1} s, {failed} failed features, {for_folds} for a fold; {refused_fades} of the seven fades refused by check",
        secs(start)
    );
    // An import of a fold.
    let dir = common::workdir("fold-review", "switch");
    std::fs::copy(
        fixtures().join("fades/fade-4-8-0.5.step"),
        dir.join("fold.step"),
    )
    .unwrap();
    let mut s = common::session_in(&dir);
    s.execute("file.new", &json!({"units": "mm", "tolerance": 0.02}))
        .unwrap();
    let imported = s.execute("file.import", &json!({"path": "fold.step"}));
    println!(
        "REVIEW switch {on} import: {}",
        match &imported {
            Ok(v) => v.to_string().chars().take(400).collect::<String>(),
            Err(e) => e.to_string(),
        }
    );
    if on {
        assert!(refused_fades >= 6, "{refused_fades} of the seven refused");
        assert_eq!(for_folds, 0, "documents refused for a fold");
    }
}
