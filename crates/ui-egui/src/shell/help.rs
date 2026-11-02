//! The Help menu: About 3DCraft, the user guide, the keyboard shortcuts
//! (also F1) and, on the desktop, the folder 3DCraft keeps its data in.
//!
//! The shortcuts window is made from one table, [`shortcuts`]. Its keys
//! are the values the key handling itself uses (the File menu's
//! shortcuts, the tabs', [`FIT_KEY`] and [`CLEAR_KEY`] without modifiers,
//! as the viewport and the list take them, and [`help_shortcut`]), so a
//! key changed there changes here. What a key does is written in words,
//! as are the mouse rows: those are kept true by hand.
//!
//! The shell does not open a browser or a file manager itself: it asks
//! the host ([`Shell::take_external`]), which knows the platform. In the
//! browser the page opens a link itself (egui's `open_url`, which the web
//! runner turns into `window.open`).

use super::{
    EditAction, FileAction, Shell, close_shortcut, next_shortcut, player, previous_shortcut,
};
use crate::theme::color;
use std::path::PathBuf;

/// The user guide, on GitHub.
pub const GUIDE_URL: &str = "https://github.com/johancoppens/3dcraft/tree/main/docs/guide";
/// How to connect an agent over MCP, in the guide.
pub const AGENT_SETUP_URL: &str = "https://github.com/johancoppens/3dcraft/blob/main/docs/guide/01-install.md#connect-an-ai-agent-over-mcp";
/// The source code.
pub const REPO_URL: &str = "https://github.com/johancoppens/3dcraft";
/// The licence 3DCraft is published under.
pub const LICENCE: &str = "MIT";
/// The version of this build (the engine's, which bug reports name).
pub const VERSION: &str = craft3d_engine::VERSION;

/// F: fly to the selection, or frame everything when nothing is
/// selected; over the viewport and over the feature list.
pub(crate) const FIT_KEY: egui::Key = egui::Key::F;
/// Esc: clear the selection, over the viewport.
pub(crate) const CLEAR_KEY: egui::Key = egui::Key::Escape;

/// F1: the keyboard shortcuts, wherever the pointer is.
pub fn help_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F1)
}

/// Something outside the window to open: a web page, or a folder in the
/// platform's file manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum External {
    Url(String),
    Folder(PathBuf),
}

/// What the shortcuts window groups its rows by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutGroup {
    Files,
    Edit,
    View,
    History,
    Mouse,
}

impl ShortcutGroup {
    pub const ALL: [ShortcutGroup; 5] = [
        ShortcutGroup::Files,
        ShortcutGroup::Edit,
        ShortcutGroup::View,
        ShortcutGroup::History,
        ShortcutGroup::Mouse,
    ];

    pub fn title(self) -> &'static str {
        match self {
            ShortcutGroup::Files => "Files and documents",
            ShortcutGroup::Edit => "Undo and redo (not while typing in a field)",
            ShortcutGroup::View => "View and selection",
            ShortcutGroup::History => "History player (over the viewport or the feature list)",
            ShortcutGroup::Mouse => "Mouse",
        }
    }
}

/// What is pressed: a key with its modifiers, or something done with the
/// mouse, in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutInput {
    Keys(egui::KeyboardShortcut),
    Mouse(&'static str),
}

/// One row of the shortcuts window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub group: ShortcutGroup,
    pub input: ShortcutInput,
    pub action: &'static str,
}

impl Shortcut {
    /// The keys as the platform writes them (Ctrl+S, or ⌘S on a Mac), or
    /// the mouse's words.
    pub fn input_text(&self, mac: bool) -> String {
        match self.input {
            ShortcutInput::Keys(keys) => keys.format(&egui::ModifierNames::NAMES, mac),
            ShortcutInput::Mouse(words) => words.to_string(),
        }
    }
}

/// Every shortcut of the window, in the order the window lists them.
pub fn shortcuts() -> Vec<Shortcut> {
    use ShortcutGroup::{Edit, Files, History, Mouse, View};
    let keys = |group, shortcut, action| Shortcut {
        group,
        input: ShortcutInput::Keys(shortcut),
        action,
    };
    let bare = |key| egui::KeyboardShortcut::new(egui::Modifiers::NONE, key);
    let mouse = |words, action| Shortcut {
        group: Mouse,
        input: ShortcutInput::Mouse(words),
        action,
    };
    let mut rows: Vec<Shortcut> = FileAction::ALL
        .iter()
        .filter_map(|action| {
            let what = match action {
                FileAction::New => "New document",
                FileAction::Open => "Open a document",
                FileAction::Save => "Save",
                FileAction::SaveAs => "Save as",
                FileAction::Import => "Import a STEP file",
                FileAction::Export => "Export the bodies or the selection",
                FileAction::ExportJig | FileAction::OpenForEditing => "",
            };
            action
                .shortcut()
                .map(|shortcut| keys(Files, shortcut, what))
        })
        .collect();
    rows.extend([
        keys(Files, close_shortcut(), "Close the document"),
        keys(Files, next_shortcut(), "Next document"),
        keys(Files, previous_shortcut(), "Previous document"),
    ]);
    for action in EditAction::ALL {
        let what = match action {
            EditAction::Undo => "Undo the last change to the document",
            EditAction::Redo => "Redo the change last undone",
        };
        rows.extend(
            action
                .shortcuts()
                .iter()
                .map(|shortcut| keys(Edit, *shortcut, what)),
        );
    }
    rows.extend([
        keys(
            View,
            bare(FIT_KEY),
            "Fly to the selection, or frame everything",
        ),
        keys(
            View,
            bare(CLEAR_KEY),
            "Fold the Studio or Analysis options, else clear the selection",
        ),
        keys(View, help_shortcut(), "Show these shortcuts"),
        keys(History, bare(player::PLAY_KEY), "Play or pause the history"),
        keys(History, bare(player::BACK_KEY), "A step back"),
        keys(History, bare(player::FORWARD_KEY), "A step forward"),
        keys(History, bare(player::START_KEY), "The first step"),
        keys(
            History,
            bare(player::END_KEY),
            "The end of the history (the view only: the rollback marker stays)",
        ),
        mouse("Drag", "Orbit"),
        mouse("Right or middle drag, Shift+drag", "Pan"),
        mouse("Wheel, pinch", "Zoom about the point under the pointer"),
        mouse("Click", "Select a vertex, an edge or a face"),
        mouse("Shift+click", "Add to or take from the selection"),
        mouse("Click again, Alt+click", "Select the next face behind"),
        mouse("Double-click in the viewport", "Select the whole body"),
        mouse("Right click", "List every face under the pointer"),
        mouse("Ctrl+drag", "Select what lies inside a box"),
        mouse(
            "Double-click a feature",
            "Select the feature and fly to what it made",
        ),
        mouse("Click the view cube", "Turn to that view"),
    ]);
    rows
}

/// What the Help menu remembers between frames.
#[derive(Default)]
pub(crate) struct HelpMemory {
    /// What to open outside the window, until the host takes it.
    pub(super) external: Option<External>,
    /// Where 3DCraft keeps its data (autosaves, crash recovery, the
    /// recent files), when the host keeps any.
    data_folder: Option<PathBuf>,
}

impl Shell {
    /// A page or a folder the Help menu asked to open, for the host to
    /// open (the browser's page opens links itself, and never queues
    /// one).
    pub fn take_external(&mut self) -> Option<External> {
        self.help.external.take()
    }

    /// The folder Help, Open data folder shows: where the host keeps the
    /// autosaves, what can be recovered and the recent files. `None`
    /// leaves the item out.
    pub fn set_data_folder(&mut self, folder: Option<PathBuf>) {
        self.help.data_folder = folder;
    }

    pub fn data_folder(&self) -> Option<&std::path::Path> {
        self.help.data_folder.as_deref()
    }

    /// What About 3DCraft copies: the version and commit, the platform and
    /// the window's own details (frontend, renderer), as the first lines
    /// of a bug report (`session.report`) name them.
    pub fn system_info(&self) -> String {
        let mut out = format!(
            "3DCraft {VERSION}\ncommit: {}\nplatform: {}\n",
            craft3d_engine::build_commit().unwrap_or("unknown"),
            craft3d_engine::platform()
        );
        for (name, value) in self.host_details() {
            out.push_str(&format!("{name}: {value}\n"));
        }
        out
    }

    /// Opens `target` outside the window: in the browser at once, else by
    /// way of the host.
    pub(crate) fn open_external(&mut self, ctx: &egui::Context, target: External) {
        #[cfg(target_arch = "wasm32")]
        if let External::Url(url) = &target {
            ctx.open_url(egui::OpenUrl::new_tab(url));
            return;
        }
        self.help.external = Some(target);
        ctx.request_repaint();
    }

    /// F1, wherever the pointer is (not while typing into a field).
    pub(super) fn help_keys(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        if ui.input_mut(|i| i.consume_shortcut(&help_shortcut())) {
            self.state.dialogs.shortcuts = !self.state.dialogs.shortcuts;
        }
    }

    /// The Help menu, after File.
    pub(super) fn help_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Help", |ui| {
            if ui.button("User guide").clicked() {
                self.open_external(ui.ctx(), External::Url(GUIDE_URL.to_string()));
                ui.close();
            }
            let f1 = ui.ctx().format_shortcut(&help_shortcut());
            if ui
                .add(egui::Button::new("Keyboard shortcuts").shortcut_text(f1))
                .clicked()
            {
                self.state.dialogs.shortcuts = true;
                ui.close();
            }
            #[cfg(not(target_arch = "wasm32"))]
            self.agents_menu_item(ui);
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(folder) = self.help.data_folder.clone() {
                let item = ui.button("Open data folder").on_hover_text(format!(
                    "Autosaves, crash recovery and the recent files: {}",
                    folder.display()
                ));
                if item.clicked() {
                    self.open_external(ui.ctx(), External::Folder(folder));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("About 3DCraft").clicked() {
                self.state.dialogs.about = true;
                ui.close();
            }
        });
    }

    /// The Help menu's windows, those that are open; and a frame soon
    /// for the host to take what the menus or the start screen asked for.
    pub(super) fn help_windows(&mut self, ui: &egui::Ui) {
        if self.help.external.is_some()
            || self.agents_pending()
            || self.recent.chosen.is_some()
            || self.start.chosen.is_some()
            || self.file_action.is_some()
        {
            ui.ctx().request_repaint();
        }
        if self.state.dialogs.about {
            self.about_window(ui.ctx());
        }
        if self.state.dialogs.shortcuts {
            self.shortcuts_window(ui.ctx());
        }
        self.agents_popover(ui.ctx());
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        let report = self.renderer_report();
        let commit = craft3d_engine::build_commit().unwrap_or("unknown");
        let mut open = true;
        let mut go = None;
        let mut copy = false;
        egui::Window::new("About 3DCraft")
            .id(egui::Id::new("craft3d-about"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(egui::RichText::new(super::PRODUCT).size(22.0).strong());
                ui.colored_label(dim, format!("Version {VERSION} (commit {commit})"));
                ui.add_space(tokens.inset * 0.6);
                ui.label("A parametric 3D CAD modeller built for AI: every action is a command, for the window, the command line and agents over MCP.");
                ui.add_space(tokens.inset * 0.6);
                egui::Grid::new("craft3d-about-facts")
                    .num_columns(2)
                    .spacing([tokens.inset, tokens.inset * 0.4])
                    .show(ui, |ui| {
                        ui.colored_label(dim, "Licence");
                        ui.label(LICENCE);
                        ui.end_row();
                        ui.colored_label(dim, "Platform");
                        ui.label(craft3d_engine::platform());
                        ui.end_row();
                        ui.colored_label(dim, "Renderer");
                        ui.add(egui::Label::new(report.describe()).wrap());
                        ui.end_row();
                        ui.colored_label(dim, "User guide");
                        if ui.link(GUIDE_URL).clicked() {
                            go = Some(GUIDE_URL);
                        }
                        ui.end_row();
                        ui.colored_label(dim, "Source");
                        if ui.link(REPO_URL).clicked() {
                            go = Some(REPO_URL);
                        }
                        ui.end_row();
                    });
                ui.add_space(tokens.inset * 0.6);
                copy = ui
                    .button("Copy system info")
                    .on_hover_text(
                        "The version, commit, platform and graphics, to paste into a bug report",
                    )
                    .clicked();
            });
        if copy {
            ctx.copy_text(self.system_info());
        }
        if let Some(url) = go {
            self.open_external(ctx, External::Url(url.to_string()));
        }
        if !open {
            self.state.dialogs.about = false;
        }
    }

    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        let mac = ctx.os() == egui::os::OperatingSystem::Mac;
        let rows = shortcuts();
        // As tall as the window allows, less room for the title.
        let height = (ctx.content_rect().height() - 120.0).max(120.0);
        let mut open = true;
        egui::Window::new("Keyboard shortcuts")
            .id(egui::Id::new("craft3d-shortcuts"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                // One grid, so the columns line up across the groups.
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .min_scrolled_height(height)
                    .show(ui, |ui| {
                        egui::Grid::new("craft3d-shortcuts-grid")
                            .num_columns(2)
                            .spacing([tokens.inset * 2.0, tokens.inset * 0.3])
                            .show(ui, |ui| {
                                for (index, group) in ShortcutGroup::ALL.into_iter().enumerate() {
                                    // An empty row between the groups
                                    // (a grid takes no spacing).
                                    if index > 0 {
                                        ui.label("");
                                        ui.end_row();
                                    }
                                    ui.label(egui::RichText::new(group.title()).strong());
                                    ui.end_row();
                                    for row in rows.iter().filter(|row| row.group == group) {
                                        ui.colored_label(dim, row.input_text(mac));
                                        ui.label(row.action);
                                        ui.end_row();
                                    }
                                }
                            });
                    });
            });
        if !open {
            self.state.dialogs.shortcuts = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_two_rows_name_the_same_keys() {
        let rows = shortcuts();
        let keys: Vec<_> = rows
            .iter()
            .filter_map(|row| match row.input {
                ShortcutInput::Keys(keys) => Some(keys),
                ShortcutInput::Mouse(_) => None,
            })
            .collect();
        for (i, a) in keys.iter().enumerate() {
            for b in keys.iter().skip(i + 1) {
                assert_ne!(a, b, "{a:?} twice");
            }
        }
    }

    #[test]
    fn the_table_holds_every_shortcut_the_shell_reacts_to() {
        let rows = shortcuts();
        let has = |shortcut: egui::KeyboardShortcut| {
            rows.iter()
                .any(|row| row.input == ShortcutInput::Keys(shortcut))
        };
        let bare = |key| egui::KeyboardShortcut::new(egui::Modifiers::NONE, key);
        for action in FileAction::ALL {
            if let Some(shortcut) = action.shortcut() {
                assert!(has(shortcut), "{action:?}");
            }
        }
        for action in EditAction::ALL {
            for shortcut in action.shortcuts() {
                assert!(has(*shortcut), "{action:?} {shortcut:?}");
            }
        }
        for shortcut in [
            close_shortcut(),
            next_shortcut(),
            previous_shortcut(),
            help_shortcut(),
            bare(egui::Key::F),
            bare(egui::Key::Escape),
            bare(egui::Key::Space),
            bare(egui::Key::ArrowLeft),
            bare(egui::Key::ArrowRight),
            bare(egui::Key::Home),
            bare(egui::Key::End),
        ] {
            assert!(has(shortcut), "{shortcut:?}");
        }
        for words in ["Drag", "Wheel, pinch", "Double-click a feature"] {
            assert!(
                rows.iter()
                    .any(|row| row.input == ShortcutInput::Mouse(words)),
                "{words}"
            );
        }
        // Every group has rows, and each row an action.
        for group in ShortcutGroup::ALL {
            assert!(rows.iter().any(|row| row.group == group), "{group:?}");
        }
        assert!(rows.iter().all(|row| !row.action.is_empty()));
    }

    #[test]
    fn keys_are_written_the_platforms_way() {
        let save = shortcuts()
            .into_iter()
            .find(|row| row.action == "Save as")
            .unwrap();
        assert_eq!(save.input_text(false), "Ctrl+Shift+S");
        assert!(save.input_text(true).contains('S'));
        let fit = shortcuts()
            .into_iter()
            .find(|row| row.action.starts_with("Fly to the selection"))
            .unwrap();
        assert_eq!(fit.input_text(false), "F");
    }
}
