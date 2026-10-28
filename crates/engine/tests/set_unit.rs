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

#[test]
fn what_cannot_be_kept_is_refused_and_bad_params_are_errors() {
    let mut s = part("mm", 1.0);
    run(
        &mut s,
        "variable.measure",
        json!({"name": "v", "measure": "volume", "of": "Row"}),
    );
    let e = error(
        &mut s,
        "document.setUnit",
        json!({"units": "in", "keep": "sizes"}),
    );
    assert!(e.contains("outputs"), "{e}");
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
