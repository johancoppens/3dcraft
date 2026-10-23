//! The ends of an extrusion through the commands: symmetric about the
//! sketch plane, through all, and up to a face. Every solid is measured
//! against a closed form worked out by hand, at three scales, and checked
//! deep; the names say where the faces came from; the end follows the face
//! it was pointed at, and a lost face fails the feature with the reason.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, failed, new_document, refused, run, session};
use craft3d_engine::{Access, Session, Workspace};
use serde_json::{Value, json};
use std::f64::consts::PI;
use std::path::PathBuf;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn workdir(name: &str) -> PathBuf {
    common::workdir("extrude-ends", name)
}

fn sketch(session: &mut Session, name: &str, plane: &str, offset: f64) {
    run(
        session,
        "sketch.create",
        json!({"name": name, "plane": plane, "offset": offset}),
    );
}

fn rectangle(session: &mut Session, sketch: &str, x: (f64, f64), y: (f64, f64)) {
    run(
        session,
        "sketch.polyline",
        json!({"sketch": sketch, "closed": true,
               "points": [[x.0, y.0], [x.1, y.0], [x.1, y.1], [x.0, y.1]]}),
    );
}

fn circle(session: &mut Session, sketch: &str, center: [f64; 2], radius: f64) {
    run(
        session,
        "sketch.circle",
        json!({"sketch": sketch, "center": center, "radius": radius}),
    );
}

fn face_names(body: &Value) -> Vec<String> {
    let mut names: Vec<String> = body["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

fn assert_close(actual: &Value, expected: f64, relative: f64, what: &str) {
    let actual = actual
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: {actual}"));
    assert!(
        (actual - expected).abs() <= relative * expected.abs(),
        "{what}: {actual}, expected {expected} (off by {:e})",
        (actual - expected) / expected
    );
}

fn assert_measures(body: &Value, volume: f64, area: f64, relative: f64, what: &str) {
    assert_close(
        &body["volume"],
        volume,
        relative,
        &format!("{what}: volume"),
    );
    assert_close(&body["area"], area, relative, &format!("{what}: area"));
}

/// Every body is valid and none passes through itself.
fn assert_sound(session: &mut Session, what: &str) {
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{what}: {check}");
}

fn z_range(body: &Value) -> (f64, f64) {
    (
        body["bbox"][0][2].as_f64().unwrap(),
        body["bbox"][1][2].as_f64().unwrap(),
    )
}

/// A box, half to each side of the sketch plane: `w h d`, and the sketch
/// plane cuts it in two halves.
#[test]
fn a_symmetric_box_has_half_on_each_side() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        let (w, h, d) = (4.0 * k, 3.0 * k, 5.0 * k);
        sketch(&mut s, "Profile", "xy", 2.0 * k);
        rectangle(&mut s, "Profile", (k, k + w), (-k, h - k));
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Profile", "distance": d, "symmetric": true, "name": "Box"}),
        );
        let solid = body(&mut s, "Box");
        assert_measures(
            &solid,
            w * h * d,
            2.0 * (w * h + w * d + h * d),
            1e-12,
            "symmetric",
        );
        let (low, high) = z_range(&solid);
        assert!((low - (2.0 * k - d / 2.0)).abs() < 1e-5 * k, "{solid}");
        assert!((high - (2.0 * k + d / 2.0)).abs() < 1e-5 * k, "{solid}");
        assert_close(&solid["centroid"][2], 2.0 * k, 1e-12, "centroid");
        // Reversed is the same solid: both sides.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Box", "reversed": true}),
        );
        assert_eq!(z_range(&body(&mut s, "Box")), (low, high));
        // One-sided and reversed it hangs below the sketch plane.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Box", "symmetric": false}),
        );
        let solid = body(&mut s, "Box");
        let (low, high) = z_range(&solid);
        assert!((low - (2.0 * k - d)).abs() < 1e-5 * k, "{solid}");
        assert!((high - 2.0 * k).abs() < 1e-5 * k, "{solid}");
        assert_sound(&mut s, "symmetric");
    }
}

/// A round and a square hole through a plate: the plate less the two
/// columns, with their walls added to its surface.
#[test]
fn through_all_goes_through_a_plate() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        let (w, h, t, lift) = (40.0 * k, 30.0 * k, 5.0 * k, 10.0 * k);
        built(
            &mut s,
            "feature.block",
            json!({"size": [w, h, t], "position": [0, 0, lift], "name": "Plate"}),
        );
        // A taller body beside the profile is not in the way.
        built(
            &mut s,
            "feature.block",
            json!({"size": [k, k, 100.0 * k], "position": [60.0 * k, 0, 0], "name": "Mast"}),
        );
        let (r, a) = (3.0 * k, 4.0 * k);
        sketch(&mut s, "Holes", "xy", 0.0);
        circle(&mut s, "Holes", [10.0 * k, 15.0 * k], r);
        rectangle(
            &mut s,
            "Holes",
            (20.0 * k, 20.0 * k + a),
            (10.0 * k, 10.0 * k + a),
        );
        let cut = built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Holes", "throughAll": true, "cut": "Plate", "name": "Drilled"}),
        );
        assert_eq!(cut["sweep"]["status"], "ok", "{cut}");
        let holes = PI * r * r + a * a;
        let solid = body(&mut s, "Drilled");
        assert_measures(
            &solid,
            (w * h - holes) * t,
            2.0 * (w * h - holes) + 2.0 * (w + h) * t + (2.0 * PI * r + 4.0 * a) * t,
            1e-9,
            "plate with holes",
        );
        assert_sound(&mut s, "through all");

        // The plate is made thicker and moved: the holes still go through.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Plate", "size": [w, h, 3.0 * t], "position": [0, 0, 2.0 * lift]}),
        );
        let solid = body(&mut s, "Drilled");
        assert_close(
            &solid["volume"],
            (w * h - holes) * 3.0 * t,
            1e-9,
            "thicker plate",
        );
        // What "all" is: the extrusion ends past the plate's far side, by
        // a hundredth of the way, and not at the top of the mast.
        let document = run(&mut s, "document.inspect", json!({}));
        let sweep = document["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["params"]["type"] == "extrude")
            .unwrap()
            .clone();
        assert_eq!(sweep["params"]["end"]["type"], "throughAll", "{sweep}");
        // Kept apart from the plate, the extrusion can be measured.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Drilled", "keepTool": true}),
        );
        let columns = body(&mut s, sweep["name"].as_str().unwrap());
        let far = 2.0 * lift + 3.0 * t;
        let (low, high) = z_range(&columns);
        assert!(low.abs() < 1e-5 * k, "{columns}");
        assert!((high - far * 1.01).abs() < 1e-5 * k, "{high} against {far}");
        assert_close(&columns["volume"], holes * far * 1.01, 1e-9, "columns");
    }
}

#[test]
fn through_all_in_both_directions_and_against_the_normal() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    built(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 4], "position": [-10, -10, 6], "name": "Upper"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 8, "height": 3, "position": [0, 0, -9], "name": "Lower"}),
    );
    sketch(&mut s, "Pin", "xy", 0.0);
    circle(&mut s, "Pin", [0.0, 0.0], 1.0);
    // Along the normal: through the upper block only.
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Pin", "throughAll": true, "name": "Rod"}),
    );
    let (low, high) = z_range(&body(&mut s, "Rod"));
    assert!(
        low.abs() < 1e-5 && (high - 10.1).abs() < 1e-5,
        "{low} {high}"
    );
    // Against it: through the lower cylinder, whose far side is at -9.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Rod", "reversed": true}),
    );
    let rod = body(&mut s, "Rod");
    let (low, high) = z_range(&rod);
    assert!(
        (low + 9.09).abs() < 1e-5 && high.abs() < 1e-5,
        "{low} {high}"
    );
    // The start cap is the one in the sketch plane, whichever way.
    assert_eq!(
        face_names(&rod),
        ["F4:endCap(e2)", "F4:side(e2)", "F4:startCap(e2)"]
    );
    // Both ways: from below the cylinder to above the block.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Rod", "symmetric": true}),
    );
    let rod = body(&mut s, "Rod");
    let (low, high) = z_range(&rod);
    assert!((low + 9.09).abs() < 1e-5 && (high - 10.1).abs() < 1e-5);
    assert_close(&rod["volume"], PI * 19.19, 1e-9, "both ways");
    assert_sound(&mut s, "through all both ways");

    // Nothing in the way: an error that says so, and the rest is built.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Rod", "symmetric": false, "reversed": false}),
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Upper", "position": [40, 40, 6]}),
    );
    let document = run(&mut s, "document.inspect", json!({}));
    let rod = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Rod")
        .unwrap();
    assert_eq!(rod["status"], "failed", "{rod}");
    let reason = rod["error"].as_str().unwrap();
    assert!(
        reason.contains("there is nothing to go through along the sketch's normal")
            && reason.contains("none of the 2 bodies"),
        "{reason}"
    );
    assert_eq!(document["bodies"].as_array().unwrap().len(), 2);

    // In an empty document there is nothing at all.
    let mut s = session();
    new_document(&mut s, k);
    sketch(&mut s, "Pin", "xy", 0.0);
    circle(&mut s, "Pin", [0.0, 0.0], 1.0);
    let reason = failed(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Pin", "throughAll": true}),
    );
    assert!(reason.contains("none of the 0 bodies"), "{reason}");
}

/// A round profile up to an inclined plane: a cylinder of the mean
/// height, with an ellipse for a lid. The plane is the floor of a tilted
/// cylinder, which passes through that cylinder's position.
#[test]
fn up_to_an_inclined_plane_and_it_follows_the_plane() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        let (a, h) = (2.0 * k, 7.0 * k);
        let tilt = 0.4_f64;
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": 20.0 * k, "height": 3.0 * k, "position": [0, 0, h],
                   "axis": [tilt.sin(), 0, tilt.cos()], "name": "Roof"}),
        );
        sketch(&mut s, "Round", "xy", 0.0);
        circle(&mut s, "Round", [0.0, 0.0], a);
        // The floor of a cylinder is its face 1.
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Round", "upTo": "F1:face(1)", "name": "Post"}),
        );
        let closed = |h: f64| {
            (
                PI * a * a * h,
                PI * a * a + 2.0 * PI * a * h + PI * a * a / tilt.cos(),
            )
        };
        let (volume, area) = closed(h);
        let post = body(&mut s, "Post");
        assert_measures(&post, volume, area, 1e-9, "inclined plane");
        assert_eq!(
            face_names(&post),
            ["F3:endCap(e2)", "F3:side(e2)", "F3:startCap(e2)"]
        );
        assert_sound(&mut s, "inclined plane");

        // The roof is lifted: the post grows with it.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Roof", "position": [0, 0, 1.5 * h]}),
        );
        let (volume, area) = closed(1.5 * h);
        let post = body(&mut s, "Post");
        assert_measures(&post, volume, area, 1e-9, "lifted roof");
        // A sketch on the slanted end lies in the roof's plane.
        let on_end = run(
            &mut s,
            "sketch.create",
            json!({"face": "F3:endCap(e2)", "name": "OnEnd"}),
        );
        assert_eq!(on_end["status"], "ok", "{on_end}");
    }
}

/// A rectangle up to a lying cylinder, a round profile up to a ball from
/// outside and from inside: the closed forms of `crates/kernel/tests/
/// extrude_to.rs`, through the commands.
#[test]
fn up_to_a_cylinder_and_a_sphere() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        let (a, l, r, h) = (1.5 * k, 4.0 * k, 2.5 * k, 6.0 * k);
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": r, "height": 10.0 * k, "position": [0, -3.0 * k, h],
                   "axis": [0, 1, 0], "name": "Pipe"}),
        );
        sketch(&mut s, "Foot", "xy", 0.0);
        rectangle(&mut s, "Foot", (-a, a), (0.0, l));
        // The wall of a cylinder is its face 0.
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Foot", "upTo": "F1:face(0)", "name": "Saddle"}),
        );
        let drop = (r * r - a * a).sqrt();
        let front = 2.0 * a * h - a * drop - r * r * (a / r).asin();
        let saddle = body(&mut s, "Saddle");
        assert_measures(
            &saddle,
            l * front,
            2.0 * a * l + 2.0 * front + 2.0 * l * (h - drop) + 2.0 * r * (a / r).asin() * l,
            1e-9,
            "up to a cylinder",
        );
        assert_eq!(
            face_names(&saddle),
            [
                "F3:endCap(e5,e6,e7,e8)",
                "F3:side(e5)",
                "F3:side(e6)",
                "F3:side(e7)",
                "F3:side(e8)",
                "F3:startCap(e5,e6,e7,e8)"
            ]
        );

        // A ball beside it; from a plane below it, and from one inside it.
        let (b, rr, cx) = (1.5 * k, 4.0 * k, 30.0 * k);
        built(
            &mut s,
            "feature.sphere",
            json!({"radius": rr, "position": [cx, 0, 9.0 * k], "name": "Ball"}),
        );
        let fall = (rr * rr - b * b).sqrt();
        let cap_volume = 2.0 * PI / 3.0 * (rr.powi(3) - fall.powi(3));
        let cap_area = 2.0 * PI * rr * (rr - fall);
        for (name, offset, inside) in [("Below", 0.0, false), ("Within", 8.0 * k, true)] {
            sketch(&mut s, name, "xy", offset);
            circle(&mut s, name, [cx, 0.0], b);
            let peg = format!("{name}Peg");
            built(
                &mut s,
                "feature.extrude",
                json!({"sketch": name, "upTo": "F4:face(0)", "name": peg}),
            );
            let h = 9.0 * k - offset;
            let (volume, height) = if inside {
                (PI * b * b * h + cap_volume, h + fall)
            } else {
                (PI * b * b * h - cap_volume, h - fall)
            };
            assert_measures(
                &body(&mut s, &peg),
                volume,
                PI * b * b + 2.0 * PI * b * height + cap_area,
                1e-9,
                name,
            );
        }
        assert_sound(&mut s, "cylinder and sphere");
    }
}

/// A rectangle up to a freeform face: the underside of a block whose
/// underside was drawn as a spline (a parabola, as a B-spline of degree 2
/// whose control points lie evenly in x). The volume is the rectangle's
/// height times the area under the parabola, the end its arc length times
/// that height.
#[test]
fn up_to_a_freeform_face() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        // y(x) = 4 + 8 t (1 - t), t = (x + 6) / 12, times k.
        sketch(&mut s, "Wave", "xy", 0.0);
        let spline = run(
            &mut s,
            "sketch.spline",
            json!({"sketch": "Wave", "degree": 2,
                   "control": [[-6.0 * k, 4.0 * k], [0.0, 8.0 * k], [6.0 * k, 4.0 * k]]}),
        );
        let (curve, start, end) = (
            spline["entity"].as_str().unwrap().to_string(),
            spline["start"].clone(),
            spline["end"].clone(),
        );
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Wave",
                   "points": [end, [6.0 * k, 12.0 * k], [-6.0 * k, 12.0 * k], start]}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Wave", "distance": 5.0 * k, "name": "Arch"}),
        );
        let wall = format!("F2:side({curve})");
        assert!(face_names(&body(&mut s, "Arch")).contains(&wall));
        // On the zx plane sketch x is the world's z and sketch y its x;
        // the normal is the world's y.
        let (x0, x1, z0, z1) = (-2.0, 3.0, 1.0, 4.0);
        sketch(&mut s, "Bar", "zx", 0.0);
        rectangle(&mut s, "Bar", (z0 * k, z1 * k), (x0 * k, x1 * k));
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Bar", "upTo": wall, "name": "Prop"}),
        );
        let t = |x: f64| (x + 6.0) / 12.0;
        let y = |x: f64| 4.0 + 8.0 * t(x) * (1.0 - t(x));
        let primitive =
            |x: f64| 12.0 * (4.0 * t(x) + 4.0 * t(x).powi(2) - 8.0 / 3.0 * t(x).powi(3));
        let under = primitive(x1) - primitive(x0);
        let slope = |x: f64| (2.0 - 4.0 * t(x)) / 3.0;
        let arc = |s: f64| (s * (1.0 + s * s).sqrt() + s.asinh()) / 2.0;
        let length = 9.0 * (arc(slope(x0)) - arc(slope(x1)));
        let height = z1 - z0;
        let prop = body(&mut s, "Prop");
        assert_measures(
            &prop,
            height * under * k.powi(3),
            ((x1 - x0) * height + height * length + 2.0 * under + height * (y(x0) + y(x1))) * k * k,
            1e-7,
            "up to a freeform face",
        );
        assert_eq!(face_names(&prop).len(), 6);
        assert_sound(&mut s, "freeform");

        // The arch is redrawn higher: the prop follows it.
        built(
            &mut s,
            "sketch.edit",
            json!({"sketch": "Wave", "entity": "e2", "at": [0.0, 10.0 * k]}),
        );
        // Now y(x) = 4 + 12 t (1 - t): the area under it grows by half the
        // bulge's.
        let primitive = |x: f64| 12.0 * (4.0 * t(x) + 6.0 * t(x).powi(2) - 4.0 * t(x).powi(3));
        let under = primitive(x1) - primitive(x0);
        assert_close(
            &body(&mut s, "Prop")["volume"],
            height * under * k.powi(3),
            1e-7,
            "redrawn arch",
        );
    }
}

/// The face an extrusion ends on is a reference by name: it survives what
/// does not touch the face, moves with it, and when the face is gone the
/// extrusion fails with the name it lost while everything else is built.
#[test]
fn a_lost_face_fails_the_extrusion_and_nothing_else() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    built(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 4], "position": [-10, -10, 10], "name": "Slab"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 5, "height": 5, "position": [0, 0, 6], "name": "Boss"}),
    );
    // The boss hangs out of the slab's underside: its floor (face 1) is at
    // z = 6.
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "Slab", "tool": "Boss", "name": "Part"}),
    );
    let floor = "F3:copy(1)/F2:face(1)";
    assert!(face_names(&body(&mut s, "Part")).contains(&floor.to_string()));
    built(
        &mut s,
        "feature.sphere",
        json!({"radius": 2, "position": [40, 0, 0], "name": "Bystander"}),
    );
    sketch(&mut s, "Round", "xy", 0.0);
    circle(&mut s, "Round", [1.0, 0.0], 2.0);
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Round", "upTo": floor, "name": "Post"}),
    );
    assert_close(
        &body(&mut s, "Post")["volume"],
        PI * 4.0 * 6.0,
        1e-9,
        "post",
    );
    // After the post, a feature that does not depend on it.
    built(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "position": [-40, 0, 0], "name": "Later"}),
    );

    // The boss is made longer downwards: its floor moves, the post follows.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Boss", "position": [0, 0, 3], "height": 8}),
    );
    assert_close(
        &body(&mut s, "Post")["volume"],
        PI * 4.0 * 3.0,
        1e-9,
        "moved",
    );

    // The boss is pulled up into the slab: its floor is no longer a face
    // of the part. The post fails and says which face it lost; it does
    // not land on the slab's underside.
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Boss", "position": [0, 0, 11], "height": 2}),
    );
    let document = run(&mut s, "document.inspect", json!({}));
    let feature = |name: &str| {
        document["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == name)
            .unwrap()
            .clone()
    };
    let post = feature("Post");
    assert_eq!(post["status"], "failed", "{post}");
    let reason = post["error"].as_str().unwrap();
    assert!(
        reason.contains("upTo: `Part` no longer has a face F3:copy(1)/F2:face(1); its faces are: F3:copy(0)/F1:face(0)"),
        "{reason}"
    );
    for name in ["Part", "Bystander", "Later"] {
        assert_eq!(feature(name)["status"], "ok", "{name}");
    }
    let names: Vec<&str> = document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Part", "Bystander", "Later"]);

    // Repaired by pointing it at a face that is there: the slab's
    // underside, as if it had been made that way.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Post", "upTo": "F3:copy(0)/F1:face(0)"}),
    );
    assert_close(
        &body(&mut s, "Post")["volume"],
        PI * 4.0 * 10.0,
        1e-9,
        "repaired",
    );

    // The union suppressed passes the slab on unchanged: its underside is
    // still there, as the copy the union would have made of it.
    run(&mut s, "feature.suppress", json!({"feature": "Part"}));
    assert_close(
        &body(&mut s, "Post")["volume"],
        PI * 4.0 * 10.0,
        1e-9,
        "through the suppressed union",
    );
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Part", "suppressed": false}),
    );
    // The feature the face comes from has no body (its target is
    // suppressed), is rolled back past, or is moved after the post: each
    // its own reason.
    run(&mut s, "feature.suppress", json!({"feature": "Slab"}));
    let reason = run(&mut s, "document.inspect", json!({}))["features"][5]["error"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        reason.contains("`Part`, which makes the face") && reason.contains("`Slab` is suppressed"),
        "{reason}"
    );
    run(
        &mut s,
        "feature.suppress",
        json!({"feature": "Slab", "suppressed": false}),
    );
    let moved = failed(
        &mut s,
        "feature.move",
        json!({"feature": "Post", "position": 0}),
    );
    assert!(
        moved.contains("comes after this feature in the history"),
        "{moved}"
    );
}

#[test]
fn a_face_of_a_later_feature_is_refused_with_the_reason() {
    let mut s = session();
    new_document(&mut s, 1.0);
    sketch(&mut s, "Round", "xy", 0.0);
    circle(&mut s, "Round", [0.0, 0.0], 2.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [20, 20, 4], "position": [-10, -10, 10], "name": "Slab"}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Round", "upTo": "F2:face(0)", "name": "Post"}),
    );
    // The slab is moved after the post.
    run(
        &mut s,
        "feature.move",
        json!({"feature": "Slab", "position": 2}),
    );
    let document = run(&mut s, "document.inspect", json!({}));
    let post = &document["features"][1];
    assert_eq!(
        (&post["name"], &post["status"]),
        (&json!("Post"), &json!("failed"))
    );
    let reason = post["error"].as_str().unwrap();
    assert!(
        reason.contains(
            "upTo: `Slab`, which makes the face F2:face(0), comes after this feature in the history"
        ),
        "{reason}"
    );
    assert_eq!(document["features"][2]["status"], "ok");
    // Deleted: the reference is lost, and says so.
    run(&mut s, "feature.delete", json!({"feature": "Slab"}));
    let document = run(&mut s, "document.inspect", json!({}));
    let reason = document["features"][1]["error"].as_str().unwrap();
    assert!(
        reason.contains("upTo: there is no feature F2 for the face F2:face(0) (was it deleted?)"),
        "{reason}"
    );
}

#[test]
fn bad_ends_are_errors() {
    let mut s = session();
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": [10, 10, 4], "position": [-5, -5, 10], "name": "Slab"}),
    );
    sketch(&mut s, "Round", "xy", 0.0);
    circle(&mut s, "Round", [0.0, 0.0], 2.0);
    // Refused outright: the parameters make no sense.
    for (params, expected) in [
        (
            json!({"sketch": "Round"}),
            "missing parameter `distance` (or `throughAll` or `upTo`",
        ),
        (
            json!({"sketch": "Round", "distance": 5, "throughAll": true}),
            "give one of `distance`, `throughAll` and `upTo`, not several",
        ),
        (
            json!({"sketch": "Round", "upTo": "F1:face(0)", "throughAll": true}),
            "not several",
        ),
        (
            json!({"sketch": "Round", "upTo": 5}),
            "`upTo` must be a string",
        ),
        (
            json!({"sketch": "Round", "upTo": "the top"}),
            "feature.extrude: `upTo`:",
        ),
        (
            json!({"sketch": "Round", "upTo": "F1:edge(0)"}),
            "`upTo` must name a face",
        ),
        (
            json!({"sketch": "Round", "throughAll": "yes"}),
            "`throughAll` must be true or false",
        ),
        (
            json!({"sketch": "Round", "throughAll": true, "reversed": 1}),
            "`reversed` must be true or false",
        ),
        (
            json!({"sketch": "Round", "upTo": "F1:face(0)", "symmetric": true}),
            "`symmetric` does not go with `upTo`",
        ),
        (
            json!({"sketch": "Round", "distance": [1]}),
            "`distance` must be an expression",
        ),
    ] {
        let message = refused(&mut s, "feature.extrude", params.clone());
        assert!(message.contains(expected), "{params}: {message}");
    }
    // Accepted, and failed at regeneration with the reason; the document
    // keeps the feature so that it can be repaired.
    for (params, expected) in [
        (
            json!({"sketch": "Round", "upTo": "F9:face(0)"}),
            "upTo: there is no feature F9 for the face F9:face(0)",
        ),
        (
            json!({"sketch": "Round", "upTo": "F1:face(77)"}),
            "upTo: `Slab` no longer has a face F1:face(77); its faces are: F1:face(0), F1:face(1)",
        ),
        (
            json!({"sketch": "Round", "upTo": "F2:side(e2)"}),
            "upTo: `Round` is a sketch, which has no named faces",
        ),
        // A side of the slab: parallel to the direction.
        (
            json!({"sketch": "Round", "upTo": "F1:face(2)"}),
            "the face is parallel to the direction of the extrusion",
        ),
        // Its underside, going down.
        (
            json!({"sketch": "Round", "upTo": "F1:face(0)", "reversed": true}),
            "the face lies behind the sketch plane",
        ),
        (json!({"sketch": "Round", "distance": "0 / 0"}), "distance"),
        (
            json!({"sketch": "Round", "distance": "0 mm"}),
            "distance must be a length other than zero",
        ),
        (
            json!({"sketch": "Round", "throughAll": true, "reversed": true}),
            "there is nothing to go through against the sketch's normal",
        ),
    ] {
        let reason = failed(&mut s, "feature.extrude", params.clone());
        assert!(reason.contains(expected), "{params}: {reason}");
        run(&mut s, "edit.undo", json!({}));
    }
    // A profile beside the slab misses its underside.
    sketch(&mut s, "Aside", "xy", 0.0);
    circle(&mut s, "Aside", [30.0, 0.0], 2.0);
    let reason = failed(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Aside", "upTo": "F1:face(0)"}),
    );
    assert!(reason.contains("the profile misses the face"), "{reason}");
    run(&mut s, "edit.undo", json!({}));

    // Editing: one end at a time, and only on an extrusion.
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Round", "distance": 3, "name": "Post"}),
    );
    for (params, expected) in [
        (
            json!({"feature": "Post", "distance": 5, "upTo": "F1:face(0)"}),
            "not several",
        ),
        (
            json!({"feature": "Slab", "throughAll": true}),
            "`throughAll` is not a parameter of a block",
        ),
        (
            json!({"feature": "Post", "upTo": "nope"}),
            "feature.edit: `upTo`:",
        ),
    ] {
        let message = refused(&mut s, "feature.edit", params.clone());
        assert!(message.contains(expected), "{params}: {message}");
    }
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Post", "symmetric": true}),
    );
    let message = refused(
        &mut s,
        "feature.edit",
        json!({"feature": "Post", "upTo": "F1:face(0)"}),
    );
    assert!(
        message.contains("`symmetric` does not go with `upTo`"),
        "{message}"
    );
    // From a distance to a face, to through all, and back to the distance.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Post", "upTo": "F1:face(0)", "symmetric": false}),
    );
    assert_close(
        &body(&mut s, "Post")["volume"],
        PI * 4.0 * 10.0,
        1e-9,
        "up to",
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Post", "throughAll": true}),
    );
    assert_close(
        &body(&mut s, "Post")["volume"],
        PI * 4.0 * 14.0 * 1.01,
        1e-9,
        "through",
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Post", "throughAll": false}),
    );
    assert_close(
        &body(&mut s, "Post")["volume"],
        PI * 4.0 * 3.0,
        1e-9,
        "distance",
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Post", "throughAll": true}),
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Post", "distance": 2}),
    );
    assert_close(
        &body(&mut s, "Post")["volume"],
        PI * 4.0 * 2.0,
        1e-9,
        "distance",
    );
}

/// Every end in one document, saved and reopened: the same bodies, bit
/// for bit in what the document reports of them.
#[test]
fn every_end_is_saved_and_reopened_as_the_same_bodies() {
    let dir = workdir("files");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 20, "height": 3, "position": [0, 0, 7],
               "axis": [0.3, 0, 1], "name": "Roof"}),
    );
    built(
        &mut s,
        "feature.sphere",
        json!({"radius": 4, "position": [30, 0, -9], "name": "Ball"}),
    );
    sketch(&mut s, "Round", "xy", 0.0);
    circle(&mut s, "Round", [0.0, 0.0], 2.0);
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Round", "upTo": "F1:face(1)", "name": "Post"}),
    );
    sketch(&mut s, "Peg", "xy", 0.0);
    circle(&mut s, "Peg", [30.0, 0.0], 1.5);
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Peg", "upTo": "F2:face(0)", "reversed": true, "name": "Down"}),
    );
    sketch(&mut s, "Slot", "xy", 0.0);
    rectangle(&mut s, "Slot", (5.0, 7.0), (-1.0, 1.0));
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Slot", "throughAll": true, "cut": "Roof", "name": "Slotted"}),
    );
    sketch(&mut s, "Wing", "yz", 50.0);
    rectangle(&mut s, "Wing", (0.0, 2.0), (0.0, 3.0));
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Wing", "distance": 6, "symmetric": true, "reversed": true, "name": "Fin"}),
    );
    assert_sound(&mut s, "all ends");
    let before = run(&mut s, "document.inspect", json!({}));
    run(&mut s, "file.save", json!({"path": "ends.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "ends.3dcraft"}));
    let after = run(&mut s, "document.inspect", json!({}));
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    assert_eq!(before["bodies"].as_array().unwrap().len(), 5);
    // The ends are in the file as they were given.
    let text =
        String::from_utf8_lossy(&std::fs::read(dir.join("ends.3dcraft")).unwrap()).to_string();
    for expected in ["\"upToFace\"", "\"throughAll\"", "\"reversed\": true"] {
        assert!(text.contains(expected), "{expected} is not in the file");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
