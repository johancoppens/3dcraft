//! Lofts through the commands: sketches on offset and datum planes,
//! `feature.loft`, and what the document says of the result. Every solid
//! with a closed form is measured against it at three scales and checked
//! deep; the others are held to identities (a boolean's set identities,
//! a file that reopens to the same bodies, an edit that regenerates to
//! what a full regeneration gives, which `run` asserts after every
//! command).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{
    deep, failure, feature_brief as feature, inspect, refused_with as refused, run,
};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::{PI, TAU};
use std::path::PathBuf;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

type P = [f64; 2];

fn workdir(name: &str) -> PathBuf {
    common::workdir("loft", name)
}

fn new() -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({}));
    s
}

/// A sketch `name` on the xy plane at height `z` with a closed polygon:
/// its points are e1..en, its sides e(n+1)..e(2n).
fn polygon(s: &mut Session, name: &str, z: f64, points: &[P]) {
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": z, "name": name}),
    );
    run(
        s,
        "sketch.polyline",
        json!({"sketch": name, "closed": true, "points": points}),
    );
}

fn square(half: f64) -> Vec<P> {
    vec![[-half, -half], [half, -half], [half, half], [-half, half]]
}

/// A sketch `name` at height `z` with a circle: its centre is e1, the
/// circle e2.
fn circle(s: &mut Session, name: &str, z: f64, center: P, radius: f64) {
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": z, "name": name}),
    );
    run(
        s,
        "sketch.circle",
        json!({"sketch": name, "center": center, "radius": radius}),
    );
}

fn turned(points: &[P], angle: f64) -> Vec<P> {
    let (s, c) = angle.psin_cos();
    points
        .iter()
        .map(|p| [p[0] * c - p[1] * s, p[0] * s + p[1] * c])
        .collect()
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

fn face_names(s: &mut Session, name: &str) -> Vec<String> {
    body(s, name)["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect()
}

fn close(a: f64, b: f64, relative: f64) -> bool {
    (a - b).abs() <= relative * b.abs()
}

/// See `twisted_volume` in the kernel's loft tests: the cross-section
/// between matched polygons is a polygon whose area is quadratic in the
/// height.
fn twisted_volume(p: &[P], q: &[P], h: f64) -> f64 {
    let cross = |a: P, b: P| a[0] * b[1] - a[1] * b[0];
    let n = p.len();
    let (mut a0, mut a1, mut mixed) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let j = (i + 1) % n;
        a0 += 0.5 * cross(p[i], p[j]);
        a1 += 0.5 * cross(q[i], q[j]);
        mixed += 0.5 * (cross(p[i], q[j]) + cross(q[i], p[j]));
    }
    h * (a0 / 3.0 + mixed / 6.0 + a1 / 3.0)
}

#[test]
fn two_profiles_give_the_solids_with_closed_forms_at_every_scale() {
    for k in SCALES {
        // A square to the same square: a box, with flat sides one can
        // sketch on.
        let mut s = new();
        polygon(&mut s, "Low", 0.0, &square(k));
        polygon(&mut s, "High", 3.0 * k, &square(k));
        let made = run(
            &mut s,
            "feature.loft",
            json!({"sections": ["Low", "High"], "name": "Box"}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        let found = body(&mut s, "Box");
        assert!(close(
            found["volume"].as_f64().unwrap(),
            12.0 * k.ppowi(3),
            1e-12
        ));
        assert!(close(found["area"].as_f64().unwrap(), 32.0 * k * k, 1e-12));
        assert_eq!(
            face_names(&mut s, "Box"),
            [
                "F3:side(e5,e5)",
                "F3:side(e6,e6)",
                "F3:side(e7,e7)",
                "F3:side(e8,e8)",
                "F3:startCap(e5,e6,e7,e8)",
                "F3:endCap(e5,e6,e7,e8)"
            ]
        );
        let on_side = run(
            &mut s,
            "sketch.create",
            json!({"face": "F3:side(e6,e6)", "name": "OnSide"}),
        );
        assert_eq!(on_side["status"], "ok", "{on_side}");
        deep(&mut s);

        // A circle to a coaxial circle: the frustum of a cone.
        let mut s = new();
        let (r0, r1, h) = (2.0 * k, 0.75 * k, 3.0 * k);
        circle(&mut s, "Low", 0.0, [k, -k], r0);
        circle(&mut s, "High", h, [k, -k], r1);
        run(
            &mut s,
            "feature.loft",
            json!({"sections": ["Low", "High"], "name": "Frustum"}),
        );
        let found = body(&mut s, "Frustum");
        let exact = PI * h / 3.0 * (r0 * r0 + r0 * r1 + r1 * r1);
        assert!(close(found["volume"].as_f64().unwrap(), exact, 1e-12));
        let slant = (h * h + (r0 - r1).ppowi(2)).sqrt();
        let exact = PI * (r0 + r1) * slant + PI * (r0 * r0 + r1 * r1);
        assert!(close(found["area"].as_f64().unwrap(), exact, 1e-12));
        assert_eq!(
            face_names(&mut s, "Frustum"),
            ["F3:side(e2,e2)", "F3:startCap(e2)", "F3:endCap(e2)"]
        );
        deep(&mut s);

        // A square to a smaller square: the frustum of a pyramid.
        let mut s = new();
        let (a, b, h) = (2.0 * k, 0.5 * k, 5.0 * k);
        polygon(&mut s, "Low", 0.0, &square(a));
        polygon(&mut s, "High", h, &square(b));
        run(
            &mut s,
            "feature.loft",
            json!({"sections": ["Low", "High"], "name": "Pyramid"}),
        );
        let (a1, a2) = (4.0 * a * a, 4.0 * b * b);
        let exact = h / 3.0 * (a1 + a2 + (a1 * a2).sqrt());
        assert!(close(volume(&mut s, "Pyramid"), exact, 1e-12));
        deep(&mut s);

        // A rectangle to the same rectangle turned by 30 degrees: a
        // twisted prism, whose sides are not flat.
        let mut s = new();
        let corners = [[-2.0 * k, -k], [2.0 * k, -k], [2.0 * k, k], [-2.0 * k, k]];
        let above = turned(&corners, 30f64.to_radians());
        polygon(&mut s, "Low", 0.0, &corners);
        polygon(&mut s, "High", 4.0 * k, &above);
        run(
            &mut s,
            "feature.loft",
            json!({"sections": ["Low", "High"], "name": "Twist"}),
        );
        let exact = twisted_volume(&corners, &above, 4.0 * k);
        assert!(close(volume(&mut s, "Twist"), exact, 1e-9));
        deep(&mut s);
        let on_side = run(
            &mut s,
            "sketch.create",
            json!({"face": "F3:side(e6,e6)", "name": "OnSide"}),
        );
        assert_eq!(on_side["status"], "failed", "{on_side}");
        assert!(on_side["error"].as_str().unwrap().contains("flat"));
        // Started one curve on, the twist is 120 degrees the other way.
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Twist", "sections": ["Low", {"sketch": "High", "start": "e6"}]}),
        );
        let shifted: Vec<P> = (0..4).map(|i| above[(i + 1) % 4]).collect();
        let exact = twisted_volume(&corners, &shifted, 4.0 * k);
        assert!(close(volume(&mut s, "Twist"), exact, 1e-9));
        assert!(face_names(&mut s, "Twist").contains(&"F3:side(e5,e6)".to_string()));
    }
}

#[test]
fn several_profiles_are_passed_through_smoothly_or_ruled() {
    for k in SCALES {
        // Circles whose radii grow evenly with the height: a cone, smooth
        // or ruled.
        let mut s = new();
        for i in 0..4 {
            let z = f64::from(i) * 2.0 * k;
            circle(&mut s, &format!("S{i}"), z, [0.0, 0.0], k + 0.25 * z);
        }
        let sections = json!(["S0", "S1", "S2", "S3"]);
        run(
            &mut s,
            "feature.loft",
            json!({"sections": sections, "name": "Cone"}),
        );
        let (r0, r1, h) = (k, 2.5 * k, 6.0 * k);
        let exact = PI * h / 3.0 * (r0 * r0 + r0 * r1 + r1 * r1);
        assert!(close(volume(&mut s, "Cone"), exact, 1e-9));
        assert_eq!(face_names(&mut s, "Cone").len(), 6);
        deep(&mut s);
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Cone", "ruled": true}),
        );
        assert!(close(volume(&mut s, "Cone"), exact, 1e-9));
        assert_eq!(
            face_names(&mut s, "Cone"),
            [
                "F5:side(e2,e2)",
                "F5:side(e2,e2,e2)",
                "F5:side(e2,e2,e2,e2)",
                "F5:startCap(e2)",
                "F5:endCap(e2)"
            ]
        );
        deep(&mut s);

        // A square, a circle and a square: cut to match, smooth.
        let mut s = new();
        polygon(&mut s, "Foot", 0.0, &square(2.0 * k));
        circle(&mut s, "Waist", 3.0 * k, [0.0, 0.0], k);
        polygon(&mut s, "Head", 6.0 * k, &square(1.5 * k));
        let made = run(
            &mut s,
            "feature.loft",
            json!({"sections": ["Foot", "Waist", "Head"], "name": "Vase"}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        let smooth = volume(&mut s, "Vase");
        deep(&mut s);
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Vase", "ruled": true}),
        );
        let ruled = volume(&mut s, "Vase");
        deep(&mut s);
        // Between the prism of the waist and that of the foot, and the
        // smooth one is not the ruled one.
        for v in [smooth, ruled] {
            assert!(v > PI * k * k * 6.0 * k && v < 16.0 * k * k * 6.0 * k);
        }
        assert!(!close(smooth, ruled, 1e-3));
    }
}

/// The datum plane `name`: the zx plane turned by `degrees` about z.
/// Returns its origin, x axis and y axis (normal × x).
fn spoke(s: &mut Session, name: &str, degrees: f64) -> [[f64; 3]; 3] {
    let made = run(
        s,
        "datum.plane",
        json!({"from": "zx", "about": "z", "angle": format!("{degrees} deg"), "name": name}),
    );
    let v = |key: &str| -> [f64; 3] {
        let a = made["datum"][key].as_array().unwrap();
        [
            a[0].as_f64().unwrap(),
            a[1].as_f64().unwrap(),
            a[2].as_f64().unwrap(),
        ]
    };
    let (o, n, x) = (v("origin"), v("normal"), v("x"));
    let y = [
        n[1] * x[2] - n[2] * x[1],
        n[2] * x[0] - n[0] * x[2],
        n[0] * x[1] - n[1] * x[0],
    ];
    [o, x, y]
}

/// A rectangle sketched on datum plane `plane`; returns its area times
/// the distance of its centre from the world's z axis.
fn rectangle_on(s: &mut Session, plane: &str, frame: [[f64; 3]; 3], x: P, y: P) -> f64 {
    let name = format!("On{plane}");
    run(s, "sketch.create", json!({"plane": plane, "name": name}));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": name, "closed": true,
               "points": [[x[0], y[0]], [x[1], y[0]], [x[1], y[1]], [x[0], y[1]]]}),
    );
    let (cx, cy) = ((x[0] + x[1]) / 2.0, (y[0] + y[1]) / 2.0);
    let [o, ax, ay] = frame;
    let c: Vec<f64> = (0..3).map(|i| o[i] + ax[i] * cx + ay[i] * cy).collect();
    (x[1] - x[0]) * (y[1] - y[0]) * c[0].phypot(c[1])
}

#[test]
fn profiles_on_datum_planes_that_are_not_parallel_make_a_bend_and_a_ring() {
    for k in SCALES {
        // A bend: the same rectangle on planes through the z axis.
        let mut s = new();
        let angles = [0.0, 30.0, 65.0, 90.0];
        let mut moment = 0.0;
        let mut sections = Vec::new();
        for (i, angle) in angles.iter().enumerate() {
            let plane = format!("P{i}");
            let frame = spoke(&mut s, &plane, *angle);
            // Along the axis the frame's x or y is z: away from the axis
            // is the other one. Put the rectangle off the axis either way.
            let radial_is_x = frame[1][2].abs() < 0.5;
            let (x, y) = if radial_is_x {
                ([3.0 * k, 5.0 * k], [-k, k])
            } else {
                ([-k, k], [3.0 * k, 5.0 * k])
            };
            moment = rectangle_on(&mut s, &plane, frame, x, y);
            sections.push(format!("On{plane}"));
        }
        assert!(close(moment, 4.0 * k * k * 4.0 * k, 1e-12));
        let made = run(
            &mut s,
            "feature.loft",
            json!({"sections": sections, "ruled": true, "name": "Bend"}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        // Each band is the wedge between two planes through the axis:
        // sin(angle) times the area times the distance of its centre.
        let exact: f64 = angles
            .windows(2)
            .map(|w| (w[1] - w[0]).to_radians().psin() * moment)
            .sum();
        assert!(close(volume(&mut s, "Bend"), exact, 1e-11));
        deep(&mut s);
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Bend", "ruled": false}),
        );
        let smooth = volume(&mut s, "Bend");
        assert!(close(smooth, PI / 2.0 * moment, 2e-2) && smooth > exact);
        deep(&mut s);

        // A ring: the rectangle on six planes round the axis, closed.
        let mut s = new();
        let mut sections = Vec::new();
        for i in 0..6 {
            let plane = format!("P{i}");
            let frame = spoke(&mut s, &plane, 60.0 * f64::from(i));
            let radial_is_x = frame[1][2].abs() < 0.5;
            let (x, y) = if radial_is_x {
                ([3.0 * k, 5.0 * k], [-k, k])
            } else {
                ([-k, k], [3.0 * k, 5.0 * k])
            };
            rectangle_on(&mut s, &plane, frame, x, y);
            sections.push(format!("On{plane}"));
        }
        let made = run(
            &mut s,
            "feature.loft",
            json!({"sections": sections, "ruled": true, "closed": true, "name": "Ring"}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        let exact = 6.0 * (TAU / 6.0).psin() * moment;
        assert!(close(volume(&mut s, "Ring"), exact, 1e-11));
        assert_eq!(face_names(&mut s, "Ring").len(), 24);
        deep(&mut s);
        run(
            &mut s,
            "feature.edit",
            json!({"feature": "Ring", "ruled": false}),
        );
        assert!(close(volume(&mut s, "Ring"), TAU * moment, 1e-2));
        deep(&mut s);
    }
}

/// A leaf of two splines in sketch `name` at height `z`.
fn leaf(s: &mut Session, name: &str, z: f64, k: f64, fat: f64) {
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": z, "name": name}),
    );
    run(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": [[-2.0 * k, 0.0], [-k, -fat * k], [0.5 * k, -1.2 * fat * k], [2.0 * k, 0.0]]}),
    );
    // Back over the top, between the same two points.
    run(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": ["e4", [k, fat * k], [-k, 0.8 * fat * k], "e1"]}),
    );
}

#[test]
fn a_leaf_of_splines_lofts_and_survives_saving_and_booleans() {
    let dir = workdir("leaf");
    for k in SCALES {
        let mut s = Session::new(Workspace {
            read: Access::Root(dir.clone()),
            write: Access::Root(dir.clone()),
        });
        // The tolerance follows the size at 1e6, as in the STEP tests: a
        // boolean refuses freeform faces of that size at a tolerance of
        // 1e-6 ("the tolerance is finer than rounding allows at its
        // size"; `a_loft_at_1e6_in_a_boolean` in the kernel's loft tests
        // is the reproducer). The loft itself needs no such thing.
        run(&mut s, "file.new", json!({"tolerance": 1e-6 * k.max(1.0)}));
        leaf(&mut s, "Small", 0.0, k, 1.0);
        leaf(&mut s, "Large", 4.0 * k, 1.5 * k, 1.4);
        leaf(&mut s, "Tip", 7.0 * k, 0.8 * k, 0.7);
        let made = run(
            &mut s,
            "feature.loft",
            json!({"sections": ["Small", "Large", "Tip"], "name": "Leaf"}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        deep(&mut s);
        let lofted = volume(&mut s, "Leaf");
        let before = inspect(&mut s)["bodies"].clone();
        run(&mut s, "file.save", json!({"path": "leaf.3dcraft"}));
        run(&mut s, "file.close", json!({}));
        run(&mut s, "file.open", json!({"path": "leaf.3dcraft"}));
        assert_eq!(inspect(&mut s)["bodies"], before);

        // With a block and with a cylinder: |A ∪ B| + |A ∩ B| = |A| + |B|
        // and |A − B| = |A| − |A ∩ B|.
        for tool in ["block", "cylinder"] {
            let mut volumes = Vec::new();
            for op in ["union", "intersect", "subtract"] {
                run(&mut s, "file.close", json!({"discard": true}));
                run(&mut s, "file.open", json!({"path": "leaf.3dcraft"}));
                let other = if tool == "block" {
                    run(
                        &mut s,
                        "feature.block",
                        json!({"position": [-0.5 * k, -3.0 * k, 2.0 * k], "size": [4.0 * k, 6.0 * k, 3.0 * k], "name": "Tool"}),
                    );
                    72.0 * k.ppowi(3)
                } else {
                    run(
                        &mut s,
                        "feature.cylinder",
                        json!({"position": [0.3 * k, 0.1 * k, -k], "radius": 0.6 * k, "height": 10.0 * k, "name": "Tool"}),
                    );
                    PI * 0.36 * 10.0 * k.ppowi(3)
                };
                let made = run(
                    &mut s,
                    "feature.boolean",
                    json!({"op": op, "target": "Leaf", "tool": "Tool", "name": "Result"}),
                );
                assert_eq!(made["status"], "ok", "{tool} {op} at {k}: {made}");
                deep(&mut s);
                volumes.push((volume(&mut s, "Result"), other));
            }
            let (union, other) = volumes[0];
            let (common, difference) = (volumes[1].0, volumes[2].0);
            assert!(
                close(union + common, lofted + other, 1e-7),
                "{tool} at {k}: {union} + {common} against {lofted} + {other}"
            );
            assert!(close(difference, lofted - common, 1e-7), "{tool} at {k}");
            assert!(common > 0.0 && common < lofted.min(other));
        }
    }
}

#[test]
fn names_survive_edits_and_a_lost_sketch_fails_the_loft_alone() {
    let k = 10.0;
    let mut s = new();
    polygon(&mut s, "Low", 0.0, &square(2.0 * k));
    polygon(&mut s, "High", 5.0 * k, &square(k));
    run(
        &mut s,
        "feature.block",
        json!({"position": [50, 50, 0], "size": [5, 5, 5], "name": "Bystander"}),
    );
    run(
        &mut s,
        "feature.loft",
        json!({"sections": ["Low", "High"], "name": "Tower"}),
    );
    // A sketch on the end cap, extruded: it stands on the loft.
    let cap = "F4:endCap(e5,e6,e7,e8)";
    assert!(face_names(&mut s, "Tower").contains(&cap.to_string()));
    run(
        &mut s,
        "sketch.create",
        json!({"face": cap, "name": "OnTop"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "OnTop", "center": [0, 0], "radius": 4}),
    );
    let pin = run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "OnTop", "distance": 7, "name": "Pin"}),
    );
    assert_eq!(pin["status"], "ok", "{pin}");
    let centroid = |s: &mut Session| body(s, "Pin")["centroid"][2].as_f64().unwrap();
    assert!((centroid(&mut s) - (5.0 * k + 3.5)).abs() < 1e-9);
    // Its edges and vertices have names, by the faces they lie on.
    let edges = run(&mut s, "document.edges", json!({"feature": "Tower"}));
    let names: Vec<&str> = edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 12);
    assert!(
        names.contains(&"F4:between[F4:startCap(e5,e6,e7,e8),F4:side(e5,e5)]"),
        "{names:?}"
    );
    assert!(
        names.contains(&"F4:between[F4:side(e5,e5),F4:side(e6,e6)]"),
        "{names:?}"
    );
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 12);

    // An upstream edit of a profile: a corner of the lower square moves.
    // The loft follows, the pin stays where it is, and what was built is
    // what a full regeneration builds (`run` asserts that).
    let before = volume(&mut s, "Tower");
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "Low", "entity": "e1", "at": [-3.0 * k, -3.0 * k]}),
    );
    let after = volume(&mut s, "Tower");
    assert!(after > before);
    assert_eq!(feature(&mut s, "Pin")["status"], "ok");
    assert!((centroid(&mut s) - (5.0 * k + 3.5)).abs() < 1e-9);
    deep(&mut s);

    // The upper sketch moves up through a plane offset: the pin goes along.
    run(
        &mut s,
        "sketch.edit",
        json!({"sketch": "High", "offset": 8.0 * k}),
    );
    assert!((centroid(&mut s) - (8.0 * k + 3.5)).abs() < 1e-9);

    // A sketch that fails takes the loft with it, which says which
    // section; the pin fails for the loft, and nothing else does.
    run(&mut s, "feature.suppress", json!({"feature": "High"}));
    let reason = failure(&mut s, "Tower");
    assert!(
        reason.starts_with("sections: section 2: `High` is suppressed"),
        "{reason}"
    );
    assert_eq!(feature(&mut s, "Bystander")["status"], "ok");
    assert_eq!(feature(&mut s, "OnTop")["status"], "failed");
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "High", "suppressed": false}),
    );
    assert_eq!(feature(&mut s, "Tower")["status"], "ok");

    // An open profile fails its sketch's use, and the loft names it.
    run(
        &mut s,
        "sketch.delete",
        json!({"sketch": "High", "entity": "e8"}),
    );
    let reason = failure(&mut s, "Tower");
    assert!(
        reason.starts_with("sections: section 2: sketch `High`"),
        "{reason}"
    );
    assert!(reason.contains("not closed"), "{reason}");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(feature(&mut s, "Tower")["status"], "ok");

    // Deleted, the sketch is gone: the loft fails with the parameter, the
    // pin fails because of the loft (causedBy), the bystander stands.
    run(&mut s, "feature.delete", json!({"feature": "High"}));
    let reason = failure(&mut s, "Tower");
    assert!(
        reason.starts_with("sections: section 2: there is no feature"),
        "{reason}"
    );
    assert_eq!(feature(&mut s, "Bystander")["status"], "ok");
    let pin = feature(&mut s, "Pin");
    assert_eq!(pin["status"], "failed");
    // Re-pointed at another sketch, the loft is built again.
    polygon(&mut s, "Other", 6.0 * k, &square(1.5 * k));
    run(
        &mut s,
        "feature.move",
        json!({"feature": "Other", "position": 1}),
    );
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Tower", "sections": ["Low", "Other"]}),
    );
    assert_eq!(feature(&mut s, "Tower")["status"], "ok");
    deep(&mut s);
}

#[test]
fn cut_join_and_intersect_combine_a_loft_at_once() {
    let k = 1.0;
    for (how, expected) in [
        ("cut", 1000.0 - 28.0 / 3.0 * 3.0),
        ("join", 1000.0),
        ("intersect", 28.0),
    ] {
        let mut s = new();
        run(
            &mut s,
            "feature.block",
            json!({"position": [-5, -5, 0], "size": [10, 10, 10], "name": "Stock"}),
        );
        polygon(&mut s, "Low", 2.0, &square(2.0 * k));
        polygon(&mut s, "High", 5.0, &square(k));
        let mut params = json!({"sections": ["Low", "High"], "name": "Result"});
        params[how] = "Stock".into();
        let made = run(&mut s, "feature.loft", params);
        assert_eq!(made["status"], "ok", "{how}: {made}");
        assert_eq!(made["sweep"]["status"], "ok", "{made}");
        // The frustum of a pyramid, 3 high: (16 + 4 + 8) · 3 / 3 = 28.
        assert!(close(volume(&mut s, "Result"), expected, 1e-9), "{how}");
        deep(&mut s);
    }
}

#[test]
fn what_cannot_be_lofted_is_refused_or_fails_with_the_reason() {
    let k = 1.0;
    let mut s = new();
    polygon(&mut s, "Low", 0.0, &square(k));
    polygon(&mut s, "High", 3.0, &square(k));
    run(
        &mut s,
        "feature.block",
        json!({"position": [9, 9, 9], "size": [1, 1, 1], "name": "Block"}),
    );
    // Parameters that are not a loft: refused, nothing added.
    let count = inspect(&mut s)["features"].as_array().unwrap().len();
    for (params, word) in [
        (json!({}), "sections"),
        (json!({"sections": "Low"}), "list of sketches"),
        (json!({"sections": ["Low"]}), "at least 2"),
        (json!({"sections": []}), "at least 2"),
        (json!({"sections": ["Low", "Nowhere"]}), "no feature named"),
        (json!({"sections": ["Low", "Block"]}), "not a sketch"),
        (json!({"sections": ["Low", "Low"]}), "each sketch once"),
        (json!({"sections": ["Low", 99]}), "no feature F99"),
        (json!({"sections": ["Low", -1.5]}), "feature id or name"),
        (
            json!({"sections": ["Low", {"start": "e5"}]}),
            "missing `sketch`",
        ),
        (
            json!({"sections": ["Low", {"sketch": "High", "start": "top"}]}),
            "curve of the sketch",
        ),
        (
            json!({"sections": ["Low", {"sketch": "High", "twist": 1}]}),
            "no parameter `twist`",
        ),
        (
            json!({"sections": ["Low", "High"], "closed": true}),
            "at least 3",
        ),
        (
            json!({"sections": ["Low", "High"], "ruled": "yes"}),
            "ruled",
        ),
        (
            json!({"sections": ["Low", "High"], "cut": "Nowhere"}),
            "Nowhere",
        ),
        (json!({"sections": ["Low", "High"], "twist": 3}), "twist"),
    ] {
        let message = refused(&mut s, "feature.loft", params.clone());
        assert!(message.contains(word), "{params}: {message}");
    }
    assert_eq!(inspect(&mut s)["features"].as_array().unwrap().len(), count);

    // What regeneration finds: the feature is there and failed.
    let fails = |s: &mut Session, sections: Value, word: &str| {
        let made = run(
            s,
            "feature.loft",
            json!({"sections": sections, "name": "Bad"}),
        );
        assert_eq!(made["status"], "failed", "{made}");
        let reason = made["error"].as_str().unwrap().to_string();
        assert!(reason.starts_with("sections: "), "{reason}");
        assert!(reason.contains(word), "{reason}");
        run(s, "feature.delete", json!({"feature": "Bad"}));
    };
    // A start that is no curve of the profile (e1 is a point).
    fails(
        &mut s,
        json!(["Low", {"sketch": "High", "start": "e1"}]),
        "e1 is not a curve of the profile",
    );
    // Half a turn of twist: every side folds through the axis.
    fails(
        &mut s,
        json!(["Low", {"sketch": "High", "start": "e7"}]),
        "folds onto itself",
    );
    // Two profiles in one sketch; a profile with a hole; an empty sketch.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "offset": 6, "name": "Two"}),
    );
    for x in [0.0, 5.0] {
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Two", "center": [x, 0], "radius": 1}),
        );
    }
    fails(&mut s, json!(["Low", "Two"]), "2 closed profiles");
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "offset": 6, "name": "Holed"}),
    );
    for r in [1.0, 2.0] {
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Holed", "center": [0, 0], "radius": r}),
        );
    }
    fails(&mut s, json!(["Low", "Holed"]), "has a hole");
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "offset": 6, "name": "Empty"}),
    );
    fails(&mut s, json!(["Low", "Empty"]), "section 2");
    // A sketch in the plane of the first, beside it; and one whose plane
    // cuts through the first.
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Beside"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Beside", "center": [9, 0], "radius": 1}),
    );
    fails(&mut s, json!(["Low", "Beside"]), "lies along the loft");
    run(
        &mut s,
        "datum.plane",
        json!({"from": "xy", "about": "x", "angle": "45 deg", "name": "Tilted"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "Tilted", "offset": 0.5, "name": "Across"}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Across", "center": [0, 0], "radius": 1}),
    );
    fails(
        &mut s,
        json!(["Low", "Across"]),
        "planes cross within the profiles",
    );
    // A bend that runs on into its own beginning passes through itself.
    let mut sections = Vec::new();
    for (i, angle) in [0.0, 166.0, 332.0, 498.0].iter().enumerate() {
        let plane = format!("P{i}");
        let frame = spoke(&mut s, &plane, *angle);
        let radial_is_x = frame[1][2].abs() < 0.5;
        let (x, y) = if radial_is_x {
            ([3.0, 5.0], [-1.0, 1.0])
        } else {
            ([-1.0, 1.0], [3.0, 5.0])
        };
        rectangle_on(&mut s, &plane, frame, x, y);
        sections.push(format!("On{plane}"));
    }
    let made = run(
        &mut s,
        "feature.loft",
        json!({"sections": sections, "ruled": true, "name": "Spiral"}),
    );
    assert_eq!(made["status"], "failed", "{made}");
    let reason = made["error"].as_str().unwrap();
    assert!(reason.contains("passes through itself"), "{reason}");
    assert!(reason.contains("side("), "{reason}");
    // Whatever failed, the document is still sound.
    let check = run(&mut s, "document.check", json!({}));
    assert_eq!(check["bodiesChecked"], 1, "{check}");
}

/// A section of a wind-turbine blade at height `z`: a cambered aerofoil
/// of two splines (`target/work/rotor.steps.json`, the developer's rotor).
fn aerofoil(s: &mut Session, name: &str, z: f64, upper: [P; 4], lower: [P; 2]) {
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": z, "name": name}),
    );
    run(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": upper}),
    );
    run(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": ["e4", lower[0], lower[1], "e1"]}),
    );
}

/// The sections of the twisted rotor blade at 18 m and at the tip, 21.5
/// m: chords of 1.1 m and 0.55 m.
fn outer_blade(s: &mut Session) {
    aerofoil(
        s,
        "S18",
        18.0,
        [
            [-0.329887, -0.008638],
            [-0.332104, 0.076033],
            [-0.004031, 0.153947],
            [0.769736, 0.020156],
        ],
        [[0.167161, -0.080352], [-0.328073, -0.077915]],
    );
    aerofoil(
        s,
        "Tip",
        21.5,
        [[-0.165, 0.0], [-0.165, 0.0363], [0.0, 0.066], [0.385, 0.0]],
        [[0.0825, -0.0363], [-0.165, -0.0297]],
    );
}

/// The outer sections of the twisted rotor blade, where the tip follows
/// the section at 18 m directly: lofted smoothly, the surface turns so
/// tightly around the nose near the tip that the chords of the edge along
/// the nose, within the tolerance of that edge, lay beside the face and
/// over the next one. The self-intersection check's mesh could not be
/// made ("patch 1 needs about 2050 segments per side", before
/// 2026-10-14); now those chords are cut finer. From an aerofoil and from
/// a circle at the root; the ruled loft through the same sections always
/// built, and the two hold each other's volume.
#[test]
fn slender_twisted_blade_sections_loft_smoothly() {
    for root in ["aerofoil", "circle"] {
        let mut s = Session::new(Workspace::none());
        run(&mut s, "file.new", json!({"units": "m", "tolerance": 1e-4}));
        let mut sections = Vec::new();
        if root == "circle" {
            circle(&mut s, "Root", 1.0, [0.0, 0.0], 0.6);
            sections.push("Root");
        }
        aerofoil(
            &mut s,
            "S8",
            8.0,
            [
                [-0.765669, -0.148831],
                [-0.825698, 0.159989],
                [-0.109143, 0.561491],
                [1.786561, 0.347272],
            ],
            [[0.442863, -0.234404], [-0.716555, -0.401502]],
        );
        outer_blade(&mut s);
        sections.extend(["S8", "S18", "Tip"]);
        let made = run(
            &mut s,
            "feature.loft",
            json!({"sections": sections, "name": "Blade"}),
        );
        assert_eq!(made["status"], "ok", "{root}: {made}");
        let ruled = run(
            &mut s,
            "feature.loft",
            json!({"sections": sections, "ruled": true, "name": "Ruled"}),
        );
        assert_eq!(ruled["status"], "ok", "{root}: {ruled}");
        deep(&mut s);
        let (smooth, ruled) = (volume(&mut s, "Blade"), volume(&mut s, "Ruled"));
        assert!(
            close(smooth, ruled, 0.1),
            "{root}: {smooth} against {ruled}"
        );
    }
}

/// From a circle of 1 m at 13 m to the same outer sections, smoothly: the
/// surface through the 12 % thin tip overshoots and its upper and lower
/// sides cross. That is still found and refused, by the faces that cross;
/// ruled, the same sections build.
#[test]
fn a_smooth_blade_whose_sides_cross_is_refused() {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "m", "tolerance": 1e-4}));
    circle(&mut s, "Root", 13.0, [0.0, 0.0], 1.0);
    outer_blade(&mut s);
    let sections = ["Root", "S18", "Tip"];
    let made = run(
        &mut s,
        "feature.loft",
        json!({"sections": sections, "name": "Blade"}),
    );
    assert_eq!(made["status"], "failed", "{made}");
    let reason = made["error"].as_str().unwrap();
    assert!(reason.contains("passes through itself"), "{reason}");
    assert!(reason.contains("cross"), "{reason}");
    assert!(!reason.contains("cannot be meshed"), "{reason}");
    let ruled = run(
        &mut s,
        "feature.loft",
        json!({"sections": sections, "ruled": true, "name": "Ruled"}),
    );
    assert_eq!(ruled["status"], "ok", "{ruled}");
}
