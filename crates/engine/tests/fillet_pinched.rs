//! A fillet whose section runs to nothing at a place inside its edge,
//! through the commands (docs/design/blends.md › *A fillet pinched inside
//! its edge*): a loft through "house" sections whose ridge comes down to
//! the eaves inside the loft has two roof faces that are tangent there.
//! `feature.fillet` of the ridge answered `ok` in 0.1.2, with a body whose
//! fillet face had no width at that place (`document.inspect` called it
//! valid, `document.check` could not measure it, and at a radius of 3 mm
//! it could not be exported). It is refused now, with the place in the
//! document's coordinates; the chamfer of the same edge builds as it did.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::plain::{body, built, failure, run, session};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// Half the width of the house and the height of its eaves.
const W: f64 = 10.0;
const H: f64 = 8.0;

/// The loft `Body` through house sections `step` apart along x, the ridge
/// `heights[i]` above the eaves at section `i`; the names of the two roof
/// faces, which the ridge edge lies between.
fn ridge(s: &mut Session, k: f64, heights: &[f64], step: f64, ruled: bool) -> Value {
    run(s, "file.new", json!({"units": "mm", "tolerance": 1e-6 * k}));
    let mut names = Vec::new();
    for (i, a) in heights.iter().enumerate() {
        let name = format!("S{i}");
        run(
            s,
            "sketch.create",
            json!({"name": name, "plane": "yz", "offset": i as f64 * step * k}),
        );
        let points: Vec<[f64; 2]> = [[-W, 0.0], [W, 0.0], [W, H], [0.0, H + a], [-W, H]]
            .iter()
            .map(|p| [p[0] * k, p[1] * k])
            .collect();
        run(
            s,
            "sketch.polyline",
            json!({"sketch": name, "points": points, "closed": true}),
        );
        names.push(name);
    }
    built(
        s,
        "feature.loft",
        json!({"sections": names, "ruled": ruled, "name": "Body"}),
    );
    let listing = run(s, "document.edges", json!({"feature": "Body"}));
    let along = (heights.len() - 1) as f64 * step * k;
    let at = |point: &Value, i: usize| point[i].as_f64().unwrap();
    listing["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            let (start, end) = (&e["start"], &e["end"]);
            at(start, 1).abs() <= 1e-6 * k
                && at(end, 1).abs() <= 1e-6 * k
                && at(start, 2) > 0.2 * H * k
                && (at(end, 0) - at(start, 0)).abs() >= 0.99 * along
        })
        .unwrap_or_else(|| panic!("no ridge in {listing}"))["faces"]
        .clone()
}

#[test]
fn a_fillet_of_a_ridge_whose_faces_become_tangent_inside_is_refused_with_the_place() {
    for k in [1e-3, 1.0, 1e6] {
        let mut s = session();
        // The ridge 6, 2, 0, 2: tangent at the third section (x = 30),
        // below the eaves for a little over two millimetres after it.
        let roofs = ridge(&mut s, k, &[6.0, 2.0, 0.0, 2.0], 15.0, false);
        let base = body(&mut s, "Body")["volume"].as_f64().unwrap();
        for r in [0.3, 1.0, 3.0] {
            let made = run(
                &mut s,
                "feature.fillet",
                json!({"edges": [roofs], "radius": r * k, "name": "Round"}),
            );
            assert_eq!(made["status"], "failed", "radius {r}: {made}");
            let said = failure(&mut s, "Round");
            assert!(said.contains("edges.0 (between "), "{said}");
            assert!(
                said.contains("convex on one side of (")
                    && said.contains(" mm and concave on the other"),
                "{said}"
            );
            assert!(said.contains("the first of 2 such places"), "{said}");
            assert!(said.contains("whatever the radius"), "{said}");
            assert!(!said.contains("internal error"), "{said}");
            if k == 1.0 {
                assert!(said.contains("of (30, 0, 8) mm"), "{said}");
            }
            run(&mut s, "feature.delete", json!({"feature": "Round"}));
        }
        // The chamfer of the same edge builds as it did, through the place.
        built(
            &mut s,
            "feature.chamfer",
            json!({"edges": [roofs], "distance": k, "name": "Bevel"}),
        );
        let bevelled = body(&mut s, "Bevel");
        assert_eq!(bevelled["valid"], true, "{bevelled}");
        let removed = base - bevelled["volume"].as_f64().unwrap();
        // The section in closed form (the kernel's `fillet_pinched` test).
        let expected = 6.970_097_067_6 * k * k * k;
        assert!(
            (removed - expected).abs() <= 2.25e-5 * k * k * k,
            "{removed} against {expected}"
        );
        let check = run(&mut s, "document.check", json!({"deep": true}));
        assert_eq!(check["ok"], true, "{check}");
    }
}

#[test]
fn an_edge_tangent_at_its_middle_only_is_not_called_tangent_along_itself() {
    let k = 1.0;
    let mut s = session();
    // 6, 0, 6: tangent at the middle section, convex on both sides.
    let roofs = ridge(&mut s, k, &[6.0, 0.0, 6.0], 25.0, false);
    run(
        &mut s,
        "feature.fillet",
        json!({"edges": [roofs], "radius": 1.0, "name": "Round"}),
    );
    let said = failure(&mut s, "Round");
    assert!(
        said.contains("the two faces become tangent at (25, 0, 8) mm, a place inside the edge"),
        "{said}"
    );
    assert!(!said.contains("tangent along"), "{said}");
    run(&mut s, "feature.delete", json!({"feature": "Round"}));
    run(
        &mut s,
        "feature.chamfer",
        json!({"edges": [roofs], "distance": 1.0, "name": "Bevel"}),
    );
    let said = failure(&mut s, "Bevel");
    assert!(
        said.contains(
            "the two faces are tangent at the middle of the edge, at (25, 0, 8) mm, though not all along it"
        ),
        "{said}"
    );
    run(&mut s, "feature.delete", json!({"feature": "Bevel"}));
    // 6, 0, −6: from convex to concave at the middle.
    let roofs = ridge(&mut s, k, &[6.0, 0.0, -6.0], 25.0, false);
    run(
        &mut s,
        "feature.fillet",
        json!({"edges": [roofs], "radius": 1.0, "name": "Round"}),
    );
    let said = failure(&mut s, "Round");
    assert!(
        said.contains("convex on one side of (25, 0, 8) mm and concave on the other"),
        "{said}"
    );
    // 6, 3, −3: convex at the middle, concave near the far end. It was an
    // internal error.
    let roofs = ridge(&mut s, k, &[6.0, 3.0, -3.0], 25.0, false);
    run(
        &mut s,
        "feature.fillet",
        json!({"edges": [roofs], "radius": 1.0, "name": "Round"}),
    );
    let said = failure(&mut s, "Round");
    assert!(
        said.contains("convex on one side of (39.0487, 0, 8) mm and concave on the other"),
        "{said}"
    );
    assert!(!said.contains("internal error"), "{said}");
}

/// A ridge that stops short of the eaves at its far end: the fillet
/// narrows to its end and is built, as before, against the volume the
/// kernel's test works out from the ball's definition.
#[test]
fn a_fillet_that_narrows_to_the_end_of_its_edge_is_built() {
    let k = 1.0;
    let mut s = session();
    let roofs = ridge(&mut s, k, &[6.0, 0.0005], 40.0, true);
    let base = body(&mut s, "Body")["volume"].as_f64().unwrap();
    built(
        &mut s,
        "feature.fillet",
        json!({"edges": [roofs], "radius": 1.0, "name": "Round"}),
    );
    let rounded = body(&mut s, "Round");
    assert_eq!(rounded["valid"], true, "{rounded}");
    let removed = base - rounded["volume"].as_f64().unwrap();
    assert!((removed - 0.621_426_919_3).abs() <= 1.5e-5, "{removed}");
    let check = run(&mut s, "document.check", json!({"deep": true}));
    assert_eq!(check["ok"], true, "{check}");
}
