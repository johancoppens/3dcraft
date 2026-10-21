//! A crack that thins to a point (the Garabit valley's finding 4,
//! 2026-10-21). In pier 2 a girder lies on a leg's web at 1.19°, so the
//! two walls of the crack between them stay within twice the tolerance of
//! each other for some 96 µm from the line where they cross, and a
//! diagonal's end face cuts the crack off 28 µm from where the girder's
//! end does. The union was refused as zero thickness twice over, though
//! its shell nowhere comes back to itself: the walls of the crack meet at
//! one vertex (where the leg, the girder and the diagonal meet) and leave
//! it at 1.19°, and the diagonal's face, between the crack's walls, has a
//! corner of 1.5° there. The boolean's look at two walls of one shell
//! measured how far the place was from their shared edge only where the
//! place's foot was inside the edge (here it is past the edge's end, the
//! vertex); `topo::touches` took the far end of the corner's one side,
//! within 1.5 µm of its other side, for a vertex on an edge. Held to: the
//! three bars as placed in the pier (the leg as the plate of its web), at
//! 1e-3, 1 and 1e6: one shell, valid, no touch, and `vol(U ∪ C) = vol(U) +
//! vol(C) − vol(U ∩ C)`; and a prism whose face has a corner of 1/600 of
//! a radian, which touches nothing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::BooleanOp;
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Transform, Vec3};
use craft3d_topo::{Body, TouchScope, check, touches};

/// A block from `lo` to `hi` in its own axes, turned by `degrees` about
/// `axis` and moved by `to` (as `feature.transform` places a body), all
/// times `k`.
fn placed(k: f64, lo: [f64; 3], hi: [f64; 3], axis: [f64; 3], degrees: f64, to: [f64; 3]) -> Body {
    let frame = Frame::WORLD
        .at(Vec3::new(lo[0] * k, lo[1] * k, lo[2] * k))
        .unwrap();
    let size = |i: usize| (hi[i] - lo[i]) * k;
    let block = primitives::block(&frame, size(0), size(1), size(2)).unwrap();
    let axis = Vec3::new(axis[0], axis[1], axis[2]).normalized().unwrap();
    let place = Transform::rotation(axis, degrees.to_radians())
        .unwrap()
        .then(&Transform::translation(Vec3::new(
            to[0] * k,
            to[1] * k,
            to[2] * k,
        )));
    block.transformed(&place).unwrap()
}

/// The leg's web (the plate of its section the girder lies on), the
/// girder and the diagonal, as `union-pier2-chain-zero-thickness.json`
/// places them.
fn bars(k: f64) -> [Body; 3] {
    let leg = placed(
        k,
        [-0.3, 0.18, 0.0],
        [0.2, 0.3, 35.597086790],
        [-0.061354423798, -0.022369577796, -0.997865339948],
        90.216497735,
        [-202.406245420, -5.456801584, 795.887300000],
    );
    let girder = placed(
        k,
        [-0.2, -0.172, 0.0],
        [0.2, 0.172, 12.035868886],
        [-0.287864085894, 0.315959878342, 0.904048462933],
        95.183908932,
        [-201.781432651, -4.011445283, 811.474159296],
    );
    let diagonal = placed(
        k,
        [-0.09, -0.08, 0.0],
        [0.09, 0.08, 6.195228699],
        [-0.311682593122, -0.031603893655, -0.949660547275],
        178.811438119,
        [-201.787614800, -4.131072960, 811.314000000],
    );
    [leg, girder, diagonal]
}

#[test]
fn a_crack_that_thins_to_a_point_is_one_shell() {
    for k in SCALES {
        let tol = tolerance(k);
        let [leg, girder, diagonal] = bars(k);
        let joined = solve(&leg, &girder, BooleanOp::Union, k).body;
        let all = solve(&joined, &diagonal, BooleanOp::Union, k).body;
        assert_eq!(all.shells.len(), 1, "at {k}");
        let found = touches(&all, 2.0 * tol.linear, 4, TouchScope::All).unwrap();
        assert_eq!(found, [], "at {k}");
        let common = solve(&joined, &diagonal, BooleanOp::Intersect, k).body;
        let (u, c, both, whole) = (
            mass(&joined).volume,
            mass(&diagonal).volume,
            mass(&common).volume,
            mass(&all).volume,
        );
        assert!(
            (whole - (u + c - both)).abs() <= 1e-9 * whole,
            "at {k}: {whole} is not {u} + {c} - {both}"
        );
    }
}

#[test]
fn a_face_with_a_sharp_corner_does_not_touch_itself() {
    for k in SCALES {
        let tol = tolerance(k);
        // A spike: from its tip two sides leave at 1/600 of a radian; at
        // 900 tolerances from the tip they are 1.5 tolerances apart, so
        // the far end of the upper side lies within twice the tolerance of
        // the lower side (a sketch takes them as apart: more than one
        // tolerance), and nowhere near another vertex.
        let t = 1e-6 * k;
        let length = 900.0 * t;
        let corner = [length, 1.5 * t];
        let points = [
            [0.0, 0.0],
            [2.0 * length, 0.0],
            [2.0 * length, length],
            corner,
        ];
        let curves: Vec<SketchCurve> = (0..4)
            .map(|i| SketchCurve {
                id: i as u64 + 1,
                curve: Curve2::Line {
                    start: points[i],
                    end: points[(i + 1) % 4],
                },
            })
            .collect();
        let profiles = detect_profiles(&curves, &tol).unwrap();
        let prism = extrude(&profiles, &Frame::WORLD, 0.0, length, &tol)
            .unwrap()
            .body;
        assert_eq!(check(&prism, &tol), []);
        let found = touches(&prism, 2.0 * tol.linear, 4, TouchScope::All).unwrap();
        assert_eq!(found, [], "at {k}");
    }
}
