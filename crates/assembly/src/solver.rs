//! The solver: one solve, and a sequence of driven values solved by
//! continuation on one branch.

use crate::analysis::{
    dependencies, diagnose_rest, driver_loop_mates, free_motions, moved_mates, rank_and_nulls,
    redundancies,
};
use crate::newton::{MAX_ITERATIONS, MAX_STEP, evaluate, gauss_newton, run};
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

/// The largest change of a drive in one sub-step when a rest solve is
/// carried from where its parts are placed (dimensionless: radians, or
/// the scale): the most one Newton step may move anything, so a drive far
/// from the start walks there as a far start walks in, instead of leaping
/// across branches in one prediction the continuity check is too loose to
/// catch (its bound grows with the step).
const REST_STEP: f64 = MAX_STEP;

/// Newton iterations a rest solve's carry may spend, all sub-steps
/// together: ten solves' worth, so a placement whose direct solve failed
/// costs at most a few times a solve more. A carry that runs out is not
/// used, and the direct solve's failure stands.
const REST_ITERATIONS: usize = 10 * MAX_ITERATIONS;

/// Newton iterations one sub-step of a rest carry may spend: a quarter of
/// a solve's. A sub-step's corrector starts from a prediction at most half
/// a radian away and converges in a few iterations on the branch (two on
/// average over the 40 turns of a lead screw); one that does not is cut,
/// so a limit position is found by halving (to ε of the drives' change, as
/// a motion's) within the carry's budget: about 1000 iterations for the
/// double-rocker of the tests, where a full solve's 200 per try spent
/// 7000. Cutting only to √ε was tried and refused: a Jansen leg carried
/// from 250° to 80° passes a near-singular pose at 193° only in steps
/// finer than that.
const REST_STEP_ITERATIONS: usize = MAX_ITERATIONS / 4;

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

/// Which angles of a start take their turns from an earlier solve.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Turns {
    /// None: a placement, every angle as measured (a driven one the turns
    /// nearest its drive, a limited one nearest its range).
    Measured,
    /// Every angle no drive sets (a joint held by hand, `view.drive`).
    Every,
    /// The angles a coupling takes (a moment of a motion).
    Coupled,
}

/// How a carry goes: a sub-step changes a drive by at most `largest`
/// (dimensionless), its corrector spends at most `per_step` iterations,
/// the whole carry at most `budget`, a sub-step is not cut below `finest`
/// of the drives' change, and with `analyse` the state reached is analysed
/// (rank, free motions) as [`Solver::last`].
#[derive(Clone, Copy)]
struct Pace {
    largest: f64,
    budget: usize,
    per_step: usize,
    finest: f64,
    analyse: bool,
}

impl Pace {
    /// A motion's ([`Solver::advance`]): the first sub-step the whole way.
    const MOTION: Pace = Pace {
        largest: f64::INFINITY,
        budget: MAX_ADVANCE_ITERATIONS,
        per_step: MAX_ITERATIONS,
        finest: 0.0,
        analyse: true,
    };
    /// A rest solve's carry.
    const REST: Pace = Pace {
        largest: REST_STEP,
        budget: REST_ITERATIONS,
        per_step: REST_STEP_ITERATIONS,
        finest: 0.0,
        analyse: false,
    };

    fn with_finest(self, finest: f64) -> Pace {
        Pace { finest, ..self }
    }
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
    /// One solve straight from the start, as always; when it fails, from
    /// the turns the start's couplings say (see [`Self::rest`]).
    pub fn solve_at(&mut self, drives: &[f64]) -> Result<Solution, Diagnosis> {
        self.check_drives(drives)?;
        self.rest(drives, None, Turns::Measured)
    }

    /// [`Self::solve_at`], each angle no drive sets taken the full turns
    /// nearest its value in `near` (per mate, per variable, as
    /// [`Solution::values`]): a solve carried from an earlier one, whose
    /// poses do not say how many turns a joint had come.
    pub fn solve_at_near(
        &mut self,
        drives: &[f64],
        near: &[Vec<f64>],
    ) -> Result<Solution, Diagnosis> {
        self.check_drives(drives)?;
        self.rest(drives, Some(near), Turns::Every)
    }

    /// [`Self::solve_at_near`] for a moment of a motion carried from the
    /// moment before (its poses, drives and values in `near`): only the
    /// angles a coupling takes start the full turns nearest their values in
    /// `near`, since a coupling counts their turns; every other equation is
    /// periodic in an angle, and the other angles are measured in (−π, π]
    /// as from a placement. A screw that nothing drives but its coupling,
    /// many turns round, started in (−π, π] had its coupling a whole number
    /// of leads unmet and was walked back half a radian an iteration.
    pub fn solve_at_near_coupled(
        &mut self,
        drives: &[f64],
        near: &[Vec<f64>],
    ) -> Result<Solution, Diagnosis> {
        self.check_drives(drives)?;
        self.rest(drives, Some(near), Turns::Coupled)
    }

    /// The solve from the start poses at `drives`, its angles' turns taken
    /// from `near` as `turns` says: one solve straight from the start, as
    /// always, whose answer stands whenever it has one. Only when it fails
    /// (A-030: a shaft that only its coupling turns, many turns from the
    /// start's (−π, π]), the placement is solved again from the turns its
    /// couplings say, and else carried from the drive values it holds to
    /// `drives` ([`Self::carry_rest`]): so a direct answer is never
    /// replaced, and only a failure can turn into an answer.
    fn rest(
        &mut self,
        drives: &[f64],
        near: Option<&[Vec<f64>]>,
        turns: Turns,
    ) -> Result<Solution, Diagnosis> {
        // A failed solve leaves the solver as it was.
        let before = (
            self.state.clone(),
            self.drives.clone(),
            self.last.clone(),
            self.prev.clone(),
        );
        let solved = self.rest_from_start(drives, near, turns);
        if solved.is_err() {
            (self.state, self.drives, self.last, self.prev) = before;
        }
        solved
    }

    fn rest_from_start(
        &mut self,
        drives: &[f64],
        near: Option<&[Vec<f64>]>,
        turns: Turns,
    ) -> Result<Solution, Diagnosis> {
        // The start as a solve has always taken it (a moment's coupled
        // angles in (−π, π] too), so a solve that converged before
        // converges to the same bits.
        let mut start = self.sys.start_state(drives);
        if let (Turns::Every, Some(near)) = (turns, near) {
            self.sys.keep_turns(&mut start, near);
        }
        let direct = match self.correct(&start, drives) {
            Ok(c) => return self.settle(c, drives),
            Err(direct) => direct,
        };
        let c = match turns {
            // A moment whose coupled shafts had come many turns: started at
            // the turns they had in the moment before.
            Turns::Coupled => {
                let Some(near) = near else {
                    return Err(direct);
                };
                let mut carried = start.clone();
                self.sys.keep_coupled_turns(&mut carried, near);
                if carried == start {
                    return Err(direct);
                }
                self.correct(&carried, drives).map_err(|_| direct)?
            }
            Turns::Every => return Err(direct),
            Turns::Measured => self.solve_by_coupled_turns(&start, drives).ok_or(direct)?,
        };
        self.settle(c, drives)
    }

    /// A placement whose direct solve failed, solved from the turns its
    /// couplings say (A-030), or `None` where they say nothing new: an
    /// angle no drive sets that is a coupling's only such angle started at
    /// the whole turns that meet the coupling best (a screw placed with its
    /// nut 27 leads along); else, when a driven angle a coupling takes is
    /// placed whole turns from where the turns nearest its drive put it (a
    /// lead screw placed at 0 with its nut and driven 16.8 turns), carried
    /// from the placement's turns to the drives with its nut. Nothing else
    /// is tried: a failure that is not a coupling's turns stays the error
    /// it was (a carry across a linkage's near-singular pose can change
    /// circuits, which an error never does).
    fn solve_by_coupled_turns(&mut self, start: &State, drives: &[f64]) -> Option<Corrected> {
        let mut counted = start.clone();
        self.sys.coupled_start_turns(&mut counted);
        if counted != *start
            && let Ok(c) = self.correct(&counted, drives)
        {
            return Some(c);
        }
        let held = self.sys.start_drives(&counted, true);
        let nearest = self.sys.start_drives(&counted, false);
        if !self.sys.drives_apart(&held, &nearest)
            || !self.sys.drives_apart(&held, drives)
            || self.check_drives(&held).is_err()
        {
            return None;
        }
        let mut from = self.sys.start_state(&held);
        self.sys.coupled_start_turns(&mut from);
        let mut c = self.carry_rest(&from, &held, drives)?;
        // A coupled drive carried whole turns turns the joints on it as
        // many; a solve from the placement does not count them where no
        // equation does.
        if self.sys.winds(&held, drives) {
            self.sys.turns_nearest_start(&mut c.state, start);
        }
        Some(c)
    }

    /// The solution at `c`, kept as the solver's state at `drives`.
    fn settle(&mut self, c: Corrected, drives: &[f64]) -> Result<Solution, Diagnosis> {
        let sol = self.solution(&c)?;
        self.state = Some(c.state);
        self.prev = None;
        self.drives = drives.to_vec();
        self.last = Some(sol.clone());
        Ok(sol)
    }

    /// Solves `start` at the driven values `held` and carries the solution
    /// to `drives` along its branch; `None` when either cannot be done.
    fn carry_rest(&mut self, start: &State, held: &[f64], drives: &[f64]) -> Option<Corrected> {
        let c0 = self.correct(start, held).ok()?;
        let first = c0.iterations;
        self.state = Some(c0.state);
        self.prev = None;
        self.drives = held.to_vec();
        self.last = None;
        let (reached, outcome) = self.carry(drives, Pace::REST.with_finest(self.sys.eps));
        outcome.ok()?;
        let mut c = reached?;
        c.iterations = c.iterations.saturating_add(first);
        Some(c)
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
        if self.state.is_none() {
            return self.solve_at(drives);
        }
        let finest = self.sys.eps;
        let (_, outcome) = self.carry(drives, Pace::MOTION.with_finest(finest));
        outcome?;
        self.last.clone().ok_or(Diagnosis::DidNotConverge {
            iterations: 0,
            residual: f64::NAN,
        })
    }

    /// [`Self::advance`] from the state kept: the last sub-step reached
    /// (the solution there is [`Self::last`] when `pace.analyse`, or when
    /// it stopped short), and whether the drives got all the way. The total
    /// iterations are in the corrected state's.
    fn carry(&mut self, drives: &[f64], pace: Pace) -> (Option<Corrected>, Result<(), Diagnosis>) {
        let Some(mut state) = self.state.clone() else {
            return (
                None,
                Err(Diagnosis::DidNotConverge {
                    iterations: 0,
                    residual: f64::NAN,
                }),
            );
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
        let min_step = if span > 0.0 { pace.finest / span } else { 1.0 };
        // The largest sub-step, as a fraction of the whole way.
        let widest = if span > 0.0 && pace.largest.is_finite() {
            (pace.largest / span).min(1.0)
        } else {
            1.0
        };
        let base = self.base_active();
        let mut s = 0.0f64;
        let mut step = widest;
        let mut reached: Option<Corrected> = None;
        let mut failure: Option<Diagnosis> = None;
        let mut substeps = 0usize;
        let mut iterations = 0usize;
        let mut exhausted = false;
        while s < 1.0 {
            substeps += 1;
            if substeps > MAX_SUBSTEPS || iterations > pace.budget {
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
            let tried = self.correct_within(&predicted, &d_try, pace.per_step);
            iterations += match &tried {
                Ok(c) => c.iterations,
                Err(Diagnosis::DidNotConverge { iterations, .. }) => *iterations,
                Err(_) => pace.per_step,
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
                    step = (step * 2.0).min(widest);
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
        if let Some(c) = &mut reached {
            // The analysis of the last state: for the caller, or for the
            // loop a dead point names.
            if (pace.analyse || s < 1.0)
                && let Ok(sol) = self.solution(c)
            {
                self.last = Some(sol);
            }
            self.state = Some(state.clone());
            c.iterations = iterations;
        }
        if s >= 1.0 {
            if pace.analyse && self.last.is_none() {
                return (
                    reached,
                    Err(Diagnosis::DidNotConverge {
                        iterations,
                        residual: f64::NAN,
                    }),
                );
            }
            return (reached, Ok(()));
        }
        if exhausted {
            // Out of work, not at a limit position: the motion asks for more
            // sub-steps than allowed (a step too large for one advance).
            return (
                reached,
                Err(Diagnosis::DidNotConverge {
                    iterations,
                    residual: f64::NAN,
                }),
            );
        }
        let cause = failure.unwrap_or(Diagnosis::DidNotConverge {
            iterations,
            residual: f64::NAN,
        });
        // The sub-step shrank to nothing: whether the last try came to rest
        // or jumped off the branch, the drive cannot go on from here (at a
        // fold the corrector converges, but beyond the motion predicted).
        let loop_mates = self.driver_loop();
        (
            reached,
            Err(Diagnosis::DeadPoint {
                drives: self.drives.clone(),
                loop_mates,
                cause: Box::new(cause),
            }),
        )
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
        self.correct_within(start, drives, MAX_ITERATIONS)
    }

    /// [`Self::correct`] spending at most `cap` iterations.
    fn correct_within(
        &self,
        start: &State,
        drives: &[f64],
        cap: usize,
    ) -> Result<Corrected, Diagnosis> {
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
            let budget = cap.saturating_sub(used);
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
