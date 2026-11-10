//! The Bodies tab (plan of 2026-11-02, board 04): a row per result body
//! (its colour, its name, its mass or volume, its eye); the body chosen
//! opens into a card (material, density, colour, volume, mass, area, box,
//! topology, validity), all measured by `document.inspect`, never typed.
//! The material is chosen from the presets (`material.preset`, one undo
//! step). The bodies booleans used up fold into one quiet line, shown
//! only on asking (*show consumed*). Totals at the foot: how many bodies,
//! their volume and mass.

use super::features::{Dot, status_dot};
use super::props::{mass_text, volume_text};
use super::{Filter, Shell, grouped, matches};
use crate::kit::{Button, Kit, ValueStyle};
use crate::shell::side_inset;
use egui::{Rect, Sense, pos2, vec2};
use serde_json::{Value, json};

/// Most bodies listed.
const MAX_LISTED: usize = 2000;

/// A material preset as the card offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    /// `aluminium-brushed`.
    pub id: String,
    /// `Aluminium, brushed`.
    pub label: String,
    /// The material's name (`Aluminium`).
    pub material: String,
}

/// The presets of `material.presets`.
pub(crate) fn presets_of(value: &Value) -> Vec<Preset> {
    value
        .get("presets")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|p| {
                    Some(Preset {
                        id: p.get("preset")?.as_str()?.to_string(),
                        label: p.get("label")?.as_str()?.to_string(),
                        material: p
                            .get("material")
                            .and_then(|m| m.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .take(64)
                .collect()
        })
        .unwrap_or_default()
}

/// A body as the tab lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyLine {
    /// Its feature, by id.
    pub feature: String,
    pub name: String,
    /// Its own colour (`#d4d6da`), if it has one.
    pub color: Option<String>,
    /// How many colours of their own its faces carry (`faceColors` of
    /// `document.inspect`): what *Clear face colours* takes away.
    pub face_colors: usize,
    /// In the document's length unit cubed; kilograms.
    pub volume: Option<f64>,
    pub mass: Option<f64>,
    pub area: Option<f64>,
    /// Its box's size.
    pub size: Option<[f64; 3]>,
    pub material: Option<String>,
    /// kg/m³.
    pub density: Option<f64>,
    pub faces: usize,
    pub edges: usize,
    pub solid: bool,
    pub valid: bool,
    /// Used up by a later feature (a boolean's tool): only with *show
    /// consumed*.
    pub used_by: Option<String>,
}

/// The totals at the foot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BodyTotals {
    pub count: usize,
    pub volume: f64,
    /// `None` when a body has no mass (no material).
    pub mass: Option<f64>,
}

/// The bodies of `document.inspect` (with measures), then the bodies
/// later features used up.
pub fn body_lines(inspect: &Value) -> (Vec<BodyLine>, Vec<BodyLine>) {
    let f64_of =
        |v: &Value, key: &str| v.get(key).and_then(Value::as_f64).filter(|x| x.is_finite());
    let bodies: Vec<BodyLine> = inspect
        .get("bodies")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(MAX_LISTED)
                .map(|b| {
                    let size = b.get("bbox").and_then(|bb| {
                        let lo = bb.get(0)?.as_array()?;
                        let hi = bb.get(1)?.as_array()?;
                        let c = |a: &Vec<Value>, k: usize| a.get(k).and_then(Value::as_f64);
                        Some([
                            c(hi, 0)? - c(lo, 0)?,
                            c(hi, 1)? - c(lo, 1)?,
                            c(hi, 2)? - c(lo, 2)?,
                        ])
                    });
                    BodyLine {
                        feature: b
                            .get("feature")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        name: b
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        color: b
                            .get("appearance")
                            .and_then(|a| a.get("color"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        face_colors: b.get("faceColors").and_then(Value::as_u64).unwrap_or(0)
                            as usize,
                        volume: f64_of(b, "volume"),
                        mass: f64_of(b, "mass"),
                        area: f64_of(b, "area"),
                        size,
                        material: b
                            .get("material")
                            .and_then(|m| m.get("name"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        density: b.get("material").and_then(|m| f64_of(m, "density")),
                        faces: b.get("faces").and_then(Value::as_u64).unwrap_or(0) as usize,
                        edges: b.get("edges").and_then(Value::as_u64).unwrap_or(0) as usize,
                        solid: b.get("kind").and_then(Value::as_str) == Some("solid"),
                        valid: b.get("valid").and_then(Value::as_bool).unwrap_or(false),
                        used_by: None,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let consumed: Vec<BodyLine> = inspect
        .get("features")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|f| {
                    let by = f.get("bodyUsedBy")?.as_str()?;
                    Some(BodyLine {
                        feature: f
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        name: f
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        color: None,
                        face_colors: 0,
                        volume: None,
                        mass: None,
                        area: None,
                        size: None,
                        material: None,
                        density: None,
                        faces: 0,
                        edges: 0,
                        solid: true,
                        valid: true,
                        used_by: Some(by.to_string()),
                    })
                })
                .take(MAX_LISTED)
                .collect()
        })
        .unwrap_or_default();
    (bodies, consumed)
}

/// The totals of `bodies`.
pub fn totals(bodies: &[BodyLine]) -> BodyTotals {
    BodyTotals {
        count: bodies.len(),
        volume: bodies.iter().filter_map(|b| b.volume).sum(),
        mass: bodies.iter().map(|b| b.mass).sum::<Option<f64>>(),
    }
}

/// `#rrggbb` as a colour.
fn hex(text: &str) -> Option<[u8; 3]> {
    let t = text.strip_prefix('#')?;
    if t.len() != 6 {
        return None;
    }
    let byte = |k: usize| u8::from_str_radix(t.get(k..k + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

impl Shell {
    /// The bodies as the tab lists them (result bodies, then those used
    /// up), and their totals; empty until the tab was shown.
    pub fn side_bodies(&self) -> (Vec<BodyLine>, Vec<BodyLine>, BodyTotals) {
        let (bodies, consumed) = self
            .side
            .bodies
            .as_deref()
            .map(body_lines)
            .unwrap_or_default();
        let totals = totals(&bodies);
        (bodies, consumed, totals)
    }

    /// The Bodies tab.
    pub(super) fn bodies_tab(&mut self, ui: &mut egui::Ui) {
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let sk = kit.skin;
        let t = kit.t;
        let inset = f32::from(side_inset(&tokens));
        let (bodies, consumed, totals) = self.side_bodies();
        let unit = self
            .shown_status
            .as_ref()
            .map_or("mm", |s| s.length_unit)
            .to_string();
        egui::Panel::bottom("craft3d-timeline")
            .resizable(false)
            .show(ui, |ui| {
                ui.set_max_width((ui.available_width() - inset).max(40.0));
                self.timeline_ui(ui);
            });
        egui::Panel::bottom("craft3d-side-totals")
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
                let (rect, _) = ui
                    .allocate_exact_size(vec2(ui.available_width(), t.row_height), Sense::hover());
                self.side.hits.drawn.push(rect);
                let count = if totals.count == 1 {
                    "1 body".to_string()
                } else {
                    format!("{} bodies", totals.count)
                };
                let g = kit.galley(ui, count, kit.medium_font(ui), sk.text);
                ui.painter().galley(
                    pos2(rect.left() + 2.0, rect.center().y - g.size().y * 0.5),
                    g,
                    kit.c(sk.text),
                );
                let mut right = volume_text(totals.volume, &unit);
                if let Some(m) = totals.mass {
                    right = format!("{right} · {}", mass_text(m));
                }
                let g = kit.galley(ui, right, kit.font(), sk.text_dim);
                ui.painter().galley(
                    pos2(
                        rect.right() - 2.0 - g.size().x,
                        rect.center().y - g.size().y * 0.5,
                    ),
                    g,
                    kit.c(sk.text_dim),
                );
            });
        ui.scope(|ui| {
            ui.set_max_width((ui.available_width() - inset).max(40.0));
            self.side_filter(ui, Filter::Bodies, "Filter bodies");
            self.side_problem(ui);
        });
        if self.side.bodies.is_none() {
            if self.side.problem.is_some() {
                return;
            }
            let text = "The bodies are measured where they are built.";
            ui.label(egui::RichText::new(text).color(kit.c(sk.text_dim)));
            return;
        }
        let filter = self.state.side_panel.body_filter.clone();
        let chosen = self.state.side_panel.body.clone();
        let hidden_ids: Vec<String> = self
            .rows
            .iter()
            .filter(|r| r.hidden)
            .map(|r| r.id.clone())
            .collect();
        let in_use = self.jig_in_use();
        let presets = self.side.presets.clone();
        let mut pick: Option<String> = None;
        let mut eye: Option<(String, bool)> = None;
        let mut preset: Option<(String, Option<String>)> = None;
        let mut show_consumed = self.state.side_panel.show_consumed;
        let mut rows_drawn = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt(("craft3d-side-bodies", self.viewing))
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
                        for body in bodies.iter().filter(|b| {
                            matches(&b.name, &filter)
                                || b.material.as_deref().is_some_and(|m| matches(m, &filter))
                        }) {
                            let selected = chosen.as_deref() == Some(body.feature.as_str());
                            let hidden = hidden_ids.contains(&body.feature);
                            let (rect, response) = ui.allocate_exact_size(
                                vec2(width, kit.hit(t.row_height + 4.0)),
                                Sense::click(),
                            );
                            rows_drawn.push((body.feature.clone(), rect));
                            let painter = ui.painter();
                            if selected {
                                painter.rect_filled(rect, 0.0, kit.c(sk.selection));
                            } else if response.hovered() {
                                painter.rect_filled(rect, kit.radius(), kit.c(sk.hover));
                            }
                            let side = t.nav_icon_size;
                            let swatch = Rect::from_center_size(
                                pos2(
                                    rect.left() + t.nav_spacing * 4.0 + side * 0.5,
                                    rect.center().y,
                                ),
                                vec2(side, side),
                            );
                            let fill = body.color.as_deref().and_then(hex).unwrap_or(t.body);
                            painter.rect(
                                swatch,
                                egui::CornerRadius::same(3),
                                kit.c(fill),
                                egui::Stroke::new(1.0, kit.c(sk.field_stroke)),
                                egui::StrokeKind::Inside,
                            );
                            // The eye at the right, on hover or when hidden.
                            let eye_rect = Rect::from_center_size(
                                pos2(
                                    rect.right() - t.nav_spacing * 4.0 - side * 0.5,
                                    rect.center().y,
                                ),
                                vec2(side, side),
                            );
                            let show_eye = !in_use && (response.hovered() || hidden || selected);
                            if show_eye {
                                let ink = if hidden { sk.faint } else { sk.text_dim };
                                crate::kit::glyphs::paint(
                                    painter,
                                    crate::kit::Glyph::Eye,
                                    eye_rect,
                                    kit.c(ink),
                                    &t,
                                );
                                if hidden {
                                    painter.line_segment(
                                        [eye_rect.left_bottom(), eye_rect.right_top()],
                                        egui::Stroke::new(t.icon_stroke, kit.c(ink)),
                                    );
                                }
                                self.side.hits.eyes.push((body.feature.clone(), eye_rect));
                            }
                            let value = match (body.mass, body.volume) {
                                (Some(m), _) => mass_text(m).replace(".0 g", " g"),
                                (None, Some(v)) => volume_text(v, &unit),
                                _ => String::new(),
                            };
                            let value_ink = if selected { sk.text } else { sk.text_dim };
                            let g = kit.galley(ui, value, kit.font(), value_ink);
                            let value_right = eye_rect.left() - t.nav_icon_gap;
                            let value_w = g.size().x;
                            painter.galley(
                                pos2(value_right - value_w, rect.center().y - g.size().y * 0.5),
                                g,
                                kit.c(value_ink),
                            );
                            let x = swatch.right() + t.nav_icon_gap + 2.0;
                            let font = if selected {
                                kit.medium_font(ui)
                            } else {
                                kit.font()
                            };
                            let mut job = egui::text::LayoutJob::simple_singleline(
                                body.name.clone(),
                                font,
                                kit.c(sk.text),
                            );
                            job.wrap = egui::text::TextWrapping::truncate_at_width(
                                (value_right - value_w - t.nav_icon_gap - x).max(0.0),
                            );
                            let name = painter.layout_job(job);
                            painter.galley(
                                pos2(x, rect.center().y - name.size().y * 0.5),
                                name,
                                kit.c(sk.text),
                            );
                            if response.clicked() {
                                let pointer = response.interact_pointer_pos();
                                if show_eye
                                    && pointer.is_some_and(|p| eye_rect.expand(2.0).contains(p))
                                {
                                    eye = Some((body.feature.clone(), hidden));
                                } else {
                                    pick = Some(body.feature.clone());
                                }
                            }
                            if selected
                                && let Some(change) =
                                    self.body_card(ui, &kit, body, &unit, &presets, in_use)
                            {
                                preset = Some((body.feature.clone(), change));
                            }
                        }
                        if !consumed.is_empty() {
                            kit.divider(ui);
                            let note = format!("{} used up", consumed.len());
                            let header = kit.fold_header(
                                ui,
                                &mut show_consumed,
                                "Tool bodies",
                                None,
                                Some((&note, sk.text_dim)),
                            );
                            self.side
                                .hits
                                .targets
                                .push(("show consumed".to_string(), header.rect));
                            if show_consumed {
                                for body in &consumed {
                                    let (rect, _) = ui.allocate_exact_size(
                                        vec2(width, t.row_height),
                                        Sense::hover(),
                                    );
                                    rows_drawn.push((body.feature.clone(), rect));
                                    let g =
                                        kit.galley(ui, body.name.clone(), kit.font(), sk.text_dim);
                                    ui.painter().galley(
                                        pos2(
                                            rect.left() + t.nav_icon_size * 1.5,
                                            rect.center().y - g.size().y * 0.5,
                                        ),
                                        g,
                                        kit.c(sk.text_dim),
                                    );
                                    let by = self
                                        .rows
                                        .iter()
                                        .find(|r| Some(&r.id) == body.used_by.as_ref())
                                        .map_or_else(
                                            || body.used_by.clone().unwrap_or_default(),
                                            |r| r.name.clone(),
                                        );
                                    let g =
                                        kit.galley(ui, format!("into {by}"), kit.font(), sk.faint);
                                    ui.painter().galley(
                                        pos2(
                                            rect.right() - 2.0 - g.size().x,
                                            rect.center().y - g.size().y * 0.5,
                                        ),
                                        g,
                                        kit.c(sk.faint),
                                    );
                                }
                            }
                        }
                    });
            });
        self.side.hits.rows = rows_drawn;
        self.state.side_panel.show_consumed = show_consumed;
        if let Some((id, hidden)) = eye {
            self.toggle_visible(id, hidden);
        }
        if let Some(id) = pick {
            if chosen.as_deref() == Some(id.as_str()) {
                self.state.side_panel.body = None;
            } else {
                self.state.side_panel.body = Some(id.clone());
                self.queue_select(json!({"items": [{"kind": "body", "feature": id}]}));
            }
        }
        if let Some((body, change)) = preset {
            match change {
                Some(p) => {
                    self.side_queue("material.preset", json!({"body": body, "preset": p}), None)
                }
                None => {
                    self.side_queue("body.material", json!({"body": body, "clear": true}), None)
                }
            }
        }
    }

    /// The card of the body chosen. Returns a material asked for: a
    /// preset's id, or `None` inside to clear it.
    fn body_card(
        &mut self,
        ui: &mut egui::Ui,
        kit: &Kit,
        body: &super::BodyLine,
        unit: &str,
        presets: &[Preset],
        in_use: bool,
    ) -> Option<Option<String>> {
        let t = kit.t;
        let sk = kit.skin;
        let mut asked = None;
        let mut clear_faces = false;
        ui.add_space(t.nav_spacing * 2.0);
        let label_w = (ui.available_width() * 0.32).clamp(56.0, 110.0);
        let pad = t.nav_icon_size * 1.5;
        ui.horizontal(|ui| {
            ui.add_space(pad);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                // The material: a preset, or none.
                ui.horizontal(|ui| {
                    let (r, _) =
                        ui.allocate_exact_size(vec2(label_w, t.control_height), Sense::hover());
                    let g = kit.galley(ui, "Material", kit.font(), sk.text_dim);
                    ui.painter().galley(
                        pos2(r.left(), r.center().y - g.size().y * 0.5),
                        g,
                        kit.c(sk.text_dim),
                    );
                    let mut options: Vec<&str> = vec!["None"];
                    options.extend(presets.iter().map(|p| p.label.as_str()));
                    let current = body
                        .material
                        .as_deref()
                        .and_then(|m| presets.iter().position(|p| p.material == m).map(|k| k + 1));
                    let mut chosen = current.unwrap_or(0);
                    if body.material.is_some() && current.is_none() {
                        // A material of its own: shown by name.
                        let g = kit.galley(
                            ui,
                            body.material.clone().unwrap_or_default(),
                            kit.font(),
                            sk.text,
                        );
                        let (r, _) = ui.allocate_exact_size(
                            vec2(ui.available_width(), t.control_height),
                            Sense::hover(),
                        );
                        ui.painter().galley(
                            pos2(r.left(), r.center().y - g.size().y * 0.5),
                            g,
                            kit.c(sk.text),
                        );
                    } else if !in_use {
                        let width = ui.available_width();
                        let drop = kit.dropdown(
                            ui,
                            ("craft3d-side-material", &body.feature),
                            &mut chosen,
                            &options,
                            width,
                        );
                        self.side
                            .hits
                            .targets
                            .push(("material".to_string(), drop.rect));
                        if drop.changed() {
                            asked = Some(presets.get(chosen.wrapping_sub(1)).map(|p| p.id.clone()));
                        }
                    }
                });
                let mut row = |label: &str, value: String| {
                    let r = kit.prop_row(ui, label, &value, ValueStyle::Plain, label_w);
                    self.side.hits.drawn.push(r.rect);
                };
                if let Some(d) = body.density {
                    row("Density", format!("{} g/cm³", grouped(d / 1000.0, 2)));
                }
                if let Some(c) = &body.color {
                    row("Colour", c.clone());
                }
                if let Some(v) = body.volume {
                    row("Volume", volume_text(v, unit));
                }
                row(
                    "Mass",
                    body.mass
                        .map_or_else(|| "no material".to_string(), mass_text),
                );
                if let Some(a) = body.area {
                    row("Area", format!("{} {unit}²", grouped(a, 0)));
                }
                if let Some([x, y, z]) = body.size {
                    let n = craft3d_engine::history_view::number;
                    row(
                        "Box",
                        format!(
                            "{} × {} × {} {unit}",
                            n((x * 10.0).round() / 10.0),
                            n((y * 10.0).round() / 10.0),
                            n((z * 10.0).round() / 10.0)
                        ),
                    );
                }
                row(
                    "Topology",
                    format!("{} faces · {} edges", body.faces, body.edges),
                );
                // Valid: the dot carries its tick.
                let (r, _) = ui.allocate_exact_size(
                    vec2(ui.available_width(), t.label_row_height),
                    Sense::hover(),
                );
                let g = kit.galley(ui, "Check", kit.font(), sk.text_dim);
                ui.painter().galley(
                    pos2(r.left(), r.center().y - g.size().y * 0.5),
                    g,
                    kit.c(sk.text_dim),
                );
                let rr = t.nav_icon_size * 0.42;
                let dot = if body.valid { Dot::Ok } else { Dot::Failed };
                status_dot(
                    ui.painter(),
                    kit,
                    pos2(r.left() + label_w + rr, r.center().y),
                    rr,
                    dot,
                );
                let text = match (body.valid, body.solid) {
                    (true, true) => "Valid closed solid",
                    (true, false) => "Valid sheet",
                    (false, _) => "Not valid",
                };
                let g = kit.galley(ui, text, kit.font(), sk.text);
                ui.painter().galley(
                    pos2(
                        r.left() + label_w + 2.0 * rr + t.nav_icon_gap,
                        r.center().y - g.size().y * 0.5,
                    ),
                    g,
                    kit.c(sk.text),
                );
                // Faces with colours of their own (which win over the
                // body's): one step takes them away.
                if body.face_colors > 0 && !in_use {
                    let label = match body.face_colors {
                        1 => "Clear face colour".to_string(),
                        n => format!("Clear {n} face colours"),
                    };
                    let button = kit.button(ui, Button::secondary(&label));
                    self.side
                        .hits
                        .targets
                        .push(("clear face colours".to_string(), button.rect));
                    if button.clicked() {
                        clear_faces = true;
                    }
                }
            });
        });
        ui.add_space(t.nav_spacing * 2.0);
        if clear_faces {
            self.side_queue(
                "face.appearance",
                json!({"body": body.feature, "clear": true}),
                None,
            );
        }
        asked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_and_what_was_used_up() {
        let inspect = json!({
            "bodies": [
                {"feature": "F5", "name": "A", "volume": 1000.0, "mass": 0.0027, "kind": "solid", "valid": true, "faces": 6, "edges": 12,
                 "bbox": [[0, 0, 0], [10, 10, 10]], "material": {"name": "Aluminium", "density": 2700.0}},
                {"feature": "F7", "name": "B", "volume": 500.0, "kind": "solid", "valid": true}
            ],
            "features": [
                {"id": "F1", "name": "T", "bodyUsedBy": "F5"},
                {"id": "F2", "name": "U"}
            ]
        });
        let (bodies, consumed) = body_lines(&inspect);
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0].size, Some([10.0, 10.0, 10.0]));
        assert_eq!(consumed.len(), 1);
        assert_eq!(consumed[0].used_by.as_deref(), Some("F5"));
        let t = totals(&bodies);
        assert_eq!(t.count, 2);
        assert!((t.volume - 1500.0).abs() < 1e-9);
        // One body without a material: no total mass.
        assert_eq!(t.mass, None);
        assert_eq!(hex("#d4d6da"), Some([0xd4, 0xd6, 0xda]));
        assert_eq!(hex("nope"), None);
    }
}
