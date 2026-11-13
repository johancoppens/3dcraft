//! A coordinate held at a value by an independent row ends on that value
//! to the last bit after every solve that converges, whatever solve ended
//! there before: the same values give the same held points, a thousand
//! re-solves do not wander, and a point no constraint holds at a value is
//! the same root within rounding. A coordinate whose holder is redundant
//! is within the tolerance of its value, not on it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use craft3d_solve::{Constraint as C, Entity, EntityId, Sketch, Solution, SolveOptions, solve};

fn solved_with(sketch: &Sketch, options: &SolveOptions) -> Solution {
    match solve(sketch, options) {
        Ok(s) => s,
        Err(e) => panic!("solve failed: {e}"),
    }
}

/// `sketch` starting from the coordinates of `from` (the previous solution,
/// as the document keeps it), its constraints and values its own.
fn starting_at(sketch: &Sketch, from: &Solution) -> Sketch {
    Sketch {
        entities: from.entities.clone(),
        constraints: sketch.constraints.clone(),
        splines: sketch.splines.clone(),
        image_starts: Vec::new(),
    }
}

fn point(solution: &Solution, id: EntityId) -> [f64; 2] {
    solution.point(id).unwrap()
}

fn held(s: &mut Sketch, point: EntityId, [x, y]: [f64; 2]) {
    s.add_constraint(C::CoordinateX { point, value: x });
    s.add_constraint(C::CoordinateY { point, value: y });
}

/// An arc from `a` to `b` drawn through `p` above the chord, as a path from
/// `a` to `b` through `p` draws it: counter-clockwise from `b` to `a`, its
/// centre drawn at `centre`. The ends and `p` held at their places.
struct Through {
    sketch: Sketch,
    centre: EntityId,
    a: EntityId,
    b: EntityId,
    p: EntityId,
}

fn through(a: [f64; 2], b: [f64; 2], p: [f64; 2], centre: [f64; 2]) -> Through {
    let mut s = Sketch::new();
    let ia = s.add_point(a[0], a[1]);
    let ib = s.add_point(b[0], b[1]);
    let ic = s.add_point(centre[0], centre[1]);
    let arc = s.add_arc(ic, ib, ia);
    let ip = s.add_point(p[0], p[1]);
    held(&mut s, ia, a);
    held(&mut s, ib, b);
    s.add_constraint(C::PointOnCircle {
        point: ip,
        circle: arc,
    });
    held(&mut s, ip, p);
    Through {
        sketch: s,
        centre: ic,
        a: ia,
        b: ib,
        p: ip,
    }
}

fn bits(p: [f64; 2]) -> [u64; 2] {
    p.map(f64::to_bits)
}

/// The corners of a rail section, arcs of radius 1 whose centres and ends
/// are held at values of the section's width (as a path of expressions
/// draws them), re-solved a thousand times as the width goes from 50 to
/// 113.15 and back: every held coordinate is its value to the last bit
/// each time, so each return gives the sketch it gave before, bit for bit,
/// and at the width of 50 every radius is 1 exactly (50 - 49 in floating
/// point). Before, each solve started where the last one ended and left the
/// held coordinates wherever Newton's method stopped within its polishing
/// floor: the radii drifted a few units in the last place below 1.
#[test]
fn a_thousand_re_solves_leave_held_arcs_where_they_are_held() {
    let (hh, rail, wall, rr) = (50.0, 8.5, 1.5, 1.0);
    // [centre, start, end] of the three corners at half width `hw`.
    let corners = |hw: f64| -> [[[f64; 2]; 3]; 3] {
        [
            [
                [hw - rr, hh - rail + rr],
                [hw - rr, hh - rail],
                [hw, hh - rail + rr],
            ],
            [[hw - rr, hh - rr], [hw, hh - rr], [hw - rr, hh]],
            [
                [hw - rail + rr, hh - rr],
                [hw - rail + rr, hh],
                [hw - rail, hh - rr],
            ],
        ]
    };
    let sketch = |hw: f64| {
        let mut s = Sketch::new();
        let mut arcs = Vec::new();
        for [c, a, b] in corners(hw) {
            let (ic, ia, ib) = (
                s.add_point(c[0], c[1]),
                s.add_point(a[0], a[1]),
                s.add_point(b[0], b[1]),
            );
            arcs.push(s.add_arc(ic, ia, ib));
            held(&mut s, ic, c);
            held(&mut s, ia, a);
            held(&mut s, ib, b);
        }
        // A wall point held by a value no arc shares, for good measure.
        let w = s.add_point(hw - wall, hh - wall);
        held(&mut s, w, [hw - wall, hh - wall]);
        (s, arcs)
    };
    let options = common::options();
    let widths = [50.0, 113.15];
    let (start, arcs) = sketch(widths[0]);
    let mut last = solved_with(&start, &options);
    let mut first: [Option<Vec<Entity>>; 2] = [None, None];
    for round in 0..1000 {
        let which = round % 2;
        let hw = widths[which];
        let (changed, _) = sketch(hw);
        let solution = solved_with(&starting_at(&changed, &last), &options);
        let what = format!("round {round}, half width {hw}");
        assert!(solution.is_consistent(), "{what}");
        for (n, [c, a, b]) in corners(hw).into_iter().enumerate() {
            let Entity::Arc { center, start, end } = solution.entities[arcs[n].index()] else {
                panic!("{what}: not an arc");
            };
            assert_eq!(bits(point(&solution, center)), bits(c), "{what}");
            assert_eq!(bits(point(&solution, start)), bits(a), "{what}");
            assert_eq!(bits(point(&solution, end)), bits(b), "{what}");
            if which == 0 {
                assert_eq!(solution.radius(arcs[n]), Some(1.0), "{what}: corner {n}");
            }
        }
        match &first[which] {
            Some(entities) => assert_eq!(&solution.entities, entities, "{what}"),
            None => first[which] = Some(solution.entities.clone()),
        }
        last = solution;
    }
}

/// The same final values reached by different edits: held points are the
/// same to the last bit; the centre, which no constraint holds at a value,
/// is the same point within rounding (a few units in the last place of the
/// radius, not bit for bit: Newton's last step from another start rounds
/// differently), and both are the closed form.
#[test]
fn the_same_values_by_different_edits_give_the_same_arc() {
    let options = common::options();
    let h = 20.0;
    let centre = |b: f64| [0.0, (b * b - h * h) / (2.0 * b)];
    let build = |b: f64| through([-h, 0.0], [h, 0.0], [0.0, b], centre(b));
    let edit = |path: &[f64]| -> Solution {
        let mut last = solved_with(&build(path[0]).sketch, &options);
        for b in &path[1..] {
            let changed = through([-h, 0.0], [h, 0.0], [0.0, *b], centre(path[0]));
            last = solved_with(&starting_at(&changed.sketch, &last), &options);
        }
        last
    };
    let ids = build(4.0);
    let fresh = solved_with(&ids.sketch, &options);
    let paths: [&[f64]; 4] = [
        &[6.0, 4.0],
        &[6.0, 2.0, 4.0],
        &[6.0, 1e-4, -3.0, 4.0],
        &[0.5, -7.0, 19.0, 4.0],
    ];
    let r = (16.0 + h * h) / 8.0;
    for path in paths {
        let solution = edit(path);
        let what = format!("{path:?}");
        assert!(solution.is_consistent(), "{what}");
        for id in [ids.a, ids.b, ids.p] {
            assert_eq!(
                bits(point(&solution, id)),
                bits(point(&fresh, id)),
                "{what}"
            );
        }
        let [cx, cy] = point(&solution, ids.centre);
        let [fx, fy] = point(&fresh, ids.centre);
        let [ex, ey] = centre(4.0);
        for (x, y) in [(cx, cy), (fx, fy)] {
            assert!(
                (x - ex).abs() <= 1e-13 * r && (y - ey).abs() <= 1e-13 * r,
                "{what}: ({x}, {y}) against ({ex}, {ey})"
            );
        }
    }
}

/// What "held exactly" does not cover: a coordinate whose holder is not
/// independent. q.x is held at 5, p is kept straight above q (vertical),
/// and p.x is held at 5 + 4e-7: that holder repeats what the other two
/// already decide, within the tolerance (redundant). p.x starts on its
/// value (it is within the tolerance of it) and the vertical row takes it
/// back to q's 5; the holder holds within the tolerance, not to the bit.
/// The result is stable: solved again, it is the same, bit for bit.
#[test]
fn a_redundant_holder_is_met_within_the_tolerance_not_exactly() {
    use craft3d_solve::ConstraintStatus;
    let options = common::options();
    let mut s = Sketch::new();
    let q = s.add_point(5.0, 0.0);
    let p = s.add_point(5.0, 3.0);
    s.add_constraint(C::CoordinateX {
        point: q,
        value: 5.0,
    });
    s.add_constraint(C::VerticalPoints { a: q, b: p });
    s.add_constraint(C::CoordinateX {
        point: p,
        value: 5.0 + 4e-7,
    });
    let solution = solved_with(&s, &options);
    let statuses: Vec<ConstraintStatus> = solution.constraints.iter().map(|c| c.status).collect();
    assert_eq!(
        statuses,
        [
            ConstraintStatus::Satisfied,
            ConstraintStatus::Satisfied,
            ConstraintStatus::Redundant
        ]
    );
    let (qx, px) = (point(&solution, q)[0], point(&solution, p)[0]);
    assert_eq!(qx, 5.0);
    assert!((px - 5.0).abs() <= 1e-12, "{px}");
    let again = solved_with(&starting_at(&s, &solution), &options);
    assert_eq!(again.entities, solution.entities);
}
