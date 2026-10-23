//! The bug report of the window: the engine's report
//! (`craft3d_engine::bug_report`) with the renderer and its graphics
//! adapter, and the status bar's own line when the failure it shows did
//! not come from a command. The status bar offers it with "Copy details"
//! when the last command failed; `ui.get` gives it as `report`.

use super::Shell;
use craft3d_engine::Session;

/// The label of the status bar's button that copies the report.
pub const COPY_DETAILS: &str = "Copy details";
/// Its label once it copied the report.
pub const COPIED: &str = "Copied";
/// Its tooltip.
pub const COPY_TOOLTIP: &str = "Copy a plain-text report of this error (version, platform, renderer, the command and its params) to paste into a bug report";

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
        if serial != self.failure.as_ref().map(|f| f.serial) {
            self.failure = session.last_failure().cloned();
        }
        let details = self.host_details();
        if session.host_details() != details.as_slice() {
            session.set_host_details(details);
        }
    }

    /// The plain-text report to paste into a bug report: 3DCraft's version
    /// and commit, the platform, the renderer and the last command that
    /// failed with its params and error.
    pub fn error_report(&self) -> String {
        let status = self
            .last_command
            .as_ref()
            .filter(|report| !report.ok)
            .map(|report| (report.label.as_str(), report.text.as_str()));
        craft3d_engine::bug_report(self.failure.as_ref(), &self.host_details(), status)
    }

    /// The label of the status bar's button: "Copied" while the report on
    /// the clipboard is the one it would copy now.
    pub(super) fn copy_label(&self) -> &'static str {
        if self.copied.as_deref() == Some(self.error_report().as_str()) {
            COPIED
        } else {
            COPY_DETAILS
        }
    }

    /// Puts [`Self::error_report`] on the clipboard.
    pub(super) fn copy_error_report(&mut self, ctx: &egui::Context) {
        let report = self.error_report();
        ctx.copy_text(report.clone());
        self.copied = Some(report);
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

    #[test]
    fn the_report_names_the_failed_command_and_the_renderer() {
        let mut shell = Shell::new();
        let mut session = Session::new(Workspace::memory());
        session.execute("file.new", &json!({})).unwrap();
        let params = json!({"size": "not a size"});
        let error = session.execute("feature.block", &params).unwrap_err();
        shell.last_command = Some(CommandReport {
            label: "feature.block".into(),
            ok: false,
            text: error.message().chars().take(20).collect(),
        });
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
        // The session knows the renderer too, for `session.report`.
        let report = session.execute("session.report", &json!({})).unwrap();
        assert!(report["text"].as_str().unwrap().contains(&renderer));

        // A failure the status bar shows that no command made.
        shell.last_command = Some(CommandReport {
            label: "download".into(),
            ok: false,
            text: "blocked by the browser".into(),
        });
        let text = shell.error_report();
        assert!(
            text.contains("failed: download\nmessage: blocked by the browser\n"),
            "{text}"
        );
        assert!(text.contains("command: feature.block\n"), "{text}");
    }
}
