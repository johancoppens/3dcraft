//! Pictures of surface analysis for the developer to look at
//! (docs/design/surfacing.md, stage 1): zebra stripes and the curvature
//! map of a mouse dome made by `feature.surface.network`, and of a model
//! named by `CRAFT3D_ANALYSIS_MODEL` (the turbofan), into
//! `target/evidence/analysis/`. Ignored: run with `--ignored`; the pictures
//! are printed as evidence lines too (`craft3d_testkit::evidence`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::path::PathBuf;

fn mm(x: f64) -> String {
    format!("{x} mm")
}

fn spline(s: &mut Session, name: &str, plane: &str, offset: f64, through: Value) {
    built(
        s,
        "sketch.create",
        json!({"plane": plane, "offset": mm(offset), "name": name}),
    );
    run(
        s,
        "sketch.spline",
        json!({"sketch": name, "through": through}),
    );
}

/// A computer mouse's dome: three sections across (planes x = 0, 50, 100)
/// and three curves lengthwise (the two sides on the base, y = ±30, and
/// the ridge, y = 0), every one crossing every other.
fn mouse(s: &mut Session) {
    // yz: sketch (x, y) is world (offset, x, y).
    for (name, x, top) in [
        ("Back", 0.0, 15.0),
        ("Middle", 50.0, 38.0),
        ("Front", 100.0, 12.0),
    ] {
        spline(
            s,
            name,
            "yz",
            x,
            json!([
                [-30.0, 0.0],
                [-20.0, 0.62 * top],
                [0.0, top],
                [20.0, 0.62 * top],
                [30.0, 0.0]
            ]),
        );
    }
    // zx: sketch (x, y) is world (y, offset, x).
    spline(
        s,
        "Ridge",
        "zx",
        0.0,
        json!([
            [15.0, 0.0],
            [32.0, 25.0],
            [38.0, 50.0],
            [30.0, 75.0],
            [12.0, 100.0]
        ]),
    );
    for (name, y) in [("Left", -30.0), ("Right", 30.0)] {
        built(
            s,
            "sketch.create",
            json!({"plane": "zx", "offset": mm(y), "name": name}),
        );
        run(
            s,
            "sketch.line",
            json!({"sketch": name, "from": [0.0, 0.0], "to": [0.0, 100.0]}),
        );
    }
    built(
        s,
        "feature.surface.network",
        json!({"u": ["Back", "Middle", "Front"], "v": ["Left", "Ridge", "Right"], "name": "Dome"}),
    );
}

fn evidence_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/analysis");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn pictures(s: &mut Session, dir: &std::path::Path, stem: &str, views: &[&str]) {
    for view in views {
        for (kind, analysis) in [
            ("zebra", json!({"mode": "zebra", "stripes": 32})),
            ("highlight", json!({"mode": "highlight", "stripes": 12})),
            (
                "curvature",
                json!({"mode": "curvature", "curvature": "mean"}),
            ),
            (
                "gaussian",
                json!({"mode": "curvature", "curvature": "gaussian"}),
            ),
        ] {
            let name = format!("{stem}-{kind}-{view}.png");
            let out = run(
                s,
                "view.render",
                json!({"path": name, "width": 900, "height": 650, "view": view, "analysis": analysis}),
            );
            println!("{name}: {}", out["analysis"]);
            let bytes = std::fs::read(dir.join(&name)).unwrap();
            craft3d_testkit::evidence::print(&name, &bytes);
            std::fs::write(evidence_dir().join(&name), bytes).unwrap();
        }
    }
}

#[test]
#[ignore = "evidence for the developer: run with --ignored"]
fn analysis_evidence() {
    let dir = workdir("analysis", "evidence");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    mouse(&mut s);
    let face = body(&mut s, "Dome")["faceNames"][0]
        .as_str()
        .unwrap()
        .to_string();
    // Combs on the dome's edges, on its own picture.
    let out = run(
        &mut s,
        "view.render",
        json!({"path": "mouse-combs-iso.png", "width": 900, "height": 650,
               "analysis": {"mode": "curvature", "curvature": "max",
                            "combs": {"edges": true, "iso": [{"face": face, "u": 0.5}, {"face": face, "v": 0.5}], "density": 60}}}),
    );
    println!("combs: {}", out["analysis"]);
    std::fs::write(
        evidence_dir().join("mouse-combs-iso.png"),
        std::fs::read(dir.join("mouse-combs-iso.png")).unwrap(),
    )
    .unwrap();
    pictures(&mut s, &dir, "mouse", &["iso", "left", "top"]);
    if let Ok(model) = std::env::var("CRAFT3D_ANALYSIS_MODEL") {
        std::fs::copy(&model, dir.join("model.3dcraft")).unwrap();
        run(&mut s, "file.open", json!({"path": "model.3dcraft"}));
        pictures(&mut s, &dir, "model", &["iso", "front"]);
    }
}

/// The guide's fin held to its post (docs/guide/features.md,
/// "Curvature-continuous seams"), held in `kind`.
fn fin(s: &mut Session, kind: &str) {
    for (cmd, params) in [
        (
            "feature.cylinder",
            json!({"radius": "20 mm", "height": "30 mm", "name": "Post"}),
        ),
        ("sketch.create", json!({"name": "Seam", "plane": "zx"})),
        (
            "sketch.line",
            json!({"sketch": "Seam", "from": [0, 20], "to": [30, 20]}),
        ),
        (
            "sketch.create",
            json!({"name": "Far", "plane": "yz", "offset": "35 mm"}),
        ),
        (
            "sketch.line",
            json!({"sketch": "Far", "from": [25, 0], "to": [25, 30]}),
        ),
        ("sketch.create", json!({"name": "Bottom"})),
        (
            "sketch.spline",
            json!({"sketch": "Bottom", "through": [[20, 0], [23, 6.5], [27, 13], [35, 25]]}),
        ),
        ("sketch.create", json!({"name": "Top", "offset": "30 mm"})),
        (
            "sketch.spline",
            json!({"sketch": "Top", "through": [[20, 0], [23, 6.5], [27, 13], [35, 25]]}),
        ),
        (
            "feature.surface.network",
            json!({"u": ["Seam", "Far"], "v": ["Bottom", "Top"], "edges": {"uFirst": {"kind": kind, "face": "F1:face(0)"}}, "name": "Fin"}),
        ),
    ] {
        run(s, cmd, params);
    }
}

#[test]
#[ignore = "evidence for the developer: run with --ignored"]
fn fin_evidence() {
    for kind in ["curvature", "tangent"] {
        let dir = workdir("analysis", &format!("fin-{kind}"));
        let mut s = session_in(&dir);
        new_document(&mut s, 1.0);
        fin(&mut s, kind);
        let face = body(&mut s, "Fin")["faceNames"][0]
            .as_str()
            .unwrap()
            .to_string();
        for (what, analysis) in [
            ("zebra", json!({"mode": "zebra", "stripes": 24})),
            (
                "curvature",
                json!({"mode": "curvature", "curvature": "mean"}),
            ),
            (
                "combs",
                json!({"mode": "curvature", "curvature": "mean",
                       "combs": {"iso": [{"face": face, "u": 0.2}, {"face": face, "u": 0.5}, {"face": face, "u": 0.8}], "density": 120}}),
            ),
        ] {
            let name = format!("fin-{kind}-{what}.png");
            let out = run(
                &mut s,
                "view.render",
                json!({"path": name, "width": 1000, "height": 700, "view": "iso", "analysis": analysis}),
            );
            println!("{name}: {}", out["analysis"]);
            let bytes = std::fs::read(dir.join(&name)).unwrap();
            craft3d_testkit::evidence::print(&name, &bytes);
            std::fs::write(evidence_dir().join(&name), bytes).unwrap();
        }
    }
}
