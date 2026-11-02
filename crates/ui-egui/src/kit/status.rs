//! The kit's states: the agent's status pill, banners, progress bars and
//! the spinner (boards c1 and c2). Progress is information, so it is drawn
//! in the info accent; a failure is red, what waits for a decision amber.

use super::glyphs::{self, Glyph};
use super::{Kit, centred_left, sane};
use crate::theme::{Rgb, color, color_alpha};
use craft3d_math::Portable;
use egui::{CornerRadius, Frame, Margin, Rect, Response, Sense, Shape, Stroke, Ui, pos2, vec2};

/// The agent's state, as its pill on the status bar shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillState {
    /// Agents may not connect.
    Off,
    /// Listening, nobody yet.
    Waiting,
    Connected,
    /// Connected and running a command: a turning arc.
    Working,
    /// Waits for the person (a folder, a consent): amber.
    Asks,
    /// The connection was lost: red until dismissed.
    Lost,
}

impl PillState {
    pub const ALL: [PillState; 6] = [
        PillState::Off,
        PillState::Waiting,
        PillState::Connected,
        PillState::Working,
        PillState::Asks,
        PillState::Lost,
    ];
}

/// A banner's meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BannerKind {
    Error,
    Info,
    /// Waits for a decision (the recovery bar).
    Attention,
}

impl Kit {
    /// The agent's pill on the status bar (chrome: the same in both
    /// appearances). A working pill asks for the next frame, its arc turns.
    pub fn pill(&self, ui: &mut Ui, state: PillState, text: &str) -> Response {
        let t = &self.t;
        let h = t.pill_height;
        let (fill, rim, mark, ink) = match state {
            PillState::Off => (None, Some(t.pill_off_rim), None, t.chrome_text_dim),
            PillState::Waiting => (
                None,
                Some(t.pill_connected),
                Some(t.pill_wait_text),
                t.pill_wait_text,
            ),
            PillState::Connected => (
                Some(t.pill_connected),
                None,
                Some(t.pill_wait_text),
                t.chrome_text,
            ),
            PillState::Working => (
                Some(t.pill_connected),
                None,
                Some(t.pill_wait_text),
                t.chrome_text,
            ),
            PillState::Asks => (
                Some(t.plate_attention_fill),
                Some(t.accent),
                Some(t.accent),
                t.pill_asks_text,
            ),
            PillState::Lost => (
                None,
                Some(t.chrome_failed),
                Some(t.chrome_failed),
                t.chrome_failed,
            ),
        };
        let galley = self.galley(ui, text, self.font(), ink);
        let dot = h * 0.18;
        let pad = h * 0.5;
        let mark_room = if mark.is_some() {
            2.0 * dot + t.nav_icon_gap
        } else {
            0.0
        };
        let width = pad * 2.0 + mark_room + galley.size().x;
        let (outer, response) = ui.allocate_exact_size(
            sane(vec2(width.max(t.hit_min), self.hit(h))),
            Sense::click(),
        );
        if !ui.is_rect_visible(outer) {
            return response;
        }
        let rect = Rect::from_center_size(outer.center(), vec2(width, h));
        let painter = ui.painter();
        let radius = CornerRadius::same(super::controls::radius_of(h));
        if let Some(fill) = fill {
            painter.rect_filled(rect, radius, color(fill));
        }
        if let Some(rim) = rim {
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, color(rim)),
                egui::StrokeKind::Inside,
            );
        }
        let mut x = rect.left() + pad;
        if let Some(mark) = mark {
            let c = pos2(x + dot, rect.center().y);
            match state {
                PillState::Waiting => {
                    painter.circle_stroke(c, dot, Stroke::new(1.5, color(mark)));
                }
                PillState::Working => {
                    painter.circle_stroke(c, dot, Stroke::new(1.5, color_alpha(mark, 90)));
                    let turn = (ui.input(|i| i.time) * 4.0) as f32;
                    painter.add(arc(c, dot, turn, turn + 4.0, Stroke::new(1.5, color(mark))));
                    ui.ctx().request_repaint();
                }
                _ => {
                    painter.circle_filled(c, dot, color(mark));
                }
            }
            x += 2.0 * dot + t.nav_icon_gap;
        }
        painter.galley(
            pos2(x, rect.center().y - galley.size().y * 0.5),
            galley,
            color(ink),
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
        response
    }

    /// A banner across the width there is: the bar of its kind at the
    /// left, an icon, the title in its ink, the body, then `actions` (kit
    /// buttons) in a row.
    pub fn banner(
        &self,
        ui: &mut Ui,
        kind: BannerKind,
        title: Option<&str>,
        body: &str,
        actions: impl FnOnce(&Kit, &mut Ui),
    ) -> Response {
        let t = self.t;
        let s = self.skin;
        let (fill, bar, icon, icon_ink, title_ink) = match kind {
            BannerKind::Error => (s.failed_fill, s.failed, Glyph::Warning, s.failed, s.failed),
            BannerKind::Info => (s.info_fill, s.info_bar, Glyph::Info, s.info_ink, s.text),
            BannerKind::Attention => (
                s.attention_fill,
                s.attention,
                Glyph::Warning,
                s.attention,
                s.text,
            ),
        };
        let pad = t.inset + 2.0;
        let side = t.nav_icon_size;
        let kit = *self;
        let shown = Frame::new()
            .fill(color(fill))
            .corner_radius(CornerRadius::same(t.notice_radius))
            .inner_margin(Margin {
                left: (pad + t.info_bar_width) as i8,
                right: pad as i8,
                top: (pad * 0.6) as i8,
                bottom: (pad * 0.6) as i8,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    let (place, _) =
                        ui.allocate_exact_size(vec2(side, t.row_height), Sense::hover());
                    glyphs::paint(
                        ui.painter(),
                        icon,
                        centred_left(place, vec2(side * 0.9, side * 0.9)),
                        color(icon_ink),
                        &t,
                    );
                    ui.add_space(t.nav_icon_gap * 0.5);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if let Some(title) = title {
                            let galley = kit.galley(ui, title, kit.medium_font(ui), title_ink);
                            let (rect, _) = ui.allocate_exact_size(
                                sane(vec2(galley.size().x, t.row_height)),
                                Sense::hover(),
                            );
                            ui.painter().galley(
                                pos2(rect.left(), rect.center().y - galley.size().y * 0.5),
                                galley,
                                color(title_ink),
                            );
                        }
                        let job = egui::text::LayoutJob::simple(
                            body.to_string(),
                            kit.font(),
                            color(s.text),
                            ui.available_width(),
                        );
                        let galley = ui.painter().layout_job(job);
                        let height = galley.size().y.max(t.row_height);
                        let (rect, _) = ui.allocate_exact_size(
                            sane(vec2(galley.size().x, height)),
                            Sense::hover(),
                        );
                        ui.painter().galley(
                            pos2(rect.left(), rect.center().y - galley.size().y * 0.5),
                            galley,
                            color(s.text),
                        );
                        ui.horizontal(|ui| actions(&kit, ui));
                    });
                });
            });
        let rect = shown.response.rect;
        let r = t.notice_radius;
        ui.painter().rect_filled(
            Rect::from_min_max(
                rect.min,
                pos2(rect.left() + t.info_bar_width, rect.bottom()),
            ),
            CornerRadius {
                nw: r,
                sw: r,
                ne: 0,
                se: 0,
            },
            color(bar),
        );
        shown.response
    }

    /// A progress bar `width` wide: what it does on a line, the bar under
    /// it, `fraction` of it done in the info accent.
    pub fn progress(&self, ui: &mut Ui, text: &str, fraction: f32, width: f32) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let width = width.min(ui.available_width());
        let galley = self.galley(ui, text, self.font(), s.text);
        let gap = t.nav_spacing * 2.0;
        let (rect, response) = ui.allocate_exact_size(
            sane(vec2(width, t.row_height + gap + t.progress_height)),
            Sense::hover(),
        );
        let painter = ui.painter();
        painter.galley(
            pos2(
                rect.left(),
                rect.top() + (t.row_height - galley.size().y) * 0.5,
            ),
            galley,
            color(s.text),
        );
        let bar = Rect::from_min_size(
            pos2(rect.left(), rect.bottom() - t.progress_height),
            vec2(rect.width(), t.progress_height),
        );
        let r = CornerRadius::same(super::controls::radius_of(t.progress_height));
        painter.rect_filled(bar, r, color(s.track));
        let f = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        painter.rect_filled(
            Rect::from_min_size(bar.min, vec2(bar.width() * f, bar.height())),
            r,
            color(s.info_bar),
        );
        response
    }

    /// Work without a known end: a turning arc in the info accent, then
    /// what is being done. Asks for the next frame while it shows.
    pub fn spinner(&self, ui: &mut Ui, text: &str) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let galley = self.galley(ui, text, self.font(), s.text);
        let side = t.work_spinner.max(4.0);
        let (rect, response) = ui.allocate_exact_size(
            sane(vec2(side + t.nav_icon_gap + galley.size().x, t.row_height)),
            Sense::hover(),
        );
        let c = pos2(rect.left() + side * 0.5, rect.center().y);
        let r = side * 0.5;
        ui.painter()
            .circle_stroke(c, r, Stroke::new(1.5, color(s.track)));
        let turn = (ui.input(|i| i.time) * 4.0) as f32;
        ui.painter().add(arc(
            c,
            r,
            turn,
            turn + 2.0,
            Stroke::new(1.5, color(s.info_bar)),
        ));
        ui.ctx().request_repaint();
        ui.painter().galley(
            pos2(
                rect.left() + side + t.nav_icon_gap,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            color(s.text),
        );
        response
    }
}

/// An arc of a circle from angle `a0` to `a1` (radians).
fn arc(c: egui::Pos2, r: f32, a0: f32, a1: f32, stroke: Stroke) -> Shape {
    let n = 16;
    let points = (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            c + vec2(a.pcos(), a.psin()) * r
        })
        .collect();
    Shape::line(points, stroke)
}

/// The fill and ink of a banner kind on the panels, for the tests.
pub fn banner_inks(kit: &Kit, kind: BannerKind) -> (Rgb, Rgb) {
    let s = &kit.skin;
    match kind {
        BannerKind::Error => (s.failed_fill, s.failed),
        BannerKind::Info => (s.info_fill, s.info_ink),
        BannerKind::Attention => (s.attention_fill, s.text),
    }
}
