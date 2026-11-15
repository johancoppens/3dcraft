//! When a face gets a grid graded to its shape (`tess/src/graded.rs`),
//! and when it does not: `craft3d_tess::graded_grids` counts the times.
//!
//! One test, so that nothing else in this process moves the count: a face
//! whose even grid is made never reaches the graded grid, nor does a face
//! whose even grid is refused on a surface drawn evenly enough, at any
//! tolerance; only a face drawn unevenly whose even grid is refused does.
//! That is what makes every mesh that was made before the graded grid
//! the same mesh after it.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::{NurbsSurface, Surface};
use craft3d_math::{Frame, Vec3};
use craft3d_tess::{TessError, TessParams, graded_grids, tessellate_body_with_threads};
use craft3d_testkit::{solids, trimmed};
use craft3d_topo::Body;

/// The block 4 by 3 by 2 with its top (the face in `z = 2`) on the plane
/// drawn as a bilinear sheet with the corner weights `weights`.
fn plate(weights: [f64; 4]) -> Body {
    let mut body = solids::block(&Frame::WORLD, 4.0, 3.0, 2.0).unwrap().body;
    let top = body
        .faces
        .iter()
        .position(|face| match &face.surface {
            Surface::Plane(plane) => {
                plane.frame().z().z > 0.99 && (plane.frame().origin().z - 2.0).abs() < 1e-9
            }
            _ => false,
        })
        .unwrap();
    let knots = vec![0.0, 0.0, 1.0, 1.0];
    let corner = |x: f64, y: f64| Vec3::new(x, y, 2.0);
    body.faces[top].surface = Surface::Nurbs(
        NurbsSurface::new(
            1,
            1,
            knots.clone(),
            knots,
            2,
            2,
            vec![
                corner(0.0, 0.0),
                corner(0.0, 3.0),
                corner(4.0, 0.0),
                corner(4.0, 3.0),
            ],
            weights.to_vec(),
        )
        .unwrap(),
    );
    body
}

#[test]
fn only_a_face_drawn_unevenly_whose_even_grid_is_refused_gets_a_graded_grid() {
    let at = |chord: f64, max_segments: u32| TessParams {
        chordal_tolerance: chord,
        max_segments,
        ..TessParams::default()
    };
    assert_eq!(graded_grids(), 0);

    // Every reference solid and trimmed fixture, the spline ones among
    // them, at three scales: meshed or refused, never graded. The limits
    // of 4 and 16 a side refuse the freeform roofs as too fine, on
    // surfaces that are drawn evenly: they stay refused.
    let (mut meshed, mut too_fine) = (0, 0);
    for k in [1e-3, 1.0, 1e6] {
        let frame = Frame::WORLD;
        let mut bodies: Vec<Body> = solids::all(&frame, k)
            .unwrap()
            .into_iter()
            .map(|reference| reference.body)
            .collect();
        for nurbs in [false, true] {
            bodies.push(
                trimmed::domed_block(&frame, 3.0 * k, 2.0 * k, 1.5 * k, nurbs)
                    .unwrap()
                    .body,
            );
        }
        for body in &bodies {
            for (chord, max_segments) in [(1e-2, 1024), (1e-3, 1024), (1e-2, 16), (1e-4, 4)] {
                match tessellate_body_with_threads(body, &at(chord * k, max_segments), 1) {
                    Ok(_) => meshed += 1,
                    Err(TessError::TooFine { .. }) => too_fine += 1,
                    Err(e) => panic!("at {k}, chord {chord}, limit {max_segments}: {e}"),
                }
            }
        }
    }
    assert!(
        meshed > 40 && too_fine > 10,
        "{meshed} meshed, {too_fine} too fine"
    );
    assert_eq!(
        graded_grids(),
        0,
        "a face on a surface drawn evenly was graded"
    );

    // The plate with an even top, and with weights within four apart:
    // their even grids are made, or refused; never graded.
    for weights in [[1.0; 4], [1.0, 2.0, 2.0, 1.0], [1.0, 4.0, 4.0, 1.0]] {
        for (chord, max_segments) in [(4e-3, 1024), (4e-6, 4)] {
            let made = tessellate_body_with_threads(&plate(weights), &at(chord, max_segments), 1);
            assert!(
                matches!(made, Ok(_) | Err(TessError::TooFine { .. })),
                "{weights:?}: {made:?}"
            );
        }
    }
    assert_eq!(graded_grids(), 0, "a face drawn evenly enough was graded");

    // Thirty apart along one parameter: drawn unevenly, and its even grid
    // fits the limit. It keeps that grid (hundreds of thousands of
    // triangles, stopped here by a budget): not graded.
    let even = tessellate_body_with_threads(
        &plate([1.0, 1.0, 30.0, 30.0]),
        &TessParams {
            max_triangles: Some(5_000),
            ..at(2e-2, 1024)
        },
        1,
    );
    assert!(
        matches!(even, Err(TessError::OverBudget { .. })),
        "{even:?}"
    );
    assert_eq!(
        graded_grids(),
        0,
        "a face whose even grid is made was graded"
    );

    // The plate of the boolean checklist, 1, h, h, 1, a thousand and a
    // million apart: its even grid is refused (10 737 and 10.7 million
    // cells a side), it is graded, and it is the block's twelve triangles.
    let mut before = 0;
    for heavy in [1e3, 1e6] {
        let mesh =
            tessellate_body_with_threads(&plate([1.0, heavy, heavy, 1.0]), &at(2e-2, 1024), 1)
                .unwrap_or_else(|e| panic!("{heavy}: {e}"));
        assert_eq!(mesh.triangles.len(), 12, "{heavy}");
        assert!(mesh.weld().unwrap().is_closed(), "{heavy}");
        assert!(graded_grids() > before, "{heavy}: not graded");
        before = graded_grids();
    }
}
