//! Features written by their names in the persistent names a command is
//! given: `Plate:face(1)` for `F2:face(1)`, also inside a name
//! (`F7:copy(1)/Pin:side(e2)`, `Plate:endCap(Off:offset(e1))`).
//!
//! A generated script knows the names it gives its features, not the ids
//! the document hands out, so it could not name a face without building
//! first. Before a command runs, every string of its parameters that reads
//! as a persistent name once its features' names are looked up in the
//! active document is replaced by that name with ids
//! ([`PersistentName::parse_with_features`]); the command then sees ids
//! only, as from a file, and what it stores has ids only (the format and
//! the naming do not change). The lookup is done when the command runs, so
//! a script may name a feature made by an earlier step.
//!
//! A string that does not read as a name stays as it is, so the command
//! says what is wrong with it; when that is a feature name no feature has
//! (or several have), [`Resolved::explain`] adds it to the command's error.

use crate::CommandError;
use craft3d_doc::{Document, FeatureId, PersistentName};
use serde_json::Value;

/// Parameters whose strings are free text, never a face or an edge: not
/// looked into, so a label that happens to read `Plate:face(1)` stays.
const FREE_TEXT: [&str; 10] = [
    "name",
    "rename",
    "label",
    "description",
    "text",
    "title",
    "path",
    "message",
    "group",
    "author",
];

/// How deep parameters are looked into, and how many strings at most.
const MAX_DEPTH: usize = 32;
const MAX_STRINGS: usize = 100_000;

/// What the lookup found.
#[derive(Default)]
pub(crate) struct Resolved {
    /// The parameters with ids, when a string changed.
    pub params: Option<Value>,
    /// Strings that read as names but for a feature name no feature has,
    /// or several have: the string and the reason.
    unknown: Vec<(String, String)>,
}

impl Resolved {
    /// The command's error with the reason a feature name was not found,
    /// when the error is about a string that had one.
    pub(crate) fn explain(&self, error: CommandError) -> CommandError {
        let message = error.message();
        for (text, reason) in &self.unknown {
            let shown: String = text.chars().take(60).collect();
            if message.contains(&format!("`{shown}`")) && !message.contains(reason.as_str()) {
                return CommandError::new(format!("{message} ({reason})"));
            }
        }
        error
    }
}

/// The id of the feature called `name` in `doc`, or why there is none.
fn feature_called(doc: &Document, name: &str) -> Result<FeatureId, String> {
    let mut found = doc.features.iter().filter(|f| f.name == name);
    match (found.next(), found.next()) {
        (Some(feature), None) => Ok(feature.id),
        (Some(_), Some(_)) => Err(format!(
            "several features are called `{name}`; write the id of the one meant (document.inspect lists them)"
        )),
        (None, _) => Err(format!(
            "no feature is called `{name}`; document.inspect lists the features with their names and ids"
        )),
    }
}

/// Looks up the features' names in every name of `params` (see the module
/// documentation).
pub(crate) fn resolve(doc: &Document, params: &Value) -> Resolved {
    let mut resolved = Resolved::default();
    // Most commands have no name to look up: no copy of their parameters.
    let mut budget = MAX_STRINGS;
    if !any_candidate(params, 0, &mut budget) {
        return resolved;
    }
    let mut budget = MAX_STRINGS;
    let mut out = params.clone();
    let changed = walk(doc, &mut out, 0, &mut budget, &mut resolved.unknown);
    if changed {
        resolved.params = Some(out);
    }
    resolved
}

/// Whether `value` has a string that may be a name with a feature's name:
/// one with a `:` that is no name with ids.
fn any_candidate(value: &Value, depth: usize, budget: &mut usize) -> bool {
    if depth > MAX_DEPTH || *budget == 0 {
        return false;
    }
    match value {
        Value::String(text) => {
            *budget -= 1;
            text.contains(':') && text.parse::<PersistentName>().is_err()
        }
        Value::Array(items) => items
            .iter()
            .any(|item| any_candidate(item, depth + 1, budget)),
        Value::Object(map) => map.iter().any(|(key, item)| {
            !FREE_TEXT.contains(&key.as_str()) && any_candidate(item, depth + 1, budget)
        }),
        _ => false,
    }
}

/// Rewrites the names in `value`; true when one changed.
fn walk(
    doc: &Document,
    value: &mut Value,
    depth: usize,
    budget: &mut usize,
    unknown: &mut Vec<(String, String)>,
) -> bool {
    if depth > MAX_DEPTH || *budget == 0 {
        return false;
    }
    match value {
        Value::String(text) => {
            *budget -= 1;
            match rewrite(doc, text) {
                Ok(Some(with_ids)) => {
                    *text = with_ids;
                    true
                }
                Ok(None) => false,
                Err(reason) => {
                    if unknown.len() < 16 {
                        unknown.push((text.clone(), reason));
                    }
                    false
                }
            }
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= walk(doc, item, depth + 1, budget, unknown);
            }
            changed
        }
        Value::Object(map) => {
            let mut changed = false;
            for (key, item) in map.iter_mut() {
                if !FREE_TEXT.contains(&key.as_str()) {
                    changed |= walk(doc, item, depth + 1, budget, unknown);
                }
            }
            changed
        }
        _ => false,
    }
}

/// `text` with its features' names replaced by their ids, when it reads as
/// a persistent name only so; `Ok(None)` when it is a name with ids
/// already or no name at all; `Err` when it reads as a name but for a
/// feature name that names no one feature.
fn rewrite(doc: &Document, text: &str) -> Result<Option<String>, String> {
    if !text.contains(':') || text.parse::<PersistentName>().is_ok() {
        return Ok(None);
    }
    let missing = std::cell::RefCell::new(None::<String>);
    let lookup = |name: &str| {
        feature_called(doc, name).inspect_err(|reason| {
            missing.borrow_mut().get_or_insert_with(|| reason.clone());
        })
    };
    match PersistentName::parse_with_features(text, &lookup) {
        Ok(name) => Ok(Some(name.to_string())),
        Err(_) => match missing.into_inner() {
            // Every feature found, but no name all the same: the command
            // says why.
            None => Ok(None),
            Some(reason) => {
                // A name only with the features found: a name it is, of a
                // feature that is not there.
                let any = |_: &str| Ok(FeatureId(0));
                if PersistentName::parse_with_features(text, &any).is_ok() {
                    Err(reason)
                } else {
                    Ok(None)
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_doc::{Block, Expr, FeatureKind, Orientation, Units};
    use serde_json::json;

    fn doc() -> Document {
        let mut doc = Document::new(Units::default());
        for name in ["Plate", "Pin"] {
            let block = Block {
                position: ["0", "0", "0"].map(Expr::new),
                size: ["1", "1", "1"].map(Expr::new),
                orientation: Orientation::default(),
            };
            doc.add_feature(Some(name), FeatureKind::Block(block))
                .unwrap();
        }
        doc
    }

    #[test]
    fn names_with_feature_names_get_ids_and_the_rest_stays() {
        let doc = doc();
        let params = json!({
            "edges": [["Plate:face(1)", "F1:face(2)"], ["Pin:startCap", "Pin:face(0)"]],
            "upTo": "Pin:face(1)",
            "name": "Plate:face(1)",
            "distance": "2 mm",
            "feature": "Plate",
        });
        let resolved = resolve(&doc, &params);
        assert_eq!(
            resolved.params.unwrap(),
            json!({
                "edges": [["F1:face(1)", "F1:face(2)"], ["F2:startCap()", "F2:face(0)"]],
                "upTo": "F2:face(1)",
                "name": "Plate:face(1)",
                "distance": "2 mm",
                "feature": "Plate",
            })
        );
        // Nothing to change: no copy.
        assert!(
            resolve(&doc, &json!({"edges": [["F1:face(1)", "F1:face(2)"]]}))
                .params
                .is_none()
        );
    }

    #[test]
    fn an_unknown_feature_name_is_explained_in_the_commands_error() {
        let doc = doc();
        let resolved = resolve(&doc, &json!({"face": "Plat:face(1)"}));
        assert!(resolved.params.is_none());
        let error = CommandError::new(
            "x: `face`: `Plat:face(1)` is not a name: the feature must be written F and a number",
        );
        let explained = resolved.explain(error).message().to_string();
        assert!(
            explained.contains("no feature is called `Plat`"),
            "{explained}"
        );
        // Text that is no name stays without a reason.
        let resolved = resolve(&doc, &json!({"face": "a: b", "x": "Plate:roof(1)"}));
        assert!(resolved.params.is_none() && resolved.unknown.is_empty());
    }

    #[test]
    fn a_name_two_features_share_is_refused() {
        let mut doc = doc();
        // Only an old file can hold it; built by hand here.
        doc.features[1].name = "Plate".to_string();
        let resolved = resolve(&doc, &json!({"face": "Plate:face(1)"}));
        assert!(resolved.params.is_none());
        assert!(
            resolved.unknown[0].1.contains("several features"),
            "{:?}",
            resolved.unknown
        );
    }

    #[test]
    fn hostile_parameters_are_bounded() {
        let doc = doc();
        let mut deep = json!("Plate:face(1)");
        for _ in 0..200 {
            deep = json!([deep]);
        }
        // Too deep to be looked into: left as it is, no panic.
        assert!(resolve(&doc, &deep).params.is_none());
        let many = Value::Array(vec![json!("Plate:face(1)"); MAX_STRINGS + 10]);
        let resolved = resolve(&doc, &many).params.unwrap();
        let rewritten = resolved
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| *v == "F1:face(1)")
            .count();
        assert_eq!(rewritten, MAX_STRINGS);
    }
}
