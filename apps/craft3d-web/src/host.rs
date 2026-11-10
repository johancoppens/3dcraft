//! The session and the shell of the page, and what the File menu, the
//! tabs and the files the person hands the page do with them. Nothing
//! here touches the browser: the page ([`crate::page`]) asks the person
//! for a file when [`Outcome::ask`] says so, hands the bytes over with
//! [`WebHost::receive`], and offers each [`Download`] to the person. So
//! the whole flow runs in native tests.
//!
//! Where the browser has the File System Access API (Chrome, Edge,
//! Chromebooks) the page asks with the system's file dialogs
//! ([`WebHost::pickers`]): Save, Save As and Export name the file in a
//! save dialog ([`Outcome::pick`]; an export is made before, with its
//! format chosen in the dialog over the page), and a document opened or
//! saved through one is saved back to that file without asking again
//! ([`WebHost::writable`]). Such a write ends later: a tab is closed and a
//! document stays saved only once it succeeded
//! ([`WebHost::close_document`], [`WebHost::write_failed`]). Elsewhere a
//! saved or exported file is a download and the dialog over the page
//! ([`NameDialog`]) names it.
//!
//! The session keeps its files in memory (`craft3d_engine::Access::Memory`):
//! a file the person chose is put there under its name, the same command
//! as on the desktop reads it (`file.open`, `file.import`), and what
//! `file.save` and `file.export` write there is taken out again as a
//! download. Nothing stays in memory after an action.

use craft3d_engine::{Session, Workspace};
use craft3d_ui_egui::files::{
    ASSEMBLY_EXTENSION, EXPORT_FORMATS, EXTENSION, ExportPlan, JIG_EXTENSION, export_params,
    extension_of, file_stem, suggested_jig_name, suggested_name,
};
use craft3d_ui_egui::kit::{Button, Dialog, Kit};
use craft3d_ui_egui::shell::ExportRequest;
use craft3d_ui_egui::{CommandReport, DocAction, FileAction, Shell, Work};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// A file for the person to save: its name and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub name: String,
    pub bytes: Vec<u8>,
    /// The document this is the saved file of (its id in the session),
    /// for the page to write it back to the file it came from; `None`
    /// for an export.
    pub document: Option<u64>,
    /// True when the document is to be closed once this file is written
    /// (Save in the question before closing a tab): the page closes it
    /// only after the write succeeded ([`WebHost::close_document`]).
    pub close: bool,
}

impl Download {
    /// An exported file (not a document's).
    fn export(name: String, bytes: Vec<u8>) -> Self {
        Self {
            name,
            bytes,
            document: None,
            close: false,
        }
    }
}

/// What the page names a file for with the system's save dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// The document `document` (its id), saved as a `.3dcraft` file, and
    /// closed once it is written when `close`.
    Save {
        document: u64,
        suggested: String,
        close: bool,
    },
    /// An export, already made: `file` is written where the dialog says.
    /// The format is chosen before (the dialog over the page), so a dialog
    /// is only shown for bytes that exist, and an export that fails never
    /// touches the person's file.
    Export { file: Download },
}

impl Pick {
    /// The kinds of file the dialog offers: one, the file's.
    pub fn types(&self) -> Vec<PickerType> {
        match self {
            Pick::Save { .. } => vec![document_type()],
            Pick::Export { file } if extension_of(&file.name).as_deref() == Some(JIG_EXTENSION) => {
                vec![jig_type()]
            }
            Pick::Export { file } => {
                let extension = extension_of(&file.name);
                export_types(extension.as_deref())
                    .into_iter()
                    .filter(|t| {
                        extension
                            .as_deref()
                            .is_some_and(|e| t.extensions.contains(&e))
                    })
                    .collect()
            }
        }
    }

    /// The name the dialog proposes.
    pub fn suggested(&self) -> &str {
        match self {
            Pick::Save { suggested, .. } => suggested,
            Pick::Export { file } => &file.name,
        }
    }

    /// True when the file the dialog chose, `name`, can hold what is
    /// written: a `.3dcraft` name for a document, an extension of the
    /// export's format for an export (never STEP bytes under `.stl`).
    pub fn accepts(&self, name: &str) -> bool {
        let Some(chosen) = extension_of(name) else {
            return false;
        };
        self.types()
            .iter()
            .any(|t| t.extensions.contains(&chosen.as_str()))
    }
}

fn document_type() -> PickerType {
    PickerType {
        description: "3DCraft document",
        mime: "application/x-craft3d",
        extensions: vec![EXTENSION],
    }
}

/// A published Jig (`.3dcraftjig`): the same format, opened to be used.
fn jig_type() -> PickerType {
    PickerType {
        description: "3DCraft Jig",
        mime: "application/x-craft3d-jig",
        extensions: vec![JIG_EXTENSION],
    }
}

/// A kind of file in the system's file dialog: a description, a media
/// type and its extensions (without the dot).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerType {
    pub description: &'static str,
    pub mime: &'static str,
    pub extensions: Vec<&'static str>,
}

/// The kinds of file Open and Import choose from.
pub fn open_types(ask: Ask) -> Vec<PickerType> {
    match ask {
        Ask::Open => vec![
            PickerType {
                description: "3DCraft document, Jig or assembly",
                mime: "application/x-craft3d",
                extensions: vec![EXTENSION, JIG_EXTENSION, ASSEMBLY_EXTENSION],
            },
            jig_type(),
        ],
        Ask::Import => vec![PickerType {
            description: "STEP",
            mime: "model/step",
            extensions: vec!["step", "stp"],
        }],
    }
}

/// The media type the file dialog files a format under (the dialog
/// filters by the extensions; the type only has to be well formed).
fn export_mime(extension: &str) -> &'static str {
    match extension {
        "step" | "stp" => "model/step",
        "stl" => "model/stl",
        "3mf" => "model/3mf",
        "obj" => "model/obj",
        "dxf" => "image/vnd.dxf",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// Every export format as a kind of file for the save dialog, the one
/// with extension `first` (the last format exported) first.
pub fn export_types(first: Option<&str>) -> Vec<PickerType> {
    let mut types: Vec<PickerType> = EXPORT_FORMATS
        .iter()
        .map(|(format, extensions)| PickerType {
            description: format,
            mime: export_mime(extensions.first().copied().unwrap_or_default()),
            extensions: extensions.to_vec(),
        })
        .collect();
    if let Some(at) = types
        .iter()
        .position(|t| first.is_some_and(|f| t.extensions.contains(&f)))
    {
        let chosen = types.remove(at);
        types.insert(0, chosen);
    }
    types
}

/// The number of the job a command was handed over under, when it runs
/// where the bodies are (`{deferred: true, job}`).
fn deferred(result: &Value) -> Option<u64> {
    (result.get("deferred") == Some(&Value::Bool(true)))
        .then(|| result.get("job").and_then(Value::as_u64))
        .flatten()
}

/// The keys the page keeps from the browser (`keydown`'s default is
/// prevented; egui still gets them), `key` in lower case: Ctrl (⌘) with
/// E (Export; Chrome and Firefox would focus the address bar's search) or
/// I (Import; Firefox's page information), not with Shift, which stay
/// the browser's developer tools; and Undo and Redo, Ctrl with Z,
/// Shift+Z or Y. Never with Alt.
pub fn page_keeps_key(command: bool, alt: bool, shift: bool, key: &str) -> bool {
    if !command || alt {
        return false;
    }
    match key {
        "e" | "i" | "y" => !shift,
        "z" => true,
        _ => false,
    }
}

/// True when a browser refused a file dialog (`why`, the error's name and
/// message) because the click or key press that asked for it came too
/// long ago, or none did: the next click may well show it. Any other
/// refusal (a cross-origin iframe, a sandbox, a browser that has the API
/// but not for this page) is for good, and the page falls back to
/// downloads and the file input for the rest of the session.
pub fn refusal_is_about_activation(why: &str) -> bool {
    let why = why.to_ascii_lowercase();
    why.contains("user gesture") || why.contains("activation")
}

/// What the page asks the person for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// A document to open (`.3dcraft`).
    Open,
    /// A STEP file to import.
    Import,
}

/// What an action leaves for the page to do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub ask: Option<Ask>,
    /// A file to name with the system's save dialog (with
    /// [`WebHost::pickers`] only).
    pub pick: Option<Pick>,
    pub downloads: Vec<Download>,
}

/// What the dialog over the page names a file for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Save As (or Save of a document without a name): a `.3dcraft`
    /// download. With the document to close once it is saved (Save in
    /// the question before closing a tab).
    Save { close: Option<usize> },
    /// Export as Jig: a `.3dcraftjig` (`jig.export`).
    ExportJig,
}

/// The dialog in which the person names the file, while it is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameDialog {
    pub purpose: Purpose,
    pub name: String,
}

/// The most bytes of a name, as a file the person hands in or names.
const MAX_NAME: usize = 200;

/// A file name from the browser (a dropped or chosen file's, or one the
/// person typed) as a name in the session's memory: its last part, without
/// control characters, at most [`MAX_NAME`] bytes; `None` when nothing is
/// left or it is only dots.
pub fn memory_name(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut clean: String = last.chars().filter(|c| !c.is_control()).collect();
    clean = clean.trim().to_string();
    while clean.len() > MAX_NAME {
        clean.pop();
    }
    (!clean.is_empty() && !clean.chars().all(|c| c == '.')).then_some(clean)
}

/// `name` ending in `.extension`: as it is when it has that extension
/// (in any case), else with it appended, so a document saved as `x.stl`
/// is `x.stl.3dcraft` and never a `.3dcraft` file under another format's
/// name.
fn with_extension(name: &str, extension: &str) -> String {
    if extension_of(name).as_deref() == Some(extension) {
        name.to_string()
    } else {
        format!("{name}.{extension}")
    }
}

/// What a file handed to the page is, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Received {
    /// A `.3dcraft` document: opened.
    Document,
    /// A STEP file: imported into the active document.
    Step,
}

pub fn received_kind(name: &str) -> Option<Received> {
    match extension_of(name)?.as_str() {
        EXTENSION | JIG_EXTENSION | ASSEMBLY_EXTENSION => Some(Received::Document),
        "step" | "stp" | "fcstd" | "brep" | "brp" => Some(Received::Step),
        _ => None,
    }
}

/// The report of an open done a piece per frame, held until its picture
/// is shown ([`WebHost::release_report`]): with the document it opened and
/// what the status bar said when it was held.
#[derive(Clone, Debug)]
struct HeldReport {
    label: String,
    outcome: Result<Value, String>,
    document: Option<u64>,
    before: Option<CommandReport>,
    /// [`WebHost::said`] when it was held: anything said since, the same
    /// words included, is newer.
    said: u64,
}

/// A dialog's answer kept while an edit made where the bodies are waits.
#[derive(Clone, Debug)]
enum HeldAnswer {
    Export(Box<ExportRequest>),
    Name(NameDialog),
}

pub struct WebHost {
    pub session: Session,
    pub shell: Shell,
    pub dialog: Option<NameDialog>,
    /// True when the page names files with the system's file dialogs
    /// (the File System Access API): Save, Save As and Export then ask
    /// through [`Outcome::pick`], not [`NameDialog`].
    pub pickers: bool,
    /// The documents (by id) whose file the page can write to again: Save
    /// goes there without asking. Only with [`WebHost::pickers`].
    pub writable: BTreeSet<u64>,
    /// A file that goes with an export written through a file dialog (an
    /// OBJ's material library), waiting for the person to ask for its own
    /// dialog ([`WebHost::companion_ui`]): a browser shows one file dialog
    /// per click.
    pub companion: Option<Download>,
    /// The export waiting for the system's save dialog: what its export
    /// dialog chose is kept once that names the file
    /// ([`WebHost::export_picked`]).
    pending_export: Option<ExportRequest>,
    /// Exports run where the bodies are (the regenerating worker), by the
    /// number of their command, with what the export dialog asked when
    /// it asked: their files come with the answer ([`WebHost::answered`]).
    deferred_exports: Vec<(u64, Option<ExportRequest>)>,
    /// What the export dialog offers, being read where the bodies are
    /// (`crate::jobs::EXPORT_CHOICES`): the job's number and the document
    /// it is for; the dialog opens with its answer.
    export_choices: Option<(u64, Option<u64>)>,
    /// The Parameters form's command, made where the bodies are: the form
    /// is told its result when the answer comes.
    form_job: Option<u64>,
    /// Since when (egui's clock, s) an edit made where the bodies are
    /// waits ([`WebHost::waiting_ui`]).
    edit_since: Option<f64>,
    /// An edit the worker had was cancelled: the page ends the worker.
    worker_restart: bool,
    /// A dialog over the page answered while an edit waits: done once it
    /// is applied, in the order asked ([`WebHost::pump`]).
    held_answers: Vec<HeldAnswer>,
    /// The long work is done a piece per frame ([`WebHost::prepare`]), as
    /// the page does: its one thread draws between the pieces.
    stepwise: bool,
    /// What the work in progress is about, for the status bar: the command
    /// or `file.open`, and the file's name.
    about: Option<(String, Option<String>)>,
    /// What an open done a piece per frame answered, said once its scene
    /// is there ([`WebHost::release_report`]).
    held_report: Option<HeldReport>,
    /// How many times the host said something in the status bar (a
    /// report, a note, a command done): what a held report compares by.
    said: u64,
    /// A file named in the page's address is being fetched.
    fetching: Option<String>,
    /// The address of a published example to fetch, for the page.
    example_fetch: Option<String>,
    /// The built-in fonts the chosen example waits for, once said; and
    /// those whose fetch failed (the example then runs and says why).
    example_fonts: Option<Vec<String>>,
    fonts_failed: BTreeSet<String>,
    /// The scene to show is not there yet: the window is drawn as it was
    /// (`Shell::ui_away`), with the work over it.
    away: bool,
    /// The whole answer of the last action reported, which the status bar
    /// says in words (`answer::summary`).
    last_answer: Option<Value>,
    /// An example from the start screen being built, a step at a time.
    example_run: Option<ExampleRun>,
}

/// An example from the start screen being built a step at a time
/// ([`WebHost::pump`]): each step is run as a command the person asked
/// for (`Session::execute_deferrable`), so that a step that needs the
/// bodies (a feature's status, a measure, an assertion) is made in the
/// regenerating worker, never on the page's one thread, and the steps
/// after it wait for its answer, as edits asked for while one waits do.
/// Before 2026-11-04 the whole script ran on the page at once: the fuel
/// tank's shell and its measures kept the page from drawing for minutes.
struct ExampleRun {
    example: &'static craft3d_ui_egui::shell::Example,
    /// Its commands and their params, in order.
    steps: Vec<(String, Value)>,
    /// The next step to run.
    next: usize,
    /// The document it is built in (`file.new`).
    doc: u64,
    /// The step handed to the worker, by its job's number, and its index.
    waiting: Option<(u64, usize)>,
}

/// Most steps of an example run on the page in one frame, when none of
/// them needs the bodies (each that does is handed to the worker).
const EXAMPLE_STEPS_A_FRAME: usize = 8;

/// The budget of scenes the page keeps (`Session::set_scene_budget`),
/// where the desktop's default is 1.5 GiB: the scenes of other documents
/// and of earlier states (undo steps) are let go, least recently shown
/// first, until the scenes kept are within it, and the meshes of bodies
/// are kept within a budget of the same size. The active document's own
/// scenes are always kept, beyond the budget if they must be: a document
/// whose scene alone is larger takes what it needs. A WebAssembly module
/// has 4 GiB of memory at most and never gives any back, and with the
/// desktop's budget the scene of every undo step was kept: a Jig's
/// parameters set some hundred times filled it (R1 B7, 2026-10-23: a tab
/// of 9.1 GB after 1 h 20 min). A state let go is meshed again when it is
/// shown again (an undo).
pub const WEB_SCENE_BUDGET: usize = 256 << 20;

/// How long an edit made where the bodies are waits before the page says
/// so with Cancel, in seconds (the work indicator's own wait).
const EDIT_NOTE_AFTER: f64 = 0.35;

/// Most dialog answers kept while an edit waits.
const MAX_HELD: usize = 8;

/// What an edit the person cancelled while it was made says.
pub const CANCELLED: &str = "cancelled: the edit was not made";

impl WebHost {
    /// A host whose session keeps its files in memory, around `shell`,
    /// within the page's memory budgets ([`WEB_SCENE_BUDGET`]; the bodies
    /// a regeneration keeps have their own wasm budget in `craft3d_regen`).
    pub fn new(shell: Shell) -> Self {
        let mut session = Session::new(Workspace::memory());
        session.set_scene_budget(WEB_SCENE_BUDGET);
        Self {
            session,
            shell,
            dialog: None,
            pickers: false,
            writable: BTreeSet::new(),
            companion: None,
            pending_export: None,
            deferred_exports: Vec::new(),
            export_choices: None,
            form_job: None,
            edit_since: None,
            worker_restart: false,
            held_answers: Vec::new(),
            stepwise: false,
            about: None,
            fetching: None,
            example_fetch: None,
            example_fonts: None,
            fonts_failed: BTreeSet::new(),
            away: false,
            last_answer: None,
            example_run: None,
            held_report: None,
            said: 0,
        }
    }

    /// Does the long work (regenerating, meshing, the assertions) a piece
    /// per frame with [`WebHost::prepare`], as the page does: a browser
    /// draws nothing while its one thread works, and a frame that takes
    /// seconds looks frozen. Off by default (the work is done where it is
    /// asked for, all at once).
    pub fn set_stepwise(&mut self, stepwise: bool) {
        self.stepwise = stepwise;
        self.session.set_stepwise(stepwise);
    }

    /// The file `name` named in the page's address is being fetched: shown
    /// as opening until it is there ([`WebHost::receive`]) or failed
    /// ([`WebHost::fetch_failed`]).
    pub fn fetching(&mut self, name: &str) {
        let stem = file_stem(name);
        self.fetching = Some(name.to_string());
        self.about = Some(("file.open".to_string(), stem));
        self.show_work(None);
        self.away = true;
    }

    /// The address of a published example the page should fetch now
    /// (`examples/<file>`, beside the page), once.
    pub fn take_example_fetch(&mut self) -> Option<String> {
        self.example_fetch.take()
    }

    /// Whether a script example must wait for built-in fonts its texts
    /// use that are not here yet (the page fetches them, `crate::fonts`):
    /// each is wanted, and the wait is said once. A font that failed to
    /// come is not waited for: the example runs and says why.
    fn waits_for_fonts(&mut self, example: &craft3d_ui_egui::shell::Example) -> bool {
        let missing: Vec<String> = example
            .fonts()
            .into_iter()
            .filter(|f| !craft3d_engine::fonts::ready(f) && !self.fonts_failed.contains(f))
            // Only a built-in font is fetched; another is the document's.
            .filter(|f| craft3d_engine::fonts::want(f).is_ok())
            .collect();
        if missing.is_empty() {
            return false;
        }
        if self.example_fonts.as_ref() != Some(&missing) {
            self.note(
                "example",
                true,
                format!(
                    "fetching the font {} for {}",
                    missing.join(", "),
                    example.title
                ),
            );
            self.example_fonts = Some(missing);
        }
        true
    }

    /// What came of fetching built-in font `name` (the page's): said, and
    /// a failure remembered so a waiting example runs and says why.
    pub fn font_fetched(&mut self, name: &str, outcome: Result<(), String>) {
        let waiting = self
            .example_fonts
            .as_ref()
            .is_some_and(|f| f.iter().any(|f| f == name));
        match outcome {
            Ok(()) => {
                self.fonts_failed.remove(name);
                if !waiting {
                    self.note(
                        "font",
                        true,
                        format!("the font {name} is here: run the text command again"),
                    );
                }
            }
            Err(why) => {
                self.fonts_failed.insert(name.to_string());
                self.note(
                    "font",
                    false,
                    format!("the font {name} could not be fetched: {why}"),
                );
            }
        }
    }

    /// The name of the file being fetched, while it is.
    pub fn fetching_name(&self) -> Option<&str> {
        self.fetching.as_deref()
    }

    /// The file being fetched could not be had: no longer opening.
    pub fn fetch_failed(&mut self) {
        if self.fetching.take().is_some() {
            self.about = None;
            self.shell.set_work(None);
            self.away = false;
        }
    }

    /// True while the window is drawn as it was, the scene to show not
    /// being there yet (`Shell::ui_away` instead of `Shell::ui`).
    pub fn away(&self) -> bool {
        self.away
    }

    /// Does a piece of the long work the active document still needs
    /// (`Session::prepare_some`), until `stop` says the frame is used up,
    /// and tells the shell what is being done. Called once per frame
    /// before it is drawn. Returns true while work is left (the page asks
    /// for the next frame).
    pub fn prepare(&mut self, stop: impl Fn() -> bool + Clone + 'static) -> bool {
        if self.fetching.is_some() {
            self.show_work(None);
            self.away = true;
            return true;
        }
        // A commit of the parameter form waits a frame for its command
        // (`WebHost::pump`): that frame says so.
        if self.shell.parameters_busy() && self.about.is_none() {
            self.about = Some(("param.set".to_string(), None));
        }
        if !self.stepwise {
            return false;
        }
        let quality = self.shell.scene_quality();
        // Regenerated and meshed in a worker (`scenes.rs`): nothing to do
        // here but say so until the scene is there; the window is drawn
        // as it was meanwhile, and takes edits. No work is left on the
        // page, so no frame is asked for: the worker's answer asks for
        // the frame that takes it (`worker.rs`), and the page looks at
        // the deadline now and then (`page.rs`).
        if self.session.regen_elsewhere() {
            self.away = !self.session.scene_ready(quality);
            self.release_report();
            // A moment of the motion on its way: the player says it plays
            // (`Playing`), and the next one is asked for as soon as it
            // came, so the work never ends while it plays; not said as
            // work, as the desktop does not for a moment's short job.
            let moment = self.session.motion().is_some_and(|m| m.pending);
            if moment || self.session.scene_job(quality).is_none() {
                self.about = None;
                self.shell.set_work(None);
                return false;
            }
            let phase = if self.away {
                "regenerating"
            } else {
                "refining"
            };
            self.show_work(Some(craft3d_engine::Progress {
                phase,
                feature: None,
                name: None,
                step: 0,
                steps: 0,
            }));
            return false;
        }
        let step = self.session.prepare_some(quality, stop);
        self.away = !self.session.scene_ready(quality);
        self.release_report();
        match step {
            None => {
                self.about = None;
                self.shell.set_work(None);
                false
            }
            // The assertions say themselves how far they are (the shell's
            // own status), with the picture shown.
            Some(step) if step.phase == "checking" => {
                self.about = None;
                self.shell.set_work(None);
                true
            }
            Some(step) => {
                self.show_work(Some(step));
                true
            }
        }
    }

    /// Tells the shell what is being done: the step, or only what the
    /// work is about.
    fn show_work(&mut self, step: Option<craft3d_engine::Progress>) {
        let (label, subject) = self
            .about
            .clone()
            .unwrap_or_else(|| ("meshing".to_string(), None));
        let work = match step {
            Some(step) => Work {
                label,
                subject,
                phase: step.phase.to_string(),
                feature: step.feature,
                name: step.name,
                step: step.step,
                steps: step.steps,
                seconds: 0.0,
            },
            None => Work {
                label,
                subject,
                phase: "running".to_string(),
                ..Work::default()
            },
        };
        self.shell.set_work(Some(work));
    }

    /// The index of the open document `id`, as `file.activate` takes it.
    fn index_of(&self, id: u64) -> Option<usize> {
        self.session
            .document_statuses()
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.index)
    }

    /// Saves the document `id` (not necessarily the one shown) as `name`,
    /// which becomes its name; the one shown before is shown again. The
    /// file is the download, with `close` carried for the page: it closes
    /// the document only once the file is written.
    pub fn save_document(&mut self, id: u64, name: &str, close: bool) -> Vec<Download> {
        let shown = self.session.document_status().map(|s| s.id);
        let Some(index) = self.index_of(id) else {
            self.finish(
                "file.save",
                &Err("the document to save was closed meanwhile".to_string()),
            );
            return Vec::new();
        };
        if shown != Some(id) {
            let done = self.run("file.activate", &json!({ "document": index }));
            if let Err(e) = done {
                self.finish("file.activate", &Err(e));
                return Vec::new();
            }
        }
        let mut downloads = self.save_as(name, None);
        if let Some(back) = shown.filter(|&s| s != id).and_then(|s| self.index_of(s)) {
            let report = self.shell.last_command.clone();
            let _ = self.run("file.activate", &json!({ "document": back }));
            self.shell.sync(&mut self.session);
            self.shell.last_command = report;
        }
        for download in &mut downloads {
            download.close = close;
        }
        downloads
    }

    /// Closes the document `id` once its file is written (Save in the
    /// question before closing a tab). It is refused, and says so, when
    /// it was changed again since.
    pub fn close_document(&mut self, id: u64) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let closed = self.run("file.close", &json!({ "document": index }));
        self.finish("file.close", &closed);
    }

    /// The file a save made did not reach the person's file: the document
    /// counts as unsaved again (it asks before it is closed, and the
    /// browser before the page is left), and the next Save asks where.
    pub fn write_failed(&mut self, id: u64) {
        self.session.mark_unsaved(id);
        self.writable.remove(&id);
        self.shell.sync(&mut self.session);
    }

    /// The browser did not show the save dialog for `pick` (`why`): an
    /// export, already made, is downloaded (with its companion); a save
    /// is asked for with the dialog over the page and downloaded.
    pub fn without_picker(&mut self, pick: Pick, why: &str) -> Outcome {
        let mut outcome = Outcome::default();
        let text =
            format!("the browser did not show its file dialog ({why}): the file is downloaded");
        match pick {
            Pick::Export { file } => {
                outcome.downloads.push(file);
                outcome.downloads.extend(self.companion.take());
                // Downloaded: the person has it.
                self.export_picked();
                self.note("file.export", true, text);
            }
            Pick::Save {
                document,
                suggested,
                close,
            } => {
                let shown = self.session.document_status();
                match shown.filter(|s| s.id == document) {
                    Some(status) => {
                        self.dialog = Some(NameDialog {
                            purpose: Purpose::Save {
                                close: close.then_some(status.index),
                            },
                            name: suggested,
                        });
                        self.note("file.save", false, text);
                    }
                    None => self.note_error(
                        "file.save",
                        format!(
                            "the browser did not show its file dialog ({why}), and another \
                             document is shown now: nothing was saved; save again"
                        ),
                    ),
                }
            }
        }
        outcome
    }

    /// Shows `text` in the status bar under `label`, as a command's
    /// report.
    /// A notice (a fallback that still works, such as a download in place
    /// of the system's dialog) stays there; a failure of what the person
    /// asked for is [`Self::note_error`].
    pub fn note(&mut self, label: &str, ok: bool, text: String) {
        self.said = self.said.wrapping_add(1);
        self.shell.last_command = Some(CommandReport {
            label: label.to_string(),
            ok,
            text,
        });
    }

    /// A failure of what the person asked for: in the status bar and in
    /// the error dialog.
    pub fn note_error(&mut self, label: &str, text: String) {
        self.shell.show_error(label, &text);
        self.note(label, false, text);
    }

    fn report(&mut self, label: &str, outcome: &Result<Value, String>) {
        self.said = self.said.wrapping_add(1);
        self.last_answer = outcome.as_ref().ok().cloned();
        let (ok, text) = match outcome {
            Ok(result) => (true, craft3d_ui_egui::answer::summary(label, result)),
            Err(error) => {
                self.shell.show_error(label, error);
                (false, error.clone())
            }
        };
        self.shell.last_command = Some(CommandReport {
            label: label.to_string(),
            ok,
            text: text.chars().take(160).collect(),
        });
    }

    /// Runs a command the person asked for: one that needs the bodies the
    /// page does not have is handed to the worker that has them
    /// (`Session::execute_deferrable`; the answer comes to
    /// [`Self::answered`]). The shell's own reads (what the export
    /// dialog offers) answer at once.
    fn run(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.session.execute_deferrable(command, params)
        }))
        .unwrap_or_else(|_| {
            Err(craft3d_engine::CommandError::new(
                "internal error (a bug in 3DCraft, please report it); the document was kept",
            ))
        })
        .map_err(|e| e.message().to_string())
    }

    /// Reports `outcome` under `label`, empties the memory and shows what
    /// the session holds now.
    fn finish(&mut self, label: &str, outcome: &Result<Value, String>) {
        self.forget_files();
        // Done a piece per frame: shown once the scene is there, and only
        // then said done (the status bar said "Opened" while the page
        // still showed "Opening…", 2026-11-04). A failure is said at once.
        if self.stepwise && !self.session.scene_ready(self.shell.scene_quality()) {
            // Only an open or an import waits: another edit (a parameter
            // set) is said done when it is made, its picture following.
            if outcome.is_ok() && matches!(label, "file.open" | "file.import" | "example") {
                self.last_answer = outcome.as_ref().ok().cloned();
                self.held_report = Some(HeldReport {
                    label: label.to_string(),
                    outcome: outcome.clone(),
                    document: self.session.document_status().map(|s| s.id),
                    before: self.shell.last_command.clone(),
                    said: self.said,
                });
            } else {
                self.report(label, outcome);
            }
            self.away = true;
            return;
        }
        self.report(label, outcome);
        self.shell.sync(&mut self.session);
    }

    /// The report of a file opened a piece per frame, once its scene is
    /// there ([`Self::finish`]).
    /// Said only if nothing was said since (a later open that failed, a
    /// close, a parameter set: those are newer and stay) and the document
    /// it opened is still the one shown; else it is dropped.
    fn release_report(&mut self) {
        let Some(held) = self.held_report.as_ref() else {
            return;
        };
        let shown = self.session.document_status().map(|s| s.id);
        if self.shell.last_command != held.before
            || self.said != held.said
            || shown != held.document
        {
            self.held_report = None;
            return;
        }
        if !self.away
            && let Some(held) = self.held_report.take()
        {
            self.report(&held.label, &held.outcome);
        }
    }

    /// Takes every file out of the session's memory: nothing stays there
    /// after an action.
    fn forget_files(&mut self) -> Vec<Download> {
        let names = self.session.file_names();
        names
            .into_iter()
            .filter_map(|name| {
                let bytes = self.session.take_file(&name)?;
                Some(Download::export(name, bytes))
            })
            .collect()
    }

    /// True when a document has changes that are not saved (downloaded).
    pub fn has_unsaved(&self) -> bool {
        self.session
            .document_statuses()
            .iter()
            .any(|status| status.close_asks)
    }

    /// Opens or imports a file the person handed the page (chosen, dropped,
    /// or named in the address): a `.3dcraft` document is opened, a STEP
    /// file imported into the active document (a new one when there is
    /// none, closed again when the file cannot be imported). `wanted` is
    /// what the person asked for, when they asked (Open or Import): a file
    /// of the other kind is refused then.
    pub fn receive(
        &mut self,
        name: &str,
        bytes: Vec<u8>,
        wanted: Option<Ask>,
    ) -> Result<Value, String> {
        // The file being fetched is here (named as the page names it, or
        // by its path in the address).
        if self
            .fetching
            .as_deref()
            .is_some_and(|f| memory_name(f) == memory_name(name))
        {
            self.fetching = None;
        }
        let label = match received_kind(name) {
            Some(Received::Step) => "file.import",
            _ => "file.open",
        };
        self.about = Some((label.to_string(), file_stem(name)));
        let outcome = self.receive_inner(name, bytes, wanted);
        self.finish(label, &outcome);
        outcome
    }

    fn receive_inner(
        &mut self,
        name: &str,
        bytes: Vec<u8>,
        wanted: Option<Ask>,
    ) -> Result<Value, String> {
        let Some(name) = memory_name(name) else {
            return Err(format!("`{name}` is not a file name"));
        };
        let kind = received_kind(&name).ok_or_else(|| {
            format!("`{name}` is not a 3DCraft document (.3dcraft), a Jig (.3dcraftjig), a STEP file (.step, .stp) or a FreeCAD document (.FCStd)")
        })?;
        match (wanted, kind) {
            (Some(Ask::Open), Received::Step) => {
                return Err(format!(
                    "`{name}` is a STEP file; import it (File, Import…) to add its solids to a document"
                ));
            }
            (Some(Ask::Import), Received::Document) => {
                return Err(format!(
                    "`{name}` is a 3DCraft document; open it (File, Open…)"
                ));
            }
            _ => {}
        }
        self.session
            .put_file(&name, bytes)
            .map_err(|e| e.message().to_string())?;
        match kind {
            Received::Document => self.run("file.open", &json!({ "path": name })),
            Received::Step => {
                let made = if self.session.document_status().is_none() {
                    self.run("file.new", &json!({}))
                        .map_err(|e| format!("file.new: {e}"))?;
                    true
                } else {
                    false
                };
                let imported = self.run("file.import", &json!({ "path": name }));
                if imported.is_err() && made {
                    let _ = self.run("file.close", &json!({ "discard": true }));
                }
                imported
            }
        }
    }

    /// Does what the File menu, its shortcuts and the tabs asked for since
    /// the last frame.
    pub fn pump(&mut self) -> Outcome {
        let mut outcome = Outcome::default();
        // An edit made where the bodies are waits for its answer: what was
        // asked for after it waits too, in its queue, and runs on the
        // document the edit leaves, in the order asked (as the desktop's
        // engine thread runs one task after the other).
        if self.session.edits_waiting() {
            // Open and Import only ask for a file, which the browser lets
            // the page do only in the frame of the click: asked now, the
            // file taken once the edit is done (`page.rs`). The rest waits.
            match self.shell.take_file_action() {
                Some(action @ (FileAction::Open | FileAction::Import)) => {
                    return self.file_action(action);
                }
                Some(action) => self.shell.request_file_action(action),
                None => {}
            }
            return outcome;
        }
        // The dialogs answered while the edit waited: done now, after it,
        // one per frame, in the order answered.
        if !self.held_answers.is_empty() {
            return match self.held_answers.remove(0) {
                HeldAnswer::Export(request) => self.export_request(*request),
                HeldAnswer::Name(dialog) => self.answer(dialog, true),
            };
        }
        if let Some(action) = self.shell.take_file_action() {
            outcome = self.file_action(action);
        }
        if let Some(action) = self.shell.take_doc_action() {
            let more = self.doc_action(action);
            outcome.downloads.extend(more.downloads);
            outcome.ask = outcome.ask.or(more.ask);
            outcome.pick = outcome.pick.or(more.pick);
        }
        // Undo and Redo of the Edit menu and Ctrl+Z, in the order asked,
        // each for the document active when it was asked; not a commit of
        // the Parameters form (no command_done), as on the desktop.
        while let Some(request) = self.shell.take_edit_action() {
            let command = request.action.command();
            let session = &mut self.session;
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                craft3d_ui_egui::run_edit(session, request)
            }))
            .unwrap_or_else(|_| {
                Err("internal error (a bug in 3DCraft, please report it)".to_string())
            });
            self.finish(command, &done);
        }
        // An example from the start screen, built in a new document; a
        // published Jig or assembly is fetched from beside the page first
        // (the page reads `take_example_fetch`) and opened as a file is.
        // One being built goes on (another chosen meanwhile waits).
        if self.example_run.is_some() {
            self.build_example();
        } else if let Some(name) = self.shell.peek_example().and_then(|e| e.to_fetch()) {
            let _ = self.shell.take_example();
            self.fetching(name);
            self.example_fetch = Some(format!("examples/{name}"));
        } else if let Some(example) = self.shell.peek_example()
            && self.waits_for_fonts(example)
        {
            // Its texts' fonts are being fetched: it runs once they are
            // here (a frame after `font_fetched`).
        } else if let Some(example) = self.shell.peek_example()
            && let Some(steps) = craft3d_ui_egui::shell::example_steps(example)
        {
            let _ = self.shell.take_example();
            self.example_fonts = None;
            self.start_example(example, steps);
        } else if let Some(example) = self.shell.take_example() {
            self.example_fonts = None;
            let session = &mut self.session;
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                craft3d_ui_egui::shell::run_example(session, example)
            }))
            .unwrap_or_else(|_| {
                Err("internal error (a bug in 3DCraft, please report it)".to_string())
            });
            self.finish("example", &built);
        }
        // The Parameters tab's param.set: the frame before this one
        // showed that it is regenerating, now the page's one thread does
        // it.
        if let Some(asked) = self.shell.take_command() {
            self.about = Some((asked.command.to_string(), None));
            let done = self.run(asked.command, &asked.params);
            // Made where the bodies are: the form is told the real result
            // when its answer comes ([`Self::answered`]).
            if let Ok(result) = &done
                && let Some(token) = deferred(result)
            {
                self.form_job = Some(token);
                return outcome;
            }
            self.said = self.said.wrapping_add(1);
            self.shell.command_done(done.clone());
            self.finish(asked.command, &done);
        }
        outcome
    }

    /// While an edit made where the bodies are has waited a moment: a
    /// small window that says what it is, with Cancel (the edit is then
    /// not made, and a late answer is dropped). What Cancel leaves for
    /// the page.
    pub fn waiting_ui(&mut self, ctx: &egui::Context) -> Outcome {
        let now = ctx.input(|i| i.time);
        // An example being built shows the window from its first step to
        // its last, with the step it is at.
        let building = self.example_progress().and_then(|(step, of)| {
            let run = self.example_run.as_ref()?;
            Some(format!(
                "the example {}: step {step} of {of}",
                run.example.title
            ))
        });
        let Some(command) = self
            .session
            .edit_waiting()
            .map(str::to_string)
            .or_else(|| building.clone())
        else {
            self.edit_since = None;
            return Outcome::default();
        };
        let since = *self.edit_since.get_or_insert(now);
        if now - since < EDIT_NOTE_AFTER {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return Outcome::default();
        }
        let label = match &building {
            Some(building) => building.as_str(),
            None => Session::command(&command).map_or(command.as_str(), |spec| spec.label),
        };
        let mut cancel = false;
        // A small plate of the kit's (on the panel's ground) at the bottom
        // of the page: the spinner, what it does, Cancel.
        let tokens = self.shell.tokens;
        let kit = Kit::panel(&tokens);
        let fill = if craft3d_ui_egui::theme::is_light(tokens.panel_fill) {
            tokens.panel_fill
        } else {
            tokens.popover_fill
        };
        egui::Area::new(egui::Id::new("craft3d-web-working"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -48.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(craft3d_ui_egui::theme::color(fill))
                    .stroke(egui::Stroke::new(
                        1.0,
                        craft3d_ui_egui::theme::color(tokens.field_stroke),
                    ))
                    .corner_radius(egui::CornerRadius::same(tokens.dialog_radius))
                    .inner_margin(egui::Margin::same(
                        (tokens.dialog_padding * 0.5).clamp(0.0, 32.0) as i8,
                    ))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = tokens.nav_gap;
                            ui.add(
                                egui::Spinner::new()
                                    .color(craft3d_ui_egui::theme::color(tokens.text_dim)),
                            );
                            kit.paragraph(ui, &format!("Working on {label}…"), false);
                            if kit.button(ui, Button::secondary("Cancel")).clicked() {
                                cancel = true;
                            }
                        });
                    });
            });
        if cancel {
            return self.cancel_edits();
        }
        Outcome::default()
    }

    /// Cancels every edit that waits for the worker's answer, and the step
    /// of the example being built (an edit or a read): not made, said so,
    /// and what was kept waiting behind it runs.
    pub fn cancel_edits(&mut self) -> Outcome {
        self.edit_since = None;
        // The step of an example that waits for the worker, an edit or a
        // read (`document.check`), is cancelled with the edits.
        let step = self.example_run.as_ref().and_then(|run| run.waiting);
        // The worker may be working on it, or hang on it: it is ended,
        // and the next job starts a new one ([`Self::take_worker_restart`]).
        self.worker_restart |= self.session.edit_posted()
            || step.is_some_and(|(token, _)| self.session.command_posted(token));
        let mut answered = self.session.edits_failed(CANCELLED);
        if let Some((token, _)) = step
            && !answered.iter().any(|answer| answer.token == token)
        {
            answered.extend(self.session.command_dropped(token, CANCELLED));
        }
        let outcome = self.answered(answered);
        // An example between two steps: not built further.
        if let Some(run) = &self.example_run {
            let index = run.waiting.map_or(run.next, |(_, index)| index);
            let command = run
                .steps
                .get(index)
                .map_or_else(String::new, |(command, _)| command.clone());
            self.example_failed(index, &command, CANCELLED);
        }
        outcome
    }

    /// Starts building `example` from its script's `steps`, in a new
    /// document in millimetres ([`ExampleRun`]).
    fn start_example(
        &mut self,
        example: &'static craft3d_ui_egui::shell::Example,
        steps: Result<Vec<(String, Value)>, String>,
    ) {
        let steps = match steps {
            Ok(steps) => steps,
            Err(why) => return self.finish("example", &Err(why)),
        };
        self.about = Some(("example".to_string(), Some(example.title.to_string())));
        if let Err(why) = self.run("file.new", &json!({ "units": "mm" })) {
            return self.finish("example", &Err(format!("file.new: {why}")));
        }
        let Some(doc) = self.session.document_status().map(|status| status.id) else {
            return self.finish("example", &Err("file.new: no document".to_string()));
        };
        self.example_run = Some(ExampleRun {
            example,
            steps,
            next: 0,
            doc,
            waiting: None,
        });
        self.build_example();
    }

    /// The steps of the example being built that can run now: until one
    /// is handed to the worker (the rest wait for its answer), one made
    /// work on the page (a page that regenerates itself: a frame between
    /// two such steps), or [`EXAMPLE_STEPS_A_FRAME`] of them. Done, it is
    /// reported as the example; a step that fails closes its document
    /// again, as [`craft3d_ui_egui::shell::run_example`] does.
    fn build_example(&mut self) {
        for _ in 0..EXAMPLE_STEPS_A_FRAME {
            let Some(run) = &self.example_run else {
                return;
            };
            if run.waiting.is_some() {
                return;
            }
            let (example, index, total, doc) = (run.example, run.next, run.steps.len(), run.doc);
            if self.session.document_status().map(|status| status.id) != Some(doc) {
                self.example_run = None;
                let why = format!(
                    "example {}: its document was closed, or another one shown, before it was built",
                    example.id
                );
                return self.finish("example", &Err(why));
            }
            let Some((command, params)) = run.steps.get(index).cloned() else {
                self.example_run = None;
                let built = json!({ "example": example.id, "steps": total });
                return self.finish("example", &Ok(built));
            };
            let work = self.session.work_here();
            match self.run(&command, &params) {
                Ok(result) => {
                    let handed = deferred(&result);
                    if let Some(run) = &mut self.example_run {
                        run.next = index.saturating_add(1);
                        run.waiting = handed.map(|token| (token, index));
                    }
                    if handed.is_some() || self.session.work_here() != work {
                        return self.example_shown();
                    }
                }
                Err(why) => return self.example_failed(index, &command, &why),
            }
        }
        self.example_shown();
    }

    /// The example being built, as far as it is: its document shown, and
    /// the step it is at in the status bar.
    fn example_shown(&mut self) {
        let Some((step, of)) = self.example_progress() else {
            return;
        };
        let title = self
            .example_run
            .as_ref()
            .map_or("", |run| run.example.title);
        self.note(
            "example",
            true,
            format!("Building {title}: step {step} of {of}"),
        );
        self.forget_files();
        if !(self.stepwise && !self.session.scene_ready(self.shell.scene_quality())) {
            self.shell.sync(&mut self.session);
        }
    }

    /// Step `index` of the example being built failed with `why`: its
    /// document is closed again and the failure said.
    fn example_failed(&mut self, index: usize, command: &str, why: &str) {
        let Some(run) = self.example_run.take() else {
            return;
        };
        if self.session.document_status().map(|status| status.id) == Some(run.doc) {
            let _ = self.run("file.close", &json!({ "discard": true }));
        }
        let message = craft3d_ui_egui::shell::example_step_failed(run.example, index, command, why);
        self.finish("example", &Err(message));
    }

    /// The step of the example being built (from 1) and how many it has;
    /// `None` when none is being built.
    pub fn example_progress(&self) -> Option<(usize, usize)> {
        self.example_run.as_ref().map(|run| {
            let at = run.waiting.map_or(run.next, |(_, index)| index);
            (at.saturating_add(1).min(run.steps.len()), run.steps.len())
        })
    }

    /// Keeps a dialog's answer until the edit that waits is applied: in
    /// order, at most [`MAX_HELD`] (one more is said, not kept).
    fn hold(&mut self, held: HeldAnswer) {
        if self.held_answers.len() < MAX_HELD {
            self.held_answers.push(held);
        } else {
            self.note_error(
                "file.export",
                "too many dialogs answered while an edit is made; answer it again once it is done"
                    .to_string(),
            );
        }
    }

    /// True once after an edit the worker had was cancelled: the page
    /// ends that worker (`crate::scenes::Scenes::restart`).
    pub fn take_worker_restart(&mut self) -> bool {
        std::mem::take(&mut self.worker_restart)
    }

    /// Does what the File menu asked for, as far as that goes without the
    /// person: a file to choose is asked of the page, a name of the
    /// dialog.
    pub fn file_action(&mut self, action: FileAction) -> Outcome {
        let mut outcome = Outcome::default();
        match action {
            FileAction::New => {
                let made = self.run("file.new", &json!({}));
                self.finish("file.new", &made);
            }
            FileAction::NewAssembly => {
                let made = self.run("assembly.new", &json!({}));
                self.finish("assembly.new", &made);
            }
            FileAction::Open => outcome.ask = Some(Ask::Open),
            FileAction::Import => outcome.ask = Some(Ask::Import),
            FileAction::Save | FileAction::SaveAs => {
                let Some(status) = self.session.document_status() else {
                    self.report(
                        "file.save",
                        &Err("there is no document to save".to_string()),
                    );
                    return outcome;
                };
                // Save goes to the file the document has without asking:
                // a download of it again, or (with the file dialogs) the
                // file it was opened from or saved to, when the page can
                // write there.
                // A Jig in use is never saved over its file: Save saves it
                // as a model, named first.
                let own = status.path.clone().filter(|_| {
                    action == FileAction::Save
                        && !status.jig_in_use
                        && (!self.pickers || self.writable.contains(&status.id))
                });
                match own {
                    Some(own) => outcome.downloads = self.save_as(&own, None),
                    None if self.pickers => {
                        outcome.pick = Some(Pick::Save {
                            document: status.id,
                            suggested: suggested_name(status.path.as_deref()),
                            close: false,
                        });
                    }
                    None => {
                        self.dialog = Some(NameDialog {
                            purpose: Purpose::Save { close: None },
                            name: suggested_name(status.path.as_deref()),
                        });
                    }
                }
            }
            // The export dialog first, the desktop's own (what, the format,
            // the options): its answer is made into the file in the frame
            // of its click ([`Self::dialog_ui`]), then downloaded or, with
            // the file dialogs, named in the system's ([`Pick::Export`]).
            // Without them the dialog asks the name itself.
            FileAction::Export => {
                self.shell.set_export_names_file(!self.pickers);
                if self.shell.export_dialog_open() || self.export_choices.is_some() {
                    return outcome;
                }
                // What the dialog offers needs the bodies (what is
                // selected, the patterns): read where they are when the
                // page does not have them, and the dialog opens with the
                // answer ([`Self::answered`]).
                let document = self.session.document_status().map(|s| s.id);
                if document.is_some()
                    && let Some(token) = self
                        .session
                        .hand_over(crate::jobs::EXPORT_CHOICES, &json!({}))
                {
                    self.export_choices = Some((token, document));
                    return outcome;
                }
                if let Err(e) = self.shell.open_export(&mut self.session) {
                    self.report("file.export", &Err(e));
                }
            }
            FileAction::ExportJig => {
                let Some(status) = self.session.document_status() else {
                    self.report(
                        "jig.export",
                        &Err("there is no document to publish".to_string()),
                    );
                    return outcome;
                };
                self.dialog = Some(NameDialog {
                    purpose: Purpose::ExportJig,
                    name: suggested_jig_name(status.path.as_deref()),
                });
            }
            FileAction::OpenForEditing => {
                let made = self.run("jig.edit", &json!({}));
                self.finish("jig.edit", &made);
            }
        }
        outcome
    }

    /// Publishes the active document as a Jig named `name` (the extension
    /// added when it lacks it): the file is the download.
    pub fn export_jig_as(&mut self, name: &str) -> Vec<Download> {
        let Some(name) = memory_name(name).map(|n| with_extension(&n, JIG_EXTENSION)) else {
            self.finish("jig.export", &Err(format!("`{name}` is not a file name")));
            return Vec::new();
        };
        let exported = self.run("jig.export", &json!({ "path": name }));
        let mut downloads = Vec::new();
        if exported.is_ok()
            && let Some(bytes) = self.session.take_file(&name)
        {
            downloads.push(Download::export(name.clone(), bytes));
        }
        self.finish("jig.export", &exported);
        downloads
    }

    /// What a tab asked for: show a document, close it, or save it (a
    /// download) and close it.
    pub fn doc_action(&mut self, action: DocAction) -> Outcome {
        let mut outcome = Outcome::default();
        match action {
            DocAction::Activate(document) => {
                let done = self.run("file.activate", &json!({ "document": document }));
                self.finish("file.activate", &done);
            }
            DocAction::Close { document, discard } => {
                let done = self.run(
                    "file.close",
                    &json!({ "document": document, "discard": discard }),
                );
                self.finish("file.close", &done);
            }
            DocAction::SaveAndClose(document) => {
                let shown = self.run("file.activate", &json!({ "document": document }));
                if let Err(e) = shown {
                    self.finish("file.activate", &Err(e));
                    return outcome;
                }
                let Some(status) = self.session.document_status() else {
                    return outcome;
                };
                // A Jig in use is saved as a model, named first.
                let own = status.path.clone().filter(|_| {
                    !status.jig_in_use && (!self.pickers || self.writable.contains(&status.id))
                });
                match own {
                    // Written through a file dialog's handle: the page
                    // closes the tab once the write succeeded.
                    Some(own) if self.pickers => {
                        outcome.downloads = self.save_document(status.id, &own, true);
                    }
                    // A download: closed now (the browser has the file).
                    Some(own) => outcome.downloads = self.save_as(&own, Some(document)),
                    None if self.pickers => {
                        self.shell.sync(&mut self.session);
                        outcome.pick = Some(Pick::Save {
                            document: status.id,
                            suggested: suggested_name(status.path.as_deref()),
                            close: true,
                        });
                    }
                    None => {
                        self.shell.sync(&mut self.session);
                        self.dialog = Some(NameDialog {
                            purpose: Purpose::Save {
                                close: Some(document),
                            },
                            name: suggested_name(status.path.as_deref()),
                        });
                    }
                }
            }
        }
        outcome
    }

    /// Saves the active document as `name` (`.3dcraft` added when it has no
    /// extension), which becomes its name: the file is the download. With
    /// `close`, the document is closed once it is saved.
    pub fn save_as(&mut self, name: &str, close: Option<usize>) -> Vec<Download> {
        let Some(name) = memory_name(name).map(|n| with_extension(&n, EXTENSION)) else {
            self.finish("file.save", &Err(format!("`{name}` is not a file name")));
            return Vec::new();
        };
        let id = self.session.document_status().map(|s| s.id);
        let saved = self.run("file.save", &json!({ "path": name, "overwrite": true }));
        let file = self.session.take_file(&name);
        let mut downloads = Vec::new();
        let outcome = match (saved, file) {
            (Ok(result), Some(bytes)) => {
                downloads.push(Download {
                    name: name.clone(),
                    bytes,
                    document: id,
                    close: false,
                });
                match close {
                    Some(_) => {
                        let index = self.session.document_status().map(|s| s.index);
                        self.run("file.close", &json!({ "document": index }))
                            .map(|closed| json!({ "saved": result, "closed": closed }))
                    }
                    None => Ok(result),
                }
            }
            (Ok(_), None) => Err(format!("file.save: `{name}` was not written")),
            (Err(e), _) => Err(e),
        };
        self.finish("file.save", &outcome);
        downloads
    }

    /// Exports to a file named `name`, in the format its extension names:
    /// every body, or the bodies, faces or sketch selected as far as
    /// `file.export` can take them. The file (and an OBJ's material
    /// library) are the downloads.
    pub fn export_as(&mut self, name: &str) -> Vec<Download> {
        let plan = self.shell.state.export.plan();
        let files = self.export_plan(name, plan, true);
        // Exported: the format the next export dialog chooses first.
        if !files.is_empty()
            && let Some(format) = craft3d_ui_egui::files::ExportFormat::of_name(name)
        {
            self.shell.state.export.format = format;
        }
        files
    }

    /// [`Self::export_as`] as the export dialog was answered: what goes
    /// in and the options ([`craft3d_ui_egui::files::export_params`]).
    pub fn export_with(&mut self, name: &str, plan: ExportPlan) -> Vec<Download> {
        self.export_plan(name, plan, false)
    }

    /// [`Self::export_with`], or the plan with what is selected now when
    /// `following` ([`Self::export_as`]). Where the page does not have
    /// the bodies, the whole export is a job of the worker that has them
    /// (`crate::jobs::EXPORT`: its choices resolved there, the file
    /// written there), whose files come with the answer: the page never
    /// regenerates or meshes for it.
    fn export_plan(&mut self, name: &str, mut plan: ExportPlan, following: bool) -> Vec<Download> {
        let Some(name) = memory_name(name) else {
            self.finish("file.export", &Err(format!("`{name}` is not a file name")));
            return Vec::new();
        };
        if self.session.document_status().is_some()
            && let Some(token) = self.session.hand_over(
                crate::jobs::EXPORT,
                &crate::jobs::export_params(&name, &plan, following),
            )
        {
            self.deferred_exports.push((token, None));
            self.note("file.export", true, format!("Exporting `{name}`…"));
            self.forget_files();
            return Vec::new();
        }
        if following {
            plan.what = craft3d_ui_egui::files::following_selection(&mut self.session);
        }
        let exported = export_params(&mut self.session, &name, &plan).and_then(|extra| {
            let mut params = json!({ "path": name });
            if let Some(map) = params.as_object_mut() {
                for (key, value) in extra {
                    map.insert(key.to_string(), value);
                }
            }
            self.run("file.export", &params)
        });
        let mut downloads = Vec::new();
        if let Ok(result) = &exported
            && let Some(token) = deferred(result)
        {
            // Run where the bodies are: the files come with its answer.
            self.deferred_exports.push((token, None));
            self.note("file.export", true, format!("Exporting `{name}`…"));
            self.forget_files();
            return downloads;
        }
        if let Ok(result) = &exported {
            if let Some(bytes) = self.session.take_file(&name) {
                downloads.push(Download::export(name.clone(), bytes));
            }
            if let Some(library) = result.get("mtl").and_then(Value::as_str)
                && let Some(bytes) = self.session.take_file(library)
            {
                downloads.push(Download::export(library.to_string(), bytes));
            }
        }
        self.finish("file.export", &exported);
        downloads
    }

    /// The dialog's answer: the file named, or nothing when it was
    /// cancelled. An export with the file dialogs is made here and left
    /// for the system's save dialog ([`Outcome::pick`]); a file that goes
    /// with it waits in [`WebHost::companion`].
    pub fn answer(&mut self, dialog: NameDialog, confirmed: bool) -> Outcome {
        let mut outcome = Outcome::default();
        if !confirmed {
            let label = match dialog.purpose {
                Purpose::Save { .. } => "file.save",
                Purpose::ExportJig => "jig.export",
            };
            self.report(label, &Err("cancelled".to_string()));
            return outcome;
        }
        match dialog.purpose {
            Purpose::Save { close } => outcome.downloads = self.save_as(&dialog.name, close),
            Purpose::ExportJig => {
                let mut files = self.export_jig_as(&dialog.name).into_iter();
                match files.next() {
                    Some(file) if self.pickers => outcome.pick = Some(Pick::Export { file }),
                    Some(file) => outcome.downloads.push(file),
                    None => {}
                }
            }
        }
        outcome
    }

    /// Draws the small window that offers the file that goes with an
    /// export (an OBJ's material library) while one waits; the file when
    /// the person asked to save it (a click, so the page may show the
    /// system's save dialog for it).
    pub fn companion_ui(&mut self, ctx: &egui::Context) -> Option<Download> {
        let file = self.companion.take()?;
        let mut answer = None;
        let tokens = self.shell.tokens;
        let kit = Kit::panel(&tokens);
        let dialog =
            Dialog::new(egui::Id::new("craft3d-web-companion"), "Material library").closable(false);
        kit.dialog(ctx, dialog, |kit, ui| {
            kit.paragraph(
                ui,
                &format!(
                    "The OBJ's colours are in `{}`. Save it next to the OBJ.",
                    file.name
                ),
                false,
            );
            kit.dialog_footer(ui, |ui| {
                kit.footer_right(ui, |ui| {
                    if kit.button(ui, Button::primary("Save…")).clicked() {
                        answer = Some(true);
                    }
                    if kit.button(ui, Button::secondary("Skip")).clicked() {
                        answer = Some(false);
                    }
                });
            });
        });
        // Only its buttons decide (a click beside it or Escape would drop
        // the file unseen).
        match answer {
            Some(true) => Some(file),
            Some(false) => {
                self.note(
                    "file.export",
                    true,
                    format!("`{}` (the OBJ's colours) was not saved", file.name),
                );
                None
            }
            None => {
                self.companion = Some(file);
                None
            }
        }
    }

    /// The export dialog's answer (`craft3d_ui_egui`'s, the desktop's
    /// own): the file is made now, for the document it was asked for, and
    /// downloaded, or with the file dialogs left for the system's save
    /// dialog ([`Outcome::pick`], that format's type only); a file that
    /// goes with it waits in [`WebHost::companion`].
    pub fn export_request(&mut self, request: ExportRequest) -> Outcome {
        let mut outcome = Outcome::default();
        let shown = self.session.document_status().map(|s| s.id);
        if shown != request.document {
            self.note_error(
                "file.export",
                "another document is shown now than the one the export was asked for: nothing was exported; export again".to_string(),
            );
            return outcome;
        }
        let handed = self.deferred_exports.len();
        let files = self.export_with(&request.name, request.plan.clone());
        if files.is_empty()
            && self.deferred_exports.len() > handed
            && let Some((_, waiting @ None)) = self.deferred_exports.last_mut()
        {
            *waiting = Some(request);
            return outcome;
        }
        // What the dialog chose is kept once the file is the person's: a
        // download now, a file named in the system's save dialog when the
        // page writes it ([`Self::export_picked`]); a cancel or a failure
        // changes nothing.
        self.pending_export = None;
        if files.is_empty() {
            return outcome;
        }
        if self.pickers {
            let mut files = files.into_iter();
            if let Some(file) = files.next() {
                outcome.pick = Some(Pick::Export { file });
            }
            self.companion = files.next();
            self.pending_export = Some(request);
        } else {
            outcome.downloads = files;
            self.shell.export_written(&request);
        }
        outcome
    }

    /// The commands run where the bodies are, answered
    /// (`crate::scenes::Frame::commands`): an export's files are offered
    /// as its export dialog would have offered them (through the system's
    /// save dialog where the page has one; a browser that refuses it now,
    /// so long after the click, gets a download instead); every other
    /// answer is reported as the command's own, and an error of a click
    /// that selects in the status bar only.
    pub fn answered(&mut self, answered: Vec<craft3d_engine::Answered>) -> Outcome {
        let mut outcome = Outcome::default();
        for answer in answered {
            // A step of the example being built, made in the worker: the
            // next steps run now (a failure ends the example).
            if let Some(run) = &mut self.example_run
                && let Some((token, index)) = run.waiting
                && token == answer.token
            {
                run.waiting = None;
                match &answer.result {
                    Ok(_) => self.example_shown(),
                    Err(why) => self.example_failed(index, &answer.command, why),
                }
                continue;
            }
            if let Some((token, document)) = self.export_choices
                && token == answer.token
            {
                self.export_choices = None;
                self.choices_answered(document, answer.result);
                continue;
            }
            let export = self
                .deferred_exports
                .iter()
                .position(|(token, _)| *token == answer.token)
                .map(|at| self.deferred_exports.remove(at));
            let mut files: Vec<Download> = answer
                .files
                .iter()
                .filter_map(|name| {
                    let bytes = self.session.take_file(name)?;
                    Some(Download::export(name.clone(), bytes))
                })
                .collect();
            // The file asked for first, then what goes with it (an OBJ's
            // library), as an export on the page hands them on.
            let asked = answer
                .params
                .get("path")
                .or_else(|| answer.params.get("name"))
                .and_then(Value::as_str);
            files.sort_by_key(|file| Some(file.name.as_str()) != asked);
            let form = self.form_job == Some(answer.token);
            if form {
                self.form_job = None;
                self.said = self.said.wrapping_add(1);
                self.shell.command_done(answer.result.clone());
            }
            // Cancelled by the person: said, no error dialog.
            if matches!(&answer.result, Err(why) if why.ends_with(CANCELLED)) {
                self.note(&answer.command, false, CANCELLED.to_string());
                continue;
            }
            if form {
                self.finish(&answer.command, &answer.result);
                continue;
            }
            let Some((_, request)) = export else {
                if answer.edited {
                    // An edit made where the bodies are, now applied
                    // here: reported, and the window shows the new state
                    // as after any edit.
                    self.finish(&answer.command, &answer.result);
                } else if answer.command == "select.set" {
                    if let Err(why) = &answer.result {
                        self.note("select.set", false, why.clone());
                    }
                } else {
                    self.report(&answer.command, &answer.result);
                }
                continue;
            };
            self.report("file.export", &answer.result);
            if answer.result.is_err() || files.is_empty() {
                continue;
            }
            // Exported: the format the next export dialog chooses first.
            if let Some(format) = files
                .first()
                .and_then(|f| craft3d_ui_egui::files::ExportFormat::of_name(&f.name))
            {
                self.shell.state.export.format = format;
            }
            match request {
                Some(request) if self.pickers => {
                    let mut files = files.drain(..);
                    if let Some(file) = files.next() {
                        outcome.pick = Some(Pick::Export { file });
                    }
                    self.companion = files.next();
                    self.pending_export = Some(request);
                }
                Some(request) => {
                    outcome.downloads.append(&mut files);
                    self.shell.export_written(&request);
                }
                None => outcome.downloads.append(&mut files),
            }
        }
        outcome
    }

    /// What the export dialog offers, read where the bodies are for the
    /// document `document`: the dialog opens with it while that document
    /// is still the one shown; an error says why it cannot.
    fn choices_answered(&mut self, document: Option<u64>, result: Result<Value, String>) {
        let shown = self.session.document_status().map(|s| s.id);
        if shown != document {
            self.note_error(
                "file.export",
                "another document is shown now than the one Export was asked for; export again"
                    .to_string(),
            );
            return;
        }
        let opened = result.and_then(|value| {
            let choices: craft3d_ui_egui::files::ExportChoices = serde_json::from_value(value)
                .map_err(|e| format!("file.export: what to export cannot be read: {e}"))?;
            self.shell.open_export_with(&mut self.session, choices)
        });
        if let Err(e) = opened {
            self.report("file.export", &Err(e));
        }
    }

    /// True while the export dialog's choices are being read where the
    /// bodies are (it opens with the answer).
    pub fn export_choices_waiting(&self) -> bool {
        self.export_choices.is_some()
    }

    /// The system's save dialog named the file of the export made last
    /// ([`Self::export_request`]): what its export dialog chose is kept
    /// for the next export.
    pub fn export_picked(&mut self) {
        if let Some(request) = self.pending_export.take() {
            self.shell.export_written(&request);
        }
    }

    /// Draws the dialog in which the person names a file, while it is open;
    /// what its answer, or the export dialog's (drawn by the shell in this
    /// frame), leaves for the page: in the frame of the click, so that the
    /// browser shows its save dialog.
    pub fn dialog_ui(&mut self, ctx: &egui::Context) -> Outcome {
        if let Some(request) = self.shell.take_export_answer() {
            // An edit waits: the export is of the document it leaves, as
            // on the desktop, and is made once it is there.
            if self.session.edits_waiting() {
                self.hold(HeldAnswer::Export(Box::new(request)));
                return Outcome::default();
            }
            return self.export_request(request);
        }
        let Some(mut dialog) = self.dialog.take() else {
            return Outcome::default();
        };
        let mut answer = None;
        let title = match dialog.purpose {
            Purpose::Save { .. } => "Save As",
            Purpose::ExportJig => "Export as Jig",
        };
        // With the file dialogs, Export goes on to the system's save
        // dialog.
        let verb = match dialog.purpose {
            Purpose::Save { .. } => "Save",
            Purpose::ExportJig if self.pickers => "Export…",
            Purpose::ExportJig => "Export",
        };
        let tokens = self.shell.tokens;
        let kit = Kit::panel(&tokens);
        let shown = kit.dialog(
            ctx,
            Dialog::new(egui::Id::new("craft3d-web-name"), title),
            |kit, ui| {
                // The label column of the About dialog's rows.
                kit.dialog_row(ui, "File name", 96.0, |ui| {
                    let field = kit.text_field(
                        ui,
                        "craft3d-web-name-field",
                        &mut dialog.name,
                        "",
                        None,
                        ui.available_width(),
                    );
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        answer = Some(true);
                    }
                });
                kit.dialog_footer(ui, |ui| {
                    kit.footer_right(ui, |ui| {
                        if kit.button(ui, Button::primary(verb)).clicked() {
                            answer = Some(true);
                        }
                        if kit.button(ui, Button::secondary("Cancel")).clicked() {
                            answer = Some(false);
                        }
                    });
                });
            },
        );
        if answer.is_none() && shown.close {
            answer = Some(false);
        }
        match answer {
            Some(true) if self.session.edits_waiting() => {
                self.hold(HeldAnswer::Name(dialog));
                Outcome::default()
            }
            Some(confirmed) => self.answer(dialog, confirmed),
            None => {
                self.dialog = Some(dialog);
                Outcome::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Portable;
    use craft3d_ui_egui::files::ExportWhat;

    #[test]
    fn the_edit_menu_undoes_and_redoes_in_the_page() {
        let mut host = modelled();
        let before = host.session.document_status().unwrap();
        assert!(before.undo.is_some());
        let id = before.id;
        host.shell
            .request_edit_action(craft3d_ui_egui::EditAction::Undo, id);
        host.pump();
        let after = host.session.document_status().unwrap();
        assert!(after.redo.is_some(), "{after:?}");
        host.shell
            .request_edit_action(craft3d_ui_egui::EditAction::Redo, id);
        host.pump();
        assert_eq!(host.session.document_status().unwrap().undo, before.undo);
    }

    #[test]
    fn the_page_keeps_undo_redo_and_the_file_keys_from_the_browser() {
        for (shift, key) in [
            (false, "z"),
            (true, "z"),
            (false, "y"),
            (false, "e"),
            (false, "i"),
        ] {
            assert!(page_keeps_key(true, false, shift, key), "{shift} {key}");
            assert!(!page_keeps_key(false, false, shift, key), "no Ctrl: {key}");
            assert!(!page_keeps_key(true, true, shift, key), "Alt: {key}");
        }
        // The developer tools and the rest stay the browser's.
        for (shift, key) in [
            (true, "i"),
            (true, "e"),
            (true, "y"),
            (false, "r"),
            (false, "w"),
        ] {
            assert!(!page_keeps_key(true, false, shift, key), "{shift} {key}");
        }
    }

    /// Regenerated in a worker, the page has no work of its own while the
    /// worker builds: `prepare` asks for no frame (the worker's answer
    /// does), while the window is drawn as it was and says it is
    /// regenerating; once the scene is there it is drawn.
    #[test]
    fn regenerated_in_a_worker_the_page_asks_for_no_frames_of_its_own() {
        use craft3d_engine::{scene_begin, scene_next};
        let mut host = host();
        host.set_stepwise(true);
        host.session.set_checks_elsewhere(true);
        host.session.set_regen_elsewhere(true);
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("feature.block", json!({"size": [10, 20, 5]})),
        ] {
            host.session.execute(id, &params).unwrap();
        }
        assert!(host.session.regen_elsewhere());
        assert!(
            !host.prepare(|| false),
            "a frame asked for while the worker builds"
        );
        assert!(host.away());
        assert_eq!(
            host.shell.work().map(|w| w.phase.as_str()),
            Some("regenerating")
        );
        let quality = host.shell.scene_quality();
        let token = host.session.scene_job(quality).unwrap();
        let request = host.session.scene_request(token).unwrap();
        scene_begin(&request.document, &request.work);
        while let Some(part) = scene_next() {
            assert!(host.session.scene_answered(token, Ok(&part)));
        }
        assert!(host.session.scene_done(token));
        assert!(!host.prepare(|| false));
        assert!(!host.away());
        assert!(host.shell.work().is_none());
    }

    /// On the page (a piece per frame) a commit of the Parameters form
    /// runs `param.set` without its regeneration: the frames after it
    /// regenerate a feature at a time (the status bar counts them), then
    /// mesh and show the new state, and the form hears that the commit
    /// went well. The window drawn meanwhile is the one before (away).
    #[test]
    fn a_parameter_set_on_the_page_is_regenerated_between_frames() {
        let mut host = host();
        host.set_stepwise(true);
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "w", "expr": "40 mm"})),
            ("feature.block", json!({"size": ["w", "20 mm", "5 mm"]})),
            (
                "feature.block",
                json!({"size": ["w", "10 mm", "10 mm"], "position": [0, 30, 0]}),
            ),
            (
                "feature.block",
                json!({"size": ["w", "10 mm", "10 mm"], "position": [0, 50, 0]}),
            ),
            (
                "variable.publish",
                json!({"name": "w", "min": "10 mm", "max": "90 mm"}),
            ),
        ] {
            host.session.execute(id, &params).unwrap();
        }
        while host.prepare(|| false) {}
        host.shell.sync(&mut host.session);
        host.shell
            .ui_param_queue(&mut host.session, Some(&json!({"name": "w", "value": 70})))
            .unwrap();
        host.pump();
        let report = host.shell.last_command.clone().unwrap();
        assert_eq!(report.label, "param.set");
        assert!(
            report.ok
                && !craft3d_ui_egui::answer::looks_like_json(&report.text)
                && host
                    .last_answer
                    .as_ref()
                    .is_some_and(|a| a.to_string().contains("\"deferred\":true")),
            "{} {:?}",
            report.text,
            host.last_answer
        );
        assert!(host.away(), "the new state is not there yet");
        let mut said = Vec::new();
        let mut frames = 0;
        while host.prepare(|| true) {
            frames += 1;
            assert!(frames < 50, "{said:?}");
            said.push(
                host.shell
                    .work()
                    .map(|w| (w.phase.clone(), w.step, w.steps)),
            );
        }
        assert_eq!(
            said.first().cloned().flatten(),
            Some(("regenerating".to_string(), 1, 3)),
            "{said:?}"
        );
        assert!(!host.away());
        host.shell.sync(&mut host.session);
        assert!(!host.shell.parameters_busy());
        let list = host.session.execute("param.list", &json!({})).unwrap();
        assert_eq!(list["parameters"][0]["value"], 70.0, "{list}");
    }

    fn host() -> WebHost {
        WebHost::new(Shell::new())
    }

    /// An export while the document is regenerated in the worker and not
    /// on the page: run in the worker (`craft3d_engine::run_command`),
    /// whose answer brings the files, the same bytes the page would have
    /// written; the page never regenerates for it.
    #[test]
    fn an_export_beside_the_page_downloads_what_the_page_would_have() {
        let want = modelled().export_as("plate.obj");
        let mut host = host();
        host.set_stepwise(true);
        host.session.set_checks_elsewhere(true);
        host.session.set_regen_elsewhere(true);
        host.session.set_commands_elsewhere(true);
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            (
                "feature.block",
                json!({"size": [40, 20, 10], "name": "Plate"}),
            ),
            (
                "body.appearance",
                json!({"body": "Plate", "color": "#2060c0"}),
            ),
            // A state not regenerated on the page (the colour, an edit
            // that follows the body, regenerated it there).
            ("variable.set", json!({"name": "k", "expr": "1"})),
            (
                "variable.publish",
                json!({"name": "k", "min": "0", "max": "2"}),
            ),
            ("param.set", json!({"values": {"k": "2"}})),
        ] {
            host.session.execute(id, &params).unwrap();
        }
        let quality = host.shell.scene_quality();
        while let Some(token) = host.session.scene_job(quality) {
            let request = host.session.scene_request(token).unwrap();
            craft3d_engine::scene_begin(&request.document, &request.work);
            while let Some(part) = craft3d_engine::scene_next() {
                host.session.scene_answered(token, Ok(&part));
            }
            host.session.scene_done(token);
        }
        assert!(!host.session.regenerated_here());
        let work = host.session.work_here();
        // Written by the worker, its choices resolved there (2026-11-02):
        // nothing now.
        let now = host.export_as("plate.obj");
        assert!(now.is_empty());
        let mut answered = Vec::new();
        for token in host.session.command_jobs() {
            let request = host.session.command_request(token).unwrap();
            let answer = crate::jobs::run(&request.document, &request.work);
            answered.extend(host.session.command_answered(token, Ok(answer)));
        }
        let mut outcome = host.answered(answered);
        outcome.downloads.splice(0..0, now);
        let files = |d: &[Download]| -> Vec<(String, Vec<u8>)> {
            d.iter()
                .map(|d| (d.name.clone(), d.bytes.clone()))
                .collect()
        };
        assert_eq!(files(&outcome.downloads), files(&want));
        assert!(host.shell.last_command.as_ref().is_some_and(|r| r.ok));
        assert_eq!(host.session.work_here(), work);
    }

    /// Bodies selected, a slider moved, then Export: only the selected
    /// bodies are written, as on a page that regenerates itself. The
    /// export's own reads (what is selected) are made in the worker with
    /// the export (2026-11-02; before, on the page).
    #[test]
    fn a_selection_then_a_slider_then_export_writes_only_the_selection() {
        let steps = [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "k", "expr": "10 mm"})),
            (
                "variable.publish",
                json!({"name": "k", "min": "1 mm", "max": "50 mm"}),
            ),
            (
                "feature.block",
                json!({"size": ["k", 20, 10], "name": "Plate"}),
            ),
            (
                "feature.block",
                json!({"size": [5, 5, 5], "position": [80, 0, 0], "name": "Other"}),
            ),
        ];
        let mut want = host();
        for (id, params) in &steps {
            want.session.execute(id, params).unwrap();
        }
        want.session
            .execute("select.set", &json!({"names": ["Plate"]}))
            .unwrap();
        want.session
            .execute("param.set", &json!({"values": {"k": "12 mm"}}))
            .unwrap();
        let want = want.export_as("part.stl");
        let mut host = host();
        host.set_stepwise(true);
        host.session.set_checks_elsewhere(true);
        host.session.set_regen_elsewhere(true);
        host.session.set_commands_elsewhere(true);
        for (id, params) in &steps {
            host.session.execute(id, params).unwrap();
        }
        let quality = host.shell.scene_quality();
        let settle = |host: &mut WebHost| {
            while let Some(token) = host.session.scene_job(quality) {
                let request = host.session.scene_request(token).unwrap();
                craft3d_engine::scene_begin(&request.document, &request.work);
                while let Some(part) = craft3d_engine::scene_next() {
                    host.session.scene_answered(token, Ok(&part));
                }
                host.session.scene_done(token);
            }
            let mut answered = Vec::new();
            for token in host.session.command_jobs() {
                let request = host.session.command_request(token).unwrap();
                let answer = crate::jobs::run(&request.document, &request.work);
                answered.extend(host.session.command_answered(token, Ok(answer)));
            }
            answered
        };
        // A state not regenerated on the page (adding the blocks
        // regenerated the one before on the page).
        host.session
            .execute("param.set", &json!({"values": {"k": "11 mm"}}))
            .unwrap();
        settle(&mut host);
        assert!(!host.session.regenerated_here());
        let click = host
            .session
            .execute_deferrable("select.set", &json!({"names": ["Plate"]}))
            .unwrap();
        assert_eq!(click["deferred"], true);
        settle(&mut host);
        host.session
            .execute("param.set", &json!({"values": {"k": "12 mm"}}))
            .unwrap();
        settle(&mut host);
        assert!(!host.session.regenerated_here());
        // Written by the worker: only the selection, the page never
        // regenerating for it.
        let work = host.session.work_here();
        let mut written = host.export_as("part.stl");
        assert!(written.is_empty());
        let answered = settle(&mut host);
        written.extend(host.answered(answered).downloads);
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].bytes, want[0].bytes);
        assert_eq!(host.shell.state.export.format.extension(), "stl");
        assert_eq!(host.session.work_here(), work);
    }

    fn modelled() -> WebHost {
        let mut host = host();
        host.session
            .execute("file.new", &json!({"units": "mm"}))
            .unwrap();
        host.session
            .execute(
                "feature.block",
                &json!({"size": [40, 20, 10], "name": "Plate"}),
            )
            .unwrap();
        host.session
            .execute(
                "body.appearance",
                &json!({"body": "Plate", "color": "#2060c0"}),
            )
            .unwrap();
        host
    }

    /// The material library's question is answered by its buttons only:
    /// Escape or a click beside it keeps the file waiting (the review of
    /// ff698f34: it was dropped unseen).
    #[test]
    fn the_material_library_waits_for_a_button() {
        let mut host = self::host();
        host.companion = Some(Download::export(
            "plate.mtl".to_string(),
            b"newmtl a".to_vec(),
        ));
        let ctx = egui::Context::default();
        for events in [
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: egui::pos2(5.0, 5.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: egui::pos2(5.0, 5.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 760.0),
                )),
                events,
                ..Default::default()
            };
            let mut got = None;
            let mut output = ctx.run_ui(input, |ui| got = host.companion_ui(ui.ctx()));
            output.textures_delta.clear();
            assert!(got.is_none());
            assert!(host.companion.is_some(), "kept waiting");
        }
    }

    /// A held "Opened" never replaces what was said after it (the review
    /// of ff698f34): a later open that failed, a close of the tab, a
    /// parameter set; a document no longer shown is not said opened.
    #[test]
    fn a_held_open_report_gives_way_to_newer_ones() {
        let bytes = modelled().save_as("a.3dcraft", None).remove(0).bytes;
        let last = |host: &WebHost| host.shell.last_command.clone();
        let settle = |host: &mut WebHost| {
            let mut frames = 0;
            while host.prepare(|| true) && frames < 60 {
                frames += 1;
            }
        };
        // (a) A, then B that fails: the failure stays.
        let mut host = self::host();
        host.set_stepwise(true);
        host.receive("a.3dcraft", bytes.clone(), None).unwrap();
        assert!(
            host.receive("b.3dcraft", b"not a document".to_vec(), None)
                .is_err()
        );
        let failed = last(&host);
        assert!(failed.as_ref().is_some_and(|c| !c.ok), "{failed:?}");
        settle(&mut host);
        assert_eq!(last(&host), failed);
        // (b) A, then its tab closed: "Closed" stays, A is not said opened.
        let mut host = self::host();
        host.set_stepwise(true);
        host.session.execute("file.new", &json!({})).unwrap();
        host.receive("a.3dcraft", bytes.clone(), None).unwrap();
        let shown = host.session.document_status().unwrap().index;
        host.doc_action(DocAction::Close {
            document: shown,
            discard: true,
        });
        let closed = last(&host);
        settle(&mut host);
        assert_eq!(last(&host), closed);
        assert!(
            last(&host).is_none_or(|c| c.label != "file.open"),
            "{:?}",
            last(&host)
        );
        // (c2) A, then a note in the very words the status bar has: newer
        // all the same, it stays.
        let mut host = self::host();
        host.set_stepwise(true);
        host.note("file.open", true, "Opened a".to_string());
        host.receive("a.3dcraft", bytes.clone(), None).unwrap();
        host.note("file.open", true, "Opened a".to_string());
        let said = host.said;
        settle(&mut host);
        assert_eq!(host.said, said, "the held report was not said again");
        // (c) A, then a note said while it loads: the note stays.
        let mut host = self::host();
        host.set_stepwise(true);
        host.receive("a.3dcraft", bytes, None).unwrap();
        host.note("param.set", true, "Set w".to_string());
        settle(&mut host);
        assert_eq!(last(&host).map(|c| c.text), Some("Set w".to_string()));
    }

    /// What the page shows, frame by frame, while a file it was handed
    /// opens a piece per frame (`WebHost::set_stepwise`): at once that it
    /// is opening (the status bar's words, the bar, nothing drawn from
    /// the session), the features counted as they are built, then the
    /// picture with the assertions being checked, then nothing left. Each
    /// frame does one piece (the stop says so as soon as it may).
    #[test]
    fn a_file_opens_a_piece_per_frame_and_says_so() {
        let mut maker = modelled();
        for (id, params) in [
            (
                "feature.block",
                json!({"size": [10, 10, 10], "position": [60, 0, 0], "name": "B"}),
            ),
            (
                "feature.block",
                json!({"size": [10, 10, 10], "position": [80, 0, 0], "name": "C"}),
            ),
            (
                "assert.add",
                json!({"name": "apart", "measure": "overlap", "is": false}),
            ),
            (
                "assert.add",
                json!({"name": "plate", "measure": "volume", "of": "Plate", "eq": "8000 * (1 mm)^3", "tol": "1e-6 * (1 mm)^3"}),
            ),
        ] {
            maker.session.execute(id, &params).unwrap();
        }
        let bytes = maker.save_as("table.3dcraft", None).remove(0).bytes;

        let mut host = host();
        host.set_stepwise(true);
        host.fetching("/jigs/table.3dcraft");
        assert!(host.away());
        assert_eq!(
            host.shell.busy().map(|(text, _)| text).as_deref(),
            Some("Opening table\u{2026}")
        );
        // Still fetching: nothing to do but say so.
        assert!(host.prepare(|| true));
        host.receive("table.3dcraft", bytes, None).unwrap();
        // Not said opened while it still opens.
        let opened = |host: &WebHost| {
            host.shell
                .last_command
                .as_ref()
                .is_some_and(|c| c.label == "file.open" && c.ok)
        };
        assert!(!opened(&host), "{:?}", host.shell.last_command);
        // A frame as the page draws it (`page.rs`): as it was while the
        // scene is not there, from the session once it is.
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut draw = |host: &mut WebHost| {
            time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 760.0),
                )),
                time: Some(time),
                ..Default::default()
            };
            let away = host.away();
            let mut output = ctx.run_ui(input, |ui| {
                let WebHost { session, shell, .. } = host;
                if away {
                    shell.ui_away(ui);
                } else {
                    shell.ui(ui, session);
                }
            });
            output.textures_delta.clear();
        };
        let mut said = Vec::new();
        let mut frames = 0;
        while host.prepare(|| true) {
            frames += 1;
            assert!(frames < 50, "{said:?}");
            assert_eq!(
                opened(&host),
                !host.away(),
                "said opened only once it is shown"
            );
            draw(&mut host);
            said.push(host.shell.busy().map(|(text, _)| text).unwrap_or_default());
        }
        draw(&mut host);
        assert_eq!(host.shell.busy(), None, "{said:?}");
        assert!(!host.away());
        assert!(opened(&host), "{:?}", host.shell.last_command);
        // The features one by one, then the picture's meshes, then the
        // assertions.
        assert_eq!(
            said.first().map(String::as_str),
            Some("Opening table\u{2026} 1/3")
        );
        assert!(
            said.iter().any(|s| s == "Opening table\u{2026} 2/3"),
            "{said:?}"
        );
        assert!(
            said.iter().any(|s| s == "Opening table\u{2026} meshing"),
            "{said:?}"
        );
        assert!(
            said.iter()
                .any(|s| s == "Checking 2 assertions\u{2026} 1/2"),
            "{said:?}"
        );
        let states = host.session.assertion_states();
        assert!(
            states
                .iter()
                .all(|a| a.status == craft3d_engine::AssertionStatus::Passed)
        );
    }

    /// Export as Jig downloads a `.3dcraftjig`; handed back to the page it
    /// opens as a Jig in use, which Save saves as a model under a name of
    /// its own (never the Jig's) and Open for editing copies into a model.
    #[test]
    fn a_jig_is_exported_opened_for_use_and_saved_as_a_model() {
        let mut host = modelled();
        for (id, params) in [
            ("variable.set", json!({"name": "w", "expr": "40 mm"})),
            ("feature.block", json!({"size": ["w", 10, 2]})),
            (
                "variable.publish",
                json!({"name": "w", "min": "10 mm", "max": "90 mm"}),
            ),
        ] {
            host.session.execute(id, &params).unwrap();
        }
        assert!(host.file_action(FileAction::ExportJig).downloads.is_empty());
        let dialog = host.dialog.take().unwrap();
        assert_eq!(dialog.purpose, Purpose::ExportJig);
        assert_eq!(dialog.name, "untitled.3dcraftjig");
        let outcome = host.answer(
            NameDialog {
                purpose: Purpose::ExportJig,
                name: "plate".to_string(),
            },
            true,
        );
        assert_eq!(outcome.downloads.len(), 1, "{outcome:?}");
        let jig = outcome.downloads[0].clone();
        assert_eq!(jig.name, "plate.3dcraftjig");
        assert!(host.session.file_names().is_empty());
        // Through the system's dialog, offered as a Jig only.
        let pick = Pick::Export { file: jig.clone() };
        assert_eq!(pick.types()[0].extensions, [JIG_EXTENSION]);
        assert!(pick.accepts("a.3dcraftjig") && !pick.accepts("a.3dcraft"));
        // Open accepts it; it opens as a Jig in use.
        assert!(open_types(Ask::Open)[0].extensions.contains(&JIG_EXTENSION));
        assert_eq!(received_kind("x.3DCraftJig"), Some(Received::Document));
        host.receive(&jig.name, jig.bytes.clone(), Some(Ask::Open))
            .unwrap();
        let status = host.session.document_status().unwrap();
        assert!(status.jig_in_use && !status.dirty && !status.close_asks);
        // Save names a model, proposing one.
        assert!(host.file_action(FileAction::Save).downloads.is_empty());
        let dialog = host.dialog.take().unwrap();
        assert_eq!(dialog.name, "plate.3dcraft");
        let saved = host.answer(dialog, true).downloads;
        assert_eq!(saved[0].name, "plate.3dcraft");
        assert!(!host.session.document_status().unwrap().jig_in_use);
        // Open for editing: a new model of a Jig in use.
        host.receive(&jig.name, jig.bytes.clone(), None).unwrap();
        // Changed and closed from its tab: Save names a model.
        host.session
            .execute("param.set", &json!({"values": {"w": 50}}))
            .unwrap();
        let status = host.session.document_status().unwrap();
        assert!(status.close_asks && !status.dirty);
        host.doc_action(DocAction::SaveAndClose(status.index));
        assert_eq!(host.dialog.as_ref().unwrap().name, "plate.3dcraft");
        host.dialog = None;
        host.file_action(FileAction::OpenForEditing);
        let status = host.session.document_status().unwrap();
        assert!(!status.jig_in_use && status.path.is_none());
        assert!(host.shell.last_command.as_ref().unwrap().ok);
    }

    #[test]
    fn names_from_the_browser_become_names_in_memory() {
        assert_eq!(
            memory_name("C:\\Users\\a\\part.3dcraft").as_deref(),
            Some("part.3dcraft")
        );
        assert_eq!(memory_name("dir/a b.step").as_deref(), Some("a b.step"));
        assert_eq!(memory_name("x\u{0}y.stl").as_deref(), Some("xy.stl"));
        for bad in ["", "  ", "..", ".", "a/", "a/.."] {
            assert_eq!(memory_name(bad), None, "{bad:?}");
        }
        assert!(memory_name(&"a".repeat(1000)).is_some_and(|n| n.len() <= MAX_NAME));
        assert_eq!(received_kind("A.3DCRAFT"), Some(Received::Document));
        assert_eq!(received_kind("legs.3DCraftAsm"), Some(Received::Document));
        assert!(
            open_types(Ask::Open)[0]
                .extensions
                .contains(&ASSEMBLY_EXTENSION)
        );
        assert_eq!(received_kind("a.STP"), Some(Received::Step));
        assert_eq!(received_kind("a.stl"), None);
        // An export's name follows its format (the shared rule,
        // `craft3d_ui_egui::files::export_file_name`).
        use craft3d_ui_egui::files::{ExportFormat, export_file_name};
        assert_eq!(export_file_name("a.step", ExportFormat::Stl), "a.stl");
        assert_eq!(with_extension("a", EXTENSION), "a.3dcraft");
        assert_eq!(with_extension("a.3DCRAFT", EXTENSION), "a.3DCRAFT");
        assert_eq!(with_extension("x.stl", EXTENSION), "x.stl.3dcraft");
        assert_eq!(export_file_name("", ExportFormat::Obj), "untitled.obj");
    }

    #[test]
    fn save_downloads_the_document_and_open_reads_it_back() {
        let mut first = modelled();
        // A document without a name asks for one.
        assert!(first.file_action(FileAction::Save).downloads.is_empty());
        let dialog = first.dialog.take().unwrap();
        assert_eq!(dialog.name, "untitled.3dcraft");
        let downloads = first
            .answer(
                NameDialog {
                    name: "plate".to_string(),
                    ..dialog
                },
                true,
            )
            .downloads;
        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].name, "plate.3dcraft");
        assert!(!first.has_unsaved());
        // Saved under another format's name, it stays a .3dcraft file.
        let renamed = first.save_as("plate.stl", None);
        assert_eq!(renamed[0].name, "plate.stl.3dcraft");
        assert!(renamed[0].bytes.starts_with(b"PK"));
        first.save_as("plate.3dcraft", None);
        // Saved once, Save goes to the same name without asking.
        let again = first.file_action(FileAction::Save).downloads;
        assert_eq!(again, downloads);
        assert!(first.session.file_names().is_empty());

        let mut second = host();
        let opened = second
            .receive("plate.3dcraft", downloads[0].bytes.clone(), Some(Ask::Open))
            .unwrap();
        assert!(opened["features"].as_u64() >= Some(1), "{opened}");
        assert!(!second.shell.tree_rows().is_empty());
        assert!(second.session.file_names().is_empty());
        // A file of the other kind than asked for is refused.
        assert!(second.receive("a.step", vec![], Some(Ask::Open)).is_err());
        assert!(
            second
                .receive("a.3dcraft", vec![], Some(Ask::Import))
                .is_err()
        );
        assert!(second.receive("a.txt", vec![1], None).is_err());
        assert!(
            second
                .receive("junk.3dcraft", b"junk".to_vec(), None)
                .is_err()
        );
        assert_eq!(second.session.document_statuses().len(), 1);
    }

    #[test]
    fn export_downloads_every_format_and_an_obj_its_library() {
        let mut host = modelled();
        for (format, extensions) in EXPORT_FORMATS {
            let extension = extensions[0];
            let downloads = host.export_as(&format!("plate.{extension}"));
            match extension {
                // Nothing selected: the model as seen (2026-10-23).
                "dxf" | "svg" => {
                    assert_eq!(downloads.len(), 1, "{format}");
                    let report = host.shell.last_command.as_ref().unwrap();
                    assert!(
                        report.ok
                            && !craft3d_ui_egui::answer::looks_like_json(&report.text)
                            && host
                                .last_answer
                                .as_ref()
                                .is_some_and(|a| a.to_string().contains("\"bodies\":1")),
                        "{} {:?}",
                        report.text,
                        host.last_answer
                    );
                }
                "obj" => {
                    let names: Vec<&str> = downloads.iter().map(|d| d.name.as_str()).collect();
                    assert_eq!(names, ["plate.obj", "plate.mtl"]);
                }
                _ => {
                    assert_eq!(downloads.len(), 1, "{format}");
                    assert!(!downloads[0].bytes.is_empty());
                }
            }
            assert!(host.session.file_names().is_empty(), "{format}");
        }
        // The next export proposes the last format tried (SVG).
        // (the shell's export dialog since 2026-10-27, no longer the
        // page's own name dialog).
        assert_eq!(host.file_action(FileAction::Export), Outcome::default());
        assert!(host.dialog.is_none());
        assert_eq!(host.shell.state.export.name, "untitled.svg");
        host.shell.answer_export(false);
        assert_eq!(host.shell.take_export_answer(), None);
        // A cancelled dialog is no error, and every export above worked
        // (the outlines with nothing selected are drawings of the model).
        assert_eq!(host.shell.error_dialog().map(|e| e.title.as_str()), None);
        // A failed export is an error: an edge selected, to an outline.
        host.session
            .execute(
                "select.query",
                &json!({"kind": "edge", "limit": 1, "then": "replace"}),
            )
            .unwrap();
        assert!(host.export_as("plate.svg").is_empty());
        assert_eq!(
            host.shell.error_dialog().map(|e| e.title.as_str()),
            Some("Could not export")
        );
    }

    /// File › Export opens the shell's export dialog first, as on the
    /// desktop (2026-10-27): what, the format, the quality; its answer is
    /// the download, named for the format, of the document it was asked
    /// for.
    #[test]
    fn the_export_dialog_comes_first_and_exports_what_it_chose() {
        use craft3d_ui_egui::files::{ExportFormat, ExportQuality};
        let mut host = modelled();
        // A face selected: the dialog (the desktop's own) chooses the
        // selection, in DXF; the model is every body, despite the face.
        host.session
            .execute(
                "select.set",
                &json!({"items": [{"kind": "face", "name": "F1:face(0)", "feature": "F1"}]}),
            )
            .unwrap();
        let outcome = host.file_action(FileAction::Export);
        assert_eq!(outcome, Outcome::default(), "nothing before it is answered");
        assert!(host.dialog.is_none(), "not the page's own name dialog");
        assert!(
            host.shell.export_names_file(),
            "downloads: it asks the name"
        );
        assert_eq!(host.shell.state.export.what, ExportWhat::Selection);
        assert_eq!(host.shell.state.export.format, ExportFormat::Dxf);
        host.shell.set_export_what(ExportWhat::Model).unwrap();
        host.shell.set_export_format(ExportFormat::Step).unwrap();
        host.shell.state.export.name = "plate".to_string();
        host.shell.answer_export(true);
        let request = host.shell.take_export_answer().unwrap();
        assert_eq!(request.name, "plate.step");
        let outcome = host.export_request(request);
        assert_eq!(outcome.downloads.len(), 1, "every body, despite the face");
        assert_eq!(outcome.downloads[0].name, "plate.step");
        assert!(host.shell.error_dialog().is_none());
        // A mesh at the quality chosen: coarse, 20 degrees; a name typed
        // with another format's extension is named for the mesh.
        host.file_action(FileAction::Export);
        host.shell.set_export_what(ExportWhat::Model).unwrap();
        host.shell.set_export_format(ExportFormat::Stl).unwrap();
        host.shell.state.export.quality = ExportQuality::Coarse;
        host.shell.state.export.name = "plate.dxf".to_string();
        host.shell.answer_export(true);
        let request = host.shell.take_export_answer().unwrap();
        let outcome = host.export_request(request);
        assert_eq!(outcome.downloads.len(), 1);
        assert_eq!(outcome.downloads[0].name, "plate.stl");
        let report = host.shell.last_command.as_ref().unwrap();
        assert!(
            report.ok
                && !craft3d_ui_egui::answer::looks_like_json(&report.text)
                && host
                    .last_answer
                    .as_ref()
                    .is_some_and(|a| a.to_string().contains("\"angle\":20.0")),
            "{} {:?}",
            report.text,
            host.last_answer
        );
        // The format chosen last is kept for the page's preferences.
        assert_eq!(host.shell.state.export.format, ExportFormat::Stl);
        // Another document shown before the file was made: nothing.
        host.file_action(FileAction::Export);
        host.shell.answer_export(true);
        let request = host.shell.take_export_answer().unwrap();
        host.session.execute("file.new", &json!({})).unwrap();
        let outcome = host.export_request(request);
        assert!(outcome.downloads.is_empty());
        let error = host.shell.error_dialog().unwrap();
        assert!(error.message.starts_with("Another document"), "{error:?}");
    }

    /// The export dialog offers a sheet-metal part's flat pattern; chosen,
    /// the download is the pattern with its bend lines
    /// (repro-sheet-metal-enclosure-1).
    #[test]
    fn the_dialog_exports_a_flat_pattern() {
        let mut host = host();
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("sketch.create", json!({"plane": "xy", "name": "S"})),
            (
                "sketch.rect",
                json!({"sketch": "S", "from": [0, 0], "to": [50, 40]}),
            ),
            (
                "feature.sheetMetal",
                json!({"sketch": "S", "name": "Part", "thickness": "1 mm", "radius": "1 mm",
                    "kFactor": 0.4, "flanges": [{"edge": "e6", "length": "15 mm", "angle": "90 deg"}]}),
            ),
        ] {
            host.session.execute(id, &params).unwrap();
        }
        host.file_action(FileAction::Export);
        let flat = ExportWhat::SheetMetal("Part".to_string());
        host.shell.set_export_what(flat).unwrap();
        host.shell
            .set_export_format(craft3d_ui_egui::files::ExportFormat::Svg)
            .unwrap();
        host.shell.answer_export(true);
        let request = host.shell.take_export_answer().unwrap();
        assert_eq!(request.name, "untitled.svg");
        let outcome = host.export_request(request);
        assert_eq!(outcome.downloads.len(), 1);
        let svg = String::from_utf8_lossy(&outcome.downloads[0].bytes).to_string();
        assert!(svg.contains("bendUp"), "{svg}");
        assert!(host.shell.error_dialog().is_none());
    }

    #[test]
    fn a_step_file_is_imported_into_a_new_document() {
        let mut source = modelled();
        let step = source.export_as("plate.step").remove(0).bytes;
        let mut host = host();
        let imported = host.receive("plate.step", step, Some(Ask::Import)).unwrap();
        assert_eq!(imported["bodies"].as_array().map(Vec::len), Some(1));
        assert_eq!(host.session.document_statuses().len(), 1);
        // A STEP file that does not read leaves no document behind.
        let mut empty = WebHost::new(Shell::new());
        assert!(empty.receive("bad.step", b"junk".to_vec(), None).is_err());
        assert!(empty.session.document_statuses().is_empty());
    }

    #[test]
    fn tabs_save_and_close_through_the_dialog() {
        let mut host = modelled();
        assert!(host.has_unsaved());
        let outcome = host.doc_action(DocAction::SaveAndClose(0));
        assert!(outcome.downloads.is_empty());
        let dialog = host.dialog.take().unwrap();
        assert_eq!(dialog.purpose, Purpose::Save { close: Some(0) });
        // A download: the tab closes at once (the browser has the file).
        let downloads = host.answer(dialog, true).downloads;
        assert_eq!(downloads.len(), 1);
        assert!(!downloads[0].close);
        assert!(host.session.document_statuses().is_empty());
        // Without a document, Save and Export say so.
        host.file_action(FileAction::Export);
        assert!(host.dialog.is_none());
        assert!(!host.shell.last_command.as_ref().unwrap().ok);
        assert_eq!(host.file_action(FileAction::Open).ask, Some(Ask::Open));
        assert_eq!(host.file_action(FileAction::Import).ask, Some(Ask::Import));
    }

    /// The Parameters tab on the page: its commit waits for a frame that
    /// says it is regenerating, then runs in the pump (one thread), and
    /// the form and the status bar hear how it went.
    #[test]
    fn the_parameter_form_commits_in_the_pump_after_a_busy_frame() {
        let mut host = host();
        for (id, params) in [
            ("file.new", json!({"units": "mm"})),
            ("variable.set", json!({"name": "w", "expr": "40 mm"})),
            ("feature.block", json!({"size": ["w", "20 mm", "5 mm"]})),
            (
                "variable.publish",
                json!({"name": "w", "min": "10 mm", "max": "90 mm"}),
            ),
            ("param.set", json!({"values": {"w": 70}})),
        ] {
            host.session.execute(id, &params).unwrap();
        }
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut frame = |host: &mut WebHost, events: Vec<egui::Event>| {
            time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 760.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            };
            host.pump();
            let mut output = ctx.run_ui(input, |ui| {
                let WebHost { session, shell, .. } = host;
                shell.ui(ui, session);
            });
            output.textures_delta.clear();
        };
        for _ in 0..4 {
            frame(&mut host, Vec::new());
        }
        let reset = host
            .shell
            .parameter_hits()
            .iter()
            .find(|(name, _)| name == "Reset all")
            .map(|(_, rect)| rect.center())
            .unwrap();
        for pressed in [true, false] {
            frame(
                &mut host,
                vec![egui::Event::PointerButton {
                    pos: reset,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        // Asked, not run yet: the next frame shows it is busy first.
        assert!(host.shell.parameters_busy());
        frame(&mut host, Vec::new());
        assert!(host.shell.parameters_busy(), "drawn busy, not yet run");
        frame(&mut host, Vec::new());
        assert!(!host.shell.parameters_busy());
        let list = host.session.execute("param.list", &json!({})).unwrap();
        assert_eq!(list["parameters"][0]["value"], 40.0, "{list}");
        let report = host.shell.last_command.clone().unwrap();
        assert_eq!(report.label, "param.set");
        assert!(report.ok, "{}", report.text);
    }

    #[test]
    fn with_file_dialogs_save_asks_once_and_then_writes_back() {
        let mut host = modelled();
        host.pickers = true;
        let id = host.session.document_status().unwrap().id;
        // The first Save names the file in the system's dialog, not ours.
        let outcome = host.file_action(FileAction::Save);
        assert!(host.dialog.is_none() && outcome.downloads.is_empty());
        let pick = outcome.pick.unwrap();
        assert_eq!(
            pick,
            Pick::Save {
                document: id,
                suggested: "untitled.3dcraft".to_string(),
                close: false
            }
        );
        assert_eq!(pick.types().len(), 1);
        assert_eq!(pick.types()[0].extensions, [EXTENSION]);
        // Only a .3dcraft name holds a document.
        assert!(pick.accepts("plate.3dcraft") && pick.accepts("P.3DCRAFT"));
        assert!(!pick.accepts("plate.stl") && !pick.accepts("plate"));
        // The page saves under the name chosen and can write there again.
        let saved = host.save_document(id, "plate.3dcraft", false);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].document, Some(id));
        host.writable.insert(id);
        // Save now writes back without asking; Save As still asks.
        let again = host.file_action(FileAction::Save);
        assert!(again.pick.is_none());
        assert_eq!(again.downloads[0].name, "plate.3dcraft");
        assert_eq!(again.downloads[0].document, Some(id));
        assert!(host.file_action(FileAction::SaveAs).pick.is_some());
        // A document with a name the page cannot write to (dropped, or
        // opened from the address) asks where to save.
        host.writable.clear();
        let asked = host.file_action(FileAction::Save).pick.unwrap();
        assert_eq!(asked.suggested(), "plate.3dcraft");
        assert!(host.dialog.is_none());
    }

    #[test]
    fn a_tab_closes_only_once_its_file_is_written() {
        let mut host = modelled();
        host.pickers = true;
        let id = host.session.document_status().unwrap().id;
        host.save_document(id, "plate.3dcraft", false);
        host.writable.insert(id);
        host.session
            .execute("feature.block", &json!({"size": [1, 1, 1]}))
            .unwrap();
        // Save in the question before closing: the file is made, the tab
        // stays until the page says the write succeeded.
        let outcome = host.doc_action(DocAction::SaveAndClose(0));
        assert_eq!(outcome.downloads.len(), 1);
        assert!(outcome.downloads[0].close);
        assert_eq!(host.session.document_statuses().len(), 1);
        // The write failed: still open, and unsaved again, so nothing is
        // lost and the browser still asks before the page is left.
        host.write_failed(id);
        assert_eq!(host.session.document_statuses().len(), 1);
        assert!(host.has_unsaved());
        assert!(!host.writable.contains(&id));
        // Without a file it can write to, the next try asks where.
        let pick = host.doc_action(DocAction::SaveAndClose(0)).pick.unwrap();
        assert!(matches!(pick, Pick::Save { close: true, .. }));
        // Written this time: the page closes it.
        let saved = host.save_document(id, "plate.3dcraft", true);
        assert!(saved[0].close && !host.has_unsaved());
        host.close_document(id);
        assert!(host.session.document_statuses().is_empty());
        host.close_document(id);
    }

    #[test]
    fn a_document_not_shown_is_saved_and_the_shown_one_stays() {
        let mut host = modelled();
        host.pickers = true;
        let first = host.session.document_status().unwrap().id;
        host.session.execute("file.new", &json!({})).unwrap();
        let second = host.session.document_status().unwrap().id;
        // The dialog was answered while the second document was shown.
        let saved = host.save_document(first, "plate.3dcraft", false);
        assert_eq!(saved[0].document, Some(first));
        assert_eq!(host.session.document_status().unwrap().id, second);
        let statuses = host.session.document_statuses();
        let saved_one = statuses.iter().find(|s| s.id == first).unwrap();
        assert_eq!(saved_one.path.as_deref(), Some("plate.3dcraft"));
        assert!(!saved_one.dirty);
        // A document closed meanwhile is said, not saved.
        assert!(host.save_document(9_999_999, "x.3dcraft", false).is_empty());
        assert!(!host.shell.last_command.as_ref().unwrap().ok);
    }

    /// File › Export answered in the export dialog as `what` in `format`:
    /// what the page is left to do.
    fn exported(
        host: &mut WebHost,
        what: ExportWhat,
        format: craft3d_ui_egui::files::ExportFormat,
    ) -> Outcome {
        assert_eq!(host.file_action(FileAction::Export), Outcome::default());
        host.shell.set_export_what(what).unwrap();
        host.shell.set_export_format(format).unwrap();
        host.shell.answer_export(true);
        let request = host.shell.take_export_answer().unwrap();
        host.export_request(request)
    }

    #[test]
    fn with_file_dialogs_the_export_is_made_before_the_dialog() {
        use craft3d_ui_egui::files::ExportFormat;
        let mut host = modelled();
        host.pickers = true;
        host.shell.state.export.format = ExportFormat::Stl;
        // The format is chosen first, in the export dialog over the page,
        // which does not ask the name: the system's dialog does.
        assert!(host.file_action(FileAction::Export).pick.is_none());
        assert!(host.dialog.is_none() && !host.shell.export_names_file());
        assert_eq!(host.shell.state.export.name, "untitled.stl");
        host.shell.answer_export(true);
        let request = host.shell.take_export_answer().unwrap();
        let outcome = host.export_request(request);
        assert!(outcome.downloads.is_empty());
        let pick = outcome.pick.unwrap();
        let Pick::Export { file } = &pick else {
            panic!("{pick:?}")
        };
        assert_eq!(file.name, "untitled.stl");
        assert!(!file.bytes.is_empty() && file.document.is_none());
        // The save dialog offers that format only, and takes no other.
        let types = pick.types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].extensions, ["stl"]);
        assert!(types[0].mime.contains('/'));
        assert!(pick.accepts("part.STL") && !pick.accepts("part.step"));
        // An export that fails shows no dialog at all: an edge selected
        // cannot go to an outline (the dialog no longer offers it; the
        // selection changed after it was answered).
        host.session
            .execute(
                "select.set",
                &serde_json::json!({"items": [{"kind": "face", "name": "F1:face(0)", "feature": "F1"}]}),
            )
            .unwrap();
        assert_eq!(host.file_action(FileAction::Export), Outcome::default());
        host.shell.set_export_what(ExportWhat::Selection).unwrap();
        host.shell.answer_export(true);
        let request = host.shell.take_export_answer().unwrap();
        host.session
            .execute(
                "select.query",
                &serde_json::json!({"kind": "edge", "limit": 1, "then": "replace"}),
            )
            .unwrap();
        let failed = host.export_request(request);
        assert_eq!(failed, Outcome::default());
        assert!(!host.shell.last_command.as_ref().unwrap().ok);
        host.session
            .execute("select.set", &serde_json::json!({"mode": "clear"}))
            .unwrap();
        // An OBJ's material library waits for its own click.
        let obj = exported(&mut host, ExportWhat::Model, ExportFormat::Obj);
        assert!(matches!(obj.pick, Some(Pick::Export { ref file }) if file.name == "untitled.obj"));
        assert_eq!(host.companion.as_ref().unwrap().name, "untitled.mtl");
        let step = Pick::Export {
            file: Download::export("a.step".to_string(), vec![1]),
        };
        assert!(step.accepts("a.stp") && step.accepts("a.step"));
        assert_eq!(received_kind("bracket.FCStd"), Some(Received::Step));
        assert_eq!(export_types(None)[0].extensions, ["step", "stp"]);
        assert_eq!(export_types(Some("svg"))[0].description, "SVG");
        assert_eq!(open_types(Ask::Import)[0].extensions, ["step", "stp"]);
        // Open takes a document, a published Jig (2026-10-23) or an
        // assembly (2026-10-30).
        assert_eq!(
            open_types(Ask::Open)[0].extensions,
            [EXTENSION, JIG_EXTENSION, ASSEMBLY_EXTENSION]
        );
    }

    #[test]
    fn a_refused_file_dialog_falls_back_to_downloads() {
        // Chrome's words for a click that came too long ago, and for good.
        assert!(refusal_is_about_activation(
            "SecurityError: Must be handling a user gesture to show a file picker."
        ));
        assert!(refusal_is_about_activation(
            "SecurityError: no user activation"
        ));
        assert!(!refusal_is_about_activation(
            "SecurityError: Cross origin sub frames aren't allowed to show a file picker."
        ));
        assert!(!refusal_is_about_activation("TypeError: bad options"));
        let mut host = modelled();
        host.pickers = true;
        // An export already made is downloaded, with its companion.
        let obj = exported(
            &mut host,
            ExportWhat::Model,
            craft3d_ui_egui::files::ExportFormat::Obj,
        )
        .pick
        .unwrap();
        let outcome = host.without_picker(obj, "SecurityError: no");
        let names: Vec<&str> = outcome.downloads.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["untitled.obj", "untitled.mtl"]);
        assert!(host.companion.is_none());
        // A save is named in the dialog over the page.
        let id = host.session.document_status().unwrap().id;
        let save = Pick::Save {
            document: id,
            suggested: "a.3dcraft".to_string(),
            close: true,
        };
        assert!(host.without_picker(save.clone(), "x").downloads.is_empty());
        assert_eq!(
            host.dialog.take().unwrap().purpose,
            Purpose::Save { close: Some(0) }
        );
        // Not when another document is shown by then: said, not guessed.
        host.session.execute("file.new", &json!({})).unwrap();
        host.without_picker(save, "x");
        assert!(host.dialog.is_none());
        assert!(!host.shell.last_command.as_ref().unwrap().ok);
    }

    #[test]
    fn an_example_from_the_start_screen_is_built_in_a_new_document() {
        let mut host = host();
        let example = &craft3d_ui_egui::shell::EXAMPLES[0];
        host.shell.choose_example(example);
        // Built a few steps a frame (`build_example`).
        host.pump();
        while host.example_progress().is_some() {
            host.pump();
        }
        assert_eq!(host.session.document_statuses().len(), 1);
        let report = host.shell.last_command.as_ref().unwrap();
        assert!(report.ok && report.label == "example", "{}", report.text);
        // Nothing is left in memory, and nothing is recent in the browser.
        assert!(host.session.file_names().is_empty());
        assert!(host.shell.recent_files().is_empty());
    }

    /// A script example whose text needs a built-in font that is not here
    /// (on the page it is fetched on first use) waits for it, then runs:
    /// it does not fail and close its new document on the first click.
    #[test]
    fn an_example_with_text_waits_for_its_font_then_builds() {
        let mut host = host();
        let example = craft3d_ui_egui::shell::EXAMPLES
            .iter()
            .find(|e| e.id == "nameplate")
            .unwrap();
        assert_eq!(example.fonts(), ["sans"]);
        host.shell.choose_example(example);
        if !craft3d_engine::fonts::ready("sans") {
            host.pump();
            // Waiting: no document yet, the font wanted, the wait said.
            assert!(host.session.document_statuses().is_empty());
            assert!(
                craft3d_engine::fonts::wanted()
                    .iter()
                    .any(|(name, _)| *name == "sans")
            );
            let report = host.shell.last_command.as_ref().unwrap();
            assert!(report.ok && report.text.contains("fetching the font sans"));
            host.pump();
            assert!(host.session.document_statuses().is_empty());
            // The page fetched it.
            let (_, file) = craft3d_engine::fonts::wanted()
                .into_iter()
                .find(|(name, _)| *name == "sans")
                .unwrap();
            // Where the engine finds it natively: CRAFT3D_FONTS, else the
            // checkout's assets/fonts (`cargo xtask fonts`).
            let dir = std::env::var_os("CRAFT3D_FONTS")
                .filter(|d| !d.is_empty())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts")
                });
            let bytes = std::fs::read(dir.join(file)).unwrap();
            host.font_fetched("sans", craft3d_engine::fonts::register("sans", &bytes));
        }
        host.pump();
        while host.example_progress().is_some() {
            host.pump();
        }
        assert_eq!(host.session.document_statuses().len(), 1);
        let report = host.shell.last_command.as_ref().unwrap();
        assert!(report.ok && report.label == "example", "{}", report.text);
    }

    /// A font that cannot be fetched is not waited for: the example runs
    /// and says why it failed.
    #[test]
    fn a_font_that_failed_lets_the_example_run_and_say_why() {
        let mut host = host();
        host.font_fetched("serif", Err("offline".into()));
        assert!(!host.shell.last_command.as_ref().unwrap().ok);
        assert!(host.fonts_failed.contains("serif"));
        host.font_fetched("serif", Ok(()));
        assert!(!host.fonts_failed.contains("serif"));
    }

    /// An assembly on a page without the worker (none on the page,
    /// `?pageregen`, or sent home: `Session::regen_elsewhere` is off): it
    /// is solved and placed on the page, a piece per frame; `joint.set`
    /// and the motion over `t` move what the page shows, and its tree
    /// lists the instances and mates. With the worker it is placed there
    /// (`beside_tests.rs` › `an_assembly_is_placed_and_played_in_the_worker`;
    /// before 2026-11-03 an assembly was always placed on the page).
    #[test]
    fn an_assembly_is_solved_and_placed_on_the_page() {
        let mut host = host();
        host.set_stepwise(true);
        host.session.set_checks_elsewhere(true);
        let run = |host: &mut WebHost, id: &str, params: Value| {
            host.session
                .execute(id, &params)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
        };
        run(&mut host, "file.new", json!({}));
        run(
            &mut host,
            "variable.set",
            json!({"name": "len", "expr": "24 mm"}),
        );
        run(
            &mut host,
            "variable.publish",
            json!({"name": "len", "min": "10 mm", "max": "500 mm"}),
        );
        run(
            &mut host,
            "feature.block",
            json!({"name": "Bar", "size": ["len", 4, 2]}),
        );
        run(
            &mut host,
            "datum.axis",
            json!({"name": "End1", "through": [[2, 2, 0], [2, 2, 1]]}),
        );
        run(
            &mut host,
            "datum.axis",
            json!({"name": "End2", "through": [["len - 2 mm", 2, 0], ["len - 2 mm", 2, 1]]}),
        );
        run(&mut host, "file.save", json!({"path": "bar.3dcraft"}));
        run(&mut host, "assembly.new", json!({}));
        for (name, len) in [
            ("Ground", "64 mm"),
            ("Crank", "24 mm"),
            ("Coupler", "64 mm"),
            ("Rocker", "24 mm"),
        ] {
            run(
                &mut host,
                "instance.add",
                json!({"source": "bar.3dcraft", "name": name, "body": "Bar", "params": {"len": len}}),
            );
        }
        for (name, a, b) in [
            ("Base", "world/z", "Ground/End1"),
            ("Drive", "Ground/End1", "Crank/End1"),
            ("Elbow", "Crank/End2", "Coupler/End1"),
            ("Knee", "Coupler/End2", "Rocker/End2"),
            ("Hip", "Ground/End2", "Rocker/End1"),
        ] {
            let kind = if name == "Base" { "fixed" } else { "revolute" };
            run(
                &mut host,
                "mate.add",
                json!({"kind": kind, "a": a, "b": b, "name": name, "flip": true}),
            );
        }
        assert!(!host.session.regen_elsewhere());
        let crank_at = |host: &mut WebHost| -> [f64; 2] {
            for _ in 0..1000 {
                if !host.prepare(|| false) {
                    break;
                }
            }
            host.shell.sync(&mut host.session);
            let scene = host.shell.shown_scene_arc().expect("a scene");
            let crank = scene
                .bodies
                .iter()
                .find(|b| b.name == "Crank")
                .expect("Crank shown");
            let (lo, hi) = crank.mesh.bbox().corners().expect("a box");
            [(lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0]
        };
        for angle in [60.0_f64, 150.0] {
            run(
                &mut host,
                "joint.set",
                json!({"mate": "Drive", "value": format!("{angle} deg")}),
            );
            let mid = crank_at(&mut host);
            let a = angle.to_radians();
            assert!(
                (mid[0] - 10.0 * a.pcos()).abs() < 1.0 && (mid[1] - 10.0 * a.psin()).abs() < 1.0,
                "{angle} deg: the crank is at {mid:?}"
            );
        }
        let names: Vec<String> = host
            .shell
            .tree_rows()
            .iter()
            .map(|r| r.name.clone())
            .collect();
        assert_eq!(names, ["Ground", "Crank", "Coupler", "Rocker"]);
        assert_eq!(
            host.shell.assembly_outline().map(|o| o.mates.len()),
            Some(5)
        );
        // The motion over t, solved on the page.
        run(&mut host, "variable.set", json!({"name": "t", "expr": "0"}));
        run(
            &mut host,
            "variable.set",
            json!({"name": "Drive_angle", "expr": "360 deg * t"}),
        );
        run(&mut host, "view.motion", json!({"t": 0.25}));
        let mid = crank_at(&mut host);
        assert!(
            mid[0].abs() < 1.0 && (mid[1] - 10.0).abs() < 1.0,
            "t = 0.25: {mid:?}"
        );
    }
}
