#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use super::*;
use craft3d_math::Portable;

fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn a_periodic_bspline_is_the_curve_freecad_evaluates() {
    let brep = craft3d_brep::read(&data("periodic.brp")).unwrap();
    let c = brep.curve(1).unwrap();
    let craft3d_brep::Curve::BSpline(b) = c else {
        panic!("{c:?}")
    };
    assert!(b.periodic);
    let expected: serde_json::Value = serde_json::from_slice(&data("expected.json")).unwrap();
    let place = shape::tests_place();
    let (curve, _) = shape::tests_curve(c, &place, (0.0, 1.0)).unwrap();
    for row in expected["periodic"]["points"].as_array().unwrap() {
        let t = row[0].as_f64().unwrap();
        let p: Vec<f64> = row[1]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect();
        let q = curve.eval(t);
        let d = ((q.x - p[0]).ppowi(2) + (q.y - p[1]).ppowi(2) + (q.z - p[2]).ppowi(2)).sqrt();
        assert!(d < 1e-9, "t {t}: {q:?} against {p:?} ({d:e})");
    }
}

fn options() -> Options {
    Options {
        metres_per_unit: 0.001,
        tolerance: craft3d_math::Tolerance::new(1e-6, 1e-9).unwrap(),
        loosest: 1e-3,
    }
}

#[test]
fn a_composite_location_applies_its_parts_in_order() {
    // A box placed, then moved again, then put in a placed compound: the
    // file's location 4 is `2 2 1 3 1 0`. FreeCAD's box of the result.
    let bytes = data("moved.brp");
    assert!(is_brep(&bytes) && !is_fcstd(&bytes));
    let parsed = parse(&bytes).unwrap();
    assert_eq!(parsed.items().len(), 1);
    let read = parsed.build(0, &options()).unwrap();
    let expected: serde_json::Value = serde_json::from_slice(&data("expected.json")).unwrap();
    let row = &expected["moved.brp"][0];
    let (lo, hi) = read.body.bbox().unwrap().corners().unwrap();
    let b: Vec<f64> = row["bbox"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect();
    for (k, (ours, theirs)) in [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]
        .iter()
        .zip(&b)
        .enumerate()
    {
        assert!(
            (ours - theirs).abs() < 1e-9,
            "bbox[{k}]: {ours} against {theirs}"
        );
    }
    let mass = craft3d_topo::mass_properties(&read.body).unwrap();
    assert!((mass.volume - 6.0).abs() < 1e-9);
}

/// Every cut of a file and every byte changed: an error or a body, never
/// a panic.
fn never_panics(bytes: &[u8], step: usize) {
    for at in (0..bytes.len()).step_by(step) {
        if let Ok(p) = parse(bytes.get(..at).unwrap()) {
            for i in 0..p.items().len() {
                let _ = p.build(i, &options());
            }
        }
        let mut b = bytes.to_vec();
        b[at] = b[at].wrapping_add(17);
        if let Ok(p) = parse(&b) {
            for i in 0..p.items().len() {
                let _ = p.build(i, &options());
            }
        }
    }
}

#[test]
fn damaged_brep_files_are_errors_never_panics() {
    never_panics(&data("moved.brp"), 1);
    never_panics(&data("periodic.brp"), 1);
}

#[test]
fn damaged_documents_are_errors_never_panics() {
    // The deflated entries make most changes a CRC or inflate error; a
    // stored copy of the archive lets the changes reach the XML and the
    // BRep text.
    let bytes = data("box.FCStd");
    never_panics(&bytes, 7);
    let entries = craft3d_zipstore::read(&bytes).unwrap();
    let owned: Vec<(String, Vec<u8>)> = entries
        .iter()
        .map(|e| (e.name.to_string(), e.data().unwrap().into_owned()))
        .collect();
    let refs: Vec<(&str, &[u8])> = owned
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let stored = craft3d_zipstore::write_stored(&refs).unwrap();
    assert_eq!(parse(&stored).unwrap().items().len(), 1);
    // Only the entries' own bytes are changed (a changed header is the
    // archive's business, tested in craft3d-zipstore); the CRC is
    // rewritten each time, so the change reaches the reader.
    for (k, (name, data)) in owned.iter().enumerate() {
        for at in (0..data.len()).step_by(3) {
            let mut changed = owned.clone();
            changed[k].1[at] = changed[k].1[at].wrapping_add(13);
            let refs: Vec<(&str, &[u8])> = changed
                .iter()
                .map(|(n, d)| (n.as_str(), d.as_slice()))
                .collect();
            let file = craft3d_zipstore::write_stored(&refs).unwrap();
            if let Ok(p) = parse(&file) {
                for i in 0..p.items().len() {
                    let _ = p.build(i, &options());
                }
            }
            let _ = name;
        }
    }
}

#[test]
fn hand_written_brep_edge_cases() {
    // A shape that refers to itself, to a later shape, to a location that
    // does not exist; a composite location of a later one; absurd counts;
    // a binary file.
    let head = "CASCADE Topology V1, (c) Matra-Datavision\n";
    for bad in [
        format!(
            "{head}Locations 0\nCurve2ds 0\nCurves 0\nSurfaces 0\nTShapes 1\nCo\n\n0000000\n+1 0 *\n\n+1 0"
        ),
        format!("{head}Locations 0\nTShapes 2\nCo\n\n0000000\n+1 0 *\nCo\n\n0000000\n*\n\n+1 0"),
        format!("{head}Locations 0\nTShapes 1\nCo\n\n0000000\n*\n\n+1 5"),
        format!("{head}Locations 1\n2 1 1 0\nTShapes 0\n"),
        format!("{head}Locations 99999999999\n"),
        format!("{head}Locations 0\nCurves 1\n7 0 0 3 4000000000 2\n"),
        format!("{head}Locations 0\nCurves 1\n7 0 0 99 4 2\n"),
        format!("{head}Locations 0\nCurves 1\n2 0 0 0 0 0 1 1 0 0 0 1 0 -1\n"),
        format!("{head}Locations 0\nCurves 1\n1 nan 0 0 1 0 0\n"),
        format!(
            "{head}Locations 0\nCurves 1\n8 0 1 8 0 1 8 0 1 8 0 1 8 0 1 8 0 1 8 0 1 8 0 1 8 0 1 8 0 1 1 0 0 0 1 0 0\n"
        ),
        "Open CASCADE Topology V1\u{0}\u{1}".to_string(),
        String::new(),
    ] {
        assert!(craft3d_brep::read(bad.as_bytes()).is_err(), "{bad}");
        assert!(parse(bad.as_bytes()).is_err(), "{bad}");
    }
    // An empty shape reads, with nothing to import.
    let empty = format!("{head}Locations 0\nTShapes 1\nCo\n\n0000000\n*\n\n+1 0");
    assert!(parse(empty.as_bytes()).unwrap().items().is_empty());
    // A degenerate edge on its own does not become an edge; an edge
    // without a 3D curve that is not degenerate is refused, named.
    let no_curve = format!(
        "{head}Locations 0\nCurves 0\nSurfaces 1\n1 0 0 0 0 0 1 1 0 0 0 1 0\nTShapes 6\nVe\n1e-7\n0 0 0\n0 0\n\n0101101\n*\nVe\n1e-7\n1 0 0\n0 0\n\n0101101\n*\nEd\n 1e-7 1 1 0\n0\n\n0101000\n+6 0 -5 0 *\nWi\n\n0101100\n+4 0 -4 0 *\nFa\n0 1e-7 1 0\n\n0101000\n+3 0 *\nSh\n\n0101000\n+2 0 *\n\n+1 0"
    );
    let parsed = parse(no_curve.as_bytes()).unwrap();
    let e = parsed.build(0, &options()).unwrap_err();
    assert!(e.0.contains("no 3D curve"), "{e}");
}
