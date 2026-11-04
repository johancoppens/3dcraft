//! The kit's rows: captions, section and fold headers, label/value rows,
//! list rows with an icon, and the tab bar of a panel (board c1).

use super::glyphs::{self, Glyph};
use super::{Choice, Kit, centred_left, sane};
use crate::theme::{Rgb, color};
use egui::{CornerRadius, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

/// The least share of a list row a plain value keeps when a long name
/// pushes it (the name wins up to there).
const VALUE_LEAST_SHARE: f32 = 0.3;

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
    /// Something is nested under it: a chevron in front of the icon,
    /// down when it is open.
    pub fold: Option<bool>,
    /// The drag grip at the row's left (the row under the pointer).
    pub grip: bool,
    /// The eye is struck through: the feature is hidden in the view.
    pub hidden: bool,
    /// A small mark after the name: the agent's ✦ ([`RowMark::Agent`]),
    /// or a dot in an ink (a check that fails since this step).
    pub mark: Option<RowMark>,
    /// The row is a dependency of the one under the pointer: the info
    /// fill, the info bar at its left and an arrow at its right, up for
    /// what that row uses, down for what uses it.
    pub depends: Option<Depends>,
    /// The value is drawn as a field (the row under the pointer, whose
    /// value a click edits): framed, in the text's ink.
    pub value_field: bool,
    /// The value is not drawn: the caller puts a field in its place
    /// ([`RowHits::value`]).
    pub value_hidden: bool,
    /// The target that flies to what the row made (⌖), before the eye.
    pub zoom: bool,
}

/// A mark after a row's name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RowMark {
    /// Made by an agent: a small four-pointed star in the info ink.
    Agent,
    /// A dot in this ink.
    Dot(Rgb),
}

/// Which way a dependency runs, from the row under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depends {
    /// That row uses this one.
    Uses,
    /// This one uses that row.
    UsedBy,
}

/// Where a list row put its parts, for the caller to hit them.
#[derive(Clone, Debug)]
pub struct RowHits {
    pub response: Response,
    /// The fold chevron.
    pub fold: Option<Rect>,
    /// The eye.
    pub eye: Option<Rect>,
    /// The grip.
    pub grip: Option<Rect>,
    /// The target that flies to it.
    pub zoom: Option<Rect>,
    /// Where the value is (or would be, when hidden): its field's place.
    pub value: Rect,
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
            fold: None,
            grip: false,
            hidden: false,
            mark: None,
            depends: None,
            value_field: false,
            value_hidden: false,
            zoom: false,
        }
    }
    #[must_use]
    pub fn zoom(mut self, zoom: bool) -> Self {
        self.zoom = zoom;
        self
    }
    #[must_use]
    pub fn fold(mut self, open: Option<bool>) -> Self {
        self.fold = open;
        self
    }
    #[must_use]
    pub fn grip(mut self, grip: bool) -> Self {
        self.grip = grip;
        self
    }
    #[must_use]
    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }
    #[must_use]
    pub fn mark(mut self, mark: Option<RowMark>) -> Self {
        self.mark = mark;
        self
    }
    #[must_use]
    pub fn depends(mut self, depends: Option<Depends>) -> Self {
        self.depends = depends;
        self
    }
    #[must_use]
    pub fn value_field(mut self, field: bool) -> Self {
        self.value_field = field;
        self
    }
    #[must_use]
    pub fn value_hidden(mut self, hidden: bool) -> Self {
        self.value_hidden = hidden;
        self
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
        let state = *open;
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, state, title)
        });
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
        self.list_row_hits(ui, row).response
    }

    /// [`Self::list_row`], with where its parts are.
    pub fn list_row_hits(&self, ui: &mut Ui, row: ListRow<'_>) -> RowHits {
        let t = &self.t;
        let s = &self.skin;
        let width = ui.available_width();
        let (rect, response) =
            ui.allocate_exact_size(sane(vec2(width, self.hit(t.row_height))), Sense::click());
        let mut hits = RowHits {
            response,
            fold: None,
            eye: None,
            grip: None,
            zoom: None,
            value: Rect::from_min_size(rect.right_top(), vec2(0.0, rect.height())),
        };
        if !ui.is_rect_visible(rect) {
            return hits;
        }
        let painter = ui.painter().clone();
        let hovered = hits.response.hovered() || row.shown_hovered;
        if row.selected {
            painter.rect_filled(rect, self.radius(), color(s.selection));
        } else if row.failed {
            painter.rect_filled(rect, self.radius(), color(s.failed_fill));
        } else if row.depends.is_some() {
            painter.rect_filled(rect, CornerRadius::ZERO, color(s.info_fill));
            painter.rect_filled(
                Rect::from_min_size(rect.min, vec2(t.info_bar_width, rect.height())),
                CornerRadius::ZERO,
                color(s.info_bar),
            );
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
        let side = t.nav_icon_size.min(rect.height());
        if row.grip {
            let place = Rect::from_center_size(
                pos2(rect.left() + pad * 0.6, rect.center().y),
                vec2(pad, side),
            );
            grip_dots(&painter, place, color(s.text_dim));
            hits.grip = Some(place.expand2(vec2(2.0, 0.0)));
        }
        // Room for the grip at the left, before the fold's chevron.
        let mut x = rect.left() + pad * 1.5 + f32::from(row.indent) * t.nav_icon_size;
        // The fold's chevron sits in the gutter before the icon.
        let chevron = side * 0.75;
        if let Some(open) = row.fold {
            let place = Rect::from_center_size(
                pos2(x + chevron * 0.5, rect.center().y),
                vec2(chevron, chevron),
            );
            let glyph = if open {
                Glyph::ChevronDown
            } else {
                Glyph::ChevronRight
            };
            glyphs::paint(&painter, glyph, place, color(s.text_dim), t);
            hits.fold = Some(place.expand(3.0));
        }
        x += chevron + t.nav_icon_gap * 0.5;
        if let Some(glyph) = row.icon {
            let ink = if row.failed {
                s.failed
            } else if row.selected {
                s.text
            } else {
                row.icon_ink.unwrap_or(s.text_dim)
            };
            glyphs::paint(
                &painter,
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
        let arrow = row.depends.map(|d| (d, right - side * 0.5));
        if arrow.is_some() {
            right -= side + t.nav_icon_gap;
        }
        if row.eye || row.hidden {
            let place =
                Rect::from_center_size(pos2(right - side * 0.5, rect.center().y), vec2(side, side));
            let ink = if row.hidden { s.faint } else { s.text_dim };
            glyphs::paint(&painter, Glyph::Eye, place, color(ink), t);
            if row.hidden {
                painter.line_segment(
                    [place.left_bottom(), place.right_top()],
                    Stroke::new(t.icon_stroke, color(ink)),
                );
            }
            hits.eye = Some(place.expand(2.0));
            right -= side + t.nav_icon_gap;
        }
        if row.zoom {
            let place =
                Rect::from_center_size(pos2(right - side * 0.5, rect.center().y), vec2(side, side));
            glyphs::paint(
                &painter,
                Glyph::Nav(crate::shell::icons::Icon::ZoomTo),
                place,
                color(s.text_dim),
                t,
            );
            hits.zoom = Some(place.expand(2.0));
            right -= side + t.nav_icon_gap;
        }
        if let Some((depends, cx)) = arrow {
            arrow_glyph(
                &painter,
                pos2(cx, rect.center().y),
                side * 0.8,
                depends == Depends::Uses,
                Stroke::new(t.icon_stroke + 0.2, color(s.info_ink)),
            );
        }
        let value_ink = if row.depends.is_some() && !row.failed {
            s.text_dim
        } else {
            value_ink
        };
        let mark_room = if row.mark.is_some() { side } else { 0.0 };
        // The name wins: a plain value that leaves the name too little
        // room is cut first (`revolute · 0.0° · dri…`), down to a share
        // of the row; a field's value keeps its width (it is typed into).
        let value_room = if row.value_field || row.value_hidden {
            f32::INFINITY
        } else {
            let total = (right - x - mark_room - t.nav_icon_gap).max(0.0);
            let name_width = self.galley(ui, row.name, self.font(), name_ink).size().x;
            (total - name_width).max(total * VALUE_LEAST_SHARE)
        };
        let value = {
            let ink = if row.value_field { s.text } else { value_ink };
            let mut job = egui::text::LayoutJob::simple_singleline(
                row.value.to_string(),
                self.font(),
                color(ink),
            );
            if value_room.is_finite() {
                job.wrap = egui::text::TextWrapping::truncate_at_width(value_room.max(1.0));
            }
            painter.layout_job(job)
        };
        let value_width = value.size().x;
        let field_pad = t.field_text_pad * 0.6;
        let value_rect = Rect::from_min_max(
            pos2(right - value_width, rect.top()),
            pos2(right, rect.bottom()),
        );
        hits.value = if row.value_field || row.value_hidden {
            value_rect.expand2(vec2(field_pad, -1.0))
        } else {
            value_rect
        };
        if row.value_field && !row.value_hidden && !row.value.is_empty() {
            painter.rect(
                hits.value,
                self.field_radius(),
                color(s.field),
                Stroke::new(1.0, color(s.field_stroke)),
                egui::StrokeKind::Inside,
            );
        }
        if !row.value_hidden {
            let ink = if row.value_field { s.text } else { value_ink };
            painter.galley(
                pos2(right - value_width, rect.center().y - value.size().y * 0.5),
                value,
                color(ink),
            );
        }
        // The name is cut where the value begins, leaving room for its mark.
        let left_of_value = if row.value_field || row.value_hidden {
            hits.value.left()
        } else {
            right - value_width
        };
        let room = (left_of_value - t.nav_icon_gap - x - mark_room).max(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(
            row.name.to_string(),
            self.font(),
            color(name_ink),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let name = painter.layout_job(job);
        let name_right = x + name.size().x;
        painter.galley(
            pos2(x, rect.center().y - name.size().y * 0.5),
            name,
            color(name_ink),
        );
        if let Some(mark) = row.mark {
            let at = pos2(name_right + t.nav_icon_gap + side * 0.3, rect.center().y);
            match mark {
                RowMark::Agent => star(&painter, at, side * 0.32, color(s.info_bar)),
                RowMark::Dot(ink) => {
                    painter.circle_filled(at, side * 0.2, color(ink));
                }
            }
        }
        hits.response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                true,
                row.selected,
                row.name,
            )
        });
        hits
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
            hit.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, tab.label)
            });
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

/// The six dots of a drag grip, in `place`.
pub(crate) fn grip_dots(painter: &egui::Painter, place: Rect, ink: egui::Color32) {
    let r = (place.width().min(place.height()) * 0.09).clamp(0.8, 2.5);
    let dx = place.width() * 0.22;
    let dy = place.height() * 0.24;
    for i in -1..=1 {
        for j in [-1.0, 1.0] {
            painter.circle_filled(
                pos2(place.center().x + j * dx, place.center().y + i as f32 * dy),
                r,
                ink,
            );
        }
    }
}

/// An arrow up (or down) centred at `at`, `size` long.
fn arrow_glyph(painter: &egui::Painter, at: egui::Pos2, size: f32, up: bool, stroke: Stroke) {
    let h = size * 0.5;
    let dir = if up { -1.0 } else { 1.0 };
    let tip = pos2(at.x, at.y + dir * h);
    painter.line_segment([pos2(at.x, at.y - dir * h), tip], stroke);
    let w = size * 0.32;
    painter.line_segment([tip, pos2(at.x - w, tip.y - dir * w)], stroke);
    painter.line_segment([tip, pos2(at.x + w, tip.y - dir * w)], stroke);
}

/// A four-pointed star of radius `r` at `at`: the agent's mark, two
/// slim diamonds crossed (each convex, so each fills cleanly).
fn star(painter: &egui::Painter, at: egui::Pos2, r: f32, ink: egui::Color32) {
    if !(r.is_finite() && r > 0.0) {
        return;
    }
    let w = r * 0.28;
    for (dx, dy) in [(0.0, 1.0), (1.0, 0.0)] {
        let (px, py) = (dy * w, dx * w);
        painter.add(egui::Shape::convex_polygon(
            vec![
                pos2(at.x - dx * r, at.y - dy * r),
                pos2(at.x + px, at.y - py),
                pos2(at.x + dx * r, at.y + dy * r),
                pos2(at.x - px, at.y + py),
            ],
            ink,
            Stroke::NONE,
        ));
    }
}
