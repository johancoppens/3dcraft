//! The rest solve keeps the promises of the Method: the pose the mates and
//! drives describe on the branch the assembly was put together in, or a
//! note that says it could not.
//!
//! A-016 (the assemblies' feedback): a mechanism put together at one drive
//! value and solved at rest at another was one Newton solve from its start,
//! which could land on a mirrored loop and say nothing. A-036: a free joint
//! took part of a misfit it had nothing to do with. Every expected number
//! here is a closed form computed apart from the solver: circle
//! intersections on named branches, a joint's value where it was placed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::*;
use craft3d_assembly::{Frame, Mat3, MateKind, MateSpec, Pose, Problem, Solution, Solver};
use craft3d_math::{Portable, Vec3};

/// One Jansen leg put together at crank angle `th0` (its bodies' own
/// coordinates are the world's there, so every start pose is the identity
/// and every mate holds), on a crank driven by `th1 − th0` from there.
/// With `crank_at_drive` the crank alone starts turned to the drive, so its
/// pin misses the leg's. Returns the problem, the foot and the foot point
/// G in the foot's coordinates.
fn leg(th0: f64, th1: f64, s: f64, crank_at_drive: bool) -> (Problem, usize, Vec3) {
    let mut p = planar(8, s, 100.0);
    pin(&mut p, 0, 1, Vec3::ZERO, "crank");
    p.mates[0].driven[0] = Some(th1 - th0);
    let (foot, g) = add_leg(&mut p, 1, 2, th0, s, 0.0);
    if crank_at_drive {
        p.start[1] = Pose::new(Mat3::exp(Vec3::Z * (th1 - th0)), Vec3::ZERO);
    }
    (p, foot, g)
}

/// Before the fix, the leg put together at 100° was solved at 260° and
/// 270° with its foot on the mirrored loop (154 mm and 76 mm off, the
/// solve "ok", no degree of freedom left). Now every crank angle of a turn,
/// from four angles it was put together at, at three scales, lands on the
/// branches of the strandbeest-leg Jig's generator; where the solve
/// straight from the placement lands elsewhere (as at 260° from 100°), the
/// note says the carried pose is shown.
#[test]
fn a_leg_put_together_at_one_crank_angle_rests_on_its_branch_at_any_other() {
    for s in SCALES {
        for th0d in [30.0f64, 100.0, 190.0, 250.0] {
            for k in 0..36 {
                let th1d = 10.0 * k as f64;
                let (th0, th1) = (th0d.to_radians(), th1d.to_radians());
                let (p, foot, g) = leg(th0, th1, s, false);
                let sol = Solver::new(&p).unwrap().solve().unwrap();
                let want = jansen_joints(th1, s, 0.0).unwrap()[7];
                let got = sol.poses[foot].point(g);
                assert!(
                    close(got, want, 1e-6 * p.scale),
                    "put together at {th0d}°, solved at {th1d}° (scale {s}): foot {got:?}, closed form {want:?}"
                );
                assert_eq!(sol.dof, 0);
                let note = sol.note.as_deref().unwrap_or("");
                assert!(
                    note.is_empty() || note.starts_with("carried from the placement"),
                    "{th0d}° → {th1d}°: {note}"
                );
            }
        }
    }
}

/// The crank turned to the drive and the leg left where it was put
/// together: the pins miss at the start and are closed there. The parts as
/// placed do not meet their mates, so where the carried and the direct
/// solve differ the note says the branch is not certain; nowhere else.
#[test]
fn a_leg_whose_crank_starts_at_the_drive_closes_on_its_branch() {
    for s in SCALES {
        for th0d in [30.0f64, 100.0, 190.0, 250.0] {
            for k in 0..12 {
                let th1d = 30.0 * k as f64;
                let (th0, th1) = (th0d.to_radians(), th1d.to_radians());
                let (p, foot, g) = leg(th0, th1, s, true);
                let sol = Solver::new(&p).unwrap().solve().unwrap();
                let want = jansen_joints(th1, s, 0.0).unwrap()[7];
                assert!(
                    close(sol.poses[foot].point(g), want, 1e-6 * p.scale),
                    "{th0d}° → {th1d}° (scale {s})"
                );
                if let Some(note) = &sol.note {
                    assert!(
                        note.starts_with("its parts as placed do not meet their mates"),
                        "{note}"
                    );
                }
            }
        }
    }
}

/// The same leg with its instances numbered the other way round (and so
/// its mates' sides and the factorisation's order changed) rests at the
/// same pose.
#[test]
fn the_rest_pose_does_not_depend_on_the_order_of_the_instances() {
    let (th0, th1) = (100f64.to_radians(), 265f64.to_radians());
    let (p, foot, g) = leg(th0, th1, 1.0, false);
    let a = Solver::new(&p).unwrap().solve().unwrap();
    // Reverse the instances 1..8 (the ground stays 0).
    let n = p.instances;
    let map = |i: usize| if i == 0 { 0 } else { n - i };
    let mut q = p.clone();
    for m in &mut q.mates {
        m.a.0 = map(m.a.0);
        m.b.0 = map(m.b.0);
    }
    for i in 0..n {
        q.start[map(i)] = p.start[i];
    }
    let b = Solver::new(&q).unwrap().solve().unwrap();
    for i in 0..n {
        let (pa, pb) = (a.poses[i], b.poses[map(i)]);
        for corner in [Vec3::ZERO, Vec3::X * 50.0, Vec3::Y * 50.0] {
            assert!(
                close(pa.point(corner), pb.point(corner), 1e-6),
                "instance {i}"
            );
        }
    }
    let want = jansen_joints(th1, 1.0, 0.0).unwrap()[7];
    assert!(close(b.poses[map(foot)].point(g), want, 1e-6 * p.scale));
}

/// A Grashof double-rocker (ground 4, input 3, coupler 1, output 3.5): the
/// input reaches 38.6°..78.6° on one circuit and its mirror on the other,
/// `|AD|² = 25 − 24 cos θ` between `(3.5 − 1)²` and `(3.5 + 1)²`. Put
/// together at 60° and driven to −60°, it cannot get there without being
/// taken apart: the rest solve still finds the pose at −60°, and says that
/// it is not the one the assembly reaches from where it was put together.
#[test]
fn a_drive_out_of_reach_of_the_branch_is_solved_and_said_so() {
    for s in SCALES {
        let (g, a, b, c) = (4.0, 3.0, 1.0, 3.5);
        let th0 = 60f64.to_radians();
        let pa = v2(a * th0.pcos(), a * th0.psin());
        let pd = v2(g, 0.0);
        let pb = circ(pa, b, pd, c, 1.0).unwrap();
        let mut p = planar(4, s, 10.0);
        let input = pin(&mut p, 0, 1, Vec3::ZERO, "input");
        pin(&mut p, 1, 2, pa * s, "A");
        pin(&mut p, 2, 3, pb * s, "B");
        pin(&mut p, 0, 3, pd * s, "D");
        p.mates[input].driven[0] = Some(-120f64.to_radians());
        let sol = Solver::new(&p).unwrap().solve().unwrap();
        let note = sol
            .note
            .clone()
            .expect("the rest solve says it was not carried");
        assert!(note.contains("not carried"), "{note}");
        // It is the pose at −60° all the same: the input's tip there.
        let tip = sol.poses[1].point(pa * s);
        let want = v2(a * (-th0).pcos(), a * (-th0).psin()) * s;
        assert!(close(tip, want, 1e-6 * p.scale), "{tip:?} vs {want:?}");
        // Within its circuit it is carried and says nothing.
        let mut q = p.clone();
        q.mates[input].driven[0] = Some(-15f64.to_radians());
        let sol = Solver::new(&q).unwrap().solve().unwrap();
        assert_eq!(sol.note, None);
        let th = 45f64.to_radians();
        let tip_at = v2(a * th.pcos(), a * th.psin());
        let want_a = tip_at * s;
        let want_b = circ(tip_at, b, pd, c, 1.0).unwrap() * s;
        assert!(close(sol.poses[1].point(pa * s), want_a, 1e-6 * p.scale));
        assert!(close(sol.poses[2].point(pb * s), want_b, 1e-6 * p.scale));
    }
}

/// A turret (1) on a free swivel about z through the origin of the ground
/// (0), and a part (2) on it by a `kind` mate whose axis is y at
/// (40, 0, 50), the part placed turned by `misfit` about z. Instances
/// numbered `order`.
fn turret(kind: MateKind, misfit: f64, s: f64, order: [usize; 3]) -> Problem {
    let mut p = planar(3, s, 100.0);
    let [ground, turret, part] = order;
    p.grounded = vec![ground];
    pin(&mut p, ground, turret, Vec3::ZERO, "swivel");
    let f = Frame::from_z_x(Vec3::new(40.0, 0.0, 50.0) * s, Vec3::Y, Vec3::X).unwrap();
    p.mates
        .push(MateSpec::new(kind, (turret, f), (part, f)).named("arm"));
    p.start[part] = Pose::new(Mat3::exp(Vec3::Z * misfit), Vec3::ZERO);
    p
}

/// A-036: nothing drives the swivel, and the part on the turret was
/// placed turned against its mate. The part turns back onto the turret,
/// and the swivel keeps its value (0) and the turret its pose. Before,
/// the least-squares steps spread the misfit over everything that can turn
/// about z alike: the swivel, the turret and the part each took a third
/// (0.04 rad of 0.12, the minimum-norm closure), and the solve said
/// nothing. On a revolute (free too) the part keeps the pitch it was
/// placed at, measured as its x axis about the mate's y axis: 0 for a part
/// turned less than a quarter turn, π for one turned more (2.5 rad). Held
/// to the tolerance: angles within 10 ε, the part's points within 10 ε of
/// the scale of where that pitch puts them.
#[test]
fn a_free_swivel_keeps_its_value_when_a_part_on_it_was_placed_turned() {
    for s in SCALES {
        for (kind, dof) in [(MateKind::Fixed, 1), (MateKind::Revolute, 2)] {
            for misfit in [0.12f64, -0.3, 2.5] {
                let p = turret(kind, misfit, s, [0, 1, 2]);
                let eps = p.tolerance / p.scale;
                let sol = Solver::new(&p).unwrap().solve().unwrap();
                let what = format!("{kind:?}, misfit {misfit}, scale {s}");
                assert_eq!(sol.dof, dof, "{what}");
                assert_eq!(sol.note, None, "{what}");
                let swivel = sol.value(0, 0).unwrap();
                assert!(swivel.abs() <= 10.0 * eps, "{what}: swivel {swivel}");
                assert!(turn(&sol.poses[1]).abs() <= 10.0 * eps, "{what}");
                let pitch = match kind {
                    MateKind::Revolute if misfit.pcos() < 0.0 => std::f64::consts::PI,
                    _ => 0.0,
                };
                if kind == MateKind::Revolute {
                    let got = sol.value(1, 0).unwrap();
                    assert!((got - pitch).abs() <= 10.0 * eps, "{what}: pitch {got}");
                }
                // The pitch turns the part about y through the pivot (a
                // right-handed turn about y takes x towards −z).
                let at = Vec3::new(40.0, 0.0, 50.0) * s;
                let arm = 10.0 * s;
                let want = at + Vec3::new(pitch.pcos(), 0.0, -pitch.psin()) * arm;
                let got = sol.poses[2].point(at + Vec3::X * arm);
                assert!(
                    close(sol.poses[2].point(at), at, 10.0 * eps * p.scale),
                    "{what}"
                );
                assert!(
                    close(got, want, 10.0 * eps * p.scale),
                    "{what}: the part's point is at {got:?}, not {want:?}"
                );
            }
        }
    }
}

/// The same with the instances numbered otherwise: the same pose.
#[test]
fn a_kept_swivel_does_not_depend_on_the_order_of_the_instances() {
    let base = Solver::new(&turret(MateKind::Fixed, 0.3, 1.0, [0, 1, 2]))
        .unwrap()
        .solve()
        .unwrap();
    for order in [[2, 0, 1], [1, 2, 0], [0, 2, 1]] {
        let sol = Solver::new(&turret(MateKind::Fixed, 0.3, 1.0, order))
            .unwrap()
            .solve()
            .unwrap();
        for (i, &k) in order.iter().enumerate() {
            for q in [Vec3::ZERO, Vec3::new(40.0, 10.0, 50.0)] {
                assert!(
                    close(base.poses[i].point(q), sol.poses[k].point(q), 1e-8),
                    "{order:?}: instance {i}"
                );
            }
        }
    }
}

/// A free hinge with nothing to close: the arm on the turret is driven
/// from 0 to 0.5 rad and the swivel's value and the turret's turn about
/// it stay as they were put, bit for bit (the rest of the turret's pose
/// holds the swivel's mate to the tolerance).
#[test]
fn a_free_swivel_with_nothing_to_close_does_not_turn_bit_for_bit() {
    for s in SCALES {
        let mut p = planar(3, s, 100.0);
        pin(&mut p, 0, 1, Vec3::ZERO, "swivel");
        let f = Frame::from_z_x(Vec3::new(15.0, 0.0, 100.0) * s, Vec3::Y, Vec3::X).unwrap();
        p.mates.push(
            MateSpec::new(MateKind::Revolute, (1, f), (2, f))
                .named("pitch")
                .driven(0.5),
        );
        let sol = Solver::new(&p).unwrap().solve().unwrap();
        assert_eq!(sol.value(0, 0), Some(0.0), "scale {s}");
        assert_eq!(turn(&sol.poses[1]), 0.0, "scale {s}");
        let eps = p.tolerance / p.scale;
        let top = Vec3::new(15.0, 0.0, 100.0) * s;
        assert!(close(sol.poses[1].point(top), top, 10.0 * eps * p.scale));
    }
}

/// An undriven double-rocker (as above) whose input was placed at 0°,
/// outside the 38.6°..78.6° it can reach, the rest of it at 60°: the loop
/// is closed where the solve closes it. Its four joints move only together
/// (one free motion), so none of them is the one to keep: the document does
/// not say which, and the solve says nothing.
#[test]
fn a_free_loop_is_closed_and_none_of_its_joints_is_chosen() {
    let (g, a, b, c) = (4.0, 3.0, 1.0, 3.5);
    let th0 = 60f64.to_radians();
    let pa = v2(a * th0.pcos(), a * th0.psin());
    let pd = v2(g, 0.0);
    let pb = circ(pa, b, pd, c, 1.0).unwrap();
    let mut p = planar(4, 1.0, 10.0);
    pin(&mut p, 0, 1, Vec3::ZERO, "input");
    pin(&mut p, 1, 2, pa, "A");
    pin(&mut p, 2, 3, pb, "B");
    pin(&mut p, 0, 3, pd, "D");
    p.start[1] = Pose::new(Mat3::exp(Vec3::Z * -th0), Vec3::ZERO);
    let sol = Solver::new(&p).unwrap().solve().unwrap();
    assert_eq!(sol.dof, 1);
    assert_eq!(sol.note, None);
    // The loop is closed: the coupler's length holds between the tips.
    let tip = sol.poses[1].point(pa);
    let knee = sol.poses[3].point(pb);
    assert!(((tip - knee).length() - b).abs() < 1e-6, "{tip:?} {knee:?}");
}

/// A free screw (lead 2) and its nut on a slider, coupled, the nut placed
/// 27 leads and 0.3 up: they move only together, so neither is kept,
/// whichever mate comes first and however large the assembly is (a first
/// version kept the first of them in the mates' order, or the screw alone
/// in an assembly over 318 across, where the nut's rate fell under 10⁻³ of
/// the screw's). The free swivel beside them, with a part on it placed
/// turned, is kept at 0 in every case.
#[test]
fn which_free_joints_are_kept_does_not_depend_on_order_or_size() {
    let lead = 2.0;
    let mut results = Vec::new();
    for size in [100.0, 10_000.0] {
        for nut_first in [true, false] {
            let mut p = planar(5, 1.0, size);
            let up = Frame::at(Vec3::ZERO);
            let screw = MateSpec::new(MateKind::Revolute, (0, up), (1, up)).named("screw");
            let nut = MateSpec::new(MateKind::Slider, (0, up), (2, up)).named("nut");
            let (n, sc) = if nut_first { (0, 1) } else { (1, 0) };
            if nut_first {
                p.mates.push(nut);
                p.mates.push(screw);
            } else {
                p.mates.push(screw);
                p.mates.push(nut);
            }
            p.couplings.push(craft3d_assembly::CouplingSpec {
                kind: craft3d_assembly::CouplingKind::Screw,
                mates: (n, sc),
                value: lead,
            });
            p.start[2] = Pose::at(Vec3::new(0.0, 0.0, 27.0 * lead + 0.3));
            // The swivel and a part fixed on it, placed turned by 0.12.
            pin(&mut p, 0, 3, v2(50.0, 0.0), "swivel");
            let f = Frame::at(Vec3::new(60.0, 0.0, 0.0));
            p.mates
                .push(MateSpec::new(MateKind::Fixed, (3, f), (4, f)).named("arm"));
            p.start[4] = Pose::new(Mat3::exp(Vec3::Z * 0.12), Vec3::ZERO);
            let sol = Solver::new(&p).unwrap().solve().unwrap();
            let what = format!("size {size}, nut first {nut_first}");
            assert_eq!(sol.note, None, "{what}");
            let eps = p.tolerance / p.scale;
            let swivel = sol.value(2, 0).unwrap();
            assert!(swivel.abs() <= 10.0 * eps, "{what}: swivel {swivel}");
            // The coupling holds: the nut's travel is a lead a turn.
            let travel = sol.value(n, 0).unwrap();
            let turned = sol.value(sc, 0).unwrap();
            assert!(
                (travel - lead * turned / (2.0 * std::f64::consts::PI)).abs() < 1e-6 * size,
                "{what}: {travel} against {turned}"
            );
            results.push((travel, turned));
        }
    }
    // Neither joint is held where it was placed: the nut is not kept 54.3
    // up (the screw then turned 27 turns), whatever comes first and
    // whatever the size. Where along their free motion the solve leaves
    // them is its own gauge (0.1.2's too), which differs between the
    // orders by some 5·10⁻⁵ and between the sizes by 2·10⁻³, far below the
    // 54 a choice made.
    for (travel, turned) in &results {
        assert!((travel - (27.0 * lead + 0.3)).abs() > 1.0, "{results:?}");
        assert!(turned.abs() < 1.0, "{results:?}");
    }
    for pair in results.chunks(2) {
        assert!((pair[0].0 - pair[1].0).abs() < 1e-3, "{results:?}");
        assert!((pair[0].1 - pair[1].1).abs() < 1e-3, "{results:?}");
    }
}

/// The review's construction: a Jansen leg put together exactly at `th0`
/// and its crank alone left turned by every 10° of a turn, the drive
/// holding the crank at `th0`. The rest solve must give the leg as it was
/// put together, at three scales (0.1.2 and its parent: 27 of 432 on the
/// mirrored loop, all at 190°; a carried answer from a start whose mates do
/// not hold is shown with a note, the direct one is never replaced
/// silently).
#[test]
fn a_leg_whose_crank_alone_was_turned_rests_as_it_was_put_together() {
    let mut off = 0;
    for s in SCALES {
        for th0d in [30.0f64, 100.0, 190.0, 250.0] {
            for k in 0..36 {
                let th0 = th0d.to_radians();
                let (mut p, foot, g) = leg(th0, th0, s, false);
                p.start[1] = Pose::new(
                    Mat3::exp(Vec3::Z * (10.0 * k as f64).to_radians()),
                    Vec3::ZERO,
                );
                let sol = Solver::new(&p).unwrap().solve().unwrap();
                let want = jansen_joints(th0, s, 0.0).unwrap()[7];
                if !close(sol.poses[foot].point(g), want, 1e-6 * p.scale) {
                    off += 1;
                }
            }
        }
    }
    assert_eq!(off, 0, "{off} of 432 legs on the mirrored loop");
}

/// A four-bar (ground 4 from the origin along x, crank `a` on the origin,
/// coupler `b`, rocker `c` on (4, 0)) put together at crank angle `th0` on
/// circuit `sg` (the coupler's far pin left of the line from its crank pin
/// to the rocker's pivot for +1), its crank alone turned `dd` further, and
/// driven to `th1`. With `pivots` each body's origin is at its own pivot
/// (the crank's at the origin, the coupler's at its crank pin, the
/// rocker's at its pivot), else at the world's origin. Returns the problem
/// and the coupler's far pin in the coupler's coordinates.
fn four_bar(
    (a, b, c): (f64, f64, f64),
    sg: f64,
    th0: f64,
    dd: f64,
    th1: f64,
    s: f64,
    pivots: bool,
) -> Option<(Problem, Vec3)> {
    let pa = v2(a * th0.pcos(), a * th0.psin()) * s;
    let pd = v2(4.0, 0.0) * s;
    let pb = circ(pa, b * s, pd, c * s, sg)?;
    let o = if pivots {
        [Vec3::ZERO, Vec3::ZERO, pa, pd]
    } else {
        [Vec3::ZERO; 4]
    };
    let mut p = planar(4, s, 10.0);
    for (i, j, w) in [(0, 1, Vec3::ZERO), (1, 2, pa), (2, 3, pb), (0, 3, pd)] {
        p.mates.push(MateSpec::new(
            MateKind::Revolute,
            (i, Frame::at(w - o[i])),
            (j, Frame::at(w - o[j])),
        ));
    }
    for (k, origin) in o.iter().enumerate().skip(1) {
        p.start[k] = Pose::at(*origin);
    }
    p.start[1] = Pose::new(Mat3::exp(Vec3::Z * dd), Vec3::ZERO);
    p.mates[0].driven[0] = Some(th1 - th0);
    Some((p, pb - o[2]))
}

/// The second review's blocker, and A-016 on the simplest loop: Grashof
/// crank-rockers whose crank turns all the way round on either circuit,
/// with couplers ever nearer the change point (3.02, 3.002 and 3.0002 on a
/// ground 4, crank 1, rocker 2, where `1 + 4 = 3 + 2` is the change point;
/// their circuits pass within 0.44, 0.14 and 0.044 of each other at a crank
/// of 180°) and one far from it (coupler 3.5, rocker 3). Put together on
/// either circuit at every 40°, exactly or with the crank alone turned 90°
/// on, and solved at rest at every 60° (+5°), at three scales and with the
/// bodies' origins at the world's or at their pivots: the coupler's far
/// pin is where the closed form puts it on the circuit it was put together
/// in, every time. Wherever the carried and the direct solve differ, a
/// note says so: that the carried pose is shown, where the parts as placed
/// meet their mates; that the branch is not certain, where they do not
/// (the crank turned). The parent of the fix and 0.1.2, on this grid of 5,184:
/// 252 of 2,592 exact placements and 150 of 2,592 turned ones on the other
/// circuit (2 errors), all at the near-change couplers. The carry of the
/// second review's branch (half-radian sub-steps) put 61 and 67 of 863 and
/// 864 exact placements of the review's own probe on the other circuit at
/// 3.02 and 3.002 where 0.1.2 was right; this carry without its short
/// first sub-step 51 here, without the cut of a sub-step that moved too
/// near a singular pose 87 (all at 3.0002); this one (`REST_REACH` in
/// `solver.rs`) none, and none of 41,472 on a grid twice as fine each way.
#[test]
fn a_four_bar_rests_on_the_circuit_it_was_put_together_in() {
    let mut off = Vec::new();
    for links in [
        (1.0, 3.5, 3.0),
        (1.0, 3.02, 2.0),
        (1.0, 3.002, 2.0),
        (1.0, 3.0002, 2.0),
    ] {
        for s in SCALES {
            for pivots in [false, true] {
                for sg in [1.0, -1.0] {
                    for i in 0..9 {
                        for dd in [0.0f64, 90.0] {
                            for j in 0..6 {
                                let th0 = (40.0 * i as f64).to_radians();
                                let th1 = (60.0 * j as f64 + 5.0).to_radians();
                                let Some((p, pb)) =
                                    four_bar(links, sg, th0, dd.to_radians(), th1, s, pivots)
                                else {
                                    continue;
                                };
                                let what = format!(
                                    "{links:?} circuit {sg} at {}° (crank {dd}° on) to {}°, scale {s}, origins at pivots {pivots}",
                                    40 * i,
                                    60 * j + 5
                                );
                                let ta = v2(links.0 * th1.pcos(), links.0 * th1.psin()) * s;
                                let want = circ(ta, links.1 * s, v2(4.0, 0.0) * s, links.2 * s, sg)
                                    .unwrap();
                                let sol = Solver::new(&p).unwrap().solve().unwrap();
                                if !close(sol.poses[2].point(pb), want, 1e-6 * p.scale) {
                                    off.push(what.clone());
                                }
                                if dd == 0.0 {
                                    let note = sol.note.as_deref().unwrap_or("");
                                    assert!(
                                        note.is_empty()
                                            || note.starts_with("carried from the placement"),
                                        "{what}: {note}"
                                    );
                                } else if let Some(note) = &sol.note {
                                    assert!(
                                        note.starts_with(
                                            "its parts as placed do not meet their mates"
                                        ),
                                        "{what}: {note}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        off.is_empty(),
        "{} on the other circuit:\n{}",
        off.len(),
        off.join("\n")
    );
}

/// The reviewer's swivel and elbow: a free swivel (0 to 1), an arm (2)
/// fixed on it 40 along x, and a free elbow (2 to 3) at 60 along x; the arm
/// is placed turned by `misfit` about z, and the forearm (3) turned by 0.3
/// of it and moved 0.5 along x. The arm turns back onto the swivel's part,
/// the swivel keeps its value (0) and the elbow its own as placed, the
/// forearm's turn less the arm's: −0.7 of the misfit. Before (0.1.2),
/// the least-squares steps spread the misfit over both joints (at a misfit
/// of 0.12, 0.049 rad on the swivel and −0.041 on the elbow). Held to ten
/// tolerances, at three scales.
#[test]
fn a_free_swivel_and_a_free_elbow_keep_their_values() {
    for s in SCALES {
        for misfit in [0.05f64, 0.12, 0.5, 1.5, 3.0] {
            let mut p = planar(4, s, 100.0);
            pin(&mut p, 0, 1, Vec3::ZERO, "swivel");
            let f = Frame::at(Vec3::new(40.0, 0.0, 0.0) * s);
            p.mates
                .push(MateSpec::new(MateKind::Fixed, (1, f), (2, f)).named("arm"));
            pin(&mut p, 2, 3, v2(60.0, 0.0) * s, "elbow");
            p.start[2] = Pose::new(Mat3::exp(Vec3::Z * misfit), Vec3::ZERO);
            p.start[3] = Pose::new(
                Mat3::exp(Vec3::Z * (0.3 * misfit)),
                Vec3::new(0.5, 0.0, 0.0) * s,
            );
            let sol = Solver::new(&p).unwrap().solve().unwrap();
            let what = format!("misfit {misfit}, scale {s}");
            let eps = p.tolerance / p.scale;
            assert_eq!(sol.dof, 2, "{what}");
            assert_eq!(sol.note, None, "{what}");
            let swivel = sol.value(0, 0).unwrap();
            let elbow = sol.value(2, 0).unwrap();
            assert!(swivel.abs() <= 10.0 * eps, "{what}: swivel {swivel}");
            assert!(
                (elbow + 0.7 * misfit).abs() <= 10.0 * eps,
                "{what}: elbow {elbow}"
            );
            // The arm on the swivel's part, which has not turned; the
            // forearm turned by the elbow's value about the elbow.
            assert!(turn(&sol.poses[1]).abs() <= 10.0 * eps, "{what}");
            assert!(turn(&sol.poses[2]).abs() <= 10.0 * eps, "{what}");
            let hand = v2(80.0, 0.0) * s;
            let elbow_at = v2(60.0, 0.0) * s;
            let want = elbow_at + v2((-0.7 * misfit).pcos(), (-0.7 * misfit).psin()) * (20.0 * s);
            assert!(
                close(sol.poses[3].point(hand), want, 10.0 * eps * p.scale),
                "{what}: the forearm's end at {:?}, not {want:?}",
                sol.poses[3].point(hand)
            );
        }
    }
}

/// What [`keeps_the_rule`] found over a set of placements.
#[derive(Default, Debug)]
struct Kept {
    cases: usize,
    right: usize,
    /// Not the closed form of the circuit placed on, with a note.
    wrong_noted: usize,
    /// Not the closed form, without a note: the direct solve's own answer
    /// (0.1.2's), bit for bit.
    wrong_as_direct: usize,
}

/// The rule every rest solve keeps (the third review): an answer without a
/// note is the one solve straight from the placement, bit for bit
/// (`solve_at_near` with no turns to take is that solve alone); an answer
/// that says the carried and the direct solve disagree differs from the
/// direct one; an answer that says the carry did not get there is the
/// direct one. So nothing the direct solve (0.1.2) got right comes out
/// otherwise without a note. `right` says whether an answer is the closed
/// form of the circuit it was placed on.
fn keeps_the_rule(p: &Problem, right: impl Fn(&Solution) -> bool, what: &str, kept: &mut Kept) {
    let mut solver = Solver::new(p).unwrap();
    let drives = solver.problem_drives();
    let direct = solver.solve_at_near(&drives, &[]);
    let sol = Solver::new(p)
        .unwrap()
        .solve()
        .unwrap_or_else(|e| panic!("{what}: {}", e.message(p)));
    let same = direct.as_ref().is_ok_and(|d| d.poses == sol.poses);
    match sol.note.as_deref() {
        None => {
            if let Ok(d) = &direct {
                assert_eq!(
                    d.poses, sol.poses,
                    "{what}: without a note, not the direct answer"
                );
            }
        }
        Some(n) if n.starts_with("carried from the placement") || n.contains("rests elsewhere") => {
            assert!(
                !same,
                "{what}: says the solves disagree, but it is the direct answer"
            );
        }
        Some(n) if n.starts_with("solved straight from the placement") => {
            assert!(
                same,
                "{what}: says it is the direct answer, but it is not: {n}"
            );
        }
        Some(n)
            if n.contains("it is solved there and carried")
                || n.starts_with("the solve straight from the placement failed") =>
        {
            assert!(direct.is_err(), "{what}: says the direct solve failed: {n}");
        }
        Some(n) => panic!("{what}: an unexpected note: {n}"),
    }
    kept.cases += 1;
    if right(&sol) {
        kept.right += 1;
    } else if sol.note.is_some() {
        kept.wrong_noted += 1;
    } else {
        kept.wrong_as_direct += 1;
    }
}

/// The third review's offset slider-crank (crank 1, offset 0.5, rods 1.52
/// and 1.502: circuits 0.49 and 0.14 apart where they pass closest, at a
/// crank of 270°), put together exactly or with its crank turned 90° on,
/// at three scales, solved at rest at every 60° (+7°). Before the step of
/// a rest carry was a tenth of a radian, 25 exact placements came out on
/// the other circuit without a note (the piston at 0.409456 instead of
/// −0.165717 from 170° to 277°). Now every answer is on its circuit by the
/// closed form or says it is not the direct one.
#[test]
fn an_offset_slider_crank_rests_on_its_circuit_or_says_so() {
    let mut kept = Kept::default();
    for l in [1.52f64, 1.502] {
        for s in SCALES {
            for sg in [1.0f64, -1.0] {
                for i in 0..12 {
                    for dd in [0.0f64, 90.0] {
                        for j in 0..6 {
                            let (th0, th1) = (
                                (30.0 * i as f64).to_radians(),
                                (60.0 * j as f64 + 7.0).to_radians(),
                            );
                            let piston = |th: f64| {
                                let ay = th.psin() - 0.5;
                                let disc = l * l - ay * ay;
                                (disc >= 0.0).then(|| th.pcos() + sg * disc.sqrt())
                            };
                            let (Some(b0), Some(b1)) = (piston(th0), piston(th1)) else {
                                continue;
                            };
                            let pa = v2(th0.pcos(), th0.psin()) * s;
                            let pb = v2(b0, 0.5) * s;
                            let want = v2(b1, 0.5) * s;
                            let mut p = planar(4, s, 5.0);
                            pin(&mut p, 0, 1, Vec3::ZERO, "crank");
                            pin(&mut p, 1, 2, pa, "A");
                            pin(&mut p, 2, 3, pb, "B");
                            let f = Frame::from_z_x(pb, Vec3::X, Vec3::Y).unwrap();
                            p.mates.push(
                                MateSpec::new(MateKind::Slider, (0, f), (3, f)).named("bore"),
                            );
                            p.mates[0].driven[0] = Some(th1 - th0);
                            p.start[1] =
                                Pose::new(Mat3::exp(Vec3::Z * dd.to_radians()), Vec3::ZERO);
                            let what = format!(
                                "rod {l}, circuit {sg}, {}° (+{dd}°) to {}°, scale {s}",
                                30 * i,
                                60 * j + 7
                            );
                            keeps_the_rule(
                                &p,
                                |sol| close(sol.poses[3].point(pb), want, 1e-6 * p.scale),
                                &what,
                                &mut kept,
                            );
                        }
                    }
                }
            }
        }
    }
    assert_eq!(kept.wrong_as_direct, 0, "{kept:?}");
    assert!(kept.cases > 1000, "{kept:?}");
}

/// Four-bars nearer the change point than `a_four_bar_rests_on_the_circuit_
/// it_was_put_together_in`'s: coupler 3.00002 (circuits 0.014 of the
/// scale apart at a crank of 180°), on its circuit by the closed form or
/// with a note, every time (before, 23 exact placements on the other
/// circuit without one); and 3.0000002 (1.4·10⁻⁴ apart), where even a
/// motion in 1° frames can land on the other circuit: there an answer
/// without a note is the direct solve's, bit for bit, right or not, and
/// nothing the direct solve got right comes out otherwise without a note.
/// The grid takes in the fourth review's four exact placements at 3.00002
/// where the carry itself lands on the other circuit (circuit −1 at 80°
/// driven to 187°, 217° and 247°; circuit +1 at 280° driven to 127°): on
/// its circuit, or a note.
#[test]
fn four_bars_at_the_change_point_keep_the_rule() {
    // Every 30° to every 60° (+7°), and the fourth review's cases.
    let mut grid: Vec<(f64, f64, f64)> = Vec::new();
    for sg in [1.0, -1.0] {
        for i in 0..12 {
            for j in 0..6 {
                grid.push((sg, 30.0 * i as f64, 60.0 * j as f64 + 7.0));
            }
        }
    }
    grid.extend([
        (-1.0, 80.0, 187.0),
        (-1.0, 80.0, 217.0),
        (-1.0, 80.0, 247.0),
        (1.0, 280.0, 127.0),
    ]);
    for (b, resolved) in [(3.00002f64, true), (3.0000002, false)] {
        let mut kept = Kept::default();
        for s in SCALES {
            for &(sg, th0d, th1d) in &grid {
                let (th0, th1) = (th0d.to_radians(), th1d.to_radians());
                let Some((p, pb)) = four_bar((1.0, b, 2.0), sg, th0, 0.0, th1, s, false) else {
                    continue;
                };
                let ta = v2(th1.pcos(), th1.psin()) * s;
                let Some(want) = circ(ta, b * s, v2(4.0, 0.0) * s, 2.0 * s, sg) else {
                    continue;
                };
                let what = format!("coupler {b}, circuit {sg}, {th0d}° to {th1d}°, scale {s}");
                keeps_the_rule(
                    &p,
                    |sol| close(sol.poses[2].point(pb), want, 1e-6 * p.scale),
                    &what,
                    &mut kept,
                );
            }
        }
        if resolved {
            assert_eq!(kept.wrong_as_direct, 0, "coupler {b}: {kept:?}");
        }
        assert!(kept.cases > 300, "{kept:?}");
    }
}

/// A spatial RSSR four-bar: a crank about z, a rocker about y through
/// (3, 0, 1), the coupler on two balls (lengths 1, 4.2 or 4.23, and 2), the
/// rocker's angle in closed form on each assembly mode. Every answer on
/// its mode or with a note.
#[test]
fn a_spatial_four_bar_rests_on_its_assembly_mode_or_says_so() {
    let (a, c) = (1.0f64, 2.0f64);
    let pivot = Vec3::new(3.0, 0.0, 1.0);
    let mut kept = Kept::default();
    for b in [4.2f64, 4.23] {
        for sg in [1.0f64, -1.0] {
            let rocker = |th: f64| -> Option<Vec3> {
                let am = Vec3::new(a * th.pcos(), a * th.psin(), 0.0);
                let q = pivot - am;
                let w = (b * b - q.dot(q) - c * c) / (2.0 * c);
                let (pp, qq) = (q.x, -q.z);
                let rr = pp.phypot(qq);
                if w.abs() > rr {
                    return None;
                }
                let phi = qq.patan2(pp) + sg * (w / rr).pacos();
                Some(pivot + Vec3::new(phi.pcos(), 0.0, -phi.psin()) * c)
            };
            for i in 0..12 {
                for dd in [0.0f64, 90.0] {
                    for j in 0..6 {
                        let (th0, th1) = (
                            (30.0 * i as f64).to_radians(),
                            (60.0 * j as f64 + 7.0).to_radians(),
                        );
                        let (Some(r0), Some(want)) = (rocker(th0), rocker(th1)) else {
                            continue;
                        };
                        let a0 = Vec3::new(a * th0.pcos(), a * th0.psin(), 0.0);
                        let mut p = planar(4, 1.0, 5.0);
                        pin(&mut p, 0, 1, Vec3::ZERO, "crank");
                        p.mates.push(
                            MateSpec::new(MateKind::Ball, (1, Frame::at(a0)), (2, Frame::at(a0)))
                                .named("ball A"),
                        );
                        p.mates.push(
                            MateSpec::new(MateKind::Ball, (2, Frame::at(r0)), (3, Frame::at(r0)))
                                .named("ball R"),
                        );
                        let f = Frame::from_z_x(pivot, Vec3::Y, Vec3::X).unwrap();
                        p.mates.push(
                            MateSpec::new(MateKind::Revolute, (0, f), (3, f)).named("rocker"),
                        );
                        p.mates[0].driven[0] = Some(th1 - th0);
                        p.start[1] = Pose::new(Mat3::exp(Vec3::Z * dd.to_radians()), Vec3::ZERO);
                        let what = format!(
                            "coupler {b}, mode {sg}, {}° (+{dd}°) to {}°",
                            30 * i,
                            60 * j + 7
                        );
                        keeps_the_rule(
                            &p,
                            |sol| close(sol.poses[3].point(r0), want, 1e-5),
                            &what,
                            &mut kept,
                        );
                    }
                }
            }
        }
    }
    assert_eq!(kept.wrong_as_direct, 0, "{kept:?}");
    assert!(kept.cases > 200, "{kept:?}");
}

/// A five-bar whose two cranks (on the origin and on (3, 0), both 1) are
/// driven at once, the couplers 2.2 and 1.85 or 1.805 meeting on circuit
/// `sg`: its pose in closed form, or a note (a drive whose straight path
/// passes a fold is not carried past it, and says so).
#[test]
fn a_five_bar_driven_by_both_cranks_keeps_the_rule() {
    let (d, b1) = (3.0f64, 2.2f64);
    let mut kept = Kept::default();
    for b2 in [1.85f64, 1.805] {
        for sg in [1.0f64, -1.0] {
            for i in 0..4 {
                for k in 0..4 {
                    let (t10, t20) = (
                        (90.0 * i as f64).to_radians(),
                        (90.0 * k as f64 + 15.0).to_radians(),
                    );
                    for (d1, d2) in [(2.5f64, -1.9f64), (-2.8, 2.2), (1.7, 3.0), (0.9, -0.4)] {
                        let crank1 = |t: f64| v2(t.pcos(), t.psin());
                        let crank2 = |t: f64| v2(d, 0.0) + v2(t.pcos(), t.psin());
                        let Some(p0) = circ(crank1(t10), b1, crank2(t20), b2, sg) else {
                            continue;
                        };
                        let Some(want) = circ(crank1(t10 + d1), b1, crank2(t20 + d2), b2, sg)
                        else {
                            continue;
                        };
                        let mut p = planar(5, 1.0, 5.0);
                        pin(&mut p, 0, 1, Vec3::ZERO, "crank 1");
                        pin(&mut p, 0, 2, v2(d, 0.0), "crank 2");
                        pin(&mut p, 1, 3, crank1(t10), "A1");
                        pin(&mut p, 2, 4, crank2(t20), "A2");
                        pin(&mut p, 3, 4, p0, "P");
                        p.mates[0].driven[0] = Some(d1);
                        p.mates[1].driven[0] = Some(d2);
                        let what = format!(
                            "couplers {b1}, {b2}, circuit {sg}, from ({}°, {}°) by ({d1}, {d2})",
                            90 * i,
                            90 * k + 15
                        );
                        keeps_the_rule(
                            &p,
                            |sol| close(sol.poses[3].point(p0), want, 1e-5),
                            &what,
                            &mut kept,
                        );
                    }
                }
            }
        }
    }
    assert_eq!(kept.wrong_as_direct, 0, "{kept:?}");
    assert!(kept.cases > 100, "{kept:?}");
}

/// The rigid motion of the plane taking `p1`, `p2` to `q1`, `q2`.
fn rigid(p1: Vec3, p2: Vec3, q1: Vec3, q2: Vec3) -> Pose {
    let turn = (q2.y - q1.y).patan2(q2.x - q1.x) - (p2.y - p1.y).patan2(p2.x - p1.x);
    let r = Mat3::exp(Vec3::Z * turn);
    let t = q1 - r.apply(p1);
    Pose::new(r, Vec3::new(t.x, t.y, 0.0))
}

/// 4, 8 and 12 Jansen legs on one crankshaft, throws at `k · 360° / n`,
/// each leg put together at its throw (exactly) or at a crank of 30° of
/// its own (its pins away from its throw, as a walker is saved), the crank
/// driven to every 60° (+7°): every foot on the Jig's branch, or a note.
#[test]
fn legs_on_one_crankshaft_rest_on_their_branches_or_say_so() {
    let mut kept = Kept::default();
    for n in [4usize, 8, 12] {
        for own in [None, Some(30.0f64)] {
            for k in 0..6 {
                let crank = (60.0 * k as f64 + 7.0).to_radians();
                let mut p = planar(2 + 6 * n, 1.0, 100.0);
                pin(&mut p, 0, 1, Vec3::ZERO, "crank");
                p.mates[0].driven[0] = Some(crank);
                let mut feet = Vec::new();
                for i in 0..n {
                    let phase = (360.0 * i as f64 / n as f64).to_radians();
                    let z = 10.0 * i as f64;
                    let first = 2 + 6 * i;
                    let (foot, g) = add_leg(&mut p, 1, first, phase, 1.0, z);
                    feet.push((foot, g, phase, z));
                    if let Some(own) = own {
                        let from = jansen_joints(phase, 1.0, z).unwrap();
                        let to = jansen_joints(own.to_radians(), 1.0, z).unwrap();
                        for (body, (u, v)) in [(1, 3), (1, 5), (2, 3), (2, 5), (4, 6), (5, 6)]
                            .iter()
                            .enumerate()
                        {
                            p.start[first + body] = rigid(from[*u], from[*v], to[*u], to[*v]);
                        }
                    }
                }
                let what = format!("{n} legs, put together at {own:?}, crank {}°", 60 * k + 7);
                keeps_the_rule(
                    &p,
                    |sol| {
                        feet.iter().all(|&(foot, g, phase, z)| {
                            let want = jansen_joints(phase + crank, 1.0, z).unwrap()[7];
                            close(sol.poses[foot].point(g), want, 1e-6 * 100.0)
                        })
                    },
                    &what,
                    &mut kept,
                );
            }
        }
    }
    assert_eq!(kept.wrong_as_direct, 0, "{kept:?}");
}

/// Four-bars placed a little off their mates (from 0.3 of the tolerance to
/// 3·10⁻³ of the scale), couplers 3.02 and 3.002: on their circuit, or a
/// note, every time.
#[test]
fn four_bars_placed_a_little_off_rest_on_their_circuit_or_say_so() {
    let mut kept = Kept::default();
    for b in [3.02f64, 3.002] {
        for delta in [3e-10f64, 3e-6, 3e-3] {
            for sg in [1.0, -1.0] {
                for i in 0..12 {
                    for j in 0..6 {
                        let th0 = (30.0 * i as f64).to_radians();
                        let th1 = (60.0 * j as f64 + 7.0).to_radians();
                        let Some((mut p, pb)) =
                            four_bar((1.0, b, 2.0), sg, th0, 0.0, th1, 1.0, false)
                        else {
                            continue;
                        };
                        let ta = v2(th1.pcos(), th1.psin());
                        let Some(want) = circ(ta, b, v2(4.0, 0.0), 2.0, sg) else {
                            continue;
                        };
                        let l = p.scale;
                        p.start[2] = Pose::new(
                            Mat3::exp(Vec3::Z * (0.7 * delta)),
                            Vec3::new(delta, -0.6 * delta, 0.0) * l,
                        );
                        p.start[3] = Pose::new(
                            Mat3::exp(Vec3::Z * (-0.4 * delta)),
                            Vec3::new(-0.8 * delta, 0.5 * delta, 0.0) * l,
                        );
                        let what = format!(
                            "coupler {b}, off by {delta}, circuit {sg}, {}° to {}°",
                            30 * i,
                            60 * j + 7
                        );
                        keeps_the_rule(
                            &p,
                            |sol| close(sol.poses[2].point(pb), want, 1e-4),
                            &what,
                            &mut kept,
                        );
                    }
                }
            }
        }
    }
    assert_eq!(kept.wrong_as_direct, 0, "{kept:?}");
}
