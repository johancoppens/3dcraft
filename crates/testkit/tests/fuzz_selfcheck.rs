//! Findings of the random-design fuzzer (docs/testing.md, oracle 11) of
//! 2026-10-20 that were the self-check's or the tessellator's fault, not
//! the kernel's, each reduced to the body that fooled it. The fuzzer's
//! own reproducers are `crates/engine/tests/fuzz_findings/`.
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
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_tess::{SelfCheckParams, TessParams, body_self_intersections, tessellate_body};
use craft3d_topo::check;

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

/// Seed 20754684: a cylinder wall with a slot cut through it by a block
/// that leaves, beyond the block's far face, a skin of the wall thinner
/// than the first mesh's chord: the wall's face narrows there to an
/// isthmus bounded by two lines in the plane of that face, and the
/// isthmus bulges out of the plane by less than the chord. Its mesh
/// gets no point inside and lies in the plane exactly, as the mesh of the
/// flat face does: the self-check took the two for faces lying on each
/// other (an overlap in one plane, "which no approximation brings
/// about"), measured it 0.46 deep in the plane against a margin of
/// 0.095, and called a valid body wrong. The surfaces are asked now: an
/// overlap of meshes is an overlap of faces only where the faces
/// themselves are at the same place.
#[test]
fn a_skin_of_a_cylinder_meshed_flat_is_no_overlap() {
    for k in SCALES {
        let tol = tolerance(k);
        // A cylinder of radius 25, 1500 long (so that the first chord, a
        // thousandth of the diagonal, is 1.5: more than the skin's 1, and
        // the rims' halves get five chords each, none of whose points
        // lies in the isthmus), with a slot cut by a block 4 thick whose
        // far face at y = 24 leaves a skin 1 thick. The chord of y = 24
        // is 14 wide (a 7-24-25 triangle: the lines come out exact).
        let cylinder = primitives::cylinder(&Frame::WORLD, 25.0 * k, 1500.0 * k).unwrap();
        let slot = Frame::WORLD.at(v(-50.0 * k, 20.0 * k, 750.0 * k)).unwrap();
        let block = primitives::block(&slot, 100.0 * k, 4.0 * k, 20.0 * k).unwrap();
        let slotted = boolean(&cylinder, &block, BooleanOp::Subtract, &tol)
            .unwrap()
            .body;
        assert_eq!(check(&slotted, &tol), [], "scale {k}");
        assert_eq!(slotted.shells.len(), 1, "scale {k}");
        // One mesh only: the overlap of the meshes is there, and is a
        // finding within the margin, since the surfaces are a skin apart.
        let coarse = SelfCheckParams {
            refinements: 0,
            ..SelfCheckParams::new(tol.linear)
        };
        let found = body_self_intersections(&slotted, &coarse).unwrap();
        assert!(found.chordal_tolerance > 1.0 * k, "scale {k}: {found}");
        assert_eq!(found.certain_crossings, 0, "scale {k}: {found}");
        assert!(found.marginal_crossings > 0, "scale {k}: {found}");
        // Looked at again finer, the strip gets its points and nothing is
        // left.
        let found = body_self_intersections(&slotted, &SelfCheckParams::new(tol.linear)).unwrap();
        assert!(found.is_spotless(), "scale {k}: {found}");
        assert!(found.passes > 1, "scale {k}: {found}");
    }
}

/// Seed 20792240: three tori stacked by their tube's radius and united,
/// a ball taken out where the inner rims of two of them meet, and the
/// body mirrored. The mirror negates the tube parameter of every torus
/// (`geom::transform`), and the mirrored face with the ball's bite could
/// not be meshed: "a triangle 3.6e-11 across is still off the surface or
/// faces against it", at every mirror plane tried. The body before the
/// mirror meshes fine, and so does the mirror image of the tori alone.
#[test]
fn a_mirrored_torus_with_a_bite_is_meshed() {
    for k in SCALES {
        let tol = tolerance(k);
        let (major, minor) = (7.5 * k, 5.0 * k);
        let base = Frame::WORLD.at(v(0.0, -7.5 * k, 7.5 * k)).unwrap();
        let mut body = primitives::torus(&base, major, minor).unwrap();
        for i in 1..3 {
            let frame = base
                .at(base.origin() + v(0.0, 0.0, 5.0 * k * i as f64))
                .unwrap();
            let next = primitives::torus(&frame, major, minor).unwrap();
            body = boolean(&body, &next, BooleanOp::Union, &tol).unwrap().body;
        }
        let centre = Frame::WORLD
            .at(v(2.2414386804201335 * k, -5.258561319579866 * k, 15.0 * k))
            .unwrap();
        let ball = primitives::sphere(&centre, 2.5 * k).unwrap();
        let bitten = boolean(&body, &ball, BooleanOp::Subtract, &tol)
            .unwrap()
            .body;
        assert_eq!(check(&bitten, &tol), [], "scale {k}");
        let params = TessParams {
            chordal_tolerance: 2.5e-3 * k,
            ..TessParams::default()
        };
        tessellate_body(&bitten, &params).unwrap();
        for normal in [v(1.0, 1.0, 0.0), Vec3::X, Vec3::Y, Vec3::Z] {
            let mirror = Transform::mirror(v(0.0, -5.0 * k, 15.0 * k), normal).unwrap();
            let image = bitten.transformed(&mirror).unwrap();
            assert_eq!(check(&image, &tol), [], "scale {k}, normal {normal:?}");
            if let Err(e) = tessellate_body(&image, &params) {
                panic!("scale {k}, normal {normal:?}: {e}");
            }
        }
    }
}
