//! The picture in the viewport: the scene's bodies shaded by a renderer
//! (the GPU's or the CPU reference renderer, see [`crate::backend`]), and
//! the lines drawn over that image (the edges of the bodies, sketches, the
//! world axes) projected with the same camera.
//!
//! The lines are tested against the renderer's depth buffer, a pixel at a
//! time: the part of a body's edge that lies behind a body is left out,
//! and the part of a sketch behind a body is marked hidden, which is
//! drawn dimmed, so a sketch inside or behind a part can still be found.
//! The world axes are a gizmo and always drawn in full.
//!
//! No egui here: the window paints the [`Overlay`] with its painter, and
//! `ui.screenshot` rasterises the same overlay into the same image, so the
//! two show the same thing.

use crate::backend::{Backend, BodyRenderer};
use crate::camera::{Projected, Projector};
use crate::state::CameraState;
use crate::theme::{Rgb, Tokens};
use craft3d_engine::Scene;
use craft3d_math::Vec3;
use craft3d_render::{Frame, Highlight, RenderParams};

/// Smallest and largest side of a viewport image, in pixels.
pub const MIN_SIDE: u32 = 16;
pub const MAX_SIDE: u32 = 4096;

/// The world axes are drawn this long, as a fraction of the camera's
/// distance to its target: the same size on screen at any zoom.
pub const AXIS_LENGTH: f64 = 0.06;

/// What a line of the overlay is, which decides its colour and width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Sketch geometry that is part of a profile.
    Sketch,
    /// Construction geometry of a sketch.
    Construction,
    /// Geometry of the selected feature's sketch.
    Selected,
    /// A world axis at the origin: 0 is X, 1 is Y, 2 is Z.
    Axis(usize),
    /// An edge of a body.
    Edge,
}

/// A line in image pixels (x right, y down, from the top-left corner).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayLine {
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub kind: LineKind,
    /// Behind a body: drawn dimmed.
    pub hidden: bool,
}

/// A dot in image pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayPoint {
    pub at: [f64; 2],
    pub kind: LineKind,
    /// Behind a body: drawn dimmed.
    pub hidden: bool,
}

/// Everything drawn over the rendered image.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Overlay {
    pub lines: Vec<OverlayLine>,
    /// The point entities of sketches, as dots.
    pub points: Vec<OverlayPoint>,
}

/// How opaque a line or dot is drawn, 0 to 255: full, or the theme's
/// opacity for what is hidden behind a body.
pub fn opacity(hidden: bool, tokens: &Tokens) -> u8 {
    if hidden { tokens.hidden_opacity } else { 255 }
}

impl LineKind {
    pub fn color(self, tokens: &Tokens) -> Rgb {
        match self {
            LineKind::Sketch => tokens.sketch,
            LineKind::Construction => tokens.sketch_construction,
            LineKind::Selected => tokens.sketch_selected,
            LineKind::Axis(axis) => tokens.axes.get(axis).copied().unwrap_or(tokens.text_dim),
            LineKind::Edge => tokens.edge,
        }
    }

    /// In points.
    pub fn width(self, tokens: &Tokens) -> f32 {
        match self {
            LineKind::Sketch | LineKind::Selected => tokens.sketch_width,
            LineKind::Construction => tokens.construction_width,
            LineKind::Axis(_) => tokens.axis_width,
            LineKind::Edge => tokens.edge_width,
        }
    }
}

/// The longest line is cut into at most this many pieces for the depth
/// test: more than the diagonal of the largest image.
const MAX_DEPTH_STEPS: f64 = 8192.0;

/// How much further than the surface under it a line may be and still
/// count as lying on that surface, as a fraction of the camera's distance
/// to its target. A sketch on a face and an edge between two faces are at
/// the very depth of the surface, give or take rounding.
pub const DEPTH_SLACK: f64 = 1e-6;

/// The GPU renderer's distance is an `f32` computation: sixteen roundings
/// of 2⁻²⁴ on numbers as large as the eye's distance to the model's centre
/// plus the model's diagonal (the `reach` of docs/architecture.md §7.2).
/// The depth test of the overlay allows a GPU frame that much more.
pub const GPU_DEPTH_ERROR: f64 = 16.0 / 16_777_216.0;

/// How far behind the surface under it a line may be and still count as
/// lying on it, in model units: [`DEPTH_SLACK`] of the camera's distance,
/// and for a frame the GPU made its [`GPU_DEPTH_ERROR`] of the reach too.
pub fn depth_slack(backend: Backend, scene: Option<&Scene>, camera: &CameraState) -> f64 {
    let base = camera.distance.abs() * DEPTH_SLACK;
    match backend {
        Backend::Cpu => base,
        Backend::Gpu => {
            let reach = scene
                .and_then(|scene| scene.bbox.center().map(|c| (c, scene.bbox.diagonal())))
                .map_or(0.0, |(center, diagonal)| {
                    (camera.eye() - center).length() + diagonal
                });
            let extra = reach * GPU_DEPTH_ERROR;
            if extra.is_finite() {
                base + extra
            } else {
                base
            }
        }
    }
}

/// The renderer's depth buffer, as the lines are tested against it.
#[derive(Clone, Copy)]
struct DepthMap<'a> {
    width: usize,
    height: usize,
    /// Distance along the view direction per pixel, row-major; infinite
    /// where there is no body.
    depth: &'a [f64],
    slack: f64,
}

impl<'a> DepthMap<'a> {
    /// `None` when the buffer is not the size it says, or the slack is
    /// not a number: nothing is hidden then, rather than the wrong thing.
    fn new(frame: &'a Frame, slack: f64) -> Option<Self> {
        let (width, height) = (frame.width as usize, frame.height as usize);
        (width.checked_mul(height) == Some(frame.depth.len()) && slack.is_finite()).then_some(
            Self {
                width,
                height,
                depth: &frame.depth,
                slack,
            },
        )
    }

    /// True when a body is in front of the point at pixel (`x`, `y`) and
    /// `depth`. The point is compared with the surface under its own
    /// pixel and under the eight around it, and is hidden only when all
    /// of them are nearer: an edge on a surface that slopes away from the
    /// viewer is further than the surface one pixel towards the viewer,
    /// and still on it.
    fn covers(&self, x: f64, y: f64, depth: f64) -> bool {
        if !(x.is_finite() && y.is_finite() && depth.is_finite()) {
            return false;
        }
        let (px, py) = (x.floor(), y.floor());
        if px < 0.0 || py < 0.0 || px >= self.width as f64 || py >= self.height as f64 {
            return false;
        }
        // Within the image by the check above.
        let (px, py) = (px as usize, py as usize);
        for ny in py.saturating_sub(1)..=(py + 1).min(self.height.saturating_sub(1)) {
            for nx in px.saturating_sub(1)..=(px + 1).min(self.width.saturating_sub(1)) {
                let surface = self
                    .depth
                    .get(ny * self.width + nx)
                    .copied()
                    .unwrap_or(f64::INFINITY);
                // A surface at or behind the point does not hide it; nor
                // does a depth that is not a number.
                let nearer = surface + self.slack < depth;
                if !nearer {
                    return false;
                }
            }
        }
        true
    }
}

/// The stretches of the segment `a`-`b` that are in front of every body
/// and behind one, as `(from, to, hidden)` with `from` and `to` fractions
/// of the segment, in order and covering 0 to 1.
fn stretches(
    map: Option<&DepthMap<'_>>,
    projector: &Projector,
    a: &Projected,
    b: &Projected,
) -> Vec<(f64, f64, bool)> {
    let whole = vec![(0.0, 1.0, false)];
    let Some(map) = map else {
        return whole;
    };
    // Only what is over the image can be behind something in it.
    let Some((first, last)) = clip_range(
        [a.x, a.y],
        [b.x, b.y],
        [0.0, 0.0],
        [map.width as f64, map.height as f64],
    ) else {
        return whole;
    };
    let pixels = ((b.x - a.x).abs().max((b.y - a.y).abs()) * (last - first)).ceil();
    if !(pixels.is_finite() && last > first) {
        // A dot: one test decides.
        let hidden = map.covers(a.x, a.y, a.depth);
        return vec![(0.0, 1.0, hidden)];
    }
    // At least one, at most MAX_DEPTH_STEPS: fits an integer.
    let steps = pixels.clamp(1.0, MAX_DEPTH_STEPS) as usize;
    let mut out: Vec<(f64, f64, bool)> = Vec::new();
    let mut push = |from: f64, to: f64, hidden: bool| {
        if to <= from {
            return;
        }
        match out.last_mut() {
            Some(last) if last.2 == hidden => last.1 = to,
            _ => out.push((from, to, hidden)),
        }
    };
    push(0.0, first, false);
    for step in 0..steps {
        let at = |k: usize| first + (last - first) * (k as f64 / steps as f64);
        let (from, to) = (at(step), at(step + 1));
        // Tested at the middle of the piece.
        let t = 0.5 * (from + to);
        let hidden = map.covers(
            a.x + (b.x - a.x) * t,
            a.y + (b.y - a.y) * t,
            projector.depth_between(a, b, t),
        );
        push(from, to, hidden);
    }
    push(last, 1.0, false);
    if out.is_empty() { whole } else { out }
}

/// The lines over an image of `width` × `height` pixels seen with
/// `camera`, with nothing hidden: every edge of every body shows, as in a
/// wireframe. `selected` is the id of the selected feature, whose sketch
/// is highlighted. A camera that looks nowhere gives an empty overlay.
pub fn overlay(
    scene: Option<&Scene>,
    camera: &CameraState,
    width: u32,
    height: u32,
    selected: Option<&str>,
) -> Overlay {
    build_overlay(scene, camera, width, height, selected, None)
}

/// The lines over `frame`, the image of the scene's bodies seen with
/// `camera` as the CPU renderer made it: what [`overlay`] gives, tested
/// against the frame's depth buffer. Edges of bodies are cut where a body
/// is in front of them; sketch lines and points are marked hidden there.
pub fn overlay_on(
    scene: Option<&Scene>,
    camera: &CameraState,
    frame: &Frame,
    selected: Option<&str>,
) -> Overlay {
    overlay_on_with(
        scene,
        camera,
        frame,
        selected,
        depth_slack(Backend::Cpu, scene, camera),
    )
}

/// [`overlay_on`] with the depth test's slack given in model units (see
/// [`depth_slack`]): for a frame another renderer made.
pub fn overlay_on_with(
    scene: Option<&Scene>,
    camera: &CameraState,
    frame: &Frame,
    selected: Option<&str>,
    slack: f64,
) -> Overlay {
    let map = DepthMap::new(frame, slack);
    build_overlay(
        scene,
        camera,
        frame.width,
        frame.height,
        selected,
        map.as_ref(),
    )
}

fn build_overlay(
    scene: Option<&Scene>,
    camera: &CameraState,
    width: u32,
    height: u32,
    selected: Option<&str>,
    map: Option<&DepthMap<'_>>,
) -> Overlay {
    let mut out = Overlay::default();
    let Some(projector) = Projector::new(&camera.to_render(), f64::from(width), f64::from(height))
    else {
        return out;
    };
    // `tested`: against the depth buffer; `dimmed`: what is hidden is
    // kept and marked, not left out.
    let mut line = |a: Vec3, b: Vec3, kind: LineKind, tested: bool, dimmed: bool| {
        let Some((from, to)) = projector.segment(a, b) else {
            return;
        };
        let at = |t: f64| [from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t];
        for (start, end, hidden) in stretches(map.filter(|_| tested), &projector, &from, &to) {
            if hidden && !dimmed {
                continue;
            }
            out.lines.push(OverlayLine {
                // The ends themselves, not the ends computed back.
                from: if start <= 0.0 {
                    [from.x, from.y]
                } else {
                    at(start)
                },
                to: if end >= 1.0 { [to.x, to.y] } else { at(end) },
                kind,
                hidden,
            });
        }
    };
    // The edges of the bodies first, then the world axes over them (an
    // edge along an axis must not paint it out), then the sketches.
    for body in scene.into_iter().flat_map(|scene| &scene.bodies) {
        for edge in &body.edges {
            for pair in edge.windows(2) {
                if let [a, b] = *pair {
                    line(a, b, LineKind::Edge, true, false);
                }
            }
        }
    }
    let length = camera.distance * AXIS_LENGTH;
    for (axis, direction) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
        line(
            Vec3::ZERO,
            direction * length,
            LineKind::Axis(axis),
            false,
            false,
        );
    }
    let Some(scene) = scene else {
        return out;
    };
    for sketch in &scene.sketches {
        let chosen = selected == Some(sketch.feature.as_str());
        for polyline in &sketch.polylines {
            let kind = match (polyline.construction, chosen) {
                (true, _) => LineKind::Construction,
                (false, true) => LineKind::Selected,
                (false, false) => LineKind::Sketch,
            };
            for pair in polyline.points.windows(2) {
                if let [a, b] = *pair {
                    line(a, b, kind, true, true);
                }
            }
        }
    }
    for sketch in &scene.sketches {
        let kind = if selected == Some(sketch.feature.as_str()) {
            LineKind::Selected
        } else {
            LineKind::Sketch
        };
        for point in &sketch.points {
            if let Some(p) = projector.project(*point) {
                out.points.push(OverlayPoint {
                    at: [p.x, p.y],
                    kind,
                    hidden: map.is_some_and(|map| map.covers(p.x, p.y, p.depth)),
                });
            }
        }
    }
    out
}

/// How an image is to be rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quality {
    pub width: u32,
    pub height: u32,
    /// Supersampling per axis, 1 to 4.
    pub ssaa: u32,
}

/// What either renderer is given for an image of `quality` in the theme's
/// colours, with no face highlighted.
pub fn render_params(quality: Quality, tokens: &Tokens) -> RenderParams {
    RenderParams {
        width: quality.width,
        height: quality.height,
        ssaa: quality.ssaa,
        background: tokens.viewport_background,
        color: tokens.body,
        highlight: Highlight::NONE,
    }
}

/// The faces of `scene` that `face` names (a persistent name, with the
/// feature of its body when given), as the renderers highlight them in the
/// theme's colour. No face when the scene has none of that name: a
/// highlight is view state and may outlive the face it names.
pub fn face_highlight(
    scene: Option<&Scene>,
    feature: Option<&str>,
    face: &str,
    tokens: &Tokens,
) -> Highlight {
    scene
        .and_then(|scene| scene.faces_named(feature, face).ok())
        .map_or(Highlight::NONE, |faces| {
            Highlight::new(tokens.face_highlight, faces.ids).0
        })
}

/// The bodies of `scene` as the CPU reference renderer shades them, in the
/// theme's colours. No scene, or a scene without bodies, is the background.
pub fn render_bodies(
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
) -> Result<Frame, String> {
    let params = render_params(quality, tokens);
    let empty = craft3d_tess::Mesh::default();
    let mesh = scene.map_or(&empty, |scene| &scene.mesh);
    craft3d_render::render(mesh, &camera.to_render(), &params).map_err(|e| e.to_string())
}

/// One image of the viewport as the window shows it: the bodies as
/// pixels, and what is drawn over them as lines in the image's pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Composed {
    pub width: u32,
    pub height: u32,
    /// 3 bytes per pixel, row-major from the top-left.
    pub rgb: Vec<u8>,
    pub overlay: Overlay,
    /// Fraction of the pixels covered by a body.
    pub coverage: f64,
    pub triangles: usize,
    /// Which renderer shaded the bodies.
    pub backend: Backend,
}

/// Renders the bodies with the CPU reference renderer and works out the
/// overlay for the same camera and image size: [`compose_with`] on the
/// oracle.
pub fn compose(
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
    selected: Option<&str>,
) -> Result<Composed, String> {
    compose_with(
        &mut BodyRenderer::cpu(),
        scene,
        camera,
        quality,
        tokens,
        selected,
    )
}

/// Renders the bodies with `renderer` and works out the overlay for the
/// same camera and image size, tested against the frame's depth buffer
/// with the slack that renderer needs. No egui and no shared state: the
/// window calls it on its render thread.
pub fn compose_with(
    renderer: &mut BodyRenderer,
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
    selected: Option<&str>,
) -> Result<Composed, String> {
    let (frame, backend) = renderer.render(scene, camera, quality, tokens)?;
    let coverage = frame.coverage();
    let overlay = overlay_on_with(
        scene,
        camera,
        &frame,
        selected,
        depth_slack(backend, scene, camera),
    );
    Ok(Composed {
        width: frame.width,
        height: frame.height,
        rgb: frame.rgb,
        overlay,
        coverage,
        triangles: scene.map_or(0, Scene::triangles),
        backend,
    })
}

/// A finished picture of the viewport: bodies and overlay.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// 3 bytes per pixel, row-major from the top-left.
    pub rgb: Vec<u8>,
    /// Fraction of the pixels covered by a body.
    pub coverage: f64,
    /// Fraction of the pixels the overlay changed.
    pub overlay_coverage: f64,
    pub triangles: usize,
}

/// Renders the bodies with the CPU reference renderer and draws the
/// overlay into the image: what the window shows, without a window, from
/// the oracle (`ui.screenshot`). `scale` is the number of pixels per
/// point, for the widths of the lines.
pub fn picture(
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
    selected: Option<&str>,
    scale: f64,
) -> Result<Picture, String> {
    let composed = compose(scene, camera, quality, tokens, selected)?;
    Ok(rasterise(composed, tokens, scale))
}

/// [`picture`] with faces highlighted: what the window shows when a face
/// was clicked or pointed at.
#[allow(clippy::too_many_arguments)]
pub fn picture_with(
    scene: Option<&Scene>,
    camera: &CameraState,
    quality: Quality,
    tokens: &Tokens,
    selected: Option<&str>,
    highlight: Highlight,
    scale: f64,
) -> Result<Picture, String> {
    let mut renderer = BodyRenderer::cpu();
    renderer.set_highlight(highlight);
    let composed = compose_with(&mut renderer, scene, camera, quality, tokens, selected)?;
    Ok(rasterise(composed, tokens, scale))
}

/// Draws the overlay of `composed` into its pixels. `scale` is the number
/// of pixels per point, for the widths of the lines.
pub fn rasterise(composed: Composed, tokens: &Tokens, scale: f64) -> Picture {
    let Composed {
        width,
        height,
        mut rgb,
        overlay,
        coverage,
        triangles,
        backend: _,
    } = composed;
    let before = rgb.clone();
    let scale = if scale.is_finite() && scale > 0.0 {
        scale.min(16.0)
    } else {
        1.0
    };
    for line in &overlay.lines {
        draw_line(
            &mut rgb,
            width,
            height,
            line.from,
            line.to,
            f64::from(line.kind.width(tokens)) * scale,
            line.kind.color(tokens),
            opacity(line.hidden, tokens),
        );
    }
    for point in &overlay.points {
        // A dot is a line without length.
        draw_line(
            &mut rgb,
            width,
            height,
            point.at,
            point.at,
            f64::from(tokens.point_radius) * 2.0 * scale,
            point.kind.color(tokens),
            opacity(point.hidden, tokens),
        );
    }
    let changed = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .zip(before.as_chunks::<3>().0)
        .filter(|(a, b)| a != b)
        .count();
    let pixels = (rgb.len() / 3).max(1);
    Picture {
        width,
        height,
        rgb,
        coverage,
        overlay_coverage: changed as f64 / pixels as f64,
        triangles,
    }
}

/// The part of the segment inside the box `[lo, hi]` on both axes
/// (Liang–Barsky), or `None` when it misses the box.
fn clip(a: [f64; 2], b: [f64; 2], lo: [f64; 2], hi: [f64; 2]) -> Option<([f64; 2], [f64; 2])> {
    let (t0, t1) = clip_range(a, b, lo, hi)?;
    let at = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    Some((at(t0), at(t1)))
}

/// The same as fractions of the segment: it is inside the box from the
/// first to the second.
fn clip_range(a: [f64; 2], b: [f64; 2], lo: [f64; 2], hi: [f64; 2]) -> Option<(f64, f64)> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (delta, start, low, high) in [(dx, a[0], lo[0], hi[0]), (dy, a[1], lo[1], hi[1])] {
        if delta == 0.0 {
            if start < low || start > high {
                return None;
            }
            continue;
        }
        let (mut near, mut far) = ((low - start) / delta, (high - start) / delta);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    (t0.is_finite() && t1.is_finite()).then_some((t0, t1))
}

/// Draws an anti-aliased line of `width` pixels with round ends into an RGB
/// image, `opacity` (0 to 255) opaque. Coordinates may be anything, the
/// image is never indexed outside.
#[allow(clippy::too_many_arguments)]
pub fn draw_line(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    from: [f64; 2],
    to: [f64; 2],
    line_width: f64,
    color: Rgb,
    opacity: u8,
) {
    let finite = from.iter().chain(&to).all(|c| c.is_finite());
    if !finite || !line_width.is_finite() || width == 0 || height == 0 {
        return;
    }
    let half = (line_width * 0.5).clamp(0.35, 64.0);
    let (w, h) = (f64::from(width), f64::from(height));
    // Only what can touch the image.
    let pad = half + 1.0;
    let Some((a, b)) = clip(from, to, [-pad, -pad], [w + pad, h + pad]) else {
        return;
    };
    // Walk the axis the line runs along most (`u`), one column of pixels at
    // a time, and shade the pixels of that column near the line (`v`) by
    // their distance to the segment. Every pixel is visited once.
    let steep = (b[1] - a[1]).abs() > (b[0] - a[0]).abs();
    let uv = |p: [f64; 2]| if steep { [p[1], p[0]] } else { p };
    let (a, b) = (uv(a), uv(b));
    let (du, dv) = (b[0] - a[0], b[1] - a[1]);
    let length = du.hypot(dv);
    // How far from the line's centre a column can still be inked: the
    // perpendicular reach, seen along the column. A line is at most 45°
    // from its own main axis.
    let slope = if du != 0.0 { dv / du } else { 0.0 };
    let reach = ((half + 1.0) * (1.0 + slope * slope).sqrt()).ceil() as i64 + 1;
    let (u_size, v_size) = if steep {
        (i64::from(height), i64::from(width))
    } else {
        (i64::from(width), i64::from(height))
    };
    // Clipped to the image plus the pad above, so these fit an integer.
    let first = ((a[0].min(b[0]) - pad).floor() as i64).max(0);
    let last = ((a[0].max(b[0]) + pad).floor() as i64).min(u_size - 1);
    for pu in first..=last {
        let su = pu as f64 + 0.5;
        let t = if du != 0.0 {
            ((su - a[0]) / du).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let centre = (a[1] + dv * t).floor() as i64;
        for pv in (centre - reach).max(0)..=(centre + reach).min(v_size - 1) {
            let sv = pv as f64 + 0.5;
            // Distance from the pixel centre to the segment.
            let along = if length > 0.0 {
                (((su - a[0]) * du + (sv - a[1]) * dv) / (length * length)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let distance = (su - (a[0] + du * along)).hypot(sv - (a[1] + dv * along));
            let alpha = (half + 0.5 - distance).clamp(0.0, 1.0) * f64::from(opacity) / 255.0;
            if alpha <= 0.0 {
                continue;
            }
            let (px, py) = if steep { (pv, pu) } else { (pu, pv) };
            // Both are within the image: non-negative and below its size.
            let index = (py as usize * width as usize + px as usize) * 3;
            let Some(pixel) = rgb.get_mut(index..index + 3) else {
                continue;
            };
            for (channel, &ink) in pixel.iter_mut().zip(&color) {
                let under = f64::from(*channel);
                let blended = under + (f64::from(ink) - under) * alpha;
                *channel = blended.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: Rgb = [0, 0, 0];
    const WHITE: Rgb = [255, 255, 255];

    fn canvas(w: u32, h: u32) -> Vec<u8> {
        vec![0; (w * h * 3) as usize]
    }

    fn pixel(rgb: &[u8], w: u32, x: u32, y: u32) -> Rgb {
        let i = ((y * w + x) * 3) as usize;
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    }

    #[test]
    fn a_horizontal_line_covers_its_row_and_nothing_far_from_it() {
        let (w, h) = (40, 20);
        let mut rgb = canvas(w, h);
        // Along the middle of row 10, two pixels wide.
        draw_line(&mut rgb, w, h, [5.0, 10.5], [30.0, 10.5], 2.0, WHITE, 255);
        for x in 6..30 {
            assert_eq!(pixel(&rgb, w, x, 10), WHITE, "x {x}");
            // Half covered above and below: the line is 2 wide about the
            // centre of row 10, so it reaches half-way into rows 9 and 11.
            for y in [9, 11] {
                let [v, ..] = pixel(&rgb, w, x, y);
                assert!((100..=160).contains(&v), "({x}, {y}) is {v}");
            }
            for y in [0, 5, 7, 13, 19] {
                assert_eq!(pixel(&rgb, w, x, y), BLACK, "({x}, {y})");
            }
        }
        // Round ends: nothing more than the half width past them.
        for x in [0, 1, 2, 33, 39] {
            assert_eq!(pixel(&rgb, w, x, 10), BLACK, "x {x}");
        }
    }

    #[test]
    fn a_diagonal_line_is_as_heavy_as_a_straight_one() {
        // The ink on a line is its length times its width, whatever its
        // direction: 20 long, 2 wide, plus the round ends (π · 1²).
        let expected = 20.0 * 2.0 + std::f64::consts::PI;
        let (w, h) = (60, 60);
        let (s, c) = (0.6_f64, 0.8_f64);
        for (from, to) in [
            ([10.0, 30.0], [30.0, 30.0]),
            ([30.0, 10.0], [30.0, 30.0]),
            ([10.0, 10.0], [10.0 + 20.0 * c, 10.0 + 20.0 * s]),
            ([40.0, 10.0], [40.0 - 20.0 * s, 10.0 + 20.0 * c]),
        ] {
            let mut rgb = canvas(w, h);
            draw_line(&mut rgb, w, h, from, to, 2.0, WHITE, 255);
            let ink: f64 = rgb
                .as_chunks::<3>()
                .0
                .iter()
                .map(|p| f64::from(p[0]) / 255.0)
                .sum();
            assert!(
                (ink - expected).abs() < expected * 0.06,
                "{from:?} -> {to:?}: {ink} vs {expected}"
            );
        }
    }

    #[test]
    fn lines_with_hostile_coordinates_draw_what_is_inside_and_never_panic() {
        let (w, h) = (32, 24);
        let ends = [
            -1e300,
            -1e9,
            -1.0,
            0.0,
            12.5,
            31.999,
            1e9,
            1e300,
            f64::NAN,
            f64::INFINITY,
        ];
        let mut rgb = canvas(w, h);
        for &ax in &ends {
            for &ay in &ends {
                for &b in &ends {
                    for width in [0.0, 1.5, 1e9, f64::NAN, -2.0] {
                        draw_line(&mut rgb, w, h, [ax, ay], [b, 12.0], width, WHITE, 255);
                    }
                }
            }
        }
        assert_eq!(rgb.len(), (w * h * 3) as usize);
        // A line from far outside to far outside still crosses the image.
        let mut rgb = canvas(w, h);
        draw_line(&mut rgb, w, h, [-1e9, 12.5], [1e9, 12.5], 1.0, WHITE, 255);
        for x in 0..w {
            assert_eq!(pixel(&rgb, w, x, 12), WHITE, "x {x}");
        }
        // No image, no drawing.
        draw_line(&mut [], 0, 0, [0.0, 0.0], [5.0, 5.0], 1.0, WHITE, 255);
        // A buffer shorter than its claimed size is not indexed past.
        let mut short = vec![0; 10];
        draw_line(&mut short, w, h, [0.0, 0.0], [31.0, 23.0], 3.0, WHITE, 255);
    }

    #[test]
    fn the_axes_are_drawn_from_the_origin_with_x_to_the_right_seen_from_the_front() {
        let mut camera = CameraState::default();
        camera.set_view("front").unwrap();
        let overlay = overlay(None, &camera, 400, 400, None);
        assert_eq!(overlay.lines.len(), 3);
        assert!(overlay.points.is_empty());
        let [x, y, z] = [overlay.lines[0], overlay.lines[1], overlay.lines[2]];
        assert_eq!(
            (x.kind, y.kind, z.kind),
            (LineKind::Axis(0), LineKind::Axis(1), LineKind::Axis(2))
        );
        for line in [x, y, z] {
            assert!((line.from[0] - 200.0).abs() < 1e-9 && (line.from[1] - 200.0).abs() < 1e-9);
        }
        // 6 % of the distance at the target's depth: the viewport shows
        // 2 · tan(15°) of the distance over 400 pixels.
        let length = 0.06 / (2.0 * 15.0_f64.to_radians().tan()) * 400.0;
        assert!((x.to[0] - (200.0 + length)).abs() < 1e-9, "{x:?}");
        assert!((x.to[1] - 200.0).abs() < 1e-9);
        assert!((z.to[1] - (200.0 - length)).abs() < 1e-9, "{z:?}");
        assert!((z.to[0] - 200.0).abs() < 1e-9);
        // Y points away from the viewer: it stays on the middle, a little
        // foreshortened towards it.
        assert!((y.to[0] - 200.0).abs() < 1e-9 && (y.to[1] - 200.0).abs() < 1e-9);
    }
}
