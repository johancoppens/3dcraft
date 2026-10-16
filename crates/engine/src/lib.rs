//! The engine: the façade every frontend talks to (docs/design/engine.md).
//!
//! A [`Session`] holds open documents and runs commands. Every action is a
//! command with a stable id and JSON parameters, so the UI, the CLI, the
//! control channel and the MCP server all do exactly the same things
//! (AGENTS.md rule 1).
//!
//! Deviations from the plan, for now: commands run synchronously on the
//! caller's thread (no job pool, no events). Regeneration runs after every
//! edit and rebuilds only the features the edit touches
//! (`craft3d_regen::RegenCache`). The one thread the engine starts is the
//! autosave thread of a session that was given a recovery directory
//! (`recovery.rs`).

mod appearance;
mod appearance_cmds;
mod base64;
mod blend_cmds;
mod body_cmds;
mod commands;
mod datum_cmds;
#[cfg(not(target_arch = "wasm32"))]
mod drawing_cmds;
mod face_cmds;
mod file_cmds;
mod helix_cmds;
mod interference_cmds;
mod loft_cmds;
mod mass;
mod offset_cmds;
mod params;
mod pick_cmds;
mod query_cmds;
mod recovery;
mod recovery_cmds;
#[cfg(not(target_arch = "wasm32"))]
mod recovery_store;
mod replay_cmds;
mod scene;
mod section;
mod sketch_cmds;
mod sketch_path_cmds;
mod sketch_pattern_cmds;
mod sketch_spline_cmds;
#[cfg(not(target_arch = "wasm32"))]
mod step_cmds;
mod sweep_cmds;
mod sweep_path_cmds;
mod workspace;

pub use pick_cmds::{
    DEFAULT_PICK_RADIUS, MAX_CORNER_FACES, MAX_PICK_RADIUS, PickKind, SceneCut, SceneEdges,
    SceneFaces, SceneNear, ScenePick, ScenePointed, SceneVertices,
};
#[cfg(not(target_arch = "wasm32"))]
pub use recovery::SystemClock;
pub use recovery::{
    Clock, DEFAULT_AUTOSAVE_INTERVAL, MAX_AUTOSAVE_INTERVAL, MIN_AUTOSAVE_INTERVAL, ManualClock,
    Recoverable, RecoveryNotify, RecoveryOptions, RecoveryOverview, default_recovery_directory,
    parse_autosave_interval,
};
pub use scene::{
    AUTOMATIC_RELATIVE_TOLERANCE, DATUM_AXIS_EXTENT, DATUM_PLANE_EXTENT, DEFAULT_SCENE_BUDGET,
    DISPLAY_MAX_ANGLE, DISPLAY_MAX_ANGLE_DEGREES, DISPLAY_MAX_TRIANGLES,
    DISPLAY_RELATIVE_TOLERANCE, DocumentStatus, FeatureState, MAX_ARC_SEGMENTS,
    MIN_CIRCLE_SEGMENTS, PREVIEW_RELATIVE_TOLERANCE, Refined, Refinement, Scene, SceneBody,
    SceneDatum, SceneDatumShape, SceneFeature, ScenePolyline, SceneQuality, SceneSketch,
    SketchCurve, arc_segments, base64 as encode_base64,
};
pub use section::{MAX_SECTION_EXTENT, SECTION_CAP_COLOR, SectionPlane, SectionSpec};
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
#[derive(Clone)]
pub(crate) struct OpenDocument {
    /// Which open document this is, within the process: never reused, so
    /// a recovery file is never taken for another document's.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub id: u64,
    pub history: History,
    /// The path it was opened from or last saved to, as the request gave it.
    pub path: Option<String>,
    /// The revision that is on disk.
    pub saved_revision: Option<u64>,
    /// What the file at `path` looked like when the document was opened
    /// from it or last saved to it.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub disk: Option<recovery::FileStamp>,
    /// Set while this is a document taken from a recovery file and not
    /// saved since (`recovery.open`).
    pub recovered: Option<recovery::Recovered>,
    /// Regeneration of `history.revision()`.
    regen: Option<(u64, Arc<Regen>)>,
    /// What the last regeneration found per feature, so the next one
    /// rebuilds only what changed.
    pub(crate) cache: craft3d_regen::RegenCache,
}

/// The id of the next open document.
fn next_document_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl OpenDocument {
    pub(crate) fn new(document: Document, path: Option<String>) -> Self {
        let history = History::new(document);
        let saved_revision = path.is_some().then(|| history.revision());
        Self {
            id: next_document_id(),
            history,
            path,
            saved_revision,
            disk: None,
            recovered: None,
            regen: None,
            cache: craft3d_regen::RegenCache::default(),
        }
    }

    pub(crate) fn dirty(&self) -> bool {
        self.saved_revision != Some(self.history.revision())
    }

    /// True when `file.close` refuses it without `discard`: it has changes
    /// that are not on disk and are worth keeping (it has a file, or
    /// features). A new document nothing was added to closes freely.
    pub(crate) fn close_asks(&self) -> bool {
        self.dirty() && (self.path.is_some() || !self.history.current().features.is_empty())
    }

    /// The bodies and statuses of the current document.
    pub(crate) fn regen(&mut self) -> Arc<Regen> {
        let revision = self.history.revision();
        match &self.regen {
            Some((cached, regen)) if *cached == revision => Arc::clone(regen),
            _ => {
                let regen = Arc::new(craft3d_regen::regenerate_cached(
                    self.history.current(),
                    &mut self.cache,
                ));
                self.regen = Some((revision, Arc::clone(&regen)));
                regen
            }
        }
    }
}

/// Open documents plus the rules for touching files. A clone is an
/// independent session with the same documents (bodies and meshes are
/// shared until one of them changes).
#[derive(Clone)]
pub struct Session {
    pub(crate) documents: Vec<OpenDocument>,
    pub(crate) active: usize,
    pub(crate) workspace: Workspace,
    /// What a viewer draws for the active document (`scene.rs`).
    pub(crate) scene: scene::SceneCache,
    /// Autosave and crash recovery (`recovery.rs`); none in a clone.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) recovery: recovery::RecoveryHandle,
    /// The open documents and the revisions of their regenerations when
    /// the kernel last let go of the trees of surfaces no body holds
    /// ([`Session::release_unused_trees`]).
    trees_held_for: Vec<(u64, Option<u64>)>,
}

/// Most documents open at once.
pub const MAX_DOCUMENTS: usize = 64;

impl Session {
    pub fn new(workspace: Workspace) -> Self {
        Self {
            documents: Vec::new(),
            active: 0,
            workspace,
            scene: scene::SceneCache::default(),
            recovery: recovery::RecoveryHandle::default(),
            trees_held_for: Vec::new(),
        }
    }

    /// Lets the kernel go of the trees it keeps for freeform surfaces that
    /// no body of an open document holds any more
    /// (`craft3d_kernel::release_unused`), when a document was closed or
    /// regenerated since it last did. Every body a regeneration made is
    /// held, also those that went into later features, so that an edit
    /// further down finds the trees of what it starts from. Results do not
    /// change: a tree is a function of its surface.
    fn release_unused_trees(&mut self) {
        let now: Vec<(u64, Option<u64>)> = self
            .documents
            .iter()
            .map(|d| (d.id, d.regen.as_ref().map(|(revision, _)| *revision)))
            .collect();
        if now == self.trees_held_for {
            return;
        }
        let bodies: Vec<&craft3d_topo::Body> = self
            .documents
            .iter()
            .filter_map(|d| d.regen.as_ref())
            .flat_map(|(_, regen)| regen.features.iter())
            .filter_map(|f| f.body.as_deref())
            .collect();
        craft3d_regen::release_unused_trees(&bodies);
        self.trees_held_for = now;
    }

    /// The revision of the active document: it moves with every edit
    /// applied to it, and stays when a command fails. `None` without a
    /// document.
    pub fn document_revision(&self) -> Option<u64> {
        self.documents
            .get(self.active)
            .map(|d| d.history.revision())
    }

    /// The command registry, in a fixed order.
    pub fn commands() -> &'static [CommandSpec] {
        static ALL: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
        ALL.get_or_init(|| {
            [
                commands::SPECS,
                file_cmds::SPECS,
                sketch_cmds::SPECS,
                sketch_path_cmds::SPECS,
                sketch_pattern_cmds::SPECS,
                sketch_spline_cmds::SPECS,
                sweep_cmds::SPECS,
                loft_cmds::SPECS,
                sweep_path_cmds::SPECS,
                body_cmds::SPECS,
                appearance_cmds::SPECS,
                blend_cmds::SPECS,
                face_cmds::SPECS,
                datum_cmds::SPECS,
                helix_cmds::SPECS,
                offset_cmds::SPECS,
                query_cmds::SPECS,
                interference_cmds::SPECS,
                pick_cmds::SPECS,
                recovery_cmds::SPECS,
                replay_cmds::SPECS,
            ]
            .concat()
        })
    }

    pub fn command(id: &str) -> Option<&'static CommandSpec> {
        Self::commands().iter().find(|c| c.id == id)
    }

    /// Regenerates the active document from nothing and compares it with
    /// what the session holds, which was built up edit by edit. `None`
    /// when no document is open. For tests: the two must never differ.
    pub fn regeneration_matches_a_full_one(&mut self) -> Option<bool> {
        let open = self.documents.get_mut(self.active)?;
        let held = open.regen();
        Some(*held == craft3d_regen::regenerate(open.history.current()))
    }

    /// How many features the last regeneration of the active document
    /// rebuilt, and how many it reused.
    pub fn last_regeneration(&self) -> Option<(usize, usize)> {
        Some(self.documents.get(self.active)?.cache.last_run())
    }

    /// How many entries of what features are made from the last
    /// regeneration of the active document looked at
    /// (`craft3d_regen::RegenCache::last_run_work`): for tests and
    /// benchmarks that hold regeneration to grow with the history, not
    /// with its square.
    pub fn last_regeneration_work(&self) -> Option<usize> {
        Some(self.documents.get(self.active)?.cache.last_run_work())
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
    ///
    /// Afterwards the recovery store, when the session has one, is told
    /// which documents changed (`recovery.rs`): no disk is touched for
    /// that on this thread, except to remove the recovery file of a
    /// document that was just saved or closed. After a caught panic the
    /// last good state of every document is autosaved at once.
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
            Ok(result) => {
                self.recovery_sync(false);
                self.release_unused_trees();
                result
            }
            Err(payload) => {
                self.recovery_sync(true);
                self.release_unused_trees();
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
