//! The Variables tab (plan of 2026-11-02, boards 02 and 03): the
//! document's variables as a table, name, value or formula, result,
//! grouped as the document groups them (a public parameter's group, else
//! a prefix two or more share). A formula is in the info ink on a line of
//! its own under its name, its result at the right. A click on a value or
//! a formula edits it in place: Enter sets it (`variable.set`, one undo
//! step), Escape or a click elsewhere lets it go; what the command
//! refuses is said at the field. The document's units fold away at the
//! foot.

use super::{Editing, Filter, Shell, matches};
use crate::kit::{Button, Glyph, Kit};
use crate::shell::side_inset;
use craft3d_engine::VariableEntry;
use egui::{Rect, Sense, pos2, vec2};
use serde_json::json;

/// Most variables listed.
const MAX_LISTED: usize = 2000;

/// The groups of `variables` in the order they first appear; the
/// variables without a group come first, under no heading.
pub fn groups(variables: &[VariableEntry]) -> Vec<(String, Vec<&VariableEntry>)> {
    let mut out: Vec<(String, Vec<&VariableEntry>)> = vec![(String::new(), Vec::new())];
    for v in variables.iter().take(MAX_LISTED) {
        match out.iter_mut().find(|(g, _)| *g == v.group) {
            Some((_, list)) => list.push(v),
            None => out.push((v.group.clone(), vec![v])),
        }
    }
    out.retain(|(_, list)| !list.is_empty());
    out
}

impl Shell {
    /// The Variables tab.
    pub(super) fn variables_tab(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let sk = kit.skin;
        let t = kit.t;
        let inset = f32::from(side_inset(&tokens));
        let view = self.side.view.clone();
        egui::Panel::bottom("craft3d-timeline")
            .resizable(false)
            .show(ui, |ui| {
                ui.set_max_width((ui.available_width() - inset).max(40.0));
                self.timeline_ui(ui);
            });
        egui::Panel::bottom("craft3d-side-units")
            .resizable(false)
            // Clear of the split at the right, as the rows above (a
            // panel's own width does not shrink to a smaller max width).
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 0,
                right: side_inset(&tokens),
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| {
                kit.divider(ui);
                let mut open = self.state.side_panel.is_open("units");
                let note = view.as_ref().map_or_else(String::new, |v| {
                    format!(
                        "{} · {} · tolerance {}",
                        v.length_unit,
                        v.angle_unit,
                        tolerance_text(v.tolerance)
                    )
                });
                let header =
                    kit.fold_header(ui, &mut open, "Units", None, Some((&note, sk.text_dim)));
                self.side.hits.drawn.push(header.rect);
                self.side
                    .hits
                    .targets
                    .push(("units".to_string(), header.rect));
                if header.changed() {
                    self.state.side_panel.set_open("units", open);
                }
            });
        ui.scope(|ui| {
            ui.set_max_width((ui.available_width() - inset).max(40.0));
            self.side_filter(ui, Filter::Variables, "Filter variables");
            self.side_problem(ui);
        });
        let Some(view) = view else {
            return;
        };
        let filter = self.state.side_panel.variable_filter.clone();
        let shown: Vec<VariableEntry> = view
            .variables
            .iter()
            .filter(|v| matches(&v.name, &filter) || matches(&v.expr, &filter))
            .cloned()
            .collect();
        let editing = self.side.editing.clone();
        let focus = std::mem::take(&mut self.side.focus_pending);
        let errors = self.side.errors.clone();
        let mut start: Option<(String, String)> = None;
        let mut commit: Option<(String, String)> = None;
        let mut cancel = false;
        let mut fold: Option<(String, bool)> = None;
        let mut add = false;
        let mut rows_drawn = Vec::new();
        let jig = self.jig_in_use();
        egui::ScrollArea::vertical()
            .id_salt(("craft3d-side-variables", self.viewing))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 0,
                        right: side_inset(&tokens),
                        top: 0,
                        bottom: 0,
                    })
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let width = ui.available_width();
                        let value_x = (width * 0.36).clamp(64.0, 160.0);
                        // The table's head.
                        let (head, _) = ui.allocate_exact_size(vec2(width, t.label_row_height), Sense::hover());
                        let painter = ui.painter();
                        for (text, x, right) in [
                            ("Name", head.left() + 2.0, false),
                            ("Value or formula", head.left() + value_x, false),
                            ("Result", head.right() - 2.0, true),
                        ] {
                            let g = kit.galley(ui, text, kit.small_font(), sk.text_dim);
                            let x = if right { x - g.size().x } else { x };
                            painter.galley(pos2(x, head.center().y - g.size().y * 0.5), g, kit.c(sk.text_dim));
                        }
                        if shown.is_empty() {
                            let text = if view.variables.is_empty() {
                                "No variables yet."
                            } else {
                                "No variable holds the filter."
                            };
                            ui.label(egui::RichText::new(text).color(kit.c(sk.text_dim)));
                        }
                        for (group, list) in groups(&shown) {
                            if !group.is_empty() {
                                let mut open = !self.state.side_panel.folded.contains(&group);
                                let header = kit.fold_header(ui, &mut open, &group, None, None);
                                self.side.hits.folds.push((format!("group:{group}"), header.rect));
                                if header.changed() {
                                    fold = Some((group.clone(), open));
                                }
                                if !open {
                                    continue;
                                }
                            }
                            for v in list {
                                let key = format!("var:{}", v.name);
                                let this_editing = matches!(&editing, Some(Editing::Variable { name, .. }) if *name == v.name);
                                let lines = if v.formula && !this_editing { 2.0 } else { 1.0 };
                                let (rect, response) = ui.allocate_exact_size(
                                    vec2(width, kit.hit(t.row_height) * lines),
                                    Sense::click(),
                                );
                                rows_drawn.push((v.name.clone(), rect));
                                let painter = ui.painter();
                                if this_editing || response.hovered() {
                                    painter.rect_filled(rect, 0.0, kit.c(sk.hover));
                                }
                                let y = rect.top() + t.row_height * 0.5;
                                // The name, cut before the value's column.
                                let mut job = egui::text::LayoutJob::simple_singleline(
                                    v.name.clone(),
                                    kit.font(),
                                    kit.c(sk.text),
                                );
                                job.wrap = egui::text::TextWrapping::truncate_at_width((value_x - t.nav_icon_gap - 2.0).max(0.0));
                                let name = painter.layout_job(job);
                                let mut cut = name.elided;
                                painter.galley(pos2(rect.left() + 2.0, y - name.size().y * 0.5), name, kit.c(sk.text));
                                // The result at the right.
                                let (result, ink) = match (&v.value, &v.error) {
                                    (Some(value), _) => (value.clone(), sk.text_dim),
                                    (None, _) => ("—".to_string(), sk.failed),
                                };
                                let g = kit.galley(ui, result, kit.font(), ink);
                                let result_w = g.size().x;
                                if !this_editing {
                                    painter.galley(pos2(rect.right() - 2.0 - result_w, y - g.size().y * 0.5), g, kit.c(ink));
                                }
                                let value_rect = Rect::from_min_max(
                                    pos2(rect.left() + value_x, rect.top()),
                                    pos2(rect.right() - result_w - t.nav_icon_gap, rect.top() + t.row_height),
                                );
                                self.side.hits.values.push((v.name.clone(), value_rect));
                                if this_editing {
                                    let text = match &editing {
                                        Some(Editing::Variable { text, .. }) => text.clone(),
                                        _ => String::new(),
                                    };
                                    let mut text = text;
                                    let place = Rect::from_min_max(
                                        pos2(rect.left() + value_x - t.field_text_pad, rect.top()),
                                        pos2(rect.right() - 2.0, rect.top() + kit.hit(t.row_height)),
                                    );
                                    let field = ui
                                        .scope_builder(egui::UiBuilder::new().max_rect(place), |ui| {
                                            kit.number_field(ui, ("craft3d-side-var", &v.name), &mut text, "", place.width(), errors.contains_key(&key))
                                        })
                                        .inner;
                                    self.side.hits.drawn.push(field.rect);
                                    if focus {
                                        field.request_focus();
                                    }
                                    if field.changed() {
                                        self.side.editing = Some(Editing::Variable { name: v.name.clone(), text: text.clone() });
                                    }
                                    if field.lost_focus() {
                                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                            commit = Some((v.name.clone(), text.trim().to_string()));
                                        } else {
                                            cancel = true;
                                        }
                                    }
                                } else if v.formula {
                                    let mut job = egui::text::LayoutJob::simple_singleline(
                                        format!("= {}", v.expr),
                                        kit.font(),
                                        kit.c(sk.info_ink),
                                    );
                                    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - rect.left() - value_x - 2.0).max(0.0));
                                    let g = painter.layout_job(job);
                                    cut |= g.elided;
                                    let fy = rect.top() + t.row_height * 1.5;
                                    painter.galley(pos2(rect.left() + value_x, fy - g.size().y * 0.5), g, kit.c(sk.info_ink));
                                } else {
                                    let mut job = egui::text::LayoutJob::simple_singleline(
                                        v.expr.clone(),
                                        kit.font(),
                                        kit.c(sk.text),
                                    );
                                    job.wrap = egui::text::TextWrapping::truncate_at_width(value_rect.width().max(0.0));
                                    let g = painter.layout_job(job);
                                    cut |= g.elided;
                                    painter.galley(pos2(value_rect.left(), y - g.size().y * 0.5), g, kit.c(sk.text));
                                }
                                // A Jig in use changes only its parameters.
                                if response.clicked() && !this_editing && !jig {
                                    start = Some((v.name.clone(), v.expr.clone()));
                                }
                                // Cut to the panel: the whole on hover.
                                if cut {
                                    let whole = format!("{} = {}", v.name, v.expr);
                                    self.side.hits.cut.push((v.name.clone(), whole.clone()));
                                    response.on_hover_text(whole);
                                }
                                // Why it does not evaluate, or what the
                                // command refused, at the field.
                                let why = errors.get(&key).or(v.error.as_ref());
                                if let Some(why) = why {
                                    let label = ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(why)
                                                .color(kit.c(sk.failed))
                                                .size(t.secondary_font - 1.0),
                                        )
                                        .wrap(),
                                    );
                                    self.side.hits.drawn.push(label.rect);
                                }
                            }
                        }
                        // A variable being added: its name, its field.
                        if let Some(Editing::NewVariable { name, text }) = &editing {
                            let (rect, _) = ui.allocate_exact_size(
                                vec2(width, kit.hit(t.row_height)),
                                Sense::hover(),
                            );
                            let g = kit.galley(ui, name.clone(), kit.font(), sk.text);
                            ui.painter().galley(
                                pos2(rect.left() + 2.0, rect.center().y - g.size().y * 0.5),
                                g,
                                kit.c(sk.text),
                            );
                            let mut text = text.clone();
                            let key = format!("var:{name}");
                            let place = Rect::from_min_max(
                                pos2(rect.left() + value_x - t.field_text_pad, rect.top()),
                                pos2(rect.right() - 2.0, rect.bottom()),
                            );
                            let field = ui
                                .scope_builder(egui::UiBuilder::new().max_rect(place), |ui| {
                                    kit.number_field(ui, ("craft3d-side-var-new", name), &mut text, "", place.width(), errors.contains_key(&key))
                                })
                                .inner;
                            self.side.hits.values.push((name.clone(), place));
                            if focus {
                                field.request_focus();
                            }
                            if field.changed() {
                                self.side.editing = Some(Editing::NewVariable { name: name.clone(), text: text.clone() });
                            }
                            if field.lost_focus() {
                                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty() {
                                    commit = Some((name.clone(), text.trim().to_string()));
                                } else {
                                    cancel = true;
                                }
                            }
                        }
                        ui.add_space(t.nav_gap);
                        let button = kit.button(ui, Button::secondary("Add variable").icon(Glyph::Plus).enabled(!jig));
                        self.side.hits.targets.push(("add variable".to_string(), button.rect));
                        add = button.clicked();
                    });
            });
        self.side.hits.rows = rows_drawn;
        if let Some((group, open)) = fold {
            self.state.side_panel.folded.retain(|g| *g != group);
            if !open && self.state.side_panel.folded.len() < crate::state::MAX_UNFOLDED {
                self.state.side_panel.folded.push(group);
            }
        }
        if let Some((name, expr)) = commit {
            let key = format!("var:{name}");
            self.side_queue(
                "variable.set",
                json!({"name": name, "expr": expr}),
                Some(key),
            );
            self.side.editing = None;
        } else if cancel {
            self.side.editing = None;
        }
        if let Some((name, text)) = start {
            self.side.editing = Some(Editing::Variable { name, text });
            self.side.focus_pending = true;
        }
        if add {
            // Made when its value is given: one undo step.
            let taken = |n: &str| view.variables.iter().any(|v| v.name == n);
            let name = (1..=MAX_LISTED)
                .map(|k| format!("v{k}"))
                .find(|n| !taken(n));
            if let Some(name) = name {
                self.side.editing = Some(Editing::NewVariable {
                    name,
                    text: String::new(),
                });
                self.side.focus_pending = true;
            }
        }
    }
}

/// The tolerance as written: `1e-6`.
fn tolerance_text(t: f64) -> String {
    if !t.is_finite() || t <= 0.0 {
        return "?".to_string();
    }
    // Rust's exponent form is how the document writes it: `1e-6`.
    format!("{t:e}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(name: &str, group: &str) -> VariableEntry {
        VariableEntry {
            name: name.to_string(),
            expr: "1".to_string(),
            value: Some("1".to_string()),
            error: None,
            formula: false,
            group: group.to_string(),
            public: false,
        }
    }

    #[test]
    fn groups_keep_the_documents_order() {
        let vars = vec![
            v("a", ""),
            v("base_t", "Base"),
            v("hole_d", "Hole"),
            v("base_r", "Base"),
            v("b", ""),
        ];
        let g = groups(&vars);
        let names: Vec<(&str, Vec<&str>)> = g
            .iter()
            .map(|(g, l)| (g.as_str(), l.iter().map(|v| v.name.as_str()).collect()))
            .collect();
        assert_eq!(
            names,
            [
                ("", vec!["a", "b"]),
                ("Base", vec!["base_t", "base_r"]),
                ("Hole", vec!["hole_d"])
            ]
        );
        assert_eq!(tolerance_text(1e-6), "1e-6");
    }
}
