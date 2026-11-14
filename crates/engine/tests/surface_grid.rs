//! Sheets through grids of points through the commands
//! (docs/design/surfacing.md › *A sheet through a grid of points*): a grid
//! on a slanted plane is that plane (its area in closed form), named by
//! the borders of the grid; the solid under a paraboloid's grid has the
//! volume of its integral, as an exact body and as a mesh; a grid read
//! from a table follows the table, its factor, a rename and a deletion;
//! moving the points renames nothing; saved, opened and through STEP the
//! same; hostile input is an error that says what to change. At three
//! scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{
    body, built, error, failed, failure, feature, new_document, run, session_in, workdir,
};
use craft3d_engine::Session;
use serde_json::{Value, json};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];

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

/// The rows of points `f(r, c)` (millimetres) as `points` takes them.
fn rows(count: usize, width: usize, f: impl Fn(usize, usize) -> [f64; 3]) -> Value {
    let rows: Vec<Value> = (0..count)
        .map(|r| {
            let row: Vec<Value> = (0..width)
                .map(|c| {
                    let [x, y, z] = f(r, c);
                    json!([format!("{x} mm"), format!("{y} mm"), format!("{z} mm")])
                })
                .collect();
            Value::Array(row)
        })
        .collect();
    Value::Array(rows)
}

/// The names of the edges and vertices of the body of `feature`.
fn edge_and_vertex_names(s: &mut Session, feature: &str) -> (Vec<String>, Vec<String>) {
    let listed = run(s, "document.edges", json!({"feature": feature}));
    let names = |key: &str| -> Vec<String> {
        let mut out: Vec<String> = listed[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        out.sort();
        out
    };
    (names("edges"), names("vertices"))
}

/// The volume enclosed by a binary STL: the sum of the signed volumes of
/// the tetrahedra from the origin to its triangles.
fn stl_volume(bytes: &[u8]) -> f64 {
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    let mut total = 0.0;
    for t in 0..count {
        let at = 84 + 50 * t + 12;
        let v = |k: usize| -> [f64; 3] {
            let o = at + 12 * k;
            let f = |i: usize| {
                f64::from(f32::from_le_bytes(
                    bytes[o + 4 * i..o + 4 * i + 4].try_into().unwrap(),
                ))
            };
            [f(0), f(1), f(2)]
        };
        let (a, b, c) = (v(0), v(1), v(2));
        total += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]))
            / 6.0;
    }
    total
}

#[test]
fn a_grid_on_a_slanted_plane_is_that_plane_named_by_its_borders() {
    for k in SCALES {
        let dir = workdir("surface-grid", &format!("plane-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        // Uneven steps, on the plane z = 0.3 x + 0.2 y + k. At Greville's
        // parameters the sheet through them runs back along its rows
        // (x(u) through 0, 1, 2.5, 3, 5 at 0, 1/6, 1/2, 5/6, 1 falls by
        // 3.4e-5 near u = 0.61, so the flat sheet folds over itself
        // there): refused, by the proof of its normal; chord spacing
        // follows the steps.
        let xs = [0.0, 1.0, 2.5, 3.0, 5.0];
        let ys = [0.0, 2.0, 3.0, 6.0];
        let points = rows(4, 5, |r, c| {
            let (x, y) = (xs[c] * k, ys[r] * k);
            [x, y, 0.3 * x + 0.2 * y + k]
        });
        let why = failed(
            &mut s,
            "feature.surface.grid",
            json!({"points": points.clone()}),
        );
        assert!(
            why.contains("folds near row") && why.contains("chord"),
            "{why}"
        );
        run(&mut s, "edit.undo", json!({}));
        let made = built(
            &mut s,
            "feature.surface.grid",
            json!({"points": points, "spacing": "chord", "name": "Slope"}),
        );
        assert_eq!(made["grid"]["rows"], 4, "{made}");
        assert_eq!(made["grid"]["columns"], 5, "{made}");
        assert!(
            number(&made["grid"]["deviation"]) <= number(&made["grid"]["bound"]),
            "{made}"
        );
        let sheet = body(&mut s, "Slope");
        assert_eq!(sheet["kind"], "sheet", "{sheet}");
        assert_eq!(sheet["valid"], true, "{sheet}");
        let slope = (1.0_f64 + 0.09 + 0.04).sqrt();
        near(
            number(&sheet["area"]),
            slope * 5.0 * 6.0 * k * k,
            1e-9,
            "the plane's area",
        );
        assert_eq!(sheet["faceNames"], json!(["F1:face(0)"]), "{sheet}");
        let (edges, vertices) = edge_and_vertex_names(&mut s, "Slope");
        assert_eq!(
            edges,
            ["F1:edge(0)", "F1:edge(1)", "F1:edge(2)", "F1:edge(3)"],
        );
        assert_eq!(
            vertices,
            [
                "F1:vertex(0)",
                "F1:vertex(1)",
                "F1:vertex(2)",
                "F1:vertex(3)"
            ],
        );

        // Saved and opened again, and through STEP: the same sheet.
        run(
            &mut s,
            "file.save",
            json!({"path": "slope.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "slope.3dcraft"}));
        near(
            number(&body(&mut s, "Slope")["area"]),
            number(&sheet["area"]),
            1e-12,
            "reopened",
        );
        run(&mut s, "file.export", json!({"path": "slope.step"}));
        new_document(&mut s, k);
        run(
            &mut s,
            "file.import",
            json!({"path": "slope.step", "names": ["Back"]}),
        );
        let back = body(&mut s, "Back");
        assert_eq!(back["valid"], true, "{back}");
        near(
            number(&back["area"]),
            number(&sheet["area"]),
            1e-7,
            "through STEP",
        );
    }
}

#[test]
fn the_solid_under_a_paraboloid_grid_has_the_volume_of_its_integral() {
    for k in SCALES {
        let dir = workdir("surface-grid", &format!("paraboloid-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        // z = (x² + y²) / k + k over [-k, k]², 17 × 17 points.
        let n = 17;
        let at = |i: usize| k * (-1.0 + 2.0 * i as f64 / (n - 1) as f64);
        let points = rows(n, n, |r, c| {
            let (x, y) = (at(c), at(r));
            [x, y, (x * x + y * y) / k + k]
        });
        let made = built(
            &mut s,
            "feature.surface.grid",
            json!({"points": points, "spacing": "chord", "name": "Bowl"}),
        );
        assert!(
            number(&made["grid"]["deviation"]) <= number(&made["grid"]["bound"]),
            "{made}"
        );
        // A block inside the sheet's footprint reaching above it, cut by
        // the sheet: the part behind it (the sheet looks up) is the solid
        // under the bowl.
        let a = 0.9 * k;
        built(
            &mut s,
            "feature.block",
            json!({"size": [format!("{} mm", 2.0 * a), format!("{} mm", 2.0 * a), format!("{} mm", 4.0 * k)], "position": [format!("{} mm", -a), format!("{} mm", -a), "0 mm"], "name": "Blank"}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "intersect", "target": "Blank", "tool": "Bowl", "keepTool": true, "name": "Under"}),
        );
        // ∫∫ (x² + y²) / k + k over [-a, a]²: 8 a⁴ / (3 k) + 4 a² k.
        let exact = 8.0 * a.ppowi(4) / (3.0 * k) + 4.0 * a * a * k;
        let under = body(&mut s, "Under");
        assert_eq!(under["kind"], "solid", "{under}");
        assert_eq!(under["valid"], true, "{under}");
        near(
            number(&under["volume"]),
            exact,
            2e-5,
            "the solid under the bowl",
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
        // Its mesh encloses the same volume, within what meshing at the
        // normal quality leaves.
        run(
            &mut s,
            "file.export",
            json!({"path": "under.stl", "bodies": ["Under"]}),
        );
        let bytes = std::fs::read(dir.join("under.stl")).unwrap();
        near(stl_volume(&bytes), exact, 2e-3, "the mesh of the solid");
    }
}

#[test]
fn a_grid_read_from_a_table_follows_the_table_and_its_factor() {
    let dir = workdir("surface-grid", "table");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    // Heights in metres, five a row, four rows; the key numbers the rows.
    let heights = |bump: f64| {
        json!([
            [1, 100, 110, 120, 110, 100],
            [2, 105, 130, 150, 125, 104],
            [3, 102, 128, 140 + bump as i64, 120, 101],
            [4, 100, 108, 115, 109, 100]
        ])
    };
    let columns = json!([
        "row",
        {"name": "a", "unit": "m"}, {"name": "b", "unit": "m"}, {"name": "c", "unit": "m"},
        {"name": "d", "unit": "m"}, {"name": "e", "unit": "m"}
    ]);
    run(
        &mut s,
        "table.set",
        json!({"name": "heights", "columns": columns, "rows": heights(0.0)}),
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "exaggeration", "expr": "2"}),
    );
    let made = built(
        &mut s,
        "feature.surface.grid",
        json!({"table": "heights", "step": ["10 mm", "8 mm"], "origin": ["0 mm", "0 mm", "-100 mm"], "factor": "exaggeration / 1000", "name": "Relief"}),
    );
    assert_eq!(made["grid"]["rows"], 4, "{made}");
    assert_eq!(made["grid"]["columns"], 5, "{made}");
    // The sheet's highest point: above the highest point of the grid
    // (150 m at 1:1000 times 2 is 300 mm, from -100 mm), as an
    // interpolating cubic rises between its points.
    let top = |s: &mut Session| number(&body(s, "Relief")["bbox"][1][2]);
    let first = top(&mut s);
    assert!(first >= 200.0 - 1e-3 && first < 210.0, "{first}");
    // The sheet looks up (+z) whatever the steps' signs: the same heights
    // run the other way along y, cut from the same block, leave the same
    // solid under them.
    built(
        &mut s,
        "feature.surface.grid",
        json!({"table": "heights", "step": ["10 mm", "-8 mm"], "origin": ["0 mm", "24 mm", "-100 mm"], "factor": "exaggeration / 1000", "name": "Mirrored"}),
    );
    for (name, sheet) in [("Under", "Relief"), ("Under mirrored", "Mirrored")] {
        built(
            &mut s,
            "feature.block",
            json!({"size": ["39 mm", "23 mm", "500 mm"], "position": ["0.5 mm", "0.5 mm", "-150 mm"], "name": format!("{name} blank")}),
        );
        built(
            &mut s,
            "feature.boolean",
            json!({"op": "intersect", "target": format!("{name} blank"), "tool": sheet, "keepTool": true, "name": name}),
        );
    }
    let (a, b) = (
        number(&body(&mut s, "Under")["volume"]),
        number(&body(&mut s, "Under mirrored")["volume"]),
    );
    near(b, a, 1e-9, "the solid under the mirrored grid");
    assert!(a < 39.0 * 23.0 * 400.0, "the part below the sheet: {a}");
    let corner = |s: &mut Session, name: &str| -> Vec<f64> {
        let listed = run(s, "document.edges", json!({"feature": "Relief"}));
        let v = listed["vertices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == name)
            .unwrap()
            .clone();
        v["point"].as_array().unwrap().iter().map(number).collect()
    };
    assert_eq!(corner(&mut s, "F1:vertex(0)"), [0.0, 0.0, 100.0]);
    assert_eq!(corner(&mut s, "F1:vertex(3)"), [40.0, 24.0, 100.0]);

    // The factor's variable rebuilds it.
    run(
        &mut s,
        "variable.set",
        json!({"name": "exaggeration", "expr": "3"}),
    );
    // z = -100 mm + height · factor: what stands above -100 mm grows by
    // half.
    near(
        top(&mut s) + 100.0,
        1.5 * (first + 100.0),
        1e-4,
        "exaggerated three times",
    );
    // So does a change of the table (no expression names it).
    run(
        &mut s,
        "table.set",
        json!({"name": "heights", "columns": columns, "rows": heights(20.0)}),
    );
    assert!(
        top(&mut s) > 1.5 * (first + 100.0) - 100.0 + 10.0,
        "the bump is in"
    );

    // A rename follows into the feature; a deletion is refused while it
    // reads the table, and with force fails it with the reason.
    run(
        &mut s,
        "table.rename",
        json!({"name": "heights", "to": "terrain"}),
    );
    assert_eq!(feature(&mut s, "Relief")["status"], "ok");
    let listed = run(&mut s, "table.list", json!({"name": "terrain"}));
    assert!(listed["usedBy"].to_string().contains("Relief"), "{listed}");
    let why = error(&mut s, "table.delete", json!({"name": "terrain"}));
    assert!(why.contains("Relief"), "{why}");
    run(
        &mut s,
        "table.delete",
        json!({"name": "terrain", "force": true}),
    );
    assert!(
        failure(&mut s, "Relief").contains("no table named `terrain`"),
        "{}",
        failure(&mut s, "Relief")
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(feature(&mut s, "Relief")["status"], "ok");

    // Saved and opened: the same sheet, still reading the table.
    let area = number(&body(&mut s, "Relief")["area"]);
    run(
        &mut s,
        "file.save",
        json!({"path": "relief.3dcraft", "overwrite": true}),
    );
    run(&mut s, "file.open", json!({"path": "relief.3dcraft"}));
    near(
        number(&body(&mut s, "Relief")["area"]),
        area,
        1e-12,
        "reopened",
    );
    run(
        &mut s,
        "variable.set",
        json!({"name": "exaggeration", "expr": "1"}),
    );
    assert!(top(&mut s) < 100.0, "still live after opening");
}

#[test]
fn moving_the_points_renames_nothing_and_what_follows_stays() {
    let dir = workdir("surface-grid", "names");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let wave = |amplitude: f64| {
        rows(5, 6, move |r, c| {
            let (x, y) = (c as f64 * 4.0, r as f64 * 5.0);
            [x, y, amplitude * ((x * 0.3).psin() + (y * 0.2).pcos())]
        })
    };
    built(
        &mut s,
        "feature.surface.grid",
        json!({"points": wave(1.0), "name": "Panel"}),
    );
    built(
        &mut s,
        "feature.plate",
        json!({"faces": ["F1:face(0)"], "thickness": "1 mm", "name": "Skin"}),
    );
    let before = edge_and_vertex_names(&mut s, "Panel");
    let skin = number(&body(&mut s, "Skin")["volume"]);
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Panel", "points": wave(2.5)}),
    );
    assert_eq!(edited["status"], "ok", "{edited}");
    assert_eq!(edge_and_vertex_names(&mut s, "Panel"), before);
    assert_eq!(names_of_body(&mut s, "Panel"), ["F1:face(0)"]);
    assert_eq!(feature(&mut s, "Skin")["status"], "ok");
    assert!(
        (number(&body(&mut s, "Skin")["volume"]) - skin).abs() > 1e-6,
        "the plate followed the new points"
    );
    // Chord spacing is another sheet through the same points.
    let area = number(&body(&mut s, "Panel")["area"]);
    run(
        &mut s,
        "feature.edit",
        json!({"feature": "Panel", "spacing": "chord"}),
    );
    assert!((number(&body(&mut s, "Panel")["area"]) - area).abs() > 1e-9);
    // Undo and redo.
    run(&mut s, "edit.undo", json!({}));
    near(
        number(&body(&mut s, "Panel")["area"]),
        area,
        1e-12,
        "undone",
    );
    run(&mut s, "edit.redo", json!({}));
    assert!((number(&body(&mut s, "Panel")["area"]) - area).abs() > 1e-9);
}

fn names_of_body(s: &mut Session, name: &str) -> Vec<String> {
    common::names_of(&body(s, name))
}

#[test]
fn hostile_grids_are_errors_that_say_what_to_change() {
    let dir = workdir("surface-grid", "hostile");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let grid = "feature.surface.grid";
    let flat = |r: usize, c: usize| [c as f64, r as f64, 0.0];
    run(
        &mut s,
        "table.set",
        json!({"name": "narrow", "columns": ["row", "h"], "rows": [[1, 1], [2, 2]]}),
    );
    run(
        &mut s,
        "table.set",
        json!({"name": "worded", "columns": ["row", "a", "b"], "rows": [[1, 1, 2], [2, "high", 3]]}),
    );
    run(
        &mut s,
        "table.set",
        json!({"name": "good", "columns": ["row", "a", "b"], "rows": [[1, 1, 2], [2, 2, 3]]}),
    );
    // Refused by the command, with what to change.
    let wide: Vec<Value> = (0..16385).map(|c| json!([c, 0, 0])).collect();
    let many: Vec<Value> =
        std::iter::once(Value::Array((0..512).map(|c| json!([c, 0, 0])).collect()))
            .chain((0..512).map(|_| json!([])))
            .collect();
    for (params, says) in [
        (json!({}), "give `points`"),
        (
            json!({"points": rows(2, 2, flat), "table": "good"}),
            "not both",
        ),
        (json!({"points": [[[0, 0, 0], [1, 0, 0]]]}), "2 to"),
        (json!({"points": [[[0, 0, 0]], [[0, 1, 0]]]}), "2 to"),
        (
            json!({"points": [[[0, 0, 0], [1, 0, 0]], [[0, 1, 0]]]}),
            "row 2 of `points` has 1 points",
        ),
        (
            json!({"points": [[[0, 0, 0], [1, 0]], [[0, 1, 0], [1, 1, 0]]]}),
            "row 1, point 2",
        ),
        (
            json!({"points": [[[0, 0, 0], [1, 0, true]], [[0, 1, 0], [1, 1, 0]]]}),
            "an expression",
        ),
        (
            json!({"points": [[[0, 0, 0], [1, 0, " "]], [[0, 1, 0], [1, 1, 0]]]}),
            "empty expression",
        ),
        (json!({"points": "rows"}), "list of rows"),
        (json!({"points": [[wide.clone()], [wide]]}), "2 to"),
        (json!({"points": many}), "at most 262144 points"),
        (
            json!({"points": rows(2, 2, flat), "spacing": "even"}),
            "greville or chord",
        ),
        (
            json!({"points": rows(2, 2, flat), "step": [1, 1]}),
            "with `points` each point is given whole",
        ),
        (
            json!({"table": "missing", "step": [1, 1]}),
            "no table named `missing`",
        ),
        (json!({"table": 7, "step": [1, 1]}), "the name of a table"),
        (json!({"table": "good"}), "needs `step`"),
        (
            json!({"table": "good", "step": [1, 1, 1]}),
            "`step` is [x, y]",
        ),
        (
            json!({"table": "good", "step": [1, 1], "origin": [0, 0]}),
            "origin",
        ),
    ] {
        let why = error(&mut s, grid, params.clone());
        assert!(why.contains(says), "{params}: {why}");
    }
    // Accepted, and the feature fails with the reason.
    for (params, says) in [
        (
            json!({"points": rows(3, 3, |r, c| if (r, c) == (1, 1) { [f64::NAN, 0.0, 0.0] } else { flat(r, c) })
                .to_string().replace("\"NaN mm\"", "\"sqrt(-1) * 1 mm\"").parse::<Value>().unwrap()}),
            "points[1][1].x",
        ),
        (
            json!({"points": rows(3, 4, |r, c| if (r, c) == (1, 2) { flat(1, 1) } else { flat(r, c) })}),
            "row 2: points 2 and 3",
        ),
        (
            json!({"points": rows(3, 5, |r, c| [[0.0, 1.0, 2.0, 1.5, 1.0][c], r as f64, 0.0])}),
            "doubles back on itself",
        ),
        (
            json!({"points": rows(4, 6, |r, c| if r == 2 { [[0.0, 1.0, 2.0, 1.2, 0.4, -0.4][c], 2.0, if c >= 3 { 0.3 } else { 0.0 }] } else { flat(r, c) })}),
            "folds near row",
        ),
        (
            json!({"table": "narrow", "step": [1, 1]}),
            "at least 2 of each",
        ),
        (
            json!({"table": "worded", "step": [1, 1]}),
            "holds the text \"high\"",
        ),
        (
            json!({"table": "good", "step": [1, 1], "factor": "1 deg"}),
            "a length is needed",
        ),
        (
            json!({"table": "good", "step": ["0 mm", 1]}),
            "other than zero",
        ),
        (json!({"table": "good", "step": ["1 deg", 1]}), "step.x"),
    ] {
        let why = failed(&mut s, grid, params.clone());
        assert!(why.contains(says), "{params}: {why}");
        run(&mut s, "edit.undo", json!({}));
    }
    // feature.edit refuses what the command refuses, and leaves the
    // feature as it was.
    built(
        &mut s,
        grid,
        json!({"points": rows(3, 3, flat), "name": "Ok"}),
    );
    for params in [
        json!({"feature": "Ok", "points": [[[0, 0, 0]]]}),
        json!({"feature": "Ok", "points": rows(2, 2, flat), "factor": 2}),
        json!({"feature": "Ok", "table": "missing", "step": [1, 1]}),
        json!({"feature": "Ok", "table": "good"}),
        json!({"feature": "Ok", "spacing": "uniform"}),
        json!({"feature": "Ok", "step": [1, 1, 1]}),
    ] {
        error(&mut s, "feature.edit", params);
    }
    assert_eq!(feature(&mut s, "Ok")["status"], "ok");
    // From points to a table and back.
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ok", "table": "good", "step": ["2 mm", "3 mm"]}),
    );
    assert_eq!(edited["status"], "ok", "{edited}");
    assert_eq!(edited["grid"]["columns"], 2, "{edited}");
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ok", "factor": "3"}),
    );
    assert_eq!(edited["status"], "ok", "{edited}");
    let edited = run(
        &mut s,
        "feature.edit",
        json!({"feature": "Ok", "points": rows(2, 3, flat)}),
    );
    assert_eq!(edited["status"], "ok", "{edited}");
    assert_eq!(edited["grid"]["rows"], 2, "{edited}");
}
