//! The popovers of the Studio and Analysis chips: a panel of options
//! anchored under the camera plate, right-aligned against the view cube
//! (beside it where there is room, under it where there is not), never
//! over the cube, the plates or the display bar.
//!
//! One shape for both (2026-10-30, the developer: the studio panel sat
//! loose in the viewport and came back cut off after it was folded): a
//! header that folds the options away (a click, or Escape), a width from
//! the tokens (`popover_width`) and a height that follows what the
//! options hold, so folding and unfolding gives the same rect every time;
//! the options scroll only when the viewport is too short for them.

use super::Shell;
use super::controls::ControlsLayout;
use crate::theme::{Tokens, color};
use egui::{Rect, pos2, vec2};

/// Where a popover may go in the viewport `rect` whose controls are laid
/// out as `layout`: its top left corner, its width and the most height
/// it may take. Under the camera plate, its right edge a gap left of the
/// view cube when it fits there; else under the cube against the right
/// margin, narrower where it would reach the triad; above the display bar
/// and the triad where its columns cross them. `None` when there is no
/// room for its header and a row.
pub fn popover_place(rect: Rect, layout: &ControlsLayout, tokens: &Tokens) -> Option<Rect> {
    if !rect.is_finite() {
        return None;
    }
    let area = rect.shrink(tokens.nav_margin);
    let gap = tokens.nav_gap;
    let width = tokens.popover_width.min(area.width());
    let least = 2.0 * tokens.nav_height + 2.0 * tokens.nav_inset;
    if !(width.is_finite() && width > 0.0) {
        return None;
    }
    // The plates and the triad below it: the popover ends a gap above
    // those its columns cross.
    let below = [layout.bar, layout.section_plate, layout.triad];
    let bottom = |left: f32, right: f32| {
        below
            .into_iter()
            .flatten()
            .filter(|r| r.right() > left && r.left() < right)
            .fold(area.bottom(), |b, r| b.min(r.top() - gap))
    };
    let below_plate = layout.camera_plate.map_or(area.top(), |p| p.bottom() + gap);
    // (right, top, width) in the order they are tried.
    let mut places = Vec::new();
    match layout.cube {
        Some(cube) => {
            // Beside the cube, under the plate.
            let right = cube.left() - gap;
            if right - area.left() >= width {
                places.push((right, below_plate, width));
            }
            // Under the cube; then, narrower, clear of the triad.
            places.push((area.right(), cube.bottom() + gap, width));
            if let Some(triad) = layout.triad {
                let narrow = (area.right() - triad.right() - gap).min(width);
                places.push((area.right(), cube.bottom() + gap, narrow));
            }
        }
        None => places.push((area.right(), below_plate, width)),
    }
    places.into_iter().find_map(|(right, top, width)| {
        let left = right - width;
        let height = bottom(left, right) - top;
        (width.is_finite() && width >= least && height.is_finite() && height >= least)
            .then(|| Rect::from_min_size(pos2(left, top), vec2(width, height)))
    })
}

/// What a popover drew: its header (the fold) and the whole panel.
pub(super) struct Drawn {
    pub header: Rect,
    pub panel: Rect,
}

impl Shell {
    /// A popover called `title` at `place` ([`popover_place`]); `open`
    /// unfolds its options, drawn by `options`, and is flipped by a click
    /// on the header.
    pub(super) fn popover(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        title: &str,
        place: Rect,
        open: &mut bool,
        options: impl FnOnce(&mut Self, &mut egui::Ui),
    ) -> Drawn {
        let tokens = self.tokens;
        let stroke = 1.0;
        let inset = tokens.nav_inset;
        let inner_width = (place.width() - 2.0 * (inset + stroke)).max(0.0);
        let mut header = Rect::NOTHING;
        let area = egui::Area::new(egui::Id::new(id))
            .fixed_pos(place.min)
            .constrain(false)
            .show(ui.ctx(), |ui| {
                // Its own room, not the size the area had last frame (folded,
                // that would keep the options cut short once unfolded).
                let room = Rect::from_min_size(ui.max_rect().min, place.size());
                ui.scope_builder(egui::UiBuilder::new().max_rect(room), |ui| {
                    egui::Frame::new()
                        .fill(color(tokens.panel_fill))
                        .stroke(egui::Stroke::new(stroke, color(tokens.plate_rim)))
                        .corner_radius(tokens.nav_control_radius)
                        .inner_margin(inset)
                        .show(ui, |ui| {
                            ui.set_width(inner_width);
                            let (rect, response) = ui.allocate_exact_size(
                                vec2(inner_width, tokens.nav_height),
                                egui::Sense::click(),
                            );
                            header = rect;
                            let ink = if response.hovered() {
                                color(tokens.text)
                            } else {
                                color(tokens.text_dim)
                            };
                            ui.painter().text(
                                pos2(rect.left(), rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                title,
                                egui::FontId::proportional(tokens.control_font),
                                ink,
                            );
                            chevron(ui.painter(), rect, *open, ink, &tokens);
                            let response = response.on_hover_text(if *open {
                                "Fold the options away (Esc)"
                            } else {
                                "Show the options"
                            });
                            if response.clicked() {
                                *open = !*open;
                                ui.ctx().request_repaint();
                            }
                            if *open {
                                let spacing = ui.spacing().item_spacing.y;
                                let room = place.height()
                                    - 2.0 * (inset + stroke)
                                    - tokens.nav_height
                                    - spacing
                                    - 1.0;
                                egui::ScrollArea::both()
                                    .id_salt((id, "options"))
                                    .auto_shrink([false, true])
                                    .max_height(room.max(0.0))
                                    .show(ui, |ui| options(self, ui));
                            }
                        });
                });
            });
        Drawn {
            header,
            panel: area.response.rect,
        }
    }

    /// Escape closes the narrow layout's open drawer, else folds an open
    /// popover (the analysis', else the studio look's); `false` when
    /// none was open.
    pub(super) fn fold_popover(&mut self) -> bool {
        if self.narrow() && self.state.panels.drawer {
            // The narrow layout's drawer first.
            self.state.panels.drawer = false;
            true
        } else if self.state.analysis.is_on() && self.analysis_panel_open {
            self.analysis_panel_open = false;
            true
        } else if self.state.look.is_studio()
            && !self.state.analysis.is_on()
            && self.look_panel_open
        {
            self.look_panel_open = false;
            true
        } else {
            false
        }
    }
}

/// The fold's chevron at the right end of the header `rect`: pointing
/// down while the options are open, right while folded.
fn chevron(painter: &egui::Painter, rect: Rect, open: bool, ink: egui::Color32, tokens: &Tokens) {
    let r = tokens.nav_icon_size * 0.22;
    let c = pos2(rect.right() - tokens.nav_icon_size * 0.5, rect.center().y);
    let points = if open {
        [
            c + vec2(-r, -r * 0.5),
            c + vec2(0.0, r * 0.5),
            c + vec2(r, -r * 0.5),
        ]
    } else {
        [
            c + vec2(-r * 0.5, -r),
            c + vec2(r * 0.5, 0.0),
            c + vec2(-r * 0.5, r),
        ]
    };
    let stroke = egui::Stroke::new(tokens.nav_icon_width, ink);
    painter.line_segment([points[0], points[1]], stroke);
    painter.line_segment([points[1], points[2]], stroke);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{ControlWidths, controls_layout};

    /// At every size from tiny to large, with the section on and off: the
    /// place is inside the viewport less its margin and clear of the
    /// cube, the plates and the display bar; at the sizes the window
    /// opens at it is beside the cube.
    #[test]
    fn the_popover_keeps_clear_of_every_control() {
        let tokens = Tokens::default();
        let widths = ControlWidths::fallback(&tokens);
        let mut shown = 0;
        for w in (40..=1800).step_by(37) {
            for h in (40..=1100).step_by(29) {
                for section in [false, true] {
                    let rect = Rect::from_min_size(pos2(10.0, 30.0), vec2(w as f32, h as f32));
                    let layout = controls_layout(rect, &tokens, &widths, section);
                    let Some(place) = popover_place(rect, &layout, &tokens) else {
                        continue;
                    };
                    shown += 1;
                    let area = rect.shrink(tokens.nav_margin);
                    assert!(area.contains_rect(place), "{place:?} in {area:?}");
                    for (name, part) in layout.parts() {
                        assert!(
                            !part.intersects(place),
                            "{w}×{h} {section}: over the {name}: {place:?} {part:?}"
                        );
                    }
                }
            }
        }
        assert!(shown > 100, "{shown}");
        for (w, h) in [(1300.0, 850.0), (800.0, 560.0)] {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h));
            let layout = controls_layout(rect, &tokens, &widths, false);
            let place = popover_place(rect, &layout, &tokens).unwrap();
            let cube = layout.cube.unwrap();
            assert_eq!(place.right(), cube.left() - tokens.nav_gap);
            assert_eq!(
                place.top(),
                layout.camera_plate.unwrap().bottom() + tokens.nav_gap
            );
        }
    }

    #[test]
    fn no_place_in_a_viewport_that_is_no_rect() {
        let tokens = Tokens::default();
        let layout = ControlsLayout::default();
        let nan = Rect::from_min_size(pos2(f32::NAN, 0.0), vec2(100.0, 100.0));
        assert_eq!(popover_place(nan, &layout, &tokens), None);
        let tiny = Rect::from_min_size(pos2(0.0, 0.0), vec2(10.0, 10.0));
        assert_eq!(popover_place(tiny, &layout, &tokens), None);
    }
}
