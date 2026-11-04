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

pub mod agent_log;
pub mod analysis;
mod analysis_cmds;
mod angle_warnings;
pub mod animation;
mod appearance;
mod appearance_cmds;
mod asm;
mod asm_cmds;
mod asm_export;
mod asm_measure;
mod asm_outline;
mod asm_solve;
mod assert_check;
mod assert_cmds;
mod bake_cmds;
mod base64;
mod below_cmds;
mod bendseq_cmds;
mod blend_cmds;
mod body_cmds;
mod checks_elsewhere;
mod commands;
mod commands_elsewhere;
mod continuity_cmds;
mod cut_cmds;
mod datum_cmds;
mod develop_cmds;
mod drawing_cmds;
mod export_bodies;
mod face_cmds;
mod file_cmds;
mod fold_cmds;
pub mod fonts;
mod helix_cmds;
pub mod history_view;
mod import_cmds;
mod interference_cmds;
mod jig_cmds;
mod loft_cmds;
pub mod look;
mod mass;
mod material_cmds;
mod measure_pieces;
mod memory_files;
mod meshes;
mod motion;
mod motion_build;
mod name_refs;
mod offset_cmds;
mod open_bytes;
mod output_cmds;
mod param_cmds;
mod params;
mod pick_cmds;
pub mod pictures;
mod plate_cmds;
mod query_cmds;
mod recovery;
mod recovery_cmds;
#[cfg(not(target_arch = "wasm32"))]
mod recovery_store;
mod regen_elsewhere;
mod replay_cmds;
mod report;
mod sample_cmds;
mod scene;
mod scene_box;
mod scene_view;
mod scene_wire;
mod scrub;
mod section;
mod select_cmds;
mod selection;
mod set_cmds;
mod sheet_cmds;
mod sheetmetal_cmds;
mod sketch_cmds;
mod sketch_conflicts;
mod sketch_curve_cmds;
mod sketch_path_cmds;
mod sketch_pattern_cmds;
mod sketch_spline_cmds;
mod sketch_warnings;
mod solid_texture;
mod split_cmds;
mod step_cmds;
mod stepwise;
mod surface_cmds;
mod sweep_cmds;
mod sweep_path_cmds;
mod table_cmds;
pub mod task_score;
mod text_cmds;
mod tolerance_cmds;
mod trimfill_cmds;
mod typed;
mod unit_cmds;
mod verify;
mod verify_bound;
mod verify_cmds;
mod view_curves;
mod view_elsewhere;
mod view_export;
mod visibility_cmds;
mod when_cmds;
mod workspace;

pub use asm_outline::{AssemblyOutline, JointHandle, OutlineInstance, OutlineMate};
pub use assert_cmds::{AssertionState, AssertionStatus};
pub use checks_elsewhere::{CheckRequest, measure_checks};
pub use commands::{DARK_BACKGROUND, EXPORT_QUALITIES, VIEWPORT_BACKGROUND};
pub use commands_elsewhere::{Answered, CommandAnswer, CommandRequest};
pub use craft3d_regen::progress::Progress;
pub use history_view::{HistoryEntry, HistoryView, KeyEdit, VariableEntry};
pub use memory_files::{MAX_MEMORY_BYTES, MAX_MEMORY_FILE_BYTES, MAX_MEMORY_FILES};
pub use meshes::{BodyFacts, EdgePolylines};
pub use motion::{COLLISION_COLOR, MotionView, TIMELINE, TraceState, TraceView};
pub use output_cmds::{OutputState, OutputStatus};
pub use param_cmds::step_grid;
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
pub use regen_elsewhere::{
    SceneRequest, SceneStep, run_command, run_on_state, scene_begin, scene_bytes, scene_forget,
    scene_next, scene_side_work, scene_step,
};
pub use report::{
    FailedCommand, MAX_REPORT_FIELD_CHARS, VERSION, bug_report, build_commit, platform,
    set_build_commit,
};
pub use sample_cmds::with_point_sink;
pub use scene::{
    AUTOMATIC_RELATIVE_TOLERANCE, DATUM_AXIS_EXTENT, DATUM_PLANE_EXTENT, DEFAULT_SCENE_BUDGET,
    DISPLAY_MAX_ANGLE, DISPLAY_MAX_ANGLE_DEGREES, DISPLAY_MAX_TRIANGLES,
    DISPLAY_RELATIVE_TOLERANCE, DocumentStatus, FeatureState, MAX_ARC_SEGMENTS,
    MIN_CIRCLE_SEGMENTS, PREVIEW_RELATIVE_TOLERANCE, Refined, Refinement, Scene, SceneBody,
    SceneDatum, SceneDatumShape, SceneFeature, ScenePlacement, ScenePolyline, SceneQuality,
    SceneSketch, SketchCurve, arc_segments, base64 as encode_base64,
};
pub use scrub::{ScrubPosition, ScrubRegeneration, step_caption};
pub use section::{MAX_SECTION_EXTENT, SECTION_CAP_COLOR, SectionPlane, SectionSpec};
pub use selection::{
    MAX_SELECTION, SelectMode, SelectionItem, SelectionKind, SelectionView,
    summary as selection_summary,
};
pub use typed::TypedKind;
pub use workspace::{Access, Workspace};
pub use workspace::{canonical, plain};

use craft3d_doc::Document;
use craft3d_ops::History;
use craft3d_regen::{Need, Regen};
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
    /// The parameters that hold persistent names of faces, edges or
    /// vertices, or subjects made of them (a face name, `[face name, ...]`,
    /// edges as pairs of faces, a subject, a filter, a plane that may be a
    /// face): where a feature may be written by its name, looked up before
    /// the command runs (`name_refs`). A command opts in by listing them;
    /// nothing else of its parameters is looked into.
    pub names: &'static [&'static str],
    pub(crate) run: fn(&mut Session, &Value) -> Result<Value, CommandError>,
}

/// An open document with its undo history and what was derived from it.
#[derive(Clone)]
pub(crate) struct OpenDocument {
    /// Which open document this is, within the process: never reused, so
    /// a recovery file is never taken for another document's, and a
    /// window's view of a tab stays with its document.
    pub id: u64,
    pub history: History,
    /// The path it was opened from or last saved to, as the request gave it.
    pub path: Option<String>,
    /// The file on disk it was opened from or last saved to, absolute,
    /// whatever root `path` is relative to; `None` in memory, for a new
    /// document and for a recovered one until it is saved.
    pub file: Option<String>,
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
    /// What is selected in it: window state, not saved and not undone
    /// (`selection.rs`).
    pub(crate) selection: selection::Selection,
    /// The assertions checked against `history.revision()`
    /// (`assert_cmds.rs`).
    pub(crate) assertions: assert_check::Checks,
    /// Where the view is in the history (`view.scrub`): view state, not
    /// saved and not undone (`scrub.rs`).
    pub(crate) scrub: scrub::Scrub,
    /// The thumbnail drawn for a save, with the revision it shows, so a
    /// Jig saved again unchanged is not drawn again (`file.save`).
    pub(crate) thumbnail: Option<(u64, Arc<Vec<u8>>)>,
    /// A published Jig opened to be used (a `.3dcraftjig` file): only its
    /// parameters change, it is never saved over its file, and its
    /// parameter changes are not unsaved changes (`jig_cmds.rs`). Derived
    /// from how it was opened, not stored in the file.
    pub(crate) jig_in_use: bool,
    /// The assertions and outputs of a state measured elsewhere, whole
    /// (`regen_elsewhere.rs`): the job handed out and the last answer.
    pub(crate) state_checks: regen_elsewhere::StateChecks,
    /// Where the view is in the motion over `t` (`view.motion`), and the
    /// traces made over it: view state, not saved and not undone
    /// (`motion.rs`).
    pub(crate) motion: motion::Motion,
    /// For an assembly: what was read and derived for it (`asm.rs`).
    pub(crate) asm: asm::AsmState,
    /// What agents did to it (`agent_log.rs`): session state, not saved
    /// and not undone.
    pub(crate) agent_log: agent_log::AgentLog,
    /// Its file, when the person opened or saved it there themselves (no
    /// agent acting): the folder a host may give agents as "the open
    /// document's folder". An agent's open or save as clears it.
    pub(crate) person_file: Option<String>,
    /// While a command is tried without the bodies
    /// (`commands_elsewhere.rs`): `Some`, and true once it asked for a
    /// regeneration that is not there (it was handed an empty one).
    pub(crate) forbid_regen: Option<bool>,
    /// What regenerating it cost this session: shared with the session
    /// that holds it ([`Session::work_here`]).
    pub(crate) work: Arc<WorkHere>,
    /// Its assembly and the moments of its motion as made elsewhere
    /// (`view_elsewhere.rs`): view state, not saved and not undone.
    pub(crate) remote: view_elsewhere::Remote,
}

/// How often a session regenerated a document and meshed a body itself,
/// on its own thread: shared by the session, its documents and its
/// clones (a command tried without the bodies is tried on one), so a
/// host that has that work done elsewhere (the page's Web Worker,
/// `regen_elsewhere.rs`) can check that it never does it
/// ([`Session::work_here`]).
#[derive(Debug, Default)]
pub(crate) struct WorkHere {
    regenerations: std::sync::atomic::AtomicU64,
    /// The session shows its views from elsewhere
    /// ([`Session::regen_elsewhere`] as asked for, without the active
    /// document's kind): a document's assembly and moments of its motion
    /// are then made by the other side too (`view_elsewhere.rs`).
    elsewhere: std::sync::atomic::AtomicBool,
    /// Assemblies placed here (a placement solved and its bodies placed,
    /// not one kept): what a host that has them placed elsewhere checks
    /// never moves.
    placements: std::sync::atomic::AtomicU64,
}

impl WorkHere {
    pub(crate) fn regenerated(&self) {
        self.regenerations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn placed(&self) {
        self.placements
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn set_elsewhere(&self, elsewhere: bool) {
        self.elsewhere
            .store(elsewhere, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn elsewhere(&self) -> bool {
        self.elsewhere.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// [`Session::work_here`]: the regenerations of a document (whole or
/// a piece of one, a scrubbed state's too) and the bodies meshed for its
/// scenes, since the session was made. A body is meshed for an export or
/// a measure only once a regeneration made it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkCount {
    pub regenerations: u64,
    pub meshes: u64,
    /// Assemblies placed (solved and their bodies placed) here.
    pub placements: u64,
}

/// A regeneration cache with the budget of bodies the environment asks
/// for: `CRAFT3D_BODY_BUDGET_MB` is the most megabytes of bodies that went
/// into later features that are kept between commands
/// (`craft3d_regen::DEFAULT_BODY_BUDGET` otherwise; what a bigger history
/// costs is derived again when an edit needs it).
fn new_cache() -> craft3d_regen::RegenCache {
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut cache = craft3d_regen::RegenCache::default();
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(megabytes) = std::env::var("CRAFT3D_BODY_BUDGET_MB")
        .ok()
        .and_then(|text| text.trim().parse::<usize>().ok())
    {
        cache.set_body_budget(megabytes.saturating_mul(1 << 20));
    }
    cache
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
            file: None,
            saved_revision,
            disk: None,
            recovered: None,
            regen: None,
            cache: new_cache(),
            selection: selection::Selection::default(),
            assertions: assert_check::Checks::default(),
            scrub: scrub::Scrub::default(),
            thumbnail: None,
            jig_in_use: false,
            state_checks: regen_elsewhere::StateChecks::default(),
            motion: motion::Motion::default(),
            asm: asm::AsmState::default(),
            agent_log: agent_log::AgentLog::default(),
            person_file: None,
            forbid_regen: None,
            work: Arc::default(),
            remote: view_elsewhere::Remote::default(),
        }
    }

    /// The document its bodies are named in: the document itself, or for
    /// an assembly the part that holds its placed bodies (`asm.rs`), so
    /// that an export or a measure names them after their instances.
    pub(crate) fn bodies_doc(&mut self) -> Arc<Document> {
        match self.placed() {
            Some(placed) => Arc::clone(&placed.doc),
            None => Arc::clone(self.history.current()),
        }
    }

    /// For an assembly: its instances placed (`asm.rs`); `None` for a
    /// part.
    pub(crate) fn placed(&mut self) -> Option<Arc<asm::Placed>> {
        let doc = Arc::clone(self.history.current());
        if !doc.is_assembly() {
            return None;
        }
        let revision = self.history.revision();
        // Placed elsewhere and not here: a command tried without the
        // bodies asked for them (`commands_elsewhere.rs`), and runs where
        // they are.
        if self.forbid_regen.is_some() && self.work.elsewhere() && !self.asm.is_placed(revision) {
            self.forbid_regen = Some(true);
            return None;
        }
        if !self.asm.is_placed(revision) {
            self.work.placed();
        }
        Some(self.asm.placed(&doc, self.path.as_deref(), revision))
    }

    /// Changes not on disk. A Jig in use has none: its file is never
    /// written, and setting its parameters is using it, as a form is.
    pub(crate) fn dirty(&self) -> bool {
        !self.jig_in_use && self.saved_revision != Some(self.history.revision())
    }

    /// True when `file.close` refuses it without `discard`: it has changes
    /// that are not on disk and are worth keeping (it has a file, features,
    /// or an assembly's instances, mates or couplings). A new document
    /// nothing was added to closes freely.
    ///
    /// A Jig in use asks when its parameters changed since it was opened
    /// (docs/decisions.md, 2026-10-23): its file is never written, so the
    /// values set are kept only by saving them as a model.
    pub(crate) fn close_asks(&self) -> bool {
        if self.jig_in_use {
            return self.parameters_changed();
        }
        let doc = self.history.current();
        self.dirty()
            && (self.path.is_some()
                || !doc.features.is_empty()
                || doc.assembly.as_ref().is_some_and(|a| !a.is_empty()))
    }

    /// A Jig in use whose parameters were changed since it was opened (or
    /// last saved as a model).
    pub(crate) fn parameters_changed(&self) -> bool {
        self.jig_in_use && self.saved_revision != Some(self.history.revision())
    }

    /// The bodies and statuses of the current document. A body that went
    /// into a later feature may be missing when the cache let go of it to
    /// stay within its budget ([`OpenDocument::regen_needing`] derives it
    /// again).
    pub(crate) fn regen(&mut self) -> Arc<Regen> {
        self.regen_needing(Need::Bodies)
    }

    /// [`OpenDocument::regen`] with the bodies `need` asks for present:
    /// bodies the cache had let go of are derived again, the same bits.
    pub(crate) fn regen_needing(&mut self, need: Need) -> Arc<Regen> {
        // An assembly's bodies are its instances', placed.
        if let Some(placed) = self.placed() {
            return Arc::clone(&placed.regen);
        }
        let revision = self.history.revision();
        let missing = |regen: &Regen| {
            regen.features.iter().any(|f| {
                f.body.is_none()
                    && f.consumed_by.is_some()
                    && f.is_built()
                    && match need {
                        Need::Bodies => false,
                        Need::All => true,
                        Need::These(ids) => ids.contains(&f.id),
                    }
            })
        };
        match &self.regen {
            Some((cached, regen)) if *cached == revision && !missing(regen) => Arc::clone(regen),
            // A command tried without the bodies asked for them: it is run
            // where they are instead (`commands_elsewhere.rs`), and what it
            // did with this empty regeneration is thrown away.
            _ if self.forbid_regen.is_some() => {
                self.forbid_regen = Some(true);
                Arc::new(Regen::default())
            }
            _ => {
                self.work.regenerated();
                let regen = Arc::new(craft3d_regen::regenerate_cached_needing(
                    self.history.current(),
                    &mut self.cache,
                    need,
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
    /// Per open document and regeneration revision, the keys of the trees
    /// its bodies hold (`craft3d_regen::held_tree_keys`), worked out once
    /// per revision: an edit of one document does not write out the
    /// surfaces of the others again (40 ms an edit with the fuel tank open,
    /// 2026-11-04).
    tree_keys: Vec<(u64, Option<u64>, std::collections::BTreeSet<u64>)>,
    /// [`Session::set_body_budget`], for documents opened later.
    body_budget: Option<usize>,
    /// [`Session::set_lean_sampling`].
    pub(crate) lean_sampling: bool,
    /// [`Session::set_bake_in_background`].
    pub(crate) bake_in_background: bool,
    /// [`Session::set_motion_live`].
    motion_live: bool,
    /// The files of a session without a file system (`memory_files.rs`).
    pub(crate) memory: memory_files::MemoryFiles,
    /// The last command that failed, for the bug report (`report.rs`).
    last_failure: Option<report::FailedCommand>,
    /// What the frontend says about itself in the bug report.
    host_details: Vec<(String, String)>,
    /// The camera of the window's viewport, for `file.export` of the view
    /// as seen (`view: "current"`), as the window last said, with the
    /// document (its id) it was showing then.
    pub(crate) window_camera: Option<(u64, craft3d_render::Camera)>,
    /// While a command that adds features runs with `when`: the condition
    /// every feature it adds gets, in the same edit (`when_cmds`).
    pub(crate) new_when: Option<craft3d_doc::Expr>,
    /// The agent the commands run for, and its intent; `None` for the
    /// person (`agent_log.rs`).
    pub(crate) actor: Option<agent_log::Actor>,
    /// The active document was made active by an agent (`file.activate`,
    /// `file.open`, `recovery.open`, …), not by the person.
    pub(crate) agent_activated: bool,
    /// The host does the long work a piece at a time between its frames
    /// ([`Session::prepare_some`], `stepwise.rs`): assertions are not
    /// measured on a thread of their own nor all at once when asked.
    pub(crate) stepwise: bool,
    /// How far the last call of [`Session::prepare_some`] that stopped
    /// while meshing got: the scene revision, the phase and the bodies.
    pub(crate) reached: Option<(u64, &'static str, usize)>,
    /// The document (its id) and the state in which the last call of
    /// [`Session::prepare_some`] panicked: nothing more is done for it.
    pub(crate) prepare_broken: Option<(u64, u64)>,
    /// The host has the assertions and outputs measured elsewhere (a Web
    /// Worker: `checks_elsewhere.rs`) and hands their jobs over.
    pub(crate) checks_elsewhere: bool,
    /// The last number a job of checks was handed out under.
    pub(crate) checks_token: u64,
    /// The host has the active document regenerated and meshed elsewhere
    /// (a Web Worker: `regen_elsewhere.rs`) and hands the scenes' jobs
    /// over.
    pub(crate) regen_elsewhere: bool,
    /// The job of scenes handed out last, while it waits.
    pub(crate) scene_job: Option<regen_elsewhere::SceneJob>,
    /// The last number a job of scenes was handed out under.
    pub(crate) scene_token: u64,
    /// The digest of the assembly sources last handed to the worker that
    /// builds the scenes in full (`view_elsewhere::sources_digest`): the
    /// next job names them by it. Forgotten when that worker is started
    /// again ([`Session::scene_worker_restarted`]).
    pub(crate) sources_sent: Option<[u8; 32]>,
    /// The host's renderer besides the CPU's (`pictures.rs`).
    pub(crate) pictures: pictures::Hosted<dyn pictures::PictureRenderer>,
    /// The host's video encoder for `view.replay` (`pictures.rs`).
    pub(crate) video: pictures::Hosted<dyn pictures::VideoEncoder>,
    /// The commands that need the bodies are run where they are, by the
    /// host's worker, while the document is not regenerated here
    /// (`commands_elsewhere.rs`): the jobs handed out and waiting.
    pub(crate) commands: commands_elsewhere::Commands,
    /// What this session regenerated and meshed itself
    /// ([`Session::work_here`]).
    pub(crate) work: Arc<WorkHere>,
    /// While a command runs for another session (`run_on_state`): the
    /// undo steps its edits made, for that session to make the same.
    pub(crate) edit_log: Option<Vec<commands_elsewhere::EditStep>>,
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
            tree_keys: Vec::new(),
            body_budget: None,
            lean_sampling: false,
            bake_in_background: false,
            motion_live: false,
            memory: memory_files::MemoryFiles::default(),
            last_failure: None,
            host_details: Vec::new(),
            window_camera: None,
            new_when: None,
            actor: None,
            agent_activated: false,
            stepwise: false,
            reached: None,
            prepare_broken: None,
            checks_elsewhere: false,
            checks_token: 0,
            regen_elsewhere: false,
            scene_job: None,
            scene_token: 0,
            sources_sent: None,
            pictures: pictures::Hosted::default(),
            video: pictures::Hosted::default(),
            commands: commands_elsewhere::Commands::default(),
            work: Arc::default(),
            edit_log: None,
        }
    }

    /// How often this session regenerated a document and meshed a body
    /// itself since it was made (what it was answered from elsewhere does
    /// not count): a host that has the long work done by its worker
    /// checks with it that its own thread never does it.
    pub fn work_here(&self) -> WorkCount {
        use std::sync::atomic::Ordering::Relaxed;
        WorkCount {
            regenerations: self.work.regenerations.load(Relaxed),
            meshes: self.scene.meshes.made(),
            placements: self.work.placements.load(Relaxed),
        }
    }

    /// The camera the window's viewport shows the active document with:
    /// what `file.export {view: "current"}` draws to DXF or SVG. A window
    /// says it whenever it syncs; it holds for the document active now
    /// only (another one made active in a batch since has none). Without
    /// one (`None`) that view is the iso view. View state: nothing in a
    /// document changes.
    pub fn set_window_camera(&mut self, camera: Option<craft3d_render::Camera>) {
        let active = self.document_status().map(|status| status.id);
        self.window_camera = camera.zip(active).map(|(camera, id)| (id, camera));
    }

    /// Lets the kernel go of the trees it keeps for freeform surfaces that
    /// no body of an open document holds any more
    /// (`craft3d_kernel::release_unused`), when a document was closed or
    /// regenerated since it last did. Every body a regeneration made is
    /// held, also those that went into later features, so that an edit
    /// further down finds the trees of what it starts from. Results do not
    /// change: a tree is a function of its surface.
    /// [`Session::release_unused_trees`] even when no open document
    /// changed: after work in a session of its own (`document.sample`'s
    /// points), whose bodies are gone.
    pub(crate) fn release_trees_now(&mut self) {
        self.trees_held_for.clear();
        self.release_unused_trees();
    }

    fn release_unused_trees(&mut self) {
        let now: Vec<(u64, Option<u64>)> = self
            .documents
            .iter()
            .map(|d| (d.id, d.regen.as_ref().map(|(revision, _)| *revision)))
            .collect();
        if now == self.trees_held_for {
            return;
        }
        // A document closed: the lines of thicknesses kept for its bodies
        // (and any other's) go too, measured again when asked for.
        if self
            .trees_held_for
            .iter()
            .any(|(id, _)| !now.iter().any(|(n, _)| n == id))
        {
            crate::verify::forget_kept_lines();
        }
        let mut keys = Vec::with_capacity(self.documents.len());
        for document in &self.documents {
            let revision = document.regen.as_ref().map(|(revision, _)| *revision);
            let known = self
                .tree_keys
                .iter()
                .position(|(id, at, _)| *id == document.id && *at == revision);
            let held = match known {
                Some(at) => self.tree_keys.swap_remove(at).2,
                None => {
                    let bodies: Vec<&craft3d_topo::Body> = document
                        .regen
                        .iter()
                        .flat_map(|(_, regen)| regen.features.iter())
                        .filter_map(|f| f.body.as_deref())
                        .collect();
                    craft3d_regen::held_tree_keys(&bodies)
                }
            };
            keys.push((document.id, revision, held));
        }
        let held: std::collections::BTreeSet<u64> = keys
            .iter()
            .flat_map(|(_, _, held)| held.iter().copied())
            .collect();
        craft3d_regen::release_unheld_trees(&held);
        self.tree_keys = keys;
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

    /// The active document as it is now, as the bytes of its `.3dcraft`
    /// file (without a thumbnail): two sessions whose documents give the
    /// same bytes hold the same design. `None` without a document or when
    /// it cannot be written.
    pub fn document_bytes(&self) -> Option<Vec<u8>> {
        let open = self.documents.get(self.active)?;
        craft3d_format::to_bytes(open.history.current()).ok()
    }

    /// The command registry, in a fixed order. A new command that puts
    /// back features without anyone making them (a revert, a "back to the
    /// last good state", a reopening) must also be listed in
    /// `agent_log::NO_PROVENANCE`, or who made those features is rewritten.
    pub fn commands() -> &'static [CommandSpec] {
        static ALL: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
        ALL.get_or_init(|| {
            [
                commands::SPECS,
                file_cmds::SPECS,
                agent_log::SPECS,
                asm_cmds::SPECS,
                asm_measure::SPECS,
                import_cmds::SPECS,
                sketch_cmds::SPECS,
                sketch_path_cmds::SPECS,
                sketch_pattern_cmds::SPECS,
                sketch_spline_cmds::SPECS,
                text_cmds::SPECS,
                sketch_curve_cmds::SPECS,
                sweep_cmds::SPECS,
                loft_cmds::SPECS,
                develop_cmds::SPECS,
                analysis_cmds::SPECS,
                surface_cmds::SPECS,
                trimfill_cmds::SPECS,
                continuity_cmds::SPECS,
                plate_cmds::SPECS,
                sweep_path_cmds::SPECS,
                body_cmds::SPECS,
                appearance_cmds::SPECS,
                material_cmds::SPECS,
                blend_cmds::SPECS,
                face_cmds::SPECS,
                datum_cmds::SPECS,
                helix_cmds::SPECS,
                offset_cmds::SPECS,
                sheet_cmds::SPECS,
                fold_cmds::SPECS,
                sheetmetal_cmds::SPECS,
                bendseq_cmds::SPECS,
                cut_cmds::SPECS,
                split_cmds::SPECS,
                query_cmds::SPECS,
                interference_cmds::SPECS,
                verify_cmds::SPECS,
                below_cmds::SPECS,
                assert_cmds::SPECS,
                output_cmds::SPECS,
                pick_cmds::SPECS,
                select_cmds::SPECS,
                set_cmds::SPECS,
                param_cmds::SPECS,
                table_cmds::SPECS,
                jig_cmds::SPECS,
                sample_cmds::SPECS,
                recovery_cmds::SPECS,
                replay_cmds::SPECS,
                visibility_cmds::SPECS,
                when_cmds::SPECS,
                scrub::SPECS,
                motion::SPECS,
                bake_cmds::SPECS,
                report::SPECS,
                unit_cmds::SPECS,
                tolerance_cmds::SPECS,
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
        let held = open.regen_needing(Need::All);
        Some(*held == craft3d_regen::regenerate(open.history.current()))
    }

    /// Whether `document.sample` lets go of what the kernel keeps per
    /// process (intersections, projections, meshes of faces) after every
    /// point, so that a long run holds one point's memory
    /// (`craft3d-cli check-range` asks for it), rather than once at the end
    /// of the run (the default: a window's run is short, and its open
    /// document's answers stay warm meanwhile). No result changes.
    pub fn set_lean_sampling(&mut self, lean: bool) {
        self.lean_sampling = lean;
    }

    /// Whether `motion.bake` without `background` bakes in the background
    /// (a window: its thread must never wait seconds for a bake, and the
    /// bake stays cancellable) rather than in the command (the default:
    /// a script waits for it). Natively only; a browser bakes in the
    /// command.
    pub fn set_bake_in_background(&mut self, background: bool) {
        self.bake_in_background = background;
    }

    /// Says whether the window's own viewport is what asks for the scene
    /// now (true) or something that needs the exact picture (false, the
    /// default: a screenshot, a frame-exact recording, a script). While it
    /// is true, the moment of a playing motion may be shown provisionally
    /// (docs/design/motion.md › *Shapes in the background*) and settles
    /// as the viewer polls ([`Self::poll_motion`]); while it is false,
    /// every picture is of the exact moment, made where it is asked for if
    /// it must be. Commands always see it false: [`Self::execute`] turns
    /// it off while one runs, so `view.render`, exports and assertions
    /// never show a provisional moment.
    pub fn set_motion_live(&mut self, live: bool) {
        self.motion_live = live;
        for open in &mut self.documents {
            open.motion.live = live;
        }
    }

    /// Sets, for every open document and those opened later, the most
    /// bytes of bodies that went into later features that are kept
    /// between commands ([`craft3d_regen::RegenCache::set_body_budget`]).
    pub fn set_body_budget(&mut self, bytes: usize) {
        self.body_budget = Some(bytes);
        for document in &mut self.documents {
            document.cache.set_body_budget(bytes);
        }
    }

    /// Sets, for every open document, the most bytes of an assembly's
    /// configurations kept between commands, and of a part's moments made
    /// ahead (`motion.bake`); beyond it the least recently used go first
    /// (`asm::DEFAULT_CONFIG_BUDGET` unless set). A motion made after this
    /// takes the budget with it; documents opened later have the default.
    pub fn set_config_budget(&mut self, bytes: usize) {
        for document in &mut self.documents {
            document.asm.set_config_budget(bytes);
            document.motion_forget_sweep();
        }
    }

    /// About how many bytes the bodies kept take that went into later
    /// features of the active document, and how many such bodies the
    /// cache let go of; `None` without a document.
    pub fn bodies_held(&self) -> Option<(usize, usize)> {
        Some(self.documents.get(self.active)?.cache.bodies_held())
    }

    /// How many bodies the last regeneration of the active document
    /// derived again, having let go of them.
    pub fn last_regeneration_rederived(&self) -> Option<usize> {
        Some(self.documents.get(self.active)?.cache.last_run_rederived())
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

    /// The features of the active document that fail now, each as the
    /// entries of an edit's `failed` (`{feature, id, error, causedBy?}`),
    /// in history order; empty without a document. Regenerates when the
    /// document changed since it was last regenerated.
    pub fn failed_features(&mut self) -> Vec<Value> {
        let mut out = Value::Object(serde_json::Map::new());
        if !self.has_document()
            || commands::add_failed_waiting(self, &[], None, &mut out, true).is_err()
        {
            return Vec::new();
        }
        out.get("failed")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
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
        let before = (
            self.documents
                .iter()
                .map(|open| (open.id, open.file.clone()))
                .collect::<Vec<_>>(),
            self.document().ok().map(|open| open.id),
        );
        let outcome = self.logged(id, |session| session.execute_unrecorded(id, params));
        self.track_who_opened(id, before);
        // The report itself is not what a report is about.
        if let Err(error) = &outcome
            && id != "session.report"
        {
            self.record_failure(id, params, error.message());
        }
        outcome
    }

    fn execute_unrecorded(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        // A command never sees a provisional moment (`set_motion_live`).
        let live = self.motion_live;
        if live {
            self.set_motion_live(false);
        }
        let outcome = self.execute_exact(id, params);
        if live {
            self.set_motion_live(true);
        }
        outcome
    }

    fn execute_exact(&mut self, id: &str, params: &Value) -> Result<Value, CommandError> {
        let asked = params;
        let Some(spec) = Self::command(id) else {
            let hint = commands::closest(id)
                .map(|c| format!("; did you mean `{c}`?"))
                .unwrap_or_default();
            return Err(CommandError::new(format!("unknown command `{id}`{hint}")));
        };
        if let Some(reason) = self.disabled_reason(spec) {
            return Err(CommandError::new(format!("{id}: {reason}")));
        }
        // Features written by their names in face and edge names get their
        // ids, from the document as it is now (`name_refs`), in the
        // parameters the spec lists as holding names.
        let resolved = match self.document() {
            Ok(open) if !spec.names.is_empty() => {
                name_refs::resolve(open.history.current(), spec.names, params)
                    .map_err(|e| CommandError::new(format!("{id}: {e}")))?
            }
            _ => name_refs::Resolved::default(),
        };
        let mut params = match &resolved.params {
            Some(with_ids) => std::borrow::Cow::Borrowed(with_ids),
            None => std::borrow::Cow::Borrowed(params),
        };
        // A command that adds features takes their condition with them.
        self.new_when = when_cmds::take_when(spec, &mut params)?;
        let run = spec.run;
        // Not regenerated here while the host has it regenerated
        // elsewhere: a command that turns out to need the bodies is run
        // where they are (`commands_elsewhere.rs`).
        let trial = self.try_without_bodies(id);
        let mut outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self, &params)));
        if let Some(snapshot) = trial {
            match self.after_trial(snapshot, id, &params, asked) {
                commands_elsewhere::Trial::Kept => {}
                commands_elsewhere::Trial::Deferred(result) => {
                    self.new_when = None;
                    return Ok(result);
                }
                commands_elsewhere::Trial::RunHere => {
                    outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run(self, &params)
                    }));
                    self.ran_here();
                }
            }
        }
        self.new_when = None;
        match outcome {
            Ok(result) => {
                self.recovery_sync(false);
                self.release_unused_trees();
                // Every error names the command, also one said by a part
                // that does not know it (a path outside the write root).
                result.map_err(|e| {
                    let e = resolved.explain(e);
                    if e.message().starts_with(id) {
                        e
                    } else {
                        CommandError::new(format!("{id}: {e}"))
                    }
                })
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

    /// `params` with the features written by their names in the persistent
    /// names under `keys` replaced by their ids, from the active document,
    /// as a command gets them (`CommandSpec::names`): for a frontend's own
    /// methods that take face, edge or vertex names (the window's
    /// highlight and section). `params` as they are without a document;
    /// `Err` with the reason for a name of a feature that is not there.
    pub fn with_feature_ids(&self, keys: &[&str], params: &Value) -> Result<Value, String> {
        let Ok(open) = self.document() else {
            return Ok(params.clone());
        };
        let resolved = name_refs::resolve(open.history.current(), keys, params)?;
        Ok(resolved.params.unwrap_or_else(|| params.clone()))
    }

    /// Notes, after a command, who made the active document active and who
    /// gave it its file: the person (no actor) or an agent. `before` is
    /// every open document's id and file, and the active one's id.
    fn track_who_opened(&mut self, id: &str, before: (Vec<(u64, Option<String>)>, Option<u64>)) {
        let by_agent = self.actor.is_some();
        let (documents, active_before) = before;
        let Some(open) = self.documents.get_mut(self.active) else {
            return;
        };
        // A change of the active document is whoever made it; the person
        // activating the document already active (a click on its tab)
        // makes it theirs again.
        if active_before != Some(open.id) || (!by_agent && id == "file.activate") {
            self.agent_activated = by_agent;
        }
        let file_before = documents
            .iter()
            .find(|(id, _)| *id == open.id)
            .map(|(_, file)| file);
        let gave_file = match file_before {
            // Opened (or made) by this command.
            None => true,
            Some(file) => *file != open.file,
        };
        if gave_file {
            open.person_file = if by_agent { None } else { open.file.clone() };
        }
    }

    /// The file of the active document when the person opened or saved it
    /// themselves and made it active themselves; `None` otherwise (a new or
    /// recovered document, one an agent opened, saved elsewhere or made
    /// active). What a host may give agents as "the open document's
    /// folder".
    pub fn person_document_file(&self) -> Option<String> {
        if self.agent_activated {
            return None;
        }
        self.document().ok()?.person_file.clone()
    }

    /// The file of open document `index` when the person opened or saved
    /// it there themselves (an agent's file is `None`): what a host may
    /// list among the person's recent files.
    pub fn person_file_of(&self, index: usize) -> Option<String> {
        self.documents.get(index)?.person_file.clone()
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
        let mut document = document;
        document.work = Arc::clone(&self.work);
        if let Some(budget) = self.body_budget {
            document.cache.set_body_budget(budget);
        }
        self.documents.push(document);
        self.active = self.documents.len() - 1;
        Ok(self.active)
    }
}
