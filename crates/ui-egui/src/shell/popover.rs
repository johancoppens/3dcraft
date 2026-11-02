//! The popovers of the Studio and Analysis chips: the chip's options on
//! the camera plate's own surface, hanging from the plate with a pointer
//! toward the chip, its right edge on the plate's right edge.
//!
//! One shape for both. The chips are split buttons (2026-10-31, the
//! developer: the panel did not flow with the view cube, and folded it
//! was an empty bar): the chip switches the look or the analysis on and
//! off, the caret on its right opens and closes the options. Closed is
//! gone (the look stays as it is); switching a chip on opens them. While
//! a popover is open the view cube moves down under it, or is left out
//! where there is no room for it there, and comes back when it closes.
//! The width is the tokens' (`popover_width`), the height follows what
//! the options hold; they scroll only where the viewport is too short.

use super::Shell;
use super::controls::ControlsLayout;
use crate::theme::{Tokens, color_alpha};
use egui::{Rect, pos2, vec2};

/// The plates and the triad a popover, or the cube moved under it, must
/// end above.
fn below(layout: &ControlsLayout) -> [Option<Rect>; 4] {
    [
        layout.bar,
        layout.section_plate,
        layout.player_plate,
        layout.triad,
    ]
}

/// The lowest bottom in the columns `left..right` of the viewport's
/// `area`: a gap above every plate or the triad those columns cross.
fn bottom_in(area: Rect, layout: &ControlsLayout, gap: f32, left: f32, right: f32) -> f32 {
    below(layout)
        .into_iter()
        .flatten()
        .filter(|r| r.right() > left && r.left() < right)
        .fold(area.bottom(), |b, r| b.min(r.top() - gap))
}

/// Where a popover may go in the viewport `rect` whose controls are laid
/// out as `layout`: its top left corner, its width and the most height
/// it may take. Hanging from the camera plate (a gap under it, the
/// pointer in that gap), its right edge on the plate's right edge, as
/// wide as `popover_width` or the room left of that edge; above the
/// display bar and the triad where its columns cross them. The cube does
/// not bound it: the cube moves out of its way ([`cube_out_of_the_way`]).
/// `None` when there is no room for two rows.
pub fn popover_place(rect: Rect, layout: &ControlsLayout, tokens: &Tokens) -> Option<Rect> {
    if !rect.is_finite() {
        return None;
    }
    let area = rect.shrink(tokens.nav_margin);
    let gap = tokens.nav_gap;
    let (right, top) = match layout.camera_plate {
        Some(plate) => (plate.right().min(area.right()), plate.bottom() + gap),
        None => (area.right(), area.top()),
    };
    let width = tokens.popover_width.min(right - area.left());
    let least = 2.0 * tokens.nav_height + 2.0 * tokens.nav_inset;
    let left = right - width;
    let height = bottom_in(area, layout, gap, left, right) - top;
    (width.is_finite() && width >= least && height.is_finite() && height >= least)
        .then(|| Rect::from_min_size(pos2(left, top), vec2(width, height)))
}

/// Where the view cube goes while the popover `panel` is open: where it
/// was when the two do not meet; else a gap under the panel, against the
/// same right edge, when it fits there above the plates and the triad;
/// else nowhere (it is left out until the popover closes).
pub fn cube_out_of_the_way(
    cube: Rect,
    panel: Rect,
    rect: Rect,
    layout: &ControlsLayout,
    tokens: &Tokens,
) -> Option<Rect> {
    let gap = tokens.nav_gap;
    if !cube.intersects(panel.expand(gap * 0.5)) {
        return Some(cube);
    }
    let area = rect.shrink(tokens.nav_margin);
    let moved = cube.translate(vec2(0.0, panel.bottom() + gap - cube.top()));
    let bottom = bottom_in(area, layout, gap, moved.left(), moved.right());
    (moved.is_finite() && moved.bottom() <= bottom && area.contains_rect(moved)).then_some(moved)
}

/// The chip of a split button and its caret: the caret is the right
/// `nav_caret_width` of `rect`.
pub fn split_chip(rect: Rect, tokens: &Tokens) -> (Rect, Rect) {
    let caret = tokens.nav_caret_width.clamp(0.0, rect.width().max(0.0));
    let x = rect.right() - caret;
    (
        Rect::from_min_max(rect.min, pos2(x, rect.bottom())),
        Rect::from_min_max(pos2(x, rect.top()), rect.max),
    )
}

/// The ids of the areas egui fades in as they appear: the popovers'
/// (`Shell::popover`) and the narrow layout's drawer.
pub const FADING_AREAS: [&str; 3] = ["studio-options", "analysis-options", "craft3d-drawer"];

impl Shell {
    /// Whether a popover or the drawer is still fading in at egui's time
    /// `now` (egui fades an area in over its animation time once it
    /// becomes visible): a picture of the window waits for it.
    pub fn popover_fading(&self, ctx: &egui::Context, now: f64) -> bool {
        let fade = f64::from(ctx.global_style().animation_time);
        FADING_AREAS.iter().any(|id| {
            egui::AreaState::load(ctx, egui::Id::new(id))
                .and_then(|area| area.last_became_visible_at)
                .is_some_and(|at| now - at < fade)
        })
    }

    /// A popover at `place` ([`popover_place`]) whose pointer points at
    /// `chip`, its options drawn by `options` on the plates' surface.
    /// Returns the panel's rect.
    pub(super) fn popover(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        place: Rect,
        chip: Rect,
        options: impl FnOnce(&mut Self, &mut egui::Ui),
    ) -> Rect {
        let tokens = self.tokens;
        let stroke = 1.0;
        let inset = tokens.nav_inset;
        let inner_width = (place.width() - 2.0 * (inset + stroke)).max(0.0);
        let fill = color_alpha(tokens.plate_fill, tokens.popover_opacity);
        let rim = color_alpha(tokens.plate_rim, tokens.plate_rim_opacity);
        let area = egui::Area::new(egui::Id::new(id))
            .fixed_pos(place.min)
            .constrain(false)
            .show(ui.ctx(), |ui| {
                // Its own room, not the size the area had last frame.
                let room = Rect::from_min_size(ui.max_rect().min, place.size());
                ui.scope_builder(egui::UiBuilder::new().max_rect(room), |ui| {
                    ui.style_mut().visuals = tokens.plate_visuals();
                    egui::Frame::new()
                        .fill(fill)
                        .stroke(egui::Stroke::new(stroke, rim))
                        .corner_radius(tokens.nav_radius)
                        .inner_margin(inset)
                        .show(ui, |ui| {
                            ui.set_width(inner_width);
                            let room = place.height() - 2.0 * (inset + stroke) - 1.0;
                            egui::ScrollArea::both()
                                .id_salt((id, "options"))
                                .auto_shrink([false, true])
                                .max_height(room.max(0.0))
                                .show(ui, |ui| options(self, ui));
                        });
                });
            });
        let panel = area.response.rect;
        // The pointer: a small triangle on the top edge, under the chip
        // where the panel reaches it, else at the panel's nearer end.
        let reach = tokens.popover_pointer;
        if reach > 0.0 && panel.is_finite() && panel.width() > 0.0 {
            let side = f32::from(tokens.nav_radius) + reach;
            let lo = panel.left() + side;
            let hi = panel.right() - side;
            if lo <= hi {
                let x = chip.center().x.clamp(lo, hi);
                // Its base over the rim, so the two read as one outline.
                let top = panel.top() + stroke;
                let painter = ui.ctx().layer_painter(area.response.layer_id);
                let (a, apex, b) = (
                    pos2(x - reach, top),
                    pos2(x, top - reach),
                    pos2(x + reach, top),
                );
                painter.add(egui::Shape::convex_polygon(
                    vec![a, apex, b],
                    fill,
                    egui::Stroke::NONE,
                ));
                let line = egui::Stroke::new(stroke, rim);
                painter.line_segment([a, apex], line);
                painter.line_segment([apex, b], line);
            }
        }
        panel
    }

    /// Escape closes the narrow layout's open drawer, else an open
    /// popover (the analysis', else the studio look's); `false` when
    /// none was open. The look or the analysis stays on.
    pub(super) fn close_popover(&mut self) -> bool {
        if self.narrow() && self.state.panels.drawer {
            // The narrow layout's drawer first.
            self.state.panels.drawer = false;
            true
        } else if self.state.analysis.is_on() && self.state.panels.analysis_options {
            self.state.panels.analysis_options = false;
            true
        } else if self.state.look.is_studio()
            && !self.state.analysis.is_on()
            && self.state.panels.look_options
        {
            self.state.panels.look_options = false;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{ControlWidths, controls_layout};

    /// At every size from tiny to large, with the section on and off: the
    /// place is inside the viewport less its margin and clear of the
    /// plates and the triad; the cube, moved out of its way, is clear of
    /// it and of every other part, or left out; at the sizes the window
    /// opens at the popover hangs from the camera plate, right edges
    /// aligned, and the cube still has a place under it.
    #[test]
    fn the_popover_keeps_clear_of_every_control() {
        // The desktop's sizes, and touch sizes with the compact player's
        // plate (as the narrow layout lays them out).
        for (tokens, player) in [
            (Tokens::default(), false),
            (Tokens::default().touch(), false),
            (Tokens::default().touch(), true),
        ] {
            the_popover_keeps_clear_with(&tokens, player);
        }
    }

    fn the_popover_keeps_clear_with(tokens: &Tokens, player: bool) {
        let tokens = *tokens;
        let widths = ControlWidths::fallback(&tokens);
        let mut shown = 0;
        for w in (40..=1800).step_by(37) {
            for h in (40..=1100).step_by(29) {
                for section in [false, true] {
                    let rect = Rect::from_min_size(pos2(10.0, 30.0), vec2(w as f32, h as f32));
                    let mut layout = controls_layout(rect, &tokens, &widths, section);
                    if player {
                        let side =
                            crate::shell::player::button_side_for(tokens.touch_target, &tokens);
                        crate::shell::layout::compact_player_place(
                            &mut layout,
                            rect,
                            &tokens,
                            5.0 * side + tokens.compact_player_label + 2.0 * tokens.nav_padding,
                            side,
                        );
                    }
                    let Some(place) = popover_place(rect, &layout, &tokens) else {
                        continue;
                    };
                    shown += 1;
                    let area = rect.shrink(tokens.nav_margin);
                    assert!(area.contains_rect(place), "{place:?} in {area:?}");
                    for (name, part) in layout.parts() {
                        if name == "cube" {
                            continue;
                        }
                        assert!(
                            !part.intersects(place),
                            "{w}×{h} {section}: over the {name}: {place:?} {part:?}"
                        );
                    }
                    if let Some(plate) = layout.camera_plate {
                        assert!(place.top() > plate.bottom(), "{w}×{h}");
                        assert!(place.right() <= plate.right() + 1e-3, "{w}×{h}");
                    }
                    // The cube out of the way of the whole place (the
                    // most the panel takes).
                    if let Some(cube) = layout.cube
                        && let Some(moved) =
                            cube_out_of_the_way(cube, place, rect, &layout, &tokens)
                    {
                        assert!(area.contains_rect(moved), "{w}×{h}: {moved:?}");
                        assert!(!moved.intersects(place), "{w}×{h}: {moved:?} {place:?}");
                        for (name, part) in layout.parts() {
                            if name != "cube" {
                                assert!(
                                    !part.intersects(moved),
                                    "{w}×{h} {section}: cube over the {name}"
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(shown > 100, "{shown}");
        for (w, h) in [(1600.0, 1000.0), (1024.0, 700.0)] {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h));
            let layout = controls_layout(rect, &tokens, &widths, false);
            let place = popover_place(rect, &layout, &tokens).unwrap();
            let plate = layout.camera_plate.unwrap();
            assert_eq!(place.right(), plate.right());
            assert_eq!(place.top(), plate.bottom() + tokens.nav_gap);
            // A panel of a typical height (the studio look's options):
            // the cube goes under it.
            let panel = Rect::from_min_size(place.min, vec2(place.width(), 300.0));
            let cube = layout.cube.unwrap();
            let moved = cube_out_of_the_way(cube, panel, rect, &layout, &tokens).unwrap();
            assert_eq!(moved.top(), panel.bottom() + tokens.nav_gap);
            assert_eq!(moved.right(), cube.right());
        }
    }

    #[test]
    fn the_caret_is_the_right_end_of_the_chip() {
        let tokens = Tokens::default();
        let chip = Rect::from_min_size(pos2(100.0, 10.0), vec2(90.0, 28.0));
        let (main, caret) = split_chip(chip, &tokens);
        assert_eq!(main.left(), chip.left());
        assert_eq!(caret.right(), chip.right());
        assert_eq!(main.right(), caret.left());
        assert_eq!(caret.width(), tokens.nav_caret_width);
        // A chip narrower than the caret is all caret, never negative.
        let (main, caret) = split_chip(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(5.0, 28.0)),
            &tokens,
        );
        assert!(main.width() >= 0.0 && caret.width() <= 5.0);
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
