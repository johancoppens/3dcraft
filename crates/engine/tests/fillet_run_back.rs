//! A fillet of one edge alone whose sections across the edge run back along
//! it, built along the balls' own path (2026-11-25; docs/design/blends.md ›
//! *Sections that run back along the edge* › *Built along the balls'
//! path*).
//!
//! Where an edge bends more tightly than the fillet reaches from it, the
//! balls in the planes square to the edge come out of order: those planes
//! meet in the edge's axis of curvature, and a ball wholly past it moves
//! back along the edge as the plane moves on. The construction in place
//! (`rolling::blend_open`) takes its sections in those planes, so its face
//! would fold over itself, and it refuses (`rolling::folds`, `Fold::Along`).
//! The balls themselves go on: their centres run along the curve where the
//! two faces moved in by the radius meet, and the fillet is the envelope of
//! the balls, touched by each along its great circle square to that curve.
//! That is how a chain's fillets are built (`chain::along_spine`, since
//! 2026-10-14): the balls walked along their path, the sections square to
//! a cubic through their centres. A lone open edge refused so is now built
//! that way, as a chain of one piece (`rolling::along_the_path`,
//! `chain::build_alone`), held to what the construction in place holds its
//! own face to (tangent to both faces along borders that stay on them, no
//! fold across or along over what is kept, no loop left behind).
//!
//! *Oracles*, besides the deep check:
//!
//! - **The volume.** What the fillet takes away (20791978, a convex edge)
//!   or adds (20821184, a concave one), worked out outside the code from
//!   the primitives' parameters alone (`plan/run-back-fillets/
//!   volume_oracle.py`): the centre curve in closed form (20791978: the
//!   plane moved in by r and the cone moved in by r) or by Newton's method
//!   (20821184: the torus's tube and the cylinder, each shrunk by r), and
//!   in each plane square to it the region between the ball's arc and the
//!   two faces, out to where the ray from the centre leaves them, cut by
//!   the third faces at the ends, integrated with the volume element of
//!   the map from (place on the curve, angle, distance) to space:
//!   Gauss–Legendre between the places where a ray changes the face it
//!   leaves by, adaptive Gauss–Kronrod along the curve. It gives the closed
//!   forms of a cylinder's rim, convex and concave, and of the part of one
//!   beyond a plane to 1e-10. Held to the document's tolerance times
//!   π r (L + 4 r), more than the fillet face's area (L the edge's length):
//!   whole, and beyond planes across the stretch where the sections run
//!   back. It also gives what the construction in place builds below the
//!   threshold.
//! - **The met-edge twin `main` builds.** The same edge filleted together
//!   with an edge that meets it at an end goes through the chain's
//!   construction on `main`, at these radii too (the review of
//!   check/nightly-findings): beyond the meet's reach both bodies hold the
//!   same volume below a plane, to the same bound.

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

/// The steps with every length (positions, heights, radii, distances, a
/// torus's radii) times `k`.
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

/// Random design 20791978 as shrunk (scale 1e-3, tolerance 1e-9): a cone
/// whose base is moved in by an eighth of its radius, intersected with a
/// cylinder lying across it; the edge where the cylinder's base (a plane
/// 0.0025 from the cone's axis) meets the cone's side is a branch of a
/// hyperbola whose vertex lies near the edge's end on the cylinder's wall.
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

/// 20791978's edge: the cylinder's base and the cone's side, from V4 (on
/// the cone's moved base) to V3 (on the cylinder's wall).
const HYPERBOLA: [&str; 2] = ["F4:copy(0)/F3:face(1)", "F4:copy(1)/F1:face(0)"];
const V4: [f64; 3] = [
    0.003_940_172_215_153_641,
    0.005_000_866_131_745_651,
    0.011_285_714_610_281_633,
];
const V3: [f64; 3] = [
    0.010_131_725_720_842_899,
    0.004_882_915_687_564_786,
    0.016_889_205_522_678_474,
];
const HYPERBOLA_LENGTH: f64 = 0.008_535_946_092_545_02;

/// The edges that meet the hyperbola at V4: the line where the cylinder's
/// base meets the cone's base, and the circle where the cone's side does.
const AT_V4: [[&str; 2]; 2] = [
    ["F4:copy(0)/F3:face(1)", "F4:copy(1)/F1:face(1)"],
    ["F4:copy(1)/F1:face(0)", "F4:copy(1)/F1:face(1)"],
];

/// Random design 20821184 as shrunk (scale 1e-3, tolerance 1e-9): a ball
/// less the part of a torus inside a cylinder, which leaves a cavity; the
/// cavity's rim where the torus meets the cylinder's wall runs from the
/// cylinder's base round and back to it.
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
const RIM_LENGTH: f64 = 0.014_855_480_215_256_631;

/// The edges that meet the rim at both its ends, in the cylinder's base:
/// where the torus meets it, and where the cylinder's wall does.
const AT_THE_BASE: [[&str; 2]; 2] = [
    ["F5:copy(1)/F3:face(0)", "F5:copy(1,1)/F1:face(1)"],
    ["F5:copy(1,1)/F1:face(0)", "F5:copy(1,1)/F1:face(1)"],
];

/// What the fillet of 20791978's edge alone takes away, by the oracle
/// (`volume_oracle.py s1 r`): below the threshold (0.00054418) by the
/// construction in place, above it along the balls' path.
const TAKEN: [(f64, f64); 9] = [
    (0.0005, 1.874_961_488_614_432_5e-9),
    (0.00054, 2.196_459_223_959_779_7e-9),
    (0.000_544_1, 2.230_896_573_928_363_5e-9),
    (0.000_544_2, 2.231_739_959_945_775_2e-9),
    (0.00055, 2.280_937_392_884_474e-9),
    (0.0006, 2.727_984_966_840_424e-9),
    (0.000_625, 2.966_924_302_897_688e-9),
    (0.0008, 4.925_988_261_812_911e-9),
    (0.001, 7.764_814_636_452_032e-9),
];

/// The same beyond the planes across the chord from V4 to V3 at 0.65 and
/// 0.8 of it (the side of V3, which holds the stretch where the sections
/// run back: from 0.836 of the edge by angle at 0.000625): (radius, place,
/// volume).
const TAKEN_BEYOND: [(f64, f64, f64); 4] = [
    (0.00055, 0.65, 1.199_321_338_549_748_4e-9),
    (0.00055, 0.8, 7.211_603_898_250_815e-10),
    (0.000_625, 0.65, 1.537_452_992_927_323_3e-9),
    (0.000_625, 0.8, 8.941_344_856_368_57e-10),
];

/// What the fillet of 20821184's rim alone adds (`volume_oracle.py s2 r`;
/// the threshold 0.00045825).
const ADDED: [(f64, f64); 7] = [
    (0.0004, 2.195_216_487_567_656_5e-9),
    (0.00045, 2.781_257_420_696_016e-9),
    (0.000_458_2, 2.883_877_089_577_366_4e-9),
    (0.000_458_4, 2.886_402_787_249_353e-9),
    (0.00046, 2.906_647_372_806_952_4e-9),
    (0.0005, 3.435_185_994_040_966e-9),
    (0.000_625, 5.357_889_812_796_923e-9),
];

/// The same above the cylinder's level 0.002 (which holds the top of the
/// stretch where the sections run back, 0.076 to 0.332 of the rim by angle
/// at 0.000625, and none of a fillet in the cylinder's base): (radius,
/// volume).
const ADDED_ABOVE: [(f64, f64); 3] = [
    (0.00046, 2.516_276_088_341_397_5e-9),
    (0.0005, 2.961_152_361_021_792_7e-9),
    (0.000_625, 4.545_854_424_473_309_4e-9),
];

/// The bound a fillet's volume is held to: the document's tolerance times
/// π r (L + 4 r), more than the fillet face's area (its arc is shorter
/// than half a turn, and it runs at most two radii past each end).
fn bound(tolerance: f64, radius: f64, length: f64) -> f64 {
    tolerance * PI * radius * (length + 4.0 * radius)
}

fn volume(s: &mut Session, feature: &str) -> f64 {
    let m = s
        .execute("document.measure", &json!({"feature": feature}))
        .unwrap();
    m["volume"].as_f64().unwrap()
}

fn below(s: &mut Session, feature: &str, plane: &Value) -> f64 {
    let m = s
        .execute("measure.below", &json!({"of": feature, "plane": plane}))
        .unwrap();
    m["volume"]
        .as_f64()
        .unwrap_or_else(|| panic!("below {plane}: {m}"))
}

/// Fillets `edges` by `radius` as `R`, which has to be built and pass the
/// deep check, with no fold among its warnings.
#[track_caller]
fn filleted(s: &mut Session, edges: &Value, radius: f64) {
    let made = s
        .execute(
            "feature.fillet",
            &json!({"chain": false, "edges": edges, "radius": radius, "name": "R"}),
        )
        .unwrap();
    assert_eq!(made["status"], "ok", "{edges} at {radius}: {made}");
    deeply_valid(s, &format!("{edges} at {radius}"));
}

/// The deep check passes and warns of no fold: a face that folds over
/// itself is a warning of `document.check`, never part of its `ok`.
#[track_caller]
fn deeply_valid(s: &mut Session, what: &str) {
    let check = s.execute("document.check", &json!({"deep": true})).unwrap();
    assert_eq!(check["ok"], true, "{what}: {check}");
    let folds: Vec<&Value> = check["warnings"]
        .as_array()
        .map(|w| w.iter().filter(|w| w["kind"] == "fold").collect())
        .unwrap_or_default();
    assert!(folds.is_empty(), "{what}: a fold: {folds:?}");
}

/// The plane across the chord from V4 to V3 at `at` of it, below it the
/// side of V3.
fn beyond(at: f64) -> Value {
    let n: Vec<f64> = (0..3).map(|i| V3[i] - V4[i]).collect();
    let p: Vec<f64> = (0..3).map(|i| V4[i] + at * n[i]).collect();
    json!({"point": p, "normal": [-n[0], -n[1], -n[2]]})
}

/// The level `h` along 20821184's cylinder, below it the side above `h`.
fn above(h: f64) -> Value {
    let a = 1.0 / 3.0_f64.sqrt();
    json!({"point": [0.01 + h * a, 0.01 + h * a, 0.0075 + h * a], "normal": [-a, -a, -a]})
}

#[track_caller]
fn near(got: f64, want: f64, within: f64, what: &str) {
    assert!(
        (got - want).abs() <= within,
        "{what}: {got:e} against {want:e}, off by {:e} (allowed {within:e})",
        got - want
    );
}

/// 20791978's edge alone at radii either side of where its sections first
/// run back (0.00054418): what the fillet takes away is the oracle's, whole
/// and beyond the planes across the stretch where they run back, and the
/// body passes the deep check. On check/nightly-findings the radii from
/// 0.0005442 up were refused (*"… fold over itself …"*), and on `main`
/// before it failed from 0.000555 with *"internal error … not tangent …"*.
#[test]
fn seed_20791978_alone_takes_away_what_the_rolling_ball_does() {
    for (radius, oracle) in TAKEN {
        let mut s = session(1e-9, &seed_20791978());
        let before = volume(&mut s, "N4");
        filleted(&mut s, &json!([HYPERBOLA]), radius);
        let taken = before - volume(&mut s, "R");
        let within = bound(1e-9, radius, HYPERBOLA_LENGTH);
        near(taken, oracle, within, &format!("radius {radius}"));
    }
    for (radius, at, oracle) in TAKEN_BEYOND {
        let mut s = session(1e-9, &seed_20791978());
        let before = below(&mut s, "N4", &beyond(at));
        filleted(&mut s, &json!([HYPERBOLA]), radius);
        let taken = before - below(&mut s, "R", &beyond(at));
        let within = bound(1e-9, radius, HYPERBOLA_LENGTH);
        near(
            taken,
            oracle,
            within,
            &format!("radius {radius} beyond {at}"),
        );
    }
}

/// 20821184's rim alone either side of where its sections first run back
/// (0.00045825): what the fillet adds is the oracle's, whole and above the
/// level 0.002, and the body passes the deep check. Refused from 0.0004584
/// on check/nightly-findings; on `main` before it refused by the check
/// that the body does not pass through itself at 0.00046 and 0.00047, and
/// an internal error from 0.00048.
#[test]
fn seed_20821184_alone_adds_what_the_rolling_ball_does() {
    for (radius, oracle) in ADDED {
        let mut s = session(1e-9, &seed_20821184());
        let before = volume(&mut s, "N6");
        filleted(&mut s, &json!([RIM]), radius);
        let added = volume(&mut s, "R") - before;
        let within = bound(1e-9, radius, RIM_LENGTH);
        near(added, oracle, within, &format!("radius {radius}"));
    }
    for (radius, oracle) in ADDED_ABOVE {
        let mut s = session(1e-9, &seed_20821184());
        let before = below(&mut s, "N6", &above(0.002));
        filleted(&mut s, &json!([RIM]), radius);
        let added = below(&mut s, "R", &above(0.002)) - before;
        let within = bound(1e-9, radius, RIM_LENGTH);
        near(
            added,
            oracle,
            within,
            &format!("radius {radius} above 0.002"),
        );
    }
}

/// The twin `main` builds: the same edge with an edge that meets it at an
/// end, through the chain's construction. Beyond the meet's reach (the
/// side of V3 past 0.65 of 20791978's chord; above the level 0.002 of
/// 20821184's cylinder, whose rim meets its neighbours at both ends) the
/// lone fillet holds the twin's volume.
#[test]
fn the_lone_fillet_is_its_met_twin_beyond_the_meet() {
    for radius in [0.00055, 0.0006, 0.000_625] {
        let mut lone = session(1e-9, &seed_20791978());
        filleted(&mut lone, &json!([HYPERBOLA]), radius);
        for other in AT_V4 {
            let mut twin = session(1e-9, &seed_20791978());
            filleted(&mut twin, &json!([HYPERBOLA, other]), radius);
            for at in [0.65, 0.8] {
                near(
                    below(&mut lone, "R", &beyond(at)),
                    below(&mut twin, "R", &beyond(at)),
                    bound(1e-9, radius, HYPERBOLA_LENGTH),
                    &format!("20791978 with {other:?} at {radius}, beyond {at}"),
                );
            }
        }
    }
    for radius in [0.00046, 0.0005, 0.000_625] {
        let mut lone = session(1e-9, &seed_20821184());
        filleted(&mut lone, &json!([RIM]), radius);
        for other in AT_THE_BASE {
            let mut twin = session(1e-9, &seed_20821184());
            filleted(&mut twin, &json!([RIM, other]), radius);
            near(
                below(&mut lone, "R", &above(0.002)),
                below(&mut twin, "R", &above(0.002)),
                bound(1e-9, radius, RIM_LENGTH),
                &format!("20821184 with {other:?} at {radius}, above 0.002"),
            );
        }
    }
}

/// The two faces of the edge of the body `T` (turned whole by a
/// transform, which keeps its topology) whose faces have the short names
/// `short`.
fn turned_edge(s: &mut Session, short: &Value) -> Value {
    let sorted = |v: &Value| {
        let mut names: Vec<String> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_string())
            .collect();
        names.sort();
        names
    };
    let listed = s
        .execute("document.edges", &json!({"feature": "T"}))
        .unwrap();
    let found: Vec<&Value> = listed["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| sorted(&e["short"]) == sorted(short))
        .collect();
    let [one] = found.as_slice() else {
        panic!("not one edge of T by {short}: {listed}");
    };
    one["faces"].clone()
}

/// Both seeds at their own radius, 0.000625, at the scales 1e-3 (their
/// own) to 1e6 with the tolerance alike, as they are and turned whole by
/// 37° about (1, 2, 3): built, deeply valid, and the volume taken away or
/// added the oracle's times the scale cubed.
#[test]
fn at_every_scale_and_turned() {
    // (Both at 0.000625.)
    let (taken, added) = (TAKEN[6].1, ADDED[6].1);
    for k in [1.0, 1e3, 1e6, 1e9] {
        for (steps, body, edge, length, oracle, sign) in [
            (
                seed_20791978(),
                "N4",
                HYPERBOLA,
                HYPERBOLA_LENGTH,
                taken,
                -1.0,
            ),
            (seed_20821184(), "N6", RIM, RIM_LENGTH, added, 1.0),
        ] {
            let steps = scaled(&steps, k);
            for turn in [false, true] {
                let mut s = session(1e-9 * k, &steps);
                let (target, edges) = if turn {
                    let listed = s
                        .execute("document.edges", &json!({"feature": body, "between": edge}))
                        .unwrap();
                    let short = listed["edges"][0]["short"].clone();
                    let made = s
                        .execute(
                            "feature.transform",
                            &json!({"source": body, "angle": "37 deg", "axis": [1.0, 2.0, 3.0], "pivot": [0.3 * k, -0.2 * k, 0.1 * k], "name": "T"}),
                        )
                        .unwrap();
                    assert_eq!(made["status"], "ok", "{made}");
                    ("T", turned_edge(&mut s, &short))
                } else {
                    (body, json!(edge))
                };
                let before = volume(&mut s, target);
                filleted(&mut s, &json!([edges]), 0.000_625 * k);
                let change = sign * (volume(&mut s, "R") - before) / (k * k * k);
                near(
                    change,
                    oracle,
                    bound(1e-9, 0.000_625, length),
                    &format!("{body} at {k}, turned {turn}"),
                );
            }
        }
    }
}

/// A closed edge whose sections run back is not handed to the balls' path
/// (its ring is taken from a first pass in the planes across the edge):
/// refused, in words that say it is not built yet. A rod of radius 5 cut
/// by a plane 75° from its axis: the rim is an ellipse (a = 5 / cos 75°,
/// b = 5) whose curvature radius at its upper end is b² / a = 1.2941, where
/// the faces meet at 15°; the ball's centre lies r / sin 7.5° from the edge
/// along the bisector, 0.9914 of it towards the ellipse's centre of
/// curvature, so its sections start to run back from r = 1.2941 sin 7.5° /
/// 0.9914 = 0.1704 (worked out by hand). Built at 0.15, refused at 0.2. The
/// fillet exists there (the balls' centres lie on an ellipse whose
/// curvature radius is 1.24 at least): owed. The words are what is pinned
/// (a regression test); the bracket by the closed form.
#[test]
fn a_closed_edge_whose_sections_run_back_is_not_built_yet() {
    let rod = |radius: f64| -> Value {
        let mut s = session(1e-6, &[]);
        let steps = [
            (
                "feature.cylinder",
                json!({"radius": 5.0, "height": 40.0, "name": "Rod"}),
            ),
            (
                "feature.planeCut",
                json!({"name": "Cut", "target": "Rod", "planes": [{"normal": [2.0 + 3.0_f64.sqrt(), 0.0, 1.0], "point": [0.0, 0.0, 20.0], "side": "below"}]}),
            ),
        ];
        for (id, params) in steps {
            let made = s.execute(id, &params).unwrap();
            assert_eq!(made["status"], "ok", "{id}: {made}");
        }
        let rim = json!([["F2:copy(0)/F1:face(0)", "F2:cut(1)"]]);
        let made = s
            .execute(
                "feature.fillet",
                &json!({"edges": rim, "radius": radius, "name": "R"}),
            )
            .unwrap();
        if made["status"] == "ok" {
            deeply_valid(&mut s, &format!("radius {radius}"));
        }
        made
    };
    assert_eq!(rod(0.15)["status"], "ok");
    let made = rod(0.2);
    let text = made["error"].as_str().unwrap_or_default();
    assert!(!text.contains(craft3d_math::BUG), "{made}");
    // (Until round 2 of 2026-11-25 the words ended "… is not built yet
    // for a lone edge: give a smaller radius", which read as if no lone
    // edge were built, and were said too where the path was tried.)
    for words in [
        "the rolling ball's sections across the edge (each in the plane square to the edge) run back along it",
        "is built along an open edge of one radius whose borders stay on its two faces, not yet along this one: give a smaller radius",
    ] {
        assert!(text.contains(words), "{made}");
    }
}
