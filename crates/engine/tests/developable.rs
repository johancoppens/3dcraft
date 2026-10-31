//! Developable sheets and unfolding through the commands
//! (docs/design/developable.md): the developable between two arcs is a
//! cone frustum (its area in closed form, its curvature nil), its unfold
//! has the same area and maps points there and back, a sphere is not
//! developable (its curvature 1/r²) and its unfold is refused, the
//! measures work as assertions, the document saves and reopens to the
//! same bodies, and bad parameters are errors. At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::checked::{body, built, error, failed, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use serde_json::{Value, json};

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

/// An arc of radius `r` about the z axis from `from` to `to` degrees, in
/// a sketch on the plane z = `z`.
fn arc(s: &mut Session, name: &str, z: f64, r: f64, from: f64, to: f64) {
    built(
        s,
        "sketch.create",
        json!({"plane": "xy", "offset": mm(z), "name": name}),
    );
    let at = |deg: f64| {
        let a = deg.to_radians();
        [r * a.pcos(), r * a.psin()]
    };
    run(
        s,
        "sketch.arc",
        json!({"sketch": name, "center": [0.0, 0.0], "start": at(from), "end": at(to)}),
    );
}

/// The frustum panel: arcs of radius 20 at z = 0 and 10 at z = 15, both
/// from 10 to 120 degrees, times `k`.
fn panel(s: &mut Session, k: f64) {
    arc(s, "Low", 0.0, 20.0 * k, 10.0, 120.0);
    arc(s, "High", 15.0 * k, 10.0 * k, 10.0, 120.0);
    let made = built(
        s,
        "feature.developable",
        json!({"a": "Low", "b": "High", "name": "Panel"}),
    );
    // The report: how far the faces were found from the developable, and
    // what the rounding of their control points leaves unresolved at the
    // faces' end rulings (an estimate, beside the deviation; review of
    // 2026-10-29: it was not reported).
    let report = &made["developable"];
    let tolerance = 1e-6 * k;
    assert!(number(&report["deviation"]) <= 0.5 * tolerance, "{made}");
    let unresolved = number(&report["unresolved"]);
    assert!((0.0..=1e-3 * tolerance).contains(&unresolved), "{made}");
}

#[test]
fn the_developable_between_coaxial_arcs_is_a_cone_frustum_and_unfolds_flat() {
    for k in SCALES {
        let dir = workdir("developable", &format!("frustum-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        panel(&mut s, k);
        let sheet = body(&mut s, "Panel");
        assert_eq!(sheet["kind"], "sheet", "{sheet}");
        assert_eq!(sheet["valid"], true, "{sheet}");
        let (big, small, h) = (20.0 * k, 10.0 * k, 15.0 * k);
        let slant = (big - small).phypot(h);
        let sweep = 110_f64.to_radians();
        let area = 0.5 * sweep * (big + small) * slant;
        near(number(&sheet["area"]), area, 1e-6, "the panel's area");
        let faces = sheet["faceNames"].clone();
        assert_eq!(faces.as_array().unwrap().len(), 1, "{faces}");

        // Developable: no curvature, within the tolerance.
        let measured = run(&mut s, "measure.developability", json!({"of": "Panel"}));
        assert_eq!(measured["developable"], true, "{measured}");
        assert!(number(&measured["curvature"]) * k * k < 1e-6, "{measured}");

        // Unfolded: the same area, flat (an annular sector).
        built(
            &mut s,
            "feature.unfold",
            json!({"faces": faces, "at": [mm(0.0), mm(-50.0 * k), mm(0.0)], "name": "Flat"}),
        );
        let flat = body(&mut s, "Flat");
        assert_eq!(flat["kind"], "sheet", "{flat}");
        near(number(&flat["area"]), area, 1e-6, "the pattern's area");

        // A point of the part goes to the pattern and back.
        let a = 60_f64.to_radians();
        let (rho, z) = (15.0 * k, 7.5 * k);
        let p = [rho * a.pcos(), rho * a.psin(), z];
        let there = run(
            &mut s,
            "unfold.map",
            json!({"unfold": "Flat", "points": [p], "to": "flat"}),
        );
        let q = there["points"][0]["point"].clone();
        assert!(number(&q[2]).abs() <= 1e-9 * k, "{there}");
        let back = run(
            &mut s,
            "unfold.map",
            json!({"unfold": "Flat", "points": [q], "to": "part"}),
        );
        let r = &back["points"][0]["point"];
        let d = (0..3)
            .map(|i| (number(&r[i]) - p[i]).ppowi(2))
            .sum::<f64>()
            .sqrt();
        assert!(d <= 1e-5 * k, "{back} against {p:?}: {d}");

        // As assertions.
        run(
            &mut s,
            "assert.add",
            json!({"name": "flat panel", "measure": "developable", "of": "Panel", "is": true}),
        );
        run(
            &mut s,
            "assert.add",
            json!({"name": "no curvature", "measure": "curvature", "of": "Panel", "max": format!("{}", 1e-6 / (k * k))}),
        );
        let listed = run(&mut s, "assert.list", json!({}));
        assert_eq!(listed["ok"], true, "{listed}");

        // Saved and opened again: the same bodies.
        run(
            &mut s,
            "file.save",
            json!({"path": "panel.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "panel.3dcraft"}));
        near(
            number(&body(&mut s, "Panel")["area"]),
            number(&sheet["area"]),
            1e-12,
            "reopened panel",
        );
        near(
            number(&body(&mut s, "Flat")["area"]),
            number(&flat["area"]),
            1e-12,
            "reopened pattern",
        );
    }
}

#[test]
fn a_sphere_is_not_developable_and_its_unfold_is_refused() {
    for k in SCALES {
        let dir = workdir("developable", &format!("sphere-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let r = 2.0 * k;
        built(
            &mut s,
            "feature.sphere",
            json!({"radius": mm(r), "name": "Ball"}),
        );
        let measured = run(&mut s, "measure.developability", json!({"of": "Ball"}));
        assert_eq!(measured["developable"], false, "{measured}");
        near(
            number(&measured["curvature"]),
            1.0 / (r * r),
            1e-9,
            "the sphere's curvature",
        );
        let face = body(&mut s, "Ball")["faceNames"][0].clone();
        let why = failed(
            &mut s,
            "feature.unfold",
            json!({"faces": [face], "name": "Flat"}),
        );
        assert!(why.contains("not developable"), "{why}");
    }
}

#[test]
fn a_cylinder_unrolls_to_its_rectangle() {
    for k in SCALES {
        let dir = workdir("developable", &format!("cylinder-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (r, h) = (3.0 * k, 5.0 * k);
        built(
            &mut s,
            "feature.cylinder",
            json!({"radius": mm(r), "height": mm(h), "name": "Can"}),
        );
        let wall = body(&mut s, "Can")["faceNames"][0].clone();
        built(
            &mut s,
            "feature.unfold",
            json!({"faces": [wall], "name": "Label"}),
        );
        let label = body(&mut s, "Label");
        near(
            number(&label["area"]),
            std::f64::consts::TAU * r * h,
            1e-6,
            "the label's area",
        );
    }
}

#[test]
fn bad_parameters_are_errors() {
    let dir = workdir("developable", "errors");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    panel(&mut s, 1.0);
    built(
        &mut s,
        "feature.block",
        json!({"size": ["1 mm", "1 mm", "1 mm"], "name": "Box"}),
    );
    for params in [
        json!({"a": "Low"}),
        json!({"a": "Low", "b": "Low"}),
        json!({"a": "Low", "b": "Box"}),
        json!({"a": "Low", "b": {"edges": []}}),
        json!({"a": "Low", "b": {"edges": ["F1:face(0)"]}}),
        json!({"a": "Low", "b": {"faces": ["x"]}}),
        json!({"a": 7.5, "b": "High"}),
    ] {
        error(&mut s, "feature.developable", params);
    }
    for params in [
        json!({}),
        json!({"faces": []}),
        json!({"faces": "Panel"}),
        json!({"faces": ["nonsense"]}),
    ] {
        error(&mut s, "feature.unfold", params);
    }
    for params in [
        json!({"unfold": "Panel", "points": [[0, 0, 0]], "to": "flat"}),
        json!({"unfold": "Panel", "points": [], "to": "flat"}),
        json!({"unfold": "Panel", "points": [[0, 0, 0]], "to": "elsewhere"}),
        json!({"unfold": "Panel", "points": [["a"]], "to": "part"}),
    ] {
        error(&mut s, "unfold.map", params);
    }
    error(&mut s, "measure.developability", json!({"of": "Nothing"}));
}

/// The edge of `feature`'s body from `a` to `b` (either way round).
fn edge_between(s: &mut Session, feature: &str, a: [f64; 3], b: [f64; 3]) -> Value {
    let edges = run(s, "document.edges", json!({"feature": feature}));
    let at = |p: &Value, q: [f64; 3]| (0..3).all(|i| (number(&p[i]) - q[i]).abs() < 1e-9);
    edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            (at(&e["start"], a) && at(&e["end"], b)) || (at(&e["start"], b) && at(&e["end"], a))
        })
        .unwrap_or_else(|| panic!("no edge from {a:?} to {b:?} in {edges}"))["name"]
        .clone()
}

#[test]
fn edges_of_bodies_are_rails_and_closed_ones_give_a_tube() {
    let dir = workdir("developable", "edges");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    // Two opposite edges of a block along x: one plane through them.
    built(
        &mut s,
        "feature.block",
        json!({"size": ["4 mm", "3 mm", "2 mm"], "name": "Box"}),
    );
    let low = edge_between(&mut s, "Box", [0.0, 0.0, 0.0], [4.0, 0.0, 0.0]);
    let high = edge_between(&mut s, "Box", [0.0, 3.0, 2.0], [4.0, 3.0, 2.0]);
    built(
        &mut s,
        "feature.developable",
        json!({"a": {"edges": [low]}, "b": {"edges": [high]}, "name": "Diagonal"}),
    );
    near(
        number(&body(&mut s, "Diagonal")["area"]),
        4.0 * 13_f64.sqrt(),
        1e-9,
        "the diagonal plane's area",
    );
    // A cylinder's rims are closed curves: the tube between them is the
    // cylinder's side (refused until 2026-10-28, when tubes were built).
    built(
        &mut s,
        "feature.cylinder",
        json!({"radius": "2 mm", "height": "3 mm", "position": ["10 mm", 0, 0], "name": "Can"}),
    );
    let edges = run(&mut s, "document.edges", json!({"feature": "Can"}));
    let rims: Vec<Value> = edges["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].clone())
        .collect();
    assert_eq!(rims.len(), 2, "{edges}");
    built(
        &mut s,
        "feature.developable",
        json!({"a": {"edges": [rims[0]]}, "b": {"edges": [rims[1]]}, "name": "Tube"}),
    );
    near(
        number(&body(&mut s, "Tube")["area"]),
        2.0 * std::f64::consts::PI * 2.0 * 3.0,
        1e-6,
        "the cylinder's side",
    );
}
