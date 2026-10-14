//! A point on a spline: it lands on the curve, takes one degree of
//! freedom, stays on when the curve is dragged, and is named when it is
//! redundant or cannot be.
//!
//! "On the curve" is judged without the solver's own arithmetic: the
//! spline is evaluated by de Boor's algorithm at 100 000 places and the
//! nearest is taken (the solver uses basis functions and a bisection).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{resolved, statuses};
use craft3d_math::Tolerance;
use craft3d_solve::ConstraintStatus::{Conflicting, Redundant, Satisfied};
use craft3d_solve::{
    Constraint as C, EntityId, MAX_SPLINE_DEGREE, Sketch, Solution, SolveError, SolveOptions, solve,
};

type P = [f64; 2];

const WAVE: [P; 7] = [
    [0.0, 0.0],
    [1.0, 2.0],
    [2.5, -1.0],
    [4.0, 3.0],
    [5.0, 0.5],
    [6.5, 1.0],
    [8.0, 0.0],
];

fn knots(degree: usize, count: usize) -> Vec<f64> {
    let spans = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|i| i as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    knots
}

fn de_boor(degree: usize, knots: &[f64], control: &[P], u: f64) -> P {
    let n = control.len();
    let k = (degree..n).rev().find(|&k| knots[k] <= u).unwrap_or(degree);
    let mut d: Vec<P> = control[k - degree..=k].to_vec();
    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = k - degree + j;
            let alpha = (u - knots[i]) / (knots[i + degree + 1 - r] - knots[i]);
            d[j] = [
                d[j - 1][0] + (d[j][0] - d[j - 1][0]) * alpha,
                d[j - 1][1] + (d[j][1] - d[j - 1][1]) * alpha,
            ];
        }
    }
    d[degree]
}

/// How far `p` is from the spline on `control`, to about the square of a
/// hundred-thousandth of its length.
fn off_curve(degree: usize, control: &[P], p: P) -> f64 {
    let knots = knots(degree, control.len());
    // A polyline is its legs exactly; chords would cut its corners.
    let steps = if degree == 1 {
        control.len() - 1
    } else {
        100_000
    };
    let mut least = f64::INFINITY;
    let mut before = control[0];
    for i in 1..=steps {
        let u = (i as f64 / steps as f64).min(1.0 - 1e-15);
        let here = if degree == 1 {
            control[i]
        } else {
            de_boor(degree, &knots, control, u)
        };
        // To the chord from the place before to this one.
        let d = [here[0] - before[0], here[1] - before[1]];
        let l2 = d[0] * d[0] + d[1] * d[1];
        let t = if l2 > 0.0 {
            (((p[0] - before[0]) * d[0] + (p[1] - before[1]) * d[1]) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        least = least.min((p[0] - before[0] - d[0] * t).hypot(p[1] - before[1] - d[1] * t));
        before = here;
    }
    least
}

struct Drawn {
    sketch: Sketch,
    control: Vec<EntityId>,
    spline: usize,
}

/// The wave of `degree`, `k` times as large, as free points with a spline
/// on them.
fn wave(degree: usize, k: f64) -> Drawn {
    let mut sketch = Sketch::new();
    let control: Vec<EntityId> = WAVE
        .iter()
        .map(|p| sketch.add_point(p[0] * k, p[1] * k))
        .collect();
    let spline = sketch.add_spline(degree, knots(degree, WAVE.len()), control.clone());
    Drawn {
        sketch,
        control,
        spline,
    }
}

fn options(k: f64) -> SolveOptions {
    SolveOptions::new(Tolerance::new(1e-6 * k, 1e-9).unwrap())
}

fn places(solution: &Solution, ids: &[EntityId]) -> Vec<P> {
    ids.iter().map(|id| solution.point(*id).unwrap()).collect()
}

#[test]
fn a_point_lands_on_a_fixed_spline_and_keeps_one_freedom() {
    for k in [1e-3, 1.0, 1e6] {
        for degree in 1..=6 {
            for start in [[2.0, 3.0], [4.2, -2.0], [6.0, 2.5], [3.1, 0.7], [0.4, 1.9]] {
                let name = format!("degree {degree} x{k} from {start:?}");
                let Drawn {
                    mut sketch,
                    control,
                    spline,
                } = wave(degree, k);
                for id in &control {
                    sketch.add_constraint(C::Fixed {
                        point: *id,
                        at: None,
                    });
                }
                let point = sketch.add_point(start[0] * k, start[1] * k);
                let on = sketch.add_constraint(C::PointOnSpline { point, spline });
                let solution =
                    solve(&sketch, &options(k)).unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(solution.constraints[on].status, Satisfied, "{name}");
                // The curve did not move, and the point is on it.
                let curve = places(&solution, &control);
                for (now, drawn) in curve.iter().zip(&WAVE) {
                    assert_eq!(*now, [drawn[0] * k, drawn[1] * k], "{name}");
                }
                let at = solution.point(point).unwrap();
                assert!(off_curve(degree, &curve, at) <= 1e-6 * k, "{name}: {at:?}");
                // It took the shortest way there: no further than it was.
                let moved = (at[0] - start[0] * k).hypot(at[1] - start[1] * k);
                assert!(
                    moved
                        <= off_curve(degree, &curve, [start[0] * k, start[1] * k]) * 1.0001
                            + 1e-6 * k,
                    "{name}"
                );
                // Two unknowns of the point, one equation.
                assert_eq!(solution.dof, 1, "{name}");
                // Solving the solution again leaves it alone.
                let again = solve(&resolved(&sketch, &solution), &options(k)).unwrap();
                assert_eq!(again.entities, solution.entities, "{name}");
            }
        }
    }
}

#[test]
fn the_degrees_of_freedom_are_one_less_per_point_on_a_spline() {
    for degree in [2, 3, 5] {
        let Drawn {
            mut sketch,
            control,
            spline,
        } = wave(degree, 1.0);
        let free = solve(&sketch, &options(1.0)).unwrap();
        assert_eq!(free.dof, 2 * WAVE.len());
        let mut expected = free.dof;
        for start in [[2.0, 3.0], [4.2, -2.0], [6.0, 2.5]] {
            let point = sketch.add_point(start[0], start[1]);
            expected += 2;
            assert_eq!(solve(&sketch, &options(1.0)).unwrap().dof, expected);
            sketch.add_constraint(C::PointOnSpline { point, spline });
            expected -= 1;
            let solution = solve(&sketch, &options(1.0)).unwrap();
            assert_eq!(solution.dof, expected, "degree {degree}");
            assert!(statuses(&solution).iter().all(|s| *s == Satisfied));
            let curve = places(&solution, &control);
            assert!(off_curve(degree, &curve, solution.point(point).unwrap()) <= 1e-6);
        }
    }
}

#[test]
fn a_spline_is_pulled_through_a_fixed_point() {
    for k in [1e-3, 1.0, 1e6] {
        for degree in [2, 3, 4] {
            let name = format!("degree {degree} x{k}");
            let Drawn {
                mut sketch,
                control,
                spline,
            } = wave(degree, k);
            // The ends stay; the points in between may move.
            for id in [control[0], control[6]] {
                sketch.add_constraint(C::Fixed {
                    point: id,
                    at: None,
                });
            }
            let target = [3.0 * k, 2.0 * k];
            let point = sketch.add_point(target[0], target[1]);
            sketch.add_constraint(C::Fixed { point, at: None });
            sketch.add_constraint(C::PointOnSpline { point, spline });
            let solution = solve(&sketch, &options(k)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                statuses(&solution).iter().all(|s| *s == Satisfied),
                "{name}"
            );
            assert_eq!(solution.point(point).unwrap(), target, "{name}");
            let curve = places(&solution, &control);
            assert!(off_curve(degree, &curve, target) <= 1e-6 * k, "{name}");
            // Five free control points, one equation.
            assert_eq!(solution.dof, 9, "{name}");
        }
    }
}

#[test]
fn a_point_stays_on_a_spline_that_is_dragged() {
    for k in [1e-3, 1.0, 1e6] {
        let degree = 3;
        let Drawn {
            mut sketch,
            control,
            spline,
        } = wave(degree, k);
        let mut pins = Vec::new();
        for id in &control {
            pins.push(sketch.add_constraint(C::Fixed {
                point: *id,
                at: None,
            }));
        }
        let point = sketch.add_point(3.0 * k, 2.0 * k);
        sketch.add_constraint(C::PointOnSpline { point, spline });
        // The point stays above one place: with the curve it is then
        // held, and goes up and down with it.
        let below = sketch.add_point(3.4 * k, 0.0);
        sketch.add_constraint(C::Fixed {
            point: below,
            at: None,
        });
        sketch.add_constraint(C::VerticalPoints { a: below, b: point });
        let mut solution = solve(&sketch, &options(k)).unwrap();
        assert_eq!(solution.dof, 0, "x{k}");
        let mut seen = Vec::new();
        // The fourth control point is moved up and down, as a dimension
        // on it would move it; each solve starts from the one before.
        for lift in [3.0, 3.6, 4.4, 2.2, 1.6, 3.0] {
            let mut next = resolved(&sketch, &solution);
            next.constraints[pins[3]] = C::Fixed {
                point: control[3],
                at: Some([4.0 * k, lift * k]),
            };
            solution =
                solve(&next, &options(k)).unwrap_or_else(|e| panic!("x{k} lift {lift}: {e}"));
            assert!(
                statuses(&solution).iter().all(|s| *s == Satisfied),
                "x{k} lift {lift}"
            );
            let curve = places(&solution, &control);
            assert_eq!(curve[3], [4.0 * k, lift * k]);
            let at = solution.point(point).unwrap();
            assert!(
                off_curve(degree, &curve, at) <= 1e-6 * k,
                "x{k} lift {lift}"
            );
            assert!((at[0] - 3.4 * k).abs() <= 1e-6 * k, "x{k} lift {lift}");
            seen.push(at);
        }
        // It moved with the curve, and came back with it.
        assert!((seen[2][1] - seen[0][1]).abs() > 1e-2 * k, "x{k}: {seen:?}");
        assert!(
            (seen[5][1] - seen[0][1]).abs() <= 1e-6 * k,
            "x{k}: {seen:?}"
        );
    }
}

#[test]
fn twice_on_the_same_spline_is_redundant_and_off_a_fixed_one_conflicts() {
    let Drawn {
        mut sketch,
        control,
        spline,
    } = wave(3, 1.0);
    for id in &control {
        sketch.add_constraint(C::Fixed {
            point: *id,
            at: None,
        });
    }
    let point = sketch.add_point(3.0, 2.0);
    let first = sketch.add_constraint(C::PointOnSpline { point, spline });
    let second = sketch.add_constraint(C::PointOnSpline { point, spline });
    let solution = solve(&sketch, &options(1.0)).unwrap();
    assert_eq!(solution.constraints[first].status, Satisfied);
    assert_eq!(solution.constraints[second].status, Redundant);
    assert_eq!(solution.dof, 1);
    // An end control point is on its own spline already.
    let end = sketch.add_constraint(C::PointOnSpline {
        point: control[6],
        spline,
    });
    let solution = solve(&sketch, &options(1.0)).unwrap();
    assert_eq!(solution.constraints[end].status, Redundant);

    // A point held off a curve that is held: the later constraint cannot
    // be, and says by how much.
    let Drawn {
        mut sketch,
        control,
        spline,
    } = wave(3, 1.0);
    for id in &control {
        sketch.add_constraint(C::Fixed {
            point: *id,
            at: None,
        });
    }
    let point = sketch.add_point(3.0, 2.0);
    sketch.add_constraint(C::Fixed {
        point,
        at: Some([3.0, 2.0]),
    });
    let on = sketch.add_constraint(C::PointOnSpline { point, spline });
    let solution = solve(&sketch, &options(1.0)).unwrap();
    let off = off_curve(3, &WAVE, [3.0, 2.0]);
    assert_eq!(
        solution.constraints[on].status,
        Conflicting,
        "{:?} {off} {:?}",
        solution.constraints[on],
        solution.point(point)
    );
    assert!(
        (solution.constraints[on].residual - off).abs() <= 1e-6,
        "{off}"
    );
    assert_eq!(solution.point(point).unwrap(), [3.0, 2.0]);
    assert_eq!(solution.dof, 0);
    // The other way round the point gives way.
    let mut swapped = sketch.clone();
    let count = swapped.constraints.len();
    swapped.constraints.swap(count - 1, count - 2);
    let solution = solve(&swapped, &options(1.0)).unwrap();
    assert_eq!(solution.constraints[count - 2].status, Satisfied);
    assert_eq!(solution.constraints[count - 1].status, Conflicting);
}

#[test]
fn beyond_its_ends_a_spline_goes_on_straight() {
    let Drawn {
        mut sketch,
        control,
        spline,
    } = wave(3, 1.0);
    for id in &control {
        sketch.add_constraint(C::Fixed {
            point: *id,
            at: None,
        });
    }
    // Before the start, whose leg runs from (0, 0) to (1, 2): on the line
    // y = 2 x. After the end, whose leg runs from (6.5, 1) to (8, 0).
    for (start, leg) in [
        ([-1.0, -2.5], ([0.0, 0.0], [1.0, 2.0])),
        ([9.0, -0.5], ([6.5, 1.0], [8.0, 0.0])),
    ] {
        let mut sketch = sketch.clone();
        let point = sketch.add_point(start[0], start[1]);
        sketch.add_constraint(C::PointOnSpline { point, spline });
        let solution = solve(&sketch, &options(1.0)).unwrap();
        let at = solution.point(point).unwrap();
        let (a, b): (P, P) = leg;
        let off = ((b[0] - a[0]) * (at[1] - a[1]) - (b[1] - a[1]) * (at[0] - a[0]))
            / (b[0] - a[0]).hypot(b[1] - a[1]);
        assert!(off.abs() <= 1e-6, "{start:?}: {at:?}");
        assert_eq!(solution.dof, 1);
    }
}

#[test]
fn what_is_not_a_spline_is_an_error_and_never_a_panic() {
    let good = wave(3, 1.0);
    let point_on = |sketch: &mut Sketch, spline: usize| {
        let point = sketch.add_point(3.0, 2.0);
        sketch.add_constraint(C::PointOnSpline { point, spline });
    };
    // A spline the sketch does not have.
    let mut sketch = good.sketch.clone();
    point_on(&mut sketch, 7);
    assert!(matches!(
        solve(&sketch, &options(1.0)),
        Err(SolveError::Unsupported { constraint: 0, .. })
    ));
    // Not a point; a control point that does not exist.
    let mut sketch = good.sketch.clone();
    let line = sketch.add_line(good.control[0], good.control[1]);
    sketch.add_constraint(C::PointOnSpline {
        point: line,
        spline: 0,
    });
    assert!(matches!(
        solve(&sketch, &options(1.0)),
        Err(SolveError::WrongKind { .. })
    ));
    let mut sketch = good.sketch.clone();
    sketch.splines[0].control[2] = EntityId(99);
    assert!(matches!(
        solve(&sketch, &options(1.0)),
        Err(SolveError::UnknownEntity { .. })
    ));
    // Degrees, knots and counts that do not fit.
    type Damage = fn(&mut Sketch);
    let broken: [Damage; 7] = [
        |s| s.splines[0].degree = 0,
        |s| s.splines[0].degree = MAX_SPLINE_DEGREE + 1,
        |s| s.splines[0].control.truncate(3),
        |s| s.splines[0].knots.truncate(4),
        |s| s.splines[0].knots[5] = f64::NAN,
        |s| s.splines[0].knots.swap(4, 6),
        |s| s.splines[0].knots[0] = -1.0,
    ];
    for (number, damage) in broken.into_iter().enumerate() {
        let mut sketch = good.sketch.clone();
        damage(&mut sketch);
        point_on(&mut sketch, 0);
        assert!(
            matches!(
                solve(&sketch, &options(1.0)),
                Err(SolveError::InvalidEntity { .. })
            ),
            "damage {number}"
        );
    }
    // A spline all on one spot has no direction: the constraint is
    // undefined there and reported, the rest solves.
    let mut sketch = Sketch::new();
    let control: Vec<EntityId> = (0..4).map(|_| sketch.add_point(1.0, 1.0)).collect();
    let spline = sketch.add_spline(3, knots(3, 4), control);
    let point = sketch.add_point(3.0, 2.0);
    let on = sketch.add_constraint(C::PointOnSpline { point, spline });
    let solution = solve(&sketch, &options(1.0)).unwrap();
    assert_eq!(solution.constraints[on].status, Conflicting);
    assert!(
        solution.constraints[on]
            .undefined
            .unwrap()
            .contains("spline")
    );
    // Coordinates that are not numbers.
    let mut sketch = good.sketch.clone();
    let point = sketch.add_point(f64::NAN, 2.0);
    sketch.add_constraint(C::PointOnSpline { point, spline: 0 });
    assert!(solve(&sketch, &options(1.0)).is_err());
}
