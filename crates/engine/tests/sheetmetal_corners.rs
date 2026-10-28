//! Closed corners of sheet metal through the commands
//! (docs/design/sheetmetal.md › *Closed corners*): a tray whose four
//! corners are closed by butt, overlap and mitre, chosen by name and by
//! an expression (a Jig's choice, 1 open .. 4 miter), with its relief's
//! shape picked by an expression too; the gap at a corner measured on the
//! solid two ways (the `cornerGap` measure and the plain `distance`
//! between the two flanges' faces) equals the gap given; the bend table
//! lists each corner's treatment; saved and reopened to the same body;
//! the flat pattern exported; bad treatments are errors or fail the
//! feature with the reason. At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, error, failed, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

/// A rectangle `w` × `d` from the origin in the xy plane; its lines'
/// entity ids (bottom, right, top, left).
fn rect(s: &mut Session, name: &str, w: f64, d: f64) -> Vec<String> {
    run(s, "sketch.create", json!({"plane": "xy", "name": name}));
    let made = run(
        s,
        "sketch.rect",
        json!({"sketch": name, "from": [0.0, 0.0], "to": [w, d]}),
    );
    made["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect()
}

fn listed(s: &mut Session, name: &str) -> Value {
    let all = run(s, "assert.list", json!({}));
    all["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("no assertion {name} in {all}"))
        .clone()
}

fn assertion(s: &mut Session, name: &str, spec: Value) -> Value {
    let mut params = spec;
    params["name"] = json!(name);
    run(s, "assert.add", params);
    listed(s, name)
}

fn tray(s: &mut Session, k: f64, kind: &str, shape: &str) -> Vec<String> {
    let lines = rect(s, "Base", 60.0 * k, 40.0 * k);
    let flanges: Vec<Value> = lines
        .iter()
        .map(|e| json!({"edge": e, "length": format!("{} mm", 10.0 * k), "angle": "90 deg"}))
        .collect();
    built(
        s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Tray", "thickness": "t", "radius": format!("{} mm", k),
               "kFactor": 0.5, "relief": {"shape": shape, "size": format!("{} mm", 3.0 * k)},
               "corner": {"kind": kind, "gap": "g"}, "flanges": flanges}),
    );
    lines
}

#[test]
fn closed_corners_have_the_gap_they_are_given() {
    for k in SCALES {
        for (kind, by) in [
            ("butt", "butt"),
            ("overlap", "overlap"),
            ("miter", "miter"),
            ("overlap", "choice"),
        ] {
            let dir = workdir("sheetmetal-corners", &format!("{kind}-{by}-{k}"));
            let mut s = session_in(&dir);
            new_document(&mut s, k);
            let (t, g) = (1.0 * k, 0.25 * k);
            run(
                &mut s,
                "variable.set",
                json!({"name": "t", "expr": format!("{t} mm")}),
            );
            run(
                &mut s,
                "variable.set",
                json!({"name": "g", "expr": format!("{g} mm")}),
            );
            // A Jig's choices: 1 open, 2 butt, 3 overlap, 4 miter; the
            // relief 1 rect, 2 round, 3 tear.
            run(&mut s, "variable.set", json!({"name": "how", "expr": "3"}));
            run(
                &mut s,
                "variable.set",
                json!({"name": "notch", "expr": "2"}),
            );
            let (written, shape) = if by == "choice" {
                ("how", "notch")
            } else {
                (kind, "round")
            };
            tray(&mut s, k, written, shape);
            let found = body(&mut s, "Tray");
            assert_eq!(found["valid"], true, "{found}");
            let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Tray"}));
            let corners = table["corners"].as_array().unwrap();
            assert_eq!(corners.len(), 4, "{table}");
            for c in corners {
                assert_eq!(c["kind"], kind, "{c}");
                assert_eq!(c["relief"], "round", "{c}");
                assert!((number(&c["gap"]) - g).abs() <= 1e-12 * k, "{c}");
                if kind == "miter" {
                    assert_eq!(c["over"], Value::Null, "{c}");
                } else {
                    assert_eq!(c["over"], c["bends"][0], "{c}");
                }
            }
            // The gap measured on the solid, by the sheet-metal measure ...
            let check = assertion(
                &mut s,
                "Gap",
                json!({"measure": "cornerGap", "of": "Tray", "eq": "g"}),
            );
            assert_eq!(check["status"], "passed", "{kind} {by} {k}: {check}");
            // ... and by the plain distance between the faces of flange 0
            // (face(2)) and flange 1 (face(4)), which meet at (60, 0).
            let check = assertion(
                &mut s,
                "Plain",
                json!({"measure": "distance", "from": {"name": "*face(2)"}, "to": {"name": "*face(4)"}, "eq": "g"}),
            );
            assert_eq!(check["status"], "passed", "{kind} {by} {k}: {check}");
            let check = assertion(
                &mut s,
                "Wider",
                json!({"measure": "cornerGap", "of": "Tray", "min": "2 * g"}),
            );
            assert_eq!(check["status"], "failed", "{check}");
            // Saved and reopened: the same body.
            run(&mut s, "file.save", json!({"path": "tray.3dcraft"}));
            let mut u = session_in(&dir);
            run(&mut u, "file.open", json!({"path": "tray.3dcraft"}));
            let again = body(&mut u, "Tray");
            assert_eq!(found["volume"], again["volume"]);
            assert_eq!(found["faceNames"], again["faceNames"]);
            // The flat pattern, with the extensions.
            run(
                &mut s,
                "file.export",
                json!({"path": "tray.dxf", "sheetMetal": "Tray"}),
            );
            assert!(dir.join("tray.dxf").exists());
        }
    }
}

/// One corner its own way: overlap covered by the other flange, with a
/// wider gap; the rest open. Switching every corner to open by the choice
/// (1) gives the open tray back.
#[test]
fn one_corner_its_own_way_and_open_by_choice() {
    let dir = workdir("sheetmetal-corners", "own");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    run(&mut s, "variable.set", json!({"name": "t", "expr": "1 mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "g", "expr": "0.2 mm"}),
    );
    run(&mut s, "variable.set", json!({"name": "how", "expr": "1"}));
    tray(&mut s, 1.0, "how", "rect");
    let open = body(&mut s, "Tray");
    run(
        &mut s,
        "sheetMetal.edit",
        json!({"feature": "Tray", "corners": [{"flanges": [1, 0], "kind": "overlap", "gap": "0.5 mm", "over": 1}]}),
    );
    let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Tray"}));
    let corners = table["corners"].as_array().unwrap();
    let kinds: Vec<&str> = corners
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds.iter().filter(|k| **k == "open").count(), 3, "{table}");
    let own = corners.iter().find(|c| c["kind"] == "overlap").unwrap();
    assert_eq!(own["over"], 1, "{own}");
    let check = assertion(
        &mut s,
        "Gap",
        json!({"measure": "cornerGap", "of": "Tray", "eq": "0.5 mm"}),
    );
    assert_eq!(check["status"], "passed", "{check}");
    run(
        &mut s,
        "sheetMetal.edit",
        json!({"feature": "Tray", "corners": []}),
    );
    let check = listed(&mut s, "Gap");
    assert_eq!(check["status"], "error", "{check}");
    assert_eq!(body(&mut s, "Tray")["volume"], open["volume"]);
}

#[test]
fn bad_corner_treatments_are_errors_or_fail_the_part() {
    let dir = workdir("sheetmetal-corners", "refusals");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let lines = rect(&mut s, "Base", 60.0, 40.0);
    let two = json!([{"edge": lines[0], "length": "10 mm", "angle": "90 deg"},
                     {"edge": lines[1], "length": "10 mm", "angle": "90 deg"}]);
    let part = |corner: Value| {
        json!({"sketch": "Base", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "flanges": two, "corner": corner})
    };
    // `mitre`, the plate joints' spelling, is miter too.
    let made = built(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "name": "Spelt", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "flanges": two, "corner": {"kind": "mitre", "gap": "0.2 mm"}}),
    );
    assert_eq!(made["status"], "ok");
    let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Spelt"}));
    assert_eq!(table["corners"][0]["kind"], "miter", "{table}");
    let e = error(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "flanges": two, "corners": [{"flanges": [0, 1], "kind": "overlap", "gap": true}]}),
    );
    assert!(e.contains("corners.0.gap"), "{e}");
    for (corner, says) in [
        (json!({"kind": "closed", "gap": "1 mm"}), "picks"),
        (json!({"kind": 2.5, "gap": "1 mm"}), "picks"),
        (json!({"kind": 7, "gap": "1 mm"}), "picks"),
        (
            json!({"kind": "overlap", "gap": "0 mm"}),
            "more than the tolerance",
        ),
        (
            json!({"kind": "overlap", "gap": "-1 mm"}),
            "more than the tolerance",
        ),
        (
            json!({"kind": "butt", "gap": "1 mm"}),
            "less than the thickness",
        ),
    ] {
        let reason = failed(&mut s, "feature.sheetMetal", part(corner.clone()));
        assert!(reason.contains(says), "{corner}: {reason}");
    }
    // Leaning in (120 degrees), closing would cut the flanges back.
    let leaning = json!([{"edge": lines[0], "length": "10 mm", "angle": "120 deg"},
                         {"edge": lines[1], "length": "10 mm", "angle": "120 deg"}]);
    let reason = failed(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "flanges": leaning, "corner": {"kind": "miter", "gap": "0.2 mm"}}),
    );
    assert!(reason.contains("cut the flange back"), "{reason}");
    let reason = failed(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "flanges": two, "relief": {"shape": "3 + 1", "size": "3 mm"}}),
    );
    assert!(
        reason.contains("relief.shape") && reason.contains("picks"),
        "{reason}"
    );
    for (params, says) in [
        (part(json!({"kind": "overlap"})), "gap"),
        (
            part(json!({"kind": "overlap", "gap": "1 mm", "lead": 0})),
            "unknown key",
        ),
        (part(json!("overlap")), "corner is"),
        (part(json!({"kind": true, "gap": "1 mm"})), "corner.kind"),
        (part(json!({"kind": ""})), "corner.kind"),
    ] {
        let e = error(&mut s, "feature.sheetMetal", params.clone());
        assert!(e.contains(says), "{params}: {e}");
    }
    let base = |corners: Value| {
        json!({"sketch": "Base", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "flanges": two, "corners": corners})
    };
    for (corners, says) in [
        (
            json!([{"flanges": [0, 0], "kind": "overlap"}]),
            "two different",
        ),
        (json!([{"flanges": [0], "kind": "overlap"}]), "two flanges"),
        (
            json!([{"flanges": [0, 9], "kind": "overlap"}]),
            "no flange 9",
        ),
        (
            json!([{"flanges": [0, 1], "kind": "overlap", "over": 4}]),
            "over",
        ),
        (json!([{"flanges": [0, 1]}]), "kind"),
        (json!({"flanges": [0, 1]}), "list"),
    ] {
        let e = error(&mut s, "feature.sheetMetal", base(corners.clone()));
        assert!(e.contains(says), "{corners}: {e}");
    }
    // Flanges 0 and 2 (bottom and top) do not meet.
    let three = json!([{"edge": lines[0], "length": "10 mm", "angle": "90 deg"},
                       {"edge": lines[1], "length": "10 mm", "angle": "90 deg"},
                       {"edge": lines[2], "length": "10 mm", "angle": "90 deg"}]);
    let reason = failed(
        &mut s,
        "feature.sheetMetal",
        json!({"sketch": "Base", "thickness": "1 mm", "radius": "1 mm", "kFactor": 0.4,
               "flanges": three, "corners": [{"flanges": [0, 2], "kind": "overlap", "gap": "0.2 mm"}]}),
    );
    assert!(reason.contains("do not meet at a corner"), "{reason}");
    let e = error(
        &mut s,
        "assert.add",
        json!({"name": "G", "measure": "cornerGap", "of": "Tray", "axis": "x", "eq": "1 mm"}),
    );
    assert!(e.contains("axis") || e.contains("unknown"), "{e}");
}

/// The guide's closed-corner example runs as written: the box builds,
/// its gap assertion passes, and the flat pattern is written.
#[test]
fn the_guide_closed_corners_run_as_it_says() {
    let guide = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/guide/09-features.md");
    let text = std::fs::read_to_string(guide).unwrap();
    let fence = "`".repeat(3);
    let section = text.split("**Closed corners.**").nth(1).unwrap();
    let block = section
        .split(&format!("{fence}json"))
        .nth(1)
        .unwrap()
        .split(&fence)
        .next()
        .unwrap();
    let steps: Value = serde_json::from_str(block).unwrap();
    let dir = workdir("sheetmetal-corners", "guide");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    for step in steps.as_array().unwrap() {
        let id = step[0].as_str().unwrap();
        let result = run(&mut s, id, step[1].clone());
        assert_ne!(result["status"], "failed", "{id}: {result}");
        if id == "assert.add" {
            let name = step[1]["name"].as_str().unwrap();
            let check = listed(&mut s, name);
            assert_eq!(check["status"], "passed", "{check}");
        }
    }
    let table = run(&mut s, "sheetMetal.bends", json!({"feature": "Box"}));
    let kinds: Vec<&str> = table["corners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["overlap"; 4], "{table}");
    assert!(dir.join("box.dxf").exists());
}

/// The area of the one face the name names.
fn face_area(s: &mut Session, name: &str) -> f64 {
    let check = assertion(
        s,
        "AreaOf",
        json!({"measure": "area", "of": {"kind": "face", "where": [{"name": name}]}, "min": "0 mm^2"}),
    );
    run(s, "assert.delete", json!({"name": "AreaOf"}));
    check["measured"]
        .as_f64()
        .unwrap_or_else(|| panic!("{check}"))
}

/// Closing a corner adds pieces to the flats' outlines, but every face
/// keeps its name and its target: each rim of the open part (a tip, a
/// side, a bend's end) is still there when the corners close, as the same
/// face (the same area: a side's rim is the extension's end now, as long
/// as the side was), and opening them again gives the names back exactly.
/// A Jig flips the kind with a choice.
#[test]
fn names_keep_their_targets_when_corners_open_and_close() {
    let dir = workdir("sheetmetal-corners", "names");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    run(&mut s, "variable.set", json!({"name": "t", "expr": "1 mm"}));
    run(
        &mut s,
        "variable.set",
        json!({"name": "g", "expr": "0.2 mm"}),
    );
    run(&mut s, "variable.set", json!({"name": "how", "expr": "1"}));
    tray(&mut s, 1.0, "how", "round");
    let names = |s: &mut Session| -> Vec<String> {
        body(s, "Tray")["faceNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_string())
            .collect()
    };
    let open = names(&mut s);
    let rims: Vec<String> = open
        .iter()
        .filter(|n| n.contains("rim("))
        .cloned()
        .collect();
    assert!(rims.len() >= 12, "{open:?}");
    let before: Vec<f64> = rims.iter().map(|n| face_area(&mut s, n)).collect();
    for how in ["3", "2", "4"] {
        run(&mut s, "variable.set", json!({"name": "how", "expr": how}));
        let closed = names(&mut s);
        for name in &open {
            assert!(
                closed.contains(name),
                "kind {how}: {name} is gone: {closed:?}"
            );
        }
        if how == "3" {
            // Overlap at 90 degrees: square ends, each as long as its side.
            for (name, area) in rims.iter().zip(&before) {
                let now = face_area(&mut s, name);
                assert!((now - area).abs() <= 1e-6 * area, "{name}: {area} -> {now}");
            }
        }
    }
    run(&mut s, "variable.set", json!({"name": "how", "expr": "1"}));
    assert_eq!(names(&mut s), open);
}
