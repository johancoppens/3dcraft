//! Pictures of text for the developer to look at (docs/design/text.md):
//! raised and engraved text on a plate, text along a circle on a ring, and
//! the single-stroke font, into `target/evidence/text/` and printed as
//! evidence lines (`craft3d_testkit::evidence`). Ignored: run with
//! `--ignored`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{built, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::json;
use std::path::PathBuf;

fn evidence_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/text");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn picture(s: &mut Session, dir: &std::path::Path, name: &str, view: &str) {
    let out = run(
        s,
        "view.render",
        json!({"path": name, "width": 800, "height": 500, "view": view}),
    );
    assert!(out.get("error").is_none(), "{out}");
    let bytes = std::fs::read(dir.join(name)).unwrap();
    craft3d_testkit::evidence::print(name, &bytes);
    std::fs::write(evidence_dir().join(name), bytes).unwrap();
}

#[test]
#[ignore = "evidence for the developer: run with --ignored"]
fn text_evidence() {
    let dir = workdir("text", "evidence");
    let mut s = session_in(&dir);
    // Raised and engraved text on a plate.
    new_document(&mut s, 1.0);
    run(
        &mut s,
        "variable.set",
        json!({"name": "partNo", "expr": "42"}),
    );
    built(
        &mut s,
        "feature.block",
        json!({"size": [90, 40, 6], "name": "Plate"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Raised", "face": "F1:face(1)"}),
    );
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "Raised", "text": "3DCraft {partNo}", "size": 9, "at": [45, 26], "align": "center", "anchor": "middle"}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Raised", "distance": 1.5, "join": "Plate", "name": "Emboss"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Cut", "face": "F1:face(1)"}),
    );
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "Cut", "text": "Engraved åéß&@", "font": "mono", "size": 6, "at": [45, 10], "align": "center", "anchor": "middle"}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Cut", "distance": -1, "cut": "Emboss", "name": "Engrave"}),
    );
    picture(&mut s, &dir, "plate-iso.png", "iso");
    picture(&mut s, &dir, "plate-top.png", "top");

    // Text along a circle on a ring.
    new_document(&mut s, 1.0);
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": 40, "height": 5, "name": "Disc"}),
    );
    run(
        &mut s,
        "sketch.create",
        json!({"name": "Around", "plane": "xy", "offset": 5}),
    );
    let circle = run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Around", "center": [0, 0], "radius": 30, "construction": true}),
    );
    run(
        &mut s,
        "sketch.textOnCurve",
        json!({"sketch": "Around", "curve": circle["entity"], "text": "TEXT ON A RING · 360°", "size": 5, "start": 47, "align": "center", "side": "right"}),
    );
    built(
        &mut s,
        "feature.extrude",
        json!({"sketch": "Around", "distance": 1.2, "join": "Disc", "name": "Ring text"}),
    );
    picture(&mut s, &dir, "ring-iso.png", "iso");
    picture(&mut s, &dir, "ring-top.png", "top");

    // The single-stroke font: every character it draws.
    new_document(&mut s, 1.0);
    run(&mut s, "sketch.create", json!({"name": "Strokes"}));
    run(
        &mut s,
        "sketch.text",
        json!({"sketch": "Strokes", "font": "stroke", "size": 6, "width": 200,
               "text": "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789 .,;:!?-+=/\\()[]{{}}'\"#%&*_<>@$^~|°€ àáâãäåèéêëìíîïòóôõöøùúûüýÿñçšžčřňÀÅÉÖØÜÑÇŠŽ"}),
    );
    // A sketch alone has no body to render: its SVG is the picture.
    run(
        &mut s,
        "file.export",
        json!({"path": "stroke.svg", "sketch": "Strokes"}),
    );
    let svg = std::fs::read(dir.join("stroke.svg")).unwrap();
    craft3d_testkit::evidence::print("stroke.svg", &svg);
    std::fs::write(evidence_dir().join("stroke.svg"), &svg).unwrap();
}

#[test]
#[ignore = "evidence for the developer: run with --ignored"]
fn wrapped_text_evidence() {
    let dir = workdir("text", "wrap-evidence");
    let mut s = session_in(&dir);
    for (shape, params) in [
        (
            "cylinder",
            json!({"radius": 20, "height": 40, "name": "Body"}),
        ),
        ("cone", json!({"radius": 30, "height": 60, "name": "Body"})),
    ] {
        for (how, depth, op) in [("raised", 1.0, "join"), ("engraved", -0.8, "cut")] {
            new_document(&mut s, 1.0);
            built(&mut s, &format!("feature.{shape}"), params.clone());
            run(&mut s, "sketch.create", json!({"name": "Label"}));
            run(
                &mut s,
                "sketch.text",
                json!({"sketch": "Label", "text": "3DCraft", "size": 6, "at": [0, 12], "align": "center"}),
            );
            let mut extrude =
                json!({"sketch": "Label", "wrap": "F1:face(0)", "distance": depth, "name": "Text"});
            extrude[op] = json!("Body");
            let made = run(&mut s, "feature.extrude", extrude.clone());
            if made["status"] != "ok" {
                // A boolean the kernel refuses is a finding: say it, and
                // show the wrapped body alone instead.
                println!("FINDING {shape} {how}: {}", made["error"]);
                run(&mut s, "edit.undo", json!({}));
                extrude.as_object_mut().unwrap().remove(op);
                built(&mut s, "feature.extrude", extrude);
            }
            for view in ["right", "iso"] {
                picture(&mut s, &dir, &format!("{shape}-{how}-{view}.png"), view);
                let _ = &made;
            }
        }
    }
}
