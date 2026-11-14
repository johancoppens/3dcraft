//! A hunt for wrong plates on curved faces and wrong unrolls of
//! developable ones (docs/design/plates.md, docs/design/developable.md),
//! seeded sizes, sides, frames and scales (1e-3 to 1e6):
//!
//! - the side of a cylinder (a face closing on itself across its seam)
//!   plated out, in or about the middle: the shell between the radii,
//!   `π h (r_o² − r_i²)`, its area the two skins and two annuli;
//! - a whole sphere and a whole torus (faces without boundary): the
//!   shells `4π/3 (r_o³ − r_i³)` and `2π² R (r_o² − r_i²)` (Pappus);
//! - the side of a cylinder and of a cone unrolled (`unfold::develop`):
//!   the pattern's area the face's (`2π r h`, `π r s`), each edge of the
//!   outline as long as its curve in the part (circles `2π r`, rulings
//!   `h` or `s`).
//!
//! Every plate also gets `topo::check`, the measure, the mesh's volume and
//! the self-intersection check.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod boolean_common;
mod hunt_sheet_common;

use boolean_common::Rng;
use craft3d_kernel::plate::{Side, plate};
use craft3d_kernel::primitives;
use craft3d_kernel::unfold::develop;
use craft3d_math::Frame;
use craft3d_math::Portable;
use craft3d_topo::{Body, FaceId, mass_properties};
use hunt_sheet_common::*;
use std::collections::BTreeMap;
use std::f64::consts::PI;

const SCALES: [f64; 4] = [1e-3, 1.0, 1.0, 1e6];

fn face_of(body: &Body, kind: &str) -> Option<FaceId> {
    body.faces
        .iter()
        .position(|f| f.surface.kind() == kind)
        .map(|i| FaceId(i as u32))
}

fn shell(r: f64, t: f64, side: Side) -> (f64, f64) {
    let (lo, hi) = side.levels(t);
    (r + lo, r + hi)
}

/// One plate of a curved face: case `n`.
fn plate_case(tally: &mut Tally, n: usize) {
    let mut rng = Rng(case_seed(0xC0_4BED, n));
    let k = SCALES[rng.below(SCALES.len() as u64) as usize];
    let tol = tolerance(k);
    let f = frame(&mut rng, k);
    let side = match rng.below(3) {
        0 => Side::Out,
        1 => Side::In,
        _ => Side::Mid,
    };
    let which = rng.below(3);
    let (body, face, volume, area, what, t) = match which {
        0 => {
            let r = rng.between(2.0, 40.0) * k;
            let h = rng.between(1.0, 60.0) * k;
            let t = rng.between(0.01, 0.9) * r;
            let body = primitives::cylinder(&f, r, h).unwrap();
            let face = face_of(&body, "cylinder").unwrap();
            let (ri, ro) = shell(r, t, side);
            let v = PI * h * (ro * ro - ri * ri);
            let a = 2.0 * PI * h * (ro + ri) + 2.0 * PI * (ro * ro - ri * ri);
            (
                body,
                face,
                v,
                a,
                format!("cylinder side r {r:e} h {h:e} t {t:e} {side:?}"),
                t,
            )
        }
        1 => {
            let r = rng.between(2.0, 40.0) * k;
            let t = rng.between(0.01, 0.9) * r;
            let body = primitives::sphere(&f, r).unwrap();
            let face = face_of(&body, "sphere").unwrap();
            let (ri, ro) = shell(r, t, side);
            let v = 4.0 / 3.0 * PI * (ro.ppowi(3) - ri.ppowi(3));
            let a = 4.0 * PI * (ro * ro + ri * ri);
            (
                body,
                face,
                v,
                a,
                format!("sphere r {r:e} t {t:e} {side:?}"),
                t,
            )
        }
        _ => {
            let rr = rng.between(10.0, 50.0) * k;
            let r = rng.between(0.1, 0.8) * rr;
            let t = rng.between(0.01, 0.9) * r;
            let body = primitives::torus(&f, rr, r).unwrap();
            let face = face_of(&body, "torus").unwrap();
            let (ri, ro) = shell(r, t, side);
            let v = 2.0 * PI * PI * rr * (ro * ro - ri * ri);
            let a = 4.0 * PI * PI * rr * (ro + ri);
            (
                body,
                face,
                v,
                a,
                format!("torus R {rr:e} r {r:e} t {t:e} {side:?}"),
                t,
            )
        }
    };
    let what = format!("{what} at k {k} in {f:?}");
    let made = guarded(|| plate(&body, &[face], t, side, &BTreeMap::new(), &tol));
    let made = match made {
        Err(p) => {
            tally.wrong.push(format!("case {n} (P): {what}: {p}"));
            tally.outcomes.push('P');
            return;
        }
        Ok(Err(e)) => {
            tally.refused(&e.to_string());
            tally.notes.push(format!("case {n} refused: {what}: {e}"));
            return;
        }
        Ok(Ok(m)) => m,
    };
    let mut found = Found::default();
    if let Some(m) = sound(&made.body, &tol, &mut found) {
        let slack = m.bound
            + 1e-9 * volume
            + if any_freeform(&made.body) {
                0.5 * tol.linear * area
            } else {
                0.0
            };
        if (m.volume - volume).abs() > slack {
            found.problems.push(format!(
                "volume {:.17e} against {volume:.17e} ({:.3e} of it)",
                m.volume,
                m.volume / volume - 1.0
            ));
        }
        let slack_a = 1e-9 * area
            + if any_freeform(&made.body) {
                1e-6 * area
            } else {
                0.0
            };
        if (m.area - area).abs() > slack_a {
            found.problems.push(format!(
                "area {:.17e} against {area:.17e} ({:.3e} of it)",
                m.area,
                m.area / area - 1.0
            ));
        }
    }
    let crosses = crosses_itself(&made.body, &tol);
    tally.record(n, &what, &found, crosses);
}

fn plates(range: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in range {
        plate_case(&mut tally, n);
    }
    tally
}

#[test]
fn plates_of_cylinders_spheres_and_tori_are_their_shells() {
    plates(0..150).hold("plates of curved faces", 0, &[], 40);
}

#[test]
#[ignore = "a wide sweep, run by hand"]
fn plates_of_cylinders_spheres_and_tori_wide() {
    plates(150..1150).hold("plates of curved faces (wide)", 150, &[], 0);
}

/// One unroll: the side of a cylinder or a cone, case `n`.
fn unroll_case(tally: &mut Tally, n: usize) {
    let mut rng = Rng(case_seed(0x0E_F01D, n));
    let k = SCALES[rng.below(SCALES.len() as u64) as usize];
    let tol = tolerance(k);
    let f = frame(&mut rng, k);
    let cone = rng.below(2) == 0;
    let r = rng.between(2.0, 40.0) * k;
    let h = rng.between(1.0, 60.0) * k;
    let (body, face, area, ruling, kind) = if cone {
        let body = primitives::cone(&f, r, h).unwrap();
        let s = r.phypot(h);
        let face = face_of(&body, "cone").unwrap();
        (body, face, PI * r * s, s, "cone")
    } else {
        let body = primitives::cylinder(&f, r, h).unwrap();
        let face = face_of(&body, "cylinder").unwrap();
        (body, face, 2.0 * PI * r * h, h, "cylinder")
    };
    let what = format!("{kind} side r {r:e} h {h:e} at k {k} in {f:?}");
    let made = guarded(|| develop(&body, &[face], &tol));
    let dev = match made {
        Err(p) => {
            tally.wrong.push(format!("case {n} (P): {what}: {p}"));
            tally.outcomes.push('P');
            return;
        }
        Ok(Err(e)) => {
            tally.refused(&e.to_string());
            tally.notes.push(format!("case {n} refused: {what}: {e}"));
            return;
        }
        Ok(Ok(d)) => d,
    };
    let mut found = Found::default();
    match dev.flat_body(&Frame::WORLD, &tol) {
        Ok(flat) => match mass_properties(&flat.body) {
            Ok(m) => {
                if (m.area - area).abs() > 1e-7 * area {
                    found.problems.push(format!(
                        "the pattern's area {:.17e} against the face's {area:.17e} ({:.3e} of it)",
                        m.area,
                        m.area / area - 1.0
                    ));
                }
            }
            Err(e) => found.notes.push(format!("the pattern not measured: {e}")),
        },
        Err(e) => found.problems.push(format!("no flat body: {e}")),
    }
    // Each curve of the outline: a circle `2π r` long (the top one of a
    // cylinder too), a ruling `h` or `s`.
    let circle = 2.0 * PI * r;
    for c in dev.outline() {
        let want = if (c.length_3d - circle).abs() <= 1e-6 * circle {
            circle
        } else if (c.length_3d - ruling).abs() <= 1e-6 * ruling {
            ruling
        } else {
            found.problems.push(format!(
                "an outline curve {:e} long in the part, neither a circle {circle:e} nor a ruling {ruling:e}",
                c.length_3d
            ));
            continue;
        };
        if (c.length_flat - want).abs() > 1e-7 * want + 4.0 * tol.linear {
            found.problems.push(format!(
                "an outline curve {:e} long in the pattern for {want:e} in the part",
                c.length_flat
            ));
        }
    }
    tally.record(n, &what, &found, false);
}

fn unrolls(range: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in range {
        unroll_case(&mut tally, n);
    }
    tally
}

#[test]
fn unrolled_cylinders_and_cones_keep_their_area_and_lengths() {
    unrolls(0..120).hold("unrolled cylinders and cones", 0, &[], 60);
}

#[test]
#[ignore = "a wide sweep, run by hand"]
fn unrolled_cylinders_and_cones_wide() {
    unrolls(120..720).hold("unrolled cylinders and cones (wide)", 120, &[], 0);
}
