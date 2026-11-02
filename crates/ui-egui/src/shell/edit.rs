//! Undo and Redo in the window: the Edit menu and Ctrl+Z, Ctrl+Shift+Z
//! and Ctrl+Y for the active document.
//!
//! Like the File menu, the shell only asks ([`EditAction`]): the host runs
//! `edit.undo` or `edit.redo` where it runs commands, and the window
//! follows the session at the next sync. A Jig in use has only the steps
//! of its parameters to undo (the engine refuses every other edit there).

use super::Shell;
use craft3d_engine::DocumentStatus;

/// Undo or Redo, asked by the Edit menu or a shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAction {
    Undo,
    Redo,
}

/// The most characters of a step's name the menu shows.
const MAX_STEP_NAME: usize = 40;

impl EditAction {
    /// In menu order.
    pub const ALL: [EditAction; 2] = [EditAction::Undo, EditAction::Redo];

    /// The command the host runs for it.
    pub fn command(self) -> &'static str {
        match self {
            EditAction::Undo => "edit.undo",
            EditAction::Redo => "edit.redo",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EditAction::Undo => "Undo",
            EditAction::Redo => "Redo",
        }
    }

    /// The name of the step it would undo or redo in the document of
    /// `status`; `None` when there is none.
    pub fn step(self, status: Option<&DocumentStatus>) -> Option<&str> {
        let status = status?;
        match self {
            EditAction::Undo => status.undo.as_deref(),
            EditAction::Redo => status.redo.as_deref(),
        }
    }

    /// The menu item: `Undo Set teeth`, or just `Undo` when there is
    /// nothing to undo.
    pub fn menu_text(self, status: Option<&DocumentStatus>) -> String {
        match self.step(status).map(str::trim).filter(|s| !s.is_empty()) {
            Some(step) => {
                let mut name: String = step
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_STEP_NAME)
                    .collect();
                if step.chars().count() > MAX_STEP_NAME {
                    name.push('…');
                }
                format!("{} {name}", self.label())
            }
            None => self.label().to_string(),
        }
    }

    /// The shortcuts that ask for it, the one the menu shows first: Ctrl+Z
    /// (⌘Z on a Mac) for Undo, Ctrl+Shift+Z and Ctrl+Y for Redo.
    pub fn shortcuts(self) -> &'static [egui::KeyboardShortcut] {
        const UNDO: [egui::KeyboardShortcut; 1] = [egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND,
            egui::Key::Z,
        )];
        const REDO: [egui::KeyboardShortcut; 2] = [
            egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
                egui::Key::Z,
            ),
            egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y),
        ];
        match self {
            EditAction::Undo => &UNDO,
            EditAction::Redo => &REDO,
        }
    }
}

/// Undo or Redo for one document: what the Edit menu or a key asked
/// for, with the document that was active then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditRequest {
    pub action: EditAction,
    /// The document's id (`DocumentStatus::id`).
    pub document: u64,
}

/// The most edit requests that wait for the host; more are dropped (a key
/// held down while a heavy undo runs).
pub const MAX_EDIT_QUEUE: usize = 16;

/// Runs `request` in `session`, as a host does with what
/// [`Shell::take_edit_action`] gives: `edit.undo` or `edit.redo` of the
/// active document, refused when another document is active now than when
/// it was asked for (a tab switched meanwhile). Whether there is a step to
/// undo or redo is the engine's to say, now, not the window's of when the
/// key was pressed.
pub fn run_edit(
    session: &mut craft3d_engine::Session,
    request: EditRequest,
) -> Result<serde_json::Value, String> {
    let command = request.action.command();
    match session.document_status() {
        Some(status) if status.id == request.document => session
            .execute(command, &serde_json::json!({}))
            .map_err(|e| e.message().to_string()),
        Some(_) => Err(format!(
            "{command}: not done: another document is active now than when it was asked for"
        )),
        None => Err(format!("{command}: not done: the document was closed")),
    }
}

impl Shell {
    /// The next Undo or Redo the Edit menu or a key asked for, oldest
    /// first. The host runs it with [`run_edit`], without the Parameters
    /// form's verdict (that is for the form's own commits).
    pub fn take_edit_action(&mut self) -> Option<EditRequest> {
        self.edit_actions.pop_front()
    }

    /// Asks for Undo or Redo of `document`, as the menu does.
    pub fn request_edit_action(&mut self, action: EditAction, document: u64) {
        if self.edit_actions.len() < MAX_EDIT_QUEUE {
            self.edit_actions
                .push_back(EditRequest { action, document });
        }
    }

    /// Undo and Redo wait: not under a modal dialog or the question
    /// before closing, and not while the Parameters form drags a value or
    /// waits for its commit (that commit is a step of its own).
    fn edit_blocked(&self) -> bool {
        self.dialog_open() || self.closing.is_some() || self.params_editing()
    }

    /// Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y, wherever the pointer is; not while
    /// a text field has the keyboard (it has its own undo) or while
    /// [`Self::edit_blocked`]. Asked for whenever there is a document:
    /// whether there is a step left is judged when it runs, so presses
    /// made while an earlier one runs are not lost to a stale status.
    pub(super) fn edit_shortcuts(&mut self, ui: &egui::Ui, status: Option<&DocumentStatus>) {
        if ui.ctx().egui_wants_keyboard_input() || self.edit_blocked() {
            return;
        }
        // Redo first: Ctrl+Shift+Z would also match Ctrl+Z.
        for action in [EditAction::Redo, EditAction::Undo] {
            let pressed = action
                .shortcuts()
                .iter()
                .any(|shortcut| ui.input_mut(|i| i.consume_shortcut(shortcut)));
            if let (true, Some(status)) = (pressed, status) {
                self.request_edit_action(action, status.id);
            }
        }
    }

    /// The Edit menu, after File: Undo and Redo with the step's name,
    /// greyed when there is nothing to undo or redo, and while
    /// [`Self::edit_blocked`].
    pub(super) fn edit_menu(
        &mut self,
        ui: &mut egui::Ui,
        status: Option<&DocumentStatus>,
    ) -> egui::Rect {
        let blocked = self.edit_blocked();
        let tokens = self.tokens;
        super::menus::kit_menu(ui, &tokens, "Edit", tokens.menu_widths[1], |kit, ui| {
            for action in EditAction::ALL {
                let shortcut = action
                    .shortcuts()
                    .first()
                    .map(|shortcut| ui.ctx().format_shortcut(shortcut))
                    .unwrap_or_default();
                let text = action.menu_text(status);
                let enabled = !blocked && action.step(status).is_some();
                let item = crate::kit::MenuItem::new(&text)
                    .shortcut(&shortcut)
                    .enabled(enabled);
                if kit.menu_item(ui, item).clicked() {
                    if let Some(status) = status {
                        self.request_edit_action(action, status.id);
                    }
                    ui.close();
                }
            }
        })
        .0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(undo: Option<&str>, redo: Option<&str>) -> DocumentStatus {
        DocumentStatus {
            index: 0,
            id: 1,
            open: 1,
            path: None,
            file: None,
            dirty: false,
            close_asks: false,
            jig_in_use: false,
            undo: undo.map(str::to_string),
            redo: redo.map(str::to_string),
            length_unit: "mm",
            angle_unit: "deg",
        }
    }

    #[test]
    fn the_menu_names_the_step() {
        let s = status(Some("Set teeth"), None);
        assert_eq!(EditAction::Undo.menu_text(Some(&s)), "Undo Set teeth");
        assert_eq!(EditAction::Redo.menu_text(Some(&s)), "Redo");
        assert_eq!(EditAction::Undo.menu_text(None), "Undo");
        let long = "x".repeat(200);
        let s = status(None, Some(&long));
        let text = EditAction::Redo.menu_text(Some(&s));
        assert!(text.ends_with('…') && text.chars().count() < 50, "{text}");
        let s = status(Some("  "), None);
        assert_eq!(EditAction::Undo.menu_text(Some(&s)), "Undo");
    }

    #[test]
    fn the_shortcuts_are_ctrl_z_and_ctrl_shift_z_or_ctrl_y() {
        let names = |action: EditAction| -> Vec<String> {
            action
                .shortcuts()
                .iter()
                .map(|s| s.format(&egui::ModifierNames::NAMES, false))
                .collect()
        };
        assert_eq!(names(EditAction::Undo), ["Ctrl+Z"]);
        assert_eq!(names(EditAction::Redo), ["Ctrl+Shift+Z", "Ctrl+Y"]);
        assert_eq!(EditAction::Undo.command(), "edit.undo");
        assert_eq!(EditAction::Redo.command(), "edit.redo");
    }
}
