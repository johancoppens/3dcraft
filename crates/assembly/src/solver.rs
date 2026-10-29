//! The solver: one solve, and a sequence of driven values solved by
//! continuation on one branch.

use crate::analysis::{
    dependencies, diagnose_rest, driver_loop_mates, free_motions, moved_mates, rank_and_nulls,
    redundancies,
};
use crate::newton::{MAX_ITERATIONS, evaluate, gauss_newton, run};
use crate::system::{Rows, State, System};
use crate::types::{DeadPoint, Diagnosis, Problem, Sequence, Solution};

/// Sub-steps allowed for one advance (halvings and doublings together).
const MAX_SUBSTEPS: usize = 4096;

/// Newton iterations allowed for one advance, all sub-steps together.
const MAX_ADVANCE_ITERATIONS: usize = 20_000;

/// The corrector may land this many tolerances (relative) off the
/// prediction beyond [`CONTINUITY`] of the motion: the accuracy of a
/// solution, so a sub-step of any size is still checked.
const CONTINUITY_FLOOR: f64 = 100.0;

/// A continuation step is accepted when the corrected solution lies within
/// this fraction of the predicted motion from the prediction: a corrector
/// that travels further has left the branch the prediction follows.
const CONTINUITY: f64 = 0.5;

/// A solver for one problem. It keeps the last solution, so
/// [`Solver::advance`] can follow the motion from it on the same branch.
#[derive(Debug)]
pub struct Solver {
    sys: System,
    state: Option<State>,
    drives: Vec<f64>,
    last: Option<Solution>,
    /// The solution before the last and its drives, for the secant.
    prev: Option<(State, Vec<f64>)>,
}

/// A solved state and how it was reached.
struct Corrected {
    state: State,
    iterations: usize,
    active: Vec<bool>,
    drives: Vec<f64>,
}

impl Solver {
    /// Checks the problem; `Err` names what is malformed.
    pub fn new(p: &Problem) -> Result<Solver, Diagnosis> {
        let sys = System::new(p)?;
        Ok(Solver {
            sys,
            state: None,
            drives: Vec::new(),
            last: None,
            prev: None,
        })
    }

    /// The problem, its frames made orthonormal and flipped mates turned.
    pub fn problem(&self) -> &Problem {
        &self.sys.problem
    }

    /// The last solution found.
    pub fn last(&self) -> Option<&Solution> {
        self.last.as_ref()
    }

    /// The problem's own driven values, in [`Problem::driven_slots`] order.
    pub fn problem_drives(&self) -> Vec<f64> {
        self.sys
            .slots
            .iter()
            .filter_map(|&(m, v)| {
                self.sys
                    .problem
                    .mates
                    .get(m)
                    .and_then(|s| s.driven.get(v).copied().flatten())
            })
            .collect()
    }

    /// Solves from the problem's start poses at its own driven values.
    pub fn solve(&mut self) -> Result<Solution, Diagnosis> {
        let drives = self.problem_drives();
        self.solve_at(&drives)
    }

    /// Solves from the problem's start poses at `drives` (in
    /// [`Problem::driven_slots`] order), forgetting any earlier solution.
    pub fn solve_at(&mut self, drives: &[f64]) -> Result<Solution, Diagnosis> {
        self.check_drives(drives)?;
        let start = self.sys.start_state(drives);
        let c = self.correct(&start, drives)?;
        let sol = self.solution(&c)?;
        self.state = Some(c.state);
        self.prev = None;
        self.drives = drives.to_vec();
        self.last = Some(sol.clone());
        Ok(sol)
    }

    /// Moves the drives to `drives` from the last solution, following the
    /// motion on its branch: the step is cut while the solution jumps
    /// further than the motion predicts. Without an earlier solution it
    /// first solves at the problem's own drives (where its start poses
    /// are) and moves from there. On an error the solver keeps the last
    /// state it reached, and [`Diagnosis::DeadPoint`] gives its drives.
    pub fn advance(&mut self, drives: &[f64]) -> Result<Solution, Diagnosis> {
        self.check_drives(drives)?;
        if self.state.is_none() {
            self.solve()?;
        }
        let Some(mut state) = self.state.clone() else {
            return self.solve_at(drives);
        };
        let from = self.drives.clone();
        // The largest change of a drive, dimensionless.
        let span = self.drive_distance(&from, drives);
        let at = |s: f64| -> Vec<f64> {
            from.iter()
                .zip(drives)
                .map(|(a, b)| if s >= 1.0 { *b } else { a + (b - a) * s })
                .collect()
        };
        let floor = CONTINUITY_FLOOR * self.sys.eps;
        let min_step = if span > 0.0 { self.sys.eps / span } else { 1.0 };
        let base = self.base_active();
        let mut s = 0.0f64;
        let mut step = 1.0f64;
        let mut reached: Option<Corrected> = None;
        let mut failure: Option<Diagnosis> = None;
        let mut substeps = 0usize;
        let mut iterations = 0usize;
        let mut exhausted = false;
        while s < 1.0 {
            substeps += 1;
            if substeps > MAX_SUBSTEPS || iterations > MAX_ADVANCE_ITERATIONS {
                exhausted = true;
                break;
            }
            let s_try = (s + step).min(1.0);
            let d_try = at(s_try);
            // Predict: along the secant of the last two solutions when the
            // drives keep their direction, else along the tangent.
            let predicted = self
                .secant(&state, &d_try)
                .or_else(|| gauss_newton(&self.sys, &state, &d_try, &base))
                .unwrap_or_else(|| state.clone());
            let tried = self.correct(&predicted, &d_try);
            iterations += match &tried {
                Ok(c) => c.iterations,
                Err(Diagnosis::DidNotConverge { iterations, .. }) => *iterations,
                Err(_) => MAX_ITERATIONS,
            };
            let ok = match &tried {
                Ok(c) => {
                    let motion = self.sys.distance(&state, &predicted);
                    self.sys.distance(&c.state, &predicted) <= CONTINUITY * motion + floor
                }
                Err(_) => false,
            };
            match tried {
                Ok(c) if ok => {
                    self.prev = Some((state.clone(), self.drives.clone()));
                    state = c.state.clone();
                    s = s_try;
                    reached = Some(c);
                    self.drives = d_try;
                    step = (step * 2.0).min(1.0);
                }
                Ok(_) => {
                    failure = Some(Diagnosis::BranchLost { drives: d_try });
                    step *= 0.5;
                }
                Err(e) => {
                    failure = Some(e);
                    step *= 0.5;
                }
            }
            if s < 1.0 && step < min_step {
                break;
            }
        }
        if let Some(c) = &reached {
            if let Ok(sol) = self.solution(c) {
                self.last = Some(sol);
            }
            self.state = Some(state.clone());
        }
        if s >= 1.0 {
            return self.last.clone().ok_or(Diagnosis::DidNotConverge {
                iterations,
                residual: f64::NAN,
            });
        }
        if exhausted {
            // Out of work, not at a limit position: the motion asks for more
            // sub-steps than allowed (a step too large for one advance).
            return Err(Diagnosis::DidNotConverge {
                iterations,
                residual: f64::NAN,
            });
        }
        let cause = failure.unwrap_or(Diagnosis::DidNotConverge {
            iterations,
            residual: f64::NAN,
        });
        // The sub-step shrank to nothing: whether the last try came to rest
        // or jumped off the branch, the drive cannot go on from here (at a
        // fold the corrector converges, but beyond the motion predicted).
        let loop_mates = self.driver_loop();
        Err(Diagnosis::DeadPoint {
            drives: self.drives.clone(),
            loop_mates,
            cause: Box::new(cause),
        })
    }

    /// The mates the motions free at the last solution move: at a limit
    /// position, the loop the drive no longer controls.
    fn driver_loop(&self) -> Vec<usize> {
        self.last
            .as_ref()
            .map(|l| self.loop_of(l))
            .unwrap_or_default()
    }

    /// The loop at a dead point, both ways it shows: the mates the free
    /// motions move (a loop that lost rank on its own, such as a coupler and
    /// rocker in line) and the mates whose rows came to depend on a
    /// driver's (the drive at its reach).
    fn loop_of(&self, sol: &Solution) -> Vec<usize> {
        let mut m = moved_mates(&self.sys, &sol.free);
        m.extend(driver_loop_mates(&sol.redundant));
        m.sort_unstable();
        m.dedup();
        m
    }

    /// The largest change between two sets of drives, dimensionless.
    fn drive_distance(&self, a: &[f64], b: &[f64]) -> f64 {
        self.sys
            .slots
            .iter()
            .zip(a.iter().zip(b))
            .fold(0.0f64, |m, (&(mt, v), (x, y))| {
                m.max(((y - x) / self.sys.unit(mt, v)).abs())
            })
    }

    /// The secant predictor: the last solution moved on by the motion from
    /// the one before, in proportion to the drives, when the new drives go
    /// on in the same direction (within 1e-6 of parallel).
    fn secant(&self, state: &State, d_new: &[f64]) -> Option<State> {
        let (prev_state, prev_drives) = self.prev.as_ref()?;
        let units: Vec<f64> = self
            .sys
            .slots
            .iter()
            .map(|&(m, v)| self.sys.unit(m, v))
            .collect();
        let back: Vec<f64> = self
            .drives
            .iter()
            .zip(prev_drives)
            .zip(&units)
            .map(|((c, p), u)| (c - p) / u)
            .collect();
        let ahead: Vec<f64> = d_new
            .iter()
            .zip(&self.drives)
            .zip(&units)
            .map(|((n, c), u)| (n - c) / u)
            .collect();
        let bb: f64 = back.iter().map(|x| x * x).sum();
        let aa: f64 = ahead.iter().map(|x| x * x).sum();
        let ab: f64 = back.iter().zip(&ahead).map(|(x, y)| x * y).sum();
        if !(bb > 0.0 && aa > 0.0) || ab <= 0.0 || ab * ab < (1.0 - 1e-6) * aa * bb {
            return None;
        }
        let ratio = ab / bb;
        let h: Vec<f64> = self
            .sys
            .difference(prev_state, state)?
            .iter()
            .map(|x| x * ratio)
            .collect();
        self.sys.step(state, &h)
    }

    /// The groups active before any limit is met: all but the limits.
    fn base_active(&self) -> Vec<bool> {
        self.sys
            .groups
            .iter()
            .map(|g| !matches!(g.rows, Rows::Limit { .. }))
            .collect()
    }

    /// Solves a sequence of driven values in order by [`Solver::advance`]
    /// (from the problem's own drives when nothing is solved yet). A frame where the rank drops is a dead point; a frame the
    /// drive cannot reach stops the sequence, with a dead point at the
    /// last value reached when the mechanism came to a limit position.
    pub fn sequence(&mut self, frames: &[Vec<f64>]) -> Sequence {
        let mut out = Sequence {
            frames: Vec::with_capacity(frames.len().min(1 << 16)),
            dead_points: Vec::new(),
            stopped: None,
        };
        if self.state.is_none()
            && let Err(e) = self.solve()
        {
            out.stopped = Some((0, e));
            return out;
        }
        let base: Option<usize> = self.last.as_ref().map(|s| s.dof);
        for (k, d) in frames.iter().enumerate() {
            match self.advance(d) {
                Ok(sol) => {
                    if let Some(b) = base
                        && sol.dof > b
                    {
                        out.dead_points.push(DeadPoint {
                            frame: k,
                            drives: sol.drives.clone(),
                            extra_dof: sol.dof - b,
                            loop_mates: self.loop_of(&sol),
                            free: sol.free.clone(),
                        });
                    }
                    out.frames.push(sol);
                }
                Err(e) => {
                    if let (
                        Diagnosis::DeadPoint {
                            drives, loop_mates, ..
                        },
                        Some(last),
                    ) = (&e, &self.last)
                    {
                        out.dead_points.push(DeadPoint {
                            frame: k,
                            drives: drives.clone(),
                            extra_dof: base.map_or(0, |b| last.dof.saturating_sub(b)),
                            loop_mates: loop_mates.clone(),
                            free: last.free.clone(),
                        });
                    }
                    out.stopped = Some((k, e));
                    break;
                }
            }
        }
        out
    }

    fn check_drives(&self, drives: &[f64]) -> Result<(), Diagnosis> {
        if drives.len() != self.sys.slots.len() {
            return Err(Diagnosis::Invalid {
                reason: format!(
                    "{} driven values for {} driven variables",
                    drives.len(),
                    self.sys.slots.len()
                ),
            });
        }
        let reach = self.sys.problem.tolerance / crate::system::FINEST_RELATIVE;
        for (&(m, v), &d) in self.sys.slots.iter().zip(drives) {
            let cap = if self.sys.unit(m, v) == 1.0 {
                crate::system::MAX_ANGLE
            } else {
                reach
            };
            if !d.is_finite() || d.abs() > cap {
                return Err(Diagnosis::Invalid {
                    reason: format!(
                        "{}: the driven value is not finite or too large",
                        self.sys.problem.mate_name(m)
                    ),
                });
            }
            let lim = self
                .sys
                .problem
                .mates
                .get(m)
                .and_then(|s| s.limits.get(v).copied().flatten());
            if let Some((lo, hi)) = lim {
                let slack = self.sys.eps * self.sys.unit(m, v);
                if d < lo - slack || d > hi + slack {
                    return Err(Diagnosis::OutOfLimits {
                        mate: m,
                        variable: v,
                        value: d,
                        limits: (lo, hi),
                    });
                }
            }
        }
        Ok(())
    }

    /// Newton from `start` at `drives`, the limits held as they are met.
    fn correct(&self, start: &State, drives: &[f64]) -> Result<Corrected, Diagnosis> {
        let sys = &self.sys;
        let mut active: Vec<bool> = sys
            .groups
            .iter()
            .map(|g| !matches!(g.rows, Rows::Limit { .. }))
            .collect();
        let limits = active.iter().filter(|a| !**a).count();
        let mut state = start.clone();
        let mut used = 0usize;
        for _pass in 0..=limits {
            let budget = MAX_ITERATIONS.saturating_sub(used);
            let Some(r) = run(sys, &state, drives, &active, budget) else {
                return Err(Diagnosis::DidNotConverge {
                    iterations: used,
                    residual: f64::INFINITY,
                });
            };
            used += r.iterations;
            if !r.converged {
                return Err(if r.stalled {
                    diagnose_rest(sys, &r.state, drives, &active, used, r.residual)
                } else {
                    Diagnosis::DidNotConverge {
                        iterations: used,
                        residual: r.residual * sys.scale,
                    }
                });
            }
            state = r.state;
            // Limits the solution passes, by more than the tolerance.
            let mut newly = false;
            for (gi, g) in sys.groups.iter().enumerate() {
                let Rows::Limit { mate, var } = g.rows else {
                    continue;
                };
                if active.get(gi).copied().unwrap_or(true) {
                    continue;
                }
                let lim = sys
                    .problem
                    .mates
                    .get(mate)
                    .and_then(|s| s.limits.get(var).copied().flatten());
                let q = state.values.get(mate).and_then(|v| v.get(var)).copied();
                if let (Some((lo, hi)), Some(q)) = (lim, q) {
                    let slack = sys.eps * sys.unit(mate, var);
                    if q < lo - slack || q > hi + slack {
                        if let Some(a) = active.get_mut(gi) {
                            *a = true;
                        }
                        newly = true;
                    }
                }
            }
            if !newly {
                return Ok(Corrected {
                    state,
                    iterations: used,
                    active,
                    drives: drives.to_vec(),
                });
            }
        }
        Err(Diagnosis::DidNotConverge {
            iterations: used,
            residual: f64::NAN,
        })
    }

    /// The solution at a solved state: values, rank, free motions and
    /// redundancies.
    fn solution(&self, c: &Corrected) -> Result<Solution, Diagnosis> {
        let sys = &self.sys;
        let fail = || Diagnosis::DidNotConverge {
            iterations: c.iterations,
            residual: f64::NAN,
        };
        let evals = evaluate(sys, &c.state, &c.drives, &c.active).ok_or_else(fail)?;
        let residual = crate::newton::max_residual(&evals);
        let (rank, nulls) = rank_and_nulls(sys, &evals).ok_or_else(fail)?;
        let deps = dependencies(sys, &evals).ok_or_else(fail)?;
        let equations = sys
            .groups
            .iter()
            .zip(&c.active)
            .filter(|(_, a)| **a)
            .map(|(g, _)| g.size)
            .sum();
        let at_limit = sys
            .groups
            .iter()
            .zip(&c.active)
            .filter_map(|(g, a)| match g.rows {
                Rows::Limit { mate, var } if *a => Some((mate, var)),
                _ => None,
            })
            .collect();
        Ok(Solution {
            poses: c.state.poses.clone(),
            values: c.state.values.clone(),
            dof: sys.unknowns.saturating_sub(rank),
            rank,
            equations,
            free: free_motions(sys, &c.state, &nulls),
            redundant: redundancies(sys, &deps),
            at_limit,
            drives: c.drives.clone(),
            floating: !sys.grounded.iter().any(|g| *g),
            residual: residual * sys.scale,
            iterations: c.iterations,
        })
    }
}
