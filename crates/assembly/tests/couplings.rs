//! Gear, rack and screw couplings against their closed forms at three
//! scales, and a planetary stage as a linear coupling.

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
