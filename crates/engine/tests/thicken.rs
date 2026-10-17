//! Thicken through the command (M9): faces of a body given a thickness
//! as a body of their own, measured against closed forms at three scales
//! and checked deep; the source keeps its body; the layer's faces carry
//! names to build on; upstream edits are followed; what does not fit
//! fails with the face and the largest thickness; files, undo and redo.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;
mod face_ops_common;

use craft3d_engine::{Access, Session, Workspace};
use face_ops_common::*;
use serde_json::json;
use std::f64::consts::PI;

#[test]
fn the_top_of_a_block_thickened_on_under_and_through_it() {
    for k in SCALES {
        for (side, bottom) in [("outside", 5.0), ("inside", 3.5), ("both", 4.25)] {
            let mut s = session();
            new_document(&mut s, k);
            base(&mut s, k);
            let made = built(
                &mut s,
                "feature.thicken",
                json!({"faces": [TOP], "thickness": 1.5 * k, "side": side, "name": "Plate"}),
            );
            assert_eq!(made["id"], "F2");
            let plate = assert_solid(&mut s, "Plate", 150.0, Some(block_area(10.0, 10.0, 1.5)), k);
            assert_close(&plate["bbox"][0][2], bottom * k, "bottom");
            // The far side, the near side and the rim along the sides.
            let mut names = names_of(&plate);
            names.sort();
            let mut expected = vec![
                "F2:copy(0)/F1:face(1)".to_string(),
                "F2:copy(1)/F1:face(1)".to_string(),
            ];
            expected.extend((2..6).map(|i| format!("F2:copy(0)/F1:face({i})")));
            expected.sort();
            assert_eq!(names, expected, "{side}");
            // The block keeps its body.
            assert_eq!(body_names(&mut s), ["Base", "Plate"]);
        }
        // Outside, the plate joined to the block is the block made taller.
        let mut s = session();
        new_document(&mut s, k);
        base(&mut s, k);
        built(
            &mut s,
            "feature.thicken",
            json!({"faces": [TOP], "thickness": 2.0 * k, "name": "Plate"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "union", "target": "Base", "tool": "Plate", "name": "Taller"}),
        );
        let taller = assert_solid(
            &mut s,
            "Taller",
            700.0,
            Some(block_area(10.0, 10.0, 7.0)),
            k,
        );
        assert_eq!(taller["faces"], 6);
    }
}

#[test]
fn a_tube_from_a_rod_and_sleeves_from_a_hole() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        built(
            &mut s,
            "feature.cylinder",
            json!({"name": "Rod", "radius": 2.0 * k, "height": 6.0 * k}),
        );
        built(
            &mut s,
            "feature.thicken",
            json!({"faces": ["F1:face(0)"], "thickness": 0.5 * k, "name": "Tube"}),
        );
        let annulus = |inner: f64, outer: f64| PI * (outer * outer - inner * inner);
        assert_solid(
            &mut s,
            "Tube",
            annulus(2.0, 2.5) * 6.0,
            Some(2.0 * PI * 4.5 * 6.0 + 2.0 * annulus(2.0, 2.5)),
            k,
        );
        for (side, inner, outer) in [("outside", 1.25, 2.0), ("inside", 2.0, 2.75)] {
            let mut s = session();
            new_document(&mut s, k);
            drilled(&mut s, k);
            built(
                &mut s,
                "feature.thicken",
                json!({"faces": [WALL], "thickness": 0.75 * k, "side": side, "name": "Sleeve"}),
            );
            assert_solid(
                &mut s,
                "Sleeve",
                annulus(inner, outer) * 5.0,
                Some(2.0 * PI * (inner + outer) * 5.0 + 2.0 * annulus(inner, outer)),
                k,
            );
        }
    }
}

#[test]
fn a_layer_follows_upstream_edits_and_its_faces_are_built_on() {
    for k in SCALES {
        let mut s = session();
        new_document(&mut s, k);
        run(
            &mut s,
            "variable.set",
            json!({"name": "t", "expr": format!("{} mm", k)}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "h", "expr": format!("{} mm", 5.0 * k)}),
        );
        built(
            &mut s,
            "feature.block",
            json!({"name": "Base", "size": [10.0 * k, 10.0 * k, "h"]}),
        );
        built(
            &mut s,
            "feature.thicken",
            json!({"faces": [TOP], "thickness": "t", "side": "both", "name": "Plate"}),
        );
        // A fillet on an edge of the far side, and a pin from the near
        // side, which looks into the block.
        built(
            &mut s,
            "feature.fillet",
            json!({"edges": [["F2:copy(0)/F1:face(1)", "F2:copy(0)/F1:face(2)"]],
                   "radius": 0.25 * k, "name": "Round"}),
        );
        built(
            &mut s,
            "sketch.create",
            json!({"name": "Mark", "face": "F2:copy(1)/F1:face(1)"}),
        );
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Mark", "center": [2.0 * k, 2.0 * k], "radius": 0.5 * k}),
        );
        built(
            &mut s,
            "feature.extrude",
            json!({"sketch": "Mark", "distance": k, "name": "Pin"}),
        );
        let round = (1.0 - PI / 4.0) * 0.0625 * 10.0;
        for (t, h) in [(1.0, 5.0), (0.5, 3.0), (2.0, 8.0)] {
            run(
                &mut s,
                "variable.set",
                json!({"name": "t", "expr": format!("{} mm", t * k)}),
            );
            run(
                &mut s,
                "variable.set",
                json!({"name": "h", "expr": format!("{} mm", h * k)}),
            );
            for name in ["Plate", "Round", "Mark", "Pin"] {
                assert_eq!(feature(&mut s, name)["status"], "ok", "{name} {t} {h}");
            }
            assert_solid(&mut s, "Round", 100.0 * t - round, None, k);
            // The pin hangs from the plate's underside into the block.
            let pin = body(&mut s, "Pin");
            assert_close(&pin["bbox"][1][2], (h - t / 2.0) * k, "top of the pin");
            assert_close(
                &pin["bbox"][0][2],
                (h - t / 2.0 - 1.0) * k,
                "foot of the pin",
            );
        }
        // Pointed at a side instead, inside.
        built(
            &mut s,
            "feature.edit",
            json!({"feature": "Plate", "faces": ["F1:face(5)"], "side": "inside", "thickness": 1.5 * k}),
        );
        assert_eq!(feature(&mut s, "Plate")["status"], "ok");
        // The edge the fillet rounds is there still, as the rim of the
        // new layer between the copies of the top and the front: 1.5
        // long now.
        let round = (1.0 - PI / 4.0) * 0.0625 * 1.5;
        let rounded = body(&mut s, "Round");
        assert_eq!(rounded["valid"], true, "{rounded}");
        assert_close(
            &rounded["volume"],
            (1.5 * 10.0 * 8.0 - round) * k * k * k,
            "volume",
        );
        // The near side of the old layer is gone, so the sketch on it
        // fails by name, and the pin with it.
        let mark = feature(&mut s, "Mark");
        assert_eq!(mark["status"], "failed", "{mark}");
        assert!(
            mark["error"]
                .as_str()
                .unwrap()
                .contains("no longer has a face F2:copy(1)/F1:face(1)"),
            "{mark}"
        );
        assert_eq!(feature(&mut s, "Pin")["causedBy"], "F4");
    }
}

#[test]
fn a_layer_too_thick_or_bad_parameters_say_why() {
    let k = 1.0;
    let mut s = session();
    refused(
        &mut s,
        "feature.thicken",
        json!({"faces": [TOP], "thickness": 1.0}),
    );
    new_document(&mut s, k);
    base(&mut s, k);
    let reason = failed(
        &mut s,
        "feature.thicken",
        json!({"faces": [TOP], "thickness": 6.0, "side": "inside", "name": "Thick"}),
    );
    assert!(reason.starts_with("thickness: 6 mm is too far"), "{reason}");
    assert!(reason.contains("what fits is below 4.9999"), "{reason}");
    run(&mut s, "edit.undo", json!({}));
    let reason = failed(
        &mut s,
        "feature.thicken",
        json!({"faces": [TOP], "thickness": 12.0, "side": "both", "name": "Thick"}),
    );
    assert!(reason.contains("what fits is below 9.9999"), "{reason}");
    run(&mut s, "edit.undo", json!({}));
    built(
        &mut s,
        "feature.block",
        json!({"name": "Other", "position": [20.0, 0.0, 0.0], "size": [1.0, 1.0, 1.0]}),
    );
    for params in [
        json!({}),
        json!({"faces": [TOP]}),
        json!({"thickness": 1.0}),
        json!({"faces": [], "thickness": 1.0}),
        json!({"faces": TOP, "thickness": 1.0}),
        json!({"faces": [TOP, TOP], "thickness": 1.0}),
        json!({"faces": [TOP, "F2:face(1)"], "thickness": 1.0}),
        json!({"faces": [TOP], "thickness": [1.0]}),
        json!({"faces": [TOP], "thickness": 1.0, "side": "up"}),
        json!({"faces": [TOP], "thickness": 1.0, "side": 1}),
        json!({"faces": [TOP], "thickness": 1.0, "far": true}),
    ] {
        refused(&mut s, "feature.thicken", params);
    }
    for (params, starts) in [
        (json!({"faces": [TOP], "thickness": 0.0}), "thickness: "),
        (json!({"faces": [TOP], "thickness": -1.0}), "thickness: "),
        (json!({"faces": [TOP], "thickness": "1 deg"}), "thickness: "),
        (
            json!({"faces": ["F1:face(9)"], "thickness": 1.0}),
            "faces.0: `Base` no longer has a face F1:face(9)",
        ),
    ] {
        let reason = failed(&mut s, "feature.thicken", params.clone());
        assert!(reason.starts_with(starts), "{params}: {reason}");
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(body_names(&mut s), ["Base", "Other"]);
}

#[test]
fn layers_are_saved_reopened_undone_and_redone() {
    let dir = workdir("thicken-files");
    let mut s = Session::new(Workspace {
        read: Access::Root(dir.clone()),
        write: Access::Root(dir.clone()),
    });
    let k = 1.0;
    new_document(&mut s, k);
    bossed(&mut s, k);
    built(
        &mut s,
        "feature.thicken",
        json!({"faces": [WALL, BOSS_TOP], "thickness": "1 mm / 2", "name": "Cap"}),
    );
    // A cap over the boss: a tube 0.5 thick and 3 high round it, and a
    // disc of radius 2.5 on top.
    assert_solid(
        &mut s,
        "Cap",
        PI * (2.5 * 2.5 - 4.0) * 3.0 + PI * 2.5 * 2.5 * 0.5,
        None,
        k,
    );
    let before = inspect(&mut s);
    run(&mut s, "edit.undo", json!({}));
    assert_ne!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(inspect(&mut s)["bodies"], before["bodies"]);
    run(&mut s, "file.save", json!({"path": "thicken.3dcraft"}));
    run(&mut s, "file.close", json!({}));
    run(&mut s, "file.open", json!({"path": "thicken.3dcraft"}));
    let after = inspect(&mut s);
    assert_eq!(after["bodies"], before["bodies"]);
    assert_eq!(after["features"], before["features"]);
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}
