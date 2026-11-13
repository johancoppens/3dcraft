//! A sketch whose solve from its stored solution fails is not stuck there.
//!
//! The solve starts from the stored solution, here one of other values; when
//! it ends in a conflict (and the retry from the held values does too), one
//! last attempt starts with every circle and arc that passes through three
//! held points centred on the circle through them: geometry the values
//! alone fix, but whose equations are not linear in it, so that from a
//! stale place (far off, or on the line of its chord) Newton's method ran
//! its centre away and a constraint was blamed. A solve that succeeds
//! before that attempt is not touched; one that truly conflicts keeps its
//! verdict.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use craft3d_math::{Portable, Tolerance};
use craft3d_solve::{
    Constraint as C, ConstraintStatus, Entity, EntityId, Sketch, Solution, SolveOptions, solve,
};

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

/// The centre of the circle through three points (the oracle: Cramer's
/// rule on two perpendicular bisectors, written here independently).
fn circumcentre(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> [f64; 2] {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    let (a2, b2, c2) = (
        a[0] * a[0] + a[1] * a[1],
        b[0] * b[0] + b[1] * b[1],
        c[0] * c[0] + c[1] * c[1],
    );
    [
        (a2 * (b[1] - c[1]) + b2 * (c[1] - a[1]) + c2 * (a[1] - b[1])) / d,
        (a2 * (c[0] - b[0]) + b2 * (a[0] - c[0]) + c2 * (b[0] - a[0])) / d,
    ]
}

#[derive(Clone, Copy, Debug)]
enum Round {
    /// An arc counter-clockwise from `b` to `a`, a point held on it.
    Arc,
    /// A full circle with its radius free, three points held on it.
    Circle,
}

struct Through {
    sketch: Sketch,
    centre: EntityId,
    round: EntityId,
    points: [EntityId; 3],
}

/// A round through `a`, `b` and `p`, all three held there, its centre drawn
/// at `centre` and (a circle) its radius at `radius`.
fn through(kind: Round, [a, b, p]: [[f64; 2]; 3], centre: [f64; 2], radius: f64) -> Through {
    let mut s = Sketch::new();
    let ia = s.add_point(a[0], a[1]);
    let ib = s.add_point(b[0], b[1]);
    let ic = s.add_point(centre[0], centre[1]);
    let round = match kind {
        Round::Arc => s.add_arc(ic, ib, ia),
        Round::Circle => s.add_circle(ic, radius),
    };
    let ip = s.add_point(p[0], p[1]);
    held(&mut s, ia, a);
    held(&mut s, ib, b);
    if let Round::Circle = kind {
        for point in [ia, ib] {
            s.add_constraint(C::PointOnCircle {
                point,
                circle: round,
            });
        }
    }
    s.add_constraint(C::PointOnCircle {
        point: ip,
        circle: round,
    });
    held(&mut s, ip, p);
    Through {
        sketch: s,
        centre: ic,
        round,
        points: [ia, ib, ip],
    }
}

/// How far an arc turns counter-clockwise from its start to its end, in
/// `[0, 2 pi)`.
fn sweep(solution: &Solution, arc: EntityId) -> f64 {
    let Entity::Arc { center, start, end } = solution.entities[arc.index()] else {
        panic!("not an arc");
    };
    let (c, s, e) = (
        point(solution, center),
        point(solution, start),
        point(solution, end),
    );
    let turn = (e[1] - c[1]).patan2(e[0] - c[0]) - (s[1] - c[1]).patan2(s[0] - c[0]);
    turn.rem_euclid(std::f64::consts::TAU)
}

/// The chord from (-h, 0) to (h, 0) and a third point (0, b): the centre is
/// (0, (b^2 - h^2) / (2 b)), the radius (b^2 + h^2) / (2 |b|), and for
/// |b| < h the centre is on the other side of the chord than the point.
/// Each value re-solved from the solution of the one before, as a document
/// is: flat (a sagitta of 1/200000 of the chord, the centre 2 million
/// half-chords out), back, across the chord and back. Several of these
/// edits failed before (the centre ran away from the stale start). For the
/// arc, across the chord it keeps the way round it was drawn with and turns
/// the long way, 2 pi - 2 asin(h / r): the one solution of its constraints.
/// At three scales, the tolerance the part's own.
#[test]
fn a_round_through_three_held_points_follows_them_flat_and_across() {
    for kind in [Round::Arc, Round::Circle] {
        for k in [1e-3, 1.0, 1e6] {
            let options = SolveOptions::new(Tolerance::new(1e-6 * k, 1e-9).unwrap());
            let h = 20.0 * k;
            let centre = |b: f64| [0.0, (b * b - h * h) / (2.0 * b)];
            let radius = |b: f64| (b * b + h * h) / (2.0 * b.abs());
            let at = |b: f64| [[-h, 0.0], [h, 0.0], [0.0, b]];
            let b0 = 6.0 * k;
            let built = through(kind, at(b0), centre(b0), radius(b0));
            let mut last = solved_with(&built.sketch, &options);
            for (step, bulge) in [6.0, 1e-4, 6.0, -3.0, 6.0, 1e-4, -3.0, -0.5, 6.0]
                .into_iter()
                .enumerate()
            {
                let b = bulge * k;
                let what = format!("{kind:?}, scale {k}, step {step}: bulge {b}");
                let changed = through(kind, at(b), centre(b0), radius(b0));
                let solution = solved_with(&starting_at(&changed.sketch, &last), &options);
                assert!(
                    solution.is_consistent(),
                    "{what}: {:?}",
                    solution.constraints
                );
                for (id, place) in built.points.iter().zip(at(b)) {
                    let [x, y] = point(&solution, *id);
                    assert!(
                        (x - place[0]).abs() <= 1e-6 * k && (y - place[1]).abs() <= 1e-6 * k,
                        "{what}: held at ({x}, {y}), not {place:?}"
                    );
                }
                let r = radius(b);
                let [cx, cy] = point(&solution, built.centre);
                let [_, ey] = centre(b);
                assert!(
                    cx.abs() <= 1e-9 * r && (cy - ey).abs() <= 1e-9 * r,
                    "{what}: centre ({cx}, {cy}), closed form (0, {ey})"
                );
                assert_eq!(cy < 0.0, b > 0.0, "{what}: centre at y = {cy}");
                let got = solution.radius(built.round).unwrap();
                assert!((got - r).abs() <= 1e-9 * r, "{what}: radius {got}, {r}");
                if let Round::Arc = kind {
                    let minor = 2.0 * (h / r).pasin();
                    let expected = if b > 0.0 {
                        minor
                    } else {
                        std::f64::consts::TAU - minor
                    };
                    let turn = sweep(&solution, built.round);
                    assert!(
                        (turn - expected).abs() <= 1e-9,
                        "{what}: the arc turns {turn}, expected {expected}"
                    );
                }
                // Solving the result again moves nothing.
                let again = solved_with(&starting_at(&changed.sketch, &solution), &options);
                assert_eq!(again.entities, solution.entities, "{what}");
                last = solution;
            }
        }
    }
}

/// Two arcs drawn through points, a shallow cap and a half circle, built at
/// one size and given another in one solve: most points move by more than
/// the arcs' radii, the half circle's along its own chord, so that its old
/// centre lies on the line of its new chord beyond its ends. The solve from
/// the old solution ran the cap's centre a million units off and blamed a
/// held coordinate, the retry from the held values ran the half circle's
/// off; now both centres are the circumcentres of their held points, as
/// from a fresh start, at three scales.
#[test]
fn arcs_moved_far_along_their_chords_are_found_again() {
    // [end, start, through] of the cap and of the half circle, at the
    // first size and at the second.
    let sizes: [[[f64; 2]; 6]; 2] = [
        [
            [0.5, 312.3259849173512],
            [10.08, 371.8259849173512],
            [2.895, 342.0759849173512],
            [5.0, 487.3259849173512],
            [5.0, 494.3259849173512],
            [1.5, 490.8259849173512],
        ],
        [
            [15.521, 267.77805096117135],
            [25.16, 289.19805096117136],
            [17.93075, 278.4880509611713],
            [5.0, 330.77805096117135],
            [5.0, 337.77805096117135],
            [1.5, 334.27805096117135],
        ],
    ];
    for k in [1e-3, 1.0, 1e6] {
        let options = SolveOptions::new(Tolerance::new(1e-6 * k, 1e-9).unwrap());
        let scaled = |size: usize| sizes[size].map(|[x, y]| [x * k, y * k]);
        let sketch = |size: usize, drawn: usize| {
            let (at, start) = (scaled(size), scaled(drawn));
            let mut s = Sketch::new();
            let mut ids = Vec::new();
            for arc in 0..2 {
                let [e, b, p] = [at[3 * arc], at[3 * arc + 1], at[3 * arc + 2]];
                let [de, db, dp] = [start[3 * arc], start[3 * arc + 1], start[3 * arc + 2]];
                let centre = circumcentre(de, db, dp);
                let ie = s.add_point(de[0], de[1]);
                let ib = s.add_point(db[0], db[1]);
                let ic = s.add_point(centre[0], centre[1]);
                let curve = s.add_arc(ic, ib, ie);
                let ip = s.add_point(dp[0], dp[1]);
                held(&mut s, ie, e);
                held(&mut s, ib, b);
                s.add_constraint(C::PointOnCircle {
                    point: ip,
                    circle: curve,
                });
                held(&mut s, ip, p);
                ids.push(ic);
            }
            (s, ids)
        };
        let (first, _) = sketch(0, 0);
        let first = solved_with(&first, &options);
        assert!(first.is_consistent(), "scale {k}");
        let (changed, centres) = sketch(1, 0);
        let edited = solved_with(&starting_at(&changed, &first), &options);
        let (fresh, _) = sketch(1, 1);
        let fresh = solved_with(&fresh, &options);
        let at = scaled(1);
        for (n, solution) in [("edited", &edited), ("fresh", &fresh)] {
            let what = format!("scale {k}, {n}");
            assert!(
                solution.is_consistent(),
                "{what}: {:?}",
                solution.constraints
            );
            for (arc, centre) in centres.iter().enumerate() {
                let [cx, cy] = point(solution, *centre);
                let [ex, ey] = circumcentre(at[3 * arc], at[3 * arc + 1], at[3 * arc + 2]);
                let size = 500.0 * k;
                assert!(
                    (cx - ex).abs() <= 1e-12 * size && (cy - ey).abs() <= 1e-12 * size,
                    "{what}: centre {arc} at ({cx}, {cy}), circumcentre ({ex}, {ey})"
                );
            }
        }
    }
}

/// A real conflict keeps its verdict: an arc through three held points
/// given a radius it cannot have conflicts from a fresh start and from a
/// stale one far off, where the last attempt (the centre on the circle
/// through the held points) is made and has the conflict too, so the
/// verdict of before stands.
#[test]
fn a_real_conflict_stays_a_conflict() {
    let options = common::options();
    // A radius of 50 for the circle through (-20, 0), (20, 0) and (0, 6),
    // whose radius is 36.33.
    let mut built = through(
        Round::Arc,
        [[-20.0, 0.0], [20.0, 0.0], [0.0, 6.0]],
        [0.0, -30.0],
        0.0,
    );
    let radius = built.sketch.add_constraint(C::Radius {
        circle: built.round,
        value: 50.0,
    });
    let solution = solved_with(&built.sketch, &options);
    assert_eq!(
        solution.constraints[radius].status,
        ConstraintStatus::Conflicting,
        "{:?}",
        solution.constraints
    );
    // The same from a stale start far below.
    let mut stale = built.sketch.clone();
    stale.entities[built.centre.index()] = Entity::Point { x: 3.0, y: -1e5 };
    let solution = solved_with(&stale, &options);
    assert!(!solution.is_consistent(), "{:?}", solution.constraints);
}
