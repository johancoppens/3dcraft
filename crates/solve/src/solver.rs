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

use crate::equations::{Equation, Fail, Prepared, start_failure};
use crate::error::{Site, SolveError};
use crate::factor::{Factor, Jacobian, Work};
use crate::model::{ConstraintReport, ConstraintStatus, Entity, Sketch, Solution, SolveOptions};
use crate::{MAX_BLAMED, RUNS_PER_SOLVE};

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
        for eq in &prep.equations {
            row_start.push(tolerance.len());
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
        }
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
                let dx = factor.min_norm(&rhs);
                let longest = dx.iter().fold(0.0f64, |m, v| m.max(v.abs()));
                if longest > 0.0 && longest.is_finite() {
                    let before = merit(&r);
                    let mut t = 1.0;
                    blocked = false;
                    for _ in 0..MAX_HALVINGS {
                        let trial: Vec<f64> = x.iter().zip(&dx).map(|(x, d)| x + t * d).collect();
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
pub(crate) fn solve(sketch: &Sketch, options: &SolveOptions) -> Result<Solution, SolveError> {
    validate(options)?;
    let mut meter = Meter {
        cap: None,
        spent: 0,
    };
    let first = settle(sketch, options, &mut meter);
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
fn solve_once(
    sketch: &Sketch,
    options: &SolveOptions,
    meter: &mut Meter,
) -> Result<Solution, SolveError> {
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

    let mut solve_arcs_last = active(count, &blamed);
    for (on, site) in solve_arcs_last.iter_mut().zip(&prep.source) {
        if matches!(site, Site::Entity(_)) {
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
        .source
        .iter()
        .any(|site| matches!(site, Site::Entity(_)));
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
            .fold(0.0f64, |norm, row| norm.hypot(row.v)),
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
        let a = s.add_point(30f64.to_radians().cos(), 30f64.to_radians().sin());
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
}
