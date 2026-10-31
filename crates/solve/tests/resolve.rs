//! A caller stores the solution and solves again later. The second solve
//! must tell the same story: `Ok`, the same conflicting constraints, and
//! nothing moved. (The seeded random runs in `robustness.rs` assert the
//! same for thousands of sketches; these are the cases with a reason.)

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use std::f64::consts::PI;

use common::{options, resolved, solved, statuses};
use craft3d_solve::ConstraintStatus::{Conflicting, Satisfied};
use craft3d_solve::{Constraint as C, Entity, EntityId, Sketch, Solution, solve};

fn bits(entities: &[Entity]) -> Vec<u64> {
    let mut out = Vec::new();
    for e in entities {
        match e {
            Entity::Point { x, y } => out.extend([x.to_bits(), y.to_bits()]),
            Entity::Circle { radius, .. } => out.push(radius.to_bits()),
            Entity::Line { .. } | Entity::Arc { .. } | Entity::Image { .. } => {}
        }
    }
    out
}

/// Solves, then solves the solution twice more: every time the same
/// statuses and the same coordinates, bit for bit.
fn solved_stably(sketch: &Sketch) -> Solution {
    let first = solved(sketch);
    let mut current = first.clone();
    for round in 0..2 {
        let again = match solve(&resolved(sketch, &current), &options()) {
            Ok(again) => again,
            Err(e) => panic!("solve {} of the same sketch failed: {e}", round + 2),
        };
        assert_eq!(statuses(&again), statuses(&first), "solve {}", round + 2);
        assert_eq!(bits(&again.entities), bits(&first.entities));
        // Without a conflict there is nothing to do. With one, the search
        // for something better runs again and comes back empty-handed.
        if first.is_consistent() {
            assert_eq!(again.iterations, 0);
        }
        assert_eq!((again.dof, again.rank), (first.dof, first.rank));
        for (a, b) in again.constraints.iter().zip(&first.constraints) {
            assert_eq!(a.undefined, b.undefined);
        }
        current = again;
    }
    first
}

/// The sketch of the engine's "broken by a variable" case: a fixed corner,
/// a horizontal base of length `w`, an upright at 90 degrees to it, 3 long.
fn base_and_upright(w: f64, at: [[f64; 2]; 3]) -> (Sketch, [EntityId; 3]) {
    let mut s = Sketch::new();
    let p = at.map(|[x, y]| s.add_point(x, y));
    let base = s.add_line(p[0], p[1]);
    let upright = s.add_line(p[1], p[2]);
    s.add_constraint(C::Fixed {
        point: p[0],
        at: Some([0.0, 0.0]),
    });
    s.add_constraint(C::Horizontal { line: base });
    s.add_constraint(C::Distance {
        a: p[0],
        b: p[1],
        value: w,
    });
    s.add_constraint(C::Angle {
        a: base,
        b: upright,
        value: PI / 2.0,
    });
    s.add_constraint(C::Distance {
        a: p[1],
        b: p[2],
        value: 3.0,
    });
    (s, p)
}

#[test]
fn a_base_shrunk_to_nothing_is_the_same_conflict_every_time_and_heals() {
    // Width 4: (0,0), (4,0), (4,3); the angle from +x to the upright is a
    // quarter turn counter-clockwise, so the upright goes up.
    let drawn = [[0.0, 0.0], [4.0, 0.3], [4.2, 3.0]];
    let (sketch, p) = base_and_upright(4.0, drawn);
    let good = solved_stably(&sketch);
    assert_eq!(statuses(&good), [Satisfied; 5]);
    assert_eq!(good.dof, 0);

    // Width 0, from the good solution: the base is a point, the angle to
    // it means nothing. Everything else holds: p1 on p0, p2 3 from it.
    let (mut broken, _) = base_and_upright(0.0, drawn);
    broken.entities = good.entities.clone();
    let sol = solved_stably(&broken);
    assert_eq!(
        statuses(&sol),
        [Satisfied, Satisfied, Satisfied, Conflicting, Satisfied]
    );
    assert_eq!(sol.constraints[3].residual, f64::INFINITY);
    assert!(sol.constraints[3].undefined.is_some());
    let (p1, p2): ([f64; 2], [f64; 2]) = (sol.point(p[1]).unwrap(), sol.point(p[2]).unwrap());
    assert!(p1[0].abs() <= 1e-9 && p1[1].abs() <= 1e-9);
    assert!((p2[0].phypot(p2[1]) - 3.0).abs() <= 1e-9);

    // Width 4 again, from the broken solution: the base gets its length
    // back (a point has no direction to grow in; +x is the convention and
    // the horizontal agrees), and with it the angle its meaning.
    let (mut healed, _) = base_and_upright(4.0, drawn);
    healed.entities = sol.entities.clone();
    let sol = solved_stably(&healed);
    assert_eq!(statuses(&sol), [Satisfied; 5]);
    assert_eq!(sol.dof, 0);
    for (id, want) in p.iter().zip([[0.0, 0.0], [4.0, 0.0], [4.0, 3.0]]) {
        let got = sol.point(*id).unwrap();
        assert!(
            (got[0] - want[0]).abs() <= 1e-9 && (got[1] - want[1]).abs() <= 1e-9,
            "{got:?} vs {want:?}"
        );
    }
}

#[test]
fn horizontal_vertical_and_a_length_in_every_order() {
    for order in 0..3 {
        let mut s = Sketch::new();
        let a = s.add_point(0.0, 0.0);
        let b = s.add_point(3.0, 4.0);
        let line = s.add_line(a, b);
        let all = [
            C::Horizontal { line },
            C::Vertical { line },
            C::Distance { a, b, value: 10.0 },
        ];
        for i in 0..3 {
            s.add_constraint(all[(order + i) % 3]);
        }
        let sol = solved_stably(&s);
        assert_eq!(
            statuses(&sol),
            [Satisfied, Satisfied, Conflicting],
            "order {order}"
        );
    }
}

#[test]
fn two_distances_between_the_same_points() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(3.0, 4.0);
    s.add_constraint(C::Distance { a, b, value: 10.0 });
    s.add_constraint(C::Distance { a, b, value: 20.0 });
    let sol = solved_stably(&s);
    assert_eq!(statuses(&sol), [Satisfied, Conflicting]);
    assert!((sol.constraints[1].residual - 10.0).abs() <= 1e-9);
}

#[test]
fn a_constraint_undefined_where_the_others_put_the_sketch() {
    // Horizontal and vertical shrink the line to a point; "parallel to the
    // other line" then has no meaning. It had one where the sketch was
    // drawn, so the first pass cannot know.
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(3.0, 4.0);
    let line = s.add_line(a, b);
    let c = s.add_point(5.0, 0.0);
    let d = s.add_point(9.0, 1.0);
    let other = s.add_line(c, d);
    s.add_constraint(C::Horizontal { line });
    s.add_constraint(C::Vertical { line });
    s.add_constraint(C::Parallel { a: line, b: other });
    let sol = solved_stably(&s);
    assert_eq!(statuses(&sol), [Satisfied, Satisfied, Conflicting]);
    assert_eq!(
        sol.constraints[2].undefined,
        Some("a line shorter than the linear tolerance has no direction")
    );
    // The other line was never asked to move.
    assert_eq!(sol.point(c).unwrap(), [5.0, 0.0]);
    assert_eq!(sol.point(d).unwrap(), [9.0, 1.0]);
}

#[test]
fn a_constraint_undefined_at_the_start_that_the_others_give_a_meaning() {
    // A line drawn as a single point, then given a length and an angle.
    // Where it starts the angle is undefined; once the length has made it
    // a line (along +x, by convention) the angle turns it: 10 long at 90
    // degrees from the base's +x direction ends at (2, 12).
    let mut s = Sketch::new();
    let o = s.add_point(0.0, 0.0);
    let e = s.add_point(8.0, 0.0);
    let base = s.add_line(o, e);
    let p = s.add_point(2.0, 2.0);
    let q = s.add_point(2.0, 2.0);
    let line = s.add_line(p, q);
    s.add_constraint(C::Fixed { point: o, at: None });
    s.add_constraint(C::Fixed { point: e, at: None });
    s.add_constraint(C::Fixed { point: p, at: None });
    s.add_constraint(C::Angle {
        a: base,
        b: line,
        value: PI / 2.0,
    });
    s.add_constraint(C::Distance {
        a: p,
        b: q,
        value: 10.0,
    });
    let sol = solved_stably(&s);
    assert_eq!(statuses(&sol), [Satisfied; 5]);
    let q = sol.point(q).unwrap();
    assert!(
        (q[0] - 2.0).abs() <= 1e-9 && (q[1] - 12.0).abs() <= 1e-9,
        "{q:?}"
    );
    assert_eq!(sol.dof, 0);
}

#[test]
fn a_fresh_constraint_on_a_line_without_length_is_named() {
    // Nothing else is wrong with the sketch and nothing gives the line a
    // length: the caller is told which constraint means nothing, and why.
    let mut s = Sketch::new();
    let o = s.add_point(0.0, 0.0);
    let e = s.add_point(8.0, 0.0);
    let base = s.add_line(o, e);
    let p = s.add_point(2.0, 2.0);
    let line = s.add_line(p, p);
    s.add_constraint(C::Horizontal { line: base });
    s.add_constraint(C::Angle {
        a: base,
        b: line,
        value: 0.5,
    });
    let sol = solved_stably(&s);
    assert_eq!(statuses(&sol), [Satisfied, Conflicting]);
    assert_eq!(sol.conflicting(), [1]);
    assert_eq!(sol.constraints[1].residual, f64::INFINITY);
    assert_eq!(
        sol.constraints[1].undefined,
        Some("a line shorter than the linear tolerance has no direction")
    );
    assert_eq!(sol.entities, s.entities);
}

#[test]
fn a_triangle_that_cannot_close_stays_where_it_was_left() {
    // Sides 1, 1, 5: the third length is blamed after a search that does
    // not converge (no dependent row to point at), and the solution it
    // leaves must be one the next solve leaves alone.
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(1.2, 0.1);
    let c = s.add_point(0.7, 0.8);
    s.add_constraint(C::Fixed { point: a, at: None });
    s.add_constraint(C::HorizontalPoints { a, b });
    s.add_constraint(C::Distance { a, b, value: 1.0 });
    s.add_constraint(C::Distance {
        a: b,
        b: c,
        value: 1.0,
    });
    s.add_constraint(C::Distance {
        a: c,
        b: a,
        value: 5.0,
    });
    let sol = solved_stably(&s);
    assert_eq!(sol.conflicting(), [4]);
}

#[test]
fn a_redundancy_that_holds_only_to_rounding_is_still_redundant() {
    // A 3-4-5 triangle from its three lengths has a right angle at b; to
    // say so adds nothing. Unlike a repeated constraint, this dependence
    // is not exact in floating point: the gradient of the perpendicularity
    // lies in the span of the others up to about 1e-16, and it is the rank
    // threshold that calls that zero.
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(4.3, 0.2);
    let c = s.add_point(3.6, 2.7);
    let ab = s.add_line(a, b);
    let bc = s.add_line(b, c);
    s.add_constraint(C::Fixed { point: a, at: None });
    s.add_constraint(C::HorizontalPoints { a, b });
    s.add_constraint(C::Distance { a, b, value: 4.0 });
    s.add_constraint(C::Distance {
        a: b,
        b: c,
        value: 3.0,
    });
    s.add_constraint(C::Distance {
        a: c,
        b: a,
        value: 5.0,
    });
    s.add_constraint(C::Perpendicular { a: ab, b: bc });
    let sol = solved_stably(&s);
    assert_eq!(sol.redundant(), [5]);
    assert!(sol.conflicting().is_empty());
    assert_eq!((sol.rank, sol.dof), (6, 0));
    let c = sol.point(c).unwrap();
    assert!((c[0] - 4.0).abs() <= 1e-9 && (c[1] - 3.0).abs() <= 1e-9);
}

#[test]
fn fixed_where_it_is_after_fixed_elsewhere() {
    // The point is drawn at (1, 2), pinned to (4, 6), and then pinned
    // "where it is". Where is that? A caller that stores the solution and
    // solves again finds the point at (4, 6), where both pins agree. The
    // answer of the first solve must be that one, not "conflicting with
    // where the point was before anything was solved".
    let mut s = Sketch::new();
    let p = s.add_point(1.0, 2.0);
    s.add_constraint(C::Fixed {
        point: p,
        at: Some([4.0, 6.0]),
    });
    s.add_constraint(C::Fixed { point: p, at: None });
    let sol = solved_stably(&s);
    assert_eq!(
        statuses(&sol),
        [Satisfied, craft3d_solve::ConstraintStatus::Redundant]
    );
    assert_eq!(sol.point(p).unwrap(), [4.0, 6.0]);
}
