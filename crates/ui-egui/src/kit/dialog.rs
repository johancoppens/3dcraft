//! The kit's dialog (boards s4–s6 of the plan): one layout for every
//! dialog of the window. A title of `dialog_title_font` (17, Semi Bold)
//! with an optional icon before it and a close × at the right; the body;
//! then the footer, with quiet *Copy* actions on the left and the buttons
//! on the right, the amber one last. Always on the panel's ground (light
//! or dark with the appearance).

use super::glyphs::{self, Glyph};
use super::{Ground, Kit, centred_left, sane};
use crate::theme::{Rgb, color, color_alpha};
use egui::{CornerRadius, Frame, Id, Margin, Rect, Sense, Shadow, Ui, pos2, vec2};

/// What a dialog is: its id, title, icon and width.
#[derive(Clone, Copy, Debug)]
pub struct Dialog<'a> {
    pub id: Id,
    pub title: &'a str,
    /// An icon before the title, in its own ink (a warning in the
    /// failure's red, an agent's question in the info ink).
    pub icon: Option<(Glyph, Rgb)>,
    /// How wide it is, in points, at most the window less a margin
    /// (`Tokens::dialog_width` unless set).
    pub width: f32,
    /// Over a dimmed window, taking every click (`egui::Modal`), or
    /// floating in the middle without one.
    pub modal: bool,
    /// A close × beside the title.
    pub closable: bool,
    /// A bottom sheet across the window (a phone, board m7) rather than a
    /// box in its middle.
    pub sheet: bool,
}

impl<'a> Dialog<'a> {
    pub fn new(id: Id, title: &'a str) -> Self {
        Self {
            id,
            title,
            icon: None,
            width: f32::INFINITY,
            modal: true,
            closable: true,
            sheet: false,
        }
    }
    #[must_use]
    pub fn icon(mut self, glyph: Glyph, ink: Rgb) -> Self {
        self.icon = Some((glyph, ink));
        self
    }
    #[must_use]
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
    #[must_use]
    pub fn modal(mut self, modal: bool) -> Self {
        self.modal = modal;
        self
    }
    #[must_use]
    pub fn sheet(mut self, sheet: bool) -> Self {
        self.sheet = sheet;
        self
    }
    #[must_use]
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }
}

/// What a dialog's frame answered: the body's own answer, whether the ×,
/// Escape or (for a modal one) a click beside it asks to close it, and
/// where it was drawn.
pub struct DialogShown<R> {
    pub inner: R,
    pub close: bool,
    pub rect: Rect,
    /// Where the × was, when there is one.
    pub close_rect: Option<Rect>,
}

impl Kit {
    /// The width a dialog gets in a window `screen` wide: what it asks
    /// for, at most the window less `inset` on each side, never less than
    /// a narrow phone's room.
    pub fn dialog_width(&self, wanted: f32, screen: f32) -> f32 {
        let room = screen - 2.0 * self.t.inset;
        let room = if room.is_finite() { room } else { wanted };
        let wanted = if wanted.is_finite() {
            wanted
        } else {
            self.t.dialog_width
        };
        wanted
            .min(room)
            .max(self.t.dialog_min_width.min(room.max(0.0)))
    }

    /// A dialog in the middle of the window: its title row, then
    /// `content`, which ends with [`Kit::dialog_footer`]. Always on the
    /// panel's ground, whatever `self` stands on.
    pub fn dialog<R>(
        &self,
        ctx: &egui::Context,
        dialog: Dialog<'_>,
        content: impl FnOnce(&Kit, &mut Ui) -> R,
    ) -> DialogShown<R> {
        let kit = Kit::new(&self.t, Ground::Panel);
        let t = kit.t;
        let s = kit.skin;
        let pad = t.dialog_padding;
        let screen = ctx.content_rect().width();
        let width = if dialog.sheet {
            screen
        } else {
            kit.dialog_width(dialog.width, screen)
        };
        let round = if dialog.sheet {
            let r = t.dialog_radius.saturating_mul(2);
            CornerRadius {
                nw: r,
                ne: r,
                sw: 0,
                se: 0,
            }
        } else {
            CornerRadius::same(t.dialog_radius)
        };
        let frame = Frame::new()
            .fill(color(s.fill))
            .stroke(egui::Stroke::new(1.0, color(s.field_stroke)))
            .corner_radius(round)
            .inner_margin(Margin::same(pad.clamp(0.0, 64.0) as i8))
            .shadow(Shadow {
                offset: [0, t.shadow_drop.saturating_mul(2)],
                blur: t.shadow_blur.saturating_mul(2),
                spread: 0,
                color: color_alpha(t.view_cube_shadow, t.shadow_opacity),
            });
        let mut close_rect = None;
        let mut close_clicked = false;
        let content = |ui: &mut Ui| {
            ui.set_width((width - 2.0 * pad).max(0.0));
            if dialog.sheet {
                kit.grabber(ui);
            }
            ui.spacing_mut().item_spacing.y = t.nav_spacing * 2.0;
            // The title row: icon, title, ×.
            let title_font = egui::FontId::new(t.dialog_title_font, heading_family(ui));
            let galley = kit.galley(ui, dialog.title, title_font, s.text);
            let height = galley.size().y.max(t.control_height);
            let (row, _) =
                ui.allocate_exact_size(sane(vec2(ui.available_width(), height)), Sense::hover());
            let mut x = row.left();
            if let Some((glyph, ink)) = dialog.icon {
                let side = t.dialog_icon;
                glyphs::paint(
                    ui.painter(),
                    glyph,
                    centred_left(
                        Rect::from_min_max(pos2(x, row.top()), row.max),
                        vec2(side, side),
                    ),
                    color(ink),
                    &t,
                );
                x += side + t.nav_icon_gap;
            }
            let room = (row.right() - x - if dialog.closable { height } else { 0.0 }).max(0.0);
            let shown = if galley.size().x > room {
                let mut job = egui::text::LayoutJob::simple_singleline(
                    dialog.title.to_string(),
                    egui::FontId::new(t.dialog_title_font, heading_family(ui)),
                    color(s.text),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(room.max(1.0));
                ui.painter().layout_job(job)
            } else {
                galley
            };
            ui.painter().galley(
                pos2(x, row.center().y - shown.size().y * 0.5),
                shown,
                color(s.text),
            );
            if dialog.closable {
                let place = Rect::from_center_size(
                    pos2(row.right() - height * 0.5, row.center().y),
                    vec2(height, height),
                );
                let response = ui
                    .interact(place, dialog.id.with("close"), Sense::click())
                    .on_hover_text("Close");
                if response.hovered() {
                    ui.painter()
                        .rect_filled(place, kit.radius(), color(s.hover));
                }
                let side = t.nav_icon_size * 0.85;
                glyphs::paint(
                    ui.painter(),
                    Glyph::Close,
                    Rect::from_center_size(place.center(), vec2(side, side)),
                    color(if response.hovered() {
                        s.text
                    } else {
                        s.text_dim
                    }),
                    &t,
                );
                close_rect = Some(place);
                close_clicked = response.clicked();
            }
            content(&kit, ui)
        };
        let (inner, rect, beside) = if dialog.modal {
            let mut modal = egui::Modal::new(dialog.id);
            if dialog.sheet {
                modal = modal.area(
                    egui::Modal::default_area(dialog.id)
                        .anchor(egui::Align2::CENTER_BOTTOM, egui::Vec2::ZERO),
                );
            }
            let shown = modal
                .frame(frame)
                .backdrop_color(color_alpha(t.view_cube_shadow, t.shadow_opacity))
                .show(ctx, content);
            let beside = shown.should_close();
            (shown.inner, shown.response.rect, beside)
        } else {
            let shown = egui::Area::new(dialog.id)
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .constrain(true)
                .show(ctx, |ui| frame.show(ui, content).inner);
            let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
            (shown.inner, shown.response.rect, escape)
        };
        DialogShown {
            inner,
            close: close_clicked || beside,
            rect,
            close_rect,
        }
    }

    /// A dialog's footer: a row whose items `add` lays out from the left
    /// (the quiet Copy actions), then, inside [`Kit::footer_right`], the
    /// buttons from the right.
    pub fn dialog_footer<R>(&self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
        ui.add_space(self.t.nav_spacing * 2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = self.t.nav_gap;
            add(ui)
        })
        .inner
    }

    /// The buttons at the right of a footer, laid out from the right: add
    /// the amber one first, so it is the last.
    pub fn footer_right<R>(&self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add)
            .inner
    }

    /// A label and its value in a dialog's two columns (About): the
    /// label dim, `label_width` wide.
    pub fn dialog_row(
        &self,
        ui: &mut Ui,
        label: &str,
        label_width: f32,
        value: impl FnOnce(&mut Ui),
    ) {
        let s = self.skin;
        ui.horizontal_top(|ui| {
            let galley = self.galley(ui, label, self.font(), s.text_dim);
            let (rect, _) = ui.allocate_exact_size(
                sane(vec2(
                    label_width,
                    galley.size().y.max(self.t.label_row_height),
                )),
                Sense::hover(),
            );
            ui.painter().galley(
                pos2(rect.left(), rect.center().y - galley.size().y * 0.5),
                galley,
                color(s.text_dim),
            );
            ui.vertical(value);
        });
    }

    /// Text in a dialog's body, wrapped at the width there is: the text's
    /// ink, or the dim one.
    pub fn paragraph(&self, ui: &mut Ui, text: &str, dim: bool) -> egui::Response {
        let ink = if dim {
            self.skin.text_dim
        } else {
            self.skin.text
        };
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .font(self.font())
                    .color(color(ink)),
            )
            .wrap(),
        )
    }

    /// A line of code or a path on the code ground (`menu_fill` on a
    /// panel, `plate_field` on a plate), in the mono font, cut at its
    /// edge with the whole of it on hover; the `program` it starts with in
    /// the info accent's light tint.
    pub fn code_line(&self, ui: &mut Ui, program: &str, text: &str) -> egui::Response {
        let t = self.t;
        let (fill, ink) = match self.ground {
            Ground::Panel => (t.menu_fill, t.plate_text),
            Ground::Plate => (t.plate_field, t.plate_text),
        };
        let (head, rest) = match text.strip_prefix(program) {
            Some(rest) if !program.is_empty() => (program, rest),
            _ => ("", text),
        };
        let mut job = egui::text::LayoutJob::default();
        for (part, part_ink) in [(head, t.info_code), (rest, ink)] {
            job.append(
                part,
                0.0,
                egui::TextFormat::simple(self.mono_font(), color(part_ink)),
            );
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width(
            (ui.available_width() - 2.0 * t.field_text_pad).max(1.0),
        );
        Frame::new()
            .fill(color(fill))
            .stroke(match self.ground {
                Ground::Plate => egui::Stroke::new(1.0, color(t.plate_field_stroke)),
                Ground::Panel => egui::Stroke::NONE,
            })
            .corner_radius(self.radius())
            .inner_margin(Margin::symmetric(
                t.field_text_pad.clamp(0.0, 32.0) as i8,
                ((t.control_height - t.secondary_font) * 0.5).clamp(0.0, 16.0) as i8,
            ))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(job).truncate())
            })
            .inner
            .on_hover_text(text)
    }
}

/// The headings' family where the fonts in force have one (Semi Bold).
fn heading_family(ui: &Ui) -> egui::FontFamily {
    ui.style()
        .text_styles
        .get(&egui::TextStyle::Heading)
        .map_or(egui::FontFamily::Proportional, |font| font.family.clone())
}
