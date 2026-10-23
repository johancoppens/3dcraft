//! An arc between held points, re-solved after the values that hold them
//! change (a Jig, 2026-10-23: ends at `r cos a`, `r sin a` and at `3 a`,
//! the angle changed from 5 to 90 degrees). The arc's own equation (its
//! ends equally far from its centre) comes first and is not linear in the
//! ends: from where the ends were solved before, it put the second
//! coordinate of an end on the old side of the circle, and that
//! coordinate's constraint was blamed. The oracle: the re-solve gives what
//! a fresh solve from the held values gives (the same verdict, the points
//! where they are held), with no conflict, at several scales, angles and
//! steps; and solving it again moves nothing, bit for bit.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{resolved, solved, statuses};
use craft3d_solve::ConstraintStatus::Conflicting;
use craft3d_solve::{Constraint as C, Entity, EntityId, Sketch, Solution};

/// How the ends of the arc are held.
#[derive(Clone, Copy, Debug)]
enum Hold {
    /// An `x` and a `y` constraint each, as sketch.point holds a point.
    Coordinates,
    /// `Fixed` at a place, as the end of a formula curve is held.
    Fixed,
}

/// A centre held at the origin, and an arc about it from `start` to `end`,
/// both held where `at` says; the points are drawn at `drawn`.
fn arc(hold: Hold, at: [[f64; 2]; 2], drawn: [[f64; 2]; 2]) -> (Sketch, [EntityId; 3]) {
    let mut s = Sketch::new();
    let c = s.add_point(0.0, 0.0);
    let ends = drawn.map(|[x, y]| s.add_point(x, y));
    s.add_constraint(C::CoordinateX {
        point: c,
        value: 0.0,
    });
    s.add_constraint(C::CoordinateY {
        point: c,
        value: 0.0,
    });
    for (point, [x, y]) in ends.into_iter().zip(at) {
        match hold {
            Hold::Coordinates => {
                s.add_constraint(C::CoordinateX { point, value: x });
                s.add_constraint(C::CoordinateY { point, value: y });
            }
            Hold::Fixed => {
                s.add_constraint(C::Fixed {
                    point,
                    at: Some([x, y]),
                });
            }
        }
    }
    s.add_arc(c, ends[0], ends[1]);
    (s, [c, ends[0], ends[1]])
}

fn ends(r: f64, a: f64, factor: f64, symmetric: bool) -> [[f64; 2]; 2] {
    let at = |deg: f64| {
        let t = deg.to_radians();
        [r * t.cos(), r * t.sin()]
    };
    if symmetric {
        [at(-a), at(a)]
    } else {
        [at(a), at(factor * a)]
    }
}

fn bits(solution: &Solution) -> Vec<u64> {
    solution
        .entities
        .iter()
        .flat_map(|e| match e {
            Entity::Point { x, y } => vec![x.to_bits(), y.to_bits()],
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn a_re_solve_after_the_held_values_change_matches_a_fresh_solve() {
    let mut cases = 0;
    for hold in [Hold::Coordinates, Hold::Fixed] {
        for r in [1e-3, 1.0, 25.4, 1e3, 1e6] {
            for symmetric in [false, true] {
                for (a0, a1) in [
                    (5.0, 12.0),
                    (5.0, 20.0),
                    (5.0, 90.0),
                    (5.0, 170.0),
                    (40.0, 3.0),
                    (60.0, 100.0),
                ] {
                    let before = ends(r, a0, 3.0, symmetric);
                    let after = ends(r, a1, 3.0, symmetric);
                    // Built at a0 and solved there.
                    let (built, _) = arc(hold, before, before);
                    let first = solved(&built);
                    assert!(first.is_consistent());
                    // The values change; the points start where they were.
                    let (changed, ids) = arc(hold, after, before);
                    let again = solved(&resolved(&changed, &first));
                    // A fresh build at a1.
                    let (fresh, _) = arc(hold, after, after);
                    let fresh = solved(&fresh);
                    let what = format!("{hold:?} r {r} symmetric {symmetric} {a0} -> {a1}");
                    assert!(
                        !statuses(&again).contains(&Conflicting),
                        "{what}: {:?}",
                        again.constraints
                    );
                    assert_eq!(statuses(&again), statuses(&fresh), "{what}");
                    for (id, [x, y]) in ids[1..].iter().zip(after) {
                        let Entity::Point { x: sx, y: sy } = again.entities[id.index()] else {
                            panic!("{what}: not a point");
                        };
                        assert!(
                            (sx - x).abs() <= 1e-9 * r && (sy - y).abs() <= 1e-9 * r,
                            "{what}: ({sx}, {sy}) for ({x}, {y})"
                        );
                    }
                    // And it stays: solving the result again moves nothing.
                    let third = solved(&resolved(&changed, &again));
                    assert_eq!(bits(&third), bits(&again), "{what}");
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 2 * 5 * 2 * 6);
}

/// The retry starts from the held values only for a sketch that has a
/// conflict without it: a real conflict stays one (two `x` constraints on
/// one point that disagree), with the same verdict as before.
#[test]
fn a_real_conflict_is_still_reported() {
    let mut s = Sketch::new();
    let p = s.add_point(1.0, 1.0);
    s.add_constraint(C::CoordinateX {
        point: p,
        value: 2.0,
    });
    s.add_constraint(C::CoordinateX {
        point: p,
        value: 3.0,
    });
    let solution = solved(&s);
    assert_eq!(statuses(&solution)[1], Conflicting);
    let Entity::Point { x, .. } = solution.entities[0] else {
        panic!("not a point");
    };
    assert_eq!(x, 2.0);
}

/// An arc whose ends are held at `at` and drawn at `drawn`, its centre free
/// and drawn at `centre`, its radius a dimension `r`.
fn free_centre(r: f64, at: [[f64; 2]; 2], drawn: [[f64; 2]; 2], centre: [f64; 2]) -> Sketch {
    let mut s = Sketch::new();
    let c = s.add_point(centre[0], centre[1]);
    let ends = drawn.map(|[x, y]| s.add_point(x, y));
    for (point, [x, y]) in ends.into_iter().zip(at) {
        s.add_constraint(C::CoordinateX { point, value: x });
        s.add_constraint(C::CoordinateY { point, value: y });
    }
    let arc = s.add_arc(c, ends[0], ends[1]);
    s.add_constraint(C::Radius {
        circle: arc,
        value: r,
    });
    s
}

/// Built at `a0` (centre drawn at the origin), then the angle set to `a1`:
/// the edited solve, and the fresh solve of the same document (every point
/// where the document has it, the held coordinates at their values).
fn edited_and_fresh(r: f64, a0: f64, a1: f64) -> (Solution, Solution) {
    let before = ends(r, a0, 3.0, false);
    let after = ends(r, a1, 3.0, false);
    let first = solved(&free_centre(r, before, before, [0.0, 0.0]));
    let Entity::Point { x: cx, y: cy } = first.entities[0] else {
        panic!("not a point");
    };
    assert!(cx.hypot(cy) <= 1e-9 * r, "built at ({cx}, {cy})");
    let edited = solved(&resolved(
        &free_centre(r, after, before, [0.0, 0.0]),
        &first,
    ));
    let fresh = solved(&free_centre(r, after, after, [cx, cy]));
    (edited, fresh)
}

fn assert_same_place(edited: &Solution, fresh: &Solution, r: f64, what: &str) {
    assert!(
        !statuses(edited).contains(&Conflicting),
        "{what}: {:?}",
        edited.constraints
    );
    assert_eq!(statuses(edited), statuses(fresh), "{what}");
    for (a, b) in edited.entities.iter().zip(&fresh.entities) {
        if let (Entity::Point { x, y }, Entity::Point { x: fx, y: fy }) = (a, b) {
            assert!(
                (x - fx).abs() <= 1e-9 * r && (y - fy).abs() <= 1e-9 * r,
                "{what}: ({x}, {y}) against a fresh ({fx}, {fy})"
            );
        }
    }
}

/// A free centre: the arc's ends are held, its radius is a dimension, so
/// the centre has two places, one on either side of the chord. After the
/// angle changes, the edited sketch solves to what a fresh solve of the
/// same document gives: the centre stays on the side it was on. (Not 90
/// degrees: its ends are a diameter apart, one centre, a redundant radius.)
#[test]
fn a_free_centre_after_a_jump_is_where_a_fresh_solve_puts_it() {
    for r in [1e-3, 1.0, 1e3, 1e6] {
        for (a0, a1) in [
            (5.0, 20.0),
            (5.0, 45.0),
            (5.0, 80.0),
            (10.0, 60.0),
            (40.0, 3.0),
            (20.0, 5.0),
        ] {
            let (edited, fresh) = edited_and_fresh(r, a0, a1);
            assert_same_place(&edited, &fresh, r, &format!("r {r} {a0} -> {a1}"));
        }
    }
}

/// Known, open: a larger jump with a free centre solves without a conflict
/// but with the centre on the mirror side of the chord, where a fresh
/// solve of the same document keeps it on its side. The retry from the
/// held values runs only after a conflict, so it does not catch this;
/// starting every solve with the held coordinates at their values would
/// (docs/design/document.md, 5.4, *Held values that jump*).
#[test]
#[ignore = "open: a free centre flips to its mirror without a conflict"]
fn a_free_centre_after_a_larger_jump_stays_on_its_side() {
    for (a0, a1) in [(5.0, 120.0), (5.0, 170.0), (60.0, 100.0)] {
        let (edited, fresh) = edited_and_fresh(1.0, a0, a1);
        assert_same_place(&edited, &fresh, 1.0, &format!("{a0} -> {a1}"));
    }
}
