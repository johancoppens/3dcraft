//! The bug report of the window: the engine's report
//! (`craft3d_engine::bug_report`) with the renderer and its graphics
//! adapter. The status bar offers it with "Copy details" when the last
//! command failed: the report goes to the clipboard and is shown in a
//! window, where it can be read before it is posted and copied by hand
//! where the clipboard is refused (a browser on a page that is not https or
//! localhost). `ui.get` gives it as `report`.
//!
//! The engine remembers the last command that failed; the status bar may
//! show another failure since (one outside the engine, such as a file
//! chooser that could not be shown), or a batch that failed in that command.
//! The shell ties the two together by the engine's failure serial: the
//! engine's failure belongs to what the status bar shows when a new one
//! came with that line, and is an earlier one otherwise.

use super::Shell;
use crate::CommandReport;
use craft3d_engine::{FailedCommand, Session};

/// The label of the status bar's button that copies the report.
pub const COPY_DETAILS: &str = "Copy details";
/// Its label once it copied the report.
pub const COPIED: &str = "Copied";
/// Its tooltip.
pub const COPY_TOOLTIP: &str = "Copy a plain-text report of this error (version, platform, graphics, the command and the values it was given) to paste into a bug report. It contains file paths and your values: read it before you post it publicly.";
/// The title of the window that shows the report.
pub const DETAILS_TITLE: &str = "Error details";

/// What the shell keeps for the report.
#[derive(Default)]
pub(super) struct ReportState {
    /// The last command that failed in the session, as of the last sync.
    failure: Option<FailedCommand>,
    /// The status bar's line as of the last sync.
    seen: Option<CommandReport>,
    /// The engine's failure serial when the status bar's line last changed.
    base: Option<u64>,
    /// Whether `failure` is the failure the status bar shows.
    current: bool,
    /// The report "Copy details" last put on the clipboard.
    copied: Option<String>,
    /// Whether the window with the report is open.
    open: bool,
}

impl Shell {
    /// What the window adds to a report: the renderer and its adapter.
    pub(super) fn host_details(&self) -> Vec<(String, String)> {
        vec![
            ("frontend".to_string(), frontend().to_string()),
            ("renderer".to_string(), self.renderer_report().describe()),
        ]
    }

    /// Keeps the failure the report names and the session's host details
    /// in step with the session (from `sync`).
    pub(super) fn sync_report(&mut self, session: &mut Session) {
        let serial = session.last_failure().map(|f| f.serial);
        if self.last_command != self.report.seen {
            self.report.current = self.failure_is_current(serial);
            self.report.base = serial;
            self.report.seen.clone_from(&self.last_command);
        }
        if serial != self.report.failure.as_ref().map(|f| f.serial) {
            self.report.failure = session.last_failure().cloned();
        }
        let details = self.host_details();
        if session.host_details() != details.as_slice() {
            session.set_host_details(details);
        }
    }

    /// Whether the engine's failure of `serial` is the one the status bar
    /// shows: it shows a failure, and the engine failed since the line
    /// before it.
    fn failure_is_current(&self, serial: Option<u64>) -> bool {
        self.last_command.as_ref().is_some_and(|r| !r.ok)
            && serial.is_some()
            && serial != self.report.base
    }

    /// The plain-text report to paste into a bug report: 3DCraft's version
    /// and commit, the platform, the renderer, the failure the status bar
    /// shows and the command that failed with its params and error.
    pub fn error_report(&self) -> String {
        let status = self
            .last_command
            .as_ref()
            .filter(|report| !report.ok)
            .map(|report| (report.label.as_str(), report.text.as_str()));
        let failure = self.report.failure.as_ref();
        // A line the last sync has not seen yet is judged as sync will.
        let current = if self.last_command == self.report.seen {
            self.report.current
        } else {
            self.failure_is_current(failure.map(|f| f.serial))
        };
        craft3d_engine::bug_report(failure, current, &self.host_details(), status)
    }

    /// The label of the status bar's button: "Copied" while the report on
    /// the clipboard is the one it would copy now. In a browser the copy
    /// may be refused without a word, so it never claims it there.
    pub(super) fn copy_label(&self) -> &'static str {
        if !cfg!(target_arch = "wasm32")
            && self.report.copied.as_deref() == Some(self.error_report().as_str())
        {
            COPIED
        } else {
            COPY_DETAILS
        }
    }

    /// Puts [`Self::error_report`] on the clipboard and opens the window
    /// that shows it.
    pub(super) fn copy_error_report(&mut self, ctx: &egui::Context) {
        let report = self.error_report();
        ctx.copy_text(report.clone());
        self.report.copied = Some(report);
        self.report.open = true;
    }

    /// Whether the window with the report is open.
    pub fn error_details_open(&self) -> bool {
        self.report.open
    }

    /// The window with the report, while it is open: the text, selectable
    /// and read-only, and a word on what is in it.
    pub(super) fn error_details(&mut self, ctx: &egui::Context) {
        if !self.report.open {
            return;
        }
        let report = self.error_report();
        let note = if cfg!(target_arch = "wasm32") {
            "This was copied to the clipboard if the browser allowed it (it needs a page on https or localhost); otherwise select the text and copy it."
        } else {
            "This was copied to the clipboard."
        };
        let tokens = self.tokens;
        let kit = crate::kit::Kit::panel(&tokens);
        let mut close = false;
        let mut copy = false;
        // Board s6's layout: the title, what it is, the report, Copy at the
        // left and Close at the right. Not modal: the window stays usable.
        let dialog = crate::kit::Dialog::new(egui::Id::new("craft3d-error-details"), DETAILS_TITLE)
            .width(tokens.details_width)
            .modal(false);
        let shown = kit.dialog(ctx, dialog, |kit, ui| {
            kit.paragraph(ui, note, false);
            kit.paragraph(
                ui,
                "It contains file paths and the values the command was given: read it before you post it publicly.",
                true,
            );
            egui::ScrollArea::vertical()
                .max_height(tokens.details_height)
                .show(ui, |ui| {
                    let mut text = report.as_str();
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                });
            kit.dialog_footer(ui, |ui| {
                copy = kit
                    .button(ui, crate::kit::Button::quiet("Copy again").icon(crate::kit::Glyph::Copy))
                    .clicked();
                kit.footer_right(ui, |ui| {
                    close = kit.button(ui, crate::kit::Button::secondary("Close")).clicked();
                });
            });
        });
        if copy {
            ctx.copy_text(report.clone());
        }
        let open = !shown.close;
        self.report.open = open && !close;
    }
}

fn frontend() -> &'static str {
    if cfg!(target_arch = "wasm32") {
        "web"
    } else {
        "desktop"
    }
}

#[cfg(test)]
mod tests {
    use crate::{CommandReport, Shell};
    use craft3d_engine::{Session, Workspace};
    use serde_json::json;

    fn failed(label: &str, text: &str) -> Option<CommandReport> {
        Some(CommandReport {
            label: label.into(),
            ok: false,
            text: text.into(),
        })
    }

    #[test]
    fn the_report_names_the_failed_command_and_the_renderer() {
        let mut shell = Shell::new();
        let mut session = Session::new(Workspace::memory());
        session.execute("file.new", &json!({})).unwrap();
        let params = json!({"size": "not a size"});
        let error = session.execute("feature.block", &params).unwrap_err();
        let short: String = error.message().chars().take(20).collect();
        shell.last_command = failed("feature.block", &short);
        shell.sync(&mut session);
        let text = shell.error_report();
        assert!(text.contains("command: feature.block\n"), "{text}");
        assert!(text.contains(&format!("params: {params}\n")), "{text}");
        assert!(
            text.contains(&format!("error: {}\n", error.message())),
            "{text}"
        );
        let renderer = format!("renderer: {}\n", shell.renderer_report().describe());
        assert!(text.contains(&renderer), "{text}");
        assert!(!text.contains("failed: "), "{text}");
        assert!(!text.contains("earlier"), "{text}");
        // The session knows the renderer too, for `session.report`.
        let report = session.execute("session.report", &json!({})).unwrap();
        assert!(report["text"].as_str().unwrap().contains(&renderer));

        // A failure the status bar shows that no command made: its own
        // lines, and the engine's failure as an earlier one, before and
        // after a sync.
        shell.last_command = failed("download", "blocked by the browser");
        for _ in 0..2 {
            let text = shell.error_report();
            assert!(
                text.contains("failed: download\nmessage: blocked by the browser\n"),
                "{text}"
            );
            assert!(
                text.contains(
                    "earlier failed command (not the failure above):\ncommand: feature.block\n"
                ),
                "{text}"
            );
            shell.sync(&mut session);
        }
    }

    #[test]
    fn a_batch_that_failed_is_reported_once_and_an_old_failure_is_earlier() {
        let mut shell = Shell::new();
        let mut session = Session::new(Workspace::memory());
        session.execute("file.new", &json!({})).unwrap();
        // A failure, then the status bar moves on to a success.
        session.execute("no.such", &json!({})).unwrap_err();
        shell.last_command = failed("no.such", "unknown command");
        shell.sync(&mut session);
        shell.last_command = Some(CommandReport {
            label: "file.new".into(),
            ok: true,
            text: "{}".into(),
        });
        shell.sync(&mut session);

        // A batch fails in its second step: the session failed again, and
        // the sync before the status bar's line came does not matter.
        session
            .execute("feature.block", &json!({"size": 1}))
            .unwrap_err();
        shell.sync(&mut session);
        shell.last_command = failed("batch of 2", "step 2 failed");
        shell.sync(&mut session);
        let text = shell.error_report();
        assert!(!text.contains("failed: "), "{text}");
        assert!(!text.contains("earlier"), "{text}");
        assert!(text.contains("command: feature.block\n"), "{text}");

        // The status bar shows a failure the engine did not have (a web
        // page's file that could not be read, labelled file.open).
        shell.last_command = failed("file.open", "the file is too large");
        shell.sync(&mut session);
        let text = shell.error_report();
        assert!(
            text.contains("failed: file.open\nmessage: the file is too large\n"),
            "{text}"
        );
        assert!(text.contains("earlier failed command"), "{text}");
    }
}
