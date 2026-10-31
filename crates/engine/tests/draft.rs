//! Draft through the command (M9): faces tilted about a neutral plane,
//! measured against closed forms (frusta of pyramids and cones, a
//! rounded frustum) at three scales and checked deep; the neutral plane
//! as a face, a world plane or a datum that moves; a fillet and a sketch
//! downstream through edits; what does not fit or is not built fails
//! with the face and the reason; files, undo and redo.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;
mod face_ops_common;

use craft3d_engine::{Access, Session, Workspace};
use face_ops_common::*;
use serde_json::json;
use std::f64::consts::PI;

const SIDES: [&str; 4] = ["F1:face(2)", "F1:face(3)", "F1:face(4)", "F1:face(5)"];
const BOTTOM: &str = "F1:face(0)";

/// The volume of a frustum of a square pyramid.
fn frustum(a: f64, b: f64, h: f64) -> f64 {
    h / 3.0 * (a * a + a * b + b * b)
}

fn cone(r1: f64, r2: f64, h: f64) -> f64 {
    PI * h / 3.0 * (r1 * r1 + r1 * r2 + r2 * r2)
}

#[test]
fn a_block_drafted_from_its_bottom_its_top_or_a_world_plane() {
    for k in SCALES {
        let degrees = 7.0_f64;
        let tan = degrees.to_radians().ptan();
        for (neutral, reversed, volume) in [
            // Narrower toward the top.
            (BOTTOM, false, frustum(10.0, 10.0 - 10.0 * tan, 5.0)),
            ("xy", false, frustum(10.0, 10.0 - 10.0 * tan, 5.0)),
            // Narrower toward the bottom.
            (TOP, false, frustum(10.0 - 10.0 * tan, 10.0, 5.0)),
            // Wider toward the top.
            (BOTTOM, true, frustum(10.0, 10.0 + 10.0 * tan, 5.0)),
        ] {
            let mut s = session();
            new_document(&mut s, k);
            base(&mut s, k);
            let made = built(
                &mut s,
                "feature.draft",
                json!({"faces": SIDES, "neutral": neutral, "angle": format!("{degrees} deg"),
                       "reversed": reversed, "name": "Tapered"}),
            );
            assert_eq!(made["id"], "F2");
            let found = assert_solid(&mut s, "Tapered", volume, None, k);
            let expected: Vec<String> =
                (0..6).map(|i| format!("F2:copy(0)/F1:face({i})")).collect();
            assert_eq!(names_of(&found), expected, "{neutral} {reversed}");
            assert_eq!(body_names(&mut s), ["Tapered"]);
        }
    }
}

#[test]
fn a_rod_a_hole_and_a_rounded_box_drafted() {
    for k in SCALES {
        let tan = 5.0_f64.to_radians().ptan();
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.cylinder",
            json!({"name": "Rod", "radius": 2.0 * k, "height": 6.0 * k}),
        );
        built(
            &mut s,
            "feature.draft",
            json!({"faces": ["F1:face(0)"], "neutral": "F1:face(1)", "angle": "5 deg",
                   "name": "Pin"}),
        );
        assert_solid(&mut s, "Pin", cone(2.0, 2.0 - 6.0 * tan, 6.0), None, k);
        let mut s = session();
        new_document(&mut s, k);
        drilled(&mut s, k);
        built(
            &mut s,
            "feature.draft",
            json!({"faces": [WALL], "neutral": "F3:copy(0)/F1:face(0)", "angle": "5 deg",
                   "name": "Tapered"}),
        );
        assert_solid(
            &mut s,
            "Tapered",
            500.0 - cone(2.0, 2.0 + 5.0 * tan, 5.0),
            None,
            k,
        );
        // A block 10 x 8 x 5 with its upright edges rounded to 1.5.
        let (a, b, h, r) = (10.0, 8.0, 5.0, 1.5);
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.block",
            json!({"name": "Base", "size": [a * k, b * k, h * k]}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F1:face(2)", "F1:face(4)"], ["F1:face(2)", "F1:face(5)"],
                             ["F1:face(3)", "F1:face(4)"], ["F1:face(3)", "F1:face(5)"]],
                   "radius": r * k, "name": "Round"}),
        );
        let mut walls: Vec<String> = SIDES
            .iter()
            .map(|side| format!("F2:copy(0)/{side}"))
            .collect();
        walls.extend((0..4).map(|i| format!("F2:fillet({i})")));
        built(
            &mut s,
            "feature.draft",
            json!({"faces": walls, "neutral": "xy", "angle": "8 deg", "name": "Moulded"}),
        );
        let t = 8.0_f64.to_radians().ptan();
        let volume = a * b * h - (a + b) * t * h * h + 4.0 / 3.0 * t * t * h * h * h
            - (4.0 - PI) * (r * r * h - r * t * h * h + t * t * h * h * h / 3.0);
        assert_solid(&mut s, "Moulded", volume, None, k);
    }
}

#[test]
fn a_draft_follows_its_datum_and_carries_a_fillet_and_a_sketch() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "level", "expr": format!("{} mm", 2.5 * k)}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "slope", "expr": "6 deg"}),
        );
        base(&mut s, k);
        built(
            &mut s,
            "datum.plane",
            json!({"from": "xy", "distance": "level", "name": "Parting"}),
        );
        built(
            &mut s,
            "feature.draft",
            json!({"faces": SIDES, "neutral": "Parting", "angle": "slope", "name": "Tapered"}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F3:copy(0)/F1:face(1)", "F3:copy(0)/F1:face(2)"]],
                   "radius": 0.5 * k, "name": "Round"}),
        );
        built(
            &mut s,
            "sketch.create",
            json!({"name": "Mark", "face": "F3:copy(0)/F1:face(5)"}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Mark", "center": [0.0, 0.0], "radius": 0.5 * k}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Mark", "distance": 2.0 * k, "name": "Peg"}),
        );
        for (level, slope) in [(2.5, 6.0_f64), (0.0, 10.0), (5.0, 3.0), (1.0, -4.0)] {
            run(
                &mut s,
                "variable.set",
                json!({"name": "level", "expr": format!("{} mm", level * k)}),
            );
            run(
                &mut s,
                "variable.set",
                json!({"name": "slope", "expr": format!("{slope} deg")}),
            );
            for name in ["Tapered", "Round", "Mark", "Peg"] {
                let found = feature(&mut s, name);
                assert_eq!(found["status"], "ok", "{level} {slope}: {found}");
            }
            // The square at the parting level is 10; the faces lean in
            // above it and out below it.
            let tan = slope.to_radians().ptan();
            let (below, above) = (level, 5.0 - level);
            let volume = frustum(10.0 + 2.0 * below * tan, 10.0, below)
                + frustum(10.0, 10.0 - 2.0 * above * tan, above);
            let tapered = feature(&mut s, "Tapered");
            assert_eq!(tapered["status"], "ok");
            let check = run(&mut s, "document.check", json!({"deep": true}));
            assert_eq!(check["ok"], true, "{check}");
            // The fillet took the drafted body; its volume is below the
            // frustum's by its round: the section r^2 (tan(a/2) - a/2)
            // for a turn a of 90 degrees less the slope between the top
            // and the leaning front, along the top's edge (its ends cut
            // by the leaning sides: about).
            let round = body(&mut s, "Round");
            let v = round["volume"].as_f64().unwrap() / (k * k * k);
            let turn = (90.0 - slope).to_radians();
            let section = 0.25 * ((turn / 2.0).ptan() - turn / 2.0);
            let cut = section * (10.0 - 2.0 * above * tan);
            assert!(
                ((volume - v) / cut - 1.0).abs() < 0.05,
                "{v} {volume} {cut}"
            );
            // The peg stands square on the leaning side: it leans out by
            // the slope, so it rises or sinks over its length.
            let peg = body(&mut s, "Peg");
            let slack = peg["bboxTolerance"].as_f64().unwrap();
            let rise =
                (peg["bbox"][1][2].as_f64().unwrap() - peg["bbox"][0][2].as_f64().unwrap()) / k;
            let (sin, cos) = slope.to_radians().abs().psin_cos();
            let expected = 2.0 * sin + 1.0 * cos;
            assert!(
                rise <= expected + 1e-9 && rise >= expected - 2.0 * slack / k,
                "{rise} {expected}"
            );
        }
    }
}

#[test]
fn a_draft_too_steep_or_of_a_face_across_the_pull_says_why() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    base(&mut s, k);
    let reason = failed(
        &mut s,
        "feature.draft",
        json!({"faces": SIDES, "neutral": BOTTOM, "angle": "60 deg", "name": "Steep"}),
    );
    assert!(reason.starts_with("angle: 60 deg is too far: "), "{reason}");
    assert!(reason.contains("what fits is below 44.99"), "{reason}");
    run(&mut s, "edit.undo", json!({}));
    let reason = failed(
        &mut s,
        "feature.draft",
        json!({"faces": [TOP, "F1:face(2)"], "neutral": BOTTOM, "angle": "5 deg", "name": "Lid"}),
    );
    assert!(reason.starts_with("faces: "), "{reason}");
    assert!(reason.contains("pull direction"), "{reason}");
    assert!(reason.contains("(the face F1:face(1))"), "{reason}");
    run(&mut s, "edit.undo", json!({}));
    // A ball is not drafted.
    built(
        &mut s,
        "feature.sphere",
        json!({"name": "Ball", "position": [20.0, 0.0, 0.0], "radius": 2.0}),
    );
    let reason = failed(
        &mut s,
        "feature.draft",
        json!({"faces": ["F2:face(0)"], "neutral": "xy", "angle": "5 deg"}),
    );
    assert!(reason.contains("sphere"), "{reason}");
}

#[test]
fn bad_parameters_of_a_draft_are_errors() {
    let k = 1.0;
    let mut s = session();
    refused(
        &mut s,
        "feature.draft",
        json!({"faces": SIDES, "neutral": "xy", "angle": "5 deg"}),
    );
    new_document(&mut s, k);
    base(&mut s, k);
    built(
        &mut s,
        "feature.block",
        json!({"name": "Other", "position": [20.0, 0.0, 0.0], "size": [1.0, 1.0, 1.0]}),
    );
    for params in [
        json!({}),
        json!({"faces": SIDES}),
        json!({"faces": SIDES, "neutral": "xy"}),
        json!({"faces": SIDES, "angle": "5 deg"}),
        json!({"neutral": "xy", "angle": "5 deg"}),
        json!({"faces": [], "neutral": "xy", "angle": "5 deg"}),
        json!({"faces": SIDES, "neutral": "nowhere", "angle": "5 deg"}),
        json!({"faces": SIDES, "neutral": 7, "angle": "5 deg"}),
        json!({"faces": SIDES, "neutral": "Other", "angle": "5 deg"}),
        json!({"faces": SIDES, "neutral": "xy", "angle": [5]}),
        json!({"faces": SIDES, "neutral": "xy", "angle": "5 deg", "reversed": "yes"}),
        json!({"faces": [TOP, "F2:face(1)"], "neutral": "xy", "angle": "5 deg"}),
        json!({"faces": SIDES, "neutral": "xy", "angle": "5 deg", "far": true}),
    ] {
        refused(&mut s, "feature.draft", params);
    }
    for (params, starts) in [
        (
            json!({"faces": SIDES, "neutral": "xy", "angle": "0 deg"}),
            "angle: ",
        ),
        (
            json!({"faces": SIDES, "neutral": "xy", "angle": "90 deg"}),
            "angle: ",
        ),
        (
            json!({"faces": SIDES, "neutral": "xy", "angle": "5 mm"}),
            "angle: ",
        ),
        (
            json!({"faces": SIDES, "neutral": "F1:face(9)", "angle": "5 deg"}),
            "neutral: ",
        ),
        (
            json!({"faces": ["F1:face(9)"], "neutral": "xy", "angle": "5 deg"}),
            "faces.0: `Base` no longer has a face F1:face(9)",
        ),
    ] {
        let reason = failed(&mut s, "feature.draft", params.clone());
        assert!(reason.starts_with(starts), "{params}: {reason}");
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(body_names(&mut s), ["Base", "Other"]);
}

#[test]
fn drafts_are_edited_saved_reopened_undone_and_redone() {
    let dir = workdir("draft-files");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    let k = 1.0;
    new_document(&mut s, k);
    base(&mut s, k);
    built(
        &mut s,
        "feature.draft",
        json!({"faces": SIDES, "neutral": BOTTOM, "angle": "4 deg", "name": "Tapered"}),
    );
    let tan = |deg: f64| deg.to_radians().ptan();
    assert_solid(
        &mut s,
        "Tapered",
        frustum(10.0, 10.0 - 10.0 * tan(4.0), 5.0),
        None,
        k,
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Tapered", "neutral": TOP, "angle": "6 deg"}),
    );
    assert_solid(
        &mut s,
        "Tapered",
        frustum(10.0 - 10.0 * tan(6.0), 10.0, 5.0),
        None,
        k,
    );
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Tapered", "faces": ["F1:face(4)", "F1:face(5)"], "reversed": true}),
    );
    assert_solid(
        &mut s,
        "Tapered",
        10.0 * 5.0 * (10.0 + 10.0 + 10.0 * tan(6.0)) / 2.0,
        None,
        k,
    );
    let before = inspect(&mut s);
    run(&mut s, "edit.undo", json!({}));
    assert_ne!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "file.save", json!({"path": "draft.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "draft.3dcraft"}));
    let after = inspect(&mut s);
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}
