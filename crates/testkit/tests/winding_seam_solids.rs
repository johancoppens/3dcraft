//! Solids whose curved faces go around their axis with holes on every
//! longitude, made by the boolean kernel: a pipe drilled through its wall
//! by two rows of radial holes, the second row half a pitch further, the
//! holes wide enough that their angular spans overlap. No straight seam
//! crosses the outer wall or the bore, so the tessellator cuts both open
//! along a seam in pieces, through holes (`crates/tess/tests/
//! winding_seam.rs` has the faces alone, with closed-form areas).
//!
//! The oracle: the welded mesh is a closed manifold, bit for bit the same
//! on a second run, and its volume converges to the volume the exact
//! B-rep has (`mass_properties`, independent of the tessellator) within
//! what the chordal tolerance explains. At three sizes.
//!
//! This lives in `testkit` because `kernel` and `tess` share a layer and
//! may not depend on each other, not even for tests.

// Test code: a failure in a helper is a failed test.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use craft3d_geom::Surface;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_tess::{TessParams, tessellate_body};
use craft3d_topo::{Body, check, mass_properties};
use std::f64::consts::TAU;

/// A pipe of outer radius 2, bore 1.6 and height 3 (times `size`) with
/// two rows of five radial holes of radius 0.7 through its wall.
fn drilled_pipe(size: f64) -> Body {
    let tol = Tolerance::default();
    let run = |a: &Body, b: &Body| boolean(a, b, BooleanOp::Subtract, &tol).unwrap().body;
    let outer = primitives::cylinder(&Frame::WORLD, 2.0 * size, 3.0 * size).unwrap();
    let bore = primitives::cylinder(
        &Frame::WORLD.at(Vec3::new(0.0, 0.0, -size)).unwrap(),
        1.6 * size,
        5.0 * size,
    )
    .unwrap();
    let mut pipe = run(&outer, &bore);
    for (row, z) in [0.9, 2.1].into_iter().enumerate() {
        for k in 0..5 {
            let angle = 0.37 + TAU / 5.0 * (k as f64 + 0.5 * row as f64);
            let out = Vec3::new(angle.cos(), angle.sin(), 0.0);
            // From inside the bore to beyond the wall.
            let frame = Frame::new(out * size + Vec3::Z * (z * size), out, Vec3::Z).unwrap();
            let drill = primitives::cylinder(&frame, 0.7 * size, 2.0 * size).unwrap();
            pipe = run(&pipe, &drill);
        }
    }
    pipe
}

fn drilled_pipe_meshes_closed(size: f64) {
    let body = drilled_pipe(size);
    assert_eq!(check(&body, &Tolerance::default()), [], "size {size}");
    // The outer wall and the bore each carry all ten holes.
    let walls = body
        .faces
        .iter()
        .filter(|f| matches!(f.surface, Surface::Cylinder(_)) && f.loops.len() == 12)
        .count();
    assert_eq!(
        walls, 2,
        "size {size}: the walls are not what this test is about"
    );
    let exact = mass_properties(&body).unwrap();
    let mut previous = f64::INFINITY;
    for relative in [3e-2, 3e-3, 3e-4] {
        let chord = relative * size;
        let params = TessParams {
            chordal_tolerance: chord,
            ..TessParams::default()
        };
        let mesh = tessellate_body(&body, &params)
            .unwrap_or_else(|e| panic!("size {size} at {chord}: {e}"));
        assert_eq!(
            mesh,
            tessellate_body(&body, &params).unwrap(),
            "size {size}"
        );
        let welded = mesh.weld().unwrap();
        assert!(welded.is_closed(), "size {size} at {chord}: not closed");
        let error = (welded.volume() - exact.volume).abs();
        println!(
            "drilled pipe, size {size:e} at {relative:e}: {} triangles, volume {} against {}, \
             relative error {:e}",
            mesh.triangles.len(),
            welded.volume(),
            exact.volume,
            error / exact.volume
        );
        assert!(
            error <= exact.area * chord,
            "size {size} at {chord}: volume {} is not {}",
            welded.volume(),
            exact.volume
        );
        assert!(error <= previous, "size {size} at {chord}: not converging");
        previous = error;
    }
    assert!(previous < 1e-3 * exact.volume, "size {size}: {previous}");
}

#[test]
fn a_pipe_with_holes_on_every_longitude_meshes_closed() {
    drilled_pipe_meshes_closed(1.0);
}

#[test]
fn a_small_pipe_with_holes_on_every_longitude_meshes_closed() {
    drilled_pipe_meshes_closed(1e-3);
}

#[test]
fn a_large_pipe_with_holes_on_every_longitude_meshes_closed() {
    drilled_pipe_meshes_closed(1e3);
}
