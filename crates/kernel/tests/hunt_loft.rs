//! A hunt for wrong answers of loft against a closed form: ruled lofts
//! through two or three random convex polygons with as many corners, in
//! parallel planes, each shifted, turned (less than a corner's share of a
//! turn) and scaled at random, the first corners matched (`Section::start`).
//! Between two such sections every level is the polygon of the corners
//! interpolated, whose area is quadratic in the height, so each band's
//! volume is Simpson's rule exactly: `h / 6 (A0 + 4 Am + A1)`, `Am` the
//! area of the polygon of the corners' midpoints. Held to it, to the
//! checker, to the volume of the closed mesh and to the same loft in a
//! frame moved some 125 times its size away and turned. A loft that
//! passes through itself (the tessellator's self-check) is counted apart:
//! the regeneration refuses it.

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
use craft3d_kernel::loft::{Options, Section, loft};
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Portable, Tolerance};
use craft3d_tess::{SelfCheckParams, body_self_intersections};
use craft3d_topo::check;
use hunt_common::{Tally, case_seed, far_motion, g, guarded, measured, mesh_volume, tolerance};
use std::f64::consts::TAU;

/// The area of the polygon `points` (counter-clockwise positive).
fn area(points: &[[f64; 2]]) -> f64 {
    0.5 * (0..points.len())
        .map(|i| {
            let (p, q) = (points[i], points[(i + 1) % points.len()]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum::<f64>()
}

/// A section of the corners `points` (times `k`) in `plane`, starting at
/// its first side.
fn section(
    points: &[[f64; 2]],
    plane: Frame,
    first: u64,
    k: f64,
    tol: &Tolerance,
) -> Option<Section> {
    let curves: Vec<SketchCurve> = (0..points.len())
        .map(|i| {
            let (p, q) = (points[i], points[(i + 1) % points.len()]);
            SketchCurve {
                id: first + i as u64,
                curve: Curve2::Line {
                    start: [p[0] * k, p[1] * k],
                    end: [q[0] * k, q[1] * k],
                },
            }
        })
        .collect();
    let profiles = detect_profiles(&curves, tol).ok()?;
    Some(Section {
        plane,
        outline: profiles.first()?.outer.clone(),
        start: Some(first),
    })
}

const SEED_LOFT: u64 = 0x0105_7000_2026_1114;

fn lofts(cases: std::ops::Range<usize>) -> Tally {
    let mut tally = Tally::new();
    for n in cases {
        let mut rng = Rng(case_seed(SEED_LOFT, n));
        let k = hunt_common::SCALES[rng.below(4) as usize];
        let tol = tolerance(k);
        let corners = 3 + rng.below(4) as usize;
        // A convex polygon: corners at increasing angles on a circle,
        // their radii a little apart.
        let base: Vec<[f64; 2]> = (0..corners)
            .map(|i| {
                let a = TAU * (i as f64 + rng.between(-0.2, 0.2)) / corners as f64;
                let r = rng.between(8.0, 12.0);
                [r * a.pcos(), r * a.psin()]
            })
            .collect();
        let count = 2 + rng.below(2) as usize;
        let mut levels = vec![0.0];
        let mut polygons = vec![base.clone()];
        for _ in 1..count {
            let turn = rng.between(-0.4, 0.4) * TAU / corners as f64;
            let scale = rng.between(0.4, 1.4);
            let shift = [rng.between(-6.0, 6.0), rng.between(-6.0, 6.0)];
            let (s, c) = (turn.psin(), turn.pcos());
            polygons.push(
                base.iter()
                    .map(|p| {
                        [
                            scale * (c * p[0] - s * p[1]) + shift[0],
                            scale * (s * p[0] + c * p[1]) + shift[1],
                        ]
                    })
                    .collect(),
            );
            levels.push(levels.last().unwrap() + rng.between(5.0, 30.0));
        }
        let frame = hunt_common::frame(&mut rng, k);
        let what =
            format!("case {n} (scale {k}): {count} sections of {corners} corners at {levels:?}");
        // Simpson per band.
        let expected: f64 = (1..count)
            .map(|b| {
                let (p, q) = (&polygons[b - 1], &polygons[b]);
                let mid: Vec<[f64; 2]> = p
                    .iter()
                    .zip(q)
                    .map(|(a, c)| [0.5 * (a[0] + c[0]), 0.5 * (a[1] + c[1])])
                    .collect();
                (levels[b] - levels[b - 1]) / 6.0 * (area(p) + 4.0 * area(&mid) + area(q))
            })
            .sum::<f64>()
            * k
            * k
            * k;
        let run = |plane: &Frame| -> Result<craft3d_topo::Body, String> {
            let sections: Option<Vec<Section>> = polygons
                .iter()
                .zip(&levels)
                .enumerate()
                .map(|(i, (p, z))| {
                    section(
                        p,
                        plane.at(plane.point(0.0, 0.0, z * k))?,
                        100 * (i as u64 + 1),
                        k,
                        &tol,
                    )
                })
                .collect();
            let sections = sections.ok_or("no sections")?;
            loft(
                &sections,
                &Options {
                    ruled: true,
                    closed: false,
                },
                &tol,
            )
            .map(|r| r.body)
            .map_err(|e| e.to_string())
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
            let allowed = m.bound + 1e-9 * expected.abs() + 2.0 * tol.linear * m.area;
            if (m.volume - expected).abs() > allowed {
                problems.push(format!(
                    "Simpson: volume {} for {} (allowed {})",
                    g(m.volume),
                    g(expected),
                    g(allowed)
                ));
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
                if (mo.volume - m.volume).abs() > allowed
                    || (mo.area - m.area).abs() > 1e-7 * m.area
                {
                    problems.push(format!(
                        "in the far frame volume {} and area {} for {} and {}",
                        g(mo.volume),
                        g(mo.area),
                        g(m.volume),
                        g(m.area)
                    ));
                }
            }
        } else {
            problems.push("cannot be measured".into());
        }
        let crosses = body_self_intersections(&after, &SelfCheckParams::new(tol.linear))
            .is_ok_and(|c| !c.is_clean());
        let c = if invalid {
            'I'
        } else if problems
            .iter()
            .any(|p| p.contains("passes the checker but"))
        {
            'M'
        } else if problems.is_empty() {
            'B'
        } else if crosses {
            'X'
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

/// Lofts seen wrong, owed (none known yet).
const KNOWN_LOFT: [usize; 0] = [];

#[test]
fn random_ruled_lofts_0_to_150_hold_simpson() {
    lofts(0..150).hold("loft", 0, &KNOWN_LOFT, 50);
}

#[test]
#[ignore = "the wide hunt, run by hand"]
fn wide_loft_0() {
    lofts(150..1150).hold("loft", 150, &KNOWN_LOFT, 0);
}

#[test]
fn hunt_loft_scratch() {
    let Some(r) = hunt_common::scratch_range() else {
        return;
    };
    let start = r.start;
    lofts(r).hold("loft", start, &KNOWN_LOFT, 0);
}
