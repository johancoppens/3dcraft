//! Carrying a sketch along when its image points jump.
//!
//! An image point has no unknowns: it is where its pattern's map puts a
//! seed point. When the map changes between two solves (the count of a
//! chained run goes from 3 to 10, a step or an angle changes), the image
//! is somewhere else at once, while the points hanging on it start where
//! they were. Newton's method linearises there, on the far side of the
//! image, and an equation such as `|q - image| = 1` then has its root on
//! the wrong side: the sketch folds back. Which root a solve lands on
//! would depend on the size of the edit rather than on the document.
//!
//! The cure is continuation (a homotopy from where the sketch was to where
//! it has to be). With `d` the jump of each image, the image lies at its
//! map plus `(1 - t) d`, and every row is `F(x) - (1 - t) r0` with `r0`
//! the rows at the starting guess (so that a changed dimension moves in the
//! same measure as the images). At `t = 0` the starting guess is a solution;
//! at `t = 1` the system is the sketch's own. The path is followed in
//! steps: each step solves at the next `t` by Newton iterations on the
//! system with `t` as one more unknown and the row `t - next` first, so
//! that the first iteration is the Euler predictor (the tangent of the path,
//! with the Jacobian where the sketch is, on the right side of every
//! image) and the following ones the corrector. A step is kept when the
//! corrections contract (each at most half the one before, the region where
//! Newton's method converges); otherwise it is halved. The result is only
//! a starting guess: [`crate::solve`] solves from it as from any other, with
//! its own diagnosis, so the verdicts and guarantees are those of the solve.

use crate::equations::{Carry, Fail, prepare};
use crate::factor::{Factor, Jacobian, Work};
use crate::model::{Entity, Sketch, SolveOptions};
use crate::solver::{Homotopy, System};

/// Most steps tried along the path, kept or halved.
const MAX_STEPS: usize = 64;
/// The shortest step along the path: below it the path is taken to turn
/// back or end (a fold or a conflict), and the sketch is carried as far as
/// it got.
const MIN_STEP: f64 = 1.0 / 1024.0;
/// Most Newton iterations of one step, the predictor included.
const MAX_ITERATIONS: usize = 8;
/// A correction must be at most this fraction of the one before it
/// (Newton's method converging, rather than wandering to another root).
const CONTRACTION: f64 = 0.5;

/// The starting guess carried along from where `sketch.image_starts` says
/// the images were to where their maps put them now, or `None` when no
/// image moved by more than the linear tolerance or the path could not be
/// followed one step.
pub(crate) fn carry(sketch: &Sketch, options: &SolveOptions) -> Option<Vec<Entity>> {
    if sketch.image_starts.is_empty() {
        return None;
    }
    let linear = options.tolerance.linear;
    let mut prep = prepare(sketch, linear).ok()?;
    let t = prep.x0.len();

    // The images are numbered in the order of the entities.
    let mut image_index = vec![None; sketch.entities.len()];
    let mut count = 0usize;
    for (slot, entity) in image_index.iter_mut().zip(&sketch.entities) {
        if matches!(entity, Entity::Image { .. }) {
            *slot = Some(count);
            count += 1;
        }
    }
    let mut moved = false;
    for (id, [sx, sy]) in &sketch.image_starts {
        let Some(Some(index)) = image_index.get(id.index()) else {
            continue;
        };
        let Some(image) = prep.images.get_mut(*index) else {
            continue;
        };
        if image.carry.is_some() {
            continue;
        }
        let Some([ax, ay]) = image.at(&prep.x0) else {
            continue;
        };
        let by = [sx - ax, sy - ay];
        if !(by[0].is_finite() && by[1].is_finite()) {
            continue;
        }
        // Within the tolerance it is the same place: nothing to carry.
        if by[0].abs().max(by[1].abs()) <= linear {
            continue;
        }
        image.carry = Some(Carry { t, by });
        moved = true;
    }
    if !moved {
        return None;
    }
    prep.x0.push(0.0);

    // As a solve: an equation without a value at the start (the direction
    // of a line without length) sits out.
    let mut active = Vec::with_capacity(prep.equations.len());
    for eq in &prep.equations {
        match eq.eval_with(&prep.x0, linear, &prep.splines, &prep.images) {
            Ok(_) => active.push(true),
            Err(Fail::Degenerate(_)) => active.push(false),
            Err(_) => return None,
        }
    }
    let mut system = System::new(&prep, options);
    let r0 = system.evaluate(&prep.x0, &active, None).ok()?;
    system.homotopy = Some(Homotopy { t, r0 });

    let mut work = Work::new(options.work_limit);
    let mut x = prep.x0.clone();
    let mut at = 0.0;
    let mut h = 1.0;
    for _ in 0..MAX_STEPS {
        if at >= 1.0 {
            break;
        }
        let to = if at + h >= 1.0 { 1.0 } else { at + h };
        match step(&system, options, &x, t, to, &active, &mut work) {
            Some(mut next) => {
                if let Some(v) = next.get_mut(t) {
                    *v = to;
                }
                x = next;
                at = to;
                h = (2.0 * h).min(1.0);
            }
            None => {
                h *= 0.5;
                if h < MIN_STEP {
                    break;
                }
            }
        }
    }
    if at <= 0.0 {
        return None;
    }

    let mut entities = sketch.entities.clone();
    for (entity, var) in entities.iter_mut().zip(&prep.entity_var) {
        let Some(var) = *var else { continue };
        match entity {
            Entity::Point { x: px, y: py } => {
                let (Some(sx), Some(sy)) = (x.get(var), x.get(var + 1)) else {
                    return None;
                };
                *px = *sx;
                *py = *sy;
            }
            Entity::Circle { radius, .. } => {
                let r = *x.get(var)?;
                if !(r.is_finite() && r >= 0.0) {
                    return None;
                }
                *radius = r;
            }
            Entity::Line { .. } | Entity::Arc { .. } | Entity::Image { .. } => {}
        }
    }
    Some(entities)
}

/// One step along the path from `x` (a solution at `x[t]`) to `t = to`:
/// the solution there, or `None` when the Newton iteration does not
/// contract to it.
fn step(
    system: &System,
    options: &SolveOptions,
    x: &[f64],
    t: usize,
    to: f64,
    active: &[bool],
    work: &mut Work,
) -> Option<Vec<f64>> {
    let skip: Vec<bool> = std::iter::once(false)
        .chain(system.skip_rows(active))
        .collect();
    let negligible: Vec<f64> = std::iter::once(0.0)
        .chain(system.negligible.iter().copied())
        .collect();
    let linear = options.tolerance.linear;
    let mut y = x.to_vec();
    let mut last: Option<f64> = None;
    for iteration in 0..=MAX_ITERATIONS {
        let mut jac = Jacobian::new(y.len());
        // The row `t - to` first: always independent, so `t` lands there.
        jac.push(t, 1.0);
        jac.end_row();
        let r = system.evaluate(&y, active, Some(&mut jac)).ok()?;
        let pin = y.get(t)? - to;
        let factor = Factor::new(&jac, &skip, &negligible, options.rank_threshold, work).ok()?;
        if iteration > 0 {
            let holds = r.iter().enumerate().all(|(row, v)| {
                let row = row + 1;
                skip.get(row).copied().unwrap_or(true)
                    || factor.dependent.get(row).copied().unwrap_or(true)
                    || v.abs() <= system.tolerance.get(row - 1).copied().unwrap_or(0.0)
            });
            if holds {
                return Some(y);
            }
        }
        if iteration == MAX_ITERATIONS {
            return None;
        }
        let rhs: Vec<f64> = std::iter::once(-pin).chain(r.iter().map(|v| -v)).collect();
        let dx = factor.min_norm(&rhs);
        // The size of a move of the sketch, `t` left out (it is no length).
        let size = dx
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != t)
            .fold(0.0f64, |s, (_, v)| s.hypot(*v));
        if !size.is_finite() {
            return None;
        }
        if let Some(before) = last
            && size > CONTRACTION * before
            && size > linear
        {
            return None;
        }
        last = Some(size);
        for (v, d) in y.iter_mut().zip(&dx) {
            *v += d;
        }
    }
    None
}
