//! A hunt for wrong answers of draft and delete face: seeded random
//! bodies, each result held to oracles that do not ask how it was built:
//!
//! - the checker, the measure against the mesh's volume, and frame
//!   invariance (the operation on the body moved far away and turned, and
//!   beside an unrelated far cube), as in `hunt_offset`;
//! - draft, closed forms: a block with some of its sides drafted (each
//!   level a rectangle with the drafted sides moved in by the height over
//!   the neutral plane times the tangent of the angle), a prism on an L or
//!   a U with every side drafted (each level the profile offset with
//!   mitred corners, of area `A - P s + 4 s^2`), a boss or a hole drafted
//!   into a cone (a frustum); points classified by rays: above the neutral
//!   plane a positive angle only takes material, below it only adds, and
//!   only within the reach of the drafted faces;
//! - delete face, identities: a block with edges filleted or chamfered,
//!   or a cylinder, cone, ball or ring united with it or taken out of it,
//!   with those faces deleted, is the block again (volume, area, and
//!   points classified against the block); a boss with its rim filleted,
//!   the fillet deleted, is the sharp boss.

// Test code: a failure in a helper is a failed test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod hunt_common;

use boolean_common::Rng;
use craft3d_geom::Surface;
use craft3d_kernel::boolean::{BooleanOp, Operand, Place, boolean};
use craft3d_kernel::chamfer::{ChamferEdge, ChamferFace, ChamferSize, chamfer};
use craft3d_kernel::delete_face::delete_faces;
use craft3d_kernel::draft::{Draft, draft};
use craft3d_kernel::extrude::extrude;
use craft3d_kernel::fillet::{FilletEdge, FilletFace, fillet};
use craft3d_kernel::primitives;
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Portable, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, FaceId, check, edge_uses};
use hunt_common::{
    Cube, Found, Measured, SCALES, Tally, allowance, case_seed, classify, distance_to, edge_near,
    frame, frames, g, guarded, identity, letter, measured, mesh, points_in_box, points_near,
    ribbon, size_of, sound, tolerance,
};
use std::f64::consts::PI;

/// `∫ s dz` and `∫ s² dz` over `z` from 0 to `c`, `s = (z - z0) tan`.
fn moments(c: f64, z0: f64, tan: f64) -> (f64, f64) {
    (
        tan * (0.5 * c * c - z0 * c),
        tan * tan * ((c - z0).ppowi(3) + z0.ppowi(3)) / 3.0,
    )
}

/// The same over `z` from `a` to `b`.
fn moments_between(a: f64, b: f64, z0: f64, tan: f64) -> (f64, f64) {
    let (sb, qb) = moments(b, z0, tan);
    let (sa, qa) = moments(a, z0, tan);
    (sb - sa, qb - qa)
}

/// What a draft case is: the body, the faces, the neutral plane and the
/// volume in closed form.
struct DraftCase {
    body: Body,
    what: String,
    faces: Vec<FaceId>,
    frame: Frame,
    z0: f64,
    angle: f64,
    expected: Option<f64>,
}

/// The faces of `body` on a plane whose outward normal is `n` (within a
/// rounding).
fn planes_facing(body: &Body, n: Vec3) -> Vec<FaceId> {
    (0..body.faces.len() as u32)
        .map(FaceId)
        .filter(|f| {
            let face = body.face(*f).unwrap();
            match &face.surface {
                Surface::Plane(p) => {
                    let m = if face.reversed {
                        -p.frame().z()
                    } else {
                        p.frame().z()
                    };
                    m.dot(n) > 1.0 - 1e-9
                }
                _ => false,
            }
        })
        .collect()
}

/// A random draft case.
fn draft_case(rng: &mut Rng, n: usize, k: f64, tol: &Tolerance) -> Option<DraftCase> {
    let frame = frame(rng, k);
    let angle = rng.between(0.5, 15.0).to_radians() * if rng.below(2) == 0 { 1.0 } else { -1.0 };
    let tan = angle.ptan();
    match n % 3 {
        0 => {
            // A block, some of its sides drafted.
            let (a, b, c) = (
                rng.between(10.0, 50.0),
                rng.between(10.0, 50.0),
                rng.between(5.0, 30.0),
            );
            let z0 = rng.between(0.0, c);
            let body = primitives::block(&frame, a * k, b * k, c * k).ok()?;
            let sides = [frame.x(), -frame.x(), frame.y(), -frame.y()];
            let mut faces = Vec::new();
            let (mut nx, mut ny) = (0.0, 0.0);
            while faces.is_empty() {
                for (i, side) in sides.iter().enumerate() {
                    if rng.below(2) == 0 {
                        faces.extend(planes_facing(&body, *side));
                        if i < 2 {
                            nx += 1.0;
                        } else {
                            ny += 1.0;
                        }
                    }
                }
            }
            let (s1, s2) = moments(c * k, z0 * k, tan);
            let (a, b, c2) = (a * k, b * k, c * k);
            let expected = a * b * c2 - (a * ny + b * nx) * s1 + nx * ny * s2;
            Some(DraftCase {
                body,
                what: format!(
                    "block {:.3} x {:.3} x {c:.3}, {nx} x sides and {ny} y sides, neutral at {z0:.3}",
                    a / k,
                    b / k
                ),
                faces,
                frame,
                z0: z0 * k,
                angle,
                expected: Some(expected),
            })
        }
        1 => {
            // A prism on an L or a U, every side drafted.
            let (w, d) = (rng.between(20.0, 40.0), rng.between(20.0, 40.0));
            let (cw, cd) = (rng.between(0.3, 0.6) * w, rng.between(0.3, 0.6) * d);
            let u = rng.below(2) == 0;
            let points: Vec<[f64; 2]> = if u {
                // A U: a notch from the middle of the top side.
                let x0 = 0.5 * (w - cw);
                vec![
                    [0.0, 0.0],
                    [w, 0.0],
                    [w, d],
                    [x0 + cw, d],
                    [x0 + cw, d - cd],
                    [x0, d - cd],
                    [x0, d],
                    [0.0, d],
                ]
            } else {
                vec![
                    [0.0, 0.0],
                    [w, 0.0],
                    [w, d - cd],
                    [w - cw, d - cd],
                    [w - cw, d],
                    [0.0, d],
                ]
            };
            let c = rng.between(5.0, 25.0);
            let z0 = rng.between(0.0, c);
            // No side may shrink to nothing: the offset stays below a
            // third of the shortest side.
            let shortest = points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .map(|(p, q)| (q[0] - p[0]).phypot(q[1] - p[1]))
                .fold(f64::INFINITY, f64::min);
            if c.max(1.0) * tan.abs() > shortest / 3.0 {
                return None;
            }
            let curves: Vec<SketchCurve> = (0..points.len())
                .map(|i| {
                    let (p, q) = (points[i], points[(i + 1) % points.len()]);
                    SketchCurve {
                        id: i as u64 + 1,
                        curve: Curve2::Line {
                            start: [p[0] * k, p[1] * k],
                            end: [q[0] * k, q[1] * k],
                        },
                    }
                })
                .collect();
            let profiles = detect_profiles(&curves, tol).ok()?;
            let body = extrude(&profiles, &frame, 0.0, c * k, tol).ok()?.body;
            let faces: Vec<FaceId> = (0..body.faces.len() as u32)
                .map(FaceId)
                .filter(|f| match &body.face(*f).unwrap().surface {
                    Surface::Plane(p) => p.frame().z().dot(frame.z()).abs() < 1e-9,
                    _ => false,
                })
                .collect();
            let area: f64 = 0.5
                * (0..points.len())
                    .map(|i| {
                        let (p, q) = (points[i], points[(i + 1) % points.len()]);
                        p[0] * q[1] - q[0] * p[1]
                    })
                    .sum::<f64>();
            let perimeter: f64 = (0..points.len())
                .map(|i| {
                    let (p, q) = (points[i], points[(i + 1) % points.len()]);
                    (q[0] - p[0]).phypot(q[1] - p[1])
                })
                .sum();
            let (s1, s2) = moments(c * k, z0 * k, tan);
            let expected = area * k * k * c * k - perimeter * k * s1 + 4.0 * s2;
            Some(DraftCase {
                body,
                what: format!(
                    "prism on {} {w:.3} x {d:.3} cut {cw:.3} x {cd:.3}, height {c:.3}, neutral at {z0:.3}",
                    if u { "a U" } else { "an L" }
                ),
                faces,
                frame,
                z0: z0 * k,
                angle,
                expected: Some(expected),
            })
        }
        _ => {
            // A boss or a hole standing in the block's top, drafted, and
            // perhaps the block's sides too.
            let union = rng.below(2) == 0;
            let r = rng.between(2.0, 7.0);
            let (x, y, zb) = (
                rng.between(12.0, 28.0),
                rng.between(10.0, 20.0),
                rng.between(5.0, 15.0),
            );
            let block = primitives::block(&frame, 40.0 * k, 30.0 * k, 20.0 * k).ok()?;
            let axis = Frame::new(frame.point(x * k, y * k, zb * k), frame.z(), frame.x())?;
            let cylinder = primitives::cylinder(&axis, r * k, 20.0 * k).ok()?;
            let op = if union {
                BooleanOp::Union
            } else {
                BooleanOp::Subtract
            };
            let body = boolean(&block, &cylinder, op, tol).ok()?.body;
            if !check(&body, tol).is_empty() {
                return None;
            }
            let z0 = rng.between(0.0, if union { zb + 20.0 } else { 20.0 });
            let mut faces: Vec<FaceId> = (0..body.faces.len() as u32)
                .map(FaceId)
                .filter(|f| matches!(body.face(*f).unwrap().surface, Surface::Cylinder(_)))
                .collect();
            let (rk, z0k) = (r * k, z0 * k);
            let mut delta = if union {
                let (s1, s2) = moments_between(20.0 * k, (zb + 20.0) * k, z0k, tan);
                PI * (-2.0 * rk * s1 + s2)
            } else {
                let (s1, s2) = moments_between(zb * k, 20.0 * k, z0k, tan);
                -PI * (2.0 * rk * s1 + s2)
            };
            let sides = rng.below(2) == 0;
            if sides {
                // Both x sides of the block: the block's part below its top.
                for side in [frame.x(), -frame.x()] {
                    faces.extend(planes_facing(&body, side));
                }
                let (s1, _) = moments(20.0 * k, z0k, tan);
                delta -= 2.0 * 30.0 * k * s1;
            }
            let v0 = measured(&body)?.volume;
            Some(DraftCase {
                body,
                what: format!(
                    "block with a {} r {r:.3} at {x:.3} {y:.3} from z {zb:.3}{}, neutral at {z0:.3}",
                    if union { "boss" } else { "hole" },
                    if sides { " and its x sides" } else { "" }
                ),
                faces,
                frame,
                z0: z0k,
                angle,
                expected: Some(v0 + delta),
            })
        }
    }
}

const SEED_DRAFT: u64 = 0x0D5A_F700_2026_1114;

/// Seeded random drafts.
fn drafts(cases: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in cases {
        let clock = std::time::Instant::now();
        let mut rng = Rng(case_seed(SEED_DRAFT, n));
        let k = SCALES[rng.below(SCALES.len() as u64) as usize];
        let tol = tolerance(k);
        let Some(case) = draft_case(&mut rng, n, k, &tol) else {
            tally.outcomes.push('S');
            continue;
        };
        let body = &case.body;
        let Some(m0) = measured(body) else {
            tally.outcomes.push('S');
            continue;
        };
        let point = case.frame.point(0.0, 0.0, case.z0);
        let pull = case.frame.z();
        let what = format!(
            "case {n} ({}, scale {k}): faces {:?} drafted {:.4} degrees",
            case.what,
            case.faces.iter().map(|f| f.0).collect::<Vec<_>>(),
            case.angle.to_degrees()
        );
        let faces = case.faces.clone();
        let angle = case.angle;
        let run = |b: &Body, t: &Transform| {
            draft(
                b,
                &Draft {
                    faces: faces.clone(),
                    point: t.point(point),
                    pull: t.vector(pull),
                    angle,
                },
                &tol,
            )
            .map(|r| r.body)
        };
        let after = match guarded(|| run(body, &identity())) {
            Err(p) => {
                tally.outcomes.push('P');
                tally.wrong.push(format!("{what}: a panic: {p}"));
                continue;
            }
            Ok(Err(e)) => {
                tally.outcomes.push('R');
                println!(
                    "{what}: refused ({:.2} s): {e}",
                    clock.elapsed().as_secs_f64()
                );
                continue;
            }
            Ok(Ok(after)) => after,
        };
        let mut found = Found::default();
        if let Some(m1) = sound(&after, &tol, &mut found) {
            if let Some(expected) = case.expected {
                let allowed = allowance(&m0, &m1, ribbon(&after), &tol) + 1e-9 * m0.volume;
                if (m1.volume - expected).abs() > allowed {
                    found.problems.push(format!(
                        "the closed form: volume {} for {} (allowed {})",
                        g(m1.volume),
                        g(expected),
                        g(allowed)
                    ));
                }
            }
            frames(
                &mut rng,
                body,
                &after,
                &m1,
                &run,
                Cube::Kept(k),
                &tol,
                &mut found,
            );
            draft_points(&mut rng, body, &after, &case, &tol, &mut found);
        }
        let c = letter(&found, &after, &tol);
        tally.outcomes.push(c);
        println!("{what}: {c} ({:.2} s)", clock.elapsed().as_secs_f64());
        for line in &found.notes {
            println!("  {line}");
        }
        for line in &found.problems {
            tally.wrong.push(format!("{what}: {c}: {line}"));
        }
    }
    tally
}

/// Points of a draft: above the neutral plane a positive angle only takes
/// material and a negative one only adds it, below the other way round;
/// nothing changes farther from the drafted faces than the height over
/// the plane times the tangent reaches.
fn draft_points(
    rng: &mut Rng,
    before: &Body,
    after: &Body,
    case: &DraftCase,
    tol: &Tolerance,
    found: &mut Found,
) {
    let size = size_of(before);
    let chord = 2e-3 * size;
    let Some(mesh) = mesh(before, chord) else {
        found
            .notes
            .push("the body before could not be meshed".into());
        return;
    };
    let which = |f: u32| case.faces.iter().any(|x| x.0 == f);
    let point = case.frame.point(0.0, 0.0, case.z0);
    let pull = case.frame.z();
    let tan = case.angle.ptan().abs();
    let reach = 1.2 * size * tan + 2.0 * chord;
    let mut pts = points_near(rng, &mesh, &which, reach, 160);
    pts.extend(points_in_box(rng, before, 60));
    let (Some(was), Some(is)) = (classify(before, &pts, tol), classify(after, &pts, tol)) else {
        found.notes.push("points could not be classified".into());
        return;
    };
    for ((p, a), b) in pts.iter().zip(&was).zip(&is) {
        let taken = *a == Place::Inside && *b == Place::Outside;
        let added = *a == Place::Outside && *b == Place::Inside;
        if !(taken || added) {
            continue;
        }
        let h = (*p - point).dot(pull);
        let away = distance_to(&mesh, &which, *p);
        let removes = h * case.angle > 0.0;
        if h.abs() > 2.0 * chord && ((removes && added) || (!removes && taken)) {
            found.problems.push(format!(
                "a point {} over the neutral plane and {} from the drafted faces {}",
                g(h),
                g(away),
                if added { "added" } else { "taken" }
            ));
            return;
        }
        if away > h.abs() * tan * 1.2 + 2.0 * chord + 20.0 * tol.linear {
            found.problems.push(format!(
                "a point {} from the drafted faces changed, {} over the neutral plane",
                g(away),
                g(h)
            ));
            return;
        }
    }
}

/// What a delete case is: the body, the faces to delete and the body that
/// should come back.
struct DeleteCase {
    body: Body,
    faces: Vec<FaceId>,
    reference: Body,
    what: String,
    frame: Frame,
}

/// A random delete case.
fn delete_case(rng: &mut Rng, n: usize, k: f64, tol: &Tolerance) -> Option<DeleteCase> {
    let frame = frame(rng, k);
    let p = |x: f64, y: f64, z: f64| frame.point(x * k, y * k, z * k);
    let block = primitives::block(&frame, 40.0 * k, 30.0 * k, 20.0 * k).ok()?;
    // The block's twelve edges, by their middles.
    let middles = [
        (20.0, 0.0, 0.0),
        (20.0, 30.0, 0.0),
        (20.0, 0.0, 20.0),
        (20.0, 30.0, 20.0),
        (0.0, 15.0, 0.0),
        (40.0, 15.0, 0.0),
        (0.0, 15.0, 20.0),
        (40.0, 15.0, 20.0),
        (0.0, 0.0, 10.0),
        (40.0, 0.0, 10.0),
        (0.0, 30.0, 10.0),
        (40.0, 30.0, 10.0),
    ];
    // Sets of edges: one; the four round the top; three at a corner.
    let sets: [&[usize]; 5] = [&[2], &[7], &[2, 3, 6, 7], &[2, 6, 8], &[0, 9]];
    match n % 4 {
        0 | 1 => {
            let set = sets[rng.below(sets.len() as u64) as usize];
            let edges: Vec<_> = set
                .iter()
                .map(|i| {
                    let (x, y, z) = middles[*i];
                    edge_near(&block, p(x, y, z))
                })
                .collect();
            let size = rng.between(0.3, 7.0);
            let (body, faces, what) = if n.is_multiple_of(4) {
                let made = fillet(
                    &block,
                    &edges
                        .iter()
                        .map(|e| FilletEdge {
                            edge: *e,
                            radius: size * k,
                        })
                        .collect::<Vec<_>>(),
                    tol,
                )
                .ok()?;
                let faces = made
                    .faces
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| !matches!(f, FilletFace::Kept(_)))
                    .map(|(i, _)| FaceId(i as u32))
                    .collect();
                (
                    made.body,
                    faces,
                    format!("block with edges {set:?} filleted {size:.3}"),
                )
            } else {
                let uses = edge_uses(&block);
                let second = rng.between(0.4, 1.0) * size;
                let made = chamfer(
                    &block,
                    &edges
                        .iter()
                        .map(|e| ChamferEdge {
                            edge: *e,
                            first: uses[e.0 as usize][0].face,
                            size: ChamferSize::Distances {
                                first: size * k,
                                second: second * k,
                            },
                        })
                        .collect::<Vec<_>>(),
                    tol,
                )
                .ok()?;
                let faces = made
                    .faces
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| !matches!(f, ChamferFace::Kept(_)))
                    .map(|(i, _)| FaceId(i as u32))
                    .collect();
                (
                    made.body,
                    faces,
                    format!("block with edges {set:?} chamfered {size:.3} and {second:.3}"),
                )
            };
            Some(DeleteCase {
                body,
                faces,
                reference: block,
                what,
                frame,
            })
        }
        2 => {
            // A second body united with the block or taken out of it.
            let union = rng.below(2) == 0;
            let at = |o: Vec3, z: Vec3, x: Vec3| Frame::new(o, z, x).unwrap();
            let (other, what) = match rng.below(5) {
                0 => {
                    let r = rng.between(2.0, 7.0);
                    let (x, y, z) = (
                        rng.between(12.0, 28.0),
                        rng.between(10.0, 20.0),
                        rng.between(5.0, 15.0),
                    );
                    (
                        primitives::cylinder(
                            &at(p(x, y, z), frame.z(), frame.x()),
                            r * k,
                            20.0 * k,
                        )
                        .ok()?,
                        format!("a standing cylinder r {r:.3} at {x:.3} {y:.3} from z {z:.3}"),
                    )
                }
                1 => {
                    let (r, h) = (rng.between(4.0, 9.0), rng.between(8.0, 20.0));
                    let (x, y, z) = (
                        rng.between(12.0, 28.0),
                        rng.between(10.0, 20.0),
                        rng.between(8.0, 16.0),
                    );
                    (
                        primitives::cone(&at(p(x, y, z), frame.z(), frame.x()), r * k, h * k)
                            .ok()?,
                        format!("a cone r {r:.3} h {h:.3} at {x:.3} {y:.3} from z {z:.3}"),
                    )
                }
                2 => {
                    let r = rng.between(3.0, 9.0);
                    let (x, y) = (rng.between(12.0, 28.0), rng.between(10.0, 20.0));
                    let z = 20.0 + rng.between(-0.8, 0.8) * r;
                    (
                        primitives::sphere(&at(p(x, y, z), frame.z(), frame.x()), r * k).ok()?,
                        format!("a ball r {r:.3} at {x:.3} {y:.3} {z:.3}"),
                    )
                }
                3 => {
                    let (major, minor) = (rng.between(5.0, 9.0), rng.between(1.5, 3.5));
                    let (x, y) = (rng.between(15.0, 25.0), rng.between(12.0, 18.0));
                    let z = 20.0 + rng.between(-0.7, 0.7) * minor;
                    (
                        primitives::torus(
                            &at(p(x, y, z), frame.z(), frame.x()),
                            major * k,
                            minor * k,
                        )
                        .ok()?,
                        format!("a ring {major:.3} / {minor:.3} at {x:.3} {y:.3} {z:.3}"),
                    )
                }
                _ => {
                    // A block standing on the top or sunk into it.
                    let (w, d, h) = (
                        rng.between(4.0, 15.0),
                        rng.between(4.0, 12.0),
                        rng.between(4.0, 15.0),
                    );
                    let (x, y, z) = (
                        rng.between(5.0, 20.0),
                        rng.between(5.0, 15.0),
                        rng.between(8.0, 18.0),
                    );
                    (
                        primitives::block(
                            &at(p(x, y, z), frame.z(), frame.x()),
                            w * k,
                            d * k,
                            h * k,
                        )
                        .ok()?,
                        format!("a block {w:.3} x {d:.3} x {h:.3} at {x:.3} {y:.3} {z:.3}"),
                    )
                }
            };
            let op = if union {
                BooleanOp::Union
            } else {
                BooleanOp::Subtract
            };
            let made = boolean(&block, &other, op, tol).ok()?;
            if !check(&made.body, tol).is_empty() {
                return None;
            }
            let faces = made
                .faces
                .iter()
                .enumerate()
                .filter(|(_, s)| s.operand == Operand::B)
                .map(|(i, _)| FaceId(i as u32))
                .collect();
            Some(DeleteCase {
                body: made.body,
                faces,
                reference: block,
                what: format!(
                    "block with {what} {}",
                    if union { "united" } else { "taken out" }
                ),
                frame,
            })
        }
        _ => {
            // A boss with its rim filleted.
            let r = rng.between(3.0, 8.0);
            let (x, y, h) = (
                rng.between(12.0, 28.0),
                rng.between(10.0, 20.0),
                rng.between(3.0, 12.0),
            );
            let axis = Frame::new(p(x, y, 10.0), frame.z(), frame.x())?;
            let cylinder = primitives::cylinder(&axis, r * k, (10.0 + h) * k).ok()?;
            let boss = boolean(&block, &cylinder, BooleanOp::Union, tol).ok()?.body;
            let rim = edge_near(&boss, p(x + r, y, 20.0 + h));
            let radius = rng.between(0.1, 0.45) * r.min(h);
            let made = fillet(
                &boss,
                &[FilletEdge {
                    edge: rim,
                    radius: radius * k,
                }],
                tol,
            );
            let made = match made {
                Ok(made) => made,
                Err(e) => {
                    println!("case {n}: the rim's fillet refused: {e}");
                    return None;
                }
            };
            let faces = made
                .faces
                .iter()
                .enumerate()
                .filter(|(_, f)| !matches!(f, FilletFace::Kept(_)))
                .map(|(i, _)| FaceId(i as u32))
                .collect();
            Some(DeleteCase {
                body: made.body,
                faces,
                reference: boss,
                what: format!(
                    "block with a boss r {r:.3} h {h:.3} at {x:.3} {y:.3}, its rim filleted {radius:.3}"
                ),
                frame,
            })
        }
    }
}

const SEED_DELETE: u64 = 0x0DE1_E7E0_2026_1114;

/// Seeded random deletions whose result is known.
fn deletions(cases: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in cases {
        let clock = std::time::Instant::now();
        let mut rng = Rng(case_seed(SEED_DELETE, n));
        let k = SCALES[rng.below(SCALES.len() as u64) as usize];
        let tol = tolerance(k);
        let Some(case) = delete_case(&mut rng, n, k, &tol) else {
            tally.outcomes.push('S');
            continue;
        };
        if case.faces.is_empty() {
            tally.outcomes.push('N');
            continue;
        }
        let body = &case.body;
        let (Some(_m0), Some(mr)) = (measured(body), measured(&case.reference)) else {
            tally.outcomes.push('S');
            continue;
        };
        let what = format!(
            "case {n} ({}, scale {k}): faces {:?} deleted",
            case.what,
            case.faces.iter().map(|f| f.0).collect::<Vec<_>>()
        );
        let faces = case.faces.clone();
        let run = |b: &Body, _: &Transform| delete_faces(b, &faces, &tol).map(|r| r.body);
        let after = match guarded(|| run(body, &identity())) {
            Err(p) => {
                tally.outcomes.push('P');
                tally.wrong.push(format!("{what}: a panic: {p}"));
                continue;
            }
            Ok(Err(e)) => {
                tally.outcomes.push('R');
                println!(
                    "{what}: refused ({:.2} s): {e}",
                    clock.elapsed().as_secs_f64()
                );
                continue;
            }
            Ok(Ok(after)) => after,
        };
        let mut found = Found::default();
        if let Some(m1) = sound(&after, &tol, &mut found) {
            let allowed = allowance(&mr, &m1, ribbon(&after), &tol);
            if (m1.volume - mr.volume).abs() > allowed {
                found.problems.push(format!(
                    "the volume is {}, the body without the feature {} (allowed {})",
                    g(m1.volume),
                    g(mr.volume),
                    g(allowed)
                ));
            }
            if (m1.area - mr.area).abs() > 1e-7 * mr.area + 1e3 * tol.linear * tol.linear {
                found.problems.push(format!(
                    "the area is {}, the body without the feature {}",
                    g(m1.area),
                    g(mr.area)
                ));
            }
            frames(
                &mut rng,
                body,
                &after,
                &m1,
                &run,
                Cube::Kept(k),
                &tol,
                &mut found,
            );
            same_points(&mut rng, body, &after, &case, &tol, &mut found);
        }
        let c = letter(&found, &after, &tol);
        tally.outcomes.push(c);
        println!("{what}: {c} ({:.2} s)", clock.elapsed().as_secs_f64());
        for line in &found.notes {
            println!("  {line}");
        }
        for line in &found.problems {
            tally.wrong.push(format!("{what}: {c}: {line}"));
        }
        let _ = case.frame;
    }
    tally
}

/// Points near the deleted faces and in the box classified against the
/// result and the body that should come back: the same, except within the
/// chord of the reference's faces.
fn same_points(
    rng: &mut Rng,
    before: &Body,
    after: &Body,
    case: &DeleteCase,
    tol: &Tolerance,
    found: &mut Found,
) {
    let size = size_of(&case.reference);
    let chord = 2e-3 * size;
    let (Some(mesh), Some(reference)) = (mesh(before, chord), mesh(&case.reference, chord)) else {
        found.notes.push("a body could not be meshed".into());
        return;
    };
    let which = |f: u32| case.faces.iter().any(|x| x.0 == f);
    let mut pts = points_near(rng, &mesh, &which, 0.1 * size, 160);
    pts.extend(points_in_box(rng, &case.reference, 80));
    let (Some(was), Some(is)) = (
        classify(&case.reference, &pts, tol),
        classify(after, &pts, tol),
    ) else {
        found.notes.push("points could not be classified".into());
        return;
    };
    for ((p, a), b) in pts.iter().zip(&was).zip(&is) {
        let differ = matches!(
            (a, b),
            (Place::Inside, Place::Outside) | (Place::Outside, Place::Inside)
        );
        if differ {
            let away = distance_to(&reference, &|_| true, *p);
            if away > 2.0 * chord + 20.0 * tol.linear {
                found.problems.push(format!(
                    "a point {} from the body without the feature is {} it, {} the result",
                    g(away),
                    if *a == Place::Inside { "in" } else { "out of" },
                    if *b == Place::Inside { "in" } else { "out of" }
                ));
                return;
            }
        }
    }
}

/// Draft cases seen wrong, owed (none known yet).
const KNOWN_DRAFT: [usize; 0] = [];

/// Delete face cases seen wrong, owed (none known yet).
const KNOWN_DELETE: [usize; 0] = [];

#[test]
fn hunt_draft_scratch() {
    let Some(r) = hunt_common::scratch_range() else {
        return;
    };
    let start = r.start;
    drafts(r).hold("draft", start, &KNOWN_DRAFT, 0);
}

#[test]
fn hunt_delete_scratch() {
    let Some(r) = hunt_common::scratch_range() else {
        return;
    };
    let start = r.start;
    deletions(r).hold("delete", start, &KNOWN_DELETE, 0);
}

#[allow(dead_code)]
fn unused(_: Measured) {}

/// The wide hunt of draft: 1500 cases in 2 parts, run by hand
/// (`-- --ignored`), not in the gate.
fn wide_draft(part: usize) {
    let r = part * 750..(part + 1) * 750;
    let start = r.start;
    drafts(r).hold("draft", start, &KNOWN_DRAFT, 0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_draft_0() {
    wide_draft(0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_draft_1() {
    wide_draft(1);
}

#[test]
fn random_draft_0_to_200_hold_their_oracles() {
    drafts(0..200).hold_pinned(
        "draft",
        0,
        &KNOWN_DRAFT,
        PINNED_DRAFT.get(0..200).unwrap_or(""),
        100,
    );
}

/// What the draft hunt's gate cases did when it was written (outcome
/// letters, `hunt_common::Tally`); empty: not pinned yet.
const PINNED_DRAFT: &str = "";

/// The wide hunt of delete: 1500 cases in 2 parts, run by hand
/// (`-- --ignored`), not in the gate.
fn wide_delete(part: usize) {
    let r = part * 750..(part + 1) * 750;
    let start = r.start;
    deletions(r).hold("delete", start, &KNOWN_DELETE, 0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_delete_0() {
    wide_delete(0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_delete_1() {
    wide_delete(1);
}

#[test]
fn random_delete_0_to_200_hold_their_oracles() {
    deletions(0..200).hold_pinned(
        "delete",
        0,
        &KNOWN_DELETE,
        PINNED_DELETE.get(0..200).unwrap_or(""),
        100,
    );
}

/// What the delete hunt's gate cases did when it was written (outcome
/// letters, `hunt_common::Tally`); empty: not pinned yet.
const PINNED_DELETE: &str = "";
