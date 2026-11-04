//! The menu bar's menus as the kit draws them (boards s4, f2 and w4):
//! File · Edit · View · Help, dark plates that drop from the black menu
//! bar, items with their keys in the dim ink and the documents' line icons
//! (the plan's middle way). The View menu holds the side panel, the status
//! bar, the appearance (System, Light, Dark) and full screen; its toggles
//! are the state's fields (`panels.tree`, `panels.status`, `appearance`),
//! so `ui.set` and `ui.get` reach them too.

use super::{FileAction, Shell};
use crate::kit::{Kit, MenuItem};
use crate::state::Appearance;

/// Ctrl+B: the side panel, shown or put away.
pub fn side_panel_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::B)
}

/// F11: full screen, on the desktop.
pub fn full_screen_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F11)
}

/// Whether the window can go full screen (not the browser's page, whose
/// full screen is the browser's).
pub const FULL_SCREEN: bool = cfg!(not(target_arch = "wasm32"));

/// A menu of the menu bar: `title`, and a plate `width` wide with the
/// kit's items that `add` lays out. Returns what `add` returned while the
/// menu was open.
pub(super) fn kit_menu<R>(
    ui: &mut egui::Ui,
    shell_tokens: &crate::theme::Tokens,
    title: &str,
    width: f32,
    add: impl FnOnce(&Kit, &mut egui::Ui) -> R,
) -> (egui::Rect, Option<R>) {
    let tokens = *shell_tokens;
    // The menu's plate is the context menu's (`Tokens::menu_visuals`):
    // one menu colour in both appearances, a step above the dark panel.
    let config =
        egui::containers::menu::MenuConfig::find(ui).style(move |style: &mut egui::Style| {
            egui::containers::menu::menu_style(style);
            style.visuals = tokens.menu_visuals();
        });
    let content = |ui: &mut egui::Ui| {
        let kit = Kit::plate(&tokens);
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.set_width(width);
        add(&kit, ui)
    };
    // Inside another menu (the phone's ⋯) a submenu that opens beside it.
    let (response, inner) = if egui::containers::menu::is_in_menu(ui) {
        egui::containers::menu::SubMenuButton::from_button(egui::Button::new(title).right_text("›"))
            .config(config)
            .ui(ui, content)
    } else {
        egui::containers::menu::MenuButton::new(title)
            .config(config)
            .ui(ui, content)
    };
    (response.rect, inner.map(|i| i.inner))
}

/// An item's keys as the platform writes them.
fn keys(ui: &egui::Ui, shortcut: Option<egui::KeyboardShortcut>) -> String {
    shortcut
        .map(|shortcut| ui.ctx().format_shortcut(&shortcut))
        .unwrap_or_default()
}

/// Where the menu bar's titles and the open menu's items were drawn in
/// the last frame, by their words.
#[derive(Default)]
pub(crate) struct MenuHits {
    pub(super) titles: Vec<(String, egui::Rect)>,
    pub(super) items: Vec<(String, egui::Rect)>,
}

impl Shell {
    /// Where the menu bar's titles (File, Edit, View, Help) were drawn in
    /// the last frame.
    pub fn menu_titles(&self) -> &[(String, egui::Rect)] {
        &self.menu_hits.titles
    }

    /// Where the items of the open menu were drawn in the last frame, by
    /// their labels (empty while no menu is open).
    pub fn menu_items(&self) -> &[(String, egui::Rect)] {
        &self.menu_hits.items
    }

    /// A kit menu item that says where it was drawn.
    fn item(&mut self, kit: &Kit, ui: &mut egui::Ui, item: MenuItem<'_>) -> egui::Response {
        let label = item.label.to_string();
        let response = kit.menu_item(ui, item);
        self.menu_hits.items.push((label, response.rect));
        response
    }

    /// The menus of the menu bar, after the drawer's button.
    pub(super) fn menus(
        &mut self,
        ui: &mut egui::Ui,
        status: Option<&craft3d_engine::DocumentStatus>,
    ) {
        self.menu_hits.titles.clear();
        self.menu_hits.items.clear();
        let has_document = status.is_some();
        let file = self.file_menu(ui, has_document);
        let edit = self.edit_menu(ui, status);
        let view = self.view_menu(ui);
        let help = self.help_menu(ui);
        self.menu_hits.titles = vec![
            ("File".to_string(), file),
            ("Edit".to_string(), edit),
            ("View".to_string(), view),
            ("Help".to_string(), help),
        ];
    }

    /// File: New Part, New Assembly, Open…, Open for editing, Open recent ›,
    /// Save, Save As…, Import…, Export…, Export as Jig…, then Close and the
    /// open documents.
    pub(super) fn file_menu(&mut self, ui: &mut egui::Ui, has_document: bool) -> egui::Rect {
        let tokens = self.tokens;
        kit_menu(ui, &tokens, "File", tokens.menu_widths[0], |kit, ui| {
            for action in FileAction::ALL {
                if action.starts_group() {
                    kit.menu_separator(ui);
                }
                let shortcut = keys(ui, action.shortcut());
                let mut item = MenuItem::new(action.label())
                    .icon_room()
                    .shortcut(&shortcut)
                    .enabled(
                        (has_document || !action.needs_document())
                            && self.file_action_available(action),
                    );
                if let Some(glyph) = action.glyph() {
                    item = item.icon(glyph);
                }
                if self.item(kit, ui, item).clicked() {
                    self.file_action = Some(action);
                    ui.close();
                }
                if action == FileAction::OpenForEditing {
                    // Open recent ›, under the ways to open.
                    self.recent_menu(ui, kit);
                }
            }
            kit.menu_separator(ui);
            self.menu_documents(ui, kit);
        })
        .0
    }

    /// View: the side panel and the status bar shown or not, the
    /// appearance, full screen.
    pub(super) fn view_menu(&mut self, ui: &mut egui::Ui) -> egui::Rect {
        let tokens = self.tokens;
        kit_menu(ui, &tokens, "View", tokens.menu_widths[2], |kit, ui| {
            let ctrl_b = keys(ui, Some(side_panel_shortcut()));
            let item = MenuItem::new("Side panel")
                .icon_room()
                .shortcut(&ctrl_b)
                .checked(self.state.panels.tree);
            if self.item(kit, ui, item).clicked() {
                self.state.panels.tree = !self.state.panels.tree;
                ui.close();
            }
            let item = MenuItem::new("Status bar").icon_room().checked(self.state.panels.status);
            if self.item(kit, ui, item).clicked() {
                self.state.panels.status = !self.state.panels.status;
                ui.close();
            }
            kit.menu_separator(ui);
            kit.menu_caption(ui, "Appearance");
            for (appearance, label, hint) in [
                (
                    Appearance::System,
                    "System",
                    "Follows your system's light or dark setting. ui.set {\"appearance\": \"system\"}",
                ),
                (
                    Appearance::Light,
                    "Light",
                    "Light panels, whatever the system says. ui.set {\"appearance\": \"light\"}",
                ),
                (
                    Appearance::Dark,
                    "Dark",
                    "Dark panels, whatever the system says. ui.set {\"appearance\": \"dark\"}",
                ),
            ] {
                let item = MenuItem::new(label)
                    .icon_room()
                    .checked(self.state.appearance == appearance);
                let response = self.item(kit, ui, item).on_hover_text(hint);
                if response.clicked() {
                    self.state.appearance = appearance;
                    ui.ctx().request_repaint();
                    ui.close();
                }
            }
            if FULL_SCREEN {
                kit.menu_separator(ui);
                let f11 = keys(ui, Some(full_screen_shortcut()));
                let full = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
                let item = MenuItem::new("Full screen")
                    .icon_room()
                    .shortcut(&f11)
                    .checked(full);
                if self.item(kit, ui, item).clicked() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
                    ui.close();
                }
            }
        })
        .0
    }

    /// Ctrl+B and F11, wherever the pointer is (not while typing into a
    /// field, nor under a dialog).
    pub(super) fn view_shortcuts(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() || self.dialog_open() {
            return;
        }
        if ui.input_mut(|i| i.consume_shortcut(&side_panel_shortcut())) {
            self.state.panels.tree = !self.state.panels.tree;
        }
        if FULL_SCREEN && ui.input_mut(|i| i.consume_shortcut(&full_screen_shortcut())) {
            let full = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
        }
    }
}
