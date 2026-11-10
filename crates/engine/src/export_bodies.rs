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
/// its own now, or one switched off now (its condition is 0, or
/// suppressed), which is left out so that one list holds for every option;
/// an empty list, an unknown name and a feature without a body are errors
/// that say which, and what there is, and so is a list whose every body is
/// switched off: there is nothing to write.
pub(crate) fn chosen(
    p: &Params,
    doc: &Document,
    regen: &Regen,
) -> Result<Option<Vec<FeatureId>>, CommandError> {
    let (chosen, off) = chosen_skipping(ID, p, doc, regen)?;
    if chosen.as_ref().is_some_and(Vec::is_empty) && !off.is_empty() {
        return fail(format!(
            "{ID}: every body `bodies` names is switched off now ({}); name a body that is on, or leave `bodies` out",
            off.join("; ")
        ));
    }
    Ok(chosen)
}

/// [`chosen`] for the command `command`, which its errors name; a list
/// whose every body is switched off is an empty list (`cut.list` lists
/// nothing).
pub(crate) fn chosen_for(
    command: &str,
    p: &Params,
    doc: &Document,
    regen: &Regen,
) -> Result<Option<Vec<FeatureId>>, CommandError> {
    chosen_skipping(command, p, doc, regen).map(|(chosen, _)| chosen)
}

/// [`chosen_for`], with the features left out because they are switched
/// off, each said as `` `Name` is inactive (its condition is 0)``.
fn chosen_skipping(
    command: &str,
    p: &Params,
    doc: &Document,
    regen: &Regen,
) -> Result<(Option<Vec<FeatureId>>, Vec<String>), CommandError> {
    let Some(value) = p.get("bodies") else {
        return Ok((None, Vec::new()));
    };
    let Value::Array(items) = value else {
        return fail(format!(
            "{command}: `bodies` must be a list of bodies, each the id (\"F3\") or name of the feature that made it"
        ));
    };
    if items.is_empty() {
        return fail(format!(
            "{command}: `bodies` is empty; name the bodies to export ({}), or leave it out to export every body",
            existing(doc, regen)
        ));
    }
    if items.len() > MAX_BODIES {
        return fail(format!(
            "{command}: `bodies` names {} bodies; at most {MAX_BODIES} at a time",
            items.len()
        ));
    }
    let mut out: Vec<FeatureId> = Vec::with_capacity(items.len());
    let mut off: Vec<String> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match feature(command, doc, regen, index, item)? {
            Named::Body(id) => {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
            Named::Off(why) => off.push(why),
        }
    }
    Ok((Some(out), off))
}

/// What a name in `bodies` is: a feature with a body of its own now, or
/// one switched off now (why, in words).
enum Named {
    Body(FeatureId),
    Off(String),
}

/// The feature `item` names, which must have a body of its own now or be
/// switched off.
fn feature(
    command: &str,
    doc: &Document,
    regen: &Regen,
    index: usize,
    item: &Value,
) -> Result<Named, CommandError> {
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
                "{command}: `bodies.{index}` must be the id (\"F3\") or name of the feature that made the body"
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
            "{command}: `bodies.{index}`: there is no feature `{shown}`; the bodies are {}",
            existing(doc, regen)
        ));
    };
    if regen.bodies().any(|(body, _)| body == id) {
        return Ok(Named::Body(id));
    }
    let name = doc.feature(id).map_or("", |f| f.name.as_str());
    if let Some(why) = crate::verify_cmds::switched_off(regen, id) {
        return Ok(Named::Off(format!("`{name}` is {why}")));
    }
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
        "{command}: `bodies.{index}`: {id} ({name}) has no body now: {why}; the bodies are {}",
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
