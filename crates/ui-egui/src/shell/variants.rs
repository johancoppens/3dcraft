//! Three mockup arrangements of the controls over the viewport, to choose
//! from (`state::UiState::controls_layout`, `ui.set {"controlsLayout":
//! "a" | "b" | "c"}`); `current` is today's ([`super::controls`]).
//!
//! They share one size system from the tokens (`nav_*`): every control is
//! `nav_height` high on a glass plate `nav_padding` larger, every plate
//! `nav_margin` from the viewport's edge, groups `nav_gap` apart with a
//! divider between them. They share one visual grammar too: a *toggle*
//! (Sketches, Datums, Section, Flip) is a chip with an icon that is tinted
//! with the accent while it is on; an *action* (Fit, Iso) is a plain
//! button that lights up under the pointer; a *choice* (Persp | Ortho,
//! X | Y | Z) is a segmented track whose chosen segment is filled. The
//! view cube is `nav_cube_size` and the triad `nav_triad_size`, always at
//! the bottom left. The section's options (axis, offset, flip) exist only
//! while the section is on.
//!
//! - **A**, *corner cluster*: the cube in the top right corner with the
//!   camera controls on a plate under it; a display bar at the bottom
//!   centre (Sketches, Datums, See-through | Section), whose section
//!   options open inline to the right of the Section chip, which stays
//!   where it is.
//! - **B**, *one stack*: everything at the bottom right as one column:
//!   the cube centred over one card as wide as its widest row, with the
//!   camera row, the display rows and the Section row at the bottom; the
//!   section's options appear above the Section row and push the rest up.
//! - **C**, *toolbar*: one slim toolbar centred along the bottom edge,
//!   every control with its icon and its name, in four groups (view,
//!   projection, display, section); the cube in the top right corner;
//!   Section opens a small floating panel over its button.
//!
//! In a viewport too narrow for the names the chips show their icons
//! only (with the name as a tooltip); what still does not fit is left
//! out, so nothing is ever drawn outside the viewport or over another
//! control.

use super::Shell;
use super::controls::{
    ControlsLayout, SECTION_PLANES, choose_axis, chosen_axis, plate_rounded, toggle_section,
};
use super::icons::{self, Icon};
use crate::state::{ControlsVariant, ProjectionKind};
use crate::theme::{Tokens, color, color_alpha};
use craft3d_engine::Scene;
use egui::{Pos2, Rect, pos2, vec2};

/// The widths the controls of the arrangements need for their texts, in
/// points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavWidths {
    /// Chips and buttons with an icon and a name.
    pub fit: f32,
    pub iso: f32,
    pub sketches: f32,
    pub datums: f32,
    pub section: f32,
    pub flip: f32,
    /// One segment of Persp | Ortho, by name only and with an icon.
    pub projection: f32,
    pub projection_icon: f32,
    /// One segment of X | Y | Z.
    pub axis: f32,
    /// The number fields.
    pub see_through: f32,
    pub offset: f32,
    /// The title of the section's floating panel.
    pub title: f32,
}

/// The title of the section's floating panel (arrangement C).
pub const SECTION_TITLE: &str = "SECTION PLANE";

impl NavWidths {
    /// The widths as the texts need them in the controls' font.
    pub fn measure(painter: &egui::Painter, tokens: &Tokens) -> Self {
        let font = egui::FontId::proportional(tokens.control_font);
        let title = egui::FontId::proportional(tokens.nav_title_font);
        let text = |s: &str| {
            let font = if s == SECTION_TITLE {
                title.clone()
            } else {
                font.clone()
            };
            painter
                .layout_no_wrap(s.to_string(), font, egui::Color32::WHITE)
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
            section: with_icon("Section"),
            flip: with_icon("Flip side"),
            projection: widest(&["Persp", "Ortho"], &plain),
            projection_icon: widest(&["Persp", "Ortho"], &with_icon),
            axis: widest(&["X", "Y", "Z"], &plain),
            see_through: field("See-through", "90 %"),
            offset: field("Offset", "-000.00 mm"),
            title: text(SECTION_TITLE),
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
            self.section,
            self.flip,
            self.projection,
            self.projection_icon,
            self.axis,
            self.see_through,
            self.offset,
            self.title,
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
            section: icon + chars(7.0),
            flip: icon + chars(9.0),
            projection: 2.0 * tokens.nav_inset + chars(5.0) + 4.0,
            projection_icon: icon + chars(5.0),
            axis: tokens.nav_height,
            see_through: tokens.field_width,
            offset: tokens.field_width,
            title: chars(13.0),
        }
    }
}

/// Where the controls of arrangement `variant` go in the viewport `rect`,
/// with the section on or off. Today's arrangement is not laid out here
/// (it has [`super::controls_layout`]): for it the layout is empty.
pub fn variant_layout(
    variant: ControlsVariant,
    rect: Rect,
    tokens: &Tokens,
    widths: &NavWidths,
    section_on: bool,
) -> ControlsLayout {
    let empty = ControlsLayout::default();
    if !(rect.is_finite() && rect.width() > 0.0 && rect.height() > 0.0) {
        return empty;
    }
    let widths = if widths.all_finite() {
        *widths
    } else {
        NavWidths::fallback(tokens)
    };
    let area = rect.shrink(tokens.nav_margin);
    if !(area.width() > 0.0 && area.height() > 0.0) {
        return empty;
    }
    let build = |compact: bool| {
        let n = Nav {
            tokens,
            w: widths,
            area,
            compact,
            on: section_on,
        };
        let mut layout = match variant {
            ControlsVariant::Current => return ControlsLayout::default(),
            ControlsVariant::A => n.a(),
            ControlsVariant::B => n.b(),
            ControlsVariant::C => n.c(),
        };
        layout.compact = compact;
        layout
    };
    let full = build(false);
    let mut layout = if complete(&full, area, section_on) {
        full
    } else {
        build(true)
    };
    prune(&mut layout, area);
    layout
}

/// The arrangements' common measures, for one viewport.
struct Nav<'a> {
    tokens: &'a Tokens,
    w: NavWidths,
    /// The viewport less the margin.
    area: Rect,
    /// Chips by their icons only.
    compact: bool,
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

    /// The triad's place, at the bottom left.
    fn triad(&self) -> Rect {
        let side = self.tokens.nav_triad_size;
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
        let inset = self.h() * 0.2;
        [pos2(mid, y + inset), pos2(mid, y + self.h() - inset)]
    }

    /// A horizontal divider in the middle of the gap above `y` on a plate
    /// from `left` to `right`.
    fn horizontal(&self, left: f32, right: f32, y: f32) -> [Pos2; 2] {
        let mid = y - self.tokens.nav_gap * 0.5;
        [pos2(left, mid), pos2(right, mid)]
    }

    /// The camera group (Fit, Iso, a divider, Persp | Ortho) from `x` on a
    /// row whose top is `y`, `projection` wide for the choice; returns
    /// where it ends.
    fn camera_row(
        &self,
        out: &mut ControlsLayout,
        x: f32,
        y: f32,
        fit: f32,
        iso: f32,
        projection: f32,
    ) -> f32 {
        let sp = self.tokens.nav_spacing;
        out.fit = Some(self.at(x, y, fit));
        let x = x + fit + sp;
        out.iso = Some(self.at(x, y, iso));
        let x = x + iso;
        out.dividers[0] = Some(self.vertical(x, y));
        let x = x + self.tokens.nav_gap;
        out.projection = Some(self.at(x, y, projection));
        x + projection
    }

    /// A: the cube top right over the camera plate; the display bar at the
    /// bottom centre with the section inline; the triad bottom left.
    fn a(&self) -> ControlsLayout {
        let t = self.tokens;
        let mut out = ControlsLayout::default();
        let (pad, sp, gap) = (self.pad(), t.nav_spacing, t.nav_gap);
        let area = self.area;
        // The cluster in the top right corner.
        let (fit, iso) = (self.chip(self.w.fit), self.chip(self.w.iso));
        let projection = 2.0 * self.w.projection;
        let camera_w = pad + fit + sp + iso + gap + projection + pad;
        let side = t.nav_cube_size;
        let middle = area.right() - camera_w.max(side) * 0.5;
        let cube = Rect::from_min_size(pos2(middle - side * 0.5, area.top()), vec2(side, side));
        let camera = Rect::from_min_size(
            pos2(middle - camera_w * 0.5, cube.bottom() + gap),
            vec2(camera_w, self.plate_h()),
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
        );
        // The display bar, centred on what it is with the section off.
        let triad = self.triad();
        out.triad = Some(triad);
        let (sketches, datums) = (self.chip(self.w.sketches), self.chip(self.w.datums));
        let section = self.chip(self.w.section);
        let see = self.w.see_through;
        let off_w = pad + sketches + sp + datums + sp + see + gap + section + pad;
        let flip = self.chip(self.w.flip);
        let axes = 3.0 * self.w.axis;
        let extra = if self.on {
            gap + axes + sp + self.w.offset + sp + flip
        } else {
            0.0
        };
        let bar_w = off_w + extra;
        let left = (area.center().x - off_w * 0.5)
            .max(triad.right() + gap)
            .min(area.right() - bar_w);
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
        out.see_through = Some(self.at(x, y, see));
        x += see;
        out.dividers[1] = Some(self.vertical(x, y));
        x += gap;
        out.section_toggle = Some(self.at(x, y, section));
        x += section;
        if self.on {
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

    /// B: one card at the bottom right, the cube centred over it; from
    /// the bottom up the Section row, its options when it is on, the
    /// display rows and the camera row.
    fn b(&self) -> ControlsLayout {
        let t = self.tokens;
        let mut out = ControlsLayout::default();
        let (pad, sp, gap, rg, h) = (
            self.pad(),
            t.nav_spacing,
            t.nav_gap,
            t.nav_row_gap,
            self.h(),
        );
        let area = self.area;
        let (fit, iso) = (self.chip(self.w.fit), self.chip(self.w.iso));
        let projection = 2.0 * self.w.projection;
        let (sketches, datums) = (self.chip(self.w.sketches), self.chip(self.w.datums));
        let flip = self.chip(self.w.flip);
        let axes = 3.0 * self.w.axis;
        let rows = [
            fit + sp + iso + gap + projection,
            sketches + sp + datums,
            self.w.see_through,
            self.chip(self.w.section),
            axes + sp + flip,
            self.w.offset,
        ];
        let inner = rows.iter().copied().fold(0.0, f32::max);
        let card_w = inner + 2.0 * pad;
        let left = area.right() - card_w;
        let x = left + pad;
        // From the bottom up.
        let mut y = area.bottom() - pad - h;
        out.section_toggle = Some(self.at(x, y, inner));
        if self.on {
            y -= rg + h;
            out.offset = Some(self.at(x, y, inner));
            y -= rg + h;
            let axes_w = inner - sp - flip;
            out.axes = Some(self.at(x, y, axes_w));
            out.side = Some(self.at(x + axes_w + sp, y, flip));
        }
        out.dividers[1] = Some(self.horizontal(left + pad, left + card_w - pad, y));
        y -= gap + h;
        out.see_through = Some(self.at(x, y, inner));
        y -= rg + h;
        let half = (inner - sp) * 0.5;
        out.sketches = Some(self.at(x, y, half));
        out.datums = Some(self.at(x + half + sp, y, half));
        out.dividers[2] = Some(self.horizontal(left + pad, left + card_w - pad, y));
        y -= gap + h;
        // The camera row: the buttons as they are, the choice the rest.
        self.camera_row(&mut out, x, y, fit, iso, inner - fit - sp - iso - gap);
        let card = Rect::from_min_max(pos2(left, y - pad), pos2(area.right(), area.bottom()));
        out.bar = Some(card);
        let side = t.nav_cube_size;
        out.cube = Some(Rect::from_min_size(
            pos2(card.center().x - side * 0.5, card.top() - gap - side),
            vec2(side, side),
        ));
        out.triad = Some(self.triad());
        out
    }

    /// C: one toolbar at the bottom centre in four groups, the cube top
    /// right, the triad bottom left, the section's floating panel over its
    /// button.
    fn c(&self) -> ControlsLayout {
        let t = self.tokens;
        let mut out = ControlsLayout::default();
        let (pad, sp, gap, rg, h) = (
            self.pad(),
            t.nav_spacing,
            t.nav_gap,
            t.nav_row_gap,
            self.h(),
        );
        let area = self.area;
        let (fit, iso) = (self.chip(self.w.fit), self.chip(self.w.iso));
        let projection = 2.0 * self.chip(self.w.projection_icon);
        let (sketches, datums) = (self.chip(self.w.sketches), self.chip(self.w.datums));
        let section = self.chip(self.w.section);
        let see = self.w.see_through;
        let bar_w = pad
            + fit
            + sp
            + iso
            + gap
            + projection
            + gap
            + sketches
            + sp
            + datums
            + sp
            + see
            + gap
            + section
            + pad;
        let triad = self.triad();
        out.triad = Some(triad);
        let left = (area.center().x - bar_w * 0.5)
            .max(triad.right() + gap)
            .min(area.right() - bar_w);
        let bar = Rect::from_min_size(
            pos2(left, area.bottom() - self.plate_h()),
            vec2(bar_w, self.plate_h()),
        );
        out.bar = Some(bar);
        let y = bar.top() + pad;
        let mut x = self.camera_row(&mut out, bar.left() + pad, y, fit, iso, projection);
        out.dividers[1] = Some(self.vertical(x, y));
        x += gap;
        out.sketches = Some(self.at(x, y, sketches));
        x += sketches + sp;
        out.datums = Some(self.at(x, y, datums));
        x += datums + sp;
        out.see_through = Some(self.at(x, y, see));
        x += see;
        out.dividers[2] = Some(self.vertical(x, y));
        x += gap;
        let toggle = self.at(x, y, section);
        out.section_toggle = Some(toggle);
        let side = t.nav_cube_size;
        out.cube = Some(Rect::from_min_size(
            pos2(area.right() - side, area.top()),
            vec2(side, side),
        ));
        if self.on {
            // The floating panel: a title, then X | Y | Z, Offset and Flip
            // as rows of one width, over the Section button.
            let ppad = 2.0 * pad;
            let title_h = t.nav_title_font + 2.0 * pad;
            let inner = (3.0 * self.w.axis)
                .max(self.w.offset)
                .max(self.w.flip)
                .max(self.w.title);
            let panel_w = inner + 2.0 * ppad;
            let panel_h = 2.0 * ppad + title_h + 3.0 * (rg + h);
            let right = (toggle.right() + pad).min(area.right());
            let left = (right - panel_w).max(area.left());
            let panel = Rect::from_min_size(
                pos2(left, bar.top() - gap - panel_h),
                vec2(panel_w, panel_h),
            );
            out.section_plate = Some(panel);
            let x = panel.left() + ppad;
            let mut y = panel.top() + ppad;
            out.section_title = Some(Rect::from_min_size(pos2(x, y), vec2(inner, title_h)));
            y += title_h + rg;
            out.axes = Some(self.at(x, y, inner));
            y += h + rg;
            out.offset = Some(self.at(x, y, inner));
            y += h + rg;
            out.side = Some(self.at(x, y, inner));
        }
        out
    }
}

/// True when every control is there (the section's options too when the
/// section is on), and every plate, the cube and the triad are inside
/// `area` and clear of each other.
fn complete(layout: &ControlsLayout, area: Rect, section_on: bool) -> bool {
    let controls = [
        layout.sketches,
        layout.datums,
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
/// kept before it (the bar first, then the camera plate, then the
/// section's), with its controls; the cube and then the triad when they
/// are outside or over something kept.
fn prune(layout: &mut ControlsLayout, area: Rect) {
    let inside = |r: Rect| area.expand(0.5).contains_rect(r);
    let mut kept: Vec<Rect> = Vec::new();
    for slot in [
        &mut layout.bar,
        &mut layout.camera_plate,
        &mut layout.section_plate,
    ] {
        if let Some(plate) = *slot {
            if inside(plate) && kept.iter().all(|k| !k.shrink(0.01).intersects(plate)) {
                kept.push(plate);
            } else {
                *slot = None;
            }
        }
    }
    let on_plate = |r: &Rect| kept.iter().any(|p| p.expand(0.01).contains_rect(*r));
    for slot in [
        &mut layout.sketches,
        &mut layout.datums,
        &mut layout.see_through,
        &mut layout.section_toggle,
        &mut layout.axes,
        &mut layout.offset,
        &mut layout.side,
        &mut layout.fit,
        &mut layout.iso,
        &mut layout.projection,
        &mut layout.section_title,
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
    }
    if response.hovered() {
        painter.rect_filled(
            rect,
            radius,
            color_alpha(tokens.glass, tokens.control_opacity),
        );
    }
    let ink = color(if on { tokens.knob } else { tokens.text_dim });
    content(painter, rect, icon, label, named, ink, tokens);
    if named {
        response
    } else {
        response.on_hover_text(label)
    }
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
        color(tokens.text),
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
    let inner = egui::CornerRadius::same(radius.saturating_sub(2));
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
            painter.rect_filled(cell.shrink(2.0), inner, color(tokens.accent));
        } else if response.hovered() {
            painter.rect_filled(
                cell.shrink(2.0),
                inner,
                color_alpha(tokens.glass, tokens.control_opacity),
            );
        }
        let ink = color(if on { tokens.knob } else { tokens.text });
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
    /// The controls over the viewport `rect` in arrangement `variant` (see
    /// the top of this module).
    pub(super) fn variant_controls(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        scene: Option<&Scene>,
        variant: ControlsVariant,
    ) {
        let tokens = self.tokens;
        let painter = ui.painter_at(rect);
        let widths = NavWidths::measure(&painter, &tokens);
        let layout = variant_layout(variant, rect, &tokens, &widths, self.state.section.on);
        self.controls_layout = Some(layout);
        for plate in [layout.bar, layout.camera_plate, layout.section_plate]
            .into_iter()
            .flatten()
        {
            plate_rounded(&painter, plate, &tokens, tokens.nav_radius);
            // A click between two controls is not a pick in the viewport.
            ui.interact(
                plate,
                ui.id()
                    .with(("nav-plate", plate.min.x.to_bits(), plate.min.y.to_bits())),
                egui::Sense::click_and_drag(),
            );
        }
        for [a, b] in layout.dividers.into_iter().flatten() {
            painter.line_segment(
                [a, b],
                egui::Stroke::new(1.0, color_alpha(tokens.plate_rim, tokens.divider_opacity)),
            );
        }
        let named = !layout.compact;
        self.nav_camera(ui, &layout, variant, named, scene);
        self.nav_show(ui, &layout, named);
        self.nav_section(ui, &layout, named, scene);
        match layout.cube {
            Some(place) => {
                self.view_cube_rect = Some(place);
                self.view_cube(ui, place, None, scene);
            }
            None => {
                self.view_cube_rect = None;
                self.cube_drag_from = None;
            }
        }
        if let Some(triad) = layout.triad {
            let painter = ui.painter_at(triad.expand(tokens.nav_triad_font));
            self.paint_triad(&painter, triad, tokens.nav_triad_font);
        }
    }

    /// Fit, Iso and Persp | Ortho.
    fn nav_camera(
        &mut self,
        ui: &mut egui::Ui,
        layout: &ControlsLayout,
        variant: ControlsVariant,
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
        // The toolbar names every control with its icon.
        let items: [(Option<Icon>, &str); 2] = if variant == ControlsVariant::C {
            [
                (Some(Icon::Perspective), "Persp"),
                (Some(Icon::Orthographic), "Ortho"),
            ]
        } else {
            [(None, "Persp"), (None, "Ortho")]
        };
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

    /// Sketches, Datums and See-through.
    fn nav_show(&mut self, ui: &mut egui::Ui, layout: &ControlsLayout, named: bool) {
        let tokens = self.tokens;
        let (Some(sketches), Some(datums), Some(field)) =
            (layout.sketches, layout.datums, layout.see_through)
        else {
            self.show_hits.clear();
            self.transparency_rect = None;
            return;
        };
        let mut show = self.state.show;
        if toggle(
            ui,
            sketches,
            Chip {
                id: "sketches",
                icon: Icon::Sketches,
                label: "Sketches",
                named,
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
                named,
            },
            show.datums,
            &tokens,
        )
        .on_hover_text("Show the datum planes, axes and points")
        .clicked()
        {
            show.datums = !show.datums;
        }
        self.show_hits = vec![("sketches", sketches), ("datums", datums)];
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
            if let Some(title) = layout.section_title {
                ui.painter().text(
                    title.left_center(),
                    egui::Align2::LEFT_CENTER,
                    SECTION_TITLE,
                    egui::FontId::proportional(tokens.nav_title_font),
                    color(tokens.text_dim),
                );
            }
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
        }
        self.section_hits = hits;
        self.commit_section(ui, section);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VARIANTS: [ControlsVariant; 3] =
        [ControlsVariant::A, ControlsVariant::B, ControlsVariant::C];

    /// The viewport of a window of 1920 × 1080 and one of 1280 × 800 (the
    /// tree on the left, the tabs over it and the status bar under it), and
    /// of the window's least size.
    fn viewports() -> [Rect; 3] {
        [
            Rect::from_min_size(pos2(270.0, 112.0), vec2(1650.0, 940.0)),
            Rect::from_min_size(pos2(270.0, 112.0), vec2(1010.0, 660.0)),
            Rect::from_min_size(pos2(270.0, 112.0), vec2(210.0, 180.0)),
        ]
    }

    fn names(layout: &ControlsLayout) -> Vec<&'static str> {
        let mut names: Vec<&str> = layout.controls().into_iter().map(|(n, _)| n).collect();
        names.extend(layout.parts().into_iter().map(|(n, _)| n));
        names
    }

    /// At both window sizes, with the section off and on, every control
    /// of every arrangement is there with its name, the cube and the
    /// triad too, inside the viewport less its margin and clear of each
    /// other; the triad is at the bottom left, the cube and the triad have
    /// the theme's sizes, and every control is the theme's height.
    #[test]
    fn every_arrangement_has_every_control_inside_the_viewport() {
        let tokens = Tokens::dark();
        let widths = NavWidths::fallback(&tokens);
        for variant in VARIANTS {
            for viewport in viewports().into_iter().take(2) {
                for on in [false, true] {
                    let layout = variant_layout(variant, viewport, &tokens, &widths, on);
                    let mut expected = vec![
                        "sketches",
                        "datums",
                        "see-through",
                        "section",
                        "fit",
                        "iso",
                        "projection",
                    ];
                    if on {
                        expected.extend(["axes", "offset", "side"]);
                    }
                    expected.extend(["cube", "triad"]);
                    let names = names(&layout);
                    for name in &expected {
                        assert!(
                            names.contains(name),
                            "{variant:?} {on} {viewport:?}: no {name} in {names:?}"
                        );
                    }
                    assert!(!layout.compact, "{variant:?} {on} {viewport:?}");
                    let area = viewport.shrink(tokens.nav_margin);
                    assert!(complete(&layout, area, on), "{variant:?} {on} {layout:?}");
                    let triad = layout.triad.unwrap();
                    assert_eq!(triad.left_bottom(), area.left_bottom());
                    assert_eq!(triad.width(), tokens.nav_triad_size);
                    assert_eq!(layout.cube.unwrap().width(), tokens.nav_cube_size);
                    for (name, rect) in layout.controls() {
                        assert_eq!(rect.height(), tokens.nav_height, "{name}");
                    }
                    // The options only while the section is on.
                    assert_eq!(layout.axes.is_some(), on);
                }
            }
        }
    }

    /// At every size, from a phone-sized viewport to a large one, with the
    /// section off and on: what is shown is inside the viewport less its
    /// margin, no plate overlaps another, the cube or the triad, every
    /// control is on a plate and clear of the others, and numbers that are
    /// not numbers give an empty layout.
    #[test]
    fn the_arrangements_never_overlap_and_stay_inside() {
        let tokens = Tokens::dark();
        let plain = NavWidths::fallback(&tokens);
        let wide = NavWidths {
            sketches: 160.0,
            see_through: 220.0,
            offset: 240.0,
            ..plain
        };
        let mut shown = 0;
        for variant in VARIANTS {
            for widths in [plain, wide] {
                for on in [false, true] {
                    for w in (20..1800).step_by(53) {
                        for h in (20..1100).step_by(47) {
                            let viewport =
                                Rect::from_min_size(pos2(3.0, 9.0), vec2(w as f32, h as f32));
                            let layout = variant_layout(variant, viewport, &tokens, &widths, on);
                            let area = viewport.shrink(tokens.nav_margin);
                            let parts = layout.parts();
                            for (k, (name, rect)) in parts.iter().enumerate() {
                                assert!(
                                    area.expand(0.5).contains_rect(*rect),
                                    "{variant:?} {name} {rect:?} in {area:?}"
                                );
                                for (other, second) in parts.iter().skip(k + 1) {
                                    assert!(
                                        !rect.shrink(0.01).intersects(*second),
                                        "{variant:?} {name} and {other} in {viewport:?}"
                                    );
                                }
                            }
                            let plates: Vec<Rect> =
                                [layout.bar, layout.camera_plate, layout.section_plate]
                                    .into_iter()
                                    .flatten()
                                    .collect();
                            let controls = layout.controls();
                            for (k, (name, rect)) in controls.iter().enumerate() {
                                assert!(
                                    plates.iter().any(|p| p.expand(0.01).contains_rect(*rect)),
                                    "{variant:?} {name} on no plate in {viewport:?}"
                                );
                                for (other, second) in controls.iter().skip(k + 1) {
                                    assert!(
                                        !rect.shrink(0.01).intersects(*second),
                                        "{variant:?} {name} and {other} in {viewport:?}"
                                    );
                                }
                            }
                            if controls.len() >= 7 {
                                shown += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(shown > 500, "{shown}");
        let nowhere = Rect::from_min_size(pos2(f32::NAN, 0.0), vec2(10.0, 10.0));
        for variant in VARIANTS {
            assert!(
                variant_layout(variant, nowhere, &tokens, &plain, true)
                    .parts()
                    .is_empty()
            );
            let broken = NavWidths {
                fit: f32::NAN,
                ..plain
            };
            let layout = variant_layout(variant, viewports()[0], &tokens, &broken, true);
            for (_, rect) in layout.parts().into_iter().chain(layout.controls()) {
                assert!(rect.min.x.is_finite() && rect.max.y.is_finite());
            }
        }
        // Today's arrangement is not laid out here.
        assert!(
            variant_layout(
                ControlsVariant::Current,
                viewports()[0],
                &tokens,
                &plain,
                true
            )
            .parts()
            .is_empty()
        );
    }

    /// The arrangements differ where the developer asked them to: A has
    /// the cube top right over the camera plate and the display bar at
    /// the bottom; B stacks everything at the bottom right, the cube
    /// centred over one card; C has one toolbar at the bottom with every
    /// control on it and the section's options on a panel over it.
    #[test]
    fn the_arrangements_put_things_where_they_say() {
        let tokens = Tokens::dark();
        let widths = NavWidths::fallback(&tokens);
        let viewport = viewports()[0];
        let area = viewport.shrink(tokens.nav_margin);
        let a = variant_layout(ControlsVariant::A, viewport, &tokens, &widths, true);
        let (cube, camera, bar) = (a.cube.unwrap(), a.camera_plate.unwrap(), a.bar.unwrap());
        assert_eq!(cube.top(), area.top());
        assert!(camera.right() <= area.right() + 0.01 && camera.top() > cube.bottom());
        assert!((camera.center().x - cube.center().x).abs() < 0.01);
        assert_eq!(bar.bottom(), area.bottom());
        // The Section chip is where it is with the section off.
        let off = variant_layout(ControlsVariant::A, viewport, &tokens, &widths, false);
        assert_eq!(off.section_toggle, a.section_toggle);
        let b = variant_layout(ControlsVariant::B, viewport, &tokens, &widths, true);
        let card = b.bar.unwrap();
        assert_eq!(card.right_bottom(), area.right_bottom());
        assert!((b.cube.unwrap().center().x - card.center().x).abs() < 0.01);
        for (name, rect) in b.controls() {
            assert!(card.contains_rect(rect), "{name}");
        }
        let b_off = variant_layout(ControlsVariant::B, viewport, &tokens, &widths, false);
        assert_eq!(b_off.section_toggle, b.section_toggle);
        assert!(
            b_off.bar.unwrap().top() > card.top(),
            "the options push the card up"
        );
        let c = variant_layout(ControlsVariant::C, viewport, &tokens, &widths, true);
        let (toolbar, panel) = (c.bar.unwrap(), c.section_plate.unwrap());
        assert_eq!(toolbar.bottom(), area.bottom());
        assert!(panel.bottom() < toolbar.top());
        for rect in [c.axes, c.offset, c.side, c.section_title] {
            assert!(panel.contains_rect(rect.unwrap()));
        }
        for rect in [
            c.fit,
            c.iso,
            c.projection,
            c.sketches,
            c.datums,
            c.see_through,
            c.section_toggle,
        ] {
            assert!(toolbar.contains_rect(rect.unwrap()));
        }
        assert_eq!(c.cube.unwrap().right_top(), area.right_top());
    }
}
