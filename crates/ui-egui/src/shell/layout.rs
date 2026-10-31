//! Where the controls over the viewport go, and their widgets.
//!
//! - **The navigation cluster** in the top right corner: the camera plate
//!   (*Fit* and *Iso*, actions; a divider; *Persp* | *Ortho*, a choice) and
//!   under it, against the same right margin, the view cube (2026-10-21;
//!   it was above; flush right since 2026-10-30). The Studio and Analysis
//!   chips are split buttons whose caret opens their options in a popover
//!   hanging from the plate; the cube moves under it (`popover.rs`).
//! - **The display bar** at the bottom centre: *Sketches*, *Datums* and
//!   *Edges* (toggles) and the *See-through* field, a divider, the *Section*
//!   toggle. While the section is on its options open inline to the right
//!   of the Section chip, after a divider: *X* | *Y* | *Z*, the *Offset*
//!   field and *Flip side* (which half is kept); the chip stays where it
//!   is. Off, the options are not there at all.
//! - **The axis triad** in the bottom left corner.
//!
//! One size system from the tokens (`nav_*`): every control is
//! `nav_height` high on a glass plate `nav_padding` larger, every plate
//! `nav_margin` from the viewport's edge, groups `nav_gap` apart with a
//! divider between them. One visual grammar: a *toggle* is a chip with an
//! icon that is tinted with the accent while it is on; an *action* is a
//! plain button that lights up under the pointer; a *choice* is a
//! segmented track whose chosen segment is filled.
//!
//! In a viewport too narrow for the names, the display bar's three
//! toggles show their icons only first (with the name as a tooltip), and
//! where that is not enough every chip does; what still does not fit is left
//! out (the triad first, then the cube), so nothing is ever drawn outside
//! the viewport or over another control. They are widgets over the
//! viewport, so a click on them is not a click in the viewport and picks
//! nothing.

use super::Shell;
use super::controls::{
    ControlsLayout, SECTION_PLANES, choose_axis, chosen_axis, plate, toggle_section,
};
use super::icons::{self, Icon};
use crate::state::ProjectionKind;
use crate::theme::{Tokens, color, color_alpha};
use craft3d_engine::Scene;
use egui::{Pos2, Rect, pos2, vec2};

/// The widths the controls need for their texts, in points, measured
/// each frame ([`ControlWidths::measure`]) so the layout fits any font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlWidths {
    /// Chips and buttons with an icon and a name.
    pub fit: f32,
    pub iso: f32,
    pub sketches: f32,
    pub datums: f32,
    pub edges: f32,
    pub studio: f32,
    pub analysis: f32,
    pub section: f32,
    pub flip: f32,
    /// One segment of Persp | Ortho.
    pub projection: f32,
    /// One segment of X | Y | Z.
    pub axis: f32,
    /// The number fields.
    pub see_through: f32,
    pub offset: f32,
}

impl ControlWidths {
    /// The widths as the texts need them in the controls' font.
    pub fn measure(painter: &egui::Painter, tokens: &Tokens) -> Self {
        let font = egui::FontId::proportional(tokens.control_font);
        let text = |s: &str| {
            painter
                .layout_no_wrap(s.to_string(), font.clone(), egui::Color32::WHITE)
                .size()
                .x
        };
        Self::from_text(tokens, &text)
    }

    /// The widths from how wide `text` says each text is.
    pub fn from_text(tokens: &Tokens, text: &dyn Fn(&str) -> f32) -> Self {
        let inset = tokens.nav_inset;
        let with_icon =
            |s: &str| inset + tokens.nav_icon_size + tokens.nav_icon_gap + text(s) + inset + 2.0;
        let plain = |s: &str| (2.0 * inset + text(s) + 4.0).max(tokens.nav_height);
        let field = |label: &str, value: &str| {
            (text(label) + text(value) + 2.0 * tokens.field_inset + tokens.field_arrow_inset)
                .max(tokens.nav_height)
        };
        let widest = |items: &[&str], f: &dyn Fn(&str) -> f32| {
            items.iter().map(|s| f(s)).fold(0.0, f32::max)
        };
        let out = Self {
            fit: with_icon("Fit"),
            iso: with_icon("Iso"),
            sketches: with_icon("Sketches"),
            datums: with_icon("Datums"),
            edges: with_icon("Edges"),
            studio: with_icon("Studio"),
            analysis: with_icon("Analysis"),
            section: with_icon("Section"),
            flip: with_icon("Flip side"),
            projection: widest(&["Persp", "Ortho"], &plain),
            axis: widest(&["X", "Y", "Z"], &plain),
            see_through: field("See-through", "90 %"),
            offset: field("Offset", "-000.00 mm"),
        };
        if out.all_finite() {
            out
        } else {
            Self::fallback(tokens)
        }
    }

    fn all_finite(&self) -> bool {
        [
            self.fit,
            self.iso,
            self.sketches,
            self.datums,
            self.edges,
            self.studio,
            self.analysis,
            self.section,
            self.flip,
            self.projection,
            self.axis,
            self.see_through,
            self.offset,
        ]
        .iter()
        .all(|w| w.is_finite() && *w >= 0.0)
    }

    /// Widths when the texts could not be measured: seven points a
    /// character.
    pub fn fallback(tokens: &Tokens) -> Self {
        let chars = |n: f32| n * 7.0;
        let icon = 2.0 * tokens.nav_inset + tokens.nav_icon_size + tokens.nav_icon_gap + 2.0;
        Self {
            fit: icon + chars(3.0),
            iso: icon + chars(3.0),
            sketches: icon + chars(8.0),
            datums: icon + chars(6.0),
            edges: icon + chars(5.0),
            studio: icon + chars(6.0),
            analysis: icon + chars(8.0),
            section: icon + chars(7.0),
            flip: icon + chars(9.0),
            projection: 2.0 * tokens.nav_inset + chars(5.0) + 4.0,
            axis: tokens.nav_height,
            see_through: tokens.field_width,
            offset: tokens.field_width,
        }
    }
}

/// The side of the triad's square place: an arm and a letter on either
/// side of the origin.
pub fn triad_side(tokens: &Tokens) -> f32 {
    2.0 * (tokens.nav_triad_arm + tokens.nav_triad_font)
}

/// Where the problem text goes in the viewport `rect` whose controls are
/// laid out as `layout`, for a text `natural` points wide on one line: its
/// left, the width it is wrapped to and its bottom. Right of the triad and
/// left of the display bar when it fits there on one line; otherwise over
/// the bar, as wide as the viewport allows. `None` when there is no room.
pub fn problem_place(
    rect: Rect,
    layout: &ControlsLayout,
    tokens: &Tokens,
    natural: f32,
) -> Option<[f32; 3]> {
    let (margin, gap) = (tokens.nav_margin, tokens.nav_gap);
    let left = layout
        .triad
        .map_or(rect.left() + margin, |t| t.right() + gap);
    let bottom = rect.bottom() - margin;
    let (right, bottom) = match layout.bar {
        Some(bar) if bar.left() - gap - left >= natural => (bar.left() - gap, bottom),
        Some(bar) => (rect.right() - margin, bar.top() - gap),
        None => (rect.right() - margin, bottom),
    };
    let width = right - left;
    (width.is_finite() && width > 0.0 && bottom.is_finite()).then_some([left, width, bottom])
}

/// Where the controls go in the viewport `rect`, with the section on or
/// off (see the top of this module).
pub fn controls_layout(
    rect: Rect,
    tokens: &Tokens,
    widths: &ControlWidths,
    section_on: bool,
) -> ControlsLayout {
    let empty = ControlsLayout::default();
    if !(rect.is_finite() && rect.width() > 0.0 && rect.height() > 0.0) {
        return empty;
    }
    let widths = if widths.all_finite() {
        *widths
    } else {
        ControlWidths::fallback(tokens)
    };
    let area = rect.shrink(tokens.nav_margin);
    if !(area.width() > 0.0 && area.height() > 0.0) {
        return empty;
    }
    let build = |compact: bool, tight: bool, wrap: bool| {
        let n = Nav {
            tokens,
            w: widths,
            area,
            compact,
            tight,
            wrap,
            on: section_on,
        };
        let mut layout = n.layout();
        layout.compact = compact;
        layout
    };
    // Named; then the display toggles by their icons; then every chip.
    // The bar is placed for its options open either way, so the choice is
    // the same with the section on and off and nothing moves when it
    // opens.
    // Last, the section's options on a plate of their own above the bar
    // (a phone).
    let full = build(false, false, false);
    let mut layout = if complete(&full, area, section_on) {
        full
    } else {
        let tight = build(false, true, false);
        if complete(&tight, area, section_on) {
            tight
        } else {
            let compact = build(true, true, false);
            let wrapped = build(true, true, true);
            if complete(&compact, area, section_on) || !fits_bar(&wrapped, area) {
                compact
            } else {
                wrapped
            }
        }
    };
    prune(&mut layout, area);
    layout
}

/// The measures of the layout, for one viewport.
struct Nav<'a> {
    tokens: &'a Tokens,
    w: ControlWidths,
    /// The viewport less the margin.
    area: Rect,
    /// Chips by their icons only.
    compact: bool,
    /// The display bar's toggles (Sketches, Datums, Edges) by their
    /// icons only.
    tight: bool,
    /// The section's options on a plate of their own above the bar.
    wrap: bool,
    /// The section is on: its options are laid out.
    on: bool,
}

impl Nav<'_> {
    fn h(&self) -> f32 {
        self.tokens.nav_height
    }

    fn pad(&self) -> f32 {
        self.tokens.nav_padding
    }

    /// A plate's height for one row.
    fn plate_h(&self) -> f32 {
        self.h() + 2.0 * self.pad()
    }

    /// A chip that is `natural` wide with its name, square without.
    fn chip(&self, natural: f32) -> f32 {
        if self.compact { self.h() } else { natural }
    }

    /// One of the display bar's toggles: [`Self::chip`], square too
    /// while the layout is tight.
    fn show_chip(&self, natural: f32) -> f32 {
        if self.tight {
            self.h()
        } else {
            self.chip(natural)
        }
    }

    /// The triad's place, at the bottom left.
    fn triad(&self) -> Rect {
        let side = triad_side(self.tokens);
        Rect::from_min_size(
            pos2(self.area.left(), self.area.bottom() - side),
            vec2(side, side),
        )
    }

    /// A control `w` wide whose left is at `x`, on a row whose top is `y`.
    fn at(&self, x: f32, y: f32, w: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, self.h()))
    }

    /// A vertical divider in the middle of the gap that starts at `x`, on
    /// a row whose top is `y`.
    fn vertical(&self, x: f32, y: f32) -> [Pos2; 2] {
        let mid = x + self.tokens.nav_gap * 0.5;
        let inset = self.h() * self.tokens.divider_inset;
        [pos2(mid, y + inset), pos2(mid, y + self.h() - inset)]
    }

    /// The camera group (Fit, Iso, a divider, Persp | Ortho, then the
    /// Studio look) from `x` on a row whose top is `y`, `projection` wide
    /// for the choice; returns where it ends.
    #[allow(clippy::too_many_arguments)]
    fn camera_row(
        &self,
        out: &mut ControlsLayout,
        x: f32,
        y: f32,
        fit: f32,
        iso: f32,
        projection: f32,
        (studio, analysis): (f32, f32),
    ) -> f32 {
        let sp = self.tokens.nav_spacing;
        out.fit = Some(self.at(x, y, fit));
        let x = x + fit + sp;
        out.iso = Some(self.at(x, y, iso));
        let x = x + iso;
        out.dividers[0] = Some(self.vertical(x, y));
        let x = x + self.tokens.nav_gap;
        out.projection = Some(self.at(x, y, projection));
        let x = x + projection + self.tokens.nav_gap;
        out.studio = Some(self.at(x, y, studio));
        let x = x + studio + self.tokens.nav_spacing;
        out.analysis = Some(self.at(x, y, analysis));
        x + analysis
    }

    /// The camera plate top right over the cube; the display bar at the
    /// bottom centre with the section inline; the triad bottom left.
    fn layout(&self) -> ControlsLayout {
        let t = self.tokens;
        let mut out = ControlsLayout::default();
        let (pad, sp, gap) = (self.pad(), t.nav_spacing, t.nav_gap);
        let area = self.area;
        // The cluster in the top right corner.
        let (fit, iso) = (self.chip(self.w.fit), self.chip(self.w.iso));
        let projection = 2.0 * self.w.projection;
        // The Studio look beside the projection: how the view is drawn,
        // on the plate with room to spare (the display bar is full at a
        // window of 1280 × 800 with the section's options open).
        // Split buttons: the chip, and a caret on its right that opens
        // and closes the popover of its options.
        let studio = self.chip(self.w.studio) + t.nav_caret_width;
        let analysis = self.chip(self.w.analysis) + t.nav_caret_width;
        let camera_w = pad + fit + sp + iso + gap + projection + gap + studio + sp + analysis + pad;
        let side = t.nav_cube_size;
        // Both flush against the right margin, the cube under the plate
        // (2026-10-30, the developer: the cube all the way to the right).
        let camera = Rect::from_min_size(
            pos2(area.right() - camera_w, area.top()),
            vec2(camera_w, self.plate_h()),
        );
        let cube = Rect::from_min_size(
            pos2(area.right() - side, camera.bottom() + gap),
            vec2(side, side),
        );
        out.cube = Some(cube);
        out.camera_plate = Some(camera);
        self.camera_row(
            &mut out,
            camera.left() + pad,
            camera.top() + pad,
            fit,
            iso,
            projection,
            (studio, analysis),
        );
        // The display bar, centred on what it is with the section off.
        let triad = self.triad();
        out.triad = Some(triad);
        let (sketches, datums, edges) = (
            self.show_chip(self.w.sketches),
            self.show_chip(self.w.datums),
            self.show_chip(self.w.edges),
        );
        let section = self.chip(self.w.section);
        let see = self.w.see_through;
        let off_w = pad + sketches + sp + datums + sp + edges + sp + see + gap + section + pad;
        let flip = self.chip(self.w.flip);
        let axes = 3.0 * self.w.axis;
        let extra = if self.wrap {
            0.0
        } else {
            gap + axes + sp + self.w.offset + sp + flip
        };
        let bar_w = if self.on { off_w + extra } else { off_w };
        // Placed for the bar with the options open, so that the Section
        // chip stays where it is when they open: centred when that fits.
        let left = (area.center().x - off_w * 0.5)
            .max(triad.right() + gap)
            .min(area.right() - off_w - extra);
        let bar = Rect::from_min_size(
            pos2(left, area.bottom() - self.plate_h()),
            vec2(bar_w, self.plate_h()),
        );
        out.bar = Some(bar);
        let y = bar.top() + pad;
        let mut x = bar.left() + pad;
        out.sketches = Some(self.at(x, y, sketches));
        x += sketches + sp;
        out.datums = Some(self.at(x, y, datums));
        x += datums + sp;
        out.edges = Some(self.at(x, y, edges));
        x += edges + sp;
        out.see_through = Some(self.at(x, y, see));
        x += see;
        out.dividers[1] = Some(self.vertical(x, y));
        x += gap;
        out.section_toggle = Some(self.at(x, y, section));
        x += section;
        if self.on && self.wrap {
            // Above the bar, right-aligned with it.
            let w = pad + axes + sp + self.w.offset + sp + flip + pad;
            let right = bar.right().max(area.left() + w).min(area.right());
            let plate = Rect::from_min_size(
                pos2(right - w, bar.top() - gap - self.plate_h()),
                vec2(w, self.plate_h()),
            );
            out.section_plate = Some(plate);
            let y = plate.top() + pad;
            let mut x = plate.left() + pad;
            out.axes = Some(self.at(x, y, axes));
            x += axes + sp;
            out.offset = Some(self.at(x, y, self.w.offset));
            x += self.w.offset + sp;
            out.side = Some(self.at(x, y, flip));
        } else if self.on {
            out.dividers[2] = Some(self.vertical(x, y));
            x += gap;
            out.axes = Some(self.at(x, y, axes));
            x += axes + sp;
            out.offset = Some(self.at(x, y, self.w.offset));
            x += self.w.offset + sp;
            out.side = Some(self.at(x, y, flip));
        }
        out
    }
}

/// Where the narrow layout's compact player goes in the viewport `rect`
/// whose controls are `layout`: a plate `width` wide (at most the
/// viewport's less its margins) for a row of buttons `side` high, a gap
/// above the display bar or the section's plate, centred on the bar. Left
/// out where it would reach the cube or the camera plate; the triad makes
/// way for it.
pub fn compact_player_place(
    layout: &mut ControlsLayout,
    rect: Rect,
    tokens: &Tokens,
    width: f32,
    side: f32,
) {
    layout.player_plate = None;
    let area = rect.shrink(tokens.nav_margin);
    let Some(bar) = layout.bar else {
        return;
    };
    let width = width.min(area.width());
    let height = side + 2.0 * tokens.nav_padding;
    if !(width.is_finite() && height.is_finite() && width > 0.0) {
        return;
    }
    let below = layout
        .section_plate
        .map_or(bar.top(), |p| p.top().min(bar.top()));
    let left = (bar.center().x - width * 0.5)
        .max(area.left())
        .min(area.right() - width);
    let plate = Rect::from_min_size(
        pos2(left, below - tokens.nav_gap - height),
        vec2(width, height),
    );
    let clear = [layout.cube, layout.camera_plate]
        .into_iter()
        .flatten()
        .all(|r| !r.intersects(plate));
    if !(clear && area.expand(0.5).contains_rect(plate)) {
        return;
    }
    if layout.triad.is_some_and(|t| t.intersects(plate)) {
        layout.triad = None;
    }
    layout.player_plate = Some(plate);
}

/// The display bar (and the section's plate, when it has one) fit inside
/// `area`: what a wrapped layout must do to be worth taking.
fn fits_bar(layout: &ControlsLayout, area: Rect) -> bool {
    [layout.bar, layout.section_plate]
        .into_iter()
        .flatten()
        .all(|r| area.expand(0.5).contains_rect(r))
}

/// True when every control is there (the section's options too when the
/// section is on), and every plate, the cube and the triad are inside
/// `area` and clear of each other.
fn complete(layout: &ControlsLayout, area: Rect, section_on: bool) -> bool {
    let controls = [
        layout.sketches,
        layout.datums,
        layout.edges,
        layout.studio,
        layout.analysis,
        layout.see_through,
        layout.section_toggle,
        layout.fit,
        layout.iso,
        layout.projection,
        layout.cube,
        layout.triad,
    ];
    let options = [layout.axes, layout.offset, layout.side];
    if controls.iter().any(Option::is_none) || (section_on && options.iter().any(Option::is_none)) {
        return false;
    }
    let parts = layout.parts();
    parts.iter().enumerate().all(|(k, (_, a))| {
        area.expand(0.5).contains_rect(*a)
            && parts
                .iter()
                .skip(k + 1)
                .all(|(_, b)| !a.shrink(0.01).intersects(*b))
    })
}

/// Leaves out what does not fit: a plate outside `area` or over a plate
/// kept before it (the bar first, then the camera plate), with its
/// controls; the cube and then the triad when they
/// are outside or over something kept.
fn prune(layout: &mut ControlsLayout, area: Rect) {
    let inside = |r: Rect| area.expand(0.5).contains_rect(r);
    let mut kept: Vec<Rect> = Vec::new();
    for slot in [
        &mut layout.bar,
        &mut layout.section_plate,
        &mut layout.camera_plate,
    ] {
        if let Some(plate) = *slot {
            if inside(plate) && kept.iter().all(|k| !k.shrink(0.01).intersects(plate)) {
                kept.push(plate);
            } else {
                *slot = None;
            }
        }
    }
    // A plate left out takes its controls with it, also one that happens
    // to lie within the other plate.
    if layout.camera_plate.is_none() {
        layout.fit = None;
        layout.iso = None;
        layout.projection = None;
        layout.studio = None;
        layout.analysis = None;
        layout.dividers[0] = None;
    }
    if layout.bar.is_none() {
        layout.section_plate = None;
    }
    if layout.bar.is_none() {
        for slot in [
            &mut layout.sketches,
            &mut layout.datums,
            &mut layout.edges,
            &mut layout.see_through,
            &mut layout.section_toggle,
            &mut layout.axes,
            &mut layout.offset,
            &mut layout.side,
        ] {
            *slot = None;
        }
        for divider in layout.dividers.iter_mut().skip(1) {
            *divider = None;
        }
    }
    let on_plate = |r: &Rect| kept.iter().any(|p| p.expand(0.01).contains_rect(*r));
    for slot in [
        &mut layout.sketches,
        &mut layout.datums,
        &mut layout.edges,
        &mut layout.studio,
        &mut layout.analysis,
        &mut layout.see_through,
        &mut layout.section_toggle,
        &mut layout.axes,
        &mut layout.offset,
        &mut layout.side,
        &mut layout.fit,
        &mut layout.iso,
        &mut layout.projection,
    ] {
        if slot.is_some_and(|r| !on_plate(&r)) {
            *slot = None;
        }
    }
    for divider in &mut layout.dividers {
        if divider.is_some_and(|[a, b]| !on_plate(&Rect::from_two_pos(a, b))) {
            *divider = None;
        }
    }
    // The section's options go together.
    if layout.axes.is_none() || layout.offset.is_none() || layout.side.is_none() {
        layout.axes = None;
        layout.offset = None;
        layout.side = None;
    }
    let mut taken = kept;
    for slot in [&mut layout.cube, &mut layout.triad] {
        if let Some(r) = *slot {
            if inside(r) && taken.iter().all(|k| !k.shrink(0.01).intersects(r)) {
                taken.push(r);
            } else {
                *slot = None;
            }
        }
    }
}

/// What a toggle shows: its id, its icon and its name, and whether the
/// name is shown (otherwise it is the tooltip).
struct Chip<'a> {
    id: &'a str,
    icon: Icon,
    label: &'a str,
    named: bool,
}

/// A toggle chip: an icon and a name (or the icon alone), tinted with the
/// accent while `on`. Returns the response.
fn toggle(
    ui: &mut egui::Ui,
    rect: Rect,
    chip: Chip<'_>,
    on: bool,
    tokens: &Tokens,
) -> egui::Response {
    let Chip {
        id,
        icon,
        label,
        named,
    } = chip;
    let response = ui.interact(rect, ui.id().with(("nav-toggle", id)), egui::Sense::click());
    let painter = ui.painter();
    let radius = egui::CornerRadius::same(tokens.nav_control_radius);
    if on {
        painter.rect_filled(
            rect,
            radius,
            color_alpha(tokens.accent, tokens.nav_on_opacity),
        );
        painter.rect_stroke(
            rect,
            radius,
            egui::Stroke::new(
                1.0,
                color_alpha(tokens.accent, tokens.nav_on_opacity.saturating_mul(2)),
            ),
            egui::StrokeKind::Inside,
        );
    } else {
        // Off but enabled: a faint rim, bright text.
        painter.rect_stroke(
            rect.shrink(0.5),
            radius,
            egui::Stroke::new(
                1.0,
                color_alpha(tokens.nav_off_text, tokens.nav_off_rim_opacity),
            ),
            egui::StrokeKind::Inside,
        );
    }
    if response.hovered() {
        painter.rect_filled(
            rect,
            radius,
            color_alpha(tokens.glass, tokens.control_opacity),
        );
    }
    let ink = color(if on { tokens.knob } else { tokens.nav_off_text });
    content(painter, rect, icon, label, named, ink, tokens);
    if named {
        response
    } else {
        response.on_hover_text(label)
    }
}

/// How the caret of a split button is: on the accent with the chip,
/// its popover open, and whether it can be clicked.
#[derive(Clone, Copy)]
struct CaretState {
    on: bool,
    open: bool,
    enabled: bool,
}

/// The caret of a split button: on the accent with the chip while it is
/// on, a divider on its left; a chevron that points down while the
/// popover is closed and up while it is open; greyed out and not
/// clickable while it is not enabled.
fn caret(
    ui: &mut egui::Ui,
    rect: Rect,
    id: &str,
    state: CaretState,
    tokens: &Tokens,
) -> egui::Response {
    let CaretState { on, open, enabled } = state;
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let response = ui.interact(rect, ui.id().with(("nav-caret", id)), sense);
    let painter = ui.painter();
    let radius = egui::CornerRadius::same(tokens.nav_control_radius);
    if on {
        painter.rect_filled(
            rect,
            radius,
            color_alpha(tokens.accent, tokens.nav_on_opacity),
        );
    } else {
        painter.rect_stroke(
            rect.shrink(0.5),
            radius,
            egui::Stroke::new(
                1.0,
                color_alpha(tokens.nav_off_text, tokens.nav_off_rim_opacity),
            ),
            egui::StrokeKind::Inside,
        );
    }
    if enabled && (response.hovered() || open) {
        painter.rect_filled(
            rect,
            radius,
            color_alpha(tokens.glass, tokens.control_opacity),
        );
    }
    let ink = color(match (on, enabled) {
        (_, false) => tokens.plate_text_dim,
        (true, true) => tokens.knob,
        (false, true) => tokens.nav_off_text,
    });
    // The divider between the chip and its caret.
    let inset = rect.height() * tokens.divider_inset;
    painter.line_segment(
        [
            pos2(rect.left(), rect.top() + inset),
            pos2(rect.left(), rect.bottom() - inset),
        ],
        egui::Stroke::new(
            tokens.divider_width,
            color_alpha(
                if on { tokens.knob } else { tokens.plate_rim },
                tokens.divider_opacity,
            ),
        ),
    );
    let r = tokens.nav_icon_size * tokens.nav_caret_chevron;
    let c = rect.center();
    let dy = if open { -r * 0.5 } else { r * 0.5 };
    let stroke = egui::Stroke::new(tokens.nav_icon_width, ink);
    painter.line_segment([c + vec2(-r, -dy), c + vec2(0.0, dy)], stroke);
    painter.line_segment([c + vec2(0.0, dy), c + vec2(r, -dy)], stroke);
    response
}

/// An action: an icon and a name on no fill, lit under the pointer.
fn button(
    ui: &mut egui::Ui,
    rect: Rect,
    id: &str,
    icon: Icon,
    label: &str,
    named: bool,
    tokens: &Tokens,
) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(("nav-button", id)), egui::Sense::click());
    let painter = ui.painter();
    let radius = egui::CornerRadius::same(tokens.nav_control_radius);
    let alpha = if response.is_pointer_button_down_on() {
        tokens.control_opacity.saturating_mul(3)
    } else if response.hovered() {
        tokens.control_opacity.saturating_mul(2)
    } else {
        0
    };
    painter.rect_filled(rect, radius, color_alpha(tokens.glass, alpha));
    content(
        painter,
        rect,
        icon,
        label,
        named,
        color(tokens.plate_text),
        tokens,
    );
    response
}

/// An icon and a name, centred together in `rect`; the icon alone, centred,
/// when not `named`.
fn content(
    painter: &egui::Painter,
    rect: Rect,
    icon: Icon,
    label: &str,
    named: bool,
    ink: egui::Color32,
    tokens: &Tokens,
) {
    let side = tokens.nav_icon_size;
    if !named {
        icons::paint(
            painter,
            icon,
            Rect::from_center_size(rect.center(), vec2(side, side)),
            ink,
            tokens,
        );
        return;
    }
    let font = egui::FontId::proportional(tokens.control_font);
    let galley = painter.layout_no_wrap(label.to_string(), font, ink);
    let width = side + tokens.nav_icon_gap + galley.size().x;
    let left = rect.center().x - width * 0.5;
    icons::paint(
        painter,
        icon,
        icons::place(left, rect.center().y, side),
        ink,
        tokens,
    );
    let text_left = left + side + tokens.nav_icon_gap;
    // A point up, as the number fields' texts are.
    painter.galley(
        pos2(text_left, rect.center().y - galley.size().y * 0.5 - 1.0),
        galley,
        ink,
    );
}

/// A choice: a segmented track, the chosen segment filled with the
/// accent. Returns the segments and the one clicked.
fn choice(
    ui: &mut egui::Ui,
    rect: Rect,
    id: &str,
    items: &[(Option<Icon>, &str)],
    active: Option<usize>,
    named: bool,
    tokens: &Tokens,
) -> (Vec<Rect>, Option<usize>) {
    let painter = ui.painter().clone();
    let radius = tokens.nav_control_radius;
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(radius),
        color_alpha(tokens.glass, tokens.control_opacity),
    );
    let n = items.len().max(1) as f32;
    let w = rect.width() / n;
    let mut cells = Vec::new();
    let mut clicked = None;
    let inner = egui::CornerRadius::same(tokens.nav_segment_radius);
    for (k, (icon, label)) in items.iter().enumerate() {
        let cell = Rect::from_min_size(
            pos2(rect.left() + w * k as f32, rect.top()),
            vec2(w, rect.height()),
        );
        cells.push(cell);
        let mut response = ui.interact(
            cell,
            ui.id().with(("nav-choice", id, k)),
            egui::Sense::click(),
        );
        let on = active == Some(k);
        if on {
            painter.rect_filled(
                cell.shrink(tokens.nav_segment_inset),
                inner,
                color(tokens.accent),
            );
        } else if response.hovered() {
            painter.rect_filled(
                cell.shrink(tokens.nav_segment_inset),
                inner,
                color_alpha(tokens.glass, tokens.control_opacity),
            );
        }
        let ink = color(if on { tokens.knob } else { tokens.plate_text });
        match icon {
            Some(icon) => {
                content(&painter, cell, *icon, label, named, ink, tokens);
                if !named {
                    response = response.on_hover_text(*label);
                }
            }
            None => {
                painter.text(
                    cell.center() - vec2(0.0, 1.0),
                    egui::Align2::CENTER_CENTER,
                    *label,
                    egui::FontId::proportional(tokens.control_font),
                    ink,
                );
            }
        }
        if response.clicked() {
            clicked = Some(k);
        }
    }
    (cells, clicked)
}

impl Shell {
    /// The controls over the viewport `rect` (see the top of this module).
    pub(super) fn viewport_controls(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        scene: Option<&Scene>,
    ) {
        let tokens = self.tokens;
        let painter = ui.painter_at(rect);
        let widths = ControlWidths::measure(&painter, &tokens);
        let mut layout = controls_layout(rect, &tokens, &widths, self.state.section.on);
        // The narrow layout's compact player, while the drawer with the
        // whole player is shut.
        let timeline = self.timeline.steps > 0 || self.timeline.has_timeline;
        // A popover that is shown has the room first: the player only
        // where it leaves the popover a place.
        let popover_shown = self.state.analysis.is_on() || self.state.look.is_studio();
        if self.narrow() && timeline && !self.state.panels.drawer {
            let side = super::player::button_side_for(tokens.touch_target, &tokens);
            let width = self.compact_player_width(side, tokens.compact_player_label)
                + 2.0 * tokens.nav_padding;
            let before = layout;
            compact_player_place(&mut layout, rect, &tokens, width, side);
            if popover_shown && super::popover_place(rect, &layout, &tokens).is_none() {
                layout = before;
            }
        }
        self.controls_layout = Some(layout);
        for place in [
            layout.bar,
            layout.section_plate,
            layout.player_plate,
            layout.camera_plate,
        ]
        .into_iter()
        .flatten()
        {
            plate(&painter, place, &tokens);
            // A click between two controls is not a pick in the viewport.
            ui.interact(
                place,
                ui.id()
                    .with(("nav-plate", place.min.x.to_bits(), place.min.y.to_bits())),
                egui::Sense::click_and_drag(),
            );
        }
        for [a, b] in layout.dividers.into_iter().flatten() {
            painter.line_segment(
                [a, b],
                egui::Stroke::new(
                    tokens.divider_width,
                    color_alpha(tokens.plate_rim, tokens.divider_opacity),
                ),
            );
        }
        let named = !layout.compact;
        if let Some(plate) = layout.player_plate {
            self.compact_player(ui, plate.shrink(tokens.nav_padding));
        }
        let popover = super::popover_place(rect, &layout, &tokens);
        self.nav_camera(ui, &layout, named, scene);
        self.nav_look(ui, &layout, named, popover);
        self.nav_analysis(ui, &layout, named, popover);
        self.nav_show(ui, &layout, named);
        self.nav_section(ui, &layout, named, scene);
        // An open popover has the corner: the cube moves down under it,
        // or out until it closes.
        let panel = [&self.look_hits, &self.analysis_hits]
            .into_iter()
            .flat_map(|hits| hits.iter())
            .find(|(name, _)| name == "panel")
            .map(|(_, r)| *r);
        if let (Some(cube), Some(panel)) = (layout.cube, panel) {
            layout.cube = super::cube_out_of_the_way(cube, panel, rect, &layout, &tokens);
            self.controls_layout = Some(layout);
        }
        match layout.cube {
            Some(place) => {
                self.view_cube_rect = Some(place);
                self.view_cube(ui, place, scene);
            }
            None => {
                self.view_cube_rect = None;
                self.cube_drag_from = None;
            }
        }
        if let Some(triad) = layout.triad {
            let painter = ui.painter_at(triad.expand(tokens.nav_triad_font));
            self.paint_triad(&painter, triad);
        }
    }

    /// Fit, Iso and Persp | Ortho.
    fn nav_camera(
        &mut self,
        ui: &mut egui::Ui,
        layout: &ControlsLayout,
        named: bool,
        scene: Option<&Scene>,
    ) {
        let tokens = self.tokens;
        let (Some(fit_rect), Some(iso_rect), Some(projection_rect)) =
            (layout.fit, layout.iso, layout.projection)
        else {
            self.camera_hits.clear();
            return;
        };
        let fit = button(ui, fit_rect, "fit", Icon::Fit, "Fit", named, &tokens)
            .on_hover_text("Frame the whole model (F)")
            .clicked();
        let iso = button(ui, iso_rect, "iso", Icon::Iso, "Iso", named, &tokens)
            .on_hover_text("Look from the front-right corner, framed")
            .clicked();
        let active = match self.state.camera.projection {
            ProjectionKind::Perspective => 0,
            ProjectionKind::Orthographic => 1,
        };
        let items: [(Option<Icon>, &str); 2] = [(None, "Persp"), (None, "Ortho")];
        let (cells, clicked) = choice(
            ui,
            projection_rect,
            "projection",
            &items,
            Some(active),
            named,
            &tokens,
        );
        let mut hits = vec![("fit", fit_rect), ("iso", iso_rect)];
        for (name, cell) in ["perspective", "orthographic"].into_iter().zip(cells) {
            hits.push((name, cell));
        }
        self.camera_hits = hits;
        let projection = match clicked {
            Some(0) => Some(ProjectionKind::Perspective),
            Some(_) => Some(ProjectionKind::Orthographic),
            None => None,
        };
        self.camera_clicks(ui, fit, iso, projection, scene);
    }

    /// The Studio look's split button, on the camera plate: the chip
    /// switches the look, the caret opens and closes its options.
    fn nav_look(
        &mut self,
        ui: &mut egui::Ui,
        layout: &ControlsLayout,
        named: bool,
        popover: Option<Rect>,
    ) {
        let Some(studio) = layout.studio else {
            self.look_caret = None;
            self.look_hits.clear();
            return;
        };
        let (main, caret_rect) = super::split_chip(studio, &self.tokens);
        let lit = self.state.look.is_studio();
        // While an analysis is on the view is drawn plainly: the look's
        // options wait until it is off.
        let blocked = self.state.analysis.is_on();
        let open = lit && self.state.panels.look_options && !blocked;
        if toggle(
            ui,
            main,
            Chip {
                id: "studio",
                icon: Icon::Studio,
                label: "Studio",
                named,
            },
            lit,
            &self.tokens,
        )
        .on_hover_text(
            "Light the model as in a photo studio: reflections, soft shadows and a ground",
        )
        .clicked()
        {
            self.state.look.mode = if lit { "plain" } else { "studio" }.to_string();
            // Switched on, its options open.
            self.state.panels.look_options = !lit;
            ui.ctx().request_repaint();
        }
        self.look_caret = Some(caret_rect);
        let state = CaretState {
            on: lit,
            open,
            enabled: !blocked,
        };
        if caret(ui, caret_rect, "studio", state, &self.tokens)
            .on_hover_text(if blocked {
                "The studio look's options: switch the analysis off first (the analysis is drawn plainly)"
            } else if open {
                "Close the studio look's options (Esc)"
            } else {
                "The studio look's options"
            })
            .clicked()
            && !blocked
        {
            if open {
                self.state.panels.look_options = false;
            } else {
                // The options are the look's: asking for them switches it on.
                self.state.look.mode = "studio".to_string();
                self.state.panels.look_options = true;
            }
            ui.ctx().request_repaint();
        }
        self.look_panel(ui, popover, studio);
    }

    /// The Analysis split button beside Studio: the chip switches an
    /// analysis on and off, the caret opens and closes its options and
    /// the legend in the popover.
    fn nav_analysis(
        &mut self,
        ui: &mut egui::Ui,
        layout: &ControlsLayout,
        named: bool,
        popover: Option<Rect>,
    ) {
        let Some(place) = layout.analysis else {
            self.analysis_caret = None;
            self.analysis_hits.clear();
            return;
        };
        let (main, caret_rect) = super::split_chip(place, &self.tokens);
        let on = self.state.analysis.is_on();
        if toggle(
            ui,
            main,
            Chip {
                id: "analysis",
                icon: Icon::Analysis,
                label: "Analysis",
                named,
            },
            on,
            &self.tokens,
        )
        .on_hover_text(
            "Analyse the surfaces: zebra stripes, highlight lines, a curvature map or draft, and curvature combs",
        )
        .clicked()
        {
            if on {
                self.analysis_off();
            } else {
                self.analysis_on();
            }
            ui.ctx().request_repaint();
        }
        let on = self.state.analysis.is_on();
        let open = on && self.state.panels.analysis_options;
        self.analysis_caret = Some(caret_rect);
        let state = CaretState {
            on,
            open,
            enabled: true,
        };
        if caret(ui, caret_rect, "analysis", state, &self.tokens)
            .on_hover_text(if open {
                "Close the analysis' options (Esc)"
            } else {
                "The analysis' options"
            })
            .clicked()
        {
            if open {
                self.state.panels.analysis_options = false;
            } else {
                if !on {
                    self.analysis_on();
                }
                self.state.panels.analysis_options = true;
            }
            ui.ctx().request_repaint();
        }
        let (true, Some(popover)) = (
            self.state.analysis.is_on() && self.state.panels.analysis_options,
            popover,
        ) else {
            self.analysis_hits.clear();
            return;
        };
        let before = self.state.analysis.clone();
        let tokens = self.tokens;
        let legend = self.analysis_view.clone();
        let problem = self.analysis_problem.clone();
        let panel = self.popover(ui, "analysis-options", popover, place, |shell, ui| {
            analysis_options(ui, &mut shell.state.analysis);
            if let Some(view) = legend.as_deref() {
                analysis_legend(ui, view, &tokens);
            }
            if let Some(problem) = &problem {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(problem).color(color(tokens.draft_shallow)),
                    )
                    .wrap(),
                );
            }
        });
        self.analysis_hits = vec![("panel".to_string(), panel)];
        if self.state.analysis != before {
            ui.ctx().request_repaint();
        }
    }

    /// Switches the last analysis on, its options open.
    fn analysis_on(&mut self) {
        self.state.analysis.mode.clone_from(&self.analysis_last);
        self.state.panels.analysis_options = true;
    }

    /// Switches the analysis off, remembering which it was.
    fn analysis_off(&mut self) {
        if self.state.analysis.mode != "none" {
            self.analysis_last.clone_from(&self.state.analysis.mode);
        }
        self.state.analysis.mode = "none".to_string();
        self.state.analysis.combs = None;
    }

    /// Sketches, Datums, Edges and See-through.
    fn nav_show(&mut self, ui: &mut egui::Ui, layout: &ControlsLayout, named: bool) {
        let tokens = self.tokens;
        let (Some(sketches), Some(datums), Some(edges), Some(field)) = (
            layout.sketches,
            layout.datums,
            layout.edges,
            layout.see_through,
        ) else {
            self.show_hits.clear();
            self.transparency_rect = None;
            return;
        };
        let mut show = self.state.show;
        // A tight bar has these three by their icons.
        let named_in = |rect: Rect| named && rect.width() > tokens.nav_height + 0.5;
        if toggle(
            ui,
            sketches,
            Chip {
                id: "sketches",
                icon: Icon::Sketches,
                label: "Sketches",
                named: named_in(sketches),
            },
            show.sketches,
            &tokens,
        )
        .on_hover_text("Show the sketches")
        .clicked()
        {
            show.sketches = !show.sketches;
        }
        if toggle(
            ui,
            datums,
            Chip {
                id: "datums",
                icon: Icon::Datums,
                label: "Datums",
                named: named_in(datums),
            },
            show.datums,
            &tokens,
        )
        .on_hover_text("Show the datum planes, axes and points")
        .clicked()
        {
            show.datums = !show.datums;
        }
        if toggle(
            ui,
            edges,
            Chip {
                id: "edges",
                icon: Icon::Edges,
                label: "Edges",
                named: named_in(edges),
            },
            show.edges,
            &tokens,
        )
        .on_hover_text("Show the bodies' edges")
        .clicked()
        {
            show.edges = !show.edges;
        }
        self.show_hits = vec![("sketches", sketches), ("datums", datums), ("edges", edges)];
        self.see_through_field(ui, field);
        self.set_show(ui, show);
    }

    /// The Section toggle and, while the section is on, X | Y | Z, Offset
    /// and Flip.
    fn nav_section(
        &mut self,
        ui: &mut egui::Ui,
        layout: &ControlsLayout,
        named: bool,
        scene: Option<&Scene>,
    ) {
        let tokens = self.tokens;
        let Some(toggle_rect) = layout.section_toggle else {
            self.section_hits.clear();
            return;
        };
        let mut section = self.state.section.clone();
        let mut hits: Vec<(&'static str, Rect)> = vec![("toggle", toggle_rect)];
        if toggle(
            ui,
            toggle_rect,
            Chip {
                id: "section",
                icon: Icon::Section,
                label: "Section",
                named,
            },
            section.on,
            &tokens,
        )
        .on_hover_text("Cut the view open with a plane")
        .clicked()
        {
            toggle_section(&mut section, scene);
        }
        let on = self.state.section.on;
        if let (true, Some(axes), Some(offset), Some(side)) =
            (on, layout.axes, layout.offset, layout.side)
        {
            let items = [(None, "X"), (None, "Y"), (None, "Z")];
            let (cells, clicked) = choice(
                ui,
                axes,
                "axes",
                &items,
                chosen_axis(&section),
                true,
                &tokens,
            );
            for ((name, _), cell) in SECTION_PLANES.iter().zip(&cells) {
                hits.push((name, *cell));
            }
            if let Some(axis) = clicked {
                choose_axis(&mut section, axis, scene);
            }
            self.offset_field(ui, offset, &mut section, on);
            hits.push(("offset", offset));
            let flip_named = named || side.width() > tokens.nav_height + 0.5;
            if toggle(
                ui,
                side,
                Chip {
                    id: "flip",
                    icon: Icon::Flip,
                    label: "Flip side",
                    named: flip_named,
                },
                section.flip,
                &tokens,
            )
            .on_hover_text("Keep the other half of the cut")
            .clicked()
            {
                section.flip = !section.flip;
            }
            hits.push(("flip", side));
        } else {
            // No Offset field: nothing is typed into it, dragged or
            // focused.
            self.forget_field("offset");
        }
        self.section_hits = hits;
        self.commit_section(ui, section);
        if !self.state.section.on {
            self.forget_field("offset");
        }
    }
}

/// The options of the analysis that is on, as number fields.
fn analysis_options(ui: &mut egui::Ui, a: &mut crate::state::AnalysisState) {
    egui::Grid::new("analysis-grid")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Analysis");
            egui::ComboBox::from_id_salt("analysis-mode")
                .selected_text(a.mode.clone())
                .show_ui(ui, |ui| {
                    for mode in ["zebra", "highlight", "curvature", "draft", "none"] {
                        ui.selectable_value(&mut a.mode, mode.to_string(), mode);
                    }
                });
            ui.end_row();
            match a.mode.as_str() {
                "zebra" | "highlight" => {
                    ui.label("Stripes");
                    ui.add(
                        egui::DragValue::new(&mut a.stripes)
                            .range(1.0..=512.0)
                            .speed(0.5),
                    );
                    ui.end_row();
                    ui.label("Direction");
                    ui.add(
                        egui::DragValue::new(&mut a.direction)
                            .range(0.0..=360.0)
                            .suffix("°"),
                    );
                    ui.end_row();
                    ui.label("Width");
                    ui.add(
                        egui::DragValue::new(&mut a.width)
                            .range(0.0..=1.0)
                            .speed(0.01),
                    );
                    ui.end_row();
                    ui.label("Offset");
                    ui.add(
                        egui::DragValue::new(&mut a.offset)
                            .range(0.0..=1.0)
                            .speed(0.01),
                    );
                    ui.end_row();
                }
                "curvature" => {
                    ui.label("Curvature");
                    egui::ComboBox::from_id_salt("analysis-curvature")
                        .selected_text(a.curvature.clone())
                        .show_ui(ui, |ui| {
                            for kind in ["gaussian", "mean", "max"] {
                                ui.selectable_value(&mut a.curvature, kind.to_string(), kind);
                            }
                        });
                    ui.end_row();
                    let mut fixed = a.range.is_some();
                    ui.label("Range");
                    ui.checkbox(&mut fixed, "fixed");
                    ui.end_row();
                    if fixed {
                        let [mut lo, mut hi] = a.range.unwrap_or([-1.0, 1.0]);
                        ui.label("Low");
                        ui.add(egui::DragValue::new(&mut lo).speed(0.001));
                        ui.end_row();
                        ui.label("High");
                        ui.add(egui::DragValue::new(&mut hi).speed(0.001));
                        ui.end_row();
                        a.range = (lo < hi).then_some([lo, hi]).or(a.range);
                        if a.range.is_none() {
                            a.range = Some([-1.0, 1.0]);
                        }
                    } else {
                        a.range = None;
                    }
                }
                "draft" => {
                    ui.label("Pull");
                    ui.horizontal(|ui| {
                        for c in &mut a.pull {
                            ui.add(egui::DragValue::new(c).speed(0.01));
                        }
                    });
                    ui.end_row();
                    ui.label("Least draft");
                    ui.add(
                        egui::DragValue::new(&mut a.min_draft)
                            .range(0.0..=90.0)
                            .speed(0.1)
                            .suffix("°"),
                    );
                    ui.end_row();
                }
                _ => {}
            }
            let mut combs = a.combs.is_some();
            ui.label("Combs");
            ui.checkbox(&mut combs, "on edges");
            ui.end_row();
            match (combs, a.combs.is_some()) {
                (true, false) => a.combs = Some(serde_json::json!({"edges": true})),
                (false, true) => a.combs = None,
                _ => {}
            }
            if let Some(serde_json::Value::Object(c)) = &mut a.combs {
                let mut density = c
                    .get("density")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(40.0);
                ui.label("Density");
                if ui
                    .add(egui::DragValue::new(&mut density).range(2.0..=400.0))
                    .changed()
                {
                    c.insert("density".into(), serde_json::json!(density.round()));
                }
                ui.end_row();
                let mut scale = c.get("scale").and_then(serde_json::Value::as_f64);
                let mut fixed = scale.is_some();
                ui.label("Scale");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut fixed, "");
                    if fixed {
                        let mut v = scale.unwrap_or(1.0);
                        ui.add(egui::DragValue::new(&mut v).range(1e-9..=1e9).speed(0.01));
                        scale = Some(v);
                    } else {
                        scale = None;
                    }
                });
                c.insert("scale".into(), serde_json::json!(scale));
                ui.end_row();
            }
        });
}

/// The legend of the analysis shown: its title, and the ramp or the
/// classes with their colours.
fn analysis_legend(ui: &mut egui::Ui, view: &crate::viewport::ViewAnalysis, tokens: &Tokens) {
    let Some((title, rows)) = crate::viewport::legend_rows(view) else {
        return;
    };
    ui.separator();
    ui.label(egui::RichText::new(title).color(color(tokens.plate_text)));
    for (swatches, words) in rows {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(28.0, 12.0), egui::Sense::hover());
            let n = swatches.len().max(1) as f32;
            for (k, rgb) in swatches.iter().enumerate() {
                let w = rect.width() / n;
                let part = Rect::from_min_size(
                    pos2(rect.left() + w * k as f32, rect.top()),
                    vec2(w, rect.height()),
                );
                ui.painter().rect_filled(part, 0.0, color(*rgb));
            }
            ui.label(words);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_math::Portable;

    /// The viewport of a window of 1920 × 1080 and one of 1280 × 800 (the
    /// tree on the left, the tabs over it and the status bar under it).
    fn viewports() -> [Rect; 2] {
        [
            Rect::from_min_size(pos2(270.0, 112.0), vec2(1650.0, 940.0)),
            Rect::from_min_size(pos2(270.0, 112.0), vec2(1010.0, 660.0)),
        ]
    }

    /// At both window sizes, with the section off and on, every control
    /// is there with its name (in the smaller window the display bar's
    /// three toggles may show their icons only), the cube and the triad too, inside the
    /// viewport less its margin and clear of each other; the cluster is
    /// in the top right corner with the cube flush right under the camera
    /// plate, the bar at the bottom, the triad bottom left, at the theme's
    /// sizes; the Section chip stays where it is when the options open.
    #[test]
    fn every_control_is_there_and_where_it_belongs() {
        let tokens = Tokens::default();
        let widths = ControlWidths::fallback(&tokens);
        for viewport in viewports() {
            let area = viewport.shrink(tokens.nav_margin);
            let off = controls_layout(viewport, &tokens, &widths, false);
            for on in [false, true] {
                let layout = controls_layout(viewport, &tokens, &widths, on);
                let mut names: Vec<&str> = layout.controls().into_iter().map(|(n, _)| n).collect();
                names.extend(layout.parts().into_iter().map(|(n, _)| n));
                let mut expected = vec![
                    "sketches",
                    "datums",
                    "edges",
                    "see-through",
                    "section",
                    "fit",
                    "iso",
                    "projection",
                    "studio",
                    "analysis",
                ];
                if on {
                    expected.extend(["axes", "offset", "side"]);
                }
                expected.extend(["bar", "camera", "cube", "triad"]);
                for name in &expected {
                    assert!(
                        names.contains(name),
                        "{on} {viewport:?}: no {name} in {names:?}"
                    );
                }
                assert_eq!(layout.axes.is_some(), on);
                assert!(!layout.compact, "{on} {viewport:?}");
                // With the Edges toggle (2026-10-30) the bar of the
                // smaller window is too narrow for the display toggles'
                // names at seven points a character: they show their
                // icons there, every other chip its name; in the larger
                // window every chip has its name.
                let shows = [layout.sketches, layout.datums, layout.edges];
                let named = |r: Option<Rect>| r.is_some_and(|r| r.width() > tokens.nav_height);
                if viewport.width() > 1200.0 {
                    assert!(shows.iter().all(|r| named(*r)), "{on} {layout:?}");
                } else {
                    assert!(
                        shows.iter().all(|r| named(*r)) || shows.iter().all(|r| !named(*r)),
                        "the three alike: {layout:?}"
                    );
                }
                for r in [layout.section_toggle, layout.fit, layout.iso, layout.studio] {
                    assert!(named(r), "{on} {viewport:?}: {r:?}");
                }
                assert!(complete(&layout, area, on), "{on} {layout:?}");
                for (name, rect) in layout.controls() {
                    assert_eq!(rect.height(), tokens.nav_height, "{name}");
                }
                let (cube, camera) = (layout.cube.unwrap(), layout.camera_plate.unwrap());
                assert_eq!(cube.width(), tokens.nav_cube_size);
                // The camera plate above the cube (2026-10-21), both flush
                // right (2026-10-30).
                assert_eq!(camera.top(), area.top());
                assert!((camera.right() - area.right()).abs() < 0.01);
                assert_eq!(cube.top(), camera.bottom() + tokens.nav_gap);
                assert!((cube.right() - area.right()).abs() < 0.01);
                assert_eq!(layout.bar.unwrap().bottom(), area.bottom());
                let triad = layout.triad.unwrap();
                assert_eq!(triad.left_bottom(), area.left_bottom());
                assert_eq!(triad.width(), triad_side(&tokens));
                assert_eq!(layout.section_toggle, off.section_toggle);
            }
            // With the section off the bar is centred in the viewport
            // where there is room for its options to open.
            let bar = off.bar.unwrap();
            if viewport.width() > 1200.0 {
                assert!((bar.center().x - area.center().x).abs() < 0.01, "{bar:?}");
            }
        }
    }

    /// At every size, from a phone-sized viewport to a large one, with the
    /// section off and on and with wide texts: what is shown is inside
    /// the viewport less its margin, no plate overlaps another, the cube
    /// or the triad, every control is on a plate and clear of the others,
    /// and numbers that are not numbers give nothing that is not either.
    #[test]
    fn the_controls_never_overlap_and_stay_inside() {
        let tokens = Tokens::default();
        let plain = ControlWidths::fallback(&tokens);
        let wide = ControlWidths {
            sketches: 160.0,
            see_through: 220.0,
            offset: 240.0,
            ..plain
        };
        let mut full = 0;
        for widths in [plain, wide] {
            for on in [false, true] {
                for w in (20..1800).step_by(37) {
                    for h in (20..1100).step_by(41) {
                        let viewport =
                            Rect::from_min_size(pos2(3.0, 9.0), vec2(w as f32, h as f32));
                        let layout = controls_layout(viewport, &tokens, &widths, on);
                        let area = viewport.shrink(tokens.nav_margin);
                        let parts = layout.parts();
                        for (k, (name, rect)) in parts.iter().enumerate() {
                            assert!(
                                area.expand(0.5).contains_rect(*rect),
                                "{name} {rect:?} in {area:?}"
                            );
                            for (other, second) in parts.iter().skip(k + 1) {
                                assert!(
                                    !rect.shrink(0.01).intersects(*second),
                                    "{name} and {other} in {viewport:?}"
                                );
                            }
                        }
                        let plates: Vec<Rect> =
                            [layout.bar, layout.section_plate, layout.camera_plate]
                                .into_iter()
                                .flatten()
                                .collect();
                        let controls = layout.controls();
                        for (k, (name, rect)) in controls.iter().enumerate() {
                            assert!(
                                plates.iter().any(|p| p.expand(0.01).contains_rect(*rect)),
                                "{name} on no plate in {viewport:?}"
                            );
                            for (other, second) in controls.iter().skip(k + 1) {
                                assert!(
                                    !rect.shrink(0.01).intersects(*second),
                                    "{name} and {other} in {viewport:?}"
                                );
                            }
                        }
                        if complete(&layout, area, on) {
                            full += 1;
                        }
                    }
                }
            }
        }
        assert!(full > 500, "{full}");
        let nowhere = Rect::from_min_size(pos2(f32::NAN, 0.0), vec2(10.0, 10.0));
        assert!(
            controls_layout(nowhere, &tokens, &plain, true)
                .parts()
                .is_empty()
        );
        let broken = ControlWidths {
            fit: f32::NAN,
            ..plain
        };
        let layout = controls_layout(viewports()[0], &tokens, &broken, true);
        assert!(layout.fit.is_some());
        for (_, rect) in layout.parts().into_iter().chain(layout.controls()) {
            assert!(rect.min.x.is_finite() && rect.max.y.is_finite());
        }
    }

    /// The problem text never runs under the display bar: beside it only
    /// when it fits there, otherwise above the bar.
    #[test]
    fn the_problem_text_keeps_clear_of_the_bar() {
        let tokens = Tokens::default();
        let widths = ControlWidths::fallback(&tokens);
        for viewport in viewports() {
            for on in [false, true] {
                let layout = controls_layout(viewport, &tokens, &widths, on);
                let bar = layout.bar.unwrap();
                for natural in [80.0, 300.0, 900.0, 3000.0] {
                    let [left, width, bottom] =
                        problem_place(viewport, &layout, &tokens, natural).unwrap();
                    let beside = left + width <= bar.left() + 0.01;
                    let above = bottom <= bar.top() + 0.01;
                    assert!(beside || above, "{viewport:?} {on} {natural}");
                    if beside {
                        assert!(natural <= width + 0.01);
                    }
                    assert!(left >= layout.triad.unwrap().right());
                    assert!(left + width <= viewport.right());
                }
            }
        }
    }

    /// The triad's arms are about 45 to 50 points long in the iso view,
    /// in balance with the cube.
    #[test]
    fn the_triad_is_in_balance_with_the_cube() {
        let tokens = Tokens::default();
        let mut camera = crate::state::CameraState::default();
        camera.set_view("iso").unwrap();
        let arms = crate::view_cube::triad(&camera).unwrap();
        // As drawn: the reach in the place the layout gives the triad.
        let widths = ControlWidths::fallback(&tokens);
        let layout = controls_layout(viewports()[1], &tokens, &widths, false);
        let reach = super::super::controls::triad_reach(&tokens, layout.triad.unwrap());
        assert_eq!(reach, tokens.nav_triad_arm);
        let longest = arms
            .iter()
            .map(|a| a.tip[0].phypot(a.tip[1]) as f32 * reach)
            .fold(0.0, f32::max);
        assert!((45.0..=50.0).contains(&longest), "{longest}");
        assert!(triad_side(&tokens) <= 1.5 * tokens.nav_cube_size);
    }
}
