//! The window's own small dialogs for what the person started (2026-10-23,
//! the developer: a failed Save or Export only showed in the status bar):
//!
//! - **An error dialog** when an action the person started from a menu, a
//!   tab or a dialog fails (Open, Save, Save As, Import, Export, New, an
//!   example): what went wrong in plain words, OK, and "Copy details", the
//!   bug report as it was when the error came (`report.rs`). Failures of
//!   an agent's commands stay in the status bar: an agent reads them from
//!   its reply, and a dialog would stop the person for nothing. Notices
//!   that something still worked another way (a download in place of the
//!   system's dialog) stay there too.
//! - **The export dialog**, first thing File › Export… shows (2026-10-27,
//!   the developer: the system's save dialog first could not say which
//!   format was picked in it): *What* goes into the file (the model, the
//!   selection, a set's sheet, a part's flat pattern, a fold's crease
//!   pattern, every part's flat patterns), the *Format*, only those that
//!   hold that, and the options (a mesh's quality; a drawing's view,
//!   projection and hidden lines; a sheet's width and gap). *Export…*
//!   then leaves the request to the host ([`Shell::take_export_answer`]):
//!   the desktop asks where with the system's save dialog (that format's
//!   filter only, the name proposed with its extension) and writes the
//!   file at once; the browser downloads it or asks with its own file
//!   dialog. Where the host has no save dialog (a browser without the File
//!   System Access API) the dialog asks the file's name too
//!   ([`Shell::set_export_names_file`]). Its state is the window's
//!   (`UiState::export`, `ui.set {"export": …}`); it belongs to the
//!   document shown when it was opened, and is refused when another one
//!   is shown by the time it is answered.
//!
//! Both are modal: nothing under them takes a click, and they are drawn
//! first in a frame, so that Enter and Escape answer them and nothing else
//! (they take those keys out of the frame's input).

use super::Shell;
use crate::files::{
    DrawingPlan, DrawingView, ExportChoices, ExportFormat, ExportKind, ExportPlan, ExportQuality,
    ExportWhat, MAX_EXPORT_TEXT, export_choices, export_file_name, plain_message,
    suggested_export_name, what_hint,
};
use craft3d_engine::Session;
use serde_json::{Value, json};

/// An error shown in a dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorDialog {
    /// What failed, as a title: "Could not save".
    pub title: String,
    /// Why, for a person.
    pub message: String,
    /// The bug report as it was when the error came: what "Copy details"
    /// copies, whatever happens in the window meanwhile.
    pub report: String,
    /// "Copy details" was clicked.
    pub copied: bool,
}

/// The export dialog while it is open: what the document offers, read
/// when it was opened. What it has chosen is `UiState::export`.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportAsk {
    pub choices: ExportChoices,
    /// The document shown when it was opened (`DocumentStatus::id`).
    pub document: Option<u64>,
    /// `UiState::export` as it was before the dialog opened: what it is
    /// again when the dialog is answered, until the file is written
    /// ([`Shell::export_written`]). So a cancel, here or in the save
    /// dialog, or a failed export changes nothing.
    pub committed: crate::state::ExportState,
}

/// What the export dialog was answered with: the file to name and how to
/// export it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportRequest {
    /// The file's name, with the format's extension: the one the system's
    /// save dialog proposes, or the file's own where there is none.
    pub name: String,
    pub format: ExportFormat,
    pub plan: ExportPlan,
    /// The document it is for (`DocumentStatus::id`): the host refuses it
    /// when another one is shown by the time the file is named.
    pub document: Option<u64>,
    /// What the dialog had chosen, kept for the next export once the file
    /// is written ([`Shell::export_written`]).
    pub chosen: crate::state::ExportState,
}

/// A sheet or a pattern chosen last for a document, with its sheet's
/// width and gap.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
struct Remembered {
    what: ExportWhat,
    width: String,
    gap: String,
}

/// What the dialogs keep.
#[derive(Default)]
pub(super) struct Dialogs {
    error: Option<ErrorDialog>,
    export: Option<ExportAsk>,
    answer: Option<ExportRequest>,
    /// The sheet or pattern chosen last, per document
    /// (`DocumentStatus::id`): sets and parts are a document's own.
    sheets: std::collections::BTreeMap<Option<u64>, Remembered>,
    /// The host has no save dialog: the export dialog asks the name.
    names_file: bool,
    /// `ui.set {"export": …}` gave these while the dialog was closed: the
    /// next opening honours them (what, when the document offers it; the
    /// sheet's width and gap; the name).
    preset: Preset,
}

/// What `ui.set` chose for the next opening of the export dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Preset {
    what: bool,
    sheet: bool,
    name: bool,
}

/// The export dialog's button: "Export…" when the system's save dialog
/// follows, "Export" where the dialog names the file itself.
pub fn export_verb(names_file: bool) -> &'static str {
    if names_file { "Export" } else { "Export…" }
}

/// The title of the error dialog for an action, by the label the status
/// bar shows it under.
pub fn error_title(label: &str) -> String {
    match label {
        "file.open" => "Could not open the file",
        "file.save" => "Could not save",
        "file.import" => "Could not import",
        "file.export" => "Could not export",
        "file.new" => "Could not make a new document",
        "file.close" => "Could not close the document",
        "file.activate" => "Could not show the document",
        "example" => "Could not build the example",
        "download" => "Could not download",
        _ => "Something went wrong",
    }
    .to_string()
}

/// The longest line the status bar keeps of a failure, in characters (as
/// the hosts cut it).
const STATUS_CHARS: usize = 160;

/// A *What* as `ui.get` and `ui.set` write it.
fn what_json(what: &ExportWhat) -> Value {
    serde_json::to_value(what).unwrap_or(Value::Null)
}

impl Shell {
    /// Shows the error dialog for an action the person started that
    /// failed, labelled as the status bar labels it (`file.save`), and
    /// puts the failure in the status bar. A cancelled dialog is no error
    /// and shows nothing.
    pub fn show_error(&mut self, label: &str, error: &str) {
        if error == "cancelled" {
            return;
        }
        self.last_command = Some(crate::CommandReport {
            label: label.to_string(),
            ok: false,
            text: error.chars().take(STATUS_CHARS).collect(),
        });
        let report = self.error_report();
        self.dialogs.error = Some(ErrorDialog {
            title: error_title(label),
            message: plain_message(error),
            report,
            copied: false,
        });
    }

    /// The error dialog, while it is open.
    pub fn error_dialog(&self) -> Option<&ErrorDialog> {
        self.dialogs.error.as_ref()
    }

    /// Closes the error dialog (its OK).
    pub fn close_error_dialog(&mut self) {
        self.dialogs.error = None;
    }

    /// Opens the export dialog for the active document, as File ›
    /// Export… does: what it offers is read now (the selection, the named
    /// sets, the sheet-metal parts and folds). It chooses the sheet or
    /// pattern chosen last for this document when it still has it, else
    /// the selection when a format holds it, else the model; the format
    /// chosen last when it holds that, else the first that does; and
    /// proposes the document's name with the format's extension. What
    /// `ui.set` chose while it was closed (what, the sheet's width and
    /// gap, the name) goes first, what when the document offers it. Nothing
    /// changes while it is open already. An error without a document.
    pub fn open_export(&mut self, session: &mut Session) -> Result<(), String> {
        if self.dialogs.export.is_some() {
            return Ok(());
        }
        if session.document_status().is_none() {
            return Err("there is no document to export".to_string());
        }
        self.follow_active_tab(session);
        let choices = export_choices(session);
        self.open_export_with(session, choices)
    }

    /// [`Self::open_export`] with what it offers read already
    /// ([`export_choices`] on the active document as it is): by a host
    /// that reads them where the bodies are (the page's worker), so that
    /// opening the dialog never regenerates on its own thread.
    pub fn open_export_with(
        &mut self,
        session: &mut Session,
        choices: ExportChoices,
    ) -> Result<(), String> {
        if self.dialogs.export.is_some() {
            return Ok(());
        }
        let Some(status) = session.document_status() else {
            return Err("there is no document to export".to_string());
        };
        self.follow_active_tab(session);
        let document = self.viewing();
        let preset = std::mem::take(&mut self.dialogs.preset);
        let mut committed = self.state.export.clone();
        committed.open = false;
        let remembered = self.dialogs.sheets.get(&document).cloned();
        let what = if preset.what && choices.offers(&self.state.export.what) {
            self.state.export.what.clone()
        } else {
            choices.first_choice(remembered.as_ref().map(|r| &r.what))
        };
        let formats = choices.formats(&what);
        let export = &mut self.state.export;
        if !preset.sheet {
            let (width, gap) = remembered.map(|r| (r.width, r.gap)).unwrap_or_default();
            export.width = width;
            export.gap = gap;
        }
        export.what = what;
        if !formats.contains(&export.format) {
            export.format = formats.first().copied().unwrap_or_default();
        }
        export.name = if preset.name {
            export_file_name(&export.name, export.format)
        } else {
            suggested_export_name(status.path.as_deref(), Some(export.format.extension()))
        };
        export.open = true;
        self.dialogs.export = Some(ExportAsk {
            choices,
            document,
            committed,
        });
        Ok(())
    }

    /// The export of `request` was written: what its dialog chose is
    /// offered next time (the format, the options, and for its document
    /// the sheet or pattern). Before that, a cancel or a failure changes
    /// nothing.
    pub fn export_written(&mut self, request: &ExportRequest) {
        let mut chosen = request.chosen.clone();
        chosen.open = false;
        // A dialog opened again meanwhile keeps what it shows, and falls
        // back to this when it is answered.
        match &mut self.dialogs.export {
            Some(ask) => ask.committed = chosen,
            None => self.state.export = chosen,
        }
        let ExportRequest {
            document, chosen, ..
        } = request;
        self.remember_sheet(*document, &chosen.what, &chosen.width, &chosen.gap);
    }

    /// Drops an export dialog `ui.set` opened when the rest of the request
    /// is refused: nothing has changed.
    pub(crate) fn discard_export_dialog(&mut self) {
        self.dialogs.export = None;
        self.state.export.open = false;
    }

    /// The export dialog, while it is open.
    pub fn export_dialog(&self) -> Option<&ExportAsk> {
        self.dialogs.export.as_ref()
    }

    /// Chooses what goes into the file, as a click on *What* does; the
    /// format becomes the first that holds it when the one chosen does
    /// not. An error, and nothing changes, when the dialog is not open or
    /// the document does not offer it.
    pub fn set_export_what(&mut self, what: ExportWhat) -> Result<(), String> {
        let Some(ask) = &self.dialogs.export else {
            return Err("the export dialog is not open; open it first".to_string());
        };
        if !ask.choices.offers(&what) {
            let offered: Vec<String> = ask
                .choices
                .whats()
                .iter()
                .filter(|w| ask.choices.offers(w))
                .map(|w| what_json(w).to_string())
                .collect();
            return Err(match (&what, &ask.choices.selection) {
                (ExportWhat::Selection, Some(selection)) => selection
                    .problem
                    .clone()
                    .unwrap_or_else(|| "the selection cannot be exported".to_string()),
                _ => format!(
                    "{} is not offered for this document; it offers {}",
                    what_json(&what),
                    offered.join(", ")
                ),
            });
        }
        self.state.export.what = what;
        self.settle_export_format();
        Ok(())
    }

    /// Chooses the format, as a click on *Format* does, and names the file
    /// for it. An error, and nothing changes, when the dialog is not open
    /// or the format does not hold what is chosen.
    pub fn set_export_format(&mut self, format: ExportFormat) -> Result<(), String> {
        let Some(ask) = &self.dialogs.export else {
            return Err("the export dialog is not open; open it first".to_string());
        };
        let formats = ask.choices.formats(&self.state.export.what);
        if !formats.contains(&format) {
            let names: Vec<&str> = formats.iter().map(|f| f.extension()).collect();
            return Err(format!(
                "a .{} file cannot hold {}; choose {}",
                format.extension(),
                ask.choices.label(&self.state.export.what).to_lowercase(),
                names.join(", ")
            ));
        }
        self.state.export.format = format;
        self.state.export.name = export_file_name(&self.state.export.name, format);
        Ok(())
    }

    /// The format becomes the first that holds what is chosen when the
    /// one chosen does not, and the name follows it.
    fn settle_export_format(&mut self) {
        let Some(ask) = &self.dialogs.export else {
            return;
        };
        let export = &mut self.state.export;
        let formats = ask.choices.formats(&export.what);
        if !formats.contains(&export.format)
            && let Some(first) = formats.first()
        {
            export.format = *first;
            export.name = export_file_name(&export.name, *first);
        }
    }

    /// Answers the export dialog as its buttons do: `true` exports
    /// (*Export…*), `false` cancels. When another document is shown than
    /// the one it was opened for, or the format no longer holds what was
    /// chosen, nothing is exported and the error dialog says so. Either
    /// way `UiState::export` is as before the dialog opened until the
    /// file is written ([`Self::export_written`]).
    pub fn answer_export(&mut self, export: bool) {
        let Some(ask) = self.dialogs.export.take() else {
            return;
        };
        let mut state = std::mem::replace(&mut self.state.export, ask.committed.clone());
        state.open = false;
        if !export {
            self.last_command = Some(crate::CommandReport {
                label: "file.export".to_string(),
                ok: false,
                text: "cancelled".to_string(),
            });
            return;
        }
        if ask.document != self.viewing() {
            self.show_error(
                "file.export",
                "another document is shown now than the one the export was asked for: nothing was exported; export again",
            );
            return;
        }
        if !ask.choices.formats(&state.what).contains(&state.format) {
            self.show_error(
                "file.export",
                &format!(
                    "a .{} file cannot hold {}: nothing was exported; export again and choose another format",
                    state.format.extension(),
                    ask.choices.label(&state.what).to_lowercase()
                ),
            );
            return;
        }
        let name = export_file_name(&state.name, state.format);
        state.name.clone_from(&name);
        self.dialogs.answer = Some(ExportRequest {
            name,
            format: state.format,
            plan: state.plan(),
            document: ask.document,
            chosen: state,
        });
    }

    /// The export dialog is open.
    pub fn export_dialog_open(&self) -> bool {
        self.dialogs.export.is_some()
    }

    /// What the export dialog was answered with, once.
    pub fn take_export_answer(&mut self) -> Option<ExportRequest> {
        self.dialogs.answer.take()
    }

    /// The host has no save dialog (the browser without the File System
    /// Access API): the export dialog asks the file's name itself.
    pub fn set_export_names_file(&mut self, names_file: bool) {
        self.dialogs.names_file = names_file;
    }

    /// The export dialog asks the file's name itself.
    pub fn export_names_file(&self) -> bool {
        self.dialogs.names_file
    }

    /// The sheet or pattern the export dialog chooses first for the
    /// document shown, with its width and gap; `None` when it chooses
    /// the selection or the model.
    pub fn export_sheet(&self) -> Option<(ExportWhat, String, String)> {
        self.dialogs
            .sheets
            .get(&self.viewing())
            .map(|r| (r.what.clone(), r.width.clone(), r.gap.clone()))
    }

    fn remember_sheet(&mut self, document: Option<u64>, what: &ExportWhat, width: &str, gap: &str) {
        /// Documents whose sheet is remembered; the oldest ids go first.
        const MOST: usize = 64;
        let sheets = &mut self.dialogs.sheets;
        if what.is_outline_only() {
            sheets.insert(
                document,
                Remembered {
                    what: what.clone(),
                    width: width.to_string(),
                    gap: gap.to_string(),
                },
            );
        } else {
            sheets.remove(&document);
        }
        while sheets.len() > MOST {
            sheets.pop_first();
        }
    }

    /// `ui.set`'s `export`: the dialog opened (as File › Export… does) or
    /// closed (cancelled), and what it chose. `before` is the state
    /// before the patch, which holds the new fields; the open dialog's
    /// fields are checked against what the document offers. An error,
    /// and nothing changes, for one it does not offer.
    pub(crate) fn ui_set_export(
        &mut self,
        session: &mut Session,
        before: &crate::state::ExportState,
        patch: &Value,
    ) -> Result<(), String> {
        let wanted = std::mem::replace(&mut self.state.export, before.clone());
        let given = |key: &str| patch.get(key).is_some();
        let was_open = self.dialogs.export.is_some();
        if wanted.open && !was_open {
            self.open_export(session)?;
        }
        // Checked against what the document offers only while it is open;
        // closed, they are what it offers first next time it does.
        let open = self.dialogs.export.is_some();
        let checked = if open && given("what") {
            self.set_export_what(wanted.what.clone())
        } else {
            Ok(())
        }
        .and_then(|()| {
            if open && given("format") {
                self.set_export_format(wanted.format)
            } else {
                Ok(())
            }
        });
        if let Err(e) = checked {
            if !was_open {
                self.dialogs.export = None;
            }
            self.state.export = before.clone();
            return Err(e);
        }
        // Closed by the request: cancelled first, so what it also gives is
        // for next time, as for a dialog that was closed already.
        if !wanted.open && self.dialogs.export.is_some() {
            self.answer_export(false);
        }
        let closed = self.dialogs.export.is_none();
        if closed {
            // What the next opening honours (`open_export`).
            let preset = &mut self.dialogs.preset;
            preset.what |= given("what");
            preset.sheet |= given("width") || given("gap");
            preset.name |= given("name");
        }
        let export = &mut self.state.export;
        if closed {
            if given("what") {
                export.what = wanted.what.clone();
            }
            if given("format") {
                export.format = wanted.format;
            }
        }
        if given("quality") {
            export.quality = wanted.quality;
        }
        if given("drawing") {
            export.drawing = wanted.drawing;
        }
        if given("width") {
            export.width = wanted.width;
        }
        if given("gap") {
            export.gap = wanted.gap;
        }
        if given("name") {
            export.name = wanted.name;
        }
        Ok(())
    }

    /// `ui.get`'s `export`: what the open dialog offers (each *What*
    /// with its label, whether a format holds it and which), the
    /// selection's problem, and whether it asks the name; null while it
    /// is closed (`state.export` holds what it chose).
    pub fn export_report(&self) -> Value {
        let Some(ask) = &self.dialogs.export else {
            return Value::Null;
        };
        let whats: Vec<Value> = ask
            .choices
            .whats()
            .iter()
            .map(|what| {
                let formats: Vec<&str> = ask
                    .choices
                    .formats(what)
                    .iter()
                    .map(|f| f.extension())
                    .collect();
                json!({
                    "what": what_json(what),
                    "label": ask.choices.label(what),
                    "formats": formats,
                })
            })
            .collect();
        json!({
            "whats": whats,
            "problem": ask.choices.selection.as_ref().and_then(|s| s.problem.clone()),
            "namesFile": self.dialogs.names_file,
            "file": export_file_name(&self.state.export.name, self.state.export.format),
        })
    }

    /// An error or export dialog is open: the question before closing a
    /// document waits for it (one modal at a time).
    pub(super) fn dialog_open(&self) -> bool {
        self.dialogs.error.is_some() || self.dialogs.export.is_some()
    }

    /// Draws the open dialog, the error first (one at a time): call it
    /// before anything else in the frame reads Enter or Escape.
    pub(super) fn dialogs_ui(&mut self, ctx: &egui::Context) {
        // A state that says open without a dialog behind it (one set
        // while the dialog could not be) is closed.
        if self.dialogs.export.is_none() {
            self.state.export.open = false;
        }
        if self.dialogs.error.is_some() {
            self.error_dialog_ui(ctx);
        } else {
            self.export_dialog_ui(ctx);
        }
    }

    fn error_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut error) = self.dialogs.error.take() else {
            return;
        };
        let tokens = self.tokens;
        let mut ok = false;
        let modal = egui::Modal::new(egui::Id::new("craft3d-error-dialog")).show(ctx, |ui| {
            ui.set_max_width(tokens.dialog_width);
            ui.heading(error.title.as_str());
            ui.add_space(tokens.inset * 0.4);
            ui.add(egui::Label::new(error.message.as_str()).wrap());
            ui.add_space(tokens.inset * 0.8);
            ui.horizontal(|ui| {
                ok = ui.button("OK").clicked();
                let label = if error.copied {
                    super::report::COPIED
                } else {
                    super::report::COPY_DETAILS
                };
                if ui
                    .small_button(label)
                    .on_hover_text(super::report::COPY_TOOLTIP)
                    .clicked()
                {
                    ctx.copy_text(error.report.clone());
                    error.copied = true;
                }
            });
            ok |= ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        });
        if !(ok || modal.should_close()) {
            self.dialogs.error = Some(error);
        }
    }

    fn export_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(ask) = self.dialogs.export.take() else {
            return;
        };
        let tokens = self.tokens;
        let names_file = self.dialogs.names_file;
        let mut answer: Option<bool> = None;
        let mut chose: Option<ExportWhat> = None;
        let mut format: Option<ExportFormat> = None;
        let export = &mut self.state.export;
        let modal = egui::Modal::new(egui::Id::new("craft3d-export-dialog")).show(ctx, |ui| {
            ui.set_max_width(tokens.dialog_width);
            ui.heading("Export");
            ui.add_space(tokens.inset * 0.4);
            ui.label("What:");
            ui.indent("craft3d-export-what", |ui| {
                for what in ask.choices.whats() {
                    let offered = ask.choices.offers(&what);
                    let mut response = ui
                        .add_enabled(
                            offered,
                            egui::RadioButton::new(export.what == what, ask.choices.label(&what)),
                        )
                        .on_hover_text(what_hint(&what));
                    if let (false, Some(problem)) = (
                        offered,
                        ask.choices
                            .selection
                            .as_ref()
                            .and_then(|s| s.problem.as_ref()),
                    ) {
                        response = response.on_disabled_hover_text(problem.as_str());
                    }
                    if response.clicked() {
                        chose = Some(what);
                    }
                }
            });
            if let Some(problem) = ask
                .choices
                .selection
                .as_ref()
                .and_then(|s| s.problem.as_ref())
            {
                ui.add(egui::Label::new(egui::RichText::new(problem.as_str()).weak()).wrap());
            }
            ui.add_space(tokens.inset * 0.4);
            ui.horizontal_wrapped(|ui| {
                ui.label("Format:");
                for offered in ask.choices.formats(&export.what) {
                    if ui
                        .selectable_label(export.format == offered, offered.label())
                        .clicked()
                    {
                        format = Some(offered);
                    }
                }
            });
            ui.add_space(tokens.inset * 0.4);
            options_ui(ui, export);
            ui.add_space(tokens.inset * 0.4);
            if names_file {
                ui.horizontal(|ui| {
                    ui.label("File name:");
                    ui.add(
                        egui::TextEdit::singleline(&mut export.name)
                            .char_limit(MAX_EXPORT_TEXT)
                            .desired_width(220.0),
                    );
                });
            } else {
                ui.label(
                    egui::RichText::new(format!(
                        "{}: the folder and the name are chosen next",
                        export_file_name(&export.name, export.format)
                    ))
                    .weak(),
                );
            }
            ui.add_space(tokens.inset * 0.6);
            ui.horizontal(|ui| {
                if ui.button(export_verb(names_file)).clicked() {
                    answer = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
            });
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                answer = Some(true);
            }
        });
        if answer.is_none() && modal.should_close() {
            answer = Some(false);
        }
        self.dialogs.export = Some(ask);
        if let Some(what) = chose {
            let _ = self.set_export_what(what);
        }
        if let Some(format) = format {
            let _ = self.set_export_format(format);
        }
        if let Some(answer) = answer {
            self.answer_export(answer);
        }
    }
}

/// The options of what is chosen in the format chosen: a mesh's quality,
/// a drawing's view, projection and hidden lines, a sheet's width and gap.
fn options_ui(ui: &mut egui::Ui, export: &mut crate::state::ExportState) {
    if export.format.is_mesh() {
        ui.horizontal(|ui| {
            ui.label("Quality:");
            for quality in ExportQuality::ALL {
                ui.radio_value(&mut export.quality, quality, quality.label())
                    .on_hover_text(quality.hint());
            }
        });
    }
    if export.what == ExportWhat::Model && export.format.kind() == ExportKind::Outline {
        drawing_options_ui(ui, &mut export.drawing);
    }
    if export.what.has_sheet() {
        ui.horizontal(|ui| {
            ui.label("Sheet width:");
            ui.add(
                egui::TextEdit::singleline(&mut export.width)
                    .hint_text("no limit")
                    .char_limit(MAX_EXPORT_TEXT)
                    .desired_width(90.0),
            )
            .on_hover_text("Rows wrap at this width: 600 mm, 24 in; empty for one row");
            ui.label("Gap:");
            ui.add(
                egui::TextEdit::singleline(&mut export.gap)
                    .hint_text("5 mm")
                    .char_limit(MAX_EXPORT_TEXT)
                    .desired_width(70.0),
            )
            .on_hover_text("Space between two outlines");
        });
    }
}

/// The choices of a drawing of the model as seen: from which view, to
/// scale or in perspective, and what becomes of the hidden lines.
fn drawing_options_ui(ui: &mut egui::Ui, drawing: &mut DrawingPlan) {
    ui.label("A drawing of the model as seen: its edges and outlines.");
    ui.horizontal(|ui| {
        ui.label("View:");
        egui::ComboBox::from_id_salt("craft3d-export-view")
            .selected_text(drawing.view.label())
            .show_ui(ui, |ui| {
                for view in DrawingView::ALL {
                    ui.selectable_value(&mut drawing.view, view, view.label());
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Projection:");
        ui.radio_value(&mut drawing.perspective, false, "Orthographic")
            .on_hover_text("To scale: one unit in the file is one unit of the model");
        ui.radio_value(&mut drawing.perspective, true, "Perspective")
            .on_hover_text("As the eye sees it, for an illustration: not to scale");
    });
    ui.horizontal(|ui| {
        ui.label("Hidden lines:");
        ui.radio_value(&mut drawing.dashed, false, "Leave out");
        ui.radio_value(&mut drawing.dashed, true, "Dashed")
            .on_hover_text("Lines behind a face drawn dashed, on a layer of their own");
    });
}
