//! What the controls over the viewport share: where they were laid out
//! ([`ControlsLayout`], placed by [`super::layout`]), the view cube and the
//! axis triad (painted and handled here), the state changes behind every
//! control (camera, display, section), and the typing into the number
//! fields. Everything here is view state that `ui.set` sets too
//! (`camera`, `view`, `show`, `transparency`, `section`); the widgets only
//! read and write [`crate::state::UiState`].

use super::number_field::{self, Typing};
use super::{Shell, TURN_SECONDS, Turn};
use crate::kit::{Kit, Slider, SliderState};
use crate::state::ProjectionKind;
use crate::theme::{Tokens, color, color_alpha};
use crate::view_cube;
use craft3d_engine::Scene;
use craft3d_math::Portable;
use egui::{Pos2, Rect, vec2};

/// Where the controls go in the viewport ([`super::layout`]): the plates,
/// every control's rectangle, the cube and the triad. A part that does
/// not fit is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ControlsLayout {
    /// The display bar at the bottom, with the section's controls.
    pub bar: Option<Rect>,
    /// The camera plate under the view cube.
    pub camera_plate: Option<Rect>,
    /// The section's options on a plate of their own above the display
    /// bar, where the viewport is too narrow for them on the bar (a
    /// phone); `None` while they are inline or off.
    pub section_plate: Option<Rect>,
    /// The compact player of the narrow layout, on its plate above the
    /// display bar (and the section's plate), while the document has a
    /// timeline and the drawer with the whole player is shut.
    pub player_plate: Option<Rect>,
    pub sketches: Option<Rect>,
    pub datums: Option<Rect>,
    /// The Edges toggle: the bodies' edges shown or not.
    pub edges: Option<Rect>,
    /// The Studio toggle: the studio look or the plain one.
    pub studio: Option<Rect>,
    /// The Analysis toggle: zebra stripes, highlight lines, a curvature
    /// map, draft, or none.
    pub analysis: Option<Rect>,
    pub see_through: Option<Rect>,
    pub section_toggle: Option<Rect>,
    pub axes: Option<Rect>,
    pub offset: Option<Rect>,
    pub side: Option<Rect>,
    pub fit: Option<Rect>,
    pub iso: Option<Rect>,
    pub projection: Option<Rect>,
    /// The view cube's square place and the triad's.
    pub cube: Option<Rect>,
    pub triad: Option<Rect>,
    /// The dividers between the groups of a plate: on the camera plate,
    /// before Section and before the section's options.
    pub dividers: [Option<[Pos2; 2]>; 3],
    /// The chips show their icons only: the viewport is too narrow for
    /// their names.
    pub compact: bool,
}

impl ControlsLayout {
    /// The plates, the cube and the triad that are shown: nothing of
    /// them overlaps.
    pub fn parts(&self) -> Vec<(&'static str, Rect)> {
        [
            ("bar", self.bar),
            ("section", self.section_plate),
            ("player", self.player_plate),
            ("camera", self.camera_plate),
            ("cube", self.cube),
            ("triad", self.triad),
        ]
        .into_iter()
        .filter_map(|(name, rect)| Some((name, rect?)))
        .collect()
    }

    /// Every control that is shown, by name.
    pub fn controls(&self) -> Vec<(&'static str, Rect)> {
        [
            ("sketches", self.sketches),
            ("datums", self.datums),
            ("edges", self.edges),
            ("studio", self.studio),
            ("analysis", self.analysis),
            ("see-through", self.see_through),
            ("section", self.section_toggle),
            ("axes", self.axes),
            ("offset", self.offset),
            ("side", self.side),
            ("fit", self.fit),
            ("iso", self.iso),
            ("projection", self.projection),
        ]
        .into_iter()
        .filter_map(|(name, rect)| Some((name, rect?)))
        .collect()
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

/// How a patch of the view cube is filled at rest: the glass over its
/// base, by one rule for faces, capsules and circles alike: how the patch
/// is turned to the theme's light (`view_cube_light`), so the cube is
/// shaded only by the light and no patch stands out by a colour of its
/// own.
pub fn cube_fill(tokens: &Tokens, facet: &view_cube::Facet) -> egui::Color32 {
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
    if opacity <= 0.0 || frame.half_side.is_nan() || frame.half_side <= 0.0 {
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
    // From a point of the text (in points, from the middle of its ink) to
    // the window: along the face, as the picture shows it.
    let per = 1.0 / frame.half_side;
    let (mut right, mut down) = (
        vec2((frame.right[0] * per) as f32, (frame.right[1] * per) as f32),
        vec2((frame.down[0] * per) as f32, (frame.down[1] * per) as f32),
    );
    // The glyphs are drawn at the size the face shows them at on
    // average (the square root of how much the map shrinks an area), not
    // straight on: a face seen at a slant then does not sample its glyph
    // images too sparsely, which looks thin and ragged.
    let shrink = (right.x * down.y - right.y * down.x).abs().sqrt();
    let shrink = if shrink.is_finite() {
        shrink.clamp(0.5, 1.0)
    } else {
        1.0
    };
    if shrink < 1.0 {
        galley = layout(size * f64::from(shrink));
        right /= shrink;
        down /= shrink;
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
                middle + vec2(rx * reach * angle.pcos(), ry * reach * angle.psin()),
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

/// A glass plate behind controls: a `plate_fill` base so the controls read
/// over any model, the glass tint over it and a faint rim.
pub(super) fn plate(painter: &egui::Painter, rect: Rect, tokens: &Tokens) {
    let radius = egui::CornerRadius::same(tokens.nav_radius);
    painter.rect_filled(
        rect,
        radius,
        color_alpha(tokens.plate_fill, tokens.plate_opacity),
    );
    painter.rect_filled(
        rect,
        radius,
        color_alpha(tokens.glass, tokens.plate_glass_opacity),
    );
    painter.rect_stroke(
        rect,
        radius,
        egui::Stroke::new(1.0, color_alpha(tokens.plate_rim, tokens.plate_rim_opacity)),
        egui::StrokeKind::Inside,
    );
}

/// How long an arm of the triad in `place` is drawn when it is seen
/// square on, in points: `nav_triad_arm`, or less in a place too small
/// for it and its letter.
pub(super) fn triad_reach(tokens: &Tokens, place: Rect) -> f32 {
    let room = place.height().min(place.width()) * 0.5 - tokens.nav_triad_font * 0.7;
    tokens.nav_triad_arm.min(room).max(0.0)
}

/// The section's base planes by the axis they are across: X, Y and Z.
pub(super) const SECTION_PLANES: [(&str, &str); 3] = [("x", "yz"), ("y", "zx"), ("z", "xy")];

/// The middle of the model along axis `axis`: where a plane that is
/// chosen in the controls starts.
fn model_middle(scene: Option<&Scene>, axis: usize) -> f64 {
    scene
        .and_then(|scene| scene.bbox.corners())
        .map_or(0.0, |(lo, hi)| {
            let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
            match (lo.get(axis), hi.get(axis)) {
                (Some(lo), Some(hi)) => (lo + hi) * 0.5,
                _ => 0.0,
            }
        })
}

/// The section switched on or off; switched on with no plane at all, it
/// is across Z through the model's middle.
pub(super) fn toggle_section(section: &mut crate::state::SectionState, scene: Option<&Scene>) {
    section.on = !section.on;
    if section.on && section.plane.is_none() && section.normal.is_none() {
        section.plane = Some("xy".to_string());
        section.offset = model_middle(scene, 2);
    }
}

/// Which of X, Y and Z the section is across, if it is across a base
/// plane.
pub(super) fn chosen_axis(section: &crate::state::SectionState) -> Option<usize> {
    SECTION_PLANES
        .iter()
        .position(|(_, plane)| section.plane.as_deref() == Some(*plane))
}

/// The section across axis `axis` (0 X, 1 Y, 2 Z) through the model's
/// middle, unless it is across that axis already.
pub(super) fn choose_axis(
    section: &mut crate::state::SectionState,
    axis: usize,
    scene: Option<&Scene>,
) {
    if Some(axis) == chosen_axis(section) {
        return;
    }
    if let Some((_, plane)) = SECTION_PLANES.get(axis) {
        section.plane = Some((*plane).to_string());
        section.feature = None;
        section.point = None;
        section.normal = None;
        section.offset = model_middle(scene, axis);
    }
}

impl Shell {
    /// A section switched on (by `ui.set`) without saying where: across Z
    /// when it names no plane, and through the middle of the model's box
    /// along the normal of a base plane (X, Y or Z) when it gives no
    /// offset, as the Section chip and the axis choice do. The middle is
    /// that of the scene shown: before the first one is drawn it is 0.
    pub(crate) fn section_through_the_middle(&mut self, offset_given: bool) {
        let scene = self.shown_scene.clone();
        let section = &mut self.state.section;
        if section.plane.is_none() && section.normal.is_none() && section.point.is_none() {
            section.plane = Some("xy".to_string());
        }
        if offset_given || section.feature.is_some() {
            return;
        }
        if let Some(axis) = chosen_axis(section) {
            section.offset = model_middle(scene.as_deref(), axis);
        }
    }

    /// What a click on Fit, on Iso or on one half of the projection does.
    pub(super) fn camera_clicks(
        &mut self,
        ui: &egui::Ui,
        fit: bool,
        iso: bool,
        projection: Option<ProjectionKind>,
        scene: Option<&Scene>,
    ) {
        if let Some(projection) = projection.filter(|p| *p != self.state.camera.projection) {
            self.turn = None;
            self.state.camera.projection = projection;
            ui.ctx().request_repaint();
        }
        if iso {
            self.turn = None;
            // The name comes from the table, so this cannot fail.
            let _ = self.state.camera.set_view("iso");
        }
        if fit || iso {
            self.fit_to(scene);
            ui.ctx().request_repaint();
        }
    }

    /// Shows `show` from now on.
    pub(super) fn set_show(&mut self, ui: &egui::Ui, show: crate::state::Show) {
        if show != self.state.show {
            self.state.show = show;
            ui.ctx().request_repaint();
        }
    }

    /// The See-through slider in `field` (one row: label, rail, value):
    /// how see-through the bodies are.
    pub(super) fn see_through_field(&mut self, ui: &mut egui::Ui, field: Rect) {
        let kit = Kit::plate(&self.tokens);
        self.slider_rails.retain(|(n, _, _)| *n != "see-through");
        let max = crate::state::MAX_TRANSPARENCY;
        let mut value = self.state.transparency;
        let (label_w, value_w) = bar_slider_widths(ui, &kit, "See-through", "90 %");
        // Clear of the chip before it, as the chips are of each other.
        let field = field.with_min_x((field.left() + 6.0).min(field.right()));
        // The compact bar has no room for the label: the tooltip says it.
        let label = if field.width() >= label_w + value_w + 40.0 {
            "See-through"
        } else {
            ""
        };
        let shown = kit.slider_in(
            ui,
            field,
            Slider::new(label, &mut value, 0.0..=max)
                .widths(label_w, value_w)
                .step(0.05)
                .shown_as(100.0, 0, "%")
                .default_value(0.0),
            Some(SliderState {
                field: "see-through",
                memory: &mut self.fields,
                typing: &mut self.typing,
            }),
        );
        // Where it takes the mouse: the rail and the knob's reach.
        self.transparency_rect = Some(shown.response.rect);
        if label.is_empty() {
            shown.response.clone().on_hover_text("See-through");
        }
        if let Some(rail) = shown.rail {
            self.slider_rails.push(("see-through", rail, shown.field));
        }
        if shown.changed && value.is_finite() {
            self.state.transparency = value.clamp(0.0, max);
            ui.ctx().request_repaint();
        }
    }

    /// The Offset slider in `rect`: where the section's plane is along
    /// its normal, across the model's extent; inert while the section is
    /// off.
    pub(super) fn offset_field(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        section: &mut crate::state::SectionState,
        on: bool,
    ) -> Rect {
        let kit = Kit::plate(&self.tokens);
        self.slider_rails.retain(|(n, _, _)| *n != "offset");
        let range = self.section_cache.as_ref().and_then(|cache| cache.range);
        let field_range = range.filter(|[lo, hi]| lo < hi);
        let unit = self.length_unit;
        let [lo, hi] = field_range.unwrap_or([section.offset - 1.0, section.offset + 1.0]);
        let mut value = section.offset;
        let (label_w, value_w) = bar_slider_widths(ui, &kit, "Offset", "-000.00 mm");
        let rect = rect.with_min_x((rect.left() + 6.0).min(rect.right()));
        let label = if rect.width() >= label_w + value_w + 40.0 {
            "Offset"
        } else {
            ""
        };
        let shown = kit.slider_in(
            ui,
            rect,
            Slider::new(label, &mut value, lo..=hi)
                .widths(label_w, value_w)
                .step(field_range.map_or(1.0, number_field::nice_step))
                .shown_as(1.0, 2, unit)
                .enabled(on && field_range.is_some()),
            Some(SliderState {
                field: "offset",
                memory: &mut self.fields,
                typing: &mut self.typing,
            }),
        );
        if let Some(rail) = shown.rail {
            self.slider_rails.push(("offset", rail, shown.field));
        }
        if shown.changed && value.is_finite() {
            section.offset = value;
        }
        shown.response.rect
    }

    /// Forgets the number field `id`: what was typed into it, a drag on
    /// it and the focus, for a field that is no longer shown.
    pub(super) fn forget_field(&mut self, id: &str) {
        self.slider_rails.retain(|(n, _, _)| *n != id);
        if self.typing.as_ref().is_some_and(|t| t.field == id) {
            self.typing = None;
        }
        if self.fields.drag.is_some_and(|d| d.field == id) {
            self.fields.drag = None;
        }
        if self.fields.focus == Some(id) {
            self.fields.focus = None;
        }
    }

    /// Makes `section` the viewport's section, when it changed and is one.
    pub(super) fn commit_section(&mut self, ui: &egui::Ui, section: crate::state::SectionState) {
        if section != self.state.section
            && let Ok(section) = section.validated()
        {
            self.state.section = section;
            self.section_lost = None;
            ui.ctx().request_repaint();
        }
    }

    /// The text typed into a number field, once Enter was pressed:
    /// evaluated as the document's expressions are, and set; or the reason
    /// it is not a number, shown at the field, with nothing changed.
    pub(super) fn resolve_typing(&mut self, session: &craft3d_engine::Session) {
        let Some(typing) = self.typing.as_mut().filter(|t| t.entered) else {
            return;
        };
        typing.entered = false;
        let text = typing.text.trim().to_string();
        let result = match typing.field {
            "see-through" => {
                let text = text.strip_suffix('%').unwrap_or(&text).trim();
                session
                    .evaluate_typed(text, craft3d_engine::TypedKind::Number)
                    .map(|v| (v / 100.0).clamp(0.0, crate::state::MAX_TRANSPARENCY))
            }
            _ => session.evaluate_typed(&text, craft3d_engine::TypedKind::Length),
        };
        match result {
            Ok(value) => {
                match typing.field {
                    "see-through" => self.state.transparency = value,
                    _ => {
                        let mut section = self.state.section.clone();
                        section.offset = value;
                        match section.validated() {
                            Ok(section) => self.state.section = section,
                            Err(error) => {
                                // The number is one, but not a plane the
                                // section can take: say why, keep typing.
                                typing.error = Some(error.to_string());
                                typing.focused = false;
                                return;
                            }
                        }
                    }
                }
                self.typing = None;
            }
            Err(error) => {
                typing.error = Some(error.message().to_string());
                typing.focused = false;
            }
        }
    }

    /// The field being typed into and what is typed, if any: `field` is
    /// `see-through` or `offset`.
    pub fn typing(&self) -> Option<&Typing> {
        self.typing.as_ref()
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

    /// The view cube in `place`: paints it and handles its mouse. A click on a facet starts a turn of the camera
    /// to that side (see [`Self::advance_turn`]), and a drag orbits the
    /// camera.
    pub(super) fn view_cube(&mut self, ui: &mut egui::Ui, place: Rect, scene: Option<&Scene>) {
        let tokens = self.tokens;
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
            if let Some(scene) = scene.filter(|scene| !scene.frame_box().is_empty()) {
                to.fit(&scene.frame_box(), self.aspect());
                to.auto_fit = true;
            }
            self.turn = Some(Turn {
                from: self.state.camera,
                to,
                started: ui.input(|i| i.time),
                last: self.state.camera,
                seconds: TURN_SECONDS,
            });
            ui.ctx().request_repaint();
        }
        let hovered = response
            .hover_pos()
            .and_then(|at| view_cube::facet_at_styled(&self.state.camera, &style, unit(at)));
        let painter = ui.painter_at(place.expand(2.0));
        // The shadow first, under everything.
        painter.add(cube_shadow(&tokens, center, arm));
        let patches = cube_patches(&self.state.camera, &style, center, arm);
        for (facet, points) in &patches {
            let over = hovered.as_ref().is_some_and(|h| h.side == facet.side);
            // A patch turning edge-on fades out gradually.
            let fade = view_cube::patch_opacity(
                facet.toward,
                f64::from(tokens.view_cube_fade),
                f64::from(tokens.view_cube_fade_full),
            ) as f32;
            painter.add(egui::Shape::convex_polygon(
                points.clone(),
                color_alpha(tokens.view_cube_base, tokens.view_cube_base_opacity)
                    .gamma_multiply(fade),
                egui::Stroke::NONE,
            ));
            let shape = if over {
                egui::Shape::convex_polygon(
                    points.clone(),
                    color_alpha(tokens.view_cube_hover, tokens.view_cube_hover_opacity)
                        .gamma_multiply(fade),
                    egui::Stroke::new(
                        tokens.view_cube_hover_width,
                        color(tokens.view_cube_hover_outline).gamma_multiply(fade),
                    ),
                )
            } else {
                egui::Shape::convex_polygon(
                    points.clone(),
                    cube_fill(&tokens, facet).gamma_multiply(fade),
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
    }

    /// The axis triad in `place`: X, Y and Z from a common origin, in
    /// their colours, each with its letter at its tip, turning with the
    /// camera; an arm that points away from the viewer is dimmer.
    pub(super) fn paint_triad(&self, painter: &egui::Painter, place: Rect) {
        let tokens = self.tokens;
        let Some(arms) = view_cube::triad(&self.state.camera) else {
            return;
        };
        let origin = place.center();
        let font = egui::FontId::proportional(tokens.nav_triad_font);
        // An arm seen square on is `nav_triad_arm` long; the letters sit
        // just past the tips, inside the place, which is as large as the
        // arm and a letter on either side ([`super::layout`]).
        let reach = triad_reach(&tokens, place);
        let width = tokens.nav_triad_width;
        painter.circle_filled(origin, width * 1.4, color(tokens.view_cube_label));
        for arm in arms {
            let ink = tokens
                .axes
                .get(arm.axis)
                .copied()
                .unwrap_or(tokens.view_cube_label);
            let alpha = if arm.toward < -0.05 { 150 } else { 255 };
            let tip = origin + vec2(arm.tip[0] as f32, arm.tip[1] as f32) * reach;
            let length = arm.tip[0].phypot(arm.tip[1]);
            // A short arm (one that points nearly at the viewer) has a
            // smaller head.
            let head = (width * 2.5).min(0.35 * length as f32 * reach);
            if length > 0.2 {
                let along = vec2(arm.tip[0] as f32, arm.tip[1] as f32) / length as f32;
                let across = vec2(-along.y, along.x);
                // The shaft up to the arrowhead, and the head.
                painter.line_segment(
                    [origin, tip - along * head],
                    egui::Stroke::new(width, color_alpha(ink, alpha)),
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
                painter.circle_filled(origin, width * 2.0, color_alpha(ink, alpha));
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
    use egui::pos2;

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
        let tokens = Tokens::default();
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
        let tokens = Tokens::default();
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
}

/// The widths of a display-bar slider's label and value: as wide as
/// `label` and the widest `value` are in the kit's font.
pub(super) fn bar_slider_widths(ui: &egui::Ui, kit: &Kit, label: &str, value: &str) -> (f32, f32) {
    let w = |s: &str| kit.galley(ui, s, kit.font(), kit.skin.text).size().x;
    let (l, v) = (w(label) + 8.0, w(value) + 4.0);
    (
        if l.is_finite() { l } else { 80.0 },
        if v.is_finite() { v } else { 48.0 },
    )
}
