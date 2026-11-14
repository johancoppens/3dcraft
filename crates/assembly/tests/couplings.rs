//! Gear, rack and screw couplings against their closed forms at three
//! scales, and a planetary stage as a linear coupling.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use craft3d_assembly::{
    CouplingKind, CouplingSpec, Frame, MateKind, MateSpec, Problem, solve, solve_sequence,
};
use craft3d_math::Vec3;
use std::f64::consts::PI;

fn coupling(kind: CouplingKind, a: usize, b: usize, value: f64) -> CouplingSpec {
    CouplingSpec {
        kind,
        mates: (a, b),
        value,
    }
}

/// The angle a pose turned about z, unwrapped near `near`.
fn turned(p: &craft3d_assembly::Pose, near: f64) -> f64 {
    let t = turn(p);
    t + 2.0 * PI * ((near - t) / (2.0 * PI)).round()
}

#[test]
fn a_gear_pair_turns_at_its_ratio_at_three_scales() {
    for s in SCALES {
        let (ra, rb) = (2.0 * s, 3.0 * s);
        let mut p = planar(3, s, 5.0);
        pin(&mut p, 0, 1, Vec3::ZERO, "gear a");
        pin(&mut p, 0, 2, v2(ra + rb, 0.0), "gear b");
        p.couplings
            .push(coupling(CouplingKind::Gear, 0, 1, rb / ra));
        p.mates[1].driven[0] = Some(0.0);
        let frames: Vec<Vec<f64>> = (1..=40).map(|k| vec![0.25 * k as f64]).collect();
        let seq = solve_sequence(&p, &frames).unwrap();
        assert!(seq.stopped.is_none(), "{:?}", seq.stopped);
        for sol in &seq.frames {
            let tb = sol.drives[0];
            let ta = -(rb / ra) * tb;
            assert!(
                (sol.values[0][0] - ta).abs() < 1e-8,
                "{} vs {ta}",
                sol.values[0][0]
            );
            assert!((turned(&sol.poses[1], ta) - ta).abs() < 1e-8);
            assert!((turned(&sol.poses[2], tb) - tb).abs() < 1e-8);
            // The pitch points roll without slip.
            assert!((ra * ta + rb * tb).abs() < 1e-8 * s);
            assert_eq!(sol.dof, 0);
        }
    }
}

#[test]
fn a_rack_moves_by_the_pinion_radius_per_radian_at_three_scales() {
    for s in SCALES {
        let r = 1.5 * s;
        let mut p = planar(3, s, 5.0);
        pin(&mut p, 0, 1, Vec3::ZERO, "pinion");
        let f = Frame::from_z_x(v2(0.0, -r), Vec3::X, Vec3::Y).unwrap();
        p.mates
            .push(MateSpec::new(MateKind::Slider, (0, f), (2, f)).named("rack"));
        p.couplings.push(coupling(CouplingKind::Rack, 1, 0, r));
        p.mates[0].driven[0] = Some(0.0);
        let frames: Vec<Vec<f64>> = (1..=20).map(|k| vec![-0.3 * k as f64]).collect();
        let seq = solve_sequence(&p, &frames).unwrap();
        assert!(seq.stopped.is_none(), "{:?}", seq.stopped);
        for sol in &seq.frames {
            let d = r * sol.drives[0];
            assert!((sol.poses[2].translation.x - d).abs() < 1e-8 * s);
            assert!(sol.poses[2].translation.y.abs() < 1e-8 * s);
            assert!((sol.values[1][0] - d).abs() < 1e-8 * s);
        }
    }
}

#[test]
fn a_screw_advances_its_lead_per_turn_at_three_scales() {
    for s in SCALES {
        let lead = 0.8 * s;
        let mut p = Problem::new(2, 1e-8 * s, 10.0 * s);
        p.grounded = vec![0];
        p.mates.push(
            MateSpec::new(
                MateKind::Cylindrical,
                (0, Frame::IDENTITY),
                (1, Frame::IDENTITY),
            )
            .named("screw"),
        );
        // One cylindrical mate's distance tied to its own angle.
        p.couplings.push(coupling(CouplingKind::Screw, 0, 0, lead));
        p.mates[0].driven[0] = Some(0.0);
        let frames: Vec<Vec<f64>> = (1..=24).map(|k| vec![PI / 4.0 * k as f64]).collect();
        let seq = solve_sequence(&p, &frames).unwrap();
        assert!(seq.stopped.is_none(), "{:?}", seq.stopped);
        for sol in &seq.frames {
            let th = sol.drives[0];
            let z = lead * th / (2.0 * PI);
            assert!((sol.poses[1].translation.z - z).abs() < 1e-8 * s, "{th}");
            assert!((turned(&sol.poses[1], th) - th).abs() < 1e-8);
            assert_eq!(sol.dof, 0);
        }
        // Three full turns: three leads.
        let last = seq.frames.last().unwrap();
        assert!((last.poses[1].translation.z - 3.0 * lead).abs() < 1e-8 * s);
    }
}

#[test]
fn a_planetary_stage_turns_at_its_closed_form_ratio() {
    // Sun (radius 2) and ring (radius 6) on the ground axis, the carrier on
    // it too; one planet (radius 2) on the carrier. With the ring held,
    // Willis: (ωs − ωc) / (ωr − ωc) = −R / S, so ωc = ωs S / (S + R).
    let (sun, ring, planet) = (2.0, 6.0, 2.0);
    let mut p = planar(4, 1.0, 6.0);
    pin(&mut p, 0, 1, Vec3::ZERO, "sun"); // mate 0: sun in ground
    pin(&mut p, 0, 2, Vec3::ZERO, "carrier"); // mate 1: carrier in ground
    pin(&mut p, 2, 3, v2(sun + planet, 0.0), "planet"); // mate 2: planet in carrier
    // Planet against carrier meshing the sun: P(θs − θc) = −S... as angles
    // relative to the carrier: S (θs − θc) = −P φ, and with the ring:
    // R (0 − θc) = P φ. Here φ is the planet's turn in the carrier.
    p.couplings.push(coupling(
        CouplingKind::Linear(vec![(0, 0, sun), (1, 0, -sun), (2, 0, planet)]),
        0,
        0,
        0.0,
    ));
    p.couplings.push(coupling(
        CouplingKind::Linear(vec![(1, 0, -ring), (2, 0, -planet)]),
        0,
        0,
        0.0,
    ));
    p.mates[0].driven[0] = Some(0.0);
    let frames: Vec<Vec<f64>> = (1..=10).map(|k| vec![0.4 * k as f64]).collect();
    let seq = solve_sequence(&p, &frames).unwrap();
    assert!(seq.stopped.is_none(), "{:?}", seq.stopped);
    for sol in &seq.frames {
        let ts = sol.drives[0];
        let tc = ts * sun / (sun + ring);
        assert!(
            (sol.values[1][0] - tc).abs() < 1e-9,
            "{} vs {tc}",
            sol.values[1][0]
        );
    }
    assert_eq!(seq.frames[0].dof, 0);
}

#[test]
fn a_coupling_without_its_variable_is_refused() {
    let mut p = planar(2, 1.0, 1.0);
    p.mates.push(MateSpec::new(
        MateKind::Ball,
        (0, Frame::IDENTITY),
        (1, Frame::IDENTITY),
    ));
    pin(&mut p, 0, 1, Vec3::ZERO, "pin");
    p.couplings.push(coupling(CouplingKind::Gear, 0, 1, 1.0));
    assert!(matches!(
        solve(&p),
        Err(craft3d_assembly::Diagnosis::Invalid { .. })
    ));
    p.couplings[0] = coupling(CouplingKind::Rack, 1, 1, 1.0);
    assert!(matches!(
        solve(&p),
        Err(craft3d_assembly::Diagnosis::Invalid { .. })
    ));
    p.couplings[0] = coupling(CouplingKind::Gear, 1, 7, 1.0);
    assert!(matches!(
        solve(&p),
        Err(craft3d_assembly::Diagnosis::Invalid { .. })
    ));
}

#[test]
fn a_large_gear_ratio_advances_in_one_step_without_a_false_dead_point() {
    // Review of 9752ce08: with the predictor capped at half a radian, a
    // gear train of ratio 1000 driven 2 rad in one advance ran out of
    // sub-steps and claimed a dead point.
    for s in SCALES {
        let (ra, rb) = (0.005 * s, 5.0 * s);
        let mut p = planar(3, s, 5.0);
        pin(&mut p, 0, 1, Vec3::ZERO, "pinion");
        pin(&mut p, 0, 2, v2(ra + rb, 0.0), "wheel");
        p.couplings
            .push(coupling(CouplingKind::Gear, 0, 1, rb / ra));
        p.mates[1].driven[0] = Some(0.0);
        let mut solver = craft3d_assembly::Solver::new(&p).unwrap();
        solver.solve().unwrap();
        let sol = solver.advance(&[2.0]).unwrap();
        assert!(
            (sol.values[0][0] + 2000.0).abs() < 1e-5,
            "{}",
            sol.values[0][0]
        );
    }
}

#[test]
fn a_small_pinion_turns_many_times_for_one_rack_stroke() {
    // The same with a pinion of radius L / 1000 driven by its rack.
    for s in SCALES {
        let r = 0.005 * s;
        let mut p = planar(3, s, 5.0);
        pin(&mut p, 0, 1, Vec3::ZERO, "pinion");
        let f = Frame::from_z_x(v2(0.0, -r), Vec3::X, Vec3::Y).unwrap();
        p.mates
            .push(MateSpec::new(MateKind::Slider, (0, f), (2, f)).named("rack"));
        p.couplings.push(coupling(CouplingKind::Rack, 1, 0, r));
        p.mates[1].driven[0] = Some(0.0);
        let mut solver = craft3d_assembly::Solver::new(&p).unwrap();
        solver.solve().unwrap();
        let sol = solver.advance(&[2.5 * s]).unwrap();
        assert!(
            (sol.values[0][0] - 2.5 * s / r).abs() < 1e-5,
            "{}",
            sol.values[0][0]
        );
    }
}

/// A-030's machine: two lead screws (lead 8) turn on the ground at
/// x = ∓100; their nuts, on them by cylindrical mates, are fixed to one bed
/// that slides along z. Screw A's angle is driven to `theta`, screw B is
/// turned by nothing but its screw coupling. Put together at height 50 with
/// every joint at 0. Instances: 0 ground, 1 bed, 2 and 3 the screws, 4 and
/// 5 the nuts; mates 0 the bed's slider, 1 and 2 the screws, 3 and 4 the
/// nuts on them, 5 and 6 the nuts on the bed.
fn lead_screws(s: f64, theta: f64) -> Problem {
    let mut p = planar(6, s, 300.0);
    let at = |x: f64, z: f64| Frame::at(Vec3::new(x, 0.0, z) * s);
    let mate =
        |kind, a: (usize, Frame), b: (usize, Frame), name| MateSpec::new(kind, a, b).named(name);
    p.mates = vec![
        mate(
            MateKind::Slider,
            (0, at(-100.0, 50.0)),
            (1, at(-100.0, 50.0)),
            "bed",
        ),
        mate(
            MateKind::Revolute,
            (0, at(-100.0, 0.0)),
            (2, at(-100.0, 0.0)),
            "screw A",
        ),
        mate(
            MateKind::Revolute,
            (0, at(100.0, 0.0)),
            (3, at(100.0, 0.0)),
            "screw B",
        ),
        mate(
            MateKind::Cylindrical,
            (2, at(-100.0, 50.0)),
            (4, at(-100.0, 50.0)),
            "nut A",
        ),
        mate(
            MateKind::Cylindrical,
            (3, at(100.0, 50.0)),
            (5, at(100.0, 50.0)),
            "nut B",
        ),
        mate(
            MateKind::Fixed,
            (1, at(-100.0, 50.0)),
            (4, at(-100.0, 50.0)),
            "nut A on bed",
        ),
        mate(
            MateKind::Fixed,
            (1, at(100.0, 50.0)),
            (5, at(100.0, 50.0)),
            "nut B on bed",
        ),
    ];
    p.couplings = vec![
        coupling(CouplingKind::Screw, 3, 1, 8.0 * s),
        coupling(CouplingKind::Screw, 4, 2, 8.0 * s),
    ];
    p.mates[1].driven[0] = Some(theta);
    p
}

/// Screw B turns as A does and the bed rises a lead a turn: θ_B = θ and
/// the bed at 8 θ / 2π, solved at rest from where it was put together, at
/// 3 to 40 turns and three scales. Before, B started at 0 (its pose says
/// nothing of turns) and the solve walked it half a radian an iteration:
/// from about 16 turns on it did not converge in 200 iterations.
#[test]
fn an_undriven_screw_follows_its_coupling_at_rest_over_many_turns() {
    for s in SCALES {
        for turns in [3.0, 15.9, 16.8, 40.0] {
            let theta = -2.0 * PI * turns;
            let p = lead_screws(s, theta);
            let sol = solve(&p).unwrap_or_else(|e| panic!("{turns} turns: {}", e.message(&p)));
            let rise = 8.0 * s * theta / (2.0 * PI);
            assert!(
                (sol.values[2][0] - theta).abs() < 1e-6,
                "{turns} turns, scale {s}: B at {}",
                sol.values[2][0]
            );
            assert!(
                (sol.values[0][0] - rise).abs() < 1e-6 * p.scale,
                "{turns} turns, scale {s}"
            );
            assert!((sol.poses[1].translation.z - rise).abs() < 1e-6 * p.scale);
            assert_eq!(sol.dof, 0);
            // The nuts' turn on their screws, which no equation counts the
            // turns of, reads in the turns nearest where it was placed (0),
            // as a solve from the placement reads it: within half a turn.
            for nut in [3, 4] {
                let a = sol.values[nut][0];
                assert!(a.abs() <= PI, "{turns} turns: nut {nut} at {a}");
                let want = -theta - 2.0 * PI * (-theta / (2.0 * PI)).round();
                assert!(
                    (a - want).abs() < 1e-6,
                    "{turns} turns: nut {nut} at {a}, not {want}"
                );
            }
        }
    }
}

/// A screw placed with its nut where many turns put it, as a jack is
/// saved: the screw starts at the turns its coupling says (27), not 0, so
/// nothing is turned round and every joint reads as placed.
#[test]
fn a_screw_placed_many_turns_round_starts_at_its_couplings_turns() {
    let turns = 27.0;
    let theta = -2.0 * PI * turns;
    let lead = 8.0;
    let mut p = lead_screws(1.0, theta);
    // The bed and nuts placed where 27 turns put them: one lead a turn.
    let rise = Vec3::new(0.0, 0.0, lead * theta / (2.0 * PI));
    for i in [1, 4, 5] {
        p.start[i] = craft3d_assembly::Pose::new(craft3d_assembly::Mat3::IDENTITY, rise);
    }
    let sol = solve(&p).unwrap();
    assert!(
        (sol.values[2][0] - theta).abs() < 1e-6,
        "B at {}",
        sol.values[2][0]
    );
    // Nothing turned: the nuts read 0 on their screws, the bed is where
    // it was placed.
    for nut in [3, 4] {
        assert!(sol.values[nut][0].abs() < 1e-6, "{}", sol.values[nut][0]);
    }
    assert!((sol.poses[1].translation - rise).length() < 1e-6);
}

/// The same moment by moment, as a motion makes them: each moment's
/// problem starts where the one before left the parts, carried with the
/// turns its couplings count (`solve_at_near_coupled`), a third of a turn
/// a moment for 40 turns. Screw B follows at every moment; before, a
/// moment started B in (−π, π] and stopped converging after 15.7 turns.
#[test]
fn an_undriven_screw_follows_its_coupling_moment_by_moment() {
    for s in SCALES {
        let mut p = lead_screws(s, 0.0);
        let mut last = solve(&p).unwrap();
        for k in 1..=120 {
            let theta = -2.0 * PI * k as f64 / 3.0;
            p.start = last.poses.clone();
            let mut solver = craft3d_assembly::Solver::new(&p).unwrap();
            solver
                .solve_at_near_coupled(&last.drives, &last.values)
                .unwrap();
            let sol = solver
                .advance(&[theta])
                .unwrap_or_else(|e| panic!("moment {k}: {}", e.message(&p)));
            let rise = 8.0 * s * theta / (2.0 * PI);
            assert!(
                (sol.values[2][0] - theta).abs() < 1e-6,
                "moment {k}, scale {s}"
            );
            assert!(
                (sol.values[0][0] - rise).abs() < 1e-6 * p.scale,
                "moment {k}"
            );
            last = sol;
        }
    }
}
