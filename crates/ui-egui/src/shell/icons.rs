//! The small line icons of the controls over the viewport, drawn with the
//! painter from a few points each (no image assets): in a square place,
//! in one colour, with lines `tokens.nav_icon_width` wide.

use crate::theme::Tokens;
use craft3d_math::Portable;
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
    /// Edges: a face with its edges inked.
    Edges,
    /// The studio look: a lamp, a disc with rays.
    Studio,
    /// Surface analysis: a disc crossed by stripes, as zebra stripes.
    Analysis,
    /// Section: a body cut by a plane, the half that is cut away hollow.
    Section,
    /// Flip the kept side: an arrow up and one down.
    Flip,
    /// The history player's first step: a bar and two triangles to it.
    Start,
    /// A step back: a bar and a triangle to it.
    StepBack,
    /// Play: a triangle to the right.
    Play,
    /// Pause: two bars.
    Pause,
    /// A step forward: a triangle and a bar.
    StepForward,
    /// The end: two triangles and a bar.
    End,
    /// Loop: a circle that runs into an arrow.
    Loop,
    /// Orbit: an ellipse about a vertical axis, with an arrow.
    Orbit,
    /// Zoom to a feature (a row of the feature list): a target, ⌖.
    ZoomTo,
    /// An AI agent (the status bar's agents pill): a four-pointed spark
    /// and a small one beside it.
    Agent,
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
    let faint = ink.gamma_multiply(tokens.disabled_opacity);
    // An outlined triangle pointing right (`dir` 1) or left (-1), its tip
    // at `tip` and its back `width` behind it (board s8: the transport
    // glyphs are outlines, as the other line icons).
    let triangle = |tip: f32, dir: f32, width: f32| {
        Shape::convex_polygon(
            vec![
                p(tip - dir * width, -0.7),
                p(tip, 0.0),
                p(tip - dir * width, 0.7),
            ],
            Color32::TRANSPARENT,
            Stroke::new(stroke.width, ink),
        )
    };
    let bar = |x: f32| Shape::line_segment([p(x, -0.7), p(x, 0.7)], stroke);
    // An arrow head at `at`, pointing along `(dx, dy)`.
    let head = |at: Pos2, dx: f32, dy: f32| {
        let len = (dx * dx + dy * dy).sqrt().max(f32::MIN_POSITIVE);
        let (ux, uy) = (dx / len * half * 0.4, dy / len * half * 0.4);
        line(vec![
            at + vec2(-ux - uy, -uy + ux),
            at,
            at + vec2(-ux + uy, -uy - ux),
        ])
    };
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
        Icon::Edges => {
            // A box seen from a corner, its faces faint, its edges bold.
            let (x, y) = (0.82, 0.47);
            let bold = Stroke::new(stroke.width * 1.4, ink);
            vec![
                Shape::convex_polygon(
                    vec![
                        p(0.0, -0.95),
                        p(x, -y),
                        p(x, y),
                        p(0.0, 0.95),
                        p(-x, y),
                        p(-x, -y),
                    ],
                    faint,
                    bold,
                ),
                Shape::line(vec![p(-x, -y), p(0.0, 0.0), p(x, -y)], bold),
                Shape::line(vec![p(0.0, 0.0), p(0.0, 0.95)], bold),
            ]
        }
        Icon::Studio => {
            let mut shapes = vec![
                Shape::circle_filled(c, half * 0.32, faint),
                Shape::circle_stroke(c, half * 0.32, stroke),
            ];
            for k in 0..8 {
                let a = std::f32::consts::FRAC_PI_4 * k as f32;
                let (sin, cos) = a.psin_cos();
                shapes.push(Shape::line_segment(
                    [p(0.55 * cos, 0.55 * sin), p(0.9 * cos, 0.9 * sin)],
                    stroke,
                ));
            }
            shapes
        }
        Icon::Analysis => {
            let mut shapes = vec![Shape::circle_stroke(c, half * 0.85, stroke)];
            for k in [-0.45_f32, 0.0, 0.45] {
                let w = (0.85_f32 * 0.85 - k * k).max(0.0).sqrt();
                shapes.push(Shape::line_segment([p(-w, k), p(w, k)], stroke));
            }
            shapes
        }
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
        Icon::Start => vec![
            bar(-0.75),
            triangle(-0.65, -1.0, 0.7),
            triangle(0.05, -1.0, 0.7),
        ],
        Icon::StepBack => vec![bar(-0.55), triangle(-0.45, -1.0, 1.0)],
        Icon::Play => vec![triangle(0.75, 1.0, 1.3)],
        Icon::Agent => {
            // A spark: four points with waisted sides, filled, from convex
            // pieces (a filled path must be convex): a small diamond in
            // the middle and a triangle to each point.
            let spark = |cx: f32, cy: f32, r: f32| {
                let w = r * 0.28;
                let waist = [
                    p(cx + w, cy - w),
                    p(cx + w, cy + w),
                    p(cx - w, cy + w),
                    p(cx - w, cy - w),
                ];
                let tips = [p(cx, cy - r), p(cx + r, cy), p(cx, cy + r), p(cx - r, cy)];
                let mut shapes = vec![Shape::convex_polygon(waist.to_vec(), ink, Stroke::NONE)];
                // Each tip between the two waist points beside it.
                let sides = [(3, 0), (0, 1), (1, 2), (2, 3)];
                for (tip, (a, b)) in tips.iter().zip(sides) {
                    if let (Some(a), Some(b)) = (waist.get(a), waist.get(b)) {
                        shapes.push(Shape::convex_polygon(vec![*tip, *b, *a], ink, Stroke::NONE));
                    }
                }
                shapes
            };
            let mut shapes = spark(-0.15, 0.15, 0.85);
            shapes.extend(spark(0.62, -0.62, 0.38));
            shapes
        }
        Icon::Pause => vec![
            Shape::rect_filled(Rect::from_min_max(p(-0.55, -0.7), p(-0.15, 0.7)), 0.0, ink),
            Shape::rect_filled(Rect::from_min_max(p(0.15, -0.7), p(0.55, 0.7)), 0.0, ink),
        ],
        Icon::StepForward => vec![triangle(0.45, 1.0, 1.0), bar(0.55)],
        Icon::End => vec![
            triangle(-0.05, 1.0, 0.7),
            triangle(0.65, 1.0, 0.7),
            bar(0.75),
        ],
        Icon::Loop => {
            // Repeat (board s8): a rounded loop in two halves, the upper
            // running right into an arrow head, the lower running left.
            let r = 0.3_f32;
            let corner = |cx: f32, cy: f32, from: f32| -> Vec<Pos2> {
                (0..=6)
                    .map(|k| {
                        let a = from + (k as f32) / 6.0 * std::f32::consts::FRAC_PI_2;
                        p(cx + r * a.pcos(), cy + r * a.psin())
                    })
                    .collect()
            };
            // Upper: from the left side up round the corner, then right.
            let mut upper = vec![p(-0.85, 0.15)];
            upper.extend(corner(-0.85 + r, -0.5 + r, std::f32::consts::PI));
            upper.push(p(0.7, -0.5));
            // Lower: from the right side down round the corner, then left.
            let mut lower = vec![p(0.85, -0.15)];
            lower.extend(corner(0.85 - r, 0.5 - r, 0.0));
            lower.push(p(-0.7, 0.5));
            vec![
                line(upper),
                head(p(0.75, -0.5), 1.0, 0.0),
                line(lower),
                head(p(-0.75, 0.5), -1.0, 0.0),
            ]
        }
        Icon::Orbit => {
            // A flat ellipse about the vertical axis, an arrow on its
            // near side going right.
            let points: Vec<Pos2> = (0..=24)
                .map(|k| {
                    let a = 0.3 + (k as f32) / 24.0 * 5.4;
                    p(0.85 * a.pcos(), 0.35 * a.psin())
                })
                .collect();
            let end = 5.7_f32;
            let tip = p(0.85 * end.pcos(), 0.35 * end.psin());
            vec![
                Shape::line_segment(
                    [p(0.0, -0.9), p(0.0, 0.9)],
                    Stroke::new(stroke.width, faint),
                ),
                line(points),
                head(tip, -0.85 * end.psin(), 0.35 * end.pcos()),
            ]
        }
        Icon::ZoomTo => {
            // A target: a ring, a dot in it and four ticks through it.
            let mut shapes = vec![
                Shape::circle_stroke(c, half * 0.55, stroke),
                Shape::circle_filled(c, stroke.width * 0.9, ink),
            ];
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                shapes.push(Shape::line_segment(
                    [p(dx * 0.3, dy * 0.3), p(dx * 0.9, dy * 0.9)],
                    stroke,
                ));
            }
            shapes
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
