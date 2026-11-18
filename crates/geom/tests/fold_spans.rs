//! `offset::regular_by_span`, the test of the fold rule's suspects
//! (docs/design/verification.md › *A face folded over itself*): a bend and
//! a hairpin of radius `r` told at 1 % on either side of `r`, on the side
//! towards their centre and never on the other; a span with a border
//! drawn into a point told as a pole; a cusp, where the normal vanishes
//! along a line, never proven. At the scales 1e-3, 1 and 1e6.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft3d_geom::offset::{SpanRegularity, regular_by_span};
use craft3d_geom::{NurbsSurface, Surface};
use craft3d_math::Vec3;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e6];
const MOST: usize = 1 << 16;

/// Points of the x z plane at `(x, z)`, extruded along y by `length`: a
/// surface of degree `degree` across (the given knots and weights) and
/// one along y. Its normal `S_u × S_v` points to the left of the curve
/// seen from +y: towards the centre of a curve that turns that way.
fn extruded(
    degree: usize,
    knots: Vec<f64>,
    points: &[(f64, f64)],
    weights: &[f64],
    length: f64,
) -> Surface {
    let mut ctrl = Vec::new();
    let mut w = Vec::new();
    for (p, weight) in points.iter().zip(weights) {
        for j in 0..2 {
            ctrl.push(Vec3::new(p.0, length * f64::from(j), p.1));
            w.push(*weight);
        }
    }
    Surface::Nurbs(
        NurbsSurface::new(
            degree,
            1,
            knots,
            vec![0.0, 0.0, 1.0, 1.0],
            points.len(),
            2,
            ctrl,
            w,
        )
        .unwrap(),
    )
}

/// A quarter of a circle of radius `r` about the origin, from (r, 0)
/// to (0, r): a bend.
fn bend(r: f64) -> Surface {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    extruded(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        &[(r, 0.0), (r, r), (0.0, r)],
        &[1.0, h, 1.0],
        2.0 * r,
    )
}

/// Half a circle of radius `r` about the origin, from (r, 0) over (0, r)
/// to (−r, 0): a hairpin, the face turning back on itself.
fn hairpin(r: f64) -> Surface {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    extruded(
        2,
        vec![0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
        &[(r, 0.0), (r, r), (0.0, r), (-r, r), (-r, 0.0)],
        &[1.0, h, 1.0, h, 1.0],
        2.0 * r,
    )
}

fn proven(surface: &Surface, d: f64) -> bool {
    regular_by_span(surface, d, MOST)
        .unwrap()
        .iter()
        .all(|s| s.regularity == SpanRegularity::Proven)
}

fn folds(surface: &Surface, d: f64) -> bool {
    regular_by_span(surface, d, MOST)
        .unwrap()
        .iter()
        .any(|s| matches!(s.regularity, SpanRegularity::Folds { .. }))
}

#[test]
fn a_bend_and_a_hairpin_are_told_at_one_percent_of_their_radius() {
    for k in SCALES {
        let r = 3.0 * k;
        for (name, surface) in [("bend", bend(r)), ("hairpin", hairpin(r))] {
            assert!(proven(&surface, 0.99 * r), "{name} at {k}: 0.99 r");
            assert!(folds(&surface, 1.01 * r), "{name} at {k}: 1.01 r");
            // The other side, away from the centre: never.
            assert!(proven(&surface, -100.0 * r), "{name} at {k}: away");
        }
    }
}

#[test]
fn a_border_drawn_into_a_point_is_a_pole() {
    for k in SCALES {
        // A bilinear patch whose border u = 0 is one point.
        let apex = Vec3::new(0.0, 0.0, k);
        let surface = Surface::Nurbs(
            NurbsSurface::new(
                1,
                1,
                vec![0.0, 0.0, 1.0, 1.0],
                vec![0.0, 0.0, 1.0, 1.0],
                2,
                2,
                vec![apex, apex, Vec3::new(k, 0.0, 0.0), Vec3::new(k, k, 0.0)],
                vec![1.0; 4],
            )
            .unwrap(),
        );
        let found = regular_by_span(&surface, 1e-6 * k, MOST).unwrap();
        assert_eq!(found.len(), 1);
        match &found[0].regularity {
            SpanRegularity::Pole { points } => {
                assert!(points.iter().any(|p| p.distance(apex) <= 1e-12 * k));
            }
            other => panic!("at {k}: {other:?}"),
        }
    }
}

#[test]
fn a_cusp_is_never_proven() {
    for k in SCALES {
        // The cubic (0, 0), (1, 1), (0, 1), (1, 0) stands still at its
        // middle: the surface has no normal along that line.
        let surface = extruded(
            3,
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            &[(0.0, 0.0), (k, k), (0.0, k), (k, 0.0)],
            &[1.0; 4],
            k,
        );
        for d in [1e-6 * k, -1e-6 * k] {
            let found = regular_by_span(&surface, d, MOST).unwrap();
            assert!(
                found
                    .iter()
                    .any(|s| !matches!(s.regularity, SpanRegularity::Proven)),
                "at {k}, {d}: {found:?}"
            );
        }
    }
}

#[test]
fn too_many_spans_are_refused_and_other_kinds_are_not_asked() {
    assert!(regular_by_span(&hairpin(1.0), 0.5, 1).is_err());
    let plane = Surface::Plane(craft3d_geom::Plane::new(craft3d_math::Frame::WORLD));
    assert!(regular_by_span(&plane, 0.5, MOST).is_err());
}
