//! The relation of two things selected, in the view (the developer's
//! design of 2026-11-17, step B): one thin dimension line between them,
//! in the accent over the section arrow's dark hairline, with heads at
//! its ends and its number on a small plate at its middle; an angle as
//! an arc about its corner. It is the side panel's measurement
//! (`measure.between`) drawn: gone with the selection, and with Escape,
//! which clears it. A click on the number copies it with its unit. Drawn
//! by egui over the picture, as the section's handle is: neither
//! renderer draws it, and a picture without two things selected is as
//! before.

use super::Shell;
use crate::camera::Projector;
use crate::theme::{color, color_alpha};
use craft3d_math::{Portable, Vec3};
use egui::{Pos2, Rect, pos2, vec2};

/// Where the dimension was drawn, in the window's points.
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionPlace {
    /// The line's ends on the screen; for an angle the points of the arc
    /// at its ends.
    pub from: Pos2,
    pub to: Pos2,
    /// An angle's corner on the screen.
    pub at: Option<Pos2>,
    /// The number's plate.
    pub number: Rect,
}

/// How far the plate keeps from the line, and its padding, in points.
const PAD: egui::Vec2 = vec2(6.0, 3.0);

/// A point of space on the screen, in the viewport `rect`.
fn on_screen(projector: &Projector, rect: Rect, p: [f64; 3]) -> Option<Pos2> {
    let seen = projector.project(Vec3::new(p[0], p[1], p[2]))?;
    let at = pos2(rect.left() + seen.x as f32, rect.top() + seen.y as f32);
    (at.x.is_finite() && at.y.is_finite()).then_some(at)
}

impl Shell {
    /// Where the dimension of the two things selected was drawn last.
    pub fn dimension_place(&self) -> Option<&DimensionPlace> {
        self.dimension.as_ref()
    }

    /// A click on the dimension's number copies it with its unit: true
    /// when it was there, so it selects nothing.
    pub(super) fn dimension_click(&mut self, ui: &egui::Ui, response: &egui::Response) -> bool {
        let (Some(place), Some(at)) = (&self.dimension, response.interact_pointer_pos()) else {
            return false;
        };
        if !(response.clicked() && place.number.expand(2.0).contains(at)) {
            return false;
        }
        if let Some(text) = self.measurement().map(|m| m.text.clone()) {
            ui.ctx().copy_text(text.clone());
            self.side.measured = Some(format!("Copied {text}"));
        }
        true
    }

    /// Paints the dimension of the two things selected over the viewport
    /// `rect`, when they have one.
    pub(super) fn paint_dimension(&mut self, ui: &egui::Ui, rect: Rect) {
        self.dimension = None;
        let Some(m) = self.measurement().cloned() else {
            return;
        };
        if m.value.is_none() || m.relation.is_empty() {
            return;
        }
        let Some(projector) = Projector::new(
            &self.state.camera.to_render(),
            f64::from(rect.width()),
            f64::from(rect.height()),
        ) else {
            return;
        };
        let tokens = self.tokens;
        let accent = color(tokens.accent);
        let halo = color_alpha(tokens.plate_fill, tokens.section_halo_opacity);
        let width = tokens.measure_line_width;
        let painter = ui.painter_at(rect);
        let head = tokens.measure_head;
        // A head at `tip` pointing along `way` (a unit vector).
        let head_at = |tip: Pos2, way: egui::Vec2| {
            let across = vec2(-way.y, way.x);
            let neck = tip - way * head;
            vec![
                tip,
                neck + across * (head * 0.38),
                neck - across * (head * 0.38),
            ]
        };
        let (from, to, at, middle, outward) = match m.at {
            // An angle: an arc about its corner from one side to the other.
            Some(corner) => {
                let (Some(c), Some(a), Some(b)) = (
                    on_screen(&projector, rect, corner),
                    on_screen(&projector, rect, m.from),
                    on_screen(&projector, rect, m.to),
                ) else {
                    return;
                };
                let (ua, ub) = ((a - c).normalized(), (b - c).normalized());
                if !(ua.x.is_finite() && ub.x.is_finite()) {
                    return;
                }
                let r = tokens.measure_arc;
                let (t0, t1) = (ua.y.patan2(ua.x), ub.y.patan2(ub.x));
                // The short way round from one side to the other.
                let mut sweep = t1 - t0;
                while sweep > std::f32::consts::PI {
                    sweep -= std::f32::consts::TAU;
                }
                while sweep < -std::f32::consts::PI {
                    sweep += std::f32::consts::TAU;
                }
                let steps = 24;
                let points: Vec<Pos2> = (0..=steps)
                    .map(|k| {
                        let t = t0 + sweep * k as f32 / steps as f32;
                        c + vec2(t.pcos(), t.psin()) * r
                    })
                    .collect();
                for (ink, w) in [(halo, width + 2.0), (accent, width)] {
                    painter.add(egui::Shape::line(points.clone(), egui::Stroke::new(w, ink)));
                    // The sides, short, from the corner.
                    for u in [ua, ub] {
                        painter
                            .line_segment([c, c + u * (r + 6.0)], egui::Stroke::new(w * 0.7, ink));
                    }
                }
                let mid = t0 + sweep * 0.5;
                let way = vec2(mid.pcos(), mid.psin());
                let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
                    return;
                };
                (first, last, Some(c), c + way * r, way)
            }
            None => {
                let (Some(a), Some(b)) = (
                    on_screen(&projector, rect, m.from),
                    on_screen(&projector, rect, m.to),
                ) else {
                    return;
                };
                let way = (b - a).normalized();
                let long = a.distance(b) > 2.5 * head;
                for (ink, w) in [(halo, width + 2.0), (accent, width)] {
                    painter.line_segment([a, b], egui::Stroke::new(w, ink));
                    if long && way.x.is_finite() {
                        for (tip, dir) in [(a, -way), (b, way)] {
                            painter.add(egui::Shape::convex_polygon(
                                head_at(tip, dir),
                                ink,
                                egui::Stroke::new(if ink == halo { 1.0 } else { 0.0 }, ink),
                            ));
                        }
                    }
                }
                let across = if way.x.is_finite() {
                    vec2(-way.y, way.x)
                } else {
                    vec2(0.0, -1.0)
                };
                // Above the line on the screen.
                let across = if across.y > 0.0 { -across } else { across };
                (a, b, None, a.lerp(b, 0.5), across)
            }
        };
        // The number on its plate, beside the line's middle.
        let galley = painter.layout_no_wrap(
            m.text.clone(),
            egui::FontId::proportional(tokens.control_font),
            accent,
        );
        let size = galley.size() + PAD * 2.0;
        let reach = outward.x.abs() * size.x * 0.5 + outward.y.abs() * size.y * 0.5;
        let mut number = Rect::from_center_size(middle + outward * (reach + 4.0), size);
        let room = rect.shrink(4.0);
        let shift = vec2(
            (room.left() - number.left()).max(0.0) - (number.right() - room.right()).max(0.0),
            (room.top() - number.top()).max(0.0) - (number.bottom() - room.bottom()).max(0.0),
        );
        if shift.x.is_finite() && shift.y.is_finite() {
            number = number.translate(shift);
        }
        painter.rect_filled(
            number,
            egui::CornerRadius::same(tokens.radius),
            color_alpha(tokens.plate_fill, tokens.plate_opacity),
        );
        painter.galley(number.min + PAD, galley, accent);
        if ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|p| number.contains(p))
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        self.dimension = Some(DimensionPlace {
            from,
            to,
            at,
            number,
        });
    }
}
