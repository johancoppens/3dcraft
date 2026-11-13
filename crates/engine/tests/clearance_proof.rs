//! A clearance never proves a bound that is false (J-018; design:
//! docs/design/verification.md › *Findings*, 2026-11-13). Two bodies that
//! share volume are an overlap, never clear with a distance "proven", and
//! the distance between faces that cross is proven no more than 0;
//! bodies apart, touching and overlapping are told at their closed-form
//! clearance and common volume, at scales 1e-3, 1 and 1e6. The oracles are
//! closed forms worked by hand (each test says how) and the kernel's
//! common volume, which no distance code takes part in.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn mm(x: f64) -> String {
    format!("{x:?} mm")
}

/// A document in mm at a tolerance of a millionth of the scale `k`.
fn document(k: f64) -> Session {
    let mut s = Session::new(Workspace::none());
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6 * k}),
    );
    s
}

fn build(s: &mut Session, steps: &[(&str, Value)]) {
    for (id, params) in steps {
        run(s, id, params.clone());
    }
}

/// What `measure.clearance` says of the pair: its status, the clearance
/// and what is proven below it.
fn clearance(s: &mut Session, a: &str, b: &str) -> (String, Option<f64>, Option<f64>) {
    let out = run(s, "measure.clearance", json!({"from": a, "to": b}));
    let status = out["pairs"][0]["status"].as_str().unwrap().to_string();
    (status, out["clearance"].as_f64(), out["atLeast"].as_f64())
}

/// The common volume of the pair, as the overlap measure finds it (the
/// kernel's intersection): 0 when none.
fn common(s: &mut Session, a: &str, b: &str) -> f64 {
    let name = format!("overlap {a} {b}");
    let out = run(
        s,
        "assert.add",
        json!({"name": name, "measure": "overlap", "of": [a, b], "is": false}),
    );
    run(s, "assert.delete", json!({"name": name}));
    // Passed: none; failed: an overlap; anything else is not an answer.
    assert!(
        out["status"] == "passed" || out["status"] == "failed",
        "{name}: {out}"
    );
    out["detail"]["overlaps"]
        .as_array()
        .map_or(0.0, |o| o.iter().filter_map(|v| v["volume"].as_f64()).sum())
}

/// The reduced case of J-018 (its reproducer was a Jig of 374 steps): a
/// bar 10 × 2 × 2 and a clamp round it, a C-shaped profile in the yz
/// plane at x = 4 extruded 2 along x, whose jaw (its underside at
/// z = 1.9) crosses the bar's top 0.1 deep and whose foot lies 0.05 under
/// the bar. They share [4, 6] × [0, 2] × [1.9, 2]: 2 × 2 × 0.1 = 0.4. No
/// corner of either lies in the other, so only faces crossing tell it;
/// the mesh measured the crossing faces 0.1 apart (corners and sides
/// alone), the search settled only the foot's faces, and the clearance
/// was answered clear, 0.05, proven at least 0.05, and an assertion of at
/// least 0.03 passed.
fn clamp(k: f64) -> Vec<(&'static str, Value)> {
    let points: Vec<[String; 2]> = [
        (-3.0, 3.0),
        (-3.0, -1.0),
        (1.0, -1.0),
        (1.0, -0.05),
        (-2.0, -0.05),
        (-2.0, 1.9),
        (5.0, 1.9),
        (5.0, 3.0),
    ]
    .iter()
    .map(|(y, z)| [mm(y * k), mm(z * k)])
    .collect();
    vec![
        (
            "feature.block",
            json!({"name": "Bar", "size": [mm(10.0 * k), mm(2.0 * k), mm(2.0 * k)]}),
        ),
        (
            "sketch.create",
            json!({"name": "C", "plane": "yz", "offset": mm(4.0 * k)}),
        ),
        (
            "sketch.polyline",
            json!({"sketch": "C", "closed": true, "points": points}),
        ),
        (
            "feature.extrude",
            json!({"sketch": "C", "distance": mm(2.0 * k), "name": "Clamp"}),
        ),
    ]
}

#[test]
fn the_clamp_of_j018_overlaps_and_no_clearance_is_proven() {
    for k in [1e-3, 1.0, 1e6] {
        let tol = 1e-6 * k;
        let mut s = document(k);
        build(&mut s, &clamp(k));
        let volume = common(&mut s, "Bar", "Clamp");
        assert!(
            (volume - 0.4 * k * k * k).abs() <= 1e-9 * k * k * k,
            "{k}: {volume}"
        );
        for (a, b) in [("Bar", "Clamp"), ("Clamp", "Bar")] {
            let (status, value, at_least) = clearance(&mut s, a, b);
            assert_eq!(status, "overlap", "{k}: {a} {b}");
            assert_eq!((value, at_least), (None, None), "{k}: {a} {b}");
        }
        // The faces cross: their surfaces are 0 apart, so nothing above
        // the tolerance is proven (it was 0.05) or found.
        let d = run(
            &mut s,
            "measure.distance",
            json!({"from": "Bar", "to": "Clamp"}),
        );
        let (found, at_least) = (
            d["distance"].as_f64().unwrap(),
            d["atLeast"].as_f64().unwrap(),
        );
        assert!(found <= tol && at_least <= tol, "{k}: {d}");
        // Assertions on it fail, as an overlap; the document's check is
        // not ok.
        for (name, min) in [("gap", 0.03 * k), ("apart", 0.0)] {
            let out = run(
                &mut s,
                "assert.add",
                json!({"name": name, "measure": "clearance", "from": "Bar", "to": "Clamp", "min": mm(min)}),
            );
            assert_eq!(out["status"], "failed", "{k}: {out}");
        }
        let check = run(&mut s, "document.check", json!({}));
        assert_eq!(check["ok"], false, "{k}: {check}");
    }
}

/// Two balls of radii 5 and 3 whose centres are 8 + g apart: g apart
/// (along the line of centres). Overlapping by h = -g, they share a lens:
/// π h² (d² + 2 d (r1 + r2) − 3 (r1 − r2)²) / (12 d), d = r1 + r2 − h.
fn balls(k: f64, g: f64) -> (Vec<(&'static str, Value)>, f64) {
    let steps = vec![
        (
            "feature.sphere",
            json!({"name": "A", "radius": mm(5.0 * k), "position": [mm(0.0), mm(0.0), mm(0.0)]}),
        ),
        (
            "feature.sphere",
            json!({"name": "B", "radius": mm(3.0 * k), "position": [mm(8.0 * k + g), mm(0.0), mm(0.0)]}),
        ),
    ];
    let (r1, r2, h) = (5.0 * k, 3.0 * k, -g);
    let d = r1 + r2 - h;
    let lens =
        PI * h * h * (d * d + 2.0 * d * (r1 + r2) - 3.0 * (r1 - r2) * (r1 - r2)) / (12.0 * d);
    (steps, lens)
}

/// A ball of radius 3 over a block's top face (20 × 20 × 5): its centre
/// 3 + g above the face, so g apart. Sunk h = -g, they share a cap of
/// π h² (3 r − h) / 3.
fn ball_on_block(k: f64, g: f64) -> (Vec<(&'static str, Value)>, f64) {
    let steps = vec![
        (
            "feature.block",
            json!({"name": "A", "size": [mm(20.0 * k), mm(20.0 * k), mm(5.0 * k)]}),
        ),
        (
            "feature.sphere",
            json!({"name": "B", "radius": mm(3.0 * k), "position": [mm(10.0 * k), mm(10.0 * k), mm(8.0 * k + g)]}),
        ),
    ];
    let (r, h) = (3.0 * k, -g);
    (steps, PI * h * h * (3.0 * r - h) / 3.0)
}

/// A pin of radius 4 − g, 40 long, through a hole of radius 4 drilled
/// through a block 20 high, on one axis: g apart (radially, all along).
/// A pin wider than the hole (g < 0) shares a tube π (r_pin² − r_hole²)
/// as long as the block, 20.
fn pin_in_hole(k: f64, g: f64) -> (Vec<(&'static str, Value)>, f64) {
    let steps = vec![
        (
            "feature.block",
            json!({"name": "Blank", "size": [mm(20.0 * k), mm(20.0 * k), mm(20.0 * k)]}),
        ),
        (
            "feature.cylinder",
            json!({"name": "Drill", "radius": mm(4.0 * k), "height": mm(30.0 * k), "position": [mm(10.0 * k), mm(10.0 * k), mm(-5.0 * k)]}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "Blank", "tool": "Drill", "name": "A"}),
        ),
        (
            "feature.cylinder",
            json!({"name": "B", "radius": mm(4.0 * k - g), "height": mm(40.0 * k), "position": [mm(10.0 * k), mm(10.0 * k), mm(-10.0 * k)]}),
        ),
    ];
    let (pin, hole) = (4.0 * k - g, 4.0 * k);
    (steps, PI * (pin * pin - hole * hole) * 20.0 * k)
}

/// A block 20 on a side with a closed cube cavity 10 on a side at its
/// middle, and a ball of radius 5 − g at the cavity's middle: g from the
/// middles of the cavity's six faces, the nearest places. A ball of
/// radius 5 + h passes through all six, sharing six caps of
/// π h² (3 r − h) / 3 (apart from each other: the ball reaches 5 + h from
/// the middle, the cavity's edges are 5 √2 from it).
fn ball_in_cavity(k: f64, g: f64) -> (Vec<(&'static str, Value)>, f64) {
    let steps = vec![
        (
            "feature.block",
            json!({"name": "Blank", "size": [mm(20.0 * k), mm(20.0 * k), mm(20.0 * k)]}),
        ),
        (
            "feature.block",
            json!({"name": "Hollow", "size": [mm(10.0 * k), mm(10.0 * k), mm(10.0 * k)], "position": [mm(5.0 * k), mm(5.0 * k), mm(5.0 * k)]}),
        ),
        (
            "feature.boolean",
            json!({"op": "subtract", "target": "Blank", "tool": "Hollow", "name": "A"}),
        ),
        (
            "feature.sphere",
            json!({"name": "B", "radius": mm(5.0 * k - g), "position": [mm(10.0 * k), mm(10.0 * k), mm(10.0 * k)]}),
        ),
    ];
    let (r, h) = (5.0 * k - g, -g);
    (steps, 6.0 * PI * h * h * (3.0 * r - h) / 3.0)
}

type Pair = fn(f64, f64) -> (Vec<(&'static str, Value)>, f64);

/// Each pair just clear (10 tolerances apart), touching, just overlapping
/// (10 tolerances deep) and overlapping 0.1 deep (scaled; a thirtieth to
/// a fiftieth of the radii): clear at its closed-form gap with no more
/// proven than that and more than the band of touching, touching at 0,
/// overlapping with no clearance and, deep enough to measure, the
/// closed-form common volume.
#[test]
fn closed_forms_just_clear_touching_and_overlapping_at_three_scales() {
    let pairs: [(&str, Pair); 4] = [
        ("balls", balls),
        ("ball on block", ball_on_block),
        ("pin in hole", pin_in_hole),
        ("ball in cavity", ball_in_cavity),
    ];
    for (name, pair) in pairs {
        for k in [1e-3, 1.0, 1e6] {
            let tol = 1e-6 * k;
            for g in [10.0 * tol, 0.0, -10.0 * tol, -0.1 * k] {
                let what = format!("{name} at {k}, {g}");
                let (steps, volume) = pair(k, g);
                let mut s = document(k);
                build(&mut s, &steps);
                let (status, value, at_least) = clearance(&mut s, "A", "B");
                let shared = common(&mut s, "A", "B");
                if g > 0.0 {
                    assert_eq!(status, "clear", "{what}");
                    // Found on the faces to their tolerance (a point within
                    // it of a face's edge counts as on the face).
                    assert!((value.unwrap() - g).abs() <= tol, "{what}: {value:?}");
                    let at_least = at_least.unwrap();
                    assert!(
                        at_least <= g * (1.0 + 1e-9) && at_least > 2.0 * tol,
                        "{what}: {at_least}"
                    );
                    assert_eq!(shared, 0.0, "{what}");
                } else if g == 0.0 {
                    assert_eq!(status, "touch", "{what}");
                    assert_eq!(value, Some(0.0), "{what}");
                    assert_eq!(shared, 0.0, "{what}");
                } else {
                    assert_eq!(status, "overlap", "{what}");
                    assert_eq!((value, at_least), (None, None), "{what}");
                    assert!(shared > 0.0, "{what}");
                    if g < -10.0 * tol {
                        assert!(
                            (shared - volume).abs() <= 1e-6 * volume,
                            "{what}: {shared} for {volume}"
                        );
                    }
                }
            }
        }
    }
}

/// A bar 10 × 2 × 2 and a tool across its top `depth` deep: a bar
/// 2 × 8 × 1 turned `angle` degrees about z, a rod of radius 0.75 lying
/// across, or a ball of radius 1 on the top edge sunk `depth` along the
/// diagonal; with or without a cube `gap` beside the bar's side (a lump
/// of the tool's own, which the search for the nearest places finds
/// first). A negative depth lifts the tool clear. Every length times `k`.
fn crossing(
    kind: &str,
    depth: f64,
    angle: f64,
    gap: Option<f64>,
    k: f64,
) -> Vec<(&'static str, Value)> {
    let l = |x: f64| mm(x * k);
    let turned = |name: &str| json!({"source": name, "angle": format!("{angle} deg"), "axis": [0, 0, 1], "pivot": [l(5.0), l(1.0), l(0.0)], "name": "T"});
    let mut steps = vec![(
        "feature.block",
        json!({"name": "A", "size": [l(10.0), l(2.0), l(2.0)]}),
    )];
    match kind {
        "bar" => {
            steps.push((
                "feature.block",
                json!({"name": "T0", "size": [l(2.0), l(8.0), l(1.0)], "position": [l(4.0), l(-3.0), l(2.0 - depth)]}),
            ));
            steps.push(("feature.transform", turned("T0")));
        }
        "rod" => {
            steps.push((
                "feature.cylinder",
                json!({"name": "T0", "radius": l(0.75), "height": l(8.0), "position": [l(5.0), l(-3.0), l(2.75 - depth)], "axis": [0, 1, 0]}),
            ));
            steps.push(("feature.transform", turned("T0")));
        }
        _ => {
            let c = (1.0 - depth) / 2.0_f64.sqrt();
            steps.push((
                "feature.sphere",
                json!({"name": "T", "radius": l(1.0), "position": [l(3.0 + angle / 30.0), l(2.0 + c), l(2.0 + c)]}),
            ));
        }
    }
    match gap {
        None => steps.push(("feature.transform", json!({"source": "T", "name": "B"}))),
        Some(gap) => {
            steps.push((
                "feature.block",
                json!({"name": "Near", "size": [l(0.5), l(0.5), l(1.0)], "position": [l(8.5), l(-0.5 - gap), l(0.5)]}),
            ));
            steps.push((
                "feature.boolean",
                json!({"op": "union", "target": "T", "tool": "Near", "name": "B"}),
            ));
        }
    }
    steps
}

/// Whenever the kernel finds a volume in common, the clearance proves no
/// distance and is no pass of an assertion of at least 0; whenever it
/// proves a positive clearance, the kernel finds none. Over tools crossing
/// a bar 0.05 to 0.3 deep, flat (13 of these were answered clear and
/// proven 0.01 or 0.03 apart), round and a ball on an edge, turned and
/// with a near lump; and the same lifted 0.05 clear; at scales 1e-3, 1
/// and 1e6.
#[test]
fn whenever_overlap_finds_a_common_volume_clearance_proves_nothing() {
    let mut overlaps = 0;
    for k in [1e-3, 1.0, 1e6] {
        let tol = 1e-6 * k;
        for kind in ["bar", "rod", "ball"] {
            for depth in [0.05, 0.1, 0.3, -0.05] {
                let angles: &[f64] = if kind == "ball" {
                    &[0.0, 60.0]
                } else {
                    &[0.0, 17.0, 45.0]
                };
                for &angle in angles {
                    for gap in [None, Some(0.01), Some(0.03)] {
                        let what =
                            format!("{kind} {depth} deep, turned {angle}, near {gap:?}, at {k}");
                        let mut s = document(k);
                        build(&mut s, &crossing(kind, depth, angle, gap, k));
                        let shared = common(&mut s, "A", "B");
                        let (status, value, at_least) = clearance(&mut s, "A", "B");
                        let out = run(
                            &mut s,
                            "assert.add",
                            json!({"name": "apart", "measure": "clearance", "from": "A", "to": "B", "min": "0 mm"}),
                        );
                        if shared > 0.0 {
                            overlaps += 1;
                            assert_eq!(status, "overlap", "{what}: {shared}");
                            assert_eq!((value, at_least), (None, None), "{what}");
                            assert_ne!(out["status"], "passed", "{what}");
                        }
                        if at_least.is_some_and(|l| l > 2.0 * tol) || status == "clear" {
                            assert_eq!(shared, 0.0, "{what}: {status} {at_least:?}");
                        }
                        if depth < 0.0 {
                            // Lifted 0.05 clear: the bar or rod's least
                            // point, or the ball's along the diagonal; the
                            // near lump is nearer when there is one.
                            assert_eq!(status, "clear", "{what}");
                            let want = gap.unwrap_or(0.05).min(0.05) * k;
                            let value = value.unwrap();
                            assert!((value - want).abs() <= tol, "{what}: {value}");
                            assert!(at_least.unwrap() <= value, "{what}");
                        }
                    }
                }
            }
        }
    }
    assert_eq!(overlaps, 3 * 72);
}

/// A freeform tool: a profile in the yz plane at x = 4 whose underside is
/// the parabola z = 2 − depth + 0.1 (y − 1)² for y from −3 to 5 (a spline
/// from `sketch.curve`), closed by lines at z = 4.5 and extruded 2 along x,
/// over the bar 10 × 2 × 2; every length times `k`. Sunk `depth` = 0.05 it
/// crosses the bar's top where |y − 1| < w = √(depth / 0.1): they share
/// 2 ∫ (depth − 0.1 s²) ds over |s| < w = (8/3) depth w (by hand). Lifted
/// (depth −g) its lowest line, at y = 1 over the middle of the bar's top,
/// is the nearest: g apart.
fn spline_tool(depth: f64, k: f64) -> Vec<(&'static str, Value)> {
    let l = |x: f64| mm(x * k);
    vec![
        (
            "feature.block",
            json!({"name": "A", "size": [l(10.0), l(2.0), l(2.0)]}),
        ),
        (
            "sketch.create",
            json!({"name": "P", "plane": "yz", "offset": l(4.0)}),
        ),
        (
            "sketch.curve",
            json!({"sketch": "P", "x": format!("{} * (8 * t - 3)", l(1.0)), "y": format!("{} + {} * (8 * t - 4)^2", l(2.0 - depth), l(0.1)), "t": [0, 1]}),
        ),
        (
            "sketch.path",
            json!({"sketch": "P", "start": "e2", "segments": [{"line": [l(5.0), l(4.5)]}, {"line": [l(-3.0), l(4.5)]}, {"line": "e1"}]}),
        ),
        (
            "feature.extrude",
            json!({"sketch": "P", "distance": l(2.0), "name": "B"}),
        ),
    ]
}

/// The spline tool crossing the bar is an overlap whose common volume the
/// kernel finds at the closed form (to the fit of the curve), and no
/// clearance is proven; lifted 0.05 and 10 tolerances clear, it is clear
/// at its gap with no more proven than that and more than the band of
/// touching. At scales 1e-3, 1 and 1e6.
#[test]
fn a_spline_tool_crossing_the_bar_and_lifted_clear() {
    for k in [1e-3, 1.0, 1e6] {
        let tol = 1e-6 * k;
        let depth = 0.05;
        let mut s = document(k);
        build(&mut s, &spline_tool(depth, k));
        let what = format!("spline tool {depth} deep at {k}");
        let want = 8.0 / 3.0 * depth * (depth / 0.1_f64).sqrt() * k * k * k;
        let shared = common(&mut s, "A", "B");
        assert!(
            (shared - want).abs() <= 1e-4 * want,
            "{what}: {shared} for {want}"
        );
        let (status, value, at_least) = clearance(&mut s, "A", "B");
        assert_eq!(status, "overlap", "{what}");
        assert_eq!((value, at_least), (None, None), "{what}");
        for g in [0.05 * k, 10.0 * tol] {
            let mut s = document(k);
            build(&mut s, &spline_tool(-g / k, k));
            let what = format!("spline tool {g} clear at {k}");
            assert_eq!(common(&mut s, "A", "B"), 0.0, "{what}");
            let (status, value, at_least) = clearance(&mut s, "A", "B");
            assert_eq!(status, "clear", "{what}");
            let (value, at_least) = (value.unwrap(), at_least.unwrap());
            assert!((value - g).abs() <= tol, "{what}: {value}");
            assert!(
                at_least <= value && at_least > 2.0 * tol,
                "{what}: {at_least}"
            );
        }
    }
}

/// The clamp of J-018's reduced case turned 37° about a slanted axis and
/// moved about 1e6 of the scale from the origin, some 1e5 times the size
/// of its triangles: still an overlap, and the faces that cross are proven
/// no more than the tolerance apart (when crossings were decided on
/// coordinates, their rounding there missed them), at scales 1e-3, 1 and
/// 1e6.
#[test]
fn the_clamp_turned_far_from_the_origin_overlaps_and_no_clearance_is_proven() {
    for k in [1e-3, 1.0, 1e6] {
        let tol = 1e-6 * k;
        let mut s = document(k);
        build(&mut s, &clamp(k));
        for (source, name) in [("Bar", "Far bar"), ("Clamp", "Far clamp")] {
            run(
                &mut s,
                "feature.transform",
                json!({"source": source, "angle": "37 deg", "axis": [0.36, 0.48, 0.8], "pivot": [mm(0.0), mm(0.0), mm(0.0)], "translate": [mm(6e5 * k), mm(-4e5 * k), mm(6.6e5 * k)], "name": name}),
            );
        }
        let what = format!("far clamp at {k}");
        for (a, b) in [("Far bar", "Far clamp"), ("Far clamp", "Far bar")] {
            let (status, value, at_least) = clearance(&mut s, a, b);
            assert_eq!(status, "overlap", "{what}: {a} {b}");
            assert_eq!((value, at_least), (None, None), "{what}");
        }
        let d = run(
            &mut s,
            "measure.distance",
            json!({"from": "Far bar", "to": "Far clamp"}),
        );
        let at_least = d["atLeast"].as_f64().unwrap();
        assert!(at_least <= tol, "{what}: {d}");
        let volume = common(&mut s, "Far bar", "Far clamp");
        assert!(
            (volume - 0.4 * k * k * k).abs() <= 1e-6 * k * k * k,
            "{what}: {volume}"
        );
    }
}
