//! What lies on one side of a plane (`measure.below`, `measure.section`,
//! `measure.levelFor`, and the assertion measures `volumeBelow`,
//! `centroidBelow`, `sectionArea`, `sectionCentroid`, `sectionInertia`,
//! `levelFor`) against closed forms of a block: the volume and centroid
//! below a level, the rectangle's moments, the level for a volume, a
//! turned plane; and, as a worked example of what a Jig builds on them,
//! the metacentric height of a floating box (BM = I/V, GM = KB + BM − KG)
//! changing sign at its known limit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

fn run(session: &mut Session, id: &str, params: Value) -> Value {
    session
        .execute(id, &params)
        .unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn refused(session: &mut Session, id: &str, params: Value) -> String {
    match session.execute(id, &params) {
        Ok(value) => panic!("{id} {params} should fail, returned {value}"),
        Err(e) => {
            let message = e.message().to_string();
            assert!(
                message.starts_with(id),
                "the error names the command: {message}"
            );
            message
        }
    }
}

#[track_caller]
fn close(got: &Value, want: f64, what: &str) {
    let got = got
        .as_f64()
        .unwrap_or_else(|| panic!("{what}: not a number: {got}"));
    let allowed = 1e-7 * want.abs().max(1.0);
    assert!(
        (got - want).abs() <= allowed,
        "{what}: {got} is not {want} within {allowed}"
    );
}

/// A block `l × b × d` mm centred on the z axis, its bottom at z = 0.
fn block(l: f64, b: f64, d: f64) -> Session {
    let mut s = Session::new(Workspace::none());
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": [l, b, d], "position": [-l / 2.0, -b / 2.0, 0], "name": "Tank"}),
    );
    s
}

#[test]
fn a_block_below_a_level_matches_its_closed_forms() {
    let (l, b, d, t) = (4000.0, 1300.0, 1000.0, 300.0);
    let mut s = block(l, b, d);
    let below = run(
        &mut s,
        "measure.below",
        json!({"of": "Tank", "offset": "300 mm"}),
    );
    close(&below["volume"], l * b * t, "volume below");
    close(&below["centroid"][2], t / 2.0, "centroid height");
    close(&below["centroid"][0], 0.0, "centroid x");
    close(
        &below["surface"],
        l * b + 2.0 * (l + b) * t,
        "surface below",
    );
    let section = &below["section"];
    close(&section["area"], l * b, "section area");
    close(&section["ix"], l * b.powi(3) / 12.0, "ix = l b³/12");
    close(&section["iy"], b * l.powi(3) / 12.0, "iy = b l³/12");
    // Zero to the rounding of sums of the size of iy (1e14 mm^4).
    let ixy = section["ixy"].as_f64().unwrap();
    assert!(ixy.abs() <= 1e-12 * b * l.powi(3) / 12.0, "ixy {ixy}");
    close(&section["major"], b * l.powi(3) / 12.0, "major");
    close(&section["minor"], l * b.powi(3) / 12.0, "minor");
    assert_eq!(below["lengthUnit"], "mm");
    // measure.section gives the same section; a plane given by its normal
    // the other way round keeps the other side.
    let alone = run(&mut s, "measure.section", json!({"offset": 300}));
    assert_eq!(alone["area"], section["area"], "{alone}");
    let above = run(
        &mut s,
        "measure.below",
        json!({"normal": "-z", "offset": "-300 mm"}),
    );
    close(&above["volume"], l * b * (d - t), "volume above");
    // The level for a volume: the fill level of the block as a tank.
    let level = run(&mut s, "measure.levelFor", json!({"volume": l * b * t}));
    close(&level["offset"], t, "level for a volume");
    assert!(level["steps"].as_u64().unwrap() <= 8, "{level}");
    // A volume with a unit, as an expression of the variables.
    run(
        &mut s,
        "variable.set",
        json!({"name": "fill", "expr": "1.56 * (1 m)^3"}),
    );
    let level = run(&mut s, "measure.levelFor", json!({"volume": "fill"}));
    close(&level["offset"], t, "level for a volume in m³");
}

/// Worked example, as a Jig for a floating body would do it with the
/// general measures: a box of relative density s floats with s of its
/// depth under (levelFor of its volume times s), KB is the centroid's
/// height below that level, BM = ix / V, and GM = KB + BM − KG changes
/// sign at B/D = √(6 s (1 − s)) (Biran & López-Pulido, *Ship
/// Hydrostatics and Stability*).
#[test]
fn a_jig_gets_the_metacentric_height_from_the_general_measures() {
    let s_rel = 0.5_f64;
    let limit = (6.0 * s_rel * (1.0 - s_rel)).sqrt() * 1000.0;
    for (b, stable) in [(limit * 1.02, true), (limit * 0.98, false)] {
        let (l, d) = (4000.0, 1000.0);
        let mut s = block(l, b, d);
        let level = run(
            &mut s,
            "measure.levelFor",
            json!({"volume": l * b * d * s_rel}),
        );
        let t = level["offset"].as_f64().unwrap();
        close(&level["offset"], s_rel * d, "draft");
        let kb = level["centroid"][2].as_f64().unwrap();
        let bm = level["section"]["ix"].as_f64().unwrap() / level["volume"].as_f64().unwrap();
        close(&json!(bm), b * b / (12.0 * t), "BM = B²/(12 T)");
        let gm = kb + bm - d / 2.0;
        assert_eq!(gm > 0.0, stable, "B {b}: GM {gm}");
    }
}

/// An assertion on a section's moment fails on a block too narrow and
/// passes once it is widened; the level for a volume as an assertion.
#[test]
fn assertions_on_a_section_break_and_pass_again() {
    let mut s = block(4000.0, 1000.0, 1000.0);
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "stiff", "measure": "sectionInertia", "of": "Tank", "offset": 500, "about": "x", "min": "4 * (1 m)^4 / 12 * 1.2"}),
    );
    assert_eq!(added["status"], "failed", "{added}");
    assert_eq!(added["unit"], "mm^4", "{added}");
    let fill = run(
        &mut s,
        "assert.add",
        json!({"name": "fill", "measure": "levelFor", "volume": "2 * (1 m)^3", "eq": "500 mm", "tol": 1e-6}),
    );
    assert_eq!(fill["status"], "passed", "{fill}");
    let volume = run(
        &mut s,
        "assert.add",
        json!({"name": "half", "measure": "volumeBelow", "offset": "500 mm", "eq": "2 * (1 m)^3", "tol": 1}),
    );
    assert_eq!(volume["status"], "passed", "{volume}");
    let height = run(
        &mut s,
        "assert.add",
        json!({"name": "centroid", "measure": "centroidBelow", "offset": 500, "axis": "z", "eq": 250}),
    );
    assert_eq!(height["status"], "passed", "{height}");
    let area = run(
        &mut s,
        "assert.add",
        json!({"name": "area", "measure": "sectionArea", "offset": 500, "eq": "4 * (1 m)^2"}),
    );
    assert_eq!(area["status"], "passed", "{area}");
    let centre = run(
        &mut s,
        "assert.add",
        json!({"name": "centre", "measure": "sectionCentroid", "offset": 500, "axis": "z", "eq": 500}),
    );
    assert_eq!(centre["status"], "passed", "{centre}");
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Tank", "size": [4000, 1100, 1000], "position": [-2000, -550, 0]}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    let status = |name: &str| {
        listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .map(|a| a["status"].clone())
            .unwrap()
    };
    assert_eq!(status("stiff"), "passed", "{listed}");
    // The tank widened: the same volume fills it less high.
    assert_eq!(status("fill"), "failed", "{listed}");
}

/// The parameters are expressions of the variables, and renaming a
/// variable renames it in the assertion.
#[test]
fn expressions_follow_the_variables() {
    let mut s = block(4000.0, 1000.0, 1000.0);
    run(
        &mut s,
        "variable.set",
        json!({"name": "level", "expr": "250 mm"}),
    );
    let added = run(
        &mut s,
        "assert.add",
        json!({"name": "v", "measure": "volumeBelow", "offset": "level", "eq": "1 * (1 m)^3"}),
    );
    assert_eq!(added["status"], "passed", "{added}");
    run(
        &mut s,
        "variable.rename",
        json!({"name": "level", "to": "h"}),
    );
    let listed = run(&mut s, "assert.list", json!({}));
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(
        listed["assertions"][0]["measure"]["offset"], "h",
        "{listed}"
    );
}

#[test]
fn what_cannot_be_measured_is_refused() {
    let mut s = block(4000.0, 1000.0, 1000.0);
    for id in ["measure.below", "measure.section"] {
        assert!(refused(&mut s, id, json!({"normal": [0, 0, 0]})).contains("normal"));
        assert!(refused(&mut s, id, json!({"xAxis": "z"})).contains("square"));
        assert!(refused(&mut s, id, json!({"offset": "nope"})).contains("offset"));
        assert!(refused(&mut s, id, json!({"offset": "5 deg"})).contains("length"));
        assert!(refused(&mut s, id, json!({"bogus": 1})).contains("bogus"));
        assert!(refused(&mut s, id, json!({"of": "Nothing"})).contains("Nothing"));
    }
    let id = "measure.levelFor";
    assert!(refused(&mut s, id, json!({})).contains("volume"));
    assert!(refused(&mut s, id, json!({"volume": 0})).contains("outside"));
    assert!(refused(&mut s, id, json!({"volume": 5e9})).contains("outside"));
    assert!(refused(&mut s, id, json!({"volume": -1})).contains("outside"));
    assert!(refused(&mut s, id, json!({"volume": "5 mm"})).contains("volume"));
    let mut empty = Session::new(Workspace::none());
    run(&mut empty, "file.new", json!({"units": "mm"}));
    assert!(refused(&mut empty, "measure.below", json!({})).contains("no bodies"));
    // Nothing below the plane: no centroid, and an assertion on it says so.
    let none = run(&mut s, "measure.below", json!({"offset": -10}));
    assert_eq!(none["volume"], 0.0);
    assert!(none["centroid"].is_null(), "{none}");
    let wrong = run(
        &mut s,
        "assert.add",
        json!({"name": "w", "measure": "centroidBelow", "offset": -10, "axis": "z", "min": 0}),
    );
    assert_eq!(wrong["status"], "error", "{wrong}");
    let bad = run(
        &mut s,
        "assert.add",
        json!({"name": "b", "measure": "sectionInertia", "offset": 10, "about": "q", "min": 0}),
    );
    assert_eq!(bad["status"], "error", "{bad}");
}
