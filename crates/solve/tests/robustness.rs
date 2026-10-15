//! Never panic, never hang: thousands of seeded random and adversarial
//! sketches go through the solver. Every one returns `Ok` or `Err` in
//! bounded time, and every `Ok` keeps the promises of a solution.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use std::time::{Duration, Instant};

/// A clock of the work this thread does: its CPU time where the system
/// reports it (Linux), the wall clock elsewhere. The bounds below are there
/// to catch a loop that is not bounded, and a machine busy with other work
/// stretches the wall clock of a solve that does the same work (gate 56:
/// 13.7 s on the wall for a sketch that takes 0.5 s alone).
struct WorkClock {
    cpu: Option<u64>,
    wall: Instant,
}

impl WorkClock {
    fn start() -> Self {
        Self {
            cpu: thread_cpu_nanos(),
            wall: Instant::now(),
        }
    }

    fn elapsed(&self) -> Duration {
        match (self.cpu, thread_cpu_nanos()) {
            (Some(start), Some(now)) => Duration::from_nanos(now.saturating_sub(start)),
            _ => self.wall.elapsed(),
        }
    }
}

/// Nanoseconds this thread has spent on a CPU, from the first field of
/// `/proc/thread-self/schedstat`; `None` where there is no such file.
fn thread_cpu_nanos() -> Option<u64> {
    std::fs::read_to_string("/proc/thread-self/schedstat")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

use common::options;
use craft3d_solve::{
    Constraint as C, ConstraintStatus, Entity, EntityId, Sketch, Solution, SolveError, solve,
};

/// xorshift64*: a fixed, portable stream.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
    /// A coordinate: mostly ordinary, sometimes a repeat of a few round
    /// values (so that points coincide, lines are axis-aligned or have no
    /// length), rarely huge, tiny or not a number.
    fn coordinate(&mut self, hostile: bool) -> f64 {
        let roll = self.unit();
        if hostile && roll < 0.01 {
            [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, -1e300][self.below(5)]
        } else if roll < 0.25 {
            [0.0, 1.0, -1.0, 2.0, 10.0][self.below(5)]
        } else if hostile && roll < 0.28 {
            [1e-12, 1e-7, 1e9, -1e15][self.below(4)]
        } else {
            (self.unit() - 0.5) * 20.0
        }
    }
    fn value(&mut self, hostile: bool) -> f64 {
        let roll = self.unit();
        if hostile && roll < 0.02 {
            [f64::NAN, f64::INFINITY, -1.0, -1e-9, 1e300][self.below(5)]
        } else if roll < 0.15 {
            [0.0, 1.0, 5.0][self.below(3)]
        } else {
            self.unit() * 10.0
        }
    }
}

fn random_sketch(rng: &mut Rng, hostile: bool, most_constraints: usize) -> Sketch {
    let mut s = Sketch::new();
    let entity_count = 4 + rng.below(21);
    let mut points: Vec<EntityId> = Vec::new();
    let mut lines: Vec<EntityId> = Vec::new();
    let mut rounds: Vec<EntityId> = Vec::new();
    // An id of the wanted kind, or (hostile) any id at all, valid or not.
    let pick = |rng: &mut Rng, of: &[EntityId], total: usize| -> EntityId {
        if of.is_empty() || (hostile && rng.chance(0.03)) {
            EntityId(rng.below(total + 3) as u32)
        } else {
            of[rng.below(of.len())]
        }
    };
    for _ in 0..entity_count {
        let total = s.entities.len();
        let roll = rng.unit();
        // The first entities are two points, a line and a circle, so that
        // every kind a constraint may ask for exists.
        let roll = match total {
            0 | 1 => 0.0,
            2 => 0.6,
            3 => 0.8,
            _ => roll,
        };
        if roll < 0.5 {
            points.push(s.add_point(rng.coordinate(hostile), rng.coordinate(hostile)));
        } else if roll < 0.75 {
            let (a, mut b) = (pick(rng, &points, total), pick(rng, &points, total));
            if !hostile && a == b {
                // A line from a point to itself is legal but has no
                // direction; leave those to the hostile run.
                b = points[(points.iter().position(|p| *p == a).unwrap() + 1) % points.len()];
            }
            lines.push(s.add_line(a, b));
        } else if roll < 0.9 {
            let c = pick(rng, &points, total);
            let r = if hostile && rng.chance(0.03) {
                -1.0
            } else {
                rng.value(hostile)
            };
            rounds.push(s.add_circle(c, r));
        } else {
            let (c, a, b) = (
                pick(rng, &points, total),
                pick(rng, &points, total),
                pick(rng, &points, total),
            );
            rounds.push(s.add_arc(c, a, b));
        }
    }
    let total = s.entities.len();
    for _ in 0..rng.below(most_constraints + 1) {
        let p = |rng: &mut Rng| pick(rng, &points, total);
        let l = |rng: &mut Rng| pick(rng, &lines, total);
        let r = |rng: &mut Rng| pick(rng, &rounds, total);
        let line_or_round = |rng: &mut Rng| {
            if rng.chance(0.5) {
                pick(rng, &lines, total)
            } else {
                pick(rng, &rounds, total)
            }
        };
        let c = match rng.below(21) {
            0 => C::Coincident {
                a: p(rng),
                b: p(rng),
            },
            1 => C::PointOnLine {
                point: p(rng),
                line: l(rng),
            },
            2 => C::PointOnCircle {
                point: p(rng),
                circle: r(rng),
            },
            3 => C::Horizontal { line: l(rng) },
            4 => C::Vertical { line: l(rng) },
            5 => C::HorizontalPoints {
                a: p(rng),
                b: p(rng),
            },
            6 => C::VerticalPoints {
                a: p(rng),
                b: p(rng),
            },
            7 => C::Parallel {
                a: l(rng),
                b: l(rng),
            },
            8 => C::Perpendicular {
                a: l(rng),
                b: l(rng),
            },
            9 => C::Tangent {
                a: line_or_round(rng),
                b: r(rng),
            },
            10 if rng.chance(0.5) => C::Equal {
                a: l(rng),
                b: l(rng),
            },
            10 => C::Equal {
                a: r(rng),
                b: r(rng),
            },
            11 => C::Concentric {
                a: r(rng),
                b: r(rng),
            },
            12 => C::Symmetric {
                a: p(rng),
                b: p(rng),
                axis: l(rng),
            },
            13 => C::Midpoint {
                point: p(rng),
                line: l(rng),
            },
            14 => C::Fixed {
                point: p(rng),
                at: if rng.chance(0.5) {
                    None
                } else {
                    Some([rng.coordinate(hostile), rng.coordinate(hostile)])
                },
            },
            15 => C::Distance {
                a: p(rng),
                b: if rng.chance(0.7) { p(rng) } else { l(rng) },
                value: rng.value(hostile),
            },
            16 => C::HorizontalDistance {
                a: p(rng),
                b: p(rng),
                value: rng.value(hostile),
            },
            17 => C::VerticalDistance {
                a: p(rng),
                b: p(rng),
                value: rng.value(hostile),
            },
            18 => C::Angle {
                a: l(rng),
                b: l(rng),
                value: (rng.value(hostile) - 5.0) * 0.7,
            },
            19 => C::Radius {
                circle: r(rng),
                value: rng.value(hostile),
            },
            _ => C::Diameter {
                circle: r(rng),
                value: rng.value(hostile),
            },
        };
        s.add_constraint(c);
    }
    s
}

/// What must hold for any `Ok`.
fn check_solution(sketch: &Sketch, solution: &Solution, what: &str) {
    let tol = options().tolerance;
    assert_eq!(solution.entities.len(), sketch.entities.len(), "{what}");
    assert_eq!(
        solution.constraints.len(),
        sketch.constraints.len(),
        "{what}"
    );
    assert_eq!(solution.dof, solution.unknowns - solution.rank, "{what}");
    for (before, after) in sketch.entities.iter().zip(&solution.entities) {
        match (before, after) {
            (Entity::Point { .. }, Entity::Point { x, y }) => {
                assert!(x.is_finite() && y.is_finite(), "{what}");
            }
            (Entity::Circle { center: a, .. }, Entity::Circle { center: b, radius }) => {
                assert!(a == b && radius.is_finite() && *radius >= 0.0, "{what}");
            }
            (a @ Entity::Line { .. }, b) | (a @ Entity::Arc { .. }, b) => {
                assert_eq!(a, b, "{what}");
            }
            _ => panic!("{what}: an entity changed kind"),
        }
        // An arc is an arc: both ends at one radius.
        if let Entity::Arc { center, start, end } = after {
            let (c, s, e) = (
                solution.point(*center).unwrap(),
                solution.point(*start).unwrap(),
                solution.point(*end).unwrap(),
            );
            let gap = (s[0] - c[0]).hypot(s[1] - c[1]) - (e[0] - c[0]).hypot(e[1] - c[1]);
            assert!(gap.abs() <= tol.linear, "{what}: arc off by {gap}");
        }
    }
    for (i, report) in solution.constraints.iter().enumerate() {
        match report.status {
            ConstraintStatus::Satisfied | ConstraintStatus::Redundant => assert!(
                report.residual <= tol.linear.max(tol.angular),
                "{what}: constraint {i} is {:?} with residual {}",
                report.status,
                report.residual
            ),
            ConstraintStatus::Conflicting => assert!(report.residual >= 0.0, "{what}"),
        }
    }
}

#[derive(Default, Debug)]
struct Tally {
    ok: usize,
    with_conflicts: usize,
    invalid: usize,
    degenerate: usize,
    did_not_converge: usize,
    other: usize,
    /// Solutions that did not come back bit-identical from a second solve
    /// (all within tolerance, or the run fails), and the furthest move.
    moved_on_resolve: usize,
    max_move: f64,
    consistent: usize,
    iterations: usize,
    slowest: Duration,
}

fn run(seed: u64, count: usize, hostile: bool, most_constraints: usize) -> Tally {
    let mut rng = Rng(seed);
    let mut tally = Tally::default();
    for i in 0..count {
        let sketch = random_sketch(&mut rng, hostile, most_constraints);
        let what = format!("seed {seed} sketch {i}");
        let started = WorkClock::start();
        let result = solve(&sketch, &options());
        let took = started.elapsed();
        tally.slowest = tally.slowest.max(took);
        // These sketches have at most 48 unknowns: a solve is milliseconds.
        // Ten seconds means a loop that is not bounded as it should be.
        assert!(took < Duration::from_secs(10), "{what} took {took:?}");
        match result {
            Ok(solution) => {
                check_solution(&sketch, &solution, &what);
                tally.ok += 1;
                tally.iterations += solution.iterations;
                if solution.is_consistent() {
                    tally.consistent += 1;
                } else {
                    tally.with_conflicts += 1;
                }
                // Deterministic.
                assert_eq!(solve(&sketch, &options()).as_ref(), Ok(&solution), "{what}");
                // Solving the solution again is a solution again.
                let again = Sketch {
                    entities: solution.entities.clone(),
                    constraints: sketch.constraints.clone(),
                    splines: sketch.splines.clone(),
                };
                // The property a caller that stores solutions relies on:
                // the same verdict, and nothing moved beyond tolerance.
                let second = match solve(&again, &options()) {
                    Ok(second) => second,
                    Err(e) => panic!("{what}: the solution does not solve again: {e}"),
                };
                check_solution(&again, &second, &what);
                assert_eq!(
                    second.conflicting(),
                    solution.conflicting(),
                    "{what}: another verdict the second time"
                );
                let mut far = 0.0f64;
                for (a, b) in solution.entities.iter().zip(&second.entities) {
                    match (a, b) {
                        (Entity::Point { x, y }, Entity::Point { x: x2, y: y2 }) => {
                            far = far.max((x - x2).abs()).max((y - y2).abs());
                        }
                        (Entity::Circle { radius, .. }, Entity::Circle { radius: r2, .. }) => {
                            far = far.max((radius - r2).abs());
                        }
                        _ => {}
                    }
                }
                assert!(far <= 1e-6, "{what}: the second solve moved {far}");
                if far > 0.0 {
                    tally.moved_on_resolve += 1;
                    tally.max_move = tally.max_move.max(far);
                }
            }
            Err(e) => {
                assert!(!e.to_string().is_empty());
                match e {
                    SolveError::UnknownEntity { .. }
                    | SolveError::WrongKind { .. }
                    | SolveError::NonFinite { .. }
                    | SolveError::InvalidEntity { .. }
                    | SolveError::InvalidValue { .. } => tally.invalid += 1,
                    SolveError::Degenerate { .. } => tally.degenerate += 1,
                    SolveError::DidNotConverge { .. } => tally.did_not_converge += 1,
                    _ => tally.other += 1,
                }
            }
        }
    }
    tally
}

#[test]
fn random_sparse_sketches() {
    // A handful of constraints on a dozen entities: mostly solvable and
    // under-constrained, which is what a sketch in progress looks like.
    let tally = run(0x5EED_0003, 3000, false, 5);
    println!("sparse: {tally:?}");
    assert!(tally.consistent >= 1500, "{tally:?}");
    assert_eq!(
        tally.invalid + tally.other + tally.degenerate,
        0,
        "{tally:?}"
    );
}

#[test]
fn random_dense_sketches() {
    // Valid ids and finite values, but up to 40 arbitrary constraints on
    // arbitrary geometry: coincident points, zero-length lines, and nearly
    // always contradictions.
    let tally = run(0x5EED_0001, 1500, false, 40);
    println!("dense: {tally:?}");
    // The generator must reach the solver, not just the validation.
    assert!(tally.ok >= 750, "{tally:?}");
    assert!(tally.with_conflicts >= 100, "{tally:?}");
    // Measured: 125. Without continuing the conflict search from the
    // prefix it has already solved, 144 of these sketches go unsolved.
    assert!(tally.did_not_converge <= 135, "{tally:?}");
    assert_eq!(
        tally.invalid + tally.other + tally.degenerate,
        0,
        "{tally:?}"
    );
}

#[test]
fn random_hostile_sketches() {
    // The same with unknown ids, wrong kinds, NaN, infinities, 1e300,
    // negative lengths.
    let tally = run(0x5EED_0002, 3000, true, 40);
    println!("hostile: {tally:?}");
    assert!(tally.ok >= 300, "{tally:?}");
    assert!(tally.invalid >= 300, "{tally:?}");
}

#[test]
fn everything_at_one_point() {
    // All points at the origin, every constraint kind on them.
    let mut s = Sketch::new();
    let p: Vec<EntityId> = (0..6).map(|_| s.add_point(0.0, 0.0)).collect();
    let l1 = s.add_line(p[0], p[1]);
    let circle = s.add_circle(p[2], 0.0);
    let arc = s.add_arc(p[3], p[4], p[5]);
    let constraints = [
        C::Coincident { a: p[0], b: p[1] },
        C::PointOnCircle {
            point: p[0],
            circle,
        },
        C::PointOnCircle {
            point: p[1],
            circle: arc,
        },
        C::Horizontal { line: l1 },
        C::Vertical { line: l1 },
        C::Equal { a: circle, b: arc },
        C::Concentric { a: circle, b: arc },
        C::Tangent { a: circle, b: arc },
        C::Midpoint {
            point: p[2],
            line: l1,
        },
        C::Distance {
            a: p[0],
            b: p[5],
            value: 3.0,
        },
        C::HorizontalDistance {
            a: p[0],
            b: p[4],
            value: 2.0,
        },
        C::VerticalDistance {
            a: p[1],
            b: p[3],
            value: 2.0,
        },
        C::Radius { circle, value: 1.0 },
        C::Diameter {
            circle: arc,
            value: 2.0,
        },
        C::Fixed {
            point: p[0],
            at: None,
        },
    ];
    for n in 0..=constraints.len() {
        let mut s = s.clone();
        s.constraints.extend_from_slice(&constraints[..n]);
        if let Ok(solution) = solve(&s, &options()) {
            check_solution(&s, &solution, &format!("first {n} constraints"));
        }
    }
}

#[test]
fn thousands_of_constraints() {
    // 3000 constraints on one line, nearly all saying the same thing.
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(3.0, 4.0);
    let line = s.add_line(a, b);
    for i in 0..3000 {
        s.add_constraint(match i % 3 {
            0 => C::Horizontal { line },
            1 => C::Distance { a, b, value: 5.0 },
            _ => C::HorizontalPoints { a, b },
        });
    }
    let started = WorkClock::start();
    let solution = solve(&s, &options()).unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    check_solution(&s, &solution, "flood");
    // Two independent equations (level, length); the other 2998 repeat.
    assert_eq!(solution.rank, 2);
    assert_eq!(solution.redundant().len(), 2998);
    assert_eq!(solution.dof, 2);

    // The same number of contradictions: every odd distance is wrong.
    let mut s = Sketch::new();
    let a = s.add_point(0.0, 0.0);
    let b = s.add_point(3.0, 4.0);
    for i in 0..3000 {
        s.add_constraint(C::Distance {
            a,
            b,
            value: if i % 2 == 0 { 5.0 } else { 6.0 },
        });
    }
    let started = WorkClock::start();
    let solution = solve(&s, &options()).unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(solution.conflicting().len(), 1500);
    assert_eq!(solution.redundant().len(), 1499);
}
