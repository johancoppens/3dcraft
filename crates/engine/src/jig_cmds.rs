//! The two roles of a Jig (docs/decisions.md, 2026-10-23): it is
//! *designed* in an ordinary `.3dcraft` document and published with
//! `jig.export` as a `.3dcraftjig` file (the same format, with its
//! thumbnail and info); it is *used* by opening that file, which opens it
//! read-only on its parameters: only `param.set` changes it, it is never
//! saved over its file (`file.save` saves it as a `.3dcraft` model) and
//! its parameter changes are no unsaved changes. `jig.edit` makes a new
//! untitled model of it to edit. Whether a document is a Jig in use is
//! derived from how it was opened (`OpenDocument::jig_in_use`), not stored
//! in the file.

use crate::commands::{Run, fail, save_to_disk, spec, thumbnail_png};
use crate::params::Params;
use crate::{CommandError, CommandSpec, OpenDocument, Session};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("jig.export", "Export as Jig", r#"{"path": string}"#,
        "Publishes the active document as a Jig: writes it to path, which must end in .3dcraftjig, under the write root, with its document info and a thumbnail (thumb.png, as file.save draws it; thumbnailError says why there is none, and the file is written all the same). Refused for a document without a public parameter (variable.publish). The document itself is unchanged: its own path and its unsaved changes stay as they are. Opening the .3dcraftjig file (file.open) opens it as a Jig in use: read-only but for its parameters.", doc: true, jig_export),
    spec!("jig.edit", "Open for Editing", "{}",
        "Makes a new untitled document of the active one, as it is now (its parameters as set), and makes it active: the way to edit a Jig in use (opened from a .3dcraftjig file), which changes only through param.set. The original stays open, unchanged. Returns the new document's number.", doc: true, jig_edit),
];

/// What an edit of a Jig in use is refused with.
pub(crate) fn in_use_refusal(command: &str) -> CommandError {
    CommandError::new(format!(
        "{command}: this is a Jig: only its parameters change here (param.set); save it as a model to edit it (file.save with a path ending in .3dcraft, or jig.edit for a new untitled model)"
    ))
}

fn jig_export(session: &mut Session, params: &Value) -> Run {
    let command = "jig.export";
    let p = Params::new(command, params, &["path"])?;
    let path = p.required_str("path")?.to_string();
    if session.document()?.jig_in_use {
        return fail(format!(
            "{command}: this is a Jig in use, opened from its .3dcraftjig file; open it for editing first (jig.edit, File, Open for editing) and export that model"
        ));
    }
    if !session.document()?.history.current().is_jig() {
        return fail(format!(
            "{command}: this document has no public parameter, so it is no Jig; publish the variables that drive it first (variable.publish)"
        ));
    }
    let resolved = session.workspace.resolve_write(&path)?;
    if !craft3d_format::is_jig_path(&path)
        || !craft3d_format::is_jig_path(&resolved.to_string_lossy())
    {
        return fail(format!(
            "{command}: a published Jig's file ends in .{}; give such a `path` (a .3dcraft model is saved with file.save)",
            craft3d_format::JIG_EXTENSION
        ));
    }
    let access = session.workspace.write.clone();
    let kept = crate::workspace::kept_path(&access, &path, &resolved);
    let absolute = std::path::absolute(&resolved)
        .ok()
        .and_then(|a| a.to_str().map(str::to_string));
    // Never over a Jig that is open to be used.
    if session.documents.iter().any(|open| {
        open.jig_in_use
            && (open.path.as_deref() == Some(kept.as_str())
                || (open.file.is_some() && open.file == absolute))
    }) {
        return fail(format!(
            "{command}: `{path}` is open as a Jig in use; close it first, or export under another name"
        ));
    }
    let in_memory = session.writes_to_memory();
    // The Jig is published whole: a rollback marker is the designer's
    // view of the history, not part of what is handed out.
    let current = session.document()?.history.current();
    let rolled_back = current.rollback.is_some();
    let document = if rolled_back {
        let mut whole = (**current).clone();
        whole.rollback = None;
        std::sync::Arc::new(whole)
    } else {
        std::sync::Arc::clone(current)
    };
    let (png, thumbnail_error) = match thumbnail_of(session, &document, rolled_back) {
        Ok(png) => (Some(png), None),
        Err(e) => (None, Some(e.message().to_string())),
    };
    let cannot = |e: String| CommandError::new(format!("{command}: cannot write `{path}`: {e}"));
    if in_memory {
        let bytes =
            craft3d_format::to_bytes_with_thumbnail(&document, png.as_deref().map(Vec::as_slice))
                .map_err(|e| cannot(e.to_string()))?;
        session
            .write_resolved(&resolved, bytes)
            .map_err(|e| cannot(e.message().to_string()))?;
    } else {
        save_to_disk(&resolved, &document, png.as_deref().map(Vec::as_slice)).map_err(cannot)?;
    }
    let mut result = json!({ "path": kept, "thumbnail": png.is_some() });
    if let Some(map) = result.as_object_mut() {
        if let Some(error) = thumbnail_error {
            map.insert("thumbnailError".into(), error.into());
        }
        if rolled_back {
            map.insert("rollbackCleared".into(), true.into());
        }
    }
    Ok(result)
}

/// The thumbnail of `document`: the active document's own when it is
/// that; else drawn from it as a document of its own for a moment (the
/// active one with its rollback marker taken away), the active document
/// put back whatever happens.
fn thumbnail_of(
    session: &mut Session,
    document: &std::sync::Arc<craft3d_doc::Document>,
    differs: bool,
) -> Result<std::sync::Arc<Vec<u8>>, CommandError> {
    if !differs {
        return thumbnail_png(session);
    }
    let active = session.active;
    let mut copy = OpenDocument::new((**document).clone(), None);
    copy.cache = session.document()?.cache.clone();
    copy.work = std::sync::Arc::clone(&session.work);
    session.documents.push(copy);
    session.active = session.documents.len().saturating_sub(1);
    let drawn = thumbnail_png(session);
    session.documents.pop();
    session.active = active;
    drawn
}

fn jig_edit(session: &mut Session, params: &Value) -> Run {
    Params::new("jig.edit", params, &[])?;
    let document = (**session.document()?.history.current()).clone();
    let was_jig = session.document()?.jig_in_use;
    // What the source's regeneration found serves the copy: nothing is
    // built again.
    let mut copy = OpenDocument::new(document, None);
    copy.cache = session.document()?.cache.clone();
    let index = session.add_document(copy)?;
    Ok(json!({ "document": index, "fromJigInUse": was_jig }))
}
