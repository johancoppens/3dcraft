//! Damped Newton (Levenberg–Marquardt) on the system's equations, the
//! normal equations assembled block-sparse and factored by `ldl`.

use crate::ldl::{Budget, factor};
use crate::system::{Eval, State, System};

/// Newton iterations allowed in one solve (all limit passes together).
pub(crate) const MAX_ITERATIONS: usize = 200;

/// Damping retries allowed per iteration.
const MAX_RETRIES: usize = 40;

/// The largest step one iteration may take, dimensionless: half the
/// scale, half a radian. A longer Newton step is shortened to it, so a far
/// start walks in instead of leaping across branches.
const MAX_STEP: f64 = 0.5;

/// The damping `μ` starts here (relative to the largest diagonal of
/// `JᵀJ`): nearly a pure Gauss–Newton step.
const MU_START: f64 = 1e-9;
const MU_MIN: f64 = 1e-12;
/// Beyond this the step is a gradient step too short to matter: the
/// iteration is at rest.
const MU_MAX: f64 = 1e10;

/// The arithmetic one factorisation may spend.
pub(crate) const FACTOR_BUDGET: u64 = 2_000_000_000;

/// Every group's residuals and Jacobian at a state; `None` for a group not
/// active.
pub(crate) type Evals = Vec<Option<Eval>>;

pub(crate) fn evaluate(sys: &System, s: &State, drives: &[f64], active: &[bool]) -> Option<Evals> {
    let mut out = Vec::with_capacity(sys.groups.len());
    for gi in 0..sys.groups.len() {
        if active.get(gi).copied().unwrap_or(false) {
            let e = sys.eval(gi, s, drives, 1.0)?;
            if e.r.iter().any(|x| !x.is_finite()) {
                return None;
            }
            out.push(Some(e));
        } else {
            out.push(None);
        }
    }
    Some(out)
}

/// The largest residual, dimensionless.
pub(crate) fn max_residual(evals: &Evals) -> f64 {
    evals
        .iter()
        .flatten()
        .flat_map(|e| e.r.iter())
        .fold(0.0f64, |m, x| m.max(x.abs()))
}

fn cost(evals: &Evals) -> f64 {
    evals
        .iter()
        .flatten()
        .flat_map(|e| e.r.iter())
        .map(|x| x * x)
        .sum()
}

/// `JᵀJ` (in the normal pattern's layout) and `Jᵀr` (block order).
pub(crate) fn normal_equations(sys: &System, evals: &Evals) -> Option<(Vec<f64>, Vec<f64>)> {
    let mut a = sys.normal.zeros();
    let mut g = vec![0.0; sys.unknowns];
    let mut m = Vec::new();
    for e in evals.iter().flatten() {
        let rows = e.r.len();
        for (i, (bi, ji)) in e.parts.iter().enumerate() {
            let si = *sys.block_sizes.get(*bi)?;
            let oi = *sys.block_offsets.get(*bi)?;
            for c in 0..si {
                let mut s = 0.0;
                for (row, r) in e.r.iter().enumerate() {
                    s += ji.get(row * si + c)? * r;
                }
                *g.get_mut(oi + c)? += s;
            }
            for (bj, jj) in e.parts.iter().take(i + 1) {
                let sj = *sys.block_sizes.get(*bj)?;
                m.clear();
                m.resize(si * sj, 0.0);
                for r in 0..si {
                    for c in 0..sj {
                        let mut s = 0.0;
                        for row in 0..rows {
                            s += ji.get(row * si + r)? * jj.get(row * sj + c)?;
                        }
                        *m.get_mut(r * sj + c)? = s;
                    }
                }
                sys.normal.add(&mut a, *bi, *bj, &m)?;
            }
        }
    }
    Some((a, g))
}

/// How a Newton run ended.
pub(crate) struct Run {
    pub(crate) state: State,
    pub(crate) converged: bool,
    /// At rest without meeting the tolerance (a least-squares minimum).
    pub(crate) stalled: bool,
    pub(crate) iterations: usize,
    /// Where the first, undamped step leads: the predictor of a
    /// continuation step.
    pub(crate) first_step: Option<State>,
    pub(crate) residual: f64,
}

/// The step `−(JᵀJ + μ D I)⁺ Jᵀr`, directions of `JᵀJ` at or below the
/// rank threshold left out (they do not change the residuals: free motion
/// stays where it is). `None` when the factorisation failed.
fn step_for(sys: &System, a: &[f64], g: &[f64], mu: f64, scale_d: f64) -> Option<Vec<f64>> {
    let mut vals = a.to_vec();
    let shift = mu * scale_d;
    sys.normal.add_identity(&mut vals, shift)?;
    let mut budget = Budget::new(FACTOR_BUDGET);
    let f = factor(&sys.normal, vals, sys.rank_threshold() + shift, &mut budget).ok()?;
    let x = f.solve(g)?;
    let mut h: Vec<f64> = x.iter().map(|v| -v).collect();
    let big = h.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if !big.is_finite() {
        return None;
    }
    if big > MAX_STEP {
        let k = MAX_STEP / big;
        for v in &mut h {
            *v *= k;
        }
    }
    Some(h)
}

/// Runs damped Newton from `start` at `drives` over the `active` groups,
/// spending at most `budget` iterations.
pub(crate) fn run(
    sys: &System,
    start: &State,
    drives: &[f64],
    active: &[bool],
    budget: usize,
) -> Option<Run> {
    let mut state = start.clone();
    let mut evals = evaluate(sys, &state, drives, active)?;
    let mut res = max_residual(&evals);
    let mut c = cost(&evals);
    let mut mu = MU_START;
    let mut first_step = None;
    let mut slow = 0usize;
    let tol = sys.eps;
    for it in 0..budget {
        if res <= tol {
            return Some(Run {
                state,
                converged: true,
                stalled: false,
                iterations: it,
                first_step,
                residual: res,
            });
        }
        let (a, g) = normal_equations(sys, &evals)?;
        let scale_d = sys.normal.max_diagonal(&a).max(1.0);
        let mut accepted = false;
        for _ in 0..MAX_RETRIES {
            let Some(h) = step_for(sys, &a, &g, mu, scale_d) else {
                mu *= 8.0;
                if mu > MU_MAX {
                    break;
                }
                continue;
            };
            let next = sys.step(&state, &h)?;
            if first_step.is_none() {
                first_step = Some(next.clone());
            }
            let Some(ne) = evaluate(sys, &next, drives, active) else {
                mu *= 8.0;
                if mu > MU_MAX {
                    break;
                }
                continue;
            };
            let nc = cost(&ne);
            if nc < c {
                // A decrease too small to matter, repeatedly: at rest.
                if nc > c * (1.0 - 1e-9) {
                    slow += 1;
                } else {
                    slow = 0;
                }
                state = next;
                evals = ne;
                res = max_residual(&evals);
                c = nc;
                mu = (mu / 4.0).max(MU_MIN);
                accepted = true;
                break;
            }
            mu *= 8.0;
            if mu > MU_MAX {
                break;
            }
        }
        if !accepted || slow >= 4 {
            let converged = res <= tol;
            return Some(Run {
                state,
                converged,
                stalled: !converged,
                iterations: it + 1,
                first_step,
                residual: res,
            });
        }
    }
    let converged = res <= tol;
    Some(Run {
        state,
        converged,
        stalled: false,
        iterations: budget,
        first_step,
        residual: res,
    })
}
