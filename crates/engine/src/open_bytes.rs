//! Opening a document the program carries as bytes rather than as a file
//! on disk or in the session's memory: the start screen's published Jigs
//! and packed assemblies (docs/decisions.md, 2026-10-30). The document is
//! opened as `file.open` opens a file, but without a file of its own, so
//! it is never written back: a Jig opens in use on its parameters (Save is
//! Save As to a model), an assembly reads the sources packed in it.

use crate::{CommandError, OpenDocument, Session};
use serde_json::{Value, json};

impl Session {
    /// Opens the document in `bytes` as a new document without a file.
    /// `name` is the file name it was published under (`gear.3dcraftjig`,
    /// `walker.3dcraftasm`): its extension says what it holds, as for
    /// `file.open`, and a file that holds something else is refused.
    pub fn open_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<Value, CommandError> {
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
        let index = self.add_document(open)?;
        let mut result = json!({ "document": index, "name": name, "features": features });
        if self.document()?.history.current().is_assembly() {
            self.read_sources(false)?;
            if let Some(map) = result.as_object_mut() {
                map.insert("assembly".into(), true.into());
            }
        }
        if let Some(map) = result.as_object_mut() {
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
    use crate::{Session, Workspace};
    use serde_json::json;

    #[test]
    fn bytes_that_are_no_document_are_refused_and_nothing_opens() {
        let mut session = Session::new(Workspace::none());
        assert!(session.open_bytes("a.3dcraftjig", b"").is_err());
        assert!(session.open_bytes("a.3dcraftasm", b"not a zip").is_err());
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
}
