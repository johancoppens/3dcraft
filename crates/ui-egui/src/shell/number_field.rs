//! A number field over the viewport, as in Blender: its label on the left
//! (dim), its value on the right (bright), a thin accent bar along the
//! bottom that shows where the value lies in its range. A horizontal drag
//! changes the value (the field's whole width is the whole range; with
//! Shift ten times finer; with Ctrl in steps), the ‹ › at its ends (shown
//! under the pointer) and the wheel over it step it, and a double click or
//! Enter under the pointer lets you type an expression with units (`width
//! / 2 - 3 mm`), evaluated as the document's expressions are when you press
//! Enter; Escape gives up.
//!
//! The arithmetic is here, without egui, so tests can ask for it; the
//! widget is [`NumberField::show`].

use crate::theme::{Tokens, color, color_alpha};
use egui::{Rect, pos2, vec2};

/// The value after a drag of `dx` points across a field `width` points
/// wide, from `start`: the whole width is the whole `range`; `fine` (Shift)
/// is ten times finer, `stepped` (Ctrl) rounds to `step`. Within the range.
/// A drag or a range that is not a number leaves the value.
pub fn dragged(
    start: f64,
    dx: f64,
    width: f64,
    range: [f64; 2],
    step: f64,
    fine: bool,
    stepped: bool,
) -> f64 {
    let [lo, hi] = range;
    if !(start.is_finite() && dx.is_finite() && width > 0.0 && lo.is_finite() && hi > lo) {
        return start;
    }
    let mut delta = dx / width * (hi - lo);
    if fine {
        delta /= 10.0;
    }
    let mut value = start + delta;
    if stepped && step > 0.0 && step.is_finite() {
        value = (value / step).round() * step;
    }
    let value = value.clamp(lo, hi);
    if value.is_finite() { value } else { start }
}

/// The value `steps` steps of `step` from `value` (an arrow or the
/// wheel), on the grid of steps and within the range.
pub fn stepped(value: f64, steps: i32, step: f64, range: [f64; 2]) -> f64 {
    let [lo, hi] = range;
    if !(value.is_finite() && step > 0.0 && step.is_finite() && lo.is_finite() && hi > lo) {
        return value;
    }
    let grid = (value / step).round();
    // On the grid already: one step; between two: to the next one that way.
    let base = if (grid * step - value).abs() <= step * 1e-9 {
        grid
    } else if steps > 0 {
        (value / step).floor()
    } else {
        (value / step).ceil()
    };
    let out = ((base + f64::from(steps)) * step).clamp(lo, hi);
    if out.is_finite() { out } else { value }
}

/// A round step for a range: about a twentieth of it, 1, 2 or 5 times a
/// power of ten.
pub fn nice_step(range: [f64; 2]) -> f64 {
    let span = range[1] - range[0];
    if !(span > 0.0 && span.is_finite()) {
        return 1.0;
    }
    let rough = span / 20.0;
    let power = 10f64.powf(rough.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * power)
        .find(|&s| s >= rough)
        .unwrap_or(rough);
    if step > 0.0 && step.is_finite() {
        step
    } else {
        1.0
    }
}

/// How a field shows its value: `decimals` places and the unit.
pub fn shown(value: f64, scale: f64, decimals: usize, unit: &str) -> String {
    let v = value * scale;
    let v = if v == 0.0 { 0.0 } else { v };
    if unit.is_empty() {
        format!("{v:.decimals$}")
    } else {
        format!("{v:.decimals$} {unit}")
    }
}

/// A field being typed into: its text and, after an Enter that did not
/// give a number, why.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Typing {
    pub field: &'static str,
    pub text: String,
    pub error: Option<String>,
    /// Enter was pressed: the text waits to be evaluated.
    pub entered: bool,
    /// The text box has had the keyboard: losing it again ends the typing.
    pub focused: bool,
}

/// What a number field shows and allows.
pub struct NumberField<'a> {
    /// Its name, for its hit and its typing.
    pub id: &'static str,
    pub label: &'a str,
    pub value: f64,
    pub range: [f64; 2],
    /// The step of Ctrl, the arrows and the wheel.
    pub step: f64,
    /// Shown as `value * scale` with `decimals` places and `unit`.
    pub scale: f64,
    pub decimals: usize,
    pub unit: &'a str,
    /// Drawn dimmed and inert.
    pub enabled: bool,
    /// Whether `range` is the value's real range (the accent bar shows
    /// where the value is in it) or a stand-in.
    pub range_known: bool,
}

/// What happened to a field in a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FieldOutcome {
    /// A new value from a drag, an arrow or the wheel.
    pub value: Option<f64>,
    /// Typing began.
    pub typing: bool,
}

/// The state of a drag on a field: where it began, at what value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldDrag {
    pub field: &'static str,
    pub start: f64,
}

impl NumberField<'_> {
    /// Draws the field in `rect` and handles its mouse and keys. `drag` is
    /// the shell's memory of a drag under way, `typing` of the field being
    /// typed into.
    pub fn show(
        &self,
        ui: &mut egui::Ui,
        rect: Rect,
        tokens: &Tokens,
        drag: &mut Option<FieldDrag>,
        typing: &mut Option<Typing>,
    ) -> FieldOutcome {
        let mut outcome = FieldOutcome::default();
        let painter = ui.painter().clone();
        let radius = egui::CornerRadius::same(tokens.field_radius);
        let font = egui::FontId::proportional(tokens.control_font);
        let editing = typing.as_ref().is_some_and(|t| t.field == self.id);
        if editing && self.enabled {
            self.typing_box(ui, rect, tokens, typing);
            return outcome;
        }
        // An inert field still takes the mouse, so a click on it is not a
        // click on the model behind it; it does nothing with it.
        let response = ui.interact(
            rect,
            ui.id().with(("number-field", self.id)),
            egui::Sense::click_and_drag(),
        );
        let fade = |c: egui::Color32| {
            if self.enabled {
                c
            } else {
                c.gamma_multiply(tokens.disabled_opacity)
            }
        };
        painter.rect_filled(
            rect,
            radius,
            fade(color_alpha(tokens.glass, tokens.control_opacity)),
        );
        let [lo, hi] = self.range;
        let frac = if hi > lo {
            ((self.value - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // The bar shows where the value lies in a range it has; an inert
        // field (a section that is off) still shows it, dimmed.
        if frac > 0.0 && frac.is_finite() && (self.enabled || self.range_known) {
            let bar = Rect::from_min_max(
                pos2(rect.left() + 4.0, rect.bottom() - 4.0),
                pos2(
                    rect.left() + 4.0 + (rect.width() - 8.0) * frac as f32,
                    rect.bottom() - 2.0,
                ),
            );
            painter.rect_filled(bar, egui::CornerRadius::same(1), fade(color(tokens.accent)));
        }
        let hovered = self.enabled && (response.hovered() || response.dragged());
        let inset = if hovered { 22.0 } else { 10.0 };
        let middle = rect.center().y - 1.0;
        if hovered {
            painter.rect_stroke(
                rect,
                radius,
                egui::Stroke::new(1.0, color(tokens.view_cube_hover_outline)),
                egui::StrokeKind::Inside,
            );
            for (x, arrow, align) in [
                (rect.left() + 9.0, "‹", egui::Align2::LEFT_CENTER),
                (rect.right() - 9.0, "›", egui::Align2::RIGHT_CENTER),
            ] {
                painter.text(
                    pos2(x, middle),
                    align,
                    arrow,
                    egui::FontId::proportional(tokens.control_font + 2.0),
                    color(tokens.text),
                );
            }
        }
        painter.text(
            pos2(rect.left() + inset, middle),
            egui::Align2::LEFT_CENTER,
            self.label,
            font.clone(),
            fade(color(tokens.text_dim)),
        );
        painter.text(
            pos2(rect.right() - inset, middle),
            egui::Align2::RIGHT_CENTER,
            shown(self.value, self.scale, self.decimals, self.unit),
            font,
            fade(color(tokens.text)),
        );
        if !self.enabled {
            return outcome;
        }
        let modifiers = ui.input(|i| i.modifiers);
        if response.drag_started() {
            *drag = Some(FieldDrag {
                field: self.id,
                start: self.value,
            });
        }
        if response.dragged()
            && let (Some(from), Some(total)) = (
                drag.filter(|d| d.field == self.id),
                response.total_drag_delta(),
            )
        {
            let value = dragged(
                from.start,
                f64::from(total.x),
                f64::from(rect.width()),
                self.range,
                self.step,
                modifiers.shift,
                modifiers.ctrl || modifiers.command,
            );
            if value != self.value {
                outcome.value = Some(value);
            }
        }
        if response.drag_stopped() {
            *drag = None;
        }
        if response.double_clicked()
            || (response.hovered() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
        {
            *typing = Some(Typing {
                field: self.id,
                text: shown(self.value, self.scale, self.decimals, ""),
                error: None,
                entered: false,
                focused: false,
            });
            outcome.typing = true;
            outcome.value = None;
            return outcome;
        }
        if response.clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            let steps = if at.x < rect.left() + 18.0 {
                -1
            } else if at.x > rect.right() - 18.0 {
                1
            } else {
                0
            };
            if steps != 0 {
                outcome.value = Some(stepped(self.value, steps, self.step, self.range));
            }
        }
        if response.hovered() {
            let scroll: f32 = ui.input(|i| {
                i.raw
                    .events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                        _ => None,
                    })
                    .sum()
            });
            if scroll != 0.0 && scroll.is_finite() {
                let steps = if scroll > 0.0 { 1 } else { -1 };
                outcome.value = Some(stepped(self.value, steps, self.step, self.range));
            }
        }
        outcome
    }

    /// The field while it is typed into: a dark box with the text, an
    /// accent outline (red, with the reason above it, after an Enter that
    /// did not give a number).
    fn typing_box(
        &self,
        ui: &mut egui::Ui,
        rect: Rect,
        tokens: &Tokens,
        typing: &mut Option<Typing>,
    ) {
        let Some(state) = typing.as_mut() else {
            return;
        };
        let painter = ui.painter().clone();
        let radius = egui::CornerRadius::same(tokens.field_radius);
        let outline = if state.error.is_some() {
            tokens.failed
        } else {
            tokens.accent
        };
        painter.rect_filled(rect, radius, color(tokens.field_typing));
        painter.rect_stroke(
            rect,
            radius,
            egui::Stroke::new(1.0, color(outline)),
            egui::StrokeKind::Inside,
        );
        let inner = rect.shrink2(vec2(8.0, 2.0));
        let id = ui.id().with(("number-text", self.id));
        let edit = ui.put(
            inner,
            egui::TextEdit::singleline(&mut state.text)
                .id(id)
                .frame(egui::Frame::NONE)
                .font(egui::FontId::proportional(tokens.control_font))
                .text_color(color(tokens.text))
                .vertical_align(egui::Align::Center),
        );
        if !state.focused {
            // As in Blender: the whole text selected, so typing replaces it.
            edit.request_focus();
            if let Some(mut text_state) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
                let all = state.text.chars().count();
                text_state
                    .cursor
                    .set_char_range(Some(egui::text_selection::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(all),
                    )));
                text_state.store(ui.ctx(), id);
            }
            state.focused = true;
        }
        if edit.changed() {
            state.error = None;
        }
        let (enter, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if escape {
            *typing = None;
            return;
        }
        if enter || (edit.lost_focus() && state.focused) {
            state.entered = true;
        }
        if let Some(error) = &state.error {
            let font = egui::FontId::proportional(tokens.control_font - 1.0);
            let galley = painter.layout_no_wrap(error.clone(), font, color(tokens.failed));
            let at = pos2(rect.left(), rect.top() - 4.0 - galley.size().y);
            let back = Rect::from_min_size(at, galley.size()).expand2(vec2(6.0, 3.0));
            painter.rect_filled(
                back,
                radius,
                color_alpha(tokens.notice_fill, tokens.notice_opacity),
            );
            painter.galley(at, galley, color(tokens.failed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_across_the_field_is_the_range_shift_finer_ctrl_in_steps() {
        let range = [0.0, 60.0];
        // A whole width is the whole range; half of it half.
        assert_eq!(dragged(0.0, 200.0, 200.0, range, 5.0, false, false), 60.0);
        assert_eq!(dragged(10.0, 50.0, 200.0, range, 5.0, false, false), 25.0);
        assert_eq!(dragged(10.0, -50.0, 200.0, range, 5.0, false, false), 0.0);
        // Shift: ten times finer.
        assert!((dragged(10.0, 50.0, 200.0, range, 5.0, true, false) - 11.5).abs() < 1e-12);
        // Ctrl: on the steps.
        assert_eq!(dragged(10.0, 37.0, 200.0, range, 5.0, false, true), 20.0);
        assert_eq!(dragged(10.0, 3.0, 200.0, range, 5.0, false, true), 10.0);
        // Within the range, whatever the drag; nothing for numbers that
        // are not.
        assert_eq!(dragged(10.0, 1e9, 200.0, range, 5.0, false, false), 60.0);
        assert_eq!(
            dragged(10.0, f64::NAN, 200.0, range, 5.0, false, false),
            10.0
        );
        assert_eq!(dragged(10.0, 5.0, 0.0, range, 5.0, false, false), 10.0);
        assert_eq!(
            dragged(10.0, 5.0, 100.0, [3.0, 3.0], 5.0, false, false),
            10.0
        );
    }

    #[test]
    fn steps_go_to_the_next_step_and_stay_in_the_range() {
        let range = [0.0, 0.9];
        assert!((stepped(0.35, 1, 0.05, range) - 0.4).abs() < 1e-12);
        assert!((stepped(0.35, -1, 0.05, range) - 0.3).abs() < 1e-12);
        assert!((stepped(0.33, 1, 0.05, range) - 0.35).abs() < 1e-12);
        assert!((stepped(0.33, -1, 0.05, range) - 0.3).abs() < 1e-12);
        assert_eq!(stepped(0.9, 1, 0.05, range), 0.9);
        assert_eq!(stepped(0.0, -1, 0.05, range), 0.0);
        assert_eq!(stepped(0.5, 1, f64::NAN, range), 0.5);
        assert_eq!(nice_step([0.0, 60.0]), 5.0);
        assert_eq!(nice_step([0.0, 0.9]), 0.05);
        assert_eq!(nice_step([0.0, 1000.0]), 50.0);
        assert_eq!(nice_step([1.0, 1.0]), 1.0);
        assert_eq!(shown(0.35, 100.0, 0, "%"), "35 %");
        assert_eq!(shown(12.5, 1.0, 2, "mm"), "12.50 mm");
        assert_eq!(shown(-0.0, 1.0, 1, ""), "0.0");
    }
}
