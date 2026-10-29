//! `document.setUnit`: a document's length unit changed after `file.new`,
//! keeping the numbers (a plain number is then of the new unit) or the
//! sizes (the model as large as it was), with the volumes it must have
//! afterwards at several scales and units.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::run;
use common::error;
use craft3d_engine::Session;
use serde_json::json;

/// Millimetres in one of `unit`.
fn mm(unit: &str) -> f64 {
    match unit {
        "um" => 1e-3,
        "mm" => 1.0,
        "cm" => 10.0,
        "m" => 1000.0,
        "in" => 25.4,
        "ft" => 304.8,
        _ => panic!("{unit}"),
    }
}

fn volume(s: &mut Session) -> f64 {
    run(s, "document.measure", json!({}))["volume"]
        .as_f64()
        .unwrap()
}

fn close(a: f64, b: f64, what: &str) {
    assert!(
        (a - b).abs() <= 1e-7 * a.abs().max(b.abs()),
        "{what}: {a} vs {b}"
    );
}

/// A part in `unit` whose numbers are `k` times those of a part of a few
/// tens of units: a block with a sketched pocket cut into it, a fillet, a
/// pattern of the result, a cylinder written in millimetres, and an
/// assertion on the pattern's volume.
fn part(unit: &str, k: f64) -> Session {
    let mut s = common::session();
    run(
        &mut s,
        "file.new",
        json!({"units": unit, "tolerance": 1e-6 * k}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "w", "expr": format!("{}", 40.0 * k)}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": ["w", 20.0 * k, 10.0 * k], "name": "Base"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "offset": 6.0 * k, "name": "Pocket"}),
    );
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "Pocket", "closed": true,
               "points": [[5.0 * k, 5.0 * k], [15.0 * k, 5.0 * k], [15.0 * k, 15.0 * k], [5.0 * k, 15.0 * k]]}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Pocket", "distance": 10.0 * k, "cut": "Base", "name": "Cut"}),
    );
    run(
        &mut s,
        "feature.fillet",
        json!({"edges": [["F1:face(2)", "F1:face(4)"]], "radius": "w / 20", "name": "Round"}),
    );
    run(
        &mut s,
        "feature.pattern",
        json!({"source": "Round", "count": 2, "step": [60.0 * k, 0, 0], "name": "Row"}),
    );
    // Written with its unit: the same size whatever the document's unit.
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": format!("{} mm", 2.0 * k * mm(unit)), "height": format!("{} mm", 5.0 * k * mm(unit)), "position": [0, -50.0 * k, 0], "name": "Pin"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "row", "measure": "volume", "of": "Row", "min": 1000.0 * k.powi(3)}),
    );
    s
}

#[test]
fn keeping_the_sizes_keeps_the_volumes_at_several_scales() {
    for (from, to) in [
        ("mm", "in"),
        ("in", "mm"),
        ("mm", "m"),
        ("m", "um"),
        ("cm", "ft"),
    ] {
        for k in [1e-2, 1.0, 1e2] {
            let mut s = part(from, k);
            let before = volume(&mut s);
            let changed = run(
                &mut s,
                "document.setUnit",
                json!({"units": to, "keep": "sizes"}),
            );
            assert_eq!(changed["units"], to, "{changed}");
            assert!(changed["rewritten"].as_u64().unwrap() > 0, "{changed}");
            let scale = mm(from) / mm(to);
            close(
                volume(&mut s),
                before * scale.powi(3),
                &format!("{from} to {to} at {k}"),
            );
            let check = run(&mut s, "document.check", json!({}));
            assert_eq!(check["ok"], true, "{check}");
            // One undo step.
            run(&mut s, "edit.undo", json!({}));
            close(volume(&mut s), before, "undone");
        }
    }
}

#[test]
fn keeping_the_numbers_rescales_what_has_no_unit() {
    let mut s = part("mm", 1.0);
    let before = volume(&mut s);
    let pin = std::f64::consts::PI * 4.0 * 5.0;
    run(
        &mut s,
        "document.setUnit",
        json!({"units": "cm", "keep": "numbers"}),
    );
    // The plain numbers are centimetres now; the pin, written in mm, is
    // as large as it was (in cm^3 a thousandth of its number).
    close(volume(&mut s), before - pin + pin / 1000.0, "numbers kept");
    let inspect = run(&mut s, "document.inspect", json!({"measures": false}));
    assert_eq!(inspect["units"]["length"], "cm", "{inspect}");
}

/// An output and the measure it is made of keep their sizes (owed until
/// 2026-10-28, when this refused); a value assertion whose expression
/// uses an output that has no value has bounds whose dimension is not
/// known, and a plain number in them is refused, naming the assertion;
/// bad params are errors.
#[test]
fn what_cannot_be_kept_is_refused_and_bad_params_are_errors() {
    let mut s = part("mm", 1.0);
    run(
        &mut s,
        "variable.measure",
        json!({"name": "v", "measure": "volume", "of": "Row"}),
    );
    let v = |s: &mut Session| {
        run(s, "output.list", json!({}))["outputs"][0]["value"]
            .as_f64()
            .unwrap()
    };
    let before = v(&mut s);
    run(
        &mut s,
        "document.setUnit",
        json!({"units": "in", "keep": "sizes"}),
    );
    close(v(&mut s), before / 25.4f64.powi(3), "the output in inches");
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "variable.output",
        json!({"name": "bad", "expr": "1 / 0"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "unknowable", "measure": "value", "expr": "bad", "max": 5}),
    );
    let e = error(
        &mut s,
        "document.setUnit",
        json!({"units": "in", "keep": "sizes"}),
    );
    assert!(
        e.contains("unknowable") && e.contains("dimension is not known"),
        "{e}"
    );
    // Written with its unit, it is no longer ambiguous: the change is
    // made, and the assertion, which cannot be measured, stays so.
    run(
        &mut s,
        "assert.edit",
        json!({"name": "unknowable", "max": "5 * (1 mm)^3"}),
    );
    let was = statuses(&mut s);
    run(
        &mut s,
        "document.setUnit",
        json!({"units": "in", "keep": "sizes"}),
    );
    assert_eq!(statuses(&mut s), was);
    run(&mut s, "edit.undo", json!({}));
    for bad in [
        json!({}),
        json!({"units": "in"}),
        json!({"units": "furlong", "keep": "sizes"}),
        json!({"units": "in", "keep": "both"}),
        json!({"units": 3, "keep": "sizes"}),
    ] {
        error(&mut s, "document.setUnit", bad);
    }
    let same = run(
        &mut s,
        "document.setUnit",
        json!({"units": "mm", "keep": "sizes"}),
    );
    assert_eq!(same["rewritten"], 0, "{same}");
}

/// The pairs of units the cases below are checked at, and the factor
/// from the first to the second.
const PAIRS: [(&str, &str); 2] = [("mm", "m"), ("in", "mm")];

fn statuses(s: &mut Session) -> Vec<(String, String)> {
    let listed = run(s, "assert.list", json!({}));
    listed["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["name"].as_str().unwrap().to_string(),
                a["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn output(s: &mut Session, name: &str) -> f64 {
    let listed = run(s, "output.list", json!({}));
    listed["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == name)
        .unwrap_or_else(|| panic!("no output {name}: {listed}"))["value"]
        .as_f64()
        .unwrap_or_else(|| panic!("{name}: {listed}"))
}

/// Outputs measured with numbers of their own (an offset, a plane's
/// point), an output computed from one, and value assertions with plain
/// bounds: a block of 20 x 10 x 6, whose volume is 1200, whose volume
/// below 2 is 400 and whose section is 200, in the new unit by the
/// factor's powers; every assertion as it was (one fails, and still does).
#[test]
fn outputs_and_value_assertions_keep_their_sizes() {
    for (from, to) in PAIRS {
        let f = mm(from) / mm(to);
        let mut s = common::session();
        run(
            &mut s,
            "file.new",
            json!({"units": from, "tolerance": 1e-6}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"size": [20, 10, 6], "name": "Base"}),
        );
        for (name, measure) in [
            ("v", json!({"measure": "volume", "of": "Base"})),
            (
                "below",
                json!({"measure": "volumeBelow", "of": "Base", "plane": "xy", "offset": 2}),
            ),
            (
                "section",
                json!({"measure": "sectionArea", "of": "Base", "plane": {"point": [0, 0, "3"], "normal": [0, 0, 1]}}),
            ),
        ] {
            let mut params = measure;
            params["name"] = json!(name);
            run(&mut s, "variable.measure", params);
        }
        run(
            &mut s,
            "variable.output",
            json!({"name": "half", "expr": "v / 2"}),
        );
        for params in [
            json!({"name": "vol", "measure": "value", "expr": "v", "min": 1100, "max": 1300}),
            json!({"name": "halfEq", "measure": "value", "expr": "half", "eq": 600}),
            json!({"name": "tooSmall", "measure": "value", "expr": "v", "max": 1000}),
            json!({"name": "belowEq", "measure": "volumeBelow", "of": "Base", "offset": "3", "eq": 600}),
            json!({"name": "sectionEq", "measure": "sectionArea", "of": "Base", "plane": {"point": [0, 0, 3], "normal": [0, 0, 1]}, "eq": 200}),
            json!({"name": "valueBelow", "measure": "value", "expr": "below / section", "eq": 2}),
            json!({"name": "symmetric", "measure": "symmetric", "of": "Base", "plane": {"point": [10, 0, 0], "normal": [1, 0, 0]}, "tol": 0.001}),
        ] {
            run(&mut s, "assert.add", params);
        }
        let before = statuses(&mut s);
        let failed: Vec<&str> = before
            .iter()
            .filter(|(_, status)| status != "passed")
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(failed, ["tooSmall"], "{before:?}");
        let changed = run(
            &mut s,
            "document.setUnit",
            json!({"units": to, "keep": "sizes"}),
        );
        assert!(changed["rewritten"].as_u64().unwrap() >= 10, "{changed}");
        assert_eq!(statuses(&mut s), before, "{from} to {to}");
        close(output(&mut s, "v"), 1200.0 * f.powi(3), "v");
        close(output(&mut s, "half"), 600.0 * f.powi(3), "half");
        close(output(&mut s, "below"), 400.0 * f.powi(3), "below");
        close(output(&mut s, "section"), 200.0 * f.powi(2), "section");
    }
}

/// Formula curves whose x(t) and y(t) are plain numbers: a parabola
/// segment `y = t^2 / 40` from -20 to 20 closed by its chord (two thirds
/// of its box, 4/3 a h) and an Archimedean spiral `r = 10 + 3 t` over one
/// turn closed along the x axis (`((a + 2 pi b)^3 - a^3) / (6 b)`), each
/// extruded by 5: as large in the new unit. One whose dimension changes
/// along t cannot be made.
#[test]
fn formula_curves_keep_their_sizes() {
    use std::f64::consts::PI;
    for (from, to) in PAIRS {
        let f = mm(from) / mm(to);
        let mut s = common::session();
        run(
            &mut s,
            "file.new",
            json!({"units": from, "tolerance": 1e-7}),
        );
        for (name, x, y, t) in [
            ("P", "t", "t^2 / 40", json!([-20, 20])),
            (
                "S",
                "(10 + 3 * t) * cos(t)",
                "(10 + 3 * t) * sin(t)",
                json!([0, "2 * pi"]),
            ),
        ] {
            run(&mut s, "sketch.create", json!({"name": name}));
            let curve = run(
                &mut s,
                "sketch.curve",
                json!({"sketch": name, "x": x, "y": y, "t": t}),
            );
            run(
                &mut s,
                "sketch.line",
                json!({"sketch": name, "from": curve["end"], "to": curve["start"]}),
            );
            run(
                &mut s,
                "feature.extrude",
                json!({"sketch": name, "distance": 5, "name": format!("{name}x")}),
            );
        }
        let parabola = 4.0 / 3.0 * 20.0 * 10.0 * 5.0;
        let spiral = ((10.0 + 2.0 * PI * 3.0).powi(3) - 1000.0) / 18.0 * 5.0;
        let near = |a: f64, b: f64, what: &str| {
            assert!((a - b).abs() <= 1e-5 * b, "{what}: {a} vs {b}");
        };
        near(volume(&mut s), parabola + spiral, "before");
        let changed = run(
            &mut s,
            "document.setUnit",
            json!({"units": to, "keep": "sizes"}),
        );
        assert!(changed["rewritten"].as_u64().unwrap() >= 4, "{changed}");
        near(volume(&mut s), (parabola + spiral) * f.powi(3), "after");
        let inspect = run(&mut s, "sketch.inspect", json!({"sketch": "P"}));
        assert!(
            inspect
                .to_string()
                .contains(&format!("(t^2 / 40) * 1 {from}")),
            "{inspect}"
        );
        assert_eq!(run(&mut s, "document.check", json!({}))["ok"], true);
    }
    // A formula that is a plain number on one side and a length on the
    // other cannot be made: `if` needs branches of one dimension, so a
    // curve's dimension never changes along t in a document the commands
    // made (setUnit still refuses one, for a file from elsewhere).
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"name": "Odd"}));
    let e = error(
        &mut s,
        "sketch.curve",
        json!({"sketch": "Odd", "x": "if(t < 0, t, t * 1 mm)", "y": "t^2 / 40 * 1 mm", "t": [-20, 20]}),
    );
    assert!(e.contains("same dimension"), "{e}");
}

/// Numbers in a named set's query and in assertions' subjects: faces of
/// at least 199 (the block's two of 200), the cylinder's side by its
/// radius 2, the bodies inside a box, the faces above 5.5 and the edges
/// from 19 to 21 long; every count as it was, and the block's volume in
/// the box by the factor cubed.
#[test]
fn numbers_in_sets_and_subjects_keep_their_sizes() {
    for (from, to) in PAIRS {
        let f = mm(from) / mm(to);
        let mut s = common::session();
        run(
            &mut s,
            "file.new",
            json!({"units": from, "tolerance": 1e-6}),
        );
        run(
            &mut s,
            "feature.block",
            json!({"size": [20, 10, 6], "name": "Base"}),
        );
        run(
            &mut s,
            "feature.cylinder",
            json!({"radius": 2, "height": 6, "position": [40, 0, 0], "name": "Pin"}),
        );
        run(
            &mut s,
            "set.define",
            json!({"name": "big", "kind": "face", "where": {"area": {"min": 199}}}),
        );
        for params in [
            json!({"name": "bigFaces", "measure": "count", "of": {"kind": "face", "where": {"set": "big"}}, "eq": 2}),
            json!({"name": "pinSide", "measure": "count", "of": {"kind": "face", "where": {"radius": 2}}, "eq": 1}),
            json!({"name": "inBox", "measure": "volume", "of": {"kind": "body", "where": {"inside": [[-1, -1, -1], [21, 11, "7"]]}}, "eq": 1200}),
            json!({"name": "tops", "measure": "count", "of": {"kind": "face", "where": {"above": {"z": 5.5}}}, "eq": 2}),
            json!({"name": "long", "measure": "count", "of": {"kind": "edge", "where": {"length": {"min": 19, "max": "21"}}}, "eq": 4}),
            json!({"name": "notSmall", "measure": "count", "of": {"kind": "face", "where": {"not": {"area": {"max": 13}}}}, "eq": 7}),
        ] {
            run(&mut s, "assert.add", params);
        }
        let before = statuses(&mut s);
        assert!(before.iter().all(|(_, st)| st == "passed"), "{before:?}");
        run(
            &mut s,
            "document.setUnit",
            json!({"units": to, "keep": "sizes"}),
        );
        assert_eq!(statuses(&mut s), before, "{from} to {to}");
        let listed = run(&mut s, "assert.list", json!({}));
        let in_box = listed["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "inBox")
            .unwrap()
            .clone();
        close(
            in_box["measured"].as_f64().unwrap(),
            1200.0 * f.powi(3),
            "inBox",
        );
        let sets = run(&mut s, "set.list", json!({}));
        assert!(
            sets.to_string().contains(&format!("(199) * (1 {from})^2")),
            "{sets}"
        );
        let held = run(
            &mut s,
            "select.query",
            json!({"kind": "face", "where": {"set": "big"}}),
        );
        assert_eq!(held["count"], 2, "{held}");
    }
}

/// A sketch's dimension and its stored points, kept as large.
#[test]
fn a_dimensioned_sketch_keeps_its_size() {
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "in"}));
    run(&mut s, "sketch.create", json!({"plane": "xy", "name": "S"}));
    // The radius, an expression, is held by a radius constraint.
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "S", "center": [1, 1], "radius": "0.75"}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "S", "distance": 2}),
    );
    let before = volume(&mut s);
    close(before, std::f64::consts::PI * 0.5625 * 2.0, "in inches");
    run(
        &mut s,
        "document.setUnit",
        json!({"units": "mm", "keep": "sizes"}),
    );
    close(volume(&mut s), before * 25.4f64.powi(3), "in millimetres");
}

/// With the history rolled back, the features after the marker are
/// converted and proven too: rolled forward, the part is as large as it
/// was.
#[test]
fn keeping_the_sizes_with_a_marker_set_converts_what_is_rolled_back() {
    let mut s = part("mm", 1.0);
    let whole = volume(&mut s);
    run(&mut s, "history.rollback", json!({"feature": "Base"}));
    run(
        &mut s,
        "document.setUnit",
        json!({"units": "in", "keep": "sizes"}),
    );
    run(&mut s, "history.rollback", json!({"feature": null}));
    close(volume(&mut s), whole / 25.4f64.powi(3), "rolled forward");
    assert_eq!(run(&mut s, "document.check", json!({}))["ok"], true);
}

/// Keeping the numbers from millimetres to metres: a dimensioned sketch
/// keeps its numbers and solves to the same shape (the same branch), now
/// in metres.
#[test]
fn keeping_the_numbers_from_mm_to_m_solves_the_same_sketch() {
    let mut s = common::session();
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(&mut s, "sketch.create", json!({"plane": "xy", "name": "S"}));
    run(
        &mut s,
        "sketch.polyline",
        json!({"sketch": "S", "closed": true, "points": [[0, 0], [30, 0], [30, 10], [0, 10]]}),
    );
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "S", "center": [15, 5], "radius": "3"}),
    );
    run(
        &mut s,
        "feature.extrude",
        json!({"sketch": "S", "distance": 2}),
    );
    let before = volume(&mut s);
    run(
        &mut s,
        "document.setUnit",
        json!({"units": "m", "keep": "numbers"}),
    );
    close(volume(&mut s), before, "the same numbers, in m^3");
    assert_eq!(run(&mut s, "document.check", json!({}))["ok"], true);
}

/// The floating measures' and the press-brake check's own lengths (a
/// centre of gravity, a displacement, the tools' points, their length and
/// clearance) keep their sizes; a mass, a density, a heel in degrees and
/// the work are not lengths and stay as written. A barge 2000 × 600 × 400
/// mm (GM 65 mm) and a U channel bendable with a punch 10 wide.
#[test]
fn floating_and_bendable_measures_keep_their_sizes() {
    let mut s = common::session();
    run(
        &mut s,
        "file.new",
        json!({"units": "mm", "tolerance": 1e-6}),
    );
    run(
        &mut s,
        "feature.block",
        json!({"size": [2000, 600, 400], "position": [-1000, -300, 0], "name": "Barge"}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "GM", "measure": "floatingGM", "of": "Barge", "volume": 3e8,
               "centerOfGravity": [0, 0, 180], "heel": 2, "eq": 65}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "GM by mass", "measure": "floatingGM", "of": "Barge", "mass": 300,
               "density": 1000, "centerOfGravity": [0, 0, 180], "min": 60}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"plane": "xy", "name": "Base"}),
    );
    let made = run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Base", "from": [0, 3000], "to": [40, 3030]}),
    );
    let lines = made["lines"].as_array().unwrap().clone();
    run(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "U", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.5,
               "flanges": [{"edge": lines[3], "length": "20 mm", "angle": "90 deg"},
                           {"edge": lines[1], "length": "20 mm", "angle": "90 deg"}]}),
    );
    run(
        &mut s,
        "assert.add",
        json!({"name": "bends", "measure": "bendable", "of": "U",
               "punch": [[-5, 0], [5, 0], [5, 100], [-5, 100]],
               "die": [[-15, -20], [15, -20], [15, 0], [-15, 0]],
               "length": 100, "clearance": 0.001, "maxWork": 20000, "is": true}),
    );
    let before = statuses(&mut s);
    assert!(
        before.iter().all(|(_, status)| status == "passed"),
        "{before:?}"
    );
    run(
        &mut s,
        "document.setUnit",
        json!({"units": "cm", "keep": "sizes"}),
    );
    assert_eq!(statuses(&mut s), before);
    let listed = run(&mut s, "assert.list", json!({}));
    let gm = &listed["assertions"][0];
    close(gm["measured"].as_f64().unwrap(), 6.5, "GM in cm");
    let stored = listed["assertions"].to_string();
    assert!(
        stored.contains("180 * 1 mm") || stored.contains("180 mm"),
        "{stored}"
    );
    assert!(stored.contains("\"mass\":300"), "{stored}");
    assert!(stored.contains("\"heel\":2"), "{stored}");
    assert!(stored.contains("\"maxWork\":20000"), "{stored}");
}
