//! Blends along freeform edges and next to cylinders through the commands
//! (the turbofan showcase's findings F4, F5 and F2): the cut edges of a
//! halved rod against the closed form of the section, a lofted fan blade's
//! leading and trailing edges and a conical casing's cut edge built,
//! checked deep, named and refilleted after an edit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::plain::{body, built, run, session};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

fn deep_ok(session: &mut Session) {
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

/// The cut edges of a rod halved by a block (finding F5): the fillet is a
/// cylinder; what it takes from the section is the corner between the
/// diameter, the rim and the ball, worked out from the ball's centre.
#[test]
fn the_cut_edges_of_a_halved_rod() {
    for k in [1e-3, 1.0, 1e6] {
        let mut s = session();
        run(
            &mut s,
            "file.new",
            json!({"units": "mm", "tolerance": 1e-6 * k}),
        );
        let (big, height, r): (f64, f64, f64) = (10.0, 20.0, 1.0);
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": big * k, "height": height * k, "name": "Rod"}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"size": [20.0 * k, 40.0 * k, 30.0 * k], "position": [0.0, -20.0 * k, -5.0 * k], "name": "Half"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Rod", "tool": "Half", "name": "HalfRod"}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F3:copy(1)/F2:face(4)", "F3:copy(0)/F1:face(0)"]], "radius": r * k, "name": "Round"}),
        );
        let mx = ((big - r) * (big - r) - r * r).sqrt();
        let psi = r.patan2(mx);
        let corner = big * big * psi / 2.0 - mx * r / 2.0 - r * r * (PI / 2.0 + psi) / 2.0;
        // The pair names both cut edges.
        let expected = (PI * big * big / 2.0 - 2.0 * corner) * height * k.ppowi(3);
        let volume = body(&mut s, "Round")["volume"].as_f64().unwrap();
        assert!(
            (volume - expected).abs() <= 1e-9 * expected,
            "{volume} is not {expected}"
        );
        deep_ok(&mut s);
    }
}

/// A section of the fan blade: two cubic splines from the leading edge to
/// the trailing edge and back.
fn section(
    s: &mut Session,
    name: &str,
    offset: f64,
    suction: [[f64; 2]; 4],
    pressure: [[f64; 2]; 2],
) {
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "name": name, "offset": offset}),
    );
    run(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": suction}),
    );
    run(
        s,
        "sketch.spline",
        json!({"sketch": name, "control": ["e4", pressure[0], pressure[1], "e1"]}),
    );
}

/// The fan blade of the turbofan (finding F2), four of its eight sections:
/// its leading and trailing edges, between the two lofted sides, rounded.
#[test]
fn the_leading_and_trailing_edges_of_a_lofted_blade() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    section(
        &mut s,
        "S0",
        23.0,
        [
            [-7.0421, 42.7558],
            [-8.9846, 43.7886],
            [-7.8518, 55.0783],
            [7.0421, 69.2442],
        ],
        [[-1.852, 55.2859], [-5.1711, 42.4405]],
    );
    section(
        &mut s,
        "S1",
        36.0,
        [
            [-10.5552, 41.1291],
            [-12.0966, 42.2231],
            [-8.3312, 55.2045],
            [10.5552, 70.8709],
        ],
        [[-2.577, 55.5928], [-8.8719, 40.8288]],
    );
    section(
        &mut s,
        "S2",
        57.0,
        [
            [-16.1471, 41.0825],
            [-17.0901, 42.1032],
            [-8.8186, 55.8268],
            [16.1471, 70.9175],
        ],
        [[-3.3603, 56.3977], [-14.7297, 40.844]],
    );
    section(
        &mut s,
        "S3",
        83.0,
        [
            [-22.9566, 43.7937],
            [-23.3181, 44.4736],
            [-9.3189, 56.9118],
            [22.9566, 68.2063],
        ],
        [[-3.8731, 57.7461], [-21.7426, 43.7257]],
    );
    built(
        &mut s,
        "feature.loft",
        json!({"sections": ["S0", "S1", "S2", "S3"], "name": "Blade"}),
    );
    let blade = body(&mut s, "Blade");
    let names: Vec<String> = blade["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    let sides: Vec<&String> = names.iter().filter(|n| n.contains("side(")).collect();
    assert_eq!(sides.len(), 2, "{names:?}");
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [[sides[0], sides[1]]], "radius": 0.3, "name": "Edges"}),
    );
    let rounded = body(&mut s, "Edges");
    assert_eq!(rounded["valid"], true);
    assert_eq!(rounded["faces"], 6);
    let faces = rounded["faceNames"].to_string();
    assert!(
        faces.contains(":fillet(0,0)") && faces.contains(":fillet(0,1)"),
        "{faces}"
    );
    let (before, after) = (
        blade["volume"].as_f64().unwrap(),
        rounded["volume"].as_f64().unwrap(),
    );
    assert!(
        after < before && after > 0.95 * before,
        "{before} -> {after}"
    );
    deep_ok(&mut s);
    // A larger radius is a change like any other.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Edges", "radius": 0.5}),
    );
    let larger = body(&mut s, "Edges")["volume"].as_f64().unwrap();
    assert!(larger < after, "{after} -> {larger}");
    deep_ok(&mut s);
}

/// A conical casing cut open along its axis: the cut edges lie between a
/// plane and a cone, and their fillet has no closed form.
#[test]
fn the_cut_edges_of_a_conical_casing() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cone",
        json!({"radius": 10.0, "height": 30.0, "name": "Cone"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [30.0, 15.0, 15.0], "position": [-15.0, 0.0, 0.0], "name": "Keep"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "intersect", "target": "Cone", "tool": "Keep", "name": "Half"}),
    );
    let half = body(&mut s, "Half");
    let edges = run(&mut s, "document.edges", json!({"feature": "Half"}));
    let cut = edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["curve"] == "line" && e["faces"].to_string().contains("F1:face(0)"))
        .unwrap_or_else(|| panic!("no straight edge on the cone in {edges}"))
        .clone();
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [cut["faces"]], "radius": 1.0, "name": "Round"}),
    );
    let after = body(&mut s, "Round")["volume"].as_f64().unwrap();
    let before = half["volume"].as_f64().unwrap();
    assert!(after < before, "{before} -> {after}");
    deep_ok(&mut s);
    run(&mut s, "feature.delete", json!({"feature": "Round"}));
    built(
        &mut s,
        "feature.chamfer",
        json!({"edges": [cut["faces"]], "distance": 0.5, "name": "Bevel"}),
    );
    let chamfered = body(&mut s, "Bevel")["volume"].as_f64().unwrap();
    assert!(chamfered < before, "{before} -> {chamfered}");
    deep_ok(&mut s);
}

/// A rod cut by a slanted plane and a plate with a slanted hole: the rim
/// and the mouth are whole ellipses, closed edges whose blends are rings
/// of spline surface, outside and inside.
#[test]
fn the_rim_of_a_slanted_cut_and_the_mouth_of_a_slanted_hole() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 5.0, "height": 20.0, "name": "Rod"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [40.0, 40.0, 20.0], "position": [-20.0, -20.0, 12.0], "name": "Cutter"}),
    );
    built(
        &mut s,
        "feature.transform",
        json!({"source": "Cutter", "angle": "15 deg", "axis": [0, 1, 0], "pivot": [0, 0, 12], "name": "Tilted"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Rod", "tool": "Tilted", "name": "Cut"}),
    );
    let before = body(&mut s, "Cut")["volume"].as_f64().unwrap();
    let edges = run(&mut s, "document.edges", json!({"feature": "Cut"}));
    let rim = edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["curve"] == "ellipse")
        .unwrap_or_else(|| panic!("no ellipse in {edges}"))
        .clone();
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [rim["faces"]], "radius": 1.0, "name": "Round"}),
    );
    let after = body(&mut s, "Round")["volume"].as_f64().unwrap();
    assert!(after < before, "{before} -> {after}");
    deep_ok(&mut s);
    run(&mut s, "feature.delete", json!({"feature": "Round"}));
    built(
        &mut s,
        "feature.chamfer",
        json!({"edges": [rim["faces"]], "distance": 0.8, "name": "Bevel"}),
    );
    deep_ok(&mut s);

    // The mouth of a slanted hole in a plate gains nothing and loses the
    // rounded rim.
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    built(
        &mut s,
        "feature.block",
        json!({"size": [30.0, 30.0, 6.0], "name": "Plate"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"name": "Drill", "position": [15.0, 15.0, -3.0], "axis": [0.3, 0.0, 1.0], "radius": 4.0, "height": 15.0}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Plate", "tool": "Drill", "name": "Drilled"}),
    );
    let before = body(&mut s, "Drilled")["volume"].as_f64().unwrap();
    let edges = run(&mut s, "document.edges", json!({"feature": "Drilled"}));
    let mouths: Vec<Value> = edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["curve"] == "ellipse")
        .map(|e| e["faces"].clone())
        .collect();
    assert_eq!(mouths.len(), 2, "{edges}");
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": mouths, "radius": 1.0, "name": "Round"}),
    );
    let after = body(&mut s, "Round")["volume"].as_f64().unwrap();
    assert!(after < before, "{before} -> {after}");
    deep_ok(&mut s);
}
