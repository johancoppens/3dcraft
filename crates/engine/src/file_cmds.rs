//! Commands about the set of open documents: which are open, and which one
//! the other commands act on.
//!
//! A document is referred to by its place in the list of open documents
//! (the `document` number that `file.new` and `file.open` return). Closing
//! a document moves the ones after it up by one; `file.list` gives the
//! numbers as they are now.

use crate::commands::{Run, fail, spec};
use crate::params::Params;
use crate::{CommandSpec, MAX_DOCUMENTS, Session};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("file.list", "List Open Documents", "{}",
        "Lists the open documents in order: number, path (null when never saved), whether it has unsaved changes, how many features and variables it has, its length unit, which one is active, recovered: true for one that recovery.open took from a recovery file and that was not saved since, jigInUse: true for a published Jig (.3dcraftjig) opened to be used, and title: the name a document without a file is shown under (an example the window's start screen opened, also once recovered), absent otherwise.", doc: false, file_list),
    spec!("file.activate", "Activate Document", r#"{"document": integer, "path": string}"#,
        "Makes another open document the active one, by its number from file.list or by the path it was opened from or saved to. Give one of the two.", doc: true, file_activate),
];

fn listing(session: &Session) -> Value {
    let documents: Vec<Value> = session
        .documents
        .iter()
        .enumerate()
        .map(|(index, open)| {
            let document = open.history.current();
            let mut row = json!({
                "document": index,
                "path": open.path,
                "dirty": open.dirty(),
                "active": index == session.active,
                "features": document.features.len(),
                "variables": document.variables.len(),
                "units": document.units.length.symbol(),
            });
            // Only a document taken from a recovery file and not saved
            // since says so (`recovery.open`).
            if open.recovered.is_some()
                && let Some(map) = row.as_object_mut()
            {
                map.insert("recovered".into(), true.into());
            }
            // A published Jig opened to be used says so (`jig_cmds.rs`).
            if open.jig_in_use
                && let Some(map) = row.as_object_mut()
            {
                map.insert("jigInUse".into(), true.into());
            }
            // An example the program carries, by its title while it has
            // no file (`open_bytes.rs`).
            if let (Some(title), Some(map)) = (&open.title, row.as_object_mut()) {
                map.insert("title".into(), title.clone().into());
            }
            row
        })
        .collect();
    let active = session.has_document().then_some(session.active);
    json!({ "active": active, "documents": documents })
}

fn file_list(session: &mut Session, params: &Value) -> Run {
    Params::new("file.list", params, &[])?;
    Ok(listing(session))
}

fn file_activate(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("file.activate", params, &["document", "path"])?;
    let count = session.documents.len();
    let index = match (p.has("document"), p.str("path")?) {
        (true, Some(_)) => {
            return fail("file.activate: give either `document` or `path`, not both");
        }
        (true, None) => {
            let Some(index) = p.integer("document", 0, MAX_DOCUMENTS as u64)? else {
                return Err(p.missing("document"));
            };
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            if index >= count {
                return fail(format!(
                    "file.activate: there is no document {index}; {count} are open (file.list)"
                ));
            }
            index
        }
        (false, Some(path)) => {
            // As given, or as a document opened by that path keeps it.
            let kept = session.workspace.resolve_read(path).ok().map(|resolved| {
                crate::workspace::kept_path(&session.workspace.read, path, &resolved)
            });
            let mut matches = session
                .documents
                .iter()
                .enumerate()
                .filter(|(_, open)| {
                    open.path.as_deref() == Some(path) || (kept.is_some() && open.path == kept)
                })
                .map(|(index, _)| index);
            match (matches.next(), matches.next()) {
                (Some(index), None) => index,
                (Some(_), Some(_)) => {
                    return fail(format!(
                        "file.activate: `{path}` is open more than once; use `document` with a number from file.list"
                    ));
                }
                (None, _) => {
                    return fail(format!(
                        "file.activate: no open document has the path `{path}` (file.list shows the paths)"
                    ));
                }
            }
        }
        (false, None) => {
            return fail("file.activate: give `document` (a number from file.list) or `path`");
        }
    };
    session.active = index;
    // An assembly shown again follows what changed of its sources since
    // (`asm_follow.rs`).
    let followed = session.follow_sources(Some(index), None);
    let mut result = listing(session);
    if let (false, Some(map)) = (followed.is_empty(), result.as_object_mut()) {
        map.insert("followed".into(), session.followed_json(&followed).into());
    }
    Ok(result)
}
