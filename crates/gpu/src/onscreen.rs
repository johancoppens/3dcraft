//! The renderer of a viewport: on a device someone else owns (the
//! window's), into a texture that is shown as it is. Nothing is read back
//! for display; the picture is multisampled, and the lines over it (the
//! edges of the bodies, sketches, the world axes, sketch points) are drawn
//! on the GPU with the depth test the CPU viewport does on its overlay.
//!
//! Three passes per frame of opaque bodies:
//!
//! 1. **Scene.** The mesh, with `scene.wgsl`, into a multisampled colour
//!    target and depth buffer ([`Onscreen::samples`] samples; one when the
//!    adapter has no multisampling for the formats). The shading is the
//!    one of the readback renderer, once per pixel at its centre; the
//!    samples only decide coverage.
//! 2. **Edge density** (only when the lines hold edges of bodies): the
//!    edges in sight inked at their own width into a texture of one
//!    channel and summed per pixel (`lines.wgsl`, `fs_ink`), then averaged
//!    per tile (`edge_tiles.wgsl`): how crowded with edges each part of
//!    the picture is, which fades the edges there in pass 3.
//! 3. **Lines.** The segments of a [`GpuLines`], with `lines.wgsl`, into
//!    the same colour target, each as a quad in screen space inked by the
//!    distance of a pixel's centre to the segment (the CPU's `draw_line`)
//!    and, when `tested`, hidden where the surface reconstructed from the
//!    samples around the segment's point is nearer than that point (the
//!    CPU viewport's `surface_hides`, in `depth_test.wgsl`), read from the
//!    depth buffer of pass 1: a hidden edge is left out, a hidden
//!    sketch is drawn with the hidden opacity. The pass resolves the
//!    samples into the shown texture.
//! 4. **Capture** (only when asked): face ids and distances at one sample
//!    per pixel, for picking and for the tests that compare the picture
//!    with the CPU renderer's.
//!
//! **See-through bodies** (`RenderParams::opacity` below 1; `peel.rs`) take
//! other passes before the lines: the front into a multisampled target of
//! its own, with its depth buffer and, per sample, a key of the triangle
//! that is in front there; the scene again, each fragment kept only on the
//! samples where its triangle is not the front (`peel_samples.wgsl`),
//! which leaves the nearest surface behind; and a pass that blends the two
//! per sample with the CPU renderer's blend and writes the mean of the
//! samples into the shown texture. The lines are then drawn straight onto
//! the shown texture, at one sample (they are inked by distance, not by
//! coverage), tested against the front's depth as ever. With one sample
//! per pixel (OpenGL) there are no keys: the second pass compares its
//! depth with the front's (`peel.wgsl`).
//!
//! **A section** (`RenderParams::section`; `cut.rs`) adds a pass before
//! the bodies that counts, per sample, the fragments that are cut away,
//! and draws the bodies with entries that leave those out and, after the
//! triangles, the cap: one triangle in the plane, kept on the samples
//! where the count is odd (`cut_samples.wgsl`; `cut_flat.wgsl` with one
//! sample per pixel). The cap is in the depth buffer, so the lines behind
//! it are hidden like behind any face. Seen through, the cap is a surface
//! of both layers. The capture counts again at one sample per pixel.
//!
//! Everything a test needs is still there: [`Onscreen::read_rgb`] reads
//! the shown picture back, [`Onscreen::read_frame`] the picture with the
//! captured ids and distances as a [`Frame`]. The window never calls them.

use crate::cut;
use crate::mesh::{GpuMesh, VERTEX_BYTES};
use crate::peel;
use crate::readback;
use crate::renderer::{
    COLOR_FORMAT, DATA_FORMAT, DEPTH_FORMAT, MESH_ATTRIBUTES, SCENE_FLOATS, Step, check_params,
    float_bytes, guarded, scene_pipeline, scene_source, scene_uniforms, target, uniform_entry,
};
use crate::{GpuError, Mesh};
use craft3d_math::{Bbox3, Vec3};
use craft3d_render::{Camera, Cut, FaceColors, Frame, NO_FACE, RenderParams};
use std::cell::Cell;
use std::sync::{Arc, Mutex, PoisonError};

/// A line over the picture, in model coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub from: Vec3,
    pub to: Vec3,
    /// sRGB bytes.
    pub color: [u8; 3],
    /// In pixels of the image. A segment without length is a dot of this
    /// diameter.
    pub width: f32,
    /// Tested against the depth buffer.
    pub tested: bool,
    /// Where it is hidden: drawn with the hidden opacity rather than left
    /// out.
    pub dimmed: bool,
    /// A world axis: `from` and `to` are multiplied by the axis length of
    /// the frame ([`OverlayParams::axis_length`]) when it is drawn, so the
    /// axes keep their size on screen without being uploaded again.
    pub axis: bool,
    /// For an edge of a body: drawn as its outline where the surface on
    /// one side steps back, faded where it is short on screen or where
    /// edges crowd ([`EdgeLook`]), and counted in how crowded they are.
    pub edge: Option<EdgeSegment>,
}

/// What a segment of an edge of a body has besides ([`Segment::edge`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeSegment {
    /// The outline's colour, sRGB bytes.
    pub outline_color: [u8; 3],
    /// The outline's width, in pixels.
    pub outline_width: f32,
    /// Two opposite corners of the box around the whole edge the segment
    /// is part of: the diagonal between them, projected, is the edge's
    /// extent on screen. `None`: the edge is never short.
    pub extent: Option<[Vec3; 2]>,
}

/// How the edges of bodies are drawn in a frame: the rules of
/// `craft3d_ui_egui::viewport` (`surface_steps_back`, `fade_short`,
/// `edge_fade`, `EdgeDensity`), with their numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeLook {
    /// Where the surface beside an edge is read, in pixels across it:
    /// near and far (`OUTLINE_NEAR`, `OUTLINE_FAR`).
    pub outline_near: f64,
    pub outline_far: f64,
    /// The step that makes an outline, in pixels' worth of the view at
    /// the edge's distance (`OUTLINE_STEP`), and the share of the
    /// surface's change per pixel allowed besides (`OUTLINE_SLOPE`).
    pub outline_step: f64,
    pub outline_slope: f64,
    /// An edge shorter on screen than `long` pixels fades, fully at
    /// `short`.
    pub fade_short: f64,
    pub fade_long: f64,
    /// Where the edges' ink covers more than `dense_cap` of a tile, each
    /// edge there keeps `dense_cap` over that share of its opacity.
    pub dense_cap: f64,
    /// The opacity a fully faded edge keeps, 0 to 1.
    pub fade_floor: f64,
    /// The side of a tile of the density, in pixels (4 to 64).
    pub tile: u32,
}

impl EdgeLook {
    /// Nothing faded, no step an outline.
    pub const PLAIN: EdgeLook = EdgeLook {
        outline_near: 1.5,
        outline_far: 3.0,
        outline_step: 1.0e30,
        outline_slope: 0.0,
        fade_short: 0.0,
        fade_long: 0.0,
        dense_cap: 1.0e30,
        fade_floor: 1.0,
        tile: 16,
    };

    /// The side of a tile, within what the passes take.
    fn tile(&self) -> u32 {
        self.tile.clamp(4, 64)
    }
}

impl Default for EdgeLook {
    fn default() -> Self {
        Self::PLAIN
    }
}

/// A translucent square over the picture and under the lines: the
/// square `center ± u ± v` in model coordinates (a datum plane), drawn
/// where the ray through a pixel's centre meets it in front of every body
/// (the lines' depth test), in `color` at `opacity`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fill {
    pub center: Vec3,
    /// Half sides.
    pub u: Vec3,
    pub v: Vec3,
    /// sRGB bytes.
    pub color: [u8; 3],
    /// 0 to 255.
    pub opacity: u8,
}

/// Largest accepted number of fills.
pub const MAX_FILLS: usize = 1 << 12;
/// The centre, the two half sides (3 × `f32` each) and the colour.
const FILL_VERTEX_BYTES: usize = 40;
/// One triangle over the whole picture.
const VERTICES_PER_FILL: usize = 3;

/// What the lines are drawn with in one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayParams {
    /// How far behind the surface under it a line may be and still count
    /// as lying on it, in model units.
    pub slack: f64,
    /// The opacity of a hidden, dimmed line, 0 to 255.
    pub hidden_opacity: u8,
    /// The length of a world axis, in model units.
    pub axis_length: f64,
    /// How the edges of bodies are drawn.
    pub edges: EdgeLook,
}

/// What a frame holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawOptions {
    /// Shade the bodies. Without it the bodies only fill the depth buffer,
    /// so the lines show what the depth test decided on the background.
    pub bodies: bool,
    /// Also draw the face ids and distances, for [`Onscreen::read_frame`].
    pub capture: bool,
}

impl Default for DrawOptions {
    fn default() -> Self {
        Self {
            bodies: true,
            capture: false,
        }
    }
}

/// Largest accepted number of segments (71 MB of instance data).
pub const MAX_SEGMENTS: usize = 1 << 20;
/// One instance per segment: two ends (3 × `f32` each), the colour
/// (`u32`), the width (`f32`), the flags (`u32`), the outline's colour
/// (`u32`) and width (`f32`), and the corners of the edge's box (3 × `f32`
/// each).
const SEGMENT_BYTES: usize = 68;
/// Two triangles, made from the vertex index.
const VERTICES_PER_SEGMENT: u32 = 6;
/// Number of `f32` in the lines uniform block (six `vec4`).
const LINES_FLOATS: usize = 24;
/// Samples the viewport asks for.
pub const WANTED_SAMPLES: u32 = 4;

const FLAG_TESTED: u32 = 1;
const FLAG_DIMMED: u32 = 2;
const FLAG_EDGE: u32 = 4;
const FLAG_EXTENT: u32 = 8;

/// The ink of the edges, summed per pixel by blending.
const INK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
/// The density of the edges per tile, and the distances the lines' depth
/// test reads: the bits of `f32` (`R32Float` does not render everywhere).
const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const DISTANCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

/// The segments on the GPU, made by [`Onscreen::upload_lines`]. They
/// belong to the renderer that made them.
pub struct GpuLines {
    buffer: Option<wgpu::Buffer>,
    /// One per segment drawn.
    instances: u32,
    /// How many of them are edges of bodies.
    edges: usize,
    /// What the positions are relative to.
    origin: Vec3,
    /// The axes, with the byte offset of each in the buffer: written again
    /// when the axis length changes.
    axes: Vec<(u64, Segment)>,
    /// The axis length the axes were last written with.
    axis_length: Cell<f64>,
    segments: usize,
    /// The fills, drawn before the segments; `None` without any.
    fills: Option<wgpu::Buffer>,
    fill_vertices: u32,
    owner: Arc<()>,
}

impl GpuLines {
    /// Number of segments that are drawn (those given, less the ones with
    /// an end the GPU cannot represent).
    pub fn segment_count(&self) -> usize {
        self.segments
    }

    /// Number of fills that are drawn.
    pub fn fill_count(&self) -> usize {
        self.fill_vertices as usize / VERTICES_PER_FILL
    }
}

/// The three vertices of a fill, positions relative to `origin`; `None`
/// for a fill with a number the GPU cannot represent.
fn pack_fill(fill: &Fill, origin: Vec3) -> Option<[u8; FILL_VERTEX_BYTES * VERTICES_PER_FILL]> {
    let center = narrow(fill.center - origin)?;
    let u = narrow(fill.u)?;
    let v = narrow(fill.v)?;
    let [r, g, b] = fill.color;
    let color =
        u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16) | (u32::from(fill.opacity) << 24);
    let mut out = [0_u8; FILL_VERTEX_BYTES * VERTICES_PER_FILL];
    for vertex in out.as_chunks_mut::<FILL_VERTEX_BYTES>().0.iter_mut() {
        let floats = [
            center[0], center[1], center[2], u[0], u[1], u[2], v[0], v[1], v[2],
        ];
        let mut bytes = float_bytes(&floats);
        bytes.extend_from_slice(&color.to_le_bytes());
        vertex.copy_from_slice(&bytes);
    }
    Some(out)
}

fn narrow(v: Vec3) -> Option<[f32; 3]> {
    // Deliberate narrowing at the GPU boundary; a value that overflows
    // becomes infinite and is rejected.
    let out = [v.x as f32, v.y as f32, v.z as f32];
    out.iter().all(|c| c.is_finite()).then_some(out)
}

fn pack_color([r, g, b]: [u8; 3]) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16) | (255 << 24)
}

/// The instance of one segment, or `None` when it cannot be drawn. An
/// edge whose box does not fit in `f32` is drawn as one that is never
/// short.
fn pack_segment(segment: &Segment, origin: Vec3, axis_length: f64) -> Option<[u8; SEGMENT_BYTES]> {
    let scale = if segment.axis { axis_length } else { 1.0 };
    let (from, to) = (segment.from * scale, segment.to * scale);
    if !(from.is_finite() && to.is_finite() && segment.width.is_finite()) {
        return None;
    }
    let a = narrow(from - origin)?;
    let b = narrow(to - origin)?;
    let edge = segment.edge;
    let extent = edge
        .and_then(|edge| edge.extent)
        .and_then(|[low, high]| Some((narrow(low - origin)?, narrow(high - origin)?)));
    let flags = if segment.tested { FLAG_TESTED } else { 0 }
        | if segment.dimmed { FLAG_DIMMED } else { 0 }
        | if edge.is_some() { FLAG_EDGE } else { 0 }
        | if extent.is_some() { FLAG_EXTENT } else { 0 };
    let width = segment.width.max(0.0);
    let outline_width = edge.map_or(width, |edge| {
        if edge.outline_width.is_finite() {
            edge.outline_width.max(0.0)
        } else {
            width
        }
    });
    let outline_color = pack_color(edge.map_or(segment.color, |edge| edge.outline_color));
    let (low, high) = extent.unwrap_or(([0.0; 3], [0.0; 3]));
    let mut bytes = float_bytes(&[a[0], a[1], a[2], b[0], b[1], b[2]]);
    bytes.extend_from_slice(&pack_color(segment.color).to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&flags.to_le_bytes());
    bytes.extend_from_slice(&outline_color.to_le_bytes());
    bytes.extend_from_slice(&outline_width.to_le_bytes());
    bytes.extend_from_slice(&float_bytes(&[
        low[0], low[1], low[2], high[0], high[1], high[2],
    ]));
    let mut out = [0_u8; SEGMENT_BYTES];
    if bytes.len() != SEGMENT_BYTES {
        return None;
    }
    out.copy_from_slice(&bytes);
    Some(out)
}

/// The depth buffer of a draw, read back ([`Onscreen::read_depth`]):
/// the reversed depth of `scene.wgsl` per pixel, with the numbers that
/// turn it back into a view distance as `depth_test.wgsl` does.
#[derive(Clone, Debug)]
pub struct DepthBuffer {
    pub width: u32,
    pub height: u32,
    /// Row-major, top row first; 0 where nothing was drawn.
    pub values: Vec<f32>,
    pub perspective: bool,
    /// The near distance in perspective, the far one in orthographic.
    pub near_or_far: f32,
    /// The depth scale of an orthographic view.
    pub scale: f32,
}

impl DepthBuffer {
    /// The view distance the lines' depth test reads at pixel `i`
    /// (`surface_at` of `depth_test.wgsl`, in `f32` as there); `None`
    /// where nothing was drawn.
    pub fn distance(&self, i: usize) -> Option<f32> {
        let z = *self.values.get(i)?;
        // Not a number, or the cleared 0: nothing was drawn.
        if z.is_nan() || z <= 0.0 {
            return None;
        }
        Some(if self.perspective {
            self.near_or_far / z
        } else {
            self.near_or_far - z / self.scale
        })
    }
}

/// The targets of one image size.
struct Targets {
    width: u32,
    height: u32,
    /// The multisampled colour target; `None` with one sample, when the
    /// shown texture is drawn into directly.
    msaa: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
    shown: wgpu::Texture,
    shown_view: wgpu::TextureView,
    lines_bind: wgpu::BindGroup,
    /// The density of the edges: the side of a tile in pixels, the ink
    /// (where the device draws it) with the group that reads it, and the
    /// tiles the lines read.
    tile: u32,
    ink: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    tiles: wgpu::TextureView,
    // Read back by tests, which a browser does not run.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    tiles_texture: wgpu::Texture,
    /// The distances of the depth buffer (`distance.wgsl`), and the group
    /// that makes them.
    distance: wgpu::TextureView,
    distance_bind: wgpu::BindGroup,
    capture: Option<Capture>,
    /// What a see-through picture needs besides; made for the first one.
    peel: Option<PeelTargets>,
    /// What a cut-open picture needs besides; made for the first one.
    cut: Option<CutTargets>,
}

/// The count of a cut-open picture (`cut.rs`), with the samples of the
/// renderer.
struct CutTargets {
    parity: wgpu::TextureView,
    cap_bind: wgpu::BindGroup,
    /// The count with the front's keys (or depth buffer), for the cap
    /// behind the front of a see-through picture; made for the first one.
    cap_behind_bind: Option<wgpu::BindGroup>,
    /// The count at one sample per pixel, for the capture of a renderer
    /// with several; made for the first capture.
    flat: Option<(wgpu::TextureView, wgpu::BindGroup)>,
}

/// The pipelines of a cut-open picture (`cut.rs`), made for the first one.
struct CutPipelines {
    /// Counts what is left out, with the samples of the renderer.
    parity: wgpu::RenderPipeline,
    color: wgpu::RenderPipeline,
    depth_only: wgpu::RenderPipeline,
    cap: wgpu::RenderPipeline,
    cap_depth_only: wgpu::RenderPipeline,
    cap_layout: wgpu::BindGroupLayout,
    /// The capture, at one sample per pixel: its own count when the
    /// renderer has several samples, the scene's data and the cap's.
    parity_flat: Option<wgpu::RenderPipeline>,
    data: wgpu::RenderPipeline,
    cap_data: wgpu::RenderPipeline,
    /// The group of `cap_data` when it is not `cap_layout`.
    cap_flat_layout: Option<wgpu::BindGroupLayout>,
    /// For a see-through picture; `None` on a device that draws none.
    peeled: Option<CutPeeled>,
}

struct CutPeeled {
    front: wgpu::RenderPipeline,
    cap_front: wgpu::RenderPipeline,
    behind: wgpu::RenderPipeline,
    cap_behind: wgpu::RenderPipeline,
    cap_behind_layout: wgpu::BindGroupLayout,
}

impl CutPipelines {
    fn build(
        device: &wgpu::Device,
        scene_layout: &wgpu::BindGroupLayout,
        peel_layout: Option<&wgpu::BindGroupLayout>,
        samples: u32,
        per_sample: bool,
    ) -> CutPipelines {
        let multisampled = samples > 1;
        let module = cut::module(device, multisampled, per_sample);
        let cap_layout = cut::cap_layout(device, multisampled, false);
        let scene = [Some(scene_layout)];
        let capped = [Some(scene_layout), Some(&cap_layout)];
        let hidden = Some(wgpu::ColorTargetState {
            format: COLOR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::empty(),
        });
        let cap_entry = if multisampled {
            "fs_cap_samples"
        } else {
            "fs_cap_color"
        };
        let build = |module: &wgpu::ShaderModule,
                     entry: &str,
                     groups: &[Option<&wgpu::BindGroupLayout>],
                     targets: &[Option<wgpu::ColorTargetState>],
                     count: u32,
                     depth: bool| {
            cut::pipeline(device, module, entry, groups, targets, count, depth)
        };
        let parity = build(
            &module,
            "fs_parity",
            &scene,
            &[cut::parity_target()],
            samples,
            false,
        );
        let color = build(
            &module,
            "fs_color_cut",
            &scene,
            &[target(COLOR_FORMAT)],
            samples,
            true,
        );
        let depth_only = build(
            &module,
            "fs_color_cut",
            &scene,
            std::slice::from_ref(&hidden),
            samples,
            true,
        );
        let cap = build(
            &module,
            cap_entry,
            &capped,
            &[target(COLOR_FORMAT)],
            samples,
            true,
        );
        let cap_depth_only = build(
            &module,
            cap_entry,
            &capped,
            std::slice::from_ref(&hidden),
            samples,
            true,
        );
        // The capture has one sample per pixel whatever the renderer has.
        let flat_module = multisampled.then(|| cut::module(device, false, false));
        let cap_flat_layout = multisampled.then(|| cut::cap_layout(device, false, false));
        let flat = flat_module.as_ref().unwrap_or(&module);
        let capped_flat = [
            Some(scene_layout),
            Some(cap_flat_layout.as_ref().unwrap_or(&cap_layout)),
        ];
        let parity_flat = multisampled
            .then(|| build(flat, "fs_parity", &scene, &[cut::parity_target()], 1, false));
        let data = build(flat, "fs_data_cut", &scene, &[target(DATA_FORMAT)], 1, true);
        let cap_data = build(
            flat,
            "fs_cap_data",
            &capped_flat,
            &[target(DATA_FORMAT)],
            1,
            true,
        );
        let peeled = peel_layout.map(|peel_layout| {
            let cap_behind_layout = cut::cap_layout(device, multisampled, true);
            let layer = [target(COLOR_FORMAT)];
            let with_key = [target(COLOR_FORMAT), target(COLOR_FORMAT)];
            let (front_entry, cap_front_entry, behind_entry, cap_behind_entry) = if multisampled {
                (
                    "fs_front_samples_cut",
                    "fs_cap_front_samples",
                    "fs_behind_samples_cut",
                    "fs_cap_behind_samples",
                )
            } else {
                (
                    "fs_front_color_cut",
                    "fs_cap_color",
                    "fs_behind_cut",
                    "fs_cap_behind",
                )
            };
            let front_targets: &[Option<wgpu::ColorTargetState>] =
                if multisampled { &with_key } else { &layer };
            CutPeeled {
                front: build(&module, front_entry, &scene, front_targets, samples, true),
                cap_front: build(
                    &module,
                    cap_front_entry,
                    &capped,
                    front_targets,
                    samples,
                    true,
                ),
                behind: build(
                    &module,
                    behind_entry,
                    &[Some(scene_layout), Some(peel_layout)],
                    &layer,
                    samples,
                    true,
                ),
                cap_behind: build(
                    &module,
                    cap_behind_entry,
                    &[Some(scene_layout), Some(&cap_behind_layout)],
                    &layer,
                    samples,
                    true,
                ),
                cap_behind_layout,
            }
        });
        CutPipelines {
            parity,
            color,
            depth_only,
            cap,
            cap_depth_only,
            cap_layout,
            parity_flat,
            data,
            cap_data,
            cap_flat_layout,
            peeled,
        }
    }
}

/// The two layers of a see-through picture, with the samples of the
/// renderer.
struct PeelTargets {
    front: wgpu::TextureView,
    /// Which triangle is in front at each sample (`peel_samples.wgsl`),
    /// with several samples per pixel.
    key: Option<wgpu::TextureView>,
    behind: wgpu::TextureView,
    /// The depth buffer of the pass behind the front.
    depth: wgpu::TextureView,
    /// The front's keys, or with one sample per pixel its depth buffer,
    /// for the pass behind it.
    peel_bind: wgpu::BindGroup,
    blend_bind: wgpu::BindGroup,
}

/// The pipelines of a see-through picture (`peel.rs`).
struct SeeThrough {
    front_pipeline: wgpu::RenderPipeline,
    behind_pipeline: wgpu::RenderPipeline,
    blend_pipeline: wgpu::RenderPipeline,
    peel_layout: wgpu::BindGroupLayout,
    blend_layout: wgpu::BindGroupLayout,
    blend_uniforms: wgpu::Buffer,
    table: peel::Table,
}

impl SeeThrough {
    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_layout: &wgpu::BindGroupLayout,
        samples: u32,
    ) -> SeeThrough {
        let multisampled = samples > 1;
        let module = peel::scene_module(device, multisampled);
        let peel_layout = peel::peel_layout(device, multisampled);
        let blend_layout = peel::blend_layout(device, multisampled, false);
        // With several samples the front also writes its keys, which the
        // pass behind it reads in place of the depth buffer.
        let (front_entry, behind_entry) = if multisampled {
            ("fs_front_samples", "fs_behind_samples")
        } else {
            ("fs_front_color", "fs_behind")
        };
        let front_targets: &[Option<wgpu::ColorTargetState>] = if multisampled {
            &[target(COLOR_FORMAT), target(COLOR_FORMAT)]
        } else {
            &[target(COLOR_FORMAT)]
        };
        let front_pipeline = scene_pipeline(
            device,
            "craft3d front",
            &module,
            front_entry,
            &[Some(scene_layout)],
            front_targets,
            samples,
            Step::One,
        );
        let behind_pipeline = scene_pipeline(
            device,
            "craft3d behind",
            &module,
            behind_entry,
            &[Some(scene_layout), Some(&peel_layout)],
            &[target(COLOR_FORMAT)],
            samples,
            Step::One,
        );
        let blend_pipeline = peel::blend_pipeline(
            device,
            &peel::blend_module(device, multisampled),
            &blend_layout,
            false,
        );
        let blend_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("craft3d blend uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&blend_uniforms, 0, &peel::blend_uniforms(samples));
        SeeThrough {
            front_pipeline,
            behind_pipeline,
            blend_pipeline,
            peel_layout,
            blend_layout,
            blend_uniforms,
            table: peel::Table::new(device),
        }
    }
}

/// The targets of the capture pass.
struct Capture {
    data: wgpu::Texture,
    data_view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// The viewport renderer on a device and queue the caller owns.
pub struct Onscreen {
    device: wgpu::Device,
    queue: wgpu::Queue,
    samples: u32,
    /// Whether the count of a section is per sample (`cut.wgsl`,
    /// `vs_parity`): with several samples per pixel on a device that can
    /// shade per sample (`DownlevelFlags::MULTISAMPLED_SHADING`); without
    /// that the count is decided at the pixel's centre as before, and the
    /// cap may miss a lone sample along a silhouette
    /// (docs/design/rendering.md, 2026-10-19).
    per_sample: bool,
    scene_uniforms: wgpu::Buffer,
    scene_layout: wgpu::BindGroupLayout,
    scene_bind: wgpu::BindGroup,
    lines_uniforms: wgpu::Buffer,
    lines_layout: wgpu::BindGroupLayout,
    color_pipeline: wgpu::RenderPipeline,
    depth_only_pipeline: wgpu::RenderPipeline,
    data_pipeline: wgpu::RenderPipeline,
    lines_pipeline: wgpu::RenderPipeline,
    /// The lines into a target with one sample (the shown texture itself),
    /// when [`Onscreen::lines_pipeline`] is for several.
    lines_flat_pipeline: Option<wgpu::RenderPipeline>,
    /// The fills (`fills.wgsl`), with the lines' bind group; and for one
    /// sample when that pipeline is for several.
    fills_pipeline: wgpu::RenderPipeline,
    fills_flat_pipeline: Option<wgpu::RenderPipeline>,
    /// What measures how crowded the edges are, or why this device does
    /// not (its edges are then faded only for being short).
    density: Result<Density, String>,
    /// Turns the depth buffer into the distances the lines read.
    distance_pipeline: wgpu::RenderPipeline,
    distance_layout: wgpu::BindGroupLayout,
    /// The nearest texel of a single-sampled depth buffer, for the
    /// distance pass ([`SAMPLED_DEPTH`]) and on OpenGL for the passes
    /// behind the front (`peel::sampled_front`).
    depth_sampler: wgpu::Sampler,
    /// What draws a see-through picture, or why this device does not.
    see_through: Result<SeeThrough, String>,
    /// What draws a cut-open picture, or why this device does not; made
    /// for the first one.
    cut: Option<Result<CutPipelines, String>>,
    /// The triangle of the cap, written every cut-open frame.
    cap_buffer: wgpu::Buffer,
    targets: Option<Targets>,
    staging: Mutex<Option<readback::Staging>>,
    drawn: bool,
    captured: bool,
    /// The depth numbers of the last draw (`scene.wgsl`: forward.w,
    /// depth.x, depth.y), to turn the depth buffer back into distances.
    depth_numbers: [f32; 3],
    /// The template of `depth_read.wgsl` filled in for this renderer's
    /// samples, and its pipeline, made for the first
    /// [`Onscreen::read_depth`].
    #[cfg(not(target_arch = "wasm32"))]
    depth_read_source: String,
    #[cfg(not(target_arch = "wasm32"))]
    depth_read_pipeline: Option<wgpu::RenderPipeline>,
    /// The look of the next draws (`Onscreen::set_studio`), and what draws
    /// the studio look, or why this device does not; made for the first
    /// studio picture.
    look: Option<crate::Studio>,
    studio: Option<Result<crate::studio::StudioOnscreen, String>>,
    /// Identifies this renderer in what it uploads.
    token: Arc<()>,
}

/// The passes of the edge density: the ink of the edges (the lines'
/// shader into [`INK_FORMAT`], summed by blending) and the mean per tile.
struct Density {
    ink_pipeline: wgpu::RenderPipeline,
    tiles_pipeline: wgpu::RenderPipeline,
    tiles_layout: wgpu::BindGroupLayout,
}

impl Density {
    fn build(
        device: &wgpu::Device,
        lines_shader: &wgpu::ShaderModule,
        lines_pipeline_layout: &wgpu::PipelineLayout,
        instance: &wgpu::VertexBufferLayout<'_>,
    ) -> Density {
        // The ink target as it will be made: a device that cannot render
        // into a half float (WebGL2 without its extension) fails here,
        // under the guard, and draws its edges unfaded by crowding.
        let _probe = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("craft3d edge ink probe"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: INK_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let primitive = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..wgpu::PrimitiveState::default()
        };
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let ink_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("craft3d edge ink"),
            layout: Some(lines_pipeline_layout),
            vertex: wgpu::VertexState {
                module: lines_shader,
                entry_point: Some("vs_ink"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(instance.clone())],
            },
            primitive,
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: lines_shader,
                entry_point: Some("fs_ink"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: INK_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: add,
                        alpha: add,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let tiles_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("craft3d edge tiles"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let tiles_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d edge tiles"),
            source: wgpu::ShaderSource::Wgsl(EDGE_TILES_SOURCE.into()),
        });
        let tiles_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("craft3d edge tiles"),
                bind_group_layouts: &[Some(&tiles_layout)],
                immediate_size: 0,
            });
        let tiles_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("craft3d edge tiles"),
            layout: Some(&tiles_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &tiles_shader,
                entry_point: Some("vs_cover"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive,
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &tiles_shader,
                entry_point: Some("fs_tiles"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[target(TILE_FORMAT)],
            }),
            multiview_mask: None,
            cache: None,
        });
        Density {
            ink_pipeline,
            tiles_pipeline,
            tiles_layout,
        }
    }
}

/// The pass over the tiles of the edge density (no template).
const EDGE_TILES_SOURCE: &str = include_str!("edge_tiles.wgsl");

/// The shaders over the picture, filled in for a depth buffer of
/// `samples` samples per pixel: the lines, the fills, the distances the
/// two read, and the copy of the depth buffer for tests.
pub(crate) struct OverlaySources {
    pub(crate) lines: String,
    pub(crate) fills: String,
    pub(crate) distance: String,
    // Read back by tests, which a browser does not run.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) depth_read: String,
}

/// How the distance pass reads a single-sampled depth buffer: through a
/// sampler that takes the nearest texel ([`DEPTH_SAMPLER_BINDING`]), at the
/// texel's centre, which gives the texel itself. OpenGL ES 3.0 (WebGL2)
/// takes a depth texture whose filter is not the nearest texel for
/// incomplete and reads 0 from it (`texelFetch` too), and a texture only
/// loaded from gets no sampler there (wgpu-hal's GL backend), so its
/// default linear filter stays. The see-through and section passes read
/// the front's depth the same way on OpenGL (`renderer::SAMPLED_FRONT`,
/// through this same sampler); keep the two alike.
const SAMPLED_DEPTH: &str = "textureSampleLevel(depth_texture, depth_sampler, \
     (vec2<f32>(p) + vec2<f32>(0.5)) / vec2<f32>(textureDimensions(depth_texture)), 0.0).r";

/// The binding of that sampler in the distance pass's group.
const DEPTH_SAMPLER_BINDING: u32 = 3;

pub(crate) fn overlay_sources(samples: u32) -> OverlaySources {
    let (depth_type, load, distance_load, sampler) = if samples > 1 {
        (
            "texture_multisampled_2d<f32>",
            "textureLoad(depth_texture, p, s).r",
            "textureLoad(depth_texture, p, s).r",
            String::new(),
        )
    } else {
        (
            "texture_2d<f32>",
            "textureLoad(depth_texture, p, 0).r",
            SAMPLED_DEPTH,
            format!("@group(0) @binding({DEPTH_SAMPLER_BINDING}) var depth_sampler: sampler;"),
        )
    };
    let fill = |template: &str, load: &str| {
        template
            .replace("{{DEPTH_TEST}}", include_str!("depth_test.wgsl"))
            .replace("{{DEPTH}}", depth_type)
            .replace("{{LOAD}}", load)
            .replace("{{SAMPLER}}", &sampler)
    };
    OverlaySources {
        lines: fill(include_str!("lines.wgsl"), load),
        fills: fill(include_str!("fills.wgsl"), load),
        distance: fill(include_str!("distance.wgsl"), distance_load),
        depth_read: fill(include_str!("depth_read.wgsl"), load),
    }
}

/// True when `adapter` renders `format` with `samples` samples.
fn multisamples(adapter: &wgpu::Adapter, format: wgpu::TextureFormat, samples: u32) -> bool {
    adapter
        .get_texture_format_features(format)
        .flags
        .sample_count_supported(samples)
}

impl Onscreen {
    /// A renderer with [`WANTED_SAMPLES`] samples per pixel when `adapter`
    /// has them for the colour and depth formats, else with one.
    ///
    /// On wgpu's OpenGL backend it is always one, because that backend
    /// (wgpu-hal 30.0.1, `src/gles`) cannot make a multisampled texture
    /// that is also read by a shader, which the lines' depth test needs of
    /// the depth buffer. Only a multisampled texture that is nothing but a
    /// render target becomes a multisampled renderbuffer (`device.rs`,
    /// `create_texture`); any other gets the target `GL_TEXTURE_2D`
    /// (`mod.rs`, `Texture::get_info_from_desc`: `GL_TEXTURE_2D_MULTISAMPLE`
    /// occurs nowhere in the backend) and is given its storage with
    /// `glTexStorage2DMultisample(GL_TEXTURE_2D, ..)`, which OpenGL refuses
    /// (`GL_INVALID_ENUM`). The texture has no storage, the framebuffer it
    /// is attached to is incomplete, and every clear, draw and resolve
    /// into it fails without wgpu saying so: the picture stays empty.
    /// `tests/gl_multisample.rs` shows it, and that four samples resolve
    /// on OpenGL when the depth buffer is only an attachment; what Mesa
    /// reports is printed with `MESA_DEBUG=1`. Multisampling on OpenGL
    /// would take a depth buffer the lines can read made in a pass of its
    /// own, at one sample. OpenGL is the last resort of the window anyway.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        adapter: &wgpu::Adapter,
    ) -> Result<Onscreen, GpuError> {
        let samples = if adapter.get_info().backend != wgpu::Backend::Gl
            && multisamples(adapter, COLOR_FORMAT, WANTED_SAMPLES)
            && multisamples(adapter, DEPTH_FORMAT, WANTED_SAMPLES)
        {
            WANTED_SAMPLES
        } else {
            1
        };
        let per_sample = samples > 1
            && adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::MULTISAMPLED_SHADING);
        let mut onscreen = Self::shaded(device, queue, samples, per_sample)?;
        // The ink of the edges is summed by blending in a half float: a
        // device that cannot draw into one with blending fades its edges
        // only for being short.
        let ink = adapter.get_texture_format_features(INK_FORMAT);
        if !(ink
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            && ink
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE))
        {
            onscreen.density = Err("it cannot blend into a half float".to_string());
        }
        Ok(onscreen)
    }

    /// A renderer with `samples` samples per pixel (1, 2, 4 or 8; the
    /// device must support them for `Rgba8Unorm` and `Depth32Float`, and
    /// with several it is taken to shade per sample, which
    /// [`Onscreen::new`] checks on the adapter: a device that cannot
    /// refuses a cut-open picture then).
    pub fn with_samples(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        samples: u32,
    ) -> Result<Onscreen, GpuError> {
        Self::shaded(device, queue, samples, samples > 1)
    }

    /// [`Onscreen::with_samples`], with whether the count of a section is
    /// per sample (`per_sample`).
    fn shaded(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        samples: u32,
        per_sample: bool,
    ) -> Result<Onscreen, GpuError> {
        if !matches!(samples, 1 | 2 | 4 | 8) {
            return Err(GpuError::Unsupported("that number of samples per pixel"));
        }
        let mut onscreen = guarded(device, || Self::build(device, queue, samples, per_sample))?;
        // On its own: a device that cannot do this still draws opaque
        // pictures, and says why when asked for a see-through one.
        onscreen.see_through = guarded(device, || {
            SeeThrough::build(device, queue, &onscreen.scene_layout, samples)
        })
        .map_err(|e| e.to_string());
        Ok(onscreen)
    }

    /// As on a device that cannot draw a cut-open picture: one with a
    /// section is refused from now on ([`GpuError::Section`]). For tests
    /// of what a caller does then.
    pub fn without_section(&mut self) {
        self.cut = Some(Err("it was turned off".to_string()));
    }

    /// Makes the pipelines of a cut-open picture, once; the error says why
    /// this device has none.
    fn ensure_cut(&mut self) -> Result<(), GpuError> {
        if self.cut.is_none() {
            let peel_layout = self.see_through.as_ref().ok().map(|s| &s.peel_layout);
            let built = guarded(&self.device, || {
                CutPipelines::build(
                    &self.device,
                    &self.scene_layout,
                    peel_layout,
                    self.samples,
                    self.per_sample,
                )
            })
            .map_err(|e| e.to_string());
            self.cut = Some(built);
        }
        match &self.cut {
            Some(Ok(_)) => Ok(()),
            Some(Err(why)) => Err(GpuError::Section(why.clone())),
            None => Err(GpuError::NothingDrawn),
        }
    }

    /// As on a device that cannot draw see-through bodies: such a picture
    /// is refused from now on ([`GpuError::SeeThrough`]). For tests of what
    /// a caller does then.
    pub fn without_see_through(&mut self) {
        self.see_through = Err("it was turned off".to_string());
    }

    /// Whether this renderer draws see-through bodies, or why not.
    pub fn see_through(&self) -> Result<(), &str> {
        self.see_through
            .as_ref()
            .map(|_| ())
            .map_err(String::as_str)
    }

    /// Whether this renderer measures how crowded the edges are (and
    /// fades them there), or why not.
    pub fn edge_density(&self) -> Result<(), &str> {
        self.density.as_ref().map(|_| ()).map_err(String::as_str)
    }

    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        samples: u32,
        per_sample: bool,
    ) -> Onscreen {
        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d scene"),
            source: wgpu::ShaderSource::Wgsl(scene_source(device, &[]).into()),
        });
        // The depth buffer is bound as an unfilterable float texture, not
        // as a depth texture: naga's GLSL backend cannot `textureLoad` a
        // depth texture, and WebGPU binds a depth format either way.
        let sources = overlay_sources(samples);
        let lines_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d lines"),
            source: wgpu::ShaderSource::Wgsl(sources.lines.as_str().into()),
        });
        let fills_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d fills"),
            source: wgpu::ShaderSource::Wgsl(sources.fills.as_str().into()),
        });

        let both = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("craft3d scene"),
            entries: &[uniform_entry(0, both)],
        });
        let scene_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("craft3d scene uniforms"),
            size: (SCENE_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scene_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d scene"),
            layout: &scene_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_uniforms.as_entire_binding(),
            }],
        });
        let lines_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("craft3d lines uniforms"),
            size: (LINES_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lines_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("craft3d lines"),
            entries: &[
                uniform_entry(0, both),
                uniform_entry(1, both),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: samples > 1,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        // The pass that makes the distances: the scene's numbers, the
        // lines' and the depth buffer, as the lines' group had them.
        let mut distance_entries = vec![
            uniform_entry(0, both),
            uniform_entry(1, both),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: samples > 1,
                },
                count: None,
            },
        ];
        if samples == 1 {
            distance_entries.push(wgpu::BindGroupLayoutEntry {
                binding: DEPTH_SAMPLER_BINDING,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                count: None,
            });
        }
        let distance_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("craft3d distances"),
            entries: &distance_entries,
        });
        let depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("craft3d depth"),
            ..wgpu::SamplerDescriptor::default()
        });
        let distance_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d distances"),
            source: wgpu::ShaderSource::Wgsl(sources.distance.as_str().into()),
        });
        let distance_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("craft3d distances"),
                bind_group_layouts: &[Some(&distance_layout)],
                immediate_size: 0,
            });
        let distance_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("craft3d distances"),
            layout: Some(&distance_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &distance_shader,
                entry_point: Some("vs_cover"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &distance_shader,
                entry_point: Some("fs_distance"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[target(DISTANCE_FORMAT)],
            }),
            multiview_mask: None,
            cache: None,
        });

        let scene_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("craft3d scene"),
                bind_group_layouts: &[Some(&scene_layout)],
                immediate_size: 0,
            });
        let mesh_layout = wgpu::VertexBufferLayout {
            array_stride: VERTEX_BYTES as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &MESH_ATTRIBUTES,
        };
        let multisample = |count: u32| wgpu::MultisampleState {
            count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        // `Greater` (reversed depth, `scene.wgsl`): on a tie the triangle
        // drawn first stays, as on the CPU.
        let depth = Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });
        // Both sides of every triangle are drawn, as on the CPU.
        let primitive = wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..wgpu::PrimitiveState::default()
        };
        let scene_pipeline =
            |label: &str, entry: &str, targets: &[Option<wgpu::ColorTargetState>], count: u32| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&scene_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &scene_shader,
                        entry_point: Some("vs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[Some(mesh_layout.clone())],
                    },
                    primitive,
                    depth_stencil: depth.clone(),
                    multisample: multisample(count),
                    fragment: Some(wgpu::FragmentState {
                        module: &scene_shader,
                        entry_point: Some(entry),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            };
        let color_pipeline = scene_pipeline(
            "craft3d bodies",
            "fs_color",
            &[target(COLOR_FORMAT)],
            samples,
        );
        let depth_only_pipeline = scene_pipeline(
            "craft3d bodies, depth only",
            "fs_color",
            &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::empty(),
            })],
            samples,
        );
        let data_pipeline = scene_pipeline("craft3d capture", "fs_data", &[target(DATA_FORMAT)], 1);

        let lines_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("craft3d lines"),
                bind_group_layouts: &[Some(&lines_layout)],
                immediate_size: 0,
            });
        let attribute = |format, offset, shader_location| wgpu::VertexAttribute {
            format,
            offset,
            shader_location,
        };
        let line_attributes = [
            attribute(wgpu::VertexFormat::Float32x3, 0, 0),
            attribute(wgpu::VertexFormat::Float32x3, 12, 1),
            attribute(wgpu::VertexFormat::Uint32, 24, 2),
            attribute(wgpu::VertexFormat::Float32, 28, 3),
            attribute(wgpu::VertexFormat::Uint32, 32, 4),
            attribute(wgpu::VertexFormat::Uint32, 36, 5),
            attribute(wgpu::VertexFormat::Float32, 40, 6),
            attribute(wgpu::VertexFormat::Float32x3, 44, 7),
            attribute(wgpu::VertexFormat::Float32x3, 56, 8),
        ];
        let instance = wgpu::VertexBufferLayout {
            array_stride: SEGMENT_BYTES as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &line_attributes,
        };
        let lines_pipeline_for = |count: u32| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("craft3d lines"),
                layout: Some(&lines_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &lines_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(instance.clone())],
                },
                primitive,
                // The depth test is in the fragment shader, against the
                // scene's depth buffer as a texture.
                depth_stencil: None,
                multisample: multisample(count),
                fragment: Some(wgpu::FragmentState {
                    module: &lines_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        // under + (ink - under) * alpha, as the CPU blends.
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let lines_pipeline = lines_pipeline_for(samples);
        let lines_flat_pipeline = (samples > 1).then(|| lines_pipeline_for(1));
        let fill_attributes = [
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 12,
                shader_location: 1,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 24,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 36,
                shader_location: 3,
            },
        ];
        let fills_pipeline_for = |count: u32| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("craft3d fills"),
                layout: Some(&lines_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &fills_shader,
                    entry_point: Some("vs_fill"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: FILL_VERTEX_BYTES as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &fill_attributes,
                    })],
                },
                primitive,
                depth_stencil: None,
                multisample: multisample(count),
                fragment: Some(wgpu::FragmentState {
                    module: &fills_shader,
                    entry_point: Some("fs_fill"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let fills_pipeline = fills_pipeline_for(samples);
        let fills_flat_pipeline = (samples > 1).then(|| fills_pipeline_for(1));
        // On its own: a device that cannot draw into the formats still
        // draws its lines.
        let density = guarded(device, || {
            Density::build(device, &lines_shader, &lines_pipeline_layout, &instance)
        })
        .map_err(|e| e.to_string());

        Onscreen {
            device: device.clone(),
            queue: queue.clone(),
            samples,
            per_sample,
            scene_uniforms,
            scene_layout,
            scene_bind,
            lines_uniforms,
            lines_layout,
            color_pipeline,
            depth_only_pipeline,
            data_pipeline,
            lines_pipeline,
            lines_flat_pipeline,
            fills_pipeline,
            fills_flat_pipeline,
            density,
            distance_pipeline,
            distance_layout,
            depth_sampler,
            see_through: Err("it was not set up".to_string()),
            cut: None,
            cap_buffer: cut::cap_buffer(device),
            targets: None,
            staging: Mutex::new(None),
            drawn: false,
            captured: false,
            depth_numbers: [0.0; 3],
            #[cfg(not(target_arch = "wasm32"))]
            depth_read_source: sources.depth_read,
            #[cfg(not(target_arch = "wasm32"))]
            depth_read_pipeline: None,
            look: None,
            studio: None,
            token: Arc::new(()),
        }
    }

    /// The look of the draws from now on: the studio look
    /// (`craft3d_render::studio`) for an opaque, whole picture of the
    /// bodies, or `None` for the plain one. In the browser on WebGL2 the
    /// studio look has no shadows and no occlusion
    /// (docs/design/rendering.md, *The studio look*, *WebGL2*).
    pub fn set_studio(&mut self, look: Option<crate::Studio>) {
        self.look = look;
    }

    /// The studio look this device draws for `look`: on WebGL2 without
    /// shadows and occlusion.
    pub fn studio_as_drawn(&self, look: &crate::Studio) -> crate::Studio {
        let mut drawn = look.clone();
        if crate::studio::simpler(&self.device) {
            drawn.look.shadows = false;
            drawn.look.ao = false;
        }
        drawn
    }

    /// Samples per pixel of the picture.
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// The device this renderer draws on.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Puts `mesh` on the GPU.
    pub fn upload(&self, mesh: &Mesh) -> Result<GpuMesh, GpuError> {
        self.upload_colored(mesh, &FaceColors::new())
    }

    /// [`Self::upload`] with faces in colours of their own
    /// (`craft3d_render::render_colored`): the colours go with the mesh, so
    /// a change of colour is a new upload.
    pub fn upload_colored(&self, mesh: &Mesh, colors: &FaceColors) -> Result<GpuMesh, GpuError> {
        guarded(&self.device, || {
            GpuMesh::upload(&self.device, &self.queue, mesh, colors, &self.token)
        })?
    }

    /// Puts `segments` on the GPU, in the order they are drawn (a later
    /// one paints over an earlier one). Segments with an end that is not
    /// finite or does not fit in `f32` are left out.
    pub fn upload_lines(&self, segments: &[Segment]) -> Result<GpuLines, GpuError> {
        self.upload_overlay(segments, &[])
    }

    /// [`Onscreen::upload_lines`] with `fills` drawn under the segments, in
    /// order. Fills with a number that does not fit in `f32` are left out.
    /// Without fills the frame is the one [`Onscreen::upload_lines`] gives.
    pub fn upload_overlay(
        &self,
        segments: &[Segment],
        fills: &[Fill],
    ) -> Result<GpuLines, GpuError> {
        if fills.len() > MAX_FILLS {
            return Err(GpuError::TooManyLines {
                segments: fills.len(),
            });
        }
        if segments.len() > MAX_SEGMENTS {
            return Err(GpuError::TooManyLines {
                segments: segments.len(),
            });
        }
        // Relative to the centre of the segments that are not axes, like
        // the mesh is to its own; the axes are at the world origin anyway.
        let ends = segments
            .iter()
            .filter(|s| !s.axis)
            .flat_map(|s| [s.from, s.to])
            .filter(|&p| narrow(p).is_some());
        let origin = Bbox3::from_points(ends)
            .center()
            .filter(|c| c.is_finite())
            .unwrap_or(Vec3::ZERO);
        let mut bytes: Vec<u8> = Vec::with_capacity(segments.len().saturating_mul(SEGMENT_BYTES));
        let mut axes = Vec::new();
        let (mut count, mut edges) = (0_usize, 0_usize);
        for segment in segments {
            let Some(packed) = pack_segment(segment, origin, 1.0) else {
                continue;
            };
            if segment.axis {
                axes.push((bytes.len() as u64, *segment));
            }
            bytes.extend_from_slice(&packed);
            count += 1;
            edges += usize::from(segment.edge.is_some());
        }
        let instances = u32::try_from(count).map_err(|_| GpuError::TooManyLines {
            segments: segments.len(),
        })?;
        if bytes.len() as u64 > self.device.limits().max_buffer_size {
            return Err(GpuError::TooManyLines {
                segments: segments.len(),
            });
        }
        let buffer = if bytes.is_empty() {
            None
        } else {
            Some(guarded(&self.device, || {
                let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("craft3d lines"),
                    size: bytes.len() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.queue.write_buffer(&buffer, 0, &bytes);
                buffer
            })?)
        };
        let fill_bytes: Vec<u8> = fills
            .iter()
            .filter_map(|fill| pack_fill(fill, origin))
            .flatten()
            .collect();
        let fill_vertices = u32::try_from(fill_bytes.len() / FILL_VERTEX_BYTES).map_err(|_| {
            GpuError::TooManyLines {
                segments: fills.len(),
            }
        })?;
        let fill_buffer = if fill_bytes.is_empty() {
            None
        } else {
            Some(guarded(&self.device, || {
                let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("craft3d fills"),
                    size: fill_bytes.len() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.queue.write_buffer(&buffer, 0, &fill_bytes);
                buffer
            })?)
        };
        Ok(GpuLines {
            buffer,
            instances,
            edges,
            origin,
            axes,
            axis_length: Cell::new(1.0),
            segments: count,
            fills: fill_buffer,
            fill_vertices,
            owner: Arc::clone(&self.token),
        })
    }

    /// Draws `mesh` as seen from `camera` with `lines` over it into the
    /// shown texture ([`Onscreen::view`]). `params.ssaa` must be 1: the
    /// picture is multisampled instead.
    pub fn draw(
        &mut self,
        mesh: &GpuMesh,
        lines: &GpuLines,
        camera: &Camera,
        params: &RenderParams,
        overlay: &OverlayParams,
        options: DrawOptions,
    ) -> Result<(), GpuError> {
        // Whatever goes wrong below, the previous frame is not this one.
        self.drawn = false;
        self.captured = false;
        if !Arc::ptr_eq(&mesh.owner, &self.token) || !Arc::ptr_eq(&lines.owner, &self.token) {
            return Err(GpuError::ForeignMesh);
        }
        if params.ssaa != 1 {
            return Err(GpuError::Unsupported(
                "supersampling (the viewport renderer multisamples instead)",
            ));
        }
        check_params(params, self.device.limits().max_texture_dimension_2d)?;
        // Bodies that are not shaded have nothing to see through.
        let see_through = options.bodies && params.opacity() < 1.0;
        if see_through && let Err(why) = &self.see_through {
            return Err(GpuError::SeeThrough(why.clone()));
        }
        let cut = cut::frame_cut(mesh, params);
        if cut.is_some() {
            self.ensure_cut()?;
        }
        let scene = scene_uniforms(mesh, camera, params, cut.as_ref())?;
        self.depth_numbers = [scene[11], scene[20], scene[21]];
        let line_uniforms = lines_uniforms(lines, camera, params, overlay, self.samples)?;
        // The studio look: for an opaque, whole picture of the bodies.
        let lit = match &self.look {
            Some(look) if options.bodies && !see_through && cut.is_none() => {
                let drawn = self.studio_as_drawn(look);
                let numbers =
                    crate::studio::studio_uniforms(mesh, &drawn, (params.width, params.height))?;
                Some((drawn, numbers))
            }
            _ => None,
        };
        if lit.is_some() && self.studio.is_none() {
            let (device, layout, samples) = (self.device.clone(), &self.scene_layout, self.samples);
            self.studio = Some(
                guarded(&device, || {
                    crate::studio::StudioOnscreen::build(&device, layout, samples, COLOR_FORMAT)
                })
                .map_err(|e| e.to_string()),
            );
        }
        let device = self.device.clone();
        if let Some((drawn, _)) = &lit {
            let queue = self.queue.clone();
            match &mut self.studio {
                Some(Ok(studio)) => guarded(&device, || {
                    studio.ensure_size(&device, (params.width, params.height));
                    studio.gpu.ensure_env(&device, &queue, &drawn.env)
                })??,
                Some(Err(why)) => {
                    return Err(GpuError::Device(format!(
                        "the studio look cannot be drawn on this device: {why}"
                    )));
                }
                None => return Err(GpuError::NothingDrawn),
            }
        }
        let studio_numbers = lit.map(|(_, numbers)| numbers);
        guarded(&device, || {
            self.ensure_targets(params, options.capture, overlay.edges.tile());
            if see_through {
                self.ensure_peel_targets();
            }
            if cut.is_some() {
                self.ensure_cut_targets(see_through, options.capture);
            }
            self.encode(
                mesh,
                lines,
                &scene,
                &line_uniforms,
                params,
                overlay,
                options,
                cut.as_ref(),
                studio_numbers.as_ref(),
            )
        })??;
        self.drawn = true;
        self.captured = options.capture;
        Ok(())
    }

    /// The shown picture (`Rgba8Unorm`, `params.width` × `params.height`)
    /// of the last successful draw. It is replaced when the image size
    /// changes.
    pub fn view(&self) -> Option<&wgpu::TextureView> {
        let targets = self.targets.as_ref().filter(|_| self.drawn)?;
        Some(&targets.shown_view)
    }

    /// The texture behind [`Onscreen::view`].
    pub fn texture(&self) -> Option<&wgpu::Texture> {
        let targets = self.targets.as_ref().filter(|_| self.drawn)?;
        Some(&targets.shown)
    }

    /// Width and height of the last successful draw.
    pub fn size(&self) -> Option<(u32, u32)> {
        let targets = self.targets.as_ref().filter(|_| self.drawn)?;
        Some((targets.width, targets.height))
    }

    /// Starts reading the shown picture back, with the captured data when
    /// `with_data`.
    fn start_readback(&self, with_data: bool) -> Result<readback::Pending, GpuError> {
        let targets = self
            .targets
            .as_ref()
            .filter(|_| self.drawn && (!with_data || self.captured))
            .ok_or(GpuError::NothingDrawn)?;
        let data = if with_data {
            Some(&targets.capture.as_ref().ok_or(GpuError::NothingDrawn)?.data)
        } else {
            None
        };
        let mut staging = self.staging.lock().unwrap_or_else(PoisonError::into_inner);
        let pending = guarded(&self.device, || {
            readback::start(
                &self.device,
                &self.queue,
                (&targets.shown, data),
                (targets.width, targets.height),
                // The capture draws one sample per pixel, at its centre.
                0.5,
                &mut staging,
            )
        })??;
        // Native wgpu maps buffers only when polled; in the browser the
        // event loop does it.
        #[cfg(not(target_arch = "wasm32"))]
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        Ok(pending)
    }

    /// Reads the shown picture back, 3 bytes per pixel: for tests.
    pub async fn read_rgb_async(&self) -> Result<Vec<u8>, GpuError> {
        let pending = self.start_readback(false)?;
        pending.mapped().await;
        pending.finish_rgb()
    }

    /// Reads the shown picture with the captured face ids and distances
    /// back as the CPU renderer's [`Frame`]: for tests. The last draw must
    /// have asked for the capture ([`DrawOptions::capture`]).
    pub async fn read_frame_async(&self) -> Result<Frame, GpuError> {
        let pending = self.start_readback(true)?;
        pending.mapped().await;
        pending.finish()
    }

    /// [`Onscreen::read_rgb_async`], blocking until the GPU is done.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_rgb(&self) -> Result<Vec<u8>, GpuError> {
        pollster::block_on(self.read_rgb_async())
    }

    /// [`Onscreen::read_frame_async`], blocking until the GPU is done.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_frame(&self) -> Result<Frame, GpuError> {
        pollster::block_on(self.read_frame_async())
    }

    /// Reads the depth buffer of the last draw back as the lines' depth
    /// test reads it (`depth_read.wgsl`: the mean of a pixel's samples,
    /// 0 where any shows nothing): for tests of that test.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_depth(&mut self) -> Result<DepthBuffer, GpuError> {
        let device = self.device.clone();
        let (width, height, values) = guarded(&device, || self.copy_depth(&device))??;
        let [perspective, x, y] = self.depth_numbers;
        Ok(DepthBuffer {
            width,
            height,
            values,
            perspective: perspective > 0.5,
            near_or_far: x,
            scale: y,
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn copy_depth(&mut self, device: &wgpu::Device) -> Result<(u32, u32, Vec<f32>), GpuError> {
        let targets = self
            .targets
            .as_ref()
            .filter(|_| self.drawn)
            .ok_or(GpuError::NothingDrawn)?;
        let (width, height) = (targets.width, targets.height);
        let too_large = || GpuError::Readback(format!("a {width}x{height} image is too large"));
        let stride = width
            .checked_mul(4)
            .and_then(|row| row.checked_next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
            .ok_or_else(too_large)?;
        let size = u64::from(stride) * u64::from(height);
        if size > device.limits().max_buffer_size {
            return Err(too_large());
        }
        let pipeline = match &self.depth_read_pipeline {
            Some(pipeline) => pipeline,
            None => {
                let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("craft3d depth read"),
                    source: wgpu::ShaderSource::Wgsl(self.depth_read_source.as_str().into()),
                });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("craft3d depth read"),
                    bind_group_layouts: &[Some(&self.lines_layout)],
                    immediate_size: 0,
                });
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("craft3d depth read"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::R32Uint,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                self.depth_read_pipeline.insert(pipeline)
            }
        };
        let copy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("craft3d depth read"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let copy_view = copy.create_view(&wgpu::TextureViewDescriptor::default());
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("craft3d depth read"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("craft3d depth read"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d depth read"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &copy_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &targets.lines_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        let values = self.read_back(device, encoder, &copy, &buffer, width, height, stride)?;
        Ok((width, height, values))
    }

    /// The density of the edges per tile of the last draw, read back
    /// (`edge_tiles.wgsl`): columns, rows and the means, row-major. For
    /// the tests that hold it to the CPU's `EdgeDensity`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_edge_density(&mut self) -> Result<(u32, u32, Vec<f32>), GpuError> {
        let device = self.device.clone();
        guarded(&device, || {
            let targets = self
                .targets
                .as_ref()
                .filter(|_| self.drawn)
                .ok_or(GpuError::NothingDrawn)?;
            let texture = &targets.tiles_texture;
            let (width, height) = (texture.width(), texture.height());
            let stride = width
                .checked_mul(4)
                .and_then(|row| row.checked_next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
                .ok_or_else(|| GpuError::Readback("too many tiles".to_string()))?;
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("craft3d edge tiles read"),
                size: u64::from(stride) * u64::from(height),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("craft3d edge tiles read"),
            });
            let values =
                self.read_back(&device, encoder, texture, &buffer, width, height, stride)?;
            Ok((width, height, values))
        })?
    }

    /// Copies `texture` (4 bytes per texel, the bits of `f32`) into
    /// `buffer` with `encoder` and maps it: the values, row-major.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::too_many_arguments)]
    fn read_back(
        &self,
        device: &wgpu::Device,
        mut encoder: wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        buffer: &wgpu::Buffer,
        width: u32,
        height: u32,
        stride: u32,
    ) -> Result<Vec<f32>, GpuError> {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                // The receiver may be gone; nothing to do then.
                let _ = sender.send(result);
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        receiver
            .recv()
            .map_err(|e| GpuError::Readback(e.to_string()))?
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| GpuError::Readback(e.to_string()))?;
        let mut values = Vec::with_capacity((width as usize).saturating_mul(height as usize));
        for row in mapped.chunks(stride as usize).take(height as usize) {
            let (fours, _) = row.as_chunks::<4>();
            values.extend(
                fours
                    .iter()
                    .take(width as usize)
                    .map(|b| f32::from_le_bytes(*b)),
            );
        }
        drop(mapped);
        buffer.unmap();
        Ok(values)
    }

    fn ensure_targets(&mut self, params: &RenderParams, capture: bool, tile: u32) {
        let (width, height) = (params.width, params.height);
        let same_size = self
            .targets
            .as_ref()
            .is_some_and(|t| (t.width, t.height, t.tile) == (width, height, tile));
        if same_size {
            let has_capture = self.targets.as_ref().is_some_and(|t| t.capture.is_some());
            if !capture || has_capture {
                return;
            }
            if let Some(targets) = &mut self.targets {
                targets.capture = Some(Self::capture_targets(&self.device, width, height));
            }
            return;
        }
        // Drop the old textures before allocating the new ones.
        self.targets = None;
        let device = &self.device;
        let samples = self.samples;
        let texture = |label: &str, format, sample_count, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                // Within the device limit: check_params ran first.
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let msaa = (samples > 1).then(|| {
            view(&texture(
                "craft3d samples colour",
                COLOR_FORMAT,
                samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ))
        });
        let depth = view(&texture(
            "craft3d depth",
            DEPTH_FORMAT,
            samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        ));
        let shown = texture(
            "craft3d viewport",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let shown_view = view(&shown);
        // One texel per tile, at least one.
        let tiles_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("craft3d edge tiles"),
            size: wgpu::Extent3d {
                width: width.div_ceil(tile).max(1),
                height: height.div_ceil(tile).max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TILE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let tiles = view(&tiles_texture);
        let ink = self.density.as_ref().ok().map(|density| {
            let ink = view(&texture(
                "craft3d edge ink",
                INK_FORMAT,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ));
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("craft3d edge tiles"),
                layout: &density.tiles_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.lines_uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&ink),
                    },
                ],
            });
            (ink, bind)
        });
        let distance = view(&texture(
            "craft3d distances",
            DISTANCE_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        ));
        let mut distance_entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: self.scene_uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: self.lines_uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&depth),
            },
        ];
        if self.samples == 1 {
            distance_entries.push(wgpu::BindGroupEntry {
                binding: DEPTH_SAMPLER_BINDING,
                resource: wgpu::BindingResource::Sampler(&self.depth_sampler),
            });
        }
        let distance_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d distances"),
            layout: &self.distance_layout,
            entries: &distance_entries,
        });
        let lines_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d lines"),
            layout: &self.lines_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.scene_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.lines_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&depth),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&tiles),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&distance),
                },
            ],
        });
        let capture = capture.then(|| Self::capture_targets(device, width, height));
        self.targets = Some(Targets {
            width,
            height,
            msaa,
            depth,
            shown,
            shown_view,
            lines_bind,
            tile,
            ink,
            tiles,
            tiles_texture,
            distance,
            distance_bind,
            capture,
            peel: None,
            cut: None,
        });
    }

    /// The count of a cut-open picture, at the size of the targets; for a
    /// see-through one the group of the cap behind the front, and for a
    /// capture of a renderer with several samples the count at one.
    fn ensure_cut_targets(&mut self, see_through: bool, capture: bool) {
        let (Some(targets), Some(Ok(pipelines))) = (&mut self.targets, &self.cut) else {
            return;
        };
        let device = &self.device;
        if targets.cut.is_none() {
            let parity = cut::parity_view(device, targets.width, targets.height, self.samples);
            let cap_bind = cut::cap_bind(
                device,
                &pipelines.cap_layout,
                self.samples > 1,
                None,
                &parity,
            );
            targets.cut = Some(CutTargets {
                parity,
                cap_bind,
                cap_behind_bind: None,
                flat: None,
            });
        }
        let Some(count) = &mut targets.cut else {
            return;
        };
        if see_through
            && count.cap_behind_bind.is_none()
            && let (Some(peeled), Some(peel)) = (&pipelines.peeled, &targets.peel)
        {
            count.cap_behind_bind = Some(cut::cap_bind(
                device,
                &peeled.cap_behind_layout,
                self.samples > 1,
                Some((
                    peel.key.as_ref().unwrap_or(&targets.depth),
                    &self.depth_sampler,
                )),
                &count.parity,
            ));
        }
        if capture
            && count.flat.is_none()
            && let Some(layout) = &pipelines.cap_flat_layout
        {
            let parity = cut::parity_view(device, targets.width, targets.height, 1);
            let bind = cut::cap_bind(device, layout, false, None, &parity);
            count.flat = Some((parity, bind));
        }
    }

    /// The layers of a see-through picture, at the size of the targets.
    fn ensure_peel_targets(&mut self) {
        let (Some(targets), Ok(pipelines)) = (&mut self.targets, &self.see_through) else {
            return;
        };
        if targets.peel.is_some() {
            return;
        }
        let device = &self.device;
        let texture = |label: &str, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: targets.width,
                    height: targets.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: self.samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let layer = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let front = view(&texture("craft3d front", COLOR_FORMAT, layer));
        let key =
            (self.samples > 1).then(|| view(&texture("craft3d front keys", COLOR_FORMAT, layer)));
        let behind = view(&texture("craft3d behind", COLOR_FORMAT, layer));
        let depth = view(&texture(
            "craft3d depth behind",
            DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ));
        let bound = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        let peel_bind = peel::peel_bind(
            device,
            &pipelines.peel_layout,
            self.samples > 1,
            key.as_ref().unwrap_or(&targets.depth),
            &self.depth_sampler,
        );
        let blend_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d blend"),
            layout: &pipelines.blend_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pipelines.blend_uniforms.as_entire_binding(),
                },
                bound(1, &front),
                bound(2, &behind),
                bound(3, &pipelines.table.view),
            ],
        });
        targets.peel = Some(PeelTargets {
            front,
            key,
            behind,
            depth,
            peel_bind,
            blend_bind,
        });
    }

    fn capture_targets(device: &wgpu::Device, width: u32, height: u32) -> Capture {
        let texture = |label: &str, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let data = texture(
            "craft3d capture data",
            DATA_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let data_view = view(&data);
        let depth = view(&texture(
            "craft3d capture depth",
            DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ));
        Capture {
            data,
            data_view,
            depth,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode(
        &self,
        mesh: &GpuMesh,
        lines: &GpuLines,
        scene: &[f32; SCENE_FLOATS],
        line_uniforms: &[f32; LINES_FLOATS],
        params: &RenderParams,
        overlay: &OverlayParams,
        options: DrawOptions,
        cut: Option<&Cut>,
        studio: Option<&([f32; crate::studio::STUDIO_FLOATS], crate::studio::Passes)>,
    ) -> Result<(), GpuError> {
        let targets = self.targets.as_ref().ok_or(GpuError::NothingDrawn)?;
        self.queue
            .write_buffer(&self.scene_uniforms, 0, &float_bytes(scene));
        let lit = match (studio, &self.studio) {
            (Some((numbers, passes)), Some(Ok(look))) => {
                self.queue
                    .write_buffer(&look.gpu.uniforms, 0, &float_bytes(numbers));
                Some((look, passes))
            }
            (Some(_), _) => return Err(GpuError::NothingDrawn),
            (None, _) => None,
        };
        self.queue
            .write_buffer(&self.lines_uniforms, 0, &float_bytes(line_uniforms));
        // The axes at this frame's length, when it changed.
        if let Some(buffer) = &lines.buffer
            && lines.axis_length.get() != overlay.axis_length
        {
            for (offset, axis) in &lines.axes {
                if let Some(packed) = pack_segment(axis, lines.origin, overlay.axis_length) {
                    self.queue.write_buffer(buffer, *offset, &packed);
                }
            }
            lines.axis_length.set(overlay.axis_length);
        }

        let [r, g, b] = params.background.map(|c| f64::from(c) / 255.0);
        let background = wgpu::Color { r, g, b, a: 1.0 };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("craft3d viewport"),
            });
        let color_view = targets.msaa.as_ref().unwrap_or(&targets.shown_view);
        let opacity = params.opacity();
        // The passes of a see-through picture, when it is one.
        let peeled = if options.bodies && opacity < 1.0 {
            let pipelines = self
                .see_through
                .as_ref()
                .map_err(|why| GpuError::SeeThrough(why.clone()))?;
            let peel = targets.peel.as_ref().ok_or(GpuError::NothingDrawn)?;
            pipelines.table.set(&self.queue, opacity);
            Some((pipelines, peel))
        } else {
            None
        };
        let draw_mesh = |pass: &mut wgpu::RenderPass<'_>| {
            // In mesh order: the depth test's tie rule depends on it.
            for chunk in &mesh.chunks {
                pass.set_vertex_buffer(0, chunk.buffer.slice(..));
                pass.draw(0..chunk.vertices, 0..1);
            }
        };
        let color_attachment = |view, load| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let depth_attachment = |view, store| {
            Some(wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store,
                }),
                stencil_ops: None,
            })
        };
        // The passes of a cut-open picture, when it is one: its pipelines,
        // its count and, when the plane has a cap, the cap's triangle.
        let cut_open = match cut {
            Some(cut) => {
                let pipelines = match &self.cut {
                    Some(Ok(pipelines)) => pipelines,
                    Some(Err(why)) => return Err(GpuError::Section(why.clone())),
                    None => return Err(GpuError::NothingDrawn),
                };
                let count = targets.cut.as_ref().ok_or(GpuError::NothingDrawn)?;
                let cap = cut::cap_vertices(mesh, cut);
                if let Some(bytes) = &cap {
                    self.queue.write_buffer(&self.cap_buffer, 0, bytes);
                }
                Some((pipelines, count, cap.is_some()))
            }
            None => None,
        };
        let cut_peeled = match (cut_open, peeled) {
            (Some((pipelines, ..)), Some(_)) => {
                Some(pipelines.peeled.as_ref().ok_or(GpuError::NothingDrawn)?)
            }
            _ => None,
        };
        if let Some((pipelines, count, true)) = cut_open {
            cut::count_pass(
                &mut encoder,
                &count.parity,
                &pipelines.parity,
                &self.scene_bind,
                mesh,
            );
        }
        // The studio look's passes before the bodies, with the background
        // drawn into the colour target; and the bodies' group 1.
        let lit = match lit {
            Some((look, passes)) => Some((
                look,
                look.encode_before(
                    &self.device,
                    &mut encoder,
                    &self.scene_bind,
                    mesh,
                    passes,
                    color_view,
                )?,
            )),
            None => None,
        };
        {
            // The bodies. Seen through, this is the front layer: its alpha
            // says where it is seen through (0 where nothing is drawn).
            let (view, clear) = match peeled {
                Some((_, peel)) => (&peel.front, wgpu::Color { r, g, b, a: 0.0 }),
                None => (color_view, background),
            };
            let load = if lit.is_some() {
                wgpu::LoadOp::Load
            } else {
                wgpu::LoadOp::Clear(clear)
            };
            // Where there is no front there is no fragment to compare a key
            // with: any clear value will do.
            let key = peeled.and_then(|(_, peel)| peel.key.as_ref());
            let attachments = [
                color_attachment(view, load),
                key.and_then(|key| color_attachment(key, wgpu::LoadOp::Clear(wgpu::Color::BLACK))),
            ];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d bodies"),
                color_attachments: attachments
                    .get(..1 + usize::from(key.is_some()))
                    .unwrap_or(&[]),
                depth_stencil_attachment: depth_attachment(&targets.depth, wgpu::StoreOp::Store),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The triangles' pipeline and, for a cut-open picture, the
            // cap's.
            let (bodies, cap) = match (cut_open, cut_peeled, peeled) {
                (Some(_), Some(cut), _) => (&cut.front, Some(&cut.cap_front)),
                (Some((cut, ..)), None, _) if options.bodies => (&cut.color, Some(&cut.cap)),
                (Some((cut, ..)), None, _) => (&cut.depth_only, Some(&cut.cap_depth_only)),
                (None, _, Some((pipelines, _))) => (&pipelines.front_pipeline, None),
                (None, _, None) if options.bodies => (&self.color_pipeline, None),
                (None, _, None) => (&self.depth_only_pipeline, None),
            };
            pass.set_bind_group(0, &self.scene_bind, &[]);
            match &lit {
                Some((look, bind)) => {
                    pass.set_pipeline(&look.main);
                    pass.set_bind_group(1, bind, &[]);
                }
                None => pass.set_pipeline(bodies),
            }
            draw_mesh(&mut pass);
            if let (Some(cap), Some((_, count, true))) = (cap, cut_open) {
                cut::draw_cap(&mut pass, cap, &count.cap_bind, &self.cap_buffer);
            }
        }
        if let Some((pipelines, peel)) = peeled {
            {
                // The surface behind the front one.
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("craft3d behind"),
                    color_attachments: &[color_attachment(
                        &peel.behind,
                        wgpu::LoadOp::Clear(background),
                    )],
                    depth_stencil_attachment: depth_attachment(&peel.depth, wgpu::StoreOp::Discard),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(cut_peeled.map_or(&pipelines.behind_pipeline, |cut| &cut.behind));
                pass.set_bind_group(0, &self.scene_bind, &[]);
                pass.set_bind_group(1, &peel.peel_bind, &[]);
                draw_mesh(&mut pass);
                if let (Some(cut_peeled), Some((_, count, true))) = (cut_peeled, cut_open) {
                    let bind = count
                        .cap_behind_bind
                        .as_ref()
                        .ok_or(GpuError::NothingDrawn)?;
                    cut::draw_cap(&mut pass, &cut_peeled.cap_behind, bind, &self.cap_buffer);
                }
            }
            // The front over the one behind, per sample, into the shown
            // texture.
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d blend"),
                color_attachments: &[color_attachment(
                    &targets.shown_view,
                    wgpu::LoadOp::Clear(background),
                )],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipelines.blend_pipeline);
            pass.set_bind_group(0, &peel.blend_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        // The distances the lines and the fills read, from the depth
        // buffer of the bodies (of the front when they are seen through).
        if lines.buffer.is_some() || lines.fills.is_some() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d distances"),
                color_attachments: &[color_attachment(
                    &targets.distance,
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                )],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.distance_pipeline);
            pass.set_bind_group(0, &targets.distance_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        // How crowded the edges in sight are: their ink, summed per pixel,
        // then its mean per tile, which the lines read.
        if let (Ok(density), Some((ink, tiles_bind)), Some(buffer)) =
            (&self.density, &targets.ink, &lines.buffer)
            && lines.edges > 0
        {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("craft3d edge ink"),
                    color_attachments: &[color_attachment(
                        ink,
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    )],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&density.ink_pipeline);
                pass.set_bind_group(0, &targets.lines_bind, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..VERTICES_PER_SEGMENT, 0..lines.instances);
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d edge tiles"),
                color_attachments: &[color_attachment(
                    &targets.tiles,
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                )],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&density.tiles_pipeline);
            pass.set_bind_group(0, tiles_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            // The lines over the bodies, and the resolve of the samples
            // into the shown texture; over a see-through picture, which is
            // in the shown texture already, straight onto that.
            let (view, resolve, pipeline, fills_pipeline) = match peeled {
                Some(_) => (
                    &targets.shown_view,
                    None,
                    self.lines_flat_pipeline
                        .as_ref()
                        .unwrap_or(&self.lines_pipeline),
                    self.fills_flat_pipeline
                        .as_ref()
                        .unwrap_or(&self.fills_pipeline),
                ),
                None => (
                    color_view,
                    targets.msaa.as_ref().map(|_| &targets.shown_view),
                    &self.lines_pipeline,
                    &self.fills_pipeline,
                ),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d lines"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        // Stored, not discarded, although the samples are
                        // not needed after the resolve: wgpu's OpenGL
                        // backend resolves nothing out of a discarded
                        // attachment.
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The fills first: the lines go over them.
            if let Some(buffer) = &lines.fills
                && lines.fill_vertices > 0
            {
                pass.set_pipeline(fills_pipeline);
                pass.set_bind_group(0, &targets.lines_bind, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..lines.fill_vertices, 0..1);
            }
            if let Some(buffer) = &lines.buffer
                && lines.instances > 0
            {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &targets.lines_bind, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..VERTICES_PER_SEGMENT, 0..lines.instances);
            }
        }
        // The count of a cut-open capture: the picture's own with one
        // sample per pixel, counted again at one sample otherwise.
        let capture_cap = match cut_open {
            Some((pipelines, count, true)) if options.capture => {
                match (&pipelines.parity_flat, &count.flat) {
                    (Some(parity), Some((view, bind))) => {
                        cut::count_pass(&mut encoder, view, parity, &self.scene_bind, mesh);
                        Some(bind)
                    }
                    (Some(_), None) => return Err(GpuError::NothingDrawn),
                    (None, _) => Some(&count.cap_bind),
                }
            }
            _ => None,
        };
        if options.capture {
            let capture = targets.capture.as_ref().ok_or(GpuError::NothingDrawn)?;
            // Where nothing is drawn: no face, at an infinite distance, as
            // in the CPU renderer's frame.
            let nothing = wgpu::Color {
                r: f64::from(NO_FACE),
                g: f64::from(f32::INFINITY.to_bits()),
                b: 0.0,
                a: 0.0,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("craft3d capture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &capture.data_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(nothing),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &capture.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(cut_open.map_or(&self.data_pipeline, |(cut, ..)| &cut.data));
            pass.set_bind_group(0, &self.scene_bind, &[]);
            for chunk in &mesh.chunks {
                pass.set_vertex_buffer(0, chunk.buffer.slice(..));
                pass.draw(0..chunk.vertices, 0..1);
            }
            if let (Some(bind), Some((pipelines, ..))) = (capture_cap, cut_open) {
                cut::draw_cap(&mut pass, &pipelines.cap_data, bind, &self.cap_buffer);
            }
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

/// The numbers of `lines.wgsl`'s uniform block.
fn lines_uniforms(
    lines: &GpuLines,
    camera: &Camera,
    params: &RenderParams,
    overlay: &OverlayParams,
    samples: u32,
) -> Result<[f32; LINES_FLOATS], GpuError> {
    let axes = camera.axes()?;
    let eye = axes.eye - lines.origin;
    // A slack that is not a number hides nothing, as on the CPU.
    let slack = if overlay.slack.is_finite() && overlay.slack >= 0.0 {
        overlay.slack
    } else {
        f64::INFINITY
    };
    let edges = &overlay.edges;
    let wide = [
        f64::from(params.width),
        f64::from(params.height),
        slack,
        f64::from(samples),
        f64::from(overlay.hidden_opacity) / 255.0,
        craft3d_render::SUBPIXEL_GRAIN,
        f64::from(edges.tile()),
        0.0,
        eye.x,
        eye.y,
        eye.z,
        0.0,
        edges.outline_near,
        edges.outline_far,
        edges.outline_step,
        edges.outline_slope,
        edges.fade_short,
        edges.fade_long,
        edges.dense_cap,
        0.0,
        edges.fade_floor,
        0.0,
        0.0,
        0.0,
    ];
    let mut out = [0.0_f32; LINES_FLOATS];
    for (slot, value) in out.iter_mut().zip(wide) {
        // Deliberate narrowing at the GPU boundary.
        *slot = value as f32;
    }
    // Infinite slack is fine; an eye that does not fit is not, nor are
    // numbers of the edges' look that are not.
    if out
        .get(8..11)
        .is_some_and(|eye| eye.iter().any(|c| !c.is_finite()))
    {
        return Err(GpuError::OutOfRange("the camera"));
    }
    if out
        .get(12..21)
        .is_some_and(|look| look.iter().any(|c| !c.is_finite()))
        || !(edges.outline_far > edges.outline_near && edges.outline_near > 0.0)
    {
        return Err(GpuError::OutOfRange("how the edges are drawn"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(from: Vec3, to: Vec3) -> Segment {
        Segment {
            from,
            to,
            color: [1, 2, 3],
            width: 1.5,
            tested: true,
            dimmed: false,
            axis: false,
            edge: None,
        }
    }

    #[test]
    fn a_segment_packs_to_one_instance_with_its_colour_width_and_flags() {
        let packed = pack_segment(
            &segment(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)),
            Vec3::new(1.0, 1.0, 1.0),
            1.0,
        )
        .unwrap();
        assert_eq!(packed.len(), SEGMENT_BYTES);
        let floats = |bytes: &[u8]| -> Vec<f32> {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect()
        };
        assert_eq!(floats(&packed[..24]), [0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(&packed[24..28], &[1, 2, 3, 255]);
        assert_eq!(floats(&packed[28..32]), [1.5]);
        assert_eq!(&packed[32..36], &FLAG_TESTED.to_le_bytes());
        // Not an edge: its outline is itself, no box.
        assert_eq!(&packed[36..40], &[1, 2, 3, 255]);
        assert_eq!(floats(&packed[40..44]), [1.5]);
        assert_eq!(floats(&packed[44..68]), [0.0; 6]);
        // An edge: its outline, its box, and the flags that say so.
        let edge = Segment {
            edge: Some(EdgeSegment {
                outline_color: [7, 8, 9],
                outline_width: 2.5,
                extent: Some([Vec3::new(1.0, 1.0, 1.0), Vec3::new(2.0, 3.0, 4.0)]),
            }),
            ..segment(Vec3::ZERO, Vec3::X)
        };
        let packed = pack_segment(&edge, Vec3::ZERO, 1.0).unwrap();
        assert_eq!(
            &packed[32..36],
            &(FLAG_TESTED | FLAG_EDGE | FLAG_EXTENT).to_le_bytes()
        );
        assert_eq!(&packed[36..40], &[7, 8, 9, 255]);
        assert_eq!(floats(&packed[40..44]), [2.5]);
        assert_eq!(floats(&packed[44..68]), [1.0, 1.0, 1.0, 2.0, 3.0, 4.0]);
        // A box that does not fit: an edge that is never short.
        let far = Segment {
            edge: Some(EdgeSegment {
                extent: Some([Vec3::ZERO, Vec3::new(1e300, 0.0, 0.0)]),
                ..edge.edge.unwrap()
            }),
            ..edge
        };
        let packed = pack_segment(&far, Vec3::ZERO, 1.0).unwrap();
        assert_eq!(&packed[32..36], &(FLAG_TESTED | FLAG_EDGE).to_le_bytes());
        // An axis is scaled by the axis length.
        let axis = Segment {
            axis: true,
            ..segment(Vec3::ZERO, Vec3::X)
        };
        let packed = pack_segment(&axis, Vec3::ZERO, 7.0).unwrap();
        assert_eq!(floats(&packed[12..24]), [7.0, 0.0, 0.0]);
    }

    #[test]
    fn a_fill_packs_to_three_vertices_and_a_hostile_one_is_left_out() {
        let fill = Fill {
            center: Vec3::new(2.0, 3.0, 4.0),
            u: Vec3::new(1.0, 0.0, 0.0),
            v: Vec3::new(0.0, 2.0, 0.0),
            color: [10, 20, 30],
            opacity: 48,
        };
        let packed = pack_fill(&fill, Vec3::new(1.0, 1.0, 1.0)).unwrap();
        assert_eq!(packed.len(), 3 * FILL_VERTEX_BYTES);
        for vertex in packed.as_chunks::<FILL_VERTEX_BYTES>().0 {
            let floats: Vec<f32> = vertex[..36]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            assert_eq!(floats, [1.0, 2.0, 3.0, 1.0, 0.0, 0.0, 0.0, 2.0, 0.0]);
            assert_eq!(&vertex[36..40], &[10, 20, 30, 48]);
        }
        for bad in [
            Fill {
                center: Vec3::new(f64::NAN, 0.0, 0.0),
                ..fill
            },
            Fill {
                u: Vec3::new(1e300, 0.0, 0.0),
                ..fill
            },
            Fill {
                v: Vec3::new(0.0, f64::INFINITY, 0.0),
                ..fill
            },
        ] {
            assert!(pack_fill(&bad, Vec3::ZERO).is_none(), "{bad:?}");
        }
    }

    /// The shaders over the picture, for one sample per pixel (OpenGL,
    /// WebGL2) and for several, validate with naga without an adapter, and
    /// the single-sample ones translate to GLSL ES 3.10, as wgpu's OpenGL
    /// backend gets them. (Not to ES 3.00, WebGL2: the depth test's array
    /// of arrays, there since 2026-10-18, needs 3.10.)
    #[test]
    fn the_overlay_shaders_validate_without_an_adapter() {
        use wgpu::naga;
        for samples in [1, 4] {
            let sources = overlay_sources(samples);
            for (name, source) in [
                ("lines", sources.lines.as_str()),
                ("fills", sources.fills.as_str()),
                ("distance", sources.distance.as_str()),
                ("depth read", sources.depth_read.as_str()),
                ("edge tiles", EDGE_TILES_SOURCE),
            ] {
                let label = format!("{name}, {samples} samples");
                assert!(!source.contains("{{"), "{label}: a marker was left");
                let module = naga::front::wgsl::parse_str(source)
                    .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
                if samples == 1 {
                    // OpenGL has one sample per pixel: what it runs is
                    // what WebGL2 (GLSL ES 3.00) takes.
                    crate::renderer::template_tests::translate_for_webgl2(&label, source);
                }
            }
            // The one pass a window runs that reads the depth buffer
            // samples it with one sample per pixel (WebGL2 reads 0 from a
            // depth texture loaded there); the depth read is for tests.
            let (loads, reads) =
                crate::renderer::template_tests::reads_of(&sources.distance, "depth_texture");
            if samples == 1 {
                assert_eq!((loads, reads), (0, 1), "distance, {samples} samples");
            } else {
                assert_eq!((loads > 0, reads), (true, 0), "distance, {samples} samples");
            }
        }
    }

    #[test]
    fn hostile_segments_are_left_out() {
        for bad in [
            segment(Vec3::new(f64::NAN, 0.0, 0.0), Vec3::X),
            segment(Vec3::ZERO, Vec3::new(f64::INFINITY, 0.0, 0.0)),
            segment(Vec3::new(1e300, 0.0, 0.0), Vec3::X),
            Segment {
                width: f32::NAN,
                ..segment(Vec3::ZERO, Vec3::X)
            },
        ] {
            assert!(pack_segment(&bad, Vec3::ZERO, 1.0).is_none(), "{bad:?}");
        }
        // A negative width is drawn at the smallest width, not refused.
        let thin = Segment {
            width: -3.0,
            ..segment(Vec3::ZERO, Vec3::X)
        };
        assert!(pack_segment(&thin, Vec3::ZERO, 1.0).is_some());
    }
}
