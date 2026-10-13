//! Commands about what a crashed 3DCraft left behind: which documents can
//! be recovered, opening one, discarding them. The design is in
//! `recovery.rs`.

use crate::commands::{Run, spec};
use crate::recovery::{Recoverable, RecoveryOverview};
use crate::{CommandSpec, Session};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("recovery.list", "List Recoverable Documents", "{}",
        "Looks in the recovery directory and lists the autosaved documents of launches that ended without a clean quit: id, name, original path, time of the autosave (seconds since the Unix epoch), size in bytes, revision, feature count, whether it can be opened and what is wrong with it. Also says whether autosave works: its directory, interval, last write and problem.", doc: false, recovery_list),
    spec!("recovery.open", "Recover Document", r#"{"id": string}"#,
        "Opens a recoverable document (an id from recovery.list) as an unsaved document that remembers its original path, and makes it the active one. Its recovery file stays until the document is saved or closed.", doc: false, recovery_open),
    spec!("recovery.discard", "Discard Recoverable Documents", r#"{"id": string, "all": boolean = false}"#,
        "Removes the recovery file of one recoverable document (an id from recovery.list), or of all of them with all: true. Give one of the two. Documents of a 3DCraft that is still running are never touched.", doc: false, recovery_discard),
];

fn recoverable_json(found: &Recoverable) -> Value {
    json!({
        "id": found.id,
        "name": found.name,
        "path": found.path,
        "time": found.time,
        "size": found.size,
        "revision": found.revision,
        "features": found.features,
        "pid": found.pid,
        "openable": found.openable,
        "problem": found.problem,
    })
}

impl RecoveryOverview {
    /// The overview as `recovery.list` and a shell's `ui.get` give it.
    pub fn to_json(&self) -> Value {
        overview_json(self)
    }
}

fn overview_json(overview: &RecoveryOverview) -> Value {
    json!({
        "enabled": overview.enabled,
        "directory": overview.directory,
        "interval": overview.interval_seconds,
        "autosaved": overview.last_autosave,
        "write_ms": overview.last_write_ms,
        "writes": overview.writes,
        "pending": overview.pending,
        "problem": overview.problem,
        "running": overview.running,
        "documents": overview.found.iter().map(recoverable_json).collect::<Vec<_>>(),
    })
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{Run, overview_json};
    use crate::commands::fail;
    use crate::params::Params;
    use crate::recovery::Recovered;
    use crate::{CommandError, MAX_DOCUMENTS, OpenDocument, Session};
    use serde_json::{Value, json};

    pub(super) fn list(session: &mut Session, params: &Value) -> Run {
        Params::new("recovery.list", params, &[])?;
        // A session without a recovery directory has nothing to list, and
        // says so in the same shape.
        if let Some(store) = &session.recovery.store {
            store.scan();
        }
        Ok(overview_json(&session.recovery_overview()))
    }

    pub(super) fn open(session: &mut Session, params: &Value) -> Run {
        let p = Params::new("recovery.open", params, &["id"])?;
        let id = p.required_str("id")?;
        if session.documents.len() >= MAX_DOCUMENTS {
            return fail(format!(
                "recovery.open: {MAX_DOCUMENTS} documents are open already; close one with file.close first"
            ));
        }
        let number = crate::next_document_id();
        let adopted = session
            .recovery_store("recovery.open")?
            .adopt(id, number)
            .map_err(|e| CommandError::new(format!("recovery.open: {e}")))?;
        let features = adopted.document.features.len();
        let mut open = OpenDocument::new(adopted.document, adopted.path.clone());
        open.id = number;
        // Nothing of it is on disk under its own name as it is now.
        open.saved_revision = None;
        open.disk = adopted.original;
        open.recovered = Some(Recovered {
            original: adopted.original,
        });
        let index = session.add_document(open)?;
        if let Some(store) = &session.recovery.store {
            store.scan();
        }
        Ok(json!({
            "document": index,
            "id": id,
            "name": adopted.name,
            "path": adopted.path,
            "features": features,
            "recovered": true,
            "warning": adopted.warning,
        }))
    }

    pub(super) fn discard(session: &mut Session, params: &Value) -> Run {
        let p = Params::new("recovery.discard", params, &["id", "all"])?;
        let id = p.str("id")?;
        let all = p.bool("all")?.unwrap_or(false);
        let wanted = match (id, all) {
            (Some(_), true) => {
                return fail("recovery.discard: give either `id` or `all`, not both");
            }
            (None, false) => {
                return fail("recovery.discard: give `id` (from recovery.list) or {\"all\": true}");
            }
            (id, _) => id,
        };
        let store = session.recovery_store("recovery.discard")?;
        let outcome = store.discard(wanted);
        let left = store.scan().len();
        let discarded = outcome.map_err(|e| CommandError::new(format!("recovery.discard: {e}")))?;
        Ok(json!({ "discarded": discarded, "left": left }))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn recovery_list(session: &mut Session, params: &Value) -> Run {
    native::list(session, params)
}

#[cfg(not(target_arch = "wasm32"))]
fn recovery_open(session: &mut Session, params: &Value) -> Run {
    native::open(session, params)
}

#[cfg(not(target_arch = "wasm32"))]
fn recovery_discard(session: &mut Session, params: &Value) -> Run {
    native::discard(session, params)
}

#[cfg(target_arch = "wasm32")]
fn no_files(command: &str) -> Run {
    crate::commands::fail(format!(
        "{command}: there is no recovery directory in a browser"
    ))
}

#[cfg(target_arch = "wasm32")]
fn recovery_list(_: &mut Session, _: &Value) -> Run {
    no_files("recovery.list")
}

#[cfg(target_arch = "wasm32")]
fn recovery_open(_: &mut Session, _: &Value) -> Run {
    no_files("recovery.open")
}

#[cfg(target_arch = "wasm32")]
fn recovery_discard(_: &mut Session, _: &Value) -> Run {
    no_files("recovery.discard")
}
