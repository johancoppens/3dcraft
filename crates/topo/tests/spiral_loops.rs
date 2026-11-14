//! A thin strip that spirals about a point of its surface's chart is a
//! right face however many turns one of its edges makes
//! (`docs/design/topology.md` › *A strip that spirals about a point of the
//! chart*): until 2026-11-14 a spline edge was followed in eight steps
//! whatever it did, and a side of 16 turns in one edge, two whole turns a
//! step, read as a straight line.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "common/spirals.rs"]
mod spirals;

use craft3d_topo::{Body, Defect, check, forget_passed, mass_properties};
use spirals::{Strip, grounds, spiral_strip, tolerance, turned};

/// What the check says of the loops of a body's faces.
fn loop_defects(body: &Body, k: f64) -> Vec<String> {
    forget_passed();
    check(body, &tolerance(k))
        .iter()
        .filter(|d| {
            matches!(
                d,
                Defect::LoopsBoundNoRegion { .. } | Defect::LoopNotSimple { .. }
            )
        })
        .map(ToString::to_string)
        .collect()
}

/// The smallest strip found refused: on a plane, the spiral from radius
/// 0.45 to 1.65 in 16 turns, the strip 0.35 of the pitch wide, its outer
/// side one edge and its inner side cut into quarter turns ("every loop
/// runs clockwise about the outward normal"), or the other way about ("it
/// winds 2 times about the point …"). Right by its measure: the area is the
/// closed form's.
#[test]
fn the_smallest_spiral_strip_is_a_face() {
    let (_, plane, at, area) = grounds(1.0).swap_remove(0);
    for cut in [(0, 4), (4, 0), (0, 0), (4, 4)] {
        let strip = Strip {
            cut,
            ..Strip::of(16.0)
        };
        let body = spiral_strip(&plane, at.as_ref(), &strip, 1e-6).unwrap();
        let want = strip.chart_area() * area.unwrap();
        let got = mass_properties(&body).unwrap().area;
        assert!(
            (got / want - 1.0).abs() < 1e-8,
            "cut {cut:?}: area {got} against {want}"
        );
        assert_eq!(
            check(&body, &tolerance(1.0)),
            Vec::new(),
            "cut {cut:?}: a right strip"
        );
    }
}

/// The strips the review found refused, and their neighbours: 15 to 33
/// turns, either side cut into quarter turns, on a plane, a cylinder, a
/// cone and a sphere, at three scales, in the world and turned a thousand
/// sizes off. None has a fault of its loops; on the plane and the cylinder
/// each measures its closed form.
#[test]
fn spiral_strips_with_a_side_cut_are_faces() {
    let mut refused = Vec::new();
    let mut count = 0;
    for k in [1e-3, 1.0, 1e6] {
        for (name, surface, at, area) in grounds(k) {
            for turns in [15.0, 16.0, 16.5, 17.0, 32.0, 33.0] {
                for cut in [(4, 0), (0, 4)] {
                    let strip = Strip {
                        cut,
                        ..Strip::of(turns)
                    };
                    let body = spiral_strip(&surface, at.as_ref(), &strip, 1e-6 * k).unwrap();
                    if let Some(area) = area {
                        let want = strip.chart_area() * area;
                        let got = mass_properties(&body).unwrap().area;
                        assert!(
                            (got / want - 1.0).abs() < 1e-7,
                            "{name} at {k}, {turns} turns, cut {cut:?}: area {got} against {want}"
                        );
                    }
                    for (place, body) in [
                        ("world", body.clone()),
                        ("turned", body.transformed(&turned(k, 1e3)).unwrap()),
                    ] {
                        count += 1;
                        let found = loop_defects(&body, k);
                        if !found.is_empty() {
                            refused.push(format!(
                                "{name} at {k} {place}, {turns} turns, cut {cut:?}: {}",
                                found.join("; ")
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        refused.is_empty(),
        "{} of {count} right strips refused:\n{}",
        refused.len(),
        refused.join("\n")
    );
}
