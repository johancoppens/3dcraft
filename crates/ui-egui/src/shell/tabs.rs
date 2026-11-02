//! The open documents in the window: a row of tabs above the viewport, a
//! list in the File menu, Ctrl+Tab and Ctrl+Shift+Tab, and the question
//! before a document with unsaved changes is closed from its tab.
//!
//! Like the File menu, the shell only asks ([`DocAction`]): the host runs
//! the command (`file.activate`, `file.close`, `file.save`) on its engine
//! thread, and the tabs follow the session at the next sync.

use super::Shell;
use crate::state::{Closing, DocTab};
use crate::theme::{color, color_alpha};
use craft3d_engine::{DocumentStatus, Session};

/// What a tab, the File menu or a shortcut asks the host to do with an
/// open document, by its number in `file.list`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocAction {
    /// Show it: `file.activate`.
    Activate(usize),
    /// Close it: `file.close`, with `discard` when the person chose to
    /// drop its unsaved changes. The document that was active stays
    /// active; when it is the one closed, its neighbour in the row is.
    Close { document: usize, discard: bool },
    /// Save it, then close it: the Save of the question. To its own file;
    /// one that has none is named in the save dialog first.
    SaveAndClose(usize),
}

/// Where closing the window is ([`Shell::advance_quit`]).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Quitting {
    /// The number of open documents, and of those with unsaved changes,
    /// when the last question was asked; `None` before the first.
    asked_with: Option<(usize, usize)>,
    /// The document made the active one to be asked about next, so the
    /// person sees what the question is about.
    shown: Option<usize>,
}

/// The answers to the question before closing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAnswer {
    Save,
    Discard,
    Cancel,
}

impl CloseAnswer {
    /// In the order the buttons are drawn.
    pub const ALL: [CloseAnswer; 3] =
        [CloseAnswer::Save, CloseAnswer::Discard, CloseAnswer::Cancel];

    pub fn label(self) -> &'static str {
        match self {
            CloseAnswer::Save => "Save",
            CloseAnswer::Discard => "Don't Save",
            CloseAnswer::Cancel => "Cancel",
        }
    }
}

/// What a click in the row of tabs was on: the tab itself, or its close
/// button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabPart {
    Tab(usize),
    Close(usize),
    /// The button that lists every open document, when the tabs do not
    /// all fit.
    All,
}

/// The mark before the name of a document with unsaved changes, and the
/// close button. Both are in egui's default fonts (a test checks).
pub const UNSAVED_MARK: char = '•';
pub const CLOSE_MARK: char = '×';

/// Close the active document: Ctrl+W (⌘W on a Mac), in the File menu as
/// Close.
pub fn close_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::W)
}

/// The next document: Ctrl+Tab; the one before: Ctrl+Shift+Tab. Both go
/// round.
pub fn next_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Tab)
}

pub fn previous_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(
        egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        egui::Key::Tab,
    )
}

/// Longest name a tab keeps, in characters (it is cut to the tab's width
/// when drawn).
const MAX_NAME_CHARS: usize = 120;

/// The name a tab shows for a file: its name without the folders it is in
/// and without its extension, on one line. `None` for a document without
/// a file, or a path with no name in it.
pub fn tab_name(path: Option<&str>) -> Option<String> {
    let file = path?.rsplit(['/', '\\']).find(|part| !part.is_empty())?;
    // `gear.3dcraft` is `gear`; `.hidden` keeps its dot.
    let stem = match file.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file,
    };
    let name: String = stem
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    (!name.is_empty()).then_some(name)
}

/// The line icon of a tab's document (the plan's middle way): a Jig's for
/// a `.3dcraftjig` (in use, or a model of one), an assembly's for a
/// `.3dcraftasm`, a part's otherwise.
pub fn tab_glyph(tab: &DocTab) -> crate::kit::Glyph {
    if tab.jig_in_use {
        crate::kit::Glyph::Jig
    } else if tab.assembly {
        crate::kit::Glyph::Assembly
    } else {
        tab.path
            .as_deref()
            .map_or(crate::kit::Glyph::Part, super::recent::file_glyph)
    }
}

/// The tabs for the open documents, in their order: named after their
/// files, and `Untitled 1`, `Untitled 2`, … for the ones never saved,
/// counted in the order of the row.
pub fn doc_tabs(statuses: &[DocumentStatus], active: Option<usize>) -> Vec<DocTab> {
    let mut untitled = 0usize;
    statuses
        .iter()
        .map(|status| {
            let name = tab_name(status.path.as_deref()).unwrap_or_else(|| {
                untitled = untitled.saturating_add(1);
                format!("Untitled {untitled}")
            });
            DocTab {
                document: status.index,
                name,
                path: status.path.clone(),
                dirty: status.dirty,
                active: active == Some(status.index),
                asks_to_close: status.close_asks,
                jig_in_use: status.jig_in_use,
                assembly: status
                    .path
                    .as_deref()
                    .is_some_and(|p| p.to_ascii_lowercase().ends_with(".3dcraftasm")),
            }
        })
        .collect()
}

impl Shell {
    /// The tabs as of the last sync: one per open document, in the order
    /// of `file.list`.
    pub fn tabs(&self) -> &[DocTab] {
        &self.tabs
    }

    /// The question before closing, while it is asked.
    pub fn closing(&self) -> Option<&Closing> {
        self.closing.as_ref()
    }

    /// Where the last frame drew the tabs and their close buttons, in
    /// points. A tab scrolled out of the row is listed where it would be.
    pub fn tab_hits(&self) -> &[(TabPart, egui::Rect)] {
        &self.tab_hits
    }

    /// Where the last frame drew the buttons of the question before
    /// closing, while it is asked.
    pub fn closing_hits(&self) -> &[(CloseAnswer, egui::Rect)] {
        &self.closing_hits
    }

    /// What a tab, the menu or a shortcut asked for since the last call.
    /// The host takes it and runs the command.
    pub fn take_doc_action(&mut self) -> Option<DocAction> {
        self.doc_action.take()
    }

    /// Asks for a document action, as a tab does.
    pub fn request_doc_action(&mut self, action: DocAction) {
        self.doc_action = Some(action);
    }

    /// Asks to make document `document` the active one; nothing when it
    /// is the active one already or there is no such tab.
    pub fn request_activate(&mut self, document: usize) {
        if let Some(tab) = self.tabs.iter().find(|tab| tab.document == document)
            && !tab.active
        {
            self.doc_action = Some(DocAction::Activate(document));
        }
    }

    /// Asks to close document `document`, as its close button does: at
    /// once when it has nothing worth keeping unsaved, otherwise after
    /// the question ([`Self::answer_closing`]).
    pub fn request_close(&mut self, document: usize) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.document == document) else {
            return;
        };
        if tab.asks_to_close {
            self.closing = Some(Closing {
                document,
                name: tab.name.clone(),
                path: tab.path.clone(),
                quitting: self.quitting.is_some(),
                jig: tab.jig_in_use,
            });
        } else {
            self.doc_action = Some(DocAction::Close {
                document,
                discard: false,
            });
        }
    }

    /// The person's answer to the question before closing. Cancel while
    /// the window closes keeps the window, with every document that is
    /// still open.
    pub fn answer_closing(&mut self, answer: CloseAnswer) {
        let Some(closing) = self.closing.take() else {
            return;
        };
        if answer == CloseAnswer::Cancel {
            self.quitting = None;
        }
        let document = closing.document;
        self.doc_action = match answer {
            CloseAnswer::Save => Some(DocAction::SaveAndClose(document)),
            CloseAnswer::Discard => Some(DocAction::Close {
                document,
                discard: true,
            }),
            CloseAnswer::Cancel => None,
        };
    }

    /// The window was asked to close (its close button, or
    /// `ui.quit {save: "ask"}`). True when it may close now: no document
    /// has unsaved changes worth asking about (then the application is
    /// asked to stop, [`Self::quit_requested`]). Otherwise the window
    /// stays and asks, one document at a time, Save, Don't Save or Cancel
    /// ([`Self::advance_quit`]); once each was saved or dropped the
    /// application stops, and Cancel keeps it. Asked again while it asks,
    /// nothing changes. As of the last sync: while a command runs the
    /// tabs are as they were before it.
    pub fn request_window_close(&mut self) -> bool {
        if self.quit {
            return true;
        }
        if self.quitting.is_some() {
            return false;
        }
        if !self.tabs.iter().any(|tab| tab.asks_to_close) {
            self.quit = true;
            return true;
        }
        self.quitting = Some(Quitting::default());
        false
    }

    /// The window was asked to close while a command runs: its tabs are
    /// as they were before the command, which may yet leave a document
    /// with unsaved changes. Nothing is decided now; once the session is
    /// back, [`Self::advance_quit`] asks about what is unsaved then, or
    /// stops the application when nothing is. Always false (the window
    /// stays for now), unless it is stopping already.
    pub fn request_window_close_when_idle(&mut self) -> bool {
        if self.quit {
            return true;
        }
        if self.quitting.is_none() {
            self.quitting = Some(Quitting::default());
        }
        false
    }

    /// True while the window is closing and asks about the documents with
    /// unsaved changes, one at a time.
    pub fn quitting(&self) -> bool {
        self.quitting.is_some()
    }

    /// The next step of closing the window, when the host has the session
    /// back and nothing it asked for is pending (no question, no document
    /// action, no save dialog): asks about the next document with unsaved
    /// changes, or, when there is none left, asks the application to
    /// stop. When the last answer closed nothing (its save failed, or the
    /// save dialog was cancelled), the window stays: what went wrong is in
    /// the status bar. True when something changed.
    pub fn advance_quit(&mut self) -> bool {
        let Some(Quitting { asked_with, shown }) = self.quitting else {
            return false;
        };
        if self.closing.is_some() || self.doc_action.is_some() {
            return false;
        }
        let asking = self.tabs.iter().filter(|tab| tab.asks_to_close).count();
        // Neither a document fewer, nor one fewer with unsaved changes:
        // the last answer did nothing.
        if asked_with.is_some_and(|(open, unsaved)| self.tabs.len() >= open && asking >= unsaved) {
            self.quitting = None;
            return true;
        }
        match self.tabs.iter().find(|tab| tab.asks_to_close) {
            None => {
                self.quitting = None;
                self.quit = true;
            }
            // Shown first, so the person sees what the question is about
            // (once: a document that cannot be shown is asked about all
            // the same).
            Some(tab) if !tab.active && shown != Some(tab.document) => {
                let document = tab.document;
                self.quitting = Some(Quitting {
                    asked_with,
                    shown: Some(document),
                });
                self.doc_action = Some(DocAction::Activate(document));
            }
            Some(tab) => {
                let document = tab.document;
                self.quitting = Some(Quitting {
                    asked_with: Some((self.tabs.len(), asking)),
                    shown: None,
                });
                self.request_close(document);
            }
        }
        true
    }

    /// The tabs follow the session; a question about a document that is
    /// not there any more as it was asked about, or has nothing unsaved
    /// any more, is dropped.
    pub(super) fn sync_tabs(&mut self, session: &Session) {
        let active = session.document_status().map(|status| status.index);
        let statuses = session.document_statuses();
        self.tabs = doc_tabs(&statuses, active);
        self.assembly_docs
            .retain(|id| statuses.iter().any(|status| status.id == *id));
        for (tab, status) in self.tabs.iter_mut().zip(&statuses) {
            tab.assembly = self.assembly_docs.contains(&status.id);
        }
        if let Some(closing) = &self.closing {
            let same = self.tabs.iter().any(|tab| {
                tab.document == closing.document
                    && tab.name == closing.name
                    && tab.path == closing.path
                    && tab.asks_to_close
            });
            if !same {
                self.closing = None;
            }
        }
    }

    /// The active tab, by its place in the row.
    fn active_tab(&self) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.active)
    }

    /// Ctrl+Tab, Ctrl+Shift+Tab and Ctrl+W, wherever the pointer is.
    pub(super) fn tab_shortcuts(&mut self, ui: &egui::Ui) {
        if self.closing.is_some() {
            return;
        }
        let count = self.tabs.len();
        // The longer shortcut first: Ctrl+Tab would match it too.
        let previous = ui.input_mut(|i| i.consume_shortcut(&previous_shortcut()));
        let next = !previous && ui.input_mut(|i| i.consume_shortcut(&next_shortcut()));
        if (previous || next)
            && count > 1
            && let Some(at) = self.active_tab()
        {
            let to = if next {
                (at + 1) % count
            } else {
                (at + count - 1) % count
            };
            if let Some(tab) = self.tabs.get(to) {
                self.doc_action = Some(DocAction::Activate(tab.document));
            }
        }
        if ui.input_mut(|i| i.consume_shortcut(&close_shortcut()))
            && let Some(tab) = self.active_tab().and_then(|at| self.tabs.get(at))
        {
            let document = tab.document;
            self.request_close(document);
        }
    }

    /// The open documents in the File menu, the active one checked, and
    /// Close.
    pub(super) fn menu_documents(&mut self, ui: &mut egui::Ui, kit: &crate::kit::Kit) {
        use crate::kit::MenuItem;
        let shortcut = ui.ctx().format_shortcut(&close_shortcut());
        let active = self
            .active_tab()
            .and_then(|at| self.tabs.get(at))
            .map(|tab| tab.document);
        let close = MenuItem::new("Close")
            .icon_room()
            .shortcut(&shortcut)
            .enabled(active.is_some());
        if kit.menu_item(ui, close).clicked() {
            if let Some(document) = active {
                self.request_close(document);
            }
            ui.close();
        }
        if self.tabs.is_empty() {
            return;
        }
        kit.menu_separator(ui);
        kit.menu_caption(ui, "Documents");
        let mut chosen = None;
        for tab in &self.tabs {
            let mark = if tab.dirty {
                format!(" {UNSAVED_MARK}")
            } else {
                String::new()
            };
            let label = format!("{}{mark}", tab.name);
            let item = MenuItem::new(&label)
                .icon(tab_glyph(tab))
                .checked(tab.active);
            if kit.menu_item(ui, item).clicked() {
                chosen = Some(tab.document);
                ui.close();
            }
        }
        if let Some(document) = chosen {
            self.request_activate(document);
        }
    }

    /// The row of tabs. Each tab is as wide as the row allows, between the
    /// theme's narrowest and widest; when even the narrowest do not fit,
    /// the row scrolls, and the active tab is brought into view whenever
    /// another one becomes active.
    pub(super) fn tab_row(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let count = self.tabs.len();
        let gaps = tokens.tab_gap * count.saturating_sub(1) as f32;
        let fair = (ui.available_width() - gaps) / count.max(1) as f32;
        // In the narrow layout a tab keeps a readable width (more of them
        // scroll, and the list at the end names them all).
        let least = if self.narrow() {
            tokens.tab_narrow_min_width
        } else {
            tokens.tab_min_width
        };
        let width = if fair.is_finite() {
            fair.clamp(least, tokens.tab_max_width)
        } else {
            least
        };
        let active = self.active_tab();
        let scroll_to = active.filter(|at| self.tab_scrolled != Some(*at));
        let font = egui::FontId::proportional(13.5);
        let mut hits = Vec::new();
        let mut clicked = None;
        let mut close = None;
        // The active tab is in view: no more scrolling to it until another
        // one becomes active. (It is asked again until it is: the first
        // frames of a panel lay it out without showing it.)
        let mut in_view = false;
        // When even the narrowest tabs do not fit (a phone, many
        // documents), a button at the row's end lists them all.
        let overflow = fair < least && count > 1;
        let menu_width = if overflow {
            tokens.tab_overflow_width
        } else {
            0.0
        };
        let row = ui.available_width();
        let mut chosen = None;
        let mut tab_strip = |ui: &mut egui::Ui| {
            egui::ScrollArea::horizontal()
                .id_salt("craft3d-tabs")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = tokens.tab_gap;
                    ui.horizontal(|ui| {
                        for (at, tab) in self.tabs.iter().enumerate() {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(width, tokens.tab_height),
                                egui::Sense::click(),
                            );
                            let side = tokens.tab_height - 8.0;
                            let close_rect = egui::Rect::from_center_size(
                                egui::pos2(rect.right() - side * 0.5 - 4.0, rect.center().y),
                                egui::vec2(side, side),
                            );
                            let closer = ui.interact(
                                close_rect,
                                ui.id().with(("craft3d-tab-close", at)),
                                egui::Sense::click(),
                            );
                            let hint = match &tab.path {
                                Some(path) => path.clone(),
                                None => "Not saved yet".to_string(),
                            };
                            let response = response.on_hover_text(hint);
                            if scroll_to == Some(at) {
                                response.scroll_to_me_animation(
                                    None,
                                    egui::style::ScrollAnimation::none(),
                                );
                                in_view = ui.clip_rect().contains_rect(rect);
                            }
                            let fill = if tab.active {
                                tokens.tab_active_fill
                            } else if response.hovered() || closer.hovered() {
                                tokens.tab_hover_fill
                            } else {
                                tokens.tab_fill
                            };
                            let painter = ui.painter_at(rect);
                            let round = egui::CornerRadius {
                                nw: tokens.radius,
                                ne: tokens.radius,
                                sw: 0,
                                se: 0,
                            };
                            painter.rect_filled(rect, round, color(fill));
                            if tab.active {
                                let line = egui::Rect::from_min_max(
                                    egui::pos2(
                                        rect.left(),
                                        rect.bottom() - tokens.tab_accent_width,
                                    ),
                                    rect.right_bottom(),
                                );
                                painter.rect_filled(line, 0.0, color(tokens.tab_accent));
                            }
                            // The document's line icon, the name cut to the
                            // room left, the "Jig" chip of a Jig in use; at
                            // the right the close button, or the unsaved
                            // mark until the pointer is on the tab (f2).
                            let text = if tab.active {
                                tokens.chrome_text
                            } else {
                                tokens.chrome_text_dim
                            };
                            let side = tokens.nav_icon_size;
                            let icon = egui::Rect::from_center_size(
                                egui::pos2(rect.left() + 10.0 + side * 0.5, rect.center().y),
                                egui::vec2(side, side),
                            );
                            crate::kit::glyphs::paint(
                                &painter,
                                tab_glyph(tab),
                                icon,
                                color(text),
                                &tokens,
                            );
                            let left = icon.right() + tokens.nav_icon_gap;
                            let chip = tab.jig_in_use.then(|| {
                                painter.layout_no_wrap(
                                    "Jig".to_string(),
                                    egui::FontId::proportional(tokens.secondary_font - 2.0),
                                    color(tokens.pill_wait_text),
                                )
                            });
                            let chip_width = chip.as_ref().map_or(0.0, |c| c.size().x + 12.0 + 6.0);
                            let room = (close_rect.left() - 4.0 - left - chip_width).max(0.0);
                            let mut job = egui::text::LayoutJob::simple_singleline(
                                tab.name.clone(),
                                font.clone(),
                                color(text),
                            );
                            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                            let size = galley.size();
                            painter.galley(
                                egui::pos2(left, rect.center().y - size.y * 0.5),
                                galley,
                                color(text),
                            );
                            if let Some(chip) = chip {
                                let chip_rect = egui::Rect::from_min_size(
                                    egui::pos2(
                                        left + size.x + 6.0,
                                        rect.center().y - tokens.chip_height * 0.4,
                                    ),
                                    egui::vec2(chip.size().x + 12.0, tokens.chip_height * 0.8),
                                );
                                painter.rect_filled(
                                    chip_rect,
                                    egui::CornerRadius::same(tokens.pill_radius),
                                    color(tokens.plate_info_fill),
                                );
                                painter.galley(
                                    chip_rect.center() - chip.size() * 0.5,
                                    chip,
                                    color(tokens.pill_wait_text),
                                );
                            }
                            let pointed = closer.hovered() || response.hovered();
                            if closer.hovered() {
                                painter.rect_filled(
                                    close_rect,
                                    egui::CornerRadius::same(tokens.radius),
                                    color_alpha(tokens.chrome_text_dim, 60),
                                );
                            }
                            if tab.dirty && !pointed {
                                painter.circle_filled(
                                    close_rect.center(),
                                    3.5,
                                    color(tokens.chrome_attention),
                                );
                            } else {
                                let tint = if pointed || tab.active {
                                    tokens.chrome_text
                                } else {
                                    tokens.chrome_text_dim
                                };
                                let side = side * 0.7;
                                crate::kit::glyphs::paint(
                                    &painter,
                                    crate::kit::Glyph::Close,
                                    egui::Rect::from_center_size(
                                        close_rect.center(),
                                        egui::vec2(side, side),
                                    ),
                                    color(tint),
                                    &tokens,
                                );
                            }
                            let closer = closer.on_hover_text("Close");
                            hits.push((TabPart::Tab(tab.document), rect));
                            hits.push((TabPart::Close(tab.document), close_rect));
                            if closer.clicked() || response.middle_clicked() {
                                close = Some(tab.document);
                            } else if response.clicked() {
                                clicked = Some(tab.document);
                            }
                        }
                    });
                });
        };
        let mut all_rect = None;
        if overflow {
            ui.horizontal(|ui| {
                ui.allocate_ui(
                    egui::vec2((row - menu_width).max(0.0), tokens.tab_height),
                    |ui| tab_strip(ui),
                );
                let all = ui
                    .menu_button(format!("All {count}"), |ui| {
                        for tab in &self.tabs {
                            let mark = if tab.dirty {
                                format!(" {UNSAVED_MARK}")
                            } else {
                                String::new()
                            };
                            if ui
                                .add(
                                    egui::Button::new(format!("{}{mark}", tab.name))
                                        .selected(tab.active),
                                )
                                .clicked()
                            {
                                chosen = Some(tab.document);
                                ui.close();
                            }
                        }
                    })
                    .response
                    .on_hover_text("Every open document");
                all_rect = Some(all.rect);
            });
        } else {
            tab_strip(ui);
        }
        if let Some(rect) = all_rect {
            hits.push((TabPart::All, rect));
        }
        if let Some(document) = chosen {
            self.request_activate(document);
        }
        self.tab_hits = hits;
        if in_view {
            self.tab_scrolled = scroll_to;
        } else if scroll_to.is_some() {
            ui.ctx().request_repaint();
        }
        if let Some(document) = close {
            self.request_close(document);
            ui.ctx().request_repaint();
        } else if let Some(document) = clicked {
            self.request_activate(document);
            ui.ctx().request_repaint();
        }
    }

    /// The question before closing a document with unsaved changes, over
    /// the whole window while it is asked: Save, Don't Save, Cancel.
    /// Escape or a click beside it cancels.
    pub(super) fn closing_dialog(&mut self, ui: &egui::Ui) {
        let Some(closing) = self.closing.clone() else {
            self.closing_hits.clear();
            return;
        };
        // One modal at a time: an error or export dialog first (a Save as
        // model… that failed says why), the question after it.
        if self.dialog_open() {
            self.closing_hits.clear();
            return;
        }
        let tokens = self.tokens;
        let more = self
            .tabs
            .iter()
            .filter(|tab| tab.asks_to_close && tab.document != closing.document)
            .count();
        let mut hits = Vec::new();
        let mut answer = None;
        let kit = crate::kit::Kit::panel(&tokens);
        let (doing, done) = if closing.quitting {
            ("quitting", "quit")
        } else {
            ("closing", "close")
        };
        let (title, body) = if closing.jig {
            // A Jig in use: its file stays as it is; the values set are
            // kept only as a model.
            (
                format!("Save the parameters of \"{}\" as a model?", closing.name),
                format!(
                    "The Jig itself is not changed. The values you set are lost if you {done} without saving them."
                ),
            )
        } else {
            (
                format!("Save the changes to \"{}\" before {doing}?", closing.name),
                format!("The changes are lost if you {done} without saving."),
            )
        };
        let dialog = crate::kit::Dialog::new(egui::Id::new("craft3d-closing"), &title)
            .width(tokens.dialog_width + 40.0)
            .closable(false);
        let shown = kit.dialog(ui.ctx(), dialog, |kit, ui| {
            kit.paragraph(ui, &body, true);
            if closing.quitting && more > 0 {
                let more = if more == 1 {
                    "1 more document has unsaved changes.".to_string()
                } else {
                    format!("{more} more documents have unsaved changes.")
                };
                kit.paragraph(ui, &more, true);
            }
            kit.dialog_footer(ui, |ui| {
                let label = |choice: CloseAnswer| match (closing.jig, choice) {
                    (true, CloseAnswer::Save) => "Save as model…",
                    (true, CloseAnswer::Discard) => "Close",
                    _ => choice.label(),
                };
                // Don't Save at the left; Cancel, then Save (amber) last.
                let mut add =
                    |ui: &mut egui::Ui, choice: CloseAnswer, button: crate::kit::Button<'_>| {
                        let response = kit.button(ui, button);
                        hits.push((choice, response.rect));
                        if response.clicked() {
                            answer = Some(choice);
                        }
                    };
                add(
                    ui,
                    CloseAnswer::Discard,
                    crate::kit::Button::quiet(label(CloseAnswer::Discard)),
                );
                kit.footer_right(ui, |ui| {
                    add(
                        ui,
                        CloseAnswer::Save,
                        crate::kit::Button::primary(label(CloseAnswer::Save)),
                    );
                    add(
                        ui,
                        CloseAnswer::Cancel,
                        crate::kit::Button::secondary(label(CloseAnswer::Cancel)),
                    );
                });
            });
        });
        // In the order of `CloseAnswer::ALL`.
        hits.sort_by_key(|(choice, _)| CloseAnswer::ALL.iter().position(|c| c == choice));
        self.closing_hits = hits;
        if answer.is_none() && shown.close {
            answer = Some(CloseAnswer::Cancel);
        }
        if let Some(answer) = answer {
            self.answer_closing(answer);
            self.closing_hits.clear();
            ui.ctx().request_repaint();
        }
    }
}
