//! A hunt for wrong answers of shell and thicken: seeded random bodies of
//! many kinds (`hunt_common`), made hollow with random thicknesses and
//! random open faces, or a random face thickened to a random side, and
//! every result held to oracles that do not ask how it was built:
//!
//! - the checker, the measure against the mesh's volume, and frame
//!   invariance (the operation on the body moved far away and turned, and
//!   beside an unrelated far cube), as in `hunt_offset`;
//! - closed forms: the hollow of a block, a cylinder, a frustum, a ball
//!   and a ring, open or closed (the sharp inward offset: every face moved
//!   in by the thickness and continued, an open face moved out); a layer
//!   on a flat face whose neighbours stand square to it is its area times
//!   the thickness, on a cylinder's wall, a ball and a ring the difference
//!   of two of them;
//! - identities: a closed shell is the solid less its inward offset; a
//!   layer outside is the outward offset less the solid, inside the solid
//!   less the inward offset, on both sides the one less the other;
//! - points classified by rays: a shell never adds material; a point of
//!   the solid within nine tenths of the thickness of a face that keeps
//!   its wall, and more than three thicknesses from an open face, stays
//!   solid; one farther than four thicknesses from every
//!   face is hollowed out; a layer lies on its side of the solid, and a
//!   point a fraction of the thickness off the face, along its normal, far
//!   from the other faces, is in it.

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
use craft3d_kernel::boolean::Place;
use craft3d_kernel::offset_face::{FaceOffset, offset_faces};
use craft3d_kernel::shell::shell;
use craft3d_kernel::thicken::{Side, thicken};
use craft3d_math::{Portable, Tolerance, Transform, Vec3};
use craft3d_topo::{Body, FaceId, face_areas};
use hunt_common::{
    Cube, Found, KINDS, Kind, Made, Measured, SCALES, Tally, allowance, case_seed, classify,
    distance_to, frames, g, guarded, identity, letter, make, measured, mesh, normal_at,
    points_in_box, points_near, prism_like, ribbon, size_of, sound, tolerance,
};
use std::f64::consts::PI;

/// The volume of `body` with `faces` offset by `distance`, or a note why
/// there is none.
fn offset_volume(
    body: &Body,
    faces: &[FaceId],
    distance: f64,
    tol: &Tolerance,
) -> Result<Measured, String> {
    let offsets: Vec<FaceOffset> = faces
        .iter()
        .map(|f| FaceOffset { face: *f, distance })
        .collect();
    match guarded(|| offset_faces(body, &offsets, tol)) {
        Err(p) => Err(format!("a panic: {p}")),
        Ok(Err(e)) => Err(format!("refused: {e}")),
        Ok(Ok(r)) => {
            let m = measured(&r.body).ok_or("cannot be measured")?;
            Ok(Measured {
                bound: m.bound + ribbon(&r.body),
                ..m
            })
        }
    }
}

/// Which axis of the frame the outward normal of the flat `face` runs
/// along, and which way: `(axis, positive)`.
fn axis_of(made: &Made, face: FaceId, centre: Vec3) -> Option<(usize, bool)> {
    let n = normal_at(&made.body, face, centre)?;
    let f = made.frame;
    [f.x(), f.y(), f.z()]
        .iter()
        .enumerate()
        .find(|(_, a)| (a.dot(n).abs() - 1.0).abs() < 1e-9)
        .map(|(i, a)| (i, a.dot(n) > 0.0))
}

/// The volume taken out of a primitive by a shell of `t` open at `open`,
/// in closed form; `None` where there is none here.
fn hollow(made: &Made, open: &[FaceId], t: f64) -> Option<f64> {
    let k = made.k;
    let body = &made.body;
    let centre = |f: FaceId| -> Vec3 {
        // A point of the face: the middle of an edge of it, or of the
        // surface's frame.
        let face = body.face(f).unwrap();
        face.loops
            .first()
            .and_then(|l| body.loop_(*l))
            .and_then(|l| l.coedges.first())
            .and_then(|c| body.edge(c.edge))
            .map_or_else(
                || match &face.surface {
                    Surface::Plane(p) => p.frame().origin(),
                    _ => made.frame.origin(),
                },
                |e| e.curve.eval(0.5 * (e.range.0 + e.range.1)),
            )
    };
    match made.kind {
        Kind::Block => {
            let dims: Vec<f64> = made.dims.iter().map(|d| d * k).collect();
            let mut inner = Vec::new();
            for (axis, length) in dims.iter().enumerate() {
                let opened = open
                    .iter()
                    .filter(|f| axis_of(made, **f, centre(**f)).map(|a| a.0) == Some(axis))
                    .count();
                inner.push(length - (2.0 - opened as f64) * t);
            }
            inner
                .iter()
                .all(|d| *d > 0.0)
                .then(|| inner.iter().product())
        }
        Kind::Cylinder => {
            let (r, h) = (made.dims[0] * k, made.dims[1] * k);
            let mut radius = r - t;
            let (mut z0, mut z1) = (t, h - t);
            for f in open {
                match &body.face(*f).unwrap().surface {
                    Surface::Cylinder(_) => radius = r,
                    Surface::Plane(_) => {
                        let n = normal_at(body, *f, centre(*f))?;
                        if n.dot(made.frame.z()) > 0.0 {
                            z1 = h;
                        } else {
                            z0 = 0.0;
                        }
                    }
                    _ => return None,
                }
            }
            (radius > 0.0 && z1 > z0).then_some(PI * radius * radius * (z1 - z0))
        }
        Kind::Frustum => {
            let (r, h, cut) = (made.dims[0] * k, made.dims[1] * k, made.dims[2] * k);
            let (mut z0, mut z1) = (t, cut - t);
            for f in open {
                match &body.face(*f).unwrap().surface {
                    Surface::Plane(_) => {
                        let n = normal_at(body, *f, centre(*f))?;
                        if n.dot(made.frame.z()) > 0.0 {
                            z1 = cut;
                        } else {
                            z0 = 0.0;
                        }
                    }
                    _ => return None,
                }
            }
            let cos = h / r.phypot(h);
            let f = |z: f64| r * (1.0 - z / h) - t / cos;
            let (a, b) = (f(z0), f(z1));
            (z1 > z0 && a > 0.0 && b > 0.0).then(|| PI * (a * a + a * b + b * b) * (z1 - z0) / 3.0)
        }
        Kind::Ball => {
            let r = made.dims[0] * k;
            (open.is_empty() && r > t).then(|| 4.0 / 3.0 * PI * (r - t).ppowi(3))
        }
        Kind::Ring => {
            let (major, minor) = (made.dims[0] * k, made.dims[1] * k);
            (open.is_empty() && minor > t).then(|| 2.0 * PI * PI * major * (minor - t).ppowi(2))
        }
        _ => None,
    }
}

/// The points of a shell: no material added; a point of the solid within
/// nine tenths of `t` of a face that keeps its wall stays solid; one
/// farther than four times `t` from every face is hollow.
fn shell_points(
    rng: &mut Rng,
    before: &Body,
    after: &Body,
    open: &[FaceId],
    t: f64,
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
    let all = |_: u32| true;
    let walls = |f: u32| !open.iter().any(|o| o.0 == f);
    let mut pts = points_near(rng, &mesh, &all, 2.0 * t, 160);
    pts.extend(points_in_box(rng, before, 100));
    let (Some(was), Some(is)) = (classify(before, &pts, tol), classify(after, &pts, tol)) else {
        found.notes.push("points could not be classified".into());
        return;
    };
    for ((p, a), b) in pts.iter().zip(&was).zip(&is) {
        if *a == Place::Outside && *b == Place::Inside {
            found.problems.push(format!(
                "a point {} from the body added to it",
                g(distance_to(&mesh, &all, *p))
            ));
            return;
        }
        if *a != Place::Inside {
            continue;
        }
        let wall = distance_to(&mesh, &walls, *p);
        // Next to an opening a wall that meets the open face at a concave
        // edge ends a thickness short of it (the convention of the shell:
        // the open face moved out, its neighbours continued to it;
        // docs/design/face-operations.md › *The kernel set's shell
        // refusals*): not held there.
        let opening = distance_to(&mesh, &|f| open.iter().any(|o| o.0 == f), *p);
        if wall < 0.9 * t - chord && opening > 3.0 * t + chord && *b == Place::Outside {
            // The nearest face, for the report.
            let nearest = (0..before.faces.len() as u32)
                .filter(|f| walls(*f))
                .min_by(|x, y| {
                    distance_to(&mesh, &|f| f == *x, *p).total_cmp(&distance_to(
                        &mesh,
                        &|f| f == *y,
                        *p,
                    ))
                })
                .unwrap_or(0);
            found.problems.push(format!(
                "a point {} from a face that keeps its wall (thickness {}) hollowed out: at ({}, {}, {}), nearest face {nearest}",
                g(wall),
                g(t),
                g(p.x),
                g(p.y),
                g(p.z)
            ));
            return;
        }
        let any = distance_to(&mesh, &all, *p);
        if any > 4.0 * t + chord && *b == Place::Inside {
            found.problems.push(format!(
                "a point {} from every face (thickness {}) left solid",
                g(any),
                g(t)
            ));
            return;
        }
    }
}

const SEED_SHELL: u64 = 0x05E1_1ED0_2026_1114;

/// Seeded random shells.
fn shells(cases: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in cases {
        let clock = std::time::Instant::now();
        let mut rng = Rng(case_seed(SEED_SHELL, n));
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
        let t = [0.005, 0.01, 0.02, 0.04, 0.08, 0.15][rng.below(6) as usize] * size;
        let count = (rng.below(3) as usize).min(body.faces.len().saturating_sub(1));
        let mut open: Vec<FaceId> = Vec::new();
        while open.len() < count {
            let f = FaceId(rng.below(body.faces.len() as u64) as u32);
            if !open.contains(&f) {
                open.push(f);
            }
        }
        let what = format!(
            "case {n} ({}): shell {} open at {:?}",
            made.what,
            g(t),
            open.iter().map(|f| f.0).collect::<Vec<_>>()
        );
        let run = |b: &Body, _: &Transform| shell(b, t, &open, &tol).map(|s| s.body);
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
            let slack = 1e-9 * m0.volume + 4.0 * tol.linear * m0.area;
            if m1.volume > m0.volume + slack {
                found.problems.push(format!(
                    "a shell gained volume: {} to {}",
                    g(m0.volume),
                    g(m1.volume)
                ));
            }
            if let Some(taken) = hollow(&made, &open, t) {
                let expected = m0.volume - taken;
                let allowed = allowance(&m0, &m1, 0.0, &tol) + 1e-9 * taken;
                if (m1.volume - expected).abs() > allowed {
                    found.problems.push(format!(
                        "the closed form: volume {} for {} (the hollow {}, allowed {})",
                        g(m1.volume),
                        g(expected),
                        g(taken),
                        g(allowed)
                    ));
                }
            }
            if open.is_empty() {
                let all: Vec<FaceId> = (0..body.faces.len() as u32).map(FaceId).collect();
                match offset_volume(body, &all, -t, &tol) {
                    Ok(inner) => {
                        let expected = m0.volume - inner.volume;
                        let allowed = allowance(&m0, &m1, ribbon(&after), &tol) + inner.bound;
                        if (m1.volume - expected).abs() > allowed {
                            found.problems.push(format!(
                                "a closed shell: volume {}, the solid less its inward offset {} (allowed {})",
                                g(m1.volume),
                                g(expected),
                                g(allowed)
                            ));
                        }
                    }
                    Err(e) => found.notes.push(format!("the inward offset: {e}")),
                }
            }
            frames(
                &mut rng,
                body,
                &after,
                &m1,
                &run,
                Cube::Shelled(k.max(20.0 * t), t),
                &tol,
                &mut found,
            );
            shell_points(&mut rng, body, &after, &open, t, &tol, &mut found);
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

/// The layer of `t` on a primitive's face to `side`, in closed form;
/// `None` where there is none here.
fn layer(made: &Made, face: FaceId, t: f64, side: Side) -> Option<f64> {
    let k = made.k;
    let surface = &made.body.face(face)?.surface;
    let (outer, inner) = match side {
        Side::Outside => (t, 0.0),
        Side::Inside => (0.0, t),
        Side::Both => (0.5 * t, 0.5 * t),
    };
    match (made.kind, surface) {
        (Kind::Cylinder, Surface::Cylinder(_)) => {
            let (r, h) = (made.dims[0] * k, made.dims[1] * k);
            (r > inner).then(|| PI * ((r + outer).ppowi(2) - (r - inner).ppowi(2)) * h)
        }
        (Kind::Ball, Surface::Sphere(_)) => {
            let r = made.dims[0] * k;
            (r > inner).then(|| 4.0 / 3.0 * PI * ((r + outer).ppowi(3) - (r - inner).ppowi(3)))
        }
        (Kind::Ring, Surface::Torus(_)) => {
            let (major, minor) = (made.dims[0] * k, made.dims[1] * k);
            (minor > inner && minor + outer < major).then(|| {
                2.0 * PI * PI * major * ((minor + outer).ppowi(2) - (minor - inner).ppowi(2))
            })
        }
        _ => None,
    }
}

/// The points of a layer: on its side of the solid, and a point a
/// fraction of `t` off the face along its normal, far from the other
/// faces, in it.
#[allow(clippy::too_many_arguments)]
fn layer_points(
    rng: &mut Rng,
    before: &Body,
    after: &Body,
    face: FaceId,
    t: f64,
    side: Side,
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
    let on = |f: u32| f == face.0;
    let others = |f: u32| f != face.0;
    let mut pts = points_near(rng, &mesh, &on, 1.5 * t, 120);
    pts.extend(points_in_box(rng, before, 60));
    // Along the normal from points of the face.
    let mut along = Vec::new();
    for q in points_near(rng, &mesh, &on, 0.0, 80) {
        // The point of the face itself, not of its mesh (which lies up to
        // its chord off the face).
        let surface = &before.face(face).unwrap().surface;
        let Ok((u, v)) = surface.project(q) else {
            continue;
        };
        let q = surface.eval(u, v);
        let Some(n) = normal_at(before, face, q) else {
            continue;
        };
        let s = match side {
            Side::Outside => rng.between(0.1, 0.9) * t,
            Side::Inside => -rng.between(0.1, 0.9) * t,
            Side::Both => rng.between(-0.45, 0.45) * t,
        };
        let p = q + n * s;
        if distance_to(&mesh, &others, p) > 1.5 * s.abs() + 2.0 * chord && s.abs() > 2.0 * chord {
            along.push(p);
        }
    }
    let (Some(was), Some(is)) = (classify(before, &pts, tol), classify(after, &pts, tol)) else {
        found.notes.push("points could not be classified".into());
        return;
    };
    for ((p, a), b) in pts.iter().zip(&was).zip(&is) {
        if *b != Place::Inside {
            continue;
        }
        let wrong_side = match side {
            Side::Outside => *a == Place::Inside,
            Side::Inside => *a == Place::Outside,
            Side::Both => false,
        };
        if wrong_side {
            found.problems.push(format!(
                "a point of the layer {} from the face on the wrong side of the solid",
                g(distance_to(&mesh, &on, *p))
            ));
            return;
        }
        let away = distance_to(&mesh, &on, *p);
        if away > 1.5 * t + 2.0 * chord + 20.0 * tol.linear {
            found.problems.push(format!(
                "a point of the layer {} from the face (thickness {})",
                g(away),
                g(t)
            ));
            return;
        }
    }
    if let Some(is) = classify(after, &along, tol) {
        for (p, b) in along.iter().zip(&is) {
            if *b == Place::Outside {
                found.problems.push(format!(
                    "a point {} off the face along its normal (thickness {}) not in the layer",
                    g(distance_to(&mesh, &on, *p)),
                    g(t)
                ));
                return;
            }
        }
    }
}

const SEED_THICKEN: u64 = 0x07C1_CE40_2026_1114;

/// Seeded random layers on one face.
fn layers(cases: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in cases {
        let clock = std::time::Instant::now();
        let mut rng = Rng(case_seed(SEED_THICKEN, n));
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
        let face = FaceId(rng.below(body.faces.len() as u64) as u32);
        let side = [Side::Outside, Side::Inside, Side::Both][rng.below(3) as usize];
        let t = [0.005, 0.01, 0.02, 0.04, 0.08, 0.15][rng.below(6) as usize] * size;
        let what = format!(
            "case {n} ({}): face {} ({}) thickened {} {side:?}",
            made.what,
            face.0,
            body.face(face).unwrap().surface.kind(),
            g(t)
        );
        let run = |b: &Body, _: &Transform| thicken(b, &[face], t, side, &tol).map(|s| s.body);
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
            // The layer between the two offsets.
            let (outer, inner) = match side {
                Side::Outside => (t, 0.0),
                Side::Inside => (0.0, t),
                Side::Both => (0.5 * t, 0.5 * t),
            };
            let at = |d: f64| {
                if d == 0.0 {
                    Ok(Measured {
                        bound: m0.bound + ribbon(body),
                        ..m0
                    })
                } else {
                    offset_volume(body, &[face], d, &tol)
                }
            };
            match (at(outer), at(-inner)) {
                (Ok(a), Ok(b)) => {
                    let expected = a.volume - b.volume;
                    let allowed =
                        allowance(&m1, &a, ribbon(&after), &tol) + b.bound + 1e-10 * a.volume;
                    if (m1.volume - expected).abs() > allowed {
                        found.problems.push(format!(
                            "the layer is {}, between the offsets {} (allowed {})",
                            g(m1.volume),
                            g(expected),
                            g(allowed)
                        ));
                    }
                }
                (Err(e), _) | (_, Err(e)) => found.notes.push(format!("an offset: {e}")),
            }
            let closed = layer(&made, face, t, side).or_else(|| {
                prism_like(body, face).and_then(|_| {
                    face_areas(body)
                        .get(face.0 as usize)
                        .and_then(|a| a.clone().ok())
                        .map(|a| a * t)
                })
            });
            if let Some(expected) = closed {
                let allowed = allowance(&m0, &m1, 0.0, &tol) + 1e-9 * expected;
                if (m1.volume - expected).abs() > allowed {
                    found.problems.push(format!(
                        "the closed form: the layer is {} for {} (allowed {})",
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
                Cube::Dropped(k),
                &tol,
                &mut found,
            );
            layer_points(&mut rng, body, &after, face, t, side, &tol, &mut found);
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

/// Shell cases seen wrong, owed (none known yet).
const KNOWN_SHELL: [usize; 0] = [];

/// Thicken cases seen wrong, owed (none known yet).
const KNOWN_THICKEN: [usize; 0] = [];

#[test]
fn hunt_shell_scratch() {
    let Some(r) = hunt_common::scratch_range() else {
        return;
    };
    let start = r.start;
    shells(r).hold("shell", start, &KNOWN_SHELL, 0);
}

#[test]
fn hunt_thicken_scratch() {
    let Some(r) = hunt_common::scratch_range() else {
        return;
    };
    let start = r.start;
    layers(r).hold("thicken", start, &KNOWN_THICKEN, 0);
}

/// The reduced form of the shell hunt's first finding (2026-11-14): a
/// block 40 x 30 x 20 with its top front edge filleted, made hollow open
/// at the fillet face (alone or with any face that is not tangent to it),
/// is refused at thicknesses below the radius with a reason ("an open face
/// with a blend along it has to be flat to be opened"), and from the
/// radius on with an internal error that calls itself a bug ("the shell
/// lost track of a face of the hollow"). A refusal, not a wrong body; owed
/// all the same: the thicknesses in [`OPEN_ROUND_KNOWN`] (times the radius)
/// give it now, and a fix has to take them off.
#[test]
fn a_shell_open_at_a_fillet_thicker_than_its_radius_is_refused_with_a_reason() {
    use craft3d_kernel::fillet::{FilletEdge, FilletFace, fillet};
    use craft3d_kernel::primitives;
    use craft3d_math::Frame;
    let mut unexpected = Vec::new();
    for k in [1e-3, 1.0, 1e3] {
        let tol = tolerance(k);
        let block = primitives::block(&Frame::WORLD, 40.0 * k, 30.0 * k, 20.0 * k).unwrap();
        let edge = hunt_common::edge_near(&block, Vec3::new(20.0 * k, 0.0, 20.0 * k));
        for r in [2.0, 2.868] {
            let made = fillet(
                &block,
                &[FilletEdge {
                    edge,
                    radius: r * k,
                }],
                &tol,
            )
            .unwrap();
            let round = made
                .faces
                .iter()
                .position(|f| matches!(f, FilletFace::Fillet(_)))
                .unwrap() as u32;
            for times in [0.5, 1.25, 2.0, 3.0] {
                let t = times * r * k;
                let known = OPEN_ROUND_KNOWN.contains(&times);
                match shell(&made.body, t, &[FaceId(round)], &tol) {
                    Ok(_) => unexpected.push(format!(
                        "scale {k}, radius {r}, thickness {times} times it: built (not looked at here)"
                    )),
                    Err(e) => {
                        let bug = e.to_string().contains("internal error");
                        if bug && !known {
                            unexpected.push(format!(
                                "scale {k}, radius {r}, thickness {times} times it: {e}"
                            ));
                        } else if !bug && known {
                            unexpected.push(format!(
                                "scale {k}, radius {r}, thickness {times} times it: refused with a reason now ({e}): take it off OPEN_ROUND_KNOWN"
                            ));
                        } else {
                            println!("scale {k}, radius {r}, thickness {times} times it: {e}");
                        }
                    }
                }
            }
        }
    }
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// The thicknesses (times the fillet's radius) at which the shell open at
/// the fillet is refused with an internal error.
const OPEN_ROUND_KNOWN: [f64; 3] = [1.25, 2.0, 3.0];

/// The wide hunt of shell: 480 cases in 8 parts, run by hand
/// (`-- --ignored`), not in the gate.
fn wide_shell(part: usize) {
    let r = part * 60..(part + 1) * 60;
    let start = r.start;
    shells(r).hold("shell", start, &KNOWN_SHELL, 0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_shell_0() {
    wide_shell(0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_shell_1() {
    wide_shell(1);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_shell_2() {
    wide_shell(2);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_shell_3() {
    wide_shell(3);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_shell_4() {
    wide_shell(4);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_shell_5() {
    wide_shell(5);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_shell_6() {
    wide_shell(6);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_shell_7() {
    wide_shell(7);
}

#[test]
fn random_shell_0_to_40_hold_their_oracles() {
    shells(0..40).hold_pinned(
        "shell",
        0,
        &KNOWN_SHELL,
        PINNED_SHELL.get(0..40).unwrap_or(""),
        12,
    );
}

/// What the shell hunt's gate cases did when it was written (outcome
/// letters, `hunt_common::Tally`); empty: not pinned yet.
const PINNED_SHELL: &str = "";

/// The wide hunt of thicken: 480 cases in 8 parts, run by hand
/// (`-- --ignored`), not in the gate.
fn wide_thicken(part: usize) {
    let r = part * 60..(part + 1) * 60;
    let start = r.start;
    layers(r).hold("thicken", start, &KNOWN_THICKEN, 0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_thicken_0() {
    wide_thicken(0);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_thicken_1() {
    wide_thicken(1);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_thicken_2() {
    wide_thicken(2);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_thicken_3() {
    wide_thicken(3);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_thicken_4() {
    wide_thicken(4);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_thicken_5() {
    wide_thicken(5);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_thicken_6() {
    wide_thicken(6);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_thicken_7() {
    wide_thicken(7);
}

#[test]
fn random_thicken_0_to_40_hold_their_oracles() {
    layers(0..40).hold_pinned(
        "thicken",
        0,
        &KNOWN_THICKEN,
        PINNED_THICKEN.get(0..40).unwrap_or(""),
        12,
    );
}

/// What the thicken hunt's gate cases did when it was written (outcome
/// letters, `hunt_common::Tally`); empty: not pinned yet.
const PINNED_THICKEN: &str = "";
