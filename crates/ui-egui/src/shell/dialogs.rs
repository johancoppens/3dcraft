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
    DrawingView, ExportChoices, ExportFormat, ExportKind, ExportPlan, ExportQuality, ExportWhat,
    MAX_EXPORT_TEXT, export_choices, export_file_name, plain_message, suggested_export_name,
    what_hint,
};
use crate::kit::{Button, Dialog, Glyph, Kit, Segment};
use crate::theme::color;
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
    /// The bodies hidden in the view (`feature.visible`) when it was
    /// opened, by their features' names: what an export of the model
    /// leaves out (`hiddenLeftOut`), said under the choice.
    pub hidden: Vec<String>,
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
    /// Where the export dialog's choices of *What* were drawn in the last
    /// frame, each with what it chooses.
    what_hits: Vec<(ExportWhat, egui::Rect)>,
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
        super::EXAMPLE_LABEL => "Could not build the example",
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
            suggested_export_name(
                status.path.as_deref(),
                status.title.as_deref(),
                Some(export.format.extension()),
            )
        };
        export.open = true;
        let hidden = self.hidden_bodies();
        self.dialogs.export = Some(ExportAsk {
            choices,
            hidden,
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

    /// Closes the export dialog when the document it was opened for is
    /// not open any more (`open`: the ids of those that are), as a cancel
    /// does but without a word in the status bar (what closed the
    /// document says that): it would otherwise stay over the next
    /// documents, offering the closed one's name (seen 2026-11-10).
    pub(super) fn export_follows_documents(&mut self, open: &[u64]) {
        let Some(ask) = &self.dialogs.export else {
            return;
        };
        if ask.document.is_some_and(|id| open.contains(&id)) {
            return;
        }
        if let Some(ask) = self.dialogs.export.take() {
            self.state.export = ask.committed;
            self.state.export.open = false;
        }
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
            // Cancelled is the person's choice, not a failure: said, not
            // in red and without Copy details.
            self.last_command = Some(crate::CommandReport {
                label: "file.export".to_string(),
                ok: true,
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
            "hidden": ask.hidden,
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

    /// The error dialog (board s6): the failure's red warning before its
    /// title, why, then Copy details at the left and OK, amber, at the
    /// right. Enter is OK; Escape or a click beside it closes it too.
    fn error_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut error) = self.dialogs.error.take() else {
            return;
        };
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let mut ok = false;
        let mut copy = false;
        let dialog = Dialog::new(egui::Id::new("craft3d-error-dialog"), &error.title)
            .icon(Glyph::Warning, tokens.failed)
            .width(tokens.dialog_wide_width);
        let shown = kit.dialog(ctx, dialog, |kit, ui| {
            kit.paragraph(ui, &error.message, false);
            kit.dialog_footer(ui, |ui| {
                let label = if error.copied {
                    super::report::COPIED
                } else {
                    super::report::COPY_DETAILS
                };
                copy = kit
                    .button(ui, Button::quiet(label).icon(Glyph::Copy))
                    .on_hover_text(super::report::COPY_TOOLTIP)
                    .clicked();
                kit.footer_right(ui, |ui| {
                    ok = kit.button(ui, Button::primary("OK")).clicked();
                });
            });
            ok |= ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        });
        if copy {
            ctx.copy_text(error.report.clone());
            error.copied = true;
        }
        if !(ok || shown.close) {
            self.dialogs.error = Some(error);
        }
    }

    /// The export dialog (board s5): a label and its control on each row
    /// (What, Format, File name, then the format's options), Cancel and
    /// Export, amber, at the right. Enter exports, Escape cancels.
    fn export_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(ask) = self.dialogs.export.take() else {
            return;
        };
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let names_file = self.dialogs.names_file;
        let mut answer: Option<bool> = None;
        let mut chose: Option<ExportWhat> = None;
        let mut format: Option<ExportFormat> = None;
        let mut what_hits = Vec::new();
        let export = &mut self.state.export;
        let label_width = 100.0;
        let dialog = Dialog::new(egui::Id::new("craft3d-export-dialog"), "Export")
            .width(tokens.export_dialog_width);
        let shown = kit.dialog(ctx, dialog, |kit, ui| {
            // What: the choices this document offers, as a segmented
            // control while they fit, else one under the other.
            let whats: Vec<ExportWhat> = ask
                .choices
                .whats()
                .into_iter()
                .filter(|what| ask.choices.offers(what))
                .collect();
            let labels: Vec<String> = whats
                .iter()
                .map(|what| short_what(&ask.choices.label(what)))
                .collect();
            let room = ui.available_width() - label_width;
            let wide: f32 = labels
                .iter()
                .map(|l| {
                    kit.galley(ui, l.as_str(), kit.font(), tokens.text).size().x
                        + 2.0 * tokens.field_inset
                })
                .sum();
            row(ui, kit, "What", label_width, |ui| {
                let mut chosen = whats
                    .iter()
                    .position(|w| *w == export.what)
                    .unwrap_or(usize::MAX);
                if wide + 8.0 <= room {
                    let segments: Vec<Segment<'_>> =
                        labels.iter().map(|l| Segment::Text(l.as_str())).collect();
                    let before = chosen;
                    let choice = kit.segmented(ui, &mut chosen, &segments);
                    for (what, rect) in whats.iter().zip(&choice.items) {
                        what_hits.push((what.clone(), *rect));
                    }
                    if chosen != before {
                        chose = whats.get(chosen).cloned();
                    }
                } else {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (index, (what, label)) in whats.iter().zip(&labels).enumerate() {
                            let mut on = index == chosen;
                            let response = kit
                                .checkbox(ui, &mut on, label)
                                .on_hover_text(what_hint(what));
                            what_hits.push((what.clone(), response.rect));
                            if response.clicked() {
                                chose = Some(what.clone());
                            }
                        }
                    });
                }
            });
            if let Some(problem) = ask
                .choices
                .selection
                .as_ref()
                .and_then(|s| s.problem.as_ref())
            {
                hint(ui, kit, label_width, problem);
            }
            // The model leaves the hidden bodies out, as the view does.
            if export.what == ExportWhat::Model
                && let Some(left) = hidden_hint(&ask.hidden)
            {
                hint(ui, kit, label_width, &left);
            }
            // Format: those that hold what is chosen.
            let formats = ask.choices.formats(&export.what);
            row(ui, kit, "Format", label_width, |ui| {
                let labels: Vec<&str> = formats.iter().map(|f| f.label()).collect();
                let mut chosen = formats
                    .iter()
                    .position(|f| *f == export.format)
                    .unwrap_or(0);
                let before = chosen;
                kit.dropdown(ui, "craft3d-export-format", &mut chosen, &labels, 200.0);
                if chosen != before {
                    format = formats.get(chosen).copied();
                }
            });
            // Not in a wrapping row: a sentence that wraps there puts its
            // next line a whole control's height lower (with touch sizes,
            // 44 points).
            kit.dialog_row(ui, "File name", label_width, |ui| {
                if names_file {
                    ui.horizontal(|ui| {
                        kit.text_field(
                            ui,
                            "craft3d-export-name",
                            &mut export.name,
                            "",
                            None,
                            280.0,
                        );
                    });
                } else {
                    // Its first line centred on the label.
                    let row = kit.t.label_row_height.max(kit.t.control_height);
                    let line = ui.fonts_mut(|f| f.row_height(&kit.font()));
                    ui.add_space(((row - line) * 0.5).max(0.0));
                    kit.paragraph(
                        ui,
                        &format!(
                            "{}: the folder and the name are chosen next",
                            export_file_name(&export.name, export.format)
                        ),
                        true,
                    );
                }
            });
            options_ui(ui, kit, label_width, export);
            // What is typed is held to its limit, as the fields were.
            for text in [&mut export.name, &mut export.width, &mut export.gap] {
                if text.chars().count() > MAX_EXPORT_TEXT {
                    *text = text.chars().take(MAX_EXPORT_TEXT).collect();
                }
            }
            kit.dialog_footer(ui, |ui| {
                kit.footer_right(ui, |ui| {
                    if kit
                        .button(ui, Button::primary(export_verb(names_file)))
                        .clicked()
                    {
                        answer = Some(true);
                    }
                    if kit.button(ui, Button::secondary("Cancel")).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                answer = Some(true);
            }
        });
        if answer.is_none() && shown.close {
            answer = Some(false);
        }
        self.dialogs.what_hits = what_hits;
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

    /// Where the export dialog's choices of *What* were drawn in the last
    /// frame, each with what it chooses (empty while it is closed).
    pub fn export_what_hits(&self) -> &[(ExportWhat, egui::Rect)] {
        &self.dialogs.what_hits
    }
}

/// What the dialog says under the model of the hidden bodies it leaves
/// out, when there are any.
pub fn hidden_hint(hidden: &[String]) -> Option<String> {
    let names = match hidden {
        [] => return None,
        [one] => format!("1 hidden body ({one}) is"),
        many if many.len() <= 3 => {
            format!("{} hidden bodies ({}) are", many.len(), many.join(", "))
        }
        many => format!("{} hidden bodies are", many.len()),
    };
    Some(format!(
        "{names} left out, as the view leaves them out; show them, or select the bodies to export, to write them"
    ))
}

/// What a choice of *What* says in the dialog: the model's "All bodies",
/// the selection without its summary (in its tooltip), the rest as they
/// are.
fn short_what(label: &str) -> String {
    match label {
        "The model" => "All bodies".to_string(),
        l if l.starts_with("The selection") => "Selection".to_string(),
        l => l.to_string(),
    }
}

/// A row of the dialog: the label dim at the left, the control after it.
fn row(ui: &mut egui::Ui, kit: &Kit, label: &str, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    kit.dialog_row(ui, label, width, |ui| {
        ui.horizontal_wrapped(|ui| add(ui));
    });
}

/// A hint under the controls, in the dim ink and the small font.
fn hint(ui: &mut egui::Ui, kit: &Kit, indent: f32, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(indent + ui.spacing().item_spacing.x);
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .font(kit.small_font())
                    .color(color(kit.skin.text_dim)),
            )
            .wrap(),
        );
    });
}

/// The options of what is chosen in the format chosen: a mesh's quality,
/// a drawing's view, projection and hidden lines, a sheet's width and gap.
fn options_ui(ui: &mut egui::Ui, kit: &Kit, label: f32, export: &mut crate::state::ExportState) {
    if export.format.is_mesh() {
        row(ui, kit, "Quality", label, |ui| {
            let segments: Vec<Segment<'_>> = ExportQuality::ALL
                .iter()
                .map(|q| Segment::Text(q.label()))
                .collect();
            let mut chosen = ExportQuality::ALL
                .iter()
                .position(|q| *q == export.quality)
                .unwrap_or(1);
            kit.segmented(ui, &mut chosen, &segments);
            if let Some(quality) = ExportQuality::ALL.get(chosen) {
                export.quality = *quality;
            }
        });
        hint(ui, kit, label, export.quality.hint());
    }
    if export.what == ExportWhat::Model && export.format.kind() == ExportKind::Outline {
        let drawing = &mut export.drawing;
        row(ui, kit, "View", label, |ui| {
            let labels: Vec<&str> = DrawingView::ALL.iter().map(|v| v.label()).collect();
            let mut chosen = DrawingView::ALL
                .iter()
                .position(|v| *v == drawing.view)
                .unwrap_or(0);
            kit.dropdown(ui, "craft3d-export-view", &mut chosen, &labels, 200.0);
            if let Some(view) = DrawingView::ALL.get(chosen) {
                drawing.view = *view;
            }
        });
        row(ui, kit, "Projection", label, |ui| {
            let mut chosen = usize::from(drawing.perspective);
            kit.segmented(
                ui,
                &mut chosen,
                &[Segment::Text("Orthographic"), Segment::Text("Perspective")],
            );
            drawing.perspective = chosen == 1;
        });
        row(ui, kit, "Hidden lines", label, |ui| {
            let mut chosen = usize::from(!drawing.dashed);
            kit.segmented(
                ui,
                &mut chosen,
                &[Segment::Text("Dashed"), Segment::Text("Leave out")],
            );
            drawing.dashed = chosen == 0;
        });
        hint(
            ui,
            kit,
            label,
            if drawing.perspective {
                "As the eye sees it, for an illustration: not to scale"
            } else {
                "To scale: one unit in the file is one unit of the model"
            },
        );
    }
    if export.what.has_sheet() {
        row(ui, kit, "Sheet width", label, |ui| {
            kit.text_field(
                ui,
                "craft3d-export-width",
                &mut export.width,
                "no limit",
                None,
                140.0,
            )
            .on_hover_text("Rows wrap at this width: 600 mm, 24 in; empty for one row");
        });
        row(ui, kit, "Gap", label, |ui| {
            kit.text_field(
                ui,
                "craft3d-export-gap",
                &mut export.gap,
                "5 mm",
                None,
                140.0,
            )
            .on_hover_text("Space between two outlines");
        });
    }
}
