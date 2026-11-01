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

#[test]
fn a_wire_of_many_edges_is_put_in_order_in_about_its_length() {
    use craft3d_topo::{Coedge, EdgeId, VertexId};
    // Review of 2026-11-02: the order was found by scanning every coedge
    // at every step. A polygon of 100 000 edges, listed backwards and
    // interleaved, in well under a second.
    let n = 100_000u32;
    let mut uses: Vec<(Coedge, VertexId, VertexId)> = (0..n)
        .map(|i| {
            (
                Coedge {
                    edge: EdgeId(i),
                    reversed: false,
                },
                VertexId(i),
                VertexId((i + 1) % n),
            )
        })
        .collect();
    uses.reverse();
    let (even, odd): (Vec<_>, Vec<_>) = uses.iter().enumerate().partition(|(k, _)| k % 2 == 0);
    let uses: Vec<_> = even.into_iter().chain(odd).map(|(_, u)| *u).collect();
    let started = std::time::Instant::now();
    let loops = shape::tests_chain(&uses).unwrap();
    assert!(started.elapsed().as_secs_f64() < 2.0);
    assert_eq!(loops.len(), 1);
    assert_eq!(loops[0].len(), n as usize);
    // A cylinder's wire as FreeCAD lists it: seam, seam reversed, bottom
    // circle reversed, top circle: one loop that crosses the seam twice.
    let c = |e: u32, r: bool| Coedge {
        edge: EdgeId(e),
        reversed: r,
    };
    let (b, t) = (VertexId(0), VertexId(1));
    let wire = [
        (c(0, false), b, t),
        (c(0, true), t, b),
        (c(1, true), b, b),
        (c(2, false), t, t),
    ];
    let loops = shape::tests_chain(&wire).unwrap();
    assert_eq!(loops.len(), 1);
    let edges: Vec<u32> = loops[0].iter().map(|c| c.edge.0).collect();
    assert_eq!(edges.len(), 4);
    assert_ne!(edges[0], edges[1]);
    // An open wire is no loop.
    assert!(shape::tests_chain(&wire[..1]).is_none());
}

/// An object of a test document: name, type, visible, shape file, links
/// as (property, targets), placement x.
type TestObject<'a> = (
    &'a str,
    &'a str,
    bool,
    Option<&'a str>,
    &'a [(&'a str, &'a [&'a str])],
    f64,
);

/// A FreeCAD document of objects `(name, type, label, visible, shape file,
/// links as (property, targets), placement x)`, with `files` beside it,
/// as a stored archive.
fn document(objects: &[TestObject<'_>], files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut list = String::new();
    let mut data = String::new();
    for (name, kind, visible, file, links, x) in objects {
        list.push_str(&format!("<Object type=\"{kind}\" name=\"{name}\"/>\n"));
        let mut props = format!(
            "<Property name=\"Label\" type=\"App::PropertyString\"><String value=\"{name} label\"/></Property>\n<Property name=\"Visibility\" type=\"App::PropertyBool\"><Bool value=\"{visible}\"/></Property>\n<Property name=\"Placement\" type=\"App::PropertyPlacement\"><PropertyPlacement Px=\"{x}\" Py=\"0\" Pz=\"0\" Q0=\"0\" Q1=\"0\" Q2=\"0\" Q3=\"1\"/></Property>\n"
        );
        if let Some(f) = file {
            props.push_str(&format!(
                "<Property name=\"Shape\" type=\"Part::PropertyPartShape\"><Part file=\"{f}\"/></Property>\n"
            ));
        }
        for (prop, targets) in links.iter() {
            props.push_str(&format!(
                "<Property name=\"{prop}\" type=\"App::PropertyLinkList\"><LinkList count=\"{}\">",
                targets.len()
            ));
            for t in targets.iter() {
                props.push_str(&format!("<Link value=\"{t}\"/>"));
            }
            props.push_str("</LinkList></Property>\n");
        }
        data.push_str(&format!(
            "<Object name=\"{name}\"><Properties>{props}</Properties></Object>\n"
        ));
    }
    let xml = format!(
        "<?xml version='1.0' encoding='utf-8'?>\n<Document SchemaVersion=\"4\"><Objects>{list}</Objects><ObjectData>{data}</ObjectData></Document>"
    );
    let mut entries: Vec<(&str, &[u8])> = vec![("Document.xml", xml.as_bytes())];
    entries.extend_from_slice(files);
    craft3d_zipstore::write_stored(&entries).unwrap()
}

#[test]
fn only_consuming_links_make_an_object_a_step() {
    let shape = data("moved.brp");
    let s = shape.as_slice();
    let files: [(&str, &[u8]); 1] = [("S.brp", s)];
    // A sketch in a second body refers to the first (ExternalGeometry):
    // the first is still a result. A hidden cut leaves its operands seen.
    let bytes = document(
        &[
            ("Body1", "PartDesign::Body", true, Some("S.brp"), &[], 0.0),
            (
                "Sketch",
                "Sketcher::SketchObject",
                true,
                None,
                &[("ExternalGeometry", &["Body1"])],
                0.0,
            ),
            (
                "Body2",
                "PartDesign::Body",
                true,
                Some("S.brp"),
                &[("Group", &["Sketch"])],
                50.0,
            ),
            ("A", "Part::Box", true, Some("S.brp"), &[], 100.0),
            ("B", "Part::Box", true, Some("S.brp"), &[], 200.0),
            (
                "Cut",
                "Part::Cut",
                false,
                Some("S.brp"),
                &[("Base", &["A"]), ("Tool", &["B"])],
                0.0,
            ),
            ("C", "Part::Box", true, Some("S.brp"), &[], 300.0),
            (
                "Fillet",
                "Part::Fillet",
                true,
                Some("S.brp"),
                &[("Base", &["C"])],
                0.0,
            ),
        ],
        &files,
    );
    let parsed = parse(&bytes).unwrap();
    let objects: Vec<&str> = parsed.items().iter().map(|i| i.object.as_str()).collect();
    assert_eq!(objects, ["Body1", "Body2", "A", "B", "Fillet"]);
    let skipped = parsed.skipped().join("; ");
    assert!(
        skipped.contains("Cut") && skipped.contains("hidden"),
        "{skipped}"
    );
    assert!(
        skipped.contains("`C label` (C): used by `Fillet label`"),
        "{skipped}"
    );
}

#[test]
fn a_group_inside_a_part_is_placed_by_the_part() {
    let shape = data("moved.brp");
    let files: [(&str, &[u8]); 1] = [("S.brp", shape.as_slice())];
    let bytes = document(
        &[
            (
                "Part",
                "App::Part",
                true,
                None,
                &[("Group", &["Folder"])],
                100.0,
            ),
            (
                "Folder",
                "App::DocumentObjectGroup",
                true,
                None,
                &[("Group", &["Box"])],
                0.0,
            ),
            ("Box", "Part::Feature", true, Some("S.brp"), &[], 0.0),
        ],
        &files,
    );
    let parsed = parse(&bytes).unwrap();
    let placed = parsed.build(0, &options()).unwrap();
    let alone = parse(&shape).unwrap().build(0, &options()).unwrap();
    let (a, _) = placed.body.bbox().unwrap().corners().unwrap();
    let (b, _) = alone.body.bbox().unwrap().corners().unwrap();
    assert!((a.x - b.x - 100.0).abs() < 1e-9, "{a:?} {b:?}");
}

#[test]
fn an_entry_many_objects_use_is_decompressed_once_within_a_budget() {
    // Review of 2026-11-02: each object decompressed and parsed its entry
    // again, with no bound on the whole: 1000 objects on one entry. Here
    // within a budget of twice the entry, which only one decompression
    // and one parse keep to.
    let mut shape = data("moved.brp");
    shape.extend(std::iter::repeat_n(b' ', 50_000));
    let files: [(&str, &[u8]); 1] = [("S.brp", shape.as_slice())];
    let names: Vec<String> = (0..1000).map(|k| format!("O{k}")).collect();
    let objects: Vec<TestObject<'_>> = names
        .iter()
        .map(|n| {
            (
                n.as_str(),
                "Part::Feature",
                true,
                Some("S.brp"),
                &[][..],
                0.0,
            )
        })
        .collect();
    let bytes = document(&objects, &files);
    let started = std::time::Instant::now();
    let parsed = parse_within(&bytes, 2 * shape.len() + 1_000_000).unwrap();
    assert_eq!(parsed.items().len(), 1000);
    assert!(started.elapsed().as_secs() < 30);
    // And a budget below what the entries need is refused, named.
    let e = parse_within(&bytes, shape.len() / 2).unwrap_err();
    assert!(e.0.contains("decompress to more than"), "{e}");
}

#[test]
fn a_torus_whose_tube_reaches_its_axis_is_read_by_the_spindle_code() {
    let axes = craft3d_brep::Axes {
        origin: [0.0; 3],
        n: [0.0, 0.0, 1.0],
        x: [1.0, 0.0, 0.0],
        y: [0.0, 1.0, 0.0],
    };
    let torus = |major, minor| craft3d_brep::Surface::Torus { axes, major, minor };
    assert_eq!(
        shape::tests_surface_kind(&torus(3.0, 1.0)).unwrap(),
        "torus"
    );
    assert_eq!(
        shape::tests_surface_kind(&torus(2.0, 3.0)).unwrap(),
        "spindle"
    );
    assert_eq!(
        shape::tests_surface_kind(&torus(2.0, 2.0)).unwrap(),
        "spindle"
    );
}
