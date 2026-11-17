//! A fillet that ends in a point: its radius runs to nothing at an end of
//! the edge, or short of it (`Fade::tip`), and the fillet face closes in a
//! vertex on the edge there, tangent to both faces all the way.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod common;

use boolean_common::{SCALES, mass, tolerance};
use craft3d_geom::Surface;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::fillet::{
    Fade, FilletFace, FilletLawEdge, FilletResult, RadiusLaw, fillet_by_law,
};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::{Body, EdgeId, FaceId, check};
use std::f64::consts::PI;

/// The box from `lo` with `size`, in `frame`, at scale `k`.
fn block(frame: &Frame, k: f64, lo: [f64; 3], size: [f64; 3]) -> Body {
    let corner = frame.point(lo[0] * k, lo[1] * k, lo[2] * k);
    let at = Frame::new(corner, frame.z(), frame.x()).unwrap();
    primitives::block(&at, size[0] * k, size[1] * k, size[2] * k).unwrap()
}

/// The edge whose middle is nearest `middle` (in `frame`, at scale 1).
fn edge_at(body: &Body, frame: &Frame, k: f64, middle: [f64; 3]) -> EdgeId {
    let target = frame.point(middle[0] * k, middle[1] * k, middle[2] * k);
    let (index, off) = body
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| {
            (
                i,
                e.curve.eval(0.5 * (e.range.0 + e.range.1)).distance(target),
            )
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    assert!(off < 1e-3 * k, "no edge at {middle:?}: {off}");
    EdgeId(index as u32)
}

/// The law of `radius` with `fade` at the end of `edge` further along
/// `up`, nothing at the other.
fn law_towards(body: &Body, edge: EdgeId, up: Vec3, radius: f64, fade: Fade) -> RadiusLaw {
    let data = body.edge(edge).unwrap();
    let along = data.curve.eval(data.range.1) - data.curve.eval(data.range.0);
    RadiusLaw {
        radius,
        fades: if along.dot(up) > 0.0 {
            [None, Some(fade)]
        } else {
            [Some(fade), None]
        },
    }
}

/// The one fillet face of a result.
fn fillet_face(made: &FilletResult) -> FaceId {
    let found: Vec<usize> = (0..made.faces.len())
        .filter(|i| matches!(made.faces[*i], FilletFace::Fillet(_)))
        .collect();
    assert_eq!(found.len(), 1, "one fillet face");
    FaceId(found[0] as u32)
}

/// What a document asks of a body before it hands it on: valid, meshed
/// watertight at two chords with the volume it encloses, and clear of
/// itself on the mesh of the self-check.
fn sound(body: &Body, k: f64, what: &str) {
    let tol = tolerance(k);
    assert_eq!(check(body, &tol), [], "{what}");
    let exact = mass(body);
    for chord in [0.05, 0.005] {
        let params = TessParams {
            chordal_tolerance: chord * k,
            ..TessParams::default()
        };
        let mesh = tessellate_body(body, &params).unwrap_or_else(|e| panic!("{what}: {e}"));
        let welded = mesh.weld().unwrap_or_else(|e| panic!("{what}: {e:?}"));
        assert!(welded.is_closed(), "{what}: the mesh is not watertight");
        assert!(
            (welded.volume() - exact.volume).abs() <= exact.area * chord * k,
            "{what}: mesh volume {} against {}",
            welded.volume(),
            exact.volume
        );
    }
    let found = body_self_intersections(body, &SelfCheckParams::new(tol.linear))
        .unwrap_or_else(|e| panic!("{what}: the self-check cannot mesh it: {e}"));
    assert!(
        found.crossings.is_empty() && found.certain_crossings == 0,
        "{what}: {} crossings",
        found.crossings.len()
    );
}

/// The block 40 × 30 × 40 whose upright edge at the origin is rounded 3
/// from the bottom, the radius running to nothing over 15 at `back` short
/// of the top: with its closed form. Between two planes at a right angle
/// the section at each height is the quarter circle of that height's
/// radius, so the fillet takes `(1 − π/4) ∫ r² dz`, and over a fade to
/// nothing `∫ r² = 9/14 R² L`.
fn tipped(frame: &Frame, k: f64, back: f64) -> (Body, FilletResult, f64) {
    let body = block(frame, k, [0.0, 0.0, 0.0], [40.0, 30.0, 40.0]);
    let edge = edge_at(&body, frame, k, [0.0, 0.0, 20.0]);
    let law = law_towards(
        &body,
        edge,
        frame.z(),
        3.0 * k,
        Fade::tip(15.0 * k, back * k),
    );
    let made = fillet_by_law(&body, &[FilletLawEdge { edge, law }], &tolerance(k))
        .unwrap_or_else(|e| panic!("back {back} at {k}: {e}"));
    let taken = (1.0 - 0.25 * PI) * 9.0 * ((40.0 - back - 15.0) + 15.0 * 9.0 / 14.0);
    (body, made, taken)
}

/// The first question of a tip (the fade's design, its first risk): is a
/// fitted rational face whose last row is exactly a point taken by what
/// comes after a fillet? Here: the validity check, the measure against
/// the closed form, the tessellator (watertight at two chords, the mesh's
/// volume the body's), the self-check on a mesh, and booleans through the
/// tip, beside it, across the face and along it. (STEP out and back:
/// `crates/io/tests/step_tip.rs`.)
#[test]
fn a_face_that_closes_in_a_point_is_taken_by_what_comes_after_it() {
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in common::frames(k).iter().enumerate() {
            for back in [0.0, 5.0] {
                let what = format!("back {back}, scale {k}, frame {index}");
                let (body, made, taken) = tipped(frame, k, back);
                // The face: three sides, a fitted rational surface whose
                // last row is one point, the tip.
                let face = made.body.face(fillet_face(&made)).unwrap();
                let Surface::Nurbs(surface) = &face.surface else {
                    panic!("{what}: not a fitted face");
                };
                let coedges: usize = face
                    .loops
                    .iter()
                    .map(|l| made.body.loop_(*l).unwrap().coedges.len())
                    .sum();
                assert_eq!(coedges, 3, "{what}");
                let (rows, across) = surface.counts();
                let tip = frame.point(0.0, 0.0, (40.0 - back) * k);
                let points = surface.control_points();
                let first_row = &points[..across];
                let last_row = &points[(rows - 1) * across..];
                let pole = if first_row[0].distance(tip) < last_row[0].distance(tip) {
                    first_row
                } else {
                    last_row
                };
                assert!(pole.iter().all(|p| *p == pole[0]), "{what}: {pole:?}");
                assert!(pole[0].distance(tip) <= 1e-12 * 40.0 * k, "{what}");
                assert!(surface.weights().iter().any(|w| *w != 1.0), "{what}");
                // Valid, measured, meshed, clear of itself.
                sound(&made.body, k, &what);
                let (before, after) = (mass(&body), mass(&made.body));
                let slack = 0.5 * PI * 3.0 * 40.0 * k * k * tol.linear;
                let off = (before.volume - after.volume - taken * k * k * k).abs();
                assert!(off <= 0.1 * slack, "{what}: the volume is {off:e} off");
                // Cut and kept by boxes: through the tip, beside it,
                // across the fade, along the face, and one whose face
                // lies in the tip itself.
                let top = 40.0 - back;
                let mut worst = 0.0_f64;
                for (name, lo, size) in [
                    (
                        "through the tip",
                        [-5.0, -5.0, top - 2.0],
                        [10.0, 10.0, 4.0],
                    ),
                    ("beside the tip", [0.5, -5.0, top - 2.0], [10.0, 10.0, 4.0]),
                    (
                        "across the fade",
                        [-5.0, -5.0, top - 8.0],
                        [10.0, 10.0, 20.0],
                    ),
                    ("along the face", [-5.0, -5.0, 10.0], [5.3, 60.0, 60.0]),
                    ("at the tip's height", [-5.0, -5.0, top], [10.0, 10.0, 20.0]),
                    (
                        "a corner in the face",
                        [0.2, 0.3, top - 6.0],
                        [10.0, 10.0, 3.0],
                    ),
                ] {
                    let tool = block(frame, k, lo, size);
                    let mut parts = 0.0;
                    for op in [BooleanOp::Subtract, BooleanOp::Intersect] {
                        let out = match boolean(&made.body, &tool, op, &tol) {
                            Ok(out) => out,
                            // (Above a tip at the top there is no body.)
                            Err(e) if e.to_string().contains("leaves nothing") => continue,
                            Err(e) => panic!("{what}, {name}, {op:?}: {e}"),
                        };
                        assert_eq!(check(&out.body, &tol), [], "{what}, {name}, {op:?}");
                        parts += mass(&out.body).volume;
                    }
                    let off = (parts - after.volume).abs() / (after.area * tol.linear);
                    worst = worst.max(off);
                    assert!(
                        off <= 1.0,
                        "{what}, {name}: the parts are {off} tolerances of area off the whole"
                    );
                }
                println!(
                    "{what}: {rows} rows, volume off {:.3} of the slack, the parts of a cut within {worst:.2e} of the area times the tolerance",
                    off / slack
                );
            }
        }
    }
}
