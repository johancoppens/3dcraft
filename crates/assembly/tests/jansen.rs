//! Jansen's linkage (the Strandbeest leg): the foot point over a crank
//! turn against circle intersections computed independently, as the
//! strandbeest-leg Jig's generator does, at three scales; and twelve legs
//! on one crankshaft per frame as a benchmark.

mod common;

use common::*;
use craft3d_assembly::{Problem, Solver};
use craft3d_math::Vec3;
use std::f64::consts::PI;

/// `legs` legs on one crankshaft, phases `k · 2π / legs`, leg k in the
/// plane z = 10 k (scaled). Instance 0 ground, 1 the crankshaft; mate 0
/// turns the crankshaft and is driven.
fn walker(legs: usize, s: f64) -> (Problem, Vec<(usize, Vec3, f64, f64)>) {
    let mut p = planar(2 + 6 * legs, s, 100.0);
    pin(&mut p, 0, 1, Vec3::ZERO, "crankshaft");
    p.mates[0].driven[0] = Some(0.0);
    let mut feet = Vec::new();
    for leg in 0..legs {
        let phase = 2.0 * PI * leg as f64 / legs as f64;
        let z = 10.0 * s * leg as f64;
        let (foot, g) = add_leg(&mut p, 1, 2 + 6 * leg, phase, s, z);
        feet.push((foot, g, phase, z));
    }
    disturb(&mut p, 0.01);
    (p, feet)
}

#[test]
fn the_foot_follows_the_circle_intersections_over_a_turn_at_three_scales() {
    for s in SCALES {
        let (p, feet) = walker(1, s);
        let tol = 1e-6 * p.scale;
        let mut solver = Solver::new(&p).unwrap();
        let first = solver.solve().unwrap();
        assert_eq!(first.dof, 0, "a driven leg has no DOF left");
        // 7 moving bodies (42) and 10 angles; 10 revolutes (60 equations)
        // and the driver; planar in 3D, so the rank is 52.
        assert_eq!(first.rank, 52);
        for k in 1..=72 {
            let th = 2.0 * PI * k as f64 / 72.0;
            let sol = solver.advance(&[th]).unwrap();
            for &(foot, g, phase, z) in &feet {
                let want = jansen_joints(th + phase, s, z).unwrap()[7];
                let got = sol.poses[foot].point(g);
                assert!(
                    close(got, want, tol),
                    "foot at {th}: {got:?} vs {want:?} (scale {s})"
                );
            }
        }
    }
}

#[test]
fn two_legs_on_one_crankshaft_stay_in_phase() {
    let (p, feet) = walker(2, 1.0);
    let mut solver = Solver::new(&p).unwrap();
    solver.solve().unwrap();
    for k in 1..=24 {
        let th = 2.0 * PI * k as f64 / 24.0;
        let sol = solver.advance(&[th]).unwrap();
        for &(foot, g, phase, z) in &feet {
            let want = jansen_joints(th + phase, 1.0, z).unwrap()[7];
            assert!(close(sol.poses[foot].point(g), want, 1e-4));
        }
    }
}

/// Twelve legs on one crankshaft, one degree per frame for a turn: every
/// foot checked, the time per frame printed. Run in release:
/// `cargo test -p craft3d-assembly --release --test jansen -- --ignored --nocapture`.
#[test]
#[ignore]
fn bench_twelve_legs_per_frame() {
    let (p, feet) = walker(12, 1.0);
    let mut solver = Solver::new(&p).unwrap();
    solver.solve().unwrap();
    let mut times = Vec::new();
    for k in 1..=360 {
        let th = (k as f64).to_radians();
        let t0 = std::time::Instant::now();
        let sol = solver.advance(&[th]).unwrap();
        times.push(t0.elapsed().as_secs_f64() * 1e3);
        for &(foot, g, phase, z) in &feet {
            let want = jansen_joints(th + phase, 1.0, z).unwrap()[7];
            assert!(close(sol.poses[foot].point(g), want, 1e-4));
        }
    }
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let max = times.iter().cloned().fold(0.0, f64::max);
    println!(
        "bench: 12 Jansen legs ({} instances, {} mates), 360 frames: mean {mean:.3} ms, max {max:.3} ms per frame",
        p.instances,
        p.mates.len()
    );
    if !cfg!(debug_assertions) {
        assert!(mean < 16.0, "mean {mean} ms per frame");
    }
}
