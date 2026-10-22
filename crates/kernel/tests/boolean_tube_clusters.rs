//! Clusters of tubes (found 2026-10-22 by bool-speed-3 while timing the
//! covers): four patterns of five thin tubes (radius 10, wall 0.3125,
//! length 15) turned a fifth of a turn each about an upright axis, the
//! patterns 3 along x and 1.7 along y apart, united one after the other.
//! The third union is refused, "an imprinted face could not be divided into
//! regions; this is a bug" (`regions::trace` meets a use of an arc twice
//! while following the loops: the order of the arcs around some vertex of
//! face 55 of the first operand is not a turn). Refused the same at
//! b133a34 (before the covers of freeform faces and the quartic turning
//! points) and with covers off: owed, not a regression.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::Body;
use std::f64::consts::TAU;

fn tolerance() -> Tolerance {
    Tolerance {
        linear: 1e-6,
        ..Tolerance::default()
    }
}

fn union(a: &Body, b: &Body) -> Body {
    boolean(a, b, BooleanOp::Union, &tolerance()).unwrap().body
}

/// A tube from `start` along `axis`: a cylinder less a longer, thinner one.
fn tube(start: Vec3, axis: Vec3) -> Body {
    let frame = Frame::new(start, axis, Vec3::new(0.0, 0.0, 1.0)).unwrap();
    let hole = Frame::new(
        start - axis.normalized().unwrap() * 15.0,
        axis,
        Vec3::new(0.0, 0.0, 1.0),
    )
    .unwrap();
    boolean(
        &primitives::cylinder(&frame, 10.0, 15.0).unwrap(),
        &primitives::cylinder(&hole, 9.6875, 45.0).unwrap(),
        BooleanOp::Subtract,
        &tolerance(),
    )
    .unwrap()
    .body
}

/// Five tubes turned about the upright axis through `about`, united.
fn pattern(about: Vec3) -> Body {
    let mut all: Option<Body> = None;
    for i in 0..5 {
        let turn =
            Transform::rotation_about(about, Vec3::new(0.0, 0.0, 1.0), f64::from(i) * TAU / 5.0)
                .unwrap();
        let one = tube(
            turn.point(Vec3::ZERO),
            turn.vector(Vec3::new(0.0, -1.0, 0.0)),
        );
        all = Some(match all {
            None => one,
            Some(so_far) => union(&so_far, &one),
        });
    }
    all.unwrap()
}

#[test]
#[ignore = "owed: the third union is refused as a bug of regions::trace (2026-10-22)"]
fn four_patterns_of_five_tubes_unite() {
    let at = Vec3::new(7.5, 2.5, 0.0);
    let mut all: Option<Body> = None;
    for j in 0..4 {
        let one = pattern(at + Vec3::new(3.0 * f64::from(j), 1.7 * f64::from(j), 0.0));
        all = Some(match all {
            None => one,
            Some(so_far) => match boolean(&so_far, &one, BooleanOp::Union, &tolerance()) {
                Ok(r) => r.body,
                Err(e) => panic!("union {j}: {e:?}"),
            },
        });
    }
    let body = all.unwrap();
    assert!(craft3d_topo::check(&body, &tolerance()).is_empty());
}
