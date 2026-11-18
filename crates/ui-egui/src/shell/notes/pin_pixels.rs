//! The notes' pins drawn into a picture of the view (`ui.screenshot`,
//! whose picture comes from the CPU reference renderer, not from egui): the
//! same pins as the window paints over the view (`paint_pin`),
//! at the same places, sizes and colours, faint behind a face, with their
//! numbers and "+n" in strokes of their own (the window's text is
//! egui's font, which a picture without the window does not have). Each
//! shape covers a pixel by how far the pixel's centre lies inside it, so
//! the edges are smooth.

use super::{Pin, PinKind};
use crate::theme::{Rgb, Tokens};

/// The strokes of the characters a pin writes, on a box 0.6 wide and 1
/// high, y down: polylines.
const GLYPHS: &[(char, &[&[[f32; 2]]])] = &[
    (
        '0',
        &[&[
            [0.0, 0.15],
            [0.15, 0.0],
            [0.45, 0.0],
            [0.6, 0.15],
            [0.6, 0.85],
            [0.45, 1.0],
            [0.15, 1.0],
            [0.0, 0.85],
            [0.0, 0.15],
        ]],
    ),
    ('1', &[&[[0.12, 0.22], [0.36, 0.0], [0.36, 1.0]]]),
    (
        '2',
        &[&[
            [0.0, 0.18],
            [0.15, 0.0],
            [0.45, 0.0],
            [0.6, 0.18],
            [0.6, 0.38],
            [0.0, 1.0],
            [0.6, 1.0],
        ]],
    ),
    (
        '3',
        &[&[
            [0.0, 0.12],
            [0.15, 0.0],
            [0.45, 0.0],
            [0.6, 0.15],
            [0.6, 0.35],
            [0.45, 0.48],
            [0.2, 0.48],
            [0.45, 0.48],
            [0.6, 0.62],
            [0.6, 0.85],
            [0.45, 1.0],
            [0.15, 1.0],
            [0.0, 0.88],
        ]],
    ),
    ('4', &[&[[0.45, 1.0], [0.45, 0.0], [0.0, 0.7], [0.6, 0.7]]]),
    (
        '5',
        &[&[
            [0.58, 0.0],
            [0.06, 0.0],
            [0.02, 0.45],
            [0.4, 0.4],
            [0.6, 0.58],
            [0.6, 0.85],
            [0.45, 1.0],
            [0.15, 1.0],
            [0.0, 0.88],
        ]],
    ),
    (
        '6',
        &[&[
            [0.55, 0.06],
            [0.4, 0.0],
            [0.2, 0.0],
            [0.0, 0.25],
            [0.0, 0.85],
            [0.15, 1.0],
            [0.45, 1.0],
            [0.6, 0.85],
            [0.6, 0.62],
            [0.45, 0.48],
            [0.15, 0.48],
            [0.0, 0.6],
        ]],
    ),
    ('7', &[&[[0.0, 0.0], [0.6, 0.0], [0.2, 1.0]]]),
    (
        '8',
        &[&[
            [0.3, 0.48],
            [0.06, 0.38],
            [0.06, 0.12],
            [0.2, 0.0],
            [0.4, 0.0],
            [0.54, 0.12],
            [0.54, 0.38],
            [0.3, 0.48],
            [0.0, 0.62],
            [0.0, 0.88],
            [0.15, 1.0],
            [0.45, 1.0],
            [0.6, 0.88],
            [0.6, 0.62],
            [0.3, 0.48],
        ]],
    ),
    (
        '9',
        &[&[
            [0.6, 0.4],
            [0.45, 0.52],
            [0.15, 0.52],
            [0.0, 0.38],
            [0.0, 0.15],
            [0.15, 0.0],
            [0.45, 0.0],
            [0.6, 0.15],
            [0.6, 0.75],
            [0.4, 1.0],
            [0.2, 1.0],
            [0.05, 0.94],
        ]],
    ),
    (
        '?',
        &[
            &[
                [0.0, 0.2],
                [0.15, 0.0],
                [0.45, 0.0],
                [0.6, 0.15],
                [0.6, 0.35],
                [0.3, 0.55],
                [0.3, 0.75],
            ],
            &[[0.3, 0.94], [0.3, 1.0]],
        ],
    ),
    (
        '+',
        &[&[[0.3, 0.25], [0.3, 0.85]], &[[0.0, 0.55], [0.6, 0.55]]],
    ),
];

/// An RGB picture, 3 bytes a pixel from the top left.
struct Canvas<'a> {
    rgb: &'a mut [u8],
    width: u32,
    height: u32,
}

impl Canvas<'_> {
    /// Mixes `ink` at `alpha` (0 to 1), times how much of each pixel
    /// `cover` says the shape covers (from its signed distance), into the
    /// pixels within `[x0, y0, x1, y1]`.
    fn blend(&mut self, bounds: [f32; 4], ink: Rgb, alpha: f32, cover: impl Fn(f32, f32) -> f32) {
        let clamp = |v: f32, most: u32| {
            if v.is_finite() {
                v.clamp(0.0, most as f32) as u32
            } else {
                0
            }
        };
        let (x0, y0) = (
            clamp(bounds[0].floor(), self.width),
            clamp(bounds[1].floor(), self.height),
        );
        let (x1, y1) = (
            clamp(bounds[2].ceil(), self.width),
            clamp(bounds[3].ceil(), self.height),
        );
        let width = self.width as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let c =
                    (cover(x as f32 + 0.5, y as f32 + 0.5).clamp(0.0, 1.0) * alpha).clamp(0.0, 1.0);
                if c <= 0.0 {
                    continue;
                }
                let at = (y as usize)
                    .saturating_mul(width)
                    .saturating_add(x as usize)
                    .saturating_mul(3);
                let Some(pixel) = self.rgb.get_mut(at..at.saturating_add(3)) else {
                    continue;
                };
                for (dst, src) in pixel.iter_mut().zip(ink) {
                    let mixed = f32::from(*dst) * (1.0 - c) + f32::from(src) * c;
                    *dst = mixed.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    fn disc(&mut self, centre: [f32; 2], r: f32, ink: Rgb, alpha: f32) {
        let [cx, cy] = centre;
        self.blend(
            [cx - r - 1.0, cy - r - 1.0, cx + r + 1.0, cy + r + 1.0],
            ink,
            alpha,
            |x, y| r + 0.5 - ((x - cx) * (x - cx) + (y - cy) * (y - cy)).sqrt(),
        );
    }

    fn ring(&mut self, centre: [f32; 2], r: f32, width: f32, ink: Rgb, alpha: f32) {
        let [cx, cy] = centre;
        let half = width * 0.5;
        let reach = r + half + 1.0;
        self.blend(
            [cx - reach, cy - reach, cx + reach, cy + reach],
            ink,
            alpha,
            |x, y| {
                let d = ((x - cx) * (x - cx) + (y - cy) * (y - cy)).sqrt();
                half + 0.5 - (d - r).abs()
            },
        );
    }

    fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32, ink: Rgb, alpha: f32) {
        let half = width * 0.5;
        let reach = half + 1.0;
        let bounds = [
            a[0].min(b[0]) - reach,
            a[1].min(b[1]) - reach,
            a[0].max(b[0]) + reach,
            a[1].max(b[1]) + reach,
        ];
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let long = dx * dx + dy * dy;
        self.blend(bounds, ink, alpha, |x, y| {
            let t = if long > 0.0 {
                (((x - a[0]) * dx + (y - a[1]) * dy) / long).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (px, py) = (a[0] + t * dx - x, a[1] + t * dy - y);
            half + 0.5 - (px * px + py * py).sqrt()
        });
    }

    /// Writes `text` (digits, "?", "+") `height` pixels high, centred on
    /// `centre` or, with `left_top`, from that corner.
    fn text(
        &mut self,
        text: &str,
        at: [f32; 2],
        height: f32,
        left_top: bool,
        ink: Rgb,
        alpha: f32,
    ) {
        let advance = height * 0.8;
        let count = text.chars().count() as f32;
        let wide = count * advance - height * 0.2;
        let (x0, y0) = if left_top {
            (at[0], at[1])
        } else {
            (at[0] - wide * 0.5, at[1] - height * 0.5)
        };
        let stroke = (height * 0.13).max(1.0);
        for (k, c) in text.chars().enumerate() {
            let Some((_, strokes)) = GLYPHS.iter().find(|(g, _)| *g == c) else {
                continue;
            };
            let left = x0 + k as f32 * advance;
            for path in strokes.iter() {
                for pair in path.windows(2) {
                    if let [p, q] = pair {
                        self.line(
                            [left + p[0] * height, y0 + p[1] * height],
                            [left + q[0] * height, y0 + q[1] * height],
                            stroke,
                            ink,
                            alpha,
                        );
                    }
                }
            }
        }
    }
}

/// The height of the number on a head of radius `r` (pixels): smaller
/// for three digits and more, so it stays inside.
fn number(id: u64, r: f32) -> f32 {
    let digits = id.to_string().len().max(1) as f32;
    if digits > 2.0 {
        r * 0.9 * 2.0 / digits
    } else {
        r * 0.9
    }
}

/// Draws `pins` (their places in points) into `rgb`, a picture of `size`
/// pixels at `scale` pixels a point, as `paint_pin` paints them.
pub(super) fn draw_pins(rgb: &mut [u8], size: [u32; 2], pins: &[Pin], tokens: &Tokens, scale: f32) {
    let [width, height] = size;
    let expected = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(3);
    if rgb.len() < expected || !scale.is_finite() || scale <= 0.0 {
        return;
    }
    let mut canvas = Canvas { rgb, width, height };
    let r = tokens.note_radius * scale;
    for pin in pins {
        let alpha = if pin.behind { 70.0 / 255.0 } else { 1.0 };
        let ink = pin.kind.ink(tokens);
        let pos = [pin.at.x * scale, pin.at.y * scale];
        let head = super::Pin::head(pin.at, tokens.note_radius);
        let head = [head.x * scale, head.y * scale];
        canvas.disc(pos, 1.5 * scale, ink, alpha);
        canvas.line(pos, [head[0], head[1] + r], scale, ink, alpha);
        match pin.kind {
            PinKind::Done => {
                let small = r * 0.75;
                let stroke = 1.2 * scale;
                canvas.disc(head, small, tokens.note_fill, alpha);
                canvas.ring(head, small, stroke, ink, alpha);
                let knee = [head[0] - 0.1 * small, head[1] + 0.35 * small];
                canvas.line([head[0] - 0.45 * small, head[1]], knee, stroke, ink, alpha);
                canvas.line(
                    knee,
                    [head[0] + 0.45 * small, head[1] - 0.35 * small],
                    stroke,
                    ink,
                    alpha,
                );
            }
            PinKind::Astray => {
                canvas.disc(head, r, tokens.note_fill, alpha);
                canvas.ring(head, r, 1.5 * scale, ink, alpha);
                canvas.text(
                    &pin.id.to_string(),
                    head,
                    number(pin.id, r),
                    false,
                    ink,
                    alpha,
                );
            }
            PinKind::ForAgent | PinKind::ForYou => {
                canvas.disc(head, r, ink, alpha);
                if pin.kind == PinKind::ForAgent {
                    // The dark plate's light rim.
                    let rim = f32::from(tokens.note_rim_opacity) / 255.0;
                    canvas.ring(head, r - 0.5 * scale, scale, tokens.note_rim, alpha * rim);
                }
                canvas.text(
                    &pin.id.to_string(),
                    head,
                    number(pin.id, r),
                    false,
                    tokens.note_ink,
                    alpha,
                );
            }
        }
        if pin.chosen {
            // A 2 pt ring of the selection's amber, 3 pt outside the head.
            canvas.ring(
                head,
                r + 4.0 * scale,
                2.0 * scale,
                tokens.note_chosen,
                alpha,
            );
        }
        if pin.more > 0 {
            canvas.text(
                &format!("+{}", pin.more),
                [head[0] + r + 2.0 * scale, head[1] - r],
                r * 0.85,
                true,
                tokens.viewport_text,
                alpha,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_character_a_pin_writes_has_strokes() {
        for c in "0123456789?+".chars() {
            assert!(GLYPHS.iter().any(|(g, s)| *g == c && !s.is_empty()), "{c}");
        }
    }

    #[test]
    fn a_pin_is_drawn_where_it_stands_and_nowhere_else() {
        let tokens = Tokens::light();
        let (w, h) = (60u32, 60u32);
        let mut rgb = vec![255u8; (w * h * 3) as usize];
        let pin = Pin {
            id: 7,
            at: egui::pos2(30.0, 50.0),
            kind: PinKind::ForYou,
            behind: false,
            chosen: false,
            more: 0,
        };
        draw_pins(&mut rgb, [w, h], &[pin], &tokens, 1.0);
        let at = |x: u32, y: u32| {
            let k = ((y * w + x) * 3) as usize;
            [rgb[k], rgb[k + 1], rgb[k + 2]]
        };
        // The head, in the colour of a note that waits for the person
        // (N4); a corner untouched.
        let head = Pin::head(egui::pos2(30.0, 50.0), tokens.note_radius);
        let fill = at(head.x as u32 + 5, head.y as u32);
        assert_eq!(fill, tokens.note_for_you, "{fill:?}");
        assert_eq!(at(1, 1), [255, 255, 255]);
        // A picture of another size than its rows is left alone.
        let mut short = vec![0u8; 10];
        draw_pins(&mut short, [w, h], &[], &tokens, 1.0);
        assert_eq!(short, vec![0u8; 10]);
    }
}
