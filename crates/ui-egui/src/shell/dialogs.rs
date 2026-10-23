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
//! - **The export dialog** after the file was chosen, when there is
//!   something to choose: the quality of a mesh (STL, 3MF, OBJ), and for
//!   a selection the format cannot hold (faces selected, a STEP file
//!   chosen) the warning and the way out (every body instead), and for a
//!   DXF or SVG file with nothing selected (a drawing of the model as
//!   seen) the view, the projection and the hidden lines, before
//!   anything is written. The host asks it ([`Shell::ask_export`]) and
//!   takes the answer ([`Shell::take_export_answer`]); it belongs to the
//!   document shown when it was asked, and is refused when another one is
//!   shown by the time it is answered.
//!
//! Both are modal: nothing under them takes a click, and they are drawn
//! first in a frame, so that Enter and Escape answer them and nothing else
//! (they take those keys out of the frame's input).

use super::Shell;
use crate::files::{
    DrawingPlan, ExportCheck, ExportPlan, ExportQuality, export_options_ui, plain_message,
};

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

/// The export dialog while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportAsk {
    /// The file chosen, as the host will write it.
    pub path: String,
    pub check: ExportCheck,
    pub plan: ExportPlan,
    /// The document shown when it was asked (`DocumentStatus::id`).
    pub document: Option<u64>,
}

/// What the dialogs keep.
#[derive(Default)]
pub(super) struct Dialogs {
    error: Option<ErrorDialog>,
    export: Option<ExportAsk>,
    answer: Option<(String, ExportPlan)>,
    /// The quality chosen last, offered next time.
    quality: ExportQuality,
    /// How the last drawing of the model as seen was made, likewise.
    drawing: DrawingPlan,
}

/// The export button of the dialog: "Export all bodies" when the
/// selection does not fit the format (the way out), else "Export".
pub fn export_verb(check: &ExportCheck) -> &'static str {
    if check.offers_all_bodies() {
        "Export all bodies"
    } else {
        "Export"
    }
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

    /// Opens the export dialog for an export to `path` that `check`
    /// describes, with the quality chosen last, for the document shown
    /// now. False, and nothing changes, while an export dialog is open
    /// already: a second Export… does not replace the first's question.
    pub fn ask_export(&mut self, path: String, check: ExportCheck) -> bool {
        if self.dialogs.export.is_some() {
            return false;
        }
        self.dialogs.export = Some(ExportAsk {
            path,
            check,
            plan: ExportPlan {
                quality: self.dialogs.quality,
                all_bodies: false,
                drawing: self.dialogs.drawing,
            },
            document: self.viewing(),
        });
        true
    }

    /// The export dialog, while it is open.
    pub fn export_dialog(&self) -> Option<&ExportAsk> {
        self.dialogs.export.as_ref()
    }

    /// Answers the export dialog as its buttons do: `Some(plan)` exports
    /// (Export, or Export all bodies), `None` cancels. When another
    /// document is shown than the one it was asked for, nothing is
    /// exported and the error dialog says so.
    pub fn answer_export(&mut self, plan: Option<ExportPlan>) {
        let Some(ask) = self.dialogs.export.take() else {
            return;
        };
        match plan {
            Some(_) if ask.document != self.viewing() => self.show_error(
                "file.export",
                "another document is shown now than the one the export was asked for: nothing was exported; export again",
            ),
            Some(plan) => {
                self.dialogs.quality = plan.quality;
                self.dialogs.drawing = plan.drawing;
                self.dialogs.answer = Some((ask.path, plan));
            }
            None => {
                self.last_command = Some(crate::CommandReport {
                    label: "file.export".to_string(),
                    ok: false,
                    text: "cancelled".to_string(),
                });
            }
        }
    }

    /// What the export dialog was answered with: the file and how to
    /// export it, once.
    pub fn take_export_answer(&mut self) -> Option<(String, ExportPlan)> {
        self.dialogs.answer.take()
    }

    /// The quality the export dialog offers next.
    pub fn export_quality(&self) -> ExportQuality {
        self.dialogs.quality
    }

    /// Remembers the quality chosen in a host's own export dialog (the
    /// browser's), to offer it next time.
    pub fn set_export_quality(&mut self, quality: ExportQuality) {
        self.dialogs.quality = quality;
    }

    /// How the export dialog offers to make a drawing of the model as
    /// seen next: as the last one.
    pub fn export_drawing(&self) -> DrawingPlan {
        self.dialogs.drawing
    }

    /// Remembers the drawing chosen in a host's own export dialog.
    pub fn set_export_drawing(&mut self, drawing: DrawingPlan) {
        self.dialogs.drawing = drawing;
    }

    /// An error or export dialog is open: the question before closing a
    /// document waits for it (one modal at a time).
    pub(super) fn dialog_open(&self) -> bool {
        self.dialogs.error.is_some() || self.dialogs.export.is_some()
    }

    /// Draws the open dialog, the error first (one at a time): call it
    /// before anything else in the frame reads Enter or Escape.
    pub(super) fn dialogs_ui(&mut self, ctx: &egui::Context) {
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
        let Some(mut ask) = self.dialogs.export.take() else {
            return;
        };
        let tokens = self.tokens;
        let mut answer: Option<Option<ExportPlan>> = None;
        let name = std::path::Path::new(&ask.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(ask.path.as_str())
            .to_string();
        let modal = egui::Modal::new(egui::Id::new("craft3d-export-dialog")).show(ctx, |ui| {
            ui.set_max_width(tokens.dialog_width);
            ui.heading("Export");
            ui.add_space(tokens.inset * 0.4);
            ui.label(format!("Export to {name}"));
            export_options_ui(ui, &ask.check, &mut ask.plan);
            ui.add_space(tokens.inset * 0.6);
            ui.horizontal(|ui| {
                if ui.button(export_verb(&ask.check)).clicked() {
                    ask.plan.all_bodies = ask.check.offers_all_bodies();
                    answer = Some(Some(ask.plan));
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(None);
                }
            });
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                ask.plan.all_bodies = ask.check.offers_all_bodies();
                answer = Some(Some(ask.plan));
            }
        });
        if answer.is_none() && modal.should_close() {
            answer = Some(None);
        }
        self.dialogs.export = Some(ask);
        if let Some(answer) = answer {
            self.answer_export(answer);
        }
    }
}
