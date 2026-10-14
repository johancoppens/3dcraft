//! Move face through the command (M9): faces of a body moved by one
//! rigid motion, measured against closed forms at three scales and
//! checked deep; a fillet on a moved face survives edits of the motion;
//! a sketch on a turned face turns with it; what does not fit or is not
//! built fails with the faces and the reason; files, undo and redo.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod face_ops_common;

use craft3d_engine::{Access, Session, Workspace};
use face_ops_common::*;
use serde_json::json;
use std::f64::consts::PI;

#[test]
fn the_top_of_a_block_shifted_and_tilted() {
    for k in SCALES {
        for d in [0.5, -1.5] {
            let mut s = session();
            new_document(&mut s, k);
            base(&mut s, k);
            let made = built(
                &mut s,
                "feature.moveFace",
                json!({"faces": [TOP], "translate": [0.0, 0.0, d * k], "name": "Moved"}),
            );
            assert_eq!(made["id"], "F2");
            let h = 5.0 + d;
            let found = assert_solid(
                &mut s,
                "Moved",
                100.0 * h,
                Some(block_area(10.0, 10.0, h)),
                k,
            );
            let expected: Vec<String> =
                (0..6).map(|i| format!("F2:copy(0)/F1:face({i})")).collect();
            assert_eq!(names_of(&found), expected);
            assert_eq!(body_names(&mut s), ["Moved"]);
        }
        // Tilted about the line along y across the middle of the top: a
        // wedge of the same volume.
        for degrees in [20.0_f64, -35.0] {
            let mut s = session();
            new_document(&mut s, k);
            base(&mut s, k);
            built(
                &mut s,
                "feature.moveFace",
                json!({"faces": [TOP], "angle": format!("{degrees} deg"), "axis": [0, 1, 0],
                       "pivot": [5.0 * k, 5.0 * k, 5.0 * k], "name": "Wedge"}),
            );
            let cos = degrees.to_radians().cos();
            assert_solid(&mut s, "Wedge", 500.0, Some(300.0 + 100.0 / cos), k);
        }
    }
}

#[test]
fn a_hole_and_a_boss_moved_across_their_face() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        drilled(&mut s, k);
        built(
            &mut s,
            "feature.moveFace",
            json!({"faces": [WALL], "translate": [2.0 * k, -k, 0.0], "name": "Shifted"}),
        );
        let (volume, area) = drilled_mass(2.0, 5.0);
        let found = assert_solid(&mut s, "Shifted", volume, Some(area), k);
        assert_eq!(found["faces"], 7);
        // The hole is at (7, 4) now: a pin of radius 1 there fills part
        // of it, and a union with it has the pin's volume above and
        // below the block only.
        built(
            &mut s,
            "feature.cylinder",
            json!({"name": "Pin", "position": [7.0 * k, 4.0 * k, -k],
                   "radius": k, "height": 7.0 * k}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": "Shifted", "tool": "Pin", "name": "Pinned"}),
        );
        assert_solid(&mut s, "Pinned", volume + PI * 7.0, None, k);
        let mut s = session();
        new_document(&mut s, k);
        bossed(&mut s, k);
        built(
            &mut s,
            "feature.moveFace",
            json!({"faces": [WALL, BOSS_TOP], "translate": [-2.5 * k, 2.0 * k, 1.0 * k],
                   "name": "Shifted"}),
        );
        let found = assert_solid(
            &mut s,
            "Shifted",
            500.0 + PI * 4.0 * 4.0,
            Some(block_area(10.0, 10.0, 5.0) + 2.0 * PI * 2.0 * 4.0),
            k,
        );
        // The boss's top is at 9 now, and at x from 0.5 to 4.5.
        let bbox = &found["bbox"];
        assert_close(&bbox[1][2], 9.0 * k, "top");
    }
}

#[test]
fn a_fillet_on_a_moved_face_survives_edits_of_the_motion() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "lift", "expr": format!("{} mm", k)}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "tilt", "expr": "0 deg"}),
        );
        base(&mut s, k);
        built(
            &mut s,
            "feature.moveFace",
            json!({"faces": [TOP], "translate": [0, 0, "lift"], "angle": "tilt",
                   "axis": [0, 1, 0], "pivot": [5.0 * k, 5.0 * k, 5.0 * k], "name": "Moved"}),
        );
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F2:copy(0)/F1:face(1)", "F2:copy(0)/F1:face(2)"]],
                   "radius": k, "name": "Round"}),
        );
        // A quarter of the square of the radius less the quarter circle,
        // along the edge.
        let round = |length: f64| (1.0 - PI / 4.0) * length;
        assert_solid(&mut s, "Round", 600.0 - round(10.0), None, k);
        for (lift, tilt) in [(2.5, 0.0_f64), (0.5, 15.0), (-1.0, -25.0)] {
            run(
                &mut s,
                "variable.set",
                json!({"name": "lift", "expr": format!("{} mm", lift * k)}),
            );
            run(
                &mut s,
                "variable.set",
                json!({"name": "tilt", "expr": format!("{tilt} deg")}),
            );
            for name in ["Moved", "Round"] {
                let found = feature(&mut s, name);
                assert_eq!(found["status"], "ok", "{lift} {tilt}: {found}");
            }
            // The edge runs across the tilted top, 10 / cos long between
            // the two side faces it ends in.
            let cos = tilt.to_radians().cos();
            assert_solid(
                &mut s,
                "Round",
                100.0 * (5.0 + lift) - round(10.0 / cos),
                None,
                k,
            );
        }
    }
}

#[test]
fn a_sketch_on_a_turned_face_turns_with_it() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        base(&mut s, k);
        let degrees: f64 = 30.0;
        built(
            &mut s,
            "feature.moveFace",
            json!({"faces": [TOP], "angle": format!("{degrees} deg"), "axis": [0, 1, 0],
                   "pivot": [5.0 * k, 5.0 * k, 5.0 * k], "name": "Wedge"}),
        );
        built(
            &mut s,
            "sketch.create",
            json!({"name": "Mark", "face": "F2:copy(0)/F1:face(1)"}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Mark", "center": [0.0, 0.0], "radius": 0.5 * k}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Mark", "distance": 2.0 * k, "name": "Pin"}),
        );
        // A rod of radius 0.5 and length 2 whose axis leans 30 degrees
        // from z: that is how high it reaches over its foot.
        let pin = assert_solid(&mut s, "Pin", PI * 0.25 * 2.0, None, k);
        let (sin, cos) = degrees.to_radians().sin_cos();
        let low = pin["bbox"][0][2].as_f64().unwrap();
        let high = pin["bbox"][1][2].as_f64().unwrap();
        let height = high - low;
        let expected = (2.0 * cos + 1.0 * sin) * k;
        // The box is of a mesh: inside the true one by its tolerance.
        let slack = pin["bboxTolerance"].as_f64().unwrap();
        assert!(
            height <= expected * (1.0 + 1e-9) && height >= expected - 2.0 * slack,
            "{height} {expected} {slack}"
        );
    }
}

#[test]
fn a_move_too_far_or_not_built_says_why() {
    let k = 1.0;
    let mut s = session();
    new_document(&mut s, k);
    base(&mut s, k);
    let reason = failed(
        &mut s,
        "feature.moveFace",
        json!({"faces": [TOP], "translate": [0, 0, -6], "name": "Flat"}),
    );
    assert!(
        reason.starts_with("translate, angle: the move is too far: the edge shrinks to nothing"),
        "{reason}"
    );
    assert!(reason.contains("what fits is below 0.8333"), "{reason}");
    assert!(reason.contains("translate (0, 0, -4.99"), "{reason}");
    run(&mut s, "edit.undo", json!({}));
    let reason = failed(
        &mut s,
        "feature.moveFace",
        json!({"faces": [TOP], "angle": "60 deg", "axis": [0, 1, 0], "pivot": [5, 5, 5],
               "name": "Steep"}),
    );
    assert!(reason.contains("angle 44.99"), "{reason}");
    // A hole's wall tilted: it would meet the top in an ellipse.
    let mut s = session();
    new_document(&mut s, k);
    drilled(&mut s, k);
    let reason = failed(
        &mut s,
        "feature.moveFace",
        json!({"faces": [WALL], "angle": "5 deg", "axis": [1, 0, 0], "pivot": [5, 5, 2.5],
               "name": "Slanted"}),
    );
    assert!(reason.starts_with("faces: "), "{reason}");
    assert!(reason.contains("line or a circle"), "{reason}");
    assert!(reason.contains(WALL), "{reason}");
    // The hole run out of the side of the block.
    run(&mut s, "edit.undo", json!({}));
    let reason = failed(
        &mut s,
        "feature.moveFace",
        json!({"faces": [WALL], "translate": [4, 0, 0], "name": "Out"}),
    );
    assert!(reason.contains("is too far"), "{reason}");
}

#[test]
fn bad_parameters_of_a_move_are_errors() {
    let k = 1.0;
    let mut s = session();
    refused(
        &mut s,
        "feature.moveFace",
        json!({"faces": [TOP], "translate": [0, 0, 1]}),
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
        json!({"faces": [TOP]}),
        json!({"translate": [0, 0, 1]}),
        json!({"faces": [], "translate": [0, 0, 1]}),
        json!({"faces": TOP, "translate": [0, 0, 1]}),
        json!({"faces": [TOP, TOP], "translate": [0, 0, 1]}),
        json!({"faces": [TOP, "F2:face(1)"], "translate": [0, 0, 1]}),
        json!({"faces": [TOP], "translate": 1.0}),
        json!({"faces": [TOP], "translate": [0, 1]}),
        json!({"faces": [TOP], "angle": [1.0]}),
        json!({"faces": [TOP], "translate": [0, 0, 1], "far": true}),
    ] {
        refused(&mut s, "feature.moveFace", params);
    }
    for (params, starts) in [
        (
            json!({"faces": [TOP], "translate": [0, 0, 0]}),
            "faces: there is nothing to move",
        ),
        (
            json!({"faces": [TOP], "translate": [0, 0, "1 deg"]}),
            "translate.z: ",
        ),
        (
            json!({"faces": [TOP], "angle": "10 mm", "axis": [0, 1, 0]}),
            "angle: ",
        ),
        (
            json!({"faces": [TOP], "angle": "10 deg", "axis": [0, 0, 0]}),
            "axis must be a direction",
        ),
        (
            json!({"faces": ["F1:face(9)"], "translate": [0, 0, 1]}),
            "faces.0: `Base` no longer has a face F1:face(9)",
        ),
    ] {
        let reason = failed(&mut s, "feature.moveFace", params.clone());
        assert!(reason.starts_with(starts), "{params}: {reason}");
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(body_names(&mut s), ["Base", "Other"]);
}

#[test]
fn moves_are_edited_saved_reopened_undone_and_redone() {
    let dir = workdir("move-files");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    let k = 1.0;
    new_document(&mut s, k);
    bossed(&mut s, k);
    built(
        &mut s,
        "feature.moveFace",
        json!({"faces": [WALL, BOSS_TOP], "translate": ["3 mm / 2", 0, 0], "name": "Shifted"}),
    );
    let volume = 500.0 + PI * 4.0 * 3.0;
    assert_solid(&mut s, "Shifted", volume, None, k);
    // Pointed at the top of the block and lifted instead.
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Shifted", "faces": [TOP_OF_3], "translate": [0, 0, 1]}),
    );
    // The block is 1 taller; the boss's top stays, so the boss is 1
    // shorter.
    assert_solid(&mut s, "Shifted", 600.0 + PI * 4.0 * 2.0, None, k);
    built(
        &mut s,
        "feature.edit",
        json!({"feature": "Shifted", "faces": [WALL, BOSS_TOP], "translate": [0, 0, 0],
               "angle": "40 deg", "pivot": [3, 5, 0]}),
    );
    assert_solid(&mut s, "Shifted", volume, None, k);
    let before = inspect(&mut s);
    run(&mut s, "edit.undo", json!({}));
    assert_ne!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "file.save", json!({"path": "move.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "move.3dcraft"}));
    let after = inspect(&mut s);
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}
