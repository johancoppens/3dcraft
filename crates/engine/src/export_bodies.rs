//! Which bodies `file.export` writes to STEP and the mesh formats: every
//! body, or those named in `bodies` (the command itself is in
//! `commands.rs`). A body is named as the scene names it: by the feature
//! that made it, its name or its id.

use crate::CommandError;
use crate::commands::fail;
use crate::params::Params;
use craft3d_doc::{Document, FeatureId};
use craft3d_regen::{Regen, Status};
use serde_json::Value;

const ID: &str = "file.export";

/// No document has more bodies than this asked for in one export; a
/// longer list is refused before anything is looked up.
const MAX_BODIES: usize = 10_000;

/// The features whose bodies to export, from `bodies`: `None` when it is
/// not given or null (every body). Each must name a feature that has a body of
/// its own now; an empty list, an unknown name and a feature without a
/// body are errors that say which, and what there is.
pub(crate) fn chosen(
    p: &Params,
    doc: &Document,
    regen: &Regen,
) -> Result<Option<Vec<FeatureId>>, CommandError> {
    let Some(value) = p.get("bodies") else {
        return Ok(None);
    };
    let Value::Array(items) = value else {
        return fail(format!(
            "{ID}: `bodies` must be a list of bodies, each the id (\"F3\") or name of the feature that made it"
        ));
    };
    if items.is_empty() {
        return fail(format!(
            "{ID}: `bodies` is empty; name the bodies to export ({}), or leave it out to export every body",
            existing(doc, regen)
        ));
    }
    if items.len() > MAX_BODIES {
        return fail(format!(
            "{ID}: `bodies` names {} bodies; at most {MAX_BODIES} at a time",
            items.len()
        ));
    }
    let mut out: Vec<FeatureId> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let id = feature(doc, regen, index, item)?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(Some(out))
}

/// The feature `item` names, which must have a body of its own now.
fn feature(
    doc: &Document,
    regen: &Regen,
    index: usize,
    item: &Value,
) -> Result<FeatureId, CommandError> {
    let found = match item {
        Value::String(text) => doc.feature_by_name(text).map(|f| f.id).or_else(|| {
            text.strip_prefix('F')
                .and_then(|digits| digits.parse::<u64>().ok())
                .and_then(|n| doc.feature(FeatureId(n)).map(|f| f.id))
        }),
        Value::Number(n) => n
            .as_u64()
            .and_then(|n| doc.feature(FeatureId(n)).map(|f| f.id)),
        _ => {
            return fail(format!(
                "{ID}: `bodies.{index}` must be the id (\"F3\") or name of the feature that made the body"
            ));
        }
    };
    let shown: String = item
        .as_str()
        .map_or_else(|| item.to_string(), str::to_string)
        .chars()
        .take(80)
        .collect();
    let Some(id) = found else {
        return fail(format!(
            "{ID}: `bodies.{index}`: there is no feature `{shown}`; the bodies are {}",
            existing(doc, regen)
        ));
    };
    if regen.bodies().any(|(body, _)| body == id) {
        return Ok(id);
    }
    let name = doc.feature(id).map_or("", |f| f.name.as_str());
    let outcome = regen.feature(id);
    let why = match (
        outcome.map(|o| &o.status),
        outcome.and_then(|o| o.consumed_by),
    ) {
        (Some(Status::Failed(_)), _) => "it failed (document.check says why)".to_string(),
        (Some(Status::Suppressed), _) => "it is suppressed".to_string(),
        (Some(Status::Inactive), _) => "it is inactive (its condition is 0)".to_string(),
        (Some(Status::RolledBack), _) => "it is after the rollback marker".to_string(),
        (_, Some(later)) => {
            // The splits of a body took it together: all of them.
            let takers = if craft3d_regen::taken_by_its_splits(doc, id, later) {
                craft3d_regen::splits_of(doc, &regen.features, id)
            } else {
                vec![later]
            };
            let named: Vec<String> = takers
                .iter()
                .map(|t| {
                    let name = doc.feature(*t).map_or("", |f| f.name.as_str());
                    format!("{t} ({name})")
                })
                .collect();
            if named.len() > 1 {
                format!(
                    "it was split into {}; export those bodies",
                    named.join(", ")
                )
            } else {
                format!("{} took it; export that body", named.join(", "))
            }
        }
        _ => "it makes no body".to_string(),
    };
    fail(format!(
        "{ID}: `bodies.{index}`: {id} ({name}) has no body now: {why}; the bodies are {}",
        existing(doc, regen)
    ))
}

/// The bodies there are, for an error: the feature of each, by name and
/// id, at most 20.
fn existing(doc: &Document, regen: &Regen) -> String {
    let names: Vec<String> = regen
        .bodies()
        .take(20)
        .map(|(id, _)| {
            let name = doc.feature(id).map_or("", |f| f.name.as_str());
            format!("{name} ({id})")
        })
        .collect();
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}
