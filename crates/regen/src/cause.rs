//! Which feature to fix when a feature fails because something it is made
//! from failed.
//!
//! A feature whose input failed says so in its reason ("`Plate` failed;
//! fix it first"), and so does what is made from that feature in turn: a
//! chain of features each waiting for the one before. [`root_cause`]
//! walks the chain to its start, the one feature that failed for a reason
//! of its own.

use crate::{FeatureResult, Status};
use craft3d_doc::{Document, FeatureId};

/// What the reason of a feature says of an input that failed. Every place
/// that reports one uses this text, and [`root_cause`] goes by it.
pub(crate) const FIX_IT_FIRST: &str = "failed; fix it first";

/// The input of `id` that failed, when `id` failed and its reason is that
/// an input failed.
fn failed_input(doc: &Document, results: &[FeatureResult], id: FeatureId) -> Option<FeatureId> {
    let position = |id: FeatureId| results.iter().position(|r| r.id == id);
    let here = position(id)?;
    let Status::Failed(reason) = &results.get(here)?.status else {
        return None;
    };
    if !reason.contains(FIX_IT_FIRST) {
        return None;
    }
    doc.feature(id)?.kind.inputs().into_iter().find(|input| {
        position(*input).is_some_and(|at| {
            at < here && matches!(results.get(at).map(|r| &r.status), Some(Status::Failed(_)))
        })
    })
}

/// For a feature that failed because something it is made from failed:
/// the feature at the start of that chain, which failed for a reason of
/// its own and is the one to fix. `None` for a feature that did not fail
/// or failed for a reason of its own. `results` are those of a
/// regeneration of `doc`, in history order.
pub fn root_cause(doc: &Document, results: &[FeatureResult], id: FeatureId) -> Option<FeatureId> {
    let mut root = None;
    let mut at = id;
    // Each step goes to a feature earlier in the history, so the walk
    // ends; the bound is for a list of results that is not a
    // regeneration's.
    for _ in 0..results.len() {
        let Some(input) = failed_input(doc, results, at) else {
            break;
        };
        root = Some(input);
        at = input;
    }
    root
}
