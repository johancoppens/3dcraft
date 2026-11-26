//! The review of the lone fillet built along the balls' own path
//! (check/run-back-fillets, 2026-11-25; docs/design/blends.md › *Sections
//! that run back along the edge* › *Built along the balls' path*).
//!
//! The builder's oracle is a volume: a fold in the face could leave the
//! volume within its bound and the deep check's `ok` too (a fold is a
//! warning of `document.check`, never part of `ok`). So the face itself is
//! held to the definition of the rolling-ball fillet, from the primitives
//! alone, at places chosen evenly along each edge and densely where the
//! sections square to the edge run back:
//!
//! - **The envelope of the balls.** The fillet of a radius r is the envelope
//!   of the balls of radius r that touch both faces: their centres run
//!   along the curve where the two faces moved in by r meet, and each ball
//!   touches the fillet face along its great circle in the plane square to
//!   that curve, between its two feet. So from the ball's centre every ray
//!   in that plane between the feet meets the body first at the fillet face,
//!   at exactly r. The centres, the feet's directions and the curve's tangent
//!   (`fillet_run_back_review/rays.txt`) are worked out outside the code
//!   (`plan/run-back-fillets/review/scripts/rays.py`: the centre curve of
//!   `volume_oracle.py` for 20791978 and 20821184, and the review's own for
//!   random design 20813467, `geometry_f.py`), only where the ball's centre
//!   and the rays to its arc lie inside the third faces by a margin.
//! - **Parity.** A line from far outside the body through a point inside
//!   the ball near the fillet meets faces that alternate between going in
//!   and coming out, and as many before the point as the point's side says
//!   (inside the material for a convex edge, outside for a concave one). A
//!   face that folds over itself is met where it should not be.
//! - **No fold warning** in the deep check.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;

type Steps = Vec<(&'static str, Value)>;
type V = [f64; 3];

/// The rows of `rays.txt`: per seed and radius, places along the edge.
const RAYS: &str = include_str!("fillet_run_back_review/rays.txt");

/// Where across the arc (from the foot on the first face to the foot on
/// the second) the rays from the centre are cast.
const ACROSS: [f64; 5] = [0.05, 0.25, 0.5, 0.75, 0.95];

struct Row {
    seed: String,
    radius: f64,
    frac: f64,
    centre: V,
    a: V,
    b: V,
    tangent: V,
}

fn rows() -> Vec<Row> {
    RAYS.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            assert_eq!(w.len(), 15, "{l}");
            let n: Vec<f64> = w[1..].iter().map(|x| x.parse().unwrap()).collect();
            Row {
                seed: w[0].to_string(),
                radius: n[0],
                frac: n[1],
                centre: [n[2], n[3], n[4]],
                a: [n[5], n[6], n[7]],
                b: [n[8], n[9], n[10]],
                tangent: [n[11], n[12], n[13]],
            }
        })
        .collect()
}

fn add(p: V, q: V) -> V {
    [p[0] + q[0], p[1] + q[1], p[2] + q[2]]
}

fn times(p: V, k: f64) -> V {
    [p[0] * k, p[1] * k, p[2] * k]
}

fn unit(p: V) -> V {
    let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    times(p, 1.0 / l)
}

/// A new document at `tolerance`, and `steps` run on it, each built.
fn session(tolerance: f64, steps: &[(&str, Value)]) -> Session {
    let mut s = Session::new(Workspace::unrestricted());
    s.execute("file.new", &json!({"units": "mm", "tolerance": tolerance}))
        .unwrap();
    for (id, params) in steps {
        let made = s.execute(id, params).unwrap();
        assert_ne!(made["status"], "failed", "{id} {params}: {made}");
    }
    s
}

/// The steps with every length times `k` (as `fillet_run_back.rs`).
fn scaled(steps: &[(&'static str, Value)], k: f64) -> Steps {
    steps
        .iter()
        .map(|(id, p)| {
            let mut p = p.clone();
            let map = p.as_object_mut().unwrap();
            for key in ["height", "radius", "distance", "major", "minor"] {
                if let Some(v) = map.get_mut(key) {
                    *v = json!(v.as_f64().unwrap() * k);
                }
            }
            if let Some(v) = map.get_mut("position") {
                let a: Vec<f64> = v
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_f64().unwrap() * k)
                    .collect();
                *v = json!(a);
            }
            (*id, p)
        })
        .collect()
}

/// Random design 20791978 as shrunk (as `fillet_run_back.rs`).
fn seed_20791978() -> Steps {
    vec![
        (
            "feature.cone",
            json!({"angle": "0 deg", "axis": [0.01, 0.01, 1.0], "height": 0.01, "name": "N0", "position": [0.01, 0.0025, 0.01], "radius": 0.0075}),
        ),
        (
            "feature.offsetFace",
            json!({"distance": -0.00125, "faces": ["F1:face(1)"], "name": "N1"}),
        ),
        (
            "feature.cylinder",
            json!({"angle": "0 deg", "axis": [0.01, 1.0, 0.01], "height": 0.01, "name": "N3", "position": [0.000_312_499_999_999_999_84, 0.005, 0.015], "radius": 0.01}),
        ),
        (
            "feature.boolean",
            json!({"keepTool": false, "name": "N4", "op": "intersect", "target": "N3", "tool": "N1"}),
        ),
    ]
}

const HYPERBOLA: [&str; 2] = ["F4:copy(0)/F3:face(1)", "F4:copy(1)/F1:face(0)"];
/// The finding's other edge (its two-edge fillet).
const FINDING_OTHER: [&str; 2] = ["F4:copy(0)/F3:face(0)", "F4:copy(1)/F1:face(1)"];
const V4: V = [
    0.003_940_172_215_153_641,
    0.005_000_866_131_745_651,
    0.011_285_714_610_281_633,
];
const V3: V = [
    0.010_131_725_720_842_899,
    0.004_882_915_687_564_786,
    0.016_889_205_522_678_474,
];

/// Random design 20821184 as shrunk (as `fillet_run_back.rs`).
fn seed_20821184() -> Steps {
    vec![
        (
            "feature.cylinder",
            json!({"angle": "90 deg", "axis": [1.0, 1.0, 1.0], "height": 0.01, "name": "N0", "position": [0.01, 0.01, 0.0075], "radius": 0.005}),
        ),
        (
            "feature.sphere",
            json!({"name": "N2", "position": [0.015, 0.011_108_439_182_435_162, 0.01], "radius": 0.01}),
        ),
        (
            "feature.torus",
            json!({"angle": "0 deg", "axis": [0.01, 0.01, -1.0], "major": 0.0075, "minor": 0.005, "name": "N3", "position": [0.0075, 0.015, 0.001_464_466_094_067_261_6]}),
        ),
        (
            "feature.boolean",
            json!({"keepTool": false, "name": "N5", "op": "intersect", "target": "N3", "tool": "N0"}),
        ),
        (
            "feature.boolean",
            json!({"keepTool": false, "name": "N6", "op": "subtract", "target": "N2", "tool": "N5"}),
        ),
    ]
}

const RIM: [&str; 2] = ["F5:copy(1)/F3:face(0)", "F5:copy(1,1)/F1:face(0)"];

/// Random design 20813467 to its second group (Heavy 45651602987's
/// record, `plan/run-back-fillets/fuzz-45651602987/`): a torus intersected
/// with a ring of square section turned about the line through (0, -5, 0)
/// along x (from 7.5 to 12.5 of it, x from 2.5 to 7.5).
fn seed_20813467() -> Steps {
    vec![
        (
            "feature.torus",
            json!({"angle": "0 deg", "axis": [0.0, 0.0, 1.0], "major": 12.5, "minor": 5.0, "name": "N0", "position": [-10.0, 2.5, -7.5]}),
        ),
        (
            "sketch.create",
            json!({"name": "S1", "offset": -5.0, "plane": "zx"}),
        ),
        (
            "sketch.polyline",
            json!({"closed": true, "points": [[7.5, 2.5], [12.5, 2.5], [12.5, 7.5], [7.5, 7.5]], "sketch": "S1"}),
        ),
        (
            "feature.revolve",
            json!({"angle": "360 deg", "axis": "y", "intersect": "N0", "name": "N1", "sketch": "S1"}),
        ),
    ]
}

/// Its fillet's two faces: the torus and the plane x = 2.5 (the face the
/// first segment of the square sweeps).
const OVAL: [&str; 2] = ["F4:copy(0)/F1:face(0)", "F4:copy(1)/F3:side(e5)"];

/// What that fillet (radius 1.25, both pieces of the oval inside the ring)
/// takes away, from the primitives alone (`volume_f.py`: the review's
/// geometry, the builder's integrator), and the length of the two pieces.
///
/// An expectation changed in round 2 (2026-11-26,
/// `plan/run-back-fillets/round-2/`): 19.360_362_356_560_22 until then,
/// short by 6.7e-5. The integrator looks across each ball's arc through 65
/// rays and counts nothing in a section none of them reaches; where the
/// second piece's region begins, at the ring's inner wall, it is reached
/// through a fan of rays that opens from nothing (the balls' angle 3.07910
/// to 3.08069, 6.58e-5 of volume, `oracle_miss.py`), and a little more is
/// left out so at the piece's end and the first piece's ends. Mended there
/// (a section the 65 rays miss looked at again through 4097,
/// `volume_f_fixed.py`). Held independently of both: the body is the
/// rolling ball's (the points within r of the inner torus cut by the moved
/// plane) along 51 260 lines through the whole second piece, every crossing
/// within 1e-5 (Check 45723944987), and its volume is the same by a mesh
/// with each triangle's bulge to its surface and by a quadrature over each
/// fillet face's own parameters (Check 45721917460).
const OVAL_TAKEN: f64 = 19.360_429_027_549_77;
const OVAL_LENGTH: f64 = 15.689_250_133_769;

/// The fillet of `edges` by `radius` as `R`: `Err` with the refusal's
/// words where it is not built.
fn fillet(s: &mut Session, edges: &Value, radius: f64) -> Result<(), String> {
    let made = s
        .execute(
            "feature.fillet",
            &json!({"chain": false, "edges": edges, "radius": radius, "name": "R"}),
        )
        .unwrap();
    if made["status"] == "failed" {
        return Err(made["error"].as_str().unwrap_or_default().to_string());
    }
    Ok(())
}

/// The deep check passes and warns of no fold; what it could not settle is
/// printed.
#[track_caller]
fn deeply_valid(s: &mut Session, what: &str) {
    let check = s.execute("document.check", &json!({"deep": true})).unwrap();
    assert_eq!(check["ok"], true, "{what}: {check}");
    let folds: Vec<&Value> = check["warnings"]
        .as_array()
        .map(|w| w.iter().filter(|w| w["kind"] == "fold").collect())
        .unwrap_or_default();
    assert!(folds.is_empty(), "{what}: a fold: {folds:?}");
    if let Some(open) = check.get("foldsNotProven") {
        println!("{what}: folds not proven: {open}");
    }
}

fn hits(s: &mut Session, from: V, direction: V) -> Vec<Value> {
    let found = s
        .execute(
            "measure.ray",
            &json!({"from": from, "direction": direction, "limit": 1000}),
        )
        .unwrap();
    found["hits"].as_array().unwrap().clone()
}

/// What the rays of `rows` found on the body of `s`, the fillet `R` of
/// radius `r` (the rows' radius times `k`, the document's tolerance
/// `tol`): the largest |distance − r| over the tolerance, and the parity
/// rays skipped for meeting an edge or grazing a face.
#[track_caller]
fn envelope(
    s: &mut Session,
    rows: &[&Row],
    k: f64,
    tol: f64,
    convex: bool,
    reach: f64,
    what: &str,
) -> (f64, usize) {
    let mut worst: f64 = 0.0;
    let mut skipped = 0;
    for row in rows {
        let r = row.radius * k;
        let c = times(row.centre, k);
        for t in ACROSS {
            let w = unit(add(times(row.a, 1.0 - t), times(row.b, t)));
            let found = hits(s, c, w);
            let first = found.first().unwrap_or_else(|| {
                panic!("{what} at {}: the ray across {t} meets nothing", row.frac)
            });
            let face = first["face"].as_str().unwrap_or_default();
            let d = first["distance"].as_f64().unwrap();
            assert!(
                face.contains(":fillet("),
                "{what} at {} across {t}: first met {face} at {d:e} (r {r:e}): {first}",
                row.frac
            );
            assert_eq!(
                first["enters"].as_bool(),
                Some(!convex),
                "{what} at {} across {t}: {first}",
                row.frac
            );
            let off = (d - r).abs() / tol;
            assert!(
                off <= 1.0,
                "{what} at {} across {t}: the fillet at {d:e} from the ball's centre, r {r:e}: {off:.3} tolerances off",
                row.frac
            );
            worst = worst.max(off);
        }
        // Lines from far outside through a point inside the ball.
        let mid = unit(add(row.a, row.b));
        let inside = add(c, times(mid, 0.5 * r));
        for direction in [
            row.tangent,
            times(row.tangent, -1.0),
            times(mid, -1.0),
            unit(add(row.tangent, times(mid, -1.0))),
            unit(add(times(row.tangent, -1.0), times(mid, -1.0))),
        ] {
            let from = add(inside, times(direction, -reach));
            let found = hits(s, from, direction);
            if found.iter().any(|h| {
                h["edge"] == true || h["tangent"] == true || h["enters"].as_bool().is_none()
            }) {
                skipped += 1;
                continue;
            }
            for (i, h) in found.iter().enumerate() {
                assert_eq!(
                    h["enters"].as_bool(),
                    Some(i % 2 == 0),
                    "{what} at {}: the line along {direction:?} meets faces out of turn: {found:?}",
                    row.frac
                );
            }
            assert_eq!(found.len() % 2, 0, "{what} at {}: {found:?}", row.frac);
            let before = found
                .iter()
                .filter(|h| h["distance"].as_f64().unwrap() < reach)
                .count();
            assert_eq!(
                before % 2 == 1,
                convex,
                "{what} at {}: the point inside the ball is on the wrong side: {found:?}",
                row.frac
            );
        }
    }
    (worst, skipped)
}

/// Built and deeply valid, each fillet is the envelope of its balls at
/// every place looked at: 20791978's edge alone below the threshold (by
/// the construction in place) and above it (along the balls' path), at
/// 0.000625 also with the finding's other edge and at a million times the
/// scale; 20821184's rim alike; 20813467's oval. The radii 0.0011 and
/// 0.00065 are probes: refused, in words that are not a bug's, or built and
/// held to the same.
#[test]
fn each_fillet_is_the_envelope_of_its_balls() {
    let all = rows();
    let mut groups: Vec<(String, f64)> = Vec::new();
    for row in &all {
        if !groups
            .iter()
            .any(|(s, r)| *s == row.seed && *r == row.radius)
        {
            groups.push((row.seed.clone(), row.radius));
        }
    }
    for (seed, radius) in groups {
        let mine: Vec<&Row> = all
            .iter()
            .filter(|r| r.seed == seed && r.radius == radius)
            .collect();
        let probe = [0.0011, 0.00065].contains(&radius);
        let mut cases: Vec<(f64, Value, &str)> = Vec::new();
        let (steps, edge, convex, size, base_tol): (Steps, [&str; 2], bool, f64, f64) =
            match seed.as_str() {
                "s1" => (seed_20791978(), HYPERBOLA, true, 0.05, 1e-9),
                "s2" => (seed_20821184(), RIM, false, 0.05, 1e-9),
                _ => (seed_20813467(), OVAL, true, 100.0, 1e-6),
            };
        if seed == "f1" {
            // The same fillet as f0's: one build for both pieces.
            continue;
        }
        cases.push((1.0, json!([edge]), "alone"));
        if (seed == "s1" || seed == "s2") && radius == 0.000_625 {
            cases.push((1e6, json!([edge]), "alone, a million times the scale"));
        }
        if seed == "s1" && radius == 0.000_625 {
            cases.push((
                1.0,
                json!([edge, FINDING_OTHER]),
                "with the finding's other edge",
            ));
        }
        let rows_here: Vec<&Row> = if seed == "f0" {
            all.iter().filter(|r| r.seed.starts_with('f')).collect()
        } else {
            mine
        };
        for (k, edges, how) in cases {
            let what = format!("{seed} radius {radius} {how}");
            let tol = base_tol * k;
            let mut s = session(tol, &scaled(&steps, k));
            if let Err(text) = fillet(&mut s, &edges, radius * k) {
                assert!(probe, "{what}: refused: {text}");
                assert!(!text.contains(craft3d_math::BUG), "{what}: {text}");
                println!("{what}: refused: {text}");
                continue;
            }
            deeply_valid(&mut s, &what);
            let (worst, skipped) = envelope(&mut s, &rows_here, k, tol, convex, size * k, &what);
            println!(
                "{what}: built; {} places, the fillet within {worst:.4} tolerances of r from the balls' centres; {skipped} parity lines skipped (an edge or a graze)",
                rows_here.len()
            );
        }
    }
}

/// 20813467's fillet (radius 1.25 along both pieces of the oval where the
/// torus meets the plane x = 2.5 inside the ring), built on this branch
/// where `main` refused it: what it takes away is the volume worked out
/// from the primitives alone, to the tolerance times π r (L + 8 r) (two
/// edges), and the edges are the two pieces worked out (their ends on the
/// plane, the length).
#[test]
fn seed_20813467_takes_away_what_the_rolling_ball_does() {
    let mut s = session(1e-6, &seed_20813467());
    let listed = s
        .execute("document.edges", &json!({"feature": "N1", "between": OVAL}))
        .unwrap();
    println!("the oval's edges: {listed}");
    let edges = listed["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2, "{listed}");
    let length: f64 = edges.iter().map(|e| e["length"].as_f64().unwrap()).sum();
    // (The listed length is a polygon through 64 points.)
    assert!(
        (length - OVAL_LENGTH).abs() <= 1e-3,
        "{length} against {OVAL_LENGTH}"
    );
    let before = volume(&mut s, "N1");
    fillet(&mut s, &json!([OVAL]), 1.25).unwrap();
    deeply_valid(&mut s, "20813467");
    let taken = before - volume(&mut s, "R");
    let within = 1e-6 * PI * 1.25 * (OVAL_LENGTH + 8.0 * 1.25);
    println!(
        "20813467: takes away {taken:.15e}, the oracle {OVAL_TAKEN:.15e}, off by {:e} (allowed {within:e})",
        taken - OVAL_TAKEN
    );
    assert!((taken - OVAL_TAKEN).abs() <= within);
}

/// What 20813467's fillet takes away from each piece's side of the plane
/// z = -5.2 (which passes between the two: the region of the first lies
/// above z = -4.19, of the second below z = -6.05, `zrange_f.py` and
/// round 2's `footprint.py`), by the same oracle, mended (`OVAL_TAKEN`;
/// until round 2 3.193_603_062_684_45 and 16.166_759_293_875_77): (above,
/// below), and the pieces' lengths.
const OVAL_TAKEN_PIECES: [f64; 2] = [3.193_603_181_884_467, 16.166_825_845_665_3];
const OVAL_PIECE_LENGTHS: [f64; 2] = [5.162_080_210_603, 10.527_169_923_166];

/// The same below planes through the oval's middle line (along x through
/// (y, z) = (2.5, -7.5)) at the angle θ about it, the normal (0, -sin θ,
/// cos θ) (`volume_f2.py`): (θ, the piece, what it takes away below). The
/// first piece's region lies at the angles 0.66 to 1.87 about that line,
/// the second's at 3.04 to 4.27 (`angle_f.py`), so below the planes at 3.1,
/// 3.25 and 3.6 lie the whole first piece and the start of the second
/// (its tip, where its sections square to the edge run back), and below
/// the plane at 1.2 a part of the first only: that plane crosses the
/// fillet the construction in place builds, and so says how near
/// `measure.below` comes where a plane crosses a fitted face. By the
/// mended oracle (`OVAL_TAKEN`, `volume_f_fixed.py`; until round 2
/// 0.623_670_602_495_068_6, 4.954_334_102_042_039, 12.669_255_403_920_065
/// and 1.880_399_949_827_951).
const OVAL_TAKEN_BELOW: [(f64, usize, f64); 4] = [
    (3.1, 1, 0.623_743_086_908_511_9),
    (3.25, 1, 4.954_400_707_740_358),
    (3.6, 1, 12.669_322_056_263_95),
    (1.2, 0, 1.880_400_048_322_651),
];

/// 20813467's fillet at the tolerances 1e-5, 1e-6 and 1e-7: what it takes
/// away from each piece's side, against the oracle, each to the tolerance
/// times π r (L + 4 r) of its piece; and below the planes of
/// `OVAL_TAKEN_BELOW` (printed). Where the difference falls with the
/// tolerance, the kernel and the oracle measure the same set.
///
/// Against the review's first oracle it failed (Checks 45695333898,
/// 45708714656, 45709384218): the second piece, built along the balls'
/// path, took away 6.2e-5 more at 1e-5 and 6.5e-5 more at 1e-6, not falling
/// with the tolerance (1.07 of its bound at 1e-6), all of it where the piece
/// starts at the ring's inner wall, while every ray found the face and its
/// end cut where the rolling ball puts them. The oracle's: its 65 rays
/// across each ball's arc left out where the region begins (`OVAL_TAKEN`).
/// Against the mended oracle (round 2), the first piece -6.9e-7 and +6.1e-8,
/// the second -4.2e-6 and -1.3e-6 (0.007 and 0.022 of the bound) at 1e-5
/// and 1e-6; the excess against the first oracle lay wholly before the
/// section square to the balls' path at 3.11 (round 2's planes). At 1e-7
/// the fillet is refused.
#[test]
fn seed_20813467_each_piece_at_three_tolerances() {
    let side = |above: bool| {
        let n = if above { -1.0 } else { 1.0 };
        json!({"point": [0.0, 0.0, -5.2], "normal": [0.0, 0.0, n]})
    };
    let turned = |theta: f64| {
        // (No trigonometry in tests: the normal's two numbers by the
        // half-angle formulas from the tangent of half the angle.)
        let t = craft3d_math::Portable::ptan(0.5 * theta);
        let (sin, cos) = (2.0 * t / (1.0 + t * t), (1.0 - t * t) / (1.0 + t * t));
        json!({"point": [0.0, 2.5, -7.5], "normal": [0.0, -sin, cos]})
    };
    let mut failed = Vec::new();
    for tol in [1e-5, 1e-6, 1e-7] {
        let mut s = session(tol, &seed_20813467());
        let before_planes: Vec<f64> = OVAL_TAKEN_BELOW
            .iter()
            .map(|(theta, _, _)| below(&mut s, "N1", &turned(*theta)))
            .collect();
        let before = [
            below(&mut s, "N1", &side(true)),
            below(&mut s, "N1", &side(false)),
        ];
        let whole = volume(&mut s, "N1");
        // (At 1e-7 refused: "the fillet face along this edge could not be
        // approximated within the tolerance", Check 45708714656.)
        if let Err(text) = fillet(&mut s, &json!([OVAL]), 1.25) {
            assert!(!text.contains(craft3d_math::BUG), "{tol}: {text}");
            println!("20813467 at {tol}: refused: {text}");
            continue;
        }
        deeply_valid(&mut s, &format!("20813467 at {tol}"));
        let taken = whole - volume(&mut s, "R");
        println!(
            "20813467 at {tol}: whole {taken:.15e}, off by {:e} ({:.3} tol pi r (L + 8 r))",
            taken - OVAL_TAKEN,
            (taken - OVAL_TAKEN) / (tol * PI * 1.25 * (OVAL_LENGTH + 8.0 * 1.25))
        );
        let mut worst: f64 = 0.0;
        for (i, above) in [true, false].into_iter().enumerate() {
            let piece = before[i] - below(&mut s, "R", &side(above));
            let off = piece - OVAL_TAKEN_PIECES[i];
            let bound = tol * PI * 1.25 * (OVAL_PIECE_LENGTHS[i] + 4.0 * 1.25);
            println!(
                "20813467 at {tol}: piece {i} takes away {piece:.15e}, the oracle {:.15e}, off by {off:e} ({:.3} of tol pi r (L + 4 r))",
                OVAL_TAKEN_PIECES[i],
                off / bound
            );
            worst = worst.max(off.abs() / bound);
        }
        for ((theta, piece, oracle), before) in OVAL_TAKEN_BELOW.iter().zip(&before_planes) {
            let taken = before - below(&mut s, "R", &turned(*theta));
            // Below the planes across the second piece, the whole first.
            let oracle = if *piece == 1 {
                oracle + OVAL_TAKEN_PIECES[0]
            } else {
                *oracle
            };
            println!(
                "20813467 at {tol}: below the plane at {theta} takes away {taken:.15e}, the oracle {oracle:.15e}, off by {:e}",
                taken - oracle
            );
        }
        if worst > 1.0 {
            failed.push((tol, worst));
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}

fn below(s: &mut Session, feature: &str, plane: &Value) -> f64 {
    let m = s
        .execute("measure.below", &json!({"of": feature, "plane": plane}))
        .unwrap();
    m["volume"]
        .as_f64()
        .unwrap_or_else(|| panic!("below {plane}: {m}"))
}

/// Short rays onto the fillet where 20813467's second piece starts (the
/// oval's tip, the ring's inner wall, where the sections square to the
/// edge run back: 0.087 to 0.203 of the piece by its angle,
/// `runback_f.py`), from the primitives alone (`surface_rays.py`): from a
/// point h beyond a point of the ball's arc that lies inside the third
/// faces (in what the fillet takes away) back towards it, in through the
/// fillet at h; from a point h inside the ball out towards it, out through
/// the fillet at h.
const SURFACE: &str = include_str!("fillet_run_back_review/surface.txt");

#[test]
fn seed_20813467_where_its_second_piece_starts_the_fillet_is_the_balls() {
    let mut s = session(1e-6, &seed_20813467());
    fillet(&mut s, &json!([OVAL]), 1.25).unwrap();
    let mut worst: f64 = 0.0;
    let mut count = 0;
    let mut wrong = Vec::new();
    for l in SURFACE.lines().filter(|l| !l.trim().is_empty()) {
        let w: Vec<&str> = l.split_whitespace().collect();
        assert_eq!(w.len(), 11, "{l}");
        let n: Vec<f64> = w[1..10].iter().map(|x| x.parse().unwrap()).collect();
        let (from, direction, h) = ([n[2], n[3], n[4]], [n[5], n[6], n[7]], n[8]);
        let enters = w[10] == "1";
        let found = hits(&mut s, from, direction);
        let at = format!("{} theta {} across {}", w[0], w[1], w[2]);
        count += 1;
        let Some(first) = found.first() else {
            println!("{at}: meets nothing");
            wrong.push(at);
            continue;
        };
        let face = first["face"].as_str().unwrap_or_default();
        let d = first["distance"].as_f64().unwrap();
        let off = (d - h).abs() / 1e-6;
        let right =
            face.contains(":fillet(") && first["enters"].as_bool() == Some(enters) && off <= 1.0;
        println!(
            "{at}: {face} at {d:e} ({off:.4} tolerances from h), enters {}",
            first["enters"]
        );
        if right {
            worst = worst.max(off);
        } else {
            wrong.push(at);
        }
    }
    println!(
        "20813467 where its second piece starts: {count} rays, {} wrong, the right within {worst:.4} tolerances",
        wrong.len()
    );
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// A probe (prints, compared outside the code, `lines_f.py`): lines along x
/// through where 20813467's second piece starts (y from -10.4 to -5.0, z
/// from -7.8 to -4.8), each with the places it crosses the filleted body's
/// faces, at the tolerance 1e-6.
#[test]
#[ignore = "a probe of the review (Check 45708714656 and after): prints, compared by lines_f.py"]
fn seed_20813467_lines_through_where_its_second_piece_starts() {
    let mut s = session(1e-6, &seed_20813467());
    fillet(&mut s, &json!([OVAL]), 1.25).unwrap();
    for i in 0..46 {
        for j in 0..31 {
            let (y, z) = (-10.4 + 0.12 * f64::from(i), -7.8 + 0.1 * f64::from(j));
            let found = hits(&mut s, [0.0, y, z], [1.0, 0.0, 0.0]);
            let listed: Vec<String> = found
                .iter()
                .map(|h| {
                    format!(
                        "{:.12} {} {} {} {}",
                        h["distance"].as_f64().unwrap(),
                        h["enters"],
                        h["edge"],
                        h["tangent"],
                        h["face"].as_str().unwrap_or_default()
                    )
                })
                .collect();
            println!("LINE {i} {j} {y:.4} {z:.4} | {}", listed.join(" ; "));
        }
    }
}

/// The rows of `ends.txt`: places near each end of an edge, the fillet run
/// on past it and cut by the third face there.
const ENDS: &str = include_str!("fillet_run_back_review/ends.txt");

/// Where across the arc `ends.txt` holds its rays.
const ACROSS_ENDS: [f64; 4] = [0.1, 0.37, 0.63, 0.9];

/// At the ends the fillet is cut where it crosses the third face: from the
/// ball's centre, a ray towards a point of its arc inside the third faces
/// leaves the body through the fillet at r (`F`), one towards a point
/// beyond them through the third face where the ray crosses it (`T`); from
/// a centre past the end (outside the body) the ray goes in through the
/// third face and out through the fillet at r (`E`). Worked out from the
/// primitives alone (`ends.py`), held to the tolerance.
#[test]
fn the_fillet_is_cut_where_the_third_face_crosses_it() {
    struct End {
        seed: String,
        radius: f64,
        frac: f64,
        centre: V,
        a: V,
        b: V,
        rays: Vec<(String, f64)>,
    }
    let rows: Vec<End> = ENDS
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            assert_eq!(w.len(), 20, "{l}");
            let n: Vec<f64> = w[1..12].iter().map(|x| x.parse().unwrap()).collect();
            End {
                seed: w[0].to_string(),
                radius: n[0],
                frac: n[1],
                centre: [n[2], n[3], n[4]],
                a: [n[5], n[6], n[7]],
                b: [n[8], n[9], n[10]],
                rays: (0..4)
                    .map(|k| (w[12 + 2 * k].to_string(), w[13 + 2 * k].parse().unwrap()))
                    .collect(),
            }
        })
        .collect();
    let mut groups: Vec<(String, f64)> = Vec::new();
    for row in &rows {
        let seed = if row.seed.starts_with('f') {
            "f".to_string()
        } else {
            row.seed.clone()
        };
        if !groups.iter().any(|(s, r)| *s == seed && *r == row.radius) {
            groups.push((seed, row.radius));
        }
    }
    for (seed, radius) in groups {
        let (steps, edge, convex, tol): (Steps, [&str; 2], bool, f64) = match seed.as_str() {
            "s1" => (seed_20791978(), HYPERBOLA, true, 1e-9),
            "s2" => (seed_20821184(), RIM, false, 1e-9),
            _ => (seed_20813467(), OVAL, true, 1e-6),
        };
        let what = format!("{seed} radius {radius}");
        let mut s = session(tol, &steps);
        fillet(&mut s, &json!([edge]), radius).unwrap();
        let mut counted = [0_usize; 3];
        let mut worst: f64 = 0.0;
        for row in rows
            .iter()
            .filter(|r| r.seed.starts_with(&seed) && r.radius == radius)
        {
            for (t, (kind, want)) in ACROSS_ENDS.into_iter().zip(&row.rays) {
                if kind == "-" {
                    continue;
                }
                let w = unit(add(times(row.a, 1.0 - t), times(row.b, t)));
                let found = hits(&mut s, row.centre, w);
                let at = format!("{what} at {} across {t} ({kind} {want:e})", row.frac);
                let is_fillet =
                    |h: &Value| h["face"].as_str().unwrap_or_default().contains(":fillet(");
                let distance = |h: &Value| h["distance"].as_f64().unwrap();
                let first = found
                    .first()
                    .unwrap_or_else(|| panic!("{at}: meets nothing"));
                let apart = match kind.as_str() {
                    "F" | "T" => {
                        assert_eq!(is_fillet(first), kind == "F", "{at}: {found:?}");
                        assert_eq!(first["enters"].as_bool(), Some(!convex), "{at}: {first}");
                        counted[usize::from(kind == "T")] += 1;
                        (distance(first) - want).abs()
                    }
                    _ => {
                        // In through the third face, out through the fillet.
                        let second = found.get(1).unwrap_or_else(|| panic!("{at}: {found:?}"));
                        assert!(!is_fillet(first), "{at}: {found:?}");
                        assert_eq!(first["enters"].as_bool(), Some(true), "{at}: {first}");
                        assert!(is_fillet(second), "{at}: {found:?}");
                        assert_eq!(second["enters"].as_bool(), Some(false), "{at}: {second}");
                        counted[2] += 1;
                        (distance(first) - want)
                            .abs()
                            .max((distance(second) - radius).abs())
                    }
                };
                let off = apart / tol;
                assert!(off <= 1.0, "{at}: {off:.3} tolerances off: {found:?}");
                worst = worst.max(off);
            }
        }
        println!(
            "{what}: {} rays to the fillet, {} to the third face, {} in through it and out through the fillet; within {worst:.4} tolerances",
            counted[0], counted[1], counted[2]
        );
    }
}

fn volume(s: &mut Session, feature: &str) -> f64 {
    let m = s
        .execute("document.measure", &json!({"feature": feature}))
        .unwrap();
    m["volume"].as_f64().unwrap()
}

/// A fillet whose radius fades is not handed to the balls' path (the
/// claim: the law's own check of a face that turns back on itself comes
/// first): 20791978's edge at 0.000625 fading to nothing over 0.003 at V4
/// is refused so (the stretch where the sections run back, at V3, keeps
/// the whole radius); fading at V3 instead it builds by the construction in
/// place (Check 45684274145), deeply valid. Never a bug's words.
#[test]
fn a_fade_along_an_edge_whose_sections_run_back() {
    for (end, label) in [(V4, "V4"), (V3, "V3")] {
        let mut s = session(1e-9, &seed_20791978());
        let listed = s
            .execute(
                "document.edges",
                &json!({"feature": "N4", "between": HYPERBOLA}),
            )
            .unwrap();
        let vertices = listed["vertices"].as_array().unwrap();
        let near = |v: &Value| {
            let p: Vec<f64> = v["point"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect();
            (0..3)
                .map(|i| (p[i] - end[i]) * (p[i] - end[i]))
                .sum::<f64>()
        };
        let vertex = vertices
            .iter()
            .min_by(|a, b| near(a).total_cmp(&near(b)))
            .unwrap();
        assert!(near(vertex) < 1e-16, "{label}: {listed}");
        let made = s
            .execute(
                "feature.fillet",
                &json!({"edges": [HYPERBOLA], "radius": 0.000_625, "fade": [{"at": vertex["name"], "over": "0.003 mm"}], "name": "R"}),
            )
            .unwrap();
        println!("the fade at {label}: {made}");
        if made["status"] == "failed" {
            let text = made["error"].as_str().unwrap_or_default();
            assert!(!text.contains(craft3d_math::BUG), "{label}: {text}");
            assert!(
                text.contains("would turn back on itself"),
                "{label}: {text}"
            );
        } else {
            deeply_valid(&mut s, &format!("the fade at {label}"));
        }
    }
}
