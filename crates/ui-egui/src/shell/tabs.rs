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
            });
        } else {
            self.doc_action = Some(DocAction::Close {
                document,
                discard: false,
            });
        }
    }

    /// The person's answer to the question before closing.
    pub fn answer_closing(&mut self, answer: CloseAnswer) {
        let Some(closing) = self.closing.take() else {
            return;
        };
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

    /// The tabs follow the session; a question about a document that is
    /// not there any more as it was asked about, or has nothing unsaved
    /// any more, is dropped.
    pub(super) fn sync_tabs(&mut self, session: &Session) {
        let active = session.document_status().map(|status| status.index);
        self.tabs = doc_tabs(&session.document_statuses(), active);
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
    pub(super) fn menu_documents(&mut self, ui: &mut egui::Ui) {
        let shortcut = ui.ctx().format_shortcut(&close_shortcut());
        let close = egui::Button::new("Close").shortcut_text(shortcut);
        let active = self
            .active_tab()
            .and_then(|at| self.tabs.get(at))
            .map(|tab| tab.document);
        if ui.add_enabled(active.is_some(), close).clicked() {
            if let Some(document) = active {
                self.request_close(document);
            }
            ui.close();
        }
        if self.tabs.is_empty() {
            return;
        }
        ui.separator();
        let mut chosen = None;
        for tab in &self.tabs {
            let mark = if tab.dirty {
                format!(" {UNSAVED_MARK}")
            } else {
                String::new()
            };
            let label = format!("{}{mark}", tab.name);
            if ui
                .add(egui::Button::new(label).selected(tab.active))
                .clicked()
            {
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
        let width = if fair.is_finite() {
            fair.clamp(tokens.tab_min_width, tokens.tab_max_width)
        } else {
            tokens.tab_min_width
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
                            response
                                .scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
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
                                egui::pos2(rect.left(), rect.bottom() - tokens.tab_accent_width),
                                rect.right_bottom(),
                            );
                            painter.rect_filled(line, 0.0, color(tokens.tab_accent));
                        }
                        // The unsaved mark, then the name, cut to the room
                        // left of the close button.
                        let mut left = rect.left() + 8.0;
                        if tab.dirty {
                            let mark = painter.layout_no_wrap(
                                UNSAVED_MARK.to_string(),
                                egui::FontId::proportional(20.0),
                                color(tokens.attention),
                            );
                            let size = mark.size();
                            painter.galley(
                                egui::pos2(left, rect.center().y - size.y * 0.5),
                                mark,
                                color(tokens.attention),
                            );
                            left += size.x + 4.0;
                        }
                        let room = (close_rect.left() - 4.0 - left).max(0.0);
                        let text = if tab.active {
                            tokens.chrome_text
                        } else {
                            tokens.chrome_text_dim
                        };
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
                        // The close button: dim until the pointer is on
                        // the tab, with a plate under the pointer.
                        if closer.hovered() {
                            painter.rect_filled(
                                close_rect,
                                egui::CornerRadius::same(tokens.radius),
                                color_alpha(tokens.chrome_text_dim, 60),
                            );
                        }
                        let tint = if closer.hovered() || response.hovered() || tab.active {
                            tokens.chrome_text
                        } else {
                            tokens.chrome_text_dim
                        };
                        painter.text(
                            close_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            CLOSE_MARK,
                            egui::FontId::proportional(15.0),
                            color(tint),
                        );
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
        let tokens = self.tokens;
        let mut hits = Vec::new();
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::new("craft3d-closing")).show(ui.ctx(), |ui| {
            ui.set_max_width(380.0);
            ui.label(
                egui::RichText::new(format!(
                    "Save the changes to \"{}\" before closing?",
                    closing.name
                ))
                .strong(),
            );
            ui.add_space(tokens.inset * 0.4);
            ui.colored_label(
                color(tokens.text_dim),
                "The changes are lost if you close without saving.",
            );
            ui.add_space(tokens.inset);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Right to left: Cancel at the right edge.
                for choice in CloseAnswer::ALL.iter().rev() {
                    let response = ui.button(choice.label());
                    hits.push((*choice, response.rect));
                    if response.clicked() {
                        answer = Some(*choice);
                    }
                }
            });
        });
        hits.reverse();
        self.closing_hits = hits;
        if answer.is_none() && modal.should_close() {
            answer = Some(CloseAnswer::Cancel);
        }
        if let Some(answer) = answer {
            self.answer_closing(answer);
            self.closing_hits.clear();
            ui.ctx().request_repaint();
        }
    }
}
