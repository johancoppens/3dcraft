//! A hunt for wrong answers of offset face and move face: seeded random
//! bodies of many kinds (`hunt_common`), a random face of each moved, and
//! every result held to oracles that do not ask how it was built:
//!
//! - the checker (`topo::check`): a body that fails it is an invalid body
//!   returned;
//! - monotony: an outward offset adds material and an inward one takes it
//!   away, by the volumes and by points classified by rays before and
//!   after (`classify_points`, which does not use the mass integrals);
//!   no point farther from the face than the operation reaches changes;
//! - a closed form where there is one: a flat face whose neighbours all
//!   stand square to it moves by its area times the distance; a flat face
//!   moved within its own plane, a round face moved square to its axis
//!   between flat faces square to it, and a flat face square to its
//!   neighbours turned about a line through its centroid in its plane
//!   leave the volume as it was; a flat face moved along its normal is
//!   the offset of it;
//! - frame invariance: the same operation on the body moved some 125
//!   times its size away and turned, and on the body with an unrelated
//!   cube added as far away (which moves the middle the measure is taken
//!   about), gives the same volume and area within the integrator's own
//!   bound, the known ribbon of edges off their faces, and what two bodies
//!   within the tolerance of each other differ by;
//! - a second measure: the volume of the closed mesh against the
//!   integrator's, within the chordal tolerance over the area;
//! - the identity: offset by +d and then by -d gives the volume back.
//!
//! A case whose result fails an oracle and also passes through itself (the
//! tessellator's self-check) is counted apart (`X`): the regeneration
//! looks for that on the mesh and refuses it, so no user meets it.
//!
//! Outcomes are letters (`hunt_common::Tally`); a case seen wrong must be
//! in the sweep's `KNOWN` list with what is wrong, so that a later fix has
//! to take it off.

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
use craft3d_kernel::move_face::{Motion, move_faces};
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_math::{Portable, Transform, Vec3};
use craft3d_topo::{Body, FaceId, check, face_areas};
use hunt_common::{
    Cube, Found, KINDS, Kind, SCALES, Tally, allowance, case_seed, critical, face_area_centroid,
    frames, g, guarded, identity, least_sine, letter, make, measured, mesh, mesh_volume,
    neighbours, points, prism_like, ribbon, size_of, sound, tolerance,
};

/// The volume of a primitive with every face moved out by `d`, in
/// closed form; `None` for the other kinds.
fn grown(made: &hunt_common::Made, d: f64) -> Option<f64> {
    use std::f64::consts::PI;
    let k = made.k;
    let dims: Vec<f64> = made.dims.iter().map(|x| x * k).collect();
    match made.kind {
        Kind::Block => Some((dims[0] + 2.0 * d) * (dims[1] + 2.0 * d) * (dims[2] + 2.0 * d)),
        Kind::Cylinder => Some(PI * (dims[0] + d).ppowi(2) * (dims[1] + 2.0 * d)),
        Kind::Ball => Some(4.0 / 3.0 * PI * (dims[0] + d).ppowi(3)),
        Kind::Ring => Some(2.0 * PI * PI * dims[0] * (dims[1] + d).ppowi(2)),
        Kind::Frustum => {
            let (r, h, cut) = (dims[0], dims[1], dims[2]);
            let cos = h / r.phypot(h);
            let f = |z: f64| r * (1.0 - z / h) + d / cos;
            let (z0, z1) = (-d, cut + d);
            let (a, b) = (f(z0), f(z1));
            Some(PI * (a * a + a * b + b * b) * (z1 - z0) / 3.0)
        }
        Kind::Filleted if made.dims[1] == 1.0 => {
            // One edge of the block 40 x 30 x 20 rounded: the fillet is
            // rebuilt with its radius grown by the distance.
            let r = dims[0] + d;
            let length = [40.0, 30.0, 20.0][(made.dims[2] as usize % 12) / 4] * k + 2.0 * d;
            (r > 0.0).then(|| {
                (40.0 * k + 2.0 * d) * (30.0 * k + 2.0 * d) * (20.0 * k + 2.0 * d)
                    - (1.0 - PI / 4.0) * r * r * length
            })
        }
        _ => None,
    }
}

const SEED_OFFSET: u64 = 0x0FF5_E7F4_2026_1114;

/// Seeded random offsets of one face, from small to well past what fits,
/// outward and inward.
fn offsets(cases: std::ops::Range<usize>) -> Tally {
    offsets_from(SEED_OFFSET, false, cases)
}

/// The offsets at critical distances: where the moved face reaches a
/// vertex of the body that is not on it (its plane through the vertex, its
/// radius through it), exactly or a hundredth or a ten-thousandth either
/// side: where an offset changes how the body hangs together.
fn critical_offsets(cases: std::ops::Range<usize>) -> Tally {
    offsets_from(SEED_CRITICAL, true, cases)
}

const SEED_CRITICAL: u64 = 0x0C41_7EC0_2026_1114;

fn offsets_from(seed: u64, at_critical: bool, cases: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in cases {
        let clock = std::time::Instant::now();
        let mut rng = Rng(case_seed(seed, n));
        let kind = KINDS[n % KINDS.len()];
        let k = SCALES[rng.below(SCALES.len() as u64) as usize];
        let tol = tolerance(k);
        let Some(made) = make(&mut rng, kind, k, &tol) else {
            tally.outcomes.push('S');
            continue;
        };
        let body = &made.body;
        let Some(m0) = measured(body) else {
            tally.outcomes.push('S');
            continue;
        };
        let face = FaceId(rng.below(body.faces.len() as u64) as u32);
        let sign = if rng.below(2) == 0 { 1.0 } else { -1.0 };
        let mut d = sign * 0.2 * (1.5_f64).ppowi(rng.below(10) as i32) * k;
        if at_critical {
            let found = critical(body, face, tol.linear);
            if found.is_empty() {
                tally.outcomes.push('N');
                continue;
            }
            let c = found[rng.below(found.len() as u64) as usize];
            let e = [-1e-2, -1e-4, 0.0, 1e-4, 1e-2][rng.below(5) as usize];
            d = c * (1.0 + e);
        }
        // One face mostly; sometimes a face and a neighbour at distances of
        // their own, or every face by the same distance.
        let mode = if at_critical { 0 } else { rng.below(6) };
        let mut moved = vec![FaceOffset { face, distance: d }];
        if mode == 4 {
            let next = neighbours(body, face);
            if !next.is_empty() {
                let other = next[rng.below(next.len() as u64) as usize];
                let sign = if rng.below(2) == 0 { 1.0 } else { -1.0 };
                let d2 = sign * 0.2 * (1.5_f64).ppowi(rng.below(10) as i32) * k;
                moved.push(FaceOffset {
                    face: other,
                    distance: d2,
                });
            }
        } else if mode == 5 {
            moved = (0..body.faces.len() as u32)
                .map(|f| FaceOffset {
                    face: FaceId(f),
                    distance: d,
                })
                .collect();
        }
        let faces: Vec<FaceId> = moved.iter().map(|m| m.face).collect();
        let what = format!(
            "case {n} ({}): {}",
            made.what,
            if mode == 5 {
                format!("every face offset {}", g(d))
            } else {
                moved
                    .iter()
                    .map(|m| {
                        format!(
                            "face {} ({}) offset {}",
                            m.face.0,
                            body.face(m.face).unwrap().surface.kind(),
                            g(m.distance)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        );
        let outward = moved.iter().all(|m| m.distance > 0.0);
        let inward = moved.iter().all(|m| m.distance < 0.0);
        let run = |b: &Body, _: &Transform| offset_faces(b, &moved, &tol).map(|r| r.body);
        let result = guarded(|| run(body, &identity()));
        let after = match result {
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
            let slack = 1e-9 * m0.volume + 4.0 * tol.linear * m0.area;
            if outward && m1.volume < m0.volume - slack {
                found.problems.push(format!(
                    "an outward offset lost volume: {} to {}",
                    g(m0.volume),
                    g(m1.volume)
                ));
            }
            if inward && m1.volume > m0.volume + slack {
                found.problems.push(format!(
                    "an inward offset gained volume: {} to {}",
                    g(m0.volume),
                    g(m1.volume)
                ));
            }
            if moved.len() == 1
                && prism_like(body, face).is_some()
                && let Some(Ok(area)) = face_areas(body).get(face.0 as usize)
            {
                let expected = m0.volume + area * d;
                let allowed = allowance(&m0, &m1, 0.0, &tol) + 1e-9 * area * d.abs();
                if (m1.volume - expected).abs() > allowed {
                    found.problems.push(format!(
                        "a flat face square to its neighbours, of area {}, moved {}: volume {} for {} (allowed {})",
                        g(*area),
                        g(d),
                        g(m1.volume),
                        g(expected),
                        g(allowed)
                    ));
                }
            }
            if mode == 5
                && let Some(expected) = grown(&made, d)
            {
                let allowed = allowance(&m0, &m1, 0.0, &tol) + 1e-9 * expected;
                if (m1.volume - expected).abs() > allowed {
                    found.problems.push(format!(
                        "every face of a primitive moved {}: volume {} for {} (allowed {})",
                        g(d),
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
            let s = faces
                .iter()
                .map(|f| least_sine(body, *f))
                .fold(1.0, f64::min)
                .max(0.2);
            let far = moved.iter().map(|m| m.distance.abs()).fold(0.0, f64::max);
            let reach = 2.0 * far / s + 50.0 * tol.linear;
            let adds = if outward {
                Some(true)
            } else if inward {
                Some(false)
            } else {
                None
            };
            points(
                &mut rng, body, &after, &faces, adds, reach, &tol, &mut found,
            );
            let back: Vec<FaceOffset> = moved
                .iter()
                .map(|m| FaceOffset {
                    face: m.face,
                    distance: -m.distance,
                })
                .collect();
            match guarded(|| offset_faces(&after, &back, &tol)) {
                Err(p) => found.problems.push(format!("a panic on the way back: {p}")),
                Ok(Err(e)) => found.notes.push(format!("the way back refused: {e}")),
                Ok(Ok(back)) => {
                    if let Some(mb) = measured(&back.body) {
                        let allowed = allowance(&m0, &mb, ribbon(&back.body), &tol);
                        if (mb.volume - m0.volume).abs() > allowed {
                            found.problems.push(format!(
                                "offset back: volume {} for {} (allowed {})",
                                g(mb.volume),
                                g(m0.volume),
                                g(allowed)
                            ));
                        }
                    }
                }
            }
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

const SEED_MOVE: u64 = 0x0A0E_FACE_2026_1114;

/// The kinds of motion the move face hunt draws.
#[derive(Clone, Copy, Debug)]
enum Way {
    /// A flat face along its normal: the offset of it.
    Normal,
    /// A flat face within its plane: nothing changes.
    InPlane,
    /// A round face square to its axis: the volume stays.
    Across,
    /// A flat face turned about a line in it through its centroid.
    Turn,
    /// Any face, any small motion: frames and mesh only.
    Any,
}

/// Seeded random moves of one face.
fn moves(cases: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in cases {
        let clock = std::time::Instant::now();
        let mut rng = Rng(case_seed(SEED_MOVE, n));
        let kind = KINDS[n % KINDS.len()];
        let k = SCALES[rng.below(SCALES.len() as u64) as usize];
        let tol = tolerance(k);
        let Some(made) = make(&mut rng, kind, k, &tol) else {
            tally.outcomes.push('S');
            continue;
        };
        let body = &made.body;
        let Some(m0) = measured(body) else {
            tally.outcomes.push('S');
            continue;
        };
        let size = size_of(body);
        let Some(fine) = mesh(body, 1e-4 * size) else {
            tally.outcomes.push('S');
            continue;
        };
        let mut way =
            [Way::Normal, Way::InPlane, Way::Across, Way::Turn, Way::Any][rng.below(5) as usize];
        // A face that suits the way, at random among those that do.
        let suits = |f: FaceId| {
            let face = body.face(f).unwrap();
            match way {
                Way::Normal | Way::InPlane => matches!(face.surface, Surface::Plane(_)),
                Way::Turn => prism_like(body, f).is_some(),
                Way::Across => {
                    let Surface::Cylinder(c) = &face.surface else {
                        return false;
                    };
                    let axis = c.frame().z();
                    neighbours(body, f)
                        .iter()
                        .all(|o| match &body.face(*o).unwrap().surface {
                            Surface::Plane(p) => p.frame().z().cross(axis).length() < 1e-12,
                            _ => false,
                        })
                }
                Way::Any => true,
            }
        };
        let mut picks: Vec<FaceId> = (0..body.faces.len() as u32)
            .map(FaceId)
            .filter(|f| suits(*f))
            .collect();
        if picks.is_empty() {
            // No face suits the way drawn: any face, any motion.
            way = Way::Any;
            picks = (0..body.faces.len() as u32).map(FaceId).collect();
        }
        let face = picks[rng.below(picks.len() as u64) as usize];
        let (area, centroid) = face_area_centroid(&fine, face.0);
        let normal =
            hunt_common::normal_at(body, face, centroid).unwrap_or(Vec3::new(0.0, 0.0, 1.0));
        let amount = 0.2 * (1.5_f64).ppowi(rng.below(9) as i32) * k;
        let sign = if rng.below(2) == 0 { 1.0 } else { -1.0 };
        let zero = Vec3::new(0.0, 0.0, 0.0);
        let random = Vec3::new(
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
            rng.between(-1.0, 1.0),
        );
        let square = |v: Vec3, n: Vec3| (v - n * v.dot(n)).normalized();
        let motion = match way {
            Way::Normal => Motion {
                translate: normal * (sign * amount),
                pivot: zero,
                axis: normal,
                angle: 0.0,
            },
            Way::InPlane => Motion {
                translate: square(random, normal).unwrap_or(zero) * amount,
                pivot: zero,
                axis: normal,
                angle: 0.0,
            },
            Way::Across => {
                let Surface::Cylinder(c) = &body.face(face).unwrap().surface else {
                    panic!("a face picked to move across is a cylinder")
                };
                Motion {
                    translate: square(random, c.frame().z()).unwrap_or(zero) * (0.3 * amount),
                    pivot: zero,
                    axis: normal,
                    angle: 0.0,
                }
            }
            Way::Turn => Motion {
                translate: zero,
                pivot: centroid,
                axis: square(random, normal).unwrap_or(zero),
                angle: sign * rng.between(0.5, 8.0).to_radians(),
            },
            Way::Any => Motion {
                translate: random * (0.3 * amount),
                pivot: centroid,
                axis: random,
                angle: rng.between(-3.0, 3.0).to_radians(),
            },
        };
        let what = format!(
            "case {n} ({}): face {} ({}) moved {way:?} by {} turned {}",
            made.what,
            face.0,
            body.face(face).unwrap().surface.kind(),
            g(motion.translate.length()),
            g(motion.angle)
        );
        let run = |b: &Body, t: &Transform| {
            let moved = Motion {
                translate: t.vector(motion.translate),
                pivot: t.point(motion.pivot),
                axis: t.vector(motion.axis),
                angle: motion.angle,
            };
            move_faces(b, &[face], &moved, &tol).map(|r| r.body)
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
            match way {
                Way::Normal => {
                    match guarded(|| {
                        offset_faces(
                            body,
                            &[FaceOffset {
                                face,
                                distance: sign * amount,
                            }],
                            &tol,
                        )
                    }) {
                        Ok(Ok(off)) => {
                            if let Some(mo) = measured(&off.body) {
                                let allowed =
                                    allowance(&m1, &mo, ribbon(&after) + ribbon(&off.body), &tol);
                                if (mo.volume - m1.volume).abs() > allowed {
                                    found.problems.push(format!(
                                        "moved along its normal: volume {}, offset as far: {} (allowed {})",
                                        g(m1.volume),
                                        g(mo.volume),
                                        g(allowed)
                                    ));
                                }
                            }
                        }
                        Ok(Err(e)) => found.notes.push(format!("the offset as far refused: {e}")),
                        Err(p) => found
                            .problems
                            .push(format!("the offset as far: a panic: {p}")),
                    }
                }
                Way::InPlane | Way::Across => {
                    let allowed = allowance(&m0, &m1, ribbon(&after), &tol);
                    if (m1.volume - m0.volume).abs() > allowed {
                        found.problems.push(format!(
                            "moved {way:?}: volume {} for {} (allowed {})",
                            g(m1.volume),
                            g(m0.volume),
                            g(allowed)
                        ));
                    }
                }
                Way::Turn => {
                    // The centroid of the mesh lies within its chord of the
                    // face's; the volume moves by that off-centre times the
                    // tangent of the angle, over the area.
                    let allowed = allowance(&m0, &m1, ribbon(&after), &tol)
                        + 1e-4 * size * motion.angle.abs().ptan() * area;
                    if (m1.volume - m0.volume).abs() > allowed {
                        found.problems.push(format!(
                            "turned about a line through its centroid: volume {} for {} (allowed {})",
                            g(m1.volume),
                            g(m0.volume),
                            g(allowed)
                        ));
                    }
                }
                Way::Any => {}
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
            // How far a point of the face goes.
            let far = fine
                .positions
                .iter()
                .map(|p| {
                    let turned = motion.part(1.0).map_or(*p, |t| t.point(*p));
                    turned.distance(*p)
                })
                .fold(0.0, f64::max);
            let s = least_sine(body, face).max(0.2);
            let reach = 2.0 * far / s + 50.0 * tol.linear;
            let adds = match way {
                Way::Normal => Some(sign > 0.0),
                _ => None,
            };
            points(
                &mut rng,
                body,
                &after,
                &[face],
                adds,
                reach,
                &tol,
                &mut found,
            );
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

/// Offset cases seen wrong, owed. Case 822 (a block united with a
/// cylinder lying along it that stands out of both end faces, scale 1e-3,
/// the end cap offset inward past the end face): an invalid body that
/// passes the checker, its cylinder face's loops crossing in its
/// parameters (the tessellator cannot mesh it); the reduced form is
/// `an_end_cap_offset_past_the_face_it_stands_out_of_is_refused_or_built_right`.
const KNOWN_OFFSET: [usize; 1] = [822];

/// Move face cases seen wrong, owed (none known yet).
const KNOWN_MOVE: [usize; 0] = [];

#[test]
fn hunt_offset_scratch() {
    let Some(r) = hunt_common::scratch_range() else {
        return;
    };
    let start = r.start;
    offsets(r).hold("offset", start, &KNOWN_OFFSET, 0);
}

#[test]
fn hunt_move_scratch() {
    let Some(r) = hunt_common::scratch_range() else {
        return;
    };
    let start = r.start;
    moves(r).hold("move", start, &KNOWN_MOVE, 0);
}

/// One offset case again, at its own scale and at others, told in detail.
#[test]
fn hunt_offset_debug() {
    let Some(n) = std::env::var("HUNT_CASE")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
    else {
        return;
    };
    for scale in [None, Some(1e-3), Some(1.0), Some(1e3)] {
        let mut rng = Rng(case_seed(SEED_OFFSET, n));
        let kind = KINDS[n % KINDS.len()];
        let drawn = SCALES[rng.below(SCALES.len() as u64) as usize];
        let k = scale.unwrap_or(drawn);
        let tol = tolerance(k);
        let made = make(&mut rng, kind, k, &tol).unwrap();
        let body = &made.body;
        let face = FaceId(rng.below(body.faces.len() as u64) as u32);
        let sign = if rng.below(2) == 0 { 1.0 } else { -1.0 };
        let d = sign * 0.2 * (1.5_f64).ppowi(rng.below(10) as i32) * k;
        println!(
            "case {n} at scale {k}: {} frame {:?}; face {} offset {}",
            made.what,
            made.frame,
            face.0,
            g(d)
        );
        for (i, f) in body.faces.iter().enumerate() {
            println!(
                "  before face {i}: {} loops {}",
                f.surface.kind(),
                f.loops.len()
            );
        }
        let r = offset_faces(body, &[FaceOffset { face, distance: d }], &tol);
        match r {
            Err(e) => println!("  refused: {e}"),
            Ok(r) => {
                let after = r.body;
                println!("  check: {:?}", check(&after, &tol));
                for (i, f) in after.faces.iter().enumerate() {
                    println!(
                        "  after face {i}: {} loops {}",
                        f.surface.kind(),
                        f.loops.len()
                    );
                }
                println!(
                    "  before mesh: {:?}",
                    mesh_volume(body, 1e-3 * size_of(body))
                );
                println!(
                    "  after mesh: {:?}",
                    mesh_volume(&after, 1e-3 * size_of(&after))
                );
                println!("  before {:?} after {:?}", measured(body), measured(&after));
            }
        }
    }
}

#[allow(dead_code)]
fn unused(_: Kind) {}

/// What an offset of the end cap of a cylinder that stands out of a
/// block's end face did, held to the block united with the cylinder
/// shortened as far (built by the boolean, not by the offset).
fn end_cap(k: f64, d: f64) -> Result<(), String> {
    use craft3d_kernel::boolean::{BooleanOp, boolean};
    use craft3d_kernel::primitives;
    use craft3d_math::Frame;
    let tol = tolerance(k);
    let block = primitives::block(&Frame::WORLD, 40.0 * k, 30.0 * k, 20.0 * k).unwrap();
    let (x, y) = (Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
    // A cylinder r 7 along x, its axis 3 over the block's top, from 1 before
    // the block's end face x = 0 to 1 past the other.
    let rod = |from: f64| {
        let axis = Frame::new(Vec3::new(from * k, 15.0 * k, 23.0 * k), x, y).unwrap();
        let cylinder = primitives::cylinder(&axis, 7.0 * k, (41.0 - from) * k).unwrap();
        boolean(&block, &cylinder, BooleanOp::Union, &tol)
            .unwrap()
            .body
    };
    let body = rod(-1.0);
    let at_start = |f: &FaceId| match &body.face(*f).unwrap().surface {
        Surface::Plane(p) => (p.frame().origin().x + k).abs() < 1e-9 * k,
        _ => false,
    };
    let cap = (0..body.faces.len() as u32)
        .map(FaceId)
        .find(at_start)
        .unwrap();
    let reference = rod(-1.0 - d);
    let mr = measured(&reference).unwrap();
    let Ok(made) = offset_faces(
        &body,
        &[FaceOffset {
            face: cap,
            distance: d * k,
        }],
        &tol,
    ) else {
        return Ok(());
    };
    let m = measured(&made.body).unwrap();
    let mut wrong = Vec::new();
    if !check(&made.body, &tol).is_empty() {
        wrong.push("not valid".to_string());
    }
    if let Err(e) = mesh_volume(&made.body, 1e-3 * size_of(&made.body)) {
        wrong.push(format!("passes the checker but {e}"));
    }
    let allowed = allowance(&m, &mr, 0.0, &tol);
    if (m.volume - mr.volume).abs() > allowed {
        wrong.push(format!(
            "volume {} for {} (off {})",
            g(m.volume),
            g(mr.volume),
            g(m.volume - mr.volume)
        ));
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(wrong.join("; "))
    }
}

/// The reduced form of the offset hunt's first finding (2026-11-14): a
/// block 40 x 30 x 20 united with a cylinder r 7 along x whose axis lies 3
/// over the top and which stands 1 out of both end faces; its end cap
/// offset inward by more than the 1 it stands out (past the block's end
/// face) is built and passes the checker, but its cylinder face's loops
/// cross in its parameters (the tessellator: "its loops do not bound a
/// region") and its volume is 36.29 short of the block united with the
/// cylinder shortened as far, with which points classified by rays agree
/// (offset by 2; 18.15 short by 1.5). By 1 (the cap in the end face) it is
/// refused by the check of simple loops; by 0.5 it is built right. Owed,
/// known: the offsets in [`END_CAP_KNOWN`] are built wrong; a fix has to
/// take them off.
#[test]
fn an_end_cap_offset_past_the_face_it_stands_out_of_is_refused_or_built_right() {
    let mut unexpected = Vec::new();
    for k in [1e-3, 1.0, 1e3] {
        for d in [-0.5, -1.0, -1.5, -2.0, -3.0] {
            let known = END_CAP_KNOWN.contains(&d);
            match end_cap(k, d) {
                Ok(()) if known => unexpected.push(format!(
                    "scale {k}, offset {d}: refused or built right now: take it off END_CAP_KNOWN"
                )),
                Err(e) if !known => unexpected.push(format!("scale {k}, offset {d}: {e}")),
                Err(e) => println!("scale {k}, offset {d}: known wrong: {e}"),
                Ok(()) => println!("scale {k}, offset {d}: refused or right"),
            }
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// The offsets of the end cap built wrong (an invalid body that passes
/// the checker), at every scale.
const END_CAP_KNOWN: [f64; 3] = [-1.5, -2.0, -3.0];

/// The wide hunt of offset: 1200 cases in 8 parts, run by hand
/// (`-- --ignored`), not in the gate.
fn wide_offset(part: usize) {
    let r = part * 150..(part + 1) * 150;
    let start = r.start;
    offsets(r).hold("offset", start, &KNOWN_OFFSET, 0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_offset_0() {
    wide_offset(0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_offset_1() {
    wide_offset(1);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_offset_2() {
    wide_offset(2);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_offset_3() {
    wide_offset(3);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_offset_4() {
    wide_offset(4);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_offset_5() {
    wide_offset(5);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_offset_6() {
    wide_offset(6);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_offset_7() {
    wide_offset(7);
}

#[test]
fn random_offset_0_to_40_hold_their_oracles() {
    offsets(0..40).hold_pinned(
        "offset",
        0,
        &KNOWN_OFFSET,
        PINNED_OFFSET.get(0..40).unwrap_or(""),
        20,
    );
}

#[test]
fn random_offset_40_to_80_hold_their_oracles() {
    offsets(40..80).hold_pinned(
        "offset",
        40,
        &KNOWN_OFFSET,
        PINNED_OFFSET.get(40..80).unwrap_or(""),
        20,
    );
}

/// What the offset hunt's gate cases did when it was written (outcome
/// letters, `hunt_common::Tally`), recorded on 2026-11-14.
const PINNED_OFFSET: &str =
    "BBBBBRBRBBBBRRRBRRBBBBBBBBBBBBBRRRBRBBBBBBBBBBBBBBBBRBBRBBBBBBBBBBBBBBBBBRRBBBBB";

/// The wide hunt of move: 1200 cases in 2 parts, run by hand
/// (`-- --ignored`), not in the gate.
fn wide_move(part: usize) {
    let r = part * 600..(part + 1) * 600;
    let start = r.start;
    moves(r).hold("move", start, &KNOWN_MOVE, 0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_move_0() {
    wide_move(0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_move_1() {
    wide_move(1);
}

#[test]
fn random_move_0_to_200_hold_their_oracles() {
    moves(0..200).hold_pinned(
        "move",
        0,
        &KNOWN_MOVE,
        PINNED_MOVE.get(0..200).unwrap_or(""),
        80,
    );
}

/// What the move hunt's gate cases did when it was written (outcome
/// letters, `hunt_common::Tally`), recorded on 2026-11-14.
const PINNED_MOVE: &str = concat!(
    "BBRBRBRBBBBBBBBBBRRBRRBRRRBBBBBBRRRRRRBRBBRBBBBBRBBBRBBBRBRRBRBBBBBBRBRBBRRRBBRB",
    "RRRBBBBRRBRRRBRBBRBRRBRBRBBBRBBRBRBBBBRBBBBBBBBBBRBRBBRBBRBBBBBBBBBBBBRRBRBBRBBB",
    "BBBBBRBBRRRBBBBRBRBBBBBBRBBBRRBRRBRBBBBB",
);

/// The wide hunt of offsets at critical distances: 1200 cases in 8 parts,
/// run by hand (`-- --ignored`), not in the gate.
fn wide_critical(part: usize) {
    let r = part * 150..(part + 1) * 150;
    let start = r.start;
    critical_offsets(r).hold("critical offset", start, &KNOWN_CRITICAL, 0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_critical_0() {
    wide_critical(0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_critical_1() {
    wide_critical(1);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_critical_2() {
    wide_critical(2);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_critical_3() {
    wide_critical(3);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_critical_4() {
    wide_critical(4);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_critical_5() {
    wide_critical(5);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_critical_6() {
    wide_critical(6);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_critical_7() {
    wide_critical(7);
}

#[test]
fn random_offsets_at_critical_distances_0_to_60_hold_their_oracles() {
    critical_offsets(0..60).hold_pinned(
        "critical offset",
        0,
        &KNOWN_CRITICAL,
        PINNED_CRITICAL.get(0..60).unwrap_or(""),
        5,
    );
}

#[test]
fn hunt_critical_scratch() {
    let Some(r) = hunt_common::scratch_range() else {
        return;
    };
    let start = r.start;
    critical_offsets(r).hold("critical offset", start, &KNOWN_CRITICAL, 0);
}

/// Offsets at critical distances seen wrong, owed: invalid bodies that
/// pass the checker (the tessellator cannot mesh them, and the measure
/// refuses some), each a face moved up to or past a face that bounds its
/// neighbours. Cases 777, 815, 872, 188 and 321: a vase's cap offset past
/// the cap across from it (the reduced form is
/// `a_cap_offset_past_the_cap_across_from_it_is_refused`); 1019: a lying
/// cone's end cap past the block's end face (as
/// `an_end_cap_offset_past_the_face_it_stands_out_of_is_refused_or_built_right`);
/// 182 and 714: a block's end face moved out or in to a slanted cylinder's
/// end; 1041: a spline-walled prism's flat side moved in to within a
/// ten-thousandth of the far side.
const KNOWN_CRITICAL: [usize; 9] = [182, 188, 321, 714, 777, 815, 872, 1019, 1041];

/// What the critical offsets' gate cases did when the hunt was written;
/// recorded on 2026-11-14.
const PINNED_CRITICAL: &str = "RRRNNRRRRRRRRRRNNBNBRNNNRRRRBRRRRRRBNRRRRNNBRBBRBRRRRNNNNBBR";

/// One offset case at a critical distance again (`HUNT_CRITICAL=n`), told
/// in detail: what the checker, the measure, the mesh and the self-check
/// say of the result.
#[test]
fn hunt_critical_debug() {
    let Some(n) = std::env::var("HUNT_CRITICAL")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
    else {
        return;
    };
    let mut rng = Rng(case_seed(SEED_CRITICAL, n));
    let kind = KINDS[n % KINDS.len()];
    let k = SCALES[rng.below(SCALES.len() as u64) as usize];
    let tol = tolerance(k);
    let made = make(&mut rng, kind, k, &tol).unwrap();
    let body = &made.body;
    let face = FaceId(rng.below(body.faces.len() as u64) as u32);
    let _sign = rng.below(2);
    let _size = rng.below(10);
    let found = critical(body, face, tol.linear);
    let c = found[rng.below(found.len() as u64) as usize];
    let e = [-1e-2, -1e-4, 0.0, 1e-4, 1e-2][rng.below(5) as usize];
    let d = c * (1.0 + e);
    println!(
        "case {n}: {} frame {:?}; face {} ({}) offset {} (critical {}, {e})",
        made.what,
        made.frame,
        face.0,
        body.face(face).unwrap().surface.kind(),
        g(d),
        g(c)
    );
    for (i, f) in body.faces.iter().enumerate() {
        println!(
            "  before face {i}: {} loops {}",
            f.surface.kind(),
            f.loops.len()
        );
    }
    println!("  before: {:?}", measured(body));
    match offset_faces(body, &[FaceOffset { face, distance: d }], &tol) {
        Err(e) => println!("  refused: {e}"),
        Ok(r) => {
            let after = r.body;
            println!("  check: {:?}", check(&after, &tol));
            println!("  measure: {:?}", craft3d_topo::mass_properties(&after));
            println!("  mesh: {:?}", mesh_volume(&after, 1e-3 * size_of(&after)));
            println!(
                "  self-check: {:?}",
                craft3d_tess::body_self_intersections(
                    &after,
                    &craft3d_tess::SelfCheckParams::new(tol.linear)
                )
                .map(|c| c.is_clean())
            );
            for (i, v) in after.vertices.iter().enumerate() {
                println!(
                    "  vertex {i}: {:?} (was {:?})",
                    v.point, body.vertices[i].point
                );
            }
        }
    }
}

/// The top cap of a block, a cylinder and a vase (a spline profile turned
/// about its axis) offset inward through the whole body: up to the height
/// built right, past the cap across from it refused. What was built
/// otherwise, by the checker, the measure and the mesh.
fn cap_through(which: usize, past: f64) -> Result<(), String> {
    use craft3d_kernel::primitives;
    use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
    use craft3d_kernel::revolve::revolve;
    use craft3d_math::Frame;
    let tol = tolerance(1.0);
    let line = |id: u64, s: [f64; 2], e: [f64; 2]| SketchCurve {
        id,
        curve: Curve2::Line { start: s, end: e },
    };
    let body = match which {
        0 => primitives::block(&Frame::WORLD, 40.0, 30.0, 20.0).unwrap(),
        1 => primitives::cylinder(&Frame::WORLD, 8.0, 20.0).unwrap(),
        _ => {
            // From r 8 at the bottom to r 12 at the top, bulging out.
            let bulge = if which == 2 { 4.0 } else { 0.0 };
            let curves = vec![
                line(1, [0.0, 0.0], [8.0, 0.0]),
                SketchCurve {
                    id: 2,
                    curve: Curve2::Spline(Spline2 {
                        degree: 3,
                        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                        control: vec![
                            [8.0, 0.0],
                            [8.0 + bulge, 20.0 / 3.0],
                            [10.0, 40.0 / 3.0],
                            [12.0, 20.0],
                        ],
                    }),
                },
                line(3, [12.0, 20.0], [0.0, 20.0]),
                line(4, [0.0, 20.0], [0.0, 0.0]),
            ];
            let profiles = detect_profiles(&curves, &tol).unwrap();
            let plane = Frame::new(
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
            )
            .unwrap();
            revolve(
                &profiles,
                &plane,
                [0.0, 0.0],
                [0.0, 1.0],
                0.0,
                std::f64::consts::TAU,
                &tol,
            )
            .unwrap()
            .body
        }
    };
    let height = |f: &FaceId| match &body.face(*f).unwrap().surface {
        Surface::Plane(p) => p.frame().origin().z,
        _ => f64::NEG_INFINITY,
    };
    let top = (0..body.faces.len() as u32)
        .map(FaceId)
        .max_by(|a, b| height(a).total_cmp(&height(b)))
        .unwrap();
    let Ok(made) = offset_faces(
        &body,
        &[FaceOffset {
            face: top,
            distance: -(20.0 + past),
        }],
        &tol,
    ) else {
        return Ok(());
    };
    if past > 0.0 {
        return Err(format!(
            "built through the bottom: check {:?}, measure {:?}, mesh {:?}",
            check(&made.body, &tol).first(),
            craft3d_topo::mass_properties(&made.body).map(|m| m.volume),
            mesh_volume(&made.body, 0.04)
        ));
    }
    let measured_ok = craft3d_topo::mass_properties(&made.body).is_ok();
    let meshed = mesh_volume(&made.body, 0.04);
    if check(&made.body, &tol).is_empty() && measured_ok && meshed.is_ok() {
        Ok(())
    } else {
        Err(format!("built and not sound: mesh {meshed:?}"))
    }
}

/// The reduced form of the critical offsets' vase cases (777, 815, 872,
/// 188, 321; 2026-11-14): a vase from r 8 to r 12 over 20, its side a
/// spline bulging out by 4, its top offset inward by 20.0001 or 20.01
/// (past its bottom) is built and passes the checker, but its side's loops
/// run the wrong way round, enclosing next to no area: the measure refuses
/// it ("unbounded face") and the tessellator cannot mesh it. By 20.5 the
/// operation's own check refuses it ("every loop runs clockwise"); the
/// same vase without the bulge, a cylinder and a block are refused from
/// 20.0001 on ("too far"). Owed, known: [`CAP_THROUGH_KNOWN`].
#[test]
fn a_cap_offset_past_the_cap_across_from_it_is_refused() {
    let names = ["block", "cylinder", "vase", "straight vase"];
    let mut unexpected = Vec::new();
    for (which, name) in names.iter().enumerate() {
        for past in [-0.5, -1e-2, 1e-4, 1e-2, 0.5] {
            let known = CAP_THROUGH_KNOWN.contains(&(which, past));
            match cap_through(which, past) {
                Ok(()) if known => unexpected.push(format!(
                    "{}, past by {past}: refused or right now: take it off CAP_THROUGH_KNOWN",
                    name
                )),
                Err(e) if !known => {
                    unexpected.push(format!("{}, past by {past}: {e}", name));
                }
                Err(e) => println!("{}, past by {past}: known wrong: {e}", name),
                Ok(()) => println!("{}, past by {past}: refused or right", name),
            }
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// The vase's top offset past its bottom by these, built invalid.
const CAP_THROUGH_KNOWN: [(usize, f64); 2] = [(2, 1e-4), (2, 1e-2)];
