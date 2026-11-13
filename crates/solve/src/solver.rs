//! The iteration and the diagnosis.
//!
//! One *run* is a damped Newton iteration from a starting point on a chosen
//! subset of the constraints. Each step is the minimum-norm solution of the
//! linearised independent equations ([`crate::factor`]), cut back by a line
//! search until the residual really decreases, so an under-constrained
//! sketch moves as little as possible and a step never leaps to another
//! branch on the strength of a linearisation that does not hold there.
//!
//! A run that ends with every independent equation within tolerance has
//! *converged*; equations that were dependent at the end are then either
//! redundant (they hold anyway) or conflicting (they do not). When a run on
//! all constraints does not converge, the constraints are added in the
//! order they were given, each batch starting from the solution of the
//! ones before; a constraint that cannot be added even from there is
//! blamed and left out.

use crate::equations::{Axis, Equation, Fail, Prepared, Pt, start_failure};
use crate::error::{Site, SolveError};
use crate::factor::{Factor, Jacobian, Work};
use crate::model::{
    ConstraintReport, ConstraintStatus, Entity, EntityId, Sketch, Solution, SolveOptions,
};
use crate::{MAX_BLAMED, RUNS_PER_SOLVE};
use craft3d_math::Portable;

/// Polishing target: an equation is done when its residual, relative to the
/// magnitude of the sketch (lengths) or in radians (angles), is below this.
/// Three orders above the rounding noise of evaluating a residual, far
/// below any sensible tolerance; the tolerance itself applies when smaller.
const FLOOR: f64 = 1e-13;
/// Sufficient-decrease constant of the line search.
const ARMIJO: f64 = 1e-4;
/// The line search gives up below a step fraction of 2^-24 (6e-8): a step
/// that has to be cut that far is not making progress.
const MAX_HALVINGS: usize = 24;
/// Size of the nudge off a singular point, relative to the sketch.
const KICK: f64 = 1e-3;
/// Most extra passes [`solve`] makes looking for a result that solving
/// again leaves alone.
const SETTLE_PASSES: usize = 6;
/// A gradient this much shorter than gradients of its kind normally are
/// (1 for a length, 1 / sketch magnitude for an angle) is rounding residue.
const NEGLIGIBLE: f64 = 1e-10;

struct Budget {
    steps_left: usize,
    steps_taken: usize,
    work: Work,
}

/// The state a run ended in.
struct Run {
    x: Vec<f64>,
    r: Vec<f64>,
    dependent: Vec<bool>,
    rank: usize,
    /// Every independent equation holds, and the iteration was not on its
    /// way into a degenerate configuration.
    converged: bool,
}

enum Stop {
    /// An equation has no value at the starting point of the run.
    Start(usize, Fail),
    Error(SolveError),
}

pub(crate) struct System<'a> {
    pub(crate) prep: &'a Prepared,
    options: &'a SolveOptions,
    /// Per equation: its first row. One more entry than equations.
    row_start: Vec<usize>,
    /// Per row: the residual below which the row holds.
    pub(crate) tolerance: Vec<f64>,
    /// Per row: the unit its residual is measured in for the merit function.
    weight: Vec<f64>,
    /// Per row: the gradient length at or below which there is no gradient.
    pub(crate) negligible: Vec<f64>,
    /// While the sketch is carried along (`crate::carry`): every row is
    /// `F(x) - (1 - x[t]) r0`, so that the starting guess, where the rows
    /// are `r0`, is a solution at `t = 0`.
    pub(crate) homotopy: Option<Homotopy>,
    /// The rows that hold one unknown at a value (an `x` or `y`, a `Fixed`
    /// at a place), as (row, unknown, value).
    held: Vec<(usize, usize, f64)>,
}

/// The continuation parameter's unknown, and the rows at its start.
pub(crate) struct Homotopy {
    pub(crate) t: usize,
    pub(crate) r0: Vec<f64>,
}

impl<'a> System<'a> {
    pub(crate) fn new(prep: &'a Prepared, options: &'a SolveOptions) -> Self {
        let mut row_start = Vec::with_capacity(prep.equations.len() + 1);
        let mut tolerance = Vec::new();
        let mut weight = Vec::new();
        let mut negligible = Vec::new();
        let mut held = Vec::new();
        for eq in &prep.equations {
            let first = tolerance.len();
            row_start.push(first);
            let holds: [Option<(usize, f64)>; 2] = match *eq {
                Equation::Coordinate(Axis::X, Pt::Own(i), value) => [Some((i, value)), None],
                Equation::Coordinate(Axis::Y, Pt::Own(i), value) => {
                    [Some((i.saturating_add(1), value)), None]
                }
                Equation::Fixed(Pt::Own(i), [x, y]) => {
                    [Some((i, x)), Some((i.saturating_add(1), y))]
                }
                _ => [None, None],
            };
            for (row, hold) in (first..).zip(holds) {
                if let Some((col, value)) = hold
                    && value.is_finite()
                {
                    held.push((row, col, value));
                }
            }
            let rows = eq.rows();
            let (tol, unit) = if eq.angular() {
                (options.tolerance.angular, 1.0)
            } else {
                (options.tolerance.linear, prep.magnitude)
            };
            for _ in 0..rows {
                // Two rows share the tolerance, so that their norm, which
                // is what the report shows, is within it.
                tolerance.push(if rows == 2 {
                    tol * std::f64::consts::FRAC_1_SQRT_2
                } else {
                    tol
                });
                weight.push(unit);
                negligible.push(if eq.angular() {
                    NEGLIGIBLE / prep.magnitude
                } else {
                    NEGLIGIBLE
                });
            }
        }
        row_start.push(tolerance.len());
        System {
            prep,
            options,
            row_start,
            tolerance,
            weight,
            negligible,
            homotopy: None,
            held,
        }
    }

    /// The rows that hold one unknown at a value (an `x` or `y`, a `Fixed`
    /// at a place) and are independent now, as (unknown, value); none
    /// while the sketch is carried along (its rows are then not those).
    fn landings(&self, independent: &[bool]) -> Vec<(usize, f64)> {
        if self.homotopy.is_some() {
            return Vec::new();
        }
        self.held
            .iter()
            .filter(|(row, ..)| independent.get(*row).copied().unwrap_or(false))
            .map(|&(_, col, value)| (col, value))
            .collect()
    }

    fn rows_of(&self, equation: usize) -> std::ops::Range<usize> {
        match (
            self.row_start.get(equation),
            self.row_start.get(equation + 1),
        ) {
            (Some(a), Some(b)) => *a..*b,
            _ => 0..0,
        }
    }

    /// Residuals of the active equations at `x` (zero for the others), and
    /// the Jacobian when asked for. Fails with the offending equation.
    pub(crate) fn evaluate(
        &self,
        x: &[f64],
        active: &[bool],
        mut jac: Option<&mut Jacobian>,
    ) -> Result<Vec<f64>, (usize, Fail)> {
        let mut r = Vec::with_capacity(self.tolerance.len());
        for (index, eq) in self.prep.equations.iter().enumerate() {
            if !active.get(index).copied().unwrap_or(false) {
                r.extend(std::iter::repeat_n(0.0, eq.rows()));
                if let Some(j) = jac.as_deref_mut() {
                    (0..eq.rows()).for_each(|_| j.end_row());
                }
                continue;
            }
            let e = eq
                .eval_with(
                    x,
                    self.options.tolerance.linear,
                    &self.prep.splines,
                    &self.prep.images,
                )
                .map_err(|f| (index, f))?;
            for row in e.rows.iter().take(e.count) {
                // The start's residual, taken off in the measure that is
                // still to come: d/dt of the row gains `r0`.
                let (shift, t) = match &self.homotopy {
                    Some(h) => {
                        let r0 = h.r0.get(r.len()).copied().unwrap_or(0.0);
                        let rest = 1.0 - x.get(h.t).copied().unwrap_or(1.0);
                        (r0, Some((h.t, rest)))
                    }
                    None => (0.0, None),
                };
                r.push(match t {
                    Some((_, rest)) => row.v - rest * shift,
                    None => row.v,
                });
                if let Some(j) = jac.as_deref_mut() {
                    let mut pending = t.map(|(col, _)| (col, shift));
                    for (col, d) in e.slots.iter().zip(row.d).take(e.used) {
                        match pending {
                            Some((t, extra)) if t == *col => {
                                j.push(*col, d + extra);
                                pending = None;
                            }
                            _ => j.push(*col, d),
                        }
                    }
                    if let Some((t, extra)) = pending {
                        j.push(t, extra);
                    }
                    j.end_row();
                }
            }
        }
        Ok(r)
    }

    pub(crate) fn skip_rows(&self, active: &[bool]) -> Vec<bool> {
        let mut skip = vec![false; self.tolerance.len()];
        for (index, on) in active.iter().enumerate() {
            if !on {
                for row in self.rows_of(index) {
                    if let Some(s) = skip.get_mut(row) {
                        *s = true;
                    }
                }
            }
        }
        skip
    }

    /// One damped Newton iteration from `start`.
    fn newton(&self, start: &[f64], active: &[bool], budget: &mut Budget) -> Result<Run, Stop> {
        let rows = self.tolerance.len();
        let skip = self.skip_rows(active);
        let mut x = start.to_vec();
        let mut steps = 0usize;
        // The latest line search had to turn down a step because it led to
        // a configuration in which an equation is undefined.
        let mut blocked = false;
        loop {
            let mut jac = Jacobian::new(x.len());
            let r = match self.evaluate(&x, active, Some(&mut jac)) {
                Ok(r) => r,
                Err((eq, fail)) if steps == 0 => return Err(Stop::Start(eq, fail)),
                // Cannot happen: this point passed the line search.
                Err(_) => {
                    return Err(Stop::Error(SolveError::DidNotConverge {
                        iterations: budget.steps_taken,
                    }));
                }
            };
            let factor = Factor::new(
                &jac,
                &skip,
                &self.negligible,
                self.options.rank_threshold,
                &mut budget.work,
            )
            .map_err(Stop::Error)?;
            let independent: Vec<bool> = (0..rows)
                .map(|i| {
                    !skip.get(i).copied().unwrap_or(true)
                        && !factor.dependent.get(i).copied().unwrap_or(true)
                })
                .collect();
            // Largest residual of the independent rows, in units of `bound`.
            let worst = |r: &[f64], bound: &dyn Fn(usize) -> f64| {
                r.iter()
                    .zip(&independent)
                    .enumerate()
                    .filter(|(_, (_, on))| **on)
                    .all(|(i, (r, _))| r.abs() <= bound(i))
            };
            let tol = |i: usize| self.tolerance.get(i).copied().unwrap_or(0.0);
            let unit = |i: usize| self.weight.get(i).copied().unwrap_or(1.0);
            let polished = worst(&r, &|i| tol(i).min(FLOOR * unit(i)));
            let out_of_steps = steps >= self.options.max_iterations || budget.steps_left == 0;

            let mut next: Option<Vec<f64>> = None;
            if !polished && !out_of_steps {
                let merit = |r: &[f64]| -> f64 {
                    r.iter()
                        .zip(&independent)
                        .enumerate()
                        .filter(|(_, (_, on))| **on)
                        .map(|(i, (r, _))| (r / unit(i)) * (r / unit(i)))
                        .sum()
                };
                let rhs: Vec<f64> = r.iter().map(|v| -v).collect();
                let mut dx = factor.min_norm(&rhs);
                // A coordinate held at a value by an independent row moves
                // by exactly what that row lacks, as the step does in exact
                // arithmetic, and a whole step puts it on the value itself:
                // the rounding of the factorisation never leaves it a few
                // units in the last place off (where it would stay, since
                // within the polishing floor nothing moves it again).
                let landings = self.landings(&independent);
                for &(col, value) in &landings {
                    if let (Some(d), Some(at)) = (dx.get_mut(col), x.get(col)) {
                        *d = value - at;
                    }
                }
                let longest = dx.iter().fold(0.0f64, |m, v| m.max(v.abs()));
                if longest > 0.0 && longest.is_finite() {
                    let before = merit(&r);
                    let mut t = 1.0;
                    blocked = false;
                    for _ in 0..MAX_HALVINGS {
                        let mut trial: Vec<f64> =
                            x.iter().zip(&dx).map(|(x, d)| x + t * d).collect();
                        if t == 1.0 {
                            for &(col, value) in &landings {
                                if let Some(at) = trial.get_mut(col) {
                                    *at = value;
                                }
                            }
                        }
                        match self.evaluate(&trial, active, None) {
                            // Also false for a NaN or infinite merit.
                            Ok(rt) if merit(&rt) <= (1.0 - 2.0 * ARMIJO * t) * before => {
                                next = Some(trial);
                                break;
                            }
                            Err((_, Fail::Degenerate(_))) => blocked = true,
                            _ => {}
                        }
                        t *= 0.5;
                    }
                }
            }
            match next {
                Some(trial) => {
                    x = trial;
                    steps += 1;
                    budget.steps_left = budget.steps_left.saturating_sub(1);
                    budget.steps_taken = budget.steps_taken.saturating_add(1);
                }
                None => {
                    // Within tolerance is not enough when the iteration was
                    // still heading for a collapse (a line shrinking to a
                    // point so that two contradictory directions "hold"):
                    // such a point counts only when it is polished.
                    let converged = polished || (worst(&r, &tol) && !blocked);
                    return Ok(Run {
                        x,
                        r,
                        dependent: factor.dependent.clone(),
                        rank: factor.rank(),
                        converged,
                    });
                }
            }
        }
    }

    /// Rows that are dependent on earlier ones and do not hold.
    fn violated(&self, run: &Run) -> Vec<usize> {
        run.dependent
            .iter()
            .zip(&run.r)
            .zip(&self.tolerance)
            .enumerate()
            .filter(|(_, ((dependent, r), tol))| **dependent && r.abs() > **tol)
            .map(|(i, _)| i)
            .collect()
    }

    /// A run from `start`. When it converges onto a point where
    /// an unsatisfied equation is locally dependent on the others, that may
    /// be a singular configuration rather than a conflict (a point straight
    /// below the centre it must keep a slanted distance to): one more run
    /// from a slightly displaced point decides, and counts only when it
    /// leaves fewer equations violated.
    fn attempt(
        &self,
        start: &[f64],
        from_guess: bool,
        active: &[bool],
        budget: &mut Budget,
    ) -> Result<Run, SolveError> {
        let run = match self.newton(start, active, budget) {
            Ok(run) => run,
            Err(Stop::Error(e)) => return Err(e),
            // Undefined for the caller's own starting guess: an error.
            Err(Stop::Start(eq, fail)) if from_guess => {
                let at = self
                    .prep
                    .source
                    .get(eq)
                    .copied()
                    .unwrap_or(Site::Constraint(0));
                return Err(start_failure(at, fail));
            }
            // Undefined where the earlier constraints put the sketch (the
            // direction of a line they shrank to a point): no solution
            // from here.
            Err(Stop::Start(..)) => {
                return Ok(Run {
                    x: start.to_vec(),
                    r: Vec::new(),
                    dependent: Vec::new(),
                    rank: 0,
                    converged: false,
                });
            }
        };
        if !run.converged {
            return Ok(run);
        }
        let violated = self.violated(&run);
        if violated.is_empty() {
            return Ok(run);
        }
        let mut displaced = run.x.clone();
        let mut moved = false;
        for (index, eq) in self.prep.equations.iter().enumerate() {
            if !self.rows_of(index).any(|row| violated.contains(&row)) {
                continue;
            }
            // Every unknown of the equation, also those its gradient is
            // zero in at this point: they are the ones that matter.
            let Ok(e) = eq.eval_with(
                &run.x,
                self.options.tolerance.linear,
                &self.prep.splines,
                &self.prep.images,
            ) else {
                continue;
            };
            for col in e.slots.iter().take(e.used) {
                if let (Some(to), Some(from)) = (displaced.get_mut(*col), run.x.get(*col)) {
                    *to = from + KICK * self.prep.size * kick_direction(*col);
                    moved = true;
                }
            }
        }
        if !moved {
            return Ok(run);
        }
        match self.newton(&displaced, active, budget) {
            Ok(second) if second.converged && self.violated(&second).len() < violated.len() => {
                Ok(second)
            }
            Err(Stop::Error(e)) => Err(e),
            _ => Ok(run),
        }
    }
}

/// A fixed, uneven displacement pattern in `[-1, -0.25] u [0.25, 1]`: no
/// two unknowns move alike, so symmetric configurations are broken.
fn kick_direction(index: usize) -> f64 {
    // Multiples of the golden ratio modulo 1 are evenly spread.
    let u = ((index as f64 + 1.0) * 0.618_033_988_749_894_9).fract();
    let h = 2.0 * u - 1.0;
    if h.abs() < 0.25 {
        if h < 0.0 { h - 0.5 } else { h + 0.5 }
    } else {
        h
    }
}

fn validate(options: &SolveOptions) -> Result<(), SolveError> {
    let positive = |v: f64| v.is_finite() && v > 0.0;
    if !positive(options.tolerance.linear) || !positive(options.tolerance.angular) {
        return Err(SolveError::InvalidOptions {
            reason: "tolerances must be positive and finite",
        });
    }
    if !(positive(options.rank_threshold) && options.rank_threshold < 1.0) {
        return Err(SolveError::InvalidOptions {
            reason: "the rank threshold must be between 0 and 1",
        });
    }
    Ok(())
}

/// The coordinates and radii of two solutions differ by at most `within`.
fn same_place(a: &[Entity], b: &[Entity], within: f64) -> bool {
    a.iter().zip(b).all(|pair| match pair {
        (Entity::Point { x, y }, Entity::Point { x: x2, y: y2 }) => {
            (x - x2).abs() <= within && (y - y2).abs() <= within
        }
        (Entity::Circle { radius, .. }, Entity::Circle { radius: r2, .. }) => {
            (radius - r2).abs() <= within
        }
        _ => true,
    })
}

/// What the carrying did, once the solve from where its path ended has
/// failed with `why` (the solve then starts from the stored coordinates):
/// a path that stopped stays the reason; one that was followed to its end
/// did not give a start the solve could use, which is a skip.
fn unusable_end(
    outcome: Option<crate::model::Carried>,
    moved: &[crate::model::EntityId],
    why: &SolveError,
) -> Option<crate::model::Carried> {
    match outcome {
        Some(crate::model::Carried::Whole) => Some(crate::model::Carried::Skipped {
            reason: format!(
                "the sketch was carried all the way, but solving from there failed ({why})"
            ),
            moved: moved.to_vec(),
        }),
        other => other,
    }
}

fn same_verdict(a: &Solution, b: &Solution) -> bool {
    a.constraints
        .iter()
        .zip(&b.constraints)
        .all(|(a, b)| a.status == b.status)
}

/// Solves until solving again changes nothing.
///
/// One pass ([`solve_once`]) reads the mirror choices, the places of points
/// fixed where they are, and which constraints are undefined, off its
/// starting guess; its result is a different guess, from which the next
/// pass may decide otherwise (a line that had no direction now has one).
/// What is returned is a fixed point of the pass, so that the caller, who
/// stores the result and solves again later, gets the same answer.
///
/// When that ends in a conflict or does not converge, one last attempt
/// starts from the circles that the held points determine
/// ([`rounds_through_held_points`]), when that start reads every choice
/// as the stored solution does ([`same_choices`]); its result is taken
/// only when it has no conflict and every arc it placed still runs
/// through its points ([`passes_through`]), and otherwise the result is
/// the one before. A sketch that solves without a conflict at the first
/// attempts never reaches it.
pub(crate) fn solve(sketch: &Sketch, options: &SolveOptions) -> Result<Solution, SolveError> {
    validate(options)?;
    let mut spent = 0u64;
    let before = solve_from_the_stored_solution(sketch, options, &mut spent);
    let stuck = match &before {
        Ok(solution) => conflicts(solution) > 0 && !only_unplaced_holders(sketch, solution),
        Err(SolveError::DidNotConverge { .. }) => true,
        Err(_) => false,
    };
    if !stuck {
        return before;
    }
    let held = held_in_place(sketch);
    let Some((start, placed)) =
        rounds_through_held_points(held.as_ref().unwrap_or(sketch), options.tolerance.linear)
    else {
        return before;
    };
    if !same_choices(sketch, &start, options.tolerance.linear) {
        return before;
    }
    // The last attempt spends what the ones before it left of the work.
    let left = options.work_limit.saturating_sub(spent);
    if left == 0 {
        return before;
    }
    let mut meter = Meter {
        cap: Some(left),
        spent: 0,
    };
    let iterations = match &before {
        Ok(solution) => solution.iterations,
        Err(SolveError::DidNotConverge { iterations }) => *iterations,
        Err(_) => 0,
    };
    match settle(&start, options, &mut meter) {
        Ok(mut solved)
            if conflicts(&solved) == 0
                && passes_through(&solved, &placed, options.tolerance.linear) =>
        {
            solved.iterations = solved.iterations.saturating_add(iterations);
            Ok(solved)
        }
        _ => before,
    }
}

/// The solve as it was before the last attempt existed: from the stored
/// solution, and once more with the held coordinates at their values when
/// that ends in a conflict. `spent` gets the work of both.
fn solve_from_the_stored_solution(
    sketch: &Sketch,
    options: &SolveOptions,
    spent: &mut u64,
) -> Result<Solution, SolveError> {
    let mut meter = Meter {
        cap: None,
        spent: 0,
    };
    let first = settle(sketch, options, &mut meter);
    *spent = meter.spent;
    let first_iterations = match &first {
        Ok(solution) => solution.iterations,
        Err(SolveError::DidNotConverge { iterations }) => *iterations,
        Err(_) => 0,
    };
    let worth_another = match &first {
        Ok(solution) => conflicts(solution) > 0 && !only_unplaced_holders(sketch, solution),
        Err(SolveError::DidNotConverge { .. }) => true,
        Err(_) => false,
    };
    if !worth_another {
        return first;
    }
    let Some(placed) = held_in_place(sketch) else {
        return first;
    };
    // The retry spends what the first attempt left of the work limit.
    let left = options.work_limit.saturating_sub(meter.spent);
    if left == 0 {
        return first;
    }
    let mut meter = Meter {
        cap: Some(left),
        spent: 0,
    };
    let second = settle(&placed, options, &mut meter).map(|mut second| {
        second.iterations = second.iterations.saturating_add(first_iterations);
        second
    });
    *spent = spent.saturating_add(meter.spent);
    match (first, second) {
        // Only a solution without conflicts replaces one with: solving it
        // again finds nothing to retry, so it stays what it is.
        (Ok(_), Ok(second)) if conflicts(&second) == 0 => Ok(second),
        (Ok(first), _) => Ok(first),
        // A verdict, conflicts and all, rather than none.
        (Err(_), Ok(second)) => Ok(second),
        (Err(first), Err(_)) => Err(match first {
            SolveError::DidNotConverge { .. } => SolveError::DidNotConverge {
                iterations: first_iterations,
            },
            other => other,
        }),
    }
}

/// Work spent by the passes of one [`settle`], and the most they may
/// spend together (`None`: each pass has the whole work limit, as the
/// first attempt always had).
struct Meter {
    cap: Option<u64>,
    spent: u64,
}

impl Meter {
    /// The budget of the next pass.
    fn next(&self, options: &SolveOptions, share: u64) -> Work {
        let whole = options.work_limit / share;
        Work::new(match self.cap {
            Some(cap) => whole.min(cap.saturating_sub(self.spent)),
            None => whole,
        })
    }

    fn add(&mut self, work: &Work) {
        self.spent = self.spent.saturating_add(work.used());
    }
}

/// Every conflict of `solution` is on a coordinate that [`held_in_place`]
/// does not place (a second `x` of one point, or one on a point fixed
/// where it is): it stays a conflict from any start, so a retry is waste.
fn only_unplaced_holders(sketch: &Sketch, solution: &Solution) -> bool {
    let placed = placed_holders(sketch);
    solution
        .constraints
        .iter()
        .enumerate()
        .filter(|(_, c)| c.status == ConstraintStatus::Conflicting)
        .all(|(i, _)| {
            use crate::model::Constraint as C;
            matches!(
                sketch.constraints.get(i),
                Some(C::CoordinateX { .. } | C::CoordinateY { .. } | C::Fixed { at: Some(_), .. })
            ) && !placed.get(i).copied().unwrap_or(false)
        })
}

/// How many constraints a solution leaves conflicting.
fn conflicts(solution: &Solution) -> usize {
    solution
        .constraints
        .iter()
        .filter(|c| c.status == ConstraintStatus::Conflicting)
        .count()
}

/// The sketch with every coordinate that a constraint holds at a value
/// (`x`, `y`, `Fixed` at a place: the first such constraint of each
/// coordinate; not on a point that a `Fixed` without a place holds where
/// it is) already at that value in the starting guess, or none when that
/// moves nothing.
///
/// A held point starts where it was solved before its values changed, and
/// Newton's method linearises there. An equation that is not linear in it
/// (an arc's ends equally far from its centre, a distance) then has the
/// root nearest that old place, not the one the held values pick: the
/// arc's equation, which comes first, settles the second coordinate of
/// a held end on the old side of the circle and the coordinate's own
/// constraint is blamed. A fresh build draws the point at its values and
/// does not meet this; this start is that of a fresh build.
fn held_in_place(sketch: &Sketch) -> Option<Sketch> {
    placed_within(sketch, f64::INFINITY)
}

/// [`held_in_place`] for the held coordinates that are at most `within`
/// from their values; the others stay where they are.
fn placed_within(sketch: &Sketch, within: f64) -> Option<Sketch> {
    use crate::model::Constraint as C;
    let mut entities = sketch.entities.clone();
    let mut moved = false;
    for (constraint, placed) in sketch.constraints.iter().zip(placed_holders(sketch)) {
        if !placed {
            continue;
        }
        let (point, values) = match *constraint {
            C::CoordinateX { point, value } => (point, [Some(value), None]),
            C::CoordinateY { point, value } => (point, [None, Some(value)]),
            C::Fixed {
                point,
                at: Some([x, y]),
            } => (point, [Some(x), Some(y)]),
            _ => continue,
        };
        let Some(Entity::Point { x, y }) = entities.get_mut(point.index()) else {
            continue;
        };
        for (slot, value) in [x, y].into_iter().zip(values) {
            if let Some(value) = value
                && *slot != value
                && (within == f64::INFINITY || (*slot - value).abs() <= within)
            {
                *slot = value;
                moved = true;
            }
        }
    }
    moved.then(|| Sketch {
        entities,
        constraints: sketch.constraints.clone(),
        splines: sketch.splines.clone(),
        image_starts: sketch.image_starts.clone(),
    })
}

/// A round the last attempt centred, and the held points it was centred
/// through.
type Placed = (EntityId, Vec<EntityId>);

/// The sketch with every circle and arc whose centre no constraint holds,
/// and which passes through three points whose coordinates constraints
/// hold at values, centred on the circle through those three (a circle's
/// radius too, to the first of them), or none when that places nothing.
///
/// Such a centre is fixed by the values alone, as one point: the circle
/// through three points that are not in line. It is the one kind of
/// geometry in a sketch that the values determine while its equations are
/// not linear in it (two points equally far from a centre): a point held
/// by values, or by lines through held points (`coincident`, `midpoint`,
/// `symmetric`, a point on a held line), is found by Newton's method from
/// any start in one step, and a point at a distance or an angle from held
/// ones, or tangent to them, has more than one place the values allow, of
/// which the start picks one. A centre iterated from a stale place, far
/// off or on the line of its chord, may not be found at all; from here it
/// is where it belongs.
///
/// The points through which a round passes: an arc's start and end, then
/// the points `pointOn` puts on it, in the order of the constraints; the
/// first three that lie apart (farther than the tolerance from each
/// other) and are held in both coordinates (`x` and `y`, or `Fixed`).
/// With the sketch, the rounds placed and the points each was placed
/// through.
fn rounds_through_held_points(sketch: &Sketch, tolerance: f64) -> Option<(Sketch, Vec<Placed>)> {
    use crate::model::Constraint as C;
    let count = sketch.entities.len();
    let mut held = vec![[false; 2]; count];
    for constraint in &sketch.constraints {
        let (point, axes) = match *constraint {
            C::CoordinateX { point, value } => (point, [value.is_finite(), false]),
            C::CoordinateY { point, value } => (point, [false, value.is_finite()]),
            C::Fixed {
                point,
                at: Some([x, y]),
            } => (point, [x.is_finite(), y.is_finite()]),
            C::Fixed { point, at: None } => (point, [true; 2]),
            _ => continue,
        };
        if let Some(h) = held.get_mut(point.index()) {
            h[0] |= axes[0];
            h[1] |= axes[1];
        }
    }
    let is_held = |id: EntityId| held.get(id.index()) == Some(&[true; 2]);
    let mut through: Vec<Vec<EntityId>> = vec![Vec::new(); count];
    for (index, entity) in sketch.entities.iter().enumerate() {
        if let Entity::Arc { start, end, .. } = *entity
            && let Some(list) = through.get_mut(index)
        {
            list.extend([start, end]);
        }
    }
    for constraint in &sketch.constraints {
        if let C::PointOnCircle { point, circle } = *constraint
            && let Some(list) = through.get_mut(circle.index())
        {
            list.push(point);
        }
    }
    let mut entities = sketch.entities.clone();
    let mut placed = Vec::new();
    for (index, entity) in sketch.entities.iter().enumerate() {
        let (center, circle) = match *entity {
            Entity::Arc { center, .. } => (center, false),
            Entity::Circle { center, .. } => (center, true),
            _ => continue,
        };
        if is_held(center) || !matches!(entities.get(center.index()), Some(Entity::Point { .. })) {
            continue;
        }
        let mut points: Vec<[f64; 2]> = Vec::new();
        let mut ids = Vec::new();
        for id in through.get(index).map(Vec::as_slice).unwrap_or(&[]) {
            if points.len() == 3 || !is_held(*id) {
                continue;
            }
            let Some(&Entity::Point { x, y }) = entities.get(id.index()) else {
                continue;
            };
            let apart = points
                .iter()
                .all(|[px, py]| (x - px).abs().max((y - py).abs()) > tolerance);
            if apart {
                points.push([x, y]);
                ids.push(*id);
            }
        }
        let (Some(&a), Some(&b), Some(&c)) = (points.first(), points.get(1), points.get(2)) else {
            continue;
        };
        let Some(at) = circumcentre(a, b, c) else {
            continue;
        };
        if let Some(slot) = entities.get_mut(center.index()) {
            *slot = Entity::Point { x: at[0], y: at[1] };
            placed.push((EntityId(u32::try_from(index).unwrap_or(u32::MAX)), ids));
        }
        if circle && let Some(Entity::Circle { radius, .. }) = entities.get_mut(index) {
            let r = (a[0] - at[0]).phypot(a[1] - at[1]);
            if r.is_finite() && r > 0.0 {
                *radius = r;
            }
        }
    }
    (!placed.is_empty()).then(|| {
        (
            Sketch {
                entities,
                constraints: sketch.constraints.clone(),
                splines: sketch.splines.clone(),
                image_starts: sketch.image_starts.clone(),
            },
            placed,
        )
    })
}

/// Whether the choices a pass reads off its starting guess (which side of
/// a line a point or a centre keeps, a tangency inside or outside, the
/// place of a point fixed where it is) are the same from `start` as from
/// `stored`: the equations `prepare` makes of the two are equal. The last
/// attempt moves centres, and a centre now on the other side of a line it
/// is tangent to, or kept at a distance from, would turn that choice over
/// and give a solution on another branch than the stored one.
fn same_choices(stored: &Sketch, start: &Sketch, min_len: f64) -> bool {
    match (
        crate::equations::prepare(stored, min_len),
        crate::equations::prepare(start, min_len),
    ) {
        (Ok(a), Ok(b)) => a.equations == b.equations,
        _ => false,
    }
}

/// Whether every arc the last attempt placed passes, between its ends in
/// the direction it runs (counter-clockwise from its start), through each
/// point it was placed through, in `solution` (within `tolerance` of an
/// end counts); circles always do. An arc whose point crossed the line of
/// its ends is solved with its circle through the point but runs the long
/// way round its centre, away from it: a shape no new document draws and
/// one that may have folded over, so it is not taken.
fn passes_through(solution: &Solution, placed: &[Placed], tolerance: f64) -> bool {
    placed.iter().all(|(round, through)| {
        let Some(&Entity::Arc { center, start, end }) = solution.entities.get(round.index()) else {
            return true;
        };
        let (Some(c), Some(s), Some(e)) = (
            solution.point(center),
            solution.point(start),
            solution.point(end),
        ) else {
            return false;
        };
        let turn = |q: [f64; 2]| {
            let a = (q[1] - c[1]).patan2(q[0] - c[0]) - (s[1] - c[1]).patan2(s[0] - c[0]);
            a.rem_euclid(std::f64::consts::TAU)
        };
        let sweep = turn(e);
        through
            .iter()
            .filter(|p| **p != start && **p != end)
            .all(|p| {
                let Some(q) = solution.point(*p) else {
                    return false;
                };
                let near = |r: [f64; 2]| (q[0] - r[0]).phypot(q[1] - r[1]) <= tolerance;
                turn(q) <= sweep || near(s) || near(e)
            })
    })
}

/// The centre of the circle through `a`, `b` and `c` (the meeting point of
/// two perpendicular bisectors, by Cramer's rule, from `a`), or none for
/// points in line or a centre that is not finite.
fn circumcentre(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<[f64; 2]> {
    let (bx, by) = (b[0] - a[0], b[1] - a[1]);
    let (cx, cy) = (c[0] - a[0], c[1] - a[1]);
    let d = 2.0 * (bx * cy - by * cx);
    if d == 0.0 || !d.is_finite() {
        return None;
    }
    let (b2, c2) = (bx * bx + by * by, cx * cx + cy * cy);
    let at = [
        a[0] + (cy * b2 - by * c2) / d,
        a[1] + (bx * c2 - cx * b2) / d,
    ];
    at.iter().all(|v| v.is_finite()).then_some(at)
}

/// Per constraint: whether [`held_in_place`] places a coordinate at its
/// value: the first `x`, `y` or `Fixed` at a place of each coordinate of a
/// point, with a finite value, unless a `Fixed` without a place holds the
/// point where it is (moving it would change what it is held at).
fn placed_holders(sketch: &Sketch) -> Vec<bool> {
    use crate::model::Constraint as C;
    let mut taken = vec![[false; 2]; sketch.entities.len()];
    for constraint in &sketch.constraints {
        if let C::Fixed { point, at: None } = *constraint
            && let Some(done) = taken.get_mut(point.index())
        {
            *done = [true; 2];
        }
    }
    sketch
        .constraints
        .iter()
        .map(|constraint| {
            let (point, axes) = match *constraint {
                C::CoordinateX { point, value } => (point, [value.is_finite(), false]),
                C::CoordinateY { point, value } => (point, [false, value.is_finite()]),
                C::Fixed {
                    point,
                    at: Some([x, y]),
                } => (point, [x.is_finite(), y.is_finite()]),
                _ => return false,
            };
            let is_point = matches!(
                sketch.entities.get(point.index()),
                Some(Entity::Point { .. })
            );
            let Some(done) = taken.get_mut(point.index()) else {
                return false;
            };
            let mut places = false;
            for (wanted, done) in axes.into_iter().zip(done.iter_mut()) {
                if wanted && !*done && is_point {
                    *done = true;
                    places = true;
                }
            }
            places
        })
        .collect()
}

/// [`solve`] from the sketch's own starting guess.
fn settle(
    sketch: &Sketch,
    options: &SolveOptions,
    meter: &mut Meter,
) -> Result<Solution, SolveError> {
    // Image points that jumped since the starting guess was solved carry
    // the sketch along first, with half the work budget of its own; the
    // solve then starts from where the path ended, with the whole budget,
    // as it had before carrying existed. Should it fail from there, it
    // solves from the stored coordinates as it did then, and says so
    // (`Carried::Skipped`): carrying never makes the solve fail that
    // succeeded without it; whether a solution that was not carried may
    // be used is the caller's to decide.
    let mut work = meter.next(options, 2);
    let mut carrying = crate::carry::carry(sketch, options, &mut work);
    meter.add(&work);
    let carried = carrying.entities.take().map(|entities| Sketch {
        entities,
        constraints: sketch.constraints.clone(),
        splines: sketch.splines.clone(),
        image_starts: Vec::new(),
    });
    let mut current = match carried {
        Some(carried) => match solve_once(&carried, options, meter) {
            Ok(solved) => solved,
            Err(why) => {
                carrying.outcome = unusable_end(carrying.outcome.take(), &carrying.moved, &why);
                solve_once(sketch, options, meter)?
            }
        },
        None => solve_once(sketch, options, meter)?,
    };
    current.carry = carrying.outcome.clone();
    let mut iterations = current.iterations;
    let mut close = false;
    for _ in 0..SETTLE_PASSES {
        let guess = Sketch {
            entities: current.entities.clone(),
            constraints: sketch.constraints.clone(),
            splines: sketch.splines.clone(),
            image_starts: Vec::new(),
        };
        let mut next = solve_once(&guess, options, meter)?;
        iterations = iterations.saturating_add(next.iterations);
        next.iterations = iterations;
        next.carry = carrying.outcome.clone();
        // From the same coordinates the next pass would return exactly
        // what this one did, verdict included.
        let settled = same_place(&current.entities, &next.entities, 0.0);
        close = same_verdict(&current, &next)
            && same_place(&current.entities, &next.entities, options.tolerance.linear);
        current = next;
        if settled {
            return Ok(current);
        }
    }
    // Not to the last bit, but the last pass kept the verdict and moved
    // nothing by as much as the tolerance: rounding, not indecision.
    if close {
        Ok(current)
    } else {
        Err(SolveError::DidNotConverge { iterations })
    }
}

/// One pass, with the budget `meter` gives it, its work counted there.
///
/// A coordinate held at a value that starts within the tolerance of it
/// starts on it: a move the tolerance cannot tell from none, which picks
/// no branch, and what is held is then its value to the last bit,
/// whatever solve ended there before (Newton's method stops once a row is
/// below its polishing floor, so a held coordinate solved from a nearby
/// start would keep what that start left in its last bits, and solves
/// from different starts would differ there).
fn solve_once(
    sketch: &Sketch,
    options: &SolveOptions,
    meter: &mut Meter,
) -> Result<Solution, SolveError> {
    let touched = placed_within(sketch, options.tolerance.linear);
    let sketch = touched.as_ref().unwrap_or(sketch);
    let mut budget = Budget {
        steps_left: options.max_iterations.saturating_mul(RUNS_PER_SOLVE),
        steps_taken: 0,
        work: meter.next(options, 1),
    };
    let result = pass(sketch, options, &mut budget);
    meter.add(&budget.work);
    result
}

fn pass(
    sketch: &Sketch,
    options: &SolveOptions,
    budget: &mut Budget,
) -> Result<Solution, SolveError> {
    let prep = crate::equations::prepare(sketch, options.tolerance.linear)?;
    let system = System::new(&prep, options);
    let count = sketch.constraints.len();
    // A constraint that has no value at the starting guess (the direction
    // of a line without length) sits this pass out, like a blamed one. If
    // the others give it a meaning, the next pass takes it in.
    let mut blamed = vec![false; count];
    for (eq, site) in prep.equations.iter().zip(&prep.source) {
        match (
            eq.eval_with(
                &prep.x0,
                options.tolerance.linear,
                &prep.splines,
                &prep.images,
            ),
            site,
        ) {
            (Err(Fail::Degenerate(_)), Site::Constraint(i)) => {
                if let Some(b) = blamed.get_mut(*i) {
                    *b = true;
                }
            }
            (Err(fail), at) => return Err(start_failure(*at, fail)),
            (Ok(_), _) => {}
        }
    }
    // Equations of the first `prefix` constraints that are not blamed, and
    // of all arcs.
    let active = |prefix: usize, blamed: &[bool]| -> Vec<bool> {
        prep.source
            .iter()
            .map(|site| match site {
                Site::Entity(_) => true,
                Site::Constraint(i) => *i < prefix && !blamed.get(*i).copied().unwrap_or(false),
            })
            .collect()
    };
    let gave_up = |budget: &Budget| SolveError::DidNotConverge {
        iterations: budget.steps_taken,
    };

    // First everything at once, from the starting guess. That is the whole
    // pass for a sketch that can be solved.
    let mut run = system.attempt(&prep.x0, true, &active(count, &blamed), budget)?;

    // Otherwise the constraints are taken in the order they were added,
    // which is how the sketch was built: `base` solves the first `good`
    // constraints (less the blamed ones). A bisection finds how many more
    // can be added in one go from there; the one after those is blamed.
    // (Too harshly, perhaps: it may fit from where the others end up. The
    // next pass of `solve` starts there and finds out.)
    let mut base = prep.x0.clone();
    let mut good: Option<usize> = None;
    let mut blames = 0;
    while !run.converged {
        if blames == MAX_BLAMED || budget.steps_left == 0 {
            return Err(gave_up(budget));
        }
        blames += 1;
        let mut lo = match good {
            Some(lo) => lo,
            None => {
                // Without any constraint only the arcs remain.
                let arcs = system.attempt(&base, true, &active(0, &blamed), budget)?;
                if !arcs.converged {
                    return Err(gave_up(budget));
                }
                base = arcs.x;
                0
            }
        };
        let mut hi = count;
        if hi <= lo {
            return Err(gave_up(budget));
        }
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            let part = system.attempt(&base, false, &active(mid, &blamed), budget)?;
            if part.converged {
                lo = mid;
                base = part.x;
            } else if budget.steps_left == 0 {
                return Err(gave_up(budget));
            } else {
                hi = mid;
            }
        }
        // The first `lo` constraints are solved at `base`; with constraint
        // `lo` added there is no solution from where the search started.
        match blamed.get_mut(lo) {
            Some(b) => *b = true,
            None => return Err(gave_up(budget)),
        }
        good = Some(hi);
        run = system.attempt(&base, false, &active(count, &blamed), budget)?;
    }

    let mut entities = sketch.entities.clone();
    for (entity, var) in entities.iter_mut().zip(&prep.entity_var) {
        let Some(var) = var else { continue };
        match entity {
            Entity::Point { x, y } => {
                if let (Some(sx), Some(sy)) = (run.x.get(*var), run.x.get(*var + 1)) {
                    *x = *sx;
                    *y = *sy;
                }
            }
            Entity::Circle { radius, .. } => {
                if let Some(r) = run.x.get(*var) {
                    *radius = *r;
                }
            }
            Entity::Line { .. } | Entity::Arc { .. } | Entity::Image { .. } => {}
        }
    }

    let mut reports = vec![
        ConstraintReport {
            status: ConstraintStatus::Satisfied,
            residual: 0.0,
            undefined: None,
        };
        count
    ];
    for (index, (eq, site)) in prep.equations.iter().zip(&prep.source).enumerate() {
        let rows = system.rows_of(index);
        let violated = rows.clone().any(|row| {
            run.dependent.get(row).copied().unwrap_or(false)
                && run
                    .r
                    .get(row)
                    .is_none_or(|r| r.abs() > system.tolerance.get(row).copied().unwrap_or(0.0))
        });
        let Site::Constraint(i) = site else {
            // An arc that is not an arc is not a solution.
            if violated {
                return Err(gave_up(budget));
            }
            continue;
        };
        let is_blamed = blamed.get(*i).copied().unwrap_or(false);
        let status = if is_blamed || violated {
            ConstraintStatus::Conflicting
        } else if rows
            .clone()
            .all(|row| run.dependent.get(row).copied().unwrap_or(false))
        {
            ConstraintStatus::Redundant
        } else {
            ConstraintStatus::Satisfied
        };
        if let Some(report) = reports.get_mut(*i) {
            let undefined = match eq.eval_with(
                &run.x,
                options.tolerance.linear,
                &prep.splines,
                &prep.images,
            ) {
                Err(Fail::Degenerate(reason)) => Some(reason),
                _ => None,
            };
            *report = ConstraintReport {
                status,
                residual: residual(eq, &run.x, options.tolerance.linear, &prep),
                undefined,
            };
        }
    }

    // Without the arcs' own equations (not the built-in equations of
    // splines through points: they only make control points of their own,
    // which no person's constraint can make redundant).
    let mut solve_arcs_last = active(count, &blamed);
    for ((on, site), eq) in solve_arcs_last
        .iter_mut()
        .zip(&prep.source)
        .zip(&prep.equations)
    {
        if matches!(site, Site::Entity(_)) && matches!(eq, Equation::ArcClosure { .. }) {
            *on = false;
        }
    }
    pinned_arc_ends(
        sketch,
        &system,
        &run.x,
        &solve_arcs_last,
        &mut budget.work,
        &mut reports,
    );

    let unknowns = run.x.len();
    Ok(Solution {
        entities,
        unknowns,
        rank: run.rank,
        dof: unknowns.saturating_sub(run.rank),
        constraints: reports,
        iterations: budget.steps_taken,
        carry: None,
    })
}

/// A point held by an `x` and a `y` coordinate is pinned, as `Fixed` pins
/// it, also when it is the end of an arc: the arc's own equation (its ends
/// equally far from its centre) then makes the second coordinate dependent
/// to first order, though it is what picks the end among the two on the
/// circle. Such a coordinate is reported satisfied, not redundant, when
/// (a) its partner on the same point is independent, and (b) without the
/// arcs' equations it is independent too: its dependence comes only from
/// an arc, which the person did not write and cannot remove. A coordinate
/// that the person's own constraints already imply stays redundant.
///
/// Diagnosis only: the solution, the rank and the degrees of freedom are
/// those of the solve; a factorisation that runs out of work leaves the
/// reports as they are.
fn pinned_arc_ends(
    sketch: &Sketch,
    system: &System<'_>,
    x: &[f64],
    without_arcs: &[bool],
    work: &mut Work,
    reports: &mut [ConstraintReport],
) {
    use crate::model::Constraint as C;
    let axis_of = |c: &C| match *c {
        C::CoordinateX { point, .. } => Some((point, 0)),
        C::CoordinateY { point, .. } => Some((point, 1)),
        _ => None,
    };
    let status = |reports: &[ConstraintReport], i: usize| reports.get(i).map(|r| r.status);
    // Redundant coordinates whose partner on the same point is satisfied.
    let candidates: Vec<usize> = sketch
        .constraints
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            let Some((point, axis)) = axis_of(c) else {
                return false;
            };
            status(reports, *i) == Some(ConstraintStatus::Redundant)
                && sketch.constraints.iter().enumerate().any(|(j, other)| {
                    axis_of(other) == Some((point, 1 - axis))
                        && status(reports, j) == Some(ConstraintStatus::Satisfied)
                })
        })
        .map(|(i, _)| i)
        .collect();
    let has_arcs = system
        .prep
        .equations
        .iter()
        .any(|eq| matches!(eq, Equation::ArcClosure { .. }));
    if candidates.is_empty() || !has_arcs {
        return;
    }
    let mut jac = Jacobian::new(x.len());
    if system.evaluate(x, without_arcs, Some(&mut jac)).is_err() {
        return;
    }
    let skip = system.skip_rows(without_arcs);
    let Ok(factor) = Factor::new(
        &jac,
        &skip,
        &system.negligible,
        system.options.rank_threshold,
        work,
    ) else {
        return;
    };
    for i in candidates {
        let Some(equation) = system
            .prep
            .source
            .iter()
            .position(|site| *site == Site::Constraint(i))
        else {
            continue;
        };
        let independent = system
            .rows_of(equation)
            .any(|row| !factor.dependent.get(row).copied().unwrap_or(true));
        if independent && let Some(report) = reports.get_mut(i) {
            report.status = ConstraintStatus::Satisfied;
        }
    }
}

/// How far an equation is from holding at `x`: the norm of its rows, or
/// infinity where it has no value.
fn residual(eq: &Equation, x: &[f64], min_len: f64, prep: &crate::equations::Prepared) -> f64 {
    match eq.eval_with(x, min_len, &prep.splines, &prep.images) {
        Ok(e) => e
            .rows
            .iter()
            .take(e.count)
            .fold(0.0f64, |norm, row| norm.phypot(row.v)),
        Err(_) => f64::INFINITY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Carried, EntityId};

    /// A whole path whose end the solve could not start from is a skip,
    /// naming the images and the solve's error; a stopped path stays
    /// stopped, and no carrying stays none.
    #[test]
    fn a_whole_path_the_solve_cannot_use_is_a_skip() {
        let why = SolveError::DidNotConverge { iterations: 7 };
        let moved = [EntityId(4)];
        match unusable_end(Some(Carried::Whole), &moved, &why) {
            Some(Carried::Skipped { reason, moved }) => {
                assert!(reason.contains("carried all the way"), "{reason}");
                assert!(reason.contains(&why.to_string()), "{reason}");
                assert_eq!(moved, vec![EntityId(4)]);
            }
            other => panic!("{other:?}"),
        }
        let stopped = Carried::Stopped {
            reached: 0.25,
            moved: moved.to_vec(),
            out_of_work: false,
        };
        assert_eq!(
            unusable_end(Some(stopped.clone()), &moved, &why),
            Some(stopped)
        );
        assert_eq!(unusable_end(None, &moved, &why), None);
    }

    /// An arc from (r, 0) at 30 degrees to 90 degrees about a held centre,
    /// its ends held at 90 and 270 degrees but drawn where they were.
    fn jumped() -> Sketch {
        use crate::model::Constraint as C;
        let mut s = Sketch::new();
        let c = s.add_point(0.0, 0.0);
        let a = s.add_point(30f64.to_radians().pcos(), 30f64.to_radians().psin());
        let b = s.add_point(0.0, 1.0);
        for (point, [x, y]) in [(c, [0.0, 0.0]), (a, [0.0, 1.0]), (b, [0.0, -1.0])] {
            s.add_constraint(C::CoordinateX { point, value: x });
            s.add_constraint(C::CoordinateY { point, value: y });
        }
        s.add_arc(c, a, b);
        s
    }

    /// The retry is taken, and the iterations of both attempts are counted.
    #[test]
    fn the_retry_counts_the_iterations_of_both_attempts() {
        let options = SolveOptions::new(craft3d_math::Tolerance::default());
        let sketch = jumped();
        let unmetered = || Meter {
            cap: None,
            spent: 0,
        };
        let first = settle(&sketch, &options, &mut unmetered()).unwrap();
        assert!(conflicts(&first) > 0, "{:?}", first.constraints);
        let placed = held_in_place(&sketch).unwrap();
        let second = settle(&placed, &options, &mut unmetered()).unwrap();
        assert_eq!(conflicts(&second), 0);
        let solution = solve(&sketch, &options).unwrap();
        assert_eq!(conflicts(&solution), 0);
        assert_eq!(
            solution.iterations,
            first.iterations + second.iterations,
            "{first:?} {second:?}"
        );
    }

    /// Both attempts together stay within the work limit: with too little
    /// left after the first, there is no retry.
    #[test]
    fn the_retry_spends_what_the_first_attempt_left() {
        let mut options = SolveOptions::new(craft3d_math::Tolerance::default());
        let mut meter = Meter {
            cap: None,
            spent: 0,
        };
        settle(&jumped(), &options, &mut meter).unwrap();
        options.work_limit = meter.spent;
        let solution = solve(&jumped(), &options).unwrap();
        assert!(conflicts(&solution) > 0, "no work was left for the retry");
        let capped = Meter {
            cap: Some(10),
            spent: 4,
        };
        assert_eq!(capped.next(&options, 1).limit(), 6);
    }

    /// A conflict between two holders of one coordinate stays from any
    /// start: no retry for it. One on a coordinate that is placed may be
    /// the start's fault: a retry.
    #[test]
    fn a_conflict_only_on_holders_that_are_not_placed_is_not_retried() {
        use crate::model::Constraint as C;
        let options = SolveOptions::new(craft3d_math::Tolerance::default());
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
        assert_eq!(placed_holders(&s), [true, false]);
        let solution = solve(&s, &options).unwrap();
        assert!(only_unplaced_holders(&s, &solution));
        let first = settle(
            &jumped(),
            &options,
            &mut Meter {
                cap: None,
                spent: 0,
            },
        )
        .unwrap();
        assert!(!only_unplaced_holders(&jumped(), &first));
    }

    /// The last attempt places a round's centre only where the held points
    /// fix it: through three held points apart (an arc's ends and a point
    /// on it; a circle's radius too), not through two (an arc with a radius
    /// has a centre on either side), not a centre that is held itself, and
    /// nothing for points in line.
    #[test]
    fn only_a_centre_the_held_points_fix_is_placed() {
        use crate::model::Constraint as C;
        let hold = |s: &mut Sketch, point: EntityId, [x, y]: [f64; 2]| {
            s.add_constraint(C::CoordinateX { point, value: x });
            s.add_constraint(C::CoordinateY { point, value: y });
        };
        let centre_of = |s: &Sketch, id: EntityId| match s.entities.get(id.index()) {
            Some(Entity::Point { x, y }) => [*x, *y],
            _ => panic!("no point"),
        };
        // An arc from (4, 0) round to (-4, 0) through (0, 2): centre
        // (0, -3), radius 5. Drawn with its centre far off.
        let arc = |third: Option<[f64; 2]>, centre_held: bool| {
            let mut s = Sketch::new();
            let a = s.add_point(4.0, 0.0);
            let b = s.add_point(-4.0, 0.0);
            let c = s.add_point(1e6, 3.0);
            let arc = s.add_arc(c, a, b);
            hold(&mut s, a, [4.0, 0.0]);
            hold(&mut s, b, [-4.0, 0.0]);
            if let Some(at) = third {
                let p = s.add_point(at[0], at[1]);
                s.add_constraint(C::PointOnCircle {
                    point: p,
                    circle: arc,
                });
                hold(&mut s, p, at);
            }
            if centre_held {
                s.add_constraint(C::Fixed { point: c, at: None });
            }
            (s, c)
        };
        let (s, c) = arc(Some([0.0, 2.0]), false);
        let (placed, rounds) = rounds_through_held_points(&s, 1e-6).unwrap();
        assert_eq!(rounds.len(), 1);
        assert_eq!(centre_of(&placed, c), [0.0, -3.0]);
        let (s, _) = arc(None, false);
        assert!(rounds_through_held_points(&s, 1e-6).is_none());
        let (s, _) = arc(Some([0.0, 2.0]), true);
        assert!(rounds_through_held_points(&s, 1e-6).is_none());
        let (s, _) = arc(Some([1.0, 0.0]), false);
        assert!(rounds_through_held_points(&s, 1e-6).is_none());
        // A point on the arc that is not held does not count.
        let (mut s, _) = arc(Some([0.0, 2.0]), false);
        s.constraints.truncate(s.constraints.len() - 2);
        assert!(rounds_through_held_points(&s, 1e-6).is_none());
        // A circle with a free radius through three held points.
        let mut s = Sketch::new();
        let c = s.add_point(7.0, 7.0);
        let circle = s.add_circle(c, 1.0);
        for at in [[4.0, 0.0], [-4.0, 0.0], [0.0, 2.0]] {
            let p = s.add_point(0.0, 0.0);
            s.add_constraint(C::PointOnCircle { point: p, circle });
            hold(&mut s, p, at);
        }
        let (placed, _) = rounds_through_held_points(&held_in_place(&s).unwrap(), 1e-6).unwrap();
        assert_eq!(centre_of(&placed, c), [0.0, -3.0]);
        assert_eq!(
            placed.entities.get(circle.index()),
            Some(&Entity::Circle {
                center: c,
                radius: 5.0
            })
        );
    }

    /// An arc through (-20, 0), (20, 0) and (0, b), held, counter-clockwise
    /// from (20, 0): stored solved at b = 6 (centre (0, -30.33)), its point
    /// now held at `b`. Flattened to b = 0.0001 the solve from the stored
    /// solution and the retry both blame the point (the centre runs off),
    /// and the last attempt finds the centre at (0, -2e6).
    fn stale_arc(b: f64) -> Sketch {
        use crate::model::Constraint as C;
        let mut s = Sketch::new();
        let a = s.add_point(-20.0, 0.0);
        let e = s.add_point(20.0, 0.0);
        let c = s.add_point(0.0, (36.0 - 400.0) / 12.0);
        let arc = s.add_arc(c, e, a);
        let p = s.add_point(0.0, 6.0);
        for (point, [x, y]) in [(a, [-20.0, 0.0]), (e, [20.0, 0.0])] {
            s.add_constraint(C::CoordinateX { point, value: x });
            s.add_constraint(C::CoordinateY { point, value: y });
        }
        s.add_constraint(C::PointOnCircle {
            point: p,
            circle: arc,
        });
        s.add_constraint(C::CoordinateX {
            point: p,
            value: 0.0,
        });
        s.add_constraint(C::CoordinateY { point: p, value: b });
        s
    }

    /// The last attempt spends only what the attempts before it left of
    /// the work limit: given exactly what it needs it solves, given half
    /// of that it runs out and the verdict of before stands, and so it
    /// does with nothing left. (Not one unit less: the last diagnosis of a
    /// pass, which arc ends are pinned, gives up quietly when out of work.)
    #[test]
    fn the_last_attempt_spends_only_what_the_others_left() {
        let mut options = SolveOptions::new(craft3d_math::Tolerance::default());
        let sketch = stale_arc(1e-4);
        let mut spent = 0;
        let before = solve_from_the_stored_solution(&sketch, &options, &mut spent).unwrap();
        assert!(conflicts(&before) > 0, "{:?}", before.constraints);
        let held = held_in_place(&sketch).unwrap_or_else(|| sketch.clone());
        let (start, _) = rounds_through_held_points(&held, options.tolerance.linear).unwrap();
        let mut meter = Meter {
            cap: None,
            spent: 0,
        };
        settle(&start, &options, &mut meter).unwrap();
        let needed = meter.spent;
        assert!(needed > 0);
        options.work_limit = spent + needed;
        let solved = solve(&sketch, &options).unwrap();
        assert_eq!(conflicts(&solved), 0, "{:?}", solved.constraints);
        for limit in [spent + needed / 2, spent] {
            options.work_limit = limit;
            let again = solve(&sketch, &options).unwrap();
            assert_eq!(again, before, "limit {limit}");
        }
    }

    /// The last attempt is not made from a start on which a choice reads
    /// otherwise than on the stored solution: here a line tangent to the
    /// arc, the arc's centre on its left as stored; were the centre placed
    /// on its right, the tangency would turn over.
    #[test]
    fn a_start_that_turns_a_choice_over_is_not_taken() {
        use crate::model::Constraint as C;
        let mut s = Sketch::new();
        let c = s.add_point(0.0, 0.0);
        let a = s.add_point(1.0, 0.0);
        let b = s.add_point(0.0, 1.0);
        let arc = s.add_arc(c, a, b);
        // A line through (3, -1) and (3, 1): the centre on its left.
        let q = s.add_point(3.0, -1.0);
        let f = s.add_point(3.0, 1.0);
        let line = s.add_line(q, f);
        s.add_constraint(C::Tangent { a: line, b: arc });
        let stored = s.clone();
        let mut moved = s;
        if let Some(centre) = moved.entities.get_mut(c.index()) {
            *centre = Entity::Point { x: 5.0, y: 0.0 };
        }
        assert!(same_choices(&stored, &stored, 1e-6));
        assert!(!same_choices(&stored, &moved, 1e-6));
    }
}
