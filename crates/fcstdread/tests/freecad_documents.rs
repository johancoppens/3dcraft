//! FreeCAD documents made by FreeCAD itself (`tests/data/make_fixtures.py`)
//! read back, against what FreeCAD says of the same objects
//! (`tests/data/expected.json`, written by the same script): which
//! objects are imported, their labels and colours, and each body's
//! volume, area and bounding box.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unreachable
)]

use craft3d_fcstdread::{Options, parse};
use craft3d_math::{Portable, Tolerance};
use serde_json::Value;

fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn expected() -> Value {
    serde_json::from_slice(&data("expected.json")).unwrap()
}

fn options() -> Options {
    let tolerance = Tolerance::new(1e-6, 1e-9).unwrap();
    Options {
        metres_per_unit: 0.001,
        tolerance,
        loosest: craft3d_math::DEFAULT_LOOSEST * 1e-6,
    }
}

fn close(a: f64, b: f64, rel: f64, what: &str) {
    let scale = b.abs().max(1.0);
    assert!(
        (a - b).abs() <= rel * scale,
        "{what}: read {a}, FreeCAD {b} (relative {:e})",
        (a - b).abs() / scale
    );
}

/// Every object FreeCAD listed, in order, read as a body with its
/// volume, area and box. `rel` is how close the measures must be: 1e-6
/// where the geometry is exact here as in FreeCAD.
fn check(file: &str, rel: f64) {
    let expected = expected();
    let rows = expected[file].as_array().unwrap();
    let parsed = parse(&data(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
    let items = parsed.items();
    assert_eq!(
        items.iter().map(|i| i.label.clone()).collect::<Vec<_>>(),
        rows.iter()
            .map(|r| r["label"].as_str().unwrap().to_string())
            .collect::<Vec<_>>(),
        "{file}: the objects imported"
    );
    for (index, row) in rows.iter().enumerate() {
        let read = parsed
            .build(index, &options())
            .unwrap_or_else(|e| panic!("{file} item {index}: {e}"));
        let label = row["label"].as_str().unwrap();
        assert_eq!(read.name, label);
        assert_eq!(
            read.body.faces.len() as u64,
            row["faces"].as_u64().unwrap(),
            "{label}: faces"
        );
        let mass = craft3d_topo::mass_properties(&read.body).unwrap();
        close(
            mass.volume,
            row["volume"].as_f64().unwrap(),
            rel,
            &format!("{label} volume"),
        );
        close(
            mass.area,
            row["area"].as_f64().unwrap(),
            rel,
            &format!("{label} area"),
        );
        // The tight box of the body, from a mesh within 1e-3 mm of it,
        // against FreeCAD's optimal box.
        let mesh = craft3d_tess::tessellate_body(
            &read.body,
            &craft3d_tess::TessParams {
                chordal_tolerance: 1e-3,
                max_segments: 1 << 16,
                ..Default::default()
            },
        )
        .unwrap();
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for p in &mesh.positions {
            for (k, x) in [p.x, p.y, p.z].into_iter().enumerate() {
                lo[k] = lo[k].min(x);
                hi[k] = hi[k].max(x);
            }
        }
        let b: Vec<f64> = row["bbox"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect();
        for (k, (ours, theirs)) in lo.iter().chain(&hi).zip(&b).enumerate() {
            assert!(
                (ours - theirs).abs() <= 3e-3,
                "{label} bbox[{k}]: read {ours}, FreeCAD {theirs}"
            );
        }
        if let Some(c) = row["color"].as_array() {
            let c: Vec<u8> = c.iter().map(|x| x.as_u64().unwrap() as u8).collect();
            assert_eq!(read.color, Some([c[0], c[1], c[2]]), "{label}: colour");
        }
    }
}

#[test]
fn a_placed_box() {
    check("box.FCStd", 1e-9);
}

#[test]
fn a_cut_is_imported_and_not_its_hidden_inputs() {
    check("cut.FCStd", 1e-6);
}

#[test]
fn fillets_with_spheres_at_the_corners() {
    check("fillet.FCStd", 1e-6);
}

#[test]
fn a_partdesign_body_is_one_body_not_its_features() {
    check("body.FCStd", 1e-6);
}

#[test]
fn revolved_and_periodic_splines() {
    // FreeCAD's own volume of the vase is 1.6e-5 off (its integrator on a
    // surface of revolution); against the volume by Pappus along the
    // profile, which the script integrates, it is exact.
    check("spline.FCStd", 1e-4);
    let parsed = parse(&data("spline.FCStd")).unwrap();
    let vase = parsed.build(0, &options()).unwrap();
    let mass = craft3d_topo::mass_properties(&vase.body).unwrap();
    close(
        mass.volume,
        expected()["vase_pappus"].as_f64().unwrap(),
        1e-9,
        "vase volume by Pappus",
    );
    // The blob is its flat section, a periodic B-spline, extruded 7 mm:
    // its volume is 7 times the section's area by Green's theorem along
    // the curve (which a test of the crate holds to FreeCAD's own points
    // of it), 1e-9 against the body's integrated volume. FreeCAD's own
    // is 4e-5 off.
    let blob = parsed.build(1, &options()).unwrap();
    let edge = blob
        .body
        .edges
        .iter()
        .find(|e| {
            matches!(e.curve, craft3d_geom::Curve3::Nurbs(_))
                && e.curve.eval(e.range.0).z.abs() < 1e-9
        })
        .unwrap();
    let craft3d_geom::Curve3::Nurbs(n) = &edge.curve else {
        unreachable!()
    };
    let (gx, gw) = gauss();
    let mut knots: Vec<f64> = n.knots().to_vec();
    knots.dedup();
    let mut area = 0.0;
    for w in knots.windows(2) {
        let (a, b) = (w[0].max(edge.range.0), w[1].min(edge.range.1));
        if b <= a {
            continue;
        }
        for (x, wt) in gx.iter().zip(&gw) {
            let t = 0.5 * (a + b) + 0.5 * (b - a) * x;
            let (p, d) = n.derivatives(t);
            area += wt * 0.5 * (b - a) * 0.5 * (p.x * d.y - p.y * d.x);
        }
    }
    let mass = craft3d_topo::mass_properties(&blob.body).unwrap();
    close(mass.volume, 7.0 * area.abs(), 1e-9, "blob volume by Green");
}

/// Gauss–Legendre nodes and weights, 24 points (Newton on the Legendre
/// polynomial).
fn gauss() -> (Vec<f64>, Vec<f64>) {
    let n = 24;
    let mut xs = Vec::new();
    let mut ws = Vec::new();
    for i in 0..n {
        let mut x = (std::f64::consts::PI * (i as f64 + 0.75) / (n as f64 + 0.5)).pcos();
        let mut dp = 0.0;
        for _ in 0..100 {
            let (mut p0, mut p1) = (1.0, x);
            for k in 2..=n {
                let p2 = ((2 * k - 1) as f64 * x * p1 - (k - 1) as f64 * p0) / k as f64;
                p0 = p1;
                p1 = p2;
            }
            dp = n as f64 * (x * p1 - p0) / (x * x - 1.0);
            let dx = p1 / dp;
            x -= dx;
            if dx.abs() < 1e-16 {
                break;
            }
        }
        xs.push(x);
        ws.push(2.0 / ((1.0 - x * x) * dp * dp));
    }
    (xs, ws)
}

#[test]
fn several_bodies_with_colours_placements_and_a_hidden_one() {
    check("multi.FCStd", 1e-6);
    let parsed = parse(&data("multi.FCStd")).unwrap();
    assert!(
        parsed.skipped().iter().any(|s| s.contains("Ghost")),
        "{:?}",
        parsed.skipped()
    );
}

#[test]
fn faces_edges_and_vertices_carry_freecads_numbers() {
    let parsed = parse(&data("box.FCStd")).unwrap();
    let read = parsed.build(0, &options()).unwrap();
    let mut faces = read.face_ids.clone();
    faces.sort_unstable();
    assert_eq!(faces, [1, 2, 3, 4, 5, 6]);
    let mut edges = read.edge_ids.clone();
    edges.sort_unstable();
    assert_eq!(edges, (1..=12).collect::<Vec<u64>>());
    let mut vertices = read.vertex_ids.clone();
    vertices.sort_unstable();
    assert_eq!(vertices, (1..=8).collect::<Vec<u64>>());
}

#[test]
fn other_units_scale_every_length() {
    let parsed = parse(&data("box.FCStd")).unwrap();
    let mut o = options();
    o.metres_per_unit = 0.0254;
    o.tolerance = Tolerance::new(1e-6 / 25.4, 1e-9).unwrap();
    o.loosest = craft3d_math::DEFAULT_LOOSEST * 1e-6 / 25.4;
    let read = parsed.build(0, &o).unwrap();
    let mass = craft3d_topo::mass_properties(&read.body).unwrap();
    close(
        mass.volume,
        6000.0 / 25.4f64.ppowi(3),
        1e-9,
        "volume in inches",
    );
    assert_eq!(read.report.length_unit, "millimetre");
}

#[test]
fn reading_the_same_bytes_twice_gives_the_same_body() {
    let bytes = data("fillet.FCStd");
    let a = parse(&bytes).unwrap().build(0, &options()).unwrap();
    let b = parse(&bytes).unwrap().build(0, &options()).unwrap();
    assert_eq!(format!("{:?}", a.body), format!("{:?}", b.body));
}
