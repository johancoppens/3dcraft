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
    /// The cap of the section plane: where the plane passes through the
    /// inside of a body. Drawn as it is, not shaded, so a cut reads as a
    /// cut whatever the light.
    pub section_cap: Rgb,
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
    /// Datum planes, axes and points: their lines, their dots, the fill of
    /// a plane and their names.
    pub datum: Rgb,
    /// Opacity of the fill of a datum plane, 0 to 255.
    pub datum_fill_opacity: u8,
    /// Width of a datum's lines, in points.
    pub datum_width: f32,
    /// Radius of a datum point's dot, in points.
    pub datum_point_radius: f32,
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
    /// The side of the view cube's place in the viewport, in points.
    pub view_cube_size: f32,
    /// The smallest side the view cube is drawn at, in points: a viewport
    /// with less room has no cube (its buttons stay).
    pub view_cube_min: f32,
    /// The view cube's proportions, in units of its half side so that
    /// they scale with it: how deep its edges are cut (a capsule and a
    /// corner's circle are as wide as the cut less the gap), the one gap
    /// between any two patches, and the corner radius of a face as a
    /// fraction of the cut.
    pub view_cube_chamfer: f32,
    pub view_cube_gap: f32,
    pub view_cube_face_rounding: f32,
    /// A patch of the view cube that faces the viewer less than this (0
    /// edge-on, 1 straight on) is not drawn: no slivers on the silhouette.
    pub view_cube_fade: f32,
    /// The glass of the view cube: the colour of its faces and of its
    /// capsules and circles, drawn over the picture without outlines.
    pub view_cube_glass: Rgb,
    pub view_cube_glass_dim: Rgb,
    /// The dark base under every patch (`panel_fill` at this opacity), so
    /// the glass and its names read over a light model too; on the dark
    /// background it is hardly seen.
    pub view_cube_base_opacity: u8,
    /// The opacity of a face (0 to 255) turned away from the light and
    /// of one facing it; the light comes from `view_cube_light`, a
    /// direction in the world (the top is lit most).
    pub view_cube_face_opacity: [u8; 2],
    pub view_cube_light: [f32; 3],
    /// The opacity of a capsule and of a corner's circle seen straight on;
    /// less as they turn away.
    pub view_cube_edge_opacity: u8,
    pub view_cube_corner_opacity: u8,
    /// The patch under the pointer: its fill, the fill's opacity, and its
    /// thin bright outline with the outline's width in points.
    pub view_cube_hover: Rgb,
    pub view_cube_hover_opacity: u8,
    pub view_cube_hover_outline: Rgb,
    pub view_cube_hover_width: f32,
    /// The names on the faces: their colour, their height in units of the
    /// cube's half side, the widest they may be as a fraction of a face's
    /// flat side, and how much a face must face the viewer to show its
    /// name (as `view_cube_fade`).
    pub view_cube_label: Rgb,
    pub view_cube_label_size: f32,
    pub view_cube_label_width: f32,
    pub view_cube_label_hide: f32,
    /// The smallest size a name on a face of the view cube is drawn at,
    /// in points.
    pub view_cube_label_min: f32,
    /// The soft shadow under the view cube: its colour, its opacity in the
    /// middle (it fades to nothing at its rim), and how far below the
    /// cube's middle it is, its half width and its half height, in units
    /// of the radius of the cube's picture.
    pub view_cube_shadow: Rgb,
    pub view_cube_shadow_opacity: u8,
    pub view_cube_shadow_size: [f32; 3],
    /// The side of the place of the axis triad left of the view cube, and
    /// the width of its arms, in points.
    pub triad_size: f32,
    pub triad_width: f32,
    /// The glass plates behind the controls over the viewport: a dark base
    /// (`panel_fill` at `plate_opacity`, so the controls read over a light
    /// model too), the glass tint over it (`glass` at
    /// `plate_glass_opacity`), a faint rim, and their corner radius.
    pub plate_opacity: u8,
    pub plate_glass_opacity: u8,
    pub plate_rim: Rgb,
    pub plate_rim_opacity: u8,
    pub plate_radius: u8,
    /// The space between a plate's edge and its controls, between two
    /// columns of controls and between two plates, between two actions,
    /// and on either side of the divider in the bottom bar, in points.
    pub plate_padding: f32,
    pub group_gap: f32,
    pub control_spacing: f32,
    pub divider_gap: f32,
    pub divider_opacity: u8,
    /// The glass tint of plates and controls, and the accent of what is
    /// on (a switch, a chosen segment, a number field's bar).
    pub glass: Rgb,
    pub accent: Rgb,
    /// A control's fill (0 to 255, the colour is `glass`), its text size
    /// and its height, in points; how opaque an inert control is drawn (0
    /// to 1).
    pub control_opacity: u8,
    pub control_font: f32,
    pub bar_height: f32,
    pub disabled_opacity: f32,
    /// A switch's track when off, and its knob; the size of its track (the
    /// knob is a circle a point inside its height), in points.
    pub switch_off: Rgb,
    pub knob: Rgb,
    pub switch_size: egui::Vec2,
    /// A number field: its least width, its corner radius, and its fill
    /// while it is typed into, in points.
    pub field_width: f32,
    pub field_radius: u8,
    pub field_typing: Rgb,
    /// A number field's paddings, in points: from its sides to its label
    /// and value, and to them when the ‹ › show (also how wide the arrows'
    /// targets are); from its sides to the arrows; from its sides to the
    /// typed text; and its accent bar: from the field's edges, its height
    /// and its corner radius.
    pub field_inset: f32,
    pub field_arrow_inset: f32,
    pub field_arrow_edge: f32,
    pub field_text_pad: f32,
    pub field_bar_inset: f32,
    pub field_bar_height: f32,
    pub field_bar_radius: u8,
    /// The space between two rows of things over the viewport, in points.
    pub bar_gap: f32,
    /// The least width of the progress bar over the viewport, in points.
    pub slider_width: f32,
    /// Space between the viewport's edge and what floats over it.
    pub inset: f32,
    /// Work in progress over the viewport: the spinner and the part of
    /// the progress bar that is done, the rest of the bar, the spinner's
    /// width and the bar's height, in points.
    pub work_accent: Rgb,
    pub work_track: Rgb,
    pub work_spinner_width: f32,
    pub work_bar: f32,
    /// The row of document tabs above the viewport: the fill of a tab, of
    /// the active one and of one under the pointer, and the active one's
    /// underline.
    pub tab_fill: Rgb,
    pub tab_active_fill: Rgb,
    pub tab_hover_fill: Rgb,
    pub tab_accent: Rgb,
    /// A tab's height, its widest and narrowest width (a longer name is
    /// cut with an ellipsis), the space between two tabs and the width of
    /// the active one's underline, in points.
    pub tab_height: f32,
    pub tab_max_width: f32,
    pub tab_min_width: f32,
    pub tab_gap: f32,
    pub tab_accent_width: f32,
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
            section_cap: craft3d_engine::SECTION_CAP_COLOR,
            sketch: [255, 196, 64],
            sketch_construction: [110, 170, 235],
            sketch_selected: [255, 120, 80],
            axes: [[226, 82, 82], [96, 190, 96], [86, 136, 240]],
            edge: [22, 24, 30],
            datum: [190, 140, 255],
            datum_fill_opacity: 48,
            datum_width: 1.25,
            datum_point_radius: 4.0,
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
            view_cube_size: 168.0,
            view_cube_min: 48.0,
            view_cube_chamfer: 0.22,
            view_cube_gap: 0.062,
            view_cube_face_rounding: 0.9,
            view_cube_fade: 0.25,
            view_cube_glass: [170, 190, 230],
            view_cube_glass_dim: [150, 168, 205],
            view_cube_base_opacity: 150,
            view_cube_face_opacity: [59, 74],
            view_cube_light: [0.3, -0.5, 0.8],
            view_cube_edge_opacity: 26,
            view_cube_corner_opacity: 30,
            view_cube_hover: [92, 140, 230],
            view_cube_hover_opacity: 170,
            view_cube_hover_outline: [170, 200, 255],
            view_cube_hover_width: 1.2,
            view_cube_label: [225, 233, 250],
            view_cube_label_size: 0.46,
            view_cube_label_width: 0.76,
            view_cube_label_hide: 0.18,
            view_cube_label_min: 7.0,
            view_cube_shadow: [0, 0, 0],
            view_cube_shadow_opacity: 60,
            view_cube_shadow_size: [0.93, 0.72, 0.09],
            triad_size: 84.0,
            triad_width: 2.0,
            plate_opacity: 200,
            plate_glass_opacity: 22,
            plate_rim: [185, 205, 245],
            plate_rim_opacity: 30,
            plate_radius: 10,
            plate_padding: 10.0,
            group_gap: 10.0,
            control_spacing: 6.0,
            divider_gap: 12.0,
            divider_opacity: 45,
            glass: [170, 190, 230],
            accent: [92, 140, 230],
            control_opacity: 32,
            control_font: 13.0,
            bar_height: 24.0,
            disabled_opacity: 0.35,
            switch_off: [96, 104, 122],
            knob: [245, 248, 255],
            switch_size: egui::vec2(18.0, 10.0),
            field_width: 160.0,
            field_radius: 6,
            field_typing: [20, 22, 28],
            field_inset: 10.0,
            field_arrow_inset: 22.0,
            field_arrow_edge: 9.0,
            field_text_pad: 8.0,
            field_bar_inset: 4.0,
            field_bar_height: 2.0,
            field_bar_radius: 1,
            bar_gap: 6.0,
            slider_width: 90.0,
            inset: 10.0,
            work_accent: [64, 196, 212],
            work_track: [58, 62, 72],
            work_spinner_width: 2.5,
            work_bar: 4.0,
            tab_fill: [44, 47, 56],
            tab_active_fill: [30, 33, 40],
            tab_hover_fill: [52, 56, 66],
            tab_accent: [64, 196, 212],
            tab_height: 26.0,
            tab_max_width: 200.0,
            tab_min_width: 72.0,
            tab_gap: 2.0,
            tab_accent_width: 2.0,
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
