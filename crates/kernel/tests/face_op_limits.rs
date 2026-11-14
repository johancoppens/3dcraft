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

/// A wedge: the triangle (0, 0), (40, 0), (40, 40 tan a) drawn on the
/// frame's xy plane, 30 thick, its edge along z at the origin `a` degrees
/// sharp. Its bottom (y = 0) moved outward by 0.5 takes the sharp edge
/// with it, to x = -0.5 / tan a: the section is then the triangle
/// (-0.5 / tan a, -0.5), (40, -0.5), (40, 40 tan a). And the bottom moved
/// 3 within its own plane changes nothing.
///
/// Until 2026-11-14 both were refused below about 13.6 degrees ("the face
/// would pass through another part of the body"): the point the operation
/// asks about just inside the bottom, 8 tolerances from the edge and 4
/// under the face, lies above the slanted face there, in the open. It is
/// asked about further from the edge now, where the wedge is thick enough
/// to hold it (`tweak::sharp_places`).
#[test]
fn faces_at_a_sharp_edge_are_moved() {
    use craft3d_kernel::extrude::extrude;
    use craft3d_kernel::move_face::{Motion, move_faces};
    use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
    use craft3d_math::Portable;
    let mut wrong = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in frames(k).iter().enumerate() {
            for degrees in [2.0_f64, 5.0, 8.5, 12.0, 13.0, 20.0, 45.0] {
                let slope = degrees.to_radians().ptan();
                let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
                    id,
                    curve: Curve2::Line {
                        start: [a[0] * k, a[1] * k],
                        end: [b[0] * k, b[1] * k],
                    },
                };
                let outline = [
                    line(1, [0.0, 0.0], [40.0, 0.0]),
                    line(2, [40.0, 0.0], [40.0, 40.0 * slope]),
                    line(3, [40.0, 40.0 * slope], [0.0, 0.0]),
                ];
                let profiles = detect_profiles(&outline, &tol).unwrap();
                let body = extrude(&profiles, frame, 0.0, 30.0 * k, &tol).unwrap().body;
                let before = measured(&body).unwrap().volume;
                let bottom = flat_face(&body, frame, k, [20.0, 0.0, 15.0], frame.y());
                let what =
                    format!("wedge of {degrees} degrees, scale {k}, frame {index}, bottom out");
                let did = offset(&what, &body, bottom, 0.5 * k, &tol);
                let section = 0.5 * (40.0 + 0.5 / slope) * (40.0 * slope + 0.5);
                held(
                    &what,
                    did,
                    Some(true),
                    30.0 * section * k * k * k,
                    "",
                    0.0,
                    &tol,
                    &mut wrong,
                );
                let what =
                    format!("wedge of {degrees} degrees, scale {k}, frame {index}, bottom along");
                let motion = Motion {
                    translate: frame.x() * (3.0 * k),
                    pivot: frame.origin(),
                    axis: frame.z(),
                    angle: 0.0,
                };
                let did = match move_faces(&body, &[bottom], &motion, &tol) {
                    Ok(made) => {
                        println!("probe {what}: built {:016x}", digest(&made.body));
                        Did::Built(made.body)
                    }
                    Err(error) => {
                        println!("probe {what}: refused: {error}");
                        Did::Refused(error)
                    }
                };
                held(&what, did, Some(true), before, "", 0.0, &tol, &mut wrong);
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The frames of the review of 2026-11-14: the world, a tilted frame, and
/// a tilted frame a thousand sizes away.
fn review_frames(k: f64) -> [Frame; 3] {
    [
        Frame::WORLD,
        Frame::new(
            Vec3::new(7.0 * k, -11.0 * k, 3.0 * k),
            Vec3::new(2.0, -1.0, 3.0),
            Vec3::new(1.0, 0.3, -0.2),
        )
        .unwrap(),
        Frame::new(
            Vec3::new(31000.0 * k, -24000.0 * k, 17000.0 * k),
            Vec3::new(-1.0, 3.0, 2.0),
            Vec3::new(0.2, 1.0, -0.7),
        )
        .unwrap(),
    ]
}

/// The cubic of the vase's side, (8, 0), (11, 7), (9, 13), (12, 20) in
/// (radius, height), at `t`.
fn vase_side(t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let b = |p: [f64; 4]| {
        u * u * u * p[0] + 3.0 * u * u * t * p[1] + 3.0 * u * t * t * p[2] + t * t * t * p[3]
    };
    [b([8.0, 11.0, 9.0, 12.0]), b([0.0, 7.0, 13.0, 20.0])]
}

/// The volume of the vase from its foot up to height `z` (scale 1): the
/// integral of `pi r^2 dz` along the cubic, by Simpson's rule in 4000
/// steps of the parameter (the height rises with it).
fn vase_volume(z: f64) -> f64 {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..200 {
        let middle = 0.5 * (lo + hi);
        if vase_side(middle)[1] < z {
            lo = middle;
        } else {
            hi = middle;
        }
    }
    let end = 0.5 * (lo + hi);
    // dz/dt of the cubic's heights 0, 7, 13, 20.
    let rise = |t: f64| {
        let u = 1.0 - t;
        3.0 * (u * u * 7.0 + 2.0 * u * t * 6.0 + t * t * 7.0)
    };
    let f = |t: f64| PI * vase_side(t)[0] * vase_side(t)[0] * rise(t);
    let steps = 4000;
    let h = end / f64::from(steps);
    let mut sum = f(0.0) + f(end);
    for i in 1..steps {
        sum += f(f64::from(i) * h) * if i % 2 == 1 { 4.0 } else { 2.0 };
    }
    sum * h / 3.0
}

/// The vase on `frame` at scale `k`, and its top.
fn vase(frame: &Frame, k: f64, tol: &Tolerance) -> (Body, FaceId) {
    let side = [[8.0, 0.0], [11.0, 7.0], [9.0, 13.0], [12.0, 20.0]];
    let body = turned(side, frame, k, tol);
    let top = flat_face(&body, frame, k, [1.0, 1.0, 20.0], frame.z());
    (body, top)
}

/// A cubic in (radius, height) from a foot at height 0, turned a whole
/// turn about the z axis of `frame`, at scale `k`.
fn turned(side: [[f64; 2]; 4], frame: &Frame, k: f64, tol: &Tolerance) -> Body {
    use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
    use craft3d_kernel::revolve::revolve;
    let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
    let line = |id: u64, a: [f64; 2], b: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line {
            start: s(a),
            end: s(b),
        },
    };
    let height = side[3][1];
    let curves = vec![
        line(1, [0.0, 0.0], side[0]),
        SketchCurve {
            id: 2,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: side.iter().map(|p| s(*p)).collect(),
            }),
        },
        line(3, side[3], [0.0, height]),
        line(4, [0.0, height], [0.0, 0.0]),
    ];
    let profiles = detect_profiles(&curves, tol).unwrap();
    // The sketch plane: x the radius, y the height.
    let plane = Frame::new(frame.origin(), frame.y() * -1.0, frame.x()).unwrap();
    revolve(
        &profiles,
        &plane,
        [0.0, 0.0],
        [0.0, 1.0],
        0.0,
        2.0 * PI,
        tol,
    )
    .unwrap()
    .body
}

/// A vase (a cubic from radius 8 at its foot to 12 at its top, 20 high,
/// turned a whole turn), its top offset inward to within tolerances of
/// its foot: a slab between the two caps, whose volume is the vase's up
/// to that height. The seven offsets a review found built right before
/// 2026-11-14 and refused after the first form of `tweak::iso_turned`
/// ("the edge shrinks to nothing or turns round", with a largest distance
/// that fits below one that builds): the top's vertex on a seam, found
/// where the top's plane meets the two halves of one turned surface, can
/// come to lie far round that circle, and its nearest point on the seam's
/// border was taken for where the seam ends. They are built again, as
/// before, and held to the closed form.
///
/// And past the foot it stays refused, by the seam that does turn round.
/// What fits is said as the vase's height (not above it by more than a
/// few tolerances, within a hundred-thousandth below it) where the search
/// for it runs down to the seam's end; where smaller offsets on the way
/// are refused for the seam's end sliding round its circle (an older
/// fault, on some frames), the seam is far from gone at the largest
/// offset found to fit, and no limit is said (`offset_face::goes_at`).
#[test]
fn a_vases_top_moves_to_within_tolerances_of_its_foot() {
    let mut wrong = Vec::new();
    for (k, index, d) in [
        (1.0, 1_usize, 19.99),
        (1.0, 1, 19.9995),
        (1.0, 0, 19.99998),
        (1.0, 0, 19.999985),
        (1.0, 0, 19.999993),
        (1e6, 1, 19.999995),
        (1e6, 2, 19.999995),
    ] {
        let tol = tolerance(k);
        let (body, top) = vase(&review_frames(k)[index], k, &tol);
        let what = format!("vase, scale {k}, frame {index}, top in by {d}");
        let did = offset(&what, &body, top, -d * k, &tol);
        held(
            &what,
            did,
            Some(true),
            vase_volume(20.0 - d) * k * k * k,
            "",
            0.0,
            &tol,
            &mut wrong,
        );
    }
    let mut said = 0;
    for k in [1e-3, 1.0, 1e6] {
        let tol = tolerance(k);
        for (index, frame) in review_frames(k).iter().enumerate() {
            let (body, top) = vase(frame, k, &tol);
            for past in [1e-4, 1e-2, 0.5] {
                let what = format!("vase, scale {k}, frame {index}, top past the foot by {past}");
                match offset(&what, &body, top, -(20.0 + past) * k, &tol) {
                    // The limit, where the search for it ran down to the
                    // seam's end.
                    did @ Did::Refused(KernelError::FaceOpTooFar { .. }) => {
                        said += 1;
                        held(
                            &what,
                            did,
                            Some(false),
                            0.0,
                            "the edge shrinks to nothing or turns round",
                            20.0 * k,
                            &tol,
                            &mut wrong,
                        );
                    }
                    // No limit, where smaller offsets were refused for
                    // the seam's end sliding (the older fault): what the
                    // search found is not where the seam goes.
                    Did::Refused(KernelError::FaceOp { reason, .. })
                        if reason.contains("the edge shrinks to nothing or turns round")
                            && reason.contains("was not found") => {}
                    Did::Refused(error) => wrong.push(format!("{what}: {error}")),
                    Did::Built(_) => wrong.push(format!("{what}: built past the foot")),
                }
            }
        }
    }
    println!("the vase's top past its foot: a limit said in {said} of 27");
    assert!(
        said >= 9,
        "a limit is said where the search is not disturbed"
    );
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A cap of a turned spline moved past the cap across from it: what is
/// said to fit is the body's height, or it is said to be short of it.
/// The largest distance that fits is found by trying smaller ones, and
/// on these bodies one distance on the way can fail by itself, with
/// distances that fit on either side of it (the cap's vertex on a seam
/// slides round the circle the cap's plane meets the turned side in, an
/// older fault, and two edges cross there); the search closes in on that
/// distance. Until 2026-11-15 it was said as the largest that fits
/// ("below 8.75" of a bowl 10 high, where 9.99 builds: the second review
/// of 2026-11-14, 16 of 651 such limits more than a thousandth short).
/// The search now tries once more, half way from there to the whole, and
/// where that fits, or where its work bound stopped it with room left,
/// what it found is said as a distance that fits and that more may.
///
/// A bowl 10 high, its foot moved up, and a bottle 30 high, its top moved
/// down, each a hundredth past the other cap, at 1e-3, 1 and 1e6 on the
/// world frame and a tilted one.
#[test]
fn a_limit_the_search_stopped_short_of_is_said_as_one_more_may_pass() {
    let mut wrong = Vec::new();
    let (mut short, mut said) = (0, 0);
    for (name, side, of_top) in [
        (
            "bowl",
            [[3.0, 0.0], [9.0, 1.0], [14.0, 4.0], [15.0, 10.0]],
            false,
        ),
        (
            "bottle",
            [[10.0, 0.0], [12.0, 14.0], [2.0, 16.0], [3.0, 30.0]],
            true,
        ),
    ] {
        let height = side[3][1];
        for k in [1e-3, 1.0, 1e6] {
            let tol = tolerance(k);
            for (index, frame) in review_frames(k).iter().enumerate().take(2) {
                let body = turned(side, frame, k, &tol);
                let at = if of_top { height } else { 0.0 };
                let cap = flat_face(&body, frame, k, [0.5, 0.3, at], frame.z());
                let past = 1e-2;
                let what = format!("{name}, scale {k}, frame {index}, past by {past}");
                match offset(&what, &body, cap, -(height + past) * k, &tol) {
                    Did::Refused(KernelError::FaceOpTooFar {
                        reason, largest, ..
                    }) => {
                        said += 1;
                        let limit = height * k;
                        if largest > limit * (1.0 + 1e-12) + 8.0 * tol.linear {
                            wrong.push(format!("{what}: {largest} said to fit of {limit}"));
                        }
                        let at_least = reason.contains("more may");
                        if largest < limit * (1.0 - 1e-5) {
                            short += 1;
                            if !at_least {
                                wrong.push(format!(
                                    "{what}: {largest} said as the largest that fits of {limit}"
                                ));
                            }
                        }
                    }
                    Did::Refused(KernelError::FaceOp { reason, .. })
                        if reason.contains("the edge shrinks to nothing or turns round")
                            && reason.contains("was not found") => {}
                    Did::Refused(error) => wrong.push(format!("{what}: {error}")),
                    Did::Built(_) => wrong.push(format!("{what}: built past the cap")),
                }
            }
        }
    }
    println!("turned caps past the cap across: a limit said in {said} of 12, {short} short");
    assert!(short > 0, "no search stopped short: the test shows nothing");
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A drilled hole: a cylinder of radius 4 from 13 over the bottom to the
/// top, and under it a cone to a point at 9. The bottom moved up 8.9 is
/// built (the block is 11.1 thick, less the hole). Moved up 14 it would
/// pass through the hole; the first vertex in its way is on the cone's rim
/// at 13, but the cone's point, which has no vertex, is in its way from 9
/// on, and between 9 and 13 the operation's checks see nothing. So it is
/// refused without a largest distance that fits: until 2026-11-14 it said
/// "below 13", which is past where the body is first wrong.
#[test]
fn a_bottom_moved_past_a_drill_point_is_refused_without_a_limit() {
    let mut wrong = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for (index, frame) in frames(k).iter().enumerate() {
            let hole = primitives::cylinder(
                &at(frame, k, [20.0, 15.0, 13.0], frame.z(), frame.x()),
                4.0 * k,
                20.0 * k,
            )
            .unwrap();
            let point = primitives::cone(
                &at(frame, k, [20.0, 15.0, 13.0], frame.z() * -1.0, frame.x()),
                4.0 * k,
                4.0 * k,
            )
            .unwrap();
            let drilled = boolean(&block(frame, k), &hole, BooleanOp::Subtract, &tol)
                .unwrap()
                .body;
            let body = boolean(&drilled, &point, BooleanOp::Subtract, &tol)
                .unwrap()
                .body;
            let bottom = flat_face(&body, frame, k, [20.0, 15.0, 0.0], frame.z());
            let what = format!("drill point, scale {k}, frame {index}, up 8.9");
            let did = offset(&what, &body, bottom, -8.9 * k, &tol);
            let volume = 1200.0 * 11.1 - PI * 16.0 * 7.0 - PI * 16.0 * 4.0 / 3.0;
            held(
                &what,
                did,
                Some(true),
                volume * k * k * k,
                "",
                0.0,
                &tol,
                &mut wrong,
            );
            let what = format!("drill point, scale {k}, frame {index}, up 14");
            match offset(&what, &body, bottom, -14.0 * k, &tol) {
                Did::Built(_) => wrong.push(format!("{what}: built")),
                Did::Refused(KernelError::FaceOp { reason, .. })
                    if reason.contains("pass through another part")
                        && reason.contains("was not found") => {}
                Did::Refused(error) => wrong.push(format!("{what}: {error}")),
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
