//! Opening a document the program carries as bytes rather than as a file
//! on disk or in the session's memory: the start screen's examples, Jigs
//! and assemblies (docs/decisions.md, 2026-10-30 and 2026-11-10). The
//! document is opened as `file.open` opens a file, but without a file of
//! its own, so it is never written back: a Jig opens in use on its
//! parameters (Save is Save As to a model), an assembly reads the sources
//! packed in it.
//!
//! **Clean until changed** (docs/decisions.md, 2026-11-10). An example
//! only looked at is not unsaved work: what was opened counts as saved,
//! as a file opened from disk does (`OpenDocument::mark_saved`), with no
//! path. So it has no unsaved mark, closing it or quitting asks nothing,
//! and no recovery file is written for it, until the first edit; after
//! that it is an unsaved document like any other, and an undo back to
//! the state it opened in is clean again, as for a saved file. Save still
//! asks where (`file.save` without a path refuses), so nothing is ever
//! written to the bytes. A Jig in use is never dirty (`OpenDocument::dirty`),
//! and its close asks once its parameters changed since it was opened:
//! that is measured against the same saved state, so a Jig the program
//! carries behaves as one opened from its `.3dcraftjig` file.
//!
//! **Its name.** An example is shown under its title
//! ([`Session::open_bytes_titled`]): the tab, the window's title and a
//! recovery file name it so, where a document without a file is
//! "Untitled". The title is the session's, not the document's: no file
//! holds it, and `craft3d-format` does not know it.

use crate::{CommandError, OpenDocument, Session};
use serde_json::{Value, json};

/// Longest title kept, in characters.
pub(crate) const MAX_TITLE_CHARS: usize = 120;

/// `text` as a title: on one line (control characters dropped), trimmed,
/// at most [`MAX_TITLE_CHARS`]; `None` when nothing is left.
pub(crate) fn clean_title(text: &str) -> Option<String> {
    let title: String = text
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TITLE_CHARS)
        .collect::<String>()
        .trim()
        .to_string();
    (!title.is_empty()).then_some(title)
}

impl Session {
    /// Opens the document in `bytes` as a new document without a file,
    /// clean until it is changed. `name` is the file name it was
    /// published under (`gear.3dcraftjig`, `walker.3dcraftasm`): its
    /// extension says what it holds, as for `file.open`, and a file that
    /// holds something else is refused.
    pub fn open_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<Value, CommandError> {
        self.open_bytes_titled(name, None, bytes)
    }

    /// [`Session::open_bytes`], shown under `title` while it has no file
    /// (the example's title; [`crate::DocumentStatus::title`]).
    pub fn open_bytes_titled(
        &mut self,
        name: &str,
        title: Option<&str>,
        bytes: &[u8],
    ) -> Result<Value, CommandError> {
        let (document, read) = craft3d_format::from_bytes_noted(bytes)
            .map_err(|e| CommandError::new(format!("cannot open `{name}`: {e}")))?;
        let asm_name = craft3d_format::is_assembly_path(name);
        if document.is_assembly() != asm_name {
            return Err(CommandError::new(if asm_name {
                format!("cannot open `{name}`: it is named as an assembly and holds a part")
            } else {
                format!(
                    "cannot open `{name}`: it holds an assembly, which only a .3dcraftasm name may"
                )
            }));
        }
        let features = document.features.len();
        let jig = craft3d_format::is_jig_path(name);
        let mut open = OpenDocument::new(document, None);
        open.jig_in_use = jig;
        // Clean until changed: the bytes are the saved state (module
        // docs).
        open.mark_saved();
        open.title = title.and_then(clean_title);
        let shown = open.title.clone();
        let index = self.add_document(open)?;
        let mut result = json!({ "document": index, "name": name, "features": features });
        if self.document()?.history.current().is_assembly() {
            // The new document is closed again when its sources cannot be
            // read: nothing half-opened is left.
            if let Err(e) = self.read_sources(false) {
                let _ = self.execute("file.close", &json!({ "document": index, "discard": true }));
                return Err(e);
            }
            if let Some(map) = result.as_object_mut() {
                map.insert("assembly".into(), true.into());
            }
        }
        if let Some(map) = result.as_object_mut() {
            if let Some(title) = shown {
                map.insert("title".into(), title.into());
            }
            if jig {
                map.insert("jigInUse".into(), true.into());
            }
            if !read.is_empty() {
                map.insert("warnings".into(), read.into());
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::clean_title;
    use crate::{Session, Workspace};
    use serde_json::json;

    #[test]
    fn bytes_that_are_no_document_are_refused_and_nothing_opens() {
        let mut session = Session::new(Workspace::none());
        assert!(session.open_bytes("a.3dcraftjig", b"").is_err());
        assert!(session.open_bytes("a.3dcraftasm", b"not a zip").is_err());
        assert!(
            session
                .open_bytes_titled("a.3dcraft", Some("A"), b"PK")
                .is_err()
        );
        assert!(session.document_statuses().is_empty());
    }

    #[test]
    fn a_part_is_refused_under_an_assembly_name_and_opens_as_a_jig() {
        let mut session = Session::new(Workspace::memory());
        session
            .execute("file.new", &json!({"units": "mm"}))
            .unwrap();
        session
            .execute("feature.block", &json!({"size": [10, 20, 30]}))
            .unwrap();
        session
            .execute("file.save", &json!({"path": "part.3dcraft"}))
            .unwrap();
        let bytes = session.take_file("part.3dcraft").unwrap();
        assert!(session.open_bytes("part.3dcraftasm", &bytes).is_err());
        let opened = session.open_bytes("part.3dcraftjig", &bytes).unwrap();
        assert_eq!(opened["jigInUse"], true, "{opened}");
        assert_eq!(opened["features"], 1);
        let statuses = session.document_statuses();
        assert_eq!(statuses.len(), 2);
        // No file of its own: Save must be told where.
        assert!(session.execute("file.save", &json!({})).is_err());
    }

    #[test]
    fn a_title_is_one_trimmed_line_of_at_most_120_characters() {
        assert_eq!(clean_title("  Moka pot\n").as_deref(), Some("Moka pot"));
        assert_eq!(clean_title("a\u{0}b\tc").as_deref(), Some("abc"));
        assert_eq!(clean_title(" \n\t"), None);
        assert_eq!(clean_title(""), None);
        assert_eq!(
            clean_title(&"x".repeat(500)).map(|t| t.chars().count()),
            Some(120)
        );
    }
}
