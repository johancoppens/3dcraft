//! Edges between faces that do not move, when a face next to them moves
//! (`kernel::tweak`, docs/design/face-operations.md › *An offset that
//! measured smaller*): their moved end is put on the border they lie
//! along, and a line or a circle among them is made again as one, through
//! its ends, the kind and the range it had: only an edge that would be
//! fitted becomes that border's iso-curve.
//!
//! The review's body (2026-11-14): a profile of one spline and three
//! lines, extruded. The two upright edges where the spline's wall meets
//! the planar sides lie along the spline wall's borders. With the first
//! form of the change they came out of a move of the top as spline curves
//! over the range 0 to 0.8 instead of lines over 0 to 4: the same
//! geometry, another kind (a STEP entity, the edges `document.edges`
//! lists, a mesh's sampling of the edge).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_geom::{Curve3, Surface};
use craft3d_kernel::extrude::{Role, extrude};
use craft3d_kernel::move_face::{Motion, move_faces};
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};

use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, FaceId, edge_faces, mass_properties};

fn kind(c: &Curve3) -> &'static str {
    match c {
        Curve3::Line(_) => "line",
        Curve3::Circle(_) => "circle",
        Curve3::Ellipse(_) => "ellipse",
        Curve3::Nurbs(_) => "nurbs",
    }
}

/// The profile of one cubic spline (two spans) and three lines, scaled by
/// `k`, extruded from 0 to `5 k`: the body and its top.
fn prism(k: f64, tolerance: &Tolerance) -> (Body, FaceId) {
    let line = |id: u64, start: [f64; 2], end: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line {
            start: [start[0] * k, start[1] * k],
            end: [end[0] * k, end[1] * k],
        },
    };
    let control = [
        [-6.0, 0.0],
        [-3.0, 2.0],
        [0.0, -1.5],
        [3.0, 2.0],
        [6.0, 0.0],
    ]
    .map(|[x, y]| [x * k, y * k])
    .to_vec();
    let curves = vec![
        SketchCurve {
            id: 1,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
                control,
            }),
        },
        line(2, [6.0, 0.0], [6.0, 12.0]),
        line(3, [6.0, 12.0], [-6.0, 12.0]),
        line(4, [-6.0, 12.0], [-6.0, 0.0]),
    ];
    let profiles = detect_profiles(&curves, tolerance).unwrap();
    let made = extrude(&profiles, &Frame::WORLD, 0.0, 5.0 * k, tolerance).unwrap();
    let top = made
        .origins
        .faces
        .iter()
        .position(|o| o.role == Role::EndCap)
        .unwrap();
    (made.body, FaceId(u32::try_from(top).unwrap()))
}

/// The edges between the spline's wall and a planar side.
fn uprights(body: &Body) -> Vec<usize> {
    let uses = edge_faces(body);
    (0..body.edges.len())
        .filter(|&i| {
            let faces = &uses[i];
            faces.len() == 2
                && faces
                    .iter()
                    .any(|f| matches!(body.faces[f.0 as usize].surface, Surface::Nurbs(_)))
                && faces
                    .iter()
                    .any(|f| matches!(body.faces[f.0 as usize].surface, Surface::Plane(_)))
                && matches!(body.edges[i].curve, Curve3::Line(_))
        })
        .collect()
}

/// The top moved down by a fifth of the height, by move face and by
/// offset face: every edge keeps its kind; the two uprights along the
/// spline wall's borders are lines over 0 to 4 k, each on the spline
/// wall to rounding; the volume is four fifths of the prism's (a prism's
/// volume is its section times its height). Before the fix of the review
/// the uprights came out as spline curves. At three scales.
#[test]
fn lines_along_a_border_of_a_face_that_stays_stay_lines() {
    for k in [1e-3, 1.0, 1e6] {
        let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        let (body, top) = prism(k, &tolerance);
        let upright = uprights(&body);
        assert_eq!(upright.len(), 2, "{k}");
        let v0 = mass_properties(&body).unwrap().volume;
        let moved = move_faces(
            &body,
            &[top],
            &Motion {
                translate: Vec3::new(0.0, 0.0, -k),
                pivot: Vec3::ZERO,
                axis: Vec3::new(0.0, 0.0, 1.0),
                angle: 0.0,
            },
            &tolerance,
        )
        .unwrap();
        let offset = offset_faces(
            &body,
            &[FaceOffset {
                face: top,
                distance: -k,
            }],
            &tolerance,
        )
        .unwrap();
        for (what, after) in [("move face", &moved.body), ("offset face", &offset.body)] {
            for (i, (e0, e1)) in body.edges.iter().zip(&after.edges).enumerate() {
                assert_eq!(kind(&e0.curve), kind(&e1.curve), "{what} {k}: edge {i}");
            }
            let uses = edge_faces(after);
            for &i in &upright {
                let e = &after.edges[i];
                let (t0, t1) = e.range;
                assert!(
                    t0.abs() <= 1e-12 * k && (t1 - 4.0 * k).abs() <= 1e-12 * k,
                    "{what} {k}: edge {i} over {:?}",
                    e.range
                );
                let wall = uses[i]
                    .iter()
                    .map(|f| &after.faces[f.0 as usize].surface)
                    .find(|s| matches!(s, Surface::Nurbs(_)))
                    .unwrap();
                for s in 0..=8 {
                    let p = e.curve.eval(t0 + (t1 - t0) * f64::from(s) / 8.0);
                    let (u, v) = wall.project(p).unwrap();
                    let off = wall.eval(u, v).distance(p);
                    assert!(
                        off <= 1e-12 * k,
                        "{what} {k}: edge {i} {off:e} off the wall"
                    );
                }
            }
            let v1 = mass_properties(after).unwrap().volume;
            assert!(
                (v1 - 0.8 * v0).abs() <= 1e-9 * v0,
                "{what} {k}: volume {v1}, not four fifths of {v0}"
            );
        }
    }
}
