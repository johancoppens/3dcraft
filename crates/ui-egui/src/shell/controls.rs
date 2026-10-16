//! The controls over the viewport, in its bottom-right corner, in three
//! groups, each on a plate of its own:
//!
//! - **Camera**, in the corner: the axis triad, under it the view cube,
//!   under that *Fit*, *iso* and the projection, *Persp* | *Ortho*.
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
    /// The view cube's square place and the triad's above it.
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
    // The cube over the camera's row (or where it would be), the triad
    // over the cube when there is room for both.
    let floor = camera_row.map_or(camera_bottom - row, |r| r.top()) - tokens.bar_gap;
    let height = floor - inner.top();
    let wide = camera_width.min(inner.width());
    let with_triad = height - tokens.triad_size >= tokens.view_cube_min.max(tokens.view_cube_size);
    let room = height - if with_triad { tokens.triad_size } else { 0.0 };
    let side = tokens.view_cube_size.min(wide).min(room);
    let cube = (side >= tokens.view_cube_min && side.is_finite())
        .then(|| Rect::from_min_size(pos2(middle - side * 0.5, floor - side), vec2(side, side)));
    let triad = cube.filter(|_| with_triad).map(|cube| {
        Rect::from_center_size(
            pos2(cube.center().x, cube.top() - tokens.triad_size * 0.5),
            vec2(tokens.triad_size, tokens.triad_size),
        )
    });
    ControlsLayout {
        camera_row,
        display_row: Some(display_row).filter(|&r| groups_fit && fits(r)),
        section_row: Some(section_row).filter(|&r| groups_fit && fits(r)),
        cube,
        triad,
    }
}

/// `rgb` a fraction `t` of the way to `to`.
fn mix(rgb: [u8; 3], to: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let mut out = [0; 3];
    for ((slot, x), y) in out.iter_mut().zip(rgb).zip(to) {
        // Between two bytes.
        *slot = (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round() as u8;
    }
    out
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
            && let Some(facet) = view_cube::facet_at(&self.state.camera, unit(at))
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
            .and_then(|at| view_cube::facet_at(&self.state.camera, unit(at)));
        let painter = ui.painter_at(area.expand(2.0));
        let at = |p: [f64; 2]| center + vec2(p[0] as f32, p[1] as f32) * arm;
        let gap = f64::from(tokens.view_cube_gap);
        for facet in view_cube::facets_gapped(&self.state.camera, gap) {
            let points: Vec<egui::Pos2> = facet.patch.iter().map(|&p| at(p)).collect();
            // A face carries the colour of the axis it is across; the cut
            // facets are the gem's own, darker, and all are lit by how
            // much they face the viewer.
            let base = match facet.axis() {
                Some(axis) => mix(
                    tokens.body,
                    tokens.axes.get(axis).copied().unwrap_or(tokens.body),
                    0.38,
                ),
                None => mix(tokens.panel_fill, tokens.body, 0.45),
            };
            let lit = mix(tokens.notice_fill, base, 0.45 + 0.55 * facet.toward);
            let over = hovered.as_ref().is_some_and(|h| h.side == facet.side);
            let fill = if over {
                mix(lit, tokens.face_highlight, 0.75)
            } else {
                lit
            };
            painter.add(egui::Shape::convex_polygon(
                points,
                color(fill),
                egui::Stroke::NONE,
            ));
            if let Some(label) = facet.label() {
                self.cube_label(&painter, &facet, label, at(facet.center()), arm);
            }
        }
        if let Some(triad) = triad {
            self.paint_triad(&painter, triad);
        }
    }

    /// The name of a face of the cube in its middle: as large as the face
    /// has room for, up to its usual size, and left out when that is too
    /// small to read (a face seen nearly edge-on).
    fn cube_label(
        &self,
        painter: &egui::Painter,
        facet: &view_cube::Facet,
        label: &str,
        middle: egui::Pos2,
        arm: f32,
    ) {
        let tokens = self.tokens;
        let full = arm * 0.24;
        let ink = color_alpha(
            tokens.notice_fill,
            (255.0 * (0.35 + 0.65 * facet.toward).clamp(0.0, 1.0)) as u8,
        );
        let galley =
            painter.layout_no_wrap(label.to_string(), egui::FontId::proportional(full), ink);
        let [w, h] = facet.room();
        let size = galley.size();
        let fit = (0.82 * w as f32 * arm / size.x.max(1e-3))
            .min(0.9 * h as f32 * arm / size.y.max(1e-3))
            .min(1.0);
        let font = full * fit;
        if font.is_nan() || font < tokens.view_cube_label_min {
            return;
        }
        let galley = if fit < 1.0 {
            painter.layout_no_wrap(label.to_string(), egui::FontId::proportional(font), ink)
        } else {
            galley
        };
        let size = galley.size();
        painter.galley(middle - size * 0.5, galley, ink);
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
        painter.circle_filled(origin, tokens.triad_width * 1.2, color(tokens.text_dim));
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

    /// At every size, from a phone-sized viewport to a large one, with
    /// the default widths and wider ones: what is shown is inside the
    /// viewport, nothing overlaps, the triad is above the cube and the
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
                    if let (Some(cube), Some(triad)) = (layout.cube, layout.triad) {
                        assert!(triad.bottom() <= cube.top() + 0.01);
                        assert!((triad.center().x - cube.center().x).abs() < 0.01);
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
