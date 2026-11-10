//! Holes across a bend by command (docs/design/sheetmetal.md › *Holes
//! across a bend*): a slot drawn in a cut-out sketch across a flange's
//! bend, at three scales: the volume at K = ½ is the flat area times the
//! thickness, the flat pattern exports with the slot as a hole of the
//! outline, the bend is left in two faces with names of their own, and the
//! part saves and reopens to the same body and names.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{assert_close, body, built, failure, new_document, run, session_in, workdir};
use serde_json::json;
use std::f64::consts::FRAC_PI_2;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

#[test]
fn a_slot_across_a_bend_by_command() {
    for k in SCALES {
        let dir = workdir("sheetmetal-cuts", &format!("slot-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (t, r, l) = (1.0 * k, 2.0 * k, 15.0 * k);
        let ba = FRAC_PI_2 * (r + 0.5 * t);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Base"}),
        );
        let made = run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Base", "from": [0.0, 0.0], "to": [50.0 * k, 40.0 * k]}),
        );
        let right = made["lines"][1].as_str().unwrap().to_string();
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Slot"}),
        );
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Slot", "from": [46.0 * k, 17.0 * k], "to": [50.0 * k + ba + 5.0 * k, 23.0 * k]}),
        );
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "Bracket", "thickness": format!("{t} mm"),
                   "radius": format!("{r} mm"), "kFactor": 0.5, "cutouts": ["Slot"],
                   "flanges": [{"edge": right, "length": format!("{l} mm"), "angle": "90 deg"}]}),
        );
        let area = 50.0 * 40.0 * k * k + 40.0 * k * (ba + l) - 6.0 * k * (9.0 * k + ba);
        let before = body(&mut s, "Bracket");
        assert_close(&before["volume"], area * t, "volume with the slot");
        let names = before["faceNames"].as_array().unwrap();
        assert!(
            names
                .iter()
                .any(|n| n.as_str().unwrap().contains("face(4294967297)")),
            "the bend's second face has a name of its own: {names:?}"
        );
        let out = run(
            &mut s,
            "file.export",
            json!({"path": "bracket.svg", "sheetMetal": "Bracket"}),
        );
        assert_close(&out["flatArea"], area, "flat area");
        run(&mut s, "file.save", json!({"path": "bracket.3dcraft"}));
        // The slot runs through the bend: the bend table lists it under
        // holesAcross with the bend it crosses (not under holes, which are
        // on one flat), and holeToBend, which measures holes beside a
        // bend, says why it has no value instead of "no holes".
        let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Bracket"}));
        assert_eq!(table["holes"], json!([]), "{table}");
        let across = table["holesAcross"].as_array().unwrap();
        assert_eq!(across.len(), 1, "{table}");
        assert_eq!(across[0]["bends"], json!([0]), "{table}");
        assert!(
            across[0]["hole"].as_str().unwrap().contains("Slot"),
            "{table}"
        );
        let a = run(
            &mut s,
            "assert.add",
            json!({"name": "hole to bend", "measure": "holeToBend", "of": "Bracket", "min": "1 mm"}),
        );
        assert_eq!(a["status"], "error", "{a}");
        let why = a["message"].as_str().unwrap();
        assert!(!why.contains("no holes,"), "{why}");
        assert!(why.contains("`Bracket` has no hole on a flat"), "{why}");
        assert!(why.contains("runs through the bend"), "{why}");
        assert!(why.contains("holesAcross"), "{why}");
        run(&mut s, "edit.undo", json!({}));
        let mut u = session_in(&dir);
        run(&mut u, "file.open", json!({"path": "bracket.3dcraft"}));
        let again = body(&mut u, "Bracket");
        assert_eq!(before["volume"], again["volume"]);
        assert_eq!(before["faceNames"], again["faceNames"]);
    }
}

/// A 50 × 40 base with a flange 15 long on its right edge, bent up 90
/// degrees about an inner radius 2 with the thickness 1 (times `k`) and
/// the K-factor 0.33, its holes from the sketches `cutouts`.
fn bracket(s: &mut craft3d_engine::Session, k: f64, name: &str, sketch: &str, cutouts: &[&str]) {
    run(s, "sketch.create", json!({"plane": "xy", "name": sketch}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": sketch, "from": [0.0, 0.0], "to": [50.0 * k, 40.0 * k]}),
    );
    let right = made["lines"][1].as_str().unwrap().to_string();
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": sketch, "name": name, "thickness": format!("{k} mm"),
               "radius": format!("{} mm", 2.0 * k), "kFactor": 0.33, "cutouts": cutouts,
               "flanges": [{"edge": right, "length": format!("{} mm", 15.0 * k), "angle": "90 deg"}]}),
    );
}

/// A later boolean's cuts carried back onto the flat pattern: a round hole
/// drilled square through the base and a slot cut by a block through the
/// bend (its walls square to the bend's axis, so radial through it) and 5
/// up the flange are holes of the exported pattern, with the flat area
/// less by their closed forms; the part drawn with the same holes in a
/// cut-out sketch has the drilled part's volume (the cut carried back is
/// the cut folded). A cut through the base at a slant is refused naming
/// why. At three scales.
#[test]
fn a_later_cut_is_carried_back_onto_the_flat_pattern() {
    for k in SCALES {
        let dir = workdir("sheetmetal-cuts", &format!("later-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let ba = FRAC_PI_2 * (2.0 + 0.33) * k;
        bracket(&mut s, k, "P", "Base", &[]);
        let mm = |v: f64| format!("{} mm", v * k);
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": "Drill", "radius": mm(2.0), "height": mm(3.0),
                   "position": [mm(20.0), mm(20.0), mm(-1.0)]}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "P", "tool": "Drill", "name": "Drilled"}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"name": "Cutter", "size": [mm(14.0), mm(6.0), mm(9.0)],
                   "position": [mm(46.0), mm(17.0), mm(-1.0)]}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "Drilled", "tool": "Cutter", "name": "Slotted"}),
        );
        let out = run(
            &mut s,
            "file.export",
            json!({"path": "p.dxf", "sheetMetal": "P"}),
        );
        assert_eq!(out["laterCuts"], 2, "{out}");
        assert_eq!(out["notCarried"], json!([]), "{out}");
        let area0 = 50.0 * 40.0 * k * k + 40.0 * k * (ba + 15.0 * k);
        let holes = std::f64::consts::PI * 4.0 * k * k + 6.0 * k * (4.0 * k + ba + 5.0 * k);
        assert_close(
            &out["flatArea"],
            area0 - holes,
            "flat area with the later cuts",
        );
        let dxf = std::fs::read_to_string(dir.join("p.dxf")).unwrap();
        assert!(
            dxf.contains("CIRCLE") || dxf.contains("ARC"),
            "the drilled hole is drawn by its circle"
        );
        // The same holes drawn flat and folded: the same solid.
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Holes"}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Holes", "center": [20.0 * k, 20.0 * k], "radius": 2.0 * k}),
        );
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Holes", "from": [46.0 * k, 17.0 * k], "to": [50.0 * k + ba + 5.0 * k, 23.0 * k]}),
        );
        bracket(&mut s, k, "Q", "Base2", &["Holes"]);
        let carried = body(&mut s, "Slotted");
        let folded = body(&mut s, "Q");
        let (a, b) = (
            carried["volume"].as_f64().unwrap(),
            folded["volume"].as_f64().unwrap(),
        );
        assert!(
            (a - b).abs() <= 1e-7 * b,
            "the cut carried back is the cut folded: {a} against {b}"
        );

        // A slanted drill: refused.
        let dir = workdir("sheetmetal-cuts", &format!("slant-{k}"));
        let mut u = session_in(&dir);
        new_document(&mut u, k);
        bracket(&mut u, k, "P", "Base", &[]);
        run(
            &mut u,
            "feature.cylinder",
            json!({"name": "Drill", "radius": mm(2.0), "height": mm(6.0), "axis": [1, 0, 1],
                   "position": [mm(18.0), mm(20.0), mm(-2.0)]}),
        );
        built(
            &mut u,
            "feature.boolean",
            json!({"op": "subtract", "target": "P", "tool": "Drill", "name": "Drilled"}),
        );
        let why = common::error(
            &mut u,
            "file.export",
            json!({"path": "p.dxf", "sheetMetal": "P"}),
        );
        assert!(why.contains("square"), "{why}");
    }
}

/// Review of 18851368: (1) a part whose bend a slot leaves in two faces
/// (they share the bend's surface) drilled straight through the bend: the
/// drill was silently left out of the export; it is a cut that is not
/// square to the sheet (its outline on the two skins differs) and refuses
/// the export. (2) Two drilled holes whose boxes overlap: each found its
/// own partner, not the first box it met. (3) The bend's second face keeps
/// its name when a flange is added. At three scales.
#[test]
fn review_findings_on_later_cuts_and_names() {
    for k in SCALES {
        let mm = |v: f64| format!("{} mm", v * k);
        let ba = FRAC_PI_2 * (2.0 + 0.33);
        // (1)
        let dir = workdir("sheetmetal-cuts", &format!("review1-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Slot"}),
        );
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Slot", "from": [46.0 * k, 17.0 * k], "to": [(50.0 + ba + 5.0) * k, 23.0 * k]}),
        );
        bracket(&mut s, k, "P", "Base", &["Slot"]);
        let c = std::f64::consts::FRAC_1_SQRT_2;
        run(
            &mut s,
            "feature.cylinder",
            json!({"name": "Drill", "radius": mm(0.5), "height": mm(2.0), "axis": [-c, 0, c],
                   "position": [mm(50.0 + 3.6 * c), mm(30.0), mm(3.0 - 3.6 * c)]}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "subtract", "target": "P", "tool": "Drill", "name": "Drilled"}),
        );
        let why = common::error(
            &mut s,
            "file.export",
            json!({"path": "p.dxf", "sheetMetal": "P"}),
        );
        assert!(why.contains("square"), "{why}");
        // (2)
        let dir = workdir("sheetmetal-cuts", &format!("review2-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        bracket(&mut s, k, "P", "Base", &[]);
        let mut target = "P".to_string();
        for (i, (x, y)) in [(20.0, 20.0), (23.5, 23.5)].into_iter().enumerate() {
            run(
                &mut s,
                "feature.cylinder",
                json!({"name": format!("D{i}"), "radius": mm(2.0), "height": mm(3.0),
                       "position": [mm(x), mm(y), mm(-1.0)]}),
            );
            built(
                &mut s,
                "feature.boolean",
                json!({"op": "subtract", "target": target, "tool": format!("D{i}"), "name": format!("B{i}")}),
            );
            target = format!("B{i}");
        }
        let out = run(
            &mut s,
            "file.export",
            json!({"path": "p.dxf", "sheetMetal": "P"}),
        );
        assert_eq!(out["laterCuts"], 2, "{out}");
        // (3)
        let dir = workdir("sheetmetal-cuts", &format!("review3-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Slot"}),
        );
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Slot", "from": [46.0 * k, 17.0 * k], "to": [(50.0 + ba + 5.0) * k, 23.0 * k]}),
        );
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Base"}),
        );
        let made = run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Base", "from": [0.0, 0.0], "to": [50.0 * k, 40.0 * k]}),
        );
        let lines: Vec<String> = made["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().to_string())
            .collect();
        let flange = |edge: &str| json!({"edge": edge, "length": mm(15.0), "angle": "90 deg"});
        built(
            &mut s,
            "feature.sheetMetal",
            json!({"sketch": "Base", "name": "P", "thickness": mm(1.0), "radius": mm(2.0),
                   "kFactor": 0.33, "cutouts": ["Slot"], "flanges": [flange(&lines[1])]}),
        );
        let piece = |s: &mut craft3d_engine::Session| {
            body(s, "P")["faceNames"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|n| n.as_str())
                .filter(|n| n.starts_with("F3:copy(1)/") && n.contains("face(4294967297)"))
                .count()
        };
        assert_eq!(piece(&mut s), 1, "the bend's second face is named");
        run(
            &mut s,
            "sheetMetal.edit",
            json!({"feature": "P", "flanges": [flange(&lines[1]), flange(&lines[3])]}),
        );
        assert_eq!(piece(&mut s), 1, "and keeps its name with a flange added");
    }
}

/// A cut-out sketch drawn after the part it cuts (the usual order when an
/// agent adds holes to a part that is there) fails the part; the reason
/// gives the exact command that puts the sketch before the part, and
/// running it builds the part with its hole.
#[test]
fn a_cutout_sketch_after_its_part_says_how_to_move_it_before() {
    let dir = workdir("sheetmetal-cuts", "after");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Base"}),
    );
    run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0.0, 0.0], "to": [50.0, 40.0]}),
    );
    built(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Plate", "thickness": "1 mm", "radius": "2 mm",
               "kFactor": 0.5}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Holes"}),
    );
    run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Holes", "from": [10.0, 10.0], "to": [20.0, 20.0]}),
    );
    run(
        &mut s,
        "sheetMetal.edit",
        json!({"feature": "Plate", "cutouts": ["Holes"]}),
    );
    let reason = failure(&mut s, "Plate");
    assert_eq!(
        reason,
        "cutouts.0: sketch: `Holes` comes after this feature in the history; move it before: \
         feature.move {\"feature\": \"F3\", \"position\": 1} (or, to add a feature in its place, \
         history.rollback to `Base` first)",
    );
    run(
        &mut s,
        "feature.move",
        json!({"feature": "F3", "position": 1}),
    );
    assert_close(
        &body(&mut s, "Plate")["volume"],
        (50.0 * 40.0 - 10.0 * 10.0) * 1.0,
        "the plate with its hole",
    );
}
