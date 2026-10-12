//! The engine: the façade every frontend talks to (docs/architecture.md §8).
//!
//! A [`Session`] holds open documents and runs commands. Every action is a
//! command with a stable id and JSON parameters, so the UI, the CLI, the
//! control channel and the MCP server all do exactly the same things
//! (AGENTS.md rule 1).
//!
//! Deviations from the plan, for now: commands run synchronously on the
//! caller's thread (no job pool, no events), and regeneration is redone in
//! full after every edit, cached per document revision.

mod base64;
mod commands;
mod params;
mod sketch_cmds;
mod workspace;

pub use workspace::{Access, Workspace};

use craft3d_doc::Document;
use craft3d_ops::History;
use craft3d_regen::Regen;
use serde_json::Value;
use std::fmt;
use std::sync::Arc;

/// Why a command did not run. The message is written for the person or
/// agent that sent the command: it names the command and the parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandError(String);

impl CommandError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CommandError {}

/// One entry of the command registry.
#[derive(Clone, Copy)]
pub struct CommandSpec {
    /// Stable id, such as `feature.block`.
    pub id: &'static str,
    pub label: &'static str,
    /// The parameters, in the notation agents read:
    /// `{"name": type = default, ...}`.
    pub params: &'static str,
    /// What it does and returns, in a sentence.
    pub summary: &'static str,
    pub needs_document: bool,
    pub(crate) run: fn(&mut Session, &Value) -> Result<Value, CommandError>,
}

/// An open document with its undo history and what was derived from it.
pub(crate) struct OpenDocument {
    pub history: History,
    /// The path it was opened from or last saved to, as the request gave it.
    pub path: Option<String>,
    /// The revision that is on disk.
    pub saved_revision: Option<u64>,
    /// Regeneration of `history.revision()`.
    regen: Option<(u64, Arc<Regen>)>,
}

impl OpenDocument {
    pub(crate) fn new(document: Document, path: Option<String>) -> Self {
        let history = History::new(document);
        let saved_revision = path.is_some().then(|| history.revision());
        Self {
            history,
            path,
            saved_revision,
            regen: None,
        }
    }

    pub(crate) fn dirty(&self) -> bool {
        self.saved_revision != Some(self.history.revision())
    }

    /// The bodies and statuses of the current document.
    pub(crate) fn regen(&mut self) -> Arc<Regen> {
        let revision = self.history.revision();
        match &self.regen {
            Some((cached, regen)) if *cached == revision => Arc::clone(regen),
            _ => {
                let regen = Arc::new(craft3d_regen::regenerate(self.history.current()));
                self.regen = Some((revision, Arc::clone(&regen)));
                regen
            }
        }
    }
}

/// Open documents plus the rules for touching files.
pub struct Session {
    pub(crate) documents: Vec<OpenDocument>,
    pub(crate) active: usize,
    pub(crate) workspace: Workspace,
}

/// Most documents open at once.
pub const MAX_DOCUMENTS: usize = 64;

impl Session {
    pub fn new(workspace: Workspace) -> Self {
        Self {
            documents: Vec::new(),
            active: 0,
            workspace,
        }
    }

    /// The command registry, in a fixed order.
    pub fn commands() -> &'static [CommandSpec] {
        static ALL: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
        ALL.get_or_init(|| [commands::SPECS, sketch_cmds::SPECS].concat())
    }

    pub fn command(id: &str) -> Option<&'static CommandSpec> {
        Self::commands().iter().find(|c| c.id == id)
    }

    pub fn has_document(&self) -> bool {
        self.documents.get(self.active).is_some()
    }

    /// Why `spec` cannot run right now, if it cannot.
    pub fn disabled_reason(&self, spec: &CommandSpec) -> Option<&'static str> {
        (spec.needs_document && !self.has_document())
            .then_some("no document is open; run file.new or file.open first")
    }

    /// Runs a command by id. `params` is a JSON object, or null for none.
    ///
    /// Never panics: a panic inside a command is caught and reported as an
    /// error (AGENTS.md, last-resort guard), and the documents stay as they
    /// were before the command, because edits are applied atomically.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        let Some(spec) = Self::command(id) else {
            let hint = commands::closest(id)
                .map(|c| format!("; did you mean `{c}`?"))
                .unwrap_or_default();
            return Err(CommandError::new(format!("unknown command `{id}`{hint}")));
        };
        if let Some(reason) = self.disabled_reason(spec) {
            return Err(CommandError::new(format!("{id}: {reason}")));
        }
        let run = spec.run;
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self, params))) {
            Ok(result) => result,
            Err(payload) => {
                let detail = payload
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "no details".to_string());
                Err(CommandError::new(format!(
                    "{id}: internal error (a bug in 3DCraft, please report it): {detail}"
                )))
            }
        }
    }

    pub(crate) fn document(&self) -> Result<&OpenDocument, CommandError> {
        self.documents.get(self.active).ok_or_else(|| {
            CommandError::new("no document is open; run file.new or file.open first")
        })
    }

    pub(crate) fn document_mut(&mut self) -> Result<&mut OpenDocument, CommandError> {
        self.documents.get_mut(self.active).ok_or_else(|| {
            CommandError::new("no document is open; run file.new or file.open first")
        })
    }

    pub(crate) fn add_document(&mut self, document: OpenDocument) -> Result<usize, CommandError> {
        if self.documents.len() >= MAX_DOCUMENTS {
            return Err(CommandError::new(format!(
                "{MAX_DOCUMENTS} documents are open already; close one with file.close first"
            )));
        }
        self.documents.push(document);
        self.active = self.documents.len() - 1;
        Ok(self.active)
    }
}
