//! Diagnosis: under-constrained (free motions named per instance),
//! redundant, over-constrained and infeasible assemblies, limits, flips and
//! malformed problems.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use craft3d_assembly::{
    CouplingKind, CouplingSpec, Diagnosis, Frame, Group, Mat3, MateKind, MateSpec, Motion, Pose,
    Problem, solve,
};
use craft3d_math::Vec3;

fn one_body(kind: MateKind, s: f64) -> Problem {
    let mut p = planar(2, s, 1.0);
    p.names = vec!["ground".into(), "Bar[3]".into()];
    let f = Frame::at(v2(0.5 * s, 0.25 * s));
    p.mates.push(MateSpec::new(kind, (0, f), (1, f)).named("m"));
    p
}

#[test]
fn free_motions_are_named_per_instance_at_three_scales() {
    for s in SCALES {
        let sol = solve(&one_body(MateKind::Revolute, s)).unwrap();
        assert_eq!(sol.dof, 1);
        let m = &sol.free[0].instances[0];
        assert_eq!(m.instance, 1);
        let Motion::Turn { axis, point, pitch } = m.motion else {
            panic!("a revolute leaves a turn: {:?}", m.motion);
        };
        assert!(axis.cross(Vec3::Z).length() < 1e-9);
        // Through the pin, on the axis.
        assert!(Vec3::new(point.x - 0.5 * s, point.y - 0.25 * s, 0.0).length() < 1e-9 * s);
        assert!(pitch.abs() < 1e-9 * s);
        let text = sol.free[0].describe(&one_body(MateKind::Revolute, s));
        assert!(text.starts_with("Bar[3] can still turn about"), "{text}");

        let sol = solve(&one_body(MateKind::Slider, s)).unwrap();
        assert_eq!(sol.dof, 1);
        let Motion::Slide { direction } = sol.free[0].instances[0].motion else {
            panic!("a slider leaves a slide");
        };
        assert!(direction.cross(Vec3::Z).length() < 1e-9);

        for (kind, dof) in [
            (MateKind::Fixed, 0),
            (MateKind::Cylindrical, 2),
            (MateKind::Planar, 3),
            (MateKind::Ball, 3),
        ] {
            let sol = solve(&one_body(kind, s)).unwrap();
            assert_eq!(sol.dof, dof, "{kind:?}");
            assert_eq!(sol.free.len(), dof);
            assert!(!sol.floating);
        }
    }
}

#[test]
fn an_unmated_or_ungrounded_assembly_is_free_as_a_whole() {
    let mut p = Problem::new(1, 1e-9, 1.0);
    let sol = solve(&p).unwrap();
    assert!(sol.floating);
    assert_eq!(sol.dof, 6);
    p.grounded = vec![0];
    let sol = solve(&p).unwrap();
    assert!(!sol.floating);
    assert_eq!(sol.dof, 0);
}

#[test]
fn a_repeated_mate_is_reported_redundant_and_holds() {
    for s in SCALES {
        let mut p = one_body(MateKind::Revolute, s);
        let again = p.mates[0].clone().named("again");
        p.mates.push(again);
        let sol = solve(&p).unwrap();
        assert_eq!(sol.dof, 1);
        assert!(
            sol.redundant
                .iter()
                .any(|r| r.groups.contains(&Group::Mate(0))
                    && r.groups.last() == Some(&Group::Mate(1))),
            "{:?}",
            sol.redundant
        );
    }
}

#[test]
fn contradicting_mates_are_over_constrained_with_their_gap() {
    for s in SCALES {
        // Bar[3]'s origin fixed to the ground's at two places 5 apart.
        let mut p = planar(2, s, 5.0);
        p.names = vec!["ground".into(), "Bar[3]".into()];
        let o = Frame::IDENTITY;
        p.mates
            .push(MateSpec::new(MateKind::Fixed, (0, o), (1, o)).named("first"));
        let g = Frame::at(v2(5.0 * s, 0.0));
        p.mates
            .push(MateSpec::new(MateKind::Fixed, (0, g), (1, o)).named("second"));
        let Err(Diagnosis::OverConstrained { groups, gap }) = solve(&p) else {
            panic!("expected over-constrained: {:?}", solve(&p));
        };
        assert_eq!(groups, vec![Group::Mate(0), Group::Mate(1)]);
        assert!((gap.length - 5.0 * s).abs() < 1e-6 * s, "gap {gap:?}");
        let text = Diagnosis::OverConstrained { groups, gap }.message(&p);
        assert!(text.contains("first") && text.contains("second"), "{text}");
    }
}

#[test]
fn a_driver_against_a_coupling_is_over_constrained() {
    let mut p = planar(3, 1.0, 5.0);
    pin(&mut p, 0, 1, Vec3::ZERO, "a");
    pin(&mut p, 0, 2, v2(5.0, 0.0), "b");
    p.couplings.push(CouplingSpec {
        kind: CouplingKind::Gear,
        mates: (0, 1),
        value: 1.5,
    });
    p.mates[0].driven[0] = Some(0.3);
    p.mates[1].driven[0] = Some(0.3);
    let Err(Diagnosis::OverConstrained { groups, gap }) = solve(&p) else {
        panic!("{:?}", solve(&p));
    };
    for g in [
        Group::Coupling(0),
        Group::Driver {
            mate: 0,
            variable: 0,
        },
        Group::Driver {
            mate: 1,
            variable: 0,
        },
    ] {
        assert!(groups.contains(&g), "{groups:?}");
    }
    // θa + 1.5 θb misses by 0.3 + 0.45 rad.
    assert!(gap.angle > 0.0);
    // Consistent drivers solve.
    p.mates[0].driven[0] = Some(-0.45);
    assert_eq!(solve(&p).unwrap().dof, 0);
}

#[test]
fn a_four_bar_too_short_to_close_is_infeasible_with_its_gap() {
    for s in SCALES {
        // Links 1, 1, 1 between pivots 4 apart: the loop misses by 1.
        let mut p = planar(4, s, 4.0);
        p.names = ["ground", "crank", "coupler", "rocker"]
            .map(String::from)
            .to_vec();
        let bar = |x: f64| Frame::at(v2(x * s, 0.0));
        let mut m = |a: usize, fa: Frame, b: usize, fb: Frame, name: &str| {
            p.mates
                .push(MateSpec::new(MateKind::Revolute, (a, fa), (b, fb)).named(name));
        };
        m(0, bar(0.0), 1, bar(0.0), "O");
        m(1, bar(1.0), 2, bar(0.0), "A");
        m(2, bar(1.0), 3, bar(0.0), "B");
        m(3, bar(1.0), 0, bar(4.0), "D");
        // Start roughly along the line, bent.
        p.start[1] = Pose::new(Mat3::rotation(Vec3::Z, 0.3), Vec3::ZERO);
        p.start[2] = Pose::new(Mat3::rotation(Vec3::Z, -0.2), v2(1.0 * s, 0.2 * s));
        p.start[3] = Pose::new(Mat3::rotation(Vec3::Z, 0.1), v2(2.2 * s, 0.1 * s));
        let Err(Diagnosis::Infeasible {
            loop_mates, gap, ..
        }) = solve(&p)
        else {
            panic!("expected infeasible: {:?}", solve(&p));
        };
        assert_eq!(loop_mates, vec![0, 1, 2, 3]);
        assert!(
            (gap.length - 1.0 * s).abs() < 1e-6 * s,
            "gap {gap:?} at scale {s}"
        );
    }
}

#[test]
fn a_limit_holds_a_free_variable_at_its_bound() {
    for s in SCALES {
        let mut p = one_body(MateKind::Revolute, s);
        p.mates[0].limits[0] = Some((0.0, 0.5));
        // Start turned past the upper limit.
        p.start[1] = Pose::new(
            Mat3::rotation(Vec3::Z, 1.0),
            v2(0.5 * s, 0.25 * s) - Mat3::rotation(Vec3::Z, 1.0).apply(v2(0.5 * s, 0.25 * s)),
        );
        let sol = solve(&p).unwrap();
        assert_eq!(sol.at_limit, vec![(0, 0)]);
        assert!((sol.values[0][0] - 0.5).abs() < 1e-8);
        assert_eq!(sol.dof, 0);
        // A driver outside the limits is refused by name.
        p.mates[0].driven[0] = Some(0.7);
        assert!(matches!(
            solve(&p),
            Err(Diagnosis::OutOfLimits { mate: 0, .. })
        ));
        p.mates[0].driven[0] = Some(0.2);
        let sol = solve(&p).unwrap();
        assert!(sol.at_limit.is_empty());
        assert!((sol.values[0][0] - 0.2).abs() < 1e-9);
    }
}

#[test]
fn a_flipped_mate_turns_the_part_over_from_the_identity() {
    // Both frames at the identity: the flip asks for a half turn, which a
    // residual of the sine of the angle would miss.
    let mut p = Problem::new(2, 1e-9, 1.0);
    p.grounded = vec![0];
    let mut m = MateSpec::new(MateKind::Fixed, (0, Frame::IDENTITY), (1, Frame::IDENTITY));
    m.flip = true;
    p.mates.push(m);
    let sol = solve(&p).unwrap();
    let r = sol.poses[1].rotation;
    assert!((r.z() + Vec3::Z).length() < 1e-9, "{r:?}");
    assert!((r.x() - Vec3::X).length() < 1e-9);
}

#[test]
fn malformed_problems_are_refused_with_a_reason() {
    let base = one_body(MateKind::Revolute, 1.0);
    let mut cases: Vec<Problem> = Vec::new();
    cases.push(Problem::new(0, 1e-9, 1.0));
    let mut p = base.clone();
    p.start.pop();
    cases.push(p);
    let mut p = base.clone();
    p.mates[0].a.0 = 9;
    cases.push(p);
    let mut p = base.clone();
    p.mates[0].b.0 = 0;
    cases.push(p);
    let mut p = base.clone();
    p.mates[0].a.1.origin.x = f64::NAN;
    cases.push(p);
    let mut p = base.clone();
    p.mates[0].a.1.axes = Mat3::from_cols(Vec3::X, Vec3::X, Vec3::ZERO);
    cases.push(p);
    let mut p = base.clone();
    p.tolerance = 0.0;
    cases.push(p);
    let mut p = base.clone();
    p.scale = f64::INFINITY;
    cases.push(p);
    let mut p = base.clone();
    p.tolerance = 1e-20;
    cases.push(p);
    let mut p = base.clone();
    p.grounded.push(5);
    cases.push(p);
    let mut p = base.clone();
    p.mates[0].driven[2] = Some(1.0);
    cases.push(p);
    let mut p = base.clone();
    p.mates[0].limits[0] = Some((1.0, 0.0));
    cases.push(p);
    let mut p = base.clone();
    p.start[1].translation.y = 1e300;
    cases.push(p);
    for (k, p) in cases.iter().enumerate() {
        match solve(p) {
            Err(Diagnosis::Invalid { reason }) => assert!(!reason.is_empty()),
            other => panic!("case {k}: {other:?}"),
        }
    }
}

#[test]
fn solving_is_deterministic() {
    let mut p = planar(4, 1.0, 4.0);
    pin(&mut p, 0, 1, Vec3::ZERO, "O");
    pin(&mut p, 1, 2, v2(0.5, 0.866), "A");
    pin(&mut p, 2, 3, v2(3.0, 2.0), "B");
    pin(&mut p, 3, 0, v2(3.5, 0.0), "D");
    disturb(&mut p, 0.02);
    let a = format!("{:?}", solve(&p));
    let b = format!("{:?}", solve(&p));
    assert_eq!(a, b);
}

#[test]
fn a_contradiction_beside_a_planar_four_bar_is_still_over_constrained() {
    // Review of 768df03d: a planar four-bar elsewhere in the assembly has
    // rank 21 of 22 at every planar state, so comparing the global rank
    // with a perturbation that leaves the plane called this Infeasible.
    for s in SCALES {
        let mut p = planar(6, s, 5.0);
        let o = Frame::IDENTITY;
        p.mates
            .push(MateSpec::new(MateKind::Fixed, (0, o), (1, o)).named("first"));
        let g = Frame::at(v2(5.0 * s, 0.0));
        p.mates
            .push(MateSpec::new(MateKind::Fixed, (0, g), (1, o)).named("second"));
        // An undriven four-bar on instances 2..5 (2 is welded to the ground).
        p.mates
            .push(MateSpec::new(MateKind::Fixed, (0, o), (2, o)).named("weld"));
        pin(&mut p, 2, 3, v2(0.0, 0.0), "O");
        pin(&mut p, 3, 4, v2(0.5 * s, 0.866 * s), "A");
        pin(&mut p, 4, 5, v2(3.0 * s, 2.0 * s), "B");
        pin(&mut p, 5, 2, v2(3.5 * s, 0.0), "D");
        let Err(Diagnosis::OverConstrained { groups, gap }) = solve(&p) else {
            panic!("expected over-constrained: {:?}", solve(&p));
        };
        assert_eq!(groups, vec![Group::Mate(0), Group::Mate(1)]);
        assert!((gap.length - 5.0 * s).abs() < 1e-6 * s, "gap {gap:?}");
    }
}

#[test]
fn a_limited_hinge_starts_on_the_turn_nearest_its_limits() {
    // Review of 768df03d: a lid hinge limited to 0..4.71 rad placed at
    // 200° was measured as −160° and snapped to its bound 0.
    for s in SCALES {
        let mut p = one_body(MateKind::Revolute, s);
        p.mates[0].limits[0] = Some((0.0, 4.71));
        let c = v2(0.5 * s, 0.25 * s);
        let r = Mat3::rotation(Vec3::Z, 200f64.to_radians());
        p.start[1] = Pose::new(r, c - r.apply(c));
        let sol = solve(&p).unwrap();
        assert!(sol.at_limit.is_empty(), "{:?}", sol.at_limit);
        assert!(
            (sol.values[0][0] - 200f64.to_radians()).abs() < 1e-9,
            "{}",
            sol.values[0][0]
        );
        // Placed at 300° (beyond 4.71 rad = 270°): held at the nearer bound,
        // 270° rather than 0 (300° is 30° from 270° and 60° from 360°).
        let r = Mat3::rotation(Vec3::Z, 300f64.to_radians());
        p.start[1] = Pose::new(r, c - r.apply(c));
        let sol = solve(&p).unwrap();
        assert_eq!(sol.at_limit, vec![(0, 0)]);
        assert!(
            (sol.values[0][0] - 4.71).abs() < 1e-9,
            "{}",
            sol.values[0][0]
        );
    }
}
