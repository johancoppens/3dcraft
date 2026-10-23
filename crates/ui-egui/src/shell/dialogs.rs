//! The window's own small dialogs for what the person started (2026-10-23,
//! the developer: a failed Save or Export only showed in the status bar):
//!
//! - **An error dialog** when an action the person started from a menu, a
//!   tab or a dialog fails (Open, Save, Save As, Import, Export, New, an
//!   example): what went wrong in plain words, OK, and "Copy details",
//!   the bug report the status bar offers too (`report.rs`). Failures of
//!   an agent's commands stay in the status bar: an agent reads them from
//!   its reply, and a dialog would stop the person for nothing.
//! - **The export dialog** after the file was chosen, when there is
//!   something to choose: the quality of a mesh (STL, 3MF, OBJ), and for
//!   a selection the format cannot hold (faces selected, a STEP file
//!   chosen) the warning and the way out (every body instead), before
//!   anything is written. The host asks it ([`Shell::ask_export`]) and
//!   takes the answer ([`Shell::take_export_answer`]).

use super::Shell;
use crate::files::{ExportCheck, ExportPlan, ExportQuality, export_options_ui, plain_message};

/// An error shown in a dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorDialog {
    /// What failed, as a title: "Could not save".
    pub title: String,
    /// Why, for a person.
    pub message: String,
}

/// The export dialog while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportAsk {
    /// The file chosen, as the host will write it.
    pub path: String,
    pub check: ExportCheck,
    pub plan: ExportPlan,
}

/// What the dialogs keep.
#[derive(Default)]
pub(super) struct Dialogs {
    error: Option<ErrorDialog>,
    export: Option<ExportAsk>,
    answer: Option<(String, ExportPlan)>,
    /// The quality chosen last, offered next time.
    quality: ExportQuality,
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
        _ => "Something went wrong",
    }
    .to_string()
}

impl Shell {
    /// Shows the error dialog for an action the person started that
    /// failed, labelled as the status bar labels it (`file.save`). A
    /// cancelled dialog is no error and shows nothing.
    pub fn show_error(&mut self, label: &str, error: &str) {
        if error == "cancelled" {
            return;
        }
        self.dialogs.error = Some(ErrorDialog {
            title: error_title(label),
            message: plain_message(error),
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
    /// describes, with the quality chosen last.
    pub fn ask_export(&mut self, path: String, check: ExportCheck) {
        self.dialogs.export = Some(ExportAsk {
            path,
            check,
            plan: ExportPlan {
                quality: self.dialogs.quality,
                all_bodies: false,
            },
        });
    }

    /// The export dialog, while it is open.
    pub fn export_dialog(&self) -> Option<&ExportAsk> {
        self.dialogs.export.as_ref()
    }

    /// Answers the export dialog as its buttons do: `Some(plan)` exports
    /// (Export, or Export all bodies), `None` cancels.
    pub fn answer_export(&mut self, plan: Option<ExportPlan>) {
        let Some(ask) = self.dialogs.export.take() else {
            return;
        };
        if let Some(plan) = plan {
            self.dialogs.quality = plan.quality;
            self.dialogs.answer = Some((ask.path, plan));
        } else {
            self.last_command = Some(crate::CommandReport {
                label: "file.export".to_string(),
                ok: false,
                text: "cancelled".to_string(),
            });
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

    /// Draws the open dialogs.
    pub(super) fn dialogs_ui(&mut self, ctx: &egui::Context) {
        self.export_dialog_ui(ctx);
        self.error_dialog_ui(ctx);
    }

    fn error_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(error) = self.dialogs.error.clone() else {
            return;
        };
        let mut ok = false;
        let mut copy = false;
        egui::Window::new(error.title.as_str())
            .id(egui::Id::new("craft3d-error-dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.add(egui::Label::new(error.message.as_str()).wrap());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ok = ui.button("OK").clicked();
                    copy = ui
                        .small_button(super::report::COPY_DETAILS)
                        .on_hover_text(super::report::COPY_TOOLTIP)
                        .clicked();
                });
                if ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape))
                {
                    ok = true;
                }
            });
        if copy {
            self.copy_error_report(ctx);
        }
        if ok {
            self.dialogs.error = None;
        }
    }

    fn export_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut ask) = self.dialogs.export.take() else {
            return;
        };
        let mut answer: Option<Option<ExportPlan>> = None;
        let name = std::path::Path::new(&ask.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(ask.path.as_str())
            .to_string();
        egui::Window::new("Export")
            .id(egui::Id::new("craft3d-export-dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.label(format!("Export to {name}"));
                export_options_ui(ui, &ask.check, &mut ask.plan);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(export_verb(&ask.check)).clicked() {
                        ask.plan.all_bodies = ask.check.offers_all_bodies();
                        answer = Some(Some(ask.plan));
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(None);
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    answer = Some(None);
                }
            });
        self.dialogs.export = Some(ask);
        if let Some(answer) = answer {
            self.answer_export(answer);
        }
    }
}
