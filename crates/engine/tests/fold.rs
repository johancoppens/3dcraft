//! Folds through the commands (docs/design/origami.md): a crease pattern
//! on a sketch's sheet folded rigidly by angles that follow a variable,
//! solved creases, the folded sheet measured and checked deep, saved and
//! reopened, its names, Kawasaki and Maekawa per vertex with the honest
//! note, the crease pattern exported to SVG and DXF with mountain and
//! valley styles; what a pattern cannot do fails the feature with the
//! reason, bad parameters are errors. At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{
    assert_close, body, built, deep, error, failed, new_document, run, session_in, workdir,
};
use craft3d_engine::Session;
use serde_json::{Value, json};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

/// A sketch with a square of side `2 c` about the origin, and the lines
/// given (from, to), construction lines when `construction`; returns the
/// lines' entity ids and the ids of their end points.
fn square_lines(
    s: &mut Session,
    name: &str,
    c: f64,
    lines: &[([f64; 2], [f64; 2])],
    construction: bool,
) -> Vec<(String, String)> {
    run(s, "sketch.create", json!({"plane": "xy", "name": name}));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": name, "points": [[-c, -c], [c, -c], [c, c], [-c, c]], "closed": true}),
    );
    lines
        .iter()
        .map(|(a, b)| {
            let made = run(
                s,
                "sketch.line",
                json!({"sketch": name, "from": a, "to": b, "construction": construction}),
            );
            (
                made["entity"].as_str().unwrap().to_string(),
                made["end"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn square_with(s: &mut Session, name: &str, c: f64, lines: &[([f64; 2], [f64; 2])]) -> Vec<String> {
    square_lines(s, name, c, lines, false)
        .into_iter()
        .map(|(l, _)| l)
        .collect()
}

fn piece_angles(inspect: &Value, line: &str) -> Vec<f64> {
    inspect["creases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["line"] == line)
        .unwrap_or_else(|| panic!("no crease {line} in {inspect}"))["pieces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| number(&p["angle"]))
        .collect()
}

#[test]
fn a_single_fold_follows_its_variable_and_keeps_the_area() {
    for k in SCALES {
        let dir = workdir("fold", &format!("single-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let c = 10.0 * k;
        run(
            &mut s,
            "variable.set",
            json!({"name": "a", "expr": "30 deg"}),
        );
        let lines = square_with(&mut s, "Paper", c, &[([0.0, -c], [0.0, c])]);
        built(
            &mut s,
            "feature.fold",
            json!({"sketch": "Paper", "name": "Folded", "fixed": [-c / 2.0, 0.0],
                   "creases": [{"line": lines[0], "kind": "valley", "angle": "a"}]}),
        );
        for degrees in [30.0f64, 90.0, 170.0] {
            run(
                &mut s,
                "variable.set",
                json!({"name": "a", "expr": format!("{degrees} deg")}),
            );
            let found = body(&mut s, "Folded");
            assert_eq!(found["kind"], "sheet", "{found}");
            assert_eq!(found["valid"], true);
            assert_close(&found["area"], 4.0 * c * c, "area");
            // The far edge at (c cos a, ., c sin a).
            let t = degrees.to_radians();
            assert_close(&found["bbox"][1][2], c * t.sin(), "height");
            let inspect = run(&mut s, "fold.inspect", json!({"fold": "Folded"}));
            assert_eq!(inspect["ok"], true, "{inspect}");
            let a = piece_angles(&inspect, &lines[0]);
            assert!((a[0] - degrees).abs() < 1e-9, "{inspect}");
            deep(&mut s);
        }
        // Names: facets as start caps, after the curves of their outlines.
        let found = body(&mut s, "Folded");
        let names: Vec<&str> = found["faceNames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(
            names.iter().all(|n| n.starts_with("F2:startCap(")),
            "{names:?}"
        );
        // Saved and reopened, it folds the same.
        run(&mut s, "file.save", json!({"path": "fold.3dcraft"}));
        let before = body(&mut s, "Folded");
        let mut t = session_in(&dir);
        run(&mut t, "file.open", json!({"path": "fold.3dcraft"}));
        let after = body(&mut t, "Folded");
        assert_eq!(before["bbox"], after["bbox"]);
        assert_eq!(before["faceNames"], after["faceNames"]);
    }
}

#[test]
fn a_degree_four_vertex_solves_its_other_creases_by_the_closed_form() {
    let (alpha, beta) = (70f64.to_radians(), 50f64.to_radians());
    for k in SCALES {
        let dir = workdir("fold", &format!("degree4-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let c = 10.0 * k;
        let ray = |a: f64| {
            let t = c / a.cos().abs().max(a.sin().abs());
            [t * a.cos(), t * a.sin()]
        };
        let dirs = [0.0, alpha, alpha + beta, PI + beta];
        let made = square_lines(
            &mut s,
            "Paper",
            c,
            &dirs.map(|a| ([0.0, 0.0], ray(a))),
            false,
        );
        let lines: Vec<String> = made.iter().map(|(l, _)| l.clone()).collect();
        run(
            &mut s,
            "variable.set",
            json!({"name": "rho", "expr": "40 deg"}),
        );
        built(
            &mut s,
            "feature.fold",
            json!({"sketch": "Paper", "name": "Vertex", "creases": [
                {"line": lines[0], "kind": "valley", "angle": "rho"},
                {"line": lines[1], "kind": "mountain"},
                {"line": lines[2], "kind": "valley"},
                {"line": lines[3], "kind": "valley"}]}),
        );
        // From the flat sheet (a driving angle of 0 is valid: the flat
        // sheet), back to it, and on.
        for degrees in [0.0f64, 10.0, 40.0, 0.0, 120.0] {
            run(
                &mut s,
                "variable.set",
                json!({"name": "rho", "expr": format!("{degrees} deg")}),
            );
            let inspect = run(&mut s, "fold.inspect", json!({"fold": "Vertex"}));
            assert_eq!(inspect["ok"], true, "{inspect}");
            let r1 = degrees.to_radians();
            let p = ((alpha + beta) / 2.0).cos() / ((alpha - beta) / 2.0).cos();
            let r2 = 2.0 * (-p * (r1 / 2.0).tan()).atan();
            let want = [r1, r2, r1, -r2].map(f64::to_degrees);
            for (line, w) in lines.iter().zip(want) {
                let got = piece_angles(&inspect, line)[0];
                assert!((got - w).abs() < 1e-7, "{degrees}: {line} {got} vs {w}");
            }
            assert_close(&body(&mut s, "Vertex")["area"], 4.0 * c * c, "area");
        }
        // Kawasaki and Maekawa hold here, and the result says what that
        // does not prove.
        let check = run(&mut s, "fold.check", json!({"fold": "Vertex"}));
        assert_eq!(check["interior"], 1, "{check}");
        assert_eq!(check["kawasakiFails"], 0);
        assert_eq!(check["maekawaFails"], 0);
        let note = check["note"].as_str().unwrap();
        assert!(
            note.contains("not sufficient")
                && note.contains("NP-hard")
                && note.contains("Bern and Hayes"),
            "{note}"
        );
        let v = &check["vertices"][0];
        assert_close(&v["kawasaki"]["sums"][0], 180.0, "sum");
        assert_eq!(v["maekawa"]["mountains"], 1);
        assert_eq!(v["maekawa"]["valleys"], 3);
        // Every crease a valley: Maekawa fails at the vertex, by 4, and
        // the fold cannot hold.
        run(
            &mut s,
            "fold.edit",
            json!({"fold": "Vertex", "set": [{"line": lines[1], "kind": "valley"}]}),
        );
        let check = run(&mut s, "fold.check", json!({"fold": "Vertex"}));
        assert_eq!(check["maekawaFails"], 1, "{check}");
        assert_eq!(check["vertices"][0]["maekawa"]["difference"], 4);
        assert!(
            check["message"]
                .as_str()
                .unwrap()
                .contains("does not fold flat")
        );
        let inspect = run(&mut s, "fold.inspect", json!({"fold": "Vertex"}));
        assert_eq!(inspect["ok"], false, "{inspect}");
        run(&mut s, "edit.undo", json!({}));
        // Move the vertex's third crease by 3 degrees (a sketch edit):
        // Kawasaki fails there by 3 degrees.
        let moved = ray(alpha + beta + 3f64.to_radians());
        run(
            &mut s,
            "sketch.edit",
            json!({"sketch": "Paper", "entity": made[2].1, "at": moved}),
        );
        let check = run(&mut s, "fold.check", json!({"fold": "Vertex"}));
        assert_eq!(check["kawasakiFails"], 1, "{check}");
        assert_close(
            &json!(number(&check["vertices"][0]["kawasaki"]["off"]).abs()),
            3.0,
            "off",
        );
    }
}

#[test]
fn the_crease_pattern_exports_with_mountain_and_valley_styles() {
    let dir = workdir("fold", "export");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let c = 10.0;
    let lines = square_with(
        &mut s,
        "Paper",
        c,
        &[
            ([-c, 0.0], [c, 0.0]),
            ([-c, -c], [c, c]),
            ([c, -c], [-c, c]),
        ],
    );
    built(
        &mut s,
        "feature.fold",
        json!({"sketch": "Paper", "name": "Waterbomb", "creases": [
            {"line": lines[0], "kind": "mountain", "angle": "36.03846874550981 deg"},
            {"line": lines[1], "kind": "valley", "angle": "49.40777113928209 deg"},
            {"line": lines[2], "kind": "valley"}]}),
    );
    let check = run(&mut s, "fold.check", json!({"fold": "Waterbomb"}));
    assert_eq!(
        (
            check["kawasakiFails"].clone(),
            check["maekawaFails"].clone()
        ),
        (json!(0), json!(0)),
        "{check}"
    );
    let svg = run(
        &mut s,
        "file.export",
        json!({"path": "waterbomb.svg", "fold": "Waterbomb"}),
    );
    assert_eq!(
        (
            svg["mountains"].clone(),
            svg["valleys"].clone(),
            svg["cuts"].clone()
        ),
        (json!(1), json!(2), json!(0)),
        "{svg}"
    );
    assert_close(
        &svg["length"],
        8.0 * c + 2.0 * c + 2.0 * 8f64.sqrt() * c,
        "length",
    );
    let text = std::fs::read_to_string(dir.join("waterbomb.svg")).unwrap();
    assert!(
        text.contains("id=\"layer-boundary\"")
            && text.contains("id=\"layer-mountain\"")
            && text.contains("id=\"layer-valley\""),
        "{text}"
    );
    assert!(text.contains("stroke=\"#ff0000\"") && text.contains("stroke=\"#0000ff\""));
    assert_eq!(text.matches("stroke-dasharray").count(), 2);
    run(
        &mut s,
        "file.export",
        json!({"path": "waterbomb.dxf", "fold": "Waterbomb"}),
    );
    let text = std::fs::read_to_string(dir.join("waterbomb.dxf")).unwrap();
    assert!(
        text.contains("DASHDOT") && text.contains("DASHED"),
        "{text}"
    );
    assert!(
        text.contains("\nmountain\n")
            && text.contains("\nvalley\n")
            && text.contains("\nboundary\n")
    );
    // Not with the keys of a sketch's or a face's outline, not to STEP.
    let e = error(
        &mut s,
        "file.export",
        json!({"path": "w.svg", "fold": "Waterbomb", "construction": true}),
    );
    assert!(e.contains("construction"), "{e}");
    let e = error(
        &mut s,
        "file.export",
        json!({"path": "w.step", "fold": "Waterbomb"}),
    );
    assert!(e.contains("fold"), "{e}");
    let e = error(
        &mut s,
        "file.export",
        json!({"path": "w.svg", "fold": "Waterbomb", "sketch": "Paper"}),
    );
    assert!(e.contains("not"), "{e}");
}

#[test]
fn what_a_fold_cannot_do_fails_the_feature_and_bad_parameters_are_errors() {
    let dir = workdir("fold", "refusals");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let c = 10.0;
    let lines: Vec<String> = square_lines(
        &mut s,
        "Paper",
        c,
        &[([0.0, -c], [0.0, 5.0]), ([-c, 0.0], [c, 0.0])],
        true,
    )
    .into_iter()
    .map(|(l, _)| l)
    .collect();
    // A crease that ends inside a facet.
    let why = failed(
        &mut s,
        "feature.fold",
        json!({"sketch": "Paper", "name": "Short", "creases": [{"line": lines[0], "kind": "valley", "angle": 30}]}),
    );
    assert!(why.contains("ends inside a facet"), "{why}");
    run(&mut s, "feature.delete", json!({"feature": "Short"}));
    // A crease on no loop with no angle: nothing determines it.
    let why = failed(
        &mut s,
        "feature.fold",
        json!({"sketch": "Paper", "name": "Free", "creases": [
            {"line": lines[1], "kind": "valley"}]}),
    );
    assert!(why.contains("nothing determines"), "{why}");
    // Folded flat: the halves lie on each other.
    run(
        &mut s,
        "fold.edit",
        json!({"fold": "Free", "creases": [{"line": lines[1], "kind": "valley", "angle": 180}]}),
    );
    let inspect = run(&mut s, "fold.inspect", json!({"fold": "Free"}));
    assert_eq!(inspect["ok"], false, "{inspect}");
    assert!(
        inspect["error"]
            .as_str()
            .unwrap()
            .contains("lie on each other"),
        "{inspect}"
    );
    // An angle past 180 degrees.
    run(
        &mut s,
        "fold.edit",
        json!({"fold": "Free", "set": [{"line": lines[1], "angle": "200 deg"}]}),
    );
    let e = error(&mut s, "fold.inspect", json!({"fold": "Free"}));
    assert!(e.contains("180"), "{e}");
    // Bad parameters.
    for (params, word) in [
        (json!({"sketch": "Paper", "creases": []}), "at least one"),
        (json!({"sketch": "Paper"}), "creases"),
        (
            json!({"sketch": "Paper", "creases": [{"line": lines[1], "kind": "pleat"}]}),
            "pleat",
        ),
        (
            json!({"sketch": "Paper", "creases": [{"line": lines[1]}]}),
            "kind",
        ),
        (
            json!({"sketch": "Paper", "creases": [{"line": "e999", "kind": "valley"}]}),
            "e999",
        ),
        (
            json!({"sketch": "Paper", "creases": [{"line": lines[1], "kind": "cut", "angle": 3}]}),
            "cut",
        ),
        (
            json!({"sketch": "Paper", "creases": [{"line": lines[1], "kind": "valley"}, {"line": lines[1], "kind": "valley"}]}),
            "twice",
        ),
        (
            json!({"sketch": "Paper", "creases": [{"line": lines[1], "kind": "valley", "width": 2}]}),
            "width",
        ),
        (
            json!({"sketch": "Paper", "creases": [{"line": lines[1], "kind": "valley"}], "fixed": [1]}),
            "fixed",
        ),
        (
            json!({"sketch": "Free", "creases": [{"line": lines[1], "kind": "valley"}]}),
            "not a sketch",
        ),
    ] {
        let e = error(&mut s, "feature.fold", params.clone());
        assert!(e.contains(word), "{params}: {e}");
    }
    let e = error(&mut s, "fold.check", json!({"fold": "Paper"}));
    assert!(e.contains("not a fold"), "{e}");
    let e = error(&mut s, "fold.edit", json!({"fold": "Free"}));
    assert!(e.contains("give"), "{e}");
    let e = error(
        &mut s,
        "fold.edit",
        json!({"fold": "Free", "remove": ["e999"]}),
    );
    assert!(e.contains("not a crease"), "{e}");
}
