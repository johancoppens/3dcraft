//! A hunt for wrong answers of revolve with spline profiles (the seeded
//! run of `revolve_random` has lines and arcs): a random star-shaped
//! profile whose sides are lines and cubic Bézier splines, clear of the
//! axis or ending on it, turned through a random angle or a whole turn,
//! held to oracles that do not ask how the body was built:
//!
//! - the checker, and the volume of the closed mesh against the measure;
//! - Pappus: the volume is the profile's area times the way its centroid
//!   goes round, the area the length of its boundary times the way the
//!   boundary's centroid goes round (and the two caps of a partial turn);
//!   the profile's area and moments are integrated here, by Gauss–Legendre
//!   on each side (exact for the lines and the cubics' polynomials);
//! - frame invariance: the same profile in a sketch plane moved some 125
//!   times its size away and turned gives the same volume and area.

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
use craft3d_kernel::profile::detect_profiles;
use craft3d_kernel::revolve::revolve;
use craft3d_math::{Frame, Vec3};
use craft3d_topo::check;
use hunt_common::{
    Tally, case_seed, curves, far_motion, g, guarded, measured, mesh_volume, pappus, profile,
    tolerance,
};
use std::f64::consts::TAU;

const SEED_REVOLVE: u64 = 0x0E7A_1E00_2026_1114;

fn revolutions(cases: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in cases {
        let mut rng = Rng(case_seed(SEED_REVOLVE, n));
        let k = hunt_common::SCALES[rng.below(4) as usize];
        let tol = tolerance(k);
        let touch = rng.below(4) == 0;
        let centre = [rng.between(8.0, 20.0), rng.between(-5.0, 5.0)];
        let sides = profile(&mut rng, touch, centre);
        // Every point of the profile right of the axis (a cubic may bulge
        // past it): else no case.
        let left = sides
            .iter()
            .flat_map(|s| (0..=64).map(move |i| s.at(f64::from(i) / 64.0).0[0]))
            .fold(f64::INFINITY, f64::min);
        if left < -1e-12 {
            tally.outcomes.push('S');
            continue;
        }
        let angle = if rng.below(3) == 0 {
            TAU
        } else {
            rng.between(0.2, 6.0)
        };
        let frame = hunt_common::frame(&mut rng, k);
        let Ok(profiles) = detect_profiles(&curves(&sides, k), &tol) else {
            tally.outcomes.push('S');
            continue;
        };
        let what = format!(
            "case {n} (scale {k}, {} sides{}, turned {:.4}): {sides:?}",
            sides.len(),
            if touch { ", a corner on the axis" } else { "" },
            angle
        );
        let run = |plane: &Frame| {
            revolve(&profiles, plane, [0.0, 0.0], [0.0, 1.0], 0.0, angle, &tol).map(|r| r.body)
        };
        let after = match guarded(|| run(&frame)) {
            Err(p) => {
                tally.outcomes.push('P');
                tally.wrong.push(format!("{what}: a panic: {p}"));
                continue;
            }
            Ok(Err(e)) => {
                tally.outcomes.push('R');
                println!("{what}: refused: {e}");
                continue;
            }
            Ok(Ok(after)) => after,
        };
        let mut problems = Vec::new();
        let mut invalid = false;
        if !check(&after, &tol).is_empty() {
            invalid = true;
            problems.push(format!("not valid: {:?}", check(&after, &tol).first()));
        } else if let Some(m) = measured(&after) {
            let (a, mx, lx, length) = pappus(&sides);
            let (a, mx, lx, length) = (a * k * k, mx * k * k * k, lx * k * k, length * k);
            let volume = mx * angle;
            let full = (angle - TAU).abs() < 1e-12;
            let area = lx * angle + if full { 0.0 } else { 2.0 * a };
            let allowed = m.bound + 1e-9 * volume.abs() + 4.0 * tol.linear * m.area;
            if (m.volume - volume).abs() > allowed {
                problems.push(format!(
                    "Pappus: volume {} for {} (allowed {})",
                    g(m.volume),
                    g(volume),
                    g(allowed)
                ));
            }
            if (m.area - area).abs() > 1e-7 * area + 4.0 * tol.linear * length {
                problems.push(format!("Pappus: area {} for {}", g(m.area), g(area)));
            }
            let size = 40.0 * k;
            match mesh_volume(&after, 1e-3 * size) {
                Ok(v) => {
                    if (v - m.volume).abs() > 2e-3 * size * m.area {
                        problems.push(format!("the mesh's volume {} for {}", g(v), g(m.volume)));
                    }
                }
                Err(e) => problems.push(format!("passes the checker but {e}")),
            }
            let motion = far_motion(&mut rng, size);
            if let Some(plane) = motion.frame(&frame)
                && let Ok(Ok(other)) = guarded(|| run(&plane))
                && let Some(mo) = measured(&other)
            {
                let allowed = m.bound + mo.bound + 1e-9 * m.volume + 0.5 * tol.linear * m.area;
                if (mo.volume - m.volume).abs() > allowed {
                    problems.push(format!(
                        "in the far frame the volume is {} for {}",
                        g(mo.volume),
                        g(m.volume)
                    ));
                }
            }
        } else {
            problems.push("cannot be measured".into());
        }
        let c = if invalid {
            'I'
        } else if problems
            .iter()
            .any(|p| p.contains("passes the checker but"))
        {
            'M'
        } else if problems.is_empty() {
            'B'
        } else {
            'W'
        };
        tally.outcomes.push(c);
        println!("{what}: {c}");
        for p in problems {
            tally.wrong.push(format!("{what}: {c}: {p}"));
        }
        let _ = Vec3::new(0.0, 0.0, 0.0);
    }
    tally
}

/// Revolutions seen wrong, owed (none known yet).
const KNOWN_REVOLVE: [usize; 0] = [];

#[test]
fn random_spline_profiles_turned_0_to_150_hold_pappus() {
    revolutions(0..150).hold("revolve", 0, &KNOWN_REVOLVE, 40);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_revolve_0() {
    revolutions(150..900).hold("revolve", 150, &KNOWN_REVOLVE, 0);
}

#[test]
fn hunt_revolve_scratch() {
    let Some(r) = hunt_common::scratch_range() else {
        return;
    };
    let start = r.start;
    revolutions(r).hold("revolve", start, &KNOWN_REVOLVE, 0);
}
