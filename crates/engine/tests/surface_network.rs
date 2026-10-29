//! Network surfaces through the commands (docs/design/surfacing.md): a
//! Coons patch of two arcs and two lines is the quarter cylinder (its area
//! in closed form) and thickens into the quarter tube (its volume in
//! closed form); a patch held tangent to a cylinder's wall meets it with
//! one tangent plane; the document saves and reopens to the same bodies;
//! curves that do not cross fail the feature by name; bad parameters are
//! errors. At three scales.

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
use std::f64::consts::FRAC_PI_2;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(x: f64) -> String {
    format!("{x} mm")
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn near(actual: f64, expected: f64, relative: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs().max(f64::MIN_POSITIVE),
        "{what}: {actual}, expected {expected} (off by {:e})",
        (actual - expected) / expected
    );
}

/// A sketch on `plane` at `offset` holding one line.
fn line(s: &mut Session, name: &str, plane: &str, offset: f64, from: [f64; 2], to: [f64; 2]) {
    built(
        s,
        "sketch.create",
        json!({"plane": plane, "offset": mm(offset), "name": name}),
    );
    run(
        s,
        "sketch.line",
        json!({"sketch": name, "from": from, "to": to}),
    );
}

/// A sketch on the plane z = `z` holding the quarter arc of radius `r`
/// about the z axis from +x to +y.
fn quarter(s: &mut Session, name: &str, z: f64, r: f64) {
    built(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": mm(z), "name": name}),
    );
    run(
        s,
        "sketch.arc",
        json!({"sketch": name, "center": [0.0, 0.0], "start": [r, 0.0], "end": [0.0, r]}),
    );
}

#[test]
fn a_coons_patch_of_arcs_and_lines_is_the_quarter_cylinder_and_thickens() {
    for k in SCALES {
        let dir = workdir("surface-network", &format!("coons-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (r, h) = (20.0 * k, 30.0 * k);
        quarter(&mut s, "Low", 0.0, r);
        quarter(&mut s, "High", h, r);
        // zx: sketch (x, y) is world (y, 0, x); yz: world (0, x, y).
        line(&mut s, "Left", "zx", 0.0, [0.0, r], [h, r]);
        line(&mut s, "Right", "yz", 0.0, [r, 0.0], [r, h]);
        let made = built(
            &mut s,
            "feature.surface.coons",
            json!({"curves": ["Low", "Right", "High", "Left"], "name": "Wall"}),
        );
        assert!(number(&made["network"]["mismatch"]) <= 1e-9 * k, "{made}");
        let sheet = body(&mut s, "Wall");
        assert_eq!(sheet["kind"], "sheet", "{sheet}");
        assert_eq!(sheet["valid"], true, "{sheet}");
        near(
            number(&sheet["area"]),
            FRAC_PI_2 * r * h,
            1e-6,
            "the wall's area",
        );
        let faces = sheet["faceNames"].clone();
        assert_eq!(faces.as_array().unwrap().len(), 1, "{faces}");
        let face = faces[0].as_str().unwrap().to_string();
        assert!(face.contains(":side("), "{face}");

        // Thickened outwards: the quarter of a tube.
        let t = 2.0 * k;
        built(
            &mut s,
            "feature.thicken",
            json!({"faces": [face], "thickness": mm(t), "side": "outside", "name": "Tube"}),
        );
        let tube = body(&mut s, "Tube");
        assert_eq!(tube["valid"], true, "{tube}");
        near(
            number(&tube["volume"]),
            FRAC_PI_2 / 2.0 * ((r + t).powi(2) - r * r) * h,
            1e-6,
            "the tube's volume",
        );

        // Saved and opened again: the same bodies.
        run(
            &mut s,
            "file.save",
            json!({"path": "wall.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "wall.3dcraft"}));
        let again = body(&mut s, "Wall");
        near(
            number(&again["area"]),
            number(&sheet["area"]),
            1e-12,
            "reopened",
        );
    }
}

#[test]
fn a_patch_held_tangent_to_a_cylinder_meets_its_wall_with_one_tangent_plane() {
    for k in SCALES {
        let dir = workdir("surface-network", &format!("tangent-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (r, h) = (20.0 * k, 30.0 * k);
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": mm(r), "height": mm(h), "name": "Post"}),
        );
        let post = body(&mut s, "Post");
        let wall = post["faceNames"][0].as_str().unwrap().to_string();
        // The seam along the wall at angle 0, the far edge out in +x, +y.
        line(&mut s, "Seam", "zx", 0.0, [0.0, r], [h, r]);
        line(
            &mut s,
            "Far",
            "yz",
            r + 15.0 * k,
            [25.0 * k, 0.0],
            [25.0 * k, h],
        );
        for (name, z) in [("Bottom", 0.0), ("Top", h)] {
            built(
                &mut s,
                "sketch.create",
                json!({"plane": "xy", "offset": mm(z), "name": name}),
            );
            run(
                &mut s,
                "sketch.spline",
                json!({"sketch": name, "through": [[r, 0.0], [r + 6.0 * k, 12.0 * k], [r + 15.0 * k, 25.0 * k]]}),
            );
        }
        let made = built(
            &mut s,
            "feature.surface.network",
            json!({"u": ["Seam", "Far"], "v": ["Bottom", "Top"],
                   "edges": {"uFirst": {"kind": "tangent", "face": wall}}, "name": "Fin"}),
        );
        let angle = number(&made["network"]["tangency"]["uFirst"]);
        assert!(angle <= 1e-9, "{made}");
        let fin = body(&mut s, "Fin");
        assert_eq!(fin["valid"], true, "{fin}");
        // A weight of 2 is accepted and keeps the seam tangent.
        let edited = run(
            &mut s,
            "feature.edit",
            json!({"feature": "Fin", "edges": {"uFirst": {"kind": "tangent", "face": wall, "weight": 2}}}),
        );
        assert_eq!(edited["status"], "ok", "{edited}");
    }
}

#[test]
fn curves_that_do_not_cross_fail_the_feature_by_name() {
    let mut s = session_in(&workdir("surface-network", "apart"));
    new_document(&mut s, 1.0);
    quarter(&mut s, "Low", 0.0, 20.0);
    quarter(&mut s, "High", 30.0, 20.0);
    line(&mut s, "Left", "zx", 0.0, [0.0, 20.0], [30.0, 20.0]);
    // A line beside the arcs' ends, 1 mm away.
    line(&mut s, "Right", "yz", 1.0, [20.0, 0.0], [20.0, 30.0]);
    let why = failed(
        &mut s,
        "feature.surface.coons",
        json!({"curves": ["Low", "Right", "High", "Left"]}),
    );
    assert!(why.contains("do not cross"), "{why}");
}

#[test]
fn bad_network_parameters_are_errors() {
    let mut s = session_in(&workdir("surface-network", "bad"));
    new_document(&mut s, 1.0);
    quarter(&mut s, "Low", 0.0, 20.0);
    quarter(&mut s, "High", 30.0, 20.0);
    line(&mut s, "Left", "zx", 0.0, [0.0, 20.0], [30.0, 20.0]);
    line(&mut s, "Right", "yz", 0.0, [20.0, 0.0], [20.0, 30.0]);
    built(
        &mut s,
        "feature.block",
        json!({"size": [1, 1, 1], "name": "Box"}),
    );
    let net = "feature.surface.network";
    for params in [
        json!({}),
        json!({"u": ["Low"], "v": ["Low", "High"]}),
        json!({"u": "Low", "v": ["Low", "High"]}),
        json!({"u": ["Low", "Box"], "v": ["Low", "High"]}),
        json!({"u": ["Low", "High"], "v": ["Low", "High"]}),
        json!({"u": ["Low", "Low"], "v": ["High", "High"]}),
        json!({"u": ["Low", "High"], "v": [{"edges": []}, "High"]}),
        json!({"u": ["Low", "High"], "v": [{"edges": ["F1:face(0)"]}, {"edges": ["nonsense"]}]}),
        json!({"u": ["Low", "High"], "v": ["Left", "Right"], "edges": {"top": {}}}),
        json!({"u": ["Low", "High"], "v": ["Left", "Right"], "edges": {"uFirst": {"kind": "tangent"}}}),
        json!({"u": ["Low", "High"], "v": ["Left", "Right"], "edges": {"uFirst": {"kind": "glue", "face": "F3:face(0)"}}}),
        json!({"u": ["Low", "High"], "v": ["Left", "Right"], "edges": {"uFirst": {"kind": "tangent", "face": "F3:face(0)", "weight": -1}}}),
        json!({"u": ["Low", "High"], "v": ["Left", "Right"], "edges": {"uFirst": {"kind": "free", "face": "F3:face(0)"}}}),
        json!({"u": ["Low", "High"], "v": ["Left", "Right"], "edges": []}),
    ] {
        error(&mut s, net, params);
    }
    error(
        &mut s,
        "feature.surface.coons",
        json!({"curves": ["Low", "High"]}),
    );
    error(
        &mut s,
        "feature.surface.coons",
        json!({"curves": ["Low", "High", 3]}),
    );
}

#[test]
fn a_loft_of_two_arcs_along_two_guides_is_the_quarter_cylinder() {
    for k in SCALES {
        let mut s = session_in(&workdir("surface-network", &format!("guided-{k}")));
        new_document(&mut s, k);
        let (r, h) = (20.0 * k, 30.0 * k);
        quarter(&mut s, "Low", 0.0, r);
        quarter(&mut s, "High", h, r);
        line(&mut s, "Left", "zx", 0.0, [0.0, r], [h, r]);
        line(&mut s, "Right", "yz", 0.0, [r, 0.0], [r, h]);
        built(
            &mut s,
            "feature.loft",
            json!({"sections": ["Low", "High"], "guides": ["Left", "Right"], "name": "Wall"}),
        );
        let sheet = body(&mut s, "Wall");
        assert_eq!(sheet["kind"], "sheet", "{sheet}");
        near(
            number(&sheet["area"]),
            FRAC_PI_2 * r * h,
            1e-6,
            "the wall's area",
        );
        error(
            &mut s,
            "feature.loft",
            json!({"sections": ["Low", "High"], "guides": ["Left", "Right"], "ruled": true}),
        );
    }
}
