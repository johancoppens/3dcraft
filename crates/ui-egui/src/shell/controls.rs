//! The controls over the viewport, in its bottom-right corner, in three
//! groups, each on a plate of its own:
//!
//! - **Camera**, in the corner: the view cube with the axis triad beside
//!   it at its lower left, under the cube *Fit*, *iso* and the projection, *Persp* | *Ortho*.
//! - **Display**, to its left on the bottom row: *Sketches*, *Datums* and
//!   the *See-through* slider.
//! - **Section**, above Display: the *Section* toggle and, while it is on,
//!   *X*, *Y*, *Z* (the axis the plane is across), the slider that moves
//!   the plane and *Flip*.
//!
//! In a viewport too narrow for the groups beside the camera they go under
//! it, and in one too small for the cube the cube is left out (its buttons
//! stay). Everything here is view state that `ui.set` sets too
//! (`camera`, `view`, `show`, `transparency`, `section`); the widgets only
//! read and write [`crate::state::UiState`]. They are widgets of their own
//! over the viewport, so a click on them is not a click in the viewport
//! and picks nothing. Sizes, spacing and colours come from the tokens.

use super::{Shell, Turn};
use crate::state::ProjectionKind;
use crate::theme::{Tokens, color, color_alpha};
use crate::view_cube;
use craft3d_engine::Scene;
use egui::{Rect, pos2, vec2};

/// How wide each row of controls is, in points: measured every frame by
/// laying the row out once unseen (egui's sizing pass), so the layout is
/// right in the frame it is drawn in, whatever the fonts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowWidths {
    pub camera: f32,
    pub display: f32,
    pub section: f32,
}

/// Where the controls go in a viewport whose usable area (inside the
/// inset) is `inner`. The rows are the rectangles the controls are laid
/// out in, from their left end, as wide as `widths` says; a part that
/// does not fit is `None`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlsLayout {
    /// Fit, iso and the projection.
    pub camera_row: Option<Rect>,
    /// Sketches, Datums, See-through.
    pub display_row: Option<Rect>,
    /// The section's toggle and, while it is on, its other controls.
    pub section_row: Option<Rect>,
    /// The view cube's square place and the triad's beside it.
    pub cube: Option<Rect>,
    pub triad: Option<Rect>,
}

impl ControlsLayout {
    /// Every part that is shown.
    pub fn parts(&self) -> Vec<(&'static str, Rect)> {
        [
            ("camera", self.camera_row),
            ("display", self.display_row),
            ("section", self.section_row),
            ("cube", self.cube),
            ("triad", self.triad),
        ]
        .into_iter()
        .filter_map(|(name, rect)| Some((name, rect?)))
        .collect()
    }
}

/// The layout of the controls in `inner` (see the top of this module).
/// The rows of Display and Section start at the same left edge, so the
/// Section toggle stays where it is when the controls it shows when on
/// appear to its right.
pub fn controls_layout(inner: Rect, tokens: &Tokens, widths: &RowWidths) -> ControlsLayout {
    let row = tokens.bar_height;
    let pitch = row + tokens.bar_gap;
    let groups = widths.display.max(widths.section);
    let camera_width = widths.camera.max(tokens.view_cube_size);
    let bottom = inner.bottom();
    // Beside the camera group when there is room; under it otherwise,
    // and left out when they do not fit the width at all.
    let groups_fit = groups <= inner.width();
    let beside = inner.width() >= groups + tokens.group_gap + camera_width;
    let (left, camera_bottom) = if beside || !groups_fit {
        (
            inner.right() - camera_width - tokens.group_gap - groups,
            bottom,
        )
    } else {
        (
            (inner.right() - groups).max(inner.left()),
            bottom - 2.0 * pitch,
        )
    };
    let fits = |rect: Rect| rect.top() >= inner.top() && rect.left() >= inner.left() - 0.5;
    let display_row = Rect::from_min_size(pos2(left, bottom - row), vec2(widths.display, row));
    let section_row =
        Rect::from_min_size(pos2(left, bottom - row - pitch), vec2(widths.section, row));
    let middle = inner.right() - camera_width.min(inner.width()) * 0.5;
    let camera_row = Rect::from_min_size(
        pos2(middle - widths.camera * 0.5, camera_bottom - row),
        vec2(widths.camera, row),
    );
    let camera_row = Some(camera_row).filter(|&r| fits(r) && widths.camera <= inner.width());
    // The cube over the camera's row (or where it would be).
    let floor = camera_row.map_or(camera_bottom - row, |r| r.top()) - tokens.bar_gap;
    let height = floor - inner.top();
    let wide = camera_width.min(inner.width());
    let side = tokens.view_cube_size.min(wide).min(height);
    let cube = (side >= tokens.view_cube_min && side.is_finite())
        .then(|| Rect::from_min_size(pos2(middle - side * 0.5, floor - side), vec2(side, side)));
    let display_row = Some(display_row).filter(|&r| groups_fit && fits(r));
    let section_row = Some(section_row).filter(|&r| groups_fit && fits(r));
    // The triad beside the cube at its lower left, raised above a row of
    // controls it would meet; left out where it has no room beside the
    // cube.
    let triad = cube.and_then(|cube| {
        let size = tokens.triad_size;
        let mut place = Rect::from_min_size(
            pos2(cube.left() - size, cube.bottom() - size),
            vec2(size, size),
        );
        for row in [display_row, section_row].into_iter().flatten() {
            let plate = row.expand2(vec2(tokens.plate_padding, tokens.bar_gap * 0.5));
            if place.intersects(plate) {
                place = place.translate(vec2(0.0, plate.top() - place.bottom()));
            }
        }
        (place.left() >= inner.left() - 0.5
            && place.top() >= cube.top() - 0.5
            && place.top() >= inner.top()
            && place.min.is_finite()
            && place.max.is_finite())
        .then_some(place)
    });
    ControlsLayout {
        camera_row,
        display_row,
        section_row,
        cube,
        triad,
    }
}

/// The patches of the view cube that `camera` sees, as they are painted
/// in a place whose middle is `center` with a picture radius of `arm`
/// points, with the proportions of `style`: each with its corners in the
/// window, the same corner never twice in a row. A patch thinner than
/// [`MIN_PATCH`] is left out: it shows nothing, and egui's anti-aliasing
/// of a sliver that thin draws spikes far outside it.
pub fn cube_patches(
    camera: &crate::state::CameraState,
    style: &view_cube::CubeStyle,
    center: egui::Pos2,
    arm: f32,
) -> Vec<(view_cube::Facet, Vec<egui::Pos2>)> {
    view_cube::facets_styled(camera, style)
        .into_iter()
        .filter_map(|facet| {
            let mut points: Vec<egui::Pos2> = Vec::with_capacity(facet.patch.len());
            for &p in &facet.patch {
                let q = center + vec2(p[0] as f32, p[1] as f32) * arm;
                if points.last().is_none_or(|last| last.distance(q) > 1e-3) {
                    points.push(q);
                }
            }
            while points.len() > 1
                && points
                    .first()
                    .zip(points.last())
                    .is_some_and(|(a, b)| a.distance(*b) <= 1e-3)
            {
                points.pop();
            }
            (points.len() >= 3 && thickness(&points) >= MIN_PATCH).then_some((facet, points))
        })
        .collect()
}

/// The thinnest a painted patch of the view cube is, in points.
pub const MIN_PATCH: f32 = 3.0;

/// How thick a convex polygon is: twice its area over its perimeter
/// (its inradius for a regular one), in its own units.
fn thickness(points: &[egui::Pos2]) -> f32 {
    let n = points.len();
    let (mut area, mut perimeter) = (0.0_f32, 0.0_f32);
    for k in 0..n {
        let (Some(a), Some(b)) = (points.get(k), points.get((k + 1) % n)) else {
            continue;
        };
        area += a.x * b.y - b.x * a.y;
        perimeter += a.distance(*b);
    }
    let t = area.abs() / perimeter.max(f32::MIN_POSITIVE);
    if t.is_finite() { t } else { 0.0 }
}

/// How a patch of the view cube is filled at rest: glass, a face lit by
/// how it is turned to the theme's light, a capsule or a circle quieter
/// and fainter as it turns away.
pub fn cube_fill(tokens: &Tokens, facet: &view_cube::Facet) -> egui::Color32 {
    match facet.kind() {
        view_cube::FacetKind::Face => {
            let [x, y, z] = tokens.view_cube_light.map(f64::from);
            let length = (x * x + y * y + z * z).sqrt();
            let n = facet.normal();
            let lit = if length > 0.0 && length.is_finite() {
                ((n[0] * x + n[1] * y + n[2] * z) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let [lo, hi] = tokens.view_cube_face_opacity.map(f64::from);
            // Between two opacities.
            let alpha = (lo + (hi - lo) * lit).round().clamp(0.0, 255.0) as u8;
            color_alpha(tokens.view_cube_glass, alpha)
        }
        kind => {
            let full = if kind == view_cube::FacetKind::Edge {
                tokens.view_cube_edge_opacity
            } else {
                tokens.view_cube_corner_opacity
            };
            let t = (facet.toward * 1.6).clamp(0.0, 1.0);
            // A fraction of an opacity.
            let alpha = (f64::from(full) * t).round().clamp(0.0, 255.0) as u8;
            color_alpha(tokens.view_cube_glass_dim, alpha)
        }
    }
}

/// The name of a face of the view cube as a mesh lying in the face's
/// plane: the text laid out by egui at the size it has on the face seen
/// straight on, its glyphs moved by the face's affine map, so that it is
/// foreshortened and sheared with the face and drawn from the same glyph
/// images as any text (crisp at any pixels per point). `center` and `arm`
/// place the picture of the cube. Nothing for a face seen too nearly
/// edge-on, or a name too small to read.
pub fn cube_label(
    painter: &egui::Painter,
    tokens: &Tokens,
    facet: &view_cube::Facet,
    center: egui::Pos2,
    arm: f32,
) -> Option<egui::Mesh> {
    let frame = facet.label_frame?;
    let name = facet.label()?.to_uppercase();
    let opacity = view_cube::label_opacity(facet.toward, f64::from(tokens.view_cube_label_hide));
    if opacity <= 0.0 || !(frame.half_side > 0.0) {
        return None;
    }
    // Points per half side of the cube.
    let unit = f64::from(arm) * frame.half_side;
    let style = view_cube::style(tokens);
    let flat = 2.0 * (1.0 - style.chamfer.clamp(0.0, 0.9));
    let ink = color_alpha(tokens.view_cube_label, (255.0 * opacity).round() as u8);
    let layout = |size: f64| {
        painter.layout_no_wrap(name.clone(), egui::FontId::proportional(size as f32), ink)
    };
    let mut size = f64::from(tokens.view_cube_label_size) * unit;
    let mut galley = layout(size);
    let widest = f64::from(tokens.view_cube_label_width) * flat * unit;
    let width = f64::from(galley.size().x);
    if width > widest && width > 0.0 {
        size *= widest / width;
        galley = layout(size);
    }
    if !(size.is_finite() && size >= f64::from(tokens.view_cube_label_min)) {
        return None;
    }
    // The ink of the glyphs, to centre on the face's middle.
    let ink_box = galley
        .rows
        .iter()
        .map(|row| row.visuals.mesh_bounds.translate(row.pos.to_vec2()))
        .reduce(|a, b| a.union(b))?;
    let [tw, th] = painter.fonts(|f| f.font_image_size()).map(|n| n as f32);
    if !(tw > 0.0 && th > 0.0) {
        return None;
    }
    // From a point of the text (in points, from the middle of its ink) to
    // the window: along the face, as the picture shows it.
    let per = 1.0 / frame.half_side;
    let (right, down) = (
        vec2((frame.right[0] * per) as f32, (frame.right[1] * per) as f32),
        vec2((frame.down[0] * per) as f32, (frame.down[1] * per) as f32),
    );
    let mut origin = center + vec2(frame.center[0] as f32, frame.center[1] as f32) * arm;
    // Seen straight on the name is drawn pixel for pixel: snapped to the
    // pixel grid, as egui's own text is.
    let ppp = painter.pixels_per_point();
    if (right.x - 1.0).abs() < 1e-3 && (down.y - 1.0).abs() < 1e-3 && ppp > 0.0 {
        let shift = ink_box.center().to_vec2();
        let corner = origin - shift;
        let snapped = egui::pos2(
            (corner.x * ppp).round() / ppp,
            (corner.y * ppp).round() / ppp,
        );
        origin = snapped + shift;
    }
    let mid = ink_box.center();
    let mut mesh = egui::Mesh::with_texture(egui::TextureId::default());
    for row in &galley.rows {
        let base = mesh.vertices.len() as u32;
        let offset = row.pos.to_vec2();
        for v in &row.visuals.mesh.vertices {
            let local = v.pos + offset - mid;
            let at = origin + right * local.x + down * local.y;
            if !(at.x.is_finite() && at.y.is_finite()) {
                return None;
            }
            mesh.vertices.push(egui::epaint::Vertex {
                pos: at,
                uv: egui::pos2(v.uv.x / tw, v.uv.y / th),
                color: ink,
            });
        }
        mesh.indices
            .extend(row.visuals.mesh.indices.iter().map(|i| i + base));
    }
    (!mesh.is_empty()).then_some(mesh)
}

/// The soft shadow under the view cube, whose picture has its middle at
/// `center` and a radius of `arm` points: an ellipse that fades from the
/// middle to nothing at its rim.
fn cube_shadow(tokens: &Tokens, center: egui::Pos2, arm: f32) -> egui::Mesh {
    let [drop, half_width, half_height] = tokens.view_cube_shadow_size;
    let middle = center + vec2(0.0, drop * arm);
    let (rx, ry) = (half_width * arm, half_height * arm);
    let full = tokens.view_cube_shadow_opacity;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(middle, color_alpha(tokens.view_cube_shadow, full));
    const AROUND: u32 = 48;
    // Two rings: half way out at half the opacity, the rim at none.
    for (reach, alpha) in [(0.5_f32, full / 2), (1.0, 0)] {
        for j in 0..AROUND {
            let angle = std::f32::consts::TAU * j as f32 / AROUND as f32;
            mesh.colored_vertex(
                middle + vec2(rx * reach * angle.cos(), ry * reach * angle.sin()),
                color_alpha(tokens.view_cube_shadow, alpha),
            );
        }
    }
    for j in 0..AROUND {
        let next = (j + 1) % AROUND;
        let (a, b) = (1 + j, 1 + next);
        mesh.add_triangle(0, a, b);
        let (c, d) = (1 + AROUND + j, 1 + AROUND + next);
        mesh.add_triangle(a, c, d);
        mesh.add_triangle(a, d, b);
    }
    mesh
}

impl Shell {
    /// The controls over the viewport `rect`, inside `inner` (see the top
    /// of this module).
    pub(super) fn viewport_controls(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        inner: Rect,
        scene: Option<&Scene>,
    ) {
        let tokens = self.tokens;
        // Each row laid out once where nothing shows and nothing can be
        // clicked, to measure it; its hits are overwritten below.
        let unseen = Rect::from_min_size(inner.min, vec2(1e6, tokens.bar_height));
        let measure =
            |ui: &mut egui::Ui, salt: &str, row: &mut dyn FnMut(&mut egui::Ui, Rect) -> f32| {
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(unseen)
                        .id_salt(("measure", salt))
                        .sizing_pass()
                        .invisible(),
                    |ui| row(ui, unseen),
                )
                .inner
            };
        let camera = measure(ui, "camera", &mut |ui, row| {
            self.camera_control(ui, row, scene)
        });
        let display = measure(ui, "display", &mut |ui, row| self.show_control(ui, row));
        let section = measure(ui, "section", &mut |ui, row| {
            self.section_control(ui, row, scene).0
        });
        let widths = RowWidths {
            camera,
            display,
            section,
        };
        let layout = controls_layout(inner, &tokens, &widths);
        self.controls_layout = Some(layout);
        let painter = ui.painter_at(rect);
        // The plates first, so the widgets are drawn on them.
        for row in [layout.camera_row, layout.display_row, layout.section_row]
            .into_iter()
            .flatten()
        {
            painter.rect_filled(
                row.expand2(vec2(tokens.plate_padding, 1.0)),
                egui::CornerRadius::same(tokens.radius.saturating_add(1)),
                color_alpha(tokens.panel_fill, tokens.plate_opacity),
            );
        }
        match layout.camera_row {
            Some(row) => {
                self.camera_control(ui, row, scene);
            }
            None => self.camera_hits.clear(),
        }
        match layout.display_row {
            Some(row) => {
                self.show_control(ui, row);
            }
            None => {
                self.show_hits.clear();
                self.transparency_rect = None;
            }
        }
        match layout.section_row {
            Some(row) => {
                self.section_control(ui, row, scene);
            }
            None => self.section_hits.clear(),
        }
        match layout.cube {
            Some(place) => {
                self.view_cube_rect = Some(place);
                self.view_cube(ui, place, layout.triad, scene);
            }
            None => {
                self.view_cube_rect = None;
                self.cube_drag_from = None;
            }
        }
    }

    /// Fit, iso and the projection, left to right in `row`. Returns the
    /// row's width.
    fn camera_control(&mut self, ui: &mut egui::Ui, row: Rect, scene: Option<&Scene>) -> f32 {
        let tokens = self.tokens;
        let mut hits: Vec<(&'static str, Rect)> = Vec::new();
        let mut view = None;
        let mut fit = false;
        let mut projection = self.state.camera.projection;
        let width = ui
            .scope_builder(egui::UiBuilder::new().max_rect(row), |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let button = ui
                        .add(egui::Button::new("Fit").small())
                        .on_hover_text("Frame the whole model (F)");
                    hits.push(("fit", button.rect));
                    fit = button.clicked();
                    let button = ui
                        .add(egui::Button::new("iso").small())
                        .on_hover_text("Look from the front-right corner, framed");
                    hits.push(("iso", button.rect));
                    if button.clicked() {
                        view = Some("iso");
                    }
                    ui.add_space(tokens.plate_padding);
                    // One control of two halves, side by side: the one
                    // that is on is filled, as a toggle is.
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for (label, kind, name, hint) in [
                        (
                            "Persp",
                            ProjectionKind::Perspective,
                            "perspective",
                            "Perspective: what is further away is smaller",
                        ),
                        (
                            "Ortho",
                            ProjectionKind::Orthographic,
                            "orthographic",
                            "Orthographic: parallel lines stay parallel, sizes are true",
                        ),
                    ] {
                        let button = ui
                            .add(
                                egui::Button::new(label)
                                    .small()
                                    .selected(projection == kind),
                            )
                            .on_hover_text(hint);
                        hits.push((name, button.rect));
                        if button.clicked() {
                            projection = kind;
                        }
                    }
                    ui.min_rect().width()
                })
                .inner
            })
            .inner;
        self.camera_hits = hits;
        if projection != self.state.camera.projection {
            self.turn = None;
            self.state.camera.projection = projection;
            ui.ctx().request_repaint();
        }
        if let Some(name) = view {
            self.turn = None;
            // The names come from the table, so this cannot fail.
            let _ = self.state.camera.set_view(name);
            ui.ctx().request_repaint();
        }
        if fit || view.is_some() {
            self.fit_to(scene);
            ui.ctx().request_repaint();
        }
        width
    }

    /// What the viewport draws besides the bodies, left to right in
    /// `row`: Sketches and Datums ([`crate::state::UiState::show`]) and
    /// how see-through the bodies are
    /// ([`crate::state::UiState::transparency`]). Returns the row's width.
    fn show_control(&mut self, ui: &mut egui::Ui, row: Rect) -> f32 {
        let tokens = self.tokens;
        let mut show = self.state.show;
        let mut transparency = self.state.transparency;
        let mut hits: Vec<(&'static str, Rect)> = Vec::new();
        let mut slider_rect = None;
        let mut slid = false;
        let width = ui
            .scope_builder(egui::UiBuilder::new().max_rect(row), |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let sketches = ui
                        .add(
                            egui::Button::new("Sketches")
                                .small()
                                .selected(show.sketches),
                        )
                        .on_hover_text("Show or hide the sketches");
                    hits.push(("sketches", sketches.rect));
                    if sketches.clicked() {
                        show.sketches = !show.sketches;
                    }
                    let datums = ui
                        .add(egui::Button::new("Datums").small().selected(show.datums))
                        .on_hover_text("Show or hide the datum planes, axes and points");
                    hits.push(("datums", datums.rect));
                    if datums.clicked() {
                        show.datums = !show.datums;
                    }
                    ui.add_space(tokens.plate_padding);
                    ui.label(egui::RichText::new("See-through").color(color(tokens.text_dim)));
                    ui.spacing_mut().slider_width = tokens.slider_width;
                    let slider = ui
                        .add(
                            egui::Slider::new(
                                &mut transparency,
                                0.0..=crate::state::MAX_TRANSPARENCY,
                            )
                            .show_value(false),
                        )
                        .on_hover_text("How much of what is behind a surface shows");
                    slider_rect = Some(slider.rect);
                    slid = slider.changed();
                    ui.min_rect().width()
                })
                .inner
            })
            .inner;
        self.show_hits = hits;
        self.transparency_rect = slider_rect;
        if slid && transparency.is_finite() {
            self.state.transparency = transparency.clamp(0.0, crate::state::MAX_TRANSPARENCY);
            ui.ctx().request_repaint();
        }
        if show != self.state.show {
            self.state.show = show;
            ui.ctx().request_repaint();
        }
        width
    }

    /// The section's controls, left to right in `row`: the toggle and,
    /// while the section is on, the axis the plane is across (X, Y, Z),
    /// a slider that moves the plane across the model's extent, and Flip,
    /// which keeps the other half. They set
    /// [`crate::state::UiState::section`]. Returns the row's width and
    /// whether it was drawn with the section on.
    fn section_control(
        &mut self,
        ui: &mut egui::Ui,
        row: Rect,
        scene: Option<&Scene>,
    ) -> (f32, bool) {
        let tokens = self.tokens;
        let mut section = self.state.section.clone();
        let range = self.section_cache.as_ref().and_then(|cache| cache.range);
        // The middle of the model along a base plane's normal: where a
        // plane that is chosen here starts.
        let middle = |axis: usize| {
            scene
                .and_then(|scene| scene.bbox.corners())
                .map_or(0.0, |(lo, hi)| {
                    let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
                    match (lo.get(axis), hi.get(axis)) {
                        (Some(lo), Some(hi)) => (lo + hi) * 0.5,
                        _ => 0.0,
                    }
                })
        };
        let mut hits: Vec<(&'static str, Rect)> = Vec::new();
        let width = ui
            .scope_builder(egui::UiBuilder::new().max_rect(row), |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let toggle = ui
                        .add(egui::Button::new("Section").small().selected(section.on))
                        .on_hover_text("Cut the picture open with a plane, to look inside");
                    hits.push(("toggle", toggle.rect));
                    if toggle.clicked() {
                        section.on = !section.on;
                        if section.on && section.plane.is_none() && section.normal.is_none() {
                            section.plane = Some("xy".to_string());
                            section.offset = middle(2);
                        }
                    }
                    if section.on {
                        ui.add_space(tokens.plate_padding);
                        for (label, name, plane, axis) in [
                            ("X", "x", "yz", 0),
                            ("Y", "y", "zx", 1),
                            ("Z", "z", "xy", 2),
                        ] {
                            let chosen = section.plane.as_deref() == Some(plane);
                            let button = ui
                                .add(egui::Button::new(label).small().selected(chosen))
                                .on_hover_text(format!("Cut across {label}"));
                            hits.push((name, button.rect));
                            if button.clicked() && !chosen {
                                section.plane = Some(plane.to_string());
                                section.feature = None;
                                section.point = None;
                                section.normal = None;
                                section.offset = middle(axis);
                            }
                        }
                        if let Some([lo, hi]) = range.filter(|[lo, hi]| lo < hi) {
                            let mut offset = section.offset.clamp(lo, hi);
                            ui.spacing_mut().slider_width = tokens.slider_width;
                            let slider = ui
                                .add(egui::Slider::new(&mut offset, lo..=hi).show_value(false))
                                .on_hover_text("Where the plane cuts: across the model");
                            hits.push(("offset", slider.rect));
                            if slider.changed() && offset.is_finite() {
                                section.offset = offset;
                            }
                        }
                        let flip = ui.small_button("Flip").on_hover_text("Keep the other half");
                        hits.push(("flip", flip.rect));
                        if flip.clicked() {
                            section.flip = !section.flip;
                        }
                    }
                    ui.min_rect().width()
                })
                .inner
            })
            .inner;
        let on = section.on;
        self.section_hits = hits;
        if section != self.state.section
            && let Ok(section) = section.validated()
        {
            self.state.section = section;
            self.section_lost = None;
            ui.ctx().request_repaint();
        }
        (width, on)
    }

    /// The radius of the cube's picture in `place`, in points.
    fn cube_arm(place: Rect) -> f32 {
        (place.width() * 0.5 * 0.97).max(1.0)
    }

    /// Where a point of the view cube's picture (in the units of
    /// [`view_cube::Facet::outline`]) is in the window, once the cube was
    /// drawn.
    pub fn view_cube_point(&self, at: [f64; 2]) -> Option<egui::Pos2> {
        let place = self.view_cube_rect?;
        let arm = Self::cube_arm(place);
        Some(place.center() + vec2(at[0] as f32, at[1] as f32) * arm)
    }

    /// The view cube in `place` and the triad in `triad`: paints them and
    /// handles the cube's mouse. A click on a facet starts a turn of the camera
    /// to that side (see [`Self::advance_turn`]), and a drag orbits the
    /// camera.
    fn view_cube(
        &mut self,
        ui: &mut egui::Ui,
        place: Rect,
        triad: Option<Rect>,
        scene: Option<&Scene>,
    ) {
        let tokens = self.tokens;
        // Only the cube takes the mouse: the triad is a picture, and a
        // click or a drag on it is one in the viewport.
        let area = triad.map_or(place, |t| t.union(place));
        let response = ui.interact(
            place,
            ui.id().with("view-cube"),
            egui::Sense::click_and_drag(),
        );
        let arm = Self::cube_arm(place);
        let center = place.center();
        let style = view_cube::style(&tokens);
        let unit = |at: egui::Pos2| {
            [
                f64::from(at.x - center.x) / f64::from(arm),
                f64::from(at.y - center.y) / f64::from(arm),
            ]
        };
        if response.drag_started() {
            self.cube_drag_from = Some(self.state.camera);
            self.turn = None;
        }
        if response.dragged() {
            // As a drag in the viewport: from where it began, by how far
            // the pointer is from there now.
            if let (Some(from), Some(total)) = (self.cube_drag_from, response.total_drag_delta()) {
                let mut camera = from;
                camera.orbit(f64::from(total.x), f64::from(total.y));
                self.state.camera = camera;
            }
            ui.ctx().request_repaint();
        } else {
            self.cube_drag_from = None;
        }
        if response.clicked()
            && let Some(at) = response.interact_pointer_pos()
            && let Some(facet) = view_cube::facet_at_styled(&self.state.camera, &style, unit(at))
        {
            let side = view_cube::clicked(&self.state.camera, &facet);
            let mut to = view_cube::turned(&self.state.camera, side);
            // Framed there, as the view buttons framed.
            if let Some(scene) = scene.filter(|scene| !scene.bbox.is_empty()) {
                to.fit(&scene.bbox, self.aspect());
                to.auto_fit = true;
            }
            self.turn = Some(Turn {
                from: self.state.camera,
                to,
                started: ui.input(|i| i.time),
                last: self.state.camera,
            });
            ui.ctx().request_repaint();
        }
        let hovered = response
            .hover_pos()
            .and_then(|at| view_cube::facet_at_styled(&self.state.camera, &style, unit(at)));
        let painter = ui.painter_at(area.expand(2.0));
        // The shadow first, under everything.
        painter.add(cube_shadow(&tokens, center, arm));
        let patches = cube_patches(&self.state.camera, &style, center, arm);
        for (facet, points) in &patches {
            let over = hovered.as_ref().is_some_and(|h| h.side == facet.side);
            let shape = if over {
                egui::Shape::convex_polygon(
                    points.clone(),
                    color_alpha(tokens.view_cube_hover, tokens.view_cube_hover_opacity),
                    egui::Stroke::new(
                        tokens.view_cube_hover_width,
                        color(tokens.view_cube_hover_outline),
                    ),
                )
            } else {
                egui::Shape::convex_polygon(
                    points.clone(),
                    cube_fill(&tokens, facet),
                    egui::Stroke::NONE,
                )
            };
            painter.add(shape);
        }
        // The names over the faces, in the faces' planes.
        for (facet, _) in &patches {
            if let Some(mesh) = cube_label(&painter, &tokens, facet, center, arm) {
                painter.add(mesh);
            }
        }
        if let Some(triad) = triad {
            self.paint_triad(&painter, triad);
        }
    }

    /// The axis triad in `place`: X, Y and Z from a common origin, in
    /// their colours, each with its letter at its tip, turning with the
    /// camera; an arm that points away from the viewer is dimmer.
    fn paint_triad(&self, painter: &egui::Painter, place: Rect) {
        let tokens = self.tokens;
        let Some(arms) = view_cube::triad(&self.state.camera) else {
            return;
        };
        let origin = place.center();
        let font = egui::FontId::proportional((place.height() * 0.17).max(8.0));
        // The letters sit just past the tips, inside the place.
        let reach = place.height() * 0.5 - font.size * 0.7;
        painter.circle_filled(origin, tokens.triad_width * 1.4, color(tokens.text));
        for arm in arms {
            let ink = tokens.axes.get(arm.axis).copied().unwrap_or(tokens.text);
            let alpha = if arm.toward < -0.05 { 150 } else { 255 };
            let tip = origin + vec2(arm.tip[0] as f32, arm.tip[1] as f32) * reach;
            let length = arm.tip[0].hypot(arm.tip[1]);
            // A short arm (one that points nearly at the viewer) has a
            // smaller head.
            let head = (tokens.triad_width * 2.5).min(0.35 * length as f32 * reach);
            if length > 0.2 {
                let along = vec2(arm.tip[0] as f32, arm.tip[1] as f32) / length as f32;
                let across = vec2(-along.y, along.x);
                // The shaft up to the arrowhead, and the head.
                painter.line_segment(
                    [origin, tip - along * head],
                    egui::Stroke::new(tokens.triad_width, color_alpha(ink, alpha)),
                );
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        tip,
                        tip - along * (head * 1.6) + across * head,
                        tip - along * (head * 1.6) - across * head,
                    ],
                    color_alpha(ink, alpha),
                    egui::Stroke::NONE,
                ));
                painter.text(
                    tip + along * (font.size * 0.6),
                    egui::Align2::CENTER_CENTER,
                    ["X", "Y", "Z"].get(arm.axis).copied().unwrap_or("?"),
                    font.clone(),
                    color_alpha(ink, alpha),
                );
            } else {
                // Seen end-on: a dot, and its letter beside the origin.
                painter.circle_filled(origin, tokens.triad_width * 2.0, color_alpha(ink, alpha));
                painter.text(
                    origin + vec2(font.size * 0.7, -font.size * 0.7),
                    egui::Align2::CENTER_CENTER,
                    ["X", "Y", "Z"].get(arm.axis).copied().unwrap_or("?"),
                    font.clone(),
                    color_alpha(ink, alpha),
                );
            }
        }
    }

    /// Where the controls over the viewport were laid out in the last
    /// frame.
    pub fn controls_layout(&self) -> Option<ControlsLayout> {
        self.controls_layout
    }

    /// Where the camera's buttons were drawn: `fit`, `iso`,
    /// `perspective`, `orthographic`.
    pub fn camera_hits(&self) -> &[(&'static str, Rect)] {
        &self.camera_hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What egui paints of the cube stays on the cube: every patch, as
    /// egui tessellates it with its anti-aliasing at one and two pixels
    /// per point, at rest and under the pointer (fill and outline), lies
    /// within the cube's silhouette, from cameras all round the cube in
    /// both projections. The allowance: egui's feathering (one pixel),
    /// half the hover outline's width, and how far a corner's circle,
    /// which lies in the corner's plane, reaches past the cut corner
    /// (measured: under a point at the default size). A facet seen nearly
    /// edge-on once drew a spike well outside the cube; such a patch is
    /// now not drawn at all.
    #[test]
    fn no_patch_of_the_cube_is_painted_outside_its_silhouette() {
        use crate::state::{CameraState, ProjectionKind};
        use egui::epaint::{Mesh, Shape, Stroke, TessellationOptions, Tessellator};
        let tokens = Tokens::dark();
        let style = view_cube::style(&tokens);
        let (center, arm) = (pos2(500.0, 400.0), 62.0_f32);
        let mut worst = f32::MIN;
        let mut cameras = 0;
        for projection in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
            for yaw in (-180..180).step_by(5) {
                for pitch in (-90..=90).step_by(5) {
                    cameras += 1;
                    let camera = CameraState {
                        yaw: f64::from(yaw) + 0.3,
                        pitch: f64::from(pitch),
                        projection,
                        ..CameraState::default()
                    };
                    let outline: Vec<[f64; 2]> = view_cube::silhouette(&camera, &style)
                        .into_iter()
                        .map(|p| [500.0 + p[0] * 62.0, 400.0 + p[1] * 62.0])
                        .collect();
                    for pixels_per_point in [1.0, 2.0] {
                        let mut tessellator = Tessellator::new(
                            pixels_per_point,
                            TessellationOptions::default(),
                            [1, 1],
                            Vec::new(),
                        );
                        for (facet, points) in cube_patches(&camera, &style, center, arm) {
                            for (fill, stroke) in [
                                (cube_fill(&tokens, &facet), Stroke::NONE),
                                (
                                    egui::Color32::WHITE,
                                    Stroke::new(tokens.view_cube_hover_width, egui::Color32::WHITE),
                                ),
                            ] {
                                let mut mesh = Mesh::default();
                                tessellator.tessellate_shape(
                                    Shape::convex_polygon(points.clone(), fill, stroke),
                                    &mut mesh,
                                );
                                let allowed = 1.0 / pixels_per_point + stroke.width * 0.5 + 1.0;
                                for v in &mesh.vertices {
                                    let at = [f64::from(v.pos.x), f64::from(v.pos.y)];
                                    let out = -view_cube::inside_by(&outline, at).unwrap() as f32;
                                    worst = worst
                                        .max(out - stroke.width * 0.5 - 1.0 / pixels_per_point);
                                    assert!(
                                        out <= allowed,
                                        "{projection:?} yaw {yaw} pitch {pitch} at {pixels_per_point}: {:?} is painted {out} points outside the cube",
                                        facet.side
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(cameras > 1000, "{cameras}");
        eprintln!("furthest outside, less feathering and outline: {worst} points");
        assert!(worst > -3.0, "the patches reach the cube's edge: {worst}");
    }

    /// The names on the faces lie in the faces: every corner of a name's
    /// glyphs is on its face's patch, at one and two pixels per point and
    /// from cameras all round; a face seen nearly edge-on has none.
    #[test]
    fn the_names_lie_on_their_faces() {
        use crate::state::CameraState;
        let tokens = Tokens::dark();
        let style = view_cube::style(&tokens);
        let (center, arm) = (pos2(300.0, 200.0), 62.0_f32);
        let mut drawn = 0;
        for pixels_per_point in [1.0, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(pixels_per_point);
            let mut painted = 0;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let painter = ui.painter().clone();
                for yaw in (-180..180).step_by(15) {
                    for pitch in (-75..=75).step_by(15) {
                        let camera = CameraState {
                            yaw: f64::from(yaw) + 0.3,
                            pitch: f64::from(pitch),
                            ..CameraState::default()
                        };
                        for (facet, points) in cube_patches(&camera, &style, center, arm) {
                            let mesh = cube_label(&painter, &tokens, &facet, center, arm);
                            let hidden = facet.toward < f64::from(tokens.view_cube_label_hide);
                            if facet.kind() != view_cube::FacetKind::Face || hidden {
                                assert!(mesh.is_none(), "{:?}", facet.side);
                                continue;
                            }
                            let Some(mesh) = mesh else { continue };
                            painted += 1;
                            let patch: Vec<[f64; 2]> = points
                                .iter()
                                .map(|p| [f64::from(p.x), f64::from(p.y)])
                                .collect();
                            for v in &mesh.vertices {
                                let at = [f64::from(v.pos.x), f64::from(v.pos.y)];
                                assert!(
                                    view_cube::inside_by(&patch, at).unwrap() > -0.5,
                                    "{yaw} {pitch} {:?}",
                                    facet.side
                                );
                                assert!(
                                    (0.0..=1.0).contains(&v.uv.x) && (0.0..=1.0).contains(&v.uv.y)
                                );
                            }
                        }
                    }
                }
            });
            output.textures_delta.clear();
            assert!(painted > 100, "{painted}");
            drawn += painted;
        }
        assert!(drawn > 200);
    }

    /// At every size, from a phone-sized viewport to a large one, with
    /// the default widths and wider ones: what is shown is inside the
    /// viewport, nothing overlaps, the triad is beside the cube and the
    /// cube above its buttons.
    #[test]
    fn the_controls_never_overlap_and_stay_inside() {
        let tokens = Tokens::dark();
        let estimate = RowWidths {
            camera: 150.0,
            display: 270.0,
            section: 52.0,
        };
        let wide = RowWidths {
            camera: 190.0,
            display: 330.0,
            section: 340.0,
        };
        let mut shown = [0; 5];
        for widths in [estimate, wide] {
            for w in (40..1600).step_by(37) {
                for h in (30..1000).step_by(29) {
                    let inner = Rect::from_min_size(pos2(5.0, 7.0), vec2(w as f32, h as f32));
                    let layout = controls_layout(inner, &tokens, &widths);
                    let parts = layout.parts();
                    for (k, (name, rect)) in parts.iter().enumerate() {
                        assert!(
                            inner.expand(0.5).contains_rect(*rect),
                            "{name} {rect:?} in {inner:?}"
                        );
                        for (other, second) in parts.iter().skip(k + 1) {
                            let overlap = rect.intersect(*second);
                            assert!(
                                overlap.width() <= 0.0 || overlap.height() <= 0.0,
                                "{name} {rect:?} and {other} {second:?} in {inner:?}"
                            );
                        }
                    }
                    // The triad beside the cube, at its lower left.
                    if let (Some(cube), Some(triad)) = (layout.cube, layout.triad) {
                        assert!(triad.right() <= cube.left() + 0.01);
                        assert!(triad.top() >= cube.top() - 0.51);
                        assert!(triad.bottom() <= cube.bottom() + 0.01);
                    }
                    if let (Some(cube), Some(row)) = (layout.cube, layout.camera_row) {
                        assert!(cube.bottom() < row.top());
                        assert!((row.center().x - cube.center().x).abs() < 0.01);
                    }
                    // The two groups of toggles are left-aligned.
                    if let (Some(a), Some(b)) = (layout.display_row, layout.section_row) {
                        assert_eq!(a.left(), b.left());
                        assert!(b.bottom() < a.top());
                    }
                    let names = ["camera", "display", "section", "cube", "triad"];
                    for (slot, name) in shown.iter_mut().zip(names) {
                        if parts.iter().any(|(n, _)| *n == name) {
                            *slot += 1;
                        }
                    }
                }
            }
        }
        assert!(shown.iter().all(|&n| n > 100), "{shown:?}");
        // At the default window's viewport everything shows, beside each
        // other, the cube in full size.
        let inner = Rect::from_min_size(pos2(270.0, 60.0), vec2(1000.0, 700.0));
        let layout = controls_layout(inner, &tokens, &estimate);
        assert_eq!(layout.parts().len(), 5);
        let cube = layout.cube.unwrap();
        assert_eq!(cube.width(), tokens.view_cube_size);
        assert_eq!(layout.display_row.unwrap().bottom(), inner.bottom());
        assert_eq!(layout.camera_row.unwrap().bottom(), inner.bottom());
        // Numbers that are not numbers give nothing that is not either.
        let broken = RowWidths {
            camera: f32::NAN,
            ..estimate
        };
        let layout = controls_layout(inner, &tokens, &broken);
        for (_, rect) in layout.parts() {
            assert!(rect.min.x.is_finite() && rect.max.y.is_finite());
        }
    }
}
