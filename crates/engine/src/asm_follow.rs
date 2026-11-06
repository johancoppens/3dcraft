//! An open assembly follows its source files: each one is read again when
//! it changed, after a save in this session (`file.save`), when its tab is
//! made active (`file.activate`) and when the host asks
//! (`assembly.follow`: the window when it gets the focus back). What
//! looks unchanged (the same size and modification time) is not read
//! again; what was read again and holds the same bytes changes nothing,
//! so nothing is regenerated. A source that changed but cannot be read
//! again keeps the last bytes read, and its instances their bodies
//! (docs/design/assemblies.md › *Following the sources*).

use crate::asm;
use crate::commands::{Run, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Blob, Document, Source};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) const SPECS: &[CommandSpec] = &[spec!(
    "assembly.follow",
    "Follow Sources",
    r#"{}"#,
    "Checks the source files of every open assembly (each tab, with the sources of its subassemblies) and reads again those that changed since they were read: a file whose size and modification time are as they were is not read, one read again with the same bytes changes nothing. An instance follows its file unless it is pinned; only a changed source is regenerated, when its assembly is shown. A source that changed and cannot be read again (gone, unreadable, outside the roots) keeps the last version read, and its instances their bodies; the Instances panel marks them, with the reason. file.save does the same for the other open documents after it wrote its file, file.activate for the document it activates, and the window when it gets the focus back. Not an undo step. Returns followed: per assembly whose sources changed or failed, {document, path, changed: the source files read again with other bytes, failed: [{path, error}]}, and failed: how many sources could not be read again.",
    doc: false,
    assembly_follow
)];

/// Which source files [`Session::read_sources_of`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reread {
    /// Only those not read yet.
    Missing,
    /// All of them.
    All,
    /// Those not read yet and those that look changed on disk (another
    /// size or modification time; in memory, every one).
    Stale,
}

/// What reading an assembly's sources again found.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SourcesRead {
    /// The source files whose bytes changed.
    pub changed: Vec<String>,
    /// The sources that changed but could not be read again, with why;
    /// they keep the last bytes read.
    pub failed: Vec<(String, String)>,
}

impl Session {
    /// Reads the source files of the assembly open as document `index`
    /// as `mode` says, with those of its subassemblies, to
    /// [`asm::MAX_DEPTH`] levels, each file once; `saved` (a file just
    /// written) is read again whatever its stamp says. Nothing for a
    /// part. The placement is redone only when what was read changed.
    pub(crate) fn read_sources_of(
        &mut self,
        index: usize,
        mode: Reread,
        saved: Option<&std::path::Path>,
    ) -> Result<SourcesRead, CommandError> {
        let base = self.asm_base_of(index)?;
        let open = self
            .documents
            .get(index)
            .ok_or_else(|| CommandError::new("no document is open"))?;
        let doc = Arc::clone(open.history.current());
        if !doc.is_assembly() {
            return Ok(SourcesRead::default());
        }
        let mut files = open.asm.files.clone();
        let mut stamps = open.asm.stamps.clone();
        let mut unfollowed = open.asm.unfollowed.clone();
        // The sources parsed for the last placement, so a file that did
        // not change is not parsed again to see whether it is a
        // subassembly (4 ms for the pi case's two parts, 2026-11-06).
        let parsed = open.asm.parsed.clone();
        let base_read = base.clone();
        let mut out = SourcesRead::default();
        let mut seen = BTreeSet::new();
        // (document, the folder its sources are relative to, depth)
        let mut stack: Vec<(Arc<Document>, String, usize)> = vec![(doc, base, 0)];
        while let Some((doc, base, depth)) = stack.pop() {
            let Some(assembly) = &doc.assembly else {
                continue;
            };
            if depth > asm::MAX_DEPTH {
                continue;
            }
            for instance in &assembly.instances {
                let (blob, sub_base) = match &instance.source {
                    Source::Packed { hash } => match assembly.packed(hash) {
                        Some(blob) => (blob.clone(), base.clone()),
                        None => continue,
                    },
                    Source::File { path, .. } => {
                        let Ok(full) = asm::join(&base, path) else {
                            continue;
                        };
                        if !seen.insert(full.clone()) || seen.len() > asm::MAX_SOURCE_FILES {
                            continue;
                        }
                        let read = self.read_one(
                            &full,
                            mode,
                            saved,
                            (&mut files, &mut stamps, &mut unfollowed),
                            &mut out,
                        );
                        match read {
                            Some(blob) => (blob, asm::dir_of(&full)),
                            None => continue,
                        }
                    }
                };
                // A subassembly's own sources, below it.
                let sub = match parsed.get(&blob.sha256()) {
                    Some(known) => known.as_ref().ok().map(Arc::clone),
                    None => craft3d_format::from_bytes(blob.bytes()).ok().map(Arc::new),
                };
                if let Some(sub) = sub
                    && sub.is_assembly()
                {
                    stack.push((sub, sub_base, depth.saturating_add(1)));
                }
            }
        }
        let open = self
            .documents
            .get_mut(index)
            .ok_or_else(|| CommandError::new("no document is open"))?;
        open.asm.stamps = stamps;
        let mut again = false;
        // Without a file, its sources are placed from where they were
        // read until it is saved.
        if open.path.is_none() && open.asm.unsaved_base != base_read {
            open.asm.unsaved_base = base_read;
            again = true;
        }
        if open.asm.unfollowed != unfollowed {
            open.asm.unfollowed = unfollowed;
            again = true;
        }
        if open.asm.files != files {
            open.asm.files = files;
            open.assertions = crate::assert_check::Checks::default();
            again = true;
        }
        if again {
            open.asm.invalidate();
        }
        Ok(out)
    }

    /// Reads one source file `full` for [`Session::read_sources_of`]:
    /// the bytes its instances are placed from, `None` when there are
    /// none.
    fn read_one(
        &self,
        full: &str,
        mode: Reread,
        saved: Option<&std::path::Path>,
        (files, stamps, unfollowed): Kept<'_>,
        out: &mut SourcesRead,
    ) -> Option<Blob> {
        let before = files.get(full).cloned();
        let stamp = self.source_stamp(full);
        let look = match (&before, mode) {
            (None, _) | (Some(_), Reread::All) => true,
            (Some(_), Reread::Missing) => false,
            (Some(Err(_)), Reread::Stale) => true,
            // The same size and modification time: not read again, unless
            // it is the file just saved (a file system whose times are
            // coarse may give it the same).
            (Some(Ok(_)), Reread::Stale) => {
                stamp.is_none()
                    || stamps.get(full) != stamp.as_ref()
                    || saved.is_some_and(|s| self.is_file(full, s))
            }
        };
        if !look {
            return before.and_then(Result::ok);
        }
        let now = self.read_source_file(full);
        match (before, now) {
            // Changed and not read: the last bytes read stay, and it is
            // read again at the next look (its stamp is not taken).
            (Some(Ok(old)), Err(why)) => {
                out.failed.push((full.to_string(), why.clone()));
                unfollowed.insert(full.to_string(), why);
                Some(old)
            }
            (before, now) => {
                let differs = match (&before, &now) {
                    (Some(Ok(a)), Ok(b)) => a.sha256() != b.sha256(),
                    (Some(_), _) => true,
                    (None, _) => false,
                };
                if differs {
                    out.changed.push(full.to_string());
                }
                unfollowed.remove(full);
                match (&now, stamp) {
                    (Ok(_), Some(stamp)) => {
                        stamps.insert(full.to_string(), stamp);
                    }
                    _ => {
                        stamps.remove(full);
                    }
                }
                files.insert(full.to_string(), now.clone());
                now.ok()
            }
        }
    }

    /// What a source file looks like on disk now; `None` in memory, where
    /// there is no file system, and for a path the roots refuse.
    fn source_stamp(&self, full: &str) -> Option<crate::recovery::FileStamp> {
        if self.reads_from_memory() {
            return None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let resolved = self.workspace.resolve_read(full).ok()?;
            crate::recovery::FileStamp::of(&resolved)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = full;
            None
        }
    }

    /// The source file `full` is the file at `written` (as the workspace
    /// resolved it to write it).
    fn is_file(&self, full: &str, written: &std::path::Path) -> bool {
        let Ok(resolved) = self.workspace.resolve_read(full) else {
            return false;
        };
        if resolved == written {
            return true;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if !self.reads_from_memory()
            && let (Ok(a), Ok(b)) = (
                std::fs::canonicalize(&resolved),
                std::fs::canonicalize(written),
            )
        {
            return a == b;
        }
        false
    }

    /// Reads again what changed of the sources of the open assemblies
    /// (all of them, or only document `only`), as `assembly.follow` does;
    /// `saved` is read again whatever its stamp says. Returns, per
    /// assembly whose sources changed or failed, its index and what was
    /// found. An assembly whose sources cannot be gone through (no base)
    /// is left as it is.
    pub(crate) fn follow_sources(
        &mut self,
        only: Option<usize>,
        saved: Option<&std::path::Path>,
    ) -> Vec<(usize, SourcesRead)> {
        let mut out = Vec::new();
        for index in 0..self.documents.len() {
            if only.is_some_and(|only| only != index) {
                continue;
            }
            let is_assembly = self
                .documents
                .get(index)
                .is_some_and(|open| open.history.current().is_assembly());
            if !is_assembly {
                continue;
            }
            if let Ok(read) = self.read_sources_of(index, Reread::Stale, saved)
                && (!read.changed.is_empty() || !read.failed.is_empty())
            {
                out.push((index, read));
            }
        }
        out
    }

    /// [`Session::follow_sources`] as an answer lists it.
    pub(crate) fn followed_json(&self, followed: &[(usize, SourcesRead)]) -> Vec<Value> {
        followed
            .iter()
            .map(|(index, read)| {
                json!({
                    "document": index,
                    "path": self.documents.get(*index).and_then(|o| o.path.clone()),
                    "changed": read.changed,
                    "failed": read.failed.iter().map(|(path, error)| json!({"path": path, "error": error})).collect::<Vec<_>>(),
                })
            })
            .collect()
    }
}

/// The parts of an assembly's `AsmState` a read updates, as they are worked on.
type Kept<'a> = (
    &'a mut BTreeMap<String, Result<Blob, String>>,
    &'a mut BTreeMap<String, crate::recovery::FileStamp>,
    &'a mut BTreeMap<String, String>,
);

fn assembly_follow(session: &mut Session, params: &Value) -> Run {
    Params::new("assembly.follow", params, &[])?;
    let followed = session.follow_sources(None, None);
    let failed: usize = followed.iter().map(|(_, r)| r.failed.len()).sum();
    Ok(json!({
        "followed": session.followed_json(&followed),
        "failed": failed,
    }))
}
