//! A re-solve lands on the solution the solver of 0.1 landed on wherever
//! that one solved: the same side of every circle. What it solves that 0.1
//! did not is on the branch the sketch was built on.
//!
//! An arc's rows that step as the power difference (linear in the centre)
//! were tried and withdrawn. The first form had the value of the power
//! difference over the chord in every solve: near the arc's start that row
//! was a spike as narrow as the distance to it, and a point moved along
//! the circle crossed to the far side of it: these cases, which it failed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use craft3d_math::{Portable, Tolerance};
use craft3d_solve::{Constraint as C, Entity, EntityId, Sketch, Solution, SolveOptions, solve};

fn solved_with(sketch: &Sketch, options: &SolveOptions) -> Solution {
    match solve(sketch, options) {
        Ok(s) => s,
        Err(e) => panic!("solve failed: {e}"),
    }
}

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

/// A quarter arc of radius 10 about a held origin, from (10, 0) to (0, 10),
/// both held, and a point on it whose y is held at `y`, its x free, drawn
/// at `drawn`.
fn quarter(y: f64, drawn: [f64; 2]) -> (Sketch, EntityId) {
    let mut s = Sketch::new();
    let c = s.add_point(0.0, 0.0);
    let a = s.add_point(10.0, 0.0);
    let b = s.add_point(0.0, 10.0);
    let arc = s.add_arc(c, a, b);
    let p = s.add_point(drawn[0], drawn[1]);
    for (point, [x, y]) in [(c, [0.0, 0.0]), (a, [10.0, 0.0]), (b, [0.0, 10.0])] {
        s.add_constraint(C::CoordinateX { point, value: x });
        s.add_constraint(C::CoordinateY { point, value: y });
    }
    s.add_constraint(C::PointOnCircle {
        point: p,
        circle: arc,
    });
    s.add_constraint(C::CoordinateY { point: p, value: y });
    (s, p)
}

/// The point on the quarter arc held at y = 5 (so at (8.66, 5)) has its y
/// edited to 0.1, 0.01 or 0.2 (from 5, 3 and 8): the point it lands on is
/// (sqrt(100 - y^2), y), the near one, where the solver of 0.1 put it and
/// where a fresh build puts it; the first rewrite of the rows put it at
/// (-sqrt(100 - y^2), y), on the far side of the circle. At three scales.
#[test]
fn a_point_on_an_arc_moved_along_it_stays_on_its_side() {
    for k in [1e-3, 1.0, 1e6] {
        let options = SolveOptions::new(Tolerance::new(1e-6 * k, 1e-9).unwrap());
        for (from, to) in [(5.0, 0.1), (3.0, 0.01), (8.0, 0.2), (5.0, 9.9), (0.1, 9.0)] {
            let scale = |s: Sketch| -> Sketch {
                let mut s = s;
                for e in &mut s.entities {
                    if let Entity::Point { x, y } = e {
                        *x *= k;
                        *y *= k;
                    }
                }
                for c in &mut s.constraints {
                    if let C::CoordinateX { value, .. } | C::CoordinateY { value, .. } = c {
                        *value *= k;
                    }
                }
                s
            };
            let x0 = (100.0f64 - from * from).sqrt();
            let (built, p) = quarter(from, [x0, from]);
            let first = solved_with(&scale(built), &options);
            assert!(first.is_consistent());
            let (edited, _) = quarter(to, [x0, from]);
            let solution = solved_with(&starting_at(&scale(edited), &first), &options);
            let what = format!("scale {k}: y {from} -> {to}");
            assert!(
                solution.is_consistent(),
                "{what}: {:?}",
                solution.constraints
            );
            let [x, y] = point(&solution, p);
            let near = (100.0f64 - to * to).sqrt() * k;
            assert!(
                (x - near).abs() <= 1e-9 * k && (y - to * k).abs() <= 1e-9 * k,
                "{what}: at ({x}, {y}), the near point is ({near}, {})",
                to * k
            );
        }
    }
}

/// xorshift64*: a fixed, portable stream.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// One case of the sweep: a sketch with an arc, built and solved fresh,
/// then one held value changed and the sketch solved again from that
/// solution, as a document is; and which unknowns the values do not hold
/// (whose solved places are compared).
struct Case {
    built: Sketch,
    edited: Sketch,
    free: Vec<(EntityId, usize)>,
    mode: u64,
    axis: usize,
}

/// An arc about `c` of radius `r` from `a0` turning `turn` (radians), and
/// a point on it `at` of the way; how it is held by `mode`:
/// 0. the centre and both ends held, the point held in x or in y only;
/// 1. the ends and the point held, the centre free (an arc through a point);
/// 2. the centre and the start held, the end in x or in y only;
/// 3. the ends held and a radius, the centre free (two centres, one on
///    either side of the chord: the start picks one).
///
/// Then one held value is moved by a random amount, up to 1.2 r.
fn case(rng: &mut Rng) -> Case {
    let mode = rng.below(4);
    let r = rng.range(0.5, 200.0);
    let c = [rng.range(-300.0, 300.0), rng.range(-300.0, 300.0)];
    let a0 = rng.range(0.0, std::f64::consts::TAU);
    let turn = rng.range(0.2, std::f64::consts::TAU - 0.2);
    let at = rng.range(0.15, 0.85);
    let on = |t: f64| [c[0] + r * t.pcos(), c[1] + r * t.psin()];
    let (s, e, p) = (on(a0), on(a0 + turn), on(a0 + at * turn));
    let mut sk = Sketch::new();
    let ic = sk.add_point(c[0], c[1]);
    let is = sk.add_point(s[0], s[1]);
    let ie = sk.add_point(e[0], e[1]);
    let arc = sk.add_arc(ic, is, ie);
    let ip = sk.add_point(p[0], p[1]);
    let axis = rng.below(2) as usize;
    // (constraint index, value) of every value held.
    let mut held: Vec<usize> = Vec::new();
    let mut hold = |sk: &mut Sketch, point: EntityId, at: [f64; 2], axes: [bool; 2]| {
        if axes[0] {
            held.push(sk.add_constraint(C::CoordinateX {
                point,
                value: at[0],
            }));
        }
        if axes[1] {
            held.push(sk.add_constraint(C::CoordinateY {
                point,
                value: at[1],
            }));
        }
    };
    let one = |axis: usize| [axis == 0, axis == 1];
    let free = match mode {
        0 => {
            hold(&mut sk, ic, c, [true; 2]);
            hold(&mut sk, is, s, [true; 2]);
            hold(&mut sk, ie, e, [true; 2]);
            sk.add_constraint(C::PointOnCircle {
                point: ip,
                circle: arc,
            });
            hold(&mut sk, ip, p, one(axis));
            vec![(ip, 1 - axis)]
        }
        1 => {
            hold(&mut sk, is, s, [true; 2]);
            hold(&mut sk, ie, e, [true; 2]);
            sk.add_constraint(C::PointOnCircle {
                point: ip,
                circle: arc,
            });
            hold(&mut sk, ip, p, [true; 2]);
            vec![(ic, 0), (ic, 1)]
        }
        2 => {
            hold(&mut sk, ic, c, [true; 2]);
            hold(&mut sk, is, s, [true; 2]);
            hold(&mut sk, ie, e, one(axis));
            hold(&mut sk, ip, p, [true; 2]);
            vec![(ie, 1 - axis)]
        }
        _ => {
            hold(&mut sk, is, s, [true; 2]);
            hold(&mut sk, ie, e, [true; 2]);
            sk.add_constraint(C::Radius {
                circle: arc,
                value: r,
            });
            hold(&mut sk, ip, p, [true; 2]);
            vec![(ic, 0), (ic, 1)]
        }
    };
    let mut edited = sk.clone();
    let which = held[rng.below(held.len() as u64) as usize];
    let by = if rng.below(3) == 0 {
        rng.range(-0.05, 0.05) * r
    } else {
        rng.range(-1.2, 1.2) * r
    };
    if let Some(C::CoordinateX { value, .. } | C::CoordinateY { value, .. }) =
        edited.constraints.get_mut(which)
    {
        *value += by;
    }
    Case {
        built: sk,
        edited,
        free,
        mode,
        axis,
    }
}

/// Whether `solved` is on the branch the case was built on, read off the
/// places with this test's own arithmetic: the centre (entity 0), the
/// arc's start and end (1, 2) and the point (4). Mode 0: the point's free
/// coordinate on the side of the centre it was built on, and the point
/// between the arc's ends; mode 1: the point between the ends (the
/// centre is fixed by the three points); mode 2: the end's free
/// coordinate on its side of the centre; mode 3: the centre on its side of
/// the chord. A reading within 1e-9 of the size of its fence counts as
/// either side.
fn on_built_branch(case: &Case, solved: &Solution) -> bool {
    let at = |entities: &[Entity], i: u32| match entities[i as usize] {
        Entity::Point { x, y } => [x, y],
        _ => panic!("not a point"),
    };
    let sign = |v: f64, size: f64| {
        if v.abs() <= 1e-9 * size.max(1.0) {
            0
        } else {
            v.signum() as i8
        }
    };
    let agree = |a: i8, b: i8| a == 0 || b == 0 || a == b;
    let (built, now) = (&case.built.entities, &solved.entities);
    let chord_side = |entities: &[Entity]| {
        let (c, s, e) = (at(entities, 0), at(entities, 1), at(entities, 2));
        let cross = (e[0] - s[0]) * (c[1] - s[1]) - (e[1] - s[1]) * (c[0] - s[0]);
        sign(
            cross,
            (e[0] - s[0]).phypot(e[1] - s[1]) * (c[0] - s[0]).phypot(c[1] - s[1]),
        )
    };
    let between = |entities: &[Entity]| {
        let (c, s, e, q) = (
            at(entities, 0),
            at(entities, 1),
            at(entities, 2),
            at(entities, 4),
        );
        let turn = |r: [f64; 2]| {
            ((r[1] - c[1]).patan2(r[0] - c[0]) - (s[1] - c[1]).patan2(s[0] - c[0]))
                .rem_euclid(std::f64::consts::TAU)
        };
        turn(q) <= turn(e) + 1e-9
    };
    let free_side = |entities: &[Entity], point: u32| {
        let (c, q) = (at(entities, 0), at(entities, point));
        sign(
            q[1 - case.axis] - c[1 - case.axis],
            (q[0] - c[0]).phypot(q[1] - c[1]),
        )
    };
    match case.mode {
        0 => agree(free_side(built, 4), free_side(now, 4)) && between(now),
        // Through three held points the centre is the one circumcentre:
        // no choice. Its side of the chord follows the points (the arc
        // turns from less than a half turn to more as its point goes past
        // the half circle); the choice is the way round, through the point.
        1 => between(now),
        2 => agree(free_side(built, 2), free_side(now, 2)),
        _ => agree(chord_side(built), chord_side(now)),
    }
}

/// The sweep's cases, with what solving them gives: per case `None` when
/// the edited sketch did not solve without a conflict, else the solved
/// places of the unknowns the values do not hold.
fn sweep_results(count: usize) -> Vec<Option<Vec<f64>>> {
    sweep(count)
        .into_iter()
        .map(|r| r.map(|(values, _)| values))
        .collect()
}

/// [`sweep_results`] with, per case solved, whether it is on the branch
/// it was built on ([`on_built_branch`]).
fn sweep(count: usize) -> Vec<Option<(Vec<f64>, bool)>> {
    let options = common::options();
    let mut rng = Rng(0x00A2_C0B5_7EED);
    (0..count)
        .map(|_| {
            let case = case(&mut rng);
            let first = solve(&case.built, &options).ok()?;
            let edited = solve(&starting_at(&case.edited, &first), &options).ok()?;
            edited.is_consistent().then(|| {
                let values = case
                    .free
                    .iter()
                    .map(|(id, axis)| point(&edited, *id)[*axis])
                    .collect();
                (values, on_built_branch(&case, &edited))
            })
        })
        .collect()
}

const SWEEP: usize = 2000;

fn sweep_file() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/arc_branches/solved-0.1.json")
}

/// Records what the solver of 0.1 (`release/0.1` at 26a296f6) solves the
/// sweep to: run with that solver's `crates/solve/src` in place,
/// `CRAFT3D_RECORD_SWEEP=1 cargo test -p craft3d-solve --test arc_branches
/// -- --ignored record_the_sweep`.
#[test]
#[ignore = "records the 0.1 solver's results; see its comment"]
fn record_the_sweep() {
    if std::env::var("CRAFT3D_RECORD_SWEEP").is_err() {
        return;
    }
    let results = sweep_results(SWEEP);
    let text = serde_json_like(&results);
    std::fs::create_dir_all(sweep_file().parent().unwrap()).unwrap();
    std::fs::write(sweep_file(), text).unwrap();
}

/// The results as JSON, by hand (the crate has no serde): one line per
/// case, `null` or an array of numbers written to round-trip.
fn serde_json_like(results: &[Option<Vec<f64>>]) -> String {
    let mut out = String::from("[\n");
    for (i, r) in results.iter().enumerate() {
        match r {
            None => out.push_str("null"),
            Some(values) => {
                let shown: Vec<String> = values.iter().map(|v| format!("{v:?}")).collect();
                out.push_str(&format!("[{}]", shown.join(", ")));
            }
        }
        out.push_str(if i + 1 == results.len() { "\n" } else { ",\n" });
    }
    out.push(']');
    out.push('\n');
    out
}

fn read_sweep() -> Vec<Option<Vec<f64>>> {
    let text = std::fs::read_to_string(sweep_file()).unwrap();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "[" && *l != "]")
        .map(|l| l.trim_end_matches(','))
        .map(|l| match l {
            "null" => None,
            _ => Some(
                l.trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(|v| v.trim().parse::<f64>().unwrap())
                    .collect(),
            ),
        })
        .collect()
}

/// Two thousand random arcs held four ways (a point on an arc, an arc
/// through a point, an end sliding on its circle, a centre given by a
/// radius), each built and then one held value moved, small or by up to
/// 1.2 radii: wherever the solver of 0.1 solved the edit without a
/// conflict, this one does too, and every unknown the values do not hold
/// (a point's free coordinate, a centre) is where that one put it, within
/// the tolerance: the same side of every circle. The 0.1 results are
/// recorded in `tests/arc_branches/solved-0.1.json` (`record_the_sweep`).
/// Where 0.1 did not solve and this one does, the result is on the branch
/// the case was built on (`on_built_branch`).
#[test]
fn a_sweep_of_single_edits_lands_where_the_solver_of_0_1_landed() {
    let old = read_sweep();
    assert_eq!(old.len(), SWEEP);
    let new = sweep(SWEEP);
    let (mut compared, mut gained) = (0, Vec::new());
    for (i, (old, new)) in old.iter().zip(&new).enumerate() {
        match (old, new) {
            (Some(old), Some((new, _))) => {
                compared += 1;
                // Both hold every constraint within the tolerance; a centre
                // far from the points that fix it is fixed by them only to
                // the tolerance times its distance over the chord, so 1e-9
                // of its size is allowed besides. Another root would be at
                // least the chord away.
                for (a, b) in old.iter().zip(new) {
                    assert!(
                        (a - b).abs() <= 1e-6 + 1e-9 * a.abs(),
                        "case {i}: 0.1 put it at {old:?}, now {new:?}"
                    );
                }
            }
            (Some(old), None) => panic!("case {i}: 0.1 solved it ({old:?}), now not"),
            // What 0.1 did not solve and this one does (the attempt from the
            // circles through three held points): on the branch the case
            // was built on.
            (None, Some((_, on_branch))) => {
                assert!(on_branch, "case {i}: solved on another branch than built");
                gained.push(i);
            }
            (None, None) => {}
        }
    }
    println!(
        "compared {compared}, solved now and not by 0.1: {} {gained:?}",
        gained.len()
    );
    assert!(compared > SWEEP / 2, "{compared}");
}

/// An arc nearly closed (350 degrees) about a held centre: its start held,
/// its end's y held and its x free. With the end's y edited (the end
/// sliding along the circle, near the start and away from it) the end
/// stays on its side of the circle: (sqrt(r^2 - y^2), y) with the sign of
/// its x before. The arc's own row (its ends equally far from the centre)
/// near closure, where the ends are close and the row's gradient in the
/// centre is small.
#[test]
fn a_nearly_closed_arc_keeps_its_end_on_its_side() {
    let options = common::options();
    for (from_deg, to_y) in [
        (-10.0f64, -0.1),
        (-10.0, -9.0),
        (-2.0, -0.5),
        (-30.0, -1e-3),
    ] {
        let r = 10.0;
        let t = from_deg.to_radians();
        let start = [r, 0.0];
        let end = [r * t.pcos(), r * t.psin()];
        let mut s = Sketch::new();
        let c = s.add_point(0.0, 0.0);
        let a = s.add_point(start[0], start[1]);
        let b = s.add_point(end[0], end[1]);
        s.add_arc(c, a, b);
        for (point, [x, y]) in [(c, [0.0, 0.0]), (a, start)] {
            s.add_constraint(C::CoordinateX { point, value: x });
            s.add_constraint(C::CoordinateY { point, value: y });
        }
        s.add_constraint(C::CoordinateY {
            point: b,
            value: end[1],
        });
        let first = solved_with(&s, &options);
        assert!(first.is_consistent());
        let mut edited = s.clone();
        let last = edited.constraints.len() - 1;
        edited.constraints[last] = C::CoordinateY {
            point: b,
            value: to_y,
        };
        let solution = solved_with(&starting_at(&edited, &first), &options);
        let what = format!("{from_deg} deg, y -> {to_y}");
        assert!(solution.is_consistent(), "{what}");
        let [x, y] = point(&solution, b);
        let near = (r * r - to_y * to_y).sqrt();
        assert!(
            (x - near).abs() <= 1e-9 * r && (y - to_y).abs() <= 1e-9 * r,
            "{what}: end at ({x}, {y}), the near point is ({near}, {to_y})"
        );
    }
}
