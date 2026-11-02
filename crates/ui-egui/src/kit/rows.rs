//! The kit's rows: captions, section and fold headers, label/value rows,
//! list rows with an icon, and the tab bar of a panel (board c1).

use super::glyphs::{self, Glyph};
use super::{Choice, Kit, centred_left, sane};
use crate::theme::{Rgb, color};
use egui::{CornerRadius, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

/// How a row's value is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueStyle {
    /// In the text's ink.
    Plain,
    /// A link in the info ink (the feature that made a face).
    Link,
    /// A persistent name: monospace, in the info ink.
    Name,
}

/// A row of a list: an icon, a name, a value at the right, and its state.
#[derive(Clone, Copy, Debug)]
pub struct ListRow<'a> {
    pub icon: Option<Glyph>,
    /// The icon's ink; `None` for the ground's dim ink.
    pub icon_ink: Option<Rgb>,
    pub name: &'a str,
    pub value: &'a str,
    pub selected: bool,
    /// Drawn as under the pointer (the gallery shows the state).
    pub shown_hovered: bool,
    /// Rolled back or not rebuilt: the name dim, the value faint.
    pub dimmed: bool,
    /// Failed: name and icon in the failure's ink.
    pub failed: bool,
    /// An eye at the right (a hidden feature, or the row under the
    /// pointer).
    pub eye: bool,
    /// How far the row is indented, in levels.
    pub indent: u8,
}

impl<'a> ListRow<'a> {
    pub fn new(name: &'a str) -> Self {
        Self {
            icon: None,
            icon_ink: None,
            name,
            value: "",
            selected: false,
            shown_hovered: false,
            dimmed: false,
            failed: false,
            eye: false,
            indent: 0,
        }
    }
    #[must_use]
    pub fn icon(mut self, icon: Glyph) -> Self {
        self.icon = Some(icon);
        self
    }
    #[must_use]
    pub fn icon_ink(mut self, ink: Rgb) -> Self {
        self.icon_ink = Some(ink);
        self
    }
    #[must_use]
    pub fn value(mut self, value: &'a str) -> Self {
        self.value = value;
        self
    }
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    #[must_use]
    pub fn hovered(mut self, hovered: bool) -> Self {
        self.shown_hovered = hovered;
        self
    }
    #[must_use]
    pub fn dimmed(mut self, dimmed: bool) -> Self {
        self.dimmed = dimmed;
        self
    }
    #[must_use]
    pub fn failed(mut self, failed: bool) -> Self {
        self.failed = failed;
        self
    }
    #[must_use]
    pub fn eye(mut self, eye: bool) -> Self {
        self.eye = eye;
        self
    }
    #[must_use]
    pub fn indent(mut self, indent: u8) -> Self {
        self.indent = indent;
        self
    }
}

/// A tab of a panel's tab bar; `dot` marks it (an agent connected).
#[derive(Clone, Copy, Debug)]
pub struct Tab<'a> {
    pub label: &'a str,
    pub dot: bool,
}

impl<'a> Tab<'a> {
    pub fn new(label: &'a str) -> Self {
        Self { label, dot: false }
    }
    #[must_use]
    pub fn dot(mut self, dot: bool) -> Self {
        self.dot = dot;
        self
    }
}

impl Kit {
    /// A table head or a group's caption: small, upper case, dim.
    pub fn caption(&self, ui: &mut Ui, text: &str) -> Response {
        let galley = self.galley(
            ui,
            text.to_uppercase(),
            self.small_font(),
            self.skin.text_dim,
        );
        let (rect, response) = ui.allocate_exact_size(
            sane(vec2(galley.size().x, self.t.row_height.min(22.0))),
            Sense::hover(),
        );
        ui.painter().galley(
            pos2(rect.left(), rect.center().y - galley.size().y * 0.5),
            galley,
            color(self.skin.text_dim),
        );
        response
    }

    /// A section's heading: the info accent's bar and the title in the
    /// medium weight.
    pub fn section_header(&self, ui: &mut Ui, text: &str) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let galley = self.galley(ui, text, self.medium_font(ui), s.text);
        let gap = t.nav_icon_gap + 1.0;
        let width = t.info_bar_width + gap + galley.size().x;
        let (rect, response) =
            ui.allocate_exact_size(sane(vec2(width, t.label_row_height)), Sense::hover());
        let bar = Rect::from_center_size(
            pos2(rect.left() + t.info_bar_width * 0.5, rect.center().y),
            vec2(t.info_bar_width, t.secondary_font + 1.0),
        );
        ui.painter().rect_filled(
            bar,
            CornerRadius::same(t.info_bar_radius),
            color(s.info_bar),
        );
        ui.painter().galley(
            pos2(bar.right() + gap, rect.center().y - galley.size().y * 0.5),
            galley,
            color(s.text),
        );
        response
    }

    /// A header that folds what is under it: a chevron, the title in the
    /// medium weight, a count, and a note at the right. A click turns
    /// `open`.
    pub fn fold_header(
        &self,
        ui: &mut Ui,
        open: &mut bool,
        title: &str,
        count: Option<usize>,
        note: Option<(&str, Rgb)>,
    ) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let width = ui.available_width();
        let (rect, mut response) =
            ui.allocate_exact_size(sane(vec2(width, self.hit(t.row_height))), Sense::click());
        if response.clicked() {
            *open = !*open;
            response.mark_changed();
        }
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if response.hovered() {
                painter.rect_filled(rect, self.radius(), color(s.hover));
            }
            let side = t.nav_icon_size * 0.75;
            let chevron = if *open {
                Glyph::ChevronDown
            } else {
                Glyph::ChevronRight
            };
            let place = centred_left(rect.translate(vec2(2.0, 0.0)), vec2(side, side));
            glyphs::paint(painter, chevron, place, color(s.text_dim), t);
            let mut x = place.right() + t.nav_icon_gap + 2.0;
            let title = self.galley(ui, title, self.medium_font(ui), s.text);
            let y = rect.center().y - title.size().y * 0.5;
            let title_width = title.size().x;
            painter.galley(pos2(x, y), title, color(s.text));
            x += title_width + t.nav_icon_gap;
            if let Some(count) = count {
                let count = self.galley(ui, count.to_string(), self.medium_font(ui), s.text);
                painter.galley(pos2(x, y), count, color(s.text));
            }
            if let Some((note, ink)) = note {
                let note = self.galley(ui, note, self.font(), ink);
                painter.galley(
                    pos2(
                        rect.right() - note.size().x - 2.0,
                        rect.center().y - note.size().y * 0.5,
                    ),
                    note,
                    color(ink),
                );
            }
        }
        response
    }

    /// A property row: the label dim at the left, the value after it at
    /// `label_width`.
    pub fn prop_row(
        &self,
        ui: &mut Ui,
        label: &str,
        value: &str,
        style: ValueStyle,
        label_width: f32,
    ) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let width = ui.available_width();
        let (rect, response) =
            ui.allocate_exact_size(sane(vec2(width, t.label_row_height)), Sense::hover());
        let painter = ui.painter();
        let label = self.galley(ui, label, self.font(), s.text_dim);
        painter.galley(
            pos2(rect.left(), rect.center().y - label.size().y * 0.5),
            label,
            color(s.text_dim),
        );
        let (font, ink) = match style {
            ValueStyle::Plain => (self.font(), s.text),
            ValueStyle::Link => (self.font(), s.info_ink),
            ValueStyle::Name => (self.mono_font(), s.info_ink),
        };
        let value = self.galley(ui, value, font, ink);
        painter.galley(
            pos2(
                rect.left() + label_width,
                rect.center().y - value.size().y * 0.5,
            ),
            value,
            color(ink),
        );
        response
    }

    /// A row of a list: icon, name, value at the right; under the pointer
    /// the hover fill, selected the selection's copper with the value in
    /// the text's ink (`text_dim` on copper is too faint).
    pub fn list_row(&self, ui: &mut Ui, row: ListRow<'_>) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let width = ui.available_width();
        let (rect, response) =
            ui.allocate_exact_size(sane(vec2(width, self.hit(t.row_height))), Sense::click());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let painter = ui.painter();
        let hovered = response.hovered() || row.shown_hovered;
        if row.selected {
            painter.rect_filled(rect, self.radius(), color(s.selection));
        } else if row.failed {
            painter.rect_filled(rect, self.radius(), color(s.failed_fill));
        } else if hovered {
            painter.rect_filled(rect, self.radius(), color(s.hover));
        }
        let (name_ink, value_ink) = if row.failed {
            (s.failed, s.failed)
        } else if row.selected {
            (s.text, s.text)
        } else if row.dimmed {
            (s.text_dim, s.faint)
        } else {
            (s.text, s.text_dim)
        };
        let pad = t.nav_spacing * 4.0;
        let mut x = rect.left() + pad + f32::from(row.indent) * t.nav_icon_size;
        if let Some(glyph) = row.icon {
            let side = t.nav_icon_size.min(rect.height());
            let ink = if row.failed {
                s.failed
            } else if row.selected {
                s.text
            } else {
                row.icon_ink.unwrap_or(s.text_dim)
            };
            glyphs::paint(
                painter,
                glyph,
                centred_left(
                    Rect::from_min_max(pos2(x, rect.top()), rect.max),
                    vec2(side, side),
                ),
                color(ink),
                t,
            );
            x += side + t.nav_icon_gap;
        }
        let mut right = rect.right() - pad;
        if row.eye || (hovered && !row.selected) {
            let side = t.nav_icon_size;
            if row.eye {
                glyphs::paint(
                    painter,
                    Glyph::Eye,
                    Rect::from_center_size(
                        pos2(right - side * 0.5, rect.center().y),
                        vec2(side, side),
                    ),
                    color(s.text_dim),
                    t,
                );
                right -= side + t.nav_icon_gap;
            }
        }
        let value = self.galley(ui, row.value, self.font(), value_ink);
        let value_width = value.size().x;
        painter.galley(
            pos2(right - value_width, rect.center().y - value.size().y * 0.5),
            value,
            color(value_ink),
        );
        // The name is cut where the value begins.
        let room = (right - value_width - t.nav_icon_gap - x).max(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(
            row.name.to_string(),
            self.font(),
            color(name_ink),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let name = ui.painter().layout_job(job);
        ui.painter().galley(
            pos2(x, rect.center().y - name.size().y * 0.5),
            name,
            color(name_ink),
        );
        response
    }

    /// A panel's tab bar: the active tab in the medium weight with the
    /// accent's underline, the others dim; a rule under them all.
    pub fn tab_bar(&self, ui: &mut Ui, active: &mut usize, tabs: &[Tab<'_>]) -> Choice {
        let t = &self.t;
        let s = &self.skin;
        let width = ui.available_width();
        let (rect, mut response) = ui.allocate_exact_size(
            sane(vec2(width, self.hit(t.panel_tab_height))),
            Sense::hover(),
        );
        let painter = ui.painter().clone();
        painter.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, color(s.divider)),
        );
        let pad = t.field_inset;
        let dot = t.info_bar_width + 1.0;
        let mut x = rect.left();
        let mut items = Vec::with_capacity(tabs.len());
        for (i, tab) in tabs.iter().enumerate() {
            let on = *active == i;
            let font = if on {
                self.medium_font(ui)
            } else {
                self.font()
            };
            let ink = if on { s.text } else { s.text_dim };
            let galley = self.galley(ui, tab.label, font, ink);
            let extra = if tab.dot {
                dot * 2.0 + t.nav_spacing * 2.0
            } else {
                0.0
            };
            let w = (galley.size().x + extra + 2.0 * pad).max(t.hit_min);
            let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height()));
            x += w;
            let hit = ui.interact(cell, response.id.with(i), Sense::click());
            if hit.clicked() && !on {
                *active = i;
                response.mark_changed();
            }
            items.push(cell);
            let text_x = cell.left() + pad;
            let text_w = galley.size().x;
            let y = cell.center().y - galley.size().y * 0.5;
            let ink = if hit.hovered() && !on { s.text } else { ink };
            painter.galley(pos2(text_x, y), galley, color(ink));
            if tab.dot {
                painter.circle_filled(
                    pos2(text_x + text_w + t.nav_spacing * 2.0 + dot, cell.center().y),
                    dot,
                    color(s.info_bar),
                );
            }
            if on {
                let w = t.tab_accent_width;
                painter.rect_filled(
                    Rect::from_min_max(
                        pos2(cell.left() + pad * 0.5, rect.bottom() - w),
                        pos2(cell.right() - pad * 0.5, rect.bottom()),
                    ),
                    CornerRadius::ZERO,
                    color(t.accent),
                );
            }
        }
        Choice { response, items }
    }

    /// A rule between two sections, across the width there is.
    pub fn divider(&self, ui: &mut Ui) {
        let width = ui.available_width();
        let (rect, _) =
            ui.allocate_exact_size(sane(vec2(width, self.t.nav_gap + 1.0)), Sense::hover());
        ui.painter().line_segment(
            [rect.left_center(), rect.right_center()],
            Stroke::new(1.0, color(self.skin.divider)),
        );
    }
}
