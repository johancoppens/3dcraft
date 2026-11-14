//! The solver: one solve, and a sequence of driven values solved by
//! continuation on one branch.

use crate::analysis::{
    ZERO_COMPONENT, dependencies, diagnose_rest, driver_loop_mates, free_motions, moved_mates,
    rank_and_nulls, redundancies,
};
use crate::ldl::Budget;
use crate::newton::{
    FACTOR_BUDGET, MAX_ITERATIONS, MU_CARRY, MU_START, evaluate, gauss_newton, run,
};
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
/// the scale). A branch can turn sharply where nothing is near singular:
/// an offset slider-crank (crank 1, rod 1.52, offset 0.5) turns at a crank
/// of 270° with its circuits 0.1 of the scale apart, and a sub-step of half
/// a radian from 254° to 277° landed on the other circuit, the corrector
/// converging fast and the prediction met (the third review). A tenth of a
/// radian resolves such turns at the review's slider-cranks and at
/// four-bars of coupler 3.0002 (circuits 0.044 apart at a crank of 180°);
/// at coupler 3.00002 (circuits 1.4·10⁻³ of the scale apart) the carry
/// still lands on the other circuit now and then with the corrector's light
/// damping ([`crate::newton::MU_CARRY`]: 8 jumps on the fourth review's
/// probe, none with a solve's), and at 3.0000002 often. Every such
/// difference from the direct solve carries a note ([`Solver::rest_placed`]),
/// which is what holds there. The rigid secant predicts a part turning
/// about a fixed axis exactly, so a lead screw winds 40 turns in these
/// steps with no Newton step but the last.
const REST_STEP: f64 = 0.1;

/// Newton iterations a rest solve's carry may spend, all sub-steps
/// together: ten solves' worth, so a placement costs at most a few times a
/// solve more however far its drives are from where its parts are placed.
/// A carry that runs out is not used, and the direct solve stands.
const REST_ITERATIONS: usize = 10 * MAX_ITERATIONS;

/// Newton iterations one sub-step of a rest carry may spend: a quarter of
/// a solve's. A sub-step's corrector starts from a prediction at most a
/// tenth of a radian away and converges in a few iterations on the branch (none
/// over the 40 turns of a lead screw, which the rigid secant predicts); one
/// that does not is cut, so a limit position is found by halving (to ε of
/// the drives' change, as a motion's) within the carry's budget: about 1000
/// iterations for the double-rocker of the tests, where a full solve's 200
/// per try spent 7000. Cutting only to √ε was tried and refused: a Jansen leg carried
/// from 250° to 80° passes a near-singular pose at 193° only in steps
/// finer than that.
const REST_STEP_ITERATIONS: usize = MAX_ITERATIONS / 4;

/// A rest carry's first sub-step, as a fraction of [`REST_STEP`]: the
/// carry knows nothing yet of how near a singular pose its start is (see
/// [`REST_REACH`]), so it starts short and doubles from there. A four-bar
/// near its change point placed ten degrees before the pose where its
/// circuits pass closest crossed to the other circuit in a first
/// half-radian sub-step.
const REST_FIRST: f64 = 1.0 / 16.0;

/// How far a rest carry's sub-step may move, as a fraction of `1/ω`: `ω`
/// is how fast the corrector's Newton steps shrink (twice its second step
/// over the square of its first, the constant of its quadratic
/// convergence), and `1/ω` estimates how far the solution is from a
/// singular pose, where another branch may come near. A sub-step moves at
/// most a quarter of that from the last solution, and one that moved more
/// than `1/ω` at the solution it reached is cut: so a carry nears a pose
/// where two branches pass close (a four-bar near its change point, whose
/// circuits come within a fraction of a degree) in steps that shrink with
/// the distance, and follows its own branch round the sharp turn there,
/// where a longer step lands on the other branch's straight continuation
/// with nothing amiss at either end (the corrector converging fast, the
/// prediction met). Far from a singular pose `ω` is small and the
/// sub-steps stay [`REST_STEP`] long.
const REST_REACH: f64 = 0.25;

/// Sub-steps a rest carry may take beyond twice those its span needs at
/// full length: the turns near singular poses. The published walker
/// carried to a crank start of 180° takes 54 in all, at ±120° 37, at ±60°
/// 20, at either end of its leg scale's range the same; the radial engine
/// carried half a turn and more 98 (with an allowance of 574 for its
/// span). A carry that needs more is out of work: the direct solve is
/// shown, and the note says the branch is not checked. One that cannot
/// fit what is left of the way in what is left of the allowance, even
/// doubling every sub-step, stops at once ([`reachable`]).
const REST_SUBSTEPS: usize = 384;

/// The note of a placement whose parts do not meet their mates at the
/// driven values they are placed at, where the carried and the direct
/// solve disagree.
const UNCERTAIN: &str = "its parts as placed do not meet their mates at the drive values they are placed at, so the branch they were put together in is not certain: carried from there to its drives it rests elsewhere than one solve straight from the placement puts it, and the carried pose is shown";

/// The note of a placement whose parts meet their mates at the driven
/// values they are placed at, where the carried and the direct solve
/// disagree: the carried pose is shown. It says that the two disagree and
/// which is shown, no more: near a change point the carry itself can land
/// on the other branch.
const CARRIED: &str = "carried from the placement, the assembly rests elsewhere than a solve straight from the placement gives; the carried pose is shown";

/// The note of a placement whose parts meet their mates at the driven
/// values they are placed at, solved by the carry alone (one solve straight
/// from it failed).
const FAILED_CARRIED: &str =
    "the solve straight from the placement failed; the pose carried from the placement is shown";

/// The note of a carry that ran out of its allowance of sub-steps: the
/// direct solve is shown.
const NOT_WITHIN: &str = "solved straight from the placement, not carried within its allowance: the branch is not checked";

/// The note of such a placement solved by the carry alone (one solve
/// straight from it failed).
const UNCERTAIN_ONLY: &str = "its parts as placed do not meet their mates at the drive values they are placed at, so the branch they were put together in is not certain: it is solved there and carried to its drives";

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
/// (dimensionless), the first by `first` of that, its corrector spends at
/// most `per_step` iterations, the whole carry at most `budget`, a
/// sub-step is not cut below `finest` of the drives' change, and with
/// `analyse` the state reached is analysed (rank, free motions) as
/// [`Solver::last`]. A rest carry (`rest`) also keeps its sub-steps within
/// [`REST_REACH`] of a singular pose, keeps the length of a sub-step that
/// succeeds after a cut one instead of doubling straight back, and moves
/// each part by its rigid motion in the secant ([`System::extrapolate`]).
#[derive(Clone, Copy)]
struct Pace {
    largest: f64,
    first: f64,
    budget: usize,
    per_step: usize,
    finest: f64,
    analyse: bool,
    rest: bool,
}

impl Pace {
    /// A motion's ([`Solver::advance`]): the first sub-step the whole way.
    const MOTION: Pace = Pace {
        largest: f64::INFINITY,
        first: 1.0,
        budget: MAX_ADVANCE_ITERATIONS,
        per_step: MAX_ITERATIONS,
        finest: 0.0,
        analyse: true,
        rest: false,
    };
    /// A rest solve's carry, and the carry that keeps its free joints.
    const REST: Pace = Pace {
        largest: REST_STEP,
        first: REST_FIRST,
        budget: REST_ITERATIONS,
        per_step: REST_STEP_ITERATIONS,
        finest: 0.0,
        analyse: false,
        rest: true,
    };

    fn with_finest(self, finest: f64) -> Pace {
        Pace { finest, ..self }
    }
}

/// Why a rest solve was not carried from where its parts are placed.
enum NotCarried {
    /// Stopped at these drive values, from a placement that meets its mates
    /// at its own drive values: a limit position, or a turn of the branch
    /// the sub-steps could not follow; which one is not proven, and the
    /// note says only that the branch is not checked past them.
    Reach(Vec<f64>),
    /// Out of its allowance of sub-steps or iterations: the branch is not
    /// checked, which the note says.
    Exhausted,
    /// Anything else (a placement that cannot be solved at its own drive
    /// values, a carry stopped from one that does not meet its mates):
    /// nothing is known of the branch, and nothing is said.
    Other,
}

/// A solved state and how it was reached.
struct Corrected {
    state: State,
    iterations: usize,
    active: Vec<bool>,
    drives: Vec<f64>,
    /// The lengths of the corrector's first two steps (0 for one not
    /// taken), dimensionless.
    steps: [f64; 2],
}

/// Whether a carry with `left` of the way still to go (a fraction), whose
/// next sub-step is `step` long and none longer than `widest`, can get
/// there in `budget` sub-steps. An accepted sub-step at most doubles the
/// next one, so the fewest it can take are those doubling from `step` up to
/// `widest` and then `widest` each. When even those do not fit, the carry
/// would run out of its allowance whatever it meets, and it stops at once:
/// the same outcome, without the work.
fn reachable(left: f64, step: f64, widest: f64, budget: usize) -> bool {
    let mut covered = 0.0f64;
    let mut next = step;
    let mut used = 0usize;
    while next < widest && covered < left && used < budget {
        covered += next;
        next = (next * 2.0).min(widest);
        used += 1;
    }
    if covered >= left {
        return true;
    }
    let more = ((left - covered) / widest).ceil();
    widest > 0.0 && more.is_finite() && used as f64 + more <= budget as f64
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
    /// from `near` as `turns` says. A moment of a motion and a joint held
    /// by hand start from a solved state, which holds its own drives: one
    /// solve straight from the start, whose answer stands whenever it has
    /// one, and when it fails a moment's coupled angles at the turns they
    /// had the moment before. A placement is solved as
    /// [`Self::rest_placed`] says.
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
        if turns == Turns::Measured {
            return self.rest_placed(&start, drives);
        }
        let direct = match self.correct(&start, drives) {
            Ok(c) => return self.settle(c, drives, None, None),
            Err(direct) => direct,
        };
        // A moment whose coupled shafts had come many turns: started at the
        // turns they had in the moment before.
        let Some(near) = near.filter(|_| turns == Turns::Coupled) else {
            return Err(direct);
        };
        let mut carried = start.clone();
        self.sys.keep_coupled_turns(&mut carried, near);
        if carried == start {
            return Err(direct);
        }
        let c = self.correct(&carried, drives).map_err(|_| direct)?;
        self.settle(c, drives, None, None)
    }

    /// A placement solved at `drives`. Where its parts hold other driven
    /// values than `drives` (a mechanism put together at one crank angle
    /// and solved at another), it is solved at the values it holds and
    /// carried from there to `drives` along its branch, besides the one
    /// solve straight from the start: that solve may land on another branch
    /// (a mirrored loop, A-016). When both find one solution (within √ε,
    /// up to the motions the mates leave free), the direct one is kept, bit
    /// for bit as before. When they differ and the parts as placed meet
    /// their mates at the values they hold, the branch they were put
    /// together in is known and the carried pose is the answer; when they
    /// do not meet them, which branch the closing solve lands on is not
    /// known: the carried pose is the answer (on the review's legs and
    /// four-bars it is the one put together where the direct one is not)
    /// and the note says the branch is not certain. A start that meets its
    /// mates but cannot be carried past a limit position leaves the direct
    /// solve with a note; any other failure of the carry (a start that
    /// cannot be closed, a carry out of work) leaves the direct solve as
    /// before. Where the
    /// direct solve fails, the turns the start's couplings say are tried
    /// first, as before (A-030), then the carry. Free joints are then kept
    /// at their start values ([`Self::keep_free`], A-036).
    fn rest_placed(&mut self, start: &State, drives: &[f64]) -> Result<Solution, Diagnosis> {
        let direct = self.correct(start, drives);
        let mut counted = start.clone();
        self.sys.coupled_start_turns(&mut counted);
        let held = self.sys.start_drives(&counted, true);
        // Carried only from driven values the drives may take: a hinge
        // placed outside its limits, or a drive out of reach, is solved
        // directly, as before, and says nothing.
        let apart = self.sys.drives_apart(&held, drives) && self.check_drives(&held).is_ok();
        let mut note = None;
        // The direct solution's own analysis, when it is kept and measured.
        let mut kept_direct: Option<Solution> = None;
        // The solution, and the start its free joints are kept at.
        let (c, from) = match direct {
            Ok(d) if !apart => (d, start.clone()),
            Ok(d) => match self.carry_placed(&held, drives, start) {
                Ok((c, holds)) => {
                    // Both on one solution, up to the motions the mates
                    // leave free there (which a gauge of the steps, not the
                    // branch, decides): the direct one, bit for bit as
                    // before.
                    let sol = self.solution(&d)?;
                    if self.sys.distance_fixed(&d.state, &c.state, &sol.free) <= self.sys.eps.sqrt()
                    {
                        kept_direct = Some(sol);
                        (d, start.clone())
                    } else {
                        // Two methods disagree: say which answer is shown.
                        note = Some(if holds { CARRIED } else { UNCERTAIN }.to_string());
                        (c, counted)
                    }
                }
                Err(NotCarried::Reach(at)) => {
                    let at = at
                        .iter()
                        .map(|d| format!("{d:.6}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    note = Some(format!(
                        "solved straight from the placement, not carried past the drive values [{at}] from where its parts are placed: the branch is not checked"
                    ));
                    (d, start.clone())
                }
                Err(NotCarried::Exhausted) => {
                    note = Some(NOT_WITHIN.to_string());
                    (d, start.clone())
                }
                Err(NotCarried::Other) => (d, start.clone()),
            },
            Err(direct) => {
                // The turns the start's couplings say (A-030), else the
                // carry. A coupled drive placed whole turns from the turns
                // nearest its drive is carried round with its nut, which
                // has no branch to doubt; any other carry says the direct
                // solve failed, and from a start that does not meet its
                // mates that its branch is not certain.
                if counted != *start
                    && let Ok(c) = self.correct(&counted, drives)
                {
                    (c, counted)
                } else if apart {
                    let nearest = self.sys.start_drives(&counted, false);
                    let winds = self.sys.drives_apart(&held, &nearest);
                    match self.carry_placed(&held, drives, start) {
                        Ok((c, holds)) => {
                            if !winds {
                                note = Some(
                                    if holds {
                                        FAILED_CARRIED
                                    } else {
                                        UNCERTAIN_ONLY
                                    }
                                    .to_string(),
                                );
                            }
                            (c, counted)
                        }
                        Err(_) => return Err(direct),
                    }
                } else {
                    return Err(direct);
                }
            }
        };
        let mut sol = match kept_direct {
            Some(sol) => sol,
            None => self.solution(&c)?,
        };
        let mut c = c;
        if let Some((kept, kept_note)) = self.keep_free(&c, &sol, &from) {
            if let Some(k) = kept {
                sol = self.solution(&k)?;
                c = k;
            }
            note = match (note, kept_note) {
                (Some(a), Some(b)) => Some(format!("{a}; {b}")),
                (a, b) => a.or(b),
            };
        }
        self.settle(c, drives, Some(sol), note)
    }

    /// The placement solved at the driven values `held` it holds (its
    /// angles at the turns its couplings say) and carried to `drives`
    /// ([`Self::carry_rest`]), with whether it met its mates at `held`.
    fn carry_placed(
        &mut self,
        held: &[f64],
        drives: &[f64],
        start: &State,
    ) -> Result<(Corrected, bool), NotCarried> {
        let mut from = self.sys.start_state(held);
        self.sys.coupled_start_turns(&mut from);
        let (mut c, holds) = self.carry_rest(&from, held, drives)?;
        // A coupled drive carried whole turns turns the joints on it as
        // many; a solve from the placement does not count them where no
        // equation does.
        if self.sys.winds(held, drives) {
            self.sys.turns_nearest_start(&mut c.state, start);
        }
        Ok((c, holds))
    }

    /// The solution at `c` (`sol` when it is measured already), kept as
    /// the solver's state at `drives`, with `note`.
    fn settle(
        &mut self,
        c: Corrected,
        drives: &[f64],
        sol: Option<Solution>,
        note: Option<String>,
    ) -> Result<Solution, Diagnosis> {
        let mut sol = match sol {
            Some(sol) => sol,
            None => self.solution(&c)?,
        };
        sol.note = note;
        self.state = Some(c.state);
        self.prev = None;
        self.drives = drives.to_vec();
        self.last = Some(sol.clone());
        Ok(sol)
    }

    /// Keeps the joints that nothing fixes where the start has them: a free
    /// revolute's angle, a free slider's distance. The solve's steps are
    /// least-squares steps in all unknowns alike, so a misfit at the start
    /// (a part placed turned against its mate) is closed partly by turning
    /// a free joint too, and the free joint ends where nothing asked it to
    /// be. A joint is kept when it alone carries a free motion: the free
    /// motions of `sol` move it in a way no other joint they move does (its
    /// row of rates is not in the span of the others'), as a free swivel
    /// does. Joints that move only together (a free loop, a screw and its
    /// nut) are left as the solve leaves them: which of them to keep is a
    /// choice the document does not make. When a joint kept is no longer at
    /// its start value, the joints kept are carried back to their start
    /// values with the solution, along its branch, as drives of the moment.
    /// The choice does not depend on the order of the mates or instances,
    /// nor on the assembly's size beyond what the null vectors resolve (a
    /// rate below 10⁻⁶ of a free motion's fastest counts as none). `None`
    /// when there is nothing to keep; else the state moved back (`None`
    /// when it could not be, with the reason).
    fn keep_free(
        &self,
        c: &Corrected,
        sol: &Solution,
        start: &State,
    ) -> Option<(Option<Corrected>, Option<String>)> {
        let sys = &self.sys;
        let k = sol.free.len();
        if k == 0 {
            return None;
        }
        let fixed: std::collections::BTreeSet<(usize, usize)> =
            sys.slots.iter().chain(&sol.at_limit).copied().collect();
        // The rate of each joint variable in each free motion (rates below
        // ZERO_COMPONENT of a motion's largest are left out already).
        let mut rows: std::collections::BTreeMap<(usize, usize), Vec<f64>> =
            std::collections::BTreeMap::new();
        for (j, f) in sol.free.iter().enumerate() {
            for &(m, v, r) in &f.variables {
                if fixed.contains(&(m, v)) {
                    continue;
                }
                if let Some(x) = rows
                    .entry((m, v))
                    .or_insert_with(|| vec![0.0; k])
                    .get_mut(j)
                {
                    *x = r / sys.unit(m, v);
                }
            }
        }
        let rows: Vec<((usize, usize), Vec<f64>)> = rows.into_iter().collect();
        // The rank of a set of rows (Gram-Schmidt; a row within
        // ZERO_COMPONENT of its own length of the span of those before it
        // adds nothing), the arithmetic capped as a factorisation's is.
        let mut budget = Budget::new(FACTOR_BUDGET);
        let mut rank_without = |skip: Option<usize>| -> Option<usize> {
            let mut basis: Vec<Vec<f64>> = Vec::new();
            for (i, (_, row)) in rows.iter().enumerate() {
                if Some(i) == skip {
                    continue;
                }
                budget
                    .spend(basis.len().saturating_add(1).saturating_mul(k))
                    .ok()?;
                let length = row.iter().map(|x| x * x).sum::<f64>().sqrt();
                let mut rest = row.clone();
                for b in &basis {
                    let d: f64 = rest.iter().zip(b).map(|(x, y)| x * y).sum();
                    for (x, y) in rest.iter_mut().zip(b) {
                        *x -= d * y;
                    }
                }
                let left = rest.iter().map(|x| x * x).sum::<f64>().sqrt();
                if left.is_finite() && left > ZERO_COMPONENT * length {
                    for x in &mut rest {
                        *x /= left;
                    }
                    basis.push(rest);
                }
            }
            Some(basis.len())
        };
        let too_many = || {
            Some((
                None,
                Some(format!(
                    "{} free joints are too many to keep at their start values",
                    rows.len()
                )),
            ))
        };
        let Some(all) = rank_without(None) else {
            return too_many();
        };
        let mut held: Vec<(usize, usize)> = Vec::new();
        for (i, (mv, _)) in rows.iter().enumerate() {
            match rank_without(Some(i)) {
                Some(r) if r < all => held.push(*mv),
                Some(_) => {}
                None => return too_many(),
            }
        }
        let value = |s: &State, (m, v): (usize, usize)| {
            s.values
                .get(m)
                .and_then(|vals| vals.get(v))
                .copied()
                .unwrap_or(f64::NAN)
        };
        let moved = held.iter().any(|&mv| {
            let d = (value(&c.state, mv) - value(start, mv)) / sys.unit(mv.0, mv.1);
            d.is_nan() || d.abs() > sys.eps
        });
        if !moved {
            return None;
        }
        let names = held
            .iter()
            .map(|&(m, _)| sys.problem.mate_name(m))
            .collect::<Vec<_>>()
            .join(", ");
        let fail = |why: String| {
            Some((
                None,
                Some(format!(
                    "the free motion of {names} moved from where the parts are placed and could not be carried back ({why})"
                )),
            ))
        };
        let mut problem = sys.problem.clone();
        for &(m, v) in &held {
            if let Some(slot) = problem.mates.get_mut(m).and_then(|s| s.driven.get_mut(v)) {
                *slot = Some(value(start, (m, v)));
            }
        }
        let mut hold = match Solver::new(&problem) {
            Ok(s) => s,
            Err(e) => return fail(e.message(&problem)),
        };
        let kept: std::collections::BTreeSet<(usize, usize)> = held.iter().copied().collect();
        let driven: std::collections::BTreeMap<(usize, usize), f64> = sys
            .slots
            .iter()
            .copied()
            .zip(c.drives.iter().copied())
            .collect();
        let mut from = Vec::with_capacity(hold.sys.slots.len());
        let mut to = Vec::with_capacity(hold.sys.slots.len());
        for &mv in &hold.sys.slots {
            if kept.contains(&mv) {
                from.push(value(&c.state, mv));
                to.push(value(start, mv));
            } else {
                let d = driven.get(&mv).copied().unwrap_or(f64::NAN);
                from.push(d);
                to.push(d);
            }
        }
        if let Err(e) = hold.check_drives(&from).and(hold.check_drives(&to)) {
            return fail(e.message(&problem));
        }
        hold.state = Some(c.state.clone());
        hold.drives = from;
        let (reached, outcome) = hold.carry(&to, Pace::REST.with_finest(hold.sys.eps));
        if let Err(e) = outcome {
            return fail(e.message(&problem));
        }
        let Some(back) = reached else {
            return fail("nothing was solved".to_string());
        };
        // The limits held, as this problem's groups: its mates and
        // couplings, its own drivers (all active), its limits.
        let lead = sys.mates.len() + sys.linears.len();
        let mut active = Vec::with_capacity(sys.groups.len());
        active.extend(back.active.iter().take(lead).copied());
        active.extend(std::iter::repeat_n(true, sys.slots.len()));
        active.extend(
            back.active
                .iter()
                .skip(lead + hold.sys.slots.len())
                .copied(),
        );
        if active.len() != sys.groups.len() {
            return fail("the limits do not match".to_string());
        }
        Some((
            Some(Corrected {
                state: back.state,
                iterations: c.iterations.saturating_add(back.iterations),
                active,
                drives: c.drives.clone(),
                steps: [0.0; 2],
            }),
            None,
        ))
    }

    /// Solves `start` at the driven values `held` and carries the solution
    /// to `drives` along its branch, with whether the start already met its
    /// mates there (the closing solve moved it by at most √ε: it was put
    /// together at those values, so its branch is known).
    fn carry_rest(
        &mut self,
        start: &State,
        held: &[f64],
        drives: &[f64],
    ) -> Result<(Corrected, bool), NotCarried> {
        let c0 = self.correct(start, held).map_err(|_| NotCarried::Other)?;
        let holds = self.sys.distance(&c0.state, start) <= self.sys.eps.sqrt();
        let first = c0.iterations;
        self.state = Some(c0.state);
        self.prev = None;
        self.drives = held.to_vec();
        self.last = None;
        let (reached, outcome) = self.carry(drives, Pace::REST.with_finest(self.sys.eps));
        match outcome {
            Ok(()) => {}
            // A limit position between, met from a placement that holds:
            // the branch it was put together in does not reach the drives.
            Err(Diagnosis::DeadPoint { drives, .. }) if holds => {
                return Err(NotCarried::Reach(drives));
            }
            // Out of its allowance (the only way a rest carry ends with
            // this diagnosis).
            Err(Diagnosis::DidNotConverge { .. }) => return Err(NotCarried::Exhausted),
            Err(_) => return Err(NotCarried::Other),
        }
        let mut c = reached.ok_or(NotCarried::Other)?;
        c.iterations = c.iterations.saturating_add(first);
        Ok((c, holds))
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
        // A rest carry may take the sub-steps its span needs at full length,
        // twice over, and REST_SUBSTEPS more.
        let max_substeps = if pace.rest && widest > 0.0 {
            let full = (1.0 / widest).ceil();
            if full.is_finite() && full < MAX_SUBSTEPS as f64 {
                (2 * full as usize)
                    .saturating_add(REST_SUBSTEPS)
                    .min(MAX_SUBSTEPS)
            } else {
                MAX_SUBSTEPS
            }
        } else {
            MAX_SUBSTEPS
        };
        let base = self.base_active();
        let mut s = 0.0f64;
        let mut step = widest * pace.first;
        let mut reached: Option<Corrected> = None;
        let mut failure: Option<Diagnosis> = None;
        let mut substeps = 0usize;
        let mut iterations = 0usize;
        let mut exhausted = false;
        // The last sub-step tried was cut.
        let mut cut = false;
        while s < 1.0 {
            substeps += 1;
            if substeps > max_substeps
                || iterations > pace.budget
                || (pace.rest && !reachable(1.0 - s, step, widest, max_substeps - substeps + 1))
            {
                exhausted = true;
                break;
            }
            let s_try = (s + step).min(1.0);
            let d_try = at(s_try);
            // Predict: along the secant of the last two solutions when the
            // drives keep their direction, else along the tangent.
            let predicted = self
                .secant(&state, &d_try, pace.rest)
                .or_else(|| gauss_newton(&self.sys, &state, &d_try, &base))
                .unwrap_or_else(|| state.clone());
            let mu = if pace.rest { MU_CARRY } else { MU_START };
            let tried = self.correct_within(&predicted, &d_try, pace.per_step, mu);
            iterations += match &tried {
                Ok(c) => c.iterations,
                Err(Diagnosis::DidNotConverge { iterations, .. }) => *iterations,
                Err(_) => pace.per_step,
            };
            let ok = match &tried {
                Ok(c) => {
                    let motion = self.sys.distance(&state, &predicted);
                    // A rest carry's sub-step that moved further than the
                    // solution it reached is from a singular pose.
                    let near = match (pace.rest, self.omega(c)) {
                        (true, Some(omega)) => self.sys.distance(&state, &c.state) * omega <= 1.0,
                        _ => true,
                    };
                    near && self.sys.distance(&c.state, &predicted) <= CONTINUITY * motion + floor
                }
                Err(_) => false,
            };
            match tried {
                Ok(c) if ok => {
                    // The next sub-step of a rest carry moves at most
                    // REST_REACH of the way to the nearest singular pose.
                    let reach = match (pace.rest, self.omega(&c)) {
                        (true, Some(omega)) => {
                            let moved = self.sys.distance(&state, &c.state);
                            (moved > 0.0).then(|| (s_try - s) * (REST_REACH / omega) / moved)
                        }
                        _ => None,
                    };
                    self.prev = Some((state.clone(), self.drives.clone()));
                    state = c.state.clone();
                    s = s_try;
                    reached = Some(c);
                    self.drives = d_try;
                    if !(pace.rest && cut) {
                        step = (step * 2.0).min(widest);
                    }
                    if let Some(reach) = reach {
                        step = step.min(reach.max(min_step));
                    }
                    cut = false;
                }
                Ok(_) => {
                    failure = Some(Diagnosis::BranchLost { drives: d_try });
                    step *= 0.5;
                    cut = true;
                }
                Err(e) => {
                    failure = Some(e);
                    step *= 0.5;
                    cut = true;
                }
            }
            if s < 1.0 && step < min_step {
                break;
            }
        }
        // A rest carry takes a sub-step whose prediction meets the
        // tolerance as it stands, without a Newton step (the rigid secant
        // predicts a turning part exactly), so over many of them its
        // solution may drift within the tolerance (a lead screw carried 40
        // turns): one Gauss–Newton step at the end brings it to the
        // accuracy a solve ends at.
        if pace.rest
            && s >= 1.0
            && let Some(c) = reached.as_mut()
            && let Some(next) = gauss_newton(&self.sys, &c.state, &self.drives, &c.active)
            && let Ok(polished) = self.correct_within(&next, &self.drives, pace.per_step, MU_CARRY)
            && self.sys.distance(&polished.state, &c.state) <= self.sys.eps.sqrt()
        {
            state = polished.state.clone();
            c.state = polished.state;
            c.active = polished.active;
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

    /// The constant of the corrector's quadratic convergence at `c`, twice
    /// its second step over the square of its first (dimensionless):
    /// `1/ω` estimates how far `c` is from a singular pose ([`REST_REACH`]).
    /// `None` when the corrector took fewer than two steps, or a first one
    /// too short to tell from the tolerance.
    fn omega(&self, c: &Corrected) -> Option<f64> {
        let [first, second] = c.steps;
        let omega = 2.0 * second / (first * first);
        (first > 1e3 * self.sys.eps && second > 0.0 && omega.is_finite()).then_some(omega)
    }

    /// The secant predictor: the last solution moved on by the motion from
    /// the one before, in proportion to the drives, when the new drives go
    /// on in the same direction (within 1e-6 of parallel). With `rigid`,
    /// each part moved on by its rigid motion ([`System::extrapolate`]).
    fn secant(&self, state: &State, d_new: &[f64], rigid: bool) -> Option<State> {
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
        if rigid {
            return self.sys.extrapolate(prev_state, state, ratio);
        }
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
        self.correct_within(start, drives, MAX_ITERATIONS, MU_START)
    }

    /// [`Self::correct`] spending at most `cap` iterations, its damping
    /// starting at `mu`.
    fn correct_within(
        &self,
        start: &State,
        drives: &[f64],
        cap: usize,
        mu: f64,
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
        let mut steps = None;
        for _pass in 0..=limits {
            let budget = cap.saturating_sub(used);
            let Some(r) = run(sys, &state, drives, &active, budget, mu) else {
                return Err(Diagnosis::DidNotConverge {
                    iterations: used,
                    residual: f64::INFINITY,
                });
            };
            used += r.iterations;
            // The first pass's first two steps: how fast it contracted.
            if steps.is_none() {
                steps = Some(r.steps);
            }
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
                    steps: steps.unwrap_or([0.0; 2]),
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
            note: None,
        })
    }
}
