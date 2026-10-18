//! The small line icons of the controls over the viewport, drawn with the
//! painter from a few points each (no image assets): in a square place,
//! in one colour, with lines `tokens.nav_icon_width` wide.

use crate::theme::Tokens;
use egui::{Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};

/// What an icon shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// Frame everything: four corner brackets.
    Fit,
    /// The iso view: a cube seen from a corner.
    Iso,
    /// Sketches: a polyline with its points.
    Sketches,
    /// Datums: a plane seen at a slant.
    Datums,
    /// Section: a body cut by a plane, the half that is cut away hollow.
    Section,
    /// Flip the kept side: an arrow up and one down.
    Flip,
}

/// Paints `icon` centred in `place` (its shorter side is the icon's).
pub fn paint(painter: &egui::Painter, icon: Icon, place: Rect, ink: Color32, tokens: &Tokens) {
    let half = place.width().min(place.height()) * 0.5;
    if !(half.is_finite() && half > 0.0) {
        return;
    }
    let c = place.center();
    // From the unit square [-1, 1]² (y down) to the place.
    let p = |x: f32, y: f32| -> Pos2 { c + vec2(x, y) * half };
    let stroke = Stroke::new(tokens.nav_icon_width, ink);
    let line = |points: Vec<Pos2>| Shape::line(points, stroke);
    let closed = |points: Vec<Pos2>| Shape::closed_line(points, stroke);
    let faint = ink.gamma_multiply(0.35);
    let shapes: Vec<Shape> = match icon {
        Icon::Fit => {
            let (a, b) = (0.85, 0.4);
            vec![
                line(vec![p(-a, -b), p(-a, -a), p(-b, -a)]),
                line(vec![p(b, -a), p(a, -a), p(a, -b)]),
                line(vec![p(a, b), p(a, a), p(b, a)]),
                line(vec![p(-b, a), p(-a, a), p(-a, b)]),
                Shape::circle_filled(c, tokens.nav_icon_width * 1.2, ink),
            ]
        }
        Icon::Iso => {
            // A hexagon and the three edges that meet at the near corner.
            let (x, y) = (0.82, 0.47);
            vec![
                closed(vec![
                    p(0.0, -0.95),
                    p(x, -y),
                    p(x, y),
                    p(0.0, 0.95),
                    p(-x, y),
                    p(-x, -y),
                ]),
                line(vec![p(-x, -y), p(0.0, 0.0), p(x, -y)]),
                line(vec![p(0.0, 0.0), p(0.0, 0.95)]),
            ]
        }
        Icon::Sketches => {
            let points = [p(-0.85, 0.6), p(-0.3, -0.55), p(0.3, 0.35), p(0.85, -0.6)];
            let mut shapes = vec![line(points.to_vec())];
            for at in points {
                shapes.push(Shape::circle_filled(at, tokens.nav_icon_width * 1.3, ink));
            }
            shapes
        }
        Icon::Datums => vec![Shape::convex_polygon(
            vec![p(-0.45, -0.5), p(0.95, -0.5), p(0.45, 0.5), p(-0.95, 0.5)],
            faint,
            stroke,
        )],
        Icon::Section => vec![
            // The half that stays, filled; the half cut away, an outline.
            Shape::convex_polygon(
                vec![p(-0.75, -0.6), p(0.0, -0.6), p(0.0, 0.6), p(-0.75, 0.6)],
                faint,
                stroke,
            ),
            closed(vec![p(0.0, -0.6), p(0.75, -0.6), p(0.75, 0.6), p(0.0, 0.6)]),
            Shape::line_segment(
                [p(0.0, -0.98), p(0.0, 0.98)],
                Stroke::new(stroke.width * 1.4, ink),
            ),
        ],
        Icon::Flip => {
            let head = 0.35;
            vec![
                line(vec![p(-0.4, 0.85), p(-0.4, -0.8)]),
                line(vec![
                    p(-0.4 - head, -0.8 + head),
                    p(-0.4, -0.85),
                    p(-0.4 + head, -0.8 + head),
                ]),
                line(vec![p(0.4, -0.85), p(0.4, 0.8)]),
                line(vec![
                    p(0.4 - head, 0.8 - head),
                    p(0.4, 0.85),
                    p(0.4 + head, 0.8 - head),
                ]),
            ]
        }
    };
    for shape in shapes {
        painter.add(shape);
    }
}

/// The square place of an icon of `side` points whose left is at `left`,
/// centred on `middle_y`.
pub fn place(left: f32, middle_y: f32, side: f32) -> Rect {
    Rect::from_min_size(pos2(left, middle_y - side * 0.5), vec2(side, side))
}
