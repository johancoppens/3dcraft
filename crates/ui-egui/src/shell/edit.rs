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

impl Shell {
    /// What the Edit menu or a shortcut asked for since the last call.
    /// The host takes it and runs [`EditAction::command`].
    pub fn take_edit_action(&mut self) -> Option<EditAction> {
        self.edit_action.take()
    }

    /// Asks for Undo or Redo, as the menu does.
    pub fn request_edit_action(&mut self, action: EditAction) {
        self.edit_action = Some(action);
    }

    /// Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y, wherever the pointer is; not while
    /// a text field has the keyboard (it has its own undo), and only when
    /// the active document has a step to undo or redo.
    pub(super) fn edit_shortcuts(&mut self, ui: &egui::Ui, status: Option<&DocumentStatus>) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        // Redo first: Ctrl+Shift+Z would also match Ctrl+Z.
        for action in [EditAction::Redo, EditAction::Undo] {
            let pressed = action
                .shortcuts()
                .iter()
                .any(|shortcut| ui.input_mut(|i| i.consume_shortcut(shortcut)));
            if pressed && action.step(status).is_some() {
                self.edit_action = Some(action);
            }
        }
    }

    /// The Edit menu, after File: Undo and Redo with the step's name,
    /// greyed when there is nothing to undo or redo.
    pub(super) fn edit_menu(&mut self, ui: &mut egui::Ui, status: Option<&DocumentStatus>) {
        ui.menu_button("Edit", |ui| {
            for action in EditAction::ALL {
                let shortcut = action
                    .shortcuts()
                    .first()
                    .map(|shortcut| ui.ctx().format_shortcut(shortcut))
                    .unwrap_or_default();
                let button = egui::Button::new(action.menu_text(status)).shortcut_text(shortcut);
                if ui
                    .add_enabled(action.step(status).is_some(), button)
                    .clicked()
                {
                    self.edit_action = Some(action);
                    ui.close();
                }
            }
        });
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
