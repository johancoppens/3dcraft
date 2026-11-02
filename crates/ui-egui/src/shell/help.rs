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
use crate::kit::{Button, Dialog, Glyph, Kit};
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
                FileAction::New => "New part",
                FileAction::NewAssembly => "New assembly",
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
        keys(
            View,
            super::menus::side_panel_shortcut(),
            "Show or put away the side panel",
        ),
        keys(
            View,
            super::menus::full_screen_shortcut(),
            "Full screen (the desktop app)",
        ),
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

    /// The Help menu, after View.
    pub(super) fn help_menu(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        use crate::kit::MenuItem;
        let tokens = self.tokens;
        super::menus::kit_menu(ui, &tokens, "Help", 240.0, |kit, ui| {
            if kit.menu_item(ui, MenuItem::new("User guide")).clicked() {
                self.open_external(ui.ctx(), External::Url(GUIDE_URL.to_string()));
                ui.close();
            }
            let f1 = ui.ctx().format_shortcut(&help_shortcut());
            if kit
                .menu_item(ui, MenuItem::new("Keyboard shortcuts").shortcut(&f1))
                .clicked()
            {
                self.state.dialogs.shortcuts = true;
                ui.close();
            }
            #[cfg(not(target_arch = "wasm32"))]
            self.agents_menu_item(ui, kit);
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(folder) = self.help.data_folder.clone() {
                let item = kit
                    .menu_item(ui, MenuItem::new("Open data folder"))
                    .on_hover_text(format!(
                        "Autosaves, crash recovery and the recent files: {}",
                        folder.display()
                    ));
                if item.clicked() {
                    self.open_external(ui.ctx(), External::Folder(folder));
                    ui.close();
                }
            }
            kit.menu_separator(ui);
            if kit.menu_item(ui, MenuItem::new("About 3DCraft")).clicked() {
                self.state.dialogs.about = true;
                ui.close();
            }
        })
        .0
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
        self.agent_prompt(ui.ctx());
    }

    /// About 3DCraft (board s4): the app icon, the name and what it is,
    /// the facts in two columns, Copy system info at the left and Close at
    /// the right.
    fn about_window(&mut self, ctx: &egui::Context) {
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let report = self.renderer_report();
        let commit = craft3d_engine::build_commit().unwrap_or("unknown");
        let dark = self.dark;
        let mut go = None;
        let mut copy = false;
        let mut close = false;
        let dialog = Dialog::new(egui::Id::new("craft3d-about"), "About 3DCraft")
            .width(tokens.dialog_width)
            .modal(false);
        let shown = kit.dialog(ctx, dialog, |kit, ui| {
            ui.horizontal(|ui| {
                let side = tokens.about_icon;
                let (place, _) =
                    ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
                crate::kit::brand::paint(ui, &tokens, crate::kit::AppIcon::Part, place, dark);
                ui.add_space(tokens.nav_icon_gap * 2.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(
                        egui::RichText::new(super::PRODUCT)
                            .font(egui::FontId::new(20.0, kit.medium_font(ui).family))
                            .color(color(tokens.text)),
                    );
                    ui.label(
                        egui::RichText::new("Parametric CAD built for AI")
                            .font(kit.font())
                            .color(color(tokens.text_dim)),
                    );
                });
            });
            let label = 96.0;
            let value = |ui: &mut egui::Ui, text: &str| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(text)
                            .font(kit.font())
                            .color(color(tokens.text)),
                    )
                    .wrap(),
                );
            };
            kit.dialog_row(ui, "Version", label, |ui| {
                value(ui, &format!("{VERSION} (commit {commit})"));
            });
            kit.dialog_row(ui, "Platform", label, |ui| {
                value(ui, &craft3d_engine::platform())
            });
            kit.dialog_row(ui, "Renderer", label, |ui| value(ui, &report.describe()));
            kit.dialog_row(ui, "Licence", label, |ui| value(ui, LICENCE));
            kit.dialog_row(ui, "Guide", label, |ui| {
                if kit
                    .button(ui, Button::link("User guide"))
                    .on_hover_text(GUIDE_URL)
                    .clicked()
                {
                    go = Some(GUIDE_URL);
                }
            });
            kit.dialog_row(ui, "Source", label, |ui| {
                let shown = REPO_URL.trim_start_matches("https://");
                if kit
                    .button(ui, Button::link(shown))
                    .on_hover_text(REPO_URL)
                    .clicked()
                {
                    go = Some(REPO_URL);
                }
            });
            kit.dialog_footer(ui, |ui| {
                copy = kit
                    .button(ui, Button::quiet("Copy system info").icon(Glyph::Copy))
                    .on_hover_text(
                        "The version, commit, platform and graphics, to paste into a bug report",
                    )
                    .clicked();
                kit.footer_right(ui, |ui| {
                    close = kit.button(ui, Button::secondary("Close")).clicked();
                });
            });
        });
        if copy {
            ctx.copy_text(self.system_info());
        }
        if let Some(url) = go {
            self.open_external(ctx, External::Url(url.to_string()));
        }
        if close || shown.close {
            self.state.dialogs.about = false;
        }
    }

    /// The keyboard shortcuts (F1, board s4): the groups in two columns,
    /// each key on a keycap at the left and what it does at the right.
    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let mac = ctx.os() == egui::os::OperatingSystem::Mac;
        let rows = shortcuts();
        let screen = ctx.content_rect();
        let two = screen.width() >= 760.0;
        let width = if two { 720.0 } else { 420.0 };
        // As tall as the window allows, less room for the title.
        let height = (screen.height() - 160.0).max(120.0);
        let dialog = Dialog::new(egui::Id::new("craft3d-shortcuts"), "Keyboard shortcuts")
            .icon(Glyph::Keyboard, tokens.text_dim)
            .width(width)
            .modal(false);
        let group_ui = |ui: &mut egui::Ui, group: ShortcutGroup| {
            kit.section_header(ui, group.title());
            for row in rows.iter().filter(|row| row.group == group) {
                ui.horizontal(|ui| {
                    keycap(ui, &kit, &row.input_text(mac));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(row.action)
                                    .font(kit.font())
                                    .color(color(tokens.text_dim)),
                            )
                            .truncate(),
                        );
                    });
                });
            }
            ui.add_space(tokens.nav_spacing * 2.0);
        };
        let shown = kit.dialog(ctx, dialog, |_, ui| {
            egui::ScrollArea::vertical()
                .max_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let (left, right): (Vec<ShortcutGroup>, Vec<ShortcutGroup>) =
                        ShortcutGroup::ALL.into_iter().partition(|g| {
                            matches!(
                                g,
                                ShortcutGroup::Files | ShortcutGroup::Edit | ShortcutGroup::View
                            )
                        });
                    if two {
                        ui.columns(2, |columns| {
                            if let [a, b] = columns {
                                a.spacing_mut().item_spacing.y = 2.0;
                                b.spacing_mut().item_spacing.y = 2.0;
                                for group in &left {
                                    group_ui(a, *group);
                                }
                                for group in &right {
                                    group_ui(b, *group);
                                }
                            }
                        });
                    } else {
                        for group in ShortcutGroup::ALL {
                            group_ui(ui, group);
                        }
                    }
                });
        });
        if shown.close {
            self.state.dialogs.shortcuts = false;
        }
    }
}

/// A key or a mouse gesture on a keycap: the field's fill, its rim, the
/// text in the small font.
fn keycap(ui: &mut egui::Ui, kit: &Kit, text: &str) {
    let t = kit.t;
    let s = kit.skin;
    let galley = kit.galley(ui, text, kit.small_font(), s.text);
    let size = egui::vec2(galley.size().x + 12.0, t.chip_height + 2.0);
    let (outer, _) =
        ui.allocate_exact_size(egui::vec2(size.x, t.row_height + 4.0), egui::Sense::hover());
    let rect = egui::Rect::from_center_size(outer.center(), size);
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(t.radius),
        color(s.field),
        egui::Stroke::new(1.0, color(s.field_stroke)),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, color(s.text));
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
