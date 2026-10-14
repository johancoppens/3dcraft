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
use crate::spline::SplineVars;
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

struct System<'a> {
    prep: &'a Prepared,
    options: &'a SolveOptions,
    /// Per equation: its first row. One more entry than equations.
    row_start: Vec<usize>,
    /// Per row: the residual below which the row holds.
    tolerance: Vec<f64>,
    /// Per row: the unit its residual is measured in for the merit function.
    weight: Vec<f64>,
    /// Per row: the gradient length at or below which there is no gradient.
    negligible: Vec<f64>,
}

impl<'a> System<'a> {
    fn new(prep: &'a Prepared, options: &'a SolveOptions) -> Self {
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
    fn evaluate(
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
                .eval_with(x, self.options.tolerance.linear, &self.prep.splines)
                .map_err(|f| (index, f))?;
            for row in e.rows.iter().take(e.count) {
                r.push(row.v);
                if let Some(j) = jac.as_deref_mut() {
                    for (col, d) in e.slots.iter().zip(row.d).take(e.used) {
                        j.push(*col, d);
                    }
                    j.end_row();
                }
            }
        }
        Ok(r)
    }

    fn skip_rows(&self, active: &[bool]) -> Vec<bool> {
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
            let Ok(e) = eq.eval_with(&run.x, self.options.tolerance.linear, &self.prep.splines)
            else {
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
    let mut current = solve_once(sketch, options)?;
    let mut iterations = current.iterations;
    let mut close = false;
    for _ in 0..SETTLE_PASSES {
        let guess = Sketch {
            entities: current.entities.clone(),
            constraints: sketch.constraints.clone(),
            splines: sketch.splines.clone(),
        };
        let mut next = solve_once(&guess, options)?;
        iterations = iterations.saturating_add(next.iterations);
        next.iterations = iterations;
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

fn solve_once(sketch: &Sketch, options: &SolveOptions) -> Result<Solution, SolveError> {
    let prep = crate::equations::prepare(sketch, options.tolerance.linear)?;
    let system = System::new(&prep, options);
    let mut budget = Budget {
        steps_left: options.max_iterations.saturating_mul(RUNS_PER_SOLVE),
        steps_taken: 0,
        work: Work::new(options.work_limit),
    };
    let count = sketch.constraints.len();
    // A constraint that has no value at the starting guess (the direction
    // of a line without length) sits this pass out, like a blamed one. If
    // the others give it a meaning, the next pass takes it in.
    let mut blamed = vec![false; count];
    for (eq, site) in prep.equations.iter().zip(&prep.source) {
        match (
            eq.eval_with(&prep.x0, options.tolerance.linear, &prep.splines),
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
    let mut run = system.attempt(&prep.x0, true, &active(count, &blamed), &mut budget)?;

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
            return Err(gave_up(&budget));
        }
        blames += 1;
        let mut lo = match good {
            Some(lo) => lo,
            None => {
                // Without any constraint only the arcs remain.
                let arcs = system.attempt(&base, true, &active(0, &blamed), &mut budget)?;
                if !arcs.converged {
                    return Err(gave_up(&budget));
                }
                base = arcs.x;
                0
            }
        };
        let mut hi = count;
        if hi <= lo {
            return Err(gave_up(&budget));
        }
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            let part = system.attempt(&base, false, &active(mid, &blamed), &mut budget)?;
            if part.converged {
                lo = mid;
                base = part.x;
            } else if budget.steps_left == 0 {
                return Err(gave_up(&budget));
            } else {
                hi = mid;
            }
        }
        // The first `lo` constraints are solved at `base`; with constraint
        // `lo` added there is no solution from where the search started.
        match blamed.get_mut(lo) {
            Some(b) => *b = true,
            None => return Err(gave_up(&budget)),
        }
        good = Some(hi);
        run = system.attempt(&base, false, &active(count, &blamed), &mut budget)?;
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
            Entity::Line { .. } | Entity::Arc { .. } => {}
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
                return Err(gave_up(&budget));
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
            let undefined = match eq.eval_with(&run.x, options.tolerance.linear, &prep.splines) {
                Err(Fail::Degenerate(reason)) => Some(reason),
                _ => None,
            };
            *report = ConstraintReport {
                status,
                residual: residual(eq, &run.x, options.tolerance.linear, &prep.splines),
                undefined,
            };
        }
    }

    let unknowns = run.x.len();
    Ok(Solution {
        entities,
        unknowns,
        rank: run.rank,
        dof: unknowns.saturating_sub(run.rank),
        constraints: reports,
        iterations: budget.steps_taken,
    })
}

/// How far an equation is from holding at `x`: the norm of its rows, or
/// infinity where it has no value.
fn residual(eq: &Equation, x: &[f64], min_len: f64, splines: &[SplineVars]) -> f64 {
    match eq.eval_with(x, min_len, splines) {
        Ok(e) => e
            .rows
            .iter()
            .take(e.count)
            .fold(0.0f64, |norm, row| norm.hypot(row.v)),
        Err(_) => f64::INFINITY,
    }
}
