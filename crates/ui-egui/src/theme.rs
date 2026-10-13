//! Design tokens: every colour, radius and width the shell draws with
//! (AGENTS.md rule 9). Widgets take them from [`Tokens`]; nothing else in
//! this crate spells out a colour.
//!
//! Colours are plain sRGB bytes, so the offscreen picture (`ui.screenshot`,
//! drawn without egui) uses the very same values as the window.

/// An opaque sRGB colour.
pub type Rgb = [u8; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// Behind the model.
    pub viewport_background: Rgb,
    /// The base colour of a shaded body.
    pub body: Rgb,
    /// The base colour of a highlighted face: the one that was clicked,
    /// or the one an agent points at (`ui.highlight`). Shaded like the
    /// body, so the face keeps its shape.
    pub face_highlight: Rgb,
    /// Sketch geometry that is part of a profile.
    pub sketch: Rgb,
    /// Construction geometry of a sketch.
    pub sketch_construction: Rgb,
    /// The sketch of the selected feature.
    pub sketch_selected: Rgb,
    /// The world axes at the origin: X, Y, Z.
    pub axes: [Rgb; 3],
    /// The edges of a body.
    pub edge: Rgb,
    /// Opacity of sketch lines and points behind a body, 0 to 255.
    pub hidden_opacity: u8,
    pub panel_fill: Rgb,
    pub text: Rgb,
    pub text_dim: Rgb,
    /// Feature states in the tree.
    pub ok: Rgb,
    pub failed: Rgb,
    pub inactive: Rgb,
    /// What waits for the person to decide: the heading of the recovery
    /// bar.
    pub attention: Rgb,
    /// The row of the selected feature.
    pub selection_fill: Rgb,
    /// The caption over the viewport.
    pub notice_fill: Rgb,
    pub notice_text: Rgb,
    /// Opacity of the caption's background, 0 to 255.
    pub notice_opacity: u8,
    /// Corner radius of buttons and rows, in points.
    pub radius: u8,
    /// Corner radius of the caption.
    pub notice_radius: u8,
    /// Width of sketch lines, in points.
    pub sketch_width: f32,
    pub construction_width: f32,
    pub axis_width: f32,
    /// Width of a body's edges, in points.
    pub edge_width: f32,
    /// Radius of a sketch point's dot, in points.
    pub point_radius: f32,
    /// Width of a highlighted edge, in points; its colour is
    /// `face_highlight`.
    pub highlight_width: f32,
    /// Radius of a highlighted vertex's dot, in points.
    pub highlight_radius: f32,
    /// How near to an edge or a vertex a click counts as a click on it, in points
    /// (the pick radius of `view.pick`, which is in pixels).
    pub pick_radius: f32,
    /// The side of the orbit gizmo, in points.
    pub gizmo_size: f32,
    /// The radius of the disc at an end of the gizmo's axes, in points.
    pub gizmo_end_radius: f32,
    /// Opacity of the gizmo's backdrop, 0 to 255.
    pub gizmo_opacity: u8,
    /// The length of the see-through slider, in points.
    pub slider_width: f32,
    /// Space between the viewport's edge and what floats over it.
    pub inset: f32,
}

impl Default for Tokens {
    fn default() -> Self {
        Self::dark()
    }
}

impl Tokens {
    /// The one theme of phase 1.
    pub fn dark() -> Self {
        Self {
            viewport_background: [30, 33, 40],
            body: [198, 204, 214],
            face_highlight: [64, 196, 212],
            sketch: [255, 196, 64],
            sketch_construction: [110, 170, 235],
            sketch_selected: [255, 120, 80],
            axes: [[226, 82, 82], [96, 190, 96], [86, 136, 240]],
            edge: [22, 24, 30],
            hidden_opacity: 70,
            panel_fill: [38, 41, 49],
            text: [222, 226, 232],
            text_dim: [140, 147, 160],
            ok: [104, 190, 120],
            failed: [236, 96, 88],
            inactive: [120, 126, 138],
            attention: [240, 180, 80],
            selection_fill: [58, 72, 104],
            notice_fill: [16, 18, 22],
            notice_text: [245, 246, 248],
            notice_opacity: 215,
            radius: 4,
            notice_radius: 6,
            sketch_width: 1.75,
            construction_width: 1.0,
            axis_width: 1.25,
            edge_width: 1.0,
            point_radius: 2.25,
            highlight_width: 3.0,
            highlight_radius: 4.0,
            pick_radius: 4.0,
            gizmo_size: 92.0,
            gizmo_end_radius: 9.0,
            gizmo_opacity: 110,
            slider_width: 90.0,
            inset: 10.0,
        }
    }

    /// Makes egui's own widgets (panels, labels, buttons) use the tokens.
    pub fn apply(&self, ctx: &egui::Context) {
        let tokens = *self;
        ctx.set_theme(egui::Theme::Dark);
        ctx.all_styles_mut(move |style| {
            let visuals = &mut style.visuals;
            visuals.dark_mode = true;
            visuals.panel_fill = color(tokens.panel_fill);
            visuals.window_fill = color(tokens.panel_fill);
            visuals.override_text_color = Some(color(tokens.text));
            visuals.selection.bg_fill = color(tokens.selection_fill);
            let radius = egui::CornerRadius::same(tokens.radius);
            for widget in [
                &mut visuals.widgets.noninteractive,
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.active,
                &mut visuals.widgets.open,
            ] {
                widget.corner_radius = radius;
            }
        });
    }
}

/// A token colour as egui takes it.
pub fn color(rgb: Rgb) -> egui::Color32 {
    let [r, g, b] = rgb;
    egui::Color32::from_rgb(r, g, b)
}

/// A token colour with an opacity.
pub fn color_alpha(rgb: Rgb, alpha: u8) -> egui::Color32 {
    let [r, g, b] = rgb;
    egui::Color32::from_rgba_unmultiplied(r, g, b, alpha)
}
