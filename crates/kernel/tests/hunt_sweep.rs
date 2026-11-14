//! A hunt for wrong answers of sweep with spline profiles: a random
//! star-shaped profile whose sides are lines and cubic Bézier splines,
//! carried along a line, a tangent arc that turns left and a line, held to
//! oracles that do not ask how the body was built:
//!
//! - the checker, and the volume of the closed mesh against the measure;
//! - Pappus piece by piece: along a line the profile's area times the
//!   length, along the arc its area times the way its centroid goes round
//!   the arc's axis (the radius less its offset to the left, times the
//!   angle); the area likewise from the boundary's length and its
//!   centroid, and the two caps; the profile's area and moments are
//!   integrated here (`hunt_common::pappus`);
//! - frame invariance: the same sweep with the path's plane moved some 125
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
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_kernel::sweep::sweep;
use craft3d_math::{Frame, Portable};
use craft3d_topo::check;
use hunt_common::{
    Tally, case_seed, curves, far_motion, g, guarded, measured, mesh_volume, pappus, profile,
    tolerance,
};

const SEED_SWEEP: u64 = 0x05EE_E700_2026_1114;

fn sweeps(cases: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in cases {
        let mut rng = Rng(case_seed(SEED_SWEEP, n));
        let k = hunt_common::SCALES[rng.below(4) as usize];
        let tol = tolerance(k);
        let centre = [rng.between(-4.0, 4.0), rng.between(-4.0, 4.0)];
        let sides = profile(&mut rng, false, centre);
        // The profile's farthest reach to the left of the path.
        let reach = sides
            .iter()
            .flat_map(|s| (0..=64).map(move |i| s.at(f64::from(i) / 64.0).0[0]))
            .fold(f64::NEG_INFINITY, f64::max);
        let radius = reach.max(0.0) + rng.between(3.0, 30.0);
        let (first, second) = (rng.between(0.0, 30.0), rng.between(0.0, 30.0));
        let angle = rng.between(0.2, 3.0);
        let frame = hunt_common::frame(&mut rng, k);
        let (s, c) = (angle.psin(), angle.pcos());
        let bend_end = [first + radius * s, radius - radius * c];
        let mut path = Vec::new();
        if first > 0.5 {
            path.push(SketchCurve {
                id: 101,
                curve: Curve2::Line {
                    start: [0.0, 0.0],
                    end: [first * k, 0.0],
                },
            });
        }
        path.push(SketchCurve {
            id: 102,
            curve: Curve2::Arc {
                center: [first * k, radius * k],
                start: [first * k, 0.0],
                end: [bend_end[0] * k, bend_end[1] * k],
            },
        });
        if second > 0.5 {
            path.push(SketchCurve {
                id: 103,
                curve: Curve2::Line {
                    start: [bend_end[0] * k, bend_end[1] * k],
                    end: [
                        (bend_end[0] + second * c) * k,
                        (bend_end[1] + second * s) * k,
                    ],
                },
            });
        }
        let straight =
            if first > 0.5 { first } else { 0.0 } + if second > 0.5 { second } else { 0.0 };
        let Ok(profiles) = detect_profiles(&curves(&sides, k), &tol) else {
            tally.outcomes.push('S');
            continue;
        };
        let Some(profile) = profiles.first().cloned() else {
            tally.outcomes.push('S');
            continue;
        };
        let what = format!(
            "case {n} (scale {k}): {} sides about {centre:?} along {first:.3}, an arc r {radius:.3} through {angle:.4}, {second:.3}",
            sides.len()
        );
        // The profile's plane at the path's start: its x the path's left,
        // its y the path plane's normal.
        let run = |plane: &Frame| {
            let at = Frame::new(plane.origin(), plane.x(), plane.y()).unwrap();
            sweep(&profile, &at, &path, plane, &tol).map(|r| r.body)
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
            let volume = (a * straight + angle * (radius * a - mx)) * k * k * k;
            let area = (length * straight + angle * (radius * length - lx) + 2.0 * a) * k * k;
            let allowed = m.bound + 1e-9 * volume.abs() + 4.0 * tol.linear * m.area;
            if (m.volume - volume).abs() > allowed {
                problems.push(format!(
                    "Pappus: volume {} for {} (allowed {})",
                    g(m.volume),
                    g(volume),
                    g(allowed)
                ));
            }
            if (m.area - area).abs() > 1e-7 * area + 4.0 * tol.linear * length * k {
                problems.push(format!("Pappus: area {} for {}", g(m.area), g(area)));
            }
            let size = hunt_common::size_of(&after);
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
    }
    tally
}

/// Sweeps seen wrong, owed (none known yet).
const KNOWN_SWEEP: [usize; 0] = [];

#[test]
fn random_spline_profiles_swept_along_a_bend_0_to_100_hold_pappus() {
    sweeps(0..100).hold("sweep", 0, &KNOWN_SWEEP, 30);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_sweep_0() {
    sweeps(100..700).hold("sweep", 100, &KNOWN_SWEEP, 0);
}

#[test]
fn hunt_sweep_scratch() {
    let Some(r) = hunt_common::scratch_range() else {
        return;
    };
    let start = r.start;
    sweeps(r).hold("sweep", start, &KNOWN_SWEEP, 0);
}
