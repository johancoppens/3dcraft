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

use craft3d_math::Portable;
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
    /// Sketch geometry that is part of a profile, and its points: the
    /// info accent's indigo (2026-10-24, the developer's choice), cool
    /// against the warm body.
    pub sketch: Rgb,
    /// Construction geometry of a sketch: a lighter tint of the same hue,
    /// drawn thinner.
    pub sketch_construction: Rgb,
    /// The sketch of the selected feature: the selection's copper
    /// (`face_highlight`), so what is selected has one colour.
    pub sketch_selected: Rgb,
    /// The world axes at the origin: X, Y, Z.
    pub axes: [Rgb; 3],
    /// The edges that belong to no body: the outline of a section's cap.
    /// A body's own edges take their colour from the body's
    /// ([`Tokens::edge_colors`]).
    pub edge: Rgb,
    /// An edge of a body is the body's colour with each sRGB channel
    /// scaled by `edge_shade` (its hue kept, its lightness lowered), as long
    /// as that contrasts with the body's colour by `edge_min_contrast` or
    /// more (WCAG ratio of relative luminances); on a body too dark for
    /// that, the colour lifted `edge_lift` of the way to white. The outline
    /// of a body against what lies behind it is scaled by
    /// `edge_silhouette_shade`: darker than the edges between its faces.
    pub edge_shade: f32,
    pub edge_min_contrast: f32,
    pub edge_lift: f32,
    pub edge_silhouette_shade: f32,
    /// Edges fade, down to `edge_fade_floor` of their opacity: an edge
    /// whose extent on screen (the diagonal of its box, projected) is
    /// below `edge_fade_long` points, fully at `edge_fade_short`; and edges
    /// where the share of the picture their ink covers (averaged over
    /// tiles of `edge_tile` points, blended smoothly between the tiles)
    /// is above `edge_dense_cap`, by that share over theirs: together
    /// they cover no more of a crowded tile than of one at the cap.
    pub edge_fade_short: f32,
    pub edge_fade_long: f32,
    pub edge_dense_cap: f32,
    pub edge_fade_floor: f32,
    pub edge_tile: f32,
    /// Datum planes, axes and points: their lines, their dots and the fill
    /// of a plane (petrol, 2026-10-24). Their names are `datum_ink`, the
    /// same hue dark enough for text on the viewport.
    pub datum: Rgb,
    pub datum_ink: Rgb,
    /// Opacity of the fill of a datum plane, 0 to 255.
    pub datum_fill_opacity: u8,
    /// Width of a datum's lines, in points.
    pub datum_width: f32,
    /// Radius of a datum point's dot, in points.
    pub datum_point_radius: f32,
    /// A point's path over the motion (`view.trace`): a deep violet,
    /// apart from the sketches' blue, the datums' petrol and the
    /// selection's copper, that stands out on the plain look's light
    /// background and on the studio look's greys (3:1 on both,
    /// `a_trace_stands_out_on_both_looks`).
    pub trace: Rgb,
    /// Width of a trace, in points.
    pub trace_width: f32,
    /// Curvature combs (`craft3d_engine::analysis::combs`): magenta,
    /// apart from the bodies, the sketches and the traces.
    pub comb: Rgb,
    /// Width of a comb's lines, in points.
    pub comb_width: f32,
    /// Surface analysis (`craft3d_render::analysis`): the stripes of
    /// zebra and highlight lines, dark and light.
    pub analysis_dark: Rgb,
    pub analysis_light: Rgb,
    /// Draft: along the pull, against it, too shallow.
    pub draft_in: Rgb,
    pub draft_out: Rgb,
    pub draft_shallow: Rgb,
    /// Where an analysis has no value.
    pub analysis_none: Rgb,
    /// The curvature map's ramp, low to high (diverging: blue, white,
    /// red).
    pub curvature_ramp: [Rgb; 5],
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
    /// Font sizes of a narrated demo's caption and of the history
    /// player's step caption under it, in points.
    pub notice_font: f32,
    pub caption_font: f32,
    /// Size of the body text and of secondary text, in points: one token
    /// for both, so a hint, a note, a feature's or an assertion's
    /// message, what a panel says while it is empty is as large as the
    /// text around it and told apart by its colour (`text_dim` on a panel,
    /// `plate_text_dim` on a plate, `chrome_text_dim` on a bar, each kept
    /// at 4.5:1 by [`Tokens::text_pairs`]). [`Tokens::apply`] sets egui's
    /// `TextStyle::Body` and `TextStyle::Small` to it, so `.small()` is
    /// this size too.
    pub secondary_font: f32,
    /// Width of sketch lines, in points.
    pub sketch_width: f32,
    pub construction_width: f32,
    pub axis_width: f32,
    /// Width of a body's edges between two of its faces, and of its
    /// outline against what lies behind it, in points.
    pub edge_width: f32,
    pub edge_silhouette_width: f32,
    /// Radius of a sketch point's dot, in points.
    pub point_radius: f32,
    /// Width of a highlighted edge, in points; its colour is
    /// `face_highlight`.
    pub highlight_width: f32,
    /// Radius of a highlighted vertex's dot, in points.
    pub highlight_radius: f32,
    /// The box dragged to select (Ctrl and a drag): its fill, the
    /// selection's colour (`face_highlight`) at this opacity (0 to 255),
    /// and its outline's width in points.
    pub select_box_opacity: u8,
    pub select_box_width: f32,
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
    /// A toggle that is off (Studio, Analysis, Section, …): enabled, so
    /// nearly as bright as `plate_text`, with a faint rim of it at
    /// `nav_off_rim_opacity`; the accent fill says which are on
    /// (2026-10-30: `plate_text_dim` read as disabled).
    pub nav_off_text: Rgb,
    pub nav_off_rim_opacity: u8,
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
    /// The second accent, cool against the warm one (the developer's
    /// choice, 2026-10-24): `accent` is what acts (a button, the active
    /// tab, a toggle that is on), `info_accent` is information and
    /// connection (the hover ring of an example, the bar beside the agent
    /// block and the start screen's headings). `info_ink` is the same hue
    /// dark enough for text on the light panels: egui's link colour (the
    /// recent files, How it works, the About dialog's links) and its
    /// `selection.stroke`, which is a text field's focus ring and also the
    /// colour of selected text. `info_code` is light enough for text on a
    /// black plate (the program's name in a line of code), and the chrome's
    /// link colour, should a link appear there.
    pub info_accent: Rgb,
    pub info_ink: Rgb,
    pub info_code: Rgb,
    /// The status bar's agents pill (2026-11-02): the green dot of a
    /// connected agent (on `info_accent`), the pill's corner radius, and
    /// the width of the agents popover above it, in points.
    pub agent_dot: Rgb,
    pub pill_radius: u8,
    pub agent_popover_width: f32,
    /// Width of the info accent's bars (beside a heading, beside the
    /// agent block) and their corner radius, and the corner radius of an
    /// example's thumbnail, in points. A heading's bar is as high as the
    /// body text's font size.
    pub info_bar_width: f32,
    pub info_bar_radius: u8,
    /// Width of the dashed outline of the history player's position in
    /// the feature list (in the info accent), in points: wide enough to
    /// read on the dark panel.
    pub player_mark_width: f32,
    pub thumb_radius: u8,
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
    /// Space between the viewport's edge and what floats over it.
    pub inset: f32,
    /// The widest a dialog over the window (an error, the export's
    /// options) lays its text out, in points.
    pub dialog_width: f32,
    /// Work in progress (2026-10-30, the developer's design after the
    /// bar across the viewport flickered under a motion played 10 times a
    /// second): a small spinner `work_spinner` points across, in the info
    /// accent (progress is information), at the start of an item of the
    /// status bar `status_work_width` points wide whatever it says, with
    /// the words after it. It shows only for work that has run longer
    /// than `work_after_seconds` (a motion's moment, a quick edit never
    /// shows it), and once shown it stays at least `work_hold_seconds`,
    /// so it never blinks. While the viewport has nothing to show yet (a
    /// first file opening), a small note in its middle,
    /// `work_note_width` points wide, says what is opening (after the
    /// same wait). The texts of the status bar that change while work or
    /// a motion runs (the last command and its answer, the renderer's
    /// numbers, the work) change at most once every
    /// `status_hold_seconds`; an error shows at once.
    pub work_spinner: f32,
    pub work_after_seconds: f32,
    pub work_hold_seconds: f32,
    pub status_hold_seconds: f32,
    pub work_note_width: f32,
    pub status_work_width: f32,
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
    /// The side panel on the left (feature tree, Parameters, player): its
    /// width when nothing set it, its narrowest and widest width, and the
    /// share of the window it may take at most, in points. The width is
    /// the person's (`state.panels.treeWidth`): only a drag on the split
    /// changes it, never what the panel shows. At `tree_min_width` the
    /// history player's five buttons still fit on a row.
    /// `tree_split_grip` is how wide the split is to grab.
    pub tree_width: f32,
    pub tree_min_width: f32,
    pub tree_max_width: f32,
    pub tree_max_share: f32,
    pub tree_split_grip: f32,
    /// The popovers of the Studio and Analysis chips: their width, in
    /// points (the height follows what they hold, scrolling only where
    /// the viewport is too short).
    pub popover_width: f32,
    /// The caret on the right of the Studio and Analysis chips, which opens
    /// and closes their popover: its width in points (a split button).
    pub nav_caret_width: f32,
    /// The caret's chevron: its half width as a share of `nav_icon_size`
    /// (drawn in `nav_icon_width`, as the icons are).
    pub nav_caret_chevron: f32,
    /// The caret's width with touch sizes ([`Tokens::touch`]): as wide as
    /// a finger needs while the camera plate, with both split buttons,
    /// still fits a phone of 390 points (the full `touch_target` would
    /// push the plate off it; 2026-10-31).
    pub touch_caret_width: f32,
    /// How far the popover's pointer reaches up from its top edge toward
    /// its chip, in points (within the gap under the camera plate).
    pub popover_pointer: f32,
    /// The one width of the sliders in a popover, in points.
    pub popover_slider_width: f32,
    /// The popovers' rows (board s1): the label's column, the value's.
    pub popover_label_width: f32,
    pub popover_value_width: f32,
    /// A popover's fill: `plate_fill` at this opacity (0 to 255), more
    /// opaque than a plate's so its rows read over any model.
    pub popover_opacity: u8,
    /// A slider's rail and knob and a check box's square in a popover:
    /// `plate_text_dim` at this opacity (0 to 255).
    pub popover_widget_opacity: u8,
    /// Below `narrow_width` points of window width the shell lays out for
    /// a phone or a small tablet (and a narrow desktop window alike): the
    /// side panel becomes a drawer behind a button, at most
    /// `drawer_share` of the window wide, and the controls over the
    /// viewport are `touch_target` high, a size a finger hits.
    pub narrow_width: f32,
    pub drawer_share: f32,
    pub touch_target: f32,
    /// The drawer's button: its ☰ three lines `drawer_icon_width` of the
    /// button's side long and `drawer_icon_gap` of it apart; the drawer
    /// slides in and out in `drawer_seconds`.
    pub drawer_icon_width: f32,
    pub drawer_icon_gap: f32,
    pub drawer_seconds: f32,
    /// A history player's button is the row's height and
    /// `player_button_extra` points more; the compact player keeps
    /// `compact_player_label` points for its `t` or step.
    pub player_button_extra: f32,
    pub compact_player_label: f32,
    /// The button that lists every open document when the tabs do not
    /// fit, and the least width of a tab in the narrow layout.
    pub tab_overflow_width: f32,
    pub tab_narrow_min_width: f32,
    /// Below this card width the start screen's Recent files and agent
    /// block stand one under the other.
    pub start_stacked_width: f32,
    /// Two taps at most `double_tap_seconds` apart and
    /// `double_tap_distance` points apart are a double tap.
    pub double_tap_seconds: f32,
    pub double_tap_distance: f32,
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
    /// The hint in the middle of an empty viewport: drawn on the viewport,
    /// which is not themed (light in both appearances), so it keeps the
    /// light set's dim ink whatever the panels are.
    pub viewport_text: Rgb,
    /// Why the viewport shows no picture or not the right one, written on
    /// the viewport: the same in both appearances, the light set's failure
    /// red a touch darker (#C0352A) so it keeps 4.5:1 on the viewport's
    /// background (`failed` itself has 4.46 there).
    pub viewport_failed: Rgb,
    /// On the black chrome, the same in both appearances: the status bar's
    /// work spinner (the light set's info accent) and a tab's unsaved mark
    /// (the light set's attention amber).
    pub chrome_work: Rgb,
    pub chrome_attention: Rgb,
    // The component kit (`crate::kit`, docs/design/rendering.md › *The
    // component kit*), 2026-11-02. The colours from here to
    // `popover_rim_opacity` change with the appearance ([`Palette`]).
    /// A row under the pointer, on a panel.
    pub row_hover_fill: Rgb,
    /// A rule between two sections of a panel.
    pub divider: Rgb,
    /// A field, a secondary button, a segmented control's track, and their
    /// outline.
    pub field_fill: Rgb,
    pub field_stroke: Rgb,
    /// The fill behind information: an info banner, the agent's card, an
    /// info chip (with `info_ink` on it).
    pub info_fill: Rgb,
    /// The fill of a failure: an error banner, a failed row, a failed chip
    /// (with `failed` on it).
    pub failed_fill: Rgb,
    /// A chip at rest.
    pub chip_fill: Rgb,
    /// The fill of what waits for a decision (the recovery bar, an
    /// attention banner).
    pub attention_fill: Rgb,
    /// A slider's rail, a switch that is off, a progress bar's rail.
    pub track: Rgb,
    /// A slider's knob and its rim.
    pub knob_fill: Rgb,
    pub knob_rim: Rgb,
    /// The feature icons of a sketch and a datum in a list: the colours
    /// they have in the viewport (`sketch`, `datum`) where those read on
    /// the panel, a lighter tint of the same hue on a dark panel.
    pub icon_sketch: Rgb,
    pub icon_datum: Rgb,
    /// Popovers and menus (always dark): their fill, the fill of a field
    /// and of a row under the pointer on them, and the opacity of their
    /// white rim. In the dark appearance they sit one step lighter than
    /// the panel, so they still lift off it.
    pub popover_fill: Rgb,
    pub popover_field: Rgb,
    pub popover_hover: Rgb,
    pub popover_rim_opacity: u8,
    /// On the plates (dark in both appearances): an "ok" that reads there
    /// (`chrome_ok` is the amber of the status bar), a field and its
    /// outline (also a slider's rail), the selected row, and the fills of
    /// information, a failure and what waits (as on a panel, dark).
    pub plate_ok: Rgb,
    pub plate_field: Rgb,
    pub plate_field_stroke: Rgb,
    pub plate_selection_fill: Rgb,
    pub plate_info_fill: Rgb,
    pub plate_failed_fill: Rgb,
    pub plate_attention_fill: Rgb,
    /// The info accent (a section's bar, a spinner) and a datum's icon on
    /// a plate: lighter tints that read on the dark plates.
    pub plate_info_accent: Rgb,
    pub plate_datum: Rgb,
    /// The agent's status pill on the status bar: the rim of "off", the
    /// text of "waiting" and of "asks".
    pub pill_off_rim: Rgb,
    pub pill_wait_text: Rgb,
    pub pill_asks_text: Rgb,
    /// The fill of the pill while an agent is connected: the light set's
    /// info accent in both appearances (the status bar is chrome).
    pub pill_connected: Rgb,
    /// The kit's sizes, in points: a row of a list, a control (button,
    /// field, segmented, icon button), a chip, the status pill, a menu's
    /// item, a panel's tab bar.
    pub row_height: f32,
    pub control_height: f32,
    /// A row that is only read (a label and its value, a section's
    /// heading): 22 with touch sizes too, since nothing there is hit.
    pub label_row_height: f32,
    pub chip_height: f32,
    pub pill_height: f32,
    pub menu_item_height: f32,
    pub panel_tab_height: f32,
    /// A menu's corner radius, and the width a menu takes at least.
    pub menu_radius: u8,
    pub menu_width: f32,
    /// A check box's side, a switch's width and height, a slider's knob
    /// (with touch sizes: `touch_knob`), a progress bar's height.
    pub check_size: f32,
    pub switch_size: [f32; 2],
    pub knob_size: f32,
    pub touch_knob: f32,
    pub progress_height: f32,
    /// The width of the pointer of a popover (its height is
    /// `popover_pointer`).
    pub popover_pointer_width: f32,
    /// The width of the line icons' strokes at 16 points (feature and
    /// document icons; they scale with their size).
    pub icon_stroke: f32,
    /// The least side of what a finger hits: 0 with a mouse, `touch_target`
    /// with touch sizes ([`Tokens::touch`]). A kit control smaller than
    /// this to the eye takes this much room for its target.
    pub hit_min: f32,
    /// The rim of the app icon on a dark ground: white at this opacity.
    pub app_icon_rim_opacity: u8,
    /// The shadow under a popover, a menu and a tooltip: `view_cube_shadow`
    /// at this opacity, `shadow_blur` points soft, `shadow_drop` points
    /// down.
    pub shadow_opacity: u8,
    pub shadow_blur: u8,
    pub shadow_drop: i8,
    /// The kit's dialog (s4–s6 of the plan): its title's size (17 Semi
    /// Bold), the room around its parts, its corner radius, and the side
    /// of the icon before its title.
    pub dialog_title_font: f32,
    pub dialog_padding: f32,
    pub dialog_radius: u8,
    pub dialog_icon: f32,
    /// The app icon in the start screen's header and the About dialog's
    /// (the plan's middle way: the original icon where 3DCraft shows
    /// itself), and the badge on a Showcase card's thumbnail.
    pub header_icon: f32,
    pub about_icon: f32,
    pub card_badge: f32,
    /// The narrowest a dialog gets on a small window, and the error
    /// details' window (its width, and how high its report scrolls).
    pub dialog_min_width: f32,
    /// A dialog with buttons in a row of three (the questions, an error),
    /// the export dialog, the shortcuts in their two columns.
    pub dialog_wide_width: f32,
    pub export_dialog_width: f32,
    pub shortcuts_width: f32,
    pub details_width: f32,
    pub details_height: f32,
    /// The menu bar's menus' widths: File, Edit, View, Help (board s4).
    pub menu_widths: [f32; 4],
    /// The Agent tab's time line: a dot's radius, the rail's width.
    pub log_dot_radius: f32,
    pub log_rail_width: f32,
    /// The room a banner over the viewport keeps for its button (Back to
    /// last good).
    pub banner_button_room: f32,
    /// A bottom sheet's share of the viewport's height (Studio and
    /// Analysis on a phone, m4 and m5), and of the window's for the Jig's
    /// parameters (m8).
    pub sheet_share: f32,
    /// A sheet's grabber: its size and how strong its ink is.
    pub grabber_size: [f32; 2],
    pub grabber_opacity: u8,
    pub half_sheet_share: f32,
    /// The side panel docked beside a Jig's model on a tablet (t2), and
    /// the narrowest window that docks it.
    pub docked_panel_width: f32,
    pub docked_min_window: f32,
}

/// The colours that change with the appearance (light or dark): the
/// panels, their text and the kit's fills. The chrome (menu bar, tabs,
/// status bar), the plates over the viewport and the viewport itself keep
/// their colours in both ([`Tokens::with_palette`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub panel_fill: Rgb,
    pub text: Rgb,
    pub text_dim: Rgb,
    pub inactive: Rgb,
    pub ok: Rgb,
    pub failed: Rgb,
    pub attention: Rgb,
    pub selection_fill: Rgb,
    pub info_accent: Rgb,
    pub info_ink: Rgb,
    pub row_hover_fill: Rgb,
    pub divider: Rgb,
    pub field_fill: Rgb,
    pub field_stroke: Rgb,
    pub info_fill: Rgb,
    pub failed_fill: Rgb,
    pub chip_fill: Rgb,
    pub attention_fill: Rgb,
    pub track: Rgb,
    pub knob_fill: Rgb,
    pub knob_rim: Rgb,
    pub icon_sketch: Rgb,
    pub icon_datum: Rgb,
    pub popover_fill: Rgb,
    pub popover_field: Rgb,
    pub popover_hover: Rgb,
    pub popover_rim_opacity: u8,
}

/// Copies the palette's fields between [`Tokens`] and [`Palette`].
macro_rules! palette_fields {
    ($($field:ident),* $(,)?) => {
        impl Tokens {
            /// The colours of these tokens that change with the appearance.
            pub fn palette(&self) -> Palette {
                Palette { $($field: self.$field),* }
            }

            /// These tokens with the colours of `palette`; every size and
            /// every other colour kept (the touch sizes too).
            #[must_use]
            pub fn with_palette(mut self, palette: Palette) -> Self {
                $(self.$field = palette.$field;)*
                self
            }
        }
    };
}

palette_fields!(
    panel_fill,
    text,
    text_dim,
    inactive,
    ok,
    failed,
    attention,
    selection_fill,
    info_accent,
    info_ink,
    row_hover_fill,
    divider,
    field_fill,
    field_stroke,
    info_fill,
    failed_fill,
    chip_fill,
    attention_fill,
    track,
    knob_fill,
    knob_rim,
    icon_sketch,
    icon_datum,
    popover_fill,
    popover_field,
    popover_hover,
    popover_rim_opacity,
);

impl Palette {
    /// The light appearance: Mission's panels (2026-10-21) and the kit's
    /// fills of the UI overhaul (2026-11-02).
    pub fn light() -> Self {
        Tokens::mission().palette()
    }

    /// The dark appearance (plan of the UI overhaul, 2026-11-02, made the
    /// darkest variant by the developer on 2026-11-04): the panels are the
    /// plates' near-black #161514, every layer above them one step
    /// lighter (fields, hover and chips; popovers and menus one step
    /// #201F1D), light warm text, the selection's copper darkened, the
    /// info accent and its ink lightened so they read on the dark panel.
    pub fn dark() -> Self {
        Self {
            panel_fill: [0x16, 0x15, 0x14],
            text: [0xED, 0xEA, 0xE4],
            text_dim: [0xA3, 0x9E, 0x95],
            inactive: [0x7A, 0x75, 0x6E],
            ok: [0x6C, 0xC0, 0x8A],
            failed: [0xF0, 0x67, 0x5A],
            attention: [0xE8, 0xB6, 0x5A],
            selection_fill: [0x5C, 0x42, 0x20],
            info_accent: [0x7F, 0x8A, 0xD0],
            info_ink: [0xA9, 0xB3, 0xEC],
            row_hover_fill: [0x22, 0x21, 0x1F],
            divider: [0x2A, 0x28, 0x26],
            field_fill: [0x1F, 0x1E, 0x1C],
            field_stroke: [0x3A, 0x38, 0x36],
            info_fill: [0x26, 0x2A, 0x44],
            failed_fill: [0x3B, 0x20, 0x1D],
            chip_fill: [0x2A, 0x29, 0x27],
            attention_fill: [0x3A, 0x2E, 0x19],
            track: [0x3A, 0x38, 0x36],
            knob_fill: [0xF5, 0xF2, 0xEC],
            knob_rim: [0x7A, 0x75, 0x6E],
            icon_sketch: [0x86, 0x92, 0xD6],
            icon_datum: [0x4F, 0xB3, 0xAE],
            popover_fill: [0x20, 0x1F, 0x1D],
            popover_field: [0x2A, 0x29, 0x27],
            popover_hover: [0x2E, 0x2D, 0x2A],
            popover_rim_opacity: 31,
        }
    }
}

impl Default for Tokens {
    fn default() -> Self {
        Self::mission()
    }
}

impl Tokens {
    /// The colours the renderers draw an analysis in.
    pub fn analysis_colors(&self) -> craft3d_render::analysis::AnalysisColors {
        craft3d_render::analysis::AnalysisColors {
            dark: self.analysis_dark,
            light: self.analysis_light,
            draft_in: self.draft_in,
            draft_out: self.draft_out,
            draft_shallow: self.draft_shallow,
            none: self.analysis_none,
            ramp: self.curvature_ramp,
        }
    }

    /// The theme, Mission: soft-white panels like a rover's thermal
    /// blankets, black chrome at the top and the bottom and black plates
    /// over the viewport, gold for what is on (black ink on gold), copper
    /// for the selection, a light warm-grey viewport.
    pub fn mission() -> Self {
        Self {
            viewport_background: craft3d_engine::VIEWPORT_BACKGROUND,
            body: [221, 217, 210],
            face_highlight: [217, 134, 44],
            section_cap: craft3d_engine::SECTION_CAP_COLOR,
            // Indigo #4C5AA6 (the info accent), its light tint #8692D6 for
            // construction, the selection's copper #D9862C.
            sketch: [76, 90, 166],
            sketch_construction: [134, 146, 214],
            sketch_selected: [217, 134, 44],
            axes: [[210, 63, 54], [58, 154, 77], [52, 103, 209]],
            // A dark warm grey, not black: the shading carries the form.
            edge: [88, 84, 78],
            // An edge 40 % of its body's lightness, an outline 25 %: on the
            // pier's red (L 0.09) that is 2.2:1; black wheels (L 0.01) get
            // light edges instead, since darker than black reads as nothing.
            edge_shade: 0.4,
            edge_min_contrast: 1.5,
            edge_lift: 0.45,
            edge_silhouette_shade: 0.25,
            edge_fade_short: 3.0,
            edge_fade_long: 12.0,
            edge_dense_cap: 0.15,
            edge_fade_floor: 0.05,
            edge_tile: 16.0,
            // Petrol #2E7D7A; its names #266C69. A plane's fill at 48
            // stands off the viewport as the old violet's did (1.25:1).
            datum: [46, 125, 122],
            datum_ink: [38, 108, 105],
            datum_fill_opacity: 48,
            datum_width: 1.25,
            datum_point_radius: 4.0,
            trace: [80, 0, 140],
            trace_width: 3.0,
            comb: [204, 51, 153],
            comb_width: 1.0,
            analysis_dark: [20, 22, 26],
            analysis_light: [245, 246, 248],
            draft_in: [67, 160, 71],
            draft_out: [30, 136, 229],
            draft_shallow: [229, 57, 53],
            analysis_none: [128, 128, 128],
            curvature_ramp: [
                [44, 123, 182],
                [171, 217, 233],
                [250, 250, 210],
                [253, 174, 97],
                [215, 25, 28],
            ],
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
            selection_fill: [236, 192, 135],
            notice_fill: [18, 18, 18],
            notice_text: [247, 242, 230],
            notice_opacity: 215,
            radius: 4,
            notice_radius: 6,
            notice_font: 17.0,
            caption_font: 15.0,
            secondary_font: 13.0,
            sketch_width: 1.75,
            construction_width: 1.0,
            axis_width: 1.25,
            edge_width: 1.0,
            edge_silhouette_width: 1.6,
            point_radius: 2.25,
            highlight_width: 3.0,
            highlight_radius: 4.0,
            select_box_opacity: 31,
            select_box_width: 1.5,
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
            nav_off_text: [232, 228, 220],
            nav_off_rim_opacity: 52,
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
            // Indigo #4C5AA6; its ink #3C4890 and its light tint #A9B3EC.
            info_accent: [76, 90, 166],
            info_ink: [60, 72, 144],
            info_code: [169, 179, 236],
            agent_dot: [110, 220, 140],
            pill_radius: 10,
            agent_popover_width: 470.0,
            info_bar_width: 3.0,
            info_bar_radius: 1,
            player_mark_width: 2.0,
            thumb_radius: 6,
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
            inset: 10.0,
            dialog_width: 420.0,
            work_spinner: 9.0,
            work_after_seconds: 0.35,
            work_hold_seconds: 0.3,
            status_hold_seconds: 0.25,
            work_note_width: 240.0,
            status_work_width: 250.0,
            tab_fill: [30, 29, 28],
            tab_active_fill: [44, 43, 41],
            tab_hover_fill: [38, 37, 36],
            tab_accent: [224, 169, 62],
            tab_height: 26.0,
            tab_max_width: 200.0,
            tab_min_width: 72.0,
            tab_gap: 2.0,
            tab_accent_width: 2.0,
            tree_width: 260.0,
            tree_min_width: 190.0,
            tree_max_width: 640.0,
            tree_max_share: 0.5,
            tree_split_grip: 6.0,
            popover_width: 340.0,
            nav_caret_width: 18.0,
            nav_caret_chevron: 0.2,
            touch_caret_width: 30.0,
            popover_pointer: 6.0,
            popover_slider_width: 130.0,
            popover_label_width: 96.0,
            popover_value_width: 56.0,
            popover_opacity: 242,
            popover_widget_opacity: 110,
            narrow_width: 800.0,
            drawer_share: 0.85,
            touch_target: 40.0,
            drawer_icon_width: 0.36,
            drawer_icon_gap: 0.12,
            drawer_seconds: 0.1,
            player_button_extra: 6.0,
            compact_player_label: 84.0,
            tab_overflow_width: 56.0,
            tab_narrow_min_width: 120.0,
            start_stacked_width: 600.0,
            double_tap_seconds: 0.35,
            double_tap_distance: 24.0,
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
            viewport_text: [104, 100, 94],
            viewport_failed: [0xC0, 0x35, 0x2A],
            chrome_work: [76, 90, 166],
            chrome_attention: [168, 106, 18],
            // The kit's light fills (plan of the side panel and of the UI
            // overhaul, 2026-11-02).
            row_hover_fill: [0xED, 0xEB, 0xE6],
            divider: [0xE2, 0xDF, 0xD9],
            field_fill: [0xEE, 0xEC, 0xE7],
            field_stroke: [0xD3, 0xCE, 0xC6],
            info_fill: [0xE6, 0xE8, 0xF3],
            failed_fill: [0xFB, 0xEB, 0xE8],
            chip_fill: [0xEA, 0xE7, 0xE1],
            attention_fill: [0xF8, 0xEE, 0xDC],
            track: [0xDA, 0xD6, 0xCF],
            knob_fill: [255, 255, 255],
            knob_rim: [0xB4, 0xAE, 0xA4],
            icon_sketch: [76, 90, 166],
            icon_datum: [46, 125, 122],
            popover_fill: [22, 21, 20],
            popover_field: [0x26, 0x25, 0x23],
            popover_hover: [44, 43, 41],
            popover_rim_opacity: 30,
            plate_ok: [0x6C, 0xC0, 0x8A],
            plate_field: [0x26, 0x25, 0x23],
            plate_field_stroke: [0x3A, 0x38, 0x36],
            plate_selection_fill: [0x5A, 0x45, 0x26],
            plate_info_fill: [0x26, 0x2A, 0x44],
            plate_failed_fill: [0x3B, 0x20, 0x1D],
            plate_attention_fill: [0x3A, 0x2E, 0x19],
            plate_info_accent: [0x7F, 0x8A, 0xD0],
            plate_datum: [0x4F, 0xB3, 0xAE],
            pill_off_rim: [0x4A, 0x47, 0x43],
            pill_wait_text: [0xC9, 0xCE, 0xF2],
            pill_asks_text: [0xF2, 0xD4, 0x9A],
            pill_connected: [76, 90, 166],
            row_height: 22.0,
            control_height: 24.0,
            label_row_height: 22.0,
            chip_height: 20.0,
            pill_height: 18.0,
            menu_item_height: 26.0,
            panel_tab_height: 34.0,
            menu_radius: 8,
            menu_width: 200.0,
            check_size: 15.0,
            switch_size: [30.0, 18.0],
            knob_size: 14.0,
            touch_knob: 24.0,
            progress_height: 4.0,
            popover_pointer_width: 12.0,
            icon_stroke: 1.4,
            hit_min: 0.0,
            app_icon_rim_opacity: 56,
            shadow_opacity: 64,
            shadow_blur: 12,
            shadow_drop: 3,
            dialog_title_font: 17.0,
            dialog_padding: 18.0,
            dialog_radius: 8,
            dialog_icon: 16.0,
            header_icon: 46.0,
            about_icon: 44.0,
            card_badge: 22.0,
            dialog_min_width: 160.0,
            dialog_wide_width: 460.0,
            export_dialog_width: 500.0,
            shortcuts_width: 720.0,
            details_width: 560.0,
            details_height: 320.0,
            menu_widths: [250.0, 250.0, 230.0, 240.0],
            log_dot_radius: 4.0,
            log_rail_width: 2.0,
            banner_button_room: 170.0,
            sheet_share: 0.5,
            grabber_size: [36.0, 4.0],
            grabber_opacity: 160,
            half_sheet_share: 0.5,
            docked_panel_width: 300.0,
            docked_min_window: 600.0,
        }
    }

    /// The light appearance: Mission ([`Tokens::mission`]).
    pub fn light() -> Self {
        Self::mission()
    }

    /// The dark appearance: Mission with the dark [`Palette`]. The chrome,
    /// the plates over the viewport and the viewport itself (its plain
    /// look's background, the bodies, sketches, datums and the selection's
    /// copper on them) are the light set's: the viewport is not themed
    /// (the developer's decision, 2026-11-02), so pictures stay the same.
    pub fn dark() -> Self {
        Self::mission().with_palette(Palette::dark())
    }

    /// The tokens of an appearance: dark or light.
    pub fn for_appearance(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }

    /// Whether these tokens have the dark appearance's panels.
    pub fn is_dark(&self) -> bool {
        !is_light(self.panel_fill)
    }

    /// The tokens of the narrow layout: the controls over the viewport and
    /// the side panel's split as large as a finger needs.
    #[must_use]
    pub fn touch(&self) -> Self {
        let mut t = *self;
        t.nav_height = t.nav_height.max(t.touch_target);
        t.tree_split_grip = t.tree_split_grip.max(t.touch_target * 0.5);
        // A finger reaches the caret of a split button: as high as a chip,
        // and as wide as still keeps the camera plate on a phone.
        t.nav_caret_width = t.nav_caret_width.max(t.touch_caret_width);
        // The kit: rows and controls a finger's size, the slider's knob
        // larger, every target at least `touch_target` (2026-11-02: it
        // stays 40, the developer's decision).
        t.row_height = t.row_height.max(t.touch_target);
        t.control_height = t.control_height.max(t.touch_target);
        t.menu_item_height = t.menu_item_height.max(t.touch_target);
        t.knob_size = t.knob_size.max(t.touch_knob);
        t.hit_min = t.hit_min.max(t.touch_target);
        t
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
        let text = egui::FontId::proportional(self.secondary_font);
        ctx.all_styles_mut(move |style| {
            style.visuals = visuals.clone();
            for kind in [egui::TextStyle::Body, egui::TextStyle::Small] {
                style.text_styles.insert(kind, text.clone());
            }
        });
    }

    /// Secondary text on a panel: a hint, a note, a message under a row
    /// ([`Tokens::secondary_font`] in `text_dim`).
    pub fn secondary(&self, text: impl Into<String>) -> egui::RichText {
        egui::RichText::new(text)
            .size(self.secondary_font)
            .color(color(self.text_dim))
    }

    /// Secondary text on a plate over the viewport (a popover):
    /// [`Tokens::secondary_font`] in `plate_text_dim`.
    pub fn plate_secondary(&self, text: impl Into<String>) -> egui::RichText {
        egui::RichText::new(text)
            .size(self.secondary_font)
            .color(color(self.plate_text_dim))
    }

    /// Secondary text in another colour (an error's red, an assertion's
    /// state) at the secondary size.
    pub fn secondary_in(&self, text: impl Into<String>, ink: Rgb) -> egui::RichText {
        egui::RichText::new(text)
            .size(self.secondary_font)
            .color(color(ink))
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
        // egui's default blue clashes with the warm greys: links and a
        // text field's focus ring take the info ink.
        visuals.hyperlink_color = color(self.info_ink);
        visuals.selection.stroke = egui::Stroke::new(1.0, color(self.info_ink));
        // Every egui widget still drawn on a panel (a scroll bar, a
        // separator, a text edit) in the kit's fills, never egui's greys.
        visuals.extreme_bg_color = color(self.field_fill);
        visuals.text_edit_bg_color = Some(color(self.field_fill));
        visuals.faint_bg_color = color(self.row_hover_fill);
        visuals.code_bg_color = color(self.field_fill);
        visuals.window_stroke = egui::Stroke::new(1.0, color(self.divider));
        visuals.widgets.noninteractive.bg_fill = color(self.panel_fill);
        visuals.widgets.noninteractive.weak_bg_fill = color(self.panel_fill);
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, color(self.divider));
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, color(self.text));
        for (widget, fill, stroke, ink) in [
            (
                &mut visuals.widgets.inactive,
                self.field_fill,
                self.field_stroke,
                self.text,
            ),
            (
                &mut visuals.widgets.hovered,
                self.row_hover_fill,
                self.field_stroke,
                self.text,
            ),
            (
                &mut visuals.widgets.active,
                self.chip_fill,
                self.field_stroke,
                self.text,
            ),
            (
                &mut visuals.widgets.open,
                self.chip_fill,
                self.field_stroke,
                self.text,
            ),
        ] {
            widget.bg_fill = color(fill);
            widget.weak_bg_fill = color(fill);
            widget.bg_stroke = egui::Stroke::new(1.0, color(stroke));
            widget.fg_stroke = egui::Stroke::new(1.0, color(ink));
        }
        self.round(&mut visuals);
        visuals
    }

    /// The visuals of what sits on a plate over the viewport (the Studio
    /// and Analysis popovers): egui's dark ones in the plates' colours, so
    /// a popover reads as part of the camera plate it hangs from: text in
    /// `plate_text`, a chosen option filled with the accent as a chosen
    /// segment is, widgets lit with the glass tint.
    pub fn plate_visuals(&self) -> egui::Visuals {
        let mut visuals = if is_light(self.plate_fill) {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        visuals.panel_fill = color(self.plate_fill);
        visuals.window_fill = color(self.plate_fill);
        visuals.override_text_color = Some(color(self.plate_text));
        visuals.hyperlink_color = color(self.info_code);
        visuals.selection.bg_fill = color_alpha(self.accent, self.nav_on_opacity);
        visuals.selection.stroke = egui::Stroke::new(self.divider_width, color(self.knob));
        // Buttons and chosen-not options lit with the glass tint; a
        // slider's rail and knob and a check box's square in the dim text
        // colour, so they read on the dark plate.
        // Each step under the pointer and pressed adds the glass's own
        // step (`control_opacity`), as a button on a plate lights up.
        let rest = self.popover_widget_opacity;
        let hover = rest.saturating_add(self.control_opacity);
        let press = hover.saturating_add(self.control_opacity);
        let glass = |steps: u8| color_alpha(self.glass, self.control_opacity.saturating_mul(steps));
        visuals.widgets.inactive.weak_bg_fill = glass(1);
        visuals.widgets.inactive.bg_fill = color_alpha(self.plate_text_dim, rest);
        visuals.widgets.hovered.weak_bg_fill = glass(2);
        visuals.widgets.hovered.bg_fill = color_alpha(self.plate_text_dim, hover);
        visuals.widgets.active.weak_bg_fill = glass(3);
        visuals.widgets.active.bg_fill = color_alpha(self.plate_text, press);
        visuals.extreme_bg_color = color_alpha(self.plate_text_dim, rest);
        self.round(&mut visuals);
        visuals
    }

    /// The menus that drop from the chrome (board s4): the kit's plate,
    /// the popover's fill with its faint rim and shadow, `menu_radius`
    /// round, dark in both appearances.
    pub fn menu_visuals(&self) -> egui::Visuals {
        let mut visuals = self.plate_visuals();
        visuals.window_fill = color(self.popover_fill);
        visuals.panel_fill = color(self.popover_fill);
        visuals.window_stroke =
            egui::Stroke::new(1.0, color_alpha(self.plate_rim, self.popover_rim_opacity));
        visuals.menu_corner_radius = egui::CornerRadius::same(self.menu_radius);
        visuals.popup_shadow = egui::Shadow {
            offset: [0, self.shadow_drop],
            blur: self.shadow_blur,
            spread: 0,
            color: color_alpha(self.view_cube_shadow, self.shadow_opacity),
        };
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
        visuals.hyperlink_color = color(self.info_code);
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
            ((c + 0.055) / 1.055).ppowf(2.4)
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
            ("info_ink on panel_fill", self.info_ink, self.panel_fill),
            (
                "info_ink on selection_fill",
                self.info_ink,
                self.selection_fill,
            ),
            ("info_code on plate_fill", self.info_code, self.plate_fill),
            ("info_code on menu_fill", self.info_code, self.menu_fill),
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
            ("nav_off_text on a plate", self.nav_off_text, plate),
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
                "viewport_text on viewport_background",
                self.viewport_text,
                self.viewport_background,
            ),
            (
                "viewport_failed (a problem) on viewport_background",
                self.viewport_failed,
                self.viewport_background,
            ),
            (
                "datum_ink (a datum's name) on viewport_background",
                self.datum_ink,
                self.viewport_background,
            ),
        ]
    }
}

impl Tokens {
    /// The kit's pairs of ink and ground (docs/design/rendering.md › *The
    /// component kit*), each with the least contrast it must keep: 4.5 for
    /// text, 3 for a part of a control or an icon (WCAG 2). Both
    /// appearances are held to them (`theme::tests`).
    pub fn kit_pairs(&self) -> Vec<(&'static str, Rgb, Rgb, f64)> {
        const TEXT: f64 = 4.5;
        const PART: f64 = 3.0;
        let mut pairs = vec![
            // Panels and dialogs.
            (
                "text on row_hover_fill",
                self.text,
                self.row_hover_fill,
                TEXT,
            ),
            (
                "text_dim on row_hover_fill",
                self.text_dim,
                self.row_hover_fill,
                TEXT,
            ),
            ("text on field_fill", self.text, self.field_fill, TEXT),
            (
                "text_dim on field_fill",
                self.text_dim,
                self.field_fill,
                TEXT,
            ),
            ("text on chip_fill", self.text, self.chip_fill, TEXT),
            ("text_dim on chip_fill", self.text_dim, self.chip_fill, TEXT),
            ("info_ink on info_fill", self.info_ink, self.info_fill, TEXT),
            ("text on info_fill", self.text, self.info_fill, TEXT),
            ("failed on failed_fill", self.failed, self.failed_fill, TEXT),
            ("text on failed_fill", self.text, self.failed_fill, TEXT),
            (
                "text on attention_fill",
                self.text,
                self.attention_fill,
                TEXT,
            ),
            (
                "failed on field_fill (a danger button)",
                self.failed,
                self.field_fill,
                TEXT,
            ),
            (
                "info_ink on field_fill",
                self.info_ink,
                self.field_fill,
                TEXT,
            ),
            ("knob on accent", self.knob, self.accent, TEXT),
            // Popovers and menus (dark in both appearances).
            (
                "plate_text on popover_fill",
                self.plate_text,
                self.popover_fill,
                TEXT,
            ),
            (
                "plate_text_dim on popover_fill",
                self.plate_text_dim,
                self.popover_fill,
                TEXT,
            ),
            (
                "info_code on popover_fill",
                self.info_code,
                self.popover_fill,
                TEXT,
            ),
            (
                "chrome_failed on popover_fill",
                self.chrome_failed,
                self.popover_fill,
                TEXT,
            ),
            (
                "plate_ok on popover_fill",
                self.plate_ok,
                self.popover_fill,
                TEXT,
            ),
            (
                "plate_text on popover_hover",
                self.plate_text,
                self.popover_hover,
                TEXT,
            ),
            (
                "plate_text_dim on popover_hover",
                self.plate_text_dim,
                self.popover_hover,
                TEXT,
            ),
            (
                "plate_text on popover_field",
                self.plate_text,
                self.popover_field,
                TEXT,
            ),
            (
                "plate_text_dim on popover_field",
                self.plate_text_dim,
                self.popover_field,
                TEXT,
            ),
            // The plates over the viewport.
            (
                "plate_ok on plate_fill",
                self.plate_ok,
                self.plate_fill,
                TEXT,
            ),
            (
                "plate_text_dim on plate_field",
                self.plate_text_dim,
                self.plate_field,
                TEXT,
            ),
            (
                "plate_text on plate_selection_fill",
                self.plate_text,
                self.plate_selection_fill,
                TEXT,
            ),
            (
                "plate_text_dim on tab_active_fill",
                self.plate_text_dim,
                self.tab_active_fill,
                TEXT,
            ),
            (
                "info_code on plate_info_fill",
                self.info_code,
                self.plate_info_fill,
                TEXT,
            ),
            (
                "chrome_failed on plate_failed_fill",
                self.chrome_failed,
                self.plate_failed_fill,
                TEXT,
            ),
            (
                "plate_text on plate_failed_fill",
                self.plate_text,
                self.plate_failed_fill,
                TEXT,
            ),
            (
                "plate_text on plate_attention_fill",
                self.plate_text,
                self.plate_attention_fill,
                TEXT,
            ),
            // The agent's pill on the status bar.
            (
                "chrome_text on pill_connected",
                self.chrome_text,
                self.pill_connected,
                TEXT,
            ),
            (
                "pill_wait_text on status_fill",
                self.pill_wait_text,
                self.status_fill,
                TEXT,
            ),
            (
                "pill_asks_text on plate_attention_fill",
                self.pill_asks_text,
                self.plate_attention_fill,
                TEXT,
            ),
            (
                "chrome_text_dim on status_fill",
                self.chrome_text_dim,
                self.status_fill,
                TEXT,
            ),
            // Parts and icons.
            (
                "info_accent on panel_fill",
                self.info_accent,
                self.panel_fill,
                PART,
            ),
            (
                "attention on attention_fill (an icon)",
                self.attention,
                self.attention_fill,
                PART,
            ),
            (
                "icon_sketch on panel_fill",
                self.icon_sketch,
                self.panel_fill,
                PART,
            ),
            (
                "icon_datum on panel_fill",
                self.icon_datum,
                self.panel_fill,
                PART,
            ),
            (
                "text (a selected row's icon) on selection_fill",
                self.text,
                self.selection_fill,
                PART,
            ),
            (
                "info_code on plate_fill (a focus ring)",
                self.info_code,
                self.plate_fill,
                PART,
            ),
            ("accent on plate_fill", self.accent, self.plate_fill, PART),
            (
                "plate_info_accent on popover_fill",
                self.plate_info_accent,
                self.popover_fill,
                PART,
            ),
            (
                "plate_datum on popover_fill",
                self.plate_datum,
                self.popover_fill,
                PART,
            ),
            (
                "info_code (a sketch's icon) on popover_fill",
                self.info_code,
                self.popover_fill,
                PART,
            ),
        ];
        // On a dark panel the amber reads as a part by itself; on the light
        // one it always carries the knob's ink (a filled button, a switch).
        if self.is_dark() {
            pairs.push(("accent on panel_fill", self.accent, self.panel_fill, PART));
        }
        pairs
    }
}

impl Tokens {
    /// The colours of a body's edges, from the body's colour: the edges
    /// between two of its faces, and its outline (see
    /// [`Tokens::edge_shade`]). Every channel is rounded once.
    pub fn edge_colors(&self, body: Rgb) -> EdgeColors {
        let scaled = |factor: f32| {
            let factor = if factor.is_finite() {
                f64::from(factor).clamp(0.0, 1.0)
            } else {
                1.0
            };
            body.map(|c| (f64::from(c) * factor).round().clamp(0.0, 255.0) as u8)
        };
        let darker = scaled(self.edge_shade);
        let inner = if contrast(darker, body) >= f64::from(self.edge_min_contrast) {
            darker
        } else {
            let lift = if self.edge_lift.is_finite() {
                f64::from(self.edge_lift).clamp(0.0, 1.0)
            } else {
                0.0
            };
            body.map(|c| {
                let c = f64::from(c);
                (c + (255.0 - c) * lift).round().clamp(0.0, 255.0) as u8
            })
        };
        EdgeColors {
            inner,
            silhouette: scaled(self.edge_silhouette_shade),
        }
    }
}

/// The colours of a body's edges ([`Tokens::edge_colors`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeColors {
    /// An edge between two faces of the body.
    pub inner: Rgb,
    /// The body's outline against what lies behind it.
    pub silhouette: Rgb,
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
        for (set, tokens) in [("light", Tokens::light()), ("dark", Tokens::dark())] {
            let mut low = Vec::new();
            for (what, ink, fill) in tokens.text_pairs() {
                let ratio = contrast(ink, fill);
                println!("{set}: {what}: {ratio:.2}");
                if ratio < 4.5 {
                    low.push(format!("{what} {ratio:.2}"));
                }
            }
            assert!(low.is_empty(), "{set}: {low:?}");
        }
    }

    /// Every pair of the kit, in both appearances: text 4.5:1, parts and
    /// icons 3:1.
    #[test]
    fn every_pair_of_the_kit_keeps_its_contrast_in_both_appearances() {
        for (set, tokens) in [("light", Tokens::light()), ("dark", Tokens::dark())] {
            let pairs = tokens.kit_pairs();
            assert!(pairs.len() > 30, "{set}: {}", pairs.len());
            let mut low = Vec::new();
            for (what, ink, fill, least) in pairs {
                let ratio = contrast(ink, fill);
                println!("{set}: {what}: {ratio:.2} (at least {least})");
                if ratio < least {
                    low.push(format!("{what} {ratio:.2} < {least}"));
                }
            }
            assert!(low.is_empty(), "{set}: {low:?}");
        }
    }

    /// The figures of the overhaul's plan (plan/mockups/ui-overhaul,
    /// *Contrast*), the design's own check of the dark set. Changed on
    /// 2026-11-04 with the developer's decision that dark is the darkest
    /// variant (panels #161514, popovers #201F1D): the ratios of the
    /// panel, its hover and the popover were worked out again by hand
    /// (WCAG 2 relative luminance, a script outside this code) and stand
    /// in the design notes' table (docs/design/rendering.md).
    #[test]
    fn the_dark_set_has_the_planned_figures() {
        let d = Tokens::dark();
        for (ink, fill, ratio) in [
            (d.text, d.panel_fill, 15.19),
            (d.text_dim, d.panel_fill, 6.85),
            (d.text_dim, d.row_hover_fill, 6.04),
            (d.text, d.selection_fill, 7.76),
            (d.info_ink, d.info_fill, 6.91),
            (d.failed, d.failed_fill, 4.82),
            (d.attention, d.attention_fill, 7.11),
            (d.plate_text_dim, d.popover_fill, 6.57),
        ] {
            let got = contrast(ink, fill);
            assert!((got - ratio).abs() < 0.01, "{ink:?} on {fill:?}: {got:.2}");
        }
        let l = Tokens::light();
        assert!((contrast(l.failed, l.failed_fill) - 4.67).abs() < 0.01);
        assert!((contrast(l.info_ink, l.info_fill) - 6.82).abs() < 0.01);
    }

    /// The dark set changes the panels and nothing that is drawn on the
    /// viewport, on the chrome or on the plates: pictures stay the same in
    /// both appearances.
    #[test]
    fn the_dark_set_leaves_the_viewport_the_chrome_and_the_plates() {
        let (l, d) = (Tokens::light(), Tokens::dark());
        assert!(d.is_dark() && !l.is_dark());
        assert_ne!(l.panel_fill, d.panel_fill);
        assert_ne!(l.popover_fill, d.popover_fill);
        for (what, a, b) in [
            (
                "viewport_background",
                l.viewport_background,
                d.viewport_background,
            ),
            ("body", l.body, d.body),
            ("face_highlight", l.face_highlight, d.face_highlight),
            ("sketch", l.sketch, d.sketch),
            ("datum", l.datum, d.datum),
            ("datum_ink", l.datum_ink, d.datum_ink),
            ("viewport_text", l.viewport_text, d.viewport_text),
            ("viewport_failed", l.viewport_failed, d.viewport_failed),
            ("chrome_work", l.chrome_work, d.chrome_work),
            ("chrome_attention", l.chrome_attention, d.chrome_attention),
            ("menu_fill", l.menu_fill, d.menu_fill),
            ("status_fill", l.status_fill, d.status_fill),
            ("plate_fill", l.plate_fill, d.plate_fill),
            ("plate_text", l.plate_text, d.plate_text),
            ("accent", l.accent, d.accent),
            ("pill_connected", l.pill_connected, d.pill_connected),
            ("view_cube_label", l.view_cube_label, d.view_cube_label),
        ] {
            assert_eq!(a, b, "{what}");
        }
        // A palette round-trips, and touch sizes keep it.
        assert_eq!(Tokens::light().with_palette(Palette::dark()), d);
        assert_eq!(d.touch().palette(), Palette::dark());
        assert_eq!(Palette::light(), l.palette());
        assert_eq!(Tokens::for_appearance(true), d);
        // The viewport's and the chrome's own inks are the light set's.
        assert_eq!(l.viewport_text, Palette::light().text_dim);
        assert_eq!(l.chrome_work, Palette::light().info_accent);
        assert_eq!(l.chrome_attention, Palette::light().attention);
    }

    /// Touch sizes make every kit target a finger's size.
    #[test]
    fn touch_sizes_raise_the_kit_targets() {
        let t = Tokens::default().touch();
        assert_eq!(t.touch_target, 40.0);
        assert!(t.row_height >= t.touch_target && t.control_height >= t.touch_target);
        assert!(t.hit_min >= t.touch_target && t.knob_size >= 24.0);
        assert_eq!(Tokens::default().hit_min, 0.0);
    }

    /// Secondary text is one legible size, the body text's: egui's body
    /// and small text styles are both that token once it is applied.
    #[test]
    fn secondary_text_is_one_legible_size() {
        let tokens = Tokens::default();
        let ctx = egui::Context::default();
        tokens.apply(&ctx);
        let style = ctx.style_of(ctx.theme());
        let body = style.text_styles[&egui::TextStyle::Body].size;
        let small = style.text_styles[&egui::TextStyle::Small].size;
        assert_eq!(small, tokens.secondary_font);
        assert_eq!(body, tokens.secondary_font, "{small} {body}");
        for tokens in [Tokens::default(), Tokens::default().touch()] {
            assert!(tokens.secondary_font >= 12.0);
        }
    }

    /// An edge contrasts with its own body by the stated ratio or more,
    /// whatever the body's colour: darker than the body on a light or an
    /// ordinary one (the pier's red, the rover's white), lighter on a very
    /// dark one (black wheels); the outline is never lighter than the
    /// edges between faces on the darker side.
    #[test]
    fn an_edge_takes_its_colour_from_its_body_and_contrasts_with_it() {
        let tokens = Tokens::default();
        let ratio = f64::from(tokens.edge_min_contrast);
        assert!(ratio >= 1.5);
        let pier = [156, 47, 37];
        let rover = [233, 230, 223];
        let wheel = [29, 29, 32];
        for body in [pier, rover, tokens.body, [91, 42, 34], [58, 58, 60]] {
            let edges = tokens.edge_colors(body);
            assert!(luminance(edges.inner) < luminance(body), "{body:?}");
            assert!(luminance(edges.silhouette) <= luminance(edges.inner));
            assert!(contrast(edges.inner, body) >= ratio, "{body:?}");
        }
        // The hue is kept: the pier's red edge is red.
        let red = tokens.edge_colors(pier).inner;
        assert_eq!(red, [62, 19, 15]);
        let edges = tokens.edge_colors(wheel);
        assert!(luminance(edges.inner) > luminance(wheel));
        assert!(contrast(edges.inner, wheel) >= ratio);
        // Every grey from black to white: the rule never gives less.
        for v in 0..=255_u8 {
            let body = [v, v, v];
            let edges = tokens.edge_colors(body);
            assert!(
                contrast(edges.inner, body) >= ratio,
                "grey {v}: {:?}",
                edges.inner
            );
        }
        // Hostile factors keep the colour within bytes.
        let mut odd = tokens;
        odd.edge_shade = f32::NAN;
        odd.edge_lift = -3.0;
        odd.edge_silhouette_shade = 9.0;
        let _ = odd.edge_colors(pier);
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
        // Links in the info accent, not egui's default blue.
        assert_eq!(panels.hyperlink_color, color(tokens.info_ink));
        assert_eq!(chrome.hyperlink_color, color(tokens.info_code));
    }

    /// The info accent is a part of a control (a ring, a bar) on the
    /// panels and the viewport: 3:1 or better, and cool, so it is told
    /// apart from the warm action accent.
    #[test]
    fn the_info_accent_reads_as_a_part_and_is_cool() {
        let tokens = Tokens::default();
        for fill in [tokens.panel_fill, tokens.viewport_background] {
            assert!(contrast(tokens.info_accent, fill) >= 3.0, "{fill:?}");
        }
        let [r, _, b] = tokens.info_accent;
        assert!(b > r);
        let [r, _, b] = tokens.accent;
        assert!(r > b);
    }

    /// A trace is the line a person follows over the whole picture while
    /// the motion plays: 3:1 against the plain look's background and
    /// against the studio look's backdrop, whose greys run from about
    /// [130, 130, 128] to [180, 177, 175] (measured on the default studio
    /// environment's picture of the strandbeest leg, 2026-10-30, corners
    /// and middle), and wider than a sketch's line.
    #[test]
    fn a_trace_stands_out_on_both_looks() {
        let tokens = Tokens::default();
        for bg in [
            tokens.viewport_background,
            [130, 130, 128],
            [150, 148, 148],
            [180, 177, 175],
            tokens.body,
        ] {
            let ratio = contrast(tokens.trace, bg);
            assert!(ratio >= 3.0, "trace on {bg:?}: {ratio:.2}");
        }
        assert!(
            tokens.trace_width > tokens.sketch_width,
            "{}",
            tokens.trace_width
        );
    }

    /// Sketches and datums are lines and dots over the viewport and over a
    /// lit body: a profile and a datum read as parts of a picture (3:1 on
    /// the viewport); the selected sketch, in the selection's copper, keeps
    /// 2.3:1 there (it is 2.34:1; the developer's choice of colour, under
    /// review); construction geometry stands back but is still seen (2:1). On the body's colour a profile and a
    /// datum keep 3:1. The four are told apart: a profile is cool (the
    /// info accent), the selected sketch warm (the selection), a datum
    /// green-blue, and construction lighter than a profile of its hue.
    #[test]
    fn sketches_and_datums_read_over_the_viewport_and_a_body() {
        let tokens = Tokens::default();
        let bg = tokens.viewport_background;
        for (what, ink, least) in [
            ("sketch", tokens.sketch, 3.0),
            ("sketch_selected", tokens.sketch_selected, 2.3),
            ("datum", tokens.datum, 3.0),
            ("sketch_construction", tokens.sketch_construction, 2.0),
        ] {
            let ratio = contrast(ink, bg);
            assert!(ratio >= least, "{what} on the viewport: {ratio:.2}");
        }
        for ink in [tokens.sketch, tokens.datum, tokens.trace] {
            assert!(contrast(ink, tokens.body) >= 3.0, "{ink:?}");
        }
        assert!(contrast(tokens.trace, bg) >= 3.0);
        let [r, g, b] = tokens.trace.map(i32::from);
        assert!(r > g && b > g);
        assert_eq!(tokens.sketch, tokens.info_accent);
        assert_eq!(tokens.sketch_selected, tokens.face_highlight);
        let [r, g, b] = tokens.sketch.map(i32::from);
        assert!(b > r && b > g);
        let [r, _, b] = tokens.sketch_selected.map(i32::from);
        assert!(r > b);
        let [r, g, b] = tokens.datum.map(i32::from);
        assert!(g > r && b > r);
        assert!(luminance(tokens.sketch_construction) > luminance(tokens.sketch));
        // The datum's name is its line's hue, darker.
        assert!(luminance(tokens.datum_ink) < luminance(tokens.datum));
    }
}
