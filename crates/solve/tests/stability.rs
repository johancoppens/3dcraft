//! Solving again moves nothing, the same input gives the same bits, a small
//! change of a dimension keeps the sketch on its branch, and what is not
//! constrained is left alone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{catalogue, l_profile, options, resolved, solved, statuses};
use craft3d_solve::{Constraint as C, Entity, EntityId, Sketch, Solution, solve};

/// The coordinates and radii of a solution, bit for bit.
fn bits(solution: &Solution) -> Vec<u64> {
    let mut out = Vec::new();
    for e in &solution.entities {
        match e {
            Entity::Point { x, y } => out.extend([x.to_bits(), y.to_bits()]),
            Entity::Circle { radius, .. } => out.push(radius.to_bits()),
            Entity::Line { .. } | Entity::Arc { .. } => {}
        }
    }
    out
}

#[test]
fn solving_a_solved_sketch_moves_nothing() {
    for k in [1e-3, 1.0, 1e6] {
        for case in catalogue(k) {
            let first = solved(&case.sketch);
            let again = solved(&resolved(&case.sketch, &first));
            assert_eq!(
                bits(&first),
                bits(&again),
                "{} at scale {k}: the second solve moved something",
                case.name
            );
            assert_eq!(again.iterations, 0, "{} at scale {k}", case.name);
            assert_eq!(statuses(&first), statuses(&again), "{}", case.name);
            assert_eq!((first.dof, first.rank), (again.dof, again.rank));
            // And a third time, from the second.
            let third = solved(&resolved(&case.sketch, &again));
            assert_eq!(bits(&again), bits(&third), "{}", case.name);
        }
    }
}

#[test]
fn solving_a_sketch_with_a_conflict_again_moves_nothing() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(3.0, 4.0);
    let line = s.add_line(a, b);
    s.add_constraint(C::Horizontal { line });
    s.add_constraint(C::Distance { a, b, value: 10.0 });
    s.add_constraint(C::Distance { a, b, value: 20.0 });
    s.add_constraint(C::Horizontal { line });
    let first = solved(&s);
    let again = solved(&resolved(&s, &first));
    assert_eq!(first.conflicting(), [2]);
    assert_eq!(first.redundant(), [3]);
    assert_eq!(bits(&first), bits(&again));
    assert_eq!(statuses(&first), statuses(&again));
}

#[test]
fn the_same_input_gives_the_same_bits() {
    for k in [1e-3, 1.0, 1e6] {
        for case in catalogue(k) {
            let one = solve(&case.sketch, &options());
            let two = solve(&case.sketch, &options());
            let (one, two) = (one.unwrap(), two.unwrap());
            assert_eq!(bits(&one), bits(&two), "{}", case.name);
            // Statuses, residuals, rank and iteration count as well.
            assert_eq!(one, two, "{}", case.name);
            for (a, b) in one.constraints.iter().zip(&two.constraints) {
                assert_eq!(a.residual.to_bits(), b.residual.to_bits());
            }
        }
    }
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Sets the value of the dimension at `index`.
fn set_value(s: &mut Sketch, index: usize, new: f64) {
    match &mut s.constraints[index] {
        C::Distance { value, .. }
        | C::HorizontalDistance { value, .. }
        | C::VerticalDistance { value, .. }
        | C::Angle { value, .. }
        | C::Radius { value, .. }
        | C::Diameter { value, .. } => *value = new,
        other => panic!("{other:?} has no value"),
    }
}

#[test]
fn a_triangle_does_not_flip_when_a_side_changes() {
    // a = (0, 0), b = (4, 0), |bc| = 3, |ca| = d. By the law of cosines
    // c.x = (d^2 + 4^2 - 3^2) / (2 * 4), and c stays above ab:
    // c.y = +sqrt(d^2 - c.x^2). d runs from 5 down to 1.2 and up to 6.8
    // (the triangle exists for 1 < d < 7), each solve starting from the
    // previous solution.
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(4.0, 0.0);
    let c = s.add_point(4.0, 3.0);
    s.add_constraint(C::Fixed { point: a, at: None });
    s.add_constraint(C::Fixed { point: b, at: None });
    s.add_constraint(C::Distance {
        a: b,
        b: c,
        value: 3.0,
    });
    let side = s.add_constraint(C::Distance {
        a: c,
        b: a,
        value: 5.0,
    });
    let down = (0..=38).map(|i| 5.0 - 0.1 * i as f64);
    let up = (0..=56).map(|i| 1.2 + 0.1 * i as f64);
    for d in down.chain(up) {
        set_value(&mut s, side, d);
        let sol = solved(&s);
        assert!(sol.is_consistent(), "d = {d}");
        let got = sol.point(c).unwrap();
        let x = (d * d + 16.0 - 9.0) / 8.0;
        let y = (d * d - x * x).sqrt();
        // Near d = 1 and d = 7 the triangle is flat and the solution is
        // ill-conditioned (error ~ tolerance / height); 1e-7 covers the
        // flattest one here, height 0.6.
        assert!(
            (got[0] - x).abs() <= 1e-7 && (got[1] - y).abs() <= 1e-7,
            "d = {d}: {got:?}, expected ({x}, {y})"
        );
        assert!(got[1] > 0.0, "d = {d}: the triangle flipped");
        s = resolved(&s, &sol);
    }
}

#[test]
fn a_fillet_keeps_its_side_when_its_radius_changes() {
    // The rounded corner of the catalogue, radius stepped from 2 to 6 and
    // back to 0.5: the centre is (r, r), the arc ends are (0, r) and
    // (r, 0), and the arc keeps running counter-clockwise the short way
    // (the cross product of its two radii stays positive, equal to r^2).
    let case = catalogue(1.0)
        .into_iter()
        .find(|c| c.name == "rounded_corner")
        .unwrap();
    let mut s = case.sketch;
    let (th, c) = (case.points[0].0, case.points[1].0);
    let tv = case.points[2].0;
    let radius = s
        .constraints
        .iter()
        .position(|c| matches!(c, C::Radius { .. }))
        .unwrap();
    let up = (0..=16).map(|i| 2.0 + 0.25 * i as f64);
    let down = (0..=22).map(|i| 6.0 - 0.25 * i as f64);
    for r in up.chain(down) {
        set_value(&mut s, radius, r);
        let sol = solved(&s);
        assert!(sol.is_consistent() && sol.dof == 0, "r = {r}");
        let (pc, ph, pv) = (
            sol.point(c).unwrap(),
            sol.point(th).unwrap(),
            sol.point(tv).unwrap(),
        );
        for (got, want) in [(pc, [r, r]), (ph, [r, 0.0]), (pv, [0.0, r])] {
            assert!(
                (got[0] - want[0]).abs() <= 1e-9 && (got[1] - want[1]).abs() <= 1e-9,
                "r = {r}: {got:?} vs {want:?}"
            );
        }
        assert!((cross(pc, pv, ph) - r * r).abs() <= 1e-8, "r = {r}");
        s = resolved(&s, &sol);
    }
}

#[test]
fn a_tangent_circle_keeps_its_side_of_the_line() {
    // The circle resting on a line (catalogue): radius from 2 down to
    // 0.25 and up to 8; the centre stays above the line at (5, r).
    let case = catalogue(1.0)
        .into_iter()
        .find(|c| c.name == "line_tangent_to_circle")
        .unwrap();
    let mut s = case.sketch;
    let c = case.points[1].0;
    let radius = s
        .constraints
        .iter()
        .position(|c| matches!(c, C::Radius { .. }))
        .unwrap();
    let down = (0..=7).map(|i| 2.0 - 0.25 * i as f64);
    let up = (0..=31).map(|i| 0.25 + 0.25 * i as f64);
    for r in down.chain(up) {
        set_value(&mut s, radius, r);
        let sol = solved(&s);
        let got = sol.point(c).unwrap();
        assert!(
            (got[0] - 5.0).abs() <= 1e-9 && (got[1] - r).abs() <= 1e-9,
            "r = {r}: {got:?}"
        );
        s = resolved(&s, &sol);
    }
}

#[test]
fn offsets_keep_their_side() {
    // A point 12 to the left of the origin; as the offset shrinks to 1 the
    // point stays on the left. At 0 the side is lost: a point on the fence
    // counts as on the right, so growing the offset again gives x = +value.
    let mut s = Sketch::new();
    let o = s.add_point(0.0, 0.0);
    let p = s.add_point(-11.0, 3.0);
    s.add_constraint(C::Fixed { point: o, at: None });
    let offset = s.add_constraint(C::HorizontalDistance {
        a: o,
        b: p,
        value: 12.0,
    });
    for i in 0..=11 {
        let value = 12.0 - i as f64;
        set_value(&mut s, offset, value);
        let sol = solved(&s);
        assert!((sol.point(p).unwrap()[0] + value).abs() <= 1e-9, "{value}");
        // The height was never constrained and never changes.
        assert_eq!(sol.point(p).unwrap()[1], 3.0);
        s = resolved(&s, &sol);
    }
    set_value(&mut s, offset, 0.0);
    let sol = solved(&s);
    assert!(sol.point(p).unwrap()[0].abs() <= 1e-9);
    // Exactly on the fence (not a rounding error to one side of it).
    let mut s = Sketch::new();
    let o = s.add_point(0.0, 0.0);
    let p = s.add_point(0.0, 3.0);
    s.add_constraint(C::Fixed { point: o, at: None });
    s.add_constraint(C::HorizontalDistance {
        a: o,
        b: p,
        value: 5.0,
    });
    assert!((solved(&s).point(p).unwrap()[0] - 5.0).abs() <= 1e-9);
}

#[test]
fn unconstrained_geometry_is_left_alone() {
    // The L profile plus a bystander line and circle: solving the profile
    // must not touch them, bit for bit.
    let (mut s, corners) = l_profile(1.0);
    let a = s.add_point(123.456, -7.25);
    let b = s.add_point(-3.5, 99.125);
    s.add_line(a, b);
    let c = s.add_point(55.5, 44.25);
    let circle = s.add_circle(c, 3.75);
    let sol = solved(&s);
    assert_eq!(sol.point(a).unwrap(), [123.456, -7.25]);
    assert_eq!(sol.point(b).unwrap(), [-3.5, 99.125]);
    assert_eq!(sol.point(c).unwrap(), [55.5, 44.25]);
    assert_eq!(sol.radius(circle).unwrap(), 3.75);
    // 12 unknowns of the profile are determined; 4 + 3 are free.
    assert_eq!(sol.dof, 7);
    let top = sol.point(corners[4]).unwrap();
    assert!((top[0] - 10.0).abs() <= 1e-9 && (top[1] - 30.0).abs() <= 1e-9);
}

#[test]
fn an_underconstrained_line_moves_as_little_as_possible() {
    // A free segment from (0, 0) to (3, 4), length 5, asked to be 7 long:
    // the nearest solution stretches it symmetrically along its own
    // direction (0.6, 0.8), one unit at each end.
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(3.0, 4.0);
    s.add_constraint(C::Distance { a, b, value: 7.0 });
    let sol = solved(&s);
    let (a, b): ([f64; 2], [f64; 2]) = (sol.point(a).unwrap(), sol.point(b).unwrap());
    for (got, want) in [(a, [-0.6, -0.8]), (b, [3.6, 4.8])] {
        assert!(
            (got[0] - want[0]).abs() <= 1e-9 && (got[1] - want[1]).abs() <= 1e-9,
            "{got:?} vs {want:?}"
        );
    }
    assert_eq!(sol.dof, 3);
}

#[test]
fn changing_one_dimension_of_the_l_profile_moves_only_what_must_move() {
    let (mut s, p) = l_profile(1.0);
    let sol = solved(&s);
    s = resolved(&s, &sol);
    // The width goes from 40 to 55: corners 1 and 2 follow, the rest stay.
    let width = s
        .constraints
        .iter()
        .position(|c| matches!(c, C::HorizontalDistance { .. }))
        .unwrap();
    set_value(&mut s, width, 55.0);
    let sol = solved(&s);
    let expected = [
        [0.0, 0.0],
        [55.0, 0.0],
        [55.0, 10.0],
        [10.0, 10.0],
        [10.0, 30.0],
        [0.0, 30.0],
    ];
    let ids: [EntityId; 6] = p;
    for (id, want) in ids.iter().zip(expected) {
        let got = sol.point(*id).unwrap();
        assert!(
            (got[0] - want[0]).abs() <= 1e-9 && (got[1] - want[1]).abs() <= 1e-9,
            "{got:?} vs {want:?}"
        );
    }
    assert_eq!(sol.dof, 0);
}
