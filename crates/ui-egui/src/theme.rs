//! Design tokens: every colour, radius and width the shell draws with
//! (AGENTS.md rule 9). Widgets take them from [`Tokens`]; nothing else in
//! this crate spells out a colour.
//!
//! Colours are plain sRGB bytes, so the offscreen picture (`ui.screenshot`,
//! drawn without egui) uses the very same values as the window.
//!
//! One theme, *Mission* (2026-10-21, the developer's choice): light. Soft-
//! white panels, black chrome at the top and the bottom (menu, tabs,
//! status bar), black plates and cube over a light warm-grey viewport, gold
//! for what is on (black ink on gold), copper for the selection. Whether
//! egui's own widgets get its light or its dark visuals follows from the
//! fills ([`is_light`]), never from a flag.

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
    /// The panels (the feature tree, a dialog) and their text.
    pub panel_fill: Rgb,
    pub text: Rgb,
    pub text_dim: Rgb,
    /// The chrome: the menu bar at the top and the status bar at the
    /// bottom, and the text on them (the row of tabs is `tab_fill`, with
    /// `chrome_text` on the active tab and `chrome_text_dim` on the
    /// others). The last command is named in the status bar in
    /// `chrome_ok` or `chrome_failed`, which read on `status_fill`.
    pub menu_fill: Rgb,
    pub status_fill: Rgb,
    pub chrome_text: Rgb,
    pub chrome_text_dim: Rgb,
    pub chrome_ok: Rgb,
    pub chrome_failed: Rgb,
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
    /// The view cube's proportions, in units of its half side so that
    /// they scale with it: how deep its edges are cut (a capsule and a
    /// corner's circle are as wide as the cut less the gap), the one gap
    /// between any two patches, and the corner radius of a face as a
    /// fraction of the cut.
    pub view_cube_chamfer: f32,
    pub view_cube_gap: f32,
    pub view_cube_face_rounding: f32,
    /// A patch of the view cube that faces the viewer less than this (0
    /// edge-on, 1 straight on) is not drawn: no sub-pixel slivers on the
    /// silhouette. From there it fades in, fully opaque from
    /// `view_cube_fade_full` on, so a face turning away dims gradually.
    pub view_cube_fade: f32,
    pub view_cube_fade_full: f32,
    /// The glass of the view cube: the tint of all its patches, faces,
    /// capsules and circles alike, drawn over the picture without
    /// outlines; its opacity is how a patch is turned from the light.
    pub view_cube_glass: Rgb,
    /// The base under every patch (`view_cube_base` at this opacity), so
    /// the glass and its names read over any model and any background.
    /// The cube is light: a near-white base, the glass a dark tint that
    /// shades the faces turned from the light, dark names.
    pub view_cube_base: Rgb,
    pub view_cube_base_opacity: u8,
    /// The opacity of a face (0 to 255) turned away from the light and
    /// of one facing it; the light comes from `view_cube_light`, a
    /// direction in the world (the top is lit most).
    pub view_cube_face_opacity: [u8; 2],
    pub view_cube_light: [f32; 3],
    /// The patch under the pointer: its fill, the fill's opacity, and its
    /// outline with the outline's width in points (none at 0).
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
    /// The glass plates behind the controls over the viewport: a base
    /// (`plate_fill` at `plate_opacity`, so the controls read over any
    /// model), the glass tint over it (`glass` at `plate_glass_opacity`)
    /// and a faint rim; their corner radius is `nav_radius`. Text and
    /// icons on a plate are `plate_text`, `plate_text_dim` for what is
    /// off.
    pub plate_fill: Rgb,
    pub plate_text: Rgb,
    pub plate_text_dim: Rgb,
    pub plate_opacity: u8,
    pub plate_glass_opacity: u8,
    pub plate_rim: Rgb,
    pub plate_rim_opacity: u8,
    /// A divider between two groups on a plate or in the status bar: its
    /// opacity (0 to 255; the colour is `plate_rim`, `text_dim` in the
    /// status bar), its width in points, and how far it stays from the top
    /// and bottom of its row, as a fraction of the row's height.
    pub divider_opacity: u8,
    pub divider_width: f32,
    pub divider_inset: f32,
    /// The glass tint of plates and controls, and the accent of what is
    /// on (a switch, a chosen segment, a number field's bar).
    pub glass: Rgb,
    pub accent: Rgb,
    /// A control's fill (0 to 255, the colour is `glass`) and its text
    /// size in points; how opaque an inert control is drawn (0 to 1).
    pub control_opacity: u8,
    pub control_font: f32,
    pub disabled_opacity: f32,
    /// The ink of what is on: the text and icon of a toggle that is on,
    /// of a chosen segment.
    pub knob: Rgb,
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
    /// The controls over the viewport (`shell/layout.rs`) share one size
    /// system: every control is `nav_height` high on a plate
    /// `nav_padding` larger on each side, `nav_margin` from the viewport's
    /// edge; `nav_gap` between two plates and between two groups on a
    /// plate (a divider sits in the middle of it), `nav_spacing` between
    /// two controls of a group, in points.
    pub nav_height: f32,
    pub nav_padding: f32,
    pub nav_margin: f32,
    pub nav_gap: f32,
    pub nav_spacing: f32,
    /// The corner radius of a plate and of a control on it: the plate's
    /// is the control's plus the padding, so the two curves run parallel.
    pub nav_radius: u8,
    pub nav_control_radius: u8,
    /// The chosen segment of a choice: how far inside its cell it is
    /// filled, in points, and its corner radius (the control's less that).
    pub nav_segment_inset: f32,
    pub nav_segment_radius: u8,
    /// The side of the view cube's place; the length of an arm of the
    /// axis triad seen square on, the width of its arms and the size of
    /// its letters, in points (the triad's place is as large as an arm and
    /// a letter on either side).
    pub nav_cube_size: f32,
    pub nav_triad_arm: f32,
    pub nav_triad_width: f32,
    pub nav_triad_font: f32,
    /// A control's icon: its side and the width of its lines, the space
    /// from the control's side to the icon (or to the text when there is
    /// no icon) and between icon and text, in points.
    pub nav_icon_size: f32,
    pub nav_icon_width: f32,
    pub nav_inset: f32,
    pub nav_icon_gap: f32,
    /// A toggle that is on: the accent at this opacity behind it (0 to
    /// 255).
    pub nav_on_opacity: u8,
    /// The status bar: the gap between two items (a divider in its
    /// middle), and the least width an item that is elided keeps, in
    /// points.
    pub status_gap: f32,
    pub status_least: f32,
    /// The size of the problem text over the viewport, in points.
    pub problem_font: f32,
}

impl Default for Tokens {
    fn default() -> Self {
        Self::mission()
    }
}

impl Tokens {
    /// The theme, Mission: soft-white panels like a rover's thermal
    /// blankets, black chrome at the top and the bottom and black plates
    /// over the viewport, gold for what is on (black ink on gold), copper
    /// for the selection, a light warm-grey viewport.
    pub fn mission() -> Self {
        Self {
            viewport_background: [235, 233, 229],
            body: [221, 217, 210],
            face_highlight: [217, 134, 44],
            section_cap: craft3d_engine::SECTION_CAP_COLOR,
            sketch: [183, 112, 12],
            sketch_construction: [63, 120, 181],
            sketch_selected: [184, 50, 31],
            axes: [[210, 63, 54], [58, 154, 77], [52, 103, 209]],
            // A dark warm grey, not black: the shading carries the form.
            edge: [88, 84, 78],
            datum: [126, 87, 194],
            datum_fill_opacity: 48,
            datum_width: 1.25,
            datum_point_radius: 4.0,
            hidden_opacity: 70,
            panel_fill: [247, 246, 243],
            text: [27, 26, 25],
            text_dim: [104, 100, 94],
            menu_fill: [21, 20, 19],
            status_fill: [21, 20, 19],
            chrome_text: [237, 234, 228],
            chrome_text_dim: [158, 153, 143],
            chrome_ok: [224, 169, 62],
            chrome_failed: [240, 103, 90],
            ok: [42, 125, 68],
            failed: [195, 54, 43],
            inactive: [157, 152, 144],
            attention: [168, 106, 18],
            selection_fill: [241, 227, 211],
            notice_fill: [18, 18, 18],
            notice_text: [247, 242, 230],
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
            view_cube_chamfer: 0.22,
            view_cube_gap: 0.062,
            view_cube_face_rounding: 0.9,
            view_cube_fade: 0.03,
            view_cube_fade_full: 0.2,
            // A light cube without lines, shaded only by the light from
            // above: off-white, the top lightest, what is turned from the
            // light a little darker, capsules and corners by the same
            // rule; dark names, a gold patch under the pointer.
            view_cube_glass: [58, 56, 53],
            view_cube_base: [252, 251, 249],
            view_cube_base_opacity: 240,
            view_cube_face_opacity: [72, 0],
            view_cube_light: [0.3, -0.5, 0.8],
            view_cube_hover: [217, 164, 65],
            view_cube_hover_opacity: 230,
            view_cube_hover_outline: [168, 106, 18],
            view_cube_hover_width: 0.0,
            view_cube_label: [27, 26, 25],
            view_cube_label_size: 0.46,
            view_cube_label_width: 0.76,
            view_cube_label_hide: 0.18,
            view_cube_label_min: 7.0,
            view_cube_shadow: [0, 0, 0],
            view_cube_shadow_opacity: 80,
            view_cube_shadow_size: [0.93, 0.72, 0.09],
            plate_fill: [22, 21, 20],
            plate_text: [245, 242, 236],
            plate_text_dim: [168, 163, 155],
            // Black plates and cube over a light viewport: nearly opaque,
            // so light text on them keeps its contrast.
            plate_opacity: 240,
            plate_glass_opacity: 22,
            plate_rim: [255, 255, 255],
            plate_rim_opacity: 30,
            divider_opacity: 45,
            divider_width: 1.0,
            divider_inset: 0.2,
            glass: [43, 42, 40],
            accent: [224, 169, 62],
            control_opacity: 32,
            control_font: 13.0,
            disabled_opacity: 0.35,
            knob: [20, 18, 16],
            field_width: 160.0,
            field_radius: 6,
            field_typing: [0, 0, 0],
            field_inset: 10.0,
            field_arrow_inset: 22.0,
            field_arrow_edge: 9.0,
            field_text_pad: 8.0,
            field_bar_inset: 4.0,
            field_bar_height: 2.0,
            field_bar_radius: 1,
            slider_width: 90.0,
            inset: 10.0,
            work_accent: [201, 138, 30],
            work_track: [220, 215, 207],
            work_spinner_width: 2.5,
            work_bar: 4.0,
            tab_fill: [30, 29, 28],
            tab_active_fill: [44, 43, 41],
            tab_hover_fill: [38, 37, 36],
            tab_accent: [224, 169, 62],
            tab_height: 26.0,
            tab_max_width: 200.0,
            tab_min_width: 72.0,
            tab_gap: 2.0,
            tab_accent_width: 2.0,
            nav_height: 28.0,
            nav_padding: 4.0,
            nav_margin: 12.0,
            nav_gap: 8.0,
            nav_spacing: 2.0,
            nav_radius: 10,
            nav_control_radius: 6,
            nav_segment_inset: 2.0,
            nav_segment_radius: 4,
            nav_cube_size: 108.0,
            nav_triad_arm: 54.0,
            nav_triad_width: 2.5,
            nav_triad_font: 14.0,
            nav_icon_size: 16.0,
            nav_icon_width: 1.5,
            nav_inset: 9.0,
            nav_icon_gap: 6.0,
            // What is on is solid gold with black ink on it.
            nav_on_opacity: 255,
            status_gap: 13.0,
            status_least: 48.0,
            problem_font: 13.0,
        }
    }

    /// Makes egui's own widgets (panels, labels, buttons) use the tokens.
    /// egui's own widgets (panels, labels, buttons, menus) in the tokens: on
    /// egui's light visuals when the panels are light, on its dark ones
    /// when they are dark.
    pub fn apply(&self, ctx: &egui::Context) {
        let visuals = self.panel_visuals();
        ctx.set_theme(if visuals.dark_mode {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        ctx.all_styles_mut(move |style| style.visuals = visuals.clone());
    }

    /// The visuals of the panels: egui's light or dark ones, whichever the
    /// panel's fill is, in the tokens' colours.
    pub fn panel_visuals(&self) -> egui::Visuals {
        let mut visuals = if is_light(self.panel_fill) {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        visuals.panel_fill = color(self.panel_fill);
        visuals.window_fill = color(self.panel_fill);
        visuals.override_text_color = Some(color(self.text));
        visuals.selection.bg_fill = color(self.selection_fill);
        self.round(&mut visuals);
        visuals
    }

    /// The visuals of the chrome (the menu bar): egui's light or dark ones,
    /// whichever `menu_fill` is, with `chrome_text` on it, so a dark menu
    /// bar over light panels has light text and a hover that reads.
    pub fn chrome_visuals(&self) -> egui::Visuals {
        let mut visuals = if is_light(self.menu_fill) {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        visuals.panel_fill = color(self.menu_fill);
        visuals.window_fill = color(self.menu_fill);
        visuals.override_text_color = Some(color(self.chrome_text));
        for (widget, fill) in [
            (&mut visuals.widgets.hovered, self.tab_hover_fill),
            (&mut visuals.widgets.active, self.tab_active_fill),
            (&mut visuals.widgets.open, self.tab_active_fill),
        ] {
            widget.weak_bg_fill = color(fill);
            widget.bg_fill = color(fill);
            widget.bg_stroke = egui::Stroke::NONE;
        }
        self.round(&mut visuals);
        visuals
    }

    fn round(&self, visuals: &mut egui::Visuals) {
        let radius = egui::CornerRadius::same(self.radius);
        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = radius;
        }
    }
}

/// The relative luminance of an sRGB colour (WCAG 2), 0 for black to 1 for
/// white.
pub fn luminance(rgb: Rgb) -> f64 {
    let channel = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b] = rgb;
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// The WCAG 2 contrast ratio of two colours, 1 to 21: text needs 4.5,
/// large text and the parts of a control 3.
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// True for a colour that dark text reads on better than light text.
pub fn is_light(rgb: Rgb) -> bool {
    contrast(rgb, [0, 0, 0]) > contrast(rgb, [255, 255, 255])
}

/// `top` at `alpha` (0 to 255) over `under`, as the painter blends it.
pub fn blend(top: Rgb, alpha: u8, under: Rgb) -> Rgb {
    let a = f64::from(alpha) / 255.0;
    let mut out = [0; 3];
    for ((o, t), u) in out.iter_mut().zip(top).zip(under) {
        *o = (f64::from(t) * a + f64::from(u) * (1.0 - a))
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    out
}

impl Tokens {
    /// What a plate looks like over the viewport's background: its base and
    /// its glass tint.
    pub fn plate_over_background(&self) -> Rgb {
        let base = blend(
            self.plate_fill,
            self.plate_opacity,
            self.viewport_background,
        );
        blend(self.glass, self.plate_glass_opacity, base)
    }

    /// Every pair of text and the fill it is drawn on, by name: each must
    /// keep a contrast of 4.5:1 (WCAG AA for text).
    pub fn text_pairs(&self) -> Vec<(&'static str, Rgb, Rgb)> {
        let plate = self.plate_over_background();
        vec![
            ("text on panel_fill", self.text, self.panel_fill),
            ("text_dim on panel_fill", self.text_dim, self.panel_fill),
            ("text on selection_fill", self.text, self.selection_fill),
            ("ok on panel_fill", self.ok, self.panel_fill),
            ("failed on panel_fill", self.failed, self.panel_fill),
            ("chrome_text on menu_fill", self.chrome_text, self.menu_fill),
            (
                "chrome_text_dim on menu_fill",
                self.chrome_text_dim,
                self.menu_fill,
            ),
            (
                "chrome_text on status_fill",
                self.chrome_text,
                self.status_fill,
            ),
            (
                "chrome_text_dim on status_fill",
                self.chrome_text_dim,
                self.status_fill,
            ),
            ("chrome_ok on status_fill", self.chrome_ok, self.status_fill),
            (
                "chrome_failed on status_fill",
                self.chrome_failed,
                self.status_fill,
            ),
            (
                "chrome_text on tab_active_fill",
                self.chrome_text,
                self.tab_active_fill,
            ),
            (
                "chrome_text_dim on tab_fill",
                self.chrome_text_dim,
                self.tab_fill,
            ),
            (
                "chrome_text_dim on tab_hover_fill",
                self.chrome_text_dim,
                self.tab_hover_fill,
            ),
            ("plate_text on a plate", self.plate_text, plate),
            ("plate_text_dim on a plate", self.plate_text_dim, plate),
            (
                "plate_text on field_typing",
                self.plate_text,
                self.field_typing,
            ),
            ("knob on accent", self.knob, self.accent),
            (
                "knob on a toggle that is on",
                self.knob,
                blend(self.accent, self.nav_on_opacity, plate),
            ),
            (
                "notice_text on notice_fill",
                self.notice_text,
                self.notice_fill,
            ),
            (
                "chrome_failed on notice_fill",
                self.chrome_failed,
                self.notice_fill,
            ),
            (
                "view_cube_label on the cube's darkest face",
                self.view_cube_label,
                blend(
                    self.view_cube_glass,
                    self.view_cube_face_opacity[0].max(self.view_cube_face_opacity[1]),
                    blend(
                        self.view_cube_base,
                        self.view_cube_base_opacity,
                        self.viewport_background,
                    ),
                ),
            ),
            (
                "view_cube_label on the patch under the pointer",
                self.view_cube_label,
                blend(
                    self.view_cube_hover,
                    self.view_cube_hover_opacity,
                    self.viewport_background,
                ),
            ),
            (
                "text_dim on viewport_background",
                self.text_dim,
                self.viewport_background,
            ),
        ]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_contrast_ratio_is_the_wcag_one() {
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
        assert!((contrast([7, 7, 7], [7, 7, 7]) - 1.0).abs() < 1e-9);
        // The figures of tokens.md for Mission (the design's own check).
        let m = Tokens::mission();
        assert!((contrast(m.text, m.panel_fill) - 16.08).abs() < 0.01);
        assert!((contrast(m.knob, m.accent) - 8.82).abs() < 0.01);
        assert!(is_light(m.panel_fill) && !is_light(m.menu_fill));
        assert!(is_light([255, 255, 255]) && !is_light([0, 0, 0]));
    }

    #[test]
    fn every_text_keeps_a_contrast_of_four_and_a_half() {
        let tokens = Tokens::default();
        let mut low = Vec::new();
        for (what, ink, fill) in tokens.text_pairs() {
            let ratio = contrast(ink, fill);
            println!("{what}: {ratio:.2}");
            if ratio < 4.5 {
                low.push(format!("{what} {ratio:.2}"));
            }
        }
        assert!(low.is_empty(), "{low:?}");
    }

    #[test]
    fn light_panels_get_light_visuals_and_the_black_chrome_dark_ones() {
        let tokens = Tokens::default();
        let panels = tokens.panel_visuals();
        assert!(!panels.dark_mode);
        assert_eq!(panels.override_text_color, Some(color(tokens.text)));
        let chrome = tokens.chrome_visuals();
        assert!(chrome.dark_mode);
        assert_eq!(chrome.override_text_color, Some(color(tokens.chrome_text)));
        assert_eq!(chrome.panel_fill, color(tokens.menu_fill));
    }
}
