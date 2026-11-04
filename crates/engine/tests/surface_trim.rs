//! Trimmed faces through the commands (docs/design/surfacing.md › *A face
//! on a surface, bounded by curves on it*): a plane trimmed by a projected
//! circle and ring has the closed-form area; a spline quarter cylinder
//! trimmed in parameter space, along a direction and to closest points has
//! the closed-form area of the region on the cylinder, and a plate on it
//! the closed-form volume; the bodies survive `.3dcraft` and STEP round
//! trips; names follow the curves, not their size; bad input is an error.
//! At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, error, failed, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI};

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

fn sketch(s: &mut Session, name: &str, plane: &str, offset: f64) {
    built(
        s,
        "sketch.create",
        json!({"plane": plane, "offset": mm(offset), "name": name}),
    );
}

fn line(s: &mut Session, name: &str, plane: &str, offset: f64, from: [f64; 2], to: [f64; 2]) {
    sketch(s, name, plane, offset);
    run(
        s,
        "sketch.line",
        json!({"sketch": name, "from": from, "to": to}),
    );
}

fn quarter(s: &mut Session, name: &str, z: f64, r: f64) {
    sketch(s, name, "xy", z);
    run(
        s,
        "sketch.arc",
        json!({"sketch": name, "center": [0.0, 0.0], "start": [r, 0.0], "end": [0.0, r]}),
    );
}

/// The quarter cylinder of radius `r` and height `h` about z, from +x to
/// +y, as a spline sheet (a Coons patch: u along the arcs, v up); the
/// name of its face.
fn spline_cylinder(s: &mut Session, r: f64, h: f64) -> String {
    quarter(s, "Low", 0.0, r);
    quarter(s, "High", h, r);
    line(s, "Left", "zx", 0.0, [0.0, r], [h, r]);
    line(s, "Right", "yz", 0.0, [r, 0.0], [r, h]);
    built(
        s,
        "feature.surface.coons",
        json!({"curves": ["Low", "Right", "High", "Left"], "name": "Wall"}),
    );
    body(s, "Wall")["faceNames"][0]
        .as_str()
        .unwrap()
        .to_string()
}

fn edge_names(s: &mut Session, feature: &str) -> Vec<String> {
    let edges = run(s, "document.edges", json!({"feature": feature}));
    let mut out: Vec<String> = edges["edges"]
        .as_array()
        .unwrap_or_else(|| panic!("{edges}"))
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    out.sort();
    out
}

#[test]
fn a_plane_trimmed_by_a_projected_circle_and_ring_has_their_area() {
    for k in SCALES {
        let dir = workdir("surface-trim", &format!("plane-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let size = 100.0 * k;
        built(
            &mut s,
            "feature.block",
            json!({"size": [mm(size), mm(size), mm(size / 4.0)], "name": "Base"}),
        );
        let (r, hole) = (30.0 * k, 12.0 * k);
        sketch(&mut s, "Disc", "xy", 2.0 * size);
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Disc", "center": [size / 2.0, size / 2.0], "radius": mm(r)}),
        );
        sketch(&mut s, "Hole", "xy", 2.0 * size);
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Hole", "center": [size / 2.0, size / 2.0 + 5.0 * k], "radius": mm(hole)}),
        );
        let top = "F1:face(1)";
        let made = built(
            &mut s,
            "feature.surface.trim",
            json!({"face": top, "outer": "Disc", "name": "Patch"}),
        );
        assert_eq!(number(&made["trim"]["deviation"]), 0.0, "{made}");
        let patch = body(&mut s, "Patch");
        assert_eq!(patch["kind"], "sheet", "{patch}");
        assert_eq!(patch["valid"], true, "{patch}");
        near(number(&patch["area"]), PI * r * r, 1e-9, "the disc");
        built(
            &mut s,
            "feature.surface.trim",
            json!({"face": top, "outer": ["Disc"], "holes": [["Hole"]], "name": "Ring"}),
        );
        let ring = body(&mut s, "Ring");
        assert_eq!(ring["valid"], true, "{ring}");
        near(
            number(&ring["area"]),
            PI * (r * r - hole * hole),
            1e-9,
            "the ring",
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
    }
}

#[test]
fn a_spline_cylinder_trimmed_three_ways_has_the_area_on_the_cylinder() {
    for k in SCALES {
        let dir = workdir("surface-trim", &format!("cylinder-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (r, h) = (20.0 * k, 30.0 * k);
        let wall = spline_cylinder(&mut s, r, h);

        // In parameter space: u over [1/4, 3/4] runs from the angle θ(1/4)
        // of the rational quarter arc (weights 1, √2/2, 1) to π/2 − θ(1/4)
        // (it is symmetric), v over [0.2, 0.6] is 0.4 h.
        sketch(&mut s, "Square", "xy", 0.0);
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Square", "from": [25.0, 20.0], "to": [75.0, 60.0]}),
        );
        let made = built(
            &mut s,
            "feature.surface.trim",
            json!({"face": wall, "outer": "Square", "space": "parameter", "name": "Uv"}),
        );
        assert_eq!(number(&made["trim"]["deviation"]), 0.0, "{made}");
        let uv = body(&mut s, "Uv");
        assert_eq!(uv["valid"], true, "{uv}");
        let theta = {
            let (u, w) = (0.25_f64, std::f64::consts::FRAC_1_SQRT_2);
            let x = (1.0 - u) * (1.0 - u) + w * 2.0 * u * (1.0 - u);
            let y = w * 2.0 * u * (1.0 - u) + u * u;
            y.patan2(x)
        };
        let sweep = FRAC_PI_2 - 2.0 * theta;
        near(
            number(&uv["area"]),
            sweep * r * 0.4 * h,
            1e-9,
            "the parameter-space square",
        );

        // Along -x from the plane x = 2r: y in [y0, y1], z in [z0, z1]
        // lands on the angles asin(y / r).
        let (y0, y1, z0, z1) = (3.0 * k, 15.0 * k, 5.0 * k, 22.0 * k);
        sketch(&mut s, "Window", "yz", 2.0 * r);
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Window", "from": [y0, z0], "to": [y1, z1]}),
        );
        let along = built(
            &mut s,
            "feature.surface.trim",
            json!({"face": wall, "outer": "Window", "along": [-1, 0, 0], "name": "Along"}),
        );
        assert!(number(&along["trim"]["deviation"]) <= 0.5e-6 * k, "{along}");
        // The deviation is the proven bound: at least the gap sampled.
        assert!(
            number(&along["trim"]["sampled"]) <= number(&along["trim"]["deviation"]),
            "{along}"
        );
        let a = body(&mut s, "Along");
        assert_eq!(a["valid"], true, "{a}");
        let angle = (y1 / r).pasin() - (y0 / r).pasin();
        near(
            number(&a["area"]),
            r * angle * (z1 - z0),
            1e-5,
            "projected along -x",
        );

        // To the closest points: the angles atan(y / 2r).
        built(
            &mut s,
            "feature.surface.trim",
            json!({"face": wall, "outer": "Window", "name": "Closest"}),
        );
        let c = body(&mut s, "Closest");
        assert_eq!(c["valid"], true, "{c}");
        let angle = (y1 / (2.0 * r)).patan() - (y0 / (2.0 * r)).patan();
        near(
            number(&c["area"]),
            r * angle * (z1 - z0),
            1e-5,
            "projected to closest points",
        );

        // A plate outwards on the square: a piece of a tube, watertight.
        let t = 2.0 * k;
        let face = body(&mut s, "Uv")["faceNames"][0]
            .as_str()
            .unwrap()
            .to_string();
        built(
            &mut s,
            "feature.plate",
            json!({"faces": [face], "thickness": mm(t), "side": "out", "name": "Tube"}),
        );
        let tube = body(&mut s, "Tube");
        assert_eq!(tube["valid"], true, "{tube}");
        near(
            number(&tube["volume"]),
            sweep / 2.0 * ((r + t).ppowi(2) - r * r) * 0.4 * h,
            1e-6,
            "the plate's volume",
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");

        // Saved and opened again; written to STEP and read back.
        run(
            &mut s,
            "file.save",
            json!({"path": "trim.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "trim.3dcraft"}));
        for name in ["Uv", "Along", "Closest"] {
            let again = body(&mut s, name);
            near(
                number(&again["area"]),
                number(&body(&mut s, name)["area"]),
                1e-12,
                "reopened",
            );
        }
        near(
            number(&body(&mut s, "Uv")["area"]),
            number(&uv["area"]),
            1e-12,
            "reopened Uv",
        );
        run(
            &mut s,
            "file.export",
            json!({"path": "trim.step", "bodies": ["Uv", "Along"]}),
        );
        new_document(&mut s, k);
        run(
            &mut s,
            "file.import",
            json!({"path": "trim.step", "names": ["UvBack", "AlongBack"]}),
        );
        let back = body(&mut s, "UvBack");
        assert_eq!(back["valid"], true, "{back}");
        near(
            number(&back["area"]),
            number(&uv["area"]),
            1e-7,
            "Uv through STEP",
        );
        let back = body(&mut s, "AlongBack");
        near(
            number(&back["area"]),
            number(&a["area"]),
            1e-7,
            "Along through STEP",
        );
    }
}

#[test]
fn names_follow_the_curves_not_their_size() {
    let dir = workdir("surface-trim", "names");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let wall = spline_cylinder(&mut s, 20.0, 30.0);
    run(&mut s, "variable.set", json!({"name": "w", "expr": "50"}));
    sketch(&mut s, "Square", "xy", 0.0);
    run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Square", "from": [10.0, 20.0], "to": ["w", 60.0]}),
    );
    built(
        &mut s,
        "feature.surface.trim",
        json!({"face": wall, "outer": "Square", "space": "parameter", "name": "Uv"}),
    );
    let before = edge_names(&mut s, "Uv");
    assert_eq!(before.len(), 4, "{before:?}");
    assert!(
        before.iter().all(|n| n.contains(":startEdge(")),
        "{before:?}"
    );
    let area = number(&body(&mut s, "Uv")["area"]);
    run(&mut s, "variable.set", json!({"name": "w", "expr": "80"}));
    let after = edge_names(&mut s, "Uv");
    assert_eq!(before, after);
    assert!(number(&body(&mut s, "Uv")["area"]) > area);
    // Turned and mirrored, its names are carried as copies (an edge's two
    // numbers in a role a copy can write).
    built(
        &mut s,
        "feature.transform",
        json!({"source": "Uv", "angle": "5 deg", "axis": [0, 0, 1], "pivot": [0, 0, 0], "name": "Turned"}),
    );
    built(
        &mut s,
        "feature.mirror",
        json!({"source": "Turned", "normal": [0, 1, 0], "name": "Mirrored"}),
    );
    assert_eq!(edge_names(&mut s, "Mirrored").len(), 4);
}

#[test]
fn a_cap_round_a_spheres_pole_and_a_window_across_a_cylinders_seam_are_trimmed() {
    for k in SCALES {
        let dir = workdir("surface-trim", &format!("pole-seam-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (r, h) = (20.0 * k, 30.0 * k);
        built(
            &mut s,
            "feature.sphere",
            json!({"radius": mm(r), "name": "Ball"}),
        );
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": mm(r), "height": mm(h), "position": [mm(100.0 * k), "0 mm", "0 mm"], "name": "Can"}),
        );
        // A circle about the sphere's +z pole, seen along -z: the cap
        // 2 pi R (R - sqrt(R^2 - a^2)) has the pole inside it.
        let a = 12.0 * k;
        sketch(&mut s, "Polar", "xy", 3.0 * r);
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Polar", "center": [0.0, 0.0], "radius": mm(a)}),
        );
        let ball = format!("{}:face(0)", id_of(&mut s, "Ball"));
        built(
            &mut s,
            "feature.surface.trim",
            json!({"face": ball, "outer": "Polar", "along": [0, 0, -1], "name": "PoleCap"}),
        );
        let cap = body(&mut s, "PoleCap");
        assert_eq!(cap["valid"], true, "{cap}");
        near(
            number(&cap["area"]),
            2.0 * PI * r * (r - (r * r - a * a).sqrt()),
            1e-5,
            "the cap round the pole",
        );
        // A window on the plane x = 100 + 3r seen along -x: on the
        // cylinder it spans its seam at +x.
        let (y0, y1, z0, z1) = (-8.0 * k, 12.0 * k, 5.0 * k, 22.0 * k);
        sketch(&mut s, "Across", "yz", 100.0 * k + 3.0 * r);
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Across", "from": [y0, z0], "to": [y1, z1]}),
        );
        let can = format!("{}:face(0)", id_of(&mut s, "Can"));
        built(
            &mut s,
            "feature.surface.trim",
            json!({"face": can, "outer": "Across", "along": [-1, 0, 0], "name": "Seam"}),
        );
        let on = body(&mut s, "Seam");
        assert_eq!(on["valid"], true, "{on}");
        near(
            number(&on["area"]),
            r * ((y1 / r).pasin() - (y0 / r).pasin()) * (z1 - z0),
            1e-5,
            "the window across the seam",
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
    }
}

/// The vertex names of a feature's body, without the feature's own id.
fn vertex_names(s: &mut Session, feature: &str) -> Vec<String> {
    let refs = run(s, "datum.references", json!({"feature": feature}));
    let mut out: Vec<String> = refs["vertices"]
        .as_array()
        .unwrap_or_else(|| panic!("{refs}"))
        .iter()
        .map(|v| {
            let name = v
                .as_str()
                .map_or_else(|| v["name"].as_str().unwrap(), |x| x);
            name.split_once(':').unwrap().1.to_string()
        })
        .collect();
    out.sort();
    out
}

#[test]
fn vertex_names_do_not_depend_on_which_way_the_loop_runs() {
    let dir = workdir("surface-trim", "vertex-names");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let wall = spline_cylinder(&mut s, 20.0, 30.0);
    // The wall mirrored: its parameters turn the other way about its
    // normal, so the same square in parameter space runs the other way
    // round its face.
    built(
        &mut s,
        "feature.mirror",
        json!({"source": "Wall", "normal": [1, 0, 0], "name": "Other"}),
    );
    let other = body(&mut s, "Other")["faceNames"][0]
        .as_str()
        .unwrap()
        .to_string();
    sketch(&mut s, "Square", "xy", 0.0);
    run(
        &mut s,
        "sketch.rect",
        json!({"sketch": "Square", "from": [10.0, 20.0], "to": [50.0, 60.0]}),
    );
    built(
        &mut s,
        "feature.surface.trim",
        json!({"face": wall, "outer": "Square", "space": "parameter", "name": "One"}),
    );
    built(
        &mut s,
        "feature.surface.trim",
        json!({"face": other, "outer": "Square", "space": "parameter", "name": "Two"}),
    );
    let (one, two) = (vertex_names(&mut s, "One"), vertex_names(&mut s, "Two"));
    assert_eq!(one.len(), 4, "{one:?}");
    assert_eq!(one, two);
}

#[test]
fn loops_that_do_not_close_or_go_round_an_axis_fail_by_name() {
    let dir = workdir("surface-trim", "fail");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let wall = spline_cylinder(&mut s, 20.0, 30.0);
    line(&mut s, "Open", "xy", 0.0, [10.0, 10.0], [50.0, 50.0]);
    let why = failed(
        &mut s,
        "feature.surface.trim",
        json!({"face": wall, "outer": "Open", "space": "parameter", "name": "Bad"}),
    );
    assert!(why.contains("close"), "{why}");
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": "5 mm", "height": "10 mm", "name": "Can"}),
    );
    sketch(&mut s, "Disc", "xy", 50.0);
    run(
        &mut s,
        "sketch.circle",
        json!({"sketch": "Disc", "center": [0.0, 0.0], "radius": "2 mm"}),
    );
    let id = run(&mut s, "document.inspect", json!({}))["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Can")
        .unwrap()["id"]
        .clone();
    let can_wall = format!("{}:face(0)", id.as_str().unwrap());
    let why = failed(
        &mut s,
        "feature.surface.trim",
        json!({"face": can_wall, "outer": "Disc", "name": "Bad2"}),
    );
    assert!(why.contains("round"), "{why}");
}

#[test]
fn bad_trim_parameters_are_errors() {
    let dir = workdir("surface-trim", "params");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    let wall = spline_cylinder(&mut s, 20.0, 30.0);
    for params in [
        json!({}),
        json!({"face": wall}),
        json!({"face": wall, "outer": []}),
        json!({"face": wall, "outer": true}),
        json!({"face": wall, "outer": "Nope"}),
        json!({"face": wall, "outer": "Low", "space": "elsewhere"}),
        json!({"face": wall, "outer": "Low", "space": "parameter", "along": [1, 0, 0]}),
        json!({"face": wall, "outer": "Low", "holes": "Low"}),
        json!({"face": wall, "outer": ["Low", "Low"]}),
        json!({"face": wall, "outer": "Low", "holes": [["Low"]]}),
        json!({"face": "nonsense", "outer": "Low"}),
        json!({"face": wall, "outer": "Low", "bogus": 1}),
    ] {
        let message = error(&mut s, "feature.surface.trim", params.clone());
        assert!(
            message.contains("feature.surface.trim"),
            "{params}: {message}"
        );
    }
}

/// The id of the feature called `name`.
fn id_of(s: &mut Session, name: &str) -> String {
    run(s, "document.inspect", json!({}))["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn analytic_faces_are_trimmed_exactly_by_their_rational_forms() {
    for k in SCALES {
        let dir = workdir("surface-trim", &format!("analytic-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (r, h) = (20.0 * k, 30.0 * k);
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": mm(r), "height": mm(h), "name": "Can"}),
        );
        built(
            &mut s,
            "feature.sphere",
            json!({"radius": mm(r), "position": [mm(100.0 * k), "0 mm", "0 mm"], "name": "Ball"}),
        );
        built(
            &mut s,
            "feature.cone",
            json!({"radius": mm(r), "height": mm(h), "position": [mm(-100.0 * k), "0 mm", "0 mm"], "name": "Hat"}),
        );
        built(
            &mut s,
            "feature.torus",
            json!({"major": mm(r), "minor": mm(r / 3.0), "position": ["0 mm", mm(100.0 * k), "0 mm"], "name": "Ring"}),
        );
        // A window on the plane y = 3r, seen along -y: on the cylinder the
        // angles asin(x / r) about +y, away from its seam.
        let (x0, x1, z0, z1) = (-8.0 * k, 12.0 * k, 5.0 * k, 22.0 * k);
        sketch(&mut s, "Window", "zx", 3.0 * r);
        // zx: sketch (x, y) is world (y, 0, x), so world x is sketch y.
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "Window", "from": [z0, x0], "to": [z1, x1]}),
        );
        let can = format!("{}:face(0)", id_of(&mut s, "Can"));
        let made = built(
            &mut s,
            "feature.surface.trim",
            json!({"face": can, "outer": "Window", "along": [0, -1, 0], "name": "OnCan"}),
        );
        assert!(number(&made["trim"]["deviation"]) <= 0.5e-6 * k, "{made}");
        assert!(
            number(&made["trim"]["sampled"]) <= number(&made["trim"]["deviation"]),
            "{made}"
        );
        let on = body(&mut s, "OnCan");
        assert_eq!(on["valid"], true, "{on}");
        let angle = (x1 / r).pasin() - (x0 / r).pasin();
        near(
            number(&on["area"]),
            r * angle * (z1 - z0),
            1e-5,
            "the window on the cylinder",
        );

        // A circle of radius a seen along -y on the sphere: a cap of area
        // 2 pi R (R - sqrt(R^2 - a^2)), about +y (away from the sphere's
        // seam and poles).
        let a = 12.0 * k;
        sketch(&mut s, "Spot", "zx", 3.0 * r);
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "Spot", "center": [0.0, 100.0 * k], "radius": mm(a)}),
        );
        let ball = format!("{}:face(0)", id_of(&mut s, "Ball"));
        built(
            &mut s,
            "feature.surface.trim",
            json!({"face": ball, "outer": "Spot", "along": [0, -1, 0], "name": "Cap"}),
        );
        let cap = body(&mut s, "Cap");
        assert_eq!(cap["valid"], true, "{cap}");
        near(
            number(&cap["area"]),
            2.0 * PI * r * (r - (r * r - a * a).sqrt()),
            1e-5,
            "the cap on the sphere",
        );

        // The cone and the torus: a window each, valid, inside the face.
        sketch(&mut s, "ConeWindow", "zx", 3.0 * r);
        run(
            &mut s,
            "sketch.rect",
            json!({"sketch": "ConeWindow", "from": [5.0 * k, -105.0 * k], "to": [12.0 * k, -95.0 * k]}),
        );
        let hat = format!("{}:face(0)", id_of(&mut s, "Hat"));
        built(
            &mut s,
            "feature.surface.trim",
            json!({"face": hat, "outer": "ConeWindow", "along": [0, -1, 0], "name": "OnHat"}),
        );
        let on_hat = body(&mut s, "OnHat");
        assert_eq!(on_hat["valid"], true, "{on_hat}");
        assert!(number(&on_hat["area"]) > 7.0 * k * 10.0 * k, "{on_hat}");
        sketch(&mut s, "RingSpot", "xy", 2.0 * r);
        run(
            &mut s,
            "sketch.circle",
            json!({"sketch": "RingSpot", "center": [0.0, 100.0 * k + r], "radius": mm(r / 6.0)}),
        );
        let ring = format!("{}:face(0)", id_of(&mut s, "Ring"));
        built(
            &mut s,
            "feature.surface.trim",
            json!({"face": ring, "outer": "RingSpot", "along": [0, 0, -1], "name": "OnRing"}),
        );
        let on_ring = body(&mut s, "OnRing");
        assert_eq!(on_ring["valid"], true, "{on_ring}");
        let flat = PI * (r / 6.0) * (r / 6.0);
        let area = number(&on_ring["area"]);
        assert!(area > flat && area < 2.0 * flat, "{on_ring}");
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");

        // Through STEP.
        run(
            &mut s,
            "file.export",
            json!({"path": "analytic.step", "bodies": ["OnCan", "Cap"]}),
        );
        new_document(&mut s, k);
        run(
            &mut s,
            "file.import",
            json!({"path": "analytic.step", "names": ["CanBack", "CapBack"]}),
        );
        near(
            number(&body(&mut s, "CanBack")["area"]),
            number(&on["area"]),
            1e-7,
            "OnCan through STEP",
        );
        near(
            number(&body(&mut s, "CapBack")["area"]),
            number(&cap["area"]),
            1e-7,
            "Cap through STEP",
        );
    }
}
