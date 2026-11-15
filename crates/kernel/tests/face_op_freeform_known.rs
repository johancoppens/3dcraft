//! A face moved out through a freeform face of its own body, and a
//! freeform face moved into a part of the body far from it: refused, with
//! the largest distance that fits where the face in the way is first met
//! by an edge; and the one kind still built wrong, pinned as known.
//!
//! The checks that look for a face moved through another part of its body
//! (`craft3d_kernel::tweak::passes`: `swept`, `pierced`) ask a freeform
//! face by walks from the spans an edge or a vertex comes near
//! (`docs/design/face-operations.md` › *Freeform faces asked again*).
//! Four kinds of case, each the smallest of its kind (the second review of
//! 2026-11-14, which found them built wrong while only planes, cylinders,
//! cones, spheres and tori were asked):
//!
//! - the floor of a round pocket raised until its rim runs out through a
//!   spline wall over it;
//! - the floor of a hole in a vase lowered until its rim runs out through
//!   the vase's turned spline side;
//! - the floor of a pocket raised until its rim runs out through the
//!   round of an edge of a spline face (a freeform fillet);
//! - a spline face moved up into an arm that hangs over it.
//!
//! Where the rim first reaches the freeform face is known in closed form
//! for the first three. Each is probed short of it (well clear, and a
//! ten-thousandth of the distance short: built, and the body the booleans
//! cut) and past it (a ten-thousandth, and the review's distances:
//! refused, the largest distance that fits said within a hundred-thousandth
//! below the closed form and not eight tolerances above it), each with the
//! pocket's seam turned three ways: its vertex where the rim comes out,
//! away from it, and so that the rim comes out half way between two of
//! the steps the rim is followed in.
//!
//! A result that is built is held to oracles that do not ask how it was
//! built: the volume against the same body built by the booleans, the
//! tessellator's self-check, and a point that lies outside the right body,
//! in the part the moved face ran out through, placed by rays.
//!
//! **Known wrong, owed**: the spline face moved straight up into the arm
//! with no vertex and no edge in its way ([`BUMP_KNOWN`]). The day it is
//! refused its test fails until it is taken off the list.

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
use craft3d_kernel::boolean::{BooleanOp, Place, boolean, classify_points};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, fillet};
use craft3d_kernel::move_face::{Motion, move_faces};
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Portable, Tolerance, Vec3};
use craft3d_topo::{Body, FaceId, check};
use hunt_common::{crosses_itself, edge_near, measured, tolerance};
use std::f64::consts::PI;

const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

/// What an operation made of a case.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Did {
    /// Built, and no oracle has anything against the body.
    Sound,
    /// Refused; as too far with this largest distance that fits, if so.
    Refused(Option<f64>),
    /// Built, and an oracle shows the body wrong.
    Wrong,
}

/// What is expected of a case.
#[derive(Clone, Copy, Debug)]
enum Expect {
    /// The moved face stays clear: a sound body.
    Right,
    /// No right body: refused.
    Refused,
    /// No right body: refused as too far, the largest distance that fits
    /// being this one.
    Limit(f64),
    /// Known built wrong.
    Known,
}

/// What the oracles say of the result of an operation: its volume against
/// `direct` (the volume of the same body built another way), the
/// self-check, and the point `outside`, which lies outside the right
/// body. `Err` for a body the checker refuses: a fault of another kind
/// than the ones looked for here.
fn judged(
    made: Result<Body, KernelError>,
    direct: Option<f64>,
    outside: Option<Vec3>,
    tol: &Tolerance,
) -> Result<(Did, String), String> {
    let body = match made {
        Ok(body) => body,
        Err(error) => {
            let largest = match &error {
                KernelError::FaceOpTooFar { largest, .. } => Some(*largest),
                _ => None,
            };
            return Ok((Did::Refused(largest), error.to_string()));
        }
    };
    if let Some(defect) = check(&body, tol).first() {
        return Err(format!("an invalid body returned: {defect:?}"));
    }
    let mut shown = Vec::new();
    let Some(m) = measured(&body) else {
        return Err("built and not measured".into());
    };
    if let Some(direct) = direct {
        // Two bodies within the tolerance of each other differ by no more
        // than the tolerance over their area.
        let allowed = m.bound + 2.0 * tol.linear * m.area + 1e-9 * direct.abs();
        if (m.volume - direct).abs() > allowed {
            shown.push(format!("volume {:e} for {direct:e}", m.volume));
        }
    }
    if crosses_itself(&body, tol) {
        shown.push("passes through itself".into());
    }
    if let Some(point) = outside {
        match classify_points(&body, &[point], tol).map_err(|e| e.to_string())?[..] {
            [Place::Outside] => {}
            [Place::Inside] => shown.push("a point outside it found inside".into()),
            ref other => return Err(format!("the point cannot be placed: {other:?}")),
        }
    }
    if shown.is_empty() {
        Ok((Did::Sound, String::new()))
    } else {
        Ok((Did::Wrong, shown.join(", ")))
    }
}

/// Holds one case to what is expected of it.
fn hold(
    what: String,
    expect: Expect,
    found: Result<(Did, String), String>,
    tol: &Tolerance,
    unexpected: &mut Vec<String>,
) {
    match (found, expect) {
        (Err(e), _) => unexpected.push(format!("{what}: {e}")),
        (Ok((Did::Sound, _)), Expect::Right) => println!("{what}: built right"),
        (Ok((Did::Wrong, shown)), Expect::Known) => println!("{what}: known wrong ({shown})"),
        (Ok((Did::Refused(_), said)), Expect::Refused) => println!("{what}: refused: {said}"),
        (Ok((Did::Refused(Some(largest)), said)), Expect::Limit(limit))
            if largest <= limit * (1.0 + 1e-12) + 8.0 * tol.linear
                && largest >= limit * (1.0 - 1e-5) =>
        {
            println!("{what}: refused at {largest} of {limit}: {said}");
        }
        (Ok((Did::Refused(_), said)), Expect::Limit(limit)) => unexpected.push(format!(
            "{what}: refused, but the largest distance that fits is {limit}: {said}"
        )),
        (Ok((Did::Refused(_), said)), Expect::Known) => unexpected.push(format!(
            "{what}: refused now ({said}): take it off the known list"
        )),
        (Ok((Did::Refused(_), said)), Expect::Right) => {
            unexpected.push(format!("{what}: a right body refused: {said}"));
        }
        (Ok((Did::Sound, _)), _) => unexpected.push(format!(
            "{what}: built, and no oracle shows what is wrong with it"
        )),
        (Ok((Did::Wrong, shown)), _) => unexpected.push(format!("{what}: built wrong: {shown}")),
    }
}

/// A cubic Bézier's coordinate at `t`.
fn cubic(t: f64, p: [f64; 4]) -> f64 {
    let u = 1.0 - t;
    u * u * u * p[0] + 3.0 * u * u * t * p[1] + 3.0 * u * t * t * p[2] + t * t * t * p[3]
}

/// `of` at the parameter where `along` (monotone over the curve) is `at`.
fn cubic_at(along: [f64; 4], at: f64, of: [f64; 4]) -> f64 {
    let rising = along[3] > along[0];
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if (cubic(mid, along) < at) == rising {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    cubic(0.5 * (lo + hi), of)
}

/// The wall's spline, in x and in y (scale 1).
const WALL_X: [f64; 4] = [25.0, 18.0, 12.0, 5.0];
const WALL_Y: [f64; 4] = [20.0, 14.0, 27.0, 20.0];

/// The height of the wall's spline over `x` (scale 1).
fn wall_y(x: f64) -> f64 {
    cubic_at(WALL_X, x, WALL_Y)
}

/// How far the point (`x`, `y`) is from the wall's spline (scale 1): the
/// least over 4000 places along it, made fine by thirds about the best.
fn wall_gap(x: f64, y: f64) -> f64 {
    let gap = |t: f64| {
        let (dx, dy) = (cubic(t, WALL_X) - x, cubic(t, WALL_Y) - y);
        (dx * dx + dy * dy).sqrt()
    };
    let steps = 4000;
    let best = (0..=steps)
        .map(|i| f64::from(i) / f64::from(steps))
        .min_by(|a, b| gap(*a).total_cmp(&gap(*b)))
        .unwrap();
    let h = 1.0 / f64::from(steps);
    let (mut lo, mut hi) = ((best - h).max(0.0), (best + h).min(1.0));
    for _ in 0..200 {
        let (a, b) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
        if gap(a) < gap(b) {
            hi = b;
        } else {
            lo = a;
        }
    }
    gap(0.5 * (lo + hi))
}

/// The vase's radius at the height `z` (scale 1).
fn vase_r(z: f64) -> f64 {
    cubic_at([0.0, 7.0, 13.0, 20.0], z, [8.0, 11.0, 9.0, 12.0])
}

/// The prism of a closed outline in the plane z = 0 (given at scale 1),
/// `length` long.
fn prism(outline: Vec<Curve2>, length: f64, k: f64, tol: &Tolerance) -> Body {
    let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
    let curves: Vec<SketchCurve> = outline
        .into_iter()
        .enumerate()
        .map(|(i, curve)| SketchCurve {
            id: i as u64 + 1,
            curve: match curve {
                Curve2::Line { start, end } => Curve2::Line {
                    start: s(start),
                    end: s(end),
                },
                Curve2::Spline(spline) => Curve2::Spline(Spline2 {
                    degree: spline.degree,
                    knots: spline.knots,
                    control: spline.control.into_iter().map(s).collect(),
                }),
                other => other,
            },
        })
        .collect();
    let profiles = detect_profiles(&curves, tol).unwrap();
    extrude(&profiles, &Frame::WORLD, 0.0, length * k, tol)
        .unwrap()
        .body
}

fn line(start: [f64; 2], end: [f64; 2]) -> Curve2 {
    Curve2::Line { start, end }
}

/// A block 30 x 20 whose top between x 5 and x 25 is a spline wall that
/// dips to 18.4 and rises to 21.6, `length` long.
fn walled(length: f64, k: f64, tol: &Tolerance) -> Body {
    prism(
        vec![
            line([0.0, 0.0], [30.0, 0.0]),
            line([30.0, 0.0], [30.0, 20.0]),
            line([30.0, 20.0], [25.0, 20.0]),
            Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: (0..4).map(|i| [WALL_X[i], WALL_Y[i]]).collect(),
            }),
            line([5.0, 20.0], [0.0, 20.0]),
            line([0.0, 20.0], [0.0, 0.0]),
        ],
        length,
        k,
        tol,
    )
}

/// A rod of radius `r` from `base` (scale 1) `height` along `axis`, its
/// seam towards `seam`.
fn rod(base: [f64; 3], axis: [f64; 3], seam: [f64; 3], r: f64, height: f64, k: f64) -> Body {
    let v = |a: [f64; 3]| Vec3::new(a[0], a[1], a[2]);
    let frame = Frame::new(v(base) * k, v(axis), v(seam)).unwrap();
    primitives::cylinder(&frame, r * k, height * k).unwrap()
}

fn cut(body: &Body, tool: &Body, tol: &Tolerance) -> Body {
    boolean(body, tool, BooleanOp::Subtract, tol).unwrap().body
}

/// The flat face of `body` square to the axis `axis` (0, 1, 2) at the
/// height `at` (scale 1) along it.
fn flat(body: &Body, axis: usize, at: f64, k: f64) -> FaceId {
    let along = |v: Vec3| [v.x, v.y, v.z][axis];
    let found: Vec<FaceId> = (0..body.faces.len() as u32)
        .map(FaceId)
        .filter(|f| match &body.face(*f).unwrap().surface {
            Surface::Plane(p) => {
                along(p.frame().z()).abs() > 0.99
                    && (along(p.frame().origin()) - at * k).abs() < 1e-6 * k
            }
            _ => false,
        })
        .collect();
    assert_eq!(found.len(), 1, "one flat face at {at} along axis {axis}");
    found[0]
}

fn offset(body: &Body, face: FaceId, distance: f64, tol: &Tolerance) -> Result<Body, KernelError> {
    offset_faces(body, &[FaceOffset { face, distance }], tol).map(|made| made.body)
}

fn volume(body: &Body) -> Option<f64> {
    measured(body).map(|m| m.volume)
}

/// The three ways a pocket's seam is turned about its axis: towards
/// `out` (where the rim comes out first: the rim's vertex lies there),
/// away from it, and a sixteenth of a half turn from it towards `beside`
/// (the rim is followed in steps of an eighth of a half turn, so it comes
/// out half way between two of them).
fn seams(out: [f64; 3], beside: [f64; 3]) -> [(&'static str, [f64; 3]); 3] {
    let (sine, cosine) = (PI / 16.0).psin_cos();
    let turned = [0, 1, 2].map(|i| cosine * out[i] + sine * beside[i]);
    [
        ("towards", out),
        ("away", out.map(|x| -x)),
        ("between", turned),
    ]
}

/// What is expected of a floor moved to `past` beyond where its rim first
/// reaches the freeform face, the distance moved there being `limit`.
fn expected(past: f64, limit: f64) -> Expect {
    if past < 0.0 {
        Expect::Right
    } else {
        Expect::Limit(limit)
    }
}

/// The probes of a rim: with the seam towards where it comes out, well
/// clear (`clear`, negative); with the seam turned each of the three ways,
/// a ten-thousandth of `limit` short and past, and the review's distances
/// `far`.
fn probes(limit: f64, clear: f64, far: &[f64]) -> Vec<(usize, f64)> {
    let near = 1e-4 * limit;
    let mut found = vec![(0, clear)];
    for seam in [0, 1, 2] {
        found.extend([(seam, -near), (seam, near)]);
        found.extend(far.iter().map(|past| (seam, *past)));
    }
    found
}

/// A round pocket of radius 3 from the bottom of the walled block up to
/// y 12, under the wall's dip (the wall is lowest over the pocket at its
/// rim at x 20.5: 18.38); its floor raised to `past` over that lowest
/// point. Below it (`past` negative) the body is the one a longer rod
/// cuts; over it the rim runs out through the wall and there is no body
/// with that floor. With what the floor did, how far it goes to reach
/// the wall (scale 1).
fn rim_through_wall(k: f64, seam: [f64; 3], past: f64) -> (Result<(Did, String), String>, f64) {
    let tol = tolerance(k);
    let solid = walled(40.0, k, &tol);
    let pocket = |height: f64| rod([17.5, -5.0, 20.0], [0.0, 1.0, 0.0], seam, 3.0, height, k);
    let body = cut(&solid, &pocket(17.0), &tol);
    let floor = flat(&body, 1, 12.0, k);
    let limit = wall_y(20.5) - 12.0;
    let to = wall_y(20.5) + past;
    let direct = boolean(&solid, &pocket(to + 5.0), BooleanOp::Subtract, &tol)
        .ok()
        .and_then(|made| volume(&made.body));
    // Inside the pocket's round, a tenth short of its rim: half way
    // between the wall and the raised floor, in what would be air over
    // the wall.
    let x = 20.4;
    let outside = (to > wall_y(x) + 0.01).then(|| Vec3::new(x, 0.5 * (wall_y(x) + to), 20.0) * k);
    (
        judged(
            offset(&body, floor, -(to - 12.0) * k, &tol),
            direct,
            outside,
            &tol,
        ),
        limit,
    )
}

/// Until 2026-11-15 the floor 0.02, 0.3 and 1 over the wall's lowest point
/// was built (the seam towards where the rim comes out): `RIM_KNOWN`,
/// taken off with the repair.
#[test]
fn a_pockets_rim_raised_out_through_a_spline_wall_is_refused() {
    let mut unexpected = Vec::new();
    let turns = seams([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    for k in SCALES {
        let tol = tolerance(k);
        let limit = wall_y(20.5) - 12.0;
        for (seam, past) in probes(limit, -0.3, &[0.02, 0.3, 1.0]) {
            let (name, towards) = turns[seam];
            let (found, limit) = rim_through_wall(k, towards, past);
            hold(
                format!("scale {k}, seam {name}, the floor {past} over the wall's lowest"),
                expected(past, limit * k),
                found,
                &tol,
                &mut unexpected,
            );
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// The lowest the wall's spline comes over `x` from `a` to `b` (scale 1),
/// by thirds (it has one dip there).
fn wall_lowest(a: f64, b: f64) -> (f64, f64) {
    let (mut lo, mut hi) = (a, b);
    for _ in 0..200 {
        let (p, q) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
        if wall_y(p) < wall_y(q) {
            hi = q;
        } else {
            lo = p;
        }
    }
    let x = 0.5 * (lo + hi);
    (x, wall_y(x))
}

/// A box from `lo` (scale 1) of `size`.
fn boxed(lo: [f64; 3], size: [f64; 3], k: f64) -> Body {
    let at = Frame::WORLD.at(Vec3::new(lo[0], lo[1], lo[2]) * k).unwrap();
    primitives::block(&at, size[0] * k, size[1] * k, size[2] * k).unwrap()
}

/// A slot through the walled block from its bottom up to y 12, x from 17
/// to 24 and z from 1 to 39, under the wall's dip; its floor (a rectangle
/// of four straight edges) raised to `past` over the wall's lowest point
/// over the slot (18.38, at x 20.41). The two edges along x leave through
/// the wall between their ends, which stay under it. With what the floor
/// did, how far it goes to reach the wall (scale 1).
fn slot_through_wall(k: f64, past: f64) -> (Result<(Did, String), String>, f64) {
    let tol = tolerance(k);
    let solid = walled(40.0, k, &tol);
    let slot = |height: f64| boxed([17.0, -5.0, 1.0], [7.0, height + 5.0, 38.0], k);
    let body = cut(&solid, &slot(12.0), &tol);
    let floor = flat(&body, 1, 12.0, k);
    let (x, low) = wall_lowest(17.0, 24.0);
    let to = low + past;
    let direct = boolean(&solid, &slot(to), BooleanOp::Subtract, &tol)
        .ok()
        .and_then(|made| volume(&made.body));
    // Half way between the wall and the raised floor over the wall's
    // lowest point, half way along the slot: air over the wall.
    let outside = (to > low + 0.01).then(|| Vec3::new(x, 0.5 * (low + to), 20.0) * k);
    (
        judged(
            offset(&body, floor, -(to - 12.0) * k, &tol),
            direct,
            outside,
            &tol,
        ),
        low - 12.0,
    )
}

/// Until 2026-11-15 (the review of the freeform faces asked again) the
/// floor 0.0005 and 0.002 over the wall's lowest point was built, and from
/// 0.02 refused with a largest distance that fits up to where the middle
/// of its straight edges reaches the wall (6.3838 of 6.3802): a straight
/// edge was followed in one step and not looked at between its ends.
#[test]
fn a_slots_straight_edges_raised_out_through_a_spline_wall_are_refused() {
    let mut unexpected = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        let (_, low) = wall_lowest(17.0, 24.0);
        let limit = low - 12.0;
        for past in [-0.3, -1e-4 * limit, 1e-4 * limit, 0.0005, 0.002, 0.02, 0.3] {
            let (found, limit) = slot_through_wall(k, past);
            hold(
                format!("scale {k}, the slot's floor {past} over the wall's lowest"),
                expected(past, limit * k),
                found,
                &tol,
                &mut unexpected,
            );
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// The height at which the vase is 9 wide (scale 1).
fn vase_nine() -> f64 {
    let (mut lo, mut hi) = (0.0_f64, 10.0_f64);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if vase_r(mid) < 9.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// A vase 20 high (its side a spline turned about z: radius 8 at the
/// foot, 12 at the top, narrower than 9 below 2.91), a hole of radius 2
/// from its top down to z 15, 7 off the axis along y (so its far side is
/// 9 off the axis); the hole's floor lowered to `past` below the height
/// at which the vase is 9 wide.
fn rim_through_turned_side(k: f64, seam: [f64; 3], past: f64) -> Result<(Did, String), String> {
    let tol = tolerance(k);
    let s = |p: [f64; 2]| [p[0] * k, p[1] * k];
    let side = |id: u64, start: [f64; 2], end: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line {
            start: s(start),
            end: s(end),
        },
    };
    let curves = vec![
        side(1, [0.0, 0.0], [8.0, 0.0]),
        SketchCurve {
            id: 2,
            curve: Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                control: vec![
                    s([8.0, 0.0]),
                    s([11.0, 7.0]),
                    s([9.0, 13.0]),
                    s([12.0, 20.0]),
                ],
            }),
        },
        side(3, [12.0, 20.0], [0.0, 20.0]),
        side(4, [0.0, 20.0], [0.0, 0.0]),
    ];
    let profiles = detect_profiles(&curves, &tol).map_err(|e| format!("{e:?}"))?;
    // The sketch's x is the radius and its y the height.
    let plane = Frame::new(
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
    )
    .unwrap();
    let solid = revolve(
        &profiles,
        &plane,
        [0.0, 0.0],
        [0.0, 1.0],
        0.0,
        std::f64::consts::TAU,
        &tol,
    )
    .map_err(|e| e.to_string())?
    .body;
    let hole = |depth: f64| rod([0.0, 7.0, 25.0], [0.0, 0.0, -1.0], seam, 2.0, depth, k);
    let body = cut(&solid, &hole(10.0), &tol);
    let floor = flat(&body, 2, 15.0, k);
    let to = vase_nine() - past;
    let direct = boolean(&solid, &hole(25.0 - to), BooleanOp::Subtract, &tol)
        .ok()
        .and_then(|made| volume(&made.body));
    // A tenth over the lowered floor, between the vase's side and the
    // hole's far side: air beside the vase.
    let z = to + 0.1;
    let outside = (vase_r(z) < 8.99).then(|| Vec3::new(0.0, 0.5 * (vase_r(z) + 9.0), z) * k);
    judged(
        offset(&body, floor, -(15.0 - to) * k, &tol),
        direct,
        outside,
        &tol,
    )
}

/// Until 2026-11-15 the floor 0.5 and 1.5 below where the vase is 9 wide
/// was built (the seam towards x): `TURNED_KNOWN`, taken off with the
/// repair.
#[test]
fn a_holes_rim_lowered_out_through_a_turned_spline_is_refused() {
    let mut unexpected = Vec::new();
    // The rim comes out on the far side of the hole from the axis, along
    // y; the review's cases had the seam towards x.
    let turns = seams([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
    for k in SCALES {
        let tol = tolerance(k);
        let limit = 15.0 - vase_nine();
        let mut cases: Vec<([f64; 3], &str, f64)> = probes(limit, -0.5, &[])
            .into_iter()
            .map(|(seam, past)| (turns[seam].1, turns[seam].0, past))
            .collect();
        cases.extend([0.5, 1.5].map(|past| ([1.0, 0.0, 0.0], "aside", past)));
        for (seam, name, past) in cases {
            hold(
                format!("scale {k}, seam {name}, the floor {past} below where the vase is 9 wide"),
                expected(past, limit * k),
                rim_through_turned_side(k, seam, past),
                &tol,
                &mut unexpected,
            );
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// Where the centre of the pocket under the round lies (scale 1).
fn under_round() -> [f64; 2] {
    [14.0, wall_y(14.0) - 1.85]
}

/// The height at which the rim of the pocket under the round reaches the
/// round (scale 1). The round of radius 1 between the wall and the cap at
/// z 10 lies, at the height z between 9 and 10, `1 - sqrt(1 - (z - 9)^2)`
/// inside the wall; the pocket of radius 1.2 comes as near the wall as
/// its centre does, less 1.2.
fn round_reached() -> f64 {
    let [x, y] = under_round();
    let gap = wall_gap(x, y) - 1.2;
    9.0 + (1.0 - (1.0 - gap) * (1.0 - gap)).sqrt()
}

/// The walled block 10 long, the edge between its wall and its cap at
/// z 10 rounded with radius 1 (a freeform round); a pocket of radius 1.2
/// from the cap at z 0 up to z 5, under the round (0.46 from the wall);
/// its floor raised to `past` over the height at which its rim reaches
/// the round (9.8434).
fn rim_through_round(k: f64, seam: [f64; 3], past: f64) -> Result<(Did, String), String> {
    let tol = tolerance(k);
    let solid = walled(10.0, k, &tol);
    let edge = edge_near(&solid, Vec3::new(15.0, wall_y(15.0), 10.0) * k);
    let solid = fillet(&solid, &[FilletEdge { edge, radius: k }], &tol)
        .map_err(|e| format!("no round: {e}"))?
        .body;
    let [x, y] = under_round();
    let pocket = |height: f64| rod([x, y, -5.0], [0.0, 0.0, 1.0], seam, 1.2, height, k);
    let body = boolean(&solid, &pocket(10.0), BooleanOp::Subtract, &tol)
        .map_err(|e| format!("no pocket: {e}"))?
        .body;
    let floor = flat(&body, 2, 5.0, k);
    let to = round_reached() + past;
    let direct = boolean(&solid, &pocket(to + 5.0), BooleanOp::Subtract, &tol)
        .ok()
        .and_then(|made| volume(&made.body));
    judged(
        offset(&body, floor, -(to - 5.0) * k, &tol),
        direct,
        None,
        &tol,
    )
}

/// Until 2026-11-15 the floor at 9.95 and 9.99 was built (the seam
/// towards x): `ROUND_KNOWN`, taken off with the repair.
#[test]
fn a_pockets_rim_raised_out_through_a_freeform_round_is_refused() {
    let mut unexpected = Vec::new();
    let reached = round_reached();
    let turns = seams([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    for k in SCALES {
        let tol = tolerance(k);
        let limit = reached - 5.0;
        for (seam, past) in probes(limit, 9.5 - reached, &[9.95 - reached, 9.99 - reached]) {
            let (name, towards) = turns[seam];
            hold(
                format!(
                    "scale {k}, seam {name}, the floor at {} ({past} past the round)",
                    reached + past
                ),
                expected(past, limit * k),
                rim_through_round(k, towards, past),
                &tol,
                &mut unexpected,
            );
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// A block 30 x 20 (+ `lift`), 40 long, whose whole top is a spline bump
/// that rises 4 in the middle, with a tower beside it and an arm that
/// hangs from the tower over the bump, its underside at y 26.5 (2.5 over
/// the bump's top) and its free end at x 10.
fn bump_under_arm(lift: f64, k: f64, tol: &Tolerance) -> Option<Body> {
    let top = 20.0 + lift;
    let block = prism(
        vec![
            line([0.0, 0.0], [30.0, 0.0]),
            line([30.0, 0.0], [30.0, top]),
            Curve2::Spline(Spline2 {
                degree: 3,
                knots: vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
                control: vec![
                    [30.0, top],
                    [22.0, top],
                    [15.0, top + 8.0],
                    [8.0, top],
                    [0.0, top],
                ],
            }),
            line([0.0, top], [0.0, 0.0]),
        ],
        40.0,
        k,
        tol,
    );
    let block_at = |lo: [f64; 3], size: [f64; 3]| {
        let at = Frame::WORLD.at(Vec3::new(lo[0], lo[1], lo[2]) * k).unwrap();
        primitives::block(&at, size[0] * k, size[1] * k, size[2] * k).unwrap()
    };
    let tower = block_at([30.0, 0.0, 10.0], [6.0, 40.0, 20.0]);
    let arm = block_at([10.0, 26.5, 10.0], [20.0, 3.5, 20.0]);
    let with_tower = boolean(&block, &tower, BooleanOp::Union, tol).ok()?.body;
    boolean(&with_tower, &arm, BooleanOp::Union, tol)
        .ok()
        .map(|made| made.body)
}

/// The bump moved up by `rise` (as a move of the face, or as an offset
/// of it): by 2 it stays under the arm, by 4 its top is in the arm
/// (moved straight up, with no vertex of the arm in its way), by 6 it is
/// past the arm's free corners as well. A move up to under the arm is
/// the body with the block that much higher. In the arm that body is the
/// union of the higher block and the arm, which has other faces than the
/// body moved: no result of an operation that keeps every face, so
/// refused is right, and what is built measures the part of the arm the
/// bump took twice.
fn bump_into_arm(k: f64, rise: f64, as_offset: bool) -> Result<(Did, String), String> {
    let tol = tolerance(k);
    let body = bump_under_arm(0.0, k, &tol).ok_or("no body")?;
    let faces: Vec<FaceId> = (0..body.faces.len() as u32)
        .map(FaceId)
        .filter(|f| {
            matches!(
                body.face(*f).unwrap().surface,
                Surface::Nurbs(_) | Surface::Bezier(_)
            )
        })
        .collect();
    let [bump] = faces[..] else {
        return Err(format!("{} freeform faces", faces.len()));
    };
    if as_offset {
        return judged(offset(&body, bump, rise * k, &tol), None, None, &tol);
    }
    let direct = bump_under_arm(rise, k, &tol).and_then(|higher| volume(&higher));
    let motion = Motion {
        translate: Vec3::new(0.0, rise * k, 0.0),
        pivot: Vec3::new(0.0, 0.0, 0.0),
        axis: Vec3::new(0.0, 0.0, 1.0),
        angle: 0.0,
    };
    judged(
        move_faces(&body, &[bump], &motion, &tol).map(|made| made.body),
        direct,
        None,
        &tol,
    )
}

/// **Known wrong, owed**: how far the bump is moved up into the arm and
/// built (`false`: as a move of the face). Moved straight up by 4 its top
/// is in the arm, between the arm's corners, with no vertex of the body
/// and no edge made again in the way: nothing says where to ask. Finding
/// it is finding where two faces of one body intersect
/// (`docs/design/face-operations.md` › *Freeform faces asked again*).
///
/// Until 2026-11-15 the list held the bump offset by 4 and moved or
/// offset by 6 as well (a corner of the arm in its way): taken off with
/// the repair.
const BUMP_KNOWN: [(f64, bool); 1] = [(4.0, false)];

#[test]
fn a_spline_face_moved_up_into_an_arm_over_it_is_refused() {
    let mut unexpected = Vec::new();
    for k in SCALES {
        let tol = tolerance(k);
        for rise in [2.0, 4.0, 6.0] {
            for as_offset in [false, true] {
                let expect = if rise < 2.5 {
                    Expect::Right
                } else if BUMP_KNOWN.contains(&(rise, as_offset)) {
                    Expect::Known
                } else {
                    Expect::Refused
                };
                hold(
                    format!(
                        "scale {k}, the bump {} {rise}",
                        if as_offset { "offset by" } else { "moved up" }
                    ),
                    expect,
                    bump_into_arm(k, rise, as_offset),
                    &tol,
                    &mut unexpected,
                );
            }
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}
