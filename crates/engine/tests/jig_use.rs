//! The two roles of a Jig (docs/decisions.md, 2026-10-23): designed in a
//! `.3dcraft` document and published with `jig.export` as a `.3dcraftjig`
//! file; used by opening that file, which only `param.set` changes, which
//! is never saved over, and which `jig.edit` copies into a new model. The
//! oracles: the published file opens to the same document, the bytes of
//! the `.3dcraftjig` never change while it is used, and a block's volume
//! follows the parameter that sizes it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{refused, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::json;

/// A plate `w` × 20 × 5 mm, `w` public from 10 to 90 mm.
fn design(s: &mut Session) {
    run(s, "file.new", json!({"units": "mm"}));
    run(s, "variable.set", json!({"name": "w", "expr": "40 mm"}));
    run(
        s,
        "feature.block",
        json!({"name": "Plate", "size": ["w", "20 mm", "5 mm"]}),
    );
}

fn publish(s: &mut Session) {
    run(
        s,
        "variable.publish",
        json!({"name": "w", "label": "Width", "min": "10 mm", "max": "90 mm"}),
    );
    run(
        s,
        "document.info",
        json!({"title": "Plate", "licence": "MIT"}),
    );
}

fn volume(s: &mut Session) -> f64 {
    run(s, "document.measure", json!({}))["volume"]
        .as_f64()
        .unwrap()
}

#[test]
fn a_jig_is_published_from_a_model_and_used_read_only() {
    let dir = workdir("jig-use", "roles");
    let mut s = session_in(&dir);
    design(&mut s);
    // No public parameter: no Jig to publish.
    let e = refused(&mut s, "jig.export", json!({"path": "plate.3dcraftjig"}));
    assert!(e.contains("variable.publish"), "{e}");
    publish(&mut s);
    // Only a .3dcraftjig path, and file.save never writes one.
    let e = refused(&mut s, "jig.export", json!({"path": "plate.3dcraft"}));
    assert!(e.contains(".3dcraftjig"), "{e}");
    let e = refused(&mut s, "file.save", json!({"path": "plate.3dcraftjig"}));
    assert!(e.contains("jig.export"), "{e}");
    let exported = run(&mut s, "jig.export", json!({"path": "plate.3dcraftjig"}));
    assert_eq!(exported["thumbnail"], true, "{exported}");
    let bytes = std::fs::read(dir.join("plate.3dcraftjig")).unwrap();
    assert!(craft3d_format::thumbnail(&bytes).unwrap().is_some());
    // The design document is unchanged by the export: still unsaved.
    let listed = run(&mut s, "file.list", json!({}));
    assert_eq!(listed["documents"][0]["path"], serde_json::Value::Null);
    assert_eq!(listed["documents"][0]["dirty"], true);

    // Using it.
    let mut u = session_in(&dir);
    let opened = run(&mut u, "file.open", json!({"path": "plate.3dcraftjig"}));
    assert_eq!(opened["jigInUse"], true, "{opened}");
    assert_eq!(
        run(&mut u, "file.list", json!({}))["documents"][0]["jigInUse"],
        true
    );
    assert!(u.document_status().unwrap().jig_in_use);
    assert!((volume(&mut u) - 40.0 * 20.0 * 5.0).abs() < 1e-6);
    // Its parameters change, and that is no unsaved change.
    run(&mut u, "param.set", json!({"values": {"w": 60}}));
    assert!((volume(&mut u) - 60.0 * 20.0 * 5.0).abs() < 1e-6);
    let status = u.document_status().unwrap();
    assert!(!status.dirty && !status.close_asks);
    // Undo and redo of its parameters are fine; nothing else changes it.
    run(&mut u, "edit.undo", json!({}));
    assert!((volume(&mut u) - 40.0 * 20.0 * 5.0).abs() < 1e-6);
    run(&mut u, "edit.redo", json!({}));
    for (id, params) in [
        ("variable.set", json!({"name": "w", "expr": "50 mm"})),
        ("feature.block", json!({"size": [1, 1, 1]})),
        ("feature.delete", json!({"feature": "Plate"})),
        ("variable.unpublish", json!({"name": "w"})),
        ("document.info", json!({"title": "Mine"})),
        ("history.rollback", json!({"feature": "Plate"})),
    ] {
        let e = refused(&mut u, id, params);
        assert!(e.contains("this is a Jig"), "{id}: {e}");
        assert!(e.contains("save it as a model"), "{id}: {e}");
    }
    // Looking is allowed: inspecting, measuring, the history's steps,
    // exports.
    run(&mut u, "document.inspect", json!({}));
    run(&mut u, "view.scrub", json!({"step": 1}));
    run(&mut u, "view.scrub", json!({"feature": null}));
    run(&mut u, "file.export", json!({"path": "plate.stl"}));
    // Save never writes the .3dcraftjig: it saves a model.
    let e = refused(&mut u, "file.save", json!({}));
    assert!(e.contains(".3dcraft"), "{e}");
    let e = refused(&mut u, "file.save", json!({"path": "plate.3dcraftjig"}));
    assert!(e.contains("never saved over"), "{e}");
    assert_eq!(std::fs::read(dir.join("plate.3dcraftjig")).unwrap(), bytes);
    run(&mut u, "file.save", json!({"path": "my-plate.3dcraft"}));
    // Saved as a model, it is edited as any other.
    assert!(!u.document_status().unwrap().jig_in_use);
    run(
        &mut u,
        "variable.set",
        json!({"name": "w", "expr": "70 mm"}),
    );
    assert_eq!(std::fs::read(dir.join("plate.3dcraftjig")).unwrap(), bytes);
}

#[test]
fn open_for_editing_makes_a_new_model_of_the_jig_as_it_is_set() {
    let dir = workdir("jig-use", "edit");
    let mut s = session_in(&dir);
    design(&mut s);
    publish(&mut s);
    run(&mut s, "jig.export", json!({"path": "plate.3dcraftjig"}));
    let mut u = session_in(&dir);
    assert!(craft3d_format::is_jig_path("A/Plate.3DCraftJig"));
    assert!(!craft3d_format::is_jig_path("plate.3dcraft"));
    assert!(!craft3d_format::is_jig_path("3dcraftjig"));
    run(&mut u, "file.open", json!({"path": "plate.3dcraftjig"}));
    run(&mut u, "param.set", json!({"values": {"w": 30}}));
    let edited = run(&mut u, "jig.edit", json!({}));
    assert_eq!(edited["document"], 1);
    assert_eq!(edited["fromJigInUse"], true);
    let status = u.document_status().unwrap();
    assert!(!status.jig_in_use && status.path.is_none());
    assert!((volume(&mut u) - 30.0 * 20.0 * 5.0).abs() < 1e-6);
    // The copy is edited freely; the Jig stays open, in use.
    run(&mut u, "feature.block", json!({"size": [1, 1, 1]}));
    run(&mut u, "file.activate", json!({"document": 0}));
    assert!(u.document_status().unwrap().jig_in_use);
    assert!(refused(&mut u, "jig.edit", json!({"x": 1})).contains("x"));
}
