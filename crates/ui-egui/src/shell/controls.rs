//! The controls over the viewport, in its bottom-right corner, in the
//! cube's glass style:
//!
//! - **The bottom bar**, one rounded glass plate: the display part,
//!   *Sketches* and *Datums* (switches) and the *See-through* number
//!   field, then a thin divider, then the camera part, *Fit* and *Iso*
//!   (actions) and *Persp* | *Ortho* (a choice).
//! - **The section plate** above the display part, on the same column
//!   grid: the *Section* switch, *X* | *Y* | *Z*, the *Offset* number field
//!   and *Front* | *Back* (which half is kept). While the section is off
//!   the plate stays and its controls but the switch are dimmed and inert.
//! - **The view cube** low over the camera part, centred on it, and the
//!   axis triad between the end of the section plate and the cube.
//!
//! In a viewport too narrow for the bar the camera part gets a plate of
//! its own over the section plate, with the display plate at the bottom;
//! in one narrower still only the camera part stays, and in one too low
//! for the cube the cube and the triad are left out. Everything here is
//! view state that `ui.set` sets too (`camera`, `view`, `show`,
//! `transparency`, `section`); the widgets only read and write
//! [`crate::state::UiState`]. They are widgets over the viewport, so a
//! click on them is not a click in the viewport and picks nothing. Sizes,
//! spacing and colours come from the tokens.

use super::number_field::{self, FieldOutcome, NumberField, Typing};
use super::{Shell, Turn};
use crate::state::ProjectionKind;
use crate::theme::{Tokens, color, color_alpha};
use crate::view_cube;
use craft3d_engine::Scene;
use egui::{Pos2, Rect, pos2, vec2};

/// The widths of the controls, in points, measured from their texts each
/// frame ([`ControlWidths::measure`]) so the grid fits any font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlWidths {
    /// The first column: the Sketches and Section switches.
    pub first: f32,
    /// The second column: the Datums switch and X | Y | Z.
    pub second: f32,
    /// The Offset field; the See-through field spans it and the fourth
    /// column.
    pub field: f32,
    /// The fourth column: Front | Back.
    pub fourth: f32,
    /// Fit, Iso, and one half of Persp | Ortho.
    pub fit: f32,
    pub iso: f32,
    pub projection: f32,
}

impl ControlWidths {
    /// The widths as the texts need them in `font` points, never narrower
    /// than the theme's minimums.
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
        let switch = |s: &str| SWITCH_TEXT + text(s) + 12.0;
        let segment = |s: &str, least: f32| (text(s) + 20.0).max(least);
        let xyz = ["X", "Y", "Z"]
            .iter()
            .map(|s| segment(s, 30.0))
            .fold(0.0, f32::max);
        let front_back = ["Front", "Back"]
            .iter()
            .map(|s| segment(s, 50.0))
            .fold(0.0, f32::max);
        let projection = ["Persp", "Ortho"]
            .iter()
            .map(|s| segment(s, 54.0))
            .fold(0.0, f32::max);
        let field = (text("Offset") + text("-000.00 mm") + 44.0).max(tokens.field_width);
        let out = Self {
            first: switch("Sketches").max(switch("Section")),
            second: switch("Datums").max(3.0 * xyz),
            field,
            fourth: 2.0 * front_back,
            fit: segment("Fit", 46.0),
            iso: segment("Iso", 46.0),
            projection,
        };
        if out.all_finite() {
            out
        } else {
            Self::fallback(tokens)
        }
    }

    fn all_finite(&self) -> bool {
        [
            self.first,
            self.second,
            self.field,
            self.fourth,
            self.fit,
            self.iso,
            self.projection,
        ]
        .iter()
        .all(|w| w.is_finite() && *w >= 0.0)
    }

    /// Widths when the texts could not be measured.
    pub fn fallback(tokens: &Tokens) -> Self {
        Self {
            first: 98.0,
            second: 94.0,
            field: tokens.field_width,
            fourth: 100.0,
            fit: 46.0,
            iso: 46.0,
            projection: 58.0,
        }
    }

    /// The display part (and the section plate): the four columns between
    /// the plate's padding.
    fn display(&self, tokens: &Tokens) -> f32 {
        2.0 * tokens.plate_padding
            + self.first
            + self.second
            + self.field
            + self.fourth
            + 3.0 * tokens.group_gap
    }

    /// The camera part's controls, without padding.
    fn camera(&self, tokens: &Tokens) -> f32 {
        self.fit + self.iso + 2.0 * self.projection + 2.0 * tokens.control_spacing
    }
}

/// Where the switch's text starts, from the switch's left: room for the
/// small pill with its knob.
const SWITCH_TEXT: f32 = 34.0;

/// Where the controls go in a viewport whose usable area (inside the
/// inset) is `inner`: the plates, every control's rectangle, the cube and
/// the triad. A part that does not fit is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ControlsLayout {
    /// The bottom bar (display and camera parts), or in a narrow viewport
    /// the display plate.
    pub bar: Option<Rect>,
    /// In a narrow viewport, the camera part's own plate.
    pub camera_plate: Option<Rect>,
    pub section_plate: Option<Rect>,
    /// The divider between the bar's two parts: its x and its top and
    /// bottom.
    pub divider: Option<[Pos2; 2]>,
    pub sketches: Option<Rect>,
    pub datums: Option<Rect>,
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
    /// The mockup arrangements' dividers between the groups of a plate
    /// (today's has the one `divider`).
    pub dividers: [Option<[Pos2; 2]>; 4],
    /// The title of the section's floating panel (arrangement C).
    pub section_title: Option<Rect>,
    /// The mockup arrangements' chips show their icons only: the viewport
    /// is too narrow for their names.
    pub compact: bool,
}

impl ControlsLayout {
    /// The plates, the cube and the triad that are shown: nothing of
    /// them overlaps.
    pub fn parts(&self) -> Vec<(&'static str, Rect)> {
        [
            ("bar", self.bar),
            ("camera", self.camera_plate),
            ("section", self.section_plate),
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

/// The layout of the controls in `inner` (see the top of this module).
pub fn controls_layout(inner: Rect, tokens: &Tokens, widths: &ControlWidths) -> ControlsLayout {
    let mut out = ControlsLayout::default();
    if !(inner.is_finite() && inner.width() > 0.0 && inner.height() > 0.0) {
        return out;
    }
    let widths = if widths.all_finite() {
        *widths
    } else {
        ControlWidths::fallback(tokens)
    };
    let pad = tokens.plate_padding;
    let h = tokens.bar_height;
    let plate_h = h + 2.0 * pad;
    let display_w = widths.display(tokens);
    let camera_w = widths.camera(tokens) + 2.0 * pad;
    let divider = tokens.divider_gap;
    let bar_w = display_w + 2.0 * divider + camera_w - pad;
    let (right, bottom) = (inner.right(), inner.bottom());
    let fits = |r: Rect| r.top() >= inner.top() - 0.01 && r.left() >= inner.left() - 0.01;
    // The display plate's left, the camera part's left (its first control)
    // and the bottom of the camera part's plate.
    let display_left;
    let camera_left;
    let camera_bottom;
    let mut display_plate = None;
    if inner.width() >= bar_w {
        // Wide: one bar.
        let bar = Rect::from_min_max(pos2(right - bar_w, bottom - plate_h), pos2(right, bottom));
        if !fits(bar) {
            return out;
        }
        out.bar = Some(bar);
        display_left = bar.left();
        let x = display_left + display_w + divider;
        out.divider = Some([
            pos2(x, bar.top() + pad + 2.0),
            pos2(x, bar.bottom() - pad - 2.0),
        ]);
        camera_left = x + divider;
        camera_bottom = bottom;
        display_plate = Some(bar);
    } else {
        // Narrow: the camera part on a plate of its own, over the section
        // and display plates when they fit the width.
        let groups = inner.width() >= display_w;
        let stacked = if groups {
            2.0 * (plate_h + tokens.group_gap)
        } else {
            0.0
        };
        camera_bottom = bottom - stacked;
        let plate = Rect::from_min_max(
            pos2(right - camera_w, camera_bottom - plate_h),
            pos2(right, camera_bottom),
        );
        display_left = right - display_w;
        if groups {
            let display =
                Rect::from_min_max(pos2(display_left, bottom - plate_h), pos2(right, bottom));
            if fits(display) {
                out.bar = Some(display);
                display_plate = Some(display);
            }
        }
        if !(fits(plate) && camera_w <= inner.width()) {
            // Not even the camera part: the display plate alone, if any.
            out.camera_plate = None;
            return finish_display(out, display_plate, display_left, tokens, &widths, inner);
        }
        out.camera_plate = Some(plate);
        camera_left = plate.left() + pad;
    }
    // The camera part's controls.
    let row_top = camera_bottom - pad - h;
    let mut x = camera_left;
    out.fit = Some(Rect::from_min_size(pos2(x, row_top), vec2(widths.fit, h)));
    x += widths.fit + tokens.control_spacing;
    out.iso = Some(Rect::from_min_size(pos2(x, row_top), vec2(widths.iso, h)));
    x += widths.iso + tokens.control_spacing;
    out.projection = Some(Rect::from_min_size(
        pos2(x, row_top),
        vec2(2.0 * widths.projection, h),
    ));
    let camera_middle = camera_left + widths.camera(tokens) * 0.5;
    out = finish_display(out, display_plate, display_left, tokens, &widths, inner);
    // The cube low over the camera part, centred on it.
    let floor = camera_bottom - plate_h;
    let side = tokens
        .view_cube_size
        .min(floor - inner.top())
        .min(inner.width());
    if side >= tokens.view_cube_min && side.is_finite() {
        let left = (camera_middle - side * 0.5).min(right - side);
        let cube = Rect::from_min_size(pos2(left, floor - side), vec2(side, side));
        let clear = out
            .section_plate
            .is_none_or(|s| !s.intersects(cube.shrink(0.01)));
        if clear {
            out.cube = Some(cube);
            out.triad = triad_place(&out, cube, tokens, inner);
        }
    }
    out
}

/// The display plate's and the section plate's controls, on one grid
/// from `left`.
fn finish_display(
    mut out: ControlsLayout,
    display: Option<Rect>,
    left: f32,
    tokens: &Tokens,
    widths: &ControlWidths,
    inner: Rect,
) -> ControlsLayout {
    let Some(display) = display else {
        return out;
    };
    let pad = tokens.plate_padding;
    let gap = tokens.group_gap;
    let h = tokens.bar_height;
    let columns = [
        left + pad,
        left + pad + widths.first + gap,
        left + pad + widths.first + widths.second + 2.0 * gap,
        left + pad + widths.first + widths.second + widths.field + 3.0 * gap,
    ];
    let [c1, c2, c3, c4] = columns;
    let row = |plate: Rect, x: f32, w: f32| {
        Rect::from_min_size(pos2(x, plate.bottom() - pad - h), vec2(w, h))
    };
    out.sketches = Some(row(display, c1, widths.first));
    out.datums = Some(row(display, c2, widths.second));
    out.see_through = Some(row(display, c3, widths.field + gap + widths.fourth));
    let display_w = widths.display(tokens);
    let section = Rect::from_min_size(
        pos2(left, display.top() - gap - h - 2.0 * pad),
        vec2(display_w, h + 2.0 * pad),
    );
    if section.top() >= inner.top() - 0.01 {
        out.section_plate = Some(section);
        out.section_toggle = Some(row(section, c1, widths.first));
        out.axes = Some(row(section, c2, widths.second));
        out.offset = Some(row(section, c3, widths.field));
        out.side = Some(row(section, c4, widths.fourth));
    }
    out
}

/// The triad's place: between the end of the section plate and the cube,
/// clear of both, as large as the theme says or as there is room for;
/// over the section plate's end when there is no room beside it.
fn triad_place(out: &ControlsLayout, cube: Rect, tokens: &Tokens, inner: Rect) -> Option<Rect> {
    let size = tokens.triad_size;
    let right = cube.left() - tokens.control_spacing;
    let place = match (out.camera_plate, out.section_plate) {
        // On the bar: over the end of the section plate, left of the cube.
        (None, Some(section)) => Rect::from_min_size(
            pos2(right - size, section.top() - tokens.control_spacing - size),
            vec2(size, size),
        ),
        // In a narrow viewport the camera's plate is over the section
        // plate, and the room left of the cube is free: beside the cube's
        // lower part.
        _ => {
            let floor = out
                .camera_plate
                .or(out.bar)
                .map_or(cube.bottom(), |b| b.top())
                - tokens.group_gap;
            Rect::from_min_size(pos2(right - size, floor - size), vec2(size, size))
        }
    };
    let clear = out
        .parts()
        .iter()
        .filter(|(name, _)| *name != "triad")
        .all(|(_, r)| !r.intersects(place.shrink(0.01)));
    (clear && inner.contains_rect(place)).then_some(place)
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

/// A colour faded as an inert control is.
fn faded(c: egui::Color32, enabled: bool, tokens: &Tokens) -> egui::Color32 {
    if enabled {
        c
    } else {
        c.gamma_multiply(tokens.disabled_opacity)
    }
}

/// A glass plate behind controls: a dark base so the controls read over a
/// light model too, the glass tint over it and a faint rim.
fn plate(painter: &egui::Painter, rect: Rect, tokens: &Tokens) {
    plate_rounded(painter, rect, tokens, tokens.plate_radius);
}

/// A glass plate (see [`plate`]) with corners of `radius` points.
pub(super) fn plate_rounded(painter: &egui::Painter, rect: Rect, tokens: &Tokens, radius: u8) {
    let radius = egui::CornerRadius::same(radius);
    painter.rect_filled(
        rect,
        radius,
        color_alpha(tokens.panel_fill, tokens.plate_opacity),
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

/// A switch: a small pill with a knob (accent when on) and its label; the
/// whole row is the target. Returns whether it was clicked.
fn switch(ui: &mut egui::Ui, rect: Rect, id: &str, label: &str, on: bool, tokens: &Tokens) -> bool {
    let response = ui.interact(rect, ui.id().with(("switch", id)), egui::Sense::click());
    let painter = ui.painter();
    let h = rect.height();
    let soft = if response.hovered() {
        tokens.control_opacity
    } else {
        tokens.control_opacity / 2
    };
    painter.rect_filled(
        rect,
        egui::CornerRadius::same((h * 0.5) as u8),
        color_alpha(tokens.glass, soft),
    );
    let pill = Rect::from_min_size(
        pos2(rect.left() + 8.0, rect.center().y - 5.0),
        tokens.switch_size,
    );
    let track = if on {
        color(tokens.accent)
    } else {
        color(tokens.switch_off)
    };
    let radius = tokens.switch_size.y * 0.5;
    painter.rect_filled(pill, egui::CornerRadius::same(radius as u8), track);
    let knob = pos2(
        if on {
            pill.right() - radius
        } else {
            pill.left() + radius
        },
        pill.center().y,
    );
    painter.circle_filled(knob, radius - 1.0, color(tokens.knob));
    painter.text(
        pos2(rect.left() + SWITCH_TEXT, rect.center().y - 1.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(tokens.control_font),
        color(if on { tokens.text } else { tokens.text_dim }),
    );
    response.clicked()
}

/// An action: a plain pill. Returns whether it was clicked.
fn action(ui: &mut egui::Ui, rect: Rect, id: &str, label: &str, tokens: &Tokens) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(("action", id)), egui::Sense::click());
    let painter = ui.painter();
    let alpha = if response.hovered() {
        tokens.control_opacity.saturating_mul(2)
    } else {
        tokens.control_opacity
    };
    painter.rect_filled(
        rect,
        egui::CornerRadius::same((rect.height() * 0.5) as u8),
        color_alpha(tokens.glass, alpha),
    );
    painter.text(
        pos2(rect.center().x, rect.center().y - 1.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(tokens.control_font),
        color(tokens.text),
    );
    response
}

/// A choice: a segmented pill, the active segment filled with the accent.
/// Returns the segments' rectangles and the one clicked, if any; an inert
/// one (dimmed) is never clicked.
fn segmented(
    ui: &mut egui::Ui,
    rect: Rect,
    id: &str,
    items: &[&str],
    active: Option<usize>,
    enabled: bool,
    tokens: &Tokens,
) -> (Vec<Rect>, Option<usize>) {
    let painter = ui.painter().clone();
    let h = rect.height();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same((h * 0.5) as u8),
        faded(
            color_alpha(tokens.glass, tokens.control_opacity),
            enabled,
            tokens,
        ),
    );
    let n = items.len().max(1) as f32;
    let w = rect.width() / n;
    let mut rects = Vec::new();
    let mut clicked = None;
    for (k, item) in items.iter().enumerate() {
        let cell = Rect::from_min_size(pos2(rect.left() + w * k as f32, rect.top()), vec2(w, h));
        rects.push(cell);
        // Inert, it still takes the click, so that is not a click on the
        // model behind it.
        let response = ui.interact(cell, ui.id().with(("segment", id, k)), egui::Sense::click());
        let on = active == Some(k);
        if on {
            painter.rect_filled(
                cell.shrink(2.0),
                egui::CornerRadius::same(((h - 4.0) * 0.5) as u8),
                faded(color(tokens.accent), enabled, tokens),
            );
        } else if enabled && response.hovered() {
            painter.rect_filled(
                cell.shrink(2.0),
                egui::CornerRadius::same(((h - 4.0) * 0.5) as u8),
                color_alpha(tokens.glass, tokens.control_opacity),
            );
        }
        let ink = if on { tokens.knob } else { tokens.text };
        painter.text(
            pos2(cell.center().x, cell.center().y - 1.0),
            egui::Align2::CENTER_CENTER,
            *item,
            egui::FontId::proportional(tokens.control_font),
            faded(color(ink), enabled, tokens),
        );
        if enabled && response.clicked() {
            clicked = Some(k);
        }
    }
    (rects, clicked)
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
    /// The controls over the viewport `rect`, inside `inner` (see the top
    /// of this module).
    pub(super) fn viewport_controls(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        inner: Rect,
        scene: Option<&Scene>,
    ) {
        let variant = self.state.controls_layout;
        if variant != crate::state::ControlsVariant::Current {
            self.variant_controls(ui, rect, scene, variant);
            return;
        }
        let tokens = self.tokens;
        let painter = ui.painter_at(rect);
        let widths = ControlWidths::measure(&painter, &tokens);
        let layout = controls_layout(inner, &tokens, &widths);
        self.controls_layout = Some(layout);
        // The plates first, so the controls are drawn on them.
        for plate_rect in [layout.bar, layout.camera_plate, layout.section_plate]
            .into_iter()
            .flatten()
        {
            plate(&painter, plate_rect, &tokens);
            // The plate takes the mouse where no control is: a click
            // between two controls is not a pick in the viewport.
            ui.interact(
                plate_rect,
                ui.id().with((
                    "plate",
                    plate_rect.min.x.to_bits(),
                    plate_rect.min.y.to_bits(),
                )),
                egui::Sense::click_and_drag(),
            );
        }
        if let Some([top, bottom]) = layout.divider {
            painter.line_segment(
                [top, bottom],
                egui::Stroke::new(1.0, color_alpha(tokens.plate_rim, tokens.divider_opacity)),
            );
        }
        self.camera_control(ui, &layout, scene);
        self.show_control(ui, &layout);
        self.section_control(ui, &layout, scene);
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

    /// Fit, Iso and the projection.
    fn camera_control(
        &mut self,
        ui: &mut egui::Ui,
        layout: &ControlsLayout,
        scene: Option<&Scene>,
    ) {
        let tokens = self.tokens;
        let mut hits: Vec<(&'static str, Rect)> = Vec::new();
        let (Some(fit_rect), Some(iso_rect), Some(projection_rect)) =
            (layout.fit, layout.iso, layout.projection)
        else {
            self.camera_hits.clear();
            return;
        };
        let fit = action(ui, fit_rect, "fit", "Fit", &tokens)
            .on_hover_text("Frame the whole model (F)")
            .clicked();
        hits.push(("fit", fit_rect));
        let iso = action(ui, iso_rect, "iso", "Iso", &tokens)
            .on_hover_text("Look from the front-right corner, framed")
            .clicked();
        hits.push(("iso", iso_rect));
        let active = match self.state.camera.projection {
            ProjectionKind::Perspective => 0,
            ProjectionKind::Orthographic => 1,
        };
        let (cells, clicked) = segmented(
            ui,
            projection_rect,
            "projection",
            &["Persp", "Ortho"],
            Some(active),
            true,
            &tokens,
        );
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

    /// What a click on Fit, on Iso or on one half of the projection does,
    /// whichever arrangement drew them.
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

    /// Sketches and Datums ([`crate::state::UiState::show`]) and how
    /// see-through the bodies are ([`crate::state::UiState::transparency`]).
    fn show_control(&mut self, ui: &mut egui::Ui, layout: &ControlsLayout) {
        let tokens = self.tokens;
        let (Some(sketches), Some(datums), Some(field)) =
            (layout.sketches, layout.datums, layout.see_through)
        else {
            self.show_hits.clear();
            self.transparency_rect = None;
            return;
        };
        let mut show = self.state.show;
        if switch(ui, sketches, "sketches", "Sketches", show.sketches, &tokens) {
            show.sketches = !show.sketches;
        }
        if switch(ui, datums, "datums", "Datums", show.datums, &tokens) {
            show.datums = !show.datums;
        }
        self.show_hits = vec![("sketches", sketches), ("datums", datums)];
        self.see_through_field(ui, field);
        self.set_show(ui, show);
    }

    /// Shows `show` from now on.
    pub(super) fn set_show(&mut self, ui: &egui::Ui, show: crate::state::Show) {
        if show != self.state.show {
            self.state.show = show;
            ui.ctx().request_repaint();
        }
    }

    /// The See-through field in `field`: how see-through the bodies are.
    pub(super) fn see_through_field(&mut self, ui: &mut egui::Ui, field: Rect) {
        let tokens = self.tokens;
        self.transparency_rect = Some(field);
        let range = [0.0, crate::state::MAX_TRANSPARENCY];
        let outcome = NumberField {
            id: "see-through",
            label: "See-through",
            value: self.state.transparency,
            range,
            step: 0.05,
            scale: 100.0,
            decimals: 0,
            unit: "%",
            enabled: true,
            range_known: true,
        }
        .show(ui, field, &tokens, &mut self.fields, &mut self.typing);
        if let Some(value) = outcome.value.filter(|v| v.is_finite()) {
            self.state.transparency = value.clamp(range[0], range[1]);
            ui.ctx().request_repaint();
        }
    }

    /// The section's controls: the switch and, active while the section
    /// is on, the axis the plane is across (X, Y, Z), the Offset field
    /// across the model's extent, and which half is kept (Front, the half
    /// the plane's normal points away from, or Back). They set
    /// [`crate::state::UiState::section`].
    fn section_control(
        &mut self,
        ui: &mut egui::Ui,
        layout: &ControlsLayout,
        scene: Option<&Scene>,
    ) {
        let tokens = self.tokens;
        let (Some(toggle), Some(axes), Some(offset), Some(side)) = (
            layout.section_toggle,
            layout.axes,
            layout.offset,
            layout.side,
        ) else {
            self.section_hits.clear();
            return;
        };
        let mut section = self.state.section.clone();
        let mut hits: Vec<(&'static str, Rect)> = vec![("toggle", toggle)];
        if switch(ui, toggle, "section", "Section", section.on, &tokens) {
            toggle_section(&mut section, scene);
        }
        let on = self.state.section.on;
        let chosen = chosen_axis(&section);
        let (cells, clicked) = segmented(ui, axes, "axes", &["X", "Y", "Z"], chosen, on, &tokens);
        if on {
            for ((name, _), cell) in SECTION_PLANES.iter().zip(&cells) {
                hits.push((name, *cell));
            }
        }
        if let Some(axis) = clicked {
            choose_axis(&mut section, axis, scene);
        }
        self.offset_field(ui, offset, &mut section, on);
        if on {
            hits.push(("offset", offset));
        }
        let (cells, clicked) = segmented(
            ui,
            side,
            "side",
            &["Front", "Back"],
            Some(usize::from(section.flip)),
            on,
            &tokens,
        );
        if on {
            for (name, cell) in ["front", "back"].into_iter().zip(cells) {
                hits.push((name, cell));
            }
        }
        if let Some(k) = clicked {
            section.flip = k == 1;
        }
        self.section_hits = hits;
        self.commit_section(ui, section);
    }

    /// The Offset field in `rect`: where the section's plane is along its
    /// normal, across the model's extent; inert while the section is off.
    pub(super) fn offset_field(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        section: &mut crate::state::SectionState,
        on: bool,
    ) {
        let tokens = self.tokens;
        let range = self.section_cache.as_ref().and_then(|cache| cache.range);
        let field_range = range.filter(|[lo, hi]| lo < hi);
        let unit = self.length_unit;
        let outcome: FieldOutcome = NumberField {
            id: "offset",
            label: "Offset",
            value: section.offset,
            range: field_range.unwrap_or([section.offset - 1.0, section.offset + 1.0]),
            step: field_range.map_or(1.0, number_field::nice_step),
            scale: 1.0,
            decimals: 2,
            unit,
            enabled: on && field_range.is_some(),
            range_known: field_range.is_some(),
        }
        .show(ui, rect, &tokens, &mut self.fields, &mut self.typing);
        if let Some(value) = outcome.value.filter(|v| v.is_finite()) {
            section.offset = value;
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

    /// The view cube in `place` and the triad in `triad`: paints them and
    /// handles the cube's mouse. A click on a facet starts a turn of the camera
    /// to that side (see [`Self::advance_turn`]), and a drag orbits the
    /// camera.
    pub(super) fn view_cube(
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
            painter.add(egui::Shape::convex_polygon(
                points.clone(),
                color_alpha(tokens.panel_fill, tokens.view_cube_base_opacity),
                egui::Stroke::NONE,
            ));
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
            let font = (triad.height() * 0.17).clamp(8.0, 12.0);
            self.paint_triad(&painter, triad, font);
        }
    }

    /// The axis triad in `place`: X, Y and Z from a common origin, in
    /// their colours, each with its letter at its tip, turning with the
    /// camera; an arm that points away from the viewer is dimmer.
    pub(super) fn paint_triad(&self, painter: &egui::Painter, place: Rect, font_size: f32) {
        let tokens = self.tokens;
        let Some(arms) = view_cube::triad(&self.state.camera) else {
            return;
        };
        let origin = place.center();
        let font = egui::FontId::proportional(font_size);
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
    /// the theme's widths and wider ones: what is shown is inside the
    /// viewport, no plate overlaps another, the cube or the triad, every
    /// control is on a plate and clear of the others, the section's
    /// controls are on the display part's grid, the cube is low over the
    /// camera part and the triad is left of it.
    #[test]
    fn the_controls_never_overlap_and_stay_inside() {
        let tokens = Tokens::dark();
        let plain = ControlWidths::fallback(&tokens);
        let wide = ControlWidths {
            first: 130.0,
            second: 120.0,
            field: 210.0,
            fourth: 130.0,
            fit: 60.0,
            iso: 60.0,
            projection: 70.0,
        };
        let mut shown = [0; 5];
        for widths in [plain, wide] {
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
                                overlap.width() <= 0.01 || overlap.height() <= 0.01,
                                "{name} {rect:?} and {other} {second:?} in {inner:?}"
                            );
                        }
                    }
                    let plates: Vec<Rect> = [layout.bar, layout.camera_plate, layout.section_plate]
                        .into_iter()
                        .flatten()
                        .collect();
                    let controls = layout.controls();
                    for (k, (name, rect)) in controls.iter().enumerate() {
                        assert!(
                            plates.iter().any(|p| p.expand(0.01).contains_rect(*rect)),
                            "{name} {rect:?} on no plate in {inner:?}"
                        );
                        for (other, second) in controls.iter().skip(k + 1) {
                            assert!(
                                !rect.shrink(0.01).intersects(*second),
                                "{name} and {other} in {inner:?}"
                            );
                        }
                    }
                    if let (Some(a), Some(b)) = (layout.sketches, layout.section_toggle) {
                        assert_eq!(a.left(), b.left());
                        assert!(b.bottom() < a.top());
                        let (see, offset) = (layout.see_through.unwrap(), layout.offset.unwrap());
                        assert_eq!(see.left(), offset.left());
                    }
                    if let (Some(cube), Some(fit), Some(projection)) =
                        (layout.cube, layout.fit, layout.projection)
                    {
                        assert!(cube.bottom() <= fit.top() - tokens.plate_padding + 0.01);
                        let middle = (fit.left() + projection.right()) * 0.5;
                        assert!(
                            (cube.center().x - middle).abs() < 0.01
                                || cube.right() >= inner.right() - 0.01
                        );
                    }
                    if let (Some(cube), Some(triad)) = (layout.cube, layout.triad) {
                        assert!(triad.right() <= cube.left() + 0.01);
                    }
                    let names = ["bar", "camera", "section", "cube", "triad"];
                    for (slot, name) in shown.iter_mut().zip(names) {
                        if parts.iter().any(|(n, _)| *n == name) {
                            *slot += 1;
                        }
                    }
                }
            }
        }
        assert!(shown.iter().all(|&n| n > 100), "{shown:?}");
        // At the default window's viewport everything shows, on one bar,
        // the cube in full size and the triad over the end of the section
        // plate, left of the cube.
        let inner = Rect::from_min_size(pos2(270.0, 60.0), vec2(1000.0, 700.0));
        let layout = controls_layout(inner, &tokens, &plain);
        assert_eq!(layout.parts().len(), 4, "{layout:?}");
        assert!(layout.camera_plate.is_none() && layout.divider.is_some());
        let cube = layout.cube.unwrap();
        assert_eq!(cube.width(), tokens.view_cube_size);
        assert_eq!(layout.bar.unwrap().bottom(), inner.bottom());
        assert_eq!(cube.bottom(), layout.bar.unwrap().top());
        let triad = layout.triad.unwrap();
        let section = layout.section_plate.unwrap();
        assert!(triad.bottom() < section.top() && triad.right() > section.right());
        assert!(triad.right() <= cube.left());
        // Numbers that are not numbers give nothing that is not either.
        let broken = ControlWidths {
            first: f32::NAN,
            ..plain
        };
        let layout = controls_layout(inner, &tokens, &broken);
        for (_, rect) in layout.parts().into_iter().chain(layout.controls()) {
            assert!(rect.min.x.is_finite() && rect.max.y.is_finite());
        }
        let nowhere = Rect::from_min_size(pos2(f32::NAN, 0.0), vec2(10.0, 10.0));
        assert!(controls_layout(nowhere, &tokens, &plain).parts().is_empty());
    }
}
