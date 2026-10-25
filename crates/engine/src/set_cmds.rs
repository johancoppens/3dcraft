//! Named sets (docs/decisions.md, 2026-10-21, B2): a query of
//! `select.query` kept in the document under a name and computed again
//! against every regeneration, so a set grows with the design as a
//! formula does. `set.define`, `set.edit`, `set.remove`, `set.list`; a set
//! is used as the filter `{set: name}` in `select.query` and as
//! `select.set {set}`. The design note is `docs/design/selection.md`.

use crate::commands::{Run, edit, fail, spec};
use crate::params::Params;
use crate::select_cmds::{Parser, run_query};
use crate::selection::feature_named;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::Document;
use serde_json::{Map, Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("set.define", "Define Set", r#"{"name": string, "kind": "body|face|edge|vertex|feature|sketch|datum" or [kind, ...] = every kind, "where": filter or [filter, ...]}"#,
        "Keeps a query under a name in the document: a named set. It is not a list of what fits now but the query itself, computed again at every regeneration, so \"the 6 mm holes\" grows with the design. kind and where are those of select.query; features in in, madeBy and copyOf are kept by id, so renaming a feature does not change the set. The query is checked first: an error says what is wrong and nothing is kept. A set may use other sets ({set: name}), but not itself. One undo step; saved with the document. Use it as {set: name} in select.query, or select.set {set: name}. Returns the set with how many items it holds now (count) and its summary.", doc: true, names: ["where"], set_define),
    spec!("set.edit", "Edit Set", r#"{"set": string, "kind": kind or [kind, ...] or null, "where": filter or null, "name": string}"#,
        "Changes a named set: kind or where replaces that part of its query (null takes it away), name renames it. One undo step. Returns the set as set.define does.", doc: true, names: ["where"], set_edit),
    spec!("set.remove", "Remove Set", r#"{"set": string}"#,
        "Takes a named set out of the document. One undo step. Returns its query.", doc: true, set_remove),
    spec!("set.list", "List Sets", "{}",
        "Lists the named sets of the active document in the order they were defined: name, kind, where, and count, how many items each holds now (computed against the document as it is now); a set whose query no longer reads (a feature it names was deleted) has error instead of count.", doc: true, set_list),
];

/// The query of `kind` and `where` as a set keeps it: checked by parsing
/// it, with the features of `in`, `madeBy` and `copyOf` by id.
fn stored(
    doc: &Document,
    name: &str,
    kind: Option<&Value>,
    filter: Option<&Value>,
) -> Result<String, String> {
    let mut query = Map::new();
    if let Some(kind) = kind.filter(|k| !k.is_null()) {
        query.insert("kind".into(), kind.clone());
    }
    if let Some(filter) = filter.filter(|f| !f.is_null()) {
        query.insert("where".into(), filter.clone());
    }
    // Checked first: what is rewritten after is bounded in depth.
    Parser::for_set(doc, name).query(&Value::Object(query.clone()), 0)?;
    if let Some(filter) = query.get_mut("where") {
        *filter = with_ids(doc, filter);
    }
    serde_json::to_string(&Value::Object(query)).map_err(|e| e.to_string())
}

/// `filter` with the feature of every `in`, `madeBy` and `copyOf` written
/// as its id (`F3`). Called on a filter only after it parsed, so its
/// nesting is bounded.
fn with_ids(doc: &Document, filter: &Value) -> Value {
    match filter {
        Value::Array(items) => Value::Array(items.iter().map(|f| with_ids(doc, f)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let feature = match value {
                        Value::String(text) => Some(text.clone()),
                        Value::Number(n) if n.is_u64() => Some(n.to_string()),
                        _ => None,
                    };
                    let value = match (key.as_str(), feature) {
                        ("in" | "madeBy" | "copyOf", Some(text)) => feature_named(doc, &text)
                            .map_or_else(|| value.clone(), |id| id.to_string().into()),
                        _ => with_ids(doc, value),
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// A set as the commands give it: name, kind and where, and what it
/// holds now (count and summary) or why it does not read.
fn set_json(session: &mut Session, name: &str) -> Run {
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let Some(set) = doc.set_named(name) else {
        return fail(format!("there is no set `{}`", cut(name)));
    };
    Ok(describe(doc, &regen, &set.name, &set.query))
}

fn describe(doc: &Document, regen: &craft3d_regen::Regen, name: &str, query: &str) -> Value {
    let value: Value = serde_json::from_str(query).unwrap_or(Value::Null);
    let mut out = json!({
        "name": name,
        "kind": value.get("kind").cloned().unwrap_or(Value::Null),
        "where": value.get("where").cloned().unwrap_or(Value::Null),
    });
    let held = Parser::new(doc)
        .query(&value, 0)
        .and_then(|query| run_query(doc, regen, query, 0, false));
    if let Some(map) = out.as_object_mut() {
        match held {
            Ok((_, count)) => {
                map.insert("count".into(), count.into());
            }
            Err(e) => {
                map.insert("error".into(), e.into());
            }
        }
    }
    out
}

fn cut(text: &str) -> String {
    text.chars().take(80).collect()
}

fn set_define(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "set.define";
    let p = Params::new(ID, params, &["name", "kind", "where"])?;
    let name = p.str("name")?.ok_or_else(|| p.missing("name"))?.to_string();
    let doc = session.document()?.history.current();
    if doc.set_named(&name).is_some() {
        return fail(format!(
            "{ID}: a set named `{}` already exists; change it with set.edit",
            cut(&name)
        ));
    }
    let query = stored(doc, &name, p.get("kind"), p.get("where"))
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let label = format!("Define set {}", cut(&name));
    edit(session, ID, &label, |doc| doc.define_set(&name, query))?;
    set_json(session, &name)
}

fn set_edit(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "set.edit";
    let p = Params::new(ID, params, &["set", "kind", "where", "name"])?;
    let name = p.str("set")?.ok_or_else(|| p.missing("set"))?.to_string();
    let rename = p.str("name")?.map(str::to_string);
    let doc = session.document()?.history.current();
    let Some(set) = doc.set_named(&name) else {
        return fail(format!("{ID}: there is no set `{}`", cut(&name)));
    };
    let old: Value = serde_json::from_str(&set.query).unwrap_or(Value::Null);
    let part = |key: &str| match p.get(key) {
        Some(given) => Some(given.clone()),
        None => old.get(key).cloned(),
    };
    let (kind, filter) = (part("kind"), part("where"));
    let query = stored(doc, &name, kind.as_ref(), filter.as_ref())
        .map_err(|e| CommandError::new(format!("{ID}: {e}")))?;
    let label = format!("Edit set {}", cut(&name));
    let to = rename.clone().unwrap_or_else(|| name.clone());
    // The sets that use it follow the new name, as formulas follow a
    // renamed variable: a set defined later under the old name does not
    // change what they hold.
    let users: Vec<(String, String)> = if to == name {
        Vec::new()
    } else {
        doc.sets
            .iter()
            .filter(|other| other.name != name)
            .filter_map(|other| {
                let value: Value = serde_json::from_str(&other.query).ok()?;
                uses(&value, &name).then(|| {
                    let renamed = renamed(&value, &name, &to);
                    serde_json::to_string(&renamed)
                        .ok()
                        .map(|text| (other.name.clone(), text))
                })?
            })
            .collect()
    };
    edit(session, ID, &label, |doc| {
        doc.define_set(&name, query)?;
        doc.rename_set(&name, &to)?;
        for (other, text) in users {
            doc.define_set(&other, text)?;
        }
        Ok(())
    })?;
    set_json(session, &to)
}

/// Whether the filter `value` uses the set `name` (`{set: name}`
/// anywhere). Stored queries were read from JSON text, so their nesting
/// is bounded.
fn uses(value: &Value, name: &str) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|item| uses(item, name)),
        Value::Object(map) => map.iter().any(|(key, inner)| {
            (key == "set" && inner.as_str() == Some(name)) || uses(inner, name)
        }),
        _ => false,
    }
}

/// `value` with every `{set: from}` made `{set: to}`.
fn renamed(value: &Value, from: &str, to: &str) -> Value {
    match value {
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| renamed(item, from, to)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, inner)| {
                    let inner = if key == "set" && inner.as_str() == Some(from) {
                        Value::String(to.to_string())
                    } else {
                        renamed(inner, from, to)
                    };
                    (key.clone(), inner)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn set_remove(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "set.remove";
    let p = Params::new(ID, params, &["set"])?;
    let name = p.str("set")?.ok_or_else(|| p.missing("set"))?.to_string();
    // A set another one uses is not taken away from under it.
    let users: Vec<String> = session
        .document()?
        .history
        .current()
        .sets
        .iter()
        .filter(|other| other.name != name)
        .filter(|other| {
            serde_json::from_str::<Value>(&other.query).is_ok_and(|value| uses(&value, &name))
        })
        .map(|other| format!("`{}`", cut(&other.name)))
        .collect();
    if !users.is_empty() {
        return fail(format!(
            "{ID}: the set `{}` is used by {}; edit or remove those first",
            cut(&name),
            users.join(", ")
        ));
    }
    let label = format!("Remove set {}", cut(&name));
    let removed = edit(session, ID, &label, |doc| doc.remove_set(&name))?;
    let query: Value = serde_json::from_str(&removed.query).unwrap_or(Value::Null);
    Ok(json!({
        "removed": removed.name,
        "kind": query.get("kind").cloned().unwrap_or(Value::Null),
        "where": query.get("where").cloned().unwrap_or(Value::Null),
    }))
}

fn set_list(session: &mut Session, params: &Value) -> Run {
    Params::new("set.list", params, &[])?;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let sets: Vec<Value> = doc
        .sets
        .iter()
        .map(|set| describe(doc, &regen, &set.name, &set.query))
        .collect();
    Ok(json!({ "sets": sets, "count": doc.sets.len() }))
}
