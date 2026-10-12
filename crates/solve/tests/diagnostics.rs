//! Redundant and conflicting constraints are named, the later constraint
//! takes the blame, bad input is an error, and the iteration is damped.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use std::f64::consts::PI;

use common::{options, solved, statuses};
use craft3d_math::Tolerance;
use craft3d_solve::ConstraintStatus::{Conflicting, Redundant, Satisfied};
use craft3d_solve::{
    Constraint as C, Entity, EntityId, EntityKind, MAX_ENTITIES, Site, Sketch, SolveError,
    SolveOptions, solve,
};

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

#[test]
fn horizontal_twice_is_redundant() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(5.0, 1.0);
    let line = s.add_line(a, b);
    s.add_constraint(C::Horizontal { line });
    s.add_constraint(C::Horizontal { line });
    s.add_constraint(C::HorizontalPoints { a: b, b: a });
    let sol = solved(&s);
    assert_eq!(statuses(&sol), [Satisfied, Redundant, Redundant]);
    // 4 unknowns, one independent equation.
    assert_eq!(sol.dof, 3);
    assert!(sol.constraints.iter().all(|c| c.residual <= 1e-6));
    assert_eq!(sol.redundant(), [1, 2]);
    assert!(sol.is_consistent());
}

#[test]
fn overdimensioned_rectangle_names_the_extra_dimensions() {
    // A rectangle made of one horizontal, one vertical and two parallels,
    // with width and height given on two sides: fully defined (DOF 0).
    // Giving the same width and height on the opposite sides, and saying
    // that the top is horizontal, adds nothing.
    let mut s = Sketch::new();
    let p = [
        s.add_point(0.2, 0.1),
        s.add_point(7.0, -0.4),
        s.add_point(8.5, 4.4),
        s.add_point(-0.3, 3.2),
    ];
    let l: Vec<EntityId> = (0..4).map(|i| s.add_line(p[i], p[(i + 1) % 4])).collect();
    s.add_constraint(C::Fixed {
        point: p[0],
        at: Some([0.0, 0.0]),
    });
    s.add_constraint(C::Horizontal { line: l[0] });
    s.add_constraint(C::Vertical { line: l[3] });
    s.add_constraint(C::Parallel { a: l[0], b: l[2] });
    s.add_constraint(C::Parallel { a: l[3], b: l[1] });
    s.add_constraint(C::Distance {
        a: p[0],
        b: p[1],
        value: 8.0,
    });
    s.add_constraint(C::Distance {
        a: p[0],
        b: p[3],
        value: 4.0,
    });
    let first_extra = s.add_constraint(C::Distance {
        a: p[3],
        b: p[2],
        value: 8.0,
    });
    s.add_constraint(C::Distance {
        a: p[1],
        b: p[2],
        value: 4.0,
    });
    s.add_constraint(C::Horizontal { line: l[2] });
    let sol = solved(&s);
    assert_eq!(first_extra, 7);
    assert_eq!(sol.redundant(), [7, 8, 9]);
    assert!(sol.conflicting().is_empty());
    assert_eq!(sol.dof, 0);
    // 8 unknowns, all determined by the first seven constraints (8 rows).
    assert_eq!(sol.rank, 8);
    let expected = [[0.0, 0.0], [8.0, 0.0], [8.0, 4.0], [0.0, 4.0]];
    for (id, want) in p.iter().zip(expected) {
        let got = sol.point(*id).unwrap();
        assert!(dist(got, want) <= 1e-9, "{got:?} vs {want:?}");
    }
}

#[test]
fn two_distances_between_the_same_points_blame_the_later() {
    for (first, second) in [(10.0, 20.0), (20.0, 10.0)] {
        let mut s = Sketch::new();
        let a = s.add_point(0.0, 0.0);
        let b = s.add_point(3.0, 4.0);
        s.add_constraint(C::Distance { a, b, value: first });
        s.add_constraint(C::Distance {
            a,
            b,
            value: second,
        });
        let sol = solved(&s);
        assert_eq!(statuses(&sol), [Satisfied, Conflicting]);
        assert_eq!(sol.conflicting(), [1]);
        assert!(!sol.is_consistent());
        let gap = dist(sol.point(a).unwrap(), sol.point(b).unwrap());
        // The first one holds; the second is off by the difference.
        assert!((gap - first).abs() <= 1e-9);
        assert!(sol.constraints[0].residual <= 1e-6);
        assert!((sol.constraints[1].residual - 10.0).abs() <= 1e-9);
        // 4 unknowns, one equation in force.
        assert_eq!(sol.dof, 3);
    }
}

#[test]
fn horizontal_and_vertical_together_need_a_free_length() {
    // Both on one line is consistent: the line shrinks to a point.
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(3.0, 4.0);
    let line = s.add_line(a, b);
    s.add_constraint(C::Horizontal { line });
    s.add_constraint(C::Vertical { line });
    let sol = solved(&s);
    assert_eq!(statuses(&sol), [Satisfied, Satisfied]);
    assert!(dist(sol.point(a).unwrap(), sol.point(b).unwrap()) <= 1e-9);
    assert_eq!(sol.dof, 2);

    // With a length as well, whichever comes last cannot hold.
    let length = C::Distance { a, b, value: 10.0 };
    let orders = [
        [C::Horizontal { line }, C::Vertical { line }, length],
        [length, C::Horizontal { line }, C::Vertical { line }],
        [C::Vertical { line }, length, C::Horizontal { line }],
    ];
    for order in orders {
        let mut s = Sketch::new();
        let a = s.add_point(0.0, 0.0);
        let b = s.add_point(3.0, 4.0);
        s.add_line(a, b);
        s.constraints.extend(order);
        let sol = solved(&s);
        assert_eq!(
            statuses(&sol),
            [Satisfied, Satisfied, Conflicting],
            "{order:?}"
        );
        assert!(sol.constraints[2].residual > 1.0);
    }
}

/// A fixed horizontal base line and a second line, constrained horizontal
/// and at 30 degrees to the base.
fn horizontal_and_angle(q: [f64; 2], angle_first: bool) -> (Sketch, EntityId, EntityId) {
    let mut s = Sketch::new();
    let o = s.add_point(0.0, 0.0);
    let a = s.add_point(10.0, 0.0);
    let base = s.add_line(o, a);
    let p = s.add_point(1.0, 3.0);
    let q = s.add_point(q[0], q[1]);
    let line = s.add_line(p, q);
    s.add_constraint(C::Fixed { point: o, at: None });
    s.add_constraint(C::Fixed { point: a, at: None });
    let horizontal = C::Horizontal { line };
    let angle = C::Angle {
        a: base,
        b: line,
        value: PI / 6.0,
    };
    if angle_first {
        s.constraints.extend([angle, horizontal]);
    } else {
        s.constraints.extend([horizontal, angle]);
    }
    (s, p, q)
}

#[test]
fn horizontal_plus_thirty_degrees_to_a_horizontal_line_is_a_conflict() {
    // Drawn exactly horizontal (the angle's gradient is parallel to the
    // horizontal's from the start) and drawn slanted (they only become
    // parallel as the iteration fails): the same verdict.
    for q in [[6.0, 3.0], [6.0, 4.0], [5.5, 0.5]] {
        let (s, p, q_id) = horizontal_and_angle(q, false);
        let sol = solved(&s);
        assert_eq!(
            statuses(&sol),
            [Satisfied, Satisfied, Satisfied, Conflicting],
            "drawn to {q:?}"
        );
        let (p, q) = (sol.point(p).unwrap(), sol.point(q_id).unwrap());
        // The earlier constraint holds: the line is horizontal.
        assert!((p[1] - q[1]).abs() <= 1e-9);
        // The angle is 0 instead of 30 degrees (or is undefined).
        assert!(sol.constraints[3].residual >= PI / 6.0 - 1e-9);
    }
}

#[test]
fn thirty_degrees_then_horizontal_blames_the_horizontal() {
    for q in [[6.0, 3.0], [6.0, 4.0]] {
        let (s, p, q_id) = horizontal_and_angle(q, true);
        let sol = solved(&s);
        assert_eq!(
            statuses(&sol),
            [Satisfied, Satisfied, Satisfied, Conflicting],
            "drawn to {q:?}"
        );
        let (p, q) = (sol.point(p).unwrap(), sol.point(q_id).unwrap());
        // The line is at 30 degrees: tan 30 = 1 / sqrt 3.
        assert!(((q[1] - p[1]) / (q[0] - p[0]) - 1.0 / 3f64.sqrt()).abs() <= 1e-9);
    }
}

#[test]
fn a_triangle_that_violates_the_triangle_inequality() {
    // Sides 1, 1 and 5 do not close. The first two can be drawn; the third
    // is the one too many.
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
    let sol = solved(&s);
    assert_eq!(sol.conflicting(), [4]);
    let (a, b, c) = (
        sol.point(a).unwrap(),
        sol.point(b).unwrap(),
        sol.point(c).unwrap(),
    );
    assert!((dist(a, b) - 1.0).abs() <= 1e-6 && (dist(b, c) - 1.0).abs() <= 1e-6);
    // c is within 2 of a, so the third side misses by at least 3.
    assert!(sol.constraints[4].residual >= 3.0 - 1e-6);
}

#[test]
fn fixed_twice() {
    let mut s = Sketch::new();
    let p = s.add_point(1.0, 2.0);
    s.add_constraint(C::Fixed { point: p, at: None });
    s.add_constraint(C::Fixed {
        point: p,
        at: Some([1.0, 2.0]),
    });
    s.add_constraint(C::Fixed {
        point: p,
        at: Some([1.0, 3.0]),
    });
    let sol = solved(&s);
    assert_eq!(statuses(&sol), [Satisfied, Redundant, Conflicting]);
    assert_eq!(sol.point(p).unwrap(), [1.0, 2.0]);
    assert!((sol.constraints[2].residual - 1.0).abs() <= 1e-12);
    assert_eq!(sol.dof, 0);
}

#[test]
fn a_constraint_that_adds_one_of_its_two_equations_is_not_redundant() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(2.0, 1.0);
    s.add_constraint(C::HorizontalPoints { a, b });
    s.add_constraint(C::Coincident { a, b });
    s.add_constraint(C::VerticalPoints { a, b });
    let sol = solved(&s);
    assert_eq!(statuses(&sol), [Satisfied, Satisfied, Redundant]);
    assert_eq!(sol.dof, 2);
}

#[test]
fn a_singular_start_is_not_mistaken_for_a_conflict() {
    // p must stay level with the origin and be 5 from (0, 3): it belongs
    // at (4, 0) or (-4, 0). Drawn at the origin itself, straight below
    // (0, 3), the distance has no gradient along the x axis; the linearised
    // system says "impossible" although a solution exists.
    let mut s = Sketch::new();
    let o = s.add_point(0.0, 0.0);
    let q = s.add_point(0.0, 3.0);
    let p = s.add_point(0.0, 0.0);
    s.add_constraint(C::Fixed { point: o, at: None });
    s.add_constraint(C::Fixed { point: q, at: None });
    s.add_constraint(C::HorizontalPoints { a: o, b: p });
    s.add_constraint(C::Distance {
        a: p,
        b: q,
        value: 5.0,
    });
    let sol = solved(&s);
    assert_eq!(statuses(&sol), [Satisfied; 4]);
    let p = sol.point(p).unwrap();
    // 3-4-5 triangle.
    assert!(
        (p[0].abs() - 4.0).abs() <= 1e-9 && p[1].abs() <= 1e-9,
        "{p:?}"
    );
    assert_eq!(sol.dof, 0);
}

#[test]
fn a_constraint_without_a_gradient_does_not_derail_the_others() {
    // Found by the random test. A line is never perpendicular to itself:
    // the cosine of the angle is 1 wherever the line goes, so its gradient
    // is zero, in floating point a residue of 1e-16. Taken for a direction,
    // that residue sent the iteration off and got the innocent second
    // constraint blamed.
    let mut s = Sketch::new();
    let a = s.add_point(1.0, -8.0);
    let b = s.add_point(0.7, -8.7);
    let line = s.add_line(a, b);
    let p = s.add_point(-1.0, 7.5);
    s.add_constraint(C::Perpendicular { a: line, b: line });
    s.add_constraint(C::VerticalDistance {
        a,
        b: p,
        value: 8.5,
    });
    let sol = solved(&s);
    assert_eq!(statuses(&sol), [Conflicting, Satisfied]);
    // cos 0 = 1.
    assert!((sol.constraints[0].residual - 1.0).abs() <= 1e-12);
    let (a, p) = (sol.point(a).unwrap(), sol.point(p).unwrap());
    assert!((p[1] - a[1] - 8.5).abs() <= 1e-9);
    // 6 unknowns, one equation in force.
    assert_eq!(sol.dof, 5);
}

#[test]
fn a_far_point_on_a_short_line_does_not_collapse_the_line() {
    // Found by the random test. A slanted segment must become vertical and
    // pass through a point far beyond its end. Solved at once, the nearest
    // "solution" of the linearised equations shrinks the segment to a
    // point; taken in order (vertical first, then the point), nothing
    // needs to shrink.
    let mut s = Sketch::new();
    let a = s.add_point(-1.57, -0.44);
    let b = s.add_point(-3.57, -4.73);
    let line = s.add_line(a, b);
    let p = s.add_point(4.42, 9.47);
    s.add_constraint(C::Vertical { line });
    s.add_constraint(C::PointOnLine { point: p, line });
    let sol = solved(&s);
    assert_eq!(statuses(&sol), [Satisfied, Satisfied]);
    let (a, b, p) = (
        sol.point(a).unwrap(),
        sol.point(b).unwrap(),
        sol.point(p).unwrap(),
    );
    assert!((a[0] - b[0]).abs() <= 1e-9 && (p[0] - a[0]).abs() <= 1e-9);
    // Making a line vertical does not change the heights of its ends
    // (4.29 apart), and nothing else asks them to move.
    assert!((a[1] - b[1] - 4.29).abs() <= 1e-9, "{a:?} {b:?}");
    // 6 unknowns, 2 equations.
    assert_eq!(sol.dof, 4);
}

#[test]
fn the_iteration_is_damped() {
    // p = (1, t) and the line from the origin to p must point along +x:
    // atan(t) = 0. Newton's method on the arctangent overshoots further at
    // every step from |t| > 1.39; from t = 2 only a damped step converges.
    let mut s = Sketch::new();
    let o = s.add_point(0.0, 0.0);
    let a = s.add_point(100.0, 0.0);
    let base = s.add_line(o, a);
    let p = s.add_point(1.0, 2.0);
    let arm = s.add_line(o, p);
    s.add_constraint(C::Fixed { point: o, at: None });
    s.add_constraint(C::Fixed { point: a, at: None });
    s.add_constraint(C::HorizontalDistance {
        a: o,
        b: p,
        value: 1.0,
    });
    s.add_constraint(C::Angle {
        a: base,
        b: arm,
        value: 0.0,
    });
    let sol = solved(&s);
    assert_eq!(statuses(&sol), [Satisfied; 4]);
    let p = sol.point(p).unwrap();
    assert!((p[0] - 1.0).abs() <= 1e-9 && p[1].abs() <= 1e-9, "{p:?}");
    assert!(sol.iterations <= 20, "{} iterations", sol.iterations);
}

#[test]
fn an_empty_sketch_solves() {
    let sol = solved(&Sketch::new());
    assert_eq!(
        (sol.unknowns, sol.rank, sol.dof, sol.iterations),
        (0, 0, 0, 0)
    );
    assert!(sol.entities.is_empty() && sol.constraints.is_empty());
}

fn error(s: &Sketch) -> SolveError {
    solve(s, &options()).unwrap_err()
}

#[test]
fn unknown_ids_are_errors() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    s.add_line(a, EntityId(7));
    assert_eq!(
        error(&s),
        SolveError::UnknownEntity {
            at: Site::Entity(1),
            id: EntityId(7)
        }
    );

    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    s.add_constraint(C::Fixed { point: a, at: None });
    s.add_constraint(C::Coincident {
        a,
        b: EntityId(u32::MAX),
    });
    assert_eq!(
        error(&s),
        SolveError::UnknownEntity {
            at: Site::Constraint(1),
            id: EntityId(u32::MAX)
        }
    );
}

#[test]
fn wrong_kinds_are_errors() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(1.0, 0.0);
    let line = s.add_line(a, b);
    let circle = s.add_circle(a, 1.0);
    let arc = s.add_arc(a, b, b);
    let wrong = |c: C, id: EntityId, found: EntityKind| {
        let mut s = s.clone();
        s.add_constraint(c);
        match error(&s) {
            SolveError::WrongKind {
                at: Site::Constraint(0),
                id: got,
                found: kind,
                ..
            } => assert_eq!((got, kind), (id, found), "{c:?}"),
            other => panic!("{c:?}: {other}"),
        }
    };
    use EntityKind::{Arc, Circle, Line, Point};
    wrong(C::Coincident { a, b: line }, line, Line);
    wrong(C::Horizontal { line: circle }, circle, Circle);
    wrong(C::Vertical { line: a }, a, Point);
    wrong(C::HorizontalPoints { a: arc, b }, arc, Arc);
    wrong(C::VerticalPoints { a, b: line }, line, Line);
    wrong(
        C::PointOnLine {
            point: a,
            line: arc,
        },
        arc,
        Arc,
    );
    wrong(
        C::PointOnCircle {
            point: a,
            circle: line,
        },
        line,
        Line,
    );
    wrong(C::Parallel { a: line, b: circle }, circle, Circle);
    wrong(C::Perpendicular { a, b: line }, a, Point);
    wrong(C::Tangent { a: line, b: line }, line, Line);
    wrong(C::Tangent { a: line, b: a }, a, Point);
    wrong(C::Tangent { a: circle, b }, b, Point);
    wrong(C::Equal { a: line, b: circle }, circle, Circle);
    wrong(C::Equal { a: circle, b: line }, line, Line);
    wrong(C::Equal { a, b }, a, Point);
    wrong(C::Concentric { a: circle, b: line }, line, Line);
    wrong(C::Symmetric { a, b, axis: circle }, circle, Circle);
    wrong(C::Midpoint { point: line, line }, line, Line);
    wrong(
        C::Fixed {
            point: arc,
            at: None,
        },
        arc,
        Arc,
    );
    wrong(
        C::Distance {
            a: line,
            b: line,
            value: 1.0,
        },
        line,
        Line,
    );
    wrong(
        C::Distance {
            a,
            b: circle,
            value: 1.0,
        },
        circle,
        Circle,
    );
    wrong(
        C::HorizontalDistance {
            a,
            b: line,
            value: 1.0,
        },
        line,
        Line,
    );
    wrong(
        C::VerticalDistance {
            a: circle,
            b,
            value: 1.0,
        },
        circle,
        Circle,
    );
    wrong(
        C::Angle {
            a: line,
            b: arc,
            value: 1.0,
        },
        arc,
        Arc,
    );
    wrong(
        C::Radius {
            circle: line,
            value: 1.0,
        },
        line,
        Line,
    );
    wrong(
        C::Diameter {
            circle: a,
            value: 1.0,
        },
        a,
        Point,
    );

    // Entities must be built on points.
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let line = s.add_line(a, a);
    s.add_circle(line, 1.0);
    assert_eq!(
        error(&s),
        SolveError::WrongKind {
            at: Site::Entity(2),
            id: line,
            expected: "point",
            found: Line
        }
    );
}

#[test]
fn values_that_are_not_finite_are_errors() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut s = Sketch::new();
        s.add_point(0.0, 0.0);
        s.add_point(bad, 0.0);
        assert_eq!(
            error(&s),
            SolveError::NonFinite {
                at: Site::Entity(1)
            }
        );

        let mut s = Sketch::new();
        let c = s.add_point(0.0, 0.0);
        s.add_circle(c, bad);
        assert_eq!(
            error(&s),
            SolveError::NonFinite {
                at: Site::Entity(1)
            }
        );

        let mut s = Sketch::new();
        let a = s.add_point(0.0, 0.0);
        let b = s.add_point(1.0, 0.0);
        let line = s.add_line(a, b);
        let circle = s.add_circle(a, 1.0);
        for c in [
            C::Distance { a, b, value: bad },
            C::Distance {
                a,
                b: line,
                value: bad,
            },
            C::HorizontalDistance { a, b, value: bad },
            C::VerticalDistance { a, b, value: bad },
            C::Angle {
                a: line,
                b: line,
                value: bad,
            },
            C::Radius { circle, value: bad },
            C::Diameter { circle, value: bad },
            C::Fixed {
                point: a,
                at: Some([0.0, bad]),
            },
        ] {
            let mut s = s.clone();
            s.add_constraint(C::Horizontal { line });
            s.add_constraint(c);
            assert_eq!(
                error(&s),
                SolveError::NonFinite {
                    at: Site::Constraint(1)
                },
                "{c:?}"
            );
        }
    }
}

#[test]
fn negative_lengths_are_errors() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(1.0, 0.0);
    let line = s.add_line(a, b);
    let circle = s.add_circle(a, 1.0);
    for c in [
        C::Distance { a, b, value: -1.0 },
        C::Distance {
            a: line,
            b,
            value: -1.0,
        },
        C::HorizontalDistance { a, b, value: -1.0 },
        C::VerticalDistance { a, b, value: -1.0 },
        C::Radius {
            circle,
            value: -1.0,
        },
        C::Diameter {
            circle,
            value: -1.0,
        },
    ] {
        let mut s = s.clone();
        s.add_constraint(c);
        assert!(
            matches!(error(&s), SolveError::InvalidValue { constraint: 0, .. }),
            "{c:?}"
        );
    }
    let mut s = Sketch::new();
    let c = s.add_point(0.0, 0.0);
    s.add_circle(c, -1.0);
    assert!(matches!(
        error(&s),
        SolveError::InvalidEntity { entity: 1, .. }
    ));
}

#[test]
fn a_direction_of_a_zero_length_line_is_undefined() {
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(4.0, 0.0);
    let c = s.add_point(2.0, 2.0);
    // Shorter than the linear tolerance of 1e-6.
    let d = s.add_point(2.0 + 1e-7, 2.0);
    let good = s.add_line(a, b);
    let dot = s.add_line(c, d);
    let same = s.add_line(a, a);
    let circle = s.add_circle(c, 1.0);
    for constraint in [
        C::Angle {
            a: good,
            b: dot,
            value: 0.5,
        },
        C::Angle {
            a: same,
            b: good,
            value: 0.5,
        },
        C::Parallel { a: good, b: dot },
        C::Perpendicular { a: dot, b: good },
        C::PointOnLine {
            point: a,
            line: dot,
        },
        C::Tangent { a: dot, b: circle },
        C::Symmetric { a, b, axis: dot },
        C::Distance {
            a,
            b: dot,
            value: 1.0,
        },
    ] {
        let mut s = s.clone();
        s.add_constraint(C::Horizontal { line: good });
        s.add_constraint(constraint);
        // Not an error: the rest of the sketch solves, and the constraint
        // that has no meaning is named, with the reason. (Until the solve
        // was made stable under re-solving this was `Err(Degenerate)`; a
        // constraint blamed for being undefined then turned into an error
        // on the next solve of the same sketch.)
        let sol = solved(&s);
        assert_eq!(statuses(&sol), [Satisfied, Conflicting], "{constraint:?}");
        assert_eq!(sol.constraints[1].residual, f64::INFINITY);
        assert_eq!(
            sol.constraints[1].undefined,
            Some("a line shorter than the linear tolerance has no direction")
        );
        assert_eq!(sol.constraints[0].undefined, None);
        // Nothing moved: the horizontal line already was.
        assert_eq!(sol.entities, s.entities);
    }
    // The same line is fine for constraints that need no direction, and
    // with a tolerance below its length it has one.
    let mut s2 = s.clone();
    s2.add_constraint(C::Horizontal { line: dot });
    s2.add_constraint(C::Equal { a: dot, b: good });
    assert!(solve(&s2, &options()).is_ok());
    let mut s3 = s.clone();
    s3.add_constraint(C::Parallel { a: good, b: dot });
    let fine = SolveOptions::new(Tolerance::new(1e-9, 1e-9).unwrap());
    assert!(solve(&s3, &fine).is_ok());
}

#[test]
fn options_are_checked() {
    let s = Sketch::new();
    let mut o = options();
    o.tolerance.linear = 0.0;
    assert!(matches!(
        solve(&s, &o),
        Err(SolveError::InvalidOptions { .. })
    ));
    let mut o = options();
    o.tolerance.angular = f64::NAN;
    assert!(matches!(
        solve(&s, &o),
        Err(SolveError::InvalidOptions { .. })
    ));
    for bad in [0.0, 1.0, -0.1, f64::NAN, f64::INFINITY] {
        let mut o = options();
        o.rank_threshold = bad;
        assert!(matches!(
            solve(&s, &o),
            Err(SolveError::InvalidOptions { .. })
        ));
    }
}

#[test]
fn limits_are_errors_not_hangs() {
    // Too many entities.
    let mut s = Sketch::new();
    s.entities = vec![Entity::Point { x: 0.0, y: 0.0 }; MAX_ENTITIES + 1];
    assert_eq!(
        error(&s),
        SolveError::TooLarge {
            what: "entities",
            count: MAX_ENTITIES + 1,
            limit: MAX_ENTITIES
        }
    );
    // More unknowns than the solver takes on.
    let mut s = Sketch::new();
    s.entities = vec![Entity::Point { x: 0.0, y: 0.0 }; MAX_ENTITIES];
    assert!(matches!(
        error(&s),
        SolveError::TooLarge {
            what: "unknowns",
            ..
        }
    ));

    // No iterations allowed: an unsolved sketch does not converge.
    let (sketch, _) = common::l_profile(1.0);
    let mut o = options();
    o.max_iterations = 0;
    assert_eq!(
        solve(&sketch, &o),
        Err(SolveError::DidNotConverge { iterations: 0 })
    );
    // ...but a solved one needs none.
    let done = common::resolved(&sketch, &solved(&sketch));
    assert!(solve(&done, &o).is_ok());

    // No arithmetic allowed.
    let mut o = options();
    o.work_limit = 10;
    assert_eq!(
        solve(&sketch, &o),
        Err(SolveError::WorkLimitExceeded { limit: 10 })
    );
}

#[test]
fn errors_explain_themselves() {
    let cases = [
        (
            SolveError::UnknownEntity {
                at: Site::Constraint(3),
                id: EntityId(9),
            },
            "constraint 3 refers to entity 9, which does not exist",
        ),
        (
            SolveError::WrongKind {
                at: Site::Entity(2),
                id: EntityId(1),
                expected: "point",
                found: EntityKind::Line,
            },
            "entity 2 needs entity 1 to be a point, but it is a line",
        ),
        (
            SolveError::DidNotConverge { iterations: 64 },
            "the sketch did not solve within tolerance after 64 iterations",
        ),
    ];
    // No independent oracle for wording: these pin the current messages.
    for (e, text) in cases {
        assert_eq!(e.to_string(), text);
    }
}
