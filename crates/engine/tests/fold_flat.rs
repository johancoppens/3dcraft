//! Folds folded flat through the commands (docs/design/origami.md ›
//! *Folded flat*): a card and a waterbomb base at 180 degrees, their layer
//! order in fold.inspect, the body exactly in the sketch's plane (area
//! kept, the deep check clean) while the picture lifts the layers apart;
//! what does not fold flat fails with the reason; creases that are copies
//! a sketch pattern made (`e11[1]`, `e11[*]` with alternating kinds); and
//! the fold measures of assertions. At three scales.

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

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

/// A square of side `2 c` about the origin and the construction lines
/// given; returns the lines' entity ids.
fn square(s: &mut Session, name: &str, c: f64, lines: &[([f64; 2], [f64; 2])]) -> Vec<String> {
    run(s, "sketch.create", json!({"plane": "xy", "name": name}));
    run(
        s,
        "sketch.polyline",
        json!({"sketch": name, "points": [[-c, -c], [c, -c], [c, c], [-c, c]], "closed": true}),
    );
    lines
        .iter()
        .map(|(a, b)| {
            run(
                s,
                "sketch.line",
                json!({"sketch": name, "from": a, "to": b, "construction": true}),
            )["entity"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

fn scene_z_extent(s: &mut Session) -> f64 {
    let scene = s.scene(None).unwrap().unwrap();
    let (lo, hi) = scene.mesh.bbox().corners().unwrap();
    hi.z - lo.z
}

#[test]
fn a_card_slid_to_180_degrees_folds_flat_in_two_layers() {
    for k in SCALES {
        let dir = workdir("fold_flat", &format!("card-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let c = 10.0 * k;
        run(
            &mut s,
            "variable.set",
            json!({"name": "a", "expr": "170 deg"}),
        );
        let lines = square(&mut s, "Paper", c, &[([0.0, -c], [0.0, c])]);
        built(
            &mut s,
            "feature.fold",
            json!({"sketch": "Paper", "name": "Card", "fixed": [-c / 2.0, 0.0],
                   "creases": [{"line": lines[0], "kind": "valley", "angle": "a"}]}),
        );
        let inspect = run(&mut s, "fold.inspect", json!({"fold": "Card"}));
        assert_eq!(inspect["flat"], false, "{inspect}");
        run(
            &mut s,
            "variable.set",
            json!({"name": "a", "expr": "180 deg"}),
        );
        let found = body(&mut s, "Card");
        assert_eq!(found["valid"], true, "{found}");
        assert_close(&found["area"], 4.0 * c * c, "area");
        // Exactly in the sketch's plane.
        assert_eq!(found["bbox"][0][2].as_f64().unwrap(), 0.0, "{found}");
        assert_eq!(found["bbox"][1][2].as_f64().unwrap(), 0.0, "{found}");
        let inspect = run(&mut s, "fold.inspect", json!({"fold": "Card"}));
        assert_eq!(inspect["ok"], true, "{inspect}");
        assert_eq!(inspect["flat"], true);
        let fixed = inspect["fixed"].as_u64().unwrap() as usize;
        let layers = &inspect["layers"];
        assert_eq!(layers["count"], 2, "{inspect}");
        // A valley: the other half on top of the fixed one.
        assert_eq!(layers["facetLayers"][fixed], 0, "{inspect}");
        assert_eq!(inspect["facets"][1 - fixed]["faceDown"], true);
        let angle = inspect["creases"][0]["pieces"][0]["angle"]
            .as_f64()
            .unwrap();
        assert!((angle - 180.0).abs() < 1e-9, "{inspect}");
        deep(&mut s);
        // The picture lifts the top layer, the body does not move.
        let z = scene_z_extent(&mut s);
        assert!(z > 1e-4 * c && z < 0.05 * c, "{z}");
        // Saved and reopened, the same.
        run(&mut s, "file.save", json!({"path": "card.3dcraft"}));
        let mut t = session_in(&dir);
        run(&mut t, "file.open", json!({"path": "card.3dcraft"}));
        assert_eq!(body(&mut t, "Card")["bbox"], found["bbox"]);
    }
}

#[test]
fn the_waterbomb_base_folds_flat_and_a_wrong_assignment_does_not() {
    for k in SCALES {
        let dir = workdir("fold_flat", &format!("waterbomb-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let c = 50.0 * k;
        let lines = square(
            &mut s,
            "Square",
            c,
            &[
                ([-c, 0.0], [c, 0.0]),
                ([-c, -c], [c, c]),
                ([c, -c], [-c, c]),
            ],
        );
        // One angle given 180 degrees: the solved creases fold flat by
        // their kinds.
        built(
            &mut s,
            "feature.fold",
            json!({"sketch": "Square", "name": "Waterbomb", "creases": [
                {"line": lines[0], "kind": "mountain", "angle": "180 deg"},
                {"line": lines[1], "kind": "valley"},
                {"line": lines[2], "kind": "valley"}]}),
        );
        let inspect = run(&mut s, "fold.inspect", json!({"fold": "Waterbomb"}));
        assert_eq!(inspect["flat"], true, "{inspect}");
        assert_eq!(inspect["facets"].as_array().unwrap().len(), 6);
        assert!(
            inspect["layers"]["count"].as_u64().unwrap() >= 3,
            "{inspect}"
        );
        assert_eq!(inspect["layers"]["cyclic"], false);
        assert_close(&body(&mut s, "Waterbomb")["area"], 4.0 * c * c, "area");
        deep(&mut s);
        let check = run(&mut s, "fold.check", json!({"fold": "Waterbomb"}));
        assert_eq!(check["kawasakiFails"], 0);
        // All valleys: Maekawa fails, and no order of the layers exists.
        let why = failed(
            &mut s,
            "fold.edit",
            json!({"fold": "Waterbomb", "set": [{"line": lines[0], "kind": "valley"}]}),
        );
        assert!(why.contains("cannot fold flat"), "{why}");
        // 180 beside 90 degrees is refused with the reason.
        let why = failed(
            &mut s,
            "fold.edit",
            json!({"fold": "Waterbomb", "set": [
                {"line": lines[0], "kind": "mountain"},
                {"line": lines[1], "angle": "90 deg"}]}),
        );
        assert!(why.contains("folds by 180 degrees while"), "{why}");
    }
}

#[test]
fn a_crease_patterned_in_the_sketch_folds_every_copy() {
    for k in SCALES {
        let dir = workdir("fold_flat", &format!("pleat-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let a = 30.0 * k;
        run(&mut s, "variable.set", json!({"name": "n", "expr": "4"}));
        run(
            &mut s,
            "variable.set",
            json!({"name": "a", "expr": format!("{a} mm")}),
        );
        run(
            &mut s,
            "variable.set",
            json!({"name": "fold", "expr": "40 deg"}),
        );
        run(
            &mut s,
            "sketch.create",
            json!({"plane": "xy", "name": "Strip"}),
        );
        run(
            &mut s,
            "sketch.polyline",
            json!({"sketch": "Strip", "points": [["0 mm", "0 mm"], ["n * a", "0 mm"], ["n * a", "a"], ["0 mm", "a"]], "closed": true}),
        );
        let line = run(
            &mut s,
            "sketch.line",
            json!({"sketch": "Strip", "from": ["a", "0 mm"], "to": ["a", "a"], "construction": true}),
        )["entity"]
            .as_str()
            .unwrap()
            .to_string();
        run(
            &mut s,
            "sketch.pattern",
            json!({"sketch": "Strip", "entities": [line], "count": "n - 1", "step": ["a", "0 mm"]}),
        );
        // The line alone would drop its copies: refused, naming them.
        let why = failed(
            &mut s,
            "feature.fold",
            json!({"sketch": "Strip", "name": "Alone", "creases": [{"line": line, "kind": "valley", "angle": "fold"}]}),
        );
        assert!(why.contains(&format!("{line}[1]")), "{why}");
        // Every copy, kinds alternating: an accordion.
        let all = format!("{line}[*]");
        built(
            &mut s,
            "feature.fold",
            json!({"sketch": "Strip", "name": "Pleats", "fixed": [a / 2.0, a / 2.0], "creases": [
                {"line": all, "kind": "valley", "angle": "fold", "alternate": true}]}),
        );
        let inspect = run(&mut s, "fold.inspect", json!({"fold": "Pleats"}));
        assert_eq!(inspect["ok"], true, "{inspect}");
        assert_eq!(inspect["facets"].as_array().unwrap().len(), 4, "{inspect}");
        let kinds: Vec<(String, String)> = inspect["creases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["line"].as_str().unwrap().to_string(),
                    c["kind"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                (line.clone(), "valley".to_string()),
                (format!("{line}[1]"), "mountain".to_string()),
                (format!("{line}[2]"), "valley".to_string()),
            ]
        );
        // One copy set apart wins over [*].
        built(
            &mut s,
            "fold.edit",
            json!({"fold": "Pleats", "set": [{"line": format!("{line}[1]"), "kind": "valley", "angle": "fold"}]}),
        );
        let inspect = run(&mut s, "fold.inspect", json!({"fold": "Pleats"}));
        assert_eq!(inspect["creases"][1]["kind"], "valley", "{inspect}");
        built(
            &mut s,
            "fold.edit",
            json!({"fold": "Pleats", "remove": [format!("{line}[1]")]}),
        );
        // Folded flat by the variable: four layers.
        run(
            &mut s,
            "variable.set",
            json!({"name": "fold", "expr": "180 deg"}),
        );
        let inspect = run(&mut s, "fold.inspect", json!({"fold": "Pleats"}));
        assert_eq!(inspect["flat"], true, "{inspect}");
        assert_eq!(inspect["layers"]["count"], 4, "{inspect}");
        assert_close(&body(&mut s, "Pleats")["area"], 4.0 * a * a, "area");
        // More copies by the variable, still every one a crease.
        run(&mut s, "variable.set", json!({"name": "n", "expr": "6"}));
        let inspect = run(&mut s, "fold.inspect", json!({"fold": "Pleats"}));
        assert_eq!(inspect["facets"].as_array().unwrap().len(), 6, "{inspect}");
        // Saved and reopened, the same.
        run(&mut s, "file.save", json!({"path": "pleats.3dcraft"}));
        let mut t = session_in(&dir);
        run(&mut t, "file.open", json!({"path": "pleats.3dcraft"}));
        let again = run(&mut t, "fold.inspect", json!({"fold": "Pleats"}));
        assert_eq!(again["creases"], inspect["creases"]);
        // What is not a copy, or not a line, is an error.
        error(
            &mut s,
            "fold.edit",
            json!({"fold": "Pleats", "set": [{"line": format!("{line}[x]"), "kind": "valley"}]}),
        );
        error(
            &mut s,
            "fold.edit",
            json!({"fold": "Pleats", "set": [{"line": all, "alternate": "yes"}]}),
        );
        error(
            &mut s,
            "fold.edit",
            json!({"fold": "Pleats", "set": [{"line": line, "kind": "valley", "alternate": true}]}),
        );
        let why = failed(
            &mut s,
            "fold.edit",
            json!({"fold": "Pleats", "set": [{"line": format!("{line}[9]"), "kind": "valley"}]}),
        );
        assert!(why.contains(&format!("{line}[9]")), "{why}");
    }
}

fn assertion(s: &mut Session, name: &str, spec: Value) -> Value {
    let mut params = spec;
    params["name"] = json!(name);
    run(s, "assert.add", params);
    let all = run(s, "assert.list", json!({}));
    all["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("no assertion {name} in {all}"))
        .clone()
}

#[test]
fn a_fold_is_measured_by_its_vertex_conditions_and_whether_it_folds_flat() {
    let dir = workdir("fold_flat", "measures");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let c = 50.0;
    let lines = square(
        &mut s,
        "Square",
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
        json!({"sketch": "Square", "name": "Waterbomb", "creases": [
            {"line": lines[0], "kind": "mountain", "angle": "30 deg"},
            {"line": lines[1], "kind": "valley", "angle": "40 deg"},
            {"line": lines[2], "kind": "valley"}]}),
    );
    for (name, measure, expected) in [
        ("Kawasaki", "kawasakiFails", json!({"eq": "0"})),
        ("Maekawa", "maekawaFails", json!({"eq": "0"})),
        ("Flat", "flatFoldable", json!({"is": true})),
    ] {
        let mut spec = json!({"measure": measure, "of": "Waterbomb"});
        for (key, value) in expected.as_object().unwrap() {
            spec[key] = value.clone();
        }
        let check = assertion(&mut s, name, spec);
        assert_eq!(check["status"], "passed", "{check}");
    }
    // All valleys: Maekawa fails at the one vertex, and it does not fold
    // flat; the rigid fold may still be built or not, the measures read
    // the pattern.
    run(
        &mut s,
        "fold.edit",
        json!({"fold": "Waterbomb", "set": [{"line": lines[0], "kind": "valley"}]}),
    );
    let all = run(&mut s, "assert.list", json!({}));
    let status = |name: &str| {
        all["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(status("Kawasaki"), "passed", "{all}");
    assert_eq!(status("Maekawa"), "failed", "{all}");
    assert_eq!(status("Flat"), "failed", "{all}");
    // The crease pattern for printing: a line width of its own.
    run(
        &mut s,
        "file.export",
        json!({"path": "printed.svg", "fold": "Waterbomb", "stroke": 0.3}),
    );
    let svg = std::fs::read_to_string(dir.join("printed.svg")).unwrap();
    assert!(svg.contains("stroke-width=\"0.3\""), "{svg}");
    run(
        &mut s,
        "file.export",
        json!({"path": "laser.svg", "fold": "Waterbomb"}),
    );
    let svg = std::fs::read_to_string(dir.join("laser.svg")).unwrap();
    assert!(svg.contains("stroke-width=\"0.01\""), "{svg}");
    error(
        &mut s,
        "file.export",
        json!({"path": "p.dxf", "fold": "Waterbomb", "stroke": 0.3}),
    );
    error(
        &mut s,
        "file.export",
        json!({"path": "p.svg", "fold": "Waterbomb", "stroke": 0}),
    );
    error(
        &mut s,
        "file.export",
        json!({"path": "p.stl", "stroke": 0.3}),
    );
    // A measure of what is no fold is an error, not a crash.
    let wrong = assertion(
        &mut s,
        "Wrong",
        json!({"measure": "kawasakiFails", "of": "Square", "eq": "0"}),
    );
    assert_eq!(wrong["status"], "error", "{wrong}");
}
