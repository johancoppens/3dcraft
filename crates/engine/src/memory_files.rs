//! Files a session keeps in memory, for a host without a file system to
//! reach (the browser): the host puts the bytes of a file the person chose
//! under a name ([`Session::put_file`]), a command reads it by that name as
//! it would read a path ([`crate::Access::Memory`]), and what a command
//! writes (`file.save`, `file.export`) is taken out again by the host
//! ([`Session::take_file`]) to hand to the person as a download.
//!
//! The same commands, with the same parameters and results, run on disk
//! and in memory: only where the bytes come from and go to differs
//! ([`Session::read_input`], [`Session::write_resolved`]).

use crate::workspace::Access;
use crate::{CommandError, Session};
use std::collections::BTreeMap;
use std::path::{Component, Path};

/// The most bytes one file in memory may have.
pub const MAX_MEMORY_FILE_BYTES: usize = 512 << 20;
/// The most bytes all files in memory may have together.
pub const MAX_MEMORY_BYTES: usize = 1 << 30;
/// The most files in memory at once.
pub const MAX_MEMORY_FILES: usize = 256;

/// The files, by their names with `/` between the parts. Shared, so that
/// a clone of the session (a command tried first, `commands_elsewhere.rs`)
/// copies no file.
#[derive(Clone, Debug, Default)]
pub(crate) struct MemoryFiles {
    files: BTreeMap<String, std::sync::Arc<Vec<u8>>>,
    total: usize,
}

/// The key of a resolved name: its parts joined by `/`, the same on every
/// platform.
fn key(resolved: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in resolved.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

impl MemoryFiles {
    fn get(&self, resolved: &Path) -> Option<&Vec<u8>> {
        self.files.get(&key(resolved)?).map(|bytes| &**bytes)
    }

    fn put(&mut self, resolved: &Path, bytes: Vec<u8>) -> Result<(), CommandError> {
        let shown = resolved.display();
        let name = key(resolved)
            .ok_or_else(|| CommandError::new(format!("`{shown}` is not a file name")))?;
        if bytes.len() > MAX_MEMORY_FILE_BYTES {
            return Err(CommandError::new(format!(
                "`{shown}` is {} bytes; a file in memory has at most {MAX_MEMORY_FILE_BYTES}",
                bytes.len()
            )));
        }
        let replaced = self.files.get(&name).map_or(0, |b| b.len());
        let total = self
            .total
            .saturating_sub(replaced)
            .saturating_add(bytes.len());
        if total > MAX_MEMORY_BYTES {
            return Err(CommandError::new(format!(
                "`{shown}` does not fit: the files in memory would have {total} bytes, at most {MAX_MEMORY_BYTES}"
            )));
        }
        if !self.files.contains_key(&name) && self.files.len() >= MAX_MEMORY_FILES {
            return Err(CommandError::new(format!(
                "`{shown}` does not fit: there are {MAX_MEMORY_FILES} files in memory already"
            )));
        }
        self.total = total;
        self.files.insert(name, std::sync::Arc::new(bytes));
        Ok(())
    }

    fn take(&mut self, resolved: &Path) -> Option<Vec<u8>> {
        let bytes = self.files.remove(&key(resolved)?)?;
        self.total = self.total.saturating_sub(bytes.len());
        Some(std::sync::Arc::try_unwrap(bytes).unwrap_or_else(|shared| (*shared).clone()))
    }
}

impl Session {
    /// Puts a file into the session's memory under `name` (a relative
    /// name, as a command's `path` gives it), replacing one of that name:
    /// what the host does with a file the person chose, before a command
    /// reads it with [`crate::Access::Memory`].
    pub fn put_file(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), CommandError> {
        let resolved = crate::Workspace::memory().resolve_read(name)?;
        self.memory.put(&resolved, bytes)
    }

    /// Takes the file of that name out of the session's memory: what a
    /// command wrote there, for the host to hand on.
    pub fn take_file(&mut self, name: &str) -> Option<Vec<u8>> {
        let resolved = crate::Workspace::memory().resolve_read(name).ok()?;
        self.memory.take(&resolved)
    }

    /// The names of the files in the session's memory, in order.
    pub fn file_names(&self) -> Vec<String> {
        self.memory.files.keys().cloned().collect()
    }

    /// Marks the open document `id` ([`crate::DocumentStatus::id`]) as not
    /// saved: what the host does when the file `file.save` wrote into
    /// memory did not reach the person's file (a refused or failed write
    /// in the browser), so the document keeps asking before it is closed.
    /// False when no open document has that id.
    pub fn mark_unsaved(&mut self, id: u64) -> bool {
        match self.documents.iter_mut().find(|open| open.id == id) {
            Some(open) => {
                open.saved_revision = None;
                true
            }
            None => false,
        }
    }

    /// True when the session's files are in its memory.
    pub(crate) fn writes_to_memory(&self) -> bool {
        self.workspace.write == Access::Memory
    }

    /// True when the session reads its files from its memory.
    pub(crate) fn reads_from_memory(&self) -> bool {
        self.workspace.read == Access::Memory
    }

    /// The bytes of the file at `resolved` (from
    /// [`crate::Workspace::resolve_read`]), at most `limit` of them: from
    /// memory or from disk, as the workspace reads. `path` is the path as
    /// the request gave it, for the error.
    pub(crate) fn read_input(
        &self,
        path: &str,
        resolved: &Path,
        limit: u64,
    ) -> Result<Vec<u8>, String> {
        let too_large = |size: u64| format!("`{path}` is {size} bytes; at most {limit} are read");
        if self.reads_from_memory() {
            let bytes = self
                .memory
                .get(resolved)
                .ok_or_else(|| format!("there is no file `{path}`"))?;
            let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            if size > limit {
                return Err(too_large(size));
            }
            return Ok(bytes.clone());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let size = std::fs::metadata(resolved)
                .map_err(|e| format!("cannot read `{path}`: {e}"))?
                .len();
            if size > limit {
                return Err(too_large(size));
            }
            std::fs::read(resolved).map_err(|e| format!("cannot read `{path}`: {e}"))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (resolved, too_large);
            Err(format!(
                "cannot read `{path}`: there is no file system here, only files in memory"
            ))
        }
    }

    /// Writes `bytes` to the file at `resolved` (from
    /// [`crate::Workspace::resolve_write`]): into memory, or to disk so
    /// that a failure leaves the old file whole, as the workspace writes.
    pub(crate) fn write_resolved(
        &mut self,
        resolved: &Path,
        bytes: Vec<u8>,
    ) -> Result<(), CommandError> {
        if self.writes_to_memory() {
            return self.memory.put(resolved, bytes);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::workspace::write_atomic(resolved, &bytes)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = bytes;
            Err(CommandError::new(format!(
                "cannot write `{}`: there is no file system here, only files in memory",
                resolved.display()
            )))
        }
    }

    /// Takes a file this session wrote to memory back out, without
    /// checking its name (a name [`Self::write_resolved`] took): to undo a
    /// write.
    pub(crate) fn forget_output(&mut self, resolved: &Path) -> Option<Vec<u8>> {
        self.memory.take(resolved)
    }

    /// What a file in memory holds now, by its resolved name.
    pub(crate) fn memory_file(&self, resolved: &Path) -> Option<&Vec<u8>> {
        self.memory.get(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_go_in_and_come_out_by_name() {
        let mut session = Session::new(crate::Workspace::memory());
        session.put_file("a.3dcraft", vec![1, 2, 3]).unwrap();
        session.put_file("./parts/b.step", vec![4]).unwrap();
        assert_eq!(session.file_names(), ["a.3dcraft", "parts/b.step"]);
        assert_eq!(session.take_file("parts/b.step"), Some(vec![4]));
        assert_eq!(session.take_file("parts/b.step"), None);
        // Replacing a file counts its bytes once.
        session.put_file("a.3dcraft", vec![0; 10]).unwrap();
        assert_eq!(session.memory.total, 10);
        assert!(session.put_file("../x", vec![]).is_err());
        assert!(session.put_file("/x", vec![]).is_err());
        assert!(session.put_file("", vec![]).is_err());
    }

    #[test]
    fn a_save_that_did_not_arrive_leaves_the_document_unsaved() {
        let mut session = Session::new(crate::Workspace::memory());
        session
            .execute("file.new", &serde_json::json!({"units": "mm"}))
            .unwrap();
        session
            .execute("feature.block", &serde_json::json!({"size": [1, 2, 3]}))
            .unwrap();
        session
            .execute("file.save", &serde_json::json!({"path": "a.3dcraft"}))
            .unwrap();
        let status = session.document_status().unwrap();
        assert!(!status.dirty && !status.close_asks);
        assert!(session.mark_unsaved(status.id));
        let after = session.document_status().unwrap();
        assert!(after.dirty && after.close_asks);
        assert_eq!(after.path.as_deref(), Some("a.3dcraft"));
        assert!(!session.mark_unsaved(status.id + 1000));
    }

    #[test]
    fn memory_is_bounded() {
        let mut files = MemoryFiles::default();
        let too_big = vec![0_u8; MAX_MEMORY_FILE_BYTES + 1];
        assert!(files.put(Path::new("big"), too_big).is_err());
        for n in 0..MAX_MEMORY_FILES {
            files.put(Path::new(&format!("f{n}")), vec![]).unwrap();
        }
        assert!(files.put(Path::new("one-more"), vec![]).is_err());
        // Replacing one that is there still works.
        files.put(Path::new("f0"), vec![1]).unwrap();
    }
}
