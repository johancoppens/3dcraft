//! Undo history for a document.
//!
//! Documents are small persistent values behind an `Arc`, so a step of
//! history is a snapshot: undo and redo swap pointers and can never leave a
//! document half-changed. (The plan describes reversible `Op`s; snapshots
//! give the same behaviour for far less code while documents are recipes
//! rather than geometry. Per-op records can come back if merging edits from
//! background jobs needs them.)
//!
//! This is session history, what the user did, not the design history (the
//! feature list), which is part of the document and saved with it.

use craft3d_doc::Document;
use std::sync::Arc;

/// Default number of undo steps kept.
pub const DEFAULT_LIMIT: usize = 200;

/// One undoable step.
#[derive(Clone, Debug)]
struct Step {
    label: String,
    before: Arc<Document>,
    after: Arc<Document>,
}

/// A document with its undo and redo stacks.
#[derive(Clone, Debug)]
pub struct History {
    current: Arc<Document>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    limit: usize,
    /// Counts every change of `current`, including undo and redo: two equal
    /// revisions mean the same document.
    revision: u64,
}

impl History {
    pub fn new(document: Document) -> Self {
        Self {
            current: Arc::new(document),
            undo: Vec::new(),
            redo: Vec::new(),
            limit: DEFAULT_LIMIT,
            revision: 0,
        }
    }

    /// Keeps at most `limit` undo steps (at least one), dropping the oldest.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.max(1);
        self
    }

    /// The document as it is now. Cheap to clone and safe to keep.
    pub fn current(&self) -> &Arc<Document> {
        &self.current
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Applies `edit` to a copy of the document. If it succeeds and changed
    /// something, that becomes one undo step labelled `label` and the redo
    /// stack is cleared. If it fails, nothing changes at all.
    pub fn apply<T, E>(
        &mut self,
        label: &str,
        edit: impl FnOnce(&mut Document) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut next = (*self.current).clone();
        let value = edit(&mut next)?;
        if next != *self.current {
            let after = Arc::new(next);
            self.undo.push(Step {
                label: label.to_string(),
                before: Arc::clone(&self.current),
                after: Arc::clone(&after),
            });
            if self.undo.len() > self.limit {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.current = after;
            self.revision += 1;
        }
        Ok(value)
    }

    /// Steps back; returns the label of the step undone, or `None` when
    /// there is nothing to undo.
    pub fn undo(&mut self) -> Option<String> {
        let step = self.undo.pop()?;
        self.current = Arc::clone(&step.before);
        self.revision += 1;
        let label = step.label.clone();
        self.redo.push(step);
        Some(label)
    }

    /// Steps forward again; returns the label of the step redone.
    pub fn redo(&mut self) -> Option<String> {
        let step = self.redo.pop()?;
        self.current = Arc::clone(&step.after);
        self.revision += 1;
        let label = step.label.clone();
        self.undo.push(step);
        Some(label)
    }

    /// Labels of the steps that can be undone, oldest first.
    pub fn undo_labels(&self) -> impl Iterator<Item = &str> {
        self.undo.iter().map(|s| s.label.as_str())
    }

    /// Labels of the steps that can be redone, next first.
    pub fn redo_labels(&self) -> impl Iterator<Item = &str> {
        self.redo.iter().rev().map(|s| s.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_doc::{DocError, Units};

    fn history() -> History {
        History::new(Document::new(Units::default()))
    }

    fn set(h: &mut History, name: &str, expr: &str) -> Result<(), DocError> {
        h.apply(&format!("Set {name}"), |doc| doc.set_variable(name, expr))
    }

    #[test]
    fn undo_and_redo_return_to_the_exact_previous_documents() {
        let mut h = history();
        let empty = Arc::clone(h.current());
        set(&mut h, "a", "1 mm").unwrap();
        let one = Arc::clone(h.current());
        set(&mut h, "b", "a * 2").unwrap();
        let two = Arc::clone(h.current());
        assert_eq!(h.undo_labels().collect::<Vec<_>>(), ["Set a", "Set b"]);

        assert_eq!(h.undo().as_deref(), Some("Set b"));
        assert!(
            Arc::ptr_eq(h.current(), &one),
            "undo restores the very same snapshot"
        );
        assert_eq!(h.undo().as_deref(), Some("Set a"));
        assert!(Arc::ptr_eq(h.current(), &empty));
        assert_eq!(h.undo(), None);
        assert_eq!(h.redo_labels().collect::<Vec<_>>(), ["Set a", "Set b"]);
        assert_eq!(h.redo().as_deref(), Some("Set a"));
        assert_eq!(h.redo().as_deref(), Some("Set b"));
        assert!(Arc::ptr_eq(h.current(), &two));
        assert_eq!(h.redo(), None);
    }

    #[test]
    fn a_new_edit_discards_the_redo_stack() {
        let mut h = history();
        set(&mut h, "a", "1 mm").unwrap();
        set(&mut h, "b", "2 mm").unwrap();
        h.undo();
        set(&mut h, "c", "3 mm").unwrap();
        assert_eq!(h.redo(), None);
        assert_eq!(
            h.current()
                .variables
                .iter()
                .map(|v| v.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "c"]
        );
    }

    #[test]
    fn a_failed_or_empty_edit_leaves_no_trace() {
        let mut h = history();
        set(&mut h, "a", "1 mm").unwrap();
        let (before, revision) = (Arc::clone(h.current()), h.revision());
        // Fails after having changed the copy.
        let result: Result<(), DocError> = h.apply("Half done", |doc| {
            doc.set_variable("b", "2 mm")?;
            doc.set_variable("sin", "3")
        });
        assert!(result.is_err());
        assert!(Arc::ptr_eq(h.current(), &before));
        // Succeeds without changing anything.
        set(&mut h, "a", "1 mm").unwrap();
        assert!(Arc::ptr_eq(h.current(), &before));
        assert_eq!(h.revision(), revision);
        assert_eq!(h.undo_labels().count(), 1);
    }

    #[test]
    fn the_oldest_steps_fall_off_at_the_limit() {
        let mut h = history().with_limit(3);
        for i in 0..10 {
            set(&mut h, "a", &format!("{i} mm")).unwrap();
        }
        assert_eq!(h.undo_labels().count(), 3);
        while h.undo().is_some() {}
        assert_eq!(h.current().variables[0].expr.source(), "6 mm");
        assert_eq!(History::new(Document::default()).with_limit(0).limit, 1);
    }

    #[test]
    fn every_change_of_the_document_bumps_the_revision() {
        let mut h = history();
        assert_eq!(h.revision(), 0);
        set(&mut h, "a", "1 mm").unwrap();
        h.undo();
        h.redo();
        assert_eq!(h.revision(), 3);
        h.redo();
        assert_eq!(h.revision(), 3);
    }
}
