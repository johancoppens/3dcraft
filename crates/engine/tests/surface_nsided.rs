//! N-sided fills through the commands (docs/design/surfacing.md › *The
//! N-sided fill*): a sphere's octant, its three quarter circles held
//! tangent to the sphere, is filled by three regular patches whose seams
//! and edges are proven G1, close to the octant's area; inner seams asked
//! in curvature say they are built tangent; a flat pentagon is filled by
//! its plane (its area exactly) and plated; the bodies survive `.3dcraft`
//! and STEP; bad input is an error. At three scales.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::checked::{body, built, error, new_document, run, session_in, workdir};
use craft3d_engine::Session;
use craft3d_math::Portable;
use serde_json::{Value, json};
use std::f64::consts::{PI, TAU};

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

/// The octant's three quarter circles of radius `r` about the origin, in
/// the planes xy, yz and zx.
fn octant_arcs(s: &mut Session, r: f64) {
    for (name, plane) in [("A", "xy"), ("B", "yz"), ("C", "zx")] {
        sketch(s, name, plane, 0.0);
        run(
            s,
            "sketch.arc",
            json!({"sketch": name, "center": [0.0, 0.0], "start": [r, 0.0], "end": [0.0, r]}),
        );
    }
}

fn orders(made: &Value) -> Vec<String> {
    made["fill"]["seams"]
        .as_array()
        .unwrap_or_else(|| panic!("{made}"))
        .iter()
        .map(|s| s["continuity"]["order"].as_str().unwrap_or("?").to_string())
        .collect()
}

#[test]
fn a_spheres_octant_is_filled_by_three_patches_proven_tangent() {
    for k in SCALES {
        let dir = workdir("surface-nsided", &format!("octant-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let r = 10.0 * k;
        built(
            &mut s,
            "feature.sphere",
            json!({"radius": mm(r), "name": "Ball"}),
        );
        octant_arcs(&mut s, r);
        let held = json!({"kind": "tangent", "face": "F1:face(0)"});
        let made = built(
            &mut s,
            "feature.surface.nsided",
            json!({"curves": ["A", "B", "C"], "edges": [held, held, held], "name": "Fill"}),
        );
        let found = orders(&made);
        assert_eq!(found.len(), 3, "{made}");
        assert!(found.iter().all(|o| o == "G1" || o == "G2"), "{found:?}");
        for e in made["fill"]["edges"].as_array().unwrap() {
            assert_eq!(e["built"], "tangent", "{e}");
            for half in e["continuity"].as_array().unwrap() {
                let o = half["order"].as_str().unwrap();
                assert!(o == "G1" || o == "G2", "{half}");
            }
        }
        let fill = body(&mut s, "Fill");
        assert_eq!(fill["kind"], "sheet", "{fill}");
        assert_eq!(fill["valid"], true, "{fill}");
        assert_eq!(fill["faceNames"].as_array().unwrap().len(), 3, "{fill}");
        assert!(
            fill["faceNames"][0].as_str().unwrap().contains(":side("),
            "{fill}"
        );
        // Within 2 % of the octant's area (the fill is not the sphere: its
        // largest distance from it is a few thousandths of the radius).
        near(
            number(&fill["area"]),
            4.0 * PI * r * r / 8.0,
            2e-2,
            "the octant",
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
        // Asked in curvature: the octant's long seams exhaust the measure and
        // the fill falls back, said per seam (`geom/tests/network_nsided.rs`).

        // Saved and opened; written to STEP and read back.
        run(
            &mut s,
            "file.save",
            json!({"path": "fill.3dcraft", "overwrite": true}),
        );
        run(&mut s, "file.open", json!({"path": "fill.3dcraft"}));
        let again = body(&mut s, "Fill");
        near(
            number(&again["area"]),
            number(&fill["area"]),
            1e-12,
            "reopened",
        );
        run(
            &mut s,
            "file.export",
            json!({"path": "fill.step", "bodies": ["Fill"]}),
        );
        new_document(&mut s, k);
        run(
            &mut s,
            "file.import",
            json!({"path": "fill.step", "names": ["Back"]}),
        );
        let back = body(&mut s, "Back");
        assert_eq!(back["valid"], true, "{back}");
        near(
            number(&back["area"]),
            number(&fill["area"]),
            1e-7,
            "through STEP",
        );
    }
}

#[test]
fn a_flat_pentagon_is_filled_by_its_plane_and_plated() {
    for k in SCALES {
        let dir = workdir("surface-nsided", &format!("pentagon-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let (rr, z) = (10.0 * k, 3.0 * k);
        let pts: Vec<[f64; 2]> = (0..5)
            .map(|i| {
                let a = i as f64 * TAU / 5.0;
                [rr * a.pcos(), rr * a.psin()]
            })
            .collect();
        let names = ["P0", "P1", "P2", "P3", "P4"];
        for i in 0..5 {
            sketch(&mut s, names[i], "xy", z);
            run(
                &mut s,
                "sketch.line",
                json!({"sketch": names[i], "from": pts[i], "to": pts[(i + 1) % 5]}),
            );
        }
        let made = built(
            &mut s,
            "feature.surface.nsided",
            json!({"curves": names, "name": "Fill"}),
        );
        assert_eq!(orders(&made).len(), 5, "{made}");
        let fill = body(&mut s, "Fill");
        assert_eq!(fill["valid"], true, "{fill}");
        let area = 5.0 / 2.0 * rr * rr * (TAU / 5.0).psin();
        near(number(&fill["area"]), area, 1e-9, "the pentagon");
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
        // A plate on one of the five patches: its area times the thickness.
        // Refused before the plate's continuation ladder (the high-degree
        // patches continued 7 mm past 8 mm patches turned over: "pole or
        // corner drawn into a point"). At one scale and outside the checked
        // runner: in a debug build a plate of these patches is slow (all
        // five are measured in release, docs/design/surfacing.md; the
        // ladder itself is pinned in kernel/tests/plate.rs).
        if k != 1.0 {
            continue;
        }
        let face = fill["faceNames"][0].clone();
        let t = 1.0 * k;
        let made = common::plain::run(
            &mut s,
            "feature.plate",
            json!({"faces": [face], "thickness": mm(t), "name": "Slab"}),
        );
        assert_eq!(made["status"], "ok", "{made}");
        let bodies = common::plain::run(&mut s, "document.inspect", json!({}))["bodies"].clone();
        let find = |name: &str| {
            bodies
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["name"] == name)
                .unwrap()
                .clone()
        };
        let slab = find("Slab");
        assert_eq!(slab["valid"], true, "{slab}");
        near(number(&slab["volume"]), area / 5.0 * t, 1e-6, "the slab");
        // Mirrored and turned, its names are carried as copies.
        common::plain::run(
            &mut s,
            "feature.mirror",
            json!({"source": "Fill", "normal": [0, 0, 1], "name": "Below"}),
        );
        let turned = common::plain::run(
            &mut s,
            "feature.transform",
            json!({"source": "Below", "angle": "5 deg", "axis": [0, 0, 1], "pivot": [0, 0, 0], "name": "Turned"}),
        );
        assert_eq!(turned["status"], "ok", "{turned}");
    }
}

#[test]
fn bad_fill_parameters_are_errors() {
    let dir = workdir("surface-nsided", "params");
    let mut s = session_in(&dir);
    new_document(&mut s, 1.0);
    octant_arcs(&mut s, 10.0);
    for params in [
        json!({}),
        json!({"curves": []}),
        json!({"curves": ["A", "B"]}),
        json!({"curves": ["A", "B", "C", "A"]}),
        json!({"curves": ["A", "B", "Nope"]}),
        json!({"curves": ["A", "B", "C"], "edges": []}),
        json!({"curves": ["A", "B", "C"], "edges": [1, 2, 3]}),
        json!({"curves": ["A", "B", "C"], "edges": [{"kind": "tangent"}, {}, {}]}),
        json!({"curves": ["A", "B", "C"], "inner": "smooth"}),
        json!({"curves": ["A", "B", "C"], "bogus": true}),
        json!({"curves": "A"}),
    ] {
        let message = error(&mut s, "feature.surface.nsided", params.clone());
        assert!(
            message.contains("feature.surface.nsided"),
            "{params}: {message}"
        );
    }
}

/// A cap of `n` great-circle arcs of the sphere of radius `r` about the
/// origin, `polar` from its +z pole: each arc drawn on a datum plane
/// through the centre and its two ends, in that plane's own frame.
fn great_cap(s: &mut Session, r: f64, n: usize, polar: f64) -> Vec<String> {
    let pts: Vec<[f64; 3]> = (0..n)
        .map(|i| {
            let a = i as f64 * TAU / n as f64;
            [
                r * polar.psin() * a.pcos(),
                r * polar.psin() * a.psin(),
                r * polar.pcos(),
            ]
        })
        .collect();
    let mut names = Vec::new();
    for i in 0..n {
        let (p, q) = (pts[i], pts[(i + 1) % n]);
        let plane = format!("Through{i}");
        let mm3 = |v: [f64; 3]| json!([mm(v[0]), mm(v[1]), mm(v[2])]);
        built(
            s,
            "datum.plane",
            json!({"through": [mm3([0.0, 0.0, 0.0]), mm3(p), mm3(q)], "name": plane}),
        );
        let name = format!("Arc{i}");
        built(s, "sketch.create", json!({"plane": plane, "name": name}));
        let frame = run(s, "sketch.inspect", json!({"sketch": name}))["frame"].clone();
        let v = |key: &str| -> [f64; 3] {
            let a = frame[key].as_array().unwrap();
            [number(&a[0]), number(&a[1]), number(&a[2])]
        };
        let (o, x, y) = (v("origin"), v("x"), v("y"));
        let local = |w: [f64; 3]| {
            let d = [w[0] - o[0], w[1] - o[1], w[2] - o[2]];
            [
                d[0] * x[0] + d[1] * x[1] + d[2] * x[2],
                d[0] * y[0] + d[1] * y[1] + d[2] * y[2],
            ]
        };
        let (c, a, b) = (local([0.0, 0.0, 0.0]), local(p), local(q));
        // Counter-clockwise the short way.
        let turn = (a[0] - c[0]) * (b[1] - c[1]) - (a[1] - c[1]) * (b[0] - c[0]);
        let (from, to) = if turn > 0.0 { (a, b) } else { (b, a) };
        run(
            s,
            "sketch.arc",
            json!({"sketch": name, "center": c, "start": from, "end": to}),
        );
        names.push(name);
    }
    names
}

#[test]
fn a_spherical_cap_held_in_curvature_is_proven_g2_everywhere() {
    for k in SCALES {
        let dir = workdir("surface-nsided", &format!("cap-{k}"));
        let mut s = session_in(&dir);
        new_document(&mut s, k);
        let r = 10.0 * k;
        built(
            &mut s,
            "feature.sphere",
            json!({"radius": mm(r), "name": "Ball"}),
        );
        let ball = run(&mut s, "document.inspect", json!({}))["features"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let curves = great_cap(&mut s, r, 3, 0.3);
        let held = json!({"kind": "curvature", "face": format!("{ball}:face(0)")});
        let made = built(
            &mut s,
            "feature.surface.nsided",
            json!({"curves": curves, "edges": [held, held, held], "inner": "curvature", "name": "Fill"}),
        );
        for seam in made["fill"]["seams"]
            .as_array()
            .unwrap_or_else(|| panic!("{made}"))
        {
            assert_eq!(seam["built"], "curvature", "{seam}");
            assert_eq!(seam["continuity"]["order"], "G2", "{seam}");
        }
        for e in made["fill"]["edges"].as_array().unwrap() {
            assert_eq!(e["built"], "curvature", "{e}");
            for half in e["continuity"].as_array().unwrap() {
                assert_eq!(half["order"], "G2", "{half}");
            }
        }
        let fill = body(&mut s, "Fill");
        assert_eq!(fill["valid"], true, "{fill}");
        // The cap's area: the spherical triangle's, R² times its excess
        // (the fill is within a few millionths of the radius of the
        // sphere).
        let polar: f64 = 0.3;
        let pts: Vec<[f64; 3]> = (0..3)
            .map(|i| {
                let a = i as f64 * TAU / 3.0;
                [
                    polar.psin() * a.pcos(),
                    polar.psin() * a.psin(),
                    polar.pcos(),
                ]
            })
            .collect();
        let mut excess = -PI;
        for i in 0..3 {
            let (p, a, b) = (pts[i], pts[(i + 2) % 3], pts[(i + 1) % 3]);
            let off = |q: [f64; 3]| {
                let d = q[0] * p[0] + q[1] * p[1] + q[2] * p[2];
                [q[0] - d * p[0], q[1] - d * p[1], q[2] - d * p[2]]
            };
            let (u, v) = (off(a), off(b));
            let dot = u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
            let len = |w: [f64; 3]| (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
            excess += (dot / (len(u) * len(v))).pacos();
        }
        near(number(&fill["area"]), r * r * excess, 1e-4, "the cap");
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
    }
}
