//! Tangency to a spline and splines that the sketch fits: a spline of
//! fixed control points (a curve given by formulas, fitted before the
//! solve) and a spline through points (a live spline, its control points
//! unknowns that its built-in equations determine).
//!
//! The answers are closed forms: the parabola `y = x^2 / k`, which a
//! quadratic Bezier on (-2k, 4k), (0, -4k), (2k, 4k) is exactly, at four
//! scales; and for the spline through points, de Boor's algorithm at many
//! places, which shares nothing with the solver.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use craft3d_math::Portable;
mod common;

use common::{resolved, statuses};
use craft3d_math::Tolerance;
use craft3d_solve::ConstraintStatus::{Conflicting, Redundant, Satisfied};
use craft3d_solve::{
    Constraint as C, EntityId, Sketch, Solution, SolveError, SolveOptions, SplineEnd, solve,
};

type P = [f64; 2];

const SCALES: [f64; 4] = [1e-3, 1.0, 1e3, 1e6];

fn options(k: f64) -> SolveOptions {
    SolveOptions::new(Tolerance::new(1e-9 * k, 1e-9).unwrap())
}

/// The parabola `y = x^2 / k` for `x` from `-2k` to `2k`, as a spline of
/// fixed control points.
fn parabola(sketch: &mut Sketch, k: f64) -> usize {
    sketch.add_fixed_spline(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![[-2.0 * k, 4.0 * k], [0.0, -4.0 * k], [2.0 * k, 4.0 * k]],
    )
}

fn close(a: P, b: P, k: f64, what: &str) {
    let gap = (a[0] - b[0]).phypot(a[1] - b[1]);
    assert!(
        gap <= 1e-7 * k,
        "{what}: {a:?} against {b:?}, {gap:e} apart"
    );
}

fn solved(sketch: &Sketch, k: f64) -> Solution {
    let solution = solve(sketch, &options(k)).unwrap_or_else(|e| panic!("{e}"));
    // Stable: solving again from the solution leaves it.
    let again = solve(&resolved(sketch, &solution), &options(k)).unwrap();
    assert_eq!(statuses(&again), statuses(&solution));
    solution
}

#[test]
fn a_point_on_a_curve_of_fixed_points_lands_on_it() {
    for k in SCALES {
        let mut s = Sketch::new();
        let curve = parabola(&mut s, k);
        let p = s.add_point(0.7 * k, 0.1 * k);
        s.add_constraint(C::CoordinateX {
            point: p,
            value: 1.3 * k,
        });
        s.add_constraint(C::PointOnSpline {
            point: p,
            spline: curve,
        });
        let solution = solved(&s, k);
        assert_eq!(solution.dof, 0, "x{k}");
        assert_eq!(statuses(&solution), [Satisfied, Satisfied]);
        close(
            solution.point(p).unwrap(),
            [1.3 * k, 1.69 * k],
            k,
            "on y = x^2",
        );
    }
}

#[test]
fn a_line_from_a_point_touches_a_parabola_where_the_closed_form_says() {
    // From (0, -k) the tangents to y = x^2 / k touch at x = k and -k,
    // slopes 2 and -2: x^2 - m k x + k^2 has a double root for m = 2.
    for k in SCALES {
        for side in [1.0, -1.0] {
            let mut s = Sketch::new();
            let curve = parabola(&mut s, k);
            let a = s.add_point(0.0, -k);
            let b = s.add_point(side * 1.2 * k, 2.6 * k);
            let line = s.add_line(a, b);
            s.add_constraint(C::Fixed { point: a, at: None });
            s.add_constraint(C::Distance {
                a,
                b,
                value: 3.0 * k,
            });
            let touch = s.add_constraint(C::TangentToSpline {
                curve: line,
                spline: curve,
                at: None,
            });
            let solution = solved(&s, k);
            assert_eq!(solution.dof, 0, "x{k}");
            assert_eq!(solution.constraints[touch].status, Satisfied);
            let unit = 3.0 * k / 5f64.sqrt();
            close(
                solution.point(b).unwrap(),
                [side * unit, -k + 2.0 * unit],
                k,
                &format!("x{k} side {side}"),
            );
        }
    }
}

#[test]
fn a_circle_touches_a_parabola_at_the_distance_of_its_centre() {
    // From (0, 2k) the nearest points of y = x^2 / k are at x^2 = 1.5 k^2,
    // at the distance k sqrt(1.75).
    for k in SCALES {
        let mut s = Sketch::new();
        let curve = parabola(&mut s, k);
        let c = s.add_point(0.0, 2.0 * k);
        let circle = s.add_circle(c, 0.6 * k);
        s.add_constraint(C::Fixed { point: c, at: None });
        s.add_constraint(C::TangentToSpline {
            curve: circle,
            spline: curve,
            at: None,
        });
        let solution = solved(&s, k);
        assert_eq!(solution.dof, 0);
        let r = solution.radius(circle).unwrap();
        assert!(
            (r - k * 1.75f64.sqrt()).abs() <= 1e-7 * k,
            "x{k}: {r} against {}",
            k * 1.75f64.sqrt()
        );
        // A centre held on the axis with its radius, 0.8 k (more than the
        // radius of curvature at the vertex, k / 2): it touches at
        // x^2 = k (c - k / 2), at the distance sqrt(k c - k^2 / 4), so its
        // height is c = 0.89 k.
        let mut s = Sketch::new();
        let curve = parabola(&mut s, k);
        let c = s.add_point(0.1 * k, 3.0 * k);
        let circle = s.add_circle(c, k);
        s.add_constraint(C::CoordinateX {
            point: c,
            value: 0.0,
        });
        s.add_constraint(C::Radius {
            circle,
            value: 0.8 * k,
        });
        s.add_constraint(C::TangentToSpline {
            curve: circle,
            spline: curve,
            at: None,
        });
        let solution = solved(&s, k);
        assert_eq!(solution.dof, 0);
        close(
            solution.point(c).unwrap(),
            [0.0, 0.89 * k],
            k,
            &format!("x{k}: the centre"),
        );
    }
}

#[test]
fn at_a_shared_end_a_line_and_an_arc_leave_along_the_curve() {
    // The parabola leaves (-2k, 4k) along (2k, -8k): direction (1, -4).
    for k in SCALES {
        let mut s = Sketch::new();
        let curve = parabola(&mut s, k);
        let a = s.add_point(-2.0 * k, 4.0 * k);
        let b = s.add_point(-1.6 * k, 3.1 * k);
        let line = s.add_line(a, b);
        s.add_constraint(C::Fixed { point: a, at: None });
        s.add_constraint(C::Distance { a, b, value: k });
        s.add_constraint(C::TangentToSpline {
            curve: line,
            spline: curve,
            at: Some(SplineEnd::Start),
        });
        let solution = solved(&s, k);
        assert_eq!(solution.dof, 0);
        let unit = k / 17f64.sqrt();
        close(
            solution.point(b).unwrap(),
            [-2.0 * k + unit, 4.0 * k - 4.0 * unit],
            k,
            "the line",
        );
        // An arc from that end, radius k: its centre on the normal.
        let mut s = Sketch::new();
        let curve = parabola(&mut s, k);
        let a = s.add_point(-2.0 * k, 4.0 * k);
        let c = s.add_point(-1.2 * k, 4.4 * k);
        let e = s.add_point(-0.5 * k, 5.0 * k);
        let arc = s.add_arc(c, a, e);
        s.add_constraint(C::Fixed { point: a, at: None });
        s.add_constraint(C::Radius {
            circle: arc,
            value: k,
        });
        s.add_constraint(C::TangentToSpline {
            curve: arc,
            spline: curve,
            at: Some(SplineEnd::Start),
        });
        let solution = solved(&s, k);
        // The arc's other end slides on its circle.
        assert_eq!(solution.dof, 1);
        close(
            solution.point(c).unwrap(),
            [-2.0 * k + 4.0 * unit, 4.0 * k + unit],
            k,
            "the arc's centre",
        );
    }
}

/// De Boor's algorithm.
fn de_boor(degree: usize, knots: &[f64], control: &[P], u: f64) -> P {
    let n = control.len();
    let u = u.min(1.0 - 1e-15);
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

const THROUGH: [P; 5] = [[0.0, 0.0], [1.0, 2.0], [2.5, 1.0], [4.0, 3.0], [5.0, 0.5]];
/// Clamped uniform cubic knots on five control points, and the Greville
/// abscissae of the inner ones.
const KNOTS: [f64; 9] = [0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0];
const GREVILLE: [f64; 3] = [1.0 / 6.0, 0.5, 5.0 / 6.0];

struct Live {
    sketch: Sketch,
    through: Vec<EntityId>,
    inner: Vec<EntityId>,
    spline: usize,
}

/// A cubic through the five points `THROUGH`, `k` times as large: the
/// first and last are its end control points, the three between are
/// control points of its own, held by its built-in equations.
fn live(k: f64, fixed: bool) -> Live {
    let mut sketch = Sketch::new();
    let through: Vec<EntityId> = THROUGH
        .iter()
        .map(|p| sketch.add_point(p[0] * k, p[1] * k))
        .collect();
    if fixed {
        for id in &through {
            sketch.add_constraint(C::Fixed {
                point: *id,
                at: None,
            });
        }
    }
    // Started anywhere: the built-in equations are linear in them.
    let inner: Vec<EntityId> = (0..3)
        .map(|i| sketch.add_point(i as f64 * k, 0.0))
        .collect();
    let control = vec![through[0], inner[0], inner[1], inner[2], through[4]];
    let spline = sketch.add_spline_through(
        3,
        KNOTS.to_vec(),
        control,
        vec![
            (through[1], GREVILLE[0]),
            (through[2], GREVILLE[1]),
            (through[3], GREVILLE[2]),
        ],
    );
    Live {
        sketch,
        through,
        inner,
        spline,
    }
}

fn control_of(solution: &Solution, live: &Live) -> Vec<P> {
    let ids = [
        live.through[0],
        live.inner[0],
        live.inner[1],
        live.inner[2],
        live.through[4],
    ];
    ids.iter().map(|id| solution.point(*id).unwrap()).collect()
}

#[test]
fn a_spline_through_points_passes_through_them_and_adds_no_freedom() {
    for k in SCALES {
        let free = live(k, false);
        let solution = solved(&free.sketch, k);
        // Five free points: ten degrees of freedom, the inner control
        // points none.
        assert_eq!(solution.dof, 10, "x{k}");
        let held = live(k, true);
        let solution = solved(&held.sketch, k);
        assert_eq!(solution.dof, 0, "x{k}");
        let control = control_of(&solution, &held);
        for (i, u) in GREVILLE.iter().enumerate() {
            let on = de_boor(3, &KNOTS, &control, *u);
            let want = [THROUGH[i + 1][0] * k, THROUGH[i + 1][1] * k];
            close(on, want, k, &format!("x{k} through {}", i + 1));
        }
    }
}

#[test]
fn a_point_and_a_line_on_a_spline_through_points() {
    for k in SCALES {
        let mut live = live(k, true);
        // Started where the document would have it: the inner control
        // points at the fit through the points (regeneration starts them
        // at the fit through the stored points), so that the place where
        // the line touches is read off the curve, not off a start that is
        // no curve through them.
        let fit = solved(&live.sketch, k);
        live.sketch.entities = fit.entities.clone();
        // A point on it at x = 3.2 k.
        let p = live.sketch.add_point(3.2 * k, 3.0 * k);
        live.sketch.add_constraint(C::CoordinateX {
            point: p,
            value: 3.2 * k,
        });
        let on = live.sketch.add_constraint(C::PointOnSpline {
            point: p,
            spline: live.spline,
        });
        // A horizontal line from x = 2.8 k to x = 5 k resting on the top of
        // the hump near x = 4 k.
        let a = live.sketch.add_point(2.8 * k, 3.6 * k);
        let b = live.sketch.add_point(5.0 * k, 3.6 * k);
        let line = live.sketch.add_line(a, b);
        live.sketch.add_constraint(C::Horizontal { line });
        live.sketch.add_constraint(C::CoordinateX {
            point: a,
            value: 2.8 * k,
        });
        live.sketch.add_constraint(C::CoordinateX {
            point: b,
            value: 5.0 * k,
        });
        let touch = live.sketch.add_constraint(C::TangentToSpline {
            curve: line,
            spline: live.spline,
            at: None,
        });
        let solution = solved(&live.sketch, k);
        assert_eq!(solution.dof, 0, "x{k}");
        assert_eq!(solution.constraints[on].status, Satisfied);
        assert_eq!(solution.constraints[touch].status, Satisfied);
        let control = control_of(&solution, &live);
        // By de Boor at a million places: the point is on the curve and
        // the line at the curve's highest point between those x.
        let (mut nearest, mut top) = (f64::INFINITY, f64::NEG_INFINITY);
        let [px, py] = solution.point(p).unwrap();
        for i in 0..=1_000_000 {
            let q = de_boor(3, &KNOTS, &control, f64::from(i) / 1e6);
            nearest = nearest.min((q[0] - px).phypot(q[1] - py));
            if q[0] >= 2.8 * k && q[0] <= 5.0 * k {
                top = top.max(q[1]);
            }
        }
        assert!(nearest <= 1e-5 * k, "x{k}: the point is {nearest:e} off");
        let y = solution.point(a).unwrap()[1];
        assert!((y - top).abs() <= 1e-7 * k, "x{k}: line at {y}, top {top}");
    }
}

#[test]
fn over_and_under_constrained_tangencies_are_named() {
    let k = 1.0;
    // The tangent line of before, then a height for its end that it
    // cannot have: the later constraint is blamed.
    let mut s = Sketch::new();
    let curve = parabola(&mut s, k);
    let a = s.add_point(0.0, -k);
    let b = s.add_point(1.2, 2.6);
    let line = s.add_line(a, b);
    s.add_constraint(C::Fixed { point: a, at: None });
    s.add_constraint(C::Distance { a, b, value: 3.0 });
    let touch = C::TangentToSpline {
        curve: line,
        spline: curve,
        at: None,
    };
    s.add_constraint(touch);
    s.add_constraint(touch);
    s.add_constraint(C::CoordinateY {
        point: b,
        value: 0.0,
    });
    let solution = solve(&s, &options(k)).unwrap();
    assert_eq!(
        statuses(&solution),
        [Satisfied, Satisfied, Satisfied, Redundant, Conflicting]
    );
    // Under-constrained: only the tangency, everything free.
    let mut s = Sketch::new();
    let curve = parabola(&mut s, k);
    let a = s.add_point(-1.0, -1.0);
    let b = s.add_point(1.2, 2.6);
    let line = s.add_line(a, b);
    s.add_constraint(C::TangentToSpline {
        curve: line,
        spline: curve,
        at: None,
    });
    let solution = solve(&s, &options(k)).unwrap();
    assert_eq!(solution.dof, 3);
    assert_eq!(statuses(&solution), [Satisfied]);
    // A line that runs along the curve nowhere has no tangency: undefined,
    // the rest solves.
    let mut s = Sketch::new();
    let straight = s.add_fixed_spline(
        2,
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]],
    );
    let a = s.add_point(0.0, 3.0);
    let b = s.add_point(2.0, 3.0);
    let line = s.add_line(a, b);
    s.add_constraint(C::Fixed { point: a, at: None });
    s.add_constraint(C::Horizontal { line });
    s.add_constraint(C::TangentToSpline {
        curve: line,
        spline: straight,
        at: None,
    });
    let solution = solve(&s, &options(k)).unwrap();
    assert_eq!(statuses(&solution)[2], Conflicting);
    assert!(solution.constraints[2].undefined.is_some());
}

#[test]
fn bad_input_is_an_error_and_every_cap_holds() {
    let k = 1.0;
    let base = || {
        let mut s = Sketch::new();
        let curve = parabola(&mut s, k);
        let a = s.add_point(0.0, -k);
        let b = s.add_point(1.2, 2.6);
        let line = s.add_line(a, b);
        s.add_constraint(C::Fixed { point: a, at: None });
        s.add_constraint(C::TangentToSpline {
            curve: line,
            spline: curve,
            at: None,
        });
        s
    };
    let broken: [fn(&mut Sketch); 8] = [
        |s| s.splines[0].control.push(EntityId(0)),
        |s| s.splines[0].fixed[1] = [f64::NAN, 0.0],
        |s| s.splines[0].fixed.truncate(2),
        |s| s.splines[0].through.push((EntityId(0), 1.5)),
        |s| s.splines[0].through.push((EntityId(2), 0.5)),
        |s| s.splines[0].through.push((EntityId(99), 0.5)),
        |s| {
            s.constraints[1] = C::TangentToSpline {
                curve: EntityId(2),
                spline: 9,
                at: None,
            }
        },
        |s| {
            s.constraints[1] = C::TangentToSpline {
                curve: EntityId(0),
                spline: 0,
                at: Some(SplineEnd::End),
            }
        },
    ];
    for (i, damage) in broken.iter().enumerate() {
        let mut s = base();
        damage(&mut s);
        let result = solve(&s, &options(k));
        assert!(result.is_err(), "case {i}: {result:?}");
    }
    // Caps: one step, or no work at all, end in an answer or an error.
    let s = base();
    let mut few = options(k);
    few.max_iterations = 1;
    match solve(&s, &few) {
        Ok(solution) => assert_eq!(solution.constraints.len(), 2),
        Err(e) => assert!(matches!(e, SolveError::DidNotConverge { .. }), "{e}"),
    }
    let mut none = options(k);
    none.work_limit = 1;
    match solve(&s, &none) {
        Ok(_) => {}
        Err(e) => assert!(
            matches!(
                e,
                SolveError::WorkLimitExceeded { .. } | SolveError::DidNotConverge { .. }
            ),
            "{e}"
        ),
    }
}

/// A line touching a spline through points of degree 7 anywhere: refused
/// as undefined with its own reason (degree 6 at most), the rest solves.
#[test]
fn a_line_touching_a_live_spline_of_degree_seven_says_degree_six() {
    let mut s = Sketch::new();
    let points: Vec<EntityId> = (0..8)
        .map(|i| s.add_point(i as f64, if i % 2 == 0 { 0.0 } else { 2.0 }))
        .collect();
    for id in &points {
        s.add_constraint(C::Fixed {
            point: *id,
            at: None,
        });
    }
    let knots: Vec<f64> = [vec![0.0; 8], vec![1.0; 8]].concat();
    let inner: Vec<EntityId> = (1..7).map(|i| s.add_point(i as f64, 1.0)).collect();
    let mut control = vec![points[0]];
    control.extend(&inner);
    control.push(points[7]);
    let through = (1..7)
        .map(|i| (points[i], knots[i + 1..=i + 7].iter().sum::<f64>() / 7.0))
        .collect();
    let spline = s.add_spline_through(7, knots, control, through);
    let a = s.add_point(0.0, 3.0);
    let b = s.add_point(7.0, 3.0);
    let line = s.add_line(a, b);
    let touch = s.add_constraint(C::TangentToSpline {
        curve: line,
        spline,
        at: None,
    });
    let solution = solve(&s, &options(1.0)).unwrap();
    assert_eq!(solution.constraints[touch].status, Conflicting);
    assert_eq!(
        solution.constraints[touch].undefined,
        Some(craft3d_solve::TOUCH_TOO_HIGH)
    );
}
