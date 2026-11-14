//! **Known wrong, owed** (the second review of 2026-11-14): a face moved a
//! little way out through a freeform face of its own body, or a freeform
//! face moved into a part of the body far from it, is built: a body that
//! passes the checker and passes through itself.
//!
//! The checks that look for a face moved through another part of its body
//! (`craft3d_kernel::tweak::passes`: `swept`, `pierced`) ask only planes,
//! cylinders, cones, spheres and tori; asking every spline surface of a
//! body cost up to five times the operation (`docs/design/face-operations.md`
//! › *The review's round*). What is not asked is not found, so four kinds
//! of case that were refused while every surface was asked are built again,
//! as they were before the checks:
//!
//! - the floor of a round pocket raised until its rim runs out through a
//!   spline wall over it;
//! - the floor of a hole in a vase lowered until its rim runs out through
//!   the vase's turned spline side;
//! - the floor of a pocket raised until its rim runs out through the
//!   round of an edge of a spline face (a freeform fillet);
//! - a spline face moved up into an arm that hangs over it.
//!
//! Each case is held to oracles that do not ask how the body was built:
//! the volume against the same body built by the booleans (which is the
//! right one where there is a right one), the tessellator's self-check,
//! and a point that lies outside the body, in the part the moved face ran
//! out through, placed by rays. A case known wrong is in its list; the day
//! it is refused (or built right) its test fails until it is taken off.
//! Next to each, a move that stays clear of the freeform face and must be
//! built right.
//!
//! With the pocket's seam turned another way some of these are refused,
//! with the limit; that is not pinned here.

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
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, FaceId, check};
use hunt_common::{crosses_itself, edge_near, measured, tolerance};

const SCALES: [f64; 3] = [1e-3, 1.0, 1e3];

/// What an operation made of a case.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Did {
    /// Built, and no oracle has anything against the body.
    Sound,
    Refused,
    /// Built, and an oracle shows the body wrong.
    Wrong,
}

/// What the oracles say of the result of an operation: its volume against
/// `direct` (the volume of the same body built another way), the
/// self-check, and the point `outside`, which lies outside the right
/// body. `Err` for a body the checker refuses: a fault of another kind
/// than the one pinned here.
fn judged(
    made: Result<Body, KernelError>,
    direct: Option<f64>,
    outside: Option<Vec3>,
    tol: &Tolerance,
) -> Result<(Did, String), String> {
    let body = match made {
        Ok(body) => body,
        Err(error) => return Ok((Did::Refused, error.to_string())),
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

/// Holds one case to what is expected of it: `clear` (the moved face
/// stays clear: a sound body), `known` (known built wrong) or neither (no
/// right body: refused).
fn hold(
    what: String,
    clear: bool,
    known: bool,
    found: Result<(Did, String), String>,
    unexpected: &mut Vec<String>,
) {
    match found {
        Err(e) => unexpected.push(format!("{what}: {e}")),
        Ok((Did::Sound, _)) if clear => println!("{what}: built right"),
        Ok((Did::Wrong, shown)) if known && !clear => println!("{what}: known wrong ({shown})"),
        Ok((Did::Refused, said)) if !known && !clear => println!("{what}: refused: {said}"),
        Ok((Did::Refused, said)) if known => unexpected.push(format!(
            "{what}: refused now ({said}): take it off the known list"
        )),
        Ok((Did::Sound, _)) => unexpected.push(format!(
            "{what}: built, and no oracle shows what is wrong with it"
        )),
        Ok((did, said)) => unexpected.push(format!("{what}: {did:?} {said}")),
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

/// The height of the wall's spline over `x` (scale 1).
fn wall_y(x: f64) -> f64 {
    cubic_at([25.0, 18.0, 12.0, 5.0], x, [20.0, 14.0, 27.0, 20.0])
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
                control: vec![[25.0, 20.0], [18.0, 14.0], [12.0, 27.0], [5.0, 20.0]],
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

/// A round pocket of radius 3 from the bottom of the walled block up to
/// y 12, under the wall's dip (the wall is lowest over the pocket at its
/// rim at x 20.5); its floor raised to `out` over that lowest point.
/// Below it (`out` negative) the body is the one a longer rod cuts; over
/// it the rim runs out through the wall and there is no body with that
/// floor.
fn rim_through_wall(k: f64, out: f64) -> Result<(Did, String), String> {
    let tol = tolerance(k);
    let solid = walled(40.0, k, &tol);
    let pocket = |height: f64| {
        rod(
            [17.5, -5.0, 20.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            3.0,
            height,
            k,
        )
    };
    let body = cut(&solid, &pocket(17.0), &tol);
    let floor = flat(&body, 1, 12.0, k);
    let to = wall_y(20.5) + out;
    let direct = boolean(&solid, &pocket(to + 5.0), BooleanOp::Subtract, &tol)
        .ok()
        .and_then(|made| volume(&made.body));
    // Inside the pocket's round, a tenth short of its rim: half way
    // between the wall and the raised floor, in what would be air over
    // the wall.
    let x = 20.4;
    let outside = (out > 0.0).then(|| Vec3::new(x, 0.5 * (wall_y(x) + to), 20.0) * k);
    judged(
        offset(&body, floor, -(to - 12.0) * k, &tol),
        direct,
        outside,
        &tol,
    )
}

/// How far over the wall's lowest point the pocket's floor is built.
const RIM_KNOWN: [f64; 3] = [0.02, 0.3, 1.0];

#[test]
fn a_pockets_rim_raised_out_through_a_spline_wall_is_refused() {
    let mut unexpected = Vec::new();
    for k in SCALES {
        for out in [-0.3, 0.02, 0.3, 1.0] {
            hold(
                format!("scale {k}, the floor {out} over the wall's lowest"),
                out < 0.0,
                RIM_KNOWN.contains(&out),
                rim_through_wall(k, out),
                &mut unexpected,
            );
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// A vase 20 high (its side a spline turned about z: radius 8 at the
/// foot, 12 at the top, narrower than 9 below some height), a hole of
/// radius 2 from its top down to z 15, 7 off the axis (so its far side is
/// 9 off the axis); the hole's floor lowered to `out` below the height at
/// which the vase is 9 wide.
fn rim_through_turned_side(k: f64, out: f64) -> Result<(Did, String), String> {
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
    let hole = |depth: f64| {
        rod(
            [0.0, 7.0, 25.0],
            [0.0, 0.0, -1.0],
            [1.0, 0.0, 0.0],
            2.0,
            depth,
            k,
        )
    };
    let body = cut(&solid, &hole(10.0), &tol);
    let floor = flat(&body, 2, 15.0, k);
    // The height at which the vase is 9 wide.
    let (mut lo, mut hi) = (0.0_f64, 10.0_f64);
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if vase_r(mid) < 9.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let to = 0.5 * (lo + hi) - out;
    let direct = boolean(&solid, &hole(25.0 - to), BooleanOp::Subtract, &tol)
        .ok()
        .and_then(|made| volume(&made.body));
    // A tenth over the lowered floor, between the vase's side and the
    // hole's far side: air beside the vase.
    let z = to + 0.1;
    let outside = (out > 0.0).then(|| Vec3::new(0.0, 0.5 * (vase_r(z) + 9.0), z) * k);
    judged(
        offset(&body, floor, -(15.0 - to) * k, &tol),
        direct,
        outside,
        &tol,
    )
}

/// How far below the height where the vase is as wide as the hole
/// reaches the hole's floor is built.
const TURNED_KNOWN: [f64; 2] = [0.5, 1.5];

#[test]
fn a_holes_rim_lowered_out_through_a_turned_spline_is_refused() {
    let mut unexpected = Vec::new();
    for k in SCALES {
        for out in [-0.5, 0.5, 1.5] {
            hold(
                format!("scale {k}, the floor {out} below where the vase is 9 wide"),
                out < 0.0,
                TURNED_KNOWN.contains(&out),
                rim_through_turned_side(k, out),
                &mut unexpected,
            );
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// The walled block 10 long, the edge between its wall and its cap at
/// z 10 rounded with radius 1 (a freeform round); a pocket of radius 1.2
/// from the cap at z 0 up to z 5, under the round (its far side 0.65
/// under the wall); its floor raised to `to`. From z 9 on the round
/// bends down over the pocket: at 9.5 the floor is still under it, at
/// 9.95 and 9.99 the rim is out through the round.
fn rim_through_round(k: f64, to: f64) -> Result<(Did, String), String> {
    let tol = tolerance(k);
    let solid = walled(10.0, k, &tol);
    let edge = edge_near(&solid, Vec3::new(15.0, wall_y(15.0), 10.0) * k);
    let solid = fillet(&solid, &[FilletEdge { edge, radius: k }], &tol)
        .map_err(|e| format!("no round: {e}"))?
        .body;
    let y = wall_y(14.0) - 1.85;
    let pocket = |height: f64| {
        rod(
            [14.0, y, -5.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            1.2,
            height,
            k,
        )
    };
    let body = boolean(&solid, &pocket(10.0), BooleanOp::Subtract, &tol)
        .map_err(|e| format!("no pocket: {e}"))?
        .body;
    let floor = flat(&body, 2, 5.0, k);
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

/// The heights at which the pocket's floor is built through the round.
const ROUND_KNOWN: [f64; 2] = [9.95, 9.99];

#[test]
fn a_pockets_rim_raised_out_through_a_freeform_round_is_refused() {
    let mut unexpected = Vec::new();
    for k in SCALES {
        for to in [9.5, 9.95, 9.99] {
            hold(
                format!("scale {k}, the floor at {to}"),
                to < 9.9,
                ROUND_KNOWN.contains(&to),
                rim_through_round(k, to),
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

/// How far up the bump is built into the arm, moved and offset alike.
const BUMP_KNOWN: [f64; 2] = [4.0, 6.0];

#[test]
fn a_spline_face_moved_up_into_an_arm_over_it_is_refused() {
    let mut unexpected = Vec::new();
    for k in SCALES {
        for rise in [2.0, 4.0, 6.0] {
            for as_offset in [false, true] {
                hold(
                    format!(
                        "scale {k}, the bump {} {rise}",
                        if as_offset { "offset by" } else { "moved up" }
                    ),
                    rise < 2.5,
                    BUMP_KNOWN.contains(&rise),
                    bump_into_arm(k, rise, as_offset),
                    &mut unexpected,
                );
            }
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}
