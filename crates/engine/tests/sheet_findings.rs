//! What the sheet-metal enclosure and the rack Jigs found the plates
//! doing wrong (2026-10-27), each from the steps that made it:
//!
//! - A stadium slot whose end radius is about the sheet's thickness (r
//!   1.5 in a 1.5 mm sheet; r 2 in a 3 mm one) gave "face cannot be
//!   tessellated" or an invalid plate. The rims and the skins' edges of
//!   the slot's arcs are traced past the arcs' ends by twice the
//!   thickness, which is more than what is left of the circle, so the
//!   traced run went more than a turn, and its circle was taken the other
//!   way round from three of its points: the edges went round the wrong
//!   half. A circle now runs the way its points turn.
//! - The rim of a plate round a circular hole was a freeform face, not
//!   the hole's cylinder, so a `radius` query found nothing and the
//!   measures on it were slow; whether the plate could be measured
//!   depended on where it lay. A ruled rim between two circles about one
//!   axis is now that cylinder or cone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_engine::{Session, Workspace};
use serde_json::{Value, json};

fn run(steps: &Value) -> (Session, Vec<Value>) {
    let mut s = Session::new(Workspace::unrestricted());
    s.execute("file.new", &json!({"units": "mm"})).unwrap();
    let mut out = Vec::new();
    for step in steps.as_array().unwrap() {
        let (id, params) = (step[0].as_str().unwrap(), &step[1]);
        out.push(
            s.execute(id, params)
                .unwrap_or_else(|e| panic!("{id} {params}: {e}")),
        );
    }
    let document = s.execute("document.inspect", &json!({})).unwrap();
    for f in document["features"].as_array().unwrap() {
        assert_eq!(f["status"], "ok", "{f}");
    }
    (s, out)
}

/// The volume of the body `name`, as `document.inspect` measures it.
fn volume(s: &mut Session, name: &str) -> f64 {
    let document = s.execute("document.inspect", &json!({})).unwrap();
    let body = document["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no body {name}"))
        .clone();
    assert_eq!(body["valid"], true, "{body}");
    body["volume"].as_f64().unwrap_or_else(|| panic!("{body}"))
}

fn checked(s: &mut Session) {
    let check = s.execute("document.check", &json!({"deep": true})).unwrap();
    assert_eq!(check["ok"], true, "{check}");
}

/// The slot of the enclosure's lid (r 1.5 in a 1.5 mm sheet, a flange
/// along one edge), at three placements and three scales of the slot's
/// position: built, checked, and its volume `(A − (π r² + 2 r l)) t` plus
/// the flange's.
#[test]
fn a_slot_whose_end_radius_is_the_thickness_is_built() {
    for (dx, dy) in [(0.0, 0.0), (1000.0, -700.0), (-12345.6, 9876.5)] {
        let at = |x: f64, y: f64| json!([x + dx, y + dy]);
        let steps = json!([
            ["sketch.create", {"plane": "xy", "name": "S"}],
            ["sketch.rect", {"sketch": "S", "from": at(-80.0, -60.0), "to": at(80.0, 60.0)}],
            ["sketch.path", {"sketch": "S", "start": at(-60.0, -28.5), "segments": [
                {"line": at(-60.0, 28.5)},
                {"arc": at(-63.0, 28.5), "center": at(-61.5, 28.5)},
                {"line": at(-63.0, -28.5)},
                {"arc": "start", "center": at(-61.5, -28.5)}]}],
            ["feature.sheetMetal", {"sketch": "S", "name": "P", "thickness": "1.5 mm",
                "radius": "1.5 mm", "kFactor": 0.4}],
        ]);
        let (mut s, _) = run(&steps);
        checked(&mut s);
        let volume = volume(&mut s, "P");
        let (r, l) = (1.5, 57.0);
        let expected = (160.0 * 120.0 - (std::f64::consts::PI * r * r + 2.0 * r * l)) * 1.5;
        assert!(
            (volume - expected).abs() <= 1e-9 * expected,
            "({dx}, {dy}): {volume} for {expected}"
        );
    }
}

/// The enclosure's repro as it was (with its flange), and the rack's slot
/// of r 2 in a 3 mm sheet: built and checked.
#[test]
fn the_enclosure_and_the_rack_slots_pass_the_check() {
    let enclosure = json!([
        ["sketch.create", {"plane": "xy", "name": "S"}],
        ["sketch.rect", {"sketch": "S", "from": [-80, -60], "to": [80, 60]}],
        ["sketch.path", {"sketch": "S", "start": [-60, -28.5], "segments": [{"line": [-60, 28.5]}, {"arc": [-63, 28.5], "center": [-61.5, 28.5]}, {"line": [-63, -28.5]}, {"arc": "start", "center": [-61.5, -28.5]}]}],
        ["feature.sheetMetal", {"sketch": "S", "name": "P", "thickness": "1.5 mm", "radius": "1.5 mm", "kFactor": 0.4, "flanges": [{"edge": "e6", "length": "15 mm", "angle": "90 deg", "direction": "down"}]}],
    ]);
    let rack = json!([
        ["sketch.create", {"plane": "xy", "name": "P", "offset": 7}],
        ["sketch.rect", {"sketch": "P", "from": [-100, 0], "to": [100, 40]}],
        ["sketch.path", {"sketch": "P", "start": ["-1.7 mm", "16.7 mm"], "segments": [{"line": ["1.7 mm", "16.7 mm"]}, {"arc": ["1.7 mm", "23.3 mm"], "through": ["5 mm", "20 mm"]}, {"line": ["-1.7 mm", "23.3 mm"]}, {"arc": "start", "through": ["-5 mm", "20 mm"]}]}],
        ["feature.sheetMetal", {"sketch": "P", "name": "Plate", "thickness": "3 mm", "radius": "2 mm", "kFactor": 0.4}],
    ]);
    for steps in [enclosure, rack] {
        let (mut s, _) = run(&steps);
        checked(&mut s);
    }
}

/// A plate's rim round a circular hole is the hole's cylinder, as an
/// extrusion's side is: the same `radius` query finds one face on each,
/// and no face of the plate is freeform. The plate with a hole is
/// measured wherever it lies (the rack's first finding: "cannot be
/// measured" depending on the place).
#[test]
fn a_plate_rim_round_a_hole_is_its_cylinder() {
    let steps = json!([
        ["sketch.create", {"plane": "xy", "name": "A", "offset": 5}],
        ["sketch.rect", {"sketch": "A", "from": [10, 10], "to": [110, 60]}],
        ["sketch.circle", {"sketch": "A", "center": ["60 mm", "35 mm"], "radius": "6 mm"}],
        ["feature.sheetMetal", {"sketch": "A", "name": "Plate", "thickness": "2 mm", "radius": "2 mm", "kFactor": 0.4}],
        ["sketch.create", {"plane": "xy", "name": "B", "offset": 50}],
        ["sketch.rect", {"sketch": "B", "from": [10, 10], "to": [110, 60]}],
        ["sketch.circle", {"sketch": "B", "center": ["60 mm", "35 mm"], "radius": "6 mm"}],
        ["feature.extrude", {"sketch": "B", "distance": "2 mm", "name": "Block"}],
        ["select.query", {"kind": "face", "where": {"in": "Plate", "surface": "cylinder", "radius": "6 mm"}}],
        ["select.query", {"kind": "face", "where": {"in": "Block", "surface": "cylinder", "radius": "6 mm"}}],
        ["select.query", {"kind": "face", "where": {"in": "Plate", "surface": "freeform"}}],
    ]);
    let (_, out) = run(&steps);
    assert_eq!(out[8]["count"], 1, "{}", out[8]);
    assert_eq!(out[9]["count"], 1, "{}", out[9]);
    assert_eq!(out[10]["count"], 0, "{}", out[10]);
    for (dx, dy) in [(-50.0, 6.0), (0.0, 0.0), (731.25, -4096.5)] {
        let steps = json!([
            ["sketch.create", {"plane": "xy", "name": "P"}],
            ["sketch.rect", {"sketch": "P", "from": [-100.0 + dx, dy], "to": [100.0 + dx, 80.0 + dy]}],
            ["sketch.circle", {"sketch": "P", "center": [-50.0 + dx, 6.0 + dy], "radius": 3.25}],
            ["feature.sheetMetal", {"sketch": "P", "name": "Plate", "thickness": "2 mm", "radius": "2 mm", "kFactor": 0.33}],
        ]);
        let (mut s, _) = run(&steps);
        checked(&mut s);
        let volume = volume(&mut s, "Plate");
        let expected = (200.0 * 80.0 - std::f64::consts::PI * 3.25 * 3.25) * 2.0;
        assert!(
            (volume - expected).abs() <= 1e-9 * expected,
            "({dx}, {dy}): {volume} for {expected}"
        );
    }
}
