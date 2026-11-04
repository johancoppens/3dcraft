//! The component kit (docs/design/rendering.md › *The component kit*): the
//! controls of the UI overhaul (plan of 2026-11-02, boards c1 and c2), each
//! painted from [`Tokens`] alone, on two grounds:
//!
//! - [`Ground::Panel`]: the side panel and the dialogs, light or dark with
//!   the appearance (`theme::Palette`);
//! - [`Ground::Plate`]: popovers, menus and the bars over the viewport,
//!   dark in both appearances (one step lighter in the dark one).
//!
//! Sizes: rows `row_height` (22), controls `control_height` (24), text
//! `secondary_font` (13) everywhere; with touch sizes (`Tokens::touch`)
//! every target is at least `touch_target` (40). Each component is a method
//! of [`Kit`] that lays itself out in a [`egui::Ui`] and returns its
//! [`egui::Response`] (or a [`Choice`] for one with several targets), so a
//! screen is converted by swapping egui's widget for the kit's.
//!
//! - [`controls`]: buttons, icon buttons, chips, segmented controls, check
//!   boxes, switches, number and text fields, dropdowns;
//! - [`slider`]: the one slider of the app, in its four forms, and the
//!   stepper for small whole numbers;
//! - [`rows`]: section and fold headers, label/value rows, list rows, the
//!   tab bar, captions;
//! - [`dialog`]: the one layout of every dialog (title, body, footer);
//! - [`overlays`]: tooltips, popovers with a pointer, menus and context
//!   menus;
//! - [`status`]: the agent's status pill, banners, progress and spinners;
//! - [`glyphs`]: the line icons (features, documents, controls), painted;
//! - [`brand`]: the app icon and its document variants, where 3DCraft
//!   shows itself;
//! - [`gallery`]: every component and icon on one board, for the pictures
//!   the design is checked against and for the tests.

pub mod brand;
pub mod controls;
pub mod dialog;
// Pictures and tests only: not in the browser build.
#[cfg(not(target_arch = "wasm32"))]
pub mod gallery;
pub mod glyphs;
pub mod overlays;
pub mod rows;
pub mod slider;
pub mod status;

pub use brand::AppIcon;
pub use controls::{Button, ButtonKind, ChipKind, Segment};
pub use dialog::{Dialog, DialogShown};
pub use glyphs::Glyph;
pub use overlays::MenuItem;
pub use rows::{Depends, ListRow, RowHits, RowMark, Tab, ValueStyle};
pub use slider::{Slider, SliderForm, SliderShown, SliderState};
pub use status::{BannerKind, PillState};

use crate::theme::{Rgb, Tokens, blend, color};
use egui::{Color32, CornerRadius, FontFamily, FontId, Rect, Response, TextStyle, Ui, Vec2, vec2};
use std::sync::Arc;

/// What a component stands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ground {
    /// A panel or a dialog: light or dark with the appearance.
    Panel,
    /// A popover, a menu, a bar over the viewport: dark in both.
    Plate,
}

/// The colours of one ground, resolved from the tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skin {
    pub fill: Rgb,
    pub text: Rgb,
    pub text_dim: Rgb,
    /// What is off or out of reach (a disabled control).
    pub faint: Rgb,
    pub field: Rgb,
    pub field_stroke: Rgb,
    pub hover: Rgb,
    pub selection: Rgb,
    pub chip: Rgb,
    pub info_fill: Rgb,
    pub info_ink: Rgb,
    pub info_bar: Rgb,
    pub failed: Rgb,
    pub failed_fill: Rgb,
    pub attention: Rgb,
    pub attention_fill: Rgb,
    pub ok: Rgb,
    pub track: Rgb,
    pub knob: Rgb,
    pub knob_rim: Rgb,
    /// A focused field's ring.
    pub focus: Rgb,
    pub divider: Rgb,
    pub icon_sketch: Rgb,
    pub icon_datum: Rgb,
}

impl Skin {
    pub fn of(t: &Tokens, ground: Ground) -> Self {
        match ground {
            Ground::Panel => Self {
                fill: t.panel_fill,
                text: t.text,
                text_dim: t.text_dim,
                faint: t.inactive,
                field: t.field_fill,
                field_stroke: t.field_stroke,
                hover: t.row_hover_fill,
                selection: t.selection_fill,
                chip: t.chip_fill,
                info_fill: t.info_fill,
                info_ink: t.info_ink,
                info_bar: t.info_accent,
                failed: t.failed,
                failed_fill: t.failed_fill,
                attention: t.attention,
                attention_fill: t.attention_fill,
                ok: t.ok,
                track: t.track,
                knob: t.knob_fill,
                knob_rim: t.knob_rim,
                focus: t.info_ink,
                divider: t.divider,
                icon_sketch: t.icon_sketch,
                icon_datum: t.icon_datum,
            },
            Ground::Plate => {
                let fill = t.popover_fill;
                Self {
                    fill,
                    text: t.plate_text,
                    text_dim: t.plate_text_dim,
                    faint: blend(t.plate_text_dim, alpha(t.disabled_opacity * 1.6), fill),
                    field: t.popover_field,
                    field_stroke: t.plate_field_stroke,
                    hover: t.popover_hover,
                    selection: t.plate_selection_fill,
                    chip: t.popover_hover,
                    info_fill: t.plate_info_fill,
                    info_ink: t.info_code,
                    info_bar: t.plate_info_accent,
                    failed: t.chrome_failed,
                    failed_fill: t.plate_failed_fill,
                    attention: t.accent,
                    attention_fill: t.plate_attention_fill,
                    ok: t.plate_ok,
                    track: t.plate_field_stroke,
                    knob: t.plate_text,
                    knob_rim: t.plate_text,
                    focus: t.info_code,
                    divider: blend(t.plate_rim, t.divider_opacity, fill),
                    icon_sketch: t.info_code,
                    icon_datum: t.plate_datum,
                }
            }
        }
    }
}

/// An opacity 0 to 1 as a byte.
fn alpha(share: f32) -> u8 {
    if share.is_finite() {
        (share.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        255
    }
}

/// A component painter: the tokens, the ground and its colours.
#[derive(Clone, Copy, Debug)]
pub struct Kit {
    pub t: Tokens,
    pub ground: Ground,
    pub skin: Skin,
}

impl Kit {
    pub fn new(tokens: &Tokens, ground: Ground) -> Self {
        Self {
            t: *tokens,
            ground,
            skin: Skin::of(tokens, ground),
        }
    }

    /// The same kit on the panels.
    pub fn panel(tokens: &Tokens) -> Self {
        Self::new(tokens, Ground::Panel)
    }

    /// The same kit on a plate (popover, menu, bar over the viewport).
    pub fn plate(tokens: &Tokens) -> Self {
        Self::new(tokens, Ground::Plate)
    }

    /// Whether these are touch sizes.
    pub fn touch(&self) -> bool {
        self.t.hit_min > 0.0
    }

    /// The body text's font.
    pub fn font(&self) -> FontId {
        FontId::proportional(self.t.secondary_font)
    }

    /// The font of hints and table heads: one point smaller, as the plan
    /// says (12 for hints and table heads).
    pub fn small_font(&self) -> FontId {
        FontId::proportional((self.t.secondary_font - 1.0).max(10.0))
    }

    /// Code and persistent names.
    pub fn mono_font(&self) -> FontId {
        FontId::monospace((self.t.secondary_font - 1.0).max(10.0))
    }

    /// The medium weight at the body text's size, where the fonts in force
    /// have it (the headings' family, `crate::fonts::style_headings`).
    pub fn medium_font(&self, ui: &Ui) -> FontId {
        let family = ui
            .style()
            .text_styles
            .get(&TextStyle::Heading)
            .map_or(FontFamily::Proportional, |font| font.family.clone());
        FontId::new(self.t.secondary_font, family)
    }

    /// Dialog and sheet titles: `dialog_title_font` (17) in the semi-bold
    /// weight where the fonts in force have it, else the headings'.
    pub fn title_font(&self, ui: &Ui) -> FontId {
        let semibold = FontFamily::Name(Arc::from(crate::fonts::SEMIBOLD));
        let family = if ui
            .ctx()
            .fonts(|f| f.definitions().families.contains_key(&semibold))
        {
            semibold
        } else {
            self.medium_font(ui).family
        };
        FontId::new(self.t.dialog_title_font, family)
    }

    /// The height a target takes: what it looks, at least `hit_min`.
    pub fn hit(&self, visual: f32) -> f32 {
        visual.max(self.t.hit_min)
    }

    pub fn radius(&self) -> CornerRadius {
        CornerRadius::same(self.t.radius)
    }

    pub fn field_radius(&self) -> CornerRadius {
        // Fields are rounder on a panel (6) than on a plate (4), whose
        // controls are smaller beside its own rounding.
        match self.ground {
            Ground::Panel => CornerRadius::same(self.t.field_radius),
            Ground::Plate => CornerRadius::same(self.t.nav_segment_radius),
        }
    }

    pub fn c(&self, rgb: Rgb) -> Color32 {
        color(rgb)
    }

    /// A one-line galley of `text`.
    pub fn galley(
        &self,
        ui: &Ui,
        text: impl Into<String>,
        font: FontId,
        ink: Rgb,
    ) -> Arc<egui::Galley> {
        ui.painter().layout_no_wrap(text.into(), font, color(ink))
    }
}

/// The answer of a component with several targets (a segmented control,
/// a tab bar): the whole, `changed` when the choice moved, and each
/// target's place.
pub struct Choice {
    pub response: Response,
    pub items: Vec<Rect>,
}

/// A size that never panics egui: finite and not negative.
pub(crate) fn sane(size: Vec2) -> Vec2 {
    let f = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
    vec2(f(size.x), f(size.y))
}

/// `v` held between `lo` and `hi` without a panic: crossed bounds give
/// `lo`, and `None` when anything is not a number.
pub(crate) fn clamped(v: f32, lo: f32, hi: f32) -> Option<f32> {
    if v.is_nan() || lo.is_nan() || hi.is_nan() {
        return None;
    }
    Some(v.max(lo).min(hi.max(lo)))
}

/// `rect` of `size`, centred vertically in `outer` at its left.
pub(crate) fn centred_left(outer: Rect, size: Vec2) -> Rect {
    Rect::from_min_size(
        egui::pos2(outer.left(), outer.center().y - size.y * 0.5),
        size,
    )
}
