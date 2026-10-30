//! New, Open, Save, Save As, Import and Export for the person at the
//! window.
//!
//! The engine's file commands run under the session's [`Workspace`]: the
//! automation roots, which keep a script or an agent on the control
//! channel inside the directories it was given. A person who picks a file
//! in the system's dialog has chosen that file themselves, and may open,
//! save, import or export it wherever they can: so the same commands
//! (`file.open`, `file.save`, `file.import`, `file.export`; everything is
//! a command) are run for them with a grant for that one choice.
//!
//! The grant is as narrow and as short as the engine can express:
//!
//! - one direction (reading to open or import, writing to save or
//!   export), the other denied;
//! - one directory, the one the chosen file is in;
//! - one command: the automation workspace is put back before this
//!   module returns, whether the command worked, failed or panicked.
//!
//! It cannot be reached from the control channel. Requests are answered on
//! the thread that owns the session, between frames, one at a time, and so
//! is this: no request runs while the grant is in place. There is no
//! method, parameter or MCP tool that asks for it; only a file action of
//! the shell (its menu and shortcuts) and a path that came out of the
//! dialog lead here. What a request can do afterwards is what it could do
//! before: a document that was opened from outside the roots can be
//! edited by an agent (it is the document in the window), but saved by one
//! only to a path beneath the write root. An imported file's bytes are
//! kept in the document (`file.import`), so an agent works with the
//! imported bodies without any access to the file itself.
//!
//! The dialog itself runs on its own thread, so the window keeps drawing
//! and answering requests while it is open ([`Picker`]). It is behind a
//! trait ([`Dialogs`]): tests answer for the person.

use crate::control::Wake;
use crate::host::Host;
use craft3d_engine::{Access, CommandError, Session, Workspace};
pub use craft3d_ui_egui::files::{ExportFormat, ExportPlan, ExportWhat};
use craft3d_ui_egui::shell::ExportRequest;
use craft3d_ui_egui::{CommandReport, FileAction};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub use craft3d_ui_egui::files::{
    EXPORT_FORMATS, EXTENSION, IMPORT_EXTENSIONS, JIG_EXTENSION, OPEN_EXTENSIONS,
    suggested_export_name, suggested_jig_name, suggested_name,
};

/// The system's file dialogs. Each call blocks until the person chose or
/// cancelled (`None`).
pub trait Dialogs: Send + Sync {
    /// Asks for a file to open.
    fn open(&self) -> Option<PathBuf>;
    /// Asks where to save, proposing `suggested` as the file's name.
    fn save(&self, suggested: &str) -> Option<PathBuf>;
    /// Asks for a file to import ([`IMPORT_EXTENSIONS`]).
    fn import(&self) -> Option<PathBuf>;
    /// Asks where to export a file of `format` (that format's filter
    /// only), proposing `suggested` as the file's name, in `folder` when
    /// one is given (the last export's).
    fn export(
        &self,
        suggested: &str,
        format: ExportFormat,
        folder: Option<&Path>,
    ) -> Option<PathBuf>;
    /// Asks where to publish the document as a Jig ([`JIG_EXTENSION`]);
    /// none by default (a host without that dialog).
    fn export_jig(&self, suggested: &str) -> Option<PathBuf> {
        let _ = suggested;
        None
    }
}

/// The dialogs of the desktop (the portal's on Linux).
pub struct SystemDialogs;

impl Dialogs for SystemDialogs {
    fn open(&self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Open")
            .add_filter("3DCraft document, Jig or assembly", &OPEN_EXTENSIONS)
            .add_filter("3DCraft Jig", &[JIG_EXTENSION])
            .add_filter("All files", &["*"])
            .pick_file()
    }

    fn save(&self, suggested: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Save As")
            .add_filter("3DCraft document", &[EXTENSION])
            .set_file_name(suggested)
            .save_file()
    }

    fn export_jig(&self, suggested: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Export as Jig")
            .add_filter("3DCraft Jig", &[JIG_EXTENSION])
            .set_file_name(suggested)
            .save_file()
    }

    fn import(&self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Import")
            .add_filter("STEP", &IMPORT_EXTENSIONS)
            .add_filter("All files", &["*"])
            .pick_file()
    }

    fn export(
        &self,
        suggested: &str,
        format: ExportFormat,
        folder: Option<&Path>,
    ) -> Option<PathBuf> {
        // The format was chosen in the export dialog: its filter only (a
        // portal does not say which filter was picked in it).
        let mut dialog = rfd::FileDialog::new()
            .set_title(format!("Export {}", format.label()))
            .add_filter(format.label(), format.extensions())
            .set_file_name(suggested);
        if let Some(folder) = folder.filter(|f| f.is_dir()) {
            dialog = dialog.set_directory(folder);
        }
        dialog.save_file()
    }
}

/// What came out of a dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Open(PathBuf),
    Save(PathBuf),
    Import(PathBuf),
    /// Where to export what the export dialog chose.
    Export(Box<ExportJob>),
    /// Where to publish the document as a Jig (`jig.export`).
    ExportJig(PathBuf),
    /// The dialog was closed without a choice.
    Cancelled(&'static str),
}

/// An export the export dialog asked for, with the file the save dialog
/// chose for it ([`Host::export_target`] checks it before it is written).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportJob {
    /// The file as the save dialog gave it, before its extension is
    /// corrected for the format.
    pub path: PathBuf,
    /// The export dialog's answer: its format, its document, what it
    /// chose (kept for the next export once the file is written).
    pub request: ExportRequest,
    /// `file.export`'s parameters besides `path`, made when Export… was
    /// clicked: a selection changed while the save dialog is open does not
    /// change what the file holds.
    pub params: ExportParams,
}

/// `file.export`'s parameters besides `path`.
pub type ExportParams = Vec<(&'static str, Value)>;

/// An export being written: what to keep once it is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportDone {
    pub request: ExportRequest,
    /// The folder it is written in: the next save dialog opens there.
    pub folder: Option<PathBuf>,
}

/// `path` named for `format`: its extension appended or corrected
/// ([`craft3d_ui_egui::files::export_file_name`]), so a portal that gave
/// back the name without one, or with another format's, never gets STL
/// bytes in a `.dxf` file.
pub fn export_path(path: &Path, format: ExportFormat) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    path.with_file_name(craft3d_ui_egui::files::export_file_name(&name, format))
}

/// Shows one dialog at a time on a thread of its own and hands back what
/// was chosen.
pub struct Picker {
    dialogs: Arc<dyn Dialogs>,
    /// Called when a choice is ready, to have it collected.
    wake: Wake,
    waiting: Option<Receiver<Choice>>,
}

impl Picker {
    pub fn new(dialogs: Arc<dyn Dialogs>, wake: Wake) -> Self {
        Self {
            dialogs,
            wake,
            waiting: None,
        }
    }

    /// True while a dialog is open.
    pub fn busy(&self) -> bool {
        self.waiting.is_some()
    }

    fn ask(
        &mut self,
        name: &str,
        ask: impl FnOnce(&dyn Dialogs) -> Choice + Send + 'static,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("a file dialog is already open".to_string());
        }
        let (send, receive) = mpsc::channel();
        let (dialogs, wake) = (Arc::clone(&self.dialogs), Arc::clone(&self.wake));
        std::thread::Builder::new()
            .name(format!("craft3d-dialog-{name}"))
            .spawn(move || {
                // If the dialog panics the channel closes, which `poll`
                // reports; nothing else is lost.
                let choice = ask(dialogs.as_ref());
                let _ = send.send(choice);
                wake();
            })
            .map_err(|e| format!("the file dialog could not be opened: {e}"))?;
        self.waiting = Some(receive);
        Ok(())
    }

    /// Opens the dialog for a file to open. An error when one is open
    /// already or it cannot be shown.
    pub fn ask_open(&mut self) -> Result<(), String> {
        self.ask("open", |dialogs| {
            dialogs
                .open()
                .map_or(Choice::Cancelled("file.open"), Choice::Open)
        })
    }

    /// Opens the dialog for where to save.
    pub fn ask_save(&mut self, suggested: String) -> Result<(), String> {
        self.ask("save", move |dialogs| {
            dialogs
                .save(&suggested)
                .map_or(Choice::Cancelled("file.save"), |path| {
                    Choice::Save(with_extension(path))
                })
        })
    }

    /// Opens the dialog for a file to import.
    pub fn ask_import(&mut self) -> Result<(), String> {
        self.ask("import", |dialogs| {
            dialogs
                .import()
                .map_or(Choice::Cancelled("file.import"), Choice::Import)
        })
    }

    /// Opens the dialog for where to export what the export dialog chose
    /// (`request`, with `file.export`'s `params` made from it), in
    /// `folder` when one is given. The file is named for the format
    /// afterwards ([`export_path`]).
    pub fn ask_export(
        &mut self,
        request: ExportRequest,
        params: ExportParams,
        folder: Option<PathBuf>,
    ) -> Result<(), String> {
        self.ask("export", move |dialogs| {
            dialogs
                .export(&request.name, request.format, folder.as_deref())
                .map_or(Choice::Cancelled("file.export"), |path| {
                    Choice::Export(Box::new(ExportJob {
                        path,
                        request,
                        params,
                    }))
                })
        })
    }

    /// Opens the dialog for where to publish the document as a Jig; a
    /// name without the extension gets it.
    pub fn ask_export_jig(&mut self, suggested: String) -> Result<(), String> {
        self.ask("export-jig", move |dialogs| {
            dialogs
                .export_jig(&suggested)
                .map_or(Choice::Cancelled("jig.export"), |path| {
                    Choice::ExportJig(with_jig_extension(path))
                })
        })
    }

    fn settle(&mut self, received: Result<Choice, bool>) -> Option<Result<Choice, String>> {
        match received {
            Ok(choice) => {
                self.waiting = None;
                Some(Ok(choice))
            }
            // Still open.
            Err(false) => None,
            Err(true) => {
                self.waiting = None;
                Some(Err("the file dialog closed unexpectedly".to_string()))
            }
        }
    }

    /// The choice, once the dialog was closed; `None` while it is open or
    /// when none was asked for.
    pub fn poll(&mut self) -> Option<Result<Choice, String>> {
        let received = match self.waiting.as_ref()?.try_recv() {
            Ok(choice) => Ok(choice),
            Err(TryRecvError::Empty) => Err(false),
            Err(TryRecvError::Disconnected) => Err(true),
        };
        self.settle(received)
    }

    /// Like [`Self::poll`], but waits for the dialog to close.
    pub fn wait(&mut self) -> Option<Result<Choice, String>> {
        let received = self.waiting.as_ref()?.recv().map_err(|_| true);
        self.settle(received)
    }
}

/// A file saved without an extension gets the native one: not every
/// dialog adds it.
pub fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        path
    } else {
        path.with_extension(EXTENSION)
    }
}

/// `path` with the Jig's extension: as it is when it has it (in any case),
/// else with it appended.
fn with_jig_extension(path: PathBuf) -> PathBuf {
    let has = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(JIG_EXTENSION));
    if has {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".");
        name.push(JIG_EXTENSION);
        PathBuf::from(name)
    }
}

/// The grant for one file the person chose: its directory, in one
/// direction. With the path as the command is to be given it.
fn grant(path: &Path, write: bool) -> Result<(Workspace, String), String> {
    let shown = path.display();
    let absolute = std::path::absolute(path)
        .map_err(|e| format!("`{shown}` cannot be used as a file path: {e}"))?;
    let (Some(directory), Some(_)) = (absolute.parent(), absolute.file_name()) else {
        return Err(format!("`{shown}` is not a file path"));
    };
    // The engine takes paths as text.
    let Some(text) = absolute.to_str() else {
        return Err(format!(
            "`{shown}` is not valid Unicode; rename the file or its folder to open or save it"
        ));
    };
    let root = Access::Root(directory.to_path_buf());
    let workspace = if write {
        Workspace {
            read: Access::None,
            write: root,
        }
    } else {
        Workspace {
            read: root,
            write: Access::None,
        }
    };
    Ok((workspace, text.to_string()))
}

/// Runs `command` on the file at `path` with the person's own access to
/// it (see the module's documentation), on `session` wherever it is: the
/// window's thread, or its engine thread for a file chosen in the open or
/// save dialog. The automation workspace is back in place when this
/// returns, whether the command worked, failed or panicked.
pub fn run_as_user(
    session: &mut Session,
    command: &str,
    path: &Path,
    write: bool,
    extra: Option<(&str, Value)>,
) -> Result<Value, String> {
    run_as_user_with(session, command, path, write, extra.into_iter().collect())
}

/// [`run_as_user`] with any number of parameters besides `path`.
pub fn run_as_user_with(
    session: &mut Session,
    command: &str,
    path: &Path,
    write: bool,
    extra: Vec<(&str, Value)>,
) -> Result<Value, String> {
    let (grant, text) = grant(path, write).map_err(|e| format!("{command}: {e}"))?;
    let automation = session.workspace().clone();
    session.set_workspace(grant);
    let mut params = json!({ "path": text });
    if let Some(map) = params.as_object_mut() {
        for (key, value) in extra {
            map.insert(key.to_string(), value);
        }
    }
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.execute(command, &params)
    }));
    session.set_workspace(automation);
    match ran {
        Ok(outcome) => outcome.map_err(|e| e.message().to_string()),
        Err(_) => {
            // Not worth a second panic.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                session.autosave_now();
            }));
            Err(
                "internal error (a bug in 3DCraft, please report it); the document was kept"
                    .to_string(),
            )
        }
    }
}

use craft3d_ui_egui::files::guarded;

/// Imports the STEP file the person chose into the active document
/// (`file.import`: one undo step, an import feature per solid), with the
/// person's own access to that one file. Without a document a new one is
/// made for it, and closed again when the file cannot be imported.
pub fn import_as_user(session: &mut Session, path: &Path) -> Result<Value, String> {
    let made = if session.document_status().is_none() {
        guarded(session, "file.new", &json!({})).map_err(|e| format!("file.new: {e}"))?;
        true
    } else {
        false
    };
    let outcome = run_as_user(session, "file.import", path, false, None);
    if outcome.is_err() && made {
        let _ = guarded(session, "file.close", &json!({ "discard": true }));
    }
    outcome
}

/// Exports to the file the person chose, in the format its extension
/// names, with the person's own access to its folder (`file.export`):
/// every body, or the bodies or faces or sketch selected as far as
/// `file.export` can take them
/// ([`craft3d_ui_egui::files::export_selection`]), a mesh at the normal
/// quality.
pub fn export_as_user(session: &mut Session, path: &Path) -> Result<Value, String> {
    let plan = ExportPlan {
        what: craft3d_ui_egui::files::following_selection(session),
        ..ExportPlan::default()
    };
    export_planned(session, path, plan)
}

/// [`export_as_user`] as the export dialog was answered: what goes in
/// (the model, the selection, a sheet or a pattern) and the options
/// ([`craft3d_ui_egui::files::export_params`]).
pub fn export_planned(
    session: &mut Session,
    path: &Path,
    plan: ExportPlan,
) -> Result<Value, String> {
    let extra = craft3d_ui_egui::files::export_params(session, &path.to_string_lossy(), &plan)?;
    run_as_user_with(session, "file.export", path, true, extra)
}

/// What Save As does with the file named in the dialog: the dialog asked
/// before replacing one ([`Host::save_chosen_as_user`]).
pub const CONFIRMED: (&str, bool) = ("overwrite", true);

impl Host {
    /// A failure the person did not cause (the list of recent files could
    /// not be written): in the status bar only, no dialog.
    pub(crate) fn report_notice(&mut self, label: &str, error: &str) {
        self.shell.last_command = Some(CommandReport {
            label: label.to_string(),
            ok: false,
            text: error.chars().take(160).collect(),
        });
    }

    /// Shows what an action the person started did in the status bar, and
    /// a failure in the error dialog as well (every caller is the person's:
    /// an agent's requests are reported by the control channel).
    pub(crate) fn report(&mut self, label: &str, outcome: &Result<Value, String>) {
        self.last_answer = outcome.as_ref().ok().cloned();
        let (ok, text) = match outcome {
            // Saved, but the picture for a gallery could not be drawn (a
            // body that cannot be meshed): said in words, first, rather
            // than at the end of the answer, where the status line cuts
            // it off.
            Ok(result) => match (
                result.get("thumbnailError").and_then(Value::as_str),
                result.get("path").and_then(Value::as_str),
            ) {
                (Some(why), Some(path)) => (true, format!("no thumbnail ({why}); saved {path}")),
                _ => (true, craft3d_ui_egui::answer::summary(label, result)),
            },
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

    /// Runs a command with the session's own (automation) file access,
    /// guarded like a request is.
    fn run(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        let session = &mut self.session;
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.execute(command, params)
        }));
        ran.unwrap_or_else(|_| {
            self.autosave_after_a_panic();
            Err(CommandError::new(
                "internal error (a bug in 3DCraft, please report it); the document was kept",
            ))
        })
        .map_err(|e| e.message().to_string())
    }

    /// Runs `command` on the file at `path` with the person's own access
    /// to it (see the module's documentation). The automation workspace is
    /// back in place when this returns, whatever happened.
    fn run_as_user(
        &mut self,
        command: &str,
        path: &Path,
        write: bool,
        extra: Option<(&str, Value)>,
    ) -> Result<Value, String> {
        run_as_user(&mut self.session, command, path, write, extra)
    }

    /// Opens the file the person chose and makes it the active document.
    pub fn open_as_user(&mut self, path: &Path) -> Result<Value, String> {
        let outcome = self.run_as_user("file.open", path, false, None);
        self.report("file.open", &outcome);
        self.shell.sync(&mut self.session);
        outcome
    }

    /// Saves the active document to the file the person chose, which
    /// becomes its own file.
    ///
    /// A recovered document is not saved over an original that changed
    /// since it was autosaved (`file.save`'s `overwrite`): a plain Save
    /// reports that, and Save As, where the person names the file in the
    /// dialog and the dialog asks before replacing one, goes through
    /// ([`Self::save_chosen_as_user`]).
    pub fn save_as_user(&mut self, path: &Path) -> Result<Value, String> {
        let outcome = self.run_as_user("file.save", path, true, None);
        self.report("file.save", &outcome);
        self.shell.sync(&mut self.session);
        outcome
    }

    /// Saves the active document to the file the person just named in the
    /// save dialog, replacing what is there: the dialog asked.
    pub fn save_chosen_as_user(&mut self, path: &Path) -> Result<Value, String> {
        let confirmed = Some((CONFIRMED.0, Value::Bool(CONFIRMED.1)));
        let outcome = self.run_as_user("file.save", path, true, confirmed);
        self.report("file.save", &outcome);
        self.shell.sync(&mut self.session);
        outcome
    }

    /// Imports the STEP file the person chose into the active document
    /// (a new one when there is none): [`import_as_user`].
    pub fn import_as_user(&mut self, path: &Path) -> Result<Value, String> {
        let outcome = import_as_user(&mut self.session, path);
        self.report("file.import", &outcome);
        self.shell.sync(&mut self.session);
        outcome
    }

    /// Exports to the file the person chose: [`export_as_user`].
    pub fn export_as_user(&mut self, path: &Path) -> Result<Value, String> {
        let plan = ExportPlan {
            what: craft3d_ui_egui::files::following_selection(&mut self.session),
            ..ExportPlan::default()
        };
        self.export_planned(path, plan)
    }

    /// Exports as the export dialog was answered: [`export_planned`].
    pub fn export_planned(&mut self, path: &Path, plan: ExportPlan) -> Result<Value, String> {
        let outcome = export_planned(&mut self.session, path, plan);
        self.report("file.export", &outcome);
        self.shell.sync(&mut self.session);
        outcome
    }

    /// Publishes the active document as a Jig to the file the person
    /// chose (`jig.export`).
    pub fn export_jig_as_user(&mut self, path: &Path) -> Result<Value, String> {
        let outcome = self.run_as_user("jig.export", path, true, None);
        self.report("jig.export", &outcome);
        self.shell.sync(&mut self.session);
        outcome
    }

    /// The file the save dialog chose for an export, named for its
    /// format ([`export_path`]), with `file.export`'s parameters and what
    /// to keep once it is written ([`Self::export_written`]). An error,
    /// said in the error dialog, when another document is shown now than
    /// the one the export was asked for, or when correcting the name would
    /// replace a file the save dialog did not ask about. Nothing is written
    /// before, and nothing is remembered.
    pub fn export_target(&mut self, job: ExportJob) -> Option<(PathBuf, ExportParams, ExportDone)> {
        let shown = self.session.document_status().map(|s| s.id);
        if shown != job.request.document {
            self.report(
                "file.export",
                &Err("another document is shown now than the one the export was asked for: nothing was exported; export again".to_string()),
            );
            return None;
        }
        let path = export_path(&job.path, job.request.format);
        // The dialog asked before replacing the file it named, not the
        // one with the extension added.
        if path != job.path && path.exists() {
            let name = |p: &Path| {
                p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            };
            self.report(
                "file.export",
                &Err(format!(
                    "`{}` exists already, and the save dialog asked about `{}`: nothing was exported; export again and choose `{}` to replace it",
                    name(&path),
                    name(&job.path),
                    name(&path)
                )),
            );
            return None;
        }
        let done = ExportDone {
            request: job.request,
            folder: path.parent().map(Path::to_path_buf),
        };
        Some((path, job.params, done))
    }

    /// An export was written: its folder is where the next save dialog
    /// opens, and what its dialog chose is offered next time.
    pub(crate) fn export_written(&mut self, done: &ExportDone) {
        if done.folder.is_some() {
            self.export_folder.clone_from(&done.folder);
        }
        self.shell.export_written(&done.request);
    }

    /// Writes an export the save dialog named ([`Self::export_target`]),
    /// with the person's access to its folder.
    pub fn export_job(&mut self, job: ExportJob) -> Option<Result<Value, String>> {
        let (path, params, done) = self.export_target(job)?;
        let outcome = run_as_user_with(&mut self.session, "file.export", &path, true, params);
        self.report("file.export", &outcome);
        if outcome.is_ok() {
            self.export_written(&done);
        }
        self.shell.sync(&mut self.session);
        Some(outcome)
    }

    /// Where the next export's save dialog opens: the last export's
    /// folder, else the document's own when it has a file.
    pub(crate) fn export_folder(&self) -> Option<PathBuf> {
        self.export_folder.clone().or_else(|| {
            let status = self.session.document_status()?;
            let path = PathBuf::from(status.path?);
            path.is_absolute()
                .then(|| path.parent().map(Path::to_path_buf))
                .flatten()
        })
    }

    /// The export dialog was answered: `file.export`'s parameters are made
    /// now, from the selection as it is when Export… was clicked, then the
    /// save dialog asks where. What cannot be exported is said at once,
    /// with no save dialog.
    pub(crate) fn ask_where_to_export(&mut self, request: ExportRequest, picker: &mut Picker) {
        let params = match craft3d_ui_egui::files::export_params(
            &mut self.session,
            &request.name,
            &request.plan,
        ) {
            Ok(params) => params,
            Err(error) => {
                self.report("file.export", &Err(error));
                return;
            }
        };
        let folder = self.export_folder();
        if let Err(error) = picker.ask_export(request, params, folder) {
            self.report("file.export", &Err(error));
        }
    }

    /// Does what the File menu asked for, as far as that goes without a
    /// dialog; opens the dialog otherwise, whose answer goes to
    /// [`Self::file_choice`].
    pub fn file_action(&mut self, action: FileAction, picker: &mut Picker) {
        let asked = match action {
            FileAction::New => {
                let outcome = self.run("file.new", &json!({}));
                self.report("file.new", &outcome);
                self.shell.sync(&mut self.session);
                return;
            }
            FileAction::Open => picker.ask_open().map_err(|e| ("file.open", e)),
            FileAction::Import => picker.ask_import().map_err(|e| ("file.import", e)),
            // The export dialog first (what, the format, the options);
            // its Export… asks where ([`Self::ask_where_to_export`]).
            FileAction::Export => {
                self.shell.set_export_names_file(false);
                self.shell
                    .open_export(&mut self.session)
                    .map_err(|e| ("file.export", e))
            }
            FileAction::ExportJig => {
                let Some(status) = self.session.document_status() else {
                    self.report(
                        "jig.export",
                        &Err("there is no document to publish".to_string()),
                    );
                    return;
                };
                picker
                    .ask_export_jig(suggested_jig_name(status.path.as_deref()))
                    .map_err(|e| ("jig.export", e))
            }
            FileAction::OpenForEditing => {
                let outcome = self.run("jig.edit", &json!({}));
                self.report("jig.edit", &outcome);
                self.shell.sync(&mut self.session);
                return;
            }
            FileAction::Save | FileAction::SaveAs => {
                let Some(status) = self.session.document_status() else {
                    self.report(
                        "file.save",
                        &Err("there is no document to save".to_string()),
                    );
                    return;
                };
                // A Jig in use is never saved over its file: Save saves
                // it as a model, named in the dialog.
                let own = status
                    .path
                    .as_deref()
                    .filter(|_| action == FileAction::Save && !status.jig_in_use);
                match own {
                    // Its own file, chosen by the person before (in a
                    // dialog or on the command line): saved there again.
                    Some(path) if Path::new(path).is_absolute() => {
                        let _ = self.save_as_user(Path::new(path));
                        return;
                    }
                    // A path beneath the automation roots, as a request
                    // gave it: saved the way a request would. When that
                    // is refused (no write root), the person chooses.
                    Some(_) => {
                        let outcome = self.run("file.save", &json!({}));
                        if outcome.is_ok() {
                            self.report("file.save", &outcome);
                            self.shell.sync(&mut self.session);
                            return;
                        }
                    }
                    None => {}
                }
                picker
                    .ask_save(suggested_name(status.path.as_deref()))
                    .map_err(|e| ("file.save", e))
            }
        };
        if let Err((label, error)) = asked {
            self.report(label, &Err(error));
        }
    }

    /// Does what the person chose in a dialog.
    pub fn file_choice(&mut self, choice: Result<Choice, String>) {
        match choice {
            Ok(Choice::Open(path)) => {
                let _ = self.open_as_user(&path);
            }
            Ok(Choice::Import(path)) => {
                let _ = self.import_as_user(&path);
            }
            Ok(Choice::Export(job)) => {
                let _ = self.export_job(*job);
            }
            Ok(Choice::ExportJig(path)) => {
                let _ = self.export_jig_as_user(&path);
            }
            Ok(Choice::Save(path)) => {
                let saved = self.save_chosen_as_user(&path);
                if std::mem::take(&mut self.close_after_save) && saved.is_ok() {
                    let session = &mut self.session;
                    let closed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::tabs::close_when_saved(session, saved, true)
                    }))
                    .unwrap_or_else(|_| {
                        Err("internal error (a bug in 3DCraft, please report it); the document was kept"
                            .to_string())
                    });
                    self.report("file.close", &closed);
                    self.shell.sync(&mut self.session);
                }
            }
            Ok(Choice::Cancelled(label)) => {
                self.close_after_save = false;
                self.report(label, &Err("cancelled".to_string()));
            }
            Err(error) => {
                self.close_after_save = false;
                self.report("file", &Err(error));
            }
        }
    }

    /// One step of the file actions, between two frames: the answer of a
    /// dialog that was closed, then what the menu asked for. True when
    /// something was done.
    pub fn pump_files(&mut self, picker: &mut Picker) -> bool {
        let mut did = false;
        if let Some(choice) = picker.poll() {
            self.file_choice(choice);
            did = true;
        }
        if let Some(request) = self.shell.take_export_answer() {
            self.ask_where_to_export(request, picker);
            did = true;
        }
        if let Some(action) = self.shell.take_file_action() {
            self.file_action(action, picker);
            did = true;
        }
        if let Some(action) = self.shell.take_doc_action() {
            self.doc_action(action, picker);
            did = true;
        }
        if let Some(request) = self.shell.take_edit_action() {
            let command = request.action.command();
            let session = &mut self.session;
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                craft3d_ui_egui::run_edit(session, request)
            }))
            .unwrap_or_else(|_| {
                Err("internal error (a bug in 3DCraft, please report it)".to_string())
            });
            self.report(command, &outcome);
            self.shell.sync(&mut self.session);
            did = true;
        }
        if let Some(path) = self.shell.take_open_recent() {
            let outcome = self.run_as_user("file.open", Path::new(&path), false, None);
            self.report("file.open", &outcome);
            self.shell.sync(&mut self.session);
            did = true;
        }
        if let Some(example) = self.shell.take_example() {
            let session = &mut self.session;
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                craft3d_ui_egui::shell::run_example(session, example)
            }))
            .unwrap_or_else(|_| {
                Err("internal error (a bug in 3DCraft, please report it)".to_string())
            });
            self.report("example", &built);
            self.shell.sync(&mut self.session);
            did = true;
        }
        self.pump_places() || did
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_engine::Session;
    use craft3d_ui_egui::window_title;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An empty directory of this test's own, by its one true name.
    fn workdir(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/tmp/craft3d-files-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(dir).unwrap()
    }

    fn subdir(dir: &Path, name: &str) -> PathBuf {
        let sub = dir.join(name);
        std::fs::create_dir_all(&sub).unwrap();
        sub
    }

    fn text(path: &Path) -> &str {
        path.to_str().unwrap()
    }

    /// The names in a directory, sorted.
    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// Writes a document with one block of the given height, as a person
    /// with a file of their own would have it.
    fn write_part(path: &Path, height: f64) {
        let mut s = Session::new(Workspace::unrestricted());
        s.execute("file.new", &json!({})).unwrap();
        s.execute("feature.block", &json!({"size": [10, 10, height]}))
            .unwrap();
        s.execute("file.save", &json!({"path": text(path)}))
            .unwrap();
    }

    /// A measured volume to the millionth: the blocks here have whole
    /// volumes, and the measure is an integral, exact to rounding.
    fn whole(measured: &Value) -> f64 {
        (measured["volume"].as_f64().unwrap() * 1e6).round() / 1e6
    }

    fn volume(host: &mut Host) -> f64 {
        whole(
            &host
                .session
                .execute("document.measure", &json!({}))
                .unwrap(),
        )
    }

    /// The volume of the document in a file, read by a session of its own.
    fn volume_on_disk(path: &Path) -> f64 {
        let mut s = Session::new(Workspace::unrestricted());
        s.execute("file.open", &json!({"path": text(path)}))
            .unwrap();
        whole(&s.execute("document.measure", &json!({})).unwrap())
    }

    /// A request as an agent on the control channel sends it. The error,
    /// or the result.
    fn agent(host: &mut Host, command: &str, params: Value) -> Result<Value, String> {
        let line = json!({"id": 1, "method": "engine.execute",
                          "params": {"command": command, "params": params}})
        .to_string();
        let reply: Value = serde_json::from_str(&host.handle_line(&line).unwrap()).unwrap();
        if reply["ok"] == true {
            Ok(reply["result"].clone())
        } else {
            Err(reply["error"].as_str().unwrap().to_string())
        }
    }

    fn rooted(read: Option<&Path>, write: Option<&Path>) -> Workspace {
        let access =
            |dir: Option<&Path>| dir.map_or(Access::None, |d| Access::Root(d.to_path_buf()));
        Workspace {
            read: access(read),
            write: access(write),
        }
    }

    /// Answers the dialogs for the person, in order, and notes what was
    /// asked.
    #[derive(Default)]
    struct Person {
        answers: Mutex<Vec<Option<PathBuf>>>,
        asked: Mutex<Vec<String>>,
    }

    impl Person {
        fn who_answers(answers: &[Option<&Path>]) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers.iter().map(|a| a.map(Path::to_path_buf)).collect()),
                asked: Mutex::default(),
            })
        }

        fn answer(&self, question: String) -> Option<PathBuf> {
            self.asked.lock().unwrap().push(question);
            let mut answers = self.answers.lock().unwrap();
            assert!(!answers.is_empty(), "a dialog nobody expected");
            answers.remove(0)
        }

        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Dialogs for Person {
        fn open(&self) -> Option<PathBuf> {
            self.answer("open".to_string())
        }

        fn save(&self, suggested: &str) -> Option<PathBuf> {
            self.answer(format!("save {suggested}"))
        }

        fn import(&self) -> Option<PathBuf> {
            self.answer("import".to_string())
        }

        /// `export part.stl [STL]`, and ` in FOLDER` when it opens in one.
        fn export(
            &self,
            suggested: &str,
            format: ExportFormat,
            folder: Option<&Path>,
        ) -> Option<PathBuf> {
            let mut question = format!("export {suggested} [{}]", format.label());
            if let Some(folder) = folder {
                question.push_str(&format!(" in {}", folder.display()));
            }
            self.answer(question)
        }

        fn export_jig(&self, suggested: &str) -> Option<PathBuf> {
            self.answer(format!("export-jig {suggested}"))
        }
    }

    fn picker(person: &Arc<Person>) -> Picker {
        Picker::new(Arc::clone(person) as Arc<dyn Dialogs>, Arc::new(|| {}))
    }

    /// Does a file action and, when it opened a dialog, waits for the
    /// person and does what they chose.
    fn act(host: &mut Host, picker: &mut Picker, action: FileAction) {
        host.file_action(action, picker);
        if let Some(choice) = picker.wait() {
            host.file_choice(choice);
        }
    }

    #[test]
    fn the_edit_menu_and_its_keys_undo_and_redo_through_the_host() {
        let mut host = Host::new(Session::new(Workspace::none()));
        let person = Person::who_answers(&[]);
        let mut dialogs = picker(&person);
        host.session.execute("file.new", &json!({})).unwrap();
        host.session
            .execute("feature.block", &json!({"size": [10, 10, 5]}))
            .unwrap();
        assert_eq!(volume(&mut host), 500.0);
        let id = host.session.document_status().unwrap().id;
        host.shell
            .request_edit_action(craft3d_ui_egui::EditAction::Undo, id);
        assert!(host.pump_files(&mut dialogs));
        assert_eq!(last(&host).0, "edit.undo");
        assert!(last(&host).1, "{:?}", last(&host));
        assert_eq!(volume(&mut host), 0.0);
        host.shell
            .request_edit_action(craft3d_ui_egui::EditAction::Redo, id);
        assert!(host.pump_files(&mut dialogs));
        assert_eq!(volume(&mut host), 500.0);
        // Nothing to redo: reported, not a crash.
        host.shell
            .request_edit_action(craft3d_ui_egui::EditAction::Redo, id);
        assert!(host.pump_files(&mut dialogs));
        assert_eq!(last(&host).0, "edit.redo");
        assert!(!last(&host).1);
        // For a document that is no longer the active one: refused, with
        // a note, and the active one is untouched.
        host.session.execute("file.new", &json!({})).unwrap();
        host.session
            .execute("feature.block", &json!({"size": [1, 1, 1]}))
            .unwrap();
        host.shell
            .request_edit_action(craft3d_ui_egui::EditAction::Undo, id);
        assert!(host.pump_files(&mut dialogs));
        let (label, ok, said) = last(&host);
        assert_eq!((label.as_str(), ok), ("edit.undo", false));
        assert!(said.contains("another document"), "{said}");
        assert_eq!(volume(&mut host), 1.0);
        assert!(person.asked().is_empty());
    }

    fn last(host: &Host) -> (String, bool, String) {
        let report = host.shell.last_command.clone().unwrap();
        (report.label, report.ok, report.text)
    }

    #[test]
    fn the_person_opens_a_file_outside_the_roots_and_a_request_still_cannot() {
        let dir = workdir("open");
        let (mine, shared) = (subdir(&dir, "mine"), subdir(&dir, "shared"));
        let part = mine.join("part.3dcraft");
        write_part(&part, 5.0);
        let on_disk = std::fs::read(&part).unwrap();
        let automation = rooted(Some(&shared), Some(&shared));
        let mut host = Host::new(Session::new(automation.clone()));

        // A request cannot reach the file.
        let refused = agent(&mut host, "file.open", json!({"path": text(&part)})).unwrap_err();
        assert!(refused.contains("outside the read root"), "{refused}");
        assert!(host.session.document_status().is_none());

        // The person can: it is the file they chose.
        let result = host.open_as_user(&part).unwrap();
        assert_eq!(result["features"], 1);
        let status = host.session.document_status().unwrap();
        assert_eq!(status.path.as_deref(), Some(text(&part)));
        assert!(!status.dirty);
        assert_eq!(volume(&mut host), 500.0);
        assert_eq!(host.shell.tree_rows().len(), 1, "the tree shows it already");
        let (label, ok, _) = last(&host);
        assert_eq!((label.as_str(), ok), ("file.open", true));

        // Nothing was widened: the session is under the automation roots
        // exactly as before, and a request is refused exactly as before.
        assert_eq!(*host.session.workspace(), automation);
        let refused = agent(&mut host, "file.open", json!({"path": text(&part)})).unwrap_err();
        assert!(refused.contains("outside the read root"), "{refused}");
        let sibling = mine.join("other.3dcraft");
        for params in [
            json!({}),
            json!({"path": text(&part)}),
            json!({"path": text(&sibling)}),
        ] {
            let refused = agent(&mut host, "file.save", params.clone()).unwrap_err();
            assert!(
                refused.contains("outside the write root"),
                "{params}: {refused}"
            );
        }
        let refused = agent(
            &mut host,
            "file.export",
            json!({"path": text(&mine.join("a.stl"))}),
        )
        .unwrap_err();
        assert!(refused.contains("outside the write root"), "{refused}");
        // A request may edit the document in the window, and save it
        // where it may write.
        agent(
            &mut host,
            "feature.edit",
            json!({"feature": "F1", "size": [10, 10, 7]}),
        )
        .unwrap();
        assert!(host.session.document_status().unwrap().dirty);
        agent(&mut host, "file.save", json!({"path": "copy.3dcraft"})).unwrap();
        assert_eq!(volume_on_disk(&shared.join("copy.3dcraft")), 700.0);
        // The person's file and folder are as they were.
        assert_eq!(std::fs::read(&part).unwrap(), on_disk);
        assert_eq!(listing(&mine), ["part.3dcraft"]);

        // A file that is not a document, and one that is not there: an
        // error for the status bar, and the roots are back in place.
        let junk = mine.join("junk.3dcraft");
        std::fs::write(&junk, b"not a document").unwrap();
        for bad in [junk, mine.join("missing.3dcraft"), mine.clone()] {
            let error = host.open_as_user(&bad).unwrap_err();
            assert!(error.contains("file.open"), "{error}");
            assert_eq!(*host.session.workspace(), automation);
            let (label, ok, text) = last(&host);
            assert_eq!((label.as_str(), ok), ("file.open", false));
            assert!(!text.is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_as_writes_the_chosen_file_and_nothing_else() {
        let dir = workdir("save-as");
        let (mine, elsewhere) = (subdir(&dir, "mine"), subdir(&dir, "elsewhere"));
        // No file access at all for requests.
        let mut host = Host::new(Session::new(Workspace::none()));
        host.session.execute("file.new", &json!({})).unwrap();
        host.session
            .execute("feature.block", &json!({"size": [10, 10, 3]}))
            .unwrap();
        assert!(host.session.document_status().unwrap().dirty);

        let chosen = mine.join("bracket.3dcraft");
        let result = host.save_as_user(&chosen).unwrap();
        assert_eq!(result["path"], text(&chosen));
        // That file, with this document in it; nothing beside it, nothing
        // anywhere else.
        assert_eq!(listing(&mine), ["bracket.3dcraft"]);
        assert!(listing(&elsewhere).is_empty());
        assert_eq!(volume_on_disk(&chosen), 300.0);
        let status = host.session.document_status().unwrap();
        assert_eq!(status.path.as_deref(), Some(text(&chosen)));
        assert!(!status.dirty);
        // The window's title: the file's name, and no mark.
        let title = |host: &Host| window_title(host.session.document_status().as_ref());
        assert_eq!(title(&host), "bracket.3dcraft – 3DCraft");
        let (label, ok, _) = last(&host);
        assert_eq!((label.as_str(), ok), ("file.save", true));
        // Requests are where they were: nowhere.
        assert_eq!(*host.session.workspace(), Workspace::none());
        for command in ["file.save", "file.open"] {
            let refused = agent(&mut host, command, json!({"path": text(&chosen)})).unwrap_err();
            assert!(refused.contains("access"), "{command}: {refused}");
        }
        assert!(agent(&mut host, "file.save", json!({})).is_err());

        // Under another name, in another folder: the first file stays as
        // it is and the document moves to the new one.
        host.session
            .execute(
                "feature.edit",
                &json!({"feature": "F1", "size": [10, 10, 4]}),
            )
            .unwrap();
        assert_eq!(title(&host), "bracket.3dcraft* – 3DCraft", "changed");
        let second = elsewhere.join("bracket-v2.3dcraft");
        host.save_as_user(&second).unwrap();
        assert_eq!(volume_on_disk(&chosen), 300.0);
        assert_eq!(volume_on_disk(&second), 400.0);
        assert_eq!(
            host.session.document_status().unwrap().path.as_deref(),
            Some(text(&second))
        );

        // Where nothing can be saved: an error, and the document is still
        // the one of the second file, changed.
        host.session
            .execute(
                "feature.edit",
                &json!({"feature": "F1", "size": [10, 10, 9]}),
            )
            .unwrap();
        let nowhere = dir.join("no-such-folder").join("x.3dcraft");
        let error = host.save_as_user(&nowhere).unwrap_err();
        assert!(error.contains("file.save"), "{error}");
        assert!(!nowhere.exists());
        let status = host.session.document_status().unwrap();
        assert!(status.dirty);
        assert_eq!(status.path.as_deref(), Some(text(&second)));
        assert_eq!(*host.session.workspace(), Workspace::none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Jig saved at the window whose thumbnail cannot be drawn (here: no
    /// bodies; in the field, a body that could not be meshed) is saved,
    /// and the status line says first that it has no thumbnail and why.
    #[test]
    fn a_save_without_its_thumbnail_says_so_in_words() {
        let dir = workdir("no-thumbnail");
        let mut host = Host::new(Session::new(Workspace::none()));
        host.session.execute("file.new", &json!({})).unwrap();
        host.session
            .execute("variable.set", &json!({"name": "a", "expr": "1 mm"}))
            .unwrap();
        host.session
            .execute("variable.publish", &json!({"name": "a"}))
            .unwrap();
        let chosen = dir.join("empty.3dcraft");
        let result = host.save_as_user(&chosen).unwrap();
        assert_eq!(result["thumbnail"], false, "{result}");
        assert_eq!(listing(&dir), ["empty.3dcraft"]);
        let (label, ok, said) = last(&host);
        assert_eq!((label.as_str(), ok), ("file.save", true));
        assert!(
            said.starts_with("no thumbnail (") && said.contains("no bodies"),
            "{said}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two roles of a Jig at the window: Export as Jig publishes a
    /// model as a `.3dcraftjig`; opening that file uses it (read-only but
    /// for its parameters); Save then asks for a model's name and never
    /// writes the Jig's file; Open for editing makes a new model of it.
    #[test]
    fn a_jig_is_published_opened_for_use_and_saved_as_a_model() {
        let dir = workdir("jig");
        let mine = subdir(&dir, "mine");
        let mut host = Host::new(Session::new(Workspace::none()));
        let person = Person::who_answers(&[
            Some(&mine.join("plate")),
            Some(&mine.join("plate.3dcraftjig")),
            Some(&mine.join("my-plate.3dcraft")),
        ]);
        let mut dialogs = picker(&person);
        act(&mut host, &mut dialogs, FileAction::New);
        for (command, params) in [
            ("variable.set", json!({"name": "w", "expr": "40 mm"})),
            ("feature.block", json!({"size": ["w", 10, 2]})),
            (
                "variable.publish",
                json!({"name": "w", "min": "10 mm", "max": "90 mm"}),
            ),
        ] {
            host.session.execute(command, &params).unwrap();
        }
        host.shell.sync(&mut host.session);
        // Export as Jig: the name gets the Jig's extension.
        act(&mut host, &mut dialogs, FileAction::ExportJig);
        assert_eq!(person.asked(), ["export-jig untitled.3dcraftjig"]);
        let (label, ok, text) = last(&host);
        assert_eq!((label.as_str(), ok), ("jig.export", true), "{text}");
        let jig = mine.join("plate.3dcraftjig");
        let bytes = std::fs::read(&jig).unwrap();
        // Open: the Jig in use.
        act(&mut host, &mut dialogs, FileAction::Open);
        let status = host.session.document_status().unwrap();
        assert!(status.jig_in_use && !status.dirty);
        assert!(host.shell.jig_in_use());
        host.session
            .execute("param.set", &json!({"values": {"w": 60}}))
            .unwrap();
        // Save asks for a model's name, proposing one.
        act(&mut host, &mut dialogs, FileAction::Save);
        assert_eq!(person.asked().last().unwrap(), "save plate.3dcraft");
        assert_eq!(
            std::fs::read(&jig).unwrap(),
            bytes,
            "the Jig is never saved over"
        );
        assert_eq!(volume_on_disk(&mine.join("my-plate.3dcraft")), 1200.0);
        assert!(!host.session.document_status().unwrap().jig_in_use);
        // Open for editing: a new untitled model of a Jig in use.
        let again = Person::who_answers(&[Some(&jig)]);
        let mut dialogs = picker(&again);
        act(&mut host, &mut dialogs, FileAction::Open);
        assert!(host.session.document_status().unwrap().jig_in_use);
        act(&mut host, &mut dialogs, FileAction::OpenForEditing);
        let (label, ok, text) = last(&host);
        assert_eq!((label.as_str(), ok), ("jig.edit", true), "{text}");
        let status = host.session.document_status().unwrap();
        assert!(!status.jig_in_use && status.path.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_goes_to_the_documents_own_file_and_asks_only_when_there_is_none() {
        let dir = workdir("save");
        let mine = subdir(&dir, "mine");
        let mut host = Host::new(Session::new(Workspace::none()));

        // Nothing to save: no dialog, an error in the status bar.
        let nobody = Person::who_answers(&[]);
        let mut dialogs = picker(&nobody);
        for action in [FileAction::Save, FileAction::SaveAs] {
            act(&mut host, &mut dialogs, action);
            let (label, ok, text) = last(&host);
            assert_eq!((label.as_str(), ok), ("file.save", false));
            assert!(text.contains("no document"), "{text}");
        }

        // New: a document without a file. Save asks where, proposing a
        // name; the answer has no extension and gets the native one.
        let person = Person::who_answers(&[
            Some(&mine.join("gear")),
            None,
            Some(&mine.join("gear2.3dcraft")),
        ]);
        let mut dialogs = picker(&person);
        act(&mut host, &mut dialogs, FileAction::New);
        assert_eq!(last(&host).0, "file.new");
        host.session
            .execute("feature.block", &json!({"size": [10, 10, 2]}))
            .unwrap();
        act(&mut host, &mut dialogs, FileAction::Save);
        assert_eq!(person.asked(), ["save untitled.3dcraft"]);
        let gear = mine.join("gear.3dcraft");
        assert_eq!(listing(&mine), ["gear.3dcraft"]);
        assert_eq!(volume_on_disk(&gear), 200.0);
        assert!(!host.session.document_status().unwrap().dirty);

        // Now it has a file: Save writes it again without asking.
        host.session
            .execute(
                "feature.edit",
                &json!({"feature": "F1", "size": [10, 10, 6]}),
            )
            .unwrap();
        act(&mut host, &mut dialogs, FileAction::Save);
        assert_eq!(person.asked().len(), 1, "no second dialog");
        assert_eq!(volume_on_disk(&gear), 600.0);
        assert!(!host.session.document_status().unwrap().dirty);
        assert_eq!(listing(&mine), ["gear.3dcraft"]);

        // Save As asks even so, proposing the file's name. Cancelled:
        // nothing is written and the document is as it was.
        host.session
            .execute(
                "feature.edit",
                &json!({"feature": "F1", "size": [10, 10, 8]}),
            )
            .unwrap();
        act(&mut host, &mut dialogs, FileAction::SaveAs);
        assert_eq!(person.asked()[1], "save gear.3dcraft");
        let (label, ok, text) = last(&host);
        assert_eq!(
            (label.as_str(), ok, text.as_str()),
            ("file.save", false, "cancelled")
        );
        assert_eq!(volume_on_disk(&gear), 600.0);
        assert_eq!(listing(&mine), ["gear.3dcraft"]);
        let status = host.session.document_status().unwrap();
        assert!(status.dirty);
        assert_eq!(status.path.as_deref(), Some(text_of(&gear)));
        // And answered: the new file is written, the old one stays.
        act(&mut host, &mut dialogs, FileAction::SaveAs);
        assert_eq!(volume_on_disk(&mine.join("gear2.3dcraft")), 800.0);
        assert_eq!(volume_on_disk(&gear), 600.0);
        assert_eq!(person.asked().len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn text_of(path: &Path) -> &str {
        text(path)
    }

    /// A host whose session autosaves into `dir` when asked to, by a
    /// clock that stands still.
    fn recovering(dir: &Path) -> Host {
        let mut session = Session::new(Workspace::none());
        session.enable_recovery(craft3d_engine::RecoveryOptions::manual(
            dir,
            Arc::new(craft3d_engine::ManualClock::new(1_800_000_000)),
        ));
        Host::new(session)
    }

    #[test]
    fn a_recovered_document_replaces_a_changed_original_only_through_save_as() {
        let dir = workdir("recovered");
        let (mine, recovery) = (subdir(&dir, "mine"), subdir(&dir, "recovery"));
        let part = mine.join("part.3dcraft");
        write_part(&part, 5.0);

        // The person opens their file, an edit is autosaved, the app dies.
        let mut crashed = recovering(&recovery);
        crashed.open_as_user(&part).unwrap();
        crashed
            .session
            .execute(
                "feature.edit",
                &json!({"feature": "F1", "size": [10, 10, 7]}),
            )
            .unwrap();
        crashed.session.autosave_now();
        drop(crashed);
        // Meanwhile the file on disk became another version.
        write_part(&part, 9.0);

        let mut host = recovering(&recovery);
        let found = host.session.recovery_overview().found;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path.as_deref(), Some(text(&part)));
        host.session
            .execute("recovery.open", &json!({"id": found[0].id}))
            .unwrap();
        assert_eq!(volume(&mut host), 700.0);
        assert!(host.session.document_status().unwrap().dirty);

        // Save: its own file, without a dialog, as always; refused,
        // because that file is no longer the one the recovery was of.
        let person = Person::who_answers(&[Some(&part)]);
        let mut dialogs = picker(&person);
        act(&mut host, &mut dialogs, FileAction::Save);
        assert_eq!(person.asked(), Vec::<String>::new());
        let (label, ok, said) = last(&host);
        assert_eq!((label.as_str(), ok), ("file.save", false));
        assert!(said.contains("changed on disk"), "{said}");
        assert_eq!(volume_on_disk(&part), 900.0);
        assert!(host.session.document_status().unwrap().dirty);
        assert_eq!(listing(&recovery).len(), 3, "the recovery file is kept");

        // Save As: the person names the file in the dialog, which asks
        // before replacing one. That is the confirmation.
        act(&mut host, &mut dialogs, FileAction::SaveAs);
        assert_eq!(person.asked(), ["save part.3dcraft"]);
        let (label, ok, said) = last(&host);
        assert_eq!((label.as_str(), ok), ("file.save", true), "{said}");
        assert_eq!(volume_on_disk(&part), 700.0);
        assert!(!host.session.document_status().unwrap().dirty);
        assert_eq!(listing(&recovery).len(), 1, "only the lock is left");
        // The workspace is the automation's again.
        assert_eq!(*host.session.workspace(), Workspace::none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_asks_for_the_file_and_opens_what_was_chosen() {
        let dir = workdir("open-dialog");
        let mine = subdir(&dir, "mine");
        let part = mine.join("part.3dcraft");
        write_part(&part, 5.0);
        let person = Person::who_answers(&[None, Some(&part)]);
        let mut dialogs = picker(&person);
        let mut host = Host::new(Session::new(Workspace::none()));
        // Cancelled: no document.
        act(&mut host, &mut dialogs, FileAction::Open);
        assert_eq!(
            last(&host),
            ("file.open".to_string(), false, "cancelled".to_string())
        );
        assert!(host.session.document_status().is_none());
        // Chosen: opened, through the shell's own request as the menu
        // makes it.
        host.shell.request_file_action(FileAction::Open);
        assert!(host.pump_files(&mut dialogs));
        assert!(dialogs.busy());
        let choice = dialogs.wait().unwrap();
        assert_eq!(choice, Ok(Choice::Open(part.clone())));
        host.file_choice(choice);
        assert_eq!(volume(&mut host), 500.0);
        assert_eq!(person.asked(), ["open", "open"]);
        // Nothing more to do.
        assert!(!host.pump_files(&mut dialogs));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_document_that_lives_under_the_roots_is_saved_there_without_asking() {
        let dir = workdir("rooted");
        let (shared, mine) = (subdir(&dir, "shared"), subdir(&dir, "mine"));
        let mut host = Host::new(Session::new(rooted(Some(&shared), Some(&shared))));
        // A request made and saved it, by a path relative to its root.
        agent(&mut host, "file.new", json!({})).unwrap();
        agent(&mut host, "feature.block", json!({"size": [10, 10, 1]})).unwrap();
        agent(&mut host, "file.save", json!({"path": "made.3dcraft"})).unwrap();
        agent(
            &mut host,
            "feature.edit",
            json!({"feature": "F1", "size": [10, 10, 2]}),
        )
        .unwrap();
        // The person presses Ctrl+S: saved where it lives, no dialog.
        let nobody = Person::who_answers(&[]);
        let mut dialogs = picker(&nobody);
        act(&mut host, &mut dialogs, FileAction::Save);
        assert!(last(&host).1, "{:?}", last(&host));
        assert_eq!(volume_on_disk(&shared.join("made.3dcraft")), 200.0);
        assert!(!host.session.document_status().unwrap().dirty);
        assert!(nobody.asked().is_empty());

        // Where requests may read but not write, that path cannot be
        // saved to by them: the person is asked instead.
        let mut host = Host::new(Session::new(rooted(Some(&shared), None)));
        agent(&mut host, "file.open", json!({"path": "made.3dcraft"})).unwrap();
        agent(
            &mut host,
            "feature.edit",
            json!({"feature": "F1", "size": [10, 10, 3]}),
        )
        .unwrap();
        let person = Person::who_answers(&[Some(&mine.join("kept.3dcraft"))]);
        let mut dialogs = picker(&person);
        act(&mut host, &mut dialogs, FileAction::Save);
        assert_eq!(person.asked(), ["save made.3dcraft"]);
        assert_eq!(volume_on_disk(&mine.join("kept.3dcraft")), 300.0);
        assert_eq!(
            volume_on_disk(&shared.join("made.3dcraft")),
            200.0,
            "untouched"
        );
        assert_eq!(*host.session.workspace(), rooted(Some(&shared), None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_dialog_at_a_time_and_the_window_is_woken_when_it_closes() {
        /// A person who takes their time: the dialog stays open until the
        /// test says so.
        struct Slow {
            close: Mutex<mpsc::Receiver<Option<PathBuf>>>,
        }
        impl Dialogs for Slow {
            fn open(&self) -> Option<PathBuf> {
                self.close.lock().unwrap().recv().unwrap()
            }
            fn save(&self, _: &str) -> Option<PathBuf> {
                panic!("the save dialog fell over")
            }
            fn import(&self) -> Option<PathBuf> {
                self.open()
            }
            fn export(
                &self,
                suggested: &str,
                _: ExportFormat,
                _: Option<&Path>,
            ) -> Option<PathBuf> {
                self.save(suggested)
            }
        }
        let (close, closing) = mpsc::channel();
        let woken = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&woken);
        let mut picker = Picker::new(
            Arc::new(Slow {
                close: Mutex::new(closing),
            }),
            Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert!(!picker.busy() && picker.poll().is_none() && picker.wait().is_none());
        picker.ask_open().unwrap();
        assert!(picker.busy());
        // While it is open: nothing to collect, and no second dialog.
        assert!(picker.poll().is_none());
        let error = picker.ask_open().unwrap_err();
        assert!(error.contains("already open"), "{error}");
        assert!(picker.ask_save("x".to_string()).is_err());
        let mut host = Host::new(Session::new(Workspace::none()));
        host.file_action(FileAction::Open, &mut picker);
        assert!(!last(&host).1);
        assert!(last(&host).2.contains("already open"));
        assert_eq!(woken.load(Ordering::SeqCst), 0);
        // The person chooses.
        close
            .send(Some(PathBuf::from("/somewhere/a.3dcraft")))
            .unwrap();
        assert_eq!(
            picker.wait(),
            Some(Ok(Choice::Open(PathBuf::from("/somewhere/a.3dcraft"))))
        );
        assert!(!picker.busy());
        // A dialog that falls over is an error, and the next one works.
        picker.ask_save("x".to_string()).unwrap();
        let error = picker.wait().unwrap().unwrap_err();
        assert!(error.contains("unexpectedly"), "{error}");
        host.file_choice(Err(error));
        assert!(!last(&host).1);
        picker.ask_open().unwrap();
        close.send(None).unwrap();
        assert_eq!(picker.wait(), Some(Ok(Choice::Cancelled("file.open"))));
        // Woken once per dialog that gave an answer.
        for _ in 0..2000 {
            if woken.load(Ordering::SeqCst) == 2 {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(woken.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_grant_is_one_folder_in_one_direction() {
        let dir = workdir("grant");
        let file = dir.join("a.3dcraft");
        let (workspace, path) = grant(&file, false).unwrap();
        assert_eq!(workspace, rooted(Some(&dir), None));
        assert_eq!(path, text(&file));
        let (workspace, path) = grant(&file, true).unwrap();
        assert_eq!(workspace, rooted(None, Some(&dir)));
        // The engine accepts that path under that grant, and no other
        // folder: not the one above, not one beside it.
        assert_eq!(workspace.resolve_write(&path).unwrap(), file);
        assert!(workspace.resolve_read(&path).is_err());
        assert!(
            workspace
                .resolve_write(text(&dir.parent().unwrap().join("b.3dcraft")))
                .is_err()
        );
        assert!(
            workspace
                .resolve_write(text(&dir.join("../b.3dcraft")))
                .is_err()
        );
        // A relative path is the person's, from where the program runs:
        // made absolute, never left to the roots to place.
        let (workspace, path) = grant(Path::new("rel.3dcraft"), true).unwrap();
        let here = std::env::current_dir().unwrap();
        assert_eq!(path, text(&here.join("rel.3dcraft")));
        assert_eq!(workspace.write, Access::Root(here));
        // No file name, nothing to grant.
        for bad in ["", "/"] {
            assert!(grant(Path::new(bad), true).is_err(), "`{bad}`");
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let odd = dir.join(std::ffi::OsStr::from_bytes(b"caf\xe9.3dcraft"));
            let error = grant(&odd, true).unwrap_err();
            assert!(error.contains("Unicode"), "{error}");
            let mut host = Host::new(Session::new(Workspace::none()));
            assert!(host.open_as_user(&odd).unwrap_err().contains("Unicode"));
            assert_eq!(*host.session.workspace(), Workspace::none());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_for_the_dialog_and_the_extension() {
        assert_eq!(suggested_name(None), "untitled.3dcraft");
        assert_eq!(suggested_name(Some("")), "untitled.3dcraft");
        assert_eq!(
            suggested_name(Some("/home/me/parts/gear.3dcraft")),
            "gear.3dcraft"
        );
        assert_eq!(suggested_name(Some("rel/gear.3dcraft")), "gear.3dcraft");
        assert_eq!(suggested_name(Some("gear")), "gear");
        let ext = |p: &str| with_extension(PathBuf::from(p));
        assert_eq!(ext("/a/gear"), PathBuf::from("/a/gear.3dcraft"));
        assert_eq!(ext("/a/gear.3dcraft"), PathBuf::from("/a/gear.3dcraft"));
        // Another extension is the person's business.
        assert_eq!(ext("/a/gear.bak"), PathBuf::from("/a/gear.bak"));
        assert_eq!(ext("/a.b/gear"), PathBuf::from("/a.b/gear.3dcraft"));
        // The export dialog: the document's name, the last format.
        assert_eq!(suggested_export_name(None, None), "untitled.step");
        assert_eq!(
            suggested_export_name(Some("/home/me/gear.3dcraft"), Some("stl")),
            "gear.stl"
        );
        assert_eq!(suggested_export_name(Some(""), Some("dxf")), "untitled.dxf");
        let kind = |p: &str| craft3d_ui_egui::files::export_kind(p);
        use craft3d_ui_egui::files::ExportKind;
        assert_eq!(kind("a.STEP"), Some(ExportKind::Bodies));
        assert_eq!(kind("a.3mf"), Some(ExportKind::Bodies));
        assert_eq!(kind("a.Svg"), Some(ExportKind::Outline));
        assert_eq!(kind("a.dwg"), None);
        assert_eq!(kind("a"), None);
        // The file the save dialog gave back, named for the format chosen
        // before it (a portal does not say which filter was picked).
        let path = |p: &str, f| export_path(Path::new(p), f);
        assert_eq!(
            path("/a/part", ExportFormat::Stl),
            PathBuf::from("/a/part.stl")
        );
        assert_eq!(
            path("/a/part.stl", ExportFormat::Dxf),
            PathBuf::from("/a/part.dxf")
        );
        assert_eq!(
            path("/a.b/part.STP", ExportFormat::Step),
            PathBuf::from("/a.b/part.STP")
        );
        assert_eq!(
            path("/a/part.v2", ExportFormat::Svg),
            PathBuf::from("/a/part.v2.svg")
        );
    }

    /// Writes a STEP file of one block of the given height, as a person
    /// with a file from another modeller would have it.
    fn write_step(path: &Path, height: f64) {
        let mut s = Session::new(Workspace::unrestricted());
        s.execute("file.new", &json!({})).unwrap();
        s.execute("feature.block", &json!({"size": [10, 10, height]}))
            .unwrap();
        s.execute("file.export", &json!({"path": text(path)}))
            .unwrap();
    }

    #[test]
    fn import_reads_the_chosen_step_file_into_the_document_as_one_step() {
        let dir = workdir("import");
        let (mine, shared) = (subdir(&dir, "mine"), subdir(&dir, "shared"));
        let part = mine.join("part.step");
        write_step(&part, 5.0);
        let on_disk = std::fs::read(&part).unwrap();
        let automation = rooted(Some(&shared), Some(&shared));
        let mut host = Host::new(Session::new(automation.clone()));
        let person = Person::who_answers(&[None, Some(&part), Some(&part)]);
        let mut dialogs = picker(&person);

        // Cancelled: nothing happens, not even a new document.
        act(&mut host, &mut dialogs, FileAction::Import);
        assert_eq!(
            last(&host),
            ("file.import".to_string(), false, "cancelled".to_string())
        );
        assert!(host.session.document_status().is_none());

        // Without a document: a new one, with the file's solid in it, as
        // one undo step.
        host.shell.request_file_action(FileAction::Import);
        assert!(host.pump_files(&mut dialogs));
        let choice = dialogs.wait().unwrap();
        assert_eq!(choice, Ok(Choice::Import(part.clone())));
        host.file_choice(choice);
        let (label, ok, said) = last(&host);
        assert_eq!((label.as_str(), ok), ("file.import", true), "{said}");
        assert_eq!(volume(&mut host), 500.0);
        assert_eq!(host.shell.tree_rows().len(), 1, "the tree shows it already");
        assert_eq!(person.asked(), ["import", "import"]);

        // The agent works with the imported body, but was given no access
        // to the file: the automation roots are as they were.
        assert_eq!(*host.session.workspace(), automation);
        let measured = agent(&mut host, "document.measure", json!({})).unwrap();
        assert_eq!(whole(&measured), 500.0);
        let refused = agent(&mut host, "file.import", json!({"path": text(&part)})).unwrap_err();
        assert!(refused.contains("outside the read root"), "{refused}");

        // Into the document that is open: a second import feature.
        act(&mut host, &mut dialogs, FileAction::Import);
        assert!(last(&host).1, "{:?}", last(&host));
        assert_eq!(volume(&mut host), 1000.0);
        // Each import is one undo step.
        agent(&mut host, "edit.undo", json!({})).unwrap();
        assert_eq!(volume(&mut host), 500.0);
        agent(&mut host, "edit.undo", json!({})).unwrap();
        assert!(agent(&mut host, "edit.undo", json!({})).is_err());
        agent(&mut host, "edit.redo", json!({})).unwrap();
        assert_eq!(volume(&mut host), 500.0);
        assert_eq!(
            std::fs::read(&part).unwrap(),
            on_disk,
            "the file is untouched"
        );

        // A file that cannot be imported: an error for the status bar, the
        // document as it was, the roots back in place.
        let junk = mine.join("junk.step");
        std::fs::write(&junk, b"not a STEP file").unwrap();
        for bad in [junk.clone(), mine.join("missing.step")] {
            let error = host.import_as_user(&bad).unwrap_err();
            assert!(error.contains("file.import"), "{error}");
            assert_eq!(*host.session.workspace(), automation);
            let (label, ok, _) = last(&host);
            assert_eq!((label.as_str(), ok), ("file.import", false));
            assert_eq!(volume(&mut host), 500.0);
        }
        // Without a document, the new one made for it is closed again.
        let mut empty = Host::new(Session::new(Workspace::none()));
        assert!(empty.import_as_user(&junk).is_err());
        assert!(empty.session.document_status().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A block (F1, 10 × 10 × 5) and a cylinder beside it (F2).
    fn two_bodies(host: &mut Host) -> f64 {
        host.session.execute("file.new", &json!({})).unwrap();
        host.session
            .execute("feature.block", &json!({"size": [10, 10, 5]}))
            .unwrap();
        host.session
            .execute(
                "feature.cylinder",
                &json!({"radius": 2, "height": 5, "position": [20, 0, 0]}),
            )
            .unwrap();
        let measured = host
            .session
            .execute("document.measure", &json!({}))
            .unwrap();
        measured["volume"].as_f64().unwrap()
    }

    fn select(host: &mut Host, names: &[&str]) {
        host.session
            .execute("select.set", &json!({ "names": names }))
            .unwrap();
    }

    /// The bytes of a file as text, for the text formats and for what a
    /// stored ZIP (3MF) holds uncompressed.
    fn lossy(path: &Path) -> String {
        String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned()
    }

    #[test]
    fn export_writes_every_format_to_the_chosen_file_and_reads_back() {
        let dir = workdir("export");
        let mine = subdir(&dir, "mine");
        let mut host = Host::new(Session::new(Workspace::none()));
        let total = two_bodies(&mut host);

        // STEP: the exact bodies, read back by the importer.
        let step = mine.join("part.step");
        let result = host.export_as_user(&step).unwrap();
        assert_eq!(result["bodies"], 2);
        let mut reader = Session::new(Workspace::unrestricted());
        reader.execute("file.new", &json!({})).unwrap();
        let imported = reader
            .execute("file.import", &json!({"path": text(&step)}))
            .unwrap();
        assert!(imported.to_string().contains("import"), "{imported}");
        let back = reader.execute("document.measure", &json!({})).unwrap();
        let back = back["volume"].as_f64().unwrap();
        assert!((back - total).abs() < 1e-6 * total, "{back} {total}");

        // The meshes: every body, as many triangles as the export says.
        for name in ["part.stl", "part.3mf", "part.obj", "PART.STL"] {
            let path = mine.join(name);
            let result = host.export_as_user(&path).unwrap();
            assert_eq!(
                (result["bodies"].clone(), result["closed"].clone()),
                (json!(2), json!(2))
            );
            let triangles = result["triangles"].as_u64().unwrap() as usize;
            assert!(triangles > 12, "{result}");
            let bytes = std::fs::read(&path).unwrap();
            match name.to_ascii_lowercase().rsplit('.').next().unwrap() {
                "stl" => {
                    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
                    assert_eq!(count, triangles);
                    assert_eq!(bytes.len(), 84 + 50 * count);
                }
                "3mf" => {
                    assert_eq!(&bytes[..2], b"PK");
                    let text = lossy(&path);
                    assert!(text.contains("3D/3dmodel.model"));
                    assert_eq!(text.matches("<triangle ").count(), triangles);
                    assert_eq!(text.matches("<object ").count(), 2);
                }
                _ => {
                    let text = lossy(&path);
                    assert_eq!(
                        text.lines().filter(|l| l.starts_with("f ")).count(),
                        triangles
                    );
                    assert_eq!(text.lines().filter(|l| l.starts_with("o ")).count(), 2);
                }
            }
            let (label, ok, _) = last(&host);
            assert_eq!((label.as_str(), ok), ("file.export", true));
        }

        // Nothing selected, to an outline format: the model as seen
        // (the window's view, to scale; 2026-10-23, before that refused).
        for name in ["seen.dxf", "seen.svg"] {
            let result = host.export_as_user(&mine.join(name)).unwrap();
            assert!(result["visible"].as_u64().unwrap_or(0) > 0, "{result}");
            assert_eq!(result["projection"], "orthographic");
            assert!(mine.join(name).exists());
        }
        // The block's top face selected: its outline, 10 × 10.
        select(&mut host, &["F1:face(1)"]);
        let result = host.export_as_user(&mine.join("top.dxf")).unwrap();
        assert_eq!(result["lines"], 4, "{result}");
        let dxf = lossy(&mine.join("top.dxf"));
        assert!(
            dxf.contains("ENTITIES") && dxf.trim_end().ends_with("EOF"),
            "{dxf}"
        );
        let result = host.export_as_user(&mine.join("top.svg")).unwrap();
        assert_eq!(result["lines"], 4, "{result}");
        let svg = lossy(&mine.join("top.svg"));
        assert!(
            svg.contains("<svg") && svg.contains("width=\"10mm\""),
            "{svg}"
        );
        // Flat faces of two bodies (the block's top, the cylinder's base):
        // one sheet, a square and a circle.
        select(&mut host, &["F1:face(1)", "F2:face(1)"]);
        let result = host.export_as_user(&mine.join("two.dxf")).unwrap();
        assert_eq!(
            (result["lines"].clone(), result["circles"].clone()),
            (json!(4), json!(1)),
            "{result}"
        );

        // The cylinder's wall is not flat: file.export says so.
        select(&mut host, &["F2:face(0)"]);
        let error = host.export_as_user(&mine.join("side.dxf")).unwrap_err();
        assert!(error.contains("file.export"), "{error}");
        assert!(!mine.join("side.dxf").exists());

        // Nobody's access was widened.
        assert_eq!(*host.session.workspace(), Workspace::none());
        for command in ["file.export", "file.import"] {
            assert!(agent(&mut host, command, json!({"path": text(&step)})).is_err());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_refuses_a_selection_the_format_cannot_hold_and_says_why() {
        let dir = workdir("export-selection");
        let mine = subdir(&dir, "mine");
        let mut host = Host::new(Session::new(Workspace::none()));
        two_bodies(&mut host);
        let stl = mine.join("part.stl");

        // A face, or a body and a face, to a mesh or STEP file: refused,
        // naming what is selected, and nothing is written.
        for names in [&["F1:face(1)"][..], &["F1", "F2:face(1)"]] {
            select(&mut host, names);
            for path in [stl.clone(), mine.join("part.step")] {
                let error = host.export_as_user(&path).unwrap_err();
                assert!(
                    error.contains("cannot hold what is selected")
                        && error.contains("Select only bodies")
                        && error.contains("clear the selection"),
                    "{names:?}: {error}"
                );
                let (label, ok, said) = last(&host);
                assert_eq!((label.as_str(), ok), ("file.export", false));
                assert_eq!(said, error.chars().take(160).collect::<String>());
            }
            assert!(listing(&mine).is_empty(), "{names:?}");
        }
        let error = host.export_as_user(&stl).unwrap_err();
        assert!(error.contains("1 body, 1 face"), "{error}");
        // Bodies, or an edge, to an outline: refused.
        for names in [&["F1"][..], &["F1:face(1)", "F2"]] {
            select(&mut host, names);
            let error = host.export_as_user(&mine.join("a.dxf")).unwrap_err();
            assert!(
                error.contains("select only flat faces"),
                "{names:?}: {error}"
            );
        }
        let edges = host
            .session
            .execute(
                "select.query",
                &json!({"kind": "edge", "limit": 1, "then": "replace"}),
            )
            .unwrap();
        assert_eq!(edges["count"].as_u64().map(|c| c > 0), Some(true));
        let error = host.export_as_user(&mine.join("a.svg")).unwrap_err();
        assert!(error.contains("select only flat faces"), "{error}");
        assert!(listing(&mine).is_empty());

        // Every body selected is the whole document: written.
        select(&mut host, &["F1", "F2"]);
        let result = host.export_as_user(&stl).unwrap();
        assert_eq!(result["bodies"], 2);
        assert_eq!(listing(&mine), ["part.stl"]);

        // One body of two: that body alone, to every body format, by the
        // number of its feature's id (a string is read as a name first,
        // and an old file may name a feature like another's id); the STEP
        // file reads back to the cylinder's volume.
        select(&mut host, &["F2"]);
        assert_eq!(
            craft3d_ui_egui::files::export_selection(&mut host.session, &stl.to_string_lossy()),
            Ok(Some(("bodies", json!([2]))))
        );
        let pins = ["pin.step", "pin.stl", "pin.3mf", "pin.obj"];
        for name in pins {
            let result = host.export_as_user(&mine.join(name)).unwrap();
            assert_eq!(result["bodies"], 1, "{name}: {result}");
            let (label, ok, _) = last(&host);
            assert_eq!((label.as_str(), ok), ("file.export", true));
        }
        let mut reader = Session::new(Workspace::unrestricted());
        reader.execute("file.new", &json!({})).unwrap();
        reader
            .execute(
                "file.import",
                &json!({"path": text(&mine.join("pin.step"))}),
            )
            .unwrap();
        let back = reader.execute("document.measure", &json!({})).unwrap();
        let back = back["volume"].as_f64().unwrap();
        let pin = 20.0 * std::f64::consts::PI;
        assert!((back - pin).abs() < 1e-6 * pin, "{back} {pin}");
        let obj = lossy(&mine.join("pin.obj"));
        let objects: Vec<&str> = obj.lines().filter(|l| l.starts_with("o ")).collect();
        assert_eq!(objects.len(), 1, "{obj}");
        assert_eq!(*host.session.workspace(), Workspace::none());
        for name in pins {
            std::fs::remove_file(mine.join(name)).unwrap();
        }

        // One sketch selected, to an outline: its lines. With a face
        // beside it, or to a mesh: refused.
        for (command, params) in [
            ("sketch.create", json!({"name": "Outline"})),
            (
                "sketch.polyline",
                json!({"sketch": "Outline", "points": [[0, 0], [8, 0], [8, 3]]}),
            ),
        ] {
            host.session.execute(command, &params).unwrap();
        }
        select(&mut host, &["Outline"]);
        let result = host.export_as_user(&mine.join("outline.svg")).unwrap();
        assert_eq!(result["lines"], 2, "{result}");
        assert!(host.export_as_user(&mine.join("outline.obj")).is_err());
        select(&mut host, &["Outline", "F1:face(1)"]);
        let error = host.export_as_user(&mine.join("both.dxf")).unwrap_err();
        assert!(error.contains("select only flat faces"), "{error}");
        assert_eq!(listing(&mine), ["outline.svg", "part.stl"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// File › Export shows the export dialog first (what, the format, the
    /// options), and only its Export… opens the save dialog: that format's
    /// filter, the name with its extension, the last folder; what the save
    /// dialog gives back is written at once, named for the format
    /// (2026-10-27, the developer: the portal kept `.stl` whatever filter
    /// was picked, and the options came only after the name was fixed by
    /// hand).
    #[test]
    fn export_shows_its_dialog_first_then_asks_where_in_that_format() {
        let dir = workdir("export-dialog");
        let mine = subdir(&dir, "mine");
        let mut host = Host::new(Session::new(Workspace::none()));

        // Nothing to export: no dialog.
        let nobody = Person::who_answers(&[]);
        let mut dialogs = picker(&nobody);
        act(&mut host, &mut dialogs, FileAction::Export);
        let (label, ok, said) = last(&host);
        assert_eq!((label.as_str(), ok), ("file.export", false));
        assert!(said.contains("no document"), "{said}");
        assert!(host.shell.export_dialog().is_none());

        two_bodies(&mut host);
        let person = Person::who_answers(&[
            None,
            Some(&mine.join("part")),
            Some(&mine.join("again.dxf")),
            Some(&mine.join("part")),
        ]);
        let mut dialogs = picker(&person);
        // The export dialog, and no save dialog yet: the model, as STEP
        // the first time. Cancelled: nothing asked, nothing written.
        act(&mut host, &mut dialogs, FileAction::Export);
        assert!(host.shell.export_dialog().is_some());
        assert!(host.shell.state.export.open);
        assert_eq!(host.shell.state.export.what, ExportWhat::Model);
        assert_eq!(host.shell.state.export.format, ExportFormat::Step);
        assert_eq!(host.shell.state.export.name, "untitled.step");
        host.shell.answer_export(false);
        assert!(!host.shell.state.export.open);
        assert_eq!(
            last(&host),
            ("file.export".to_string(), false, "cancelled".to_string())
        );
        assert!(!host.pump_files(&mut dialogs));
        assert!(person.asked().is_empty());
        // STL at fine quality, through the shell's own request as the menu
        // makes it; then the save dialog is cancelled.
        host.shell.request_file_action(FileAction::Export);
        assert!(host.pump_files(&mut dialogs));
        host.shell.set_export_format(ExportFormat::Stl).unwrap();
        assert_eq!(host.shell.state.export.name, "untitled.stl");
        host.shell.state.export.quality = craft3d_ui_egui::files::ExportQuality::Fine;
        host.shell.answer_export(true);
        assert!(host.pump_files(&mut dialogs));
        let choice = dialogs.wait().unwrap();
        assert_eq!(choice, Ok(Choice::Cancelled("file.export")));
        host.file_choice(choice);
        assert!(listing(&mine).is_empty());
        // Again: the save dialog was cancelled, so nothing was kept (the
        // review of r1-exportflow); STL at fine quality chosen again. The
        // save dialog gives a name without an extension, and the file is
        // written at once as part.stl, with no second question.
        act(&mut host, &mut dialogs, FileAction::Export);
        assert_eq!(host.shell.state.export.format, ExportFormat::Step);
        assert_eq!(
            host.shell.state.export.quality,
            craft3d_ui_egui::files::ExportQuality::Normal
        );
        host.shell.set_export_format(ExportFormat::Stl).unwrap();
        host.shell.state.export.quality = craft3d_ui_egui::files::ExportQuality::Fine;
        host.shell.answer_export(true);
        assert!(host.pump_files(&mut dialogs));
        host.file_choice(dialogs.wait().unwrap());
        assert!(host.shell.export_dialog().is_none());
        let (label, ok, said) = last(&host);
        assert!(ok && label == "file.export", "{said}");
        // Said in words; the answer itself has the quality's angle.
        assert!(
            said.starts_with("Exported") && !craft3d_ui_egui::answer::looks_like_json(&said),
            "{said}"
        );
        let answer = host.last_answer.as_ref().unwrap().to_string();
        assert!(answer.contains("\"angle\":5.0"), "{answer}");
        assert_eq!(listing(&mine), ["part.stl"]);
        // A binary STL: a header of 80 bytes, the count, the triangles.
        assert!(std::fs::read(mine.join("part.stl")).unwrap().len() > 84);
        // Exporting is not saving: the document still has changes.
        assert!(host.session.document_status().unwrap().dirty);
        // Another format's extension typed in the save dialog: corrected,
        // never STL bytes under .dxf.
        act(&mut host, &mut dialogs, FileAction::Export);
        host.shell.answer_export(true);
        assert!(host.pump_files(&mut dialogs));
        host.file_choice(dialogs.wait().unwrap());
        assert_eq!(listing(&mine), ["again.stl", "part.stl"]);
        // A name without an extension whose corrected name is a file the
        // save dialog did not ask about replacing: refused, said in the
        // error dialog, the file as it was.
        let before = std::fs::read(mine.join("part.stl")).unwrap();
        host.shell.state.export.quality = craft3d_ui_egui::files::ExportQuality::Coarse;
        act(&mut host, &mut dialogs, FileAction::Export);
        host.shell.answer_export(true);
        assert!(host.pump_files(&mut dialogs));
        host.file_choice(dialogs.wait().unwrap());
        let error = host.shell.error_dialog().unwrap().clone();
        assert_eq!(error.title, "Could not export");
        assert!(error.message.contains("exists already"), "{error:?}");
        assert_eq!(std::fs::read(mine.join("part.stl")).unwrap(), before);
        host.shell.close_error_dialog();
        // The save dialog had that format's filter and the proposed name
        // with its extension, and opened in the last export's folder.
        let in_mine = format!("export untitled.stl [STL] in {}", mine.display());
        assert_eq!(
            person.asked(),
            [
                "export untitled.stl [STL]".to_string(),
                "export untitled.stl [STL]".to_string(),
                in_mine.clone(),
                in_mine,
            ]
        );
        assert_eq!(*host.session.workspace(), Workspace::none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The dialog offers what the selection fits, the model and only the
    /// formats that hold what is chosen; the person's failures open the
    /// error dialog, and nothing is written before the save dialog named
    /// the file.
    #[test]
    fn the_dialog_offers_what_fits_and_failures_open_the_error_dialog() {
        let dir = workdir("error-dialog");
        let mine = subdir(&dir, "mine");
        let mut host = Host::new(Session::new(Workspace::none()));
        two_bodies(&mut host);
        let person = Person::who_answers(&[
            Some(&mine.join("part.step")),
            Some(&mine.join("seen")),
            Some(&mine.join("face.dxf")),
            Some(&mine.join("late.step")),
        ]);
        let mut dialogs = picker(&person);
        // A face selected: the selection to DXF or SVG, chosen first; STEP
        // cannot hold it, the model can.
        select(&mut host, &["F1:face(0)"]);
        act(&mut host, &mut dialogs, FileAction::Export);
        assert_eq!(host.shell.state.export.what, ExportWhat::Selection);
        assert_eq!(host.shell.state.export.format, ExportFormat::Dxf);
        let refused = host
            .shell
            .set_export_format(ExportFormat::Step)
            .unwrap_err();
        assert!(refused.contains("cannot hold the selection"), "{refused}");
        host.shell.set_export_what(ExportWhat::Model).unwrap();
        host.shell.set_export_format(ExportFormat::Step).unwrap();
        assert!(listing(&mine).is_empty());
        host.shell.answer_export(true);
        assert!(host.pump_files(&mut dialogs));
        host.file_choice(dialogs.wait().unwrap());
        let (label, ok, said) = last(&host);
        assert!(ok && label == "file.export", "{said}");
        assert!(
            said.starts_with("Exported") && !craft3d_ui_egui::answer::looks_like_json(&said),
            "{said}"
        );
        let answer = host.last_answer.as_ref().unwrap().to_string();
        assert!(answer.contains("\"bodies\":2"), "{answer}");
        assert_eq!(listing(&mine), ["part.step"]);
        // The model to DXF: a drawing of it as seen, from the view asked,
        // whatever is selected.
        act(&mut host, &mut dialogs, FileAction::Export);
        host.shell.set_export_what(ExportWhat::Model).unwrap();
        host.shell.set_export_format(ExportFormat::Dxf).unwrap();
        host.shell.state.export.drawing.view = craft3d_ui_egui::files::DrawingView::Front;
        host.shell.state.export.drawing.dashed = true;
        host.shell.answer_export(true);
        assert!(host.pump_files(&mut dialogs));
        host.file_choice(dialogs.wait().unwrap());
        let (label, ok, said) = last(&host);
        assert!(ok && label == "file.export", "{said}");
        assert_eq!(listing(&mine), ["part.step", "seen.dxf"]);
        assert!(
            lossy(&mine.join("seen.dxf")).contains("SECTION"),
            "a DXF file"
        );
        // An edge selected: no format holds it; the dialog says why and
        // offers the model.
        host.session
            .execute(
                "select.query",
                &json!({"kind": "edge", "limit": 1, "then": "replace"}),
            )
            .unwrap();
        act(&mut host, &mut dialogs, FileAction::Export);
        assert_eq!(host.shell.state.export.what, ExportWhat::Model);
        let refused = host
            .shell
            .set_export_what(ExportWhat::Selection)
            .unwrap_err();
        assert!(refused.contains("(1 edge) cannot be exported"), "{refused}");
        assert_eq!(host.shell.state.export.what, ExportWhat::Model);
        host.shell.answer_export(false);
        assert!(host.shell.error_dialog().is_none());
        // The selection is read when Export… is clicked: emptied while the
        // save dialog is open, the file still holds the face chosen,
        // never the model in its place (review of r1-exportflow).
        select(&mut host, &["F1:face(0)"]);
        act(&mut host, &mut dialogs, FileAction::Export);
        assert_eq!(host.shell.state.export.what, ExportWhat::Selection);
        host.shell.set_export_format(ExportFormat::Dxf).unwrap();
        host.shell.answer_export(true);
        assert!(host.pump_files(&mut dialogs));
        select(&mut host, &[]);
        host.file_choice(dialogs.wait().unwrap());
        let (label, ok, said) = last(&host);
        assert!(ok && label == "file.export", "{said}");
        select(&mut host, &["F1:face(0)"]);
        host.export_as_user(&mine.join("direct.dxf")).unwrap();
        assert_eq!(
            std::fs::read(mine.join("face.dxf")).unwrap(),
            std::fs::read(mine.join("direct.dxf")).unwrap(),
            "the face's outline, as exported with it selected"
        );
        std::fs::remove_file(mine.join("face.dxf")).unwrap();
        std::fs::remove_file(mine.join("direct.dxf")).unwrap();
        // Another document shown while the save dialog was open: nothing
        // written, said in the error dialog.
        select(&mut host, &[]);
        act(&mut host, &mut dialogs, FileAction::Export);
        host.shell.set_export_format(ExportFormat::Step).unwrap();
        host.shell.answer_export(true);
        assert!(host.pump_files(&mut dialogs));
        host.session.execute("file.new", &json!({})).unwrap();
        host.file_choice(dialogs.wait().unwrap());
        let error = host.shell.error_dialog().unwrap().clone();
        assert!(error.message.starts_with("Another document"), "{error:?}");
        assert_eq!(listing(&mine), ["part.step", "seen.dxf"]);
        host.shell.close_error_dialog();
        // A file that cannot be opened.
        let missing = dir.join("missing.3dcraft");
        let _ = host.open_as_user(&missing);
        let error = host.shell.error_dialog().unwrap();
        assert_eq!(error.title, "Could not open the file");
        assert!(!error.message.starts_with("file.open"), "{error:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
