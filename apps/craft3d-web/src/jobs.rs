//! What the regenerating worker runs besides scenes: a command that needs
//! the bodies (`craft3d_engine::run_command`, also an edit, whose undo
//! steps come back for the page to make the same), and the page's own
//! jobs that read the bodies, run there with the page's code on the
//! state they were asked for (`Session::hand_over`,
//! `craft3d_engine::run_on_state`):
//!
//! - [`EXPORT`]: the whole of an export from the menu, its choices (what
//!   is selected, the flat patterns) resolved there
//!   (`craft3d_ui_egui::files::export_where_bodies_are`); its files come
//!   back with the answer, for the save dialog or the download.
//! - [`EXPORT_CHOICES`]: what the export dialog offers
//!   (`craft3d_ui_egui::files::export_choices`), read there before the
//!   dialog opens.
//!
//! The page's thread never regenerates or meshes for either
//! (docs/design/engine.md › *Commands where the bodies are*). Natively
//! testable: the page's tests answer their fake worker with [`run`], as
//! `worker.rs`'s `craft3d_command` does in the browser.

use craft3d_engine::{CommandAnswer, CommandError, Session};
use craft3d_ui_egui::files::{ExportPlan, export_choices, export_where_bodies_are};
use serde_json::{Value, json};

/// The page's export, run where the bodies are: `{name, plan,
/// following}`.
pub const EXPORT: &str = "host.export";
/// What the export dialog offers, read where the bodies are: `{}`.
pub const EXPORT_CHOICES: &str = "host.exportChoices";

/// The answer to a job written out by `Session::command_request`: one of
/// the page's own jobs above, or a command.
pub fn run(document: &[u8], work: &str) -> CommandAnswer {
    craft3d_engine::run_on_state(document, work, run_job)
}

fn run_job(session: &mut Session, command: &str, params: &Value) -> Result<Value, CommandError> {
    match command {
        EXPORT => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| CommandError::new("file.export: the export names no file"))?;
            let plan: ExportPlan = params
                .get("plan")
                .cloned()
                .and_then(|plan| serde_json::from_value(plan).ok())
                .ok_or_else(|| {
                    CommandError::new("file.export: the export's choices cannot be read")
                })?;
            let following = params.get("following") == Some(&Value::Bool(true));
            export_where_bodies_are(session, name, &plan, following).map_err(CommandError::new)
        }
        EXPORT_CHOICES => serde_json::to_value(export_choices(session))
            .map_err(|e| CommandError::new(format!("file.export: {e}"))),
        _ => session.execute(command, params),
    }
}

/// The parameters of an [`EXPORT`] job.
pub fn export_params(name: &str, plan: &ExportPlan, following: bool) -> Value {
    json!({
        "name": name,
        "plan": serde_json::to_value(plan).unwrap_or(Value::Null),
        "following": following,
    })
}
