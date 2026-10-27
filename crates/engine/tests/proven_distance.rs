//! The proven least distance (docs/design/verification.md › *The proven
//! distance*): distances and clearances whose closed form is known are
//! found and proven to it, at three scales, so an assertion at exactly
//! the design gap passes instead of staying not proven. The cases of the
//! R1 test drives: two discs 1 apart (r1-cleanup 77), a pin in a hole
//! (55), flat faces bounded by spline edges (68), spline faces facing
//! one another (42).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{AssertionStatus, Session, Workspace};
use serde_json::{Value, json};
use std::time::Instant;

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn new() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    s
}

/// Measures `id` and checks that the value found is `want` within
/// `found_tol` and the proven bound is within `proof_tol` of the value
/// found (and never above it). Returns (found, proven).
#[allow(clippy::too_many_arguments)]
fn measured(
    s: &mut Session,
    id: &str,
    params: Value,
    key: &str,
    want: f64,
    found_tol: f64,
    proof_tol: f64,
    what: &str,
) -> (f64, f64) {
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
    assert!(
        (found - want).abs() <= found_tol,
        "{what}: found {found}, want {want}"
    );
    assert!(
        proven <= found,
        "{what}: proven {proven} above found {found}"
    );
    // Never above the closed form (to what the geometry is fitted to).
    assert!(
        proven <= want + found_tol,
        "{what}: proven {proven} above the true {want}"
    );
    assert!(
        found - proven <= proof_tol,
        "{what}: proven only {proven} of {found}"
    );
    (found, proven)
}

fn status(s: &mut Session, name: &str) -> AssertionStatus {
    s.assertion_states()
        .into_iter()
        .find(|a| a.name == name)
        .map(|a| a.status)
        .unwrap_or_else(|| panic!("no assertion {name}"))
}

/// Two parallel discs of radius 100 and 15, 1 apart (the repro of
/// r1-cleanup 77: "at least 0.434").
#[test]
fn parallel_discs_are_proven_to_their_gap_at_three_scales() {
    for k in [1e-3, 1.0, 1e6] {
        let mut s = new();
        let mm = |x: f64| format!("{} mm", x * k);
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": mm(100.0), "height": mm(6.0), "axis": [0, 1, 0], "position": [mm(0.0), mm(0.0), mm(0.0)], "name": "P1"}),
        );
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": mm(15.0), "height": mm(6.0), "axis": [0, 1, 0], "position": [mm(150.0), mm(-7.0), mm(0.0)], "name": "P2"}),
        );
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": mm(100.0), "height": mm(6.0), "axis": [0, 1, 0], "position": [mm(0.0), mm(-7.0), mm(0.0)], "name": "P3"}),
        );
        // Within the document's tolerance and the rounding of the value.
        let tol = 1e-6 + 1e-11 * k;
        measured(
            &mut s,
            "measure.clearance",
            json!({"from": "P1", "to": "P3"}),
            "clearance",
            k,
            1e-9 * k,
            tol,
            &format!("discs 1 apart at {k}"),
        );
        // Big disc to small disc: the rims, (150 - 15 - 100) across and 1
        // along the axis.
        let rims = (35.0_f64.powi(2) + 1.0).sqrt() * k;
        measured(
            &mut s,
            "measure.clearance",
            json!({"from": "P1", "to": "P2"}),
            "clearance",
            rims,
            1e-9 * k,
            tol,
            &format!("big disc to small disc at {k}"),
        );
        run(
            &mut s,
            "assert.add",
            json!({"name": "gap", "measure": "clearance", "from": "P1", "to": "P3", "min": mm(1.0)}),
        );
        assert_eq!(status(&mut s, "gap"), AssertionStatus::Passed, "at {k}");
        // A hair over the gap fails: a real place is nearer.
        run(
            &mut s,
            "assert.add",
            json!({"name": "over", "measure": "clearance", "from": "P1", "to": "P3", "min": format!("{} mm", k + (1e-4 * k).max(1e-5))}),
        );
        assert_eq!(status(&mut s, "over"), AssertionStatus::Failed, "at {k}");
    }
}

/// A plate 40 × 40 × 10 with a hole of radius 10 and a pin of radius 4
/// in it: coaxial (6 all round), flush with the plate, and off the axis
/// by 2 (4 on the near side).
#[test]
fn a_pin_in_a_hole_is_proven_to_its_clearance_at_three_scales() {
    for k in [1e-3, 1.0, 1e6] {
        for (name, pin_at, z0, height, want) in [
            ("through", 20.0, -10.0, 30.0, 6.0),
            ("flush", 20.0, 0.0, 10.0, 6.0),
            ("off the axis", 22.0, -10.0, 30.0, 4.0),
        ] {
            let mut s = new();
            let mm = |x: f64| format!("{} mm", x * k);
            run(
                &mut s,
                "feature.block",
                json!({"name": "Stock", "size": [mm(40.0), mm(40.0), mm(10.0)]}),
            );
            run(
                &mut s,
                "feature.cylinder",
                json!({"name": "Drill", "radius": mm(10.0), "height": mm(30.0), "position": [mm(20.0), mm(20.0), mm(-10.0)]}),
            );
            run(
                &mut s,
                "feature.boolean",
                json!({"op": "subtract", "target": "Stock", "tool": "Drill", "name": "Plate"}),
            );
            run(
                &mut s,
                "feature.cylinder",
                json!({"name": "Pin", "radius": mm(4.0), "height": mm(height), "position": [mm(pin_at), mm(20.0), mm(z0)]}),
            );
            let tol = 1e-6 + 1e-11 * k;
            measured(
                &mut s,
                "measure.clearance",
                json!({"from": "Pin", "to": "Plate"}),
                "clearance",
                want * k,
                1e-9 * k,
                tol,
                &format!("pin {name} at {k}"),
            );
            run(
                &mut s,
                "assert.add",
                json!({"name": "pin", "measure": "clearance", "from": "Pin", "to": "Plate", "min": mm(want)}),
            );
            assert_eq!(
                status(&mut s, "pin"),
                AssertionStatus::Passed,
                "{name} at {k}"
            );
        }
    }
}

/// A sphere above a block, and two crossed cylinders: isolated nearest
/// points on curved faces.
#[test]
fn round_faces_are_proven_to_their_distance() {
    for k in [1e-3, 1.0, 1e6] {
        let mut s = new();
        let mm = |x: f64| format!("{} mm", x * k);
        run(
            &mut s,
            "feature.block",
            json!({"name": "Base", "size": [mm(20.0), mm(20.0), mm(10.0)]}),
        );
        run(
            &mut s,
            "feature.sphere",
            json!({"name": "Ball", "radius": mm(5.0), "position": [mm(10.0), mm(10.0), mm(18.0)]}),
        );
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": "Across", "radius": mm(2.0), "height": mm(40.0), "axis": [1, 0, 0], "position": [mm(-10.0), mm(50.0), mm(0.0)]}),
        );
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": "Along", "radius": mm(3.0), "height": mm(40.0), "axis": [0, 1, 0], "position": [mm(5.0), mm(30.0), mm(10.0)]}),
        );
        let tol = 1e-6 + 1e-11 * k;
        measured(
            &mut s,
            "measure.clearance",
            json!({"from": "Ball", "to": "Base"}),
            "clearance",
            3.0 * k,
            1e-9 * k,
            tol,
            &format!("ball over the base at {k}"),
        );
        // Axes 10 apart (z = 0 and z = 10), crossing at right angles.
        measured(
            &mut s,
            "measure.clearance",
            json!({"from": "Across", "to": "Along"}),
            "clearance",
            5.0 * k,
            1e-8 * k,
            tol,
            &format!("crossed cylinders at {k}"),
        );
    }
}

/// The repro of r1-cleanup 68: a slot's two flat walls, each bounded by
/// a spline edge where it meets the plate's curved top ("found 4.15 mm
/// (at least 3.26 mm)").
#[test]
fn flat_faces_bounded_by_spline_edges_are_proven_to_their_gap() {
    let mut s = new();
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Plate", "plane": "yz"}),
    );
    run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "Plate", "x": "100 mm * (1 - 2 * t)", "y": "80 mm + 30 mm * (1 - 2 * t)^2", "t": [0, 1]}),
    );
    run(
        &mut s,
        "sketch.path",
        json!({"sketch": "Plate", "start": "e2", "segments": [{"line": ["-100 mm", "0 mm"]}, {"line": ["100 mm", "0 mm"]}, {"line": "e1"}]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Plate", "distance": "4 mm", "symmetric": true, "name": "Slab"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": ["20 mm", "4.15 mm", "100 mm"], "position": ["-10 mm", "20 mm", "40 mm"], "name": "Slot"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Slab", "tool": "Slot", "name": "Slotted"}),
    );
    measured(
        &mut s,
        "measure.distance",
        json!({"from": {"kind": "face", "where": [{"in": "Slotted"}, {"normal": {"along": "y"}}]},
               "to": {"kind": "face", "where": [{"in": "Slotted"}, {"normal": {"along": "-y"}}]}}),
        "distance",
        4.15,
        1e-9,
        5e-7,
        "slot walls",
    );
}

/// Spline faces: the top of a plate shaped as a parabola under the same
/// parabola turned over, 2 apart at the crest: nearest places on freeform
/// faces, as between gear flanks.
#[test]
fn spline_faces_are_proven_close_to_their_distance() {
    let mut s = new();
    for (name, base, top, sign) in [
        ("Low", "0 mm", "10 mm", "-"),
        ("High", "22 mm", "12 mm", "+"),
    ] {
        run(
            &mut s,
            "sketch.create",
            json!({"name": name, "plane": "yz"}),
        );
        run(
            &mut s,
            "sketch.curve",
            json!({"sketch": name, "x": "20 mm * (1 - 2 * t)", "y": format!("{top} {sign} 8 mm * (1 - 2 * t)^2"), "t": [0, 1]}),
        );
        run(
            &mut s,
            "sketch.path",
            json!({"sketch": name, "start": "e2", "segments": [{"line": ["-20 mm", base]}, {"line": ["20 mm", base]}, {"line": "e1"}]}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": name, "distance": "6 mm", "symmetric": true, "name": format!("{name}Part")}),
        );
    }
    measured(
        &mut s,
        "measure.clearance",
        json!({"from": "LowPart", "to": "HighPart"}),
        "clearance",
        2.0,
        1e-6,
        1e-4,
        "parabola under parabola",
    );
}

/// Two involutes of one base circle (radius 20) turned 0.05 rad apart are
/// parallel curves, `20 × 0.05 = 1` apart along every common normal: the
/// flanks of two teeth with backlash (r1-cleanup 42 and 71), a spline
/// face at one distance from another all along its profile.
#[test]
fn involute_flanks_are_proven_close_to_their_backlash() {
    let mut s = new();
    for (name, turn, ends) in [
        ("A", "0", [["26 mm", "-5 mm"], ["20 mm", "-5 mm"]]),
        ("B", "0.05", [["25 mm", "10 mm"], ["19 mm", "10 mm"]]),
    ] {
        run(
            &mut s,
            "sketch.create",
            json!({"name": name, "plane": "xy"}),
        );
        let at = format!("({turn} + t) * 1 rad - atan(t)");
        run(
            &mut s,
            "sketch.curve",
            json!({"sketch": name, "x": format!("20 mm * sqrt(1 + t^2) * cos({at})"), "y": format!("20 mm * sqrt(1 + t^2) * sin({at})"), "t": [0.2, 0.8]}),
        );
        run(
            &mut s,
            "sketch.path",
            json!({"sketch": name, "start": "e2", "segments": [{"line": ends[0]}, {"line": ends[1]}, {"line": "e1"}]}),
        );
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": name, "distance": "6 mm", "symmetric": true, "name": format!("Tooth{name}")}),
        );
    }
    measured(
        &mut s,
        "measure.clearance",
        json!({"from": "ToothA", "to": "ToothB"}),
        "clearance",
        1.0,
        1e-5,
        1e-5,
        "involute flanks",
    );
}

/// A ball over a hole: under it the plate's top face has a hole, so only
/// telling its pieces in from out (by their middles) keeps the plane
/// under the ball from counting; the nearest place is the rim, at
/// √(10² + 2²) − 3.
#[test]
fn a_ball_over_a_hole_is_measured_to_the_rim() {
    let mut s = new();
    run(
        &mut s,
        "feature.block",
        json!({"name": "Stock", "size": ["40 mm", "40 mm", "10 mm"]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": "10 mm", "height": "30 mm", "position": ["20 mm", "20 mm", "-10 mm"]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Stock", "tool": "Drill", "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "Ball", "radius": "3 mm", "position": ["20 mm", "20 mm", "12 mm"]}),
    );
    let want = 104.0_f64.sqrt() - 3.0;
    measured(
        &mut s,
        "measure.clearance",
        json!({"from": "Ball", "to": "Plate"}),
        "clearance",
        want,
        1e-9,
        1e-6,
        "ball over a hole",
    );
}

/// A pin in a hole a million from the origin (frames far from where the
/// pieces are: the margin of an axis that is one only to rounding).
#[test]
fn a_pin_far_from_the_origin_is_proven_no_higher_than_it_is() {
    let mut s = new();
    let at = |x: f64| format!("{} mm", 1e6 + x);
    run(
        &mut s,
        "feature.block",
        json!({"name": "Stock", "size": ["40 mm", "40 mm", "10 mm"], "position": [at(0.0), at(0.0), "0 mm"]}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "radius": "10 mm", "height": "30 mm", "position": [at(20.0), at(20.0), "-10 mm"]}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Stock", "tool": "Drill", "name": "Plate"}),
    );
    run(
        &mut s,
        "feature.cylinder",
        json!({"name": "Pin", "radius": "4 mm", "height": "30 mm", "position": [at(22.0), at(20.0), "-10 mm"]}),
    );
    measured(
        &mut s,
        "measure.clearance",
        json!({"from": "Pin", "to": "Plate"}),
        "clearance",
        4.0,
        1e-8,
        1e-6,
        "pin off its axis at 1e6",
    );
}

/// A dome turned from a spline (a revolved spline: a rational surface)
/// under a lid, 1 apart at its crest on the axis.
#[test]
fn a_turned_spline_dome_is_proven_to_its_distance() {
    let mut s = new();
    run(
        &mut s,
        "sketch.create",
        // The sketch's x along z (the axis turned about), its y along x.
        json!({"name": "Dome", "plane": "zx"}),
    );
    run(
        &mut s,
        "sketch.curve",
        json!({"sketch": "Dome", "x": "10 mm - 8 mm * t^2", "y": "20 mm * t", "t": [0, 1]}),
    );
    run(
        &mut s,
        "sketch.path",
        json!({"sketch": "Dome", "start": "e2", "segments": [{"line": ["0 mm", "20 mm"]}, {"line": ["0 mm", "0 mm"]}, {"line": "e1"}]}),
    );
    run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Dome", "axis": "x", "name": "Turned"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Lid", "size": ["10 mm", "10 mm", "2 mm"], "position": ["-5 mm", "-5 mm", "11 mm"]}),
    );
    measured(
        &mut s,
        "measure.clearance",
        json!({"from": "Turned", "to": "Lid"}),
        "clearance",
        1.0,
        1e-6,
        1e-4,
        "turned dome under a lid",
    );
}

/// A tolerant body (a STEP block with a corner 1e-4 off its edges,
/// imported loose) under a ball: its faces' outlines may lie that far
/// from their edges, which telling pieces in from out allows for.
#[test]
fn a_loose_import_is_proven_no_higher_than_it_is() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("proven-distance-loose");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "feature.block", json!({"size": [10, 20, 30]}));
    let path = dir.join("block.step");
    run(
        &mut s,
        "file.export",
        json!({"path": path.to_string_lossy()}),
    );
    let text = std::fs::read_to_string(&path).unwrap();
    let moved = text.replacen(
        "CARTESIAN_POINT('',(10.,",
        "CARTESIAN_POINT('',(10.0001,",
        1,
    );
    assert_ne!(moved, text);
    let loose = dir.join("loose.step");
    std::fs::write(&loose, moved).unwrap();
    let mut s = Session::new(Workspace::unrestricted());
    run(&mut s, "file.new", json!({"units": "mm"}));
    let imported = run(
        &mut s,
        "file.import",
        json!({"path": loose.to_string_lossy()}),
    );
    assert!(
        imported["bodies"][0]["import"]["looseVertices"]
            .as_u64()
            .unwrap()
            >= 1,
        "{imported}"
    );
    // A ball over the middle of the top (z = 30), clear of the corner.
    run(
        &mut s,
        "feature.sphere",
        json!({"name": "Ball", "radius": "2 mm", "position": ["5 mm", "10 mm", "34 mm"]}),
    );
    measured(
        &mut s,
        "measure.clearance",
        json!({"from": "Ball", "to": "F1"}),
        "clearance",
        2.0,
        1e-6,
        1e-3,
        "ball over a loose block",
    );
}
