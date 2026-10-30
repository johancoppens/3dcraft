//! The direct path of the viewport: the bodies and the lines over them
//! drawn by the GPU renderer on the window's own wgpu device
//! ([`craft3d_gpu::Onscreen`]), into a texture egui shows as it is.
//! Nothing is read back, the picture is multisampled, and every frame is
//! the sharp one: the draw costs about a millisecond, so there is no
//! preview and no render thread on this path.
//!
//! The window hands the shell its [`egui_wgpu::RenderState`] (device,
//! queue, adapter and egui's renderer, where the texture is registered)
//! once, at creation (the browser's page too: WebGPU, or WebGL2). A host
//! without one (tests, the `snapshot` example) never gets here: the shell
//! then renders as before, with a renderer of its own that reads back, or
//! on the CPU.

use crate::backend::{Backend, RendererReport};
use crate::state::CameraState;
use crate::theme::Tokens;
use crate::viewport::{
    self, AXIS_LENGTH, LineKind, MAX_SIDE, MIN_SIDE, Marks, OUTLINE_FAR, OUTLINE_NEAR,
    OUTLINE_SLOPE, OUTLINE_STEP, depth_slack,
};
use craft3d_engine::Scene;
use craft3d_gpu::{
    DrawOptions, EdgeLook, EdgeSegment, Fill, GpuError, GpuLines, GpuMesh, Onscreen, OverlayParams,
    Segment,
};
use craft3d_math::Vec3;
use craft3d_render::{Highlight, RenderParams};
use egui_wgpu::wgpu;
use std::sync::Arc;
// The browser has no clock in `std` (`std::time::Instant::now` panics
// there).
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

/// A picture read back from the device, for tests: width, height and 3
/// bytes per pixel.
pub type ReadBack = (u32, u32, Vec<u8>);

/// The density of the edges per tile, read back for tests: columns, rows
/// and the means, row-major.
pub type EdgeTiles = (u32, u32, Vec<f32>);

/// What one drawn frame was.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectFrame {
    pub width: u32,
    pub height: u32,
    /// Samples per pixel.
    pub msaa: u32,
    pub triangles: usize,
    /// The time this thread spent on the frame (uploading what changed,
    /// encoding and submitting), not the GPU's own time.
    pub milliseconds: f64,
}

/// Why a frame was not drawn on this path.
#[derive(Clone, Debug, PartialEq)]
pub enum DirectFailure {
    /// The CPU renderer would refuse it too (the camera, the mesh).
    Frame(String),
    /// This frame does not fit the device; the next one may.
    ThisFrame(String),
    /// The device is of no more use: the other paths from now on.
    ForGood(String),
}

/// What is on the GPU for one scene revision at one tolerance.
struct Uploaded {
    revision: Option<u64>,
    /// How the scene was sampled ([`Scene::sampling`]).
    tolerance: Option<(f64, Option<f64>)>,
    selected: Option<String>,
    tokens: Tokens,
    scale: f32,
    marks: Marks,
    /// Whether the lines are the ones over see-through bodies.
    see_through: bool,
    /// The curvature map's values uploaded with the mesh
    /// (`ViewAnalysis::values_key`).
    values: usize,
    mesh: GpuMesh,
    /// What the mesh's parts are, when it was uploaded in parts (an
    /// assembly): a scene with the same parts is drawn by moving them.
    parts: Option<crate::viewport::PartsKey>,
    lines: GpuLines,
    triangles: usize,
}

/// What the texture shows.
#[derive(Clone, PartialEq)]
struct Shown {
    revision: Option<u64>,
    /// How the scene was sampled ([`Scene::sampling`]).
    tolerance: Option<(f64, Option<f64>)>,
    camera: CameraState,
    size: [u32; 2],
    selected: Option<String>,
    tokens: Tokens,
    scale: f32,
    highlight: Highlight,
    marks: Marks,
    opacity: f64,
    /// The studio look: its environment (by address) and options.
    look: Option<(usize, craft3d_render::studio::Look)>,
    /// The surface analysis and its values (by address).
    analysis: Option<craft3d_render::analysis::Analysis>,
    values: usize,
}

pub struct Direct {
    onscreen: Onscreen,
    device: wgpu::Device,
    egui: Arc<egui::mutex::RwLock<egui_wgpu::Renderer>>,
    adapter: String,
    uploaded: Option<Uploaded>,
    /// The egui texture over the shown view, and the view's size.
    texture: Option<(egui::TextureId, [u32; 2])>,
    shown: Option<Shown>,
    /// The largest side the device draws.
    largest: u32,
    /// The faces the frames show highlighted.
    highlight: Highlight,
    /// The edges the frames point at.
    marks: Marks,
    /// How opaque the bodies are (`RenderParams::opacity`). A device that
    /// cannot draw see-through bodies refuses a frame below 1
    /// (`GpuError::SeeThrough`), which is then drawn another way.
    opacity: f64,
    /// The studio look, or `None` for the plain one.
    studio: Option<craft3d_gpu::Studio>,
    /// The surface analysis of the frames, or `None`.
    analysis: Option<Arc<crate::viewport::ViewAnalysis>>,
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no details".to_string())
}

/// The size the picture is drawn at for a viewport of `size` pixels: the
/// viewport's own, unless a side exceeds what the device or the renderer
/// takes, when the picture keeps its shape within the limit and is
/// stretched over the viewport.
pub fn direct_size(size: [u32; 2], largest: u32) -> [u32; 2] {
    let limit = largest.clamp(MIN_SIDE, MAX_SIDE);
    let [w, h] = size.map(|side| side.max(MIN_SIDE));
    let longest = f64::from(w.max(h));
    let factor = (f64::from(limit) / longest).min(1.0);
    let fit = |side: u32| (f64::from(side) * factor).floor() as u32;
    [fit(w).clamp(MIN_SIDE, limit), fit(h).clamp(MIN_SIDE, limit)]
}

/// The lines over the picture as the GPU draws them: what
/// `viewport::overlay` lists, in the same order (the edges of the bodies,
/// the world axes over them, the sketches, their points), with the
/// theme's colours and widths at `scale` pixels per point.
pub fn segments(
    scene: Option<&Scene>,
    selected: Option<&str>,
    tokens: &Tokens,
    scale: f32,
) -> Vec<Segment> {
    segments_marked(scene, selected, tokens, scale, &Marks::default(), false)
}

/// [`segments`] with `marks` drawn over the lines and the picture cut open
/// by the section of `marks`, as
/// `viewport::overlay_see_through` lists them: the lines after the
/// sketches' lines, the dots after their points. With `see_through` (the
/// bodies are see-through) the hidden part of a body's edge is drawn
/// dimmed, like a hidden sketch, where otherwise it is left out.
pub fn segments_marked(
    scene: Option<&Scene>,
    selected: Option<&str>,
    tokens: &Tokens,
    scale: f32,
    marks: &Marks,
    see_through: bool,
) -> Vec<Segment> {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale.min(16.0)
    } else {
        1.0
    };
    let mut out = Vec::new();
    // In a picture that is cut open an edge is drawn where it is kept, and
    // the outline of the cut is drawn as edges. A body's edges are drawn
    // in shades of its own colour (`Tokens::edge_colors`).
    for (body, color) in scene
        .into_iter()
        .filter(|_| !marks.hide_edges)
        .flat_map(viewport::body_colors)
    {
        let colors = tokens.edge_colors(color.unwrap_or(tokens.body));
        for edge in body.edges.iter() {
            let extent = edge.split_first().map(|(&first, rest)| {
                rest.iter()
                    .fold([first, first], |[lo, hi], p| [lo.min(*p), hi.max(*p)])
            });
            for pair in edge.windows(2) {
                if let [a, b] = *pair
                    && let Some([a, b]) = marks.kept(a, b)
                {
                    out.push(Segment {
                        from: a,
                        to: b,
                        color: colors.inner,
                        width: tokens.edge_width * scale,
                        tested: true,
                        dimmed: see_through,
                        axis: false,
                        edge: Some(EdgeSegment {
                            outline_color: colors.silhouette,
                            outline_width: tokens.edge_silhouette_width * scale,
                            extent,
                        }),
                    });
                }
            }
        }
    }
    let mut line = |a: Vec3, b: Vec3, kind: LineKind, tested: bool, dimmed: bool, axis: bool| {
        out.push(Segment {
            from: a,
            to: b,
            color: kind.color(tokens),
            width: kind.width(tokens) * scale,
            tested,
            dimmed,
            axis,
            edge: None,
        });
    };
    for &[a, b] in marks.outline() {
        line(a, b, LineKind::Edge, true, see_through, false);
    }
    for (axis, direction) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate() {
        line(
            Vec3::ZERO,
            direction,
            LineKind::Axis(axis),
            false,
            false,
            true,
        );
    }
    for [a, b] in marks.datum_lines() {
        line(a, b, LineKind::Datum, true, true, false);
    }
    for trace in &marks.traces {
        for pair in trace.windows(2) {
            if let [a, b] = *pair {
                line(a, b, LineKind::Trace, true, true, false);
            }
        }
    }
    for comb in &marks.combs {
        for pair in comb.windows(2) {
            if let [a, b] = *pair {
                line(a, b, LineKind::Comb, true, true, false);
            }
        }
    }
    for sketch in scene
        .into_iter()
        .flat_map(|scene| &scene.sketches)
        .filter(|_| !marks.hide_sketches)
    {
        let chosen = selected == Some(sketch.feature.as_str());
        for polyline in &sketch.polylines {
            let kind = match (polyline.construction, chosen) {
                (true, _) => LineKind::Construction,
                (false, true) => LineKind::Selected,
                (false, false) => LineKind::Sketch,
            };
            for pair in polyline.points.windows(2) {
                if let [a, b] = *pair {
                    line(a, b, kind, true, true, false);
                }
            }
        }
    }
    for &[a, b] in &marks.lines {
        if let Some([a, b]) = marks.kept(a, b) {
            line(a, b, LineKind::Highlight, true, true, false);
        }
    }
    for sketch in scene
        .into_iter()
        .flat_map(|scene| &scene.sketches)
        .filter(|_| !marks.hide_sketches)
    {
        let kind = if selected == Some(sketch.feature.as_str()) {
            LineKind::Selected
        } else {
            LineKind::Sketch
        };
        for point in &sketch.points {
            out.push(Segment {
                from: *point,
                to: *point,
                color: kind.color(tokens),
                width: tokens.point_radius * 2.0 * scale,
                tested: true,
                dimmed: true,
                axis: false,
                edge: None,
            });
        }
    }
    for point in marks.datum_points() {
        out.push(Segment {
            from: point,
            to: point,
            color: LineKind::Datum.color(tokens),
            width: LineKind::Datum.dot_radius(tokens) * 2.0 * scale,
            tested: true,
            dimmed: true,
            axis: false,
            edge: None,
        });
    }
    for point in marks
        .points
        .iter()
        .filter(|point| marks.kept(**point, **point).is_some())
    {
        out.push(Segment {
            from: *point,
            to: *point,
            color: LineKind::Highlight.color(tokens),
            width: LineKind::Highlight.dot_radius(tokens) * 2.0 * scale,
            tested: true,
            dimmed: true,
            axis: false,
            edge: None,
        });
    }
    out
}

/// How the GPU draws the edges of bodies at `scale` pixels per point: the
/// rules of the CPU viewport ([`viewport::surface_steps_back`],
/// [`viewport::fade_short`], [`viewport::edge_fade`],
/// [`viewport::EdgeDensity`]) with the theme's numbers.
pub fn edge_look(tokens: &Tokens, scale: f32) -> EdgeLook {
    let scale = if scale.is_finite() && scale > 0.0 {
        f64::from(scale.min(16.0))
    } else {
        1.0
    };
    EdgeLook {
        outline_near: OUTLINE_NEAR,
        outline_far: OUTLINE_FAR,
        outline_step: OUTLINE_STEP,
        outline_slope: OUTLINE_SLOPE,
        fade_short: f64::from(tokens.edge_fade_short) * scale,
        fade_long: f64::from(tokens.edge_fade_long) * scale,
        dense_cap: f64::from(tokens.edge_dense_cap),
        fade_floor: f64::from(tokens.edge_fade_floor),
        tile: viewport::tile_pixels(tokens, scale),
        soft: false,
    }
}

/// The squares of the datum planes of `marks` as the GPU fills them under
/// the lines, in the theme's datum colour and fill opacity: what
/// `viewport::fill_datum_planes` draws on the CPU.
pub fn fills(marks: &Marks, tokens: &Tokens) -> Vec<Fill> {
    marks
        .datum_planes()
        .into_iter()
        .map(|(center, u, v)| Fill {
            center,
            u,
            v,
            color: tokens.datum,
            opacity: tokens.datum_fill_opacity,
        })
        .collect()
}

/// A render state without a window, as eframe would make one: the
/// adapter egui-wgpu picks, a device on it, and egui's renderer for an
/// `Rgba8Unorm` target. For tests and examples that drive the shell
/// headlessly on the direct path; the window gets its state from eframe.
///
/// wgpu's debug names are left off: with them the Vulkan loader walks its
/// device list without a lock, and another thread making a device at
/// that moment crashes the process (see `craft3d_gpu`).
#[cfg(not(target_arch = "wasm32"))]
pub fn headless_render_state() -> Result<egui_wgpu::RenderState, String> {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup
        .instance_descriptor
        .flags
        .remove(wgpu::InstanceFlags::DEBUG);
    let config = egui_wgpu::WgpuConfiguration {
        wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
        ..egui_wgpu::WgpuConfiguration::default()
    };
    let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let instance = pollster::block_on(config.wgpu_setup.new_instance());
        pollster::block_on(egui_wgpu::RenderState::create(
            &config,
            &instance,
            None,
            egui_wgpu::RendererOptions::default(),
        ))
    }));
    match made {
        Ok(Ok(state)) => Ok(state),
        Ok(Err(e)) => Err(format!("no render state without a window: {e}")),
        Err(payload) => Err(format!(
            "the graphics driver failed while a render state was made: {}",
            panic_text(payload.as_ref())
        )),
    }
}

fn classify(error: GpuError) -> DirectFailure {
    let text = error.to_string();
    match error {
        GpuError::Render(_) => DirectFailure::Frame(text),
        GpuError::TargetTooLarge { .. }
        | GpuError::MeshTooLarge { .. }
        | GpuError::TooManyLines { .. }
        | GpuError::OutOfRange(_)
        | GpuError::SeeThrough(_)
        | GpuError::Section(_) => DirectFailure::ThisFrame(text),
        GpuError::NoAdapter(_)
        | GpuError::Device(_)
        | GpuError::ForeignMesh
        | GpuError::NothingDrawn
        | GpuError::Readback(_)
        | GpuError::Unsupported(_) => DirectFailure::ForGood(text),
    }
}

impl Direct {
    /// The viewport renderer on the window's device, or why there is
    /// none. A panic inside the graphics stack is a reason like any other.
    pub fn new(state: &egui_wgpu::RenderState) -> Result<Self, String> {
        let info = state.adapter.get_info();
        let adapter = format!(
            "{} ({:?}, {:?}, {})",
            info.name,
            info.backend,
            info.device_type,
            format!("{} {}", info.driver, info.driver_info).trim()
        );
        let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Onscreen::new(&state.device, &state.queue, &state.adapter)
        }))
        .unwrap_or_else(|payload| {
            Err(GpuError::Device(format!(
                "the graphics driver failed while the viewport renderer was made: {}",
                panic_text(payload.as_ref())
            )))
        });
        let onscreen = made.map_err(|e| format!("{e} (on {adapter})"))?;
        let largest = state.device.limits().max_texture_dimension_2d;
        Ok(Self {
            onscreen,
            device: state.device.clone(),
            egui: Arc::clone(&state.renderer),
            adapter,
            uploaded: None,
            texture: None,
            shown: None,
            largest,
            highlight: Highlight::NONE,
            marks: Marks::default(),
            opacity: 1.0,
            studio: None,
            analysis: None,
        })
    }

    pub fn samples(&self) -> u32 {
        self.onscreen.samples()
    }

    pub fn report(&self) -> RendererReport {
        RendererReport {
            backend: Backend::Gpu,
            adapter: Some(self.adapter.clone()),
            fallback: None,
            direct: true,
            msaa: self.onscreen.samples(),
        }
    }

    /// Which faces the frames drawn from now on show highlighted, as
    /// [`crate::backend::BodyRenderer::set_highlight`].
    pub fn set_highlight(&mut self, highlight: Highlight) {
        self.highlight = highlight;
    }

    /// The edges the frames drawn from now on point at, as
    /// [`crate::backend::BodyRenderer::set_marks`].
    pub fn set_marks(&mut self, marks: Marks) {
        self.marks = marks;
    }

    /// How opaque the bodies of the frames drawn from now on are, as
    /// [`crate::backend::BodyRenderer::set_opacity`].
    pub fn set_opacity(&mut self, opacity: f64) {
        self.opacity = opacity;
    }

    /// The look of the frames drawn from now on, as
    /// [`crate::backend::BodyRenderer::set_studio`].
    pub fn set_studio(&mut self, studio: Option<craft3d_gpu::Studio>) {
        self.studio = studio;
    }

    /// The surface analysis of the frames drawn from now on, as
    /// [`crate::backend::BodyRenderer::set_analysis`]: drawn in the plain
    /// look in place of the shading.
    pub fn set_analysis(&mut self, analysis: Option<Arc<crate::viewport::ViewAnalysis>>) {
        self.analysis = analysis;
    }

    /// As on a device that cannot draw see-through bodies: from now on a
    /// see-through frame is refused here and drawn another way. For tests
    /// of that fallback.
    pub fn without_see_through(&mut self) {
        self.onscreen.without_see_through();
    }

    /// As on a device that cannot draw a cut-open picture: from now on a
    /// frame with a section is refused here and drawn another way. For
    /// tests of that fallback.
    pub fn without_section(&mut self) {
        self.onscreen.without_section();
    }

    /// Whether the picture on screen will still improve when it is drawn
    /// again with the same inputs (the studio look's accumulation while
    /// the camera stands still): the shell then asks for another frame.
    pub fn converging(&self) -> bool {
        self.onscreen.converging()
    }

    /// The egui texture that shows the picture, once one was drawn.
    pub fn texture_id(&self) -> Option<egui::TextureId> {
        self.texture.map(|(id, _)| id)
    }

    /// Brings the texture up to date with what the viewport should show,
    /// when it is not already. `Ok(None)` when the picture was current.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        scene: Option<&Scene>,
        camera: &CameraState,
        size: [u32; 2],
        selected: Option<&str>,
        tokens: &Tokens,
        scale: f32,
    ) -> Result<Option<DirectFrame>, DirectFailure> {
        let wanted = Shown {
            revision: scene.map(|s| s.revision),
            tolerance: scene.map(Scene::sampling),
            camera: *camera,
            size,
            selected: selected.map(str::to_string),
            tokens: *tokens,
            scale,
            highlight: self.highlight,
            marks: self.marks.clone(),
            opacity: self.opacity,
            look: self
                .studio
                .as_ref()
                .map(|s| (Arc::as_ptr(&s.env) as usize, s.look)),
            analysis: self.analysis.as_ref().and_then(|a| a.analysis),
            values: self.analysis.as_ref().map_or(0, |a| a.values_key()),
        };
        // The same picture again: nothing to draw, unless the studio look
        // is still accumulating frames (`Onscreen::converging`).
        if self.shown.as_ref() == Some(&wanted)
            && self.texture.is_some()
            && !self.onscreen.converging()
        {
            return Ok(None);
        }
        let start = Instant::now();
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.draw(scene, &wanted)));
        match outcome {
            Ok(Ok(triangles)) => {
                self.register()?;
                let [width, height] = self.texture.map_or(size, |(_, size)| size);
                self.shown = Some(wanted);
                Ok(Some(DirectFrame {
                    width,
                    height,
                    msaa: self.onscreen.samples(),
                    triangles,
                    milliseconds: start.elapsed().as_secs_f64() * 1000.0,
                }))
            }
            Ok(Err(failure)) => {
                self.shown = None;
                Err(failure)
            }
            Err(payload) => {
                self.shown = None;
                Err(DirectFailure::ForGood(format!(
                    "the graphics driver failed while rendering: {}",
                    panic_text(payload.as_ref())
                )))
            }
        }
    }

    /// Uploads what changed and draws. Returns the number of triangles.
    fn draw(&mut self, scene: Option<&Scene>, wanted: &Shown) -> Result<usize, DirectFailure> {
        let mut same_mesh = self.uploaded.as_ref().is_some_and(|u| {
            u.revision == wanted.revision
                && u.tolerance == wanted.tolerance
                && u.values == wanted.values
        });
        // Through see-through bodies the hidden parts of their edges are
        // drawn: other lines, uploaded when the bodies become see-through
        // or opaque again (not while the slider moves in between).
        let see_through = RenderParams {
            opacity: wanted.opacity,
            ..RenderParams::default()
        }
        .opacity()
            < 1.0;
        let same_lines = self.uploaded.as_ref().is_some_and(|u| {
            u.revision == wanted.revision
                && u.tolerance == wanted.tolerance
                && u.selected == wanted.selected
                && u.tokens == wanted.tokens
                && u.scale == wanted.scale
                && u.marks == wanted.marks
                && u.see_through == see_through
                && u.values == wanted.values
        });
        // Another moment of the same parts (an assembly's motion): they are
        // moved, a few matrices, not uploaded again. (The lines over them,
        // its edges, are uploaded below.)
        // Values of an analysis (draft, curvature) follow the poses: such a
        // scene is uploaded whole.
        let parts = scene
            .filter(|_| wanted.values == 0)
            .and_then(crate::viewport::scene_parts);
        // Moved now: the upload is the new moment's only once its lines
        // are too (committed below; dropped when they fail).
        let mut moved = None;
        if !same_mesh
            && let (Some(parts), Some(scene), Some(uploaded)) =
                (&parts, scene, self.uploaded.as_mut())
            && uploaded.parts.as_ref() == Some(&parts.key)
        {
            self.onscreen
                .set_poses(
                    &mut uploaded.mesh,
                    &parts.poses,
                    craft3d_gpu::drawn_bounds(&scene.mesh),
                )
                .map_err(classify)?;
            moved = Some(scene.mesh.triangles.len());
            same_mesh = true;
        }
        if !same_lines {
            let lines = self.onscreen.upload_overlay(
                &segments_marked(
                    scene,
                    wanted.selected.as_deref(),
                    &wanted.tokens,
                    wanted.scale,
                    &wanted.marks,
                    see_through,
                ),
                &fills(&wanted.marks, &wanted.tokens),
            );
            let lines = match lines {
                Ok(lines) => lines,
                Err(error) => {
                    // The parts stand at the new moment, the lines at the
                    // old one: neither is kept as either.
                    if moved.is_some() {
                        self.uploaded = None;
                    }
                    return Err(classify(error));
                }
            };
            if same_mesh && let Some(uploaded) = &mut self.uploaded {
                if let Some(triangles) = moved {
                    uploaded.revision = wanted.revision;
                    uploaded.tolerance = wanted.tolerance;
                    uploaded.triangles = triangles;
                }
                uploaded.lines = lines;
                uploaded.see_through = see_through;
                uploaded.selected.clone_from(&wanted.selected);
                uploaded.tokens = wanted.tokens;
                uploaded.scale = wanted.scale;
                uploaded.marks.clone_from(&wanted.marks);
            } else {
                // The old copy goes first: twice the model would not fit
                // a small device.
                self.uploaded = None;
                let empty = craft3d_tess::Mesh::default();
                let mesh = scene.map_or(&empty, |s| &s.mesh);
                // The curvature map's own mesh, when it has one.
                let mesh = match self.analysis.as_ref().filter(|_| wanted.values != 0) {
                    Some(analysis) => analysis.mesh_or(mesh),
                    None => mesh,
                };
                let no_colors = craft3d_render::FaceColors::new();
                let colors = scene.map_or(&no_colors, |s| &s.face_colors);
                let values = self
                    .analysis
                    .as_ref()
                    .filter(|_| wanted.values != 0)
                    .map(|a| Arc::clone(&a.values))
                    .unwrap_or_default();
                let uploaded = match &parts {
                    Some(parts) => self.onscreen.upload_parts(
                        &parts.parts,
                        colors,
                        craft3d_gpu::drawn_bounds(mesh),
                    ),
                    None => self.onscreen.upload_analysed(mesh, colors, &values),
                }
                .map_err(classify)?;
                self.uploaded = Some(Uploaded {
                    parts: parts.map(|p| p.key),
                    revision: wanted.revision,
                    tolerance: wanted.tolerance,
                    selected: wanted.selected.clone(),
                    tokens: wanted.tokens,
                    scale: wanted.scale,
                    marks: wanted.marks.clone(),
                    see_through,
                    values: wanted.values,
                    mesh: uploaded,
                    lines,
                    triangles: mesh.triangles.len(),
                });
            }
        }
        let uploaded = self
            .uploaded
            .as_ref()
            .ok_or_else(|| DirectFailure::ForGood("nothing was uploaded".to_string()))?;
        let [width, height] = direct_size(wanted.size, self.largest);
        let params = RenderParams {
            width,
            height,
            ssaa: 1,
            background: wanted.tokens.viewport_background,
            color: wanted.tokens.body,
            highlight: wanted.highlight,
            opacity: wanted.opacity,
            section: wanted.marks.section(),
            analysis: wanted.analysis,
        };
        let overlay = OverlayParams {
            slack: depth_slack(Backend::Gpu, scene, &wanted.camera),
            hidden_opacity: wanted.tokens.hidden_opacity,
            axis_length: wanted.camera.distance * AXIS_LENGTH,
            edges: EdgeLook {
                soft: wanted.marks.soft_edges,
                ..edge_look(&wanted.tokens, wanted.scale)
            },
        };
        // An analysis is drawn in the plain look.
        self.onscreen
            .set_studio(self.studio.clone().filter(|_| wanted.analysis.is_none()));
        self.onscreen
            .draw(
                &uploaded.mesh,
                &uploaded.lines,
                &wanted.camera.to_render(),
                &params,
                &overlay,
                DrawOptions::default(),
            )
            .map_err(classify)?;
        Ok(uploaded.triangles)
    }

    /// Registers the shown view with egui, or again when it is a new one.
    fn register(&mut self) -> Result<(), DirectFailure> {
        let (Some(view), Some((width, height))) = (self.onscreen.view(), self.onscreen.size())
        else {
            return Err(DirectFailure::ForGood("nothing was drawn".to_string()));
        };
        let size = [width, height];
        if self.texture.is_some_and(|(_, shown)| shown == size) {
            return Ok(());
        }
        let mut egui = self.egui.write();
        let registered =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match self.texture {
                Some((id, _)) => {
                    egui.update_egui_texture_from_wgpu_texture(
                        &self.device,
                        view,
                        wgpu::FilterMode::Linear,
                        id,
                    );
                    id
                }
                None => egui.register_native_texture(&self.device, view, wgpu::FilterMode::Linear),
            }));
        match registered {
            Ok(id) => {
                self.texture = Some((id, size));
                Ok(())
            }
            Err(payload) => Err(DirectFailure::ForGood(format!(
                "the viewport texture could not be registered: {}",
                panic_text(payload.as_ref())
            ))),
        }
    }

    /// The density of the edges per tile of the picture as shown, read
    /// back: columns, rows, means. For tests; the window never calls it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_edge_density(&mut self) -> Result<EdgeTiles, String> {
        self.onscreen.read_edge_density().map_err(|e| e.to_string())
    }

    /// The picture as shown, read back: for tests. The window never
    /// calls it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_rgb(&self) -> Result<ReadBack, String> {
        let (width, height) = self.onscreen.size().ok_or("nothing was drawn")?;
        let rgb = self.onscreen.read_rgb().map_err(|e| e.to_string())?;
        Ok((width, height, rgb))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drawn_size_is_the_viewport_within_the_limits() {
        assert_eq!(direct_size([1280, 800], 8192), [1280, 800]);
        assert_eq!(direct_size([8000, 2000], 8192), [4096, 1024]);
        assert_eq!(direct_size([1000, 4000], 2048), [512, 2048]);
        assert_eq!(direct_size([0, 0], 8192), [MIN_SIDE, MIN_SIDE]);
        assert_eq!(direct_size([5000, 5000], 0), [MIN_SIDE, MIN_SIDE]);
        assert_eq!(direct_size([u32::MAX, 1], 8192), [4096, MIN_SIDE]);
    }

    #[test]
    fn what_the_cpu_refuses_too_is_an_error_and_the_rest_falls_back() {
        assert!(matches!(
            classify(GpuError::Render(craft3d_render::RenderError::InvalidSsaa(
                9
            ))),
            DirectFailure::Frame(_)
        ));
        for this_frame in [
            GpuError::TargetTooLarge {
                side: 16384,
                limit: 8192,
            },
            GpuError::MeshTooLarge { triangles: 1 << 30 },
            GpuError::TooManyLines { segments: 1 << 30 },
            GpuError::OutOfRange("the camera"),
            GpuError::SeeThrough("no such pipeline".into()),
        ] {
            assert!(matches!(classify(this_frame), DirectFailure::ThisFrame(_)));
        }
        for for_good in [
            GpuError::Device("lost".into()),
            GpuError::ForeignMesh,
            GpuError::NothingDrawn,
            GpuError::Unsupported("that"),
        ] {
            assert!(matches!(classify(for_good), DirectFailure::ForGood(_)));
        }
    }

    #[test]
    fn without_a_scene_the_lines_are_the_axes() {
        let tokens = Tokens::default();
        let lines = segments(None, None, &tokens, 2.0);
        assert_eq!(lines.len(), 3);
        for (axis, line) in lines.iter().enumerate() {
            assert!(line.axis && !line.tested);
            assert_eq!(line.color, tokens.axes[axis]);
            assert_eq!(line.width, tokens.axis_width * 2.0);
        }
        // Through see-through bodies a body's edges are dimmed where
        // hidden; the other lines are as ever.
        assert!(
            segments_marked(None, None, &tokens, 2.0, &Marks::default(), true)
                .iter()
                .all(|line| line.axis && !line.dimmed)
        );
        // A hostile scale is one pixel per point.
        assert_eq!(
            segments(None, None, &tokens, f32::NAN)[0].width,
            tokens.axis_width
        );
    }
}
