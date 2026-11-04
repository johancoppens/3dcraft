//! The one slider of the app (the developer's decision of 2026-11-04):
//! the design's look (a rail in `track`, the part up to the value in
//! `accent`, a round knob of `knob_size`, white with `knob_rim` on a
//! panel, `plate_text` on a plate; 24 px with touch sizes) with the
//! precision of the old Blender-style number field:
//!
//! - a press on the rail puts the knob there, a press on the knob takes
//!   it where it is; the drag that follows moves the value by the share of
//!   the rail the pointer travels, on a fine grid of round numbers
//!   ([`fine_step`]); with **Shift** ten times finer (on a grid ten times
//!   finer), with **Ctrl** (or Cmd) on whole steps;
//! - with the slider focused (clicked or dragged), the **arrow keys** and
//!   the **wheel** step it, ten steps at a time with Shift;
//! - **typing**: a click on the value (or Enter with the pointer over the
//!   slider) turns it into a text field; Enter sets what was typed, an
//!   expression with units where the caller evaluates it (the Jig form,
//!   the display bar), a plain number otherwise; Escape gives up;
//! - a **double click on the knob** sets the default, where there is one;
//! - a **note** (`clamped to 120 mm`) or an **error** under it.
//!
//! One component, four forms ([`SliderForm`]): a form row (label and a
//! number field with its unit, the rail under them, the range under
//! that: the Jig form, boards s7 and m8), one row (label, rail, value:
//! the popovers and the display bar), the rail alone (the players and
//! the joints) and a drag field without a rail for a value without a
//! range. Whole numbers with a few values take [`Kit::stepper`].
//!
//! The arithmetic is here without egui ([`dragged`], [`stepped`],
//! [`nice_step`], [`fine_step`], [`snapped`], [`shown`]) so tests can ask
//! for it. The state of a drag, of the focus and of typing lives with the
//! caller when it passes a [`SliderState`] (the shell, whose typed text is
//! evaluated with the document's variables), else in egui's memory under
//! the slider's id.

use super::{Ground, Kit, sane};
use crate::theme::{blend, color, color_alpha};
use craft3d_math::Portable;
use egui::epaint::RectShape;
use egui::{
    Align, CornerRadius, Frame, Id, Margin, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    Ui, pos2, vec2,
};
use std::ops::RangeInclusive;

/// The longest text a field takes, in characters: a pasted page is cut
/// there rather than laid out every frame.
pub const MAX_TYPED: usize = 200;

/// How many fine steps a range has, about ([`fine_step`]).
pub const FINE_STEPS: f64 = 100.0;

/// The value after a drag of `dx` points across a rail `width` points
/// wide, from `start`: the whole width is the whole `range`; `fine`
/// (Shift) is ten times finer, `stepped` (Ctrl) rounds to `step`. Within
/// the range. A drag or a range that is not a number leaves the value.
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

/// A round number near `rough`: 1, 2 or 5 times a power of ten, at
/// least `rough`.
fn round_up(rough: f64) -> Option<f64> {
    if !(rough > 0.0 && rough.is_finite()) {
        return None;
    }
    let power = 10f64.ppowf(rough.plog10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * power)
        .find(|&s| s >= rough * (1.0 - 1e-9))
        .unwrap_or(rough);
    (step > 0.0 && step.is_finite()).then_some(step)
}

/// A round step for a range: about a twentieth of it, 1, 2 or 5 times a
/// power of ten.
pub fn nice_step(range: [f64; 2]) -> f64 {
    round_up((range[1] - range[0]) / 20.0).unwrap_or(1.0)
}

/// The grid a plain drag snaps to: about a hundredth of the range, 1, 2
/// or 5 times a power of ten, and never coarser than `step`.
pub fn fine_step(range: [f64; 2], step: f64) -> f64 {
    let fine = round_up((range[1] - range[0]) / FINE_STEPS).unwrap_or(1.0);
    if step > 0.0 && step.is_finite() {
        fine.min(step)
    } else {
        fine
    }
}

/// `value` on the grid of `grid` (from zero), within the range; the
/// value itself for a grid or a value that is not a number.
pub fn snapped(value: f64, grid: f64, range: [f64; 2]) -> f64 {
    let [lo, hi] = range;
    if !(value.is_finite() && grid > 0.0 && grid.is_finite()) {
        return value;
    }
    let mut out = (value / grid).round() * grid;
    if lo.is_finite() && hi.is_finite() && hi >= lo {
        out = out.clamp(lo, hi);
    }
    // Clean the last bits of a decimal grid (0.1 * 3 = 0.30000000000000004).
    let decimals = decimals_of_step(grid);
    let scale = 10f64.ppowi(decimals as i32);
    let cleaned = (out * scale).round() / scale;
    if cleaned.is_finite() { cleaned } else { value }
}

/// How many decimals a grid of `step` needs, up to 9.
fn decimals_of_step(step: f64) -> usize {
    if !(step > 0.0 && step.is_finite()) {
        return 0;
    }
    for decimals in 0..9 {
        let scaled = step * 10f64.ppowi(decimals as i32);
        if (scaled - scaled.round()).abs() <= 1e-9 * scaled.abs().max(1.0) {
            return decimals;
        }
    }
    9
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

/// A range as a hint under a slider: `0.5 – 4 mm` (no unit: `8 – 40`).
pub fn range_hint(lo: f64, hi: f64, unit: &str) -> String {
    let write = |v: f64| {
        let v = if v == 0.0 { 0.0 } else { v };
        let decimals = (0..7)
            .find(|d| {
                let s = 10f64.ppowi(*d);
                ((v * s).round() - v * s).abs() <= 1e-9 * (v * s).abs().max(1.0)
            })
            .unwrap_or(6) as usize;
        format!("{v:.decimals$}")
    };
    if unit.is_empty() {
        format!("{} – {}", write(lo), write(hi))
    } else {
        format!("{} – {} {unit}", write(lo), write(hi))
    }
}

/// `text` cut to at most `max` characters, with an ellipsis when it was
/// longer.
pub fn shortened(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// A plain number typed into a slider whose caller does not evaluate
/// expressions: the number, with or without `unit` after it, divided by
/// `scale` (a percentage's 100).
pub fn typed_plain(text: &str, unit: &str, scale: f64) -> Option<f64> {
    let text = text.trim();
    let bare = if unit.is_empty() {
        text
    } else {
        text.strip_suffix(unit).map_or(text, str::trim_end)
    };
    let v = bare.parse::<f64>().ok()?;
    let v = if scale != 0.0 && scale.is_finite() {
        v / scale
    } else {
        v
    };
    v.is_finite().then_some(v)
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

/// The state of a drag on a slider: the value and the pointer's place it
/// is measured from, and whether Shift was down. Pressing or letting go
/// of Shift starts it afresh from the value reached, so the value does
/// not jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldDrag {
    pub field: &'static str,
    pub start: f64,
    /// The pointer's x where the drag is measured from.
    pub from: f32,
    pub fine: bool,
}

/// What a caller remembers of its sliders between frames: a drag under
/// way and the slider that has the focus (the one last clicked or
/// dragged; the arrows and the wheel step it). A press anywhere else
/// takes the focus away.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FieldMemory {
    pub drag: Option<FieldDrag>,
    pub focus: Option<&'static str>,
}

/// The caller's memory of its sliders and of the one typed into, and the
/// name this slider has in them. Typed text is the caller's to evaluate
/// (it waits in `typing` with `entered` set).
pub struct SliderState<'a> {
    pub field: &'static str,
    pub memory: &'a mut FieldMemory,
    pub typing: &'a mut Option<Typing>,
}

/// The four forms of the slider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SliderForm {
    /// One row: the label, the rail, the value (popovers, display bar).
    #[default]
    Row,
    /// The label and a number field with its unit, the rail under them
    /// across the width (the Jig form).
    Form,
    /// The rail alone (players, joints).
    Rail,
    /// A drag field without a rail: the label and a number field that a
    /// horizontal drag changes (a value without a range).
    Drag,
}

/// A slider: what it shows and how it may be changed.
pub struct Slider<'a> {
    pub label: &'a str,
    pub value: &'a mut f64,
    pub range: RangeInclusive<f64>,
    /// The value as shown; none: written from `scale`, `decimals` and
    /// `unit`.
    pub text: Option<String>,
    pub enabled: bool,
    pub hint: Option<&'a str>,
    /// The widths kept for the label and the value, in points.
    pub label_width: f32,
    pub value_width: f32,
    pub form: SliderForm,
    /// The step of Ctrl, the arrows and the wheel: none for
    /// [`nice_step`] of the range.
    pub step: Option<f64>,
    /// The default a double click on the knob sets.
    pub default: Option<f64>,
    /// A double click on the knob asks the caller for the default
    /// ([`SliderShown::reset`]) instead of setting `default`.
    pub reset_asked: bool,
    /// The value is shown as `value * scale` with `decimals` places and
    /// `unit`; typing reads it back the same way.
    pub unit: &'a str,
    pub scale: f64,
    pub decimals: usize,
    /// A click on the value types into it.
    pub typable: bool,
    /// Said under it: a note in the dim ink, an error in the failure's
    /// (the field ringed red).
    pub note: Option<&'a str>,
    pub error: Option<&'a str>,
    /// The field ringed in the failure's colour without a line under it
    /// (the caller says why elsewhere).
    pub alarm: bool,
    /// A typed value is taken as it is, outside the rail's range too (the
    /// knob then stays at the end): a joint's.
    pub typed_free: bool,
}

impl<'a> Slider<'a> {
    pub fn new(label: &'a str, value: &'a mut f64, range: RangeInclusive<f64>) -> Self {
        Self {
            label,
            value,
            range,
            text: None,
            enabled: true,
            hint: None,
            label_width: 96.0,
            value_width: 64.0,
            form: SliderForm::Row,
            step: None,
            default: None,
            reset_asked: false,
            unit: "",
            scale: 1.0,
            decimals: 2,
            typable: true,
            note: None,
            error: None,
            alarm: false,
            typed_free: false,
        }
    }
    #[must_use]
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }
    #[must_use]
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }
    #[must_use]
    pub fn hint_opt(mut self, hint: Option<&'a str>) -> Self {
        self.hint = hint;
        self
    }
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    #[must_use]
    pub fn widths(mut self, label: f32, value: f32) -> Self {
        self.label_width = label;
        self.value_width = value;
        self
    }
    #[must_use]
    pub fn form(mut self, form: SliderForm) -> Self {
        self.form = form;
        self
    }
    #[must_use]
    pub fn step(mut self, step: f64) -> Self {
        self.step = (step > 0.0 && step.is_finite()).then_some(step);
        self
    }
    #[must_use]
    pub fn default_value(mut self, default: f64) -> Self {
        self.default = default.is_finite().then_some(default);
        self
    }
    #[must_use]
    pub fn reset_asked(mut self, asked: bool) -> Self {
        self.reset_asked = asked;
        self
    }
    /// Shown as `value * scale` with `decimals` places and `unit`.
    #[must_use]
    pub fn shown_as(mut self, scale: f64, decimals: usize, unit: &'a str) -> Self {
        self.scale = if scale.is_finite() && scale != 0.0 {
            scale
        } else {
            1.0
        };
        self.decimals = decimals.min(9);
        self.unit = unit;
        self
    }
    #[must_use]
    pub fn typable(mut self, typable: bool) -> Self {
        self.typable = typable;
        self
    }
    #[must_use]
    pub fn note(mut self, note: Option<&'a str>) -> Self {
        self.note = note;
        self
    }
    #[must_use]
    pub fn typed_free(mut self, free: bool) -> Self {
        self.typed_free = free;
        self
    }
    #[must_use]
    pub fn alarm(mut self, alarm: bool) -> Self {
        self.alarm = alarm;
        self
    }
    #[must_use]
    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }

    fn range2(&self) -> Option<[f64; 2]> {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        (lo.is_finite() && hi.is_finite() && hi > lo).then_some([lo, hi])
    }

    fn the_step(&self, range: [f64; 2]) -> f64 {
        self.step.unwrap_or_else(|| nice_step(range))
    }

    fn written(&self, value: f64) -> String {
        match &self.text {
            Some(text) => text.clone(),
            None => shown(value, self.scale, self.decimals, self.unit),
        }
    }
}

/// What a slider did in a frame.
pub struct SliderShown {
    /// The rail's response (the field's for a drag field); `changed`
    /// when the value moved.
    pub response: Response,
    /// Where the value is typed into: the number field, or the value at
    /// the end of a row.
    pub field: Option<Rect>,
    /// The rail between the knob's two ends (none for a drag field).
    pub rail: Option<Rect>,
    /// The value changed by a press, a drag, a key or the wheel this
    /// frame (typed values come through the caller's typing, or set the
    /// value at once without one).
    pub changed: bool,
    /// The knob is being dragged.
    pub dragging: bool,
    /// A double click on the knob asked for the default
    /// ([`Slider::reset_asked`]).
    pub reset: bool,
    /// Typing began this frame.
    pub typing: bool,
}

/// The slider's own memory when the caller keeps none.
#[derive(Clone, Debug, Default)]
struct OwnState {
    memory: FieldMemory,
    typing: Option<Typing>,
}

/// The name a slider has in its own memory.
const OWN: &str = "slider";

impl Kit {
    /// The knob's diameter.
    fn knob(&self) -> f32 {
        self.t.knob_size
    }

    /// The knob's fill and rim on this ground.
    fn knob_inks(&self) -> (crate::theme::Rgb, crate::theme::Rgb) {
        match self.ground {
            Ground::Panel => (self.skin.knob, self.skin.knob_rim),
            Ground::Plate => (self.t.plate_text, self.t.plate_text),
        }
    }

    /// The height of a slider's main line in `form`.
    pub fn slider_height(&self, form: SliderForm) -> f32 {
        let t = &self.t;
        match form {
            SliderForm::Form => self.hit(t.control_height) + self.hit(self.knob() + 6.0),
            SliderForm::Rail => self.hit(self.knob() + 6.0),
            SliderForm::Row | SliderForm::Drag => self.hit(t.control_height),
        }
    }

    /// A slider across the width there is, keeping its own memory: the
    /// rail's response, `changed` when the value moved.
    pub fn slider(&self, ui: &mut Ui, slider: Slider<'_>) -> Response {
        self.slider_with(ui, slider, None).response
    }

    /// A slider across the width there is, with the caller's memory (or
    /// its own), and the lines under it (the hint, a note, an error).
    pub fn slider_with(
        &self,
        ui: &mut Ui,
        slider: Slider<'_>,
        state: Option<SliderState<'_>>,
    ) -> SliderShown {
        let t = &self.t;
        let form = slider.form;
        // The least room a slider takes: a rail needs some length; a drag
        // field is a number box, as narrow as its caller asks (the three
        // of a direction share one row of a popover).
        let least = match form {
            SliderForm::Drag => t.control_height * 1.5,
            _ => t.control_height * 3.0,
        };
        let width = ui.available_width().max(least);
        let (outer, _) =
            ui.allocate_exact_size(sane(vec2(width, self.slider_height(form))), Sense::hover());
        let hint = slider.hint.map(str::to_string);
        let note = slider.note.map(str::to_string);
        let error = slider.error.map(str::to_string);
        let enabled = slider.enabled;
        let shown = self.slider_in(ui, outer, slider, state);
        // The lines under it: in line with the rail in a row, at the
        // left edge in a form.
        let left = match (form, shown.rail) {
            (SliderForm::Row, Some(rail)) => rail.left() - self.knob() * 0.5,
            _ => outer.left(),
        };
        let mut lines: Vec<(String, crate::theme::Rgb)> = Vec::new();
        if let Some(hint) = hint {
            lines.push((
                hint,
                if enabled {
                    self.skin.text_dim
                } else {
                    self.skin.faint
                },
            ));
        }
        if let Some(note) = note {
            lines.push((note, self.skin.text_dim));
        }
        if let Some(error) = error {
            lines.push((error, self.skin.failed));
        }
        for (text, ink) in lines {
            let room = (outer.right() - left).max(0.0);
            let mut job = egui::text::LayoutJob::simple(
                shortened(&text, 400),
                self.small_font(),
                color(ink),
                room,
            );
            job.wrap.max_rows = 4;
            let galley = ui.painter().layout_job(job);
            let (place, _) =
                ui.allocate_exact_size(sane(vec2(width, galley.size().y)), Sense::hover());
            ui.painter()
                .galley(pos2(left, place.top()), galley, color(ink));
        }
        shown
    }

    /// A slider laid out in `outer` (one line: a row, the rail, a drag
    /// field; two for a form), with the caller's memory or its own. Lines
    /// under it are the caller's ([`Self::slider_with`] draws them).
    pub fn slider_in(
        &self,
        ui: &mut Ui,
        outer: Rect,
        mut slider: Slider<'_>,
        state: Option<SliderState<'_>>,
    ) -> SliderShown {
        // A slider the caller keeps the memory of is known by its name
        // there (two sliders without labels in one bar never share an id);
        // the others by their label and their place in the layout.
        let id = match &state {
            Some(state) => ui.make_persistent_id(("kit-slider", state.field)),
            None => ui
                .make_persistent_id(("kit-slider", slider.label))
                .with(ui.next_auto_id()),
        };
        let shown = match state {
            Some(state) => self.slider_parts(
                ui,
                id,
                outer,
                &mut slider,
                state.field,
                state.memory,
                state.typing,
            ),
            None => {
                let mut own: OwnState = ui.data(|d| d.get_temp::<OwnState>(id)).unwrap_or_default();
                let mut shown = self.slider_parts(
                    ui,
                    id,
                    outer,
                    &mut slider,
                    OWN,
                    &mut own.memory,
                    &mut own.typing,
                );
                // Typed text the slider reads itself: a plain number.
                if let Some(typed) = own.typing.as_mut().filter(|t| t.entered) {
                    typed.entered = false;
                    match typed_plain(&typed.text, slider.unit, slider.scale) {
                        Some(v) => {
                            let v = match slider.range2() {
                                Some([lo, hi]) if !slider.typed_free => v.clamp(lo, hi),
                                _ => v,
                            };
                            *slider.value = v;
                            own.typing = None;
                            shown.changed = true;
                            shown.response.mark_changed();
                            ui.ctx().request_repaint();
                        }
                        None => {
                            typed.error = Some("type a number".to_string());
                            typed.focused = false;
                        }
                    }
                }
                ui.data_mut(|d| d.insert_temp(id, own));
                shown
            }
        };
        let value = *slider.value;
        shown
            .response
            .widget_info(|| egui::WidgetInfo::slider(slider.enabled, value, slider.label));
        shown
    }

    /// The parts of a slider in `outer`, and what the pointer and the
    /// keys did to them.
    #[allow(clippy::too_many_arguments)]
    fn slider_parts(
        &self,
        ui: &mut Ui,
        id: Id,
        outer: Rect,
        slider: &mut Slider<'_>,
        field: &'static str,
        memory: &mut FieldMemory,
        typing: &mut Option<Typing>,
    ) -> SliderShown {
        let t = &self.t;
        let s = &self.skin;
        let knob = self.knob();
        let enabled = slider.enabled;
        let range = slider.range2();
        let step = range.map_or(1.0, |r| slider.the_step(r));
        let line = self.hit(t.control_height);
        // Where each part goes.
        let (label_place, field_place, rail_line) = match slider.form {
            SliderForm::Row => {
                let label_w = if slider.label.is_empty() {
                    0.0
                } else {
                    slider.label_width
                };
                let value_w = slider.value_width;
                let field = Rect::from_min_max(
                    pos2((outer.right() - value_w).max(outer.left()), outer.top()),
                    outer.max,
                );
                let rail = Rect::from_min_max(
                    pos2(outer.left() + label_w, outer.top()),
                    pos2(
                        (outer.right() - value_w - 6.0).max(outer.left() + label_w),
                        outer.bottom(),
                    ),
                );
                (
                    (label_w > 0.0).then(|| {
                        Rect::from_min_max(outer.min, pos2(outer.left() + label_w, outer.bottom()))
                    }),
                    Some(field),
                    Some(rail),
                )
            }
            SliderForm::Form => {
                let top = Rect::from_min_max(outer.min, pos2(outer.right(), outer.top() + line));
                let field_w = slider.value_width.min(top.width() * 0.6);
                let field = Rect::from_min_max(pos2(top.right() - field_w, top.top()), top.max);
                let rail = Rect::from_min_max(pos2(outer.left(), top.bottom()), outer.max);
                (Some(top), Some(field), Some(rail))
            }
            SliderForm::Rail => (None, None, Some(outer)),
            SliderForm::Drag => {
                let field_w = slider.value_width.min(outer.width());
                let field =
                    Rect::from_min_max(pos2(outer.right() - field_w, outer.top()), outer.max);
                (Some(outer), Some(field), None)
            }
        };
        // The rail between the knob's centre at either end.
        let rail = rail_line.map(|l| {
            Rect::from_min_max(
                pos2(l.left() + knob * 0.5, l.top()),
                pos2(
                    (l.right() - knob * 0.5).max(l.left() + knob * 0.5),
                    l.bottom(),
                ),
            )
        });
        let editing =
            enabled && slider.typable && typing.as_ref().is_some_and(|t| t.field == field);
        // The rail takes the mouse; an inert one still does, so a press
        // on it is not a press on the model behind it.
        let sense = Sense::click_and_drag();
        let target = rail
            .map(|r| r.expand2(vec2(knob * 0.5, 0.0)))
            .or(field_place)
            .unwrap_or(outer);
        let mut response = ui.interact(target, id.with("rail"), sense);
        let mut changed = false;
        let mut reset = false;
        let mut began = false;
        let value_before = *slider.value;
        let value_now = if value_before.is_finite() {
            value_before
        } else {
            0.0
        };
        let (pressed, pointer, hover, modifiers, down) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.interact_pos().or(i.pointer.latest_pos()),
                i.pointer.hover_pos(),
                i.modifiers,
                i.pointer.primary_down(),
            )
        });
        let fine = modifiers.shift;
        let whole = modifiers.ctrl || modifiers.command;
        // A press anywhere else takes the focus away.
        if memory.focus == Some(field)
            && pressed
            && !response.contains_pointer()
            && !field_place.is_some_and(|f| pointer.is_some_and(|p| f.contains(p)))
        {
            memory.focus = None;
        }
        let knob_at = |v: f64, rail: Rect| -> f32 {
            let f = range.map_or(0.0, |r| fraction(v, &(r[0]..=r[1])));
            rail.left() + rail.width().max(0.0) * f
        };
        let value_at = |x: f32, rail: Rect| -> Option<f64> {
            let [lo, hi] = range?;
            if rail.width() <= 0.0 {
                return None;
            }
            let share = f64::from(((x - rail.left()) / rail.width()).clamp(0.0, 1.0));
            Some(lo + share * (hi - lo))
        };
        if enabled && !editing {
            match (rail, range) {
                (Some(rail), Some(range)) => {
                    let grid = if whole {
                        step
                    } else if fine {
                        fine_step(range, step) / 10.0
                    } else {
                        fine_step(range, step)
                    };
                    if pressed
                        && response.contains_pointer()
                        && let Some(at) = pointer
                    {
                        memory.focus = Some(field);
                        let on_knob = (at.x - knob_at(value_now, rail)).abs() <= knob * 0.5 + 2.0;
                        let start = if on_knob {
                            value_now
                        } else {
                            value_at(at.x, rail).map_or(value_now, |v| snapped(v, grid, range))
                        };
                        memory.drag = Some(FieldDrag {
                            field,
                            start,
                            from: at.x,
                            fine,
                        });
                        if !on_knob {
                            *slider.value = start;
                            changed = true;
                        }
                    } else if let Some(mut from) = memory.drag.filter(|d| d.field == field) {
                        if down && let Some(at) = pointer {
                            if from.fine != fine {
                                // Shift pressed or let go: on from here, at
                                // the new rate.
                                from = FieldDrag {
                                    field,
                                    start: value_now,
                                    from: at.x,
                                    fine,
                                };
                                memory.drag = Some(from);
                            }
                            let dx = f64::from(at.x - from.from);
                            if dx != 0.0 {
                                let v = dragged(
                                    from.start,
                                    dx,
                                    f64::from(rail.width()),
                                    range,
                                    step,
                                    from.fine,
                                    false,
                                );
                                let v = snapped(v, grid, range);
                                if v != *slider.value {
                                    *slider.value = v;
                                    changed = true;
                                }
                            }
                        } else {
                            memory.drag = None;
                        }
                    }
                    if response.double_clicked()
                        && let Some(at) = pointer
                        && (at.x - knob_at(value_now, rail)).abs() <= knob * 0.5 + 4.0
                    {
                        memory.drag = None;
                        if slider.reset_asked {
                            reset = true;
                        } else if let Some(default) = slider.default {
                            *slider.value = default.clamp(range[0], range[1]);
                            changed = true;
                        }
                    }
                }
                (None, Some(range)) => {
                    // A drag field: a drag across it moves the value by the
                    // share of its width, as the old number field did.
                    let width = target.width();
                    if response.drag_started()
                        && let Some(at) = pointer
                    {
                        memory.focus = Some(field);
                        memory.drag = Some(FieldDrag {
                            field,
                            start: value_now,
                            from: at.x,
                            fine,
                        });
                    }
                    if response.dragged()
                        && let (Some(mut from), Some(at)) =
                            (memory.drag.filter(|d| d.field == field), pointer)
                    {
                        if from.fine != fine {
                            from = FieldDrag {
                                field,
                                start: value_now,
                                from: at.x,
                                fine,
                            };
                            memory.drag = Some(from);
                        }
                        let v = dragged(
                            from.start,
                            f64::from(at.x - from.from),
                            f64::from(width),
                            range,
                            step,
                            from.fine,
                            whole,
                        );
                        if v != *slider.value {
                            *slider.value = v;
                            changed = true;
                        }
                    }
                    if !down && memory.drag.is_some_and(|d| d.field == field) {
                        memory.drag = None;
                    }
                }
                _ => {}
            }
            // The keys and the wheel step the focused slider.
            if memory.focus == Some(field)
                && let Some(range) = range
            {
                let times = if fine { 10 } else { 1 };
                let mut steps = 0;
                // The arrows are the slider's only with the pointer over
                // it: elsewhere they step the history player as always.
                let over = hover.is_some_and(|p| outer.contains(p));
                ui.input_mut(|i| {
                    if !over {
                        return;
                    }
                    for key in [egui::Key::ArrowRight, egui::Key::ArrowUp] {
                        if i.consume_key(egui::Modifiers::SHIFT, key)
                            || i.consume_key(egui::Modifiers::NONE, key)
                        {
                            steps += times;
                        }
                    }
                    for key in [egui::Key::ArrowLeft, egui::Key::ArrowDown] {
                        if i.consume_key(egui::Modifiers::SHIFT, key)
                            || i.consume_key(egui::Modifiers::NONE, key)
                        {
                            steps -= times;
                        }
                    }
                });
                if over {
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
                        steps += if scroll > 0.0 { times } else { -times };
                    }
                    // The wheel over the focused slider is the slider's:
                    // nothing behind it scrolls.
                    ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
                }
                if steps != 0 {
                    let v = stepped(value_now, steps, step, range);
                    *slider.value = v;
                    changed = true;
                }
            }
            // Typing: a click on the value, or Enter with the pointer
            // over the slider.
            let enter_over = hover.is_some_and(|p| outer.contains(p))
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && !ui.ctx().egui_wants_keyboard_input();
            let value_clicked = field_place
                .is_some_and(|f| ui.interact(f, id.with("value"), Sense::click()).clicked())
                && slider.form != SliderForm::Drag;
            let drag_clicked = slider.form == SliderForm::Drag && response.clicked();
            if slider.typable && (enter_over || value_clicked || drag_clicked) {
                *typing = Some(Typing {
                    field,
                    text: shown(value_now, slider.scale, slider.decimals, ""),
                    error: None,
                    entered: false,
                    focused: false,
                });
                memory.drag = None;
                began = true;
            }
        }
        if changed {
            response.mark_changed();
            ui.ctx().request_repaint();
        }
        let dragging = memory.drag.is_some_and(|d| d.field == field) && down;
        let editing =
            enabled && slider.typable && typing.as_ref().is_some_and(|t| t.field == field);
        // Painting.
        if ui.is_rect_visible(outer) {
            let ink = if enabled { s.text } else { s.faint };
            let painter = ui.painter().clone();
            if let Some(place) = label_place
                && !slider.label.is_empty()
            {
                let room = match field_place {
                    Some(f) if slider.form != SliderForm::Row => {
                        (f.left() - place.left() - 6.0).max(0.0)
                    }
                    _ => place.width().max(0.0),
                };
                let mut job = egui::text::LayoutJob::simple_singleline(
                    slider.label.to_string(),
                    self.font(),
                    color(ink),
                );
                job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                let galley = painter.layout_job(job);
                painter.galley(
                    pos2(place.left(), place.center().y - galley.size().y * 0.5),
                    galley,
                    color(ink),
                );
            }
            if let (Some(rail), Some(line)) = (rail, rail_line) {
                let thick = (t.progress_height * 0.75).max(1.0);
                let y = line.center().y;
                let x = knob_at(*slider.value, rail);
                let full = Rect::from_min_max(
                    pos2(rail.left() - knob * 0.25, y - thick * 0.5),
                    pos2(rail.right() + knob * 0.25, y + thick * 0.5),
                );
                let r = CornerRadius::same(super::controls::radius_of(thick));
                painter.rect_filled(full, r, color(s.track));
                let done = if enabled { t.accent } else { s.faint };
                if range.is_some() {
                    painter.rect_filled(
                        Rect::from_min_max(full.min, pos2(x, full.bottom())),
                        r,
                        color(done),
                    );
                }
                let (fill, rim) = if enabled {
                    self.knob_inks()
                } else {
                    (blend(s.faint, 140, s.fill), s.faint)
                };
                let hot =
                    enabled && (dragging || response.hovered() || memory.focus == Some(field));
                let radius = knob * 0.5;
                painter.circle(
                    pos2(x, y),
                    radius,
                    color(fill),
                    Stroke::new(
                        if hot { 1.5 } else { 1.0 },
                        color(if hot && memory.focus == Some(field) {
                            s.focus
                        } else {
                            rim
                        }),
                    ),
                );
            }
            if let Some(place) = field_place
                && !editing
            {
                let text = slider.written(*slider.value);
                match slider.form {
                    SliderForm::Row => {
                        let galley = self.galley(ui, text, self.font(), ink);
                        painter.galley(
                            pos2(
                                place.right() - galley.size().x,
                                place.center().y - galley.size().y * 0.5,
                            ),
                            galley,
                            color(ink),
                        );
                    }
                    SliderForm::Form | SliderForm::Drag => {
                        self.value_box(
                            ui,
                            place,
                            &text,
                            slider,
                            enabled,
                            response.hovered() && slider.form == SliderForm::Drag,
                        );
                    }
                    SliderForm::Rail => {}
                }
            }
        }
        if editing && let Some(place) = field_place {
            let place =
                Rect::from_center_size(place.center(), vec2(place.width(), t.control_height));
            self.typing_box(ui, id, place, slider, typing, began);
        }
        SliderShown {
            response,
            field: field_place,
            rail,
            changed,
            dragging,
            reset,
            typing: began,
        }
    }

    /// The number field of a form or a drag field: the value and its unit
    /// in the kit's field.
    fn value_box(
        &self,
        ui: &Ui,
        place: Rect,
        text: &str,
        slider: &Slider<'_>,
        enabled: bool,
        hovered: bool,
    ) {
        let t = &self.t;
        let s = &self.skin;
        let rect = Rect::from_center_size(place.center(), vec2(place.width(), t.control_height));
        let fill = if hovered {
            self.hovered_fill(s.field)
        } else {
            s.field
        };
        let ring = if slider.error.is_some() || slider.alarm {
            Stroke::new(1.5, color(s.failed))
        } else {
            Stroke::new(1.0, color(s.field_stroke))
        };
        let painter = ui.painter();
        painter.add(RectShape::new(
            rect,
            self.field_radius(),
            color(fill),
            ring,
            StrokeKind::Inside,
        ));
        let ink = if enabled { s.text } else { s.faint };
        let pad = t.field_text_pad;
        // The value without its unit, the unit dim at the right.
        let (number, unit) = match (slider.text.as_ref(), slider.unit) {
            (None, unit) if !unit.is_empty() => (
                shown(*slider.value, slider.scale, slider.decimals, ""),
                unit.to_string(),
            ),
            _ => (text.to_string(), String::new()),
        };
        let mut right = rect.right() - pad;
        if !unit.is_empty() {
            let galley = self.galley(ui, unit, self.font(), s.text_dim);
            right -= galley.size().x;
            painter.galley(
                pos2(right, rect.center().y - galley.size().y * 0.5),
                galley,
                color(if enabled { s.text_dim } else { s.faint }),
            );
            right -= t.nav_icon_gap;
        }
        let room = (right - rect.left() - pad).max(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(number, self.font(), color(ink));
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let galley = painter.layout_job(job);
        painter.galley(
            pos2(rect.left() + pad, rect.center().y - galley.size().y * 0.5),
            galley,
            color(ink),
        );
    }

    /// The field while it is typed into: the kit's field with the focus
    /// ring (red, with the reason, after an Enter that did not give a
    /// number), the text selected when typing begins so what is typed
    /// replaces it.
    fn typing_box(
        &self,
        ui: &mut Ui,
        id: Id,
        rect: Rect,
        slider: &Slider<'_>,
        typing: &mut Option<Typing>,
        fresh: bool,
    ) {
        let t = &self.t;
        let s = &self.skin;
        let Some(state) = typing.as_mut() else {
            return;
        };
        let back = ui.painter().add(Shape::Noop);
        let pad = t.field_text_pad;
        let unit_galley = (!slider.unit.is_empty() && slider.form != SliderForm::Row)
            .then(|| self.galley(ui, slider.unit, self.font(), s.text_dim));
        let right = rect.right()
            - pad
            - unit_galley
                .as_ref()
                .map_or(0.0, |g| g.size().x + t.nav_icon_gap);
        let inner = Rect::from_min_max(
            pos2(rect.left() + pad, rect.top()),
            pos2(right.max(rect.left() + pad), rect.bottom()),
        );
        let text_id = id.with("typed");
        let panels = ui.visuals().clone();
        {
            let visuals = ui.visuals_mut();
            visuals.selection.bg_fill = color_alpha(s.focus, 90);
            visuals.selection.stroke.color = color(s.text);
            visuals.text_cursor.stroke.color = color(s.text);
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let edit = child.add(
            egui::TextEdit::singleline(&mut state.text)
                .id(text_id)
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .font(self.font())
                .char_limit(MAX_TYPED)
                .text_color(color(s.text))
                .vertical_align(Align::Center)
                .desired_width(inner.width()),
        );
        *ui.visuals_mut() = panels;
        if !state.focused {
            // The whole text selected, so typing replaces it.
            edit.request_focus();
            if let Some(mut text_state) = egui::text_edit::TextEditState::load(ui.ctx(), text_id) {
                let all = state.text.chars().count();
                text_state
                    .cursor
                    .set_char_range(Some(egui::text_selection::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(all),
                    )));
                text_state.store(ui.ctx(), text_id);
            }
            state.focused = true;
        }
        if edit.changed() {
            state.error = None;
        }
        let ring = if state.error.is_some() {
            Stroke::new(1.5, color(s.failed))
        } else {
            Stroke::new(1.5, color(s.focus))
        };
        ui.painter().set(
            back,
            RectShape::new(
                rect,
                self.field_radius(),
                color(s.field),
                ring,
                StrokeKind::Inside,
            ),
        );
        if let Some(galley) = unit_galley {
            ui.painter().galley(
                pos2(
                    rect.right() - pad - galley.size().x,
                    rect.center().y - galley.size().y * 0.5,
                ),
                galley,
                color(s.text_dim),
            );
        }
        let (enter, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        // The key that began the typing (Enter) does not end it.
        if escape && !fresh {
            *typing = None;
            return;
        }
        if !fresh && (enter || (edit.lost_focus() && state.focused)) {
            state.entered = true;
        }
        if let Some(error) = &state.error {
            // Over the field, on the notice's dark plate: it stays
            // readable on a panel and on a plate.
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                id.with("typing-error"),
            ));
            let font = self.small_font();
            let galley =
                painter.layout_no_wrap(shortened(error, 120), font, color(t.chrome_failed));
            let at = pos2(rect.left(), rect.top() - 4.0 - galley.size().y);
            let back = Rect::from_min_size(at, galley.size()).expand2(vec2(6.0, 3.0));
            painter.rect_filled(
                back,
                CornerRadius::same(t.field_radius),
                color_alpha(t.notice_fill, t.notice_opacity),
            );
            painter.galley(at, galley, color(t.chrome_failed));
        }
    }

    /// A stepper for a whole number with a few values: `−  3  +`, its
    /// response `changed` when a button moved the value or a number was
    /// typed. Each button is a target of its own (44 px with touch
    /// sizes); a click on the number types into it, as on a slider's
    /// value: Enter (or a click elsewhere) sets the whole number typed,
    /// held to the range, Escape gives up, and what is not a number keeps
    /// the field open with a red ring.
    pub fn stepper(
        &self,
        ui: &mut Ui,
        value: &mut f64,
        range: [f64; 2],
        enabled: bool,
    ) -> Response {
        let t = &self.t;
        let s = &self.skin;
        let side = self.hit(t.control_height);
        let middle = (t.control_height * 1.6).max(28.0);
        // Known by its place in the layout, not on the screen: a scroll
        // keeps its id.
        let place_id = ui.next_auto_id();
        let (outer, _) =
            ui.allocate_exact_size(sane(vec2(2.0 * side + middle, side)), Sense::hover());
        let box_rect = Rect::from_center_size(
            outer.center(),
            vec2(outer.width(), t.control_height.min(outer.height())),
        );
        let minus = Rect::from_min_size(outer.min, vec2(side, side));
        let plus = Rect::from_min_size(pos2(outer.right() - side, outer.top()), vec2(side, side));
        let id = ui.make_persistent_id(("kit-stepper", place_id));
        let lo_r = ui.interact(minus, id.with("-"), Sense::click());
        let hi_r = ui.interact(plus, id.with("+"), Sense::click());
        let mut response = ui.interact(outer, id, Sense::hover());
        let v = if value.is_finite() { *value } else { range[0] };
        let can_down = enabled && v > range[0];
        let can_up = enabled && v < range[1];
        let mut new = v;
        if lo_r.clicked() && can_down {
            new = stepped(v, -1, 1.0, range);
        }
        if hi_r.clicked() && can_up {
            new = stepped(v, 1, 1.0, range);
        }
        // The number between the buttons: a click types into it.
        let number = Rect::from_min_max(
            pos2(minus.right(), box_rect.top()),
            pos2(plus.left(), box_rect.bottom()),
        );
        let number_r = ui.interact(number, id.with("number"), Sense::click());
        let typing_id = id.with("typing");
        let mut typing: Option<Typing> = ui.data(|d| d.get_temp(typing_id));
        if !enabled {
            typing = None;
        }
        let fresh = enabled && typing.is_none() && number_r.clicked();
        if fresh {
            typing = Some(Typing {
                field: "stepper",
                text: format!("{}", v.round()),
                ..Typing::default()
            });
        }
        if typing.is_none() && enabled && number_r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        if ui.is_rect_visible(outer) || typing.is_some() {
            let ring = match &typing {
                Some(typed) if typed.error.is_some() => Stroke::new(1.5, color(s.failed)),
                Some(_) => Stroke::new(1.5, color(s.focus)),
                None => Stroke::new(1.0, color(s.field_stroke)),
            };
            ui.painter().add(RectShape::new(
                box_rect,
                self.field_radius(),
                color(s.field),
                ring,
                StrokeKind::Inside,
            ));
        }
        if let Some(state) = typing.as_mut() {
            let text_id = id.with("typed");
            let panels = ui.visuals().clone();
            {
                let visuals = ui.visuals_mut();
                visuals.selection.bg_fill = color_alpha(s.focus, 90);
                visuals.selection.stroke.color = color(s.text);
                visuals.text_cursor.stroke.color = color(s.text);
            }
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(number.shrink2(vec2(2.0, 0.0)))
                    .layout(egui::Layout::left_to_right(Align::Center)),
            );
            let edit = child.add(
                egui::TextEdit::singleline(&mut state.text)
                    .id(text_id)
                    .frame(Frame::NONE)
                    .margin(Margin::ZERO)
                    .font(self.font())
                    .char_limit(MAX_TYPED)
                    .text_color(color(s.text))
                    .horizontal_align(Align::Center)
                    .vertical_align(Align::Center)
                    .desired_width(number.width() - 4.0),
            );
            *ui.visuals_mut() = panels;
            if !state.focused {
                // The whole number selected, so what is typed replaces it.
                edit.request_focus();
                if let Some(mut text_state) =
                    egui::text_edit::TextEditState::load(ui.ctx(), text_id)
                {
                    let all = state.text.chars().count();
                    text_state.cursor.set_char_range(Some(
                        egui::text_selection::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(all),
                        ),
                    ));
                    text_state.store(ui.ctx(), text_id);
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
            if escape && !fresh {
                typing = None;
            } else if !fresh && (enter || edit.lost_focus()) {
                match typed_plain(&state.text, "", 1.0) {
                    Some(typed) => {
                        new = typed.round().clamp(range[0], range[1]);
                        typing = None;
                    }
                    None => {
                        state.error = Some("type a whole number".to_string());
                        state.focused = false;
                    }
                }
            }
        }
        ui.data_mut(|d| match &typing {
            Some(state) => {
                d.insert_temp(typing_id, state.clone());
            }
            None => {
                d.remove::<Typing>(typing_id);
            }
        });
        if new != v && new.is_finite() {
            *value = new;
            response.mark_changed();
        }
        if ui.is_rect_visible(outer) {
            let painter = ui.painter();
            for (rect, text, live, r) in [(minus, "−", can_down, &lo_r), (plus, "+", can_up, &hi_r)]
            {
                let ink = if live { s.text } else { s.faint };
                if live && r.hovered() {
                    let inner = Rect::from_center_size(
                        rect.center(),
                        vec2(rect.width(), box_rect.height()),
                    )
                    .shrink(2.0);
                    painter.rect_filled(
                        inner,
                        self.field_radius(),
                        color(self.hovered_fill(s.field)),
                    );
                }
                let galley = self.galley(ui, text, self.font(), ink);
                painter.galley(rect.center() - galley.size() * 0.5, galley, color(ink));
            }
            if typing.is_none() {
                let ink = if enabled { s.text } else { s.faint };
                let galley = self.galley(ui, format!("{}", v.round()), self.font(), ink);
                painter.galley(box_rect.center() - galley.size() * 0.5, galley, color(ink));
            }
        }
        response.widget_info(|| egui::WidgetInfo::slider(enabled, v, "stepper"));
        response
    }
}

/// Where `value` lies in `range`, 0 to 1 (0 for what means nothing).
pub fn fraction(value: f64, range: &RangeInclusive<f64>) -> f32 {
    let (lo, hi) = (*range.start(), *range.end());
    if !(value.is_finite() && lo.is_finite() && hi.is_finite()) || hi <= lo {
        return 0.0;
    }
    ((value - lo) / (hi - lo)).clamp(0.0, 1.0) as f32
}

/// The knob's centre for `value` on `rail` (for tests that press it).
pub fn knob_centre(value: f64, range: [f64; 2], rail: Rect) -> Pos2 {
    let f = fraction(value, &(range[0]..=range[1]));
    pos2(rail.left() + rail.width().max(0.0) * f, rail.center().y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_across_the_rail_is_the_range_shift_finer_ctrl_in_steps() {
        let range = [0.0, 60.0];
        assert_eq!(dragged(0.0, 200.0, 200.0, range, 5.0, false, false), 60.0);
        assert_eq!(dragged(10.0, 50.0, 200.0, range, 5.0, false, false), 25.0);
        assert_eq!(dragged(10.0, -50.0, 200.0, range, 5.0, false, false), 0.0);
        assert!((dragged(10.0, 50.0, 200.0, range, 5.0, true, false) - 11.5).abs() < 1e-12);
        assert_eq!(dragged(10.0, 37.0, 200.0, range, 5.0, false, true), 20.0);
        assert_eq!(dragged(10.0, 3.0, 200.0, range, 5.0, false, true), 10.0);
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

    /// The plain drag's grid is a round hundredth of the range, never
    /// coarser than the step; values on it are clean decimals.
    #[test]
    fn the_fine_grid_is_round_and_snapping_leaves_clean_decimals() {
        assert_eq!(fine_step([0.0, 0.9], 0.05), 0.01);
        assert_eq!(fine_step([0.5, 4.0], 0.2), 0.05);
        assert_eq!(fine_step([20.0, 100.0], 1.0), 1.0);
        assert_eq!(
            fine_step([0.0, 1000.0], 1.0),
            1.0,
            "never coarser than the step"
        );
        assert_eq!(fine_step([0.0, 0.0], 1.0), 1.0);
        assert_eq!(snapped(0.30000000000000004, 0.1, [0.0, 1.0]), 0.3);
        assert_eq!(snapped(0.226, 0.01, [0.0, 0.9]), 0.23);
        assert_eq!(snapped(5.0, 0.01, [0.0, 0.9]), 0.9);
        assert!(snapped(f64::NAN, 0.01, [0.0, 0.9]).is_nan());
        assert_eq!(snapped(0.226, f64::NAN, [0.0, 0.9]), 0.226);
        assert_eq!(range_hint(0.5, 4.0, "mm"), "0.5 – 4 mm");
        assert_eq!(range_hint(8.0, 40.0, ""), "8 – 40");
        assert_eq!(typed_plain(" 35 %", "%", 100.0), Some(0.35));
        assert_eq!(typed_plain("2.5", "", 1.0), Some(2.5));
        assert_eq!(typed_plain("abc", "", 1.0), None);
        assert_eq!(typed_plain("inf", "", 1.0), None);
    }
}
