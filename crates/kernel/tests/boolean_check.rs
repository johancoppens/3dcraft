//! The boolean's own check (`boolean::verify`): it passes every correct
//! result, and it catches results that are wrong in the ways a boolean
//! can be wrong (seeded mutations of correct results).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;

use boolean_common::*;
use craft3d_kernel::boolean::{
    BooleanOp, Operand, Verification, VerifyOptions, Violation, VolumeRule, boolean, verify,
};
use craft3d_math::{Transform, Vec3};
use craft3d_topo::Body;

fn full(a: &Body, b: &Body, op: BooleanOp, result: &Body, k: f64) -> Verification {
    verify(a, b, op, result, &tolerance(k), &VerifyOptions::default()).unwrap()
}

/// A plate and a rod through one corner of it: most of the plate's faces
/// are far from the rod.
fn plate_and_rod(k: f64) -> (Body, Body) {
    (
        boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 1.0]),
        rod(k, [0.5, 0.5, -1.0], 0.25, 3.0),
    )
}

/// A box and a ball inside it: the cavity.
fn box_and_ball(k: f64) -> (Body, Body) {
    (
        boxed(k, [0.0, 0.0, 0.0], [2.0, 2.0, 2.0]),
        ball(k, [1.0, 1.0, 1.0], 0.5),
    )
}

#[test]
fn correct_results_pass_at_every_scale() {
    let ops = [BooleanOp::Union, BooleanOp::Subtract, BooleanOp::Intersect];
    for k in SCALES {
        let pairs = [
            plate_and_rod(k),
            box_and_ball(k),
            (
                boxed(k, [0.0, 0.0, 0.0], [2.0, 2.0, 2.0]),
                boxed(k, [1.0, 1.0, 1.0], [3.0, 3.0, 3.0]),
            ),
            (
                ball(k, [0.0, 0.0, 0.0], 1.0),
                ring(k, [0.5, 0.0, 0.0], 1.0, 0.3),
            ),
            (
                rod(k, [0.0, 0.0, 0.0], 1.0, 2.0),
                spike(k, [0.3, 0.0, 1.0], 0.6, 2.0),
            ),
        ];
        for (index, (a, b)) in pairs.iter().enumerate() {
            for op in ops {
                let Ok(result) = boolean(a, b, op, &tolerance(k)) else {
                    continue;
                };
                let found = full(a, b, op, &result.body, k);
                assert!(
                    found.is_ok(),
                    "pair {index} {op:?} at {k}: {:?}",
                    found.violations
                );
                let volumes = found.volumes.expect("measured");
                assert!(
                    volumes.allowance > 0.0 && volumes.allowance < 1e-4 * volumes.a.abs(),
                    "pair {index} {op:?} at {k}: allowance {volumes:?}"
                );
                assert!(
                    found.points > 0,
                    "pair {index} {op:?} at {k}: no point told"
                );
            }
        }
        // The plate's faces far from the rod were matched.
        let (a, b) = plate_and_rod(k);
        let result = boolean(&a, &b, BooleanOp::Subtract, &tolerance(k)).unwrap();
        assert_eq!(
            full(&a, &b, BooleanOp::Subtract, &result.body, k).far_faces,
            4
        );
    }
}

/// The far faces of `a` from `b` (rule 3), by the check's own count: the
/// face of `a` that `face` is, found in the result by its surface.
fn result_face_on(result: &Body, a: &Body, face: usize) -> usize {
    let surface = &a.faces[face].surface;
    result
        .faces
        .iter()
        .position(|f| &f.surface == surface)
        .expect("a far face is in the result")
}

/// The plate face at x = 4, far from the rod at the corner by x = 0.5.
fn far_plate_face(a: &Body) -> usize {
    (0..a.faces.len())
        .position(|i| {
            let (lo, _) = a
                .face_bbox(craft3d_topo::FaceId(i as u32))
                .unwrap()
                .corners()
                .unwrap();
            lo.x > 3.0 * a.vertices.iter().map(|v| v.point.x).fold(0.0, f64::max) / 4.0
        })
        .expect("the face at x = 4")
}

fn has(found: &Verification, wanted: impl Fn(&Violation) -> bool) -> bool {
    found.violations.iter().any(wanted)
}

#[test]
fn a_dropped_face_of_the_target_is_caught() {
    for k in SCALES {
        let (a, b) = plate_and_rod(k);
        let mut result = boolean(&a, &b, BooleanOp::Subtract, &tolerance(k))
            .unwrap()
            .body;
        let gone = result_face_on(&result, &a, far_plate_face(&a));
        result.faces.remove(gone);
        let found = full(&a, &b, BooleanOp::Subtract, &result, k);
        assert!(
            has(&found, |v| matches!(
                v,
                Violation::FarFace { operand: Operand::A, problem, .. } if problem.contains("no face")
            )),
            "at {k}: {:?}",
            found.violations
        );
        // Without the volumes and the points too: it is what every
        // boolean checks of its own result.
        let cheap = verify(
            &a,
            &b,
            BooleanOp::Subtract,
            &result,
            &tolerance(k),
            &VerifyOptions::ALWAYS,
        )
        .unwrap();
        assert!(!cheap.is_ok(), "at {k}");
    }
}

#[test]
fn a_subtraction_that_adds_the_tool_is_caught() {
    for k in SCALES {
        let (a, b) = plate_and_rod(k);
        // The union passed off as the difference.
        let union = boolean(&a, &b, BooleanOp::Union, &tolerance(k))
            .unwrap()
            .body;
        let found = full(&a, &b, BooleanOp::Subtract, &union, k);
        assert!(
            has(&found, |v| matches!(
                v,
                Violation::Volume {
                    rule: VolumeRule::SubtractAdds,
                    ..
                }
            )),
            "at {k}: {:?}",
            found.violations
        );
        assert!(
            has(&found, |v| matches!(v, Violation::OutOfReach { .. })),
            "at {k}"
        );
        assert!(
            has(&found, |v| matches!(v, Violation::Membership { .. })),
            "at {k}"
        );
        // And the difference passed off as the intersection, the union
        // as the intersection.
        let less = boolean(&a, &b, BooleanOp::Subtract, &tolerance(k))
            .unwrap()
            .body;
        let found = full(&a, &b, BooleanOp::Intersect, &less, k);
        assert!(
            has(&found, |v| matches!(
                v,
                Violation::Volume {
                    rule: VolumeRule::IntersectGains,
                    ..
                }
            )),
            "at {k}: {:?}",
            found.violations
        );
        let found = full(&a, &b, BooleanOp::Union, &less, k);
        assert!(
            has(&found, |v| matches!(
                v,
                Violation::Volume {
                    rule: VolumeRule::UnionLoses,
                    ..
                }
            )),
            "at {k}: {:?}",
            found.violations
        );
    }
}

#[test]
fn a_moved_or_turned_far_face_is_caught() {
    for k in SCALES {
        let tol = tolerance(k).linear;
        let (a, b) = plate_and_rod(k);
        let good = boolean(&a, &b, BooleanOp::Subtract, &tolerance(k))
            .unwrap()
            .body;
        let at = result_face_on(&good, &a, far_plate_face(&a));
        // The surface moved by ten tolerances, its edges left where they
        // were.
        let mut moved = good.clone();
        let shift = Transform::translation(Vec3::new(10.0 * tol, 0.0, 0.0));
        moved.faces[at].surface = moved.faces[at].surface.transformed(&shift).unwrap();
        let found = full(&a, &b, BooleanOp::Subtract, &moved, k);
        assert!(
            has(&found, |v| matches!(
                v,
                Violation::FarFace { problem, .. } if problem.contains("surface")
            )),
            "at {k}: {:?}",
            found.violations
        );
        // The whole face with its edges and vertices moved: a vertex of it.
        let mut shifted = good.clone();
        let corner = shifted.edges[shifted.loops[shifted.faces[at].loops[0].0 as usize].coedges[0]
            .edge
            .0 as usize]
            .start;
        shifted.vertices[corner.0 as usize].point += Vec3::new(10.0 * tol, 0.0, 0.0);
        let found = full(&a, &b, BooleanOp::Subtract, &shifted, k);
        assert!(
            has(&found, |v| matches!(v, Violation::FarFace { .. })
                || matches!(v, Violation::OutOfReach { .. })),
            "at {k}: {:?}",
            found.violations
        );
        // Turned inside out.
        let mut turned = good.clone();
        turned.faces[at].reversed = !turned.faces[at].reversed;
        let found = full(&a, &b, BooleanOp::Subtract, &turned, k);
        assert!(
            has(&found, |v| matches!(
                v,
                Violation::FarFace { problem, .. } if problem.contains("inside out")
            )),
            "at {k}: {:?}",
            found.violations
        );
        // An edge of it cut short: its middle is no longer where it was.
        let mut bent = good;
        let first = bent.loops[bent.faces[at].loops[0].0 as usize].coedges[0].edge;
        let edge = &mut bent.edges[first.0 as usize];
        let (t0, t1) = edge.range;
        edge.range = (t0, 0.5 * (t0 + t1));
        let found = full(&a, &b, BooleanOp::Subtract, &bent, k);
        assert!(
            has(&found, |v| matches!(v, Violation::FarFace { .. })),
            "at {k}: {:?}",
            found.violations
        );
    }
}

#[test]
fn a_filled_cavity_is_caught_by_the_points_alone() {
    for k in SCALES {
        let (a, b) = box_and_ball(k);
        // The box returned as the box minus the ball inside it: the
        // volume is within bounds (no more than the box) and every face
        // of the box is there; only a point inside the ball tells.
        let found = full(&a, &b, BooleanOp::Subtract, &a, k);
        assert!(
            found
                .violations
                .iter()
                .all(|v| matches!(v, Violation::Membership { .. })),
            "at {k}: {:?}",
            found.violations
        );
        assert!(!found.is_ok(), "at {k}");
    }
}

/// The volume allowance is the tolerance band of the operands and the
/// measurement's own bound: a result off by more than that is caught, by
/// less is not.
#[test]
fn the_volume_allowance_is_the_tolerance_band() {
    for k in SCALES {
        let (a, b) = plate_and_rod(k);
        let result = boolean(&a, &b, BooleanOp::Subtract, &tolerance(k))
            .unwrap()
            .body;
        let volumes = full(&a, &b, BooleanOp::Subtract, &result, k)
            .volumes
            .unwrap();
        let areas = mass(&a).area + mass(&b).area;
        let band = 2.0 * tolerance(k).linear * areas;
        assert!(volumes.allowance >= band && volumes.allowance <= 1.01 * band);
        // The target, grown by two allowances' worth of volume across its
        // top, passed off as itself minus a tool far away.
        let lift = 2.0 * volumes.allowance / (4.0 * 3.0 * k * k);
        let far = boxed(k, [10.0, 10.0, 10.0], [11.0, 11.0, 11.0]);
        let grown = boxed(k, [0.0, 0.0, 0.0], [4.0, 3.0, 1.0 + lift / k]);
        let found = full(&a, &far, BooleanOp::Subtract, &grown, k);
        assert!(
            has(&found, |v| matches!(
                v,
                Violation::Volume {
                    rule: VolumeRule::SubtractAdds,
                    ..
                }
            )),
            "at {k}: {:?}",
            found.violations
        );
    }
}
