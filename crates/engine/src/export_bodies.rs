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
fn chosen(
    p: &Params,
    doc: &Document,
    regen: &Regen,
) -> Result<(Option<Vec<FeatureId>>, Vec<String>), CommandError> {
    let (chosen, off) = chosen_skipping(ID, p, doc, regen)?;
    if chosen.as_ref().is_some_and(Vec::is_empty) && !off.is_empty() {
        let why: Vec<String> = off
            .iter()
            .map(|(name, why)| format!("`{name}` is {why}"))
            .collect();
        return fail(format!(
            "{ID}: every body `bodies` names is switched off now ({}); name a body that is on, or leave `bodies` out",
            why.join("; ")
        ));
    }
    Ok((chosen, off.into_iter().map(|(name, _)| name).collect()))
}

/// What a 3D export writes: the features whose bodies (`None`: every
/// body), and what it left out, by the features' names: hidden bodies
/// (`hiddenLeftOut`) and bodies `bodies` names that are switched off
/// (`switchedOffLeftOut`).
pub(crate) struct Written {
    pub only: Option<Vec<FeatureId>>,
    pub hidden: Vec<String>,
    pub off: Vec<String>,
}

impl Written {
    /// Adds what was left out to a result: `hiddenLeftOut` and
    /// `switchedOffLeftOut`, each `{count, bodies}` when there are any.
    pub(crate) fn report(&self, result: &mut Value) {
        if let Some(map) = result.as_object_mut() {
            if let Some(hidden) = names_json(&self.hidden) {
                map.insert("hiddenLeftOut".into(), hidden);
            }
            if let Some(off) = names_json(&self.off) {
                map.insert("switchedOffLeftOut".into(), off);
            }
        }
    }
}

/// The bodies a 3D export (STEP, STL, 3MF, OBJ) writes and the hidden ones
/// it leaves out, by their features' names: those `bodies` names, hidden
/// or not ([`chosen`]); without it every body that is not hidden
/// (`feature.visible`), as the drawings leave hidden bodies out (J-045).
/// Every body hidden and none named is refused: there is nothing to write.
pub(crate) fn written(p: &Params, doc: &Document, regen: &Regen) -> Result<Written, CommandError> {
    let (named, off) = chosen(p, doc, regen)?;
    if named.is_some() {
        return Ok(Written {
            only: named,
            hidden: Vec::new(),
            off,
        });
    }
    let (mut shown, mut hidden) = (Vec::new(), Vec::new());
    for (id, _) in regen.bodies() {
        if doc.is_hidden(id) {
            hidden.push(
                doc.feature(id)
                    .map_or_else(|| id.to_string(), |f| f.name.clone()),
            );
        } else {
            shown.push(id);
        }
    }
    if hidden.is_empty() {
        return Ok(Written {
            only: None,
            hidden,
            off,
        });
    }
    if shown.is_empty() {
        return fail(format!(
            "{ID}: every body is hidden ({}); show one with feature.visible, or name the bodies to write with `bodies`",
            hidden.join(", ")
        ));
    }
    Ok(Written {
        only: Some(shown),
        hidden,
        off,
    })
}

/// What a result says of the bodies an export left out, when it left any
/// out: `{count, bodies}`.
pub(crate) fn names_json(names: &[String]) -> Option<Value> {
    (!names.is_empty()).then(|| serde_json::json!({ "count": names.len(), "bodies": names }))
}

/// [`chosen`] for the command `command`, which its errors name, with the
/// names of the bodies `bodies` names that were left out as switched off;
/// a list whose every body is switched off is an empty list (`cut.list`
/// lists nothing).
pub(crate) fn chosen_for(
    command: &str,
    p: &Params,
    doc: &Document,
    regen: &Regen,
) -> Result<(Option<Vec<FeatureId>>, Vec<String>), CommandError> {
    chosen_skipping(command, p, doc, regen)
        .map(|(chosen, off)| (chosen, off.into_iter().map(|(name, _)| name).collect()))
}

/// [`chosen_for`], with the features left out because they are switched
/// off: each its name and why (`inactive (its condition is 0)`).
#[allow(clippy::type_complexity)]
fn chosen_skipping(
    command: &str,
    p: &Params,
    doc: &Document,
    regen: &Regen,
) -> Result<(Option<Vec<FeatureId>>, Vec<(String, String)>), CommandError> {
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
    let mut off: Vec<(String, String)> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match feature(command, doc, regen, index, item)? {
            Named::Body(id) => {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
            Named::Off(name, why) => {
                if !off.iter().any(|(n, _)| *n == name) {
                    off.push((name, why));
                }
            }
        }
    }
    Ok((Some(out), off))
}

/// What a name in `bodies` is: a feature with a body of its own now, or
/// one switched off now (why, in words).
enum Named {
    Body(FeatureId),
    Off(String, String),
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
    if let Some(why) = crate::verify_cmds::switched_off(doc, regen, id) {
        return Ok(Named::Off(name.to_string(), why.to_string()));
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
