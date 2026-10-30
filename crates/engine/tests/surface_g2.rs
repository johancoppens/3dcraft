//! Network edges held in curvature (G2) and the proven continuity measure
//! through the commands (docs/design/surfacing.md): a fin held in
//! curvature to a cylinder's wall is G2 to it, held only tangent G1, and
//! `measure.continuity`, the assertion `continuity` and an output of it
//! tell the two apart; bad parameters are errors. At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, error, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::json;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn mm(x: f64) -> String {
    format!("{x} mm")
}

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

/// A post and the curves of a fin beside it; returns the wall's name.
fn fin_curves(s: &mut Session, k: f64) -> String {
    let (r, h) = (20.0 * k, 30.0 * k);
    built(
        s,
        "feature.cylinder",
        json!({"radius": mm(r), "height": mm(h), "name": "Post"}),
    );
    let post = body(s, "Post");
    let wall = post["faceNames"][0].as_str().unwrap().to_string();
    line(s, "Seam", "zx", 0.0, [0.0, r], [h, r]);
    line(s, "Far", "yz", r + 15.0 * k, [25.0 * k, 0.0], [25.0 * k, h]);
    for (name, z) in [("Bottom", 0.0), ("Top", h)] {
        built(
            s,
            "sketch.create",
            json!({"plane": "xy", "offset": mm(z), "name": name}),
        );
        run(
            s,
            "sketch.spline",
            json!({"sketch": name, "through": [[r, 0.0], [r + 3.0 * k, 6.5 * k], [r + 7.0 * k, 13.0 * k], [r + 15.0 * k, 25.0 * k]]}),
        );
    }
    wall
}

/// The fin's edge along its first u curve.
fn seam_edge(s: &mut Session, fin: &str) -> String {
    let all = edges_of(s, fin);
    all.iter()
        .find(|n| n.replace(' ', "").contains("startEdge(0,"))
        .cloned()
        .unwrap_or_else(|| panic!("no edge along the first u curve in {all:?}"))
}

fn edges_of(s: &mut Session, feature: &str) -> Vec<String> {
    let found = run(
        s,
        "select.query",
        json!({"kind": "edge", "where": {"madeBy": feature}}),
    );
    found["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["name"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn a_fin_held_in_curvature_is_g2_and_one_held_tangent_is_g1() {
    for k in SCALES {
        let dir = workdir("surface-g2", &format!("fin-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let wall = fin_curves(&mut s, k);
        let made = built(
            &mut s,
            "feature.surface.network",
            json!({"u": ["Seam", "Far"], "v": ["Bottom", "Top"],
                   "edges": {"uFirst": {"kind": "curvature", "face": wall}}, "name": "Smooth"}),
        );
        assert_eq!(
            made["network"]["continuity"]["uFirst"]["order"], "G2",
            "{made}"
        );
        let smooth = seam_edge(&mut s, "Smooth");
        let measured = run(
            &mut s,
            "measure.continuity",
            json!({"edge": smooth, "face": wall}),
        );
        assert_eq!(measured["order"], "G2", "{measured}");
        assert_eq!(measured["curvature"]["within"], true, "{measured}");
        // The same curves held only tangent.
        built(
            &mut s,
            "feature.surface.network",
            json!({"u": ["Seam", "Far"], "v": ["Bottom", "Top"],
                   "edges": {"uFirst": {"kind": "tangent", "face": wall}}, "name": "Plain"}),
        );
        let plain = seam_edge(&mut s, "Plain");
        let measured = run(
            &mut s,
            "measure.continuity",
            json!({"edge": plain, "face": wall}),
        );
        assert_eq!(measured["order"], "G1", "{measured}");
        assert_eq!(measured["curvature"]["within"], false, "{measured}");
        // As assertions: G2 passes for the one, fails for the other.
        let ok = run(
            &mut s,
            "assert.add",
            json!({"name": "smooth seam", "measure": "continuity", "edge": smooth, "face": wall, "min": 2}),
        );
        assert_eq!(ok["status"], "passed", "{ok}");
        let bad = run(
            &mut s,
            "assert.add",
            json!({"name": "plain seam", "measure": "continuity", "edge": plain, "face": wall, "min": 2}),
        );
        assert_eq!(bad["status"], "failed", "{bad}");
        // As an output: the order as a number.
        run(
            &mut s,
            "variable.measure",
            json!({"name": "order", "measure": "continuity", "edge": smooth, "face": wall}),
        );
        let outputs = run(&mut s, "output.list", json!({}));
        let order = outputs["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["name"] == "order")
            .unwrap();
        assert_eq!(order["value"].as_f64(), Some(2.0), "{outputs}");
    }
}

#[test]
fn bad_continuity_parameters_are_errors() {
    let dir = workdir("surface-g2", "bad");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let wall = fin_curves(&mut s, 1.0);
    error(&mut s, "measure.continuity", json!({}));
    error(&mut s, "measure.continuity", json!({"edge": 3}));
    error(&mut s, "measure.continuity", json!({"edge": wall.clone()}));
    error(
        &mut s,
        "measure.continuity",
        json!({"edge": "F99:startEdge(0,1)", "face": wall}),
    );
    error(
        &mut s,
        "measure.continuity",
        json!({"edge": "F99:startEdge(0,1)", "extra": 1}),
    );
    // The post's own edges are between two analytic faces.
    let rim = edges_of(&mut s, "Post").first().cloned().unwrap();
    error(&mut s, "measure.continuity", json!({"edge": rim}));
    // A curvature edge without a face.
    error(
        &mut s,
        "feature.surface.network",
        json!({"u": ["Seam", "Far"], "v": ["Bottom", "Top"],
               "edges": {"uFirst": {"kind": "curvature"}}}),
    );
}
