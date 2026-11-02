//! What the kit lays over the window: tooltips, popovers with a pointer,
//! menus and context menus (board c2 of the plan). Popovers and menus are
//! dark in both appearances; in the dark one they sit a step lighter than
//! the panel (`popover_fill`).

use super::glyphs::{self, Glyph};
use super::{Ground, Kit, centred_left, sane};
use crate::theme::{color, color_alpha};
use egui::{
    CornerRadius, Frame, Id, InnerResponse, Margin, Pos2, Rect, Response, Sense, Shadow, Shape,
    Stroke, Ui, pos2, vec2,
};

/// An item of a menu.
#[derive(Clone, Copy, Debug)]
pub struct MenuItem<'a> {
    pub label: &'a str,
    pub shortcut: Option<&'a str>,
    pub icon: Option<Glyph>,
    pub checked: bool,
    pub danger: bool,
    pub submenu: bool,
    pub enabled: bool,
    /// Drawn as under the pointer (the gallery shows the state).
    pub shown_hovered: bool,
}

impl<'a> MenuItem<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            shortcut: None,
            icon: None,
            checked: false,
            danger: false,
            submenu: false,
            enabled: true,
            shown_hovered: false,
        }
    }
    #[must_use]
    pub fn shortcut(mut self, keys: &'a str) -> Self {
        self.shortcut = Some(keys);
        self
    }
    #[must_use]
    pub fn icon(mut self, icon: Glyph) -> Self {
        self.icon = Some(icon);
        self
    }
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    #[must_use]
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    #[must_use]
    pub fn submenu(mut self) -> Self {
        self.submenu = true;
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

impl Kit {
    fn shadow(&self) -> Shadow {
        Shadow {
            offset: [0, self.t.shadow_drop],
            blur: self.t.shadow_blur,
            spread: 0,
            color: color_alpha(self.t.view_cube_shadow, self.t.shadow_opacity),
        }
    }

    fn rim(&self) -> Stroke {
        Stroke::new(
            1.0,
            color_alpha(self.t.plate_rim, self.t.popover_rim_opacity),
        )
    }

    /// A tooltip's box at the place the layout is at: white text on the
    /// caption's near-black, wrapped at `max_width`.
    pub fn tooltip_box(&self, ui: &mut Ui, text: &str, max_width: f32) -> Response {
        let t = &self.t;
        let pad = t.field_text_pad;
        Frame::new()
            .fill(color(t.notice_fill))
            .corner_radius(CornerRadius::same(t.notice_radius))
            .inner_margin(Margin::symmetric(pad as i8, (pad * 0.6) as i8))
            .shadow(self.shadow())
            .show(ui, |ui| {
                let job = egui::text::LayoutJob::simple(
                    text.to_string(),
                    self.font(),
                    color(t.notice_text),
                    (max_width - 2.0 * pad).max(t.control_height),
                );
                let galley = ui.painter().layout_job(job);
                let (rect, _) = ui.allocate_exact_size(sane(galley.size()), Sense::hover());
                ui.painter().galley(rect.min, galley, color(t.notice_text));
            })
            .response
    }

    /// Shows `text` as a tooltip under the pointer once it has rested on
    /// `response` for egui's tooltip delay.
    pub fn hover_tip(&self, ui: &Ui, response: &Response, text: &str) {
        if !response.hovered() {
            return;
        }
        let delay = ui.style().interaction.tooltip_delay;
        let rested = ui.input(|i| i.pointer.time_since_last_movement());
        if rested < delay {
            ui.ctx().request_repaint();
            return;
        }
        let Some(at) = ui.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let kit = *self;
        let width = self.t.dialog_width * 0.75;
        egui::Area::new(response.id.with("kit-tip"))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .fixed_pos(at + vec2(0.0, self.t.nav_icon_size + 4.0))
            .constrain(true)
            .show(ui.ctx(), |ui| {
                kit.tooltip_box(ui, text, width);
            });
    }

    /// A popover's body `width` wide with its pointer `pointer_x` points
    /// from its left edge reaching up toward what opened it: the plate
    /// (`popover_fill`), its faint rim, `nav_radius` round.
    pub fn popover_frame<R>(
        &self,
        ui: &mut Ui,
        width: f32,
        pointer_x: Option<f32>,
        add: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<R> {
        let t = self.t;
        let plate = Kit::new(&t, Ground::Plate);
        let tip = t.popover_pointer;
        if pointer_x.is_some() {
            ui.add_space(tip);
        }
        let pad = t.inset + 2.0;
        let inner = Frame::new()
            .fill(color(t.popover_fill))
            .stroke(plate.rim())
            .corner_radius(CornerRadius::same(t.nav_radius))
            .inner_margin(Margin::same(pad as i8))
            .shadow(plate.shadow())
            .show(ui, |ui| {
                ui.set_width((width - 2.0 * pad).max(0.0));
                add(ui)
            });
        if let Some(x) = pointer_x {
            let rect = inner.response.rect;
            let half = t.popover_pointer_width * 0.5;
            let x = (rect.left() + x).clamp(
                rect.left() + f32::from(t.nav_radius) + half,
                (rect.right() - f32::from(t.nav_radius) - half).max(rect.left()),
            );
            let top = rect.top();
            let points = vec![
                pos2(x - half, top + 0.5),
                pos2(x, top - tip),
                pos2(x + half, top + 0.5),
            ];
            ui.painter().add(Shape::convex_polygon(
                points.clone(),
                color(t.popover_fill),
                Stroke::NONE,
            ));
            ui.painter().add(Shape::line(points, plate.rim()));
        }
        inner
    }

    /// A popover hanging under `anchor` (the control that opened it): its
    /// right edge in line with the anchor's, within the screen, its
    /// pointer at the anchor's middle.
    pub fn popover<R>(
        &self,
        ctx: &egui::Context,
        id: Id,
        anchor: Rect,
        width: f32,
        add: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<R> {
        let screen = ctx.content_rect();
        let left = (anchor.right() - width)
            .min(screen.right() - width)
            .max(screen.left());
        let at = pos2(left, anchor.bottom() + self.t.nav_spacing);
        let pointer = anchor.center().x - left;
        let kit = *self;
        let shown = egui::Area::new(id)
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ctx, |ui| kit.popover_frame(ui, width, Some(pointer), add));
        InnerResponse::new(shown.inner.inner, shown.response)
    }

    /// A menu's plate, `width` wide at least.
    pub fn menu_frame<R>(
        &self,
        ui: &mut Ui,
        width: f32,
        add: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<R> {
        let t = self.t;
        let plate = Kit::new(&t, Ground::Plate);
        let pad = t.nav_padding;
        Frame::new()
            .fill(color(t.popover_fill))
            .stroke(plate.rim())
            .corner_radius(CornerRadius::same(t.menu_radius))
            .inner_margin(Margin::same(pad as i8))
            .shadow(plate.shadow())
            .show(ui, |ui| {
                ui.set_width((width.max(t.menu_width) - 2.0 * pad).max(0.0));
                ui.spacing_mut().item_spacing.y = 0.0;
                add(ui)
            })
    }

    /// An item of a menu (on the plate: `self` should be a plate kit):
    /// a check or an icon, the label, the keys or a chevron at the right.
    pub fn menu_item(&self, ui: &mut Ui, item: MenuItem<'_>) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let width = ui.available_width();
        let sense = if item.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) =
            ui.allocate_exact_size(sane(vec2(width, self.hit(t.menu_item_height))), sense);
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let painter = ui.painter();
        if item.enabled && (response.hovered() || item.shown_hovered) {
            painter.rect_filled(
                rect,
                CornerRadius::same(t.menu_radius.saturating_sub(t.nav_padding as u8)),
                color(s.hover),
            );
        }
        let ink = if !item.enabled {
            s.faint
        } else if item.danger {
            s.failed
        } else {
            s.text
        };
        let pad = t.field_text_pad + 2.0 - t.nav_padding;
        let mut x = rect.left() + pad;
        let side = t.nav_icon_size;
        let mark = if item.checked {
            Some((Glyph::Check, t.accent))
        } else {
            item.icon.map(|icon| (icon, s.text_dim))
        };
        if let Some((glyph, mark_ink)) = mark {
            glyphs::paint(
                painter,
                glyph,
                centred_left(
                    Rect::from_min_max(pos2(x, rect.top()), rect.max),
                    vec2(side, side),
                ),
                color(mark_ink),
                t,
            );
            x += side + t.nav_icon_gap;
        }
        let label = self.galley(ui, item.label, self.font(), ink);
        painter.galley(
            pos2(x, rect.center().y - label.size().y * 0.5),
            label,
            color(ink),
        );
        let right = rect.right() - pad;
        if item.submenu {
            let side = side * 0.75;
            glyphs::paint(
                painter,
                Glyph::ChevronRight,
                Rect::from_center_size(pos2(right - side * 0.5, rect.center().y), vec2(side, side)),
                color(s.text_dim),
                t,
            );
        } else if let Some(keys) = item.shortcut {
            let keys = self.galley(ui, keys, self.font(), s.text_dim);
            painter.galley(
                pos2(right - keys.size().x, rect.center().y - keys.size().y * 0.5),
                keys,
                color(s.text_dim),
            );
        }
        response
    }

    /// A rule between groups of a menu.
    pub fn menu_separator(&self, ui: &mut Ui) {
        let width = ui.available_width();
        let gap = self.t.nav_padding;
        let (rect, _) = ui.allocate_exact_size(sane(vec2(width, 2.0 * gap + 1.0)), Sense::hover());
        ui.painter().line_segment(
            [
                pos2(rect.left() + gap, rect.center().y),
                pos2(rect.right() - gap, rect.center().y),
            ],
            Stroke::new(1.0, color(self.skin.divider)),
        );
    }

    /// A context menu for `response`: opened by a secondary click where it
    /// was, closed when `add` returns true (an item was chosen), by a click
    /// elsewhere or by Escape. Returns whether it is open.
    pub fn context_menu(
        &self,
        ui: &Ui,
        response: &Response,
        add: impl FnOnce(&Kit, &mut Ui) -> bool,
    ) -> bool {
        let id = response.id.with("kit-context");
        let mut at: Option<Pos2> = ui.data(|d| d.get_temp(id));
        if response.secondary_clicked() {
            at = response
                .interact_pointer_pos()
                .or(Some(response.rect.center()));
        }
        let Some(pos) = at else {
            return false;
        };
        let plate = Kit::new(&self.t, Ground::Plate);
        let shown = egui::Area::new(id.with("area"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .constrain(true)
            .show(ui.ctx(), |ui| {
                plate
                    .menu_frame(ui, plate.t.menu_width, |ui| add(&plate, ui))
                    .inner
            });
        let chosen = shown.inner;
        let elsewhere = ui.input(|i| i.pointer.any_pressed())
            && !shown.response.contains_pointer()
            && !response.secondary_clicked();
        let open = !(chosen || elsewhere || ui.input(|i| i.key_pressed(egui::Key::Escape)));
        ui.data_mut(|d| {
            if open {
                d.insert_temp(id, pos);
            } else {
                d.remove::<Pos2>(id);
            }
        });
        open
    }
}
