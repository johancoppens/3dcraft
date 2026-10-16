//! Intersections inside a box: what the box cuts off must not spoil what
//! it keeps (2026-10-15, the lamp's finding B1).
//!
//! Two spline roofs that cross (a domed block of the test kit and one
//! hung upside down over it, tilted) are intersected inside seeded boxes
//! that cut their curve at random places, at three scales. Every curve
//! returned has to be within the tolerance of both surfaces along its
//! whole length, the ends the box made included. Before the pieces a box
//! cuts were refined, the span to such an end was a new cubic held to
//! nothing, and on the lamp's shell and cell the fit was refused ("an
//! intersection curve could not be fitted within the tolerance"; that
//! case, which these roofs do not reproduce, is
//! `crates/engine/tests/boolean_findings.rs`).
//!

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod boolean_common;

use boolean_common::*;
use craft3d_geom::Surface;
use craft3d_geom::intersect::{SurfaceMeeting, meet_surfaces};
use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
use craft3d_math::{Bbox3, Frame, Vec3};
use craft3d_testkit::trimmed::domed_block;
use craft3d_topo::Body;

/// The domed block upright at the origin and the one hung over it,
/// turned about a tilted axis, at scale `k`.
fn roofs(k: f64) -> (Body, Body) {
    let tilt = |v: Vec3| {
        // The same small turn for both, so nothing is axis-aligned.
        let axis = Vec3::new(0.3, -0.4, 0.86).normalized().unwrap();
        let angle: f64 = 0.37;
        let (s, c) = angle.sin_cos();
        v * c + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - c))
    };
    let upright = Frame::new(
        tilt(Vec3::new(0.0, 0.0, 0.0)) * k,
        tilt(Vec3::Z),
        tilt(Vec3::X),
    )
    .unwrap();
    let hung = Frame::new(
        tilt(Vec3::new(4.6, 0.4, 4.3)) * k,
        tilt(Vec3::new(0.08, -0.05, -1.0)),
        tilt(Vec3::new(-1.0, 0.1, 0.0)),
    )
    .unwrap();
    (
        domed_block(&upright, 4.0 * k, 3.0 * k, 2.0 * k, true)
            .unwrap()
            .body,
        domed_block(&hung, 4.2 * k, 3.4 * k, 2.1 * k, true)
            .unwrap()
            .body,
    )
}

fn roof(body: &Body) -> &Surface {
    &body
        .faces
        .iter()
        .find(|f| matches!(f.surface, Surface::Nurbs(_)))
        .unwrap()
        .surface
}

/// Boxes per scale.
const BOXES: usize = 60;

#[test]
fn boxes_that_cut_the_curve_keep_it_within_the_tolerance() {
    for k in SCALES {
        let tol = tolerance(k);
        let (a, b) = roofs(k);
        let (sa, sb) = (roof(&a), roof(&b));
        let whole = a.bbox().unwrap().corners().unwrap();
        let mut rng = Rng(0x5eed_b0c5 ^ k.to_bits());
        let mut cut = 0;
        for _ in 0..BOXES {
            let corner = |rng: &mut Rng| {
                Vec3::new(
                    rng.between(whole.0.x, whole.1.x),
                    rng.between(whole.0.y, whole.1.y),
                    rng.between(whole.0.z, whole.1.z),
                )
            };
            // A box about a random point, up to a third of the body across.
            let middle = corner(&mut rng);
            let reach = (whole.1 - whole.0) * rng.between(0.02, 0.33);
            let within = Bbox3::from_points([middle - reach, middle + reach]);
            let met = meet_surfaces(sa, sb, &within, &tol)
                .unwrap_or_else(|e| panic!("at {k} in {within:?}: {e}"));
            let SurfaceMeeting::Meets(meeting) = met else {
                continue;
            };
            for curve in &meeting.curves {
                cut += usize::from(!curve.closed);
                assert!(curve.deviation <= tol.linear, "{}", curve.deviation);
                let (t0, t1) = curve.range;
                for i in 0..=200 {
                    let p = curve.curve.eval(t0 + (t1 - t0) * f64::from(i) / 200.0);
                    for s in [sa, sb] {
                        let (u, v) = s.project(p).unwrap();
                        let off = s.eval(u, v).distance(p);
                        assert!(off <= tol.linear, "at {k}: {off} off a roof at {p:?}");
                    }
                }
            }
        }
        assert!(cut > 10, "at {k} the boxes cut the curve {cut} times");
    }
}

/// A pair of faces is intersected inside the box the two faces share
/// (`boolean/section.rs`), so what either body holds besides them does
/// not change the answer: the difference of the roofs with a far block
/// added to the tool is the difference without it, to 1e-9 (with the box
/// of the bodies the curves were fitted in a larger box, and it was not).
#[test]
fn a_far_block_in_the_tool_changes_nothing() {
    for k in SCALES {
        let (a, b) = roofs(k);
        let far = boxed(k, [-30.0, -30.0, -30.0], [-29.0, -29.0, -29.0]);
        let tool = solve(&b, &far, Union, k).body;
        let common = mass(&solve(&a, &b, Intersect, k).body).volume;
        assert!(common > 1e-3 * k * k * k, "the roofs overlap: {common}");
        let rest = solve(&a, &b, Subtract, k);
        let rest_far = solve(&a, &tool, Subtract, k);
        assert_eq!(rest.body.faces.len(), rest_far.body.faces.len());
        let (v, v_far) = (mass(&rest.body).volume, mass(&rest_far.body).volume);
        assert_close(v_far, v, "the rest with a far block in the tool");
        assert_close(v + common, mass(&a).volume, "rest and common");
    }
}

/// A piece of a tube bent along a circle of radius `ring` (a torus of
/// tube radius `tube`), turned through `turn` about the torus' axis
/// `axis` through `centre`, symmetrical about the direction `out`.
fn bent_tube(centre: Vec3, axis: Vec3, out: Vec3, ring: f64, tube: f64, turn: f64, k: f64) -> Body {
    use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
    use craft3d_kernel::revolve::revolve;
    let tol = tolerance(k);
    let plane = Frame::new(centre, out.cross(axis), out).unwrap();
    // In the plane: x along `out`, y along the axis.
    let profiles = detect_profiles(
        &[SketchCurve {
            id: 1,
            curve: Curve2::Circle {
                center: [ring, 0.0],
                radius: tube,
            },
        }],
        &tol,
    )
    .unwrap();
    revolve(
        &profiles,
        &plane,
        [0.0, 0.0],
        [0.0, 1.0],
        -0.5 * turn,
        0.5 * turn,
        &tol,
    )
    .unwrap()
    .body
}

/// The box of a face on a torus is that of the tube around the stretch of
/// its central circle the face turns through (the bike's finding B6,
/// 2026-10-15: two seat stays,
/// each a tube of 6 mm bent along a circle of 2.8 m, mirrored, were
/// refused as one, "an intersection curve could not be fitted": a face
/// on a torus had the box of the whole torus, and so had both faces of
/// the pair; their tori, mirror images, meet far from both faces, where
/// the curve was not fitted). Two such tubes 1.6 mm (times the scale)
/// apart at their nearest, whose tori cross the mirror plane
/// 0.2 rad further round: their union is both, their intersection
/// nothing, their difference the first (Pappus: π r² R θ each), upright
/// and tilted, at three scales.
#[test]
fn two_bent_tubes_whose_tori_meet_far_away_are_apart() {
    use craft3d_kernel::boolean::BooleanOp::{Intersect, Subtract, Union};
    use craft3d_math::Transform;
    let (ring, tube, turn, lean) = (2776.0, 6.0, 0.15, 0.02_f64);
    for k in SCALES {
        let axis = Vec3::new(lean.sin(), lean.cos(), 0.0);
        let first = bent_tube(
            Vec3::new(0.0, 11.0, -ring) * k,
            axis,
            Vec3::Z,
            ring * k,
            tube * k,
            turn,
            k,
        );
        let mirror = Transform::mirror(Vec3::ZERO, Vec3::Y).unwrap();
        let second = first.transformed(&mirror).unwrap();
        for tilted in [false, true] {
            let place = if tilted {
                Transform::rotation(Vec3::new(0.3, -0.5, 0.8), 0.7)
                    .unwrap()
                    .then(&Transform::translation(Vec3::new(1.5, -2.0, 0.7) * k))
            } else {
                Transform::IDENTITY
            };
            let (a, b) = (
                first.transformed(&place).unwrap(),
                second.transformed(&place).unwrap(),
            );
            let each = std::f64::consts::PI * (tube * k).powi(2) * (ring * k) * turn;
            let (one, other) = (mass(&a), mass(&b));
            assert_close(one.volume, each, "a tube");
            assert_close(other.volume, each, "its mirror image");
            let both = solve(&a, &b, Union, k);
            assert_eq!(both.body.shells.len(), 2, "at {k}, tilted {tilted}");
            let m = mass(&both.body);
            assert_close(m.volume, 2.0 * each, "the union");
            assert_close(m.area, one.area + other.area, "the union's area");
            assert_empty(&a, &b, Intersect, k);
            let rest = mass(&solve(&a, &b, Subtract, k).body);
            assert_close(rest.volume, each, "the difference");
        }
    }
}

/// An edge cut from a fitted curve is the piece of it the edge is: its
/// control points lie near the edge, not along the whole branch it was cut
/// from (2026-10-16: the turbofan's casing kept a hole's whole loop with
/// every piece of it, and its faces' boxes grew by the loop). For the three
/// booleans of the crossing roofs at three scales: every spline edge runs
/// over its curve's whole domain, and its control points are within the
/// edge's own length of the box of points along it.
#[test]
fn an_edge_cut_from_a_fitted_curve_is_its_piece_of_it() {
    use craft3d_geom::Curve3;
    for k in SCALES {
        let (a, b) = roofs(k);
        for op in [Union, Intersect, Subtract] {
            let body = solve(&a, &b, op, k).body;
            let mut splines = 0;
            for edge in &body.edges {
                let Curve3::Nurbs(curve) = &edge.curve else {
                    continue;
                };
                splines += 1;
                let (t0, t1) = edge.range;
                let (d0, d1) = curve.domain();
                assert!(
                    (t0 - d0).abs() < 1e-12 && (t1 - d1).abs() < 1e-12,
                    "{op:?} at {k}"
                );
                let along: Vec<Vec3> = (0..=64)
                    .map(|i| curve.eval(t0 + (t1 - t0) * f64::from(i) / 64.0))
                    .collect();
                let length: f64 = along.windows(2).map(|w| w[0].distance(w[1])).sum();
                let near = Bbox3::from_points(along);
                for &p in curve.control_points() {
                    assert!(
                        near.contains(p, length),
                        "{op:?} at {k}: a control point far off"
                    );
                }
            }
            assert!(splines > 0, "{op:?} at {k}: no spline edge");
        }
    }
}
