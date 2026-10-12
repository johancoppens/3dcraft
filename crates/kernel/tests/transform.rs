//! Moving and mirroring what the kernel builds: the primitives, and the
//! extrusions of the profile catalogue (lines, arcs, circles, holes,
//! several shells). The checks themselves are those of
//! `crates/topo/tests/transform.rs`, which cannot reach the kernel.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "../../topo/tests/common/transform_checks.rs"]
mod checks;
mod common;

use checks::{check_image, check_mass, check_round_trip, isometries, tolerance};
use common::{catalogue, frames};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::detect_profiles;
use craft3d_math::{Frame, Tolerance, Transform};
use craft3d_topo::{Body, check, mass_properties, merge};

fn primitive_solids(frame: &Frame, k: f64) -> Vec<(&'static str, Body)> {
    vec![
        (
            "block",
            primitives::block(frame, 4.0 * k, 2.0 * k, 1.0 * k).unwrap(),
        ),
        (
            "cylinder",
            primitives::cylinder(frame, 1.5 * k, 4.0 * k).unwrap(),
        ),
        ("cone", primitives::cone(frame, 2.0 * k, 3.0 * k).unwrap()),
        ("sphere", primitives::sphere(frame, 2.5 * k).unwrap()),
        ("torus", primitives::torus(frame, 3.0 * k, 1.0 * k).unwrap()),
    ]
}

#[test]
fn primitives_survive_every_isometry() {
    for k in [1e-3, 1.0, 1e6] {
        for (f, frame) in frames(k).iter().enumerate() {
            for (name, body) in primitive_solids(frame, k) {
                for (how, t) in isometries(k) {
                    let what = format!("{name} (frame {f}, scale {k}) under {how}");
                    let image = check_image(&what, &body, &t, k);
                    check_mass(&what, &body, &image, &t, k);
                    check_round_trip(&what, &body, &image, &t, k);
                }
            }
        }
    }
}

fn extrusions_at(k: f64) {
    let tol = Tolerance::default();
    for case in catalogue(k) {
        let profiles = detect_profiles(&case.curves, &tol).unwrap();
        for (f, frame) in frames(k).iter().enumerate() {
            let body = extrude(&profiles, frame, -2.5 * k, 1.5 * k, &tol)
                .unwrap()
                .body;
            for (how, t) in isometries(k) {
                let what = format!("{} (frame {f}, scale {k}) under {how}", case.name);
                let image = check_image(&what, &body, &t, k);
                check_mass(&what, &body, &image, &t, k);
                check_round_trip(&what, &body, &image, &t, k);
            }
        }
    }
}

#[test]
fn extrusions_survive_every_isometry_at_unit_scale() {
    extrusions_at(1.0);
}

#[test]
fn extrusions_survive_every_isometry_at_a_thousandth() {
    extrusions_at(1e-3);
}

#[test]
fn extrusions_survive_every_isometry_at_a_million() {
    extrusions_at(1e6);
}

#[test]
fn the_catalogue_has_arcs_circles_and_holes() {
    // What the tests above claim to cover.
    let tol = Tolerance::default();
    let mut curves = std::collections::BTreeSet::new();
    let mut surfaces = std::collections::BTreeSet::new();
    let (mut holes, mut shells, mut reversed) = (0, 0, 0);
    for case in catalogue(1.0) {
        let profiles = detect_profiles(&case.curves, &tol).unwrap();
        holes += profiles.iter().map(|p| p.holes.len()).sum::<usize>();
        let body = extrude(&profiles, &Frame::WORLD, 0.0, 1.0, &tol)
            .unwrap()
            .body;
        shells = shells.max(body.shells.len());
        reversed += body.faces.iter().filter(|f| f.reversed).count();
        curves.extend(body.edges.iter().map(|e| e.curve.kind()));
        surfaces.extend(body.faces.iter().map(|f| f.surface.kind()));
    }
    assert_eq!(curves.into_iter().collect::<Vec<_>>(), ["circle", "line"]);
    assert_eq!(
        surfaces.into_iter().collect::<Vec<_>>(),
        ["cylinder", "plane"]
    );
    assert!(holes >= 5 && shells >= 2, "{holes} holes, {shells} shells");
    // Hole walls and concave arcs are cylinders seen from inside.
    assert!(reversed > 0);
}

#[test]
fn mirroring_in_the_sketch_plane_is_extruding_the_other_way() {
    // An oracle that does not come from the transform at all: the mirror
    // image of an extrusion from `from` to `to` in its own sketch plane is
    // the extrusion from -to to -from.
    let tol = Tolerance::default();
    for k in [1e-3, 1.0, 1e6] {
        for case in catalogue(k) {
            let profiles = detect_profiles(&case.curves, &tol).unwrap();
            for frame in frames(k) {
                let (from, to) = (-2.5 * k, 6.0 * k);
                let body = extrude(&profiles, &frame, from, to, &tol).unwrap().body;
                let mirror = Transform::mirror(frame.origin(), frame.z()).unwrap();
                let image = body.transformed(&mirror).unwrap();
                let other = extrude(&profiles, &frame, -to, -from, &tol).unwrap().body;

                let what = format!("{} at scale {k}", case.name);
                assert_eq!(check(&image, &tolerance(k)), vec![], "{what}");
                let (a, b) = (
                    mass_properties(&image).unwrap(),
                    mass_properties(&other).unwrap(),
                );
                assert!(a.volume > 0.0, "{what}: inside out");
                assert!((a.volume - b.volume).abs() <= 1e-12 * b.volume, "{what}");
                assert!((a.area - b.area).abs() <= 1e-12 * b.area, "{what}");
                let reach = frame.origin().length() + case.perimeter + (to - from);
                assert!(
                    a.centroid.unwrap().distance(b.centroid.unwrap()) <= 1e-12 * reach,
                    "{what}: centroid {:?} vs {:?}",
                    a.centroid,
                    b.centroid
                );
                // The same box, too.
                let (lo, hi) = image.bbox().unwrap().corners().unwrap();
                let (want_lo, want_hi) = other.bbox().unwrap().corners().unwrap();
                assert!(
                    lo.distance(want_lo) <= 1e-12 * reach && hi.distance(want_hi) <= 1e-12 * reach,
                    "{what}: box"
                );
            }
        }
    }
}

#[test]
fn a_pattern_of_extrusions_merges_into_one_valid_body() {
    // What the pattern feature will do: copies turned about an axis, in
    // one body.
    let tol = Tolerance::default();
    let case = catalogue(1.0)
        .into_iter()
        .find(|c| c.name == "plate with a round hole")
        .unwrap();
    let profiles = detect_profiles(&case.curves, &tol).unwrap();
    let frame = frames(1.0)[1];
    let body = extrude(&profiles, &frame, 0.0, 2.0, &tol).unwrap().body;
    // Far enough from the plate that six copies do not touch.
    let centre = frame.point(-60.0, -60.0, 0.0);
    let copies: Vec<Body> = (0..6)
        .map(|i| {
            let angle = std::f64::consts::TAU * f64::from(i) / 6.0;
            let turn = Transform::rotation_about(centre, frame.z(), angle).unwrap();
            // Every other copy is also mirrored in the plate's mid-plane.
            let t = if i % 2 == 1 {
                turn.then(&Transform::mirror(frame.point(0.0, 0.0, 1.0), frame.z()).unwrap())
            } else {
                turn
            };
            body.transformed(&t).unwrap()
        })
        .collect();
    let refs: Vec<&Body> = copies.iter().collect();
    let (merged, offsets) = merge(&refs).unwrap();
    assert_eq!(check(&merged, &tolerance(1.0)), vec![]);
    assert_eq!(merged.shells.len(), 6);
    assert_eq!(offsets[5].faces as usize, 5 * body.faces.len());
    let (one, all) = (
        mass_properties(&body).unwrap(),
        mass_properties(&merged).unwrap(),
    );
    assert!((all.volume - 6.0 * one.volume).abs() <= 1e-12 * all.volume);
    assert!((all.area - 6.0 * one.area).abs() <= 1e-12 * all.area);
    // The centroid of a ring of equal copies is on the axis, at the
    // plate's mid-height.
    let c = all.centroid.unwrap();
    let on_axis = centre + frame.z() * 1.0;
    assert!(c.distance(on_axis) <= 1e-11 * 100.0, "{c:?} vs {on_axis:?}");
}
