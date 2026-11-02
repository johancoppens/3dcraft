//! The kit's controls: buttons, icon buttons, chips, segmented controls,
//! check boxes, switches, sliders, number and text fields and dropdowns
//! (board c1 of the plan).

use super::glyphs::{self, Glyph};
use super::{Choice, Ground, Kit, centred_left, sane};
use crate::theme::{Rgb, blend, color};
use egui::epaint::RectShape;
use egui::{
    Align, CornerRadius, Frame, Id, Margin, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
use std::ops::RangeInclusive;

/// What a button does, which its look says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// The action of a dialog or a bar: amber with the knob's ink.
    Primary,
    /// A second action: a field's fill and outline.
    Secondary,
    /// No fill until the pointer is over it (Copy details, Later).
    Quiet,
    /// Text in the info ink (How it works).
    Link,
    /// Secondary, in the failure's ink (Discard, Delete).
    Danger,
}

/// A button: its text, kind and optional icon.
#[derive(Clone, Copy, Debug)]
pub struct Button<'a> {
    pub text: &'a str,
    pub kind: ButtonKind,
    pub icon: Option<Glyph>,
    pub enabled: bool,
    /// Drawn as under the pointer (the gallery shows the state).
    pub shown_hovered: bool,
}

impl<'a> Button<'a> {
    pub fn new(text: &'a str, kind: ButtonKind) -> Self {
        Self {
            text,
            kind,
            icon: None,
            enabled: true,
            shown_hovered: false,
        }
    }
    pub fn primary(text: &'a str) -> Self {
        Self::new(text, ButtonKind::Primary)
    }
    pub fn secondary(text: &'a str) -> Self {
        Self::new(text, ButtonKind::Secondary)
    }
    pub fn quiet(text: &'a str) -> Self {
        Self::new(text, ButtonKind::Quiet)
    }
    pub fn link(text: &'a str) -> Self {
        Self::new(text, ButtonKind::Link)
    }
    pub fn danger(text: &'a str) -> Self {
        Self::new(text, ButtonKind::Danger)
    }
    #[must_use]
    pub fn icon(mut self, icon: Glyph) -> Self {
        self.icon = Some(icon);
        self
    }
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    #[must_use]
    pub fn hovered(mut self, hovered: bool) -> Self {
        self.shown_hovered = hovered;
        self
    }
}

/// A chip's meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipKind {
    Plain,
    /// Information (a feature's id, a count): info fill and ink.
    Info,
    /// What is on (Studio): amber with the knob's ink.
    On,
    Failed,
}

/// One segment of a segmented control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment<'a> {
    Text(&'a str),
    Icon(Glyph),
}

/// A slider with its label and value: `text` is the value as shown
/// (`0.00 EV`, `f/2.8`), `hint` a line under the rail.
pub struct Slider<'a> {
    pub label: &'a str,
    pub value: &'a mut f64,
    pub range: RangeInclusive<f64>,
    pub text: String,
    pub enabled: bool,
    pub hint: Option<&'a str>,
    /// The widths kept for the label and the value, in points.
    pub label_width: f32,
    pub value_width: f32,
}

impl<'a> Slider<'a> {
    pub fn new(label: &'a str, value: &'a mut f64, range: RangeInclusive<f64>) -> Self {
        let text = format!("{:.2}", *value);
        Self {
            label,
            value,
            range,
            text,
            enabled: true,
            hint: None,
            label_width: 96.0,
            value_width: 64.0,
        }
    }
    #[must_use]
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }
    #[must_use]
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    #[must_use]
    pub fn widths(mut self, label: f32, value: f32) -> Self {
        self.label_width = label;
        self.value_width = value;
        self
    }
}

/// Where `value` lies in `range`, 0 to 1 (0 for what means nothing).
pub fn fraction(value: f64, range: &RangeInclusive<f64>) -> f32 {
    let (lo, hi) = (*range.start(), *range.end());
    if !(value.is_finite() && lo.is_finite() && hi.is_finite()) || hi <= lo {
        return 0.0;
    }
    ((value - lo) / (hi - lo)).clamp(0.0, 1.0) as f32
}

impl Kit {
    /// A step darker (or lighter on a dark ground) than `fill`: under the
    /// pointer.
    pub(crate) fn hovered_fill(&self, fill: Rgb) -> Rgb {
        blend(self.skin.text, 18, fill)
    }

    /// A button.
    pub fn button(&self, ui: &mut Ui, button: Button<'_>) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let font = match button.kind {
            ButtonKind::Primary => self.medium_font(ui),
            _ => self.font(),
        };
        let pad = t.field_inset;
        let icon = button.icon.map(|_| t.nav_icon_size + t.nav_icon_gap);
        let (ink, fill, stroke) = if !button.enabled {
            (s.faint, None, Some(s.field_stroke))
        } else {
            match button.kind {
                ButtonKind::Primary => (t.knob, Some(t.accent), None),
                ButtonKind::Secondary => (s.text, Some(s.field), Some(s.field_stroke)),
                ButtonKind::Danger => (s.failed, Some(s.field), Some(s.field_stroke)),
                ButtonKind::Quiet => (s.text, None, None),
                ButtonKind::Link => (s.info_ink, None, None),
            }
        };
        let galley = self.galley(ui, button.text, font, ink);
        let width = galley.size().x + 2.0 * pad + icon.unwrap_or(0.0);
        let visual = vec2(width, t.control_height);
        let sense = if button.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (outer, response) = ui.allocate_exact_size(
            sane(vec2(width.max(t.hit_min), self.hit(t.control_height))),
            sense,
        );
        let rect = Rect::from_center_size(outer.center(), visual);
        if ui.is_rect_visible(outer) {
            let hovered = button.enabled && (response.hovered() || button.shown_hovered);
            let pressed = response.is_pointer_button_down_on();
            let fill = match (fill, hovered, button.kind) {
                (Some(fill), true, _) => Some(self.hovered_fill(fill)),
                (None, true, ButtonKind::Quiet) => Some(s.hover),
                (fill, _, _) => fill,
            };
            let fill = match (fill, pressed) {
                (Some(fill), true) => Some(self.hovered_fill(fill)),
                (fill, _) => fill,
            };
            let painter = ui.painter();
            painter.add(RectShape::new(
                rect,
                self.radius(),
                fill.map_or(egui::Color32::TRANSPARENT, color),
                stroke.map_or(Stroke::NONE, |c| Stroke::new(1.0, color(c))),
                StrokeKind::Inside,
            ));
            let mut x = rect.left() + pad;
            if let Some(glyph) = button.icon {
                let place = centred_left(
                    Rect::from_min_max(pos2(x, rect.top()), rect.max),
                    vec2(t.nav_icon_size, t.nav_icon_size),
                );
                glyphs::paint(painter, glyph, place, color(ink), t);
                x += t.nav_icon_size + t.nav_icon_gap;
            }
            let at = pos2(x, rect.center().y - galley.size().y * 0.5);
            if button.kind == ButtonKind::Link && hovered {
                let y = at.y + galley.size().y - 1.0;
                painter.line_segment(
                    [pos2(at.x, y), pos2(at.x + galley.size().x, y)],
                    Stroke::new(1.0, color(ink)),
                );
            }
            painter.galley(at, galley, color(ink));
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, button.enabled, button.text)
        });
        response
    }

    /// A square button with an icon: dim at rest, the text's ink under the
    /// pointer, amber with the knob's ink when `on`. `label` names it for
    /// assistive technology (and is the tooltip a caller may show).
    pub fn icon_button(&self, ui: &mut Ui, glyph: Glyph, on: bool, label: &str) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let side = t.control_height;
        let hit = self.hit(side);
        let (outer, response) = ui.allocate_exact_size(sane(vec2(hit, hit)), Sense::click());
        if ui.is_rect_visible(outer) {
            let rect = Rect::from_center_size(outer.center(), vec2(side, side));
            let hovered = response.hovered();
            let (fill, ink) = if on {
                (Some(t.accent), t.knob)
            } else if hovered {
                (Some(s.hover), s.text)
            } else {
                (None, s.text_dim)
            };
            if let Some(fill) = fill {
                ui.painter().rect_filled(rect, self.radius(), color(fill));
            }
            let icon = super::clamped(side - 2.0 * t.nav_spacing - 4.0, 8.0, t.nav_icon_size)
                .unwrap_or(t.nav_icon_size);
            glyphs::paint(
                ui.painter(),
                glyph,
                Rect::from_center_size(rect.center(), vec2(icon, icon)),
                color(ink),
                t,
            );
        }
        response
            .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, label));
        response
    }

    /// A chip: a short state or name in a pill.
    pub fn chip(&self, ui: &mut Ui, text: &str, kind: ChipKind) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let (fill, ink) = match kind {
            ChipKind::Plain => (s.chip, s.text),
            ChipKind::Info => (s.info_fill, s.info_ink),
            ChipKind::On => (t.accent, t.knob),
            ChipKind::Failed => (s.failed_fill, s.failed),
        };
        let font = match kind {
            ChipKind::On => self.medium_font(ui),
            _ => self.font(),
        };
        let galley = self.galley(ui, text, font, ink);
        let h = t.chip_height;
        let width = galley.size().x + h;
        let (outer, response) = ui.allocate_exact_size(
            sane(vec2(width.max(t.hit_min), self.hit(h))),
            Sense::click(),
        );
        if ui.is_rect_visible(outer) {
            let rect = Rect::from_center_size(outer.center(), vec2(width, h));
            let fill = if response.hovered() {
                self.hovered_fill(fill)
            } else {
                fill
            };
            ui.painter()
                .rect_filled(rect, CornerRadius::same(radius_of(h)), color(fill));
            ui.painter()
                .galley(rect.center() - galley.size() * 0.5, galley, color(ink));
        }
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
        response
    }

    /// A segmented control: one of `segments`, the chosen one amber.
    pub fn segmented(&self, ui: &mut Ui, chosen: &mut usize, segments: &[Segment<'_>]) -> Choice {
        let t = &self.t;
        let s = &self.skin;
        let pad = t.field_inset;
        let font = self.font();
        let widths: Vec<f32> = segments
            .iter()
            .map(|segment| {
                let w = match segment {
                    Segment::Text(text) => {
                        self.galley(ui, *text, font.clone(), s.text).size().x + 2.0 * pad
                    }
                    Segment::Icon(_) => t.control_height + 2.0 * t.nav_segment_inset + 4.0,
                };
                w.max(t.hit_min)
            })
            .collect();
        let inset = t.nav_segment_inset;
        let total: f32 = widths.iter().sum::<f32>() + 2.0 * inset;
        let (outer, mut response) = ui.allocate_exact_size(
            sane(vec2(total, self.hit(t.control_height))),
            Sense::hover(),
        );
        let track = Rect::from_center_size(outer.center(), vec2(total, t.control_height));
        let painter = ui.painter().clone();
        painter.rect_filled(track, self.field_radius(), color(s.field));
        let mut items = Vec::with_capacity(segments.len());
        let mut x = track.left() + inset;
        for (i, (segment, width)) in segments.iter().zip(&widths).enumerate() {
            let cell = Rect::from_min_size(pos2(x, outer.top()), vec2(*width, outer.height()));
            x += width;
            let id = response.id.with(i);
            let hit = ui.interact(cell, id, Sense::click());
            let picked = *chosen == i;
            hit.widget_info(|| {
                let label = match segment {
                    Segment::Text(text) => (*text).to_string(),
                    Segment::Icon(glyph) => format!("{glyph:?}"),
                };
                egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, picked, label)
            });
            if hit.clicked() && *chosen != i {
                *chosen = i;
                response.mark_changed();
            }
            items.push(cell);
            let on = *chosen == i;
            let face = Rect::from_min_max(
                pos2(cell.left(), track.top() + inset),
                pos2(cell.right(), track.bottom() - inset),
            );
            let ink = if on { t.knob } else { s.text };
            if on {
                painter.rect_filled(
                    face,
                    CornerRadius::same(t.nav_segment_radius),
                    color(t.accent),
                );
            } else if hit.hovered() {
                painter.rect_filled(
                    face,
                    CornerRadius::same(t.nav_segment_radius),
                    color(self.hovered_fill(s.field)),
                );
            }
            match segment {
                Segment::Text(text) => {
                    let galley = self.galley(ui, *text, font.clone(), ink);
                    painter.galley(face.center() - galley.size() * 0.5, galley, color(ink));
                }
                Segment::Icon(glyph) => {
                    let side = t.nav_icon_size.min(face.height());
                    glyphs::paint(
                        &painter,
                        *glyph,
                        Rect::from_center_size(face.center(), vec2(side, side)),
                        color(ink),
                        t,
                    );
                }
            }
        }
        Choice { response, items }
    }

    /// A check box with its label.
    pub fn checkbox(&self, ui: &mut Ui, on: &mut bool, text: &str) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let galley = self.galley(ui, text, self.font(), s.text);
        let side = t.check_size;
        let width = side + t.nav_icon_gap + galley.size().x;
        let (outer, mut response) = ui.allocate_exact_size(
            sane(vec2(width.max(t.hit_min), self.hit(t.control_height))),
            Sense::click(),
        );
        if response.clicked() {
            *on = !*on;
            response.mark_changed();
        }
        if ui.is_rect_visible(outer) {
            let painter = ui.painter();
            let place = centred_left(outer, vec2(side, side));
            let radius = CornerRadius::same(t.radius.saturating_sub(1));
            if *on {
                painter.rect_filled(place, radius, color(t.accent));
                glyphs::paint(painter, Glyph::Check, place.shrink(1.5), color(t.knob), t);
            } else {
                let fill = if response.hovered() {
                    self.hovered_fill(s.field)
                } else {
                    s.field
                };
                painter.rect(
                    place,
                    radius,
                    color(fill),
                    Stroke::new(1.0, color(s.field_stroke)),
                    StrokeKind::Inside,
                );
            }
            painter.galley(
                pos2(
                    place.right() + t.nav_icon_gap,
                    outer.center().y - galley.size().y * 0.5,
                ),
                galley,
                color(s.text),
            );
        }
        let state = *on;
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, state, text)
        });
        response
    }

    /// A switch with its label: amber with a dark knob when on.
    pub fn toggle(&self, ui: &mut Ui, on: &mut bool, text: &str) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let galley = self.galley(ui, text, self.font(), s.text);
        let [w, h] = t.switch_size;
        let width = w + t.nav_icon_gap + galley.size().x;
        let (outer, mut response) = ui.allocate_exact_size(
            sane(vec2(width.max(t.hit_min), self.hit(t.control_height))),
            Sense::click(),
        );
        if response.clicked() {
            *on = !*on;
            response.mark_changed();
        }
        if ui.is_rect_visible(outer) {
            let painter = ui.painter();
            let place = centred_left(outer, vec2(w, h));
            let radius = CornerRadius::same(radius_of(h));
            let knob_r = (h * 0.5 - 2.0).max(1.0);
            if *on {
                painter.rect_filled(place, radius, color(t.accent));
                painter.circle_filled(
                    pos2(place.right() - h * 0.5, place.center().y),
                    knob_r,
                    color(t.knob),
                );
            } else {
                let track = if response.hovered() {
                    self.hovered_fill(s.track)
                } else {
                    s.track
                };
                painter.rect_filled(place, radius, color(track));
                painter.circle(
                    pos2(place.left() + h * 0.5, place.center().y),
                    knob_r,
                    color(s.knob),
                    Stroke::new(0.5, color(s.knob_rim)),
                );
            }
            painter.galley(
                pos2(
                    place.right() + t.nav_icon_gap,
                    outer.center().y - galley.size().y * 0.5,
                ),
                galley,
                color(s.text),
            );
        }
        let state = *on;
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, state, text)
        });
        response
    }

    /// A slider across the width there is: label, rail, value; a hint
    /// under the rail. Dragging or clicking the rail sets the value.
    pub fn slider(&self, ui: &mut Ui, slider: Slider<'_>) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let ink = if slider.enabled { s.text } else { s.faint };
        let width = ui
            .available_width()
            .max(slider.label_width + slider.value_width + 40.0);
        let row = self.hit(t.control_height);
        let (outer, place) = ui.allocate_exact_size(sane(vec2(width, row)), Sense::hover());
        let knob = t.knob_size;
        let rail = Rect::from_min_max(
            pos2(outer.left() + slider.label_width + knob * 0.5, outer.top()),
            pos2(
                outer.right() - slider.value_width - knob * 0.5,
                outer.bottom(),
            ),
        );
        let sense = if slider.enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let id = place.id.with("rail");
        let mut response = ui.interact(rail.expand2(vec2(knob * 0.5, 0.0)), id, sense);
        if (response.dragged() || response.clicked())
            && let Some(at) = response.interact_pointer_pos()
            && rail.width() > 0.0
        {
            let share = f64::from(((at.x - rail.left()) / rail.width()).clamp(0.0, 1.0));
            let (lo, hi) = (*slider.range.start(), *slider.range.end());
            if lo.is_finite() && hi.is_finite() && hi > lo {
                let value = lo + share * (hi - lo);
                if value != *slider.value {
                    *slider.value = value;
                    response.mark_changed();
                }
            }
        }
        if ui.is_rect_visible(outer) {
            let painter = ui.painter();
            let label = self.galley(ui, slider.label, self.font(), ink);
            painter.galley(
                pos2(outer.left(), outer.center().y - label.size().y * 0.5),
                label,
                color(ink),
            );
            let value = self.galley(ui, slider.text.as_str(), self.font(), ink);
            painter.galley(
                pos2(
                    outer.right() - value.size().x,
                    outer.center().y - value.size().y * 0.5,
                ),
                value,
                color(ink),
            );
            let thick = (t.progress_height * 0.75).max(1.0);
            let y = outer.center().y;
            let f = fraction(*slider.value, &slider.range);
            let x = rail.left() + rail.width().max(0.0) * f;
            let full = Rect::from_min_max(
                pos2(rail.left(), y - thick * 0.5),
                pos2(rail.right(), y + thick * 0.5),
            );
            let r = CornerRadius::same(radius_of(thick));
            painter.rect_filled(full, r, color(s.track));
            let done = if slider.enabled { t.accent } else { s.faint };
            painter.rect_filled(
                Rect::from_min_max(full.min, pos2(x, full.bottom())),
                r,
                color(done),
            );
            let (fill, rim) = if slider.enabled {
                (s.knob, s.knob_rim)
            } else {
                (blend(s.faint, 140, s.fill), s.faint)
            };
            painter.circle(
                pos2(x, y),
                knob * 0.5,
                color(fill),
                Stroke::new(1.0, color(rim)),
            );
        }
        if let Some(hint) = slider.hint {
            let galley = self.galley(ui, hint, self.small_font(), s.text_dim);
            let (place, _) =
                ui.allocate_exact_size(sane(vec2(width, galley.size().y)), Sense::hover());
            ui.painter().galley(
                pos2(rail.left() - knob * 0.5, place.top()),
                galley,
                color(s.text_dim),
            );
        }
        let value = *slider.value;
        response.widget_info(|| egui::WidgetInfo::slider(slider.enabled, value, slider.label));
        response
    }

    /// A number field with its unit: `error` rings it in the failure's
    /// colour, focus in the info ink.
    pub fn number_field(
        &self,
        ui: &mut Ui,
        id_salt: impl std::hash::Hash + std::fmt::Debug,
        text: &mut String,
        unit: &str,
        width: f32,
        error: bool,
    ) -> Response {
        self.field(
            ui,
            ui.make_persistent_id(id_salt),
            text,
            "",
            None,
            unit,
            width,
            error,
        )
    }

    /// A text field with a hint while it is empty and an optional icon at
    /// its start (the search field: [`Glyph::Search`]).
    pub fn text_field(
        &self,
        ui: &mut Ui,
        id_salt: impl std::hash::Hash + std::fmt::Debug,
        text: &mut String,
        hint: &str,
        icon: Option<Glyph>,
        width: f32,
    ) -> Response {
        self.field(
            ui,
            ui.make_persistent_id(id_salt),
            text,
            hint,
            icon,
            "",
            width,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn field(
        &self,
        ui: &mut Ui,
        id: Id,
        text: &mut String,
        hint: &str,
        icon: Option<Glyph>,
        unit: &str,
        width: f32,
        error: bool,
    ) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let width = width.min(ui.available_width()).max(t.control_height * 2.0);
        let (outer, place) = ui.allocate_exact_size(
            sane(vec2(width, self.hit(t.control_height))),
            Sense::click(),
        );
        let rect = Rect::from_center_size(outer.center(), vec2(width, t.control_height));
        let back = ui.painter().add(Shape::Noop);
        let pad = t.field_text_pad;
        let mut left = rect.left() + pad;
        if let Some(glyph) = icon {
            let side = t.nav_icon_size * 0.875;
            glyphs::paint(
                ui.painter(),
                glyph,
                centred_left(
                    Rect::from_min_max(pos2(left, rect.top()), rect.max),
                    vec2(side, side),
                ),
                color(s.text_dim),
                t,
            );
            left += side + t.nav_icon_gap;
        }
        let unit_galley =
            (!unit.is_empty()).then(|| self.galley(ui, unit, self.font(), s.text_dim));
        let right = rect.right()
            - pad
            - unit_galley
                .as_ref()
                .map_or(0.0, |g| g.size().x + t.nav_icon_gap);
        let inner = Rect::from_min_max(
            pos2(left, outer.top()),
            pos2(right.max(left), outer.bottom()),
        );
        let edit = egui::TextEdit::singleline(text)
            .id(id)
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .font(self.font())
            .text_color(color(s.text))
            .vertical_align(Align::Center)
            .desired_width(inner.width());
        // A child that allocates nothing in `ui`: the field's room is
        // already taken, and a second allocation would move the cursor
        // back over it.
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let response = child.add(edit);
        let empty = text.is_empty();
        // The hint is the kit's own: egui fades a hint by its visuals,
        // which on a plate over light visuals leaves it unreadable.
        if empty && !hint.is_empty() && !response.has_focus() {
            let galley = self.galley(ui, hint, self.font(), s.text_dim);
            ui.painter().galley(
                pos2(inner.left(), inner.center().y - galley.size().y * 0.5),
                galley,
                color(s.text_dim),
            );
        }
        let ring = if error {
            Stroke::new(1.5, color(s.failed))
        } else if response.has_focus() {
            Stroke::new(1.5, color(s.focus))
        } else {
            Stroke::new(1.0, color(s.field_stroke))
        };
        ui.painter().set(
            back,
            RectShape::new(
                rect,
                self.field_radius(),
                color(s.field),
                ring,
                StrokeKind::Inside,
            ),
        );
        if let Some(galley) = unit_galley {
            ui.painter().galley(
                pos2(
                    rect.right() - pad - galley.size().x,
                    rect.center().y - galley.size().y * 0.5,
                ),
                galley,
                color(s.text_dim),
            );
        }
        // The whole field is the target: a tap beside the text focuses it.
        if place.clicked() {
            ui.memory_mut(|m| m.request_focus(id));
        }
        response | place
    }

    /// A dropdown: the chosen option in a field with a chevron; a click
    /// opens the options as a menu under it.
    pub fn dropdown(
        &self,
        ui: &mut Ui,
        id_salt: impl std::hash::Hash + std::fmt::Debug,
        chosen: &mut usize,
        options: &[&str],
        width: f32,
    ) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let id = ui.make_persistent_id(id_salt);
        let width = width.min(ui.available_width()).max(t.control_height * 2.0);
        let (outer, mut response) = ui.allocate_exact_size(
            sane(vec2(width, self.hit(t.control_height))),
            Sense::click(),
        );
        let rect = Rect::from_center_size(outer.center(), vec2(width, t.control_height));
        let mut open = ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
        if response.clicked() {
            open = !open;
        }
        if ui.is_rect_visible(outer) {
            let painter = ui.painter();
            let fill = if response.hovered() {
                self.hovered_fill(s.field)
            } else {
                s.field
            };
            let ring = if open {
                Stroke::new(1.5, color(s.focus))
            } else {
                Stroke::new(1.0, color(s.field_stroke))
            };
            painter.add(RectShape::new(
                rect,
                self.field_radius(),
                color(fill),
                ring,
                StrokeKind::Inside,
            ));
            let text = options.get(*chosen).copied().unwrap_or("");
            let galley = self.galley(ui, text, self.font(), s.text);
            painter.galley(
                pos2(
                    rect.left() + t.field_text_pad,
                    rect.center().y - galley.size().y * 0.5,
                ),
                galley,
                color(s.text),
            );
            let side = t.nav_icon_size * 0.75;
            glyphs::paint(
                painter,
                Glyph::ChevronDown,
                Rect::from_center_size(
                    pos2(
                        rect.right() - t.field_text_pad - side * 0.5,
                        rect.center().y,
                    ),
                    vec2(side, side),
                ),
                color(s.text_dim),
                t,
            );
        }
        if open {
            let menu = Kit::new(&self.t, Ground::Plate);
            let area = egui::Area::new(id.with("menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.left_bottom() + vec2(0.0, 2.0))
                .show(ui.ctx(), |ui| {
                    menu.menu_frame(ui, width, |ui| {
                        for (i, option) in options.iter().enumerate() {
                            let item = super::MenuItem::new(option).checked(i == *chosen);
                            if menu.menu_item(ui, item).clicked() {
                                if *chosen != i {
                                    *chosen = i;
                                    response.mark_changed();
                                }
                                open = false;
                            }
                        }
                    });
                });
            let elsewhere = ui.input(|i| i.pointer.any_pressed())
                && !area.response.contains_pointer()
                && !response.contains_pointer();
            if elsewhere || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                open = false;
            }
        }
        ui.data_mut(|d| d.insert_temp(id, open));
        let label = options.get(*chosen).copied().unwrap_or("").to_string();
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label));
        response
    }
}

/// The corner radius that makes a pill of height `h`.
pub(crate) fn radius_of(h: f32) -> u8 {
    if h.is_finite() {
        (h * 0.5).clamp(0.0, 255.0).round() as u8
    } else {
        0
    }
}
