//! Faces moved up to what ends them and a little past it: the limits an
//! offset is refused at since 2026-11-14 (`kernel::tweak::passes`,
//! `docs/design/face-operations.md` › *A face moved up to or past what
//! ends its neighbours*), each on a body whose limit is known in closed
//! form, probed a millionth of the limit either side of it, half way to it
//! and well past it, at three scales, on the world frame, a tilted one and
//! a tilted one far from the origin.
//!
//! Short of the limit the body is built, valid, meshed to its measure, and
//! of the volume its closed form gives; past it the offset is refused with
//! the reason and a largest distance that fits which is not past the
//! limit. Every probe prints what it did (`probe ...: built <digest>` or
//! `refused: ...`), so that two programs can be compared on what they
//! build short of a limit, bit for bit.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod hunt_common;

use craft3d_geom::Surface;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{BooleanOp, boolean};
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_kernel::primitives;
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, FaceId};
use hunt_common::{Found, digest, measured, sound, tolerance};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

/// The world, a tilted frame, and a tilted frame far from the origin.
fn frames(k: f64) -> [Frame; 3] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(3.0 * k, -2.0 * k, 5.0 * k),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(1.0, 0.0, 0.2),
        )
        .unwrap(),
        Frame::new(
            Vec3::new(4000.0 * k, -3000.0 * k, 2500.0 * k),
            Vec3::new(-2.0, 1.0, 2.0),
            Vec3::new(0.0, 1.0, -0.5),
        )
        .unwrap(),
    ]
}

/// The frame at `(x, y, z)` of `frame` (scale `k`) with its z along `z`
/// and its x along `x`.
fn at(frame: &Frame, k: f64, p: [f64; 3], z: Vec3, x: Vec3) -> Frame {
    Frame::new(frame.point(p[0] * k, p[1] * k, p[2] * k), z, x).unwrap()
}

/// The block 40 x 30 x 20 times `k` on `frame`.
fn block(frame: &Frame, k: f64) -> Body {
    primitives::block(frame, 40.0 * k, 30.0 * k, 20.0 * k).unwrap()
}

/// The flat face through `p` (in `frame`, scale `k`) whose normal runs
/// along `normal`, either way.
fn flat_face(body: &Body, frame: &Frame, k: f64, p: [f64; 3], normal: Vec3) -> FaceId {
    let point = frame.point(p[0] * k, p[1] * k, p[2] * k);
    let found: Vec<FaceId> = (0..body.faces.len() as u32)
        .map(FaceId)
        .filter(|f| match &body.face(*f).unwrap().surface {
            Surface::Plane(plane) => {
                let n = plane.frame().z();
                n.cross(normal).length() < 1e-9
                    && (point - plane.frame().origin()).dot(n).abs() < 1e-7 * k
            }
            _ => false,
        })
        .collect();
    assert_eq!(found.len(), 1, "one flat face through {p:?}");
    found[0]
}

/// What one probe did.
enum Did {
    Built(Body),
    Refused(KernelError),
}

fn offset(what: &str, body: &Body, face: FaceId, distance: f64, tol: &Tolerance) -> Did {
    match offset_faces(body, &[FaceOffset { face, distance }], tol) {
        Ok(made) => {
            println!("probe {what}: built {:016x}", digest(&made.body));
            Did::Built(made.body)
        }
        Err(error) => {
            println!("probe {what}: refused: {error}");
            Did::Refused(error)
        }
    }
}

/// Holds a probe short of its limit to a body that is built, sound and of
/// `volume` (within the measure's own bound and a tolerance over its
/// area), and one past it to a refusal as too far whose reason holds
/// `reason` and whose largest distance that fits is the limit: not more
/// than eight tolerances past it (a face within a few tolerances of what
/// it would run into cannot be told from one that touches it: the points
/// asked about lie four tolerances off the faces) and within a
/// hundred-thousandth of it. What does not hold goes to `wrong`.
#[allow(clippy::too_many_arguments)]
fn held(
    what: &str,
    did: Did,
    short: Option<bool>,
    volume: f64,
    reason: &str,
    limit: f64,
    tol: &Tolerance,
    wrong: &mut Vec<String>,
) {
    match (did, short) {
        // At the edge of what the checker takes for a face: built, and
        // then right, or refused by the checker as before.
        (Did::Refused(KernelError::InvalidBody { .. }), None) => {}
        (Did::Refused(error), None) => wrong.push(format!("{what}: refused at the edge: {error}")),
        (Did::Built(body), Some(true) | None) => {
            let mut found = Found::default();
            let Some(m) = sound(&body, tol, &mut found) else {
                wrong.push(format!("{what}: built and not sound: {:?}", found.problems));
                return;
            };
            if !found.problems.is_empty() {
                wrong.push(format!("{what}: {:?}", found.problems));
            }
            let allowed = m.bound + 2.0 * tol.linear * m.area + 1e-9 * volume.abs();
            if (m.volume - volume).abs() > allowed {
                wrong.push(format!(
                    "{what}: volume {} for {volume} (allowed {allowed})",
                    m.volume
                ));
            }
        }
        (Did::Refused(error), Some(true)) => {
            wrong.push(format!("{what}: refused short of it: {error}"));
        }
        (Did::Built(_), Some(false)) => wrong.push(format!("{what}: built past it")),
        (Did::Refused(error), Some(false)) => match &error {
            KernelError::FaceOpTooFar {
                reason: said,
                largest,
                ..
            } => {
                if !said.contains(reason) {
                    wrong.push(format!("{what}: refused for another reason: {error}"));
                }
                if !(*largest <= limit * (1.0 + 1e-12) + 8.0 * tol.linear
                    && *largest >= limit * (1.0 - 1e-5))
                {
                    wrong.push(format!(
                        "{what}: what fits is said to be below {largest}, the limit is {limit}"
                    ));
                }
            }
            other => wrong.push(format!("{what}: refused, but not as too far: {other}")),
        },
    }
}

/// The parts of a limit probed: half way, a hundred-thousandth and a
/// millionth short, `past` (a millionth past it where that is more than a
/// few tolerances), and half as far again; and whether each is short of
/// it. `edge`: a millionth short the moved face is one tolerance from
/// what ends it, which is where the checker stops taking the sliver
/// between them for a face (on some frames and not on others, as before):
/// built right or refused by the checker there, either is held.
fn parts(past: f64, edge: bool) -> [(f64, Option<bool>); 5] {
    [
        (0.5, Some(true)),
        (1.0 - 1e-5, Some(true)),
        (1.0 - 1e-6, if edge { None } else { Some(true) }),
        (1.0 + past, Some(false)),
        (1.5, Some(false)),
    ]
}

/// A rod of radius 7 along the block's length, its axis 3 over the top,
/// standing 1 out of both end faces, united with the block: its end cap
/// moved inward. Up to 1 the cap goes down the part that stands out, and
/// takes a slab of the whole disc with it; at 1 it reaches the block's end
/// face, which ends the rod's wall, and past it the cap's rim would cross
/// the edges the wall ends in.
#[test]
fn an_end_cap_moves_up_to_the_face_its_wall_ends_on() {
    let mut wrong = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in frames(k).iter().enumerate() {
            let rod = primitives::cylinder(
                &at(frame, k, [-1.0, 15.0, 23.0], frame.x(), frame.y()),
                7.0 * k,
                42.0 * k,
            )
            .unwrap();
            let body = boolean(&block(frame, k), &rod, BooleanOp::Union, &tol)
                .unwrap()
                .body;
            let before = measured(&body).unwrap().volume;
            let cap = flat_face(&body, frame, k, [-1.0, 15.0, 23.0], frame.x());
            for (part, short) in parts(1e-6, true) {
                let what = format!("end cap, scale {k}, frame {index}, {part} of the limit");
                let d = part * k;
                let did = offset(&what, &body, cap, -d, &tol);
                held(
                    &what,
                    did,
                    short,
                    before - PI * 49.0 * k * k * d,
                    "two edges of one face would cross or touch",
                    k,
                    &tol,
                    &mut wrong,
                );
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A blind hole of radius 3.6 from the top down to 11.4 over the bottom:
/// the bottom moved inward (up). Up to 11.4 the block gets thinner under
/// the hole; at 11.4 the bottom reaches the hole's floor, and past it the
/// bottom would pass through the hole.
#[test]
fn a_bottom_moves_up_to_the_floor_of_a_blind_hole() {
    let mut wrong = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in frames(k).iter().enumerate() {
            let hole = primitives::cylinder(
                &at(frame, k, [26.7, 10.1, 11.4], frame.z(), frame.x()),
                3.6 * k,
                20.0 * k,
            )
            .unwrap();
            let body = boolean(&block(frame, k), &hole, BooleanOp::Subtract, &tol)
                .unwrap()
                .body;
            let bottom = flat_face(&body, frame, k, [20.0, 15.0, 0.0], frame.z());
            for (part, short) in parts(1e-6, false) {
                let what = format!("blind hole, scale {k}, frame {index}, {part} of the limit");
                let d = part * 11.4 * k;
                let did = offset(&what, &body, bottom, -d, &tol);
                let volume = 1200.0 * k * k * (20.0 * k - d) - PI * 3.6 * 3.6 * 8.6 * k * k * k;
                held(
                    &what,
                    did,
                    short,
                    volume,
                    "the face would pass through another part of the body",
                    11.4 * k,
                    &tol,
                    &mut wrong,
                );
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A pocket that widens downward: a cone of radius 7 at 12 over the
/// bottom, its apex 14 above that, taken out of the block about (20, 22);
/// its mouth in the top has radius 3. The pocket's floor moved down by
/// `d` has radius `7 + d / 2`: at 2 its rim reaches the side of the block
/// 8 from the cone's axis, and past it the rim would run out through that
/// side. Short of it the pocket is the frustum from `12 − d` to 20.
#[test]
fn a_pockets_floor_moves_down_until_its_rim_reaches_the_side() {
    let mut wrong = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in frames(k).iter().enumerate() {
            let cone = primitives::cone(
                &at(frame, k, [20.0, 22.0, 12.0], frame.z(), frame.x()),
                7.0 * k,
                14.0 * k,
            )
            .unwrap();
            let body = boolean(&block(frame, k), &cone, BooleanOp::Subtract, &tol)
                .unwrap()
                .body;
            let floor = flat_face(&body, frame, k, [20.0, 22.0, 12.0], frame.z());
            // A millionth past the limit the rim stands one tolerance out
            // of the side, which is the side: twenty millionths past it.
            for (part, short) in parts(2e-5, false) {
                let what = format!("pocket, scale {k}, frame {index}, {part} of the limit");
                let d = part * 2.0;
                // The floor's outward normal looks up into the pocket: down
                // is against it.
                let did = offset(&what, &body, floor, -d * k, &tol);
                let (low, high, tall) = (7.0 + 0.5 * d, 3.0, 8.0 + d);
                let pocket = PI * tall / 3.0 * (low * low + low * high + high * high);
                held(
                    &what,
                    did,
                    short,
                    (24000.0 - pocket) * k * k * k,
                    "the face would pass through another part of the body",
                    2.0 * k,
                    &tol,
                    &mut wrong,
                );
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A groove round the top: a ring of radius 6.2 and tube radius 2.9 about
/// (18.4, 15) at 19.4 over the bottom, taken out. The bottom moved up 16
/// leaves a plate 4 thick under the groove, whose lowest line is 16.5
/// over the bottom; moved up 19.8 it would pass through the groove and
/// turn the body inside out (the plate left has less volume than the
/// groove takes). No vertex and no edge lies in the bottom's way there, so
/// where it first touches the groove is not found: the refusal says so,
/// and gives no largest distance.
#[test]
fn a_bottom_moved_through_a_groove_of_the_top_is_refused_as_inside_out() {
    let mut wrong = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in frames(k).iter().enumerate() {
            let ring = primitives::torus(
                &at(frame, k, [18.4, 15.0, 19.4], frame.z(), frame.x()),
                6.2 * k,
                2.9 * k,
            )
            .unwrap();
            let body = boolean(&block(frame, k), &ring, BooleanOp::Subtract, &tol)
                .unwrap()
                .body;
            let before = measured(&body).unwrap().volume;
            let bottom = flat_face(&body, frame, k, [20.0, 15.0, 0.0], frame.z());
            let what = format!("groove, scale {k}, frame {index}, up 16");
            let did = offset(&what, &body, bottom, -16.0 * k, &tol);
            held(
                &what,
                did,
                Some(true),
                before - 1200.0 * 16.0 * k * k * k,
                "",
                0.0,
                &tol,
                &mut wrong,
            );
            let what = format!("groove, scale {k}, frame {index}, up 19.8");
            match offset(&what, &body, bottom, -19.8 * k, &tol) {
                Did::Built(_) => wrong.push(format!("{what}: built")),
                Did::Refused(KernelError::FaceOp { reason, .. })
                    if reason.contains("turned inside out") => {}
                Did::Refused(error) => wrong.push(format!("{what}: {error}")),
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
