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
//! it has to be). An unknown `t` goes from 0 to 1 and takes each image
//! that moved with it: through its pattern's own family when the map it
//! had is of the same kind about the same points as the map it has (the
//! offset of a row, the instance number of a chained run, the angle and
//! fixed centre of a turn go from their old values to their new ones, so
//! an image of a ring swings round its centre instead of cutting across
//! it), otherwise in a straight line from its old place. Every row is
//! `F(x) - (1 - t) r0` with `r0` the rows at the starting guess, so that a
//! dimension changed in the same edit moves in the same measure as the
//! images and the starting guess is a solution at `t = 0`; at `t = 1` the
//! system is the sketch's own. The path is followed in steps: each step
//! solves at the next `t` by Newton iterations on the system with `t` as
//! one more unknown and the row `t - next` first, so that the first
//! iteration is the Euler predictor (the tangent of the path, with the
//! Jacobian where the sketch is, on the right side of every image) and the
//! following ones the corrector. A step is kept when the corrections
//! contract (each at most half the one before, the region where Newton's
//! method converges); otherwise it is halved. The result is only a
//! starting guess: [`crate::solve`] solves from it as from any other, with
//! its own diagnosis, so the verdicts and guarantees are those of the
//! solve. How far the path was followed is reported ([`Carried`]).

use crate::equations::{CROWDED, Carry, Fail, Stage, prepare};
use crate::factor::{Factor, Jacobian, Work};
use crate::model::{Carried, Entity, EntityId, ImageMap, Sketch, SolveOptions};
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

/// What [`carry`] gives the solve: the starting guess where the path
/// ended (`None`: start from the stored coordinates) and what it did
/// (`None`: no image had moved), with the images that had moved.
pub(crate) struct Carrying {
    pub entities: Option<Vec<Entity>>,
    pub outcome: Option<Carried>,
    pub moved: Vec<EntityId>,
}

const NOTHING: Carrying = Carrying {
    entities: None,
    outcome: None,
    moved: Vec::new(),
};

/// How one moved image is to be carried, before the unknowns are numbered.
enum Way {
    Straight([f64; 2]),
    Through(ImageMap),
}

/// The two maps are of one family: the same kind about the same points,
/// so that their numbers can go from one to the other.
fn same_family(old: &ImageMap, new: &ImageMap) -> bool {
    match (old, new) {
        (ImageMap::Shift { .. }, ImageMap::Shift { .. }) => true,
        (
            ImageMap::Chain { from, to, .. },
            ImageMap::Chain {
                from: from2,
                to: to2,
                ..
            },
        ) => from == from2 && to == to2,
        (
            ImageMap::Turn { center, .. },
            ImageMap::Turn {
                center: center2, ..
            },
        ) => center == center2,
        _ => false,
    }
}

fn finite(p: [f64; 2]) -> bool {
    p[0].is_finite() && p[1].is_finite()
}

/// Carries the sketch along from where `sketch.image_starts` says the
/// images were to where their maps put them now, spending from `work` (a
/// share of the solve's budget, `crate::solver::solve`).
pub(crate) fn carry(sketch: &Sketch, options: &SolveOptions, work: &mut Work) -> Carrying {
    if sketch.image_starts.is_empty() {
        return NOTHING;
    }
    let linear = options.tolerance.linear;
    let count = sketch.entities.len();

    // The first start of each image.
    let mut start_of: Vec<Option<&crate::model::ImageStart>> = vec![None; count];
    for start in &sketch.image_starts {
        if let Some(slot) = start_of.get_mut(start.image.index())
            && slot.is_none()
        {
            *slot = Some(start);
        }
    }

    // Which images moved, from the sketch as given: no unknowns are
    // numbered for a sketch whose images are where they were. In the order
    // of the entities, so that an image of an image (nested patterns)
    // comes after the one it is made from: `now` is where each point is
    // put now, `then` where it is when the path starts (a moved image at
    // its stored place, everything else where it is now). An outer image
    // whose own map did not change still moves when its inner one did.
    let mut now: Vec<Option<[f64; 2]>> = vec![None; count];
    let mut then: Vec<Option<[f64; 2]>> = vec![None; count];
    let mut ways: Vec<Option<Way>> = Vec::new();
    let mut moved = Vec::new();
    // The stage of each moved image: one after the latest stage of the
    // moved images it is made from (see `Stage`); 0 for the rest.
    let mut stage_of: Vec<usize> = vec![0; count];
    let mut stages: Vec<usize> = Vec::new();
    for (index, entity) in sketch.entities.iter().enumerate() {
        let (Entity::Point { x, y }, Some(n), Some(t)) =
            (entity, now.get_mut(index), then.get_mut(index))
        else {
            let Entity::Image { of, map } = entity else {
                continue;
            };
            // Only what comes before it: an image is made from those.
            let earlier = |places: &[Option<[f64; 2]>], id: EntityId| {
                places
                    .get(..index)
                    .and_then(|p| p.get(id.index()).copied().flatten())
            };
            let here = earlier(&now, *of).and_then(|p| map.apply(p, |id| earlier(&now, id)));
            let mut way = None;
            let mut start_place = None;
            if let (Some(start), Some(here)) = (start_of.get(index).copied().flatten(), here) {
                let from = earlier(&then, *of);
                let straight = || {
                    // At the start of the path the map is applied to where
                    // the points it uses are then.
                    let base = from.and_then(|p| map.apply(p, |id| earlier(&then, id)))?;
                    Some((
                        start.at,
                        Way::Straight([start.at[0] - base[0], start.at[1] - base[1]]),
                    ))
                };
                let chosen = match start.map {
                    // Through the family only from where the image was: the
                    // old numbers on the points as they are when the path
                    // starts put it at its old place. When they do not (the
                    // edit moved the ends of the run or the centre), the
                    // family would start the image where it never was.
                    Some(old) if same_family(&old, map) => {
                        match from.and_then(|p| old.apply(p, |id| earlier(&then, id))) {
                            Some(was)
                                if finite(was)
                                    && (was[0] - start.at[0])
                                        .abs()
                                        .max((was[1] - start.at[1]).abs())
                                        <= linear =>
                            {
                                Some((was, Way::Through(old)))
                            }
                            _ => straight(),
                        }
                    }
                    _ => straight(),
                };
                // Within the tolerance it is the same place: nothing to
                // carry.
                if let Some((was, chosen)) = chosen
                    && finite(was)
                    && finite(here)
                    && (was[0] - here[0]).abs().max((was[1] - here[1]).abs()) > linear
                {
                    way = Some(chosen);
                    start_place = Some(start.at);
                    moved.push(start.image);
                }
            }
            let mut sources = vec![*of];
            match map {
                ImageMap::Chain { from, to, .. } => sources.extend([*from, *to]),
                ImageMap::Turn {
                    center: Some(c), ..
                } => sources.push(*c),
                _ => {}
            }
            let stage = if way.is_some() {
                1 + sources
                    .iter()
                    .filter(|s| s.index() < index)
                    .filter_map(|s| stage_of.get(s.index()).copied())
                    .max()
                    .unwrap_or(0)
            } else {
                0
            };
            if let Some(slot) = stage_of.get_mut(index) {
                *slot = stage;
            }
            stages.push(stage);
            if let Some(slot) = now.get_mut(index) {
                *slot = here;
            }
            if let Some(slot) = then.get_mut(index) {
                *slot = start_place.or(here);
            }
            ways.push(way);
            continue;
        };
        *n = Some([*x, *y]);
        *t = Some([*x, *y]);
    }
    if moved.is_empty() {
        return NOTHING;
    }
    moved.sort_unstable();

    let skipped = |reason: &str| Carrying {
        entities: None,
        outcome: Some(Carried::Skipped {
            reason: reason.to_string(),
            moved: moved.clone(),
        }),
        moved: moved.clone(),
    };
    let Ok(mut prep) = prepare(sketch, linear) else {
        // The solve reports what is wrong with the sketch.
        return NOTHING;
    };
    let plain = prep.images.clone();
    let t = prep.x0.len();
    for ((image, way), entity) in prep.images.iter_mut().zip(&ways).zip(
        sketch
            .entities
            .iter()
            .filter(|e| matches!(e, Entity::Image { .. })),
    ) {
        let Entity::Image { map: new, .. } = entity else {
            continue;
        };
        image.carry = match (way, new) {
            (None, _) => None,
            (Some(Way::Straight(by)), _) => Some(Carry::Straight { t, by: *by }),
            (Some(Way::Through(ImageMap::Shift { offset })), _) => {
                Some(Carry::Shift { t, from: *offset })
            }
            (Some(Way::Through(ImageMap::Chain { k, .. })), _) => {
                Some(Carry::Chain { t, from: *k })
            }
            (
                Some(Way::Through(ImageMap::Turn { at, angle, .. })),
                ImageMap::Turn { angle: to, .. },
            ) => Some(Carry::Turn {
                t,
                from: *angle,
                to: *to,
                at: *at,
            }),
            (Some(Way::Through(_)), _) => None,
        };
    }
    prep.x0.push(0.0);
    // One stage per level of moved images made from moved images.
    let levels = stages.iter().copied().max().unwrap_or(1).max(1);
    #[allow(clippy::cast_precision_loss)]
    let level = |s: usize| s as f64 / levels as f64;
    if levels > 1 {
        for (image, stage) in prep.images.iter_mut().zip(&stages) {
            let s = (*stage).max(1);
            image.stage = Stage {
                lo: level(s - 1),
                hi: level(s),
            };
        }
    }
    let breaks: Vec<f64> = (1..=levels).map(level).collect();

    // As a solve: an equation without a value at the start (the direction
    // of a line without length) sits out. One that the path parameter
    // makes depend on more unknowns than an equation holds cannot be
    // carried; nothing is, rather than carrying without it.
    let mut active = Vec::with_capacity(prep.equations.len());
    for eq in &prep.equations {
        match eq.eval_with(&prep.x0, linear, &prep.splines, &prep.images) {
            Ok(_) => active.push(true),
            Err(Fail::Degenerate(why)) if why == CROWDED => {
                // Crowded without the path parameter too: the solve says so.
                if matches!(
                    eq.eval_with(&prep.x0, linear, &prep.splines, &plain),
                    Err(Fail::Degenerate(w)) if w == CROWDED
                ) {
                    active.push(false);
                } else {
                    return skipped(
                        "an equation on a moved image point would depend on more unknowns than one equation holds once the path is added",
                    );
                }
            }
            Err(Fail::Degenerate(_)) => active.push(false),
            Err(_) => return skipped("an equation has no value at the stored coordinates"),
        }
    }
    let mut system = System::new(&prep, options);
    let Ok(r0) = system.evaluate(&prep.x0, &active, None) else {
        return skipped("an equation has no value at the stored coordinates");
    };
    system.homotopy = Some(Homotopy { t, r0 });

    let mut x = prep.x0.clone();
    let mut at = 0.0;
    let mut h = 1.0;
    let mut out_of_work = false;
    for _ in 0..MAX_STEPS {
        if at >= 1.0 {
            break;
        }
        let to = if at + h >= 1.0 { 1.0 } else { at + h };
        // Never across the end of a stage: the path bends there.
        let to = breaks
            .iter()
            .copied()
            .find(|b| *b > at)
            .map_or(to, |b| to.min(b));
        match step(&system, options, &x, t, to, &active, prep.magnitude, work) {
            Step::Kept(mut next) => {
                if let Some(v) = next.get_mut(t) {
                    *v = to;
                }
                x = next;
                at = to;
                h = (2.0 * h).min(1.0);
            }
            Step::Halve => {
                h *= 0.5;
                if h < MIN_STEP {
                    break;
                }
            }
            // The budget is the solve's: it reports running out.
            Step::OutOfWork => {
                out_of_work = true;
                break;
            }
        }
    }
    let outcome = if at >= 1.0 {
        Carried::Whole
    } else {
        Carried::Stopped {
            reached: at,
            moved: moved.clone(),
            out_of_work,
        }
    };
    if at <= 0.0 {
        return Carrying {
            entities: None,
            outcome: Some(outcome),
            moved,
        };
    }

    let mut entities = sketch.entities.clone();
    for (entity, var) in entities.iter_mut().zip(&prep.entity_var) {
        let Some(var) = *var else { continue };
        match entity {
            Entity::Point { x: px, y: py } => {
                let (Some(sx), Some(sy)) = (x.get(var), x.get(var + 1)) else {
                    return skipped("the path left the sketch's unknowns");
                };
                *px = *sx;
                *py = *sy;
            }
            Entity::Circle { radius, .. } => match x.get(var) {
                Some(r) if r.is_finite() && *r >= 0.0 => *radius = *r,
                _ => return skipped("the path took a radius below zero"),
            },
            Entity::Line { .. } | Entity::Arc { .. } | Entity::Image { .. } => {}
        }
    }
    Carrying {
        entities: Some(entities),
        outcome: Some(outcome),
        moved,
    }
}

enum Step {
    Kept(Vec<f64>),
    Halve,
    OutOfWork,
}

/// One step along the path from `x` (a solution at `x[t]`) to `t = to`:
/// the solution there, or a shorter step when the Newton iteration does
/// not contract to it.
#[allow(clippy::too_many_arguments)]
fn step(
    system: &System,
    options: &SolveOptions,
    x: &[f64],
    t: usize,
    to: f64,
    active: &[bool],
    magnitude: f64,
    work: &mut Work,
) -> Step {
    // The path parameter is a number, the other unknowns lengths: its
    // column is taken in units of the sketch's size, so that the factor
    // weighs it against them as it weighs one length against another (at
    // 1e6 an image moves by millions per unit of `t`, and the rows of the
    // geometry that follows it would otherwise look dependent).
    let size_of_t = if magnitude.is_finite() && magnitude > 0.0 {
        1.0 / magnitude
    } else {
        1.0
    };
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
        let Ok(r) = system.evaluate(&y, active, Some(&mut jac)) else {
            return Step::Halve;
        };
        jac.scale_column(t, size_of_t);
        let Some(pin) = y.get(t).map(|v| v - to) else {
            return Step::Halve;
        };
        let factor = match Factor::new(&jac, &skip, &negligible, options.rank_threshold, work) {
            Ok(factor) => factor,
            Err(crate::SolveError::WorkLimitExceeded { .. }) => return Step::OutOfWork,
            Err(_) => return Step::Halve,
        };
        if iteration > 0 {
            // Every row, the dependent ones too: a path is followed only
            // while the sketch has a solution along it (a redundant row
            // that is consistent holds with the others; one that is not is
            // a conflict, and the path stops there).
            let holds = r.iter().enumerate().all(|(row, v)| {
                skip.get(row + 1).copied().unwrap_or(true)
                    || v.abs() <= system.tolerance.get(row).copied().unwrap_or(0.0)
            });
            if holds {
                return Step::Kept(y);
            }
        }
        if iteration == MAX_ITERATIONS {
            return Step::Halve;
        }
        let rhs: Vec<f64> = std::iter::once(-pin).chain(r.iter().map(|v| -v)).collect();
        let mut dx = factor.min_norm(&rhs);
        if let Some(dt) = dx.get_mut(t) {
            *dt *= size_of_t;
        }
        // The size of a move of the sketch, `t` left out (it is no length).
        let size = dx
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != t)
            .fold(0.0f64, |s, (_, v)| s.hypot(*v));
        if !size.is_finite() {
            return Step::Halve;
        }
        if let Some(before) = last
            && size > CONTRACTION * before
            && size > linear
        {
            return Step::Halve;
        }
        last = Some(size);
        for (v, d) in y.iter_mut().zip(&dx) {
            *v += d;
        }
    }
    Step::Halve
}
