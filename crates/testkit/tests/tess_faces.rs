//! Faces the tessellator could not mesh, found on 2026-10-20 and 21
//! (`docs/design/tessellator.md` › *Three faces the tessellator could not
//! mesh*), each at three scales and several placements:
//!
//! - a fillet of 2.5 on an arc of radius 704, turned about y, at chords
//!   of 2 to 8: the Perseverance's grouser, whose export failed at its
//!   default tolerance ("its refinement cannot go on");
//! - the crescents of the rover's drill tip, a cone whose base circle
//!   lies 3.9e-4 off a drill's wall: the self-check's mesh ran out of
//!   rounds;
//! - a torus with holes through its tube, the whole torus but for them:
//!   "its loops do not bound a region".
//!
//! Every mesh is closed, every triangle faces the way its surface does
//! at the foot of its centroid and lies within the chord of it there,
//! and the mesh's volume is the body's within the chord times the area.
//!
//! This lives in `testkit` because `kernel` and `tess` share a layer and
//! may not depend on each other, not even for tests.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, EdgeId, check, mass_properties};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

fn tolerance(k: f64) -> Tolerance {
    Tolerance {
        linear: 1e-6 * k,
        ..Tolerance::default()
    }
}

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

/// Meshes `body` at `chord` and holds the mesh to its surfaces: closed,
/// every triangle within the chord of its face's surface at the foot of
/// its centroid and facing the way the surface does at its corners (their
/// normals summed) and, where the chord is below `thinnest` (the least
/// radius of curvature of the body's faces), at the foot of its centroid
/// too (beyond it a triangle may span a whole fillet, and the foot of its
/// centroid lie on a part of the surface that is not the face); and the
/// volume within the chord times the area of the body's.
fn meshed_well(body: &Body, chord: f64, thinnest: f64, what: &str) {
    let params = TessParams {
        chordal_tolerance: chord,
        ..TessParams::default()
    };
    let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(mesh.weld().unwrap().is_closed(), "{what}: not closed");
    let mut volume = 0.0;
    for (t, &face) in mesh.face_ids.iter().enumerate() {
        let [a, b, c] = mesh.corners(t).unwrap();
        volume += a.dot(b.cross(c)) / 6.0;
        let face = &body.faces[face as usize];
        let outward = if face.reversed { -1.0 } else { 1.0 };
        let facing = (b - a).cross(c - a) * outward;
        let normal_at = |p: Vec3| {
            let (u, w) = face.surface.project(p).unwrap();
            (
                face.surface.eval(u, w).distance(p),
                face.surface.normal(u, w),
            )
        };
        let corners = [a, b, c]
            .iter()
            .filter_map(|&p| normal_at(p).1)
            .fold(Vec3::ZERO, |sum, n| sum + n);
        assert!(
            corners == Vec3::ZERO || facing.dot(corners) > 0.0,
            "{what}: triangle {t} faces against its surface at its corners"
        );
        let (off, normal) = normal_at((a + b + c) * (1.0 / 3.0));
        assert!(
            off <= chord * (1.0 + 1e-9),
            "{what}: triangle {t} is {off} off its surface"
        );
        if let (Some(normal), true) = (normal, chord < thinnest) {
            assert!(
                facing.dot(normal) > 0.0,
                "{what}: triangle {t} faces against its surface"
            );
        }
    }
    let exact = mass_properties(body).unwrap();
    assert!(
        (volume - exact.volume).abs() <= chord * exact.area,
        "{what}: volume {volume} against {}",
        exact.volume
    );
}

/// The Perseverance's grouser: a band between two arcs about (-700, 0),
/// 288 long and 8 wide, extruded 15 from z = 257.5, its two top arcs
/// filleted 2.5, turned by `degrees` about y; all times `k`.
fn grouser(k: f64, degrees: f64) -> Body {
    let tol = Tolerance {
        linear: 1e-4 * k,
        ..Tolerance::default()
    };
    let p = |x: f64, y: f64| [x * k, y * k];
    let centre = p(-700.0, 0.0);
    let curve = |id: u64, curve: Curve2| SketchCurve { id, curve };
    let curves = vec![
        curve(
            1,
            Curve2::Arc {
                center: centre,
                start: p(-10.8846, -144.0),
                end: p(-10.8846, 144.0),
            },
        ),
        curve(
            2,
            Curve2::Line {
                start: p(-10.8846, 144.0),
                end: p(-19.0595, 144.0),
            },
        ),
        curve(
            3,
            Curve2::Arc {
                center: centre,
                start: p(-19.0595, -144.0),
                end: p(-19.0595, 144.0),
            },
        ),
        curve(
            4,
            Curve2::Line {
                start: p(-19.0595, -144.0),
                end: p(-10.8846, -144.0),
            },
        ),
    ];
    let profiles = detect_profiles(&curves, &tol).unwrap();
    let plane = Frame::WORLD.at(v(0.0, 0.0, 257.5 * k)).unwrap();
    let body = extrude(&profiles, &plane, 0.0, 15.0 * k, &tol)
        .unwrap()
        .body;
    let top = 272.5 * k;
    let edges: Vec<FilletEdge> = body
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            matches!(e.curve, craft3d_geom::Curve3::Circle(_))
                && [e.start, e.end]
                    .iter()
                    .all(|id| (body.vertices[id.0 as usize].point.z - top).abs() < 1e-9 * k)
        })
        .map(|(i, _)| FilletEdge {
            edge: EdgeId(i as u32),
            radius: 2.5 * k,
        })
        .collect();
    assert_eq!(edges.len(), 2);
    let body = fillet(&body, &edges, &tol).unwrap().body;
    let turn = Transform::rotation(v(0.0, 1.0, 0.0), degrees.to_radians()).unwrap();
    let body = body.transformed(&turn).unwrap();
    assert_eq!(check(&body, &tol), [], "scale {k}, turned {degrees}");
    body
}

/// The grouser's fillet on the outer arc is a quarter of a tube of 2.5
/// around a ring of 701.5; at chords of 4 and more a chord of its rim on
/// the wall sags by more than the tube is thick and passes the far side
/// of its axis, so no triangle on it faces the way the surface does.
/// Before, the refinement crept towards the chord until there was no room
/// left ("its refinement cannot go on: a triangle 4.4e-7 across"), at
/// some turns and scales and not at others, by rounding: the rover's
/// export failed at its default tolerance of 6.2. Now the chords that no
/// triangle can stand on are cut.
#[test]
fn a_fillet_thinner_than_its_rims_sag_is_meshed_at_any_turn() {
    let mut turns: Vec<f64> = (-9..=9).map(|i| 5.0 * f64::from(i)).collect();
    for around in [7.5, 30.0, -7.5] {
        turns.extend((-4..=4).map(|i| around + 0.25 * f64::from(i)));
    }
    for k in SCALES {
        for &degrees in &turns {
            let body = grouser(k, degrees);
            for chord in [2.0, 3.0, 4.0, 5.0, 6.2, 8.0] {
                meshed_well(
                    &body,
                    chord * k,
                    2.5 * k,
                    &format!("scale {k}, turned {degrees}, chord {chord}"),
                );
            }
        }
    }
}

/// The Perseverance's drill tip: a drill (r 30, 180 long) along a tilted
/// axis and a cone (r 30, 60 high) whose base circle lies on its wall,
/// the positions given to a thousandth, so that the cone's axis is
/// 3.4e-6 rad off the drill's and its base 3.9e-4 off the wall; united.
/// All times `k`, turned by `turn`.
fn drill_tip(k: f64, turn: Option<Transform>) -> Body {
    let tol = tolerance(k);
    let normal = (v(2348.824, 300.0, 615.126) - v(2406.948, 300.0, 696.499))
        .cross(v(0.0, 1.0, 0.0))
        .normalized()
        .unwrap();
    let normal = if normal.x > 0.0 { normal } else { -normal };
    let drill_frame = Frame::new(v(2348.824, 300.0, 615.126) * k, normal, Vec3::Y).unwrap();
    let drill = primitives::cylinder(&drill_frame, 30.0 * k, 180.0 * k).unwrap();
    let tip_frame =
        Frame::from_z(v(2493.669, 300.0, 511.665) * k, v(0.813733, 0.0, -0.581238)).unwrap();
    let tip = primitives::cone(&tip_frame, 30.0 * k, 60.0 * k).unwrap();
    let (drill, tip) = match turn {
        Some(t) => (drill.transformed(&t).unwrap(), tip.transformed(&t).unwrap()),
        None => (drill, tip),
    };
    let body = boolean(&drill, &tip, BooleanOp::Union, &tol).unwrap().body;
    assert_eq!(check(&body, &tol), [], "scale {k}");
    body
}

/// The drill tip's union has two crescents outside the drill, on the
/// cone and on its base, 3.9e-4 and 7.8e-4 across and 94 long, with
/// tips at 2.6e-5 rad. Their two edges must be cut into chords that do
/// not cross: halving them, round after round, needs about twelve rounds
/// at the tips and more points than an edge may have at the self-check's
/// chord, and the face was refused ("the boundary crosses itself"). Now
/// the rounds of halving are followed by cuts across from the points of
/// the chords they cross, which pair the two edges' points up.
#[test]
fn the_crescents_of_a_cone_on_a_tilted_drill_are_meshed() {
    let turns = [
        None,
        Some(Transform::rotation(v(1.0, 1.0, 1.0), 0.3).unwrap()),
        Some(Transform::rotation(v(0.2, -1.0, 0.5), 1.1).unwrap()),
    ];
    for k in SCALES {
        for (n, turn) in turns.iter().enumerate() {
            let what = format!("scale {k}, turn {n}");
            let body = drill_tip(k, *turn);
            let found = body_self_intersections(&body, &SelfCheckParams::new(1e-6 * k))
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(found.certain_crossings, 0, "{what}: {found}");
            // The self-check's chords (a thousandth of the box's
            // diagonal, then a quarter and a sixteenth of that) and the
            // export's.
            for chord in [0.27, 0.068, 0.017, 6.0] {
                meshed_well(
                    &body,
                    chord * k,
                    30.0 * k,
                    &format!("{what}, chord {chord}"),
                );
            }
        }
    }
}

/// A torus less a rod through its middle across its axis: four holes
/// through its tube, the torus face all of the torus but for them.
/// Before, a closed surface with only holes was handled where it closes
/// to points (a sphere) and a torus was refused ("its loops do not bound
/// a region"). Now the torus is opened along a circle of each parameter
/// that no hole reaches. Rods of three radii, square to the axis and
/// tilted.
#[test]
fn a_torus_with_holes_through_its_tube_is_meshed() {
    for k in SCALES {
        let tol = tolerance(k);
        let torus = primitives::torus(&Frame::WORLD, 10.0 * k, 2.0 * k).unwrap();
        for (tilt, radius) in [(0.0, 1.0), (0.0, 0.5), (0.0, 1.5), (0.2, 1.0), (0.45, 0.7)] {
            let what = format!("scale {k}, tilt {tilt}, radius {radius}");
            let along = v(tilt, 1.0, 0.5 * tilt).normalized().unwrap();
            let axis = Frame::from_z(along * (-20.0 * k), along).unwrap();
            let rod = primitives::cylinder(&axis, radius * k, 40.0 * k).unwrap();
            let body = boolean(&torus, &rod, BooleanOp::Subtract, &tol)
                .unwrap()
                .body;
            assert_eq!(check(&body, &tol), [], "{what}");
            for chord in [0.5, 0.05, 0.005] {
                meshed_well(
                    &body,
                    chord * k,
                    radius.min(2.0) * k,
                    &format!("{what}, chord {chord}"),
                );
            }
        }
    }
}
