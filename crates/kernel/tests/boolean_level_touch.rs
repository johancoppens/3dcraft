//! A union whose level faces meet along a line where the two bodies part
//! (fuzz seeds 110965, 116424, 111609 and 115646, found 2026-11-21;
//! docs/design/booleans.md › *A union whose level tops meet along a line
//! where the bodies part*). Block A, 5 × 10 × 5, its side x = 5 cut back
//! over y 5..10 down from its top edge (by a plane through that edge, or by
//! a cylinder tangent to the side along it), and block B beside it, its
//! top level with A's. They share the side over y 0..5 and meet only along
//! the top edge over y 5..10, a groove opening below it: the union is one
//! shell that touches itself along that line, two parts of the solid that
//! meet with nothing between them. That is zero thickness, refused by the
//! developer's decision of 2026-10-15 (*A shell that touches itself*: the
//! material lies on the far sides of the walls that meet there, not
//! between them, so it is not a wall that thins to nothing).
//!
//! It was refused as *"boolean produced an invalid body"*: the merge joined
//! the two tops over the stretch where the blocks share their side into one
//! face, which then ran along the line on both sides of it, and the
//! boolean's own check refused that slit before the touch was looked for.
//! The merge now never joins faces that use two of the sewing's copies of
//! one arc (`boolean/merge.rs`).
//!
//! Held here, each through `solve_parts` (refused as zero thickness at a
//! place on the line; made without the refusal, valid, one shell, of the
//! volume in closed form, passing the boolean's own check in full, and
//! touching itself at the place by a witness that asks nothing of the
//! boolean): the reduction at four scales either way round, turned out of
//! the axes, cut back by a tangent cylinder, B reaching into A over the
//! stretch they share; and beside it what builds, and B's top higher,
//! which was refused in these words already.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::*;
use craft3d_geom::Surface;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Portable, Transform, Vec3};
use craft3d_topo::Body;

use BooleanOp::{Subtract, Union};

/// The scales of the seeds (1e-3, 1 and 1e3) and the far one.
const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

/// The polygon `points` (times `k`) in `plane`, extruded from `from` to
/// `to` (times `k`) along its normal.
fn prism(k: f64, plane: &Frame, points: &[[f64; 2]], from: f64, to: f64) -> Body {
    let tol = tolerance(k);
    let curves: Vec<SketchCurve> = (0..points.len())
        .map(|i| SketchCurve {
            id: i as u64 + 1,
            curve: Curve2::Line {
                start: points[i].map(|c| c * k),
                end: points[(i + 1) % points.len()].map(|c| c * k),
            },
        })
        .collect();
    extrude(
        &detect_profiles(&curves, &tol).unwrap(),
        plane,
        from * k,
        to * k,
        &tol,
    )
    .unwrap()
    .body
}

/// `a` less `b`, which must build.
fn less(a: &Body, b: &Body, k: f64) -> Body {
    boolean(a, b, Subtract, &tolerance(k))
        .unwrap_or_else(|e| panic!("an operand at scale {k}: {e}"))
        .body
}

fn block_a(k: f64) -> Body {
    boxed(k, [0.0, 0.0, 0.0], [5.0, 10.0, 5.0])
}

fn block_b(k: f64) -> Body {
    boxed(k, [5.0, 0.0, 0.0], [10.0, 10.0, 5.0])
}

/// Block A less the wedge between its side x = 5 and the plane
/// x = 3 + 0.4 z over y 5..10 (the seed's sketch on the zx plane at
/// y = 5, extruded 6): the plane passes through A's top edge. 225 in
/// volume.
fn slanted(k: f64) -> Body {
    // The zx plane at y = 5: sketch x along z, sketch y along x.
    let plane = Frame::new(Vec3::new(0.0, 5.0 * k, 0.0), Vec3::Y, Vec3::Z).unwrap();
    let wedge = prism(
        k,
        &plane,
        &[[6.0, 5.4], [-1.0, 2.6], [-1.0, 7.0], [6.0, 7.0]],
        0.0,
        6.0,
    );
    less(&block_a(k), &wedge, k)
}

/// What the cylinder of [`rounded`] takes off A's end over y 5..10, in
/// the xz plane: under x = 5 and outside the circle of radius 8 about
/// (-3, 5), for z from 0 to 5, `∫ (8 − sqrt(64 − s²)) ds` over s from 0
/// to 5.
fn rounded_off() -> f64 {
    40.0 - 2.5 * 39.0_f64.sqrt() - 32.0 * 0.625_f64.pasin()
}

/// Block A cut back over y 5..10 by the cylinder of radius 8 about the
/// line x = -3, z = 5, which is tangent to the side x = 5 along A's top
/// edge: A less (its half over y 5..10 less the cylinder). Volume
/// `250 − 5 rounded_off()`.
fn rounded(k: f64) -> Body {
    let half = boxed(k, [0.0, 5.0, 0.0], [5.0, 10.0, 5.0]);
    let axis = Frame::new(Vec3::new(-3.0 * k, 4.0 * k, 5.0 * k), Vec3::Y, Vec3::X).unwrap();
    let round = primitives::cylinder(&axis, 8.0 * k, 7.0 * k).unwrap();
    less(&block_a(k), &less(&half, &round, k), k)
}

/// Whether `at` lies on the line where the blocks part (x = 5, z = 5,
/// y from 5 to 10, times `k`), within the tolerance.
fn on_the_line(at: Vec3, k: f64) -> bool {
    let tol = tolerance(k).linear;
    (at.x - 5.0 * k).abs() <= tol
        && (at.z - 5.0 * k).abs() <= tol
        && at.y >= 5.0 * k - tol
        && at.y <= 10.0 * k + tol
}

/// The faces of `body` on the plane z = 5 (times `k`).
fn level_tops(body: &Body, k: f64) -> usize {
    let top = Vec3::new(0.0, 0.0, 5.0 * k);
    body.faces
        .iter()
        .filter(|face| match &face.surface {
            Surface::Plane(plane) => {
                let frame = plane.frame();
                frame.z().z.abs() > 1.0 - 1e-12
                    && (top - frame.origin()).dot(frame.z()).abs() <= 1e-9 * k
            }
            _ => false,
        })
        .count()
}

/// The union of `x` and `y`, either way round, refused as zero thickness
/// at a place `on` accepts; made without the refusal one shell of volume
/// `volume` (given for scale 1). Gives the bodies made without it.
#[track_caller]
fn refused_along_the_line(
    x: &Body,
    y: &Body,
    k: f64,
    volume: f64,
    on: &dyn Fn(Vec3) -> bool,
) -> Vec<Body> {
    let mut made = Vec::new();
    for (first, second) in [(x, y), (y, x)] {
        let (both, at) = solve_parts(first, second, Union, k);
        let at = at.unwrap_or_else(|| panic!("not refused at scale {k}"));
        assert!(on(at), "refused at {at:?}, not on the line, at scale {k}");
        assert_eq!(both.body.shells.len(), 1, "scale {k}");
        assert_close(mass(&both.body).volume, volume * k * k * k, "volume");
        made.push(both.body);
    }
    made
}

/// The reduction of the four seeds: refused at the line in its own words,
/// either way round. Made without the refusal, the two tops stay two
/// faces (joined over the stretch where the blocks share their side, they
/// would be one face running along the line on both sides of it), and the
/// body passes `topo::check` (in `solve_parts`).
#[test]
fn a_block_cut_back_to_its_top_edge_beside_a_block() {
    for k in SCALES {
        let on = |at: Vec3| on_the_line(at, k);
        for body in refused_along_the_line(&slanted(k), &block_b(k), k, 475.0, &on) {
            assert_eq!(level_tops(&body, k), 2, "scale {k}");
        }
    }
}

/// The same with A cut back by a cylinder tangent to its side along the
/// top edge (a fillet's round of the seeds 110965 and 116424): the walls
/// that meet at the line are tangent there, materials on their far sides.
#[test]
fn a_block_rounded_back_to_its_top_edge_beside_a_block() {
    for k in SCALES {
        let volume = 500.0 - 5.0 * rounded_off();
        let on = |at: Vec3| on_the_line(at, k);
        for body in refused_along_the_line(&rounded(k), &block_b(k), k, volume, &on) {
            assert_eq!(level_tops(&body, k), 2, "scale {k}");
        }
    }
}

/// Both cases turned out of the axes and moved off the origin: the place
/// is on the line turned with them.
#[test]
fn turned_out_of_the_axes() {
    for k in SCALES {
        let turn = Transform::rotation(Vec3::new(0.4, -1.0, 0.7), 1.1)
            .unwrap()
            .then(&Transform::translation(Vec3::new(3.0, -2.0, 7.0) * k));
        let back = turn.inverse().unwrap();
        let on = |at: Vec3| on_the_line(back.point(at), k);
        let b = block_b(k).transformed(&turn).unwrap();
        for (a, volume) in [
            (slanted(k), 475.0),
            (rounded(k), 500.0 - 5.0 * rounded_off()),
        ] {
            refused_along_the_line(&a.transformed(&turn).unwrap(), &b, k, volume, &on);
        }
    }
}

/// B an L that reaches into A over the stretch where they share the side
/// (x 4..10 over y 0..5, x 5..10 over y 5..10): A's top falls into a piece
/// outside B and one on B's top, and B's top outside A is joined to the
/// second; the first, which runs along the line where the blocks part, is
/// the one that must stay apart. Volume 225 + 275 − 25.
#[test]
fn a_block_reaching_into_the_cut_one_over_the_side_they_share() {
    for k in SCALES {
        let ell = prism(
            k,
            &Frame::WORLD,
            &[
                [4.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [5.0, 10.0],
                [5.0, 5.0],
                [4.0, 5.0],
            ],
            0.0,
            5.0,
        );
        let on = |at: Vec3| on_the_line(at, k);
        refused_along_the_line(&slanted(k), &ell, k, 475.0, &on);
        refused_along_the_line(&rounded(k), &ell, k, 500.0 - 5.0 * rounded_off(), &on);
    }
}

/// What lies beside the case. Built: B moved 0.05 into A (they overlap
/// along the whole side: 225 + 250 − 1.25 − 0.015625, the last the sliver
/// of B above the plane x = 3 + 0.4 z), B moved 0.05 away (two lumps),
/// B along the shared stretch only (no line where they part), and B
/// beside A uncut (one block, its tops and sides merged: 6 faces).
/// Refused at the line as before the repair: B's top 0.05 higher (its
/// side then stands above the line). Refused at the line, as the level
/// case: B moved by 0.45 of the tolerance either way, within which the
/// boolean takes the side as shared.
#[test]
fn what_lies_beside_it() {
    for k in [1e-3, 1.0, 1e6] {
        let cut = slanted(k);
        let into = boxed(k, [4.95, 0.0, 0.0], [9.95, 10.0, 5.0]);
        let both = solve(&cut, &into, Union, k);
        assert_close(mass(&both.body).volume, 473.734_375 * k * k * k, "into");
        assert_eq!(both.body.shells.len(), 1);
        let away = boxed(k, [5.05, 0.0, 0.0], [10.05, 10.0, 5.0]);
        let both = solve(&cut, &away, Union, k);
        assert_close(mass(&both.body).volume, 475.0 * k * k * k, "away");
        assert_eq!(both.body.shells.len(), 2);
        let shared = boxed(k, [5.0, 0.0, 0.0], [10.0, 5.0, 5.0]);
        let both = solve(&cut, &shared, Union, k);
        assert_close(mass(&both.body).volume, 350.0 * k * k * k, "shared");
        assert_eq!(both.body.shells.len(), 1);
        let whole = solve(&block_a(k), &block_b(k), Union, k);
        assert_close(mass(&whole.body).volume, 500.0 * k * k * k, "whole");
        assert_eq!(whole.body.faces.len(), 6);
        let on = |at: Vec3| on_the_line(at, k);
        let higher = boxed(k, [5.0, 0.0, 0.0], [10.0, 10.0, 5.05]);
        refused_along_the_line(&cut, &higher, k, 477.5, &on);
        for shift in [-0.45, 0.45] {
            let x = 5.0 + shift * 1e-6;
            let moved = boxed(k, [x, 0.0, 0.0], [10.0, 10.0, 5.0]);
            for (first, second) in [(&cut, &moved), (&moved, &cut)] {
                let (both, at) = solve_parts(first, second, Union, k);
                let at = at.unwrap_or_else(|| panic!("moved {shift}: not refused at scale {k}"));
                assert!(on(at), "moved {shift}: refused at {at:?} at scale {k}");
                assert_eq!(both.body.shells.len(), 1);
            }
        }
    }
}
