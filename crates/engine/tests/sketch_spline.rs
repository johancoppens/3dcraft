//! Splines in sketches, through the commands: drawn, constrained, part of
//! a profile, extruded into a solid with a B-spline wall, cut, saved and
//! reopened.
//!
//! The oracle for areas is independent of the kernel: Green's theorem
//! along the spline, evaluated by de Boor's algorithm on its knots and
//! integrated by Simpson's rule on a fine subdivision of every knot span.
//! The volume of an extrusion is that area times its depth.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{body, error, run, session_in};
use craft3d_engine::{Session, SketchCurve, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;

type P = [f64; 2];

fn workdir(name: &str) -> PathBuf {
    common::workdir("spline", name)
}

fn session() -> Session {
    Session::new(Workspace::none())
}

/// The status and the error of a feature.
fn status(session: &mut Session, name: &str) -> (String, String) {
    let document = run(session, "document.inspect", json!({}));
    let feature = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no feature {name}"))
        .clone();
    (
        feature["status"].as_str().unwrap().to_string(),
        feature["error"].as_str().unwrap_or("").to_string(),
    )
}

fn face_names(session: &mut Session, name: &str) -> Vec<String> {
    let mut names: Vec<String> = body(session, name)["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

fn close(actual: f64, expected: f64, relative: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{what}: {actual} against {expected} (off by {:e})",
        ((actual - expected) / expected).abs()
    );
}

// ------------------------------------------------------------- the oracle

fn knots(degree: usize, count: usize) -> Vec<f64> {
    let spans = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    knots
}

/// De Boor's algorithm: the point at knot value `u`.
fn de_boor(degree: usize, knots: &[f64], control: &[P], u: f64) -> P {
    let n = control.len();
    let k = (degree..n).rev().find(|&k| knots[k] <= u).unwrap_or(degree);
    let mut d: Vec<P> = control[k - degree..=k].to_vec();
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = k - degree + j;
            let alpha = (u - knots[i]) / (knots[i + degree + 1 - r] - knots[i]);
            d[j] = [
                d[j - 1][0] + (d[j][0] - d[j - 1][0]) * alpha,
                d[j - 1][1] + (d[j][1] - d[j - 1][1]) * alpha,
            ];
        }
    }
    d[degree]
}

/// The point of a sketch spline at `t` from 0 to 1.
fn spline_point(degree: usize, control: &[P], t: f64) -> P {
    de_boor(
        degree,
        &knots(degree, control.len()),
        control,
        t.clamp(0.0, 1.0 - 1e-15),
    )
}

/// `∫ (x dy − y dx) / 2` along a spline of degree 2 or more, from its
/// start to its end: composite Simpson on 2000 steps per knot span, the
/// derivative being the spline of one degree less on the differences of
/// the control points.
fn swept(degree: usize, control: &[P]) -> f64 {
    let knots = knots(degree, control.len());
    let derivative: Vec<P> = (0..control.len() - 1)
        .map(|i| {
            let scale = degree as f64 / (knots[i + degree + 1] - knots[i + 1]);
            [
                (control[i + 1][0] - control[i][0]) * scale,
                (control[i + 1][1] - control[i][1]) * scale,
            ]
        })
        .collect();
    let inner = &knots[1..knots.len() - 1];
    let mut sum = 0.0;
    let steps = 2000;
    for span in 0..control.len() - degree {
        let (a, b) = (knots[degree + span], knots[degree + span + 1]);
        let h = (b - a) / steps as f64;
        for i in 0..=steps {
            let u = (a + h * i as f64).clamp(a + (b - a) * 1e-14, b - (b - a) * 1e-14);
            let p = de_boor(degree, &knots, control, u);
            let d = de_boor(degree - 1, inner, &derivative, u);
            let weight = if i == 0 || i == steps {
                1.0
            } else if i % 2 == 1 {
                4.0
            } else {
                2.0
            };
            sum += (p[0] * d[1] - p[1] * d[0]) / 2.0 * weight * h / 3.0;
        }
    }
    sum
}

/// The same along a straight line.
fn swept_line(a: P, b: P) -> f64 {
    (a[0] * b[1] - a[1] * b[0]) / 2.0
}

fn scaled(points: &[P], k: f64) -> Vec<P> {
    points.iter().map(|p| [p[0] * k, p[1] * k]).collect()
}

/// The bulge of the "D": from (0, 0) out to the right and back to (0, 6).
const BULGE: [P; 5] = [[0.0, 0.0], [5.0, -1.0], [7.0, 3.0], [5.0, 7.0], [0.0, 6.0]];
/// A leaf between (0, 0) and (10, 0): its upper and its lower side.
const UPPER: [P; 6] = [
    [0.0, 0.0],
    [1.0, 3.0],
    [4.0, 4.0],
    [7.0, 2.5],
    [9.0, 2.0],
    [10.0, 0.0],
];
const LOWER: [P; 4] = [[0.0, 0.0], [3.0, -2.0], [8.0, -1.5], [10.0, 0.0]];

fn new_document(session: &mut Session, k: f64) {
    run(session, "file.new", json!({"tolerance": 1e-6 * k}));
    run(session, "sketch.create", json!({"name": "Profile"}));
}

/// The D in sketch `Profile`: the spline e6 on the points e1..e5 and the
/// line e7 from e5 back to e1. Returns its area.
fn draw_d(session: &mut Session, k: f64, degree: usize) -> f64 {
    let bulge = scaled(&BULGE, k);
    let made = run(
        session,
        "sketch.spline",
        json!({"sketch": "Profile", "control": bulge, "degree": degree}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    assert_eq!(
        (&made["entity"], &made["start"], &made["end"]),
        (&json!("e6"), &json!("e1"), &json!("e5"))
    );
    assert_eq!(made["control"], json!(["e1", "e2", "e3", "e4", "e5"]));
    assert_eq!(made["degree"], degree);
    assert_eq!(made["dof"], 10, "five free points");
    let line = run(
        session,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e5", "to": "e1"}),
    );
    assert_eq!(line["entity"], "e7");
    swept(degree, &bulge) + swept_line(bulge[4], bulge[0])
}

// -------------------------------------------------------------- the tests

#[test]
fn a_d_with_a_spline_bulge_becomes_a_solid_of_the_predicted_volume() {
    for k in [1e-3, 1.0, 1e3] {
        for degree in [2, 3, 4] {
            let name = format!("degree {degree} x{k}");
            let dir = workdir(&format!("d-{degree}-{k}"));
            let mut s = session_in(&dir);
            new_document(&mut s, k);
            let area = draw_d(&mut s, k, degree);
            let depth = 2.5 * k;
            let made = run(
                &mut s,
                "feature.extrude",
                json!({"sketch": "Profile", "distance": depth, "name": "Hull"}),
            );
            assert_eq!(made, json!({"id": "F2", "name": "Hull", "status": "ok"}));
            let solid = body(&mut s, "Hull");
            assert_eq!(solid["kind"], "solid");
            // The wall of the spline, the wall of the line and two caps;
            // two corners at each end.
            assert_eq!(
                (&solid["faces"], &solid["edges"], &solid["vertices"]),
                (&json!(4), &json!(6), &json!(4)),
                "{name}"
            );
            close(solid["volume"].as_f64().unwrap(), area * depth, 1e-9, &name);
            assert_eq!(
                face_names(&mut s, "Hull"),
                [
                    "F2:endCap(e6,e7)",
                    "F2:side(e6)",
                    "F2:side(e7)",
                    "F2:startCap(e6,e7)"
                ],
                "{name}"
            );
            let check = run(&mut s, "document.check", json!({"deep": true}));
            assert_eq!(check["ok"], true, "{name}: {check}");

            // Watertight as a mesh, and the same after saving and opening.
            let exported = run(
                &mut s,
                "file.export",
                json!({"path": "hull.3mf", "tolerance": 1e-3 * k}),
            );
            assert_eq!(
                (&exported["bodies"], &exported["closed"]),
                (&json!(1), &json!(1)),
                "{name}"
            );
            let before = run(&mut s, "document.measure", json!({}));
            let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
            run(&mut s, "file.save", json!({"path": "hull.3dcraft"}));
            run(&mut s, "file.close", json!({}));
            run(&mut s, "file.open", json!({"path": "hull.3dcraft"}));
            assert_eq!(run(&mut s, "document.measure", json!({})), before, "{name}");
            assert_eq!(
                run(&mut s, "sketch.inspect", json!({"sketch": "Profile"})),
                inspected,
                "{name}"
            );
            assert_eq!(face_names(&mut s, "Hull").len(), 4);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[test]
fn moving_a_control_point_reshapes_the_solid_and_renames_nothing() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = session();
        new_document(&mut s, k);
        let area = draw_d(&mut s, k, 3);
        let depth = 2.0 * k;
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Profile", "distance": depth, "name": "Hull"}),
        );
        // A sketch on the far cap, and one on the flat side.
        for (sketch, face) in [("Seat", "F2:endCap(e6,e7)"), ("Flank", "F2:side(e7)")] {
            let made = run(
                &mut s,
                "sketch.create",
                json!({"name": sketch, "face": face}),
            );
            assert_eq!(made["status"], "ok", "{made}");
        }
        let names = face_names(&mut s, "Hull");
        let volume = |s: &mut Session| body(s, "Hull")["volume"].as_f64().unwrap();
        close(volume(&mut s), area * depth, 1e-9, "before");

        // The middle control point further out: a fuller bulge.
        let moved = run(
            &mut s,
            "sketch.edit",
            json!({"sketch": "Profile", "entity": "e3", "at": [9.0 * k, 3.5 * k]}),
        );
        assert_eq!(moved["status"], "ok", "{moved}");
        let mut fuller = scaled(&BULGE, k);
        fuller[2] = [9.0 * k, 3.5 * k];
        let fuller_area = swept(3, &fuller) + swept_line(fuller[4], fuller[0]);
        assert!(fuller_area > area * 1.05);
        close(volume(&mut s), fuller_area * depth, 1e-9, "after the move");
        assert_eq!(face_names(&mut s, "Hull"), names, "x{k}");
        for sketch in ["Seat", "Flank"] {
            assert_eq!(status(&mut s, sketch).0, "ok", "{sketch} x{k}");
        }
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
        let spline = &inspected["entities"][5];
        assert_eq!(
            spline,
            &json!({"id": "e6", "type": "spline", "construction": false, "degree": 3,
                    "control": ["e1", "e2", "e3", "e4", "e5"], "closed": false})
        );
        assert_eq!(inspected["entities"][2]["x"], 9.0 * k);

        // Undo and redo, one step each.
        run(&mut s, "edit.undo", json!({}));
        close(volume(&mut s), area * depth, 1e-9, "after undo");
        run(&mut s, "edit.redo", json!({}));
        close(volume(&mut s), fuller_area * depth, 1e-9, "after redo");
        // The spline itself cannot be moved, only its points.
        let refused = error(
            &mut s,
            "sketch.edit",
            json!({"sketch": "Profile", "entity": "e6", "at": [0, 0]}),
        );
        assert!(
            refused.contains("spline") && refused.contains("only a point"),
            "{refused}"
        );
        // Pulled through the straight side, the spline crosses it: the
        // solid fails with the reason and comes back when the point does.
        run(
            &mut s,
            "sketch.edit",
            json!({"sketch": "Profile", "entity": "e3", "at": [-6.0 * k, 3.5 * k]}),
        );
        let (state, reason) = status(&mut s, "Hull");
        assert_eq!(state, "failed");
        assert!(
            reason.contains("e6") && reason.contains("e7") && reason.contains("cross"),
            "{reason}"
        );
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(status(&mut s, "Hull").0, "ok");

        // Deleting: the points of a spline stay until it is gone.
        let in_use = error(
            &mut s,
            "sketch.delete",
            json!({"sketch": "Profile", "entity": "e3"}),
        );
        assert!(in_use.contains("e6"), "{in_use}");
        run(
            &mut s,
            "sketch.delete",
            json!({"sketch": "Profile", "entity": "e6"}),
        );
        let (state, reason) = status(&mut s, "Hull");
        assert_eq!(state, "failed");
        assert!(
            reason.contains("not closed") && reason.contains("e7"),
            "{reason}"
        );
        run(
            &mut s,
            "sketch.delete",
            json!({"sketch": "Profile", "entity": "e3"}),
        );
        run(&mut s, "edit.undo", json!({}));
        run(&mut s, "edit.undo", json!({}));
        close(
            volume(&mut s),
            fuller_area * depth,
            1e-9,
            "after undoing the deletes",
        );
    }
}

#[test]
fn two_splines_close_a_profile_and_a_circle_is_a_hole_in_it() {
    for k in [1e-3, 1.0, 1e3] {
        let name = format!("x{k}");
        let mut s = session();
        new_document(&mut s, k);
        let (upper, lower) = (scaled(&UPPER, k), scaled(&LOWER, k));
        let first = run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Profile", "control": upper}),
        );
        assert_eq!(first["entity"], "e7");
        // The lower side shares both ends with the upper one.
        let second = run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Profile", "degree": 2,
                   "control": ["e1", lower[1], lower[2], "e6"]}),
        );
        assert_eq!(second["control"], json!(["e1", "e8", "e9", "e6"]));
        assert_eq!(second["entity"], "e10");
        // Counter-clockwise the leaf runs along the lower side and back
        // against the upper one.
        let leaf = swept(2, &lower) - swept(3, &upper);
        let depth = 1.5 * k;
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Profile", "distance": depth, "symmetric": true, "name": "Leaf"}),
        );
        let solid = body(&mut s, "Leaf");
        assert_eq!(solid["faces"], 4, "{name}");
        close(solid["volume"].as_f64().unwrap(), leaf * depth, 1e-9, &name);
        assert_eq!(
            face_names(&mut s, "Leaf"),
            [
                "F2:endCap(e7,e10)",
                "F2:side(e10)",
                "F2:side(e7)",
                "F2:startCap(e7,e10)"
            ]
        );

        // A hole drawn afterwards adds a wall and renames none.
        let radius = 1.2 * k;
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Profile", "center": [5.0 * k, 1.0 * k], "radius": radius}),
        );
        let solid = body(&mut s, "Leaf");
        assert_eq!(solid["faces"], 5, "{name}");
        close(
            solid["volume"].as_f64().unwrap(),
            (leaf - PI * radius * radius) * depth,
            1e-9,
            &format!("{name} with the hole"),
        );
        assert!(face_names(&mut s, "Leaf").contains(&"F2:side(e12)".to_string()));
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{name}: {check}");
        let picture = run(&mut s, "view.render", json!({"width": 96, "height": 64}));
        assert!(picture["coverage"].as_f64().unwrap() > 0.02, "{name}");

        // A circle across the upper side is no hole: the solid fails and
        // says which two cross.
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Profile", "center": [4.0 * k, 3.5 * k], "radius": 1.0 * k}),
        );
        let (state, reason) = status(&mut s, "Leaf");
        assert_eq!(state, "failed", "{name}");
        assert!(reason.contains("e7") && reason.contains("e14"), "{reason}");
        assert!(
            run(&mut s, "document.inspect", json!({}))["bodies"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn a_spline_through_points_passes_through_them() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = session();
        new_document(&mut s, k);
        // A hull section: keel at the origin, up the side to the deck.
        let through = scaled(
            &[
                [0.0, 0.0],
                [3.0, 0.4],
                [5.5, 1.8],
                [6.6, 4.0],
                [6.8, 6.5],
                [6.0, 9.0],
            ],
            k,
        );
        for degree in [2_usize, 3, 5] {
            let made = run(
                &mut s,
                "sketch.spline",
                json!({"sketch": "Profile", "through": through, "degree": degree,
                       "construction": true}),
            );
            let control: Vec<String> = made["control"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap().to_string())
                .collect();
            assert_eq!(control.len(), through.len());
            let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
            let at = |id: &str| -> P {
                let entity = inspected["entities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|e| e["id"] == id)
                    .unwrap();
                [entity["x"].as_f64().unwrap(), entity["y"].as_f64().unwrap()]
            };
            let points: Vec<P> = control.iter().map(|id| at(id)).collect();
            // The ends are the given ends; the curve takes each point at
            // the Greville abscissa of its control point.
            assert_eq!(points[0], through[0]);
            assert_eq!(points[5], through[5]);
            let knots = knots(degree, 6);
            for (i, wanted) in through.iter().enumerate() {
                let u = knots[i + 1..=i + degree].iter().sum::<f64>() / degree as f64;
                let got = spline_point(degree, &points, u);
                let off = (got[0] - wanted[0]).phypot(got[1] - wanted[1]);
                assert!(
                    off <= 1e-12 * 9.0 * k,
                    "degree {degree} x{k}, point {i}: off by {off:e}"
                );
            }
        }
        // Through existing points: the ends are shared, the points in
        // between are read for where they are.
        let made = run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Profile", "through": ["e1", "e2", "e3", "e6"]}),
        );
        let control = made["control"].as_array().unwrap();
        assert_eq!((&control[0], &control[3]), (&json!("e1"), &json!("e6")));
        assert!(control[1] != "e2" && control[2] != "e3", "{made}");

        // Closed by a line and extruded, the solid has the predicted
        // volume.
        let mut s = session();
        new_document(&mut s, k);
        let made = run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Profile", "through": through}),
        );
        assert_eq!(made["control"], json!(["e1", "e2", "e3", "e4", "e5", "e6"]));
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Profile", "points": ["e6", [0.0, 9.0 * k], "e1"]}),
        );
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
        let points: Vec<P> = (0..6)
            .map(|i| {
                let entity = &inspected["entities"][i];
                [entity["x"].as_f64().unwrap(), entity["y"].as_f64().unwrap()]
            })
            .collect();
        let top = [0.0, 9.0 * k];
        let area = swept(3, &points) + swept_line(points[5], top) + swept_line(top, points[0]);
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Profile", "distance": 4.0 * k, "name": "Half"}),
        );
        close(
            body(&mut s, "Half")["volume"].as_f64().unwrap(),
            area * 4.0 * k,
            1e-9,
            &format!("x{k}"),
        );
    }
}

#[test]
fn a_spline_follows_the_constraints_on_its_points() {
    for k in [1e-3, 1.0, 1e3] {
        let name = format!("x{k}");
        let mut s = session();
        new_document(&mut s, k);
        // A base line e3 (e1 to e2), a spline e7 from e2 over e4, e5 to
        // e6, and a line e8 from e6 back to e1.
        run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Profile", "from": [0, 0], "to": [10.0 * k, 0.3 * k]}),
        );
        let made = run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Profile",
                   "control": ["e2", [13.0 * k, 2.0 * k], [9.0 * k, 7.0 * k], [0.5 * k, 6.0 * k]]}),
        );
        assert_eq!(made["entity"], "e7");
        // Five points, two coordinates each: the spline adds none.
        assert_eq!(made["dof"], 10, "{name}");
        run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Profile", "from": "e6", "to": "e1"}),
        );
        let mut dof = 10;
        let mut constrain =
            |s: &mut Session, kind: &str, entities: Value, value: Option<f64>, takes: i64| {
                let mut params = json!({"sketch": "Profile", "type": kind, "entities": entities});
                if let Some(value) = value {
                    params["value"] = value.into();
                }
                let result = run(s, "sketch.constrain", params);
                assert_eq!(result["status"], "ok", "{kind}: {result}");
                assert!(result.get("conflicting").is_none(), "{kind}: {result}");
                dof -= takes;
                assert_eq!(result["dof"], dof, "{kind} x{k}: {result}");
            };
        constrain(&mut s, "fixed", json!(["e1"]), None, 2);
        constrain(&mut s, "horizontal", json!(["e3"]), None, 1);
        constrain(&mut s, "distance", json!(["e3"]), Some(10.0 * k), 1);
        constrain(&mut s, "vertical", json!(["e8"]), None, 1);
        constrain(&mut s, "distance", json!(["e1", "e6"]), Some(6.0 * k), 1);
        // Control points take constraints like any point.
        constrain(&mut s, "vertical", json!(["e2", "e4"]), None, 1);
        constrain(&mut s, "distance", json!(["e2", "e4"]), Some(2.0 * k), 1);
        constrain(&mut s, "horizontal", json!(["e5", "e6"]), None, 1);
        constrain(&mut s, "distance", json!(["e5", "e6"]), Some(9.0 * k), 1);
        // Every point is placed now. A tangency to the vertical line e8 at
        // e6 asks the last leg (e6 to e5) to be vertical, against the
        // horizontal constraint on those two points: the solver blames
        // the constraint that came last.
        let clash = run(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "tangent", "entities": ["e7", "e8"]}),
        );
        assert_eq!(clash["conflicting"], json!(["c10"]), "{name}: {clash}");
        run(&mut s, "edit.undo", json!({}));
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
        let at = |id: &str| -> P {
            let entity = inspected["entities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == id)
                .unwrap();
            [entity["x"].as_f64().unwrap(), entity["y"].as_f64().unwrap()]
        };
        let expected: [(&str, P); 6] = [
            ("e1", [0.0, 0.0]),
            ("e2", [10.0, 0.0]),
            ("e4", [10.0, 2.0]),
            ("e5", [9.0, 6.0]),
            ("e6", [0.0, 6.0]),
            ("e1", [0.0, 0.0]),
        ];
        for (id, wanted) in expected {
            let got = at(id);
            assert!(
                (got[0] - wanted[0] * k).phypot(got[1] - wanted[1] * k) <= 1e-5 * k,
                "{id} {name}: {got:?}"
            );
        }
        assert_eq!(inspected["dof"], 0, "{name}: {inspected}");

        // The solid is that of the solved points.
        let control = [at("e2"), at("e4"), at("e5"), at("e6")];
        let area =
            swept_line(at("e1"), at("e2")) + swept(3, &control) + swept_line(at("e6"), at("e1"));
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Profile", "distance": 1.0 * k, "name": "Part"}),
        );
        close(
            body(&mut s, "Part")["volume"].as_f64().unwrap(),
            area * k,
            1e-9,
            &name,
        );
        // A dimension moves a control point, and the solid follows.
        run(
            &mut s,
            "sketch.edit",
            json!({"sketch": "Profile", "constraint": "c7", "value": 4.0 * k}),
        );
        let after = body(&mut s, "Part")["volume"].as_f64().unwrap();
        let mut taller = control;
        taller[1] = [10.0 * k, 4.0 * k];
        let taller_area =
            swept_line(at("e1"), at("e2")) + swept(3, &taller) + swept_line(at("e6"), at("e1"));
        close(
            after,
            taller_area * k,
            1e-6,
            &format!("{name} after the dimension"),
        );
    }
}

/// The corner of a line and a spline, or of an arc and a spline, made
/// smooth by a tangent constraint.
#[test]
fn a_spline_is_made_tangent_to_the_curve_it_shares_an_end_with() {
    for k in [1e-3, 1.0, 1e3] {
        let name = format!("x{k}");
        let mut s = session();
        new_document(&mut s, k);
        // Line e3 from e1 (0, 0) to e2 (10, 0); spline e7 from e2 over e4,
        // e5 to e6 (0, 6); arc e9 about e8 from e6 round the left to e1.
        run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Profile", "from": [0, 0], "to": [10.0 * k, 0]}),
        );
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Profile",
                   "control": ["e2", [12.0 * k, 3.0 * k], [6.0 * k, 8.0 * k], [0.0, 6.0 * k]]}),
        );
        run(
            &mut s,
            "sketch.arc",
            json!({"sketch": "Profile", "center": [0.0, 3.0 * k], "start": "e6", "end": "e1"}),
        );
        for point in ["e1", "e2", "e6", "e8"] {
            run(
                &mut s,
                "sketch.constrain",
                json!({"sketch": "Profile", "type": "fixed", "entities": [point]}),
            );
        }
        let before = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
        assert_eq!(before["dof"], 4, "{name}: the two inner control points");
        // Tangent to the line at e2: the leg e2-e4 lies along the line.
        let made = run(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "tangent", "entities": ["e3", "e7"]}),
        );
        assert_eq!(
            (&made["status"], &made["dof"]),
            (&json!("ok"), &json!(3)),
            "{made}"
        );
        // Tangent to the arc at e6: the leg e6-e5 is square to the radius.
        let made = run(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "tangent", "entities": ["e7", "e9"]}),
        );
        assert_eq!(
            (&made["status"], &made["dof"]),
            (&json!("ok"), &json!(2)),
            "{made}"
        );
        let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
        let at = |id: &str| -> P {
            let entity = inspected["entities"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == id)
                .unwrap();
            [entity["x"].as_f64().unwrap(), entity["y"].as_f64().unwrap()]
        };
        let (e2, e4, e5, e6) = (at("e2"), at("e4"), at("e5"), at("e6"));
        // The first leg is along x; the last leg is along x too (the
        // radius from (0, 3) to (0, 6) is along y).
        assert!((e4[1] - e2[1]).abs() <= 1e-6 * k, "{name}: {e4:?}");
        assert!(
            (e4[0] - e2[0]).abs() > 1.0 * k,
            "{name}: the leg kept its length"
        );
        assert!((e5[1] - e6[1]).abs() <= 1e-6 * k, "{name}: {e5:?}");
        assert!((e5[0] - e6[0]).abs() > 1.0 * k);
        for constraint in inspected["constraints"].as_array().unwrap() {
            assert_eq!(constraint["state"], "satisfied", "{constraint}");
        }
        // The profile is smooth at both corners and extrudes.
        let made = run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Profile", "distance": 2.0 * k, "name": "Smooth"}),
        );
        assert_eq!(made["status"], "ok", "{name}: {made}");
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{name}: {check}");
        // The half disc on the left, the spline's region on the right.
        let control = [e2, e4, e5, e6];
        let area = swept_line(at("e1"), e2) + swept(3, &control) + {
            // The arc from e6 to e1 about (0, 3): half a disc of radius 3
            // plus the triangle of its chord with the origin, which is
            // flat (the chord is on the y axis).
            PI * 9.0 * k * k / 2.0
        };
        close(
            body(&mut s, "Smooth")["volume"].as_f64().unwrap(),
            area * 2.0 * k,
            1e-6,
            &name,
        );
    }
}

#[test]
fn what_a_spline_cannot_do_yet_is_refused_with_the_reason() {
    let mut s = session();
    new_document(&mut s, 1.0);
    draw_d(&mut s, 1.0, 3);
    // The constraints that take lines or circles. (A point on a spline
    // is taken since 2026-10-14: `a_point_is_held_on_a_spline`.)
    for (kind, entities) in [
        ("parallel", json!(["e6", "e7"])),
        ("equal", json!(["e6", "e7"])),
        ("horizontal", json!(["e6"])),
        ("midpoint", json!(["e3", "e6"])),
        ("radius", json!(["e6"])),
    ] {
        let mut params = json!({"sketch": "Profile", "type": kind, "entities": entities});
        if kind == "radius" {
            params["value"] = 3.into();
        }
        let refused = error(&mut s, "sketch.constrain", params);
        assert!(
            refused.contains("not supported for splines yet") && refused.contains("e6"),
            "{kind}: {refused}"
        );
    }
    // Tangent needs a shared end: a circle has none, and a line elsewhere
    // shares none.
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": [20, 0], "to": [20, 5], "construction": true}),
    );
    let refused = error(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Profile", "type": "tangent", "entities": ["e6", "e10"]}),
    );
    assert!(refused.contains("share none"), "{refused}");
    // Tied by a coincident constraint they share one; without it again,
    // the sketch says what is missing and keeps its shape.
    run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Profile", "type": "fixed", "entities": ["e5"]}),
    );
    let tie = run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Profile", "type": "coincident", "entities": ["e9", "e5"]}),
    );
    assert_eq!(tie["constraint"], "c2");
    let tangent = run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Profile", "type": "tangent", "entities": ["e6", "e10"]}),
    );
    assert_eq!(tangent["status"], "ok", "{tangent}");
    let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    let at = |i: usize| -> P {
        let entity = &inspected["entities"][i];
        [entity["x"].as_f64().unwrap(), entity["y"].as_f64().unwrap()]
    };
    // The last leg, e5 to e4, lies along the line e8-e9 now.
    let (e4, e5, e8, e9) = (at(3), at(4), at(7), at(8));
    assert!((e9[0] - e5[0]).phypot(e9[1] - e5[1]) <= 1e-6, "{inspected}");
    let (leg, line) = (
        [e4[0] - e5[0], e4[1] - e5[1]],
        [e8[0] - e9[0], e8[1] - e9[1]],
    );
    let sine =
        (leg[0] * line[1] - leg[1] * line[0]) / (leg[0].phypot(leg[1]) * line[0].phypot(line[1]));
    assert!(sine.abs() <= 1e-6, "{inspected}");
    let untied = run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "Profile", "constraint": "c2"}),
    );
    assert_eq!(untied["status"], "failed", "{untied}");
    let reason = untied["error"].as_str().unwrap();
    assert!(
        reason.contains("c3") && reason.contains("share none") && reason.contains("e6"),
        "{reason}"
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(status(&mut s, "Profile").0, "ok");

    // (A revolution of a spline, a loop of one closed spline and a
    // polyline are built since 2026-10-14: `spline_sweeps.rs`. Until then
    // this test pinned their refusals.)
}

#[test]
fn a_spline_that_crosses_itself_makes_no_solid() {
    for k in [1e-3, 1.0, 1e3] {
        let mut s = session();
        new_document(&mut s, k);
        // The spline loops over itself on its way from (0, 0) to (0, 6).
        let looped = scaled(
            &[[0.0, 0.0], [8.0, 6.0], [8.0, -2.0], [3.0, 3.0], [0.0, 6.0]],
            k,
        );
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Profile", "control": looped}),
        );
        run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Profile", "from": "e5", "to": "e1"}),
        );
        let made = run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Profile", "distance": 2.0 * k, "name": "Knot"}),
        );
        assert_eq!(made["status"], "failed", "x{k}: {made}");
        let reason = made["error"].as_str().unwrap();
        assert!(
            reason.contains("e6 crosses or touches itself"),
            "x{k}: {reason}"
        );
        assert!(
            run(&mut s, "document.inspect", json!({}))["bodies"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        // Straightened out, it is a solid.
        for (point, at) in [("e2", BULGE[1]), ("e3", BULGE[2]), ("e4", BULGE[3])] {
            run(
                &mut s,
                "sketch.edit",
                json!({"sketch": "Profile", "entity": point, "at": [at[0] * k, at[1] * k]}),
            );
        }
        assert_eq!(status(&mut s, "Knot").0, "ok", "x{k}");
    }
}

#[test]
fn bad_requests_change_nothing() {
    let mut s = session();
    new_document(&mut s, 1.0);
    draw_d(&mut s, 1.0, 3);
    let before = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    let document = run(&mut s, "document.inspect", json!({}));
    let four = json!([[20, 0], [21, 1], [22, 0], [23, 1]]);
    // (params without the sketch, words the refusal has)
    let bad: Vec<(Value, &[&str])> = vec![
        (json!({}), &["control", "through"]),
        (
            json!({"control": four, "through": four}),
            &["either", "not both"],
        ),
        (
            json!({"control": [[20, 0], [21, 1], [22, 0]]}),
            &["degree 3", "at least 4", "got 3"],
        ),
        (
            json!({"through": [[20, 0], [21, 1]], "degree": 2}),
            &["at least 3"],
        ),
        (json!({"control": []}), &["at least 4", "got 0"]),
        (json!({"control": [], "closed": true}), &["needs points"]),
        (json!({"control": four, "degree": 0}), &["degree", "1 to 7"]),
        (json!({"control": four, "degree": 8}), &["degree", "1 to 7"]),
        (
            json!({"control": four, "degree": 4_000_000_000_u64}),
            &["degree"],
        ),
        (json!({"control": four, "degree": -1}), &["degree"]),
        (json!({"control": four, "degree": 2.5}), &["degree"]),
        (json!({"control": four, "degree": "3"}), &["degree"]),
        (
            json!({"control": [[20, 0], [21, 1], [21, 1], [23, 1]]}),
            &["points 2 and 3", "same place"],
        ),
        (
            json!({"through": [[20, 0], [21, 1], [21, 1], [23, 1]]}),
            &["same place"],
        ),
        (
            json!({"control": [[20, 0], [21, 1], [21, 1.0 + 1e-9], [23, 1]]}),
            &["same place"],
        ),
        (
            json!({"control": ["e1", "e2", "e2", "e3"]}),
            &["same place"],
        ),
        (
            json!({"control": ["e1", "e2", "e3", "e2"]}),
            &["e2", "twice"],
        ),
        (json!({"control": ["e1", "e2", "e3", "e99"]}), &["e99"]),
        (
            json!({"control": ["e1", "e2", "e3", "e6"]}),
            &["e6", "spline", "not a point"],
        ),
        (
            json!({"through": ["e1", "e2", "e3", "e7"]}),
            &["e7", "line", "not a point"],
        ),
        (
            json!({"control": ["e1", "e2", "e3", "x"]}),
            &["control", "id"],
        ),
        (
            json!({"control": [[20, 0], [21, 1], [22], [23, 1]]}),
            &["two finite numbers"],
        ),
        (
            json!({"control": [[20, 0], [21, 1], [22, null], [23, 1]]}),
            &["two finite numbers"],
        ),
        (
            json!({"control": [[20, 0], [21, 1], [f64::INFINITY, 0], [23, 1]]}),
            &["two finite numbers"],
        ),
        (
            // An expression is a coordinate since 2026-10-22; `a` names
            // no variable.
            json!({"control": [[20, 0], [21, 1], ["a", 0], [23, 1]]}),
            &["control", "`a`"],
        ),
        (
            json!({"control": [[20, 0], [21, 1], {"x": 1}, [23, 1]]}),
            &["point id"],
        ),
        (json!({"control": "e1"}), &["array"]),
        (json!({"through": 7}), &["array"]),
        (json!({"control": four, "closed": "yes"}), &["closed"]),
        (
            json!({"control": four, "construction": 1}),
            &["construction"],
        ),
        (
            json!({"control": four, "knots": [0, 1]}),
            &["unknown parameter", "knots"],
        ),
        (
            json!({"control": (0..1001).map(|i| json!([i, i % 2])).collect::<Vec<_>>()}),
            &["at most 1000"],
        ),
        (
            json!({"through": (0..401).map(|i| json!([i, i % 2])).collect::<Vec<_>>()}),
            &["at most 400"],
        ),
    ];
    for (mut params, words) in bad {
        params["sketch"] = "Profile".into();
        let refused = error(&mut s, "sketch.spline", params.clone());
        for word in words {
            assert!(
                refused.contains(word),
                "{params}: `{word}` is not in: {refused}"
            );
        }
    }
    let refused = error(&mut s, "sketch.spline", json!({"control": four}));
    assert!(refused.contains("sketch"), "{refused}");
    let refused = error(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Nope", "control": four}),
    );
    assert!(refused.contains("Nope"), "{refused}");
    // Nothing changed, and nothing was added to the undo history: one undo
    // takes the line of the D away.
    assert_eq!(
        run(&mut s, "sketch.inspect", json!({"sketch": "Profile"})),
        before
    );
    assert_eq!(run(&mut s, "document.inspect", json!({})), document);
    run(&mut s, "edit.undo", json!({}));
    let after = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
    assert_eq!(after["entities"].as_array().unwrap().len(), 6);
}

#[test]
fn a_hole_is_cut_through_a_solid_with_a_spline_wall() {
    for k in [1e-3, 1.0, 1e3] {
        let name = format!("x{k}");
        let mut s = session();
        new_document(&mut s, k);
        let area = draw_d(&mut s, k, 3);
        let depth = 3.0 * k;
        run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Profile", "distance": depth, "name": "Hull"}),
        );
        // A hole through the middle, drawn on the end cap.
        run(
            &mut s,
            "sketch.create",
            json!({"name": "Bore", "face": "F2:endCap(e6,e7)"}),
        );
        let radius = 1.0 * k;
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Bore", "center": [2.5 * k, 3.0 * k], "radius": radius}),
        );
        let made = run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Bore", "distance": -depth, "cut": "Hull", "name": "Drilled"}),
        );
        assert_eq!(made["status"], "ok", "{name}: {made}");
        let solid = body(&mut s, "Drilled");
        close(
            solid["volume"].as_f64().unwrap(),
            (area - PI * radius * radius) * depth,
            1e-9,
            &name,
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{name}: {check}");
        // The wall of the spline is still called after it.
        let names = face_names(&mut s, "Drilled");
        assert!(
            names.iter().any(|n| n.contains("side(e6)")),
            "{name}: {names:?}"
        );

        // A slot that cuts through the spline wall itself: a block across
        // the bulge.
        run(
            &mut s,
            "feature.block",
            json!({"name": "Cutter", "size": [4.0 * k, 1.0 * k, 5.0 * k],
                   "position": [4.0 * k, 2.5 * k, -k]}),
        );
        let slotted = run(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Drilled", "tool": "Cutter",
                   "name": "Slotted"}),
        );
        assert_eq!(slotted["status"], "ok", "{name}: {slotted}");
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{name}: {check}");
        // The slot takes the part of the D between y = 2.5 and 3.5 that
        // lies right of x = 4: the area between the spline and x = 4
        // there, by the oracle (the spline is x(y) on that stretch).
        let bulge = scaled(&BULGE, k);
        let band = {
            // Find the parameters where the spline passes y = 2.5 and 3.5
            // by bisection (y rises along the bulge), then integrate
            // (x − 4) dy by Simpson.
            let y_at = |t: f64| spline_point(3, &bulge, t)[1];
            let solve = |y: f64| {
                let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
                for _ in 0..200 {
                    let mid = (lo + hi) / 2.0;
                    if y_at(mid) < y {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                (lo + hi) / 2.0
            };
            let (t0, t1) = (solve(2.5 * k), solve(3.5 * k));
            let steps = 20_000;
            let h = (t1 - t0) / steps as f64;
            let mut sum = 0.0;
            let mut before = spline_point(3, &bulge, t0);
            // Trapezoid on (x − 4) dy with exact chords: fine enough at
            // this many steps for 1e-7.
            for i in 1..=steps {
                let here = spline_point(3, &bulge, t0 + h * i as f64);
                sum += ((before[0] + here[0]) / 2.0 - 4.0 * k) * (here[1] - before[1]);
                before = here;
            }
            sum
        };
        close(
            body(&mut s, "Slotted")["volume"].as_f64().unwrap(),
            (area - PI * radius * radius - band) * depth,
            1e-7,
            &format!("{name} slotted"),
        );
    }
}

#[test]
fn a_spline_is_drawn_as_a_polyline_on_its_curve() {
    let mut s = session();
    new_document(&mut s, 1.0);
    draw_d(&mut s, 1.0, 3);
    let scene = s.scene(Some(1e-3)).unwrap().unwrap();
    let sketch = &scene.sketches[0];
    let drawn: Vec<_> = sketch
        .polylines
        .iter()
        .filter(|p| p.curve == SketchCurve::Spline)
        .collect();
    assert_eq!(drawn.len(), 1);
    let spline = drawn[0];
    assert_eq!(spline.entity, "e6");
    assert!(spline.points.len() > 16, "{}", spline.points.len());
    // It starts and ends on the end control points, and every point of it
    // is on the curve: nearer than a thousandth to one of 4000 samples.
    let first = spline.points[0];
    let last = spline.points[spline.points.len() - 1];
    assert_eq!((first.x, first.y, first.z), (0.0, 0.0, 0.0));
    assert_eq!((last.x, last.y, last.z), (0.0, 6.0, 0.0));
    let samples: Vec<P> = (0..=4000)
        .map(|i| spline_point(3, &BULGE, f64::from(i) / 4000.0))
        .collect();
    for point in &spline.points {
        let nearest = samples
            .iter()
            .map(|q| (point.x - q[0]).phypot(point.y - q[1]))
            .fold(f64::INFINITY, f64::min);
        assert!(nearest < 5e-3, "{point:?}: {nearest}");
    }
    // The control points are drawn as the points they are.
    assert_eq!(sketch.points.len(), 5);
}

/// How far `p` is from the sketch spline of `degree` on `control`: the
/// nearest of the chords between 100 000 places de Boor's algorithm gives.
fn off_spline(degree: usize, control: &[P], p: P) -> f64 {
    let steps = 100_000;
    let mut least = f64::INFINITY;
    let mut before = control[0];
    for i in 1..=steps {
        let here = spline_point(degree, control, f64::from(i) / f64::from(steps));
        let d = [here[0] - before[0], here[1] - before[1]];
        let l2 = d[0] * d[0] + d[1] * d[1];
        let t = if l2 > 0.0 {
            (((p[0] - before[0]) * d[0] + (p[1] - before[1]) * d[1]) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        least = least.min((p[0] - before[0] - d[0] * t).phypot(p[1] - before[1] - d[1] * t));
        before = here;
    }
    least
}

/// Where the entities `ids` of sketch `Profile` are.
fn places(session: &mut Session, ids: &[usize]) -> Vec<P> {
    let inspected = run(session, "sketch.inspect", json!({"sketch": "Profile"}));
    ids.iter()
        .map(|&id| {
            let entity = &inspected["entities"][id - 1];
            assert_eq!(entity["id"], format!("e{id}"), "{entity}");
            [entity["x"].as_f64().unwrap(), entity["y"].as_f64().unwrap()]
        })
        .collect()
}

#[test]
fn a_point_is_held_on_a_spline() {
    for k in [1e-3, 1.0, 1e6] {
        for degree in [2, 3, 4] {
            let name = format!("degree {degree} x{k}");
            let dir = workdir(&format!("on-{degree}-{k}"));
            let mut s = session_in(&dir);
            new_document(&mut s, k);
            draw_d(&mut s, k, degree);
            // A point beside the bulge: two more degrees of freedom, and
            // one less once it is on the spline.
            let point = run(
                &mut s,
                "sketch.point",
                json!({"sketch": "Profile", "at": [8.0 * k, 2.0 * k]}),
            );
            assert_eq!(
                (&point["entity"], &point["dof"]),
                (&json!("e8"), &json!(12))
            );
            let on = run(
                &mut s,
                "sketch.constrain",
                json!({"sketch": "Profile", "type": "pointOn", "entities": ["e8", "e6"]}),
            );
            assert_eq!(on["status"], "ok", "{name}: {on}");
            assert_eq!(on["dof"], 11, "{name}: {on}");
            assert!(on.get("redundant").is_none() && on.get("conflicting").is_none());
            let control = places(&mut s, &[1, 2, 3, 4, 5]);
            // The point is on the spline (both gave way: what is not
            // constrained moves as little as it can, all taken together).
            let at = places(&mut s, &[8])[0];
            assert!(
                off_spline(degree, &control, at) <= 1e-6 * k,
                "{name}: {at:?}"
            );

            // Twice is once too many, and says so.
            let twice = run(
                &mut s,
                "sketch.constrain",
                json!({"sketch": "Profile", "type": "pointOn", "entities": ["e8", "e6"]}),
            );
            assert_eq!(twice["redundant"], json!(["c2"]), "{name}: {twice}");
            assert_eq!(twice["dof"], 11, "{name}");
            run(&mut s, "edit.undo", json!({}));

            // Held at a height, on a spline whose points are held: nothing
            // is free. Then one control point is moved by a variable, and
            // the point goes with the curve.
            for id in [1, 2, 4, 5] {
                run(
                    &mut s,
                    "sketch.constrain",
                    json!({"sketch": "Profile", "type": "fixed", "entities": [format!("e{id}")]}),
                );
            }
            run(
                &mut s,
                "variable.set",
                json!({"name": "bulge", "expr": format!("{}", 7.0 * k)}),
            );
            for (direction, value) in [
                ("horizontal", "bulge".to_string()),
                ("vertical", format!("{}", 3.0 * k)),
            ] {
                run(
                    &mut s,
                    "sketch.constrain",
                    json!({"sketch": "Profile", "type": "distance", "entities": ["e1", "e3"],
                           "direction": direction, "value": value}),
                );
            }
            let held = run(
                &mut s,
                "sketch.constrain",
                json!({"sketch": "Profile", "type": "distance", "entities": ["e1", "e8"],
                       "direction": "vertical", "value": format!("{}", 2.0 * k)}),
            );
            assert_eq!(
                (&held["status"], &held["dof"]),
                (&json!("ok"), &json!(0)),
                "{name}: {held}"
            );
            let mut seen = Vec::new();
            for bulge in [7.0, 9.0, 5.5, 8.0, 7.0] {
                // (`run` holds every step to a full regeneration.)
                run(
                    &mut s,
                    "variable.set",
                    json!({"name": "bulge", "expr": format!("{}", bulge * k)}),
                );
                let inspected = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
                assert_eq!(
                    inspected["status"], "ok",
                    "{name} bulge {bulge}: {inspected}"
                );
                let control = places(&mut s, &[1, 2, 3, 4, 5]);
                // (Measured from e1, which gave way a little with the rest
                // when the point was put on the spline.)
                let origin = control[0];
                assert!(
                    (control[2][0] - origin[0] - bulge * k).abs() <= 1e-6 * k,
                    "{name}"
                );
                let at = places(&mut s, &[8])[0];
                assert!(
                    off_spline(degree, &control, at) <= 1e-6 * k,
                    "{name} bulge {bulge}: {at:?}"
                );
                assert!(
                    (at[1] - origin[1] - 2.0 * k).abs() <= 1e-6 * k,
                    "{name} bulge {bulge}"
                );
                seen.push(at[0]);
            }
            // Further out with the bulge, back in with it, and back where
            // it was.
            assert!(seen[1] > seen[0] && seen[2] < seen[0], "{name}: {seen:?}");
            assert!((seen[4] - seen[0]).abs() <= 1e-6 * k, "{name}: {seen:?}");

            // The body follows, and the file brings the same sketch back.
            let made = run(
                &mut s,
                "feature.extrude",
                json!({"sketch": "Profile", "distance": 2.0 * k, "name": "Hull"}),
            );
            assert_eq!(made["status"], "ok", "{name}: {made}");
            let before = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
            let measured = run(&mut s, "document.measure", json!({}));
            run(&mut s, "file.save", json!({"path": "on.3dcraft"}));
            run(&mut s, "file.close", json!({}));
            run(&mut s, "file.open", json!({"path": "on.3dcraft"}));
            assert_eq!(
                run(&mut s, "sketch.inspect", json!({"sketch": "Profile"})),
                before,
                "{name}"
            );
            assert_eq!(
                run(&mut s, "document.measure", json!({})),
                measured,
                "{name}"
            );

            // A point held somewhere else cannot be on the spline as
            // well: the later constraint is named, nothing panics, and
            // the sketch keeps what it had.
            let clash = run(
                &mut s,
                "sketch.constrain",
                json!({"sketch": "Profile", "type": "distance", "entities": ["e1", "e8"],
                       "direction": "horizontal", "value": format!("{}", 30.0 * k)}),
            );
            assert!(
                clash["conflicting"]
                    .as_array()
                    .is_some_and(|c| c.len() == 1),
                "{name}: {clash}"
            );
        }
    }
}

#[test]
fn a_point_on_a_spline_takes_bad_parameters_as_errors() {
    let mut s = session();
    new_document(&mut s, 1.0);
    draw_d(&mut s, 1.0, 3);
    run(
        &mut s,
        "sketch.point",
        json!({"sketch": "Profile", "at": [8, 2]}),
    );
    for (entities, word) in [
        (json!(["e6", "e8"]), "a point and a spline"),
        (json!(["e6", "e6"]), "pointOn"),
        (json!(["e8"]), "pointOn"),
        (json!(["e8", "e99"]), "e99"),
        (json!(["e8", "e6", "e7"]), "pointOn"),
    ] {
        let refused = error(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "pointOn", "entities": entities}),
        );
        assert!(refused.contains(word), "{entities}: {refused}");
    }
    let refused = error(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Profile", "type": "pointOn", "entities": ["e8", "e6"], "value": 3}),
    );
    assert!(refused.contains("value"), "{refused}");
    // An end of the spline is on it already; a control point in between
    // can be put on its own curve.
    let end = run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Profile", "type": "pointOn", "entities": ["e1", "e6"]}),
    );
    assert_eq!(end["redundant"], json!(["c1"]), "{end}");
    run(&mut s, "edit.undo", json!({}));
    let own = run(
        &mut s,
        "sketch.constrain",
        json!({"sketch": "Profile", "type": "pointOn", "entities": ["e3", "e6"]}),
    );
    assert_eq!(own["status"], "ok", "{own}");
    let control = places(&mut s, &[1, 2, 3, 4, 5]);
    assert!(off_spline(3, &control, control[2]) <= 1e-6, "{control:?}");
}
