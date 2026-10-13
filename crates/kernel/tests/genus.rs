//! The genus `topo::genus` gives for what the booleans make of a torus:
//! results whose torus face keeps the torus's handle with holes in it,
//! loses it to a cut, or is a cavity. The expected genus of each is what
//! the solid is (how many holes go through it), not a count.
//!
//! A boolean may still refuse a pair (`Unsupported`); that is printed and
//! is not this test's subject. What it returns must have the genus stated.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, check, genus};

const MAJOR: f64 = 3.0;
const MINOR: f64 = 1.0;

fn at(k: f64, x: f64, y: f64, z: f64) -> Frame {
    Frame::WORLD.at(Vec3::new(x, y, z) * k).unwrap()
}

/// A torus about the z axis through the origin.
fn torus(k: f64) -> Body {
    primitives::torus(&Frame::WORLD, MAJOR * k, MINOR * k).unwrap()
}

/// A torus about the x axis through the origin: standing on edge.
fn upright_torus(k: f64) -> Body {
    let frame = Frame::new(Vec3::ZERO, Vec3::X, Vec3::Y).unwrap();
    primitives::torus(&frame, MAJOR * k, MINOR * k).unwrap()
}

fn block(k: f64, lo: [f64; 3], hi: [f64; 3]) -> Body {
    primitives::block(
        &at(k, lo[0], lo[1], lo[2]),
        (hi[0] - lo[0]) * k,
        (hi[1] - lo[1]) * k,
        (hi[2] - lo[2]) * k,
    )
    .unwrap()
}

struct Case {
    name: &'static str,
    a: Body,
    b: Body,
    op: BooleanOp,
    genus: u32,
    shells: usize,
}

fn cases(k: f64) -> Vec<Case> {
    use BooleanOp::{Intersect, Subtract, Union};
    let case = |name, a, b, op, genus, shells| Case {
        name,
        a,
        b,
        op,
        genus,
        shells,
    };
    // A drill along the torus's axis, through the middle of its tube.
    let drill = primitives::cylinder(&at(k, MAJOR, 0.0, -3.0), 0.4 * k, 6.0 * k).unwrap();
    // Everything on the +y side of the plane through the axis.
    let half_space = block(k, [-6.0, 0.0, -3.0], [6.0, 6.0, 3.0]);
    // Everything above the plane of the tube's centre.
    let above = block(k, [-6.0, -6.0, 0.0], [6.0, 6.0, 3.0]);
    // A block the torus lies in, and one whose top is the plane z = 0.
    let around = block(k, [-6.0, -6.0, -3.0], [6.0, 6.0, 3.0]);
    let below = block(k, [-6.0, -6.0, -4.0], [6.0, 6.0, 0.0]);
    let pin = primitives::cylinder(&at(k, 0.0, 0.0, -3.0), 0.7 * k, 6.0 * k).unwrap();
    let ball = primitives::sphere(&Frame::WORLD, 2.0 * k).unwrap();
    vec![
        // The torus's own handle and the drill's hole.
        case(
            "a torus with a hole drilled through its tube",
            torus(k),
            drill,
            Subtract,
            2,
            1,
        ),
        // Cut through the axis each half is a bent rod.
        case(
            "half a torus, cut through its axis",
            torus(k),
            half_space.clone(),
            Intersect,
            0,
            1,
        ),
        case("the other half", torus(k), half_space, Subtract, 0, 1),
        // Cut across the axis each half is a ring still.
        case(
            "the upper half of a torus",
            torus(k),
            above.clone(),
            Intersect,
            1,
            1,
        ),
        case("the lower half of a torus", torus(k), above, Subtract, 1, 1),
        // A void in the shape of a torus: the block's shell and a cavity
        // with one handle.
        case(
            "a block with a torus-shaped tunnel closed in it",
            around,
            torus(k),
            Subtract,
            1,
            2,
        ),
        // A groove all the way round in the top of a block, around an
        // island: nothing goes through.
        case(
            "a block with a ring-shaped groove",
            below.clone(),
            torus(k),
            Subtract,
            0,
            1,
        ),
        // Half a ring lying on a block: its hole is a pit.
        case(
            "a ring half sunk into a block",
            below.clone(),
            torus(k),
            Union,
            0,
            1,
        ),
        // Half a ring standing on a block is a handle to hold it by.
        case(
            "a block with a handle",
            below.clone(),
            upright_torus(k),
            Union,
            1,
            1,
        ),
        // And the trench it leaves is open along its whole length.
        case(
            "a block with half a ring cut out of its top",
            below,
            upright_torus(k),
            Subtract,
            0,
            1,
        ),
        case("a ball with a hole through it", ball, pin, Subtract, 1, 1),
    ]
}

#[test]
fn what_the_booleans_make_of_a_torus_has_the_genus_it_has() {
    let mut solved = 0;
    let mut refused = Vec::new();
    for k in [1e-3, 1.0, 1e6] {
        let tolerance = Tolerance::new(1e-6 * k, 1e-9).unwrap();
        for case in cases(k) {
            let what = format!("{} at scale {k}", case.name);
            match boolean(&case.a, &case.b, case.op, &tolerance) {
                Ok(result) => {
                    assert_eq!(check(&result.body, &tolerance), vec![], "{what}");
                    assert_eq!(result.body.shells.len(), case.shells, "{what}: shells");
                    assert_eq!(genus(&result.body), Some(case.genus), "{what}");
                    solved += 1;
                }
                Err(KernelError::Unsupported { reason, .. }) => {
                    refused.push(format!("{what}: {reason}"));
                }
                Err(other) => panic!("{what}: {other}"),
            }
        }
    }
    println!("{solved} solved, {} refused", refused.len());
    for line in &refused {
        println!("  refused: {line}");
    }
    // The cuts by planes are the booleans' own tested ground
    // (`boolean_surfaces.rs`); if none of this is solved the test says
    // nothing.
    assert!(solved >= 24, "only {solved} solved");
}
