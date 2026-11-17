//! Measures in the view (the developer's design of 2026-11-17, step B,
//! and his words that day: "ik zou maten in de view willen zien"). With
//! two things selected, one dimension line between them (an angle as an
//! arc about its corner): the side panel's measurement
//! (`measure.between`) drawn. With one thing selected, its measure beside
//! it: a straight edge's length along a line beside it, a whole circle's
//! diameter and a whole cylinder's across it (Ø), an arc's and a part
//! cylinder's radius from its centre (R), a curved edge's length by its
//! middle. One thin line in the
//! accent, drawn as the section's arrow is (a core with a dark edge each
//! side, the same dart for a head: one family), one number on a small
//! plate; gone with the selection, and with Escape, which clears it. A
//! click on the number copies it with its unit. Drawn by egui over the
//! picture, as the section's handle is: neither renderer draws it.

use super::Shell;
use super::section_handle::{paint_dart, paint_slim_line};
use crate::camera::Projector;
use crate::theme::{Tokens, color, color_alpha};
use craft3d_math::{Portable, Vec3};
use egui::{Painter, Pos2, Rect, Vec2, pos2, vec2};

/// Where the dimension was drawn, in the window's points.
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionPlace {
    /// The line's ends on the screen; for an angle the points of the arc
    /// at its ends.
    pub from: Pos2,
    pub to: Pos2,
    /// An angle's corner on the screen.
    pub at: Option<Pos2>,
    /// The number's plate, and the number as shown and copied.
    pub number: Rect,
    pub text: String,
}

/// How far the plate keeps from the line, and its padding, in points.
const PAD: Vec2 = vec2(6.0, 3.0);
/// How far beside a straight edge its length is drawn, and how far the
/// lines from the edge's ends reach past it, in points.
const BESIDE: f32 = 18.0;
const PAST: f32 = 4.0;

/// What a dimension was drawn as: its ends, an angle's corner, the middle
/// the number goes beside, the way out to it, and the number.
type Drawn = (Pos2, Pos2, Option<Pos2>, Pos2, Vec2, String);

/// A point of space on the screen, in the viewport `rect`.
fn on_screen(projector: &Projector, rect: Rect, p: [f64; 3]) -> Option<Pos2> {
    let [x, y, z] = p;
    let seen = projector.project(Vec3::new(x, y, z))?;
    let at = pos2(rect.left() + seen.x as f32, rect.top() + seen.y as f32);
    (at.x.is_finite() && at.y.is_finite()).then_some(at)
}

/// A unit vector on the screen, or `None` for one that is not.
fn unit(v: Vec2) -> Option<Vec2> {
    let u = v.normalized();
    (u.x.is_finite() && u.y.is_finite() && u.length() > 0.5).then_some(u)
}

/// Square to `way` on the screen, upward (or `way` up when it is none).
fn upward_across(way: Option<Vec2>) -> Vec2 {
    let across = way.map_or(vec2(0.0, -1.0), |w| vec2(-w.y, w.x));
    if across.y > 0.0 { -across } else { across }
}

/// The ink of a dimension: its line and head in the accent, inside a dark
/// edge, as wide as the section's arrow.
struct Ink {
    fill: egui::Color32,
    edge: egui::Stroke,
    width: f32,
}

impl Ink {
    fn of(tokens: &Tokens) -> Self {
        Ink {
            fill: color(tokens.accent),
            edge: egui::Stroke::new(
                tokens.section_handle_edge_width,
                color_alpha(tokens.plate_fill, tokens.section_halo_opacity),
            ),
            width: tokens.section_arrow_width,
        }
    }

    /// A dimension line from `a` to `b`, with a head at each end when
    /// `heads` and it is long enough for them.
    fn line(&self, painter: &Painter, tokens: &Tokens, a: Pos2, b: Pos2, heads: bool) {
        paint_slim_line(painter, a, b, self.width, self.fill, self.edge);
        let long = a.distance(b) > 2.5 * tokens.section_head;
        if let (true, true, Some(way)) = (heads, long, unit(b - a)) {
            paint_dart(painter, b, way, tokens, self.fill, self.edge);
            paint_dart(painter, a, -way, tokens, self.fill, self.edge);
        }
    }

    /// A thin line from `a` to `b`: a line from an edge's end, a side of an
    /// angle, a radius.
    fn thin(&self, painter: &Painter, a: Pos2, b: Pos2) {
        painter.line_segment(
            [a, b],
            egui::Stroke::new(self.width * 0.5 + 2.0 * self.edge.width, self.edge.color),
        );
        painter.line_segment([a, b], egui::Stroke::new(self.width * 0.5, self.fill));
    }
}

impl Shell {
    /// Where the dimension of what is selected was drawn last.
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
        let text = place.text.clone();
        ui.ctx().copy_text(text.clone());
        self.side.measured = Some(format!("Copied {text}"));
        true
    }

    /// Paints the dimension of what is selected over the viewport `rect`,
    /// when it has one: the relation of two things, or the measure of one.
    pub(super) fn paint_dimension(&mut self, ui: &egui::Ui, rect: Rect) {
        self.dimension = None;
        // Being measured on its own thread: look again soon.
        if self.measuring() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }
        let Some(projector) = Projector::new(
            &self.state.camera.to_render(),
            f64::from(rect.width()),
            f64::from(rect.height()),
        ) else {
            return;
        };
        let painter = ui.painter_at(rect);
        let drawn = if let Some(m) = self.measurement().cloned() {
            if m.value.is_none() || m.relation.is_empty() {
                return;
            }
            self.paint_relation(&painter, &projector, rect, &m)
        } else if let Some(single) = self.single_measure().cloned() {
            self.paint_single(&painter, &projector, rect, &single)
        } else {
            None
        };
        let Some((from, to, at, middle, outward, text)) = drawn else {
            return;
        };
        let number = self.paint_number(&painter, rect, middle, outward, &text);
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
            text,
        });
    }

    /// The number `text` on its plate beside `middle`, `outward` of it,
    /// within the viewport `rect`; where it went.
    fn paint_number(
        &self,
        painter: &Painter,
        rect: Rect,
        middle: Pos2,
        outward: Vec2,
        text: &str,
    ) -> Rect {
        let tokens = self.tokens;
        let accent = color(tokens.accent);
        let galley = painter.layout_no_wrap(
            text.to_string(),
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
        number
    }

    /// The relation of two things: a line between them, or an arc about
    /// an angle's corner.
    fn paint_relation(
        &self,
        painter: &Painter,
        projector: &Projector,
        rect: Rect,
        m: &super::sidepanel::props::Measurement,
    ) -> Option<Drawn> {
        let tokens = self.tokens;
        let ink = Ink::of(&tokens);
        match m.at {
            // An angle: an arc about its corner from one side to the other.
            Some(corner) => {
                let c = on_screen(projector, rect, corner)?;
                let a = on_screen(projector, rect, m.from)?;
                let b = on_screen(projector, rect, m.to)?;
                let (ua, ub) = (unit(a - c)?, unit(b - c)?);
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
                // The sides, short, from the corner.
                for u in [ua, ub] {
                    ink.thin(painter, c, c + u * (r + 6.0));
                }
                painter.add(egui::Shape::line(
                    points.clone(),
                    egui::Stroke::new(ink.width + 2.0 * ink.edge.width, ink.edge.color),
                ));
                painter.add(egui::Shape::line(
                    points.clone(),
                    egui::Stroke::new(ink.width, ink.fill),
                ));
                let mid = t0 + sweep * 0.5;
                let way = vec2(mid.pcos(), mid.psin());
                let (first, last) = (*points.first()?, *points.last()?);
                Some((first, last, Some(c), c + way * r, way, m.text.clone()))
            }
            None => {
                let a = on_screen(projector, rect, m.from)?;
                let b = on_screen(projector, rect, m.to)?;
                ink.line(painter, &tokens, a, b, true);
                let out = upward_across(unit(b - a));
                Some((a, b, None, a.lerp(b, 0.5), out, m.text.clone()))
            }
        }
    }

    /// The measure of one thing: a straight edge's length along a line
    /// beside it, a diameter across its circle, a radius from its centre,
    /// a curved edge's length by its middle.
    fn paint_single(
        &self,
        painter: &Painter,
        projector: &Projector,
        rect: Rect,
        single: &super::sidepanel::props::Single,
    ) -> Option<Drawn> {
        let tokens = self.tokens;
        let ink = Ink::of(&tokens);
        // The middle of the model on the screen: a length is drawn on the
        // side of its edge away from it.
        let model = self
            .shown_scene
            .as_ref()
            .and_then(|s| s.bodies_box.corners())
            .and_then(|(lo, hi)| {
                let m = lo.lerp(hi, 0.5);
                on_screen(projector, rect, [m.x, m.y, m.z])
            })
            .unwrap_or(rect.center());
        let text = single.text.clone();
        match single.round {
            // A diameter across the circle, square to the line of sight so
            // it shows whole; a radius from the centre to the arc's middle.
            Some(([cx, cy, cz], [ax, ay, az], r)) => {
                let (centre, axis) = (Vec3::new(cx, cy, cz), Vec3::new(ax, ay, az));
                let eye = self.state.camera.toward_eye();
                let across = axis
                    .cross(eye)
                    .normalized()
                    .or_else(|| axis.cross(Vec3::X).normalized())
                    .or_else(|| axis.cross(Vec3::Y).normalized())?;
                let point = |p: Vec3| on_screen(projector, rect, [p.x, p.y, p.z]);
                if single.measure == "radius" {
                    let rim = single
                        .points
                        .get(single.points.len() / 2)
                        .map_or(centre + across * r, |&[x, y, z]| Vec3::new(x, y, z));
                    let (c, b) = (point(centre)?, point(rim)?);
                    ink.thin(painter, c, b);
                    if let Some(way) = unit(b - c) {
                        paint_dart(painter, b, way, &tokens, ink.fill, ink.edge);
                    }
                    let out = upward_across(unit(b - c));
                    Some((c, b, None, c.lerp(b, 0.5), out, text))
                } else {
                    let a = point(centre - across * r)?;
                    let b = point(centre + across * r)?;
                    ink.line(painter, &tokens, a, b, true);
                    let out = upward_across(unit(b - a));
                    Some((a, b, None, a.lerp(b, 0.5), out, text))
                }
            }
            // A straight edge: a line beside it, the lines from its ends.
            None if single.points.len() == 2 => {
                let p = on_screen(projector, rect, *single.points.first()?)?;
                let q = on_screen(projector, rect, *single.points.last()?)?;
                let way = unit(q - p)?;
                let across = vec2(-way.y, way.x);
                let middle = p.lerp(q, 0.5);
                let out = if (middle - model).dot(across) >= 0.0 {
                    across
                } else {
                    -across
                };
                let (a, b) = (p + out * BESIDE, q + out * BESIDE);
                for (from, to) in [(p, a), (q, b)] {
                    ink.thin(painter, from + out * 3.0, to + out * PAST);
                }
                ink.line(painter, &tokens, a, b, true);
                Some((a, b, None, a.lerp(b, 0.5), out, text))
            }
            // A curved edge: its length by its middle, outward of it.
            None => {
                let points: Vec<Pos2> = single
                    .points
                    .iter()
                    .filter_map(|&p| on_screen(projector, rect, p))
                    .collect();
                let middle = *points.get(points.len() / 2)?;
                let (first, last) = (*points.first()?, *points.last()?);
                let out = unit(middle - model).unwrap_or(vec2(0.0, -1.0));
                Some((first, last, None, middle, out, text))
            }
        }
    }
}
