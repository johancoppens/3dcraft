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

use craft3d_doc::sketch::{ConstraintId, EntityId, Sketch};
use craft3d_doc::{Document, FeatureId};
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
/// after it; at most [`MAX_CANDIDATES`].
fn candidates(sketch: &Sketch, conflict: ConstraintId) -> Vec<ConstraintId> {
    let Some(at) = sketch.constraints.iter().position(|c| c.id == conflict) else {
        return Vec::new();
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
    ids.iter()
        .filter(|(i, _)| *i < at)
        .rev()
        .chain(ids.iter().filter(|(i, _)| *i > at))
        .take(MAX_CANDIDATES)
        .map(|(_, id)| *id)
        .collect()
}

/// For each conflicting constraint of sketch `id` (at most
/// [`MAX_CONFLICTS`]), the constraints it contradicts:
/// `[{"constraint": "c7", "contradicts": ["c3", "c5"]}]`, the
/// contradicted ones in the sketch's order. Empty when the sketch has no
/// conflict or cannot be looked at.
pub(crate) fn contradictions(
    base: &Document,
    id: FeatureId,
    conflicting: &[ConstraintId],
) -> Vec<Value> {
    let Ok(sketch) = base.sketch(id) else {
        return Vec::new();
    };
    let order = |c: &ConstraintId| sketch.constraints.iter().position(|k| k.id == *c);
    let mut doc = base.clone();
    if doc.set_rollback(Some(id)).is_err() {
        return Vec::new();
    }
    let mut scratch = Session::new(Workspace::none());
    if scratch.add_document(OpenDocument::new(doc, None)).is_err() {
        return Vec::new();
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
    let mut out = Vec::new();
    for conflict in conflicting.iter().take(MAX_CONFLICTS) {
        // The copy says the same as the document it was taken from.
        if state_of(&mut scratch, *conflict) != Some(ConstraintState::Conflicting) {
            continue;
        }
        let mut found: Vec<ConstraintId> = Vec::new();
        for j in candidates(sketch, *conflict) {
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
        out.push(json!({
            "constraint": conflict.to_string(),
            "contradicts": found.iter().map(ToString::to_string).collect::<Vec<_>>(),
        }));
    }
    out
}
