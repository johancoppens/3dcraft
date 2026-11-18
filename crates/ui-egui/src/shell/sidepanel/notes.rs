//! The side panel's Notes tab (`shell/notes.rs` for the view; the UX
//! round of 2026-11-18, N1 to N6): the notes pinned to the model in three
//! groups, each shown only when it holds notes: **For you** (open notes
//! whose last word is an agent's), **For the agent** (the other open
//! ones, under the sentence that sets an agent to work, with a copy
//! button) and **Done**, folded away. Each row: its pin as the view draws
//! it, its text, where it is in words (the feature that made its entity;
//! a place held near, gone or hidden said in plain words), who wrote it
//! and when, small. A click on a row chooses the note and flies to what
//! it is on; the chosen row shows its replies in order, a field to reply
//! in (Enter sends `note.reply`, Shift+Enter starts a line, Escape leaves
//! the field and keeps its text until another row is chosen) and the
//! links Done (with text in the field, the closing's reply) or Open
//! again, Edit (a note the person wrote) and Delete, each a command and
//! one undo step; a refusal shows under the row in a person's words, the
//! command's own as the hover. *Hide pins* hides or shows the pins in the
//! view; *Note on selection (N)* writes a note on what is selected, or
//! says why it cannot. Offered for a part with notes, and while one is
//! being written; a document without notes shows nothing. While a note
//! waits for the person the tab is never behind ⋯ ([`offer`]).

use super::super::Shell;
use super::agent::ago;
use crate::kit::{Button, Glyph, Kit};
use crate::shell::notes::{NoteRow, PinKind, paint_head, person_words};
use crate::state::SideTab;
use crate::theme::{Tokens, color};
use serde_json::json;

/// The key under which a note's changed text is run, so its field learns
/// whether it was kept.
const NOTE_EDIT: &str = "note edit";
/// The key under which the tab's acts on a note (a reply, Done, Open
/// again, Delete) are run, so a refusal shows under its row.
const NOTE_ACT: &str = "note act";
/// How long the line for the agent says it was copied, in seconds.
const COPIED_SECONDS: f64 = 3.0;

/// Puts the Notes tab among `offered` when `notes`: after Bodies, before
/// Agent; while a note `waiting` for the person, after the first tab, so
/// it is never behind ⋯ (N1).
pub fn offer(offered: &mut Vec<SideTab>, notes: bool, waiting: bool) {
    if !notes || offered.contains(&SideTab::Notes) {
        return;
    }
    let at = if waiting {
        offered.len().min(1)
    } else {
        offered
            .iter()
            .position(|t| *t == SideTab::Agent)
            .unwrap_or(offered.len())
    };
    offered.insert(at, SideTab::Notes);
}

/// What a row, or the tab, asks for.
enum Asked {
    Choose(u64),
    Edit(u64, String),
    Typing(u64, String),
    Save(u64, String),
    Cancel,
    /// Done (`true`, with the reply field's text when there is any) or
    /// Open again.
    Resolve(u64, bool, Option<String>),
    Delete(u64),
    Reply(u64, String),
    NoteOnSelection,
    Copy,
}

/// What every row is drawn with.
struct RowCtx<'a> {
    kit: &'a Kit,
    tokens: &'a Tokens,
    now: u64,
    selected: Option<u64>,
    editing: Option<&'a (u64, String)>,
    edit_error: Option<&'a str>,
    act_error: Option<&'a (u64, String, String)>,
    scroll_to: Option<u64>,
}

impl Shell {
    /// What became of the request queued under `key` (`side_queue`):
    /// `None` while it waits, else whether it was run or why it was
    /// refused (the reason taken from the panel's errors).
    pub(crate) fn side_outcome(&mut self, key: &str) -> Option<Result<(), String>> {
        if self
            .side
            .pending
            .iter()
            .any(|r| r.key.as_deref() == Some(key))
        {
            return None;
        }
        Some(match self.side.errors.remove(key) {
            Some(why) => Err(why),
            None => Ok(()),
        })
    }

    /// Sends the changed text of note `id` (`note.edit`, as the person);
    /// its field stays until the answer (`edit_sent`): closed when kept,
    /// open with the text and the reason when refused.
    fn save_note_edit(&mut self, id: u64, text: String) {
        self.notes.editing = Some((id, text.clone()));
        self.notes.edit_sent = true;
        self.notes.edit_error = None;
        self.side_queue(
            "note.edit",
            json!({"id": id, "text": text}),
            Some(NOTE_EDIT.to_string()),
        );
    }

    /// The note whose text is being changed in the Notes tab, its text
    /// there, and why the last change was refused.
    pub fn note_editing(&self) -> Option<(u64, &str, Option<&str>)> {
        self.notes
            .editing
            .as_ref()
            .map(|(id, text)| (*id, text.as_str(), self.notes.edit_error.as_deref()))
    }

    /// Types `text` into the field of the note being changed in the Notes
    /// tab (for tests and an agent showing the person around); `save`
    /// sends it as Save does.
    pub fn type_note_edit(&mut self, text: &str, save: bool) {
        let Some(id) = self.notes.editing.as_ref().map(|(id, _)| *id) else {
            return;
        };
        self.notes.editing = Some((id, text.to_string()));
        if save {
            self.save_note_edit(id, text.to_string());
        }
    }

    /// The reply being written on the chosen row: its note and its text.
    pub fn note_reply_field(&self) -> Option<(u64, &str)> {
        self.notes
            .reply
            .as_ref()
            .map(|(id, text)| (*id, text.as_str()))
    }

    /// Types `text` into the chosen row's reply field (for tests and an
    /// agent showing the person around); `send` sends it as Enter does.
    pub fn type_note_reply(&mut self, text: &str, send: bool) {
        let Some(id) = self.state.notes.selected else {
            return;
        };
        self.notes.reply = Some((id, text.to_string()));
        if send {
            self.send_note_reply(id, text.to_string());
        }
    }

    /// Why the tab's last act on a note was refused, under its row: the
    /// note, the person's words and the command's own.
    pub fn note_act_error(&self) -> Option<(u64, &str, &str)> {
        self.notes
            .act_error
            .as_ref()
            .map(|(id, words, why)| (*id, words.as_str(), why.as_str()))
    }

    /// Runs one of the tab's acts on note `id`; `carried`: it carries the
    /// reply field's text, emptied when it is taken.
    fn note_act(
        &mut self,
        command: &'static str,
        params: serde_json::Value,
        id: u64,
        carried: bool,
    ) {
        self.notes.act_error = None;
        self.notes.act = Some((id, carried));
        self.side_queue(command, params, Some(NOTE_ACT.to_string()));
    }

    /// Sends a reply to note `id` (`note.reply`, as the person), when it
    /// says something.
    fn send_note_reply(&mut self, id: u64, text: String) {
        if text.trim().is_empty() {
            return;
        }
        self.note_act("note.reply", json!({"id": id, "text": text}), id, true);
    }

    /// The sentence that sets an agent to work on the notes, naming the
    /// document (N2).
    pub fn notes_handover(&self) -> String {
        let name = self
            .tabs()
            .iter()
            .find(|t| t.active)
            .map_or_else(|| "this model".to_string(), |t| t.name.clone());
        format!(
            "Work through the open notes of {name}: change the model where each says, and reply on each."
        )
    }

    /// Whether the Notes tab is offered: the document has notes, or one is
    /// being written.
    pub(crate) fn notes_tab_offered(&self) -> bool {
        !self.notes.rows.is_empty() || self.notes.writing.is_some()
    }

    /// A tab's words in the bar: the Notes tab with how many are open.
    pub(crate) fn tab_words(&self, tab: SideTab) -> String {
        let open = self.notes.rows.iter().filter(|r| !r.done).count();
        if tab == SideTab::Notes && open > 0 {
            format!("{} {open}", tab.label())
        } else {
            tab.label().to_string()
        }
    }

    /// What became of the tab's acts and the changed text.
    fn notes_answers(&mut self) {
        // A changed text sent: the field closes when it was kept, and stays
        // with the text and the reason when it was refused.
        if self.notes.edit_sent {
            match self.side_outcome(NOTE_EDIT) {
                None => {}
                Some(Ok(())) => {
                    self.notes.edit_sent = false;
                    self.notes.editing = None;
                }
                Some(Err(why)) => {
                    self.notes.edit_sent = false;
                    self.notes.edit_error = Some(person_words(&why));
                }
            }
        }
        // An act: a reply taken empties its field; a refusal shows under
        // the row until the next act (N6).
        if let Some((id, carried)) = self.notes.act {
            match self.side_outcome(NOTE_ACT) {
                None => {}
                Some(Ok(())) => {
                    self.notes.act = None;
                    if carried && self.notes.reply.as_ref().is_some_and(|(at, _)| *at == id) {
                        self.notes.reply = Some((id, String::new()));
                    }
                }
                Some(Err(why)) => {
                    self.notes.act = None;
                    self.notes.act_error = Some((id, person_words(&why), why));
                }
            }
        }
    }

    /// The Notes tab.
    pub(super) fn notes_tab(&mut self, ui: &mut egui::Ui) {
        self.notes_answers();
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let s = kit.skin;
        let now = self.agents_now();
        let time = ui.input(|i| i.time);
        // Notes in the view without a pin of their own or near one.
        let unpinned = self.notes.in_view.saturating_sub(self.notes.drawn.1);
        let narrow = self.narrow();
        let rows = std::sync::Arc::clone(&self.notes.rows);
        let selected = self.state.notes.selected;
        let for_you: Vec<&NoteRow> = rows.iter().filter(|r| r.for_you()).collect();
        let for_agent: Vec<&NoteRow> = rows.iter().filter(|r| !r.done && !r.for_you()).collect();
        let done: Vec<&NoteRow> = rows.iter().filter(|r| r.done).collect();
        let refusal = self.note_refusal();
        let connected = self.agents.status.connected > 0 || !self.agents.status.clients.is_empty();
        let copied = self
            .notes
            .copied
            .map(|at| time - at)
            .filter(|ago| (0.0..COPIED_SECONDS).contains(ago));
        if let Some(ago) = copied {
            ui.ctx().request_repaint_after(
                std::time::Duration::try_from_secs_f64(COPIED_SECONDS - ago).unwrap_or_default(),
            );
        }
        let editing = self.notes.editing.clone();
        let edit_error = self.notes.edit_error.clone();
        let act_error = self.notes.act_error.clone();
        let scroll_to = self.notes.scroll_to.take();
        let mut edit_focus = self.notes.edit_focus;
        let mut reply = self.notes.reply.clone();
        let mut reply_focus = self.notes.reply_focus;
        let cx = RowCtx {
            kit: &kit,
            tokens: &tokens,
            now,
            selected,
            editing: editing.as_ref(),
            edit_error: edit_error.as_deref(),
            act_error: act_error.as_ref(),
            scroll_to,
        };
        let mut asked = None;
        let mut hide = self.state.notes.hide_markers;
        let mut done_open = self.state.notes.done_open;
        let inset = crate::shell::side_inset(&tokens);
        let mut hits = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("craft3d-notes-tab")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 0,
                        right: inset,
                        top: 0,
                        bottom: 0,
                    })
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = tokens.nav_spacing;
                        let heading = |ui: &mut egui::Ui, words: &str, count: usize| {
                            ui.label(
                                egui::RichText::new(format!("{words} {count}"))
                                    .font(kit.medium_font(ui))
                                    .color(color(s.text)),
                            )
                        };
                        // Note on selection (N): greyed with the reason as
                        // its hover while it cannot write (N5).
                        let link = |ui: &mut egui::Ui,
                                        hits: &mut Vec<(String, egui::Rect)>,
                                        asked: &mut Option<Asked>| {
                            let button = kit
                                .button(
                                    ui,
                                    Button::link("Note on selection (N)").enabled(refusal.is_none()),
                                )
                                .on_hover_text("Write a note on what is selected")
                                .on_disabled_hover_text(refusal.unwrap_or_default());
                            hits.push(("note on selection".to_string(), button.rect));
                            if button.clicked() {
                                *asked = Some(Asked::NoteOnSelection);
                            }
                        };
                        let first = if !for_you.is_empty() {
                            Some(("For you", for_you.len()))
                        } else if !for_agent.is_empty() {
                            Some(("For the agent", for_agent.len()))
                        } else {
                            None
                        };
                        // The first heading, with Hide pins at its right.
                        ui.horizontal(|ui| {
                            if let Some((words, count)) = first {
                                heading(ui, words, count);
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let words = if hide { "Show pins" } else { "Hide pins" };
                                let eye = kit
                                    .button(ui, Button::quiet(words).icon(Glyph::Eye))
                                    .on_hover_text("The notes' pins in the view");
                                hits.push(("notes eye".to_string(), eye.rect));
                                if eye.clicked() {
                                    hide = !hide;
                                }
                            });
                        });
                        // The link at the right of For the agent's heading,
                        // under the first heading when that is the one.
                        let own_line = for_you.is_empty() || for_agent.is_empty();
                        if own_line {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                link(ui, &mut hits, &mut asked);
                            });
                        }
                        if unpinned > 0 {
                            kit.paragraph(
                                ui,
                                &format!(
                                    "{unpinned} notes have no pin here. Zoom in or turn the view to see them."
                                ),
                                true,
                            );
                        }
                        if for_you.is_empty() && for_agent.is_empty() {
                            // No N without a keyboard: on a phone, the
                            // press and hold's list (review-pins 2b).
                            let how = if narrow {
                                "No open notes. Press and hold a face and choose Note here… to write one."
                            } else {
                                "No open notes. Select something and press N to write one."
                            };
                            kit.paragraph(ui, how, true);
                        }
                        for row in &for_you {
                            note_row(ui, &cx, row, (&mut edit_focus, &mut reply_focus), &mut reply, &mut asked, &mut hits);
                        }
                        if !for_agent.is_empty() {
                            if !for_you.is_empty() {
                                ui.add_space(tokens.nav_gap);
                                ui.horizontal(|ui| {
                                    heading(ui, "For the agent", for_agent.len());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| link(ui, &mut hits, &mut asked),
                                    );
                                });
                            }
                            // The sentence that sets an agent to work (N2).
                            ui.horizontal(|ui| {
                                let words = if copied.is_some() {
                                    "Copied · paste it in your agent's chat"
                                } else if connected {
                                    "Tell your agent: \"Work through the notes.\""
                                } else {
                                    "No agent is connected. Tell one: \"Work through the notes.\""
                                };
                                let copy = kit.icon_button(
                                    ui,
                                    Glyph::Copy,
                                    false,
                                    "Copy the sentence for your agent",
                                );
                                hits.push(("notes copy".to_string(), copy.rect));
                                if copy.clicked() {
                                    asked = Some(Asked::Copy);
                                }
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(words)
                                            .font(kit.small_font())
                                            .color(color(s.text_dim)),
                                    )
                                    .wrap()
                                    .selectable(false),
                                );
                            });
                            for row in &for_agent {
                                note_row(ui, &cx, row, (&mut edit_focus, &mut reply_focus), &mut reply, &mut asked, &mut hits);
                            }
                        }
                        if !done.is_empty() {
                            kit.fold_header(ui, &mut done_open, "Done", Some(done.len()), None);
                            if done_open {
                                for row in &done {
                                    note_row(ui, &cx, row, (&mut edit_focus, &mut reply_focus), &mut reply, &mut asked, &mut hits);
                                }
                            }
                        }
                    });
            });
        self.side.hits.targets.extend(hits);
        self.state.notes.hide_markers = hide;
        self.state.notes.done_open = done_open;
        self.notes.edit_focus = edit_focus;
        self.notes.reply_focus = reply_focus;
        self.notes.reply = reply;
        match asked {
            Some(Asked::Choose(id)) => self.choose_note(id, true),
            Some(Asked::Edit(id, text)) => {
                self.notes.editing = Some((id, text));
                self.notes.edit_focus = true;
                self.notes.edit_error = None;
            }
            Some(Asked::Typing(id, text)) => self.notes.editing = Some((id, text)),
            Some(Asked::Save(id, text)) => self.save_note_edit(id, text),
            Some(Asked::Cancel) => {
                self.notes.editing = None;
                self.notes.edit_error = None;
            }
            Some(Asked::Resolve(id, done, said)) => {
                let carried = said.is_some();
                let params = match said {
                    Some(text) => json!({"id": id, "done": true, "reply": text}),
                    None => json!({"id": id, "done": done}),
                };
                self.note_act("note.resolve", params, id, carried);
            }
            Some(Asked::Delete(id)) => {
                self.note_act("note.delete", json!({ "id": id }), id, false);
            }
            Some(Asked::Reply(id, text)) => self.send_note_reply(id, text),
            Some(Asked::NoteOnSelection) => self.note_on_selection(),
            Some(Asked::Copy) => {
                ui.ctx().copy_text(self.notes_handover());
                self.notes.copied = Some(time);
            }
            None => {}
        }
    }
}

/// A small dim line of the panel's.
fn small(ui: &mut egui::Ui, kit: &Kit, text: &str) -> egui::Response {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(kit.small_font())
                .color(color(kit.skin.text_dim)),
        )
        .wrap()
        .selectable(false),
    )
}

/// One note's row: its pin, text, where and who; the chosen one with its
/// replies, the field to reply in and its actions, or its text in a field
/// while it is changed (`editing`, with why the last change was refused).
#[allow(clippy::too_many_arguments)]
fn note_row(
    ui: &mut egui::Ui,
    cx: &RowCtx<'_>,
    row: &NoteRow,
    (focus, reply_focus): (&mut bool, &mut bool),
    reply: &mut Option<(u64, String)>,
    asked: &mut Option<Asked>,
    hits: &mut Vec<(String, egui::Rect)>,
) {
    let kit = cx.kit;
    let t = kit.t;
    let s = kit.skin;
    let chosen = cx.selected == Some(row.id);
    let editing = cx.editing.filter(|(id, _)| *id == row.id);
    let kind = PinKind::of(row);
    let gutter = t.nav_icon_size + t.nav_icon_gap;
    // The row senses a click before its buttons are laid out (on its rect
    // of the frame before), so its links and fields stay on top of it.
    let row_ui = egui::UiBuilder::new()
        .id_salt(("note", row.id))
        .layout(egui::Layout::left_to_right(egui::Align::Min))
        .sense(egui::Sense::click());
    let shown = ui.scope_builder(row_ui, |ui| {
        // Across the panel, so the whole width chooses and is marked.
        ui.set_min_width(ui.available_width());
        let (rail, _) =
            ui.allocate_exact_size(egui::vec2(gutter, t.row_height), egui::Sense::hover());
        let centre = egui::pos2(rail.left() + t.nav_icon_size * 0.5, rail.center().y);
        // Its pin as the view draws it (N4).
        paint_head(
            ui.painter(),
            cx.tokens,
            kind,
            row.id,
            centre,
            t.nav_icon_size * 0.48,
            255,
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            match editing {
                Some((_, text)) => {
                    let mut text = text.clone();
                    let field = ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .desired_rows(2)
                            .desired_width(ui.available_width())
                            .char_limit(craft3d_engine::MAX_NOTE_CHARS),
                    );
                    if *focus {
                        field.request_focus();
                        *focus = false;
                    }
                    if let Some(why) = cx.edit_error {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(why)
                                    .font(kit.small_font())
                                    .color(color(s.failed)),
                            )
                            .wrap()
                            .selectable(false),
                        );
                    }
                    *asked = Some(Asked::Typing(row.id, text.clone()));
                    ui.horizontal(|ui| {
                        if kit.button(ui, Button::link("Save")).clicked() {
                            *asked = Some(Asked::Save(row.id, text.clone()));
                        }
                        if kit.button(ui, Button::link("Cancel")).clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            *asked = Some(Asked::Cancel);
                        }
                    });
                }
                None => {
                    let ink = if row.done { s.text_dim } else { s.text };
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&row.text)
                                .font(kit.font())
                                .color(color(ink)),
                        )
                        .wrap()
                        .selectable(false),
                    );
                }
            }
            let mut line = format!("{} · {}", row.place, row.who());
            if row.time_ms > 0 {
                line.push_str(&format!(" · {}", ago(row.time_ms, cx.now)));
            }
            if let Some(by) = &row.closed_by {
                line.push_str(&format!(" · closed by {by}"));
            }
            let said = small(ui, kit, &line);
            if let Some(why) = &row.place_hover {
                said.on_hover_text(why);
            }
            // Its replies: all of them, in order, on the chosen row; the
            // last word on the others.
            let thread = row.thread();
            let shown_lines = if chosen {
                thread.as_slice()
            } else {
                thread
                    .get(thread.len().saturating_sub(1)..)
                    .unwrap_or_default()
            };
            for (who, text, time, _) in shown_lines {
                let mut said = format!("“{text}” · {who}");
                if *time > 0 {
                    said.push_str(&format!(" · {}", ago(*time, cx.now)));
                }
                let label = egui::Label::new(
                    egui::RichText::new(said)
                        .font(kit.font())
                        .italics()
                        .color(color(s.text_dim)),
                )
                .selectable(false);
                ui.add(if chosen {
                    label.wrap()
                } else {
                    label.truncate()
                });
            }
            if chosen && editing.is_none() {
                reply_field(ui, kit, row, reply_focus, reply, asked, hits);
                ui.horizontal(|ui| {
                    let words = if row.done { "Open again" } else { "Done" };
                    let resolve = kit.button(ui, Button::link(words));
                    hits.push((format!("note resolve:{}", row.id), resolve.rect));
                    if resolve.clicked() {
                        // Done with text in the field: the closing's
                        // reply.
                        let said = reply
                            .as_ref()
                            .filter(|(id, text)| *id == row.id && !text.trim().is_empty())
                            .map(|(_, text)| text.clone());
                        *asked = Some(if row.done {
                            Asked::Resolve(row.id, false, None)
                        } else {
                            Asked::Resolve(row.id, true, said)
                        });
                    }
                    if row.yours() {
                        let edit = kit.button(ui, Button::link("Edit"));
                        hits.push((format!("note edit:{}", row.id), edit.rect));
                        if edit.clicked() {
                            *asked = Some(Asked::Edit(row.id, row.text.clone()));
                        }
                    }
                    let delete = kit.button(ui, Button::link("Delete"));
                    hits.push((format!("note delete:{}", row.id), delete.rect));
                    if delete.clicked() {
                        *asked = Some(Asked::Delete(row.id));
                    }
                });
            }
            // Why the last act on it was refused, until the next act (N6).
            if let Some((_, words, why)) = cx.act_error.filter(|(id, ..)| *id == row.id) {
                let said = ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new(words)
                                .font(kit.small_font())
                                .color(color(s.failed)),
                        )
                        .wrap()
                        .selectable(false),
                    )
                    .on_hover_text(why);
                hits.push((format!("note refused:{}", row.id), said.rect));
            }
        });
    });
    let rect = shown.response.rect;
    if chosen {
        ui.painter().rect_filled(
            rect.expand(2.0),
            kit.radius(),
            crate::theme::color_alpha(s.selection, 40),
        );
    }
    if cx.scroll_to == Some(row.id) {
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
    }
    hits.push((format!("note:{}", row.id), rect));
    if shown.response.clicked() && editing.is_none() && asked.is_none() {
        *asked = Some(Asked::Choose(row.id));
    }
}

/// The chosen row's field to reply in (N1): Enter sends the reply,
/// Shift+Enter starts a new line, Escape leaves the field and keeps its
/// text until another row is chosen.
fn reply_field(
    ui: &mut egui::Ui,
    kit: &Kit,
    row: &NoteRow,
    focus: &mut bool,
    reply: &mut Option<(u64, String)>,
    asked: &mut Option<Asked>,
    hits: &mut Vec<(String, egui::Rect)>,
) {
    let id = egui::Id::new(("craft3d-note-reply", row.id));
    let mut text = reply
        .as_ref()
        .filter(|(at, _)| *at == row.id)
        .map(|(_, text)| text.clone())
        .unwrap_or_default();
    // Enter sends, taken before the field sees it; Shift+Enter is the
    // field's: a new line.
    let focused = ui.memory(|m| m.has_focus(id));
    let enter = focused
        && ui.input_mut(|i| {
            !i.modifiers.shift && i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
        });
    let field = ui.add(
        egui::TextEdit::multiline(&mut text)
            .id(id)
            .hint_text("Reply…")
            .font(kit.font())
            .return_key(egui::KeyboardShortcut::new(
                egui::Modifiers::SHIFT,
                egui::Key::Enter,
            ))
            .desired_rows(1)
            .desired_width(ui.available_width())
            .char_limit(craft3d_engine::MAX_NOTE_CHARS),
    );
    hits.push((format!("note reply field:{}", row.id), field.rect));
    if *focus {
        field.request_focus();
        *focus = false;
    }
    if field.changed() {
        *reply = Some((row.id, text.clone()));
    }
    if enter && !text.trim().is_empty() {
        *asked = Some(Asked::Reply(row.id, text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notes_tab_goes_before_agent_and_after_the_first_while_a_note_waits() {
        let mut tabs = vec![SideTab::Features, SideTab::Bodies, SideTab::Agent];
        offer(&mut tabs, false, false);
        assert_eq!(tabs.len(), 3);
        offer(&mut tabs, true, false);
        assert_eq!(
            tabs,
            vec![
                SideTab::Features,
                SideTab::Bodies,
                SideTab::Notes,
                SideTab::Agent
            ]
        );
        offer(&mut tabs, true, false);
        assert_eq!(tabs.len(), 4);
        // A note waits for the person: never behind ⋯ (N1).
        let mut tabs = vec![
            SideTab::Features,
            SideTab::Variables,
            SideTab::Bodies,
            SideTab::Agent,
        ];
        offer(&mut tabs, true, true);
        assert_eq!(tabs.get(1), Some(&SideTab::Notes));
        let mut none = Vec::new();
        offer(&mut none, true, true);
        assert_eq!(none, vec![SideTab::Notes]);
    }
}
