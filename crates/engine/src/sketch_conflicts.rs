//! Which constraints a conflicting one contradicts (R1, the second field
//! test): the solver names the constraint it blames, the later of those
//! that disagree; this names the others.
//!
//! Method: a constraint `j` is one the conflicting constraint `c`
//! contradicts when, with `j` taken out of the sketch, `c` is no longer
//! conflicting. Each candidate is tried on a copy of the document in a
//! session of its own, rolled back to the sketch so that nothing after it
//! is built, with one regeneration cache for all of them (only the sketch
//! is solved again per candidate). The candidates are the constraints that
//! share points with `c`, directly or through other constraints and
//! curves, at most [`MAX_CANDIDATES`] of them (the latest before `c`
//! first), and at most [`MAX_CONFLICTS`] conflicting constraints are
//! explained: a diagnosis with a bounded cost, not a proof.

use std::collections::BTreeSet;

use craft3d_doc::FeatureId;
use craft3d_doc::sketch::{ConstraintId, EntityId, Sketch};
use craft3d_regen::ConstraintState;
use serde_json::{Value, json};

use crate::{OpenDocument, Session, Workspace};

/// Most constraints tried per conflicting constraint.
pub(crate) const MAX_CANDIDATES: usize = 32;
/// Most conflicting constraints explained per result.
pub(crate) const MAX_CONFLICTS: usize = 4;

/// The points (and point-like entities) an entity is made of, itself
/// included.
fn atoms(sketch: &Sketch, entity: EntityId) -> Vec<EntityId> {
    let mut out = vec![entity];
    if let Ok(e) = sketch.entity(entity) {
        out.extend(e.geometry.points());
    }
    out
}

/// The constraints connected to `conflict` through shared points, in the
/// order they are to be tried: the latest before it first, then those
/// after it; at most [`MAX_CANDIDATES`], and whether more were left out.
fn candidates(sketch: &Sketch, conflict: ConstraintId) -> (Vec<ConstraintId>, bool) {
    let Some(at) = sketch.constraints.iter().position(|c| c.id == conflict) else {
        return (Vec::new(), false);
    };
    let constraint_atoms: Vec<BTreeSet<EntityId>> = sketch
        .constraints
        .iter()
        .map(|c| {
            c.kind
                .entities()
                .into_iter()
                .flat_map(|e| atoms(sketch, e))
                .collect()
        })
        .collect();
    let curve_atoms: Vec<BTreeSet<EntityId>> = sketch
        .entities
        .iter()
        .map(|e| atoms(sketch, e.id).into_iter().collect())
        .collect();
    let mut reached: BTreeSet<EntityId> = constraint_atoms.get(at).cloned().unwrap_or_default();
    let mut joined = vec![false; sketch.constraints.len()];
    if let Some(own) = joined.get_mut(at) {
        *own = true;
    }
    // Grows until nothing new is reached: at most one round per
    // constraint and curve.
    let rounds = sketch.constraints.len() + sketch.entities.len() + 1;
    for _ in 0..rounds {
        let before = reached.len();
        for set in &curve_atoms {
            if !set.is_disjoint(&reached) {
                reached.extend(set.iter().copied());
            }
        }
        for (i, set) in constraint_atoms.iter().enumerate() {
            if let Some(j) = joined.get_mut(i)
                && !*j
                && !set.is_disjoint(&reached)
            {
                *j = true;
                reached.extend(set.iter().copied());
            }
        }
        if reached.len() == before {
            break;
        }
    }
    let ids: Vec<(usize, ConstraintId)> = sketch
        .constraints
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != at && joined.get(*i).copied().unwrap_or(false))
        .map(|(i, c)| (i, c.id))
        .collect();
    let chosen = ids
        .iter()
        .filter(|(i, _)| *i < at)
        .rev()
        .chain(ids.iter().filter(|(i, _)| *i > at))
        .take(MAX_CANDIDATES)
        .map(|(_, id)| *id)
        .collect();
    (chosen, ids.len() > MAX_CANDIDATES)
}

/// Whether `conflict` holds points of an arc: then what it contradicts may
/// be the arc's own equation (its ends equally far from its centre), which
/// no constraint stands for.
fn on_an_arc(sketch: &Sketch, conflict: ConstraintId) -> bool {
    let Ok(c) = sketch.constraint(conflict) else {
        return false;
    };
    let mine: BTreeSet<EntityId> = c
        .kind
        .entities()
        .into_iter()
        .flat_map(|e| atoms(sketch, e))
        .collect();
    sketch.entities.iter().any(|e| {
        matches!(e.geometry, craft3d_doc::sketch::Geometry::Arc { .. })
            && atoms(sketch, e.id).iter().any(|a| mine.contains(a))
    })
}

/// Most sketch solves spent explaining the conflicts of one result.
pub(crate) const MAX_PROBES: usize = 64;
/// Most milliseconds spent explaining the conflicts of one result, where
/// there is a clock (not in a browser: there [`MAX_PROBES`] bounds it).
pub(crate) const MAX_MS: f64 = 2000.0;

#[cfg(not(target_arch = "wasm32"))]
struct Clock(std::time::Instant);

#[cfg(not(target_arch = "wasm32"))]
impl Clock {
    fn start() -> Self {
        // native clock: `Clock` is cfg(not(target_arch = "wasm32")).
        Clock(std::time::Instant::now())
    }
    fn past(&self, ms: f64) -> bool {
        self.0.elapsed().as_secs_f64() * 1000.0 > ms
    }
}

#[cfg(target_arch = "wasm32")]
struct Clock;

#[cfg(target_arch = "wasm32")]
impl Clock {
    fn start() -> Self {
        Clock
    }
    fn past(&self, _ms: f64) -> bool {
        false
    }
}

/// The last answer, by document, revision and sketch: asked again for the
/// same state (a command that changed nothing), it is not worked out
/// again.
type Remembered = Option<((u64, u64, FeatureId), Explained)>;
static LAST: std::sync::Mutex<Remembered> = std::sync::Mutex::new(None);

/// What [`contradictions`] found: one entry per conflicting constraint
/// explained, and whether conflicting constraints were left unexplained
/// (more than [`MAX_CONFLICTS`], or the budget ran out first).
#[derive(Clone, Debug, Default)]
pub(crate) struct Explained {
    pub entries: Vec<Value>,
    pub truncated: bool,
}

/// For each conflicting constraint of sketch `id` of `open` (at most
/// [`MAX_CONFLICTS`]), the constraints it contradicts:
/// `{"constraint": "c7", "contradicts": ["c3", "c5"]}`, the contradicted
/// ones in the sketch's order; with `truncated: true` when not every
/// candidate was tried (more than [`MAX_CANDIDATES`], or the budget of
/// [`MAX_PROBES`] solves and [`MAX_MS`] ran out), and with `reason` when
/// none was found. The copy starts from `open`'s regeneration cache, so
/// only the sketch is solved again per candidate.
pub(crate) fn contradictions(
    open: &OpenDocument,
    id: FeatureId,
    conflicting: &[ConstraintId],
) -> Explained {
    let key = (open.id, open.history.revision(), id);
    if let Some((at, explained)) = LAST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        && *at == key
    {
        return explained.clone();
    }
    let explained = work_out(open, id, conflicting);
    *LAST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((key, explained.clone()));
    explained
}

fn work_out(open: &OpenDocument, id: FeatureId, conflicting: &[ConstraintId]) -> Explained {
    let base = open.history.current();
    let Ok(sketch) = base.sketch(id) else {
        return Explained::default();
    };
    let order = |c: &ConstraintId| sketch.constraints.iter().position(|k| k.id == *c);
    let mut copy = open.clone();
    if copy
        .history
        .apply("probe", |doc| doc.set_rollback(Some(id)))
        .is_err()
    {
        return Explained::default();
    }
    let mut scratch = Session::new(Workspace::none());
    if scratch.add_document(copy).is_err() {
        return Explained::default();
    }
    let state_of = |scratch: &mut Session, c: ConstraintId| -> Option<ConstraintState> {
        let open = scratch.document_mut().ok()?;
        let regen = open.regen();
        let solved = regen.feature(id)?.sketch.as_ref()?;
        solved
            .constraints
            .iter()
            .find(|r| r.id == c)
            .map(|r| r.state)
    };
    let clock = Clock::start();
    let mut probes = 0_usize;
    let out_of_budget = |probes: usize| probes >= MAX_PROBES || clock.past(MAX_MS);
    let mut explained = Explained {
        entries: Vec::new(),
        truncated: conflicting.len() > MAX_CONFLICTS,
    };
    for conflict in conflicting.iter().take(MAX_CONFLICTS) {
        if out_of_budget(probes) {
            explained.truncated = true;
            break;
        }
        // The copy says the same as the document it was taken from.
        if state_of(&mut scratch, *conflict) != Some(ConstraintState::Conflicting) {
            continue;
        }
        let (tried, mut truncated) = candidates(sketch, *conflict);
        let mut found: Vec<ConstraintId> = Vec::new();
        for j in tried {
            if out_of_budget(probes) {
                truncated = true;
                break;
            }
            probes += 1;
            let Ok(open) = scratch.document_mut() else {
                break;
            };
            let removed = open.history.apply("probe", |doc| {
                doc.edit_sketch(id, |s| s.remove_constraint(j))
            });
            if removed.is_err() {
                continue;
            }
            let now = state_of(&mut scratch, *conflict);
            if matches!(
                now,
                Some(ConstraintState::Satisfied | ConstraintState::Redundant)
            ) {
                found.push(j);
            }
            if let Ok(open) = scratch.document_mut() {
                let _ = open.history.undo();
            }
        }
        found.sort_by_key(|c| order(c));
        let mut entry = json!({
            "constraint": conflict.to_string(),
            "contradicts": found.iter().map(ToString::to_string).collect::<Vec<_>>(),
        });
        if let Some(map) = entry.as_object_mut() {
            if truncated {
                map.insert("truncated".into(), true.into());
            }
            if found.is_empty() {
                let reason = if truncated {
                    "not found among the constraints tried before the limit"
                } else if on_an_arc(sketch, *conflict) {
                    "no constraint alone: it contradicts an arc's own shape (its ends equally far from its centre) together with the constraints on that arc"
                } else {
                    "no constraint alone: it contradicts several of them together"
                };
                map.insert("reason".into(), reason.into());
            }
        }
        explained.entries.push(entry);
    }
    explained
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use craft3d_doc::{ConstraintKind, DistanceDirection, Expr, SketchPlane};

    #[test]
    fn candidates_are_capped_latest_first_and_say_so() {
        let mut s = Sketch::new(SketchPlane::default());
        let p = s.add_point(0.0, 0.0).unwrap();
        let q = s.add_point(5.0, 0.0).unwrap();
        let far = s.add_point(9.0, 9.0).unwrap();
        let mut ids = Vec::new();
        for i in 0..40 {
            ids.push(
                s.add_constraint(ConstraintKind::X {
                    point: p,
                    value: Expr::new(format!("{i}")),
                })
                .unwrap(),
            );
        }
        // Not connected to p: never a candidate.
        let apart = s
            .add_constraint(ConstraintKind::Fixed { point: far })
            .unwrap();
        let last = s
            .add_constraint(ConstraintKind::Distance {
                a: p,
                b: Some(q),
                direction: DistanceDirection::Direct,
                value: Expr::new("1"),
            })
            .unwrap();
        let (chosen, truncated) = candidates(&s, last);
        assert!(truncated);
        assert_eq!(chosen.len(), MAX_CANDIDATES);
        assert_eq!(chosen[0], ids[39]);
        assert!(!chosen.contains(&apart));
        // A sketch of a few: all of them, nothing cut.
        let mut small = Sketch::new(SketchPlane::default());
        let a = small.add_point(0.0, 0.0).unwrap();
        let c1 = small
            .add_constraint(ConstraintKind::Fixed { point: a })
            .unwrap();
        let c2 = small
            .add_constraint(ConstraintKind::Fixed { point: a })
            .unwrap();
        assert_eq!(candidates(&small, c2), (vec![c1], false));
    }
}
