//! Four-bar linkages (the Grashof classes, the coupler curve, a branch kept
//! over a full turn, dead points) and a slider-crank, against closed forms
//! at three scales.

mod common;

use common::*;
use craft3d_assembly::{
    Diagnosis, Frame, MateKind, MateSpec, Problem, Sequence, Solver, solve, solve_sequence,
};
use craft3d_math::Vec3;
use std::f64::consts::PI;

/// A four-bar: ground pivots O = (0, 0) and D = (d, 0), crank a, coupler b,
/// rocker c, assembled at crank angle `th0` on branch `sg`. Instances:
/// 0 ground, 1 crank, 2 coupler, 3 rocker; mate 0 (the crank) is driven.
struct FourBar {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    th0: f64,
    sg: f64,
    s: f64,
}

impl FourBar {
    fn new(a: f64, b: f64, c: f64, d: f64, th0_deg: f64, s: f64) -> FourBar {
        FourBar {
            a: a * s,
            b: b * s,
            c: c * s,
            d: d * s,
            th0: th0_deg.to_radians(),
            sg: 1.0,
            s,
        }
    }

    fn pivot_d(&self) -> Vec3 {
        v2(self.d, 0.0)
    }

    fn joints(&self, th: f64) -> Option<(Vec3, Vec3)> {
        let a = v2(self.a * th.cos(), self.a * th.sin());
        let b = circ(a, self.b, self.pivot_d(), self.c, self.sg)?;
        Some((a, b))
    }

    /// The coupler point: half way along the coupler and 0.4 b to its left.
    fn coupler_point(&self, th: f64) -> Option<Vec3> {
        let (a, b) = self.joints(th)?;
        let u = (b - a) * (1.0 / self.b);
        let n = Vec3::new(-u.y, u.x, 0.0);
        Some(a + u * (0.5 * self.b) + n * (0.4 * self.b))
    }

    fn problem(&self) -> Problem {
        let longest = self.a.max(self.b).max(self.c).max(self.d) / self.s;
        let mut p = planar(4, self.s, longest);
        let (a0, b0) = self.joints(self.th0).unwrap();
        pin(&mut p, 0, 1, Vec3::ZERO, "crank");
        pin(&mut p, 1, 2, a0, "A");
        pin(&mut p, 2, 3, b0, "B");
        pin(&mut p, 3, 0, self.pivot_d(), "D");
        p.mates[0].driven[0] = Some(0.0);
        p.names = ["ground", "crank", "coupler", "rocker"]
            .map(String::from)
            .to_vec();
        disturb(&mut p, 0.01);
        p
    }
}

fn turn_frames(n: usize, turns: f64) -> Vec<Vec<f64>> {
    (1..=n)
        .map(|k| vec![turns * 2.0 * PI * k as f64 / n as f64])
        .collect()
}

fn check_turn(fb: &FourBar, frames: usize) {
    let p = fb.problem();
    let tol = 1e-6 * p.scale;
    let seq = solve_sequence(&p, &turn_frames(frames, 1.0)).unwrap();
    assert!(seq.stopped.is_none(), "stopped: {:?}", seq.stopped);
    assert_eq!(seq.frames.len(), frames);
    assert!(seq.dead_points.is_empty());
    let (_, b0) = fb.joints(fb.th0).unwrap();
    let p0 = fb.coupler_point(fb.th0).unwrap();
    for sol in &seq.frames {
        let th = fb.th0 + sol.drives[0];
        let (_, b) = fb.joints(th).unwrap();
        let got_b = sol.poses[3].point(b0);
        assert!(close(got_b, b, tol), "B at {th}: {got_b:?} vs {b:?}");
        let got_p = sol.poses[2].point(p0);
        let want_p = fb.coupler_point(th).unwrap();
        assert!(close(got_p, want_p, tol), "coupler point at {th}");
        assert_eq!(sol.dof, 0, "a driven four-bar has no DOF left");
        // The crank's angle variable follows the drive, unwrapped.
        assert!((sol.values[0][0] - sol.drives[0]).abs() < 1e-8);
    }
    // After a full turn the linkage is back where it started.
    let last = seq.frames.last().unwrap();
    assert!(close(last.poses[3].point(b0), b0, tol));
}

#[test]
fn a_crank_rocker_turns_fully_on_one_branch_at_three_scales() {
    for s in SCALES {
        // Grashof, shortest link the crank: 1 + 4 ≤ 3 + 3.5.
        check_turn(&FourBar::new(1.0, 4.0, 3.0, 3.5, 60.0, s), 72);
    }
}

#[test]
fn a_double_crank_turns_fully_at_three_scales() {
    for s in SCALES {
        // Grashof, shortest link the ground: 1 + 4 ≤ 3 + 3.5.
        check_turn(&FourBar::new(3.0, 3.5, 4.0, 1.0, 60.0, s), 72);
    }
}

#[test]
fn a_branch_is_kept_when_the_frames_are_far_apart() {
    for s in SCALES {
        // Six frames per turn: 60° steps, cut into sub-steps on the branch.
        check_turn(&FourBar::new(1.0, 4.0, 3.0, 3.5, 60.0, s), 6);
        // The other branch, assembled crossed, stays crossed.
        let mut fb = FourBar::new(1.0, 4.0, 3.0, 3.5, 60.0, s);
        fb.sg = -1.0;
        check_turn(&fb, 6);
    }
}

fn dead_point(seq: &Sequence) -> f64 {
    assert!(
        matches!(seq.stopped, Some((_, Diagnosis::DeadPoint { .. }))),
        "expected a dead point, got {:?}",
        seq.stopped
    );
    assert_eq!(seq.dead_points.len(), 1);
    seq.dead_points[0].drives[0]
}

#[test]
fn grashof_rockers_stop_at_their_closed_form_dead_points() {
    for s in SCALES {
        // Grashof double-rocker (coupler shortest: 1 + 4 ≤ 3 + 3.5) and a
        // non-Grashof triple-rocker (3 + 5 > 3 + 3): the crank stops where
        // the coupler and rocker line up, |AD| = b + c.
        for (a, b, c, d) in [(3.0, 1.0, 3.5, 4.0), (3.0, 3.0, 3.0, 5.0)] {
            let fb = FourBar::new(a, b, c, d, 60.0, s);
            let p = fb.problem();
            let frames: Vec<Vec<f64>> = (1..=30)
                .map(|k| vec![(2.0 * k as f64).to_radians()])
                .collect();
            let seq = solve_sequence(&p, &frames).unwrap();
            let want = ((a * a + d * d - (b + c) * (b + c)) / (2.0 * a * d)).acos();
            let got = fb.th0 + dead_point(&seq);
            assert!(
                (got - want).abs() < 1e-5,
                "dead point {got} vs {want} (scale {s})"
            );
            // Every frame before it matches the closed form.
            let (_, b0) = fb.joints(fb.th0).unwrap();
            for sol in &seq.frames {
                let (_, bj) = fb.joints(fb.th0 + sol.drives[0]).unwrap();
                assert!(close(sol.poses[3].point(b0), bj, 1e-6 * p.scale));
            }
        }
    }
}

#[test]
fn an_undriven_four_bar_has_one_free_motion() {
    for s in SCALES {
        let fb = FourBar::new(1.0, 4.0, 3.0, 3.5, 60.0, s);
        let mut p = fb.problem();
        p.mates[0].driven[0] = None;
        let sol = solve(&p).unwrap();
        assert_eq!(sol.dof, 1);
        assert_eq!(sol.free.len(), 1);
        // The crank turns about z through O; the coupler and rocker move
        // with it.
        let f = &sol.free[0];
        assert_eq!(f.instances.len(), 3);
        // A planar four-bar of revolutes in 3D repeats three equations.
        assert!(!sol.redundant.is_empty());
        let text = f.describe(&p);
        assert!(text.contains("crank can still turn about"), "{text}");
    }
}

/// A slider-crank: crank r about O, rod l, the slider on the x axis.
/// Instances 0 ground, 1 crank, 2 rod, 3 slider; mates 0 crank, 1 A, 2 P,
/// 3 the slider.
fn slider_crank(s: f64, th0: f64) -> (Problem, Vec3, f64, f64) {
    let (r, l) = (1.0 * s, 3.0 * s);
    let a0 = v2(r * th0.cos(), r * th0.sin());
    let x0 = r * th0.cos() + (l * l - (r * th0.sin()).powi(2)).sqrt();
    let p0 = v2(x0, 0.0);
    let mut p = planar(4, s, 4.0);
    pin(&mut p, 0, 1, Vec3::ZERO, "crank");
    pin(&mut p, 1, 2, a0, "A");
    pin(&mut p, 2, 3, p0, "P");
    let f = Frame::from_z_x(p0, Vec3::X, Vec3::Y).unwrap();
    p.mates
        .push(MateSpec::new(MateKind::Slider, (0, f), (3, f)).named("slide"));
    disturb(&mut p, 0.01);
    (p, p0, r, l)
}

#[test]
fn a_slider_crank_follows_its_closed_form_at_three_scales() {
    for s in SCALES {
        let th0 = 60f64.to_radians();
        let (mut p, p0, r, l) = slider_crank(s, th0);
        p.mates[0].driven[0] = Some(0.0);
        let seq = solve_sequence(&p, &turn_frames(48, 1.0)).unwrap();
        assert!(seq.stopped.is_none(), "{:?}", seq.stopped);
        for sol in &seq.frames {
            let th = th0 + sol.drives[0];
            let x = r * th.cos() + (l * l - (r * th.sin()).powi(2)).sqrt();
            let got = sol.poses[3].point(p0);
            assert!(
                close(got, v2(x, 0.0), 1e-6 * p.scale),
                "slider at {th}: {got:?} vs {x}"
            );
            assert!((sol.values[3][0] - (x - p0.x)).abs() < 1e-6 * p.scale);
            assert_eq!(sol.dof, 0);
        }
    }
}

#[test]
fn a_slider_crank_driven_by_its_slider_stops_at_top_dead_centre() {
    for s in SCALES {
        let th0 = 60f64.to_radians();
        let (mut p, p0, r, l) = slider_crank(s, th0);
        p.mates[3].driven[0] = Some(0.0);
        let reach = r + l - p0.x;
        let frames: Vec<Vec<f64>> = (1..=20)
            .map(|k| vec![(reach + 0.3 * s) * k as f64 / 20.0])
            .collect();
        let seq = solve_sequence(&p, &frames).unwrap();
        let got = dead_point(&seq);
        assert!(
            (got - reach).abs() < 1e-6 * p.scale,
            "dead point {got} vs {reach} (scale {s})"
        );
        // The frames before it are on the closed form.
        for sol in &seq.frames {
            let x = p0.x + sol.drives[0];
            let cos_th = (x * x + r * r - l * l) / (2.0 * x * r);
            let a = sol.poses[1].point(v2(r * th0.cos(), r * th0.sin()));
            assert!((a.x / r - cos_th).abs() < 1e-6, "crank at slider {x}");
        }
    }
}

#[test]
fn a_solver_advances_frame_by_frame() {
    let fb = FourBar::new(1.0, 4.0, 3.0, 3.5, 60.0, 1.0);
    let p = fb.problem();
    let mut solver = Solver::new(&p).unwrap();
    let first = solver.solve().unwrap();
    assert_eq!(first.drives, vec![0.0]);
    let (_, b0) = fb.joints(fb.th0).unwrap();
    for k in 1..=10 {
        let d = 0.1 * k as f64;
        let sol = solver.advance(&[d]).unwrap();
        let (_, b) = fb.joints(fb.th0 + d).unwrap();
        assert!(close(sol.poses[3].point(b0), b, 1e-6 * p.scale));
    }
    // Wrong drive counts are refused, not panicked on.
    assert!(matches!(
        solver.advance(&[]),
        Err(Diagnosis::Invalid { .. })
    ));
    assert!(matches!(
        solver.advance(&[f64::NAN]),
        Err(Diagnosis::Invalid { .. })
    ));
}
