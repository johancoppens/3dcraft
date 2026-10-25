//! Features written by their names in the persistent names a command is
//! given: `Plate:face(1)` for `F2:face(1)`, also inside a name
//! (`F7:copy(1)/Pin:side(e2)`, `Plate:endCap(Off:offset(e1))`).
//!
//! A generated script knows the names it gives its features, not the ids
//! the document hands out, so it could not name a face without building
//! first. Before a command runs, every string of the parameters its spec
//! lists as holding names (`CommandSpec::names`: a command opts in, nothing
//! else is looked into) that reads as a persistent name once its features'
//! names are looked up in the active document is replaced by that name
//! with ids ([`PersistentName::parse_with_features`]); the command then sees
//! ids only, as from a file, and what it stores has ids only (the format
//! and the naming do not change). The lookup is done when the command runs,
//! so a script may name a feature made by an earlier step.
//!
//! In those parameters a name of a feature no feature has (or several
//! have) is refused before the command runs. A string that does not read
//! as a name at all stays as it is, and the command says what is wrong
//! with it. The command's errors show the names as the caller wrote them
//! ([`Resolved::explain`]).

use crate::CommandError;
use craft3d_doc::{Document, FeatureId, PersistentName};
use serde_json::Value;

/// How deep parameters are looked into, and how many strings at most.
const MAX_DEPTH: usize = 32;
const MAX_STRINGS: usize = 100_000;

/// What the lookup found.
#[derive(Default, Debug)]
pub struct Resolved {
    /// The parameters with ids, when a string changed.
    pub params: Option<Value>,
    /// Every name that changed: as written, and with ids.
    rewritten: Vec<(String, String)>,
}

impl Resolved {
    /// The command's error with every name it shows with ids written back
    /// as the caller wrote it (the longest first, so a name inside another
    /// is not taken for part of it).
    pub(crate) fn explain(&self, error: CommandError) -> CommandError {
        if self.rewritten.is_empty() {
            return error;
        }
        let mut pairs: Vec<&(String, String)> = self.rewritten.iter().collect();
        pairs.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.1.cmp(&b.1)));
        let mut message = error.message().to_string();
        for (written, with_ids) in pairs {
            if written != with_ids && message.contains(with_ids.as_str()) {
                message = message.replace(with_ids.as_str(), written);
            }
        }
        CommandError::new(message)
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

/// Looks up the features' names in every name under the parameters `keys`
/// of `params` (see the module documentation). `Err` with the parameter,
/// the text and the reason for a name of a feature that is not there.
pub fn resolve(doc: &Document, keys: &[&str], params: &Value) -> Result<Resolved, String> {
    let mut resolved = Resolved::default();
    let Value::Object(map) = params else {
        return Ok(resolved);
    };
    let mut budget = MAX_STRINGS;
    let mut out: Option<serde_json::Map<String, Value>> = None;
    for key in keys {
        let Some(value) = map.get(*key) else {
            continue;
        };
        let mut value = value.clone();
        let changed = walk(
            doc,
            key,
            &mut value,
            0,
            &mut budget,
            &mut resolved.rewritten,
        )?;
        if changed {
            out.get_or_insert_with(|| map.clone())
                .insert((*key).to_string(), value);
        }
    }
    resolved.params = out.map(Value::Object);
    Ok(resolved)
}

/// Rewrites the names in `value`; true when one changed.
fn walk(
    doc: &Document,
    key: &str,
    value: &mut Value,
    depth: usize,
    budget: &mut usize,
    rewritten: &mut Vec<(String, String)>,
) -> Result<bool, String> {
    if depth > MAX_DEPTH || *budget == 0 {
        return Ok(false);
    }
    match value {
        Value::String(text) => {
            *budget -= 1;
            match rewrite(doc, text) {
                Ok(Some(with_ids)) => {
                    if rewritten.len() < 1000 {
                        rewritten.push((text.clone(), with_ids.clone()));
                    }
                    *text = with_ids;
                    Ok(true)
                }
                Ok(None) => Ok(false),
                Err(reason) => {
                    let shown: String = text.chars().take(60).collect();
                    Err(format!("`{key}`: `{shown}`: {reason}"))
                }
            }
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= walk(doc, key, item, depth + 1, budget, rewritten)?;
            }
            Ok(changed)
        }
        Value::Object(map) => {
            let mut changed = false;
            for item in map.values_mut() {
                changed |= walk(doc, key, item, depth + 1, budget, rewritten)?;
            }
            Ok(changed)
        }
        _ => Ok(false),
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
    fn only_the_listed_parameters_get_ids() {
        let doc = doc();
        let params = json!({
            "edges": [["Plate:face(1)", "F1:face(2)"], ["Pin:startCap", "Pin:face(0)"]],
            "upTo": "Pin:face(1)",
            "label": "Plate:face(1)",
            "rows": [["Plate:face(1)"]],
            "distance": "2 mm",
        });
        let resolved = resolve(&doc, &["edges", "upTo", "missing"], &params).unwrap();
        assert_eq!(
            resolved.params.unwrap(),
            json!({
                "edges": [["F1:face(1)", "F1:face(2)"], ["F2:startCap()", "F2:face(0)"]],
                "upTo": "F2:face(1)",
                "label": "Plate:face(1)",
                "rows": [["Plate:face(1)"]],
                "distance": "2 mm",
            })
        );
        // Nothing to change: no copy.
        let ids = json!({"edges": [["F1:face(1)", "F1:face(2)"]]});
        assert!(resolve(&doc, &["edges"], &ids).unwrap().params.is_none());
        // Nothing listed: nothing looked into.
        assert!(resolve(&doc, &[], &params).unwrap().params.is_none());
    }

    #[test]
    fn an_unknown_feature_name_is_refused_and_errors_show_names_as_written() {
        let doc = doc();
        let error = resolve(&doc, &["face"], &json!({"face": "Plat:face(1)"})).unwrap_err();
        assert!(error.contains("`face`: `Plat:face(1)`"), "{error}");
        assert!(error.contains("no feature is called `Plat`"), "{error}");
        // Text that is no name stays, without a reason.
        let resolved =
            resolve(&doc, &["face"], &json!({"face": ["a: b", "Plate:roof(1)"]})).unwrap();
        assert!(resolved.params.is_none());
        // The command's error shows what was written.
        let resolved = resolve(
            &doc,
            &["faces"],
            &json!({"faces": ["Plate:face(1)", "Pin:copy(0)/Plate:face(1)"]}),
        )
        .unwrap();
        let error = CommandError::new("x: F2:copy(0)/F1:face(1) and F1:face(1) are not there");
        assert_eq!(
            resolved.explain(error).message(),
            "x: Pin:copy(0)/Plate:face(1) and Plate:face(1) are not there"
        );
    }

    #[test]
    fn a_name_two_features_share_is_refused() {
        let mut doc = doc();
        // Only an old file can hold it; built by hand here.
        doc.features[1].name = "Plate".to_string();
        let error = resolve(&doc, &["face"], &json!({"face": "Plate:face(1)"})).unwrap_err();
        assert!(error.contains("several features"), "{error}");
    }

    #[test]
    fn hostile_parameters_are_bounded() {
        let doc = doc();
        let mut deep = json!("Plate:face(1)");
        for _ in 0..200 {
            deep = json!([deep]);
        }
        // Too deep to be looked into: left as it is, no panic.
        let params = json!({ "faces": deep });
        assert!(resolve(&doc, &["faces"], &params).unwrap().params.is_none());
        let many = json!({ "faces": vec![json!("Plate:face(1)"); MAX_STRINGS + 10] });
        let resolved = resolve(&doc, &["faces"], &many).unwrap().params.unwrap();
        let rewritten = resolved["faces"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| *v == "F1:face(1)")
            .count();
        assert_eq!(rewritten, MAX_STRINGS);
        // Not an object: nothing to look into.
        assert!(
            resolve(&doc, &["faces"], &json!([1]))
                .unwrap()
                .params
                .is_none()
        );
    }
}
