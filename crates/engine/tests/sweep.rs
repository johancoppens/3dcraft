//! Sweeps through the commands: a profile in one sketch, a path in
//! another, `feature.sweep`, and what the document says of the result.
//! Every solid is measured against a closed form at three scales and
//! checked deep (Pappus for the bends, a prism cut by the bisecting plane
//! for the mitres; `crates/kernel/tests/sweep.rs` has the same forms
//! against the kernel alone, and the union of the pieces as a second
//! oracle); the rest is held to identities: a boolean's set identities, a
//! file that reopens to the same bodies, an edit that regenerates to what
//! a full regeneration gives (which `run` asserts after every command).
//!
//! The profiles are drawn on the zx plane, whose sketch axes are the
//! world's z and x and which faces +y; the paths on the xy plane, from
//! the origin along +y. A point of the profile at sketch (u, v) is then
//! `u` above the path's plane and `v` to the right of the path.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{
    deep, failure, feature_brief as feature, inspect, refused_with as refused, run,
};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::path::PathBuf;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

type P = [f64; 2];

fn workdir(name: &str) -> PathBuf {
    common::workdir("sweep", name)
}

fn new() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    s
}

fn scaled(points: &[P], k: f64) -> Vec<P> {
    points.iter().map(|p| [p[0] * k, p[1] * k]).collect()
}

/// A sketch `name` on the zx plane with a circle: its centre is e1, the
/// circle e2.
fn round(s: &mut Session, name: &str, center: P, radius: f64) {
    run(s, "sketch.create", json!({"plane": "zx", "name": name}));
    run(
        s,
        "sketch.circle",
        json!({"sketch": name, "center": center, "radius": radius}),
    );
}

/// A sketch `name` on the zx plane with a closed polygon of four points:
/// they are e1..e4, its sides e5..e8.
fn quad(s: &mut Session, name: &str, points: &[P]) {
    run(s, "sketch.create", json!({"plane": "zx", "name": name}));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": name, "closed": true, "points": points}),
    );
}

/// The rectangle from `u.0` to `u.1` above the path's plane and from
/// `v.0` to `v.1` to the right of the path. Its sides e5 and e7 are
/// parallel to the path's plane, e6 and e8 square to it.
fn rectangle(s: &mut Session, name: &str, u: P, v: P) {
    quad(
        s,
        name,
        &[[u[0], v[0]], [u[0], v[1]], [u[1], v[1]], [u[1], v[0]]],
    );
}

/// A sketch `name` on the xy plane with a path from the origin; returns
/// the ids of its curves.
fn path(s: &mut Session, name: &str, segments: Value, closed: bool) -> Vec<String> {
    run(s, "sketch.create", json!({"plane": "xy", "name": name}));
    let made = run(
        s,
        "sketch.path",
        json!({"sketch": name, "start": [0, 0], "segments": segments, "closed": closed}),
    );
    made["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap().to_string())
        .collect()
}

/// 10 along +y, a quarter bend of radius 5 to the left, 12 along −x.
fn bent(s: &mut Session, name: &str, k: f64) -> Vec<String> {
    path(
        s,
        name,
        json!([
            {"line": [0.0, 10.0 * k]},
            {"arc": [-5.0 * k, 15.0 * k], "center": [-5.0 * k, 10.0 * k]},
            {"line": [-17.0 * k, 15.0 * k]},
        ]),
        false,
    )
}

/// What a point `right` of the path travels along `bent`.
fn bent_length(right: f64) -> f64 {
    10.0 + (5.0 + right) * FRAC_PI_2 + 12.0
}

fn body(s: &mut Session, name: &str) -> Value {
    let document = inspect(s);
    document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name}: {}", document["features"]))
        .clone()
}

fn volume(s: &mut Session, name: &str) -> f64 {
    body(s, name)["volume"].as_f64().unwrap()
}

fn area(s: &mut Session, name: &str) -> f64 {
    body(s, name)["area"].as_f64().unwrap()
}

fn face_names(s: &mut Session, name: &str) -> Vec<String> {
    body(s, name)["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

#[track_caller]
fn close(found: f64, expected: f64, relative: f64) {
    assert!(
        (found - expected).abs() <= relative * expected.abs(),
        "found {found}, expected {expected} (off by {:e})",
        (found - expected).abs() / expected.abs()
    );
}

fn sweep(s: &mut Session, sketch: &str, along: &str, name: &str) -> Value {
    run(
        s,
        "feature.sweep",
        json!({"sketch": sketch, "path": along, "name": name}),
    )
}

#[test]
fn a_pipe_and_a_bar_along_line_bend_line_have_their_closed_forms_at_every_scale() {
    for k in SCALES {
        // A round pipe: a cylinder, a piece of a torus, a cylinder.
        let mut s = new();
        round(&mut s, "Section", [0.0, 0.0], k);
        let curves = bent(&mut s, "Route", k);
        assert_eq!(curves, ["e3", "e6", "e8"]);
        let made = sweep(&mut s, "Section", "Route", "Pipe");
        assert_eq!(made["status"], "ok", "{made}");
        assert_eq!(made["path"]["closed"], false);
        assert_eq!(made["path"]["deviation"], 0.0);
        let length = bent_length(0.0) * k;
        close(made["path"]["length"].as_f64().unwrap(), length, 1e-13);
        close(volume(&mut s, "Pipe"), PI * k * k * length, 1e-12);
        close(area(&mut s, "Pipe"), TAU * k * length + TAU * k * k, 1e-12);
        assert_eq!(
            face_names(&mut s, "Pipe"),
            [
                "F3:side(e2,e3)",
                "F3:side(e2,e6)",
                "F3:side(e2,e8)",
                "F3:startCap(e2)",
                "F3:endCap(e2)"
            ]
        );
        assert_eq!(feature(&mut s, "Pipe")["path"]["stations"], 0);
        deep(&mut s);

        // A bar 2 by 3, off the path: 1 to 3 to its right.
        let mut s = new();
        rectangle(&mut s, "Section", [-k, k], [k, 4.0 * k]);
        bent(&mut s, "Route", k);
        let made = sweep(&mut s, "Section", "Route", "Bar");
        assert_eq!(made["status"], "ok", "{made}");
        close(
            volume(&mut s, "Bar"),
            6.0 * bent_length(2.5) * k.powi(3),
            1e-12,
        );
        // The sides square to the path's plane (2 long, at 1 and at 4),
        // the two parallel to it (3 wide, their middles at 2.5), the ends.
        let sides =
            2.0 * (bent_length(1.0) + bent_length(4.0)) + 2.0 * 3.0 * bent_length(2.5) + 12.0;
        close(area(&mut s, "Bar"), sides * k * k, 1e-12);
        // The sides parallel to the path's plane are one face each along
        // the whole path, named after the profile's line alone.
        let names = face_names(&mut s, "Bar");
        assert_eq!(names.len(), 10, "{names:?}");
        for flat in ["F3:side(e5)", "F3:side(e7)"] {
            assert_eq!(names.iter().filter(|n| *n == flat).count(), 1, "{names:?}");
        }
        for piece in ["F3:side(e6,e3)", "F3:side(e6,e6)", "F3:side(e8,e8)"] {
            assert!(names.contains(&piece.to_string()), "{names:?}");
        }
        // One can sketch on the flat face that follows the whole path.
        let on_top = run(
            &mut s,
            "sketch.create",
            json!({"face": "F3:side(e7)", "name": "OnTop"}),
        );
        assert_eq!(on_top["status"], "ok", "{on_top}");
        deep(&mut s);
    }
}

/// A leaf of two splines between (−2, 0) and (2, 0), the same to either
/// side of the path: its centre is on the path. Its points are e1..e5 and
/// e7, e8; its splines e6 and e9.
fn leaf(s: &mut Session, name: &str, k: f64) {
    run(s, "sketch.create", json!({"plane": "zx", "name": name}));
    let lower = scaled(
        &[
            [-2.0, 0.0],
            [-1.0, -1.0],
            [0.0, -1.3],
            [1.0, -1.0],
            [2.0, 0.0],
        ],
        k,
    );
    run(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": lower}),
    );
    let upper = scaled(&[[1.0, 1.0], [0.0, 1.3], [-1.0, 1.0]], k);
    run(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": ["e5", upper[0], upper[1], upper[2], "e1"]}),
    );
}

#[test]
fn a_profile_of_splines_follows_pappus_along_the_bend() {
    for k in SCALES {
        let mut s = new();
        leaf(&mut s, "Section", k);
        // The area of the leaf: what a slab of it one unit thick holds.
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Section", "distance": k, "name": "Slab"}),
        );
        let leaf_area = volume(&mut s, "Slab") / k;
        run(&mut s, "feature.delete", json!({"feature": "Slab"}));
        bent(&mut s, "Route", k);
        let made = sweep(&mut s, "Section", "Route", "Blade");
        assert_eq!(made["status"], "ok", "{made}");
        close(
            volume(&mut s, "Blade"),
            leaf_area * bent_length(0.0) * k,
            1e-10,
        );
        let names = face_names(&mut s, "Blade");
        // Each spline once per piece of the path, and the two ends.
        assert_eq!(names.len(), 8, "{names:?}");
        deep(&mut s);
    }
}

#[test]
fn a_corner_of_the_path_is_mitred_at_90_60_and_120_degrees() {
    for k in SCALES {
        for degrees in [90.0_f64, 60.0, 120.0] {
            let turn = degrees.to_radians();
            let corner = json!([
                {"line": [0.0, 10.0 * k]},
                {"line": [-8.0 * k * turn.sin(), (10.0 + 8.0 * turn.cos()) * k]},
            ]);
            // What a line of the profile `right` of the path travels: the
            // two legs, and what the mitre gives it on the outside of the
            // corner (the path turns left).
            let travel = |right: f64| 18.0 + 2.0 * right * (turn / 2.0).tan();

            // A rectangle 1 by 1.5: the two prisms cut by the bisecting
            // plane.
            let mut s = new();
            rectangle(&mut s, "Section", [-0.5 * k, 0.5 * k], [-0.5 * k, k]);
            path(&mut s, "Route", corner.clone(), false);
            let made = sweep(&mut s, "Section", "Route", "Bar");
            assert_eq!(made["status"], "ok", "{degrees}° at {k}: {made}");
            close(volume(&mut s, "Bar"), 1.5 * travel(0.25) * k.powi(3), 1e-12);
            let sides = 2.0 * 1.5 * travel(0.25) + travel(-0.5) + travel(1.0) + 3.0;
            close(area(&mut s, "Bar"), sides * k * k, 1e-12);
            assert_eq!(face_names(&mut s, "Bar").len(), 8);
            deep(&mut s);

            // A rod: two cylinders cut by the bisecting plane, each half
            // the difference of its longest and shortest line longer
            // than its shortest: its length along its axis.
            let mut s = new();
            round(&mut s, "Section", [0.0, 0.25 * k], 0.5 * k);
            path(&mut s, "Route", corner, false);
            let made = sweep(&mut s, "Section", "Route", "Rod");
            assert_eq!(made["status"], "ok", "{degrees}° at {k}: {made}");
            close(
                volume(&mut s, "Rod"),
                PI * 0.25 * travel(0.25) * k.powi(3),
                1e-12,
            );
            close(
                area(&mut s, "Rod"),
                (PI * travel(0.25) + PI * 0.5) * k * k,
                1e-12,
            );
            assert_eq!(
                face_names(&mut s, "Rod"),
                [
                    "F3:side(e2,e3)",
                    "F3:side(e2,e5)",
                    "F3:startCap(e2)",
                    "F3:endCap(e2)"
                ]
            );
            deep(&mut s);
        }
    }
}

/// A rectangle 12 by 20 with corners rounded at 3, walked from the middle
/// of its right side: counter-clockwise, so its inside is to the left.
fn rounded_rectangle(s: &mut Session, name: &str, k: f64) -> Vec<String> {
    run(s, "sketch.create", json!({"plane": "xy", "name": name}));
    let at = |x: f64, y: f64| json!([x * k, y * k]);
    let made = run(
        s,
        "sketch.path",
        json!({"sketch": name, "start": at(0.0, -7.0), "segments": [
            {"line": at(0.0, 7.0)},
            {"arc": at(-3.0, 10.0), "center": at(-3.0, 7.0)},
            {"line": at(-9.0, 10.0)},
            {"arc": at(-12.0, 7.0), "center": at(-9.0, 7.0)},
            {"line": at(-12.0, -7.0)},
            {"arc": at(-9.0, -10.0), "center": at(-9.0, -7.0)},
            {"line": at(-3.0, -10.0)},
            {"arc": "start", "center": at(-3.0, -7.0)},
        ]}),
    );
    made["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_closed_path_gives_a_ring_without_ends() {
    for k in SCALES {
        // A gasket: a round cord round a rounded rectangle.
        let mut s = new();
        let r = 0.8 * k;
        round(&mut s, "Section", [0.0, 0.0], r);
        let curves = rounded_rectangle(&mut s, "Route", k);
        assert_eq!(curves.len(), 8);
        let made = sweep(&mut s, "Section", "Route", "Gasket");
        assert_eq!(made["status"], "ok", "{made}");
        assert_eq!(made["path"]["closed"], true);
        let length = (40.0 + TAU * 3.0) * k;
        close(made["path"]["length"].as_f64().unwrap(), length, 1e-13);
        close(volume(&mut s, "Gasket"), PI * r * r * length, 1e-12);
        close(area(&mut s, "Gasket"), TAU * r * length, 1e-12);
        let names = face_names(&mut s, "Gasket");
        assert_eq!(names.len(), 8, "{names:?}");
        assert!(names.iter().all(|n| n.starts_with("F3:side(e2,")));
        deep(&mut s);

        // A frame: four lines, four mitres, a moulding 2 wide and 1 high
        // on the inside of the path.
        let mut s = new();
        rectangle(&mut s, "Section", [0.0, k], [-2.0 * k, 0.0]);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Route"}),
        );
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Route", "closed": true, "points": scaled(
                &[[0.0, -5.0], [0.0, 5.0], [-16.0, 5.0], [-16.0, -5.0]], k)}),
        );
        let made = sweep(&mut s, "Section", "Route", "Frame");
        assert_eq!(made["status"], "ok", "{made}");
        // 16 by 10 outside, 12 by 6 inside, 1 high.
        close(volume(&mut s, "Frame"), 88.0 * k.powi(3), 1e-12);
        close(
            area(&mut s, "Frame"),
            (2.0 * 88.0 + 52.0 + 36.0) * k * k,
            1e-12,
        );
        assert_eq!(face_names(&mut s, "Frame").len(), 10);
        deep(&mut s);

        // A square bar round a circle of radius 6: Pappus.
        let mut s = new();
        rectangle(&mut s, "Section", [-k, k], [-k, k]);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Route"}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Route", "center": [-6.0 * k, 0.0], "radius": 6.0 * k}),
        );
        let made = sweep(&mut s, "Section", "Route", "Ring");
        assert_eq!(made["status"], "ok", "{made}");
        close(volume(&mut s, "Ring"), 4.0 * TAU * 6.0 * k.powi(3), 1e-12);
        let names = face_names(&mut s, "Ring");
        // The circle is two half turns: the faces of a side that is
        // square to the path's plane are told apart by a number.
        assert!(names.contains(&"F3:side(e6,e2)".to_string()), "{names:?}");
        assert!(
            names.contains(&"F3:piece(1)/F3:side(e6,e2)".to_string()),
            "{names:?}"
        );
        deep(&mut s);
    }
}

/// An S from the origin along +y, as a spline through six control points.
const WAVE: [P; 6] = [
    [0.0, 0.0],
    [0.0, 4.0],
    [-3.0, 7.0],
    [-3.0, 11.0],
    [1.0, 14.0],
    [1.0, 18.0],
];

/// The length of `WAVE` (a cubic B-spline with clamped uniform knots),
/// by de Boor at many parameters.
fn wave_length() -> f64 {
    let knots = [0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 3.0];
    let at = |t: f64| -> P {
        let span = (3 + t.floor() as usize).min(5);
        let mut d: Vec<P> = (0..4).map(|j| WAVE[span - 3 + j]).collect();
        for r in 1..=3 {
            for j in (r..=3).rev() {
                let i = span - 3 + j;
                let alpha = (t - knots[i]) / (knots[i + 4 - r] - knots[i]);
                d[j] = [
                    (1.0 - alpha) * d[j - 1][0] + alpha * d[j][0],
                    (1.0 - alpha) * d[j - 1][1] + alpha * d[j][1],
                ];
            }
        }
        d[3]
    };
    let n = 300_000;
    (0..n)
        .map(|i| {
            let (a, b) = (
                at(3.0 * i as f64 / n as f64),
                at(3.0 * (i + 1) as f64 / n as f64),
            );
            (b[0] - a[0]).hypot(b[1] - a[1])
        })
        .sum()
}

#[test]
fn a_sweep_along_a_spline_says_how_far_it_is_from_the_true_one() {
    let unit_length = wave_length();
    for k in SCALES {
        // Along a spline the sweep is an approximation within the
        // tolerance, which follows the size above a unit as in the STEP
        // tests: one part in 10¹² of a million is not to be had.
        let tolerance = 1e-6 * k.max(1.0);
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"tolerance": tolerance}));
        let r = 0.5 * k;
        round(&mut s, "Section", [0.0, 0.0], r);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Route"}),
        );
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Route", "control": scaled(&WAVE, k)}),
        );
        let made = sweep(&mut s, "Section", "Route", "Hose");
        assert_eq!(made["status"], "ok", "{made}");
        let deviation = made["path"]["deviation"].as_f64().unwrap();
        assert!(
            deviation > 0.0 && deviation <= tolerance / 2.0,
            "{deviation} at {k}"
        );
        assert!(made["path"]["stations"].as_u64().unwrap() >= 6, "{made}");
        // The document says the same of the feature.
        assert_eq!(feature(&mut s, "Hose")["path"], made["path"]);
        let length = unit_length * k;
        close(made["path"]["length"].as_f64().unwrap(), length, 1e-8);
        // Centred on a path in a plane, a profile sweeps its area times
        // the length of the path. The faces are within `deviation` of the
        // true ones, so the volume is within their area times that.
        let exact = PI * r * r * length;
        let found = volume(&mut s, "Hose");
        let allowed = TAU * r * length * deviation + 1e-8 * exact;
        assert!(
            (found - exact).abs() <= allowed,
            "at {k}: volume {found}, exact {exact}, allowed {allowed}"
        );
        // The circle in four quarters, each one face along the spline.
        assert_eq!(
            face_names(&mut s, "Hose"),
            [
                "F3:side(e2,e7)",
                "F3:piece(1)/F3:side(e2,e7)",
                "F3:piece(2)/F3:side(e2,e7)",
                "F3:piece(3)/F3:side(e2,e7)",
                "F3:startCap(e2)",
                "F3:endCap(e2)"
            ]
        );
        deep(&mut s);
    }
}

#[test]
fn what_cannot_be_swept_fails_with_the_parameter_and_the_place() {
    // A mitre too sharp for the line it is on.
    let mut s = new();
    rectangle(&mut s, "Section", [-1.0, 1.0], [-1.0, 1.0]);
    path(
        &mut s,
        "Route",
        json!([{"line": [0, 10]}, {"line": [-1, 10]}, {"line": [-1, 0]}]),
        false,
    );
    let made = sweep(&mut s, "Section", "Route", "Bar");
    assert_eq!(made["status"], "failed", "{made}");
    let why = failure(&mut s, "Bar");
    assert!(
        why.starts_with("path: sketch `Route`: the corner between") && why.contains("too sharp"),
        "{why}"
    );
    // Nothing is left behind, and the document is sound.
    assert!(inspect(&mut s)["bodies"].as_array().unwrap().is_empty());

    // A bend tighter than the profile is wide on its inside.
    let mut s = new();
    rectangle(&mut s, "Section", [-1.0, 1.0], [-1.0, 1.0]);
    path(
        &mut s,
        "Route",
        json!([{"line": [0, 10]}, {"arc": [-0.75, 10.75], "center": [-0.75, 10]}]),
        false,
    );
    sweep(&mut s, "Section", "Route", "Bar");
    let why = failure(&mut s, "Bar");
    assert!(
        why.starts_with("path: sketch `Route`: the bend e6 is too tight"),
        "{why}"
    );

    // A spline that bends tighter than the profile reaches.
    let mut s = new();
    rectangle(&mut s, "Section", [-0.2, 0.2], [-1.0, 1.0]);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Route"}),
    );
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Route", "control": [[0, 0], [0, 6], [-2, 6], [-2, 0]]}),
    );
    sweep(&mut s, "Section", "Route", "Bar");
    let why = failure(&mut s, "Bar");
    assert!(
        why.contains("bends too tightly") && why.contains("e5"),
        "{why}"
    );

    // A sweep that comes back through itself: the faces that cross are
    // named.
    let mut s = new();
    round(&mut s, "Section", [0.0, 0.0], 1.0);
    path(
        &mut s,
        "Route",
        json!([
            {"line": [0, 10]},
            {"arc": [-3, 13], "center": [-3, 10]},
            {"arc": [-6, 10], "center": [-3, 10]},
            {"arc": [-3, 7], "center": [-3, 10]},
            {"line": [6, 7]},
        ]),
        false,
    );
    sweep(&mut s, "Section", "Route", "Knot");
    let why = failure(&mut s, "Knot");
    assert!(
        why.starts_with("path: the sweep passes through itself: F3:side(e2,")
            && why.contains(" cross"),
        "{why}"
    );

    // A path that does not start near the profile, and one that leaves it
    // at a slant.
    let mut s = new();
    round(&mut s, "Section", [0.0, 0.0], 1.0);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Route"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Route", "points": [[0, 3], [0, 10]]}),
    );
    sweep(&mut s, "Section", "Route", "Rod");
    let why = failure(&mut s, "Rod");
    assert!(
        why.starts_with("path: sketch `Route`: the path does not start on the plane"),
        "{why}"
    );
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Route", "entity": "e1", "at": [1, 0]}),
    );
    let why = failure(&mut s, "Rod");
    assert!(why.contains("at a right angle"), "{why}");
    // Put right, the sweep is built.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Route", "entity": "e1", "at": [0, 0]}),
    );
    assert_eq!(feature(&mut s, "Rod")["status"], "ok");

    // An open profile, and two profiles.
    let mut s = new();
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "zx", "name": "Section"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Section", "points": [[-1, -1], [1, -1], [1, 1]]}),
    );
    bent(&mut s, "Route", 1.0);
    sweep(&mut s, "Section", "Route", "Bar");
    let why = failure(&mut s, "Bar");
    assert!(
        why.starts_with("sketch `Section`") && why.contains("not closed"),
        "{why}"
    );
    let mut s = new();
    round(&mut s, "Section", [0.0, 0.0], 1.0);
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Section", "center": [5, 0], "radius": 1}),
    );
    bent(&mut s, "Route", 1.0);
    sweep(&mut s, "Section", "Route", "Bar");
    let why = failure(&mut s, "Bar");
    assert!(why.starts_with("sketch: sketch `Section` has 2"), "{why}");

    // A path with a branch, and one in two parts.
    let mut s = new();
    round(&mut s, "Section", [0.0, 0.0], 1.0);
    path(
        &mut s,
        "Route",
        json!([{"line": [0, 10]}, {"line": [-5, 15]}]),
        false,
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Route", "points": ["e2", [5, 15]]}),
    );
    sweep(&mut s, "Section", "Route", "Rod");
    let why = failure(&mut s, "Rod");
    assert!(
        why.starts_with("path: sketch `Route`: more than two curves of the path meet"),
        "{why}"
    );
    let mut s = new();
    round(&mut s, "Section", [0.0, 0.0], 1.0);
    path(&mut s, "Route", json!([{"line": [0, 10]}]), false);
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Route", "points": [[3, 12], [5, 15]]}),
    );
    sweep(&mut s, "Section", "Route", "Rod");
    let why = failure(&mut s, "Rod");
    assert!(why.contains("more than one part"), "{why}");
    // Construction geometry is not part of the path: with the stray line
    // as construction, the sweep is built.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Route", "entity": "e6", "construction": true}),
    );
    assert_eq!(feature(&mut s, "Rod")["status"], "ok");

    // What the command itself refuses: the same sketch twice, something
    // that is not a sketch, a parameter it does not have.
    let mut s = new();
    round(&mut s, "Section", [0.0, 0.0], 1.0);
    run(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "name": "Block"}),
    );
    let why = refused(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Section", "path": "Section"}),
    );
    assert!(why.contains("same sketch"), "{why}");
    let why = refused(
        &mut s,
        "feature.sweep",
        json!({"sketch": "Section", "path": "Block"}),
    );
    assert!(
        why.contains("not a sketch") || why.contains("sketch"),
        "{why}"
    );
    for bad in [
        json!({"sketch": "Section"}),
        json!({"path": "Section"}),
        json!({"sketch": "Section", "path": "Nowhere"}),
        json!({"sketch": "Section", "path": "Section", "twist": 3}),
        json!({"sketch": 1e300, "path": []}),
    ] {
        refused(&mut s, "feature.sweep", bad);
    }
}

#[test]
fn sweeps_in_booleans_keep_the_set_identities_and_survive_a_file() {
    let dir = workdir("booleans");
    for k in SCALES {
        let mut s = Session::new(Workspace {
            read: Access::Root(dir.clone()),
            write: Access::Root(dir.clone()),
        });
        // The tolerance follows the size at 1e6, as for the lofts: the
        // boolean refuses a tolerance finer than rounding allows.
        run(&mut s, "file.new", json!({"tolerance": 1e-6 * k.max(1.0)}));
        rectangle(&mut s, "Section", [-k, k], [-k, k]);
        bent(&mut s, "Route", k);
        let made = sweep(&mut s, "Section", "Route", "Bar");
        assert_eq!(made["status"], "ok", "{made}");
        deep(&mut s);
        let swept = volume(&mut s, "Bar");
        close(swept, 4.0 * bent_length(0.0) * k.powi(3), 1e-12);
        let before = inspect(&mut s)["bodies"].clone();
        run(&mut s, "file.save", json!({"path": "bar.3dcraft"}));
        run(&mut s, "file.close", json!({}));
        run(&mut s, "file.open", json!({"path": "bar.3dcraft"}));
        assert_eq!(inspect(&mut s)["bodies"], before);

        // With a block across the first leg, and with one across the
        // bend: |A ∪ B| + |A ∩ B| = |A| + |B| and |A − B| = |A| − |A ∩ B|.
        for (place, position) in [
            ("leg", [-3.0 * k, 3.0 * k, -0.5 * k]),
            ("bend", [-4.5 * k, 10.5 * k, -0.25 * k]),
        ] {
            let other = 6.0 * 4.0 * 3.0 * k.powi(3);
            let mut volumes = Vec::new();
            for op in ["union", "intersect", "subtract"] {
                run(&mut s, "file.close", json!({"discard": true}));
                run(&mut s, "file.open", json!({"path": "bar.3dcraft"}));
                run(
                    &mut s,
                    "feature.block",
                    json!({"position": position, "size": [6.0 * k, 4.0 * k, 3.0 * k], "name": "Tool"}),
                );
                let made = run(
                    &mut s,
                    "feature.boolean",
                    json!({"op": op, "target": "Bar", "tool": "Tool", "name": "Result"}),
                );
                assert_eq!(made["status"], "ok", "{place} {op} at {k}: {made}");
                deep(&mut s);
                volumes.push(volume(&mut s, "Result"));
            }
            let (union, common, difference) = (volumes[0], volumes[1], volumes[2]);
            close(union + common, swept + other, 1e-9);
            close(difference, swept - common, 1e-9);
            assert!(common > 0.0 && common < swept.min(other), "{place} at {k}");
        }
    }
}

#[test]
fn cut_join_and_intersect_combine_a_sweep_at_once() {
    // A channel milled into a plate along a bent path: 2 wide, 1 deep.
    for (how, expected) in [
        ("cut", 30.0 * 30.0 * 4.0 - 2.0 * bent_length(0.0)),
        ("join", 30.0 * 30.0 * 4.0 + 2.0 * bent_length(0.0)),
        ("intersect", 2.0 * bent_length(0.0)),
    ] {
        let mut s = new();
        run(
            &mut s,
            "feature.block",
            json!({"position": [-25, -3, -4], "size": [30, 30, 4], "name": "Plate"}),
        );
        // The channel runs from 1 below the top of the plate to the top
        // when it is cut, and stands on the plate when it is joined.
        let (low, high) = if how == "join" {
            (0.0, 1.0)
        } else {
            (-1.0, 0.0)
        };
        rectangle(&mut s, "Section", [low, high], [-1.0, 1.0]);
        // From 1 inside the plate's edge, so that both ends are inside.
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Route"}),
        );
        run(
            &mut s,
            "sketch.path",
            json!({"sketch": "Route", "start": [0, 0], "segments": [
                {"line": [0, 10]},
                {"arc": [-5, 15], "center": [-5, 10]},
                {"line": [-17, 15]},
            ]}),
        );
        let mut params = json!({"sketch": "Section", "path": "Route", "name": "Result"});
        params[how] = "Plate".into();
        let made = run(&mut s, "feature.sweep", params);
        assert_eq!(made["status"], "ok", "{how}: {made}");
        assert_eq!(made["sweep"]["status"], "ok", "{made}");
        close(volume(&mut s, "Result"), expected, 1e-9);
        deep(&mut s);
    }
}

#[test]
fn names_survive_edits_and_a_lost_sketch_fails_the_sweep_alone() {
    let dir = workdir("history");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    run(&mut s, "file.new", json!({}));
    rectangle(&mut s, "Section", [-1.0, 1.0], [-1.0, 1.0]);
    let curves = bent(&mut s, "Route", 1.0);
    assert_eq!(curves, ["e3", "e6", "e8"]);
    run(
        &mut s,
        "feature.block",
        json!({"position": [50, 50, 0], "size": [5, 5, 5], "name": "Bystander"}),
    );
    let made = sweep(&mut s, "Section", "Route", "Bar");
    assert_eq!(made["status"], "ok", "{made}");
    // A sketch on the end cap, extruded: a pin that stands on the sweep's
    // far end and points on along the path (−x).
    let cap = "F4:endCap(e5,e6,e7,e8)";
    assert!(face_names(&mut s, "Bar").contains(&cap.to_string()));
    run(
        &mut s,
        "sketch.create",
        json!({"face": cap, "name": "OnEnd"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "OnEnd", "center": [0, 0], "radius": 0.5}),
    );
    let pin = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "OnEnd", "distance": 4, "name": "Pin"}),
    );
    assert_eq!(pin["status"], "ok", "{pin}");
    let pin_at = |s: &mut Session| -> Vec<f64> {
        body(s, "Pin")["centroid"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect()
    };
    let at = pin_at(&mut s);
    assert!(
        (at[0] + 19.0).abs() < 1e-9 && (at[1] - 15.0).abs() < 1e-9,
        "{at:?}"
    );
    // Its edges have names of their own, by the faces they lie on.
    let edges = run(&mut s, "document.edges", json!({"feature": "Bar"}));
    let names: Vec<&str> = edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "{names:?}");
    assert!(
        names.contains(&"F4:between[F4:side(e6,e3),F4:side(e6,e6)]"),
        "{names:?}"
    );

    // An upstream edit of the path: its last point moves out. The sweep
    // follows, the pin goes along, and what was built is what a full
    // regeneration builds (`run` asserts that).
    let before = volume(&mut s, "Bar");
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Route", "entity": "e7", "at": [-27, 15]}),
    );
    close(volume(&mut s, "Bar") - before, 40.0, 1e-9);
    assert_eq!(feature(&mut s, "Pin")["status"], "ok");
    let at = pin_at(&mut s);
    assert!((at[0] + 29.0).abs() < 1e-9, "{at:?}");
    // An upstream edit of the profile: a corner moves, and with it two
    // sides. The names stay, so the pin's sketch stays on its cap.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Section", "entity": "e3", "at": [1, 2]}),
    );
    assert_eq!(feature(&mut s, "Bar")["status"], "ok");
    assert_eq!(feature(&mut s, "Pin")["status"], "ok");
    assert!(face_names(&mut s, "Bar").contains(&cap.to_string()));
    deep(&mut s);

    // Saved and reopened: the same bodies.
    let bodies = inspect(&mut s)["bodies"].clone();
    run(&mut s, "file.save", json!({"path": "bar.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "bar.3dcraft"}));
    assert_eq!(inspect(&mut s)["bodies"], bodies);

    // A path that fails takes the sweep with it, which says which
    // parameter; the pin fails for the sweep, and nothing else does.
    run(&mut s, "feature.suppress", json!({"feature": "Route"}));
    let why = failure(&mut s, "Bar");
    assert!(why.starts_with("path: `Route` is suppressed"), "{why}");
    assert_eq!(feature(&mut s, "Bystander")["status"], "ok");
    assert_eq!(feature(&mut s, "OnEnd")["status"], "failed");
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Route", "suppressed": false}),
    );
    assert_eq!(feature(&mut s, "Bar")["status"], "ok");
    run(&mut s, "feature.suppress", json!({"feature": "Section"}));
    let why = failure(&mut s, "Bar");
    assert!(why.starts_with("sketch: `Section` is suppressed"), "{why}");
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Section", "suppressed": false}),
    );

    // Deleted, the path is gone: the sweep fails with the parameter, the
    // pin fails because of the sweep (causedBy), the bystander stands.
    run(&mut s, "feature.delete", json!({"feature": "Route"}));
    let why = failure(&mut s, "Bar");
    assert!(why.starts_with("path: there is no feature"), "{why}");
    assert_eq!(feature(&mut s, "Bystander")["status"], "ok");
    let pin = feature(&mut s, "Pin");
    assert_eq!(pin["status"], "failed");
    assert_eq!(pin["causedBy"], "F4", "{pin}");
    // Re-pointed at another path, the sweep is built again, and the pin
    // with it: its cap is named after the profile, which did not change.
    path(&mut s, "Straight", json!([{"line": [0, 25]}]), false);
    run(
        &mut s,
        "feature.move",
        json!({"feature": "Straight", "position": 1}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Bar", "path": "Straight"}),
    );
    assert_eq!(feature(&mut s, "Bar")["status"], "ok");
    assert_eq!(feature(&mut s, "Pin")["status"], "ok");
    let at = pin_at(&mut s);
    assert!((at[1] - 27.0).abs() < 1e-9 && at[0].abs() < 1.0, "{at:?}");
    // And at another profile.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "zx", "name": "Disc"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Disc", "center": [0, 0], "radius": 2}),
    );
    run(
        &mut s,
        "feature.move",
        json!({"feature": "Disc", "position": 1}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Bar", "sketch": "Disc"}),
    );
    assert_eq!(feature(&mut s, "Bar")["status"], "ok");
    close(volume(&mut s, "Bar"), PI * 4.0 * 25.0, 1e-12);
    // The cap was named after the square's sides and is now named after
    // the circle; a sketch on an end cap is on the plane where the
    // feature ends, whatever its outline, so the pin still stands there.
    assert!(face_names(&mut s, "Bar").contains(&"F4:endCap(e2)".to_string()));
    assert_eq!(feature(&mut s, "OnEnd")["status"], "ok");
    assert_eq!(feature(&mut s, "Pin")["status"], "ok");
    let why = refused(
        &mut s,
        "feature.edit",
        json!({"feature": "Bar", "path": "Disc"}),
    );
    assert!(why.contains("same sketch"), "{why}");
    let why = refused(
        &mut s,
        "feature.edit",
        json!({"feature": "Bystander", "path": "Straight"}),
    );
    assert!(why.contains("path"), "{why}");
}

#[test]
fn a_sweep_along_a_spline_keeps_the_set_identities_with_a_block() {
    // The faces along a spline are B-spline surfaces of degree 5 to 9
    // along the path: the boolean cuts them, and the pieces add up.
    let build = |op: &str| -> (f64, f64) {
        let mut s = new();
        round(&mut s, "Section", [0.0, 0.0], 3.0);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Route"}),
        );
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Route", "control":
                [[0, 0], [0, 20], [-25, 30], [-25, 55], [10, 65], [10, 90]]}),
        );
        let made = sweep(&mut s, "Section", "Route", "Hose");
        assert_eq!(made["status"], "ok", "{made}");
        let hose = volume(&mut s, "Hose");
        run(
            &mut s,
            "feature.block",
            json!({"position": [-30, 35, -1], "size": [40, 12, 8], "name": "Tool"}),
        );
        let made = run(
            &mut s,
            "feature.boolean",
            json!({"op": op, "target": "Hose", "tool": "Tool", "name": "Result"}),
        );
        assert_eq!(made["status"], "ok", "{op}: {made}");
        deep(&mut s);
        (hose, volume(&mut s, "Result"))
    };
    let (hose, union) = build("union");
    let (_, common) = build("intersect");
    let (_, difference) = build("subtract");
    let block = 40.0 * 12.0 * 8.0;
    // To 1e-7, as for the lofts: the curves in which the block cuts the
    // freeform faces are fitted within the tolerance (measured: 2.3e-9).
    close(union + common, hose + block, 1e-7);
    close(difference, hose - common, 1e-7);
    assert!(common > 0.0 && common < hose.min(block));
}

/// Finding F6 of the turbofan, as the turbofan's script drew it: an arc
/// of the path given by its centre, with its end rounded to four
/// decimals, then a line. `sketch.path` puts the end on the arc's circle
/// (2.6e-5 in) and says so, so the solver moves nothing and the first
/// line stays square to the profile; the line after the arc, to the
/// rounded far end, then leaves 2.9e-6 radians off the arc's direction,
/// which turns the profile (radius 5) by no more than 5 (1 - cos) = 2e-11
/// off the faces, so the sweep is built at the default tolerance, valid,
/// with the closed-form volume: the circle's area times the length of
/// the path (Pappus, the profile centred on it).
#[test]
fn a_path_whose_arc_end_was_rounded_is_swept_at_the_default_tolerance() {
    let mut s = new();
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "zx", "offset": "10 mm", "name": "Prof"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Prof", "center": [0, 50], "radius": 5}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Path"}),
    );
    let drawn = run(
        &mut s,
        "sketch.path",
        json!({"sketch": "Path", "start": [50, 10], "segments": [
            {"line": [50, 16]},
            {"arc": [53.9787, 28.6187], "center": [72, 16], "clockwise": true},
            {"line": [62.0088, 40.0868]},
        ]}),
    );
    assert!(drawn.get("warnings").is_none(), "{drawn}");
    let snap = &drawn["snapped"][0];
    assert_eq!(snap["segment"], 1, "{drawn}");
    let moved = snap["moved"].as_f64().unwrap();
    assert!((moved - 2.6287e-5).abs() < 1e-9, "{drawn}");
    let end = [
        snap["end"][0].as_f64().unwrap(),
        snap["end"][1].as_f64().unwrap(),
    ];
    sweep(&mut s, "Prof", "Path", "Tube");
    assert_eq!(feature(&mut s, "Tube")["status"], "ok");
    deep(&mut s);
    // Line, clockwise arc of radius 22 from the angle of 180° down to the
    // end's, line.
    let turn = PI - (end[1] - 16.0).atan2(end[0] - 72.0);
    let last = (62.0088 - end[0]).hypot(40.0868 - end[1]);
    let length = 6.0 + 22.0 * turn + last;
    close(volume(&mut s, "Tube"), PI * 25.0 * length, 1e-9);
}

/// A path that leaves the profile a little off square through the
/// commands: within the tolerance the sweep is built, as long as the
/// slanted line, with the profile's area; twice as far off it is refused.
/// The tolerance is coarse (1e-5 of the profile's radius) so that the
/// lean is larger than the angle within which two directions count as
/// one.
#[test]
fn a_lean_within_the_tolerance_is_swept_at_every_scale() {
    for k in SCALES {
        let tolerance = 1e-5 * k;
        for (slope, built) in [(0.5e-5, true), (2e-5, false)] {
            let mut s = Session::new(Workspace::none());
            run(&mut s, "file.new", json!({"tolerance": tolerance}));
            round(&mut s, "Section", [0.0, 0.0], k);
            run(
                &mut s,
                "sketch.create",
                json!({"plane": "xy", "name": "Route"}),
            );
            run(
                &mut s,
                "sketch.polyline",
                json!({"sketch": "Route", "points": [[0, 0], [slope * 10.0 * k, 10.0 * k]]}),
            );
            sweep(&mut s, "Section", "Route", "Rod");
            if built {
                assert_eq!(feature(&mut s, "Rod")["status"], "ok", "{k}");
                let length = 10.0 * k * slope.hypot(1.0);
                close(volume(&mut s, "Rod"), PI * k * k * length, 1e-12);
                deep(&mut s);
            } else {
                let why = failure(&mut s, "Rod");
                assert!(why.contains("more than the tolerance"), "{k}: {why}");
            }
        }
    }
}
