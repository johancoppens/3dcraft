//! The adaptive proofs of a clearance and a thickness
//! (docs/design/verification.md › *The proven thickness*): three models on
//! which the true bound held but the proof fell short (the planetary
//! gearbox's planet in its ring, a case shelled to its fillet radius, a
//! clamp with chamfered bore mouths), and closed forms at three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};
use std::time::Instant;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn new() -> Session {
    new_at(1.0)
}

/// A new document for a model of size about `k` × 10 mm: its tolerance
/// scaled with it.
fn new_at(k: f64) -> Session {
    let mut s = Session::new(Workspace::unrestricted());
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6 * k}),
    );
    s
}

/// Runs `id` and returns (found, proven, seconds).
fn timed(s: &mut Session, id: &str, params: Value, key: &str, what: &str) -> (f64, f64) {
    let started = Instant::now();
    let out = run(s, id, params);
    let found = out[key].as_f64().unwrap_or_else(|| panic!("{what}: {out}"));
    let proven = out["atLeast"]
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: {out}"));
    eprintln!(
        "{what}: found {found}, proven {proven}, in {:.3} s",
        started.elapsed().as_secs_f64()
    );
    assert!(proven <= found, "{what}: proven {proven} above {found}");
    (found, proven)
}

fn status(s: &mut Session, assertion: Value) -> String {
    let out = run(s, "assert.add", assertion);
    out["status"].as_str().unwrap_or_default().to_string()
}

/// The planetary gearbox as published: planet 1 in the ring, with the
/// backlash its assertion asks for (0.3 to 0.7 of jn).
#[test]
fn the_planet_in_its_ring_is_proven_to_its_backlash() {
    let mut s = Session::new(Workspace::unrestricted());
    let path = format!(
        "{}/tests/proof_fixtures/planetary-gearbox.3dcraftasm",
        env!("CARGO_MANIFEST_DIR")
    );
    run(&mut s, "file.open", json!({"path": path}));
    let started = Instant::now();
    let out = run(
        &mut s,
        "assert.add",
        json!({"name": "x", "measure": "clearance", "from": "Planet1", "to": "Ring", "min": "0.3 * jn", "max": "0.7 * jn"}),
    );
    eprintln!(
        "planet in ring: {out} in {:.3} s",
        started.elapsed().as_secs_f64()
    );
    let got = out["status"].as_str().unwrap_or_default();
    assert_eq!(got, "passed");
}

/// docs/tasks/shelled-case.json's solution, at a wall of `wall` mm.
fn shelled_case(wall: f64) -> Session {
    let task: Value = serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/../../docs/tasks/shelled-case.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    let mut s = new();
    for step in task["solution"].as_array().unwrap() {
        let (id, params) = (step[0].as_str().unwrap(), step[1].clone());
        if id == "assert.add" {
            continue;
        }
        run(&mut s, id, params);
    }
    run(
        &mut s,
        "variable.set",
        json!({"name": "wall", "expr": format!("{wall} mm")}),
    );
    s
}

#[test]
fn the_shelled_case_is_proven_to_its_wall() {
    let mut s = shelled_case(2.0);
    let (found, proven) = timed(
        &mut s,
        "measure.thickness",
        json!({"of": "Case"}),
        "thickness",
        "shelled case, wall 2",
    );
    assert!((found - 2.0).abs() < 1e-6, "{found}");
    assert!(proven >= 1.5, "proven only {proven}");
    let got = status(
        &mut s,
        json!({"name": "thin", "measure": "thickness", "of": "Case", "min": "1.5 mm"}),
    );
    assert_eq!(got, "passed");
    // Truly thinner: fails.
    let mut s = shelled_case(1.0);
    let got = status(
        &mut s,
        json!({"name": "thin", "measure": "thickness", "of": "Case", "min": "1.5 mm"}),
    );
    assert_eq!(got, "failed");
}

/// The first face of `of` the filter `where` finds.
fn face(s: &mut Session, of: &str, filter: Value) -> String {
    let mut filter = filter;
    filter["in"] = json!(of);
    let found = run(
        s,
        "select.query",
        json!({"kind": "face", "where": filter.clone()}),
    );
    found["items"][0]["name"]
        .as_str()
        .unwrap_or_else(|| panic!("no face {filter}: {found}"))
        .to_string()
}

/// A clamp: a block 80 × 40 × 30, its upright edges rounded r = 8, a bore
/// of 24 through z with both mouths chamfered `chamfer`, and two through
/// holes of 6.6 at x = 12 and x = 68. The thinnest place is from the
/// chamfer's rim to a side: 20 − 12 − chamfer.
fn clamp(chamfer: f64) -> Session {
    let mut s = new();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Blank", "size": [80, 40, 30]}),
    );
    run(
        &mut s,
        "feature.fillet",
        json!({"name": "Rounded", "radius": 8, "edges": [["F1:face(2)", "F1:face(4)"], ["F1:face(2)", "F1:face(5)"], ["F1:face(3)", "F1:face(4)"], ["F1:face(3)", "F1:face(5)"]]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "BoreTool", "radius": 12, "height": 40, "position": [40, 20, -5]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"name": "Bored", "op": "subtract", "target": "Rounded", "tool": "BoreTool"}),
    );
    let wall = face(
        &mut s,
        "Bored",
        json!({"surface": "cylinder", "radius": "12 mm"}),
    );
    let top = face(
        &mut s,
        "Bored",
        json!({"surface": "plane", "above": {"z": 29}}),
    );
    let bottom = face(
        &mut s,
        "Bored",
        json!({"surface": "plane", "below": {"z": 1}}),
    );
    run(
        &mut s,
        "feature.chamfer",
        json!({"name": "Eased", "distance": chamfer, "edges": [[top, wall.clone()], [bottom, wall]]}),
    );
    let mut last = "Eased".to_string();
    for (i, x) in [12, 68].into_iter().enumerate() {
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": format!("Drill{i}"), "radius": 3.3, "height": 40, "position": [x, 20, -5]}),
        );
        let next = if i == 1 {
            "Clamp".to_string()
        } else {
            format!("Hole{i}")
        };
        run(
            &mut s,
            "feature.boolean",
            json!({"name": next, "op": "subtract", "target": last, "tool": format!("Drill{i}")}),
        );
        last = next;
    }
    s
}

#[test]
fn the_clamp_is_proven_to_its_thinnest_wall() {
    let mut s = clamp(1.5);
    let (found, proven) = timed(
        &mut s,
        "measure.thickness",
        json!({"of": "Clamp"}),
        "thickness",
        "clamp",
    );
    assert!((found - 6.5).abs() < 1e-5, "{found}");
    assert!(proven >= 3.0, "proven only {proven}");
    let got = status(
        &mut s,
        json!({"name": "thin", "measure": "thickness", "of": "Clamp", "min": "3 mm"}),
    );
    assert_eq!(got, "passed");
    let got = status(
        &mut s,
        json!({"name": "thick", "measure": "thickness", "of": "Clamp", "min": "7 mm"}),
    );
    assert_eq!(got, "failed");
}

/// Checks a thickness against its closed form `want`: found to 1e-6 of
/// it, proven no higher than found nor than `want` (to rounding), and to
/// within a thousandth of it; an assertion just below it passes and one
/// just above fails.
fn thickness_is(s: &mut Session, of: &str, want: f64, what: &str) {
    let (found, proven) = timed(s, "measure.thickness", json!({"of": of}), "thickness", what);
    assert!((found - want).abs() <= 1e-6 * want, "{what}: found {found}");
    assert!(proven <= want * (1.0 + 1e-9), "{what}: proven {proven}");
    assert!(
        proven >= want * (1.0 - 1e-3),
        "{what}: proven only {proven}"
    );
    let min = format!("{} mm", 0.99 * want);
    let got = status(
        s,
        json!({"name": "below", "measure": "thickness", "of": of, "min": min}),
    );
    assert_eq!(got, "passed", "{what}");
    let min = format!("{} mm", 1.01 * want);
    let got = status(
        s,
        json!({"name": "above", "measure": "thickness", "of": of, "min": min}),
    );
    assert_eq!(got, "failed", "{what}");
}

/// A hollow ball: radius 10 outside, 8 inside, 2 thick everywhere (a
/// sphere with itself and with the other, concentric).
#[test]
fn a_hollow_ball_is_proven_to_its_wall_at_three_scales() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = new_at(k);
        run(
            &mut s,
            "feature.sphere",
            json!({"name": "Out", "radius": format!("{} mm", 10.0 * k)}),
        );
        run(
            &mut s,
            "feature.sphere",
            json!({"name": "In", "radius": format!("{} mm", 8.0 * k)}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"name": "Ball", "op": "subtract", "target": "Out", "tool": "In"}),
        );
        thickness_is(&mut s, "Ball", 2.0 * k, &format!("hollow ball at {k}"));
    }
}

/// A plate 40 × 40 × 10 with a hole of radius 3 countersunk at 45° from
/// radius 8 at the top: the cone's lower rim is 5 above the bottom, the
/// thinnest place (a cone with a plane, the cone with itself).
#[test]
fn a_countersunk_plate_is_proven_to_its_wall_at_three_scales() {
    for k in [1e-3, 1.0, 1e3] {
        let mm = |x: f64| format!("{} mm", x * k);
        let mut s = new_at(k);
        run(
            &mut s,
            "feature.block",
            json!({"name": "Plate", "size": [mm(40.0), mm(40.0), mm(10.0)], "position": [mm(-20.0), mm(-20.0), mm(0.0)]}),
        );
        // Radius 13 at z = 15 down to 0 at z = 2: radius 8 at the top, 3
        // at z = 5.
        run(
            &mut s,
            "feature.cone",
            json!({"name": "Sink", "radius": mm(13.0), "height": mm(13.0), "position": [mm(0.0), mm(0.0), mm(15.0)], "axis": [0, 0, -1]}),
        );
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": "Drill", "radius": mm(3.0), "height": mm(20.0), "position": [mm(0.0), mm(0.0), mm(-5.0)]}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"name": "Sunk", "op": "subtract", "target": "Plate", "tool": "Sink"}),
        );
        run(
            &mut s,
            "feature.boolean",
            json!({"name": "Holed", "op": "subtract", "target": "Sunk", "tool": "Drill"}),
        );
        thickness_is(
            &mut s,
            "Holed",
            5.0 * k,
            &format!("countersunk plate at {k}"),
        );
    }
}

/// A block whose top is a spline hump, `y = 18 − x² / 50` over x from
/// −20 to 20, extruded 30 along z: 10 thick where the hump meets the
/// sides (a spline face with a plane, with the sides at obtuse edges, and
/// with itself). A valley would meet upright sides at acute edges, where
/// the thickness tends to 0.
#[test]
fn a_spline_hump_is_proven_to_its_thinnest_at_three_scales() {
    for k in [1e-3, 1.0, 1e3] {
        let mm = |x: f64| format!("{} mm", x * k);
        let mut s = new_at(k);
        run(
            &mut s,
            "sketch.create",
            json!({"name": "Profile", "plane": "xy"}),
        );
        run(
            &mut s,
            "sketch.curve",
            json!({"sketch": "Profile", "x": format!("{} * (1 - 2 * t)", mm(20.0)), "y": format!("{} - {} * (1 - 2 * t)^2", mm(18.0), mm(8.0)), "t": [0, 1]}),
        );
        run(
            &mut s,
            "sketch.path",
            json!({"sketch": "Profile", "start": "e2", "segments": [{"line": [mm(-20.0), mm(0.0)]}, {"line": [mm(20.0), mm(0.0)]}, {"line": "e1"}]}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Profile", "distance": mm(30.0), "symmetric": true, "name": "Hump"}),
        );
        thickness_is(&mut s, "Hump", 10.0 * k, &format!("spline hump at {k}"));
    }
}

/// A shaft of radius 50 in a bore 0.01 wider, coaxial and then 0.004
/// off its axis: gaps far inside the mesh's error (a chord of about a
/// tenth), proven to their closed forms.
#[test]
fn gaps_inside_the_mesh_error_are_proven_at_three_scales() {
    for k in [1e-3, 1.0, 1e3] {
        for off in [0.0, 0.004] {
            let mm = |x: f64| format!("{} mm", x * k);
            let mut s = new_at(k);
            run(
                &mut s,
                "feature.cylinder",
                json!({"name": "Shaft", "radius": mm(50.0), "height": mm(20.0), "position": [mm(off), mm(0.0), mm(0.0)]}),
            );
            run(
                &mut s,
                "feature.block",
                json!({"name": "Housing", "size": [mm(140.0), mm(140.0), mm(10.0)], "position": [mm(-70.0), mm(-70.0), mm(5.0)]}),
            );
            run(
                &mut s,
                "feature.cylinder",
                json!({"name": "Cutter", "radius": mm(50.01), "height": mm(40.0), "position": [mm(0.0), mm(0.0), mm(-10.0)]}),
            );
            run(
                &mut s,
                "feature.boolean",
                json!({"name": "Bored", "op": "subtract", "target": "Housing", "tool": "Cutter"}),
            );
            let want = (0.01 - off) * k;
            let what = format!("shaft {off} off at {k}");
            let (found, proven) = timed(
                &mut s,
                "measure.clearance",
                json!({"from": "Shaft", "to": "Bored"}),
                "clearance",
                &what,
            );
            assert!((found - want).abs() <= 1e-6 * want, "{what}: {found}");
            assert!(proven >= want * (1.0 - 1e-3), "{what}: proven {proven}");
            assert!(proven <= want * (1.0 + 1e-9), "{what}: proven {proven}");
        }
    }
}

/// A solid ball of radius 5: every line along the inward normal passes
/// the centre and is 10 long (a sphere with itself: the focal bound and
/// the cones).
#[test]
fn a_solid_ball_is_proven_to_its_diameter_at_three_scales() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = new_at(k);
        run(
            &mut s,
            "feature.sphere",
            json!({"name": "Ball", "radius": format!("{} mm", 5.0 * k)}),
        );
        thickness_is(&mut s, "Ball", 10.0 * k, &format!("solid ball at {k}"));
    }
}

/// A solid cylinder of radius 5 and height 30: its diameter, 10, is the
/// thinnest (the round with itself, with the flat ends square to it).
#[test]
fn a_solid_cylinder_is_proven_to_its_diameter_at_three_scales() {
    for k in [1e-3, 1.0, 1e3] {
        let mm = |x: f64| format!("{} mm", x * k);
        let mut s = new_at(k);
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": "Rod", "radius": mm(5.0), "height": mm(30.0)}),
        );
        thickness_is(&mut s, "Rod", 10.0 * k, &format!("solid cylinder at {k}"));
    }
}

/// Cost (AGENTS.md rule 12): a fat torus, whose proof spends the whole
/// work allowed, and a plate 300 × 300 × 10 with 841 holes of radius 3 on
/// a pitch of 10 (about 1700 faces; the thinnest place the 4 between two
/// holes, 7 to the edges), timed in release; the numbers are in verification.md.
#[test]
fn the_cost_of_a_proof_that_spends_its_work_and_of_a_model_of_many_faces() {
    let mut s = new();
    run(
        &mut s,
        "feature.torus",
        json!({"name": "Ring", "major": 5, "minor": 4}),
    );
    let (_, proven) = timed(
        &mut s,
        "measure.thickness",
        json!({"of": "Ring"}),
        "thickness",
        "fat torus (the whole work)",
    );
    assert!((7.0..=8.0).contains(&proven), "{proven}");
    let mut s = new();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Plate", "size": [300, 300, 10]}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Holes", "plane": "xy", "offset": 10}),
    );
    for i in 0..29 {
        for j in 0..29 {
            run(
                &mut s,
                "sketch.circle",
                json!({"sketch": "Holes", "center": [10 + 10 * i, 10 + 10 * j], "radius": 3}),
            );
        }
    }
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Holes", "distance": -10, "cut": "Plate", "name": "Sieve"}),
    );
    let (found, proven) = timed(
        &mut s,
        "measure.thickness",
        json!({"of": "Sieve"}),
        "thickness",
        "sieve of 841 holes",
    );
    assert!((found - 4.0).abs() < 1e-6, "{found}");
    assert!(proven >= 3.99, "{proven}");
    let started = Instant::now();
    let got = status(
        &mut s,
        json!({"name": "web", "measure": "thickness", "of": "Sieve", "min": "3 mm"}),
    );
    eprintln!(
        "sieve, assertion min 3 mm: {got} in {:.3} s",
        started.elapsed().as_secs_f64()
    );
    assert_eq!(got, "passed");
}
