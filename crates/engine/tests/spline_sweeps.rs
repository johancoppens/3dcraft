//! What a spline in a sketch sweeps beyond a wall between two caps,
//! through the commands: a profile with a spline in it turned about an
//! axis (a vase), a spline that closes on itself as a profile of its own,
//! and a polyline. Measured, checked, cut by booleans, changed upstream,
//! saved and reopened, at 1e-3, 1 and 1e6.
//!
//! The oracle is independent of the kernel: a spline is evaluated by de
//! Boor's algorithm on its knots and integrated by Simpson's rule on a
//! fine subdivision of every knot span. A profile's area is Green's
//! theorem; what it sweeps about an axis is, by the theorems of Pappus,
//! the angle times `∬ x dA = ∮ x² / 2 dy` in volume and the angle times
//! `∫ x ds` in surface.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{body, error, run, session_in};
use craft3d_engine::{Session, Workspace};
use serde_json::json;
use std::f64::consts::{PI, TAU};
use std::path::PathBuf;

type P = [f64; 2];

fn workdir(name: &str) -> PathBuf {
    common::workdir("spline-sweeps", name)
}

fn session() -> Session {
    Session::new(Workspace::none())
}

fn volume(session: &mut Session, name: &str) -> f64 {
    body(session, name)["volume"].as_f64().unwrap()
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

/// Along a spline of degree 2 or more, from its start to its end:
/// `∫ x dy` (the area between it and the y axis), `∫ x² / 2 dy` (the
/// volume per radian of that area turned about the y axis), `∫ x ds` (the
/// surface per radian the curve sweeps) and `∫ ds`. Composite Simpson on
/// 2000 steps per knot span; the derivative is the spline of one degree
/// less on the differences of the control points.
fn along(degree: usize, control: &[P]) -> [f64; 4] {
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
    let mut sum = [0.0; 4];
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
            } * h
                / 3.0;
            sum[0] += p[0] * d[1] * weight;
            sum[1] += p[0] * p[0] / 2.0 * d[1] * weight;
            sum[2] += p[0] * d[0].phypot(d[1]) * weight;
            sum[3] += d[0].phypot(d[1]) * weight;
        }
    }
    sum
}

fn scaled(points: &[P], k: f64) -> Vec<P> {
    points.iter().map(|p| [p[0] * k, p[1] * k]).collect()
}

/// The wall of a vase, from its foot at (3, 0) up to its rim at (2, 10).
const VASE: [P; 7] = [
    [3.0, 0.0],
    [5.5, 1.0],
    [6.0, 4.0],
    [1.0, 5.5],
    [1.5, 8.0],
    [3.5, 9.0],
    [2.0, 10.0],
];

/// A ring of control points to the right of the y axis; closed, a spline
/// on it comes back to its first.
const EGG: [P; 6] = [
    [4.0, 2.0],
    [8.0, 1.0],
    [10.0, 5.0],
    [7.0, 9.0],
    [3.0, 8.0],
    [2.0, 4.0],
];

fn new_document(session: &mut Session, k: f64) {
    run(session, "file.new", json!({"tolerance": 1e-6 * k}));
    run(session, "sketch.create", json!({"name": "Profile"}));
}

/// The section of a solid vase in sketch `Profile`: the bottom e3 from e1
/// on the y axis to e2, the wall (the spline e10 on e2 and e4..e9), the
/// rim e12 from e9 to e11 on the axis, and e13 down the axis.
fn draw_vase(session: &mut Session, k: f64, degree: usize) {
    let wall = scaled(&VASE, k);
    run(
        session,
        "sketch.line",
        json!({"sketch": "Profile", "from": [0, 0], "to": wall[0]}),
    );
    let mut control = vec![json!("e2")];
    control.extend(wall[1..].iter().map(|p| json!(p)));
    let made = run(
        session,
        "sketch.spline",
        json!({"sketch": "Profile", "control": control, "degree": degree}),
    );
    assert_eq!(
        (&made["entity"], &made["end"]),
        (&json!("e10"), &json!("e9")),
        "{made}"
    );
    run(
        session,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e9", "to": [0.0, 10.0 * k]}),
    );
    let closing = run(
        session,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e11", "to": "e1"}),
    );
    assert_eq!(closing["entity"], "e13");
}

/// Where the points of sketch `Profile` are, by entity number.
fn points(session: &mut Session, ids: &[usize]) -> Vec<P> {
    let inspected = run(session, "sketch.inspect", json!({"sketch": "Profile"}));
    ids.iter()
        .map(|&id| {
            let entity = &inspected["entities"][id - 1];
            assert_eq!(entity["id"], format!("e{id}"), "{entity}");
            [entity["x"].as_f64().unwrap(), entity["y"].as_f64().unwrap()]
        })
        .collect()
}

/// The vase's section by the oracle, from where its wall's control
/// points are: its area, the volume and the surface it sweeps per radian
/// (the bottom and the rim are discs: `∫ x ds = r² / 2`).
fn vase_sums(degree: usize, wall: &[P]) -> [f64; 3] {
    let sums = along(degree, wall);
    let (foot, rim) = (wall[0][0], wall[wall.len() - 1][0]);
    [
        sums[0].abs(),
        sums[1].abs(),
        sums[2] + foot * foot / 2.0 + rim * rim / 2.0,
    ]
}

const WALL: [usize; 7] = [2, 4, 5, 6, 7, 8, 9];

// -------------------------------------------------------------- the tests

#[test]
fn a_vase_is_a_spline_profile_turned_about_an_axis() {
    for k in [1e-3, 1.0, 1e6] {
        for (degree, turn, symmetric) in [
            (3, "360 deg", false),
            (3, "90 deg", false),
            (3, "120 deg", true),
            (2, "-120 deg", false),
            (5, "360 deg", false),
        ] {
            let name = format!("degree {degree} x{k} through {turn}");
            let dir = workdir(&format!("vase-{degree}-{k}-{turn}"));
            let mut s = session_in(&dir);
            new_document(&mut s, k);
            draw_vase(&mut s, k, degree);
            let made = run(
                &mut s,
                "feature.revolve",
                json!({"sketch": "Profile", "axis": "y", "angle": turn,
                       "symmetric": symmetric, "name": "Vase"}),
            );
            assert_eq!(made, json!({"id": "F2", "name": "Vase", "status": "ok"}));
            let [section, per_radian, surface] = vase_sums(degree, &scaled(&VASE, k));
            let full = turn == "360 deg";
            let angle = if full {
                TAU
            } else if turn == "90 deg" {
                PI / 2.0
            } else {
                TAU / 3.0
            };
            let caps = if full { 0.0 } else { 2.0 * section };
            let solid = body(&mut s, "Vase");
            assert_eq!(solid["kind"], "solid");
            close(
                solid["volume"].as_f64().unwrap(),
                angle * per_radian,
                1e-9,
                &name,
            );
            close(
                solid["area"].as_f64().unwrap(),
                angle * surface + caps,
                1e-9,
                &format!("{name} area"),
            );
            // The wall is called after the spline; a full turn makes it
            // in two halves, the second numbered.
            let names = face_names(&mut s, "Vase");
            let walls: Vec<&String> = names.iter().filter(|n| n.contains("side(e10)")).collect();
            if full {
                assert_eq!(
                    walls,
                    ["F2:piece(1)/F2:side(e10)", "F2:side(e10)"],
                    "{name}"
                );
                assert_eq!(names.len(), 4, "{name}: {names:?}");
            } else {
                assert_eq!(walls, ["F2:side(e10)"], "{name}");
                assert_eq!(names.len(), 5, "{name}: {names:?}");
            }
            let check = run(&mut s, "document.check", json!({"deep": true}));
            assert_eq!(check["ok"], true, "{name}: {check}");

            // Watertight as a mesh, and the same after saving and opening.
            let exported = run(
                &mut s,
                "file.export",
                json!({"path": "vase.3mf", "tolerance": 1e-3 * k}),
            );
            assert_eq!(
                (&exported["bodies"], &exported["closed"]),
                (&json!(1), &json!(1)),
                "{name}"
            );
            let before = run(&mut s, "document.measure", json!({}));
            let inspected = run(&mut s, "document.inspect", json!({}));
            run(&mut s, "file.save", json!({"path": "vase.3dcraft"}));
            run(&mut s, "file.close", json!({}));
            run(&mut s, "file.open", json!({"path": "vase.3dcraft"}));
            assert_eq!(run(&mut s, "document.measure", json!({})), before, "{name}");
            assert_eq!(
                run(&mut s, "document.inspect", json!({}))["bodies"],
                inspected["bodies"],
                "{name}"
            );
        }
    }
}

#[test]
fn a_control_point_moved_by_a_variable_reshapes_the_vase() {
    for k in [1e-3, 1.0, 1e6] {
        let name = format!("x{k}");
        let mut s = session();
        new_document(&mut s, k);
        draw_vase(&mut s, k, 3);
        // Everything where it was drawn, except the belly (e5), which is
        // as far from the axis as a variable says.
        for id in [1, 2, 4, 6, 7, 8, 9, 11] {
            run(
                &mut s,
                "sketch.constrain",
                json!({"sketch": "Profile", "type": "fixed", "entities": [format!("e{id}")]}),
            );
        }
        run(
            &mut s,
            "variable.set",
            json!({"name": "belly", "expr": format!("{}", 6.0 * k)}),
        );
        run(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "distance", "entities": ["e1", "e5"],
                   "direction": "horizontal", "value": "belly"}),
        );
        run(
            &mut s,
            "sketch.constrain",
            json!({"sketch": "Profile", "type": "distance", "entities": ["e1", "e5"],
                   "direction": "vertical", "value": format!("{}", 4.0 * k)}),
        );
        let solved = run(&mut s, "sketch.inspect", json!({"sketch": "Profile"}));
        assert_eq!(solved["status"], "ok", "{name}: {solved}");
        let made = run(
            &mut s,
            "feature.revolve",
            json!({"sketch": "Profile", "axis": "y", "name": "Vase"}),
        );
        assert_eq!(made["status"], "ok", "{name}: {made}");
        let mut volumes = Vec::new();
        for belly in [6.0, 8.0, 4.5, 6.0] {
            // (`run` holds every step to a full regeneration.)
            run(
                &mut s,
                "variable.set",
                json!({"name": "belly", "expr": format!("{}", belly * k)}),
            );
            let wall = points(&mut s, &WALL);
            assert!(
                (wall[2][0] - belly * k).abs() <= 1e-9 * k,
                "{name}: {wall:?}"
            );
            let [_, per_radian, _] = vase_sums(3, &wall);
            let volume = volume(&mut s, "Vase");
            close(
                volume,
                TAU * per_radian,
                1e-9,
                &format!("{name} belly {belly}"),
            );
            let names = face_names(&mut s, "Vase");
            assert!(
                names.contains(&"F2:piece(1)/F2:side(e10)".to_string()),
                "{names:?}"
            );
            volumes.push(volume);
        }
        // Back where it was is the same body, to the last bit.
        assert_eq!(volumes[0], volumes[3], "{name}");
        assert!(volumes[1] > volumes[0] && volumes[2] < volumes[0], "{name}");
        // A distance that cannot be (a negative one) fails the sketch;
        // the vase fails because of it, and comes back when the variable
        // does.
        run(
            &mut s,
            "variable.set",
            json!({"name": "belly", "expr": format!("{}", -9.0 * k)}),
        );
        let document = run(&mut s, "document.inspect", json!({}));
        let feature = &document["features"][1];
        assert_eq!(feature["status"], "failed", "{name}: {feature}");
        let reason = feature["error"].as_str().unwrap();
        assert!(
            reason.contains("`Profile` failed") && reason.contains("fix it first"),
            "{name}: {reason}"
        );
        assert!(document["bodies"].as_array().unwrap().is_empty(), "{name}");
        run(
            &mut s,
            "variable.set",
            json!({"name": "belly", "expr": format!("{}", 6.0 * k)}),
        );
        assert_eq!(volume(&mut s, "Vase"), volumes[0], "{name}");
    }
}

/// A closed spline on the egg's points in sketch `Profile`: the spline e7
/// on e1..e6 and back on e1. Returns its control points, the first again
/// at the end. Clamped at e1 (`periodic: false`), so these tests keep
/// their oracle on the given polygon (since 2026-11-04 a closed spline is
/// periodic by default; `sketch_spline.rs` holds the periodic one).
fn draw_egg(session: &mut Session, k: f64, degree: usize) -> Vec<P> {
    let ring = scaled(&EGG, k);
    let made = run(
        session,
        "sketch.spline",
        json!({"sketch": "Profile", "control": ring, "degree": degree, "closed": true, "periodic": false}),
    );
    assert_eq!(made["entity"], "e7", "{made}");
    assert_eq!((&made["start"], &made["end"]), (&json!("e1"), &json!("e1")));
    let mut ring = ring;
    ring.push(ring[0]);
    ring
}

#[test]
fn a_spline_that_closes_on_itself_is_a_profile() {
    for k in [1e-3, 1.0, 1e6] {
        for degree in [2, 3, 5] {
            let name = format!("degree {degree} x{k}");
            let dir = workdir(&format!("egg-{degree}-{k}"));
            let mut s = session_in(&dir);
            new_document(&mut s, k);
            let ring = draw_egg(&mut s, k, degree);
            let sums = along(degree, &ring);
            let (area, depth) = (sums[0].abs(), 2.5 * k);
            let made = run(
                &mut s,
                "feature.extrude",
                json!({"sketch": "Profile", "distance": depth, "name": "Egg"}),
            );
            assert_eq!(made, json!({"id": "F2", "name": "Egg", "status": "ok"}));
            let solid = body(&mut s, "Egg");
            // Two walls (the spline is cut in two) and two caps.
            assert_eq!(
                (&solid["faces"], &solid["edges"], &solid["vertices"]),
                (&json!(4), &json!(6), &json!(4)),
                "{name}"
            );
            close(solid["volume"].as_f64().unwrap(), area * depth, 1e-9, &name);
            close(
                solid["area"].as_f64().unwrap(),
                2.0 * area + sums[3] * depth,
                1e-9,
                &format!("{name} area"),
            );
            assert_eq!(
                face_names(&mut s, "Egg"),
                [
                    "F2:endCap(e7)",
                    "F2:piece(1)/F2:side(e7)",
                    "F2:side(e7)",
                    "F2:startCap(e7)"
                ],
                "{name}"
            );
            let check = run(&mut s, "document.check", json!({"deep": true}));
            assert_eq!(check["ok"], true, "{name}: {check}");

            // Turned about the y axis as well: a third of a turn beside
            // the prism, then all the way round.
            for (turn, angle, faces) in [("120 deg", TAU / 3.0, 4), ("360 deg", TAU, 4)] {
                let made = run(
                    &mut s,
                    "feature.revolve",
                    json!({"sketch": "Profile", "axis": "y", "angle": turn, "name": "Ring"}),
                );
                assert_eq!(made["status"], "ok", "{name} {turn}: {made}");
                let solid = body(&mut s, "Ring");
                assert_eq!(solid["faces"], faces, "{name} {turn}");
                let caps = if angle == TAU { 0.0 } else { 2.0 * area };
                close(
                    solid["volume"].as_f64().unwrap(),
                    angle * sums[1].abs(),
                    1e-9,
                    &format!("{name} {turn}"),
                );
                close(
                    solid["area"].as_f64().unwrap(),
                    angle * sums[2] + caps,
                    1e-9,
                    &format!("{name} {turn} area"),
                );
                let check = run(&mut s, "document.check", json!({"deep": true}));
                assert_eq!(check["ok"], true, "{name} {turn}: {check}");
                if angle != TAU {
                    run(&mut s, "edit.undo", json!({}));
                }
            }
            let names = face_names(&mut s, "Ring");
            assert_eq!(
                names,
                [
                    "F3:piece(1)/F3:side(e7)",
                    "F3:piece(2)/F3:side(e7)",
                    "F3:piece(3)/F3:side(e7)",
                    "F3:side(e7)"
                ],
                "{name}"
            );
            let exported = run(
                &mut s,
                "file.export",
                json!({"path": "egg.3mf", "tolerance": 1e-3 * k}),
            );
            assert_eq!(
                (&exported["bodies"], &exported["closed"]),
                (&json!(2), &json!(2)),
                "{name}"
            );
            let before = run(&mut s, "document.measure", json!({}));
            let inspected = run(&mut s, "document.inspect", json!({}));
            run(&mut s, "file.save", json!({"path": "egg.3dcraft"}));
            run(&mut s, "file.close", json!({}));
            run(&mut s, "file.open", json!({"path": "egg.3dcraft"}));
            assert_eq!(run(&mut s, "document.measure", json!({})), before, "{name}");
            assert_eq!(
                run(&mut s, "document.inspect", json!({}))["bodies"],
                inspected["bodies"],
                "{name}"
            );
        }
    }
}

#[test]
fn a_polyline_is_a_profile_of_flat_walls() {
    for k in [1e-3, 1.0, 1e6] {
        let name = format!("x{k}");
        let mut s = session();
        new_document(&mut s, k);
        let corners = scaled(
            &[[1.0, 0.0], [7.0, 1.0], [6.0, 5.0], [3.0, 3.0], [2.0, 6.0]],
            k,
        );
        let made = run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Profile", "control": corners, "degree": 1, "closed": true}),
        );
        assert_eq!(made["entity"], "e6", "{made}");
        // The shoelace formula.
        let area: f64 = (0..5)
            .map(|i| {
                let (a, b) = (corners[i], corners[(i + 1) % 5]);
                (a[0] * b[1] - a[1] * b[0]) / 2.0
            })
            .sum();
        let made = run(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Profile", "distance": 2.0 * k, "name": "Star"}),
        );
        assert_eq!(made["status"], "ok", "{name}: {made}");
        let solid = body(&mut s, "Star");
        assert_eq!(
            (&solid["faces"], &solid["edges"], &solid["vertices"]),
            (&json!(7), &json!(15), &json!(10)),
            "{name}"
        );
        close(
            solid["volume"].as_f64().unwrap(),
            area.abs() * 2.0 * k,
            1e-12,
            &name,
        );
        let names = face_names(&mut s, "Star");
        assert_eq!(
            names,
            [
                "F2:endCap(e6)",
                "F2:piece(1)/F2:side(e6)",
                "F2:piece(2)/F2:side(e6)",
                "F2:piece(3)/F2:side(e6)",
                "F2:piece(4)/F2:side(e6)",
                "F2:side(e6)",
                "F2:startCap(e6)"
            ],
            "{name}"
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{name}: {check}");
        // A flat wall of a polyline takes a sketch like any flat face.
        let on_wall = run(
            &mut s,
            "sketch.create",
            json!({"name": "OnWall", "face": "F2:piece(2)/F2:side(e6)"}),
        );
        assert_eq!(on_wall["status"], "ok", "{name}: {on_wall}");
        // Turned about the y axis: cones and discs.
        let made = run(
            &mut s,
            "feature.revolve",
            json!({"sketch": "Profile", "axis": "y", "name": "Spun"}),
        );
        assert_eq!(made["status"], "ok", "{name}: {made}");
        let per_radian: f64 = (0..5)
            .map(|i| {
                let (a, b) = (corners[i], corners[(i + 1) % 5]);
                // ∫ x² / 2 dy along a line.
                (a[0] * a[0] + a[0] * b[0] + b[0] * b[0]) / 6.0 * (b[1] - a[1])
            })
            .sum();
        close(volume(&mut s, "Spun"), TAU * per_radian.abs(), 1e-12, &name);
    }
}

/// `volume(A ∪ B) + volume(A ∩ B) = volume(A) + volume(B)` and
/// `volume(A − B) = volume(A) − volume(A ∩ B)`, for a body `build` makes
/// and the tool `tool` adds.
fn identities(name: &str, build: &dyn Fn(&mut Session), tool: &dyn Fn(&mut Session), k: f64) {
    let mut measured = Vec::new();
    for op in ["union", "intersect", "subtract"] {
        let mut s = session();
        new_document(&mut s, k);
        build(&mut s);
        tool(&mut s);
        let (a, b) = (volume(&mut s, "Body"), volume(&mut s, "Tool"));
        let made = run(
            &mut s,
            "feature.boolean",
            json!({"op": op, "target": "Body", "tool": "Tool", "name": "Result"}),
        );
        assert_eq!(made["status"], "ok", "{name} {op}: {made}");
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{name} {op}: {check}");
        measured.push((a, b, volume(&mut s, "Result")));
    }
    let (a, b, union) = measured[0];
    let (intersection, difference) = (measured[1].2, measured[2].2);
    assert!(
        intersection > 0.01 * a.min(b) && intersection < 0.99 * a.min(b),
        "{name}"
    );
    close(
        union + intersection,
        a + b,
        1e-7,
        &format!("{name}: union and intersection"),
    );
    close(
        difference,
        a - intersection,
        1e-7,
        &format!("{name}: difference"),
    );
}

#[test]
fn booleans_with_a_vase_and_with_a_closed_spline_keep_the_set_identities() {
    for k in [1e-3, 1.0, 1e6] {
        let vase = |turn: &'static str| {
            move |s: &mut Session| {
                draw_vase(s, k, 3);
                let made = run(
                    s,
                    "feature.revolve",
                    json!({"sketch": "Profile", "axis": "y", "angle": turn, "name": "Body"}),
                );
                assert_eq!(made["status"], "ok", "{made}");
            }
        };
        let egg = |s: &mut Session| {
            draw_egg(s, k, 3);
            let made = run(
                s,
                "feature.extrude",
                json!({"sketch": "Profile", "distance": 3.0 * k, "name": "Body"}),
            );
            assert_eq!(made["status"], "ok", "{made}");
        };
        // A block through the belly of the vase and out of it, and a
        // cylinder across its neck; for the egg, a block across one end
        // and a cylinder through its top.
        let block = |size: [f64; 3], position: [f64; 3]| {
            move |s: &mut Session| {
                run(
                    s,
                    "feature.block",
                    json!({"name": "Tool", "size": size.map(|v| v * k),
                           "position": position.map(|v| v * k)}),
                );
            }
        };
        let cylinder = |radius: f64, height: f64, position: [f64; 3], axis: [f64; 3]| {
            move |s: &mut Session| {
                run(
                    s,
                    "feature.cylinder",
                    json!({"name": "Tool", "radius": radius * k, "height": height * k,
                           "position": position.map(|v| v * k), "axis": axis}),
                );
            }
        };
        identities(
            &format!("vase and block x{k}"),
            &vase("360 deg"),
            &block([9.0, 3.0, 5.0], [1.0, 2.0, -1.5]),
            k,
        );
        // Every pair at scale 1; at the others one with the vase and
        // one with the egg.
        if k == 1.0 {
            identities(
                &format!("vase and cylinder x{k}"),
                &vase("360 deg"),
                &cylinder(1.2, 9.0, [-4.5, 7.0, 0.3], [1.0, 0.0, 0.0]),
                k,
            );
            identities(
                &format!("third of a vase and block x{k}"),
                &vase("120 deg"),
                &block([9.0, 3.0, 5.0], [1.0, 2.0, -1.5]),
                k,
            );
            identities(
                &format!("egg and block x{k}"),
                &egg,
                &block([4.0, 5.0, 2.0], [6.5, 3.0, 1.5]),
                k,
            );
        }
        identities(
            &format!("egg and cylinder x{k}"),
            &egg,
            &cylinder(1.5, 6.0, [5.5, 6.0, -1.0], [0.0, 0.3, 1.0]),
            k,
        );
    }
}

/// A dome between two points of the axis, where its spline ends.
const DOME: [P; 5] = [[0.0, 0.0], [4.0, 0.0], [6.0, 3.0], [3.0, 6.0], [0.0, 6.0]];
/// The wall of a vase with a round bottom: from the axis at (0, 0) out
/// and up to its rim at (2.5, 9).
const ROUND: [P; 6] = [
    [0.0, 0.0],
    [3.0, 0.0],
    [5.0, 1.0],
    [5.5, 4.0],
    [2.0, 6.0],
    [2.5, 9.0],
];

/// The dome's section in sketch `Profile`: the spline e6 from e1 on the
/// axis to e5 on the axis, and the line e7 back down the axis.
fn draw_dome(session: &mut Session, k: f64) {
    run(
        session,
        "sketch.spline",
        json!({"sketch": "Profile", "control": scaled(&DOME, k)}),
    );
    run(
        session,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e5", "to": "e1"}),
    );
}

/// The section of a vase with a round bottom: the spline e7 from e1 on
/// the axis to e6, the rim from e6 to the axis and the axis back down.
fn draw_round_vase(session: &mut Session, k: f64) {
    run(
        session,
        "sketch.spline",
        json!({"sketch": "Profile", "control": scaled(&ROUND, k)}),
    );
    run(
        session,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e6", "to": [0.0, 9.0 * k]}),
    );
    run(
        session,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e8", "to": "e1"}),
    );
}

/// A spline that ends on the axis sweeps a face whose surface closes into
/// a point there (a pole). Refused until 2026-10-14, when the measure and
/// the booleans learnt to take such a face: a dome whose spline starts
/// and ends on the axis and a vase with a round bottom are solids of the
/// size Pappus gives, pass the deep check, mesh watertight and come back
/// from a file as they were saved.
#[test]
fn a_spline_that_ends_on_the_axis_sweeps_a_dome() {
    type Draw = fn(&mut Session, f64);
    let shapes: [(&str, Draw, &[P], f64); 2] = [
        ("dome", draw_dome, &DOME, 0.0),
        ("round vase", draw_round_vase, &ROUND, 2.5),
    ];
    for k in [1e-3, 1.0, 1e6] {
        for (shape, draw, control, rim) in shapes {
            for turn in ["360 deg", "90 deg"] {
                let name = format!("{shape} x{k} through {turn}");
                let dir = workdir(&format!("pole-{shape}-{k}-{turn}"));
                let mut s = session_in(&dir);
                new_document(&mut s, k);
                draw(&mut s, k);
                let made = run(
                    &mut s,
                    "feature.revolve",
                    json!({"sketch": "Profile", "axis": "y", "angle": turn, "name": "Solid"}),
                );
                assert_eq!(made["status"], "ok", "{name}: {made}");
                let sums = along(3, &scaled(control, k));
                let rim = rim * k;
                let (section, per_radian, surface) =
                    (sums[0].abs(), sums[1].abs(), sums[2] + rim * rim / 2.0);
                let full = turn == "360 deg";
                let angle = if full { TAU } else { PI / 2.0 };
                let caps = if full { 0.0 } else { 2.0 * section };
                let solid = body(&mut s, "Solid");
                assert_eq!(solid["kind"], "solid", "{name}");
                close(
                    solid["volume"].as_f64().unwrap(),
                    angle * per_radian,
                    1e-9,
                    &name,
                );
                close(
                    solid["area"].as_f64().unwrap(),
                    angle * surface + caps,
                    1e-9,
                    &format!("{name} area"),
                );
                let check = run(&mut s, "document.check", json!({"deep": true}));
                assert_eq!(check["ok"], true, "{name}: {check}");
                let exported = run(
                    &mut s,
                    "file.export",
                    json!({"path": "solid.3mf", "tolerance": 1e-3 * k}),
                );
                assert_eq!(
                    (&exported["bodies"], &exported["closed"]),
                    (&json!(1), &json!(1)),
                    "{name}"
                );
                let before = run(&mut s, "document.measure", json!({}));
                let inspected = run(&mut s, "document.inspect", json!({}));
                run(&mut s, "file.save", json!({"path": "solid.3dcraft"}));
                run(&mut s, "file.close", json!({}));
                run(&mut s, "file.open", json!({"path": "solid.3dcraft"}));
                assert_eq!(run(&mut s, "document.measure", json!({})), before, "{name}");
                assert_eq!(
                    run(&mut s, "document.inspect", json!({}))["bodies"],
                    inspected["bodies"],
                    "{name}"
                );
            }
        }
    }
}

/// Booleans through the commands on what a spline sweeps that ends on the
/// axis: a block through the side of the dome and one over its pole, a
/// block through the round bottom of the vase, a cylinder across it; the
/// result passes the deep check, and the three results keep the
/// identities of sets. One of them saved and reopened.
#[test]
fn booleans_with_a_dome_keep_the_set_identities() {
    for k in [1e-3, 1.0, 1e6] {
        let solid = |draw: fn(&mut Session, f64), turn: &'static str| {
            move |s: &mut Session| {
                draw(s, k);
                let made = run(
                    s,
                    "feature.revolve",
                    json!({"sketch": "Profile", "axis": "y", "angle": turn, "name": "Body"}),
                );
                assert_eq!(made["status"], "ok", "{made}");
            }
        };
        let block = |size: [f64; 3], position: [f64; 3]| {
            move |s: &mut Session| {
                run(
                    s,
                    "feature.block",
                    json!({"name": "Tool", "size": size.map(|v| v * k),
                           "position": position.map(|v| v * k)}),
                );
            }
        };
        let cylinder = |radius: f64, height: f64, position: [f64; 3], axis: [f64; 3]| {
            move |s: &mut Session| {
                run(
                    s,
                    "feature.cylinder",
                    json!({"name": "Tool", "radius": radius * k, "height": height * k,
                           "position": position.map(|v| v * k), "axis": axis}),
                );
            }
        };
        identities(
            &format!("dome and block x{k}"),
            &solid(draw_dome, "360 deg"),
            &block([6.0, 2.0, 2.0], [2.0, 1.5, -1.0]),
            k,
        );
        identities(
            &format!("dome and a block over its pole x{k}"),
            &solid(draw_dome, "360 deg"),
            &block([2.2, 2.0, 2.4], [-0.9, 5.0, -1.1]),
            k,
        );
        if k == 1.0 {
            identities(
                &format!("round vase and a block through its bottom x{k}"),
                &solid(draw_round_vase, "360 deg"),
                &block([2.4, 3.0, 2.0], [-1.1, -1.0, -0.8]),
                k,
            );
            identities(
                &format!("quarter of a round vase and cylinder x{k}"),
                &solid(draw_round_vase, "90 deg"),
                &cylinder(1.3, 9.0, [-1.0, 3.0, -1.5], [1.0, 0.2, 0.9]),
                k,
            );
        }
    }
    // Cut, saved and reopened: the same bodies.
    let dir = workdir("pole-boolean");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    draw_dome(&mut s, 1.0);
    run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Profile", "axis": "y", "name": "Body"}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"name": "Tool", "size": [2.2, 2.0, 2.4], "position": [-0.9, 5.0, -1.1]}),
    );
    let made = run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Body", "tool": "Tool", "name": "Notched"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
    let before = run(&mut s, "document.measure", json!({}));
    let inspected = run(&mut s, "document.inspect", json!({}));
    run(&mut s, "file.save", json!({"path": "notched.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "notched.3dcraft"}));
    assert_eq!(run(&mut s, "document.measure", json!({})), before);
    assert_eq!(
        run(&mut s, "document.inspect", json!({}))["bodies"],
        inspected["bodies"]
    );
}

#[test]
fn what_cannot_be_swept_fails_with_the_reason() {
    let mut s = session();
    new_document(&mut s, 1.0);
    // A wall that dips onto the axis between its ends.
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": [5, 0], "to": [3, 0]}),
    );
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Profile", "degree": 2, "control": ["e2", [-3, 4], [3, 8]]}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e5", "to": [5, 8]}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e7", "to": "e1"}),
    );
    let made = run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Profile", "axis": "y", "name": "Pinched"}),
    );
    assert_eq!(made["status"], "failed", "{made}");
    let reason = made["error"].as_str().unwrap();
    assert!(
        reason.contains("e6") && reason.contains("reaches the axis"),
        "{reason}"
    );
    // About an axis it stays clear of, the same sketch is a solid.
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": [-2, 0], "to": [-2, 8], "construction": true}),
    );
    let made = run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Profile", "axis": "e12", "angle": "200 deg", "name": "Clear"}),
    );
    assert_eq!(made["status"], "ok", "{made}");

    // (A spline that ends on the axis is no failure any more: see
    // `a_spline_that_ends_on_the_axis_sweeps_a_dome`.)
    let mut s = session();
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Profile", "control": [[0, 0], [4, 0], [6, 3], [3, 6], [0, 6]]}),
    );
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": "e5", "to": "e1"}),
    );
    // Its own straight side as the axis is that same dome (it was the
    // same refusal); about a line it stays clear of, the sketch is a
    // solid too.
    let made = run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Profile", "axis": "e7", "angle": "90 deg", "name": "Dome"}),
    );
    assert_eq!(made["status"], "ok", "its own line is the axis: {made}");
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": [-3, 0], "to": [-3, 6], "construction": true}),
    );
    let made = run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Profile", "axis": "e10", "angle": "90 deg", "name": "Dome"}),
    );
    assert_eq!(made["status"], "ok", "{made}");
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");

    // A closed spline that crosses itself, and one that crosses the axis.
    let mut s = session();
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "sketch.spline",
        json!({"sketch": "Profile", "closed": true,
               "control": [[0, 0], [4, 4], [9, -1], [9, 5], [4, 0], [0, 4]]}),
    );
    let made = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Profile", "distance": 2, "name": "Eight"}),
    );
    assert_eq!(made["status"], "failed", "{made}");
    let reason = made["error"].as_str().unwrap();
    // Periodic, eight points (the seam and one more made): the spline is e9.
    assert!(reason.contains("e9 crosses or touches itself"), "{reason}");
    let mut s = session();
    new_document(&mut s, 1.0);
    draw_egg(&mut s, 1.0, 3);
    let made = run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Profile", "axis": "x", "name": "Across"}),
    );
    assert_eq!(made["status"], "ok", "the egg is above the x axis: {made}");
    run(
        &mut s,
        "sketch.line",
        json!({"sketch": "Profile", "from": [6, -5], "to": [6, 15], "construction": true}),
    );
    let made = run(
        &mut s,
        "feature.revolve",
        json!({"sketch": "Profile", "axis": "e10", "name": "Through"}),
    );
    assert_eq!(made["status"], "failed", "{made}");
    let reason = made["error"].as_str().unwrap();
    assert!(
        reason.contains("both sides of the axis") && reason.contains("e7"),
        "{reason}"
    );
    // Bad parameters are errors, not panics.
    for params in [
        json!({"sketch": "Profile", "axis": "e7"}),
        json!({"sketch": "Profile", "axis": "y", "angle": "0 deg"}),
        json!({"sketch": "Profile", "axis": "y", "angle": "400 deg"}),
        json!({"sketch": "Nope", "axis": "y"}),
    ] {
        match s.execute("feature.revolve", &params) {
            Ok(made) => assert_eq!(made["status"], "failed", "{params}: {made}"),
            Err(e) => assert!(!e.message().contains("internal error"), "{params}: {e}"),
        }
    }
    let _ = error;
}
