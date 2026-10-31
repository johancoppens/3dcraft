//! Blends along chains of freeform edges through the commands: the
//! turbofan's core casing, cut open by a block, whose cut edges run from
//! the cone into the torus of the flange's root fillet and on along the
//! flange (built, checked deep, named, refilleted after an edit).

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
use serde_json::json;

fn deep_ok(session: &mut Session) {
    let check = run(session, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}

/// The core casing of the turbofan: the meridian revolved, the root of
/// its flange filleted, a quarter cut away by a block.
fn casing(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(
        s,
        "sketch.create",
        json!({"plane": "xy", "name": "CoreCaseSk"}),
    );
    run(
        s,
        "sketch.polyline",
        json!({"sketch": "CoreCaseSk", "closed": true, "points": [[47.4316, 76.0], [49.9316, 76.0], [43.3789, 159.0], [55.1513, 159.0], [55.1513, 162.0], [40.6421, 162.0]]}),
    );
    built(
        s,
        "feature.revolve",
        json!({"sketch": "CoreCaseSk", "axis": "y", "name": "CoreCaseRaw"}),
    );
    built(
        s,
        "feature.fillet",
        json!({"edges": [["F2:side(e8)", "F2:side(e9)"]], "radius": 0.6, "name": "CoreCase"}),
    );
    built(
        s,
        "feature.block",
        json!({"size": [200.0, 400.0, 200.0], "position": [0.0, -50.0, 0.0], "name": "CutBox"}),
    );
    built(
        s,
        "feature.boolean",
        json!({"op": "subtract", "target": "CoreCase", "tool": "CutBox", "name": "CoreCaseCut"}),
    );
}

/// Both cut faces' edges along the outside, from the cone through the
/// root fillet to the flange, rounded as two chains in one feature (each
/// given by its edge on the cone, `chain` taking the rest).
#[test]
fn the_cut_edges_of_the_core_casing() {
    let mut s = session();
    casing(&mut s);
    let before = body(&mut s, "CoreCaseCut")["volume"].as_f64().unwrap();
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [
            ["F5:copy(0)/F2:side(e8)", "F5:copy(1)/F4:face(4)"],
            ["F5:copy(0)/F2:side(e8)", "F5:copy(1)/F4:face(0)"]
        ], "radius": 0.5, "chain": true, "name": "CutRound"}),
    );
    let rounded = body(&mut s, "CutRound");
    assert_eq!(rounded["valid"], true);
    let names: Vec<String> = rounded["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap().to_string())
        .collect();
    let fillets: Vec<&String> = names
        .iter()
        .filter(|n| n.starts_with("F6:fillet("))
        .collect();
    // Three faces per chain: the cone's, the root fillet's, the flange's.
    assert_eq!(fillets.len(), 6, "{names:?}");
    let after = rounded["volume"].as_f64().unwrap();
    // Each chain is about 95 long and takes the corner of a square of the
    // radius less a quarter of the disc along it.
    let corner = 0.5 * 0.5 * (1.0 - std::f64::consts::FRAC_PI_4);
    let removed = before - after;
    assert!(
        removed > 2.0 * 90.0 * corner && removed < 2.0 * 100.0 * corner,
        "removed {removed}"
    );
    deep_ok(&mut s);
    // A smaller radius is a change like any other.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "CutRound", "radius": 0.3}),
    );
    let smaller = body(&mut s, "CutRound")["volume"].as_f64().unwrap();
    assert!(smaller > after && smaller < before, "{after} -> {smaller}");
    deep_ok(&mut s);
}

/// A blade lofted through three sections of two cubic splines each, its
/// tip face's two rims rounded in one fillet: they meet at the leading and
/// at the trailing edge, which stay sharp (the turbofan's blade tip, on a
/// blade whose nose does not curve more tightly than the radius).
#[test]
fn the_tip_of_a_lofted_blade() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    let suction = [[-1.0, 0.0], [-0.5, 0.3], [0.4, 0.3], [1.0, 0.0]];
    let pressure = [[0.4, 0.12], [-0.5, 0.12]];
    let mut names = Vec::new();
    for (i, (z, scale, angle)) in [(0.0, 1.0, 0.0), (1.5, 0.9, 0.15), (3.0, 0.8, 0.3_f64)]
        .iter()
        .enumerate()
    {
        let name = format!("Sec{i}");
        let at = |p: [f64; 2]| {
            let (sin, cos) = angle.psin_cos();
            let (x, y) = (p[0] * scale, p[1] * scale);
            json!([x * cos - y * sin, x * sin + y * cos])
        };
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": name, "offset": z}),
        );
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": name, "control": suction.iter().map(|p| at(*p)).collect::<Vec<_>>()}),
        );
        run(
            &mut s,
            "sketch.spline",
            json!({"sketch": name, "control": ["e4", at(pressure[0]), at(pressure[1]), "e1"]}),
        );
        names.push(name);
    }
    built(
        &mut s,
        "feature.loft",
        json!({"sections": names, "name": "Blade"}),
    );
    let blade = body(&mut s, "Blade");
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [
            ["F4:endCap(e5,e8)", "F4:side(e5,e5,e5)"],
            ["F4:endCap(e5,e8)", "F4:side(e8,e8,e8)"]
        ], "radius": 0.02, "name": "TipRound"}),
    );
    let rounded = body(&mut s, "TipRound");
    assert_eq!(rounded["valid"], true);
    let faces = rounded["faceNames"].to_string();
    assert!(
        faces.contains("F5:fillet(0)") && faces.contains("F5:fillet(1)"),
        "{faces}"
    );
    let (before, after) = (
        blade["volume"].as_f64().unwrap(),
        rounded["volume"].as_f64().unwrap(),
    );
    assert!(
        after < before && after > 0.99 * before,
        "{before} -> {after}"
    );
    deep_ok(&mut s);
}

/// The engine block's finding E6 in its first form: a vertical boss on a
/// horizontal one whose root into the sloped flare is filleted; the foot
/// of the vertical boss runs over the flare, the root fillet and the
/// horizontal boss and back, a closed chain whose base face changes four
/// times. Rounded with 1 mm it is built; with 3 mm its border would run
/// off the narrow strip of flare left above the root fillet, which is
/// refused with the reason.
#[test]
fn the_foot_of_a_boss_over_a_root_fillet() {
    let mut s = session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "yz", "name": "BlockSk"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "BlockSk", "closed": true, "points": [[-63.0, 220.0], [-63.0, 110.0], [-98.0, 45.0], [-98.0, -55.0], [-110.0, -55.0], [-110.0, -70.0], [110.0, -70.0], [110.0, -55.0], [98.0, -55.0], [98.0, 45.0], [63.0, 110.0], [63.0, 220.0]]}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "BlockSk", "distance": 400.0, "symmetric": true, "name": "BlockRaw"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 22.0, "height": 240.0, "axis": [1, 0, 0], "position": [-40.0, 78.0, 82.0], "name": "IsBossRaw"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "BlockRaw", "tool": "IsBossRaw", "name": "CastIs"}),
    );
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F2:side(e22)", "F3:face(0)"]], "radius": 4.0, "name": "CastIsRoot"}),
    );
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 17.0, "height": 50.0, "position": [-20.0, 82.0, 82.0], "name": "DistBossRaw"}),
    );
    built(
        &mut s,
        "feature.boolean",
        json!({"op": "union", "target": "CastIsRoot", "tool": "DistBossRaw", "name": "CastDist"}),
    );
    let before = body(&mut s, "CastDist")["volume"].as_f64().unwrap();
    let foot = json!([["F7:copy(0)/F2:side(e22)", "F7:copy(1)/F6:face(0)"]]);
    let refused = run(
        &mut s,
        "feature.fillet",
        json!({"edges": foot, "radius": 3.0, "chain": true, "name": "DistRoot"}),
    );
    assert_eq!(refused["status"], "failed", "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("runs off a face beside the edge"),
        "{refused}"
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "DistRoot", "radius": 1.0}),
    );
    let rounded = body(&mut s, "DistRoot");
    assert_eq!(rounded["valid"], true);
    let fillets = rounded["faceNames"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n.as_str().unwrap().starts_with("F8:fillet("))
        .count();
    assert_eq!(fillets, 5, "{rounded}");
    let after = rounded["volume"].as_f64().unwrap();
    // A concave fillet adds material.
    assert!(
        after > before && after < 1.001 * before,
        "{before} -> {after}"
    );
    deep_ok(&mut s);
}
