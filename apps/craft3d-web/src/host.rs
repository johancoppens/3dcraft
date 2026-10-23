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
    EXPORT_FORMATS, EXTENSION, ExportPlan, JIG_EXTENSION, export_check, export_kind,
    export_options_ui, export_params, extension_of, guarded, suggested_export_name,
    suggested_jig_name, suggested_name,
};
use craft3d_ui_egui::shell::export_verb;
use craft3d_ui_egui::{CommandReport, DocAction, FileAction, Shell};
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
                description: "3DCraft document or Jig",
                mime: "application/x-craft3d",
                extensions: vec![EXTENSION, JIG_EXTENSION],
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
    /// Export: the format follows the extension.
    Export,
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
        EXTENSION | JIG_EXTENSION => Some(Received::Document),
        "step" | "stp" => Some(Received::Step),
        _ => None,
    }
}

pub struct WebHost {
    pub session: Session,
    pub shell: Shell,
    /// The extension of the last export, for the next one's name.
    pub export_extension: Option<String>,
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
}

impl WebHost {
    /// A host whose session keeps its files in memory, around `shell`.
    pub fn new(shell: Shell) -> Self {
        Self {
            session: Session::new(Workspace::memory()),
            shell,
            export_extension: None,
            dialog: None,
            pickers: false,
            writable: BTreeSet::new(),
            companion: None,
        }
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
        let (ok, text) = match outcome {
            Ok(result) => (true, result.to_string()),
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

    fn run(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        guarded(&mut self.session, command, params)
    }

    /// Reports `outcome` under `label`, empties the memory and shows what
    /// the session holds now.
    fn finish(&mut self, label: &str, outcome: &Result<Value, String>) {
        self.report(label, outcome);
        self.forget_files();
        self.shell.sync(&mut self.session);
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
        let outcome = self.receive_inner(name, bytes, wanted);
        let label = match received_kind(name) {
            Some(Received::Step) => "file.import",
            _ => "file.open",
        };
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
            format!("`{name}` is not a 3DCraft document (.3dcraft), a Jig (.3dcraftjig) or a STEP file (.step, .stp)")
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
        if let Some(action) = self.shell.take_file_action() {
            outcome = self.file_action(action);
        }
        if let Some(action) = self.shell.take_doc_action() {
            let more = self.doc_action(action);
            outcome.downloads.extend(more.downloads);
            outcome.ask = outcome.ask.or(more.ask);
            outcome.pick = outcome.pick.or(more.pick);
        }
        // An example from the start screen, built in a new document.
        if let Some(example) = self.shell.take_example() {
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
            let done = self.run(asked.command, &asked.params);
            self.shell.command_done(done.clone());
            self.finish(asked.command, &done);
        }
        outcome
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
            FileAction::Export => {
                let Some(status) = self.session.document_status() else {
                    self.report(
                        "file.export",
                        &Err("there is no document to export".to_string()),
                    );
                    return outcome;
                };
                // The format is chosen first, in the dialog over the page:
                // with the file dialogs, the export is made before the
                // system's dialog is shown ([`Pick::Export`]).
                self.dialog = Some(NameDialog {
                    purpose: Purpose::Export,
                    name: suggested_export_name(
                        status.path.as_deref(),
                        self.export_extension.as_deref(),
                    ),
                });
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
        let plan = ExportPlan {
            quality: self.shell.export_quality(),
            all_bodies: false,
        };
        self.export_with(name, plan)
    }

    /// [`Self::export_as`] as the export dialog was answered: the quality
    /// of a mesh, and every body instead of a selection the format cannot
    /// hold ([`craft3d_ui_egui::files::export_params`]).
    pub fn export_with(&mut self, name: &str, plan: ExportPlan) -> Vec<Download> {
        let Some(name) = memory_name(name) else {
            self.finish("file.export", &Err(format!("`{name}` is not a file name")));
            return Vec::new();
        };
        if export_kind(&name).is_some() {
            self.export_extension = extension_of(&name);
        }
        let exported = export_params(&mut self.session, &name, plan).and_then(|extra| {
            let mut params = json!({ "path": name });
            if let Some(map) = params.as_object_mut() {
                for (key, value) in extra {
                    map.insert(key.to_string(), value);
                }
            }
            self.run("file.export", &params)
        });
        let mut downloads = Vec::new();
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
                Purpose::Export => "file.export",
                Purpose::ExportJig => "jig.export",
            };
            self.report(label, &Err("cancelled".to_string()));
            return outcome;
        }
        match dialog.purpose {
            Purpose::Save { close } => outcome.downloads = self.save_as(&dialog.name, close),
            Purpose::Export => {
                // The dialog's button: every body when the selection does
                // not fit the format ("Export all bodies").
                let plan = ExportPlan {
                    quality: self.shell.export_quality(),
                    all_bodies: export_check(&mut self.session, &dialog.name).offers_all_bodies(),
                };
                let files = self.export_with(&dialog.name, plan);
                if self.pickers {
                    let mut files = files.into_iter();
                    if let Some(file) = files.next() {
                        outcome.pick = Some(Pick::Export { file });
                    }
                    self.companion = files.next();
                } else {
                    outcome.downloads = files;
                }
            }
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
        egui::Window::new("Material library")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "The OBJ's colours are in `{}`. Save it next to the OBJ.",
                    file.name
                ));
                ui.horizontal(|ui| {
                    if ui.button("Save…").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Skip").clicked() {
                        answer = Some(false);
                    }
                });
            });
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

    /// Draws the dialog in which the person names a file, while it is open;
    /// what its answer leaves for the page.
    pub fn dialog_ui(&mut self, ctx: &egui::Context) -> Outcome {
        let Some(mut dialog) = self.dialog.take() else {
            return Outcome::default();
        };
        let mut answer = None;
        let title = match dialog.purpose {
            Purpose::Save { .. } => "Save As",
            Purpose::Export => "Export",
            Purpose::ExportJig => "Export as Jig",
        };
        // What the export dialog says of the format and the selection,
        // and the quality chosen last (`craft3d_ui_egui::files`).
        let check = (dialog.purpose == Purpose::Export)
            .then(|| export_check(&mut self.session, &dialog.name));
        let mut plan = ExportPlan {
            quality: self.shell.export_quality(),
            all_bodies: false,
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                if dialog.purpose == Purpose::Export {
                    ui.label("Format (the file's extension):");
                    ui.horizontal_wrapped(|ui| {
                        for (format, extensions) in EXPORT_FORMATS {
                            let extension = extensions.first().copied().unwrap_or_default();
                            let current = extension_of(&dialog.name);
                            let chosen =
                                current.as_deref().is_some_and(|e| extensions.contains(&e));
                            if ui.selectable_label(chosen, format).clicked() {
                                dialog.name = renamed(&dialog.name, extension);
                            }
                        }
                    });
                    if matches!(extension_of(&dialog.name).as_deref(), Some("dxf" | "svg")) {
                        ui.label(
                            "DXF and SVG hold the outline of the selected flat faces or sketch.",
                        );
                    }
                    if let Some(check) = &check {
                        export_options_ui(ui, check, &mut plan);
                    }
                }
                ui.label("File name:");
                let field = ui.text_edit_singleline(&mut dialog.name);
                if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    answer = Some(true);
                }
                ui.horizontal(|ui| {
                    // With the file dialogs, Export goes on to the
                    // system's save dialog.
                    let verb = match (&dialog.purpose, &check) {
                        (Purpose::Save { .. }, _) | (Purpose::Export, None) => "Save".to_string(),
                        (Purpose::Export, Some(check)) if self.pickers => {
                            format!("{}…", export_verb(check))
                        }
                        (Purpose::Export, Some(check)) => export_verb(check).to_string(),
                        (Purpose::ExportJig, _) if self.pickers => "Export…".to_string(),
                        (Purpose::ExportJig, _) => "Export".to_string(),
                    };
                    if ui.button(verb).clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(false);
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    answer = Some(false);
                }
            });
        self.shell.set_export_quality(plan.quality);
        match answer {
            Some(confirmed) => self.answer(dialog, confirmed),
            None => {
                self.dialog = Some(dialog);
                Outcome::default()
            }
        }
    }
}

/// `name` with its extension replaced by `extension` (added when it has
/// none).
fn renamed(name: &str, extension: &str) -> String {
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("untitled");
    format!("{stem}.{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> WebHost {
        WebHost::new(Shell::new())
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

    /// Export as Jig downloads a `.3dcraftjig`; handed back to the page it
    /// opens as a Jig in use, which Save saves as a model under a name of
    /// its own (never the Jig's) and Open for Editing copies into a model.
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
        // Open for Editing: a new model of a Jig in use.
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
        assert_eq!(received_kind("a.STP"), Some(Received::Step));
        assert_eq!(received_kind("a.stl"), None);
        assert_eq!(renamed("a.step", "stl"), "a.stl");
        assert_eq!(with_extension("a", EXTENSION), "a.3dcraft");
        assert_eq!(with_extension("a.3DCRAFT", EXTENSION), "a.3DCRAFT");
        assert_eq!(with_extension("x.stl", EXTENSION), "x.stl.3dcraft");
        assert_eq!(renamed("", "obj"), "untitled.obj");
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
                // Outlines need a selected face or sketch.
                "dxf" | "svg" => {
                    assert!(downloads.is_empty(), "{format}");
                    let report = host.shell.last_command.as_ref().unwrap();
                    assert!(
                        !report.ok && report.text.contains("select"),
                        "{}",
                        report.text
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
        host.file_action(FileAction::Export);
        assert_eq!(host.dialog.as_ref().unwrap().name, "untitled.svg");
        let cancelled = host.answer(host.dialog.clone().unwrap(), false);
        assert_eq!(cancelled, Outcome::default());
        // A cancelled dialog is no error; a failed export is (2026-10-23).
        assert_eq!(
            host.shell.error_dialog().map(|e| e.title.as_str()),
            Some("Could not export"),
            "the outlines above failed"
        );
    }

    /// The export dialog's quality and its way out of a selection the
    /// format cannot hold (2026-10-23).
    #[test]
    fn the_dialog_exports_at_its_quality_and_every_body_for_a_misfit_selection() {
        let mut host = modelled();
        let face = |host: &mut WebHost| {
            host.session
                .execute(
                    "select.set",
                    &json!({"items": [{"kind": "face", "name": "F1:face(0)", "feature": "F1"}]}),
                )
                .unwrap();
        };
        face(&mut host);
        let check = export_check(&mut host.session, "plate.step");
        assert!(check.offers_all_bodies());
        assert_eq!(export_verb(&check), "Export all bodies");
        host.shell
            .set_export_quality(craft3d_ui_egui::files::ExportQuality::Coarse);
        let dialog = NameDialog {
            purpose: Purpose::Export,
            name: "plate.step".to_string(),
        };
        let outcome = host.answer(dialog.clone(), true);
        assert_eq!(outcome.downloads.len(), 1, "every body, despite the face");
        assert!(host.shell.error_dialog().is_none());
        // A mesh at the quality chosen: coarse, 20 degrees.
        let outcome = host.answer(
            NameDialog {
                name: "plate.stl".to_string(),
                ..dialog
            },
            true,
        );
        assert_eq!(outcome.downloads.len(), 1);
        let report = host.shell.last_command.as_ref().unwrap();
        assert!(
            report.ok && report.text.contains("\"angle\":20.0"),
            "{}",
            report.text
        );
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

    #[test]
    fn with_file_dialogs_the_export_is_made_before_the_dialog() {
        let mut host = modelled();
        host.pickers = true;
        host.export_extension = Some("stl".to_string());
        // The format is chosen first, in the dialog over the page.
        assert!(host.file_action(FileAction::Export).pick.is_none());
        let dialog = host.dialog.take().unwrap();
        assert_eq!(dialog.name, "untitled.stl");
        let outcome = host.answer(dialog, true);
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
        // An export that fails shows no dialog at all.
        let failed = host.answer(
            NameDialog {
                purpose: Purpose::Export,
                name: "plate.dxf".to_string(),
            },
            true,
        );
        assert_eq!(failed, Outcome::default());
        assert!(!host.shell.last_command.as_ref().unwrap().ok);
        // An OBJ's material library waits for its own click.
        let obj = host.answer(
            NameDialog {
                purpose: Purpose::Export,
                name: "plate.obj".to_string(),
            },
            true,
        );
        assert!(matches!(obj.pick, Some(Pick::Export { ref file }) if file.name == "plate.obj"));
        assert_eq!(host.companion.as_ref().unwrap().name, "plate.mtl");
        let step = Pick::Export {
            file: Download::export("a.step".to_string(), vec![1]),
        };
        assert!(step.accepts("a.stp") && step.accepts("a.step"));
        assert_eq!(export_types(None)[0].extensions, ["step", "stp"]);
        assert_eq!(export_types(Some("svg"))[0].description, "SVG");
        assert_eq!(open_types(Ask::Import)[0].extensions, ["step", "stp"]);
        // Open takes a document or a published Jig (2026-10-23).
        assert_eq!(
            open_types(Ask::Open)[0].extensions,
            [EXTENSION, JIG_EXTENSION]
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
        let obj = host
            .answer(
                NameDialog {
                    purpose: Purpose::Export,
                    name: "plate.obj".to_string(),
                },
                true,
            )
            .pick
            .unwrap();
        let outcome = host.without_picker(obj, "SecurityError: no");
        let names: Vec<&str> = outcome.downloads.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["plate.obj", "plate.mtl"]);
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
        host.pump();
        assert_eq!(host.session.document_statuses().len(), 1);
        let report = host.shell.last_command.as_ref().unwrap();
        assert!(report.ok, "{}", report.text);
        // Nothing is left in memory, and nothing is recent in the browser.
        assert!(host.session.file_names().is_empty());
        assert!(host.shell.recent_files().is_empty());
    }
}
