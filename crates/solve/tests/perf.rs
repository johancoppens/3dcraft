//! Solve times. Run in release with `--nocapture` to read them:
//! `cargo test -p craft3d-solve --release --test perf -- --nocapture`.
//! The assertions are loose bounds that hold in any profile; they catch a
//! solve that has become quadratically slower, not a few percent.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use std::time::{Duration, Instant};

use common::{l_profile, options, resolved, solved};
use craft3d_solve::{Constraint as C, EntityId, Sketch, solve};

/// Best of `runs` solves.
fn time(sketch: &Sketch, runs: usize) -> Duration {
    (0..runs)
        .map(|_| {
            let started = Instant::now();
            let solution = solve(sketch, &options());
            let took = started.elapsed();
            assert!(solution.is_ok());
            took
        })
        .min()
        .unwrap()
}

/// A staircase of `steps` treads and risers, each tread 3 long and each
/// riser 2 high, drawn with every corner off by up to 0.3: `2 * steps`
/// lines and `4 * steps + 1` constraints.
fn staircase(steps: usize) -> (Sketch, Vec<EntityId>) {
    let mut s = Sketch::new();
    let wobble = |i: usize| 0.3 * ((i * 7919 % 13) as f64 / 6.0 - 1.0);
    let mut points = Vec::new();
    for i in 0..=2 * steps {
        let (x, y) = (3.0 * (i.div_ceil(2)) as f64, 2.0 * (i / 2) as f64);
        points.push(s.add_point(x + wobble(i), y + wobble(i + 5)));
    }
    s.add_constraint(C::Fixed {
        point: points[0],
        at: Some([0.0, 0.0]),
    });
    for i in 0..2 * steps {
        let (a, b) = (points[i], points[i + 1]);
        let line = s.add_line(a, b);
        if i % 2 == 0 {
            s.add_constraint(C::Horizontal { line });
            s.add_constraint(C::Distance { a, b, value: 3.0 });
        } else {
            s.add_constraint(C::Vertical { line });
            s.add_constraint(C::Distance { a, b, value: 2.0 });
        }
    }
    (s, points)
}

#[test]
fn l_profile_solve_time() {
    let (sketch, _) = l_profile(1.0);
    let first = time(&sketch, 20);
    let solution = solved(&sketch);
    let again = time(&resolved(&sketch, &solution), 20);
    println!(
        "L profile (12 unknowns, 11 constraints): {first:?} from the rough drawing \
         ({} iterations), {again:?} re-solving the solution",
        solution.iterations
    );
    assert!(first < Duration::from_millis(50));
}

#[test]
fn staircase_solve_time() {
    for steps in [25, 100, 400] {
        let (sketch, points) = staircase(steps);
        let first = time(&sketch, 3);
        let solution = solved(&sketch);
        // The last corner is `steps` treads along and `steps` risers up.
        let top = solution.point(points[2 * steps]).unwrap();
        let want = [3.0 * steps as f64, 2.0 * steps as f64];
        assert!(
            (top[0] - want[0]).abs() <= 1e-9 * want[0]
                && (top[1] - want[1]).abs() <= 1e-9 * want[1],
            "{top:?}"
        );
        assert_eq!(solution.dof, 0);
        assert!(solution.is_consistent() && solution.redundant().is_empty());
        let again = time(&resolved(&sketch, &solution), 3);
        println!(
            "staircase {steps}: {} unknowns, {} constraints: {first:?} ({} iterations), \
             {again:?} re-solving the solution",
            solution.unknowns,
            sketch.constraints.len(),
            solution.iterations
        );
        assert!(first < Duration::from_secs(20));
    }
}

/// A strip of `count` equilateral triangles of side 2, every edge given by
/// its length: point `i` belongs at `(i, sqrt(3))` for odd `i` and `(i, 0)`
/// for even `i`. All equations are nonlinear and each point is tied to its
/// four neighbours.
fn truss(count: usize) -> (Sketch, Vec<EntityId>) {
    let mut s = Sketch::new();
    let wobble = |i: usize| 0.2 * ((i * 104_729 % 17) as f64 / 8.0 - 1.0);
    let points: Vec<EntityId> = (0..count + 2)
        .map(|i| {
            let y = if i % 2 == 1 { 3f64.sqrt() } else { 0.0 };
            s.add_point(i as f64 + wobble(i), y + wobble(i + 3))
        })
        .collect();
    s.add_constraint(C::Fixed {
        point: points[0],
        at: Some([0.0, 0.0]),
    });
    s.add_constraint(C::HorizontalPoints {
        a: points[0],
        b: points[2],
    });
    for i in 0..count + 1 {
        s.add_constraint(C::Distance {
            a: points[i],
            b: points[i + 1],
            value: 2.0,
        });
        if i + 2 < count + 2 {
            s.add_constraint(C::Distance {
                a: points[i],
                b: points[i + 2],
                value: 2.0,
            });
        }
    }
    (s, points)
}

#[test]
fn truss_solve_time() {
    for count in [50, 200] {
        let (sketch, points) = truss(count);
        let first = time(&sketch, 3);
        let solution = solved(&sketch);
        for (i, id) in points.iter().enumerate() {
            let got = solution.point(*id).unwrap();
            let want = [i as f64, if i % 2 == 1 { 3f64.sqrt() } else { 0.0 }];
            // Errors accumulate along the strip; 1e-9 per unit of length.
            let close = 1e-9 * (count as f64 + 2.0);
            assert!(
                (got[0] - want[0]).abs() <= close && (got[1] - want[1]).abs() <= close,
                "point {i}: {got:?} vs {want:?}"
            );
        }
        // 2 (count + 2) unknowns; 2 + 1 + (count + 1) + count equations.
        assert_eq!(solution.dof, 0);
        assert!(solution.is_consistent() && solution.redundant().is_empty());
        println!(
            "truss {count}: {} unknowns, {} constraints: {first:?} ({} iterations)",
            solution.unknowns,
            sketch.constraints.len(),
            solution.iterations
        );
        assert!(first < Duration::from_secs(20));
    }
}
