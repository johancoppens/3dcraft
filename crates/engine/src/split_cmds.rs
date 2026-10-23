//! A body of several lumps split into one body per lump.
//!
//! `body.split` adds one split feature per lump of the source's body, all
//! in one undo step, each said by a face of that body (regen `split`:
//! the lump holding a piece of the face, numbered by where the lumps lie
//! in space when the face has pieces in several), never by the order of
//! the shells. The source's body goes into them together. On a body that
//! is split already it adds splits for the lumps that have none (one an
//! upstream edit made, one whose split was deleted). Which lumps there
//! are is read from the regeneration as it stands, as every command
//! reads the document; whether each split can be built again after an
//! upstream edit is for regeneration to say, as for every feature.

use crate::commands::{Run, edit, fail, feature_ref, feature_result, insert_feature, spec};
use crate::params::Params;
use crate::{CommandSpec, Session};
use craft3d_doc::{FeatureKind, Split};
use craft3d_regen::Status;
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("body.split", "Split Body", r#"{"source": id or name, "names": [string] = automatic}"#,
        "Splits the body of source, a solid of several lumps (a cut that fell apart, instances of a pattern that stay apart), into one body per lump: adds one split feature per lump, as one undo step, and the source's body goes into them. A cavity stays with the lump it lies in. Each split is said by a face of the source's body (face, in the result and in document.inspect): the lump that holds a piece of it, and, when pieces of that face lie in several lumps, number: which, in the order of where the lumps lie in space (by x, then y, then z of the centre of each lump). Faces keep their names with a prefix: \"F9:copy(0)/F1:face(4)\" is the piece in the lump of F9 of what the source called F7:copy(0)/F1:face(4); a reference to the source's face follows to the one split that has it, or lists the splits that do. Every lump is one split's: a lump no split holds (an upstream edit made one more, a split was deleted) fails the last split, and body.split on the source then adds splits for the lumps that have none. names gives the new splits their names, one per lump added, in the order of the result. Refused: a source whose body went into a feature that is not a split of it, a sheet, a body of one lump, a body that is split already, a body whose shells are not lumps and cavities.", doc: true, body_split),
];

fn body_split(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "body.split";
    let p = Params::new(ID, params, &["source", "names"])?;
    let source = feature_ref(session.document()?.history.current(), &p, ID, "source")?;
    let given: Option<Vec<String>> = match p.get("names") {
        None => None,
        Some(Value::Array(items)) => {
            let mut names = Vec::with_capacity(items.len());
            for item in items {
                let Some(text) = item.as_str() else {
                    return fail(format!("{ID}: `names` must be a list of strings"));
                };
                names.push(text.to_string());
            }
            Some(names)
        }
        Some(_) => return fail(format!("{ID}: `names` must be a list of strings")),
    };
    let anchors = {
        let open = session.document_mut()?;
        let regen = open.regen();
        let doc = open.history.current();
        let made_by = doc
            .feature(source)
            .map_or_else(|| source.to_string(), |f| f.name.clone());
        let Some(outcome) = regen.feature(source) else {
            return fail(format!("{ID}: there is no feature {source}"));
        };
        match &outcome.status {
            // Switched off, it passes on the body it changes.
            _ if outcome.is_built() => {}
            Status::Ok => {}
            Status::Suppressed => return fail(format!("{ID}: `{made_by}` is suppressed")),
            Status::Inactive => return fail(format!("{ID}: `{made_by}` is inactive")),
            Status::RolledBack => return fail(format!("{ID}: `{made_by}` is rolled back")),
            Status::Failed(reason) => {
                return fail(format!("{ID}: `{made_by}` failed ({reason}); fix it first"));
            }
        }
        let Some(body) = outcome.body.as_ref() else {
            return fail(format!("{ID}: `{made_by}` has no body"));
        };
        // A body its own splits took is split further; one that went
        // into anything else is that feature's.
        if let Some(taker) = outcome.consumed_by
            && !craft3d_regen::taken_by_its_splits(doc, source, taker)
        {
            let taker = doc
                .feature(taker)
                .map_or_else(|| taker.to_string(), |f| f.name.clone());
            return fail(format!(
                "{ID}: the body of `{made_by}` went into `{taker}`; split that feature instead"
            ));
        }
        let Some(names) = &outcome.names else {
            return fail(format!(
                "{ID}: `{made_by}` does not name its faces, so no lump of it can be told by one"
            ));
        };
        let plan = craft3d_regen::split_plan(doc, source, body, names)
            .map_err(|reason| crate::CommandError::new(format!("{ID}: `{made_by}`: {reason}")))?;
        if plan.lumps < 2 && plan.split_by.is_empty() {
            return fail(format!(
                "{ID}: `{made_by}` is one lump; there is nothing to split"
            ));
        }
        if plan.unsplit.is_empty() {
            let by: Vec<String> = plan
                .split_by
                .iter()
                .map(|id| {
                    doc.feature(*id)
                        .map_or_else(|| id.to_string(), |f| format!("`{}`", f.name))
                })
                .collect();
            return fail(format!(
                "{ID}: `{made_by}` is split already, into {}; every lump of it is one of them",
                by.join(", ")
            ));
        }
        plan.unsplit
    };
    if let Some(names) = &given {
        if names.len() != anchors.len() {
            return fail(format!(
                "{ID}: `names` has {} names for {} lumps to split",
                names.len(),
                anchors.len()
            ));
        }
        if let Some(twice) = names
            .iter()
            .enumerate()
            .find(|(i, name)| names.iter().take(*i).any(|other| other == *name))
        {
            return fail(format!("{ID}: `names` has `{}` twice", twice.1));
        }
    }
    let ids = edit(session, ID, "Split body", |doc| {
        let mut ids = Vec::with_capacity(anchors.len());
        for (index, (face, number)) in anchors.iter().enumerate() {
            let name = given.as_ref().and_then(|names| names.get(index));
            ids.push(insert_feature(
                doc,
                name.map(String::as_str),
                FeatureKind::Split(Split {
                    source,
                    face: face.clone(),
                    number: *number,
                }),
            )?);
        }
        Ok(ids)
    })?;
    let mut bodies = Vec::with_capacity(ids.len());
    for (id, (face, number)) in ids.iter().zip(&anchors) {
        let mut result = feature_result(session, *id)?;
        if let Some(map) = result.as_object_mut() {
            map.insert("face".into(), face.to_string().into());
            if let Some(number) = number {
                map.insert("number".into(), json!(number));
            }
        }
        bodies.push(result);
    }
    Ok(json!({ "source": source.to_string(), "bodies": bodies }))
}
