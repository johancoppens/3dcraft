//! `file.export` of the model as seen to DXF and SVG (`view_export.rs`):
//! a cube and a cylinder, whose edges, silhouettes and hidden lines are
//! known by hand, in orthographic and perspective views; hidden lines
//! left out or dashed; the same bytes twice; and what is refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::run;
use common::{session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::path::PathBuf;

fn cube(tag: &str) -> (Session, PathBuf) {
    let dir = workdir("view-export", tag);
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.block",
        json!({"size": ["10 mm", "10 mm", "10 mm"], "name": "Cube"}),
    );
    (s, dir)
}

fn cylinder(tag: &str) -> (Session, PathBuf) {
    let dir = workdir("view-export", tag);
    let mut s = session_in(&dir);
    run(&mut s, "file.new", json!({"units": "mm"}));
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": "5 mm", "height": "20 mm", "name": "Rod"}),
    );
    (s, dir)
}

/// The LINE entities of a DXF file on `layer`, as [x0, y0, x1, y1].
fn lines(dxf: &str, layer: &str) -> Vec<[f64; 4]> {
    let start = dxf.find("  2\nENTITIES\n").unwrap();
    let end = dxf.find("  2\nOBJECTS\n").unwrap();
    let mut out = Vec::new();
    for entity in dxf[start..end].split("  0\n").skip(1) {
        if !entity.starts_with("LINE\n") {
            continue;
        }
        let groups: Vec<&str> = entity.lines().collect();
        let value = |code: &str| -> &str {
            let at = groups.iter().position(|g| g.trim() == code).unwrap();
            groups[at + 1]
        };
        if value("8") != layer {
            continue;
        }
        let n = |code: &str| value(code).parse::<f64>().unwrap();
        out.push([n("10"), n("20"), n("11"), n("21")]);
    }
    out
}

fn length(l: &[f64; 4]) -> f64 {
    (l[2] - l[0]).hypot(l[3] - l[1])
}

fn export(s: &mut Session, dir: &std::path::Path, name: &str, params: Value) -> (Value, String) {
    let mut params = params;
    params["path"] = json!(name);
    let result = run(s, "file.export", params);
    (result, std::fs::read_to_string(dir.join(name)).unwrap())
}

#[test]
fn a_cube_from_the_front_is_a_square_at_scale() {
    let (mut s, dir) = cube("front");
    let (result, dxf) = export(
        &mut s,
        &dir,
        "front.dxf",
        json!({"view": "front", "hidden": "dashed"}),
    );
    assert_eq!(result["projection"], "orthographic");
    assert_eq!(result["unit"], "mm");
    // Four edges in a square; the back square lies under it and the edges
    // along the view are points, so nothing is hidden.
    let visible = lines(&dxf, "visible");
    assert_eq!(visible.len(), 4, "{visible:?}");
    assert!(lines(&dxf, "hidden").is_empty());
    for l in &visible {
        assert!((length(l) - 10.0).abs() < 1e-9, "{l:?}");
    }
    let bbox: Vec<f64> = result["bbox"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert!((bbox[2] - bbox[0] - 10.0).abs() < 1e-9 && (bbox[3] - bbox[1] - 10.0).abs() < 1e-9);
    assert!(dxf.contains("$INSUNITS\n 70\n4\n"));
}

#[test]
fn a_cube_in_iso_has_nine_visible_and_three_hidden_edges() {
    let (mut s, dir) = cube("iso");
    let (result, dxf) = export(
        &mut s,
        &dir,
        "iso.dxf",
        json!({"view": "iso", "hidden": "dashed"}),
    );
    assert_eq!(lines(&dxf, "visible").len(), 9, "{result}");
    assert_eq!(lines(&dxf, "hidden").len(), 3, "{result}");
    assert_eq!(result["visible"], 9);
    assert_eq!(result["hiddenLines"], 3);
    assert!(dxf.contains("DASHED"));
    // Every edge is drawn whole: the hidden ones too, each as long as the
    // edge seen at that angle.
    let (_, svg) = export(
        &mut s,
        &dir,
        "iso.svg",
        json!({"view": "iso", "hidden": "dashed"}),
    );
    assert!(svg.contains("id=\"layer-visible\""));
    assert!(svg.contains("id=\"layer-hidden\"") && svg.contains("stroke-dasharray"));
    // Left out: no hidden layer at all.
    let (omitted, svg) = export(&mut s, &dir, "omit.svg", json!({"view": "iso"}));
    assert!(!svg.contains("layer-hidden") && !svg.contains("dasharray"));
    assert_eq!(omitted["hidden"], "omit");
    assert_eq!(omitted["layers"], json!(["visible"]));
}

#[test]
fn a_cylinder_from_the_side_has_two_silhouette_lines() {
    let (mut s, dir) = cylinder("side");
    let (result, dxf) = export(
        &mut s,
        &dir,
        "side.dxf",
        json!({"view": "front", "hidden": "dashed"}),
    );
    let silhouettes = lines(&dxf, "silhouette");
    assert_eq!(silhouettes.len(), 2, "{result}");
    let mut xs: Vec<f64> = silhouettes.iter().map(|l| l[0]).collect();
    xs.sort_by(f64::total_cmp);
    assert!((xs[1] - xs[0] - 10.0).abs() < 1e-6, "{xs:?}");
    for l in &silhouettes {
        assert!((length(l) - 20.0).abs() < 1e-6, "{l:?}");
        assert!((l[0] - l[2]).abs() < 1e-9, "vertical: {l:?}");
    }
    // The two circles, edge on: a line each, as wide as the cylinder.
    let visible = lines(&dxf, "visible");
    assert_eq!(visible.len(), 2, "{visible:?}");
    for l in &visible {
        assert!((length(l) - 10.0).abs() < 1e-6, "{l:?}");
    }
    assert!(lines(&dxf, "hidden").is_empty());
    // From the top: the circle, and no silhouette.
    let (top, _) = export(&mut s, &dir, "top.dxf", json!({"view": "top"}));
    assert_eq!(top["silhouette"], 0);
    // In iso: two silhouettes, the far half of the bottom circle hidden.
    let (iso, dxf) = export(
        &mut s,
        &dir,
        "iso.dxf",
        json!({"view": "iso", "hidden": "dashed"}),
    );
    let silhouettes = lines(&dxf, "silhouette");
    assert_eq!(silhouettes.len(), 2, "{iso}");
    assert!(!lines(&dxf, "hidden").is_empty(), "{iso}");
}

#[test]
fn perspective_is_not_to_scale_and_says_so() {
    let (mut s, dir) = cube("perspective");
    let (ortho, _) = export(&mut s, &dir, "o.svg", json!({"view": "iso"}));
    let (persp, _) = export(
        &mut s,
        &dir,
        "p.svg",
        json!({"view": "iso", "projection": "perspective", "hidden": "dashed"}),
    );
    assert_eq!(persp["projection"], "perspective");
    assert!(persp["scale"].as_str().unwrap().contains("perspective"));
    assert!(ortho["scale"].as_str().unwrap().starts_with("1:1"));
    assert_eq!(persp["visible"], 9);
    assert_eq!(persp["hiddenLines"], 3);
    assert_ne!(ortho["bbox"], persp["bbox"]);
    // From the front in perspective the back edges show inside the front
    // square: smaller, and hidden.
    let (front, dxf) = export(
        &mut s,
        &dir,
        "pf.dxf",
        json!({"view": "front", "projection": "perspective", "hidden": "dashed"}),
    );
    assert_eq!(lines(&dxf, "visible").len(), 4, "{front}");
    assert_eq!(lines(&dxf, "hidden").len(), 8, "{front}");
}

#[test]
fn the_same_document_gives_the_same_bytes_and_hidden_bodies_are_left_out() {
    let (mut s, dir) = cylinder("same");
    run(
        &mut s,
        "feature.cylinder",
        json!({"radius": "2 mm", "height": "30 mm", "position": ["0 mm", "0 mm", "-5 mm"], "name": "Bore"}),
    );
    run(
        &mut s,
        "feature.boolean",
        json!({"op": "subtract", "target": "Rod", "tool": "Bore", "name": "Tube"}),
    );
    let (a, one) = export(&mut s, &dir, "a.dxf", json!({"hidden": "dashed"}));
    let (_, two) = export(&mut s, &dir, "b.dxf", json!({"hidden": "dashed"}));
    assert_eq!(one, two);
    // The outside's two silhouettes; the bore's are hidden.
    assert_eq!(a["silhouette"], 2, "{a}");
    assert!(a["hiddenLines"].as_u64().unwrap() > 0, "{a}");
    run(
        &mut s,
        "feature.visible",
        json!({"feature": "Tube", "visible": false}),
    );
    let error = s
        .execute("file.export", &json!({"path": "c.svg"}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("no bodies to draw"), "{error}");
}

#[test]
fn the_window_camera_is_the_current_view() {
    let (mut s, dir) = cube("current");
    // A camera from the front, as the window would say it.
    s.set_window_camera(Some(craft3d_render::Camera {
        eye: craft3d_math::Vec3::new(5.0, -100.0, 5.0),
        target: craft3d_math::Vec3::new(5.0, 5.0, 5.0),
        up: craft3d_math::Vec3::Z,
        projection: craft3d_render::Projection::Perspective { fov_y_deg: 30.0 },
    }));
    let (current, dxf) = export(&mut s, &dir, "c.dxf", json!({}));
    assert_eq!(current["view"], "current");
    assert_eq!(current["projection"], "orthographic");
    assert_eq!(lines(&dxf, "visible").len(), 4);
    s.set_window_camera(None);
    let (fallback, _) = export(&mut s, &dir, "d.dxf", json!({"view": "current"}));
    assert_eq!(fallback["view"], "iso");
}

#[test]
fn bad_parameters_are_refused_with_the_reason() {
    let (mut s, _dir) = cube("bad");
    for (params, words) in [
        (json!({"path": "a.svg", "view": "sideways"}), "unknown view"),
        (json!({"path": "a.svg", "view": 3}), "view"),
        (
            json!({"path": "a.svg", "projection": "fisheye"}),
            "unknown projection",
        ),
        (
            json!({"path": "a.svg", "hidden": "maybe"}),
            "unknown `hidden`",
        ),
        (json!({"path": "a.dxf", "xAxis": [1, 0, 0]}), "`xAxis`"),
        (json!({"path": "a.dxf", "gap": 3}), "`gap`"),
        (json!({"path": "a.stl", "view": "front"}), "`view`"),
        (json!({"path": "a.step", "hidden": "dashed"}), "`hidden`"),
        (json!({"path": "a.svg", "tolerance": -1}), "tolerance"),
        (
            json!({"path": "a.svg", "face": "F1:face(0)", "projection": "perspective"}),
            "`projection`",
        ),
    ] {
        let error = s
            .execute("file.export", &params)
            .expect_err(&params.to_string())
            .to_string();
        assert!(error.contains(words), "{params}: {error}");
        assert!(!error.contains("internal error"), "{params}: {error}");
    }
    // Without a document.
    let mut empty = common::session();
    assert!(
        empty
            .execute("file.export", &json!({"path": "a.svg"}))
            .is_err()
    );
}
