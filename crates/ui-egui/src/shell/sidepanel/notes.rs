//! The side panel's Notes tab (`shell/notes.rs` for the view): the notes
//! pinned to the model, an agent's open questions first, then the other
//! open notes, then the done ones folded away under them. Each row: its
//! number, its text, where it is in words (the feature that made its
//! entity; "by place" and "detached" said), who wrote it and when, small;
//! a done note with its reply. A click on a row chooses the note and flies
//! to what it is on; the chosen row offers Edit, Done (or Open again) and
//! Delete, each a command and one undo step. The eye at the top hides or
//! shows the markers in the view. Offered for a part with notes, and
//! while one is being written; a document without notes shows nothing.

use super::super::Shell;
use super::agent::ago;
use crate::kit::{Button, Glyph, Kit};
use crate::shell::notes::NoteRow;
use crate::state::SideTab;
use crate::theme::color;
use serde_json::json;

/// Puts the Notes tab among `offered` when `notes`: after Bodies, before
/// Agent.
pub fn offer(offered: &mut Vec<SideTab>, notes: bool) {
    if !notes || offered.contains(&SideTab::Notes) {
        return;
    }
    let at = offered
        .iter()
        .position(|t| *t == SideTab::Agent)
        .unwrap_or(offered.len());
    offered.insert(at, SideTab::Notes);
}

/// What a row asks for.
enum Asked {
    Choose(u64),
    Edit(u64, String),
    Typing(u64, String),
    Save(u64, String),
    Cancel,
    Resolve(u64, bool),
    Delete(u64),
}

impl Shell {
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

    /// The Notes tab.
    pub(super) fn notes_tab(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let s = kit.skin;
        let now = self.agents_now();
        let rows = std::sync::Arc::clone(&self.notes.rows);
        let selected = self.state.notes.selected;
        let mut open: Vec<&NoteRow> = rows.iter().filter(|r| !r.done).collect();
        // An agent's questions first.
        open.sort_by_key(|r| (!r.question(), r.id));
        let done: Vec<&NoteRow> = rows.iter().filter(|r| r.done).collect();
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
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("Open {}", open.len()))
                                    .font(kit.medium_font(ui))
                                    .color(color(s.text)),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let words = if hide { "Show in view" } else { "Hide in view" };
                                let eye = kit
                                    .button(ui, Button::quiet(words).icon(Glyph::Eye))
                                    .on_hover_text("The notes' markers in the view");
                                hits.push(("notes eye".to_string(), eye.rect));
                                if eye.clicked() {
                                    hide = !hide;
                                }
                            });
                        });
                        if open.is_empty() {
                            kit.paragraph(ui, "No open notes.", true);
                        }
                        for row in &open {
                            let editing = self.notes.editing.clone().filter(|(id, _)| *id == row.id);
                            note_row(ui, &kit, row, now, selected, editing, &mut self.notes.edit_focus, &mut asked, &mut hits);
                        }
                        if !done.is_empty() {
                            kit.fold_header(ui, &mut done_open, "Done", Some(done.len()), None);
                            if done_open {
                                for row in &done {
                                    let editing =
                                        self.notes.editing.clone().filter(|(id, _)| *id == row.id);
                                    note_row(ui, &kit, row, now, selected, editing, &mut self.notes.edit_focus, &mut asked, &mut hits);
                                }
                            }
                        }
                        kit.paragraph(
                            ui,
                            "N writes a note on what is selected. A note is for whoever works on the model next; an agent asked to work through the notes changes the model where they say and replies on each.",
                            true,
                        );
                    });
            });
        self.side.hits.targets.extend(hits);
        self.state.notes.hide_markers = hide;
        self.state.notes.done_open = done_open;
        match asked {
            Some(Asked::Choose(id)) => self.choose_note(id, true),
            Some(Asked::Edit(id, text)) => {
                self.notes.editing = Some((id, text));
                self.notes.edit_focus = true;
            }
            Some(Asked::Typing(id, text)) => self.notes.editing = Some((id, text)),
            Some(Asked::Save(id, text)) => {
                self.notes.editing = None;
                self.side_queue("note.edit", json!({"id": id, "text": text}), None);
            }
            Some(Asked::Cancel) => self.notes.editing = None,
            Some(Asked::Resolve(id, done)) => {
                self.side_queue("note.resolve", json!({"id": id, "done": done}), None);
            }
            Some(Asked::Delete(id)) => {
                if self.state.notes.selected == Some(id) {
                    self.state.notes.selected = None;
                }
                self.side_queue("note.delete", json!({ "id": id }), None);
            }
            None => {}
        }
    }
}

/// One note's row: its number, text, where and who; the chosen one with
/// its actions, or its text in a field while it is changed.
#[allow(clippy::too_many_arguments)]
fn note_row(
    ui: &mut egui::Ui,
    kit: &Kit,
    row: &NoteRow,
    now: u64,
    selected: Option<u64>,
    editing: Option<(u64, String)>,
    focus: &mut bool,
    asked: &mut Option<Asked>,
    hits: &mut Vec<(String, egui::Rect)>,
) {
    let t = kit.t;
    let s = kit.skin;
    let chosen = selected == Some(row.id);
    let gutter = t.nav_icon_size + t.nav_icon_gap;
    let shown = ui.horizontal_top(|ui| {
        let (rail, _) =
            ui.allocate_exact_size(egui::vec2(gutter, t.row_height), egui::Sense::hover());
        let centre = egui::pos2(rail.left() + t.nav_icon_size * 0.5, rail.center().y);
        let r = t.nav_icon_size * 0.42;
        let ink = if row.done { s.ok } else { t.accent };
        if row.done {
            ui.painter()
                .circle_stroke(centre, r, egui::Stroke::new(1.2, color(ink)));
            ui.painter().text(
                centre,
                egui::Align2::CENTER_CENTER,
                "✓",
                egui::FontId::proportional(r * 1.5),
                color(ink),
            );
        } else if row.question() {
            ui.painter()
                .circle_stroke(centre, r, egui::Stroke::new(1.5, color(ink)));
            ui.painter().text(
                centre,
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(r * 1.5),
                color(ink),
            );
        } else {
            ui.painter().circle_filled(centre, r, color(ink));
            ui.painter().text(
                centre,
                egui::Align2::CENTER_CENTER,
                row.id.to_string(),
                egui::FontId::proportional(r * 1.3),
                color(s.fill),
            );
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            match &editing {
                Some((_, text)) => {
                    let mut text = text.clone();
                    let field = ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .desired_rows(2)
                            .desired_width(ui.available_width()),
                    );
                    if *focus {
                        field.request_focus();
                        *focus = false;
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
                        .wrap(),
                    );
                }
            }
            let mut small = format!("{} · {}", row.place, row.who());
            if row.time_ms > 0 {
                small.push_str(&format!(" · {}", ago(row.time_ms, now)));
            }
            if let Some(by) = &row.closed_by {
                small.push_str(&format!(" · done by {by}"));
            }
            ui.label(
                egui::RichText::new(small)
                    .font(kit.small_font())
                    .color(color(s.text_dim)),
            );
            if let Some(reply) = &row.reply {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("“{reply}”"))
                            .font(kit.font())
                            .italics()
                            .color(color(s.text_dim)),
                    )
                    .wrap(),
                );
            }
            if chosen && editing.is_none() {
                ui.horizontal(|ui| {
                    let edit = kit.button(ui, Button::link("Edit"));
                    hits.push((format!("note edit:{}", row.id), edit.rect));
                    if edit.clicked() {
                        *asked = Some(Asked::Edit(row.id, row.text.clone()));
                    }
                    let words = if row.done { "Open again" } else { "Done" };
                    let resolve = kit.button(ui, Button::link(words));
                    hits.push((format!("note resolve:{}", row.id), resolve.rect));
                    if resolve.clicked() {
                        *asked = Some(Asked::Resolve(row.id, !row.done));
                    }
                    let delete = kit.button(ui, Button::link("Delete"));
                    hits.push((format!("note delete:{}", row.id), delete.rect));
                    if delete.clicked() {
                        *asked = Some(Asked::Delete(row.id));
                    }
                });
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
    let click = ui.interact(rect, ui.id().with(("note", row.id)), egui::Sense::click());
    hits.push((format!("note:{}", row.id), rect));
    if click.clicked() && editing.is_none() && asked.is_none() {
        *asked = Some(Asked::Choose(row.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notes_tab_goes_before_agent_and_only_when_offered() {
        let mut tabs = vec![SideTab::Features, SideTab::Bodies, SideTab::Agent];
        offer(&mut tabs, false);
        assert_eq!(tabs.len(), 3);
        offer(&mut tabs, true);
        assert_eq!(
            tabs,
            vec![
                SideTab::Features,
                SideTab::Bodies,
                SideTab::Notes,
                SideTab::Agent
            ]
        );
        offer(&mut tabs, true);
        assert_eq!(tabs.len(), 4);
    }
}
