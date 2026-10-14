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
//! 2. **Lines.** The segments of a [`GpuLines`], with `lines.wgsl`, into
//!    the same colour target, each as a quad in screen space inked by the
//!    distance of a pixel's centre to the segment (the CPU's `draw_line`)
//!    and, when `tested`, hidden where the surface under the pixel and the
//!    eight around it is nearer than the line (the CPU's `covers`), read
//!    from the depth buffer of pass 1: a hidden edge is left out, a hidden
//!    sketch is drawn with the hidden opacity. The pass resolves the
//!    samples into the shown texture.
//! 3. **Capture** (only when asked): face ids and distances at one sample
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
    COLOR_FORMAT, DATA_FORMAT, DEPTH_FORMAT, MESH_ATTRIBUTES, SCENE_FLOATS, check_params,
    float_bytes, guarded, scene_pipeline, scene_uniforms, target, uniform_entry,
};
use crate::{GpuError, Mesh};
use craft3d_math::{Bbox3, Vec3};
use craft3d_render::{Camera, Cut, Frame, NO_FACE, RenderParams};
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

/// Largest accepted number of segments (277 MB of vertex data).
pub const MAX_SEGMENTS: usize = 1 << 20;
/// Two ends (3 × `f32` each), the corner (2 × `f32`), the colour (`u32`),
/// the width (`f32`) and the flags (`u32`).
const LINE_VERTEX_BYTES: usize = 44;
/// Two triangles.
const VERTICES_PER_SEGMENT: usize = 6;
const SEGMENT_BYTES: usize = VERTICES_PER_SEGMENT * LINE_VERTEX_BYTES;
/// Number of `f32` in the lines uniform block (three `vec4`).
const LINES_FLOATS: usize = 12;
/// Samples the viewport asks for.
pub const WANTED_SAMPLES: u32 = 4;

const FLAG_TESTED: u32 = 1;
const FLAG_DIMMED: u32 = 2;

/// The segments on the GPU, made by [`Onscreen::upload_lines`]. They
/// belong to the renderer that made them.
pub struct GpuLines {
    buffer: Option<wgpu::Buffer>,
    vertices: u32,
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

/// The six vertices of one segment, or `None` when it cannot be drawn.
fn pack_segment(segment: &Segment, origin: Vec3, axis_length: f64) -> Option<[u8; SEGMENT_BYTES]> {
    let scale = if segment.axis { axis_length } else { 1.0 };
    let (from, to) = (segment.from * scale, segment.to * scale);
    if !(from.is_finite() && to.is_finite() && segment.width.is_finite()) {
        return None;
    }
    let a = narrow(from - origin)?;
    let b = narrow(to - origin)?;
    let [r, g, bl] = segment.color;
    let color = u32::from(r) | (u32::from(g) << 8) | (u32::from(bl) << 16) | (255 << 24);
    let flags =
        if segment.tested { FLAG_TESTED } else { 0 } | if segment.dimmed { FLAG_DIMMED } else { 0 };
    let width = segment.width.max(0.0);
    let mut out = [0_u8; SEGMENT_BYTES];
    let corners: [[f32; 2]; VERTICES_PER_SEGMENT] = [
        [0.0, -1.0],
        [1.0, -1.0],
        [1.0, 1.0],
        [0.0, -1.0],
        [1.0, 1.0],
        [0.0, 1.0],
    ];
    for (vertex, corner) in out
        .as_chunks_mut::<LINE_VERTEX_BYTES>()
        .0
        .iter_mut()
        .zip(corners)
    {
        let floats = [a[0], a[1], a[2], b[0], b[1], b[2], corner[0], corner[1]];
        let mut bytes = float_bytes(&floats);
        bytes.extend_from_slice(&color.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&flags.to_le_bytes());
        vertex.copy_from_slice(&bytes);
    }
    Some(out)
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
    ) -> CutPipelines {
        let multisampled = samples > 1;
        let module = cut::module(device, multisampled);
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
        let flat_module = multisampled.then(|| cut::module(device, false));
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
        );
        let behind_pipeline = scene_pipeline(
            device,
            "craft3d behind",
            &module,
            behind_entry,
            &[Some(scene_layout), Some(&peel_layout)],
            &[target(COLOR_FORMAT)],
            samples,
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
    /// Identifies this renderer in what it uploads.
    token: Arc<()>,
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
        Self::with_samples(device, queue, samples)
    }

    /// A renderer with `samples` samples per pixel (1, 2, 4 or 8; the
    /// device must support them for `Rgba8Unorm` and `Depth32Float`).
    pub fn with_samples(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        samples: u32,
    ) -> Result<Onscreen, GpuError> {
        if !matches!(samples, 1 | 2 | 4 | 8) {
            return Err(GpuError::Unsupported("that number of samples per pixel"));
        }
        let mut onscreen = guarded(device, || Self::build(device, queue, samples))?;
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
                CutPipelines::build(&self.device, &self.scene_layout, peel_layout, self.samples)
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

    fn build(device: &wgpu::Device, queue: &wgpu::Queue, samples: u32) -> Onscreen {
        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        // The depth buffer is bound as an unfilterable float texture, not
        // as a depth texture: naga's GLSL backend cannot `textureLoad` a
        // depth texture, and WebGPU binds a depth format either way.
        let (depth_type, load) = if samples > 1 {
            (
                "texture_multisampled_2d<f32>",
                "textureLoad(depth_texture, p, s).r",
            )
        } else {
            ("texture_2d<f32>", "textureLoad(depth_texture, p, 0).r")
        };
        let lines_source = include_str!("lines.wgsl")
            .replace("{{DEPTH}}", depth_type)
            .replace("{{LOAD}}", load);
        let lines_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d lines"),
            source: wgpu::ShaderSource::Wgsl(lines_source.into()),
        });
        let fills_source = include_str!("fills.wgsl")
            .replace("{{DEPTH}}", depth_type)
            .replace("{{LOAD}}", load);
        let fills_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("craft3d fills"),
            source: wgpu::ShaderSource::Wgsl(fills_source.into()),
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
            ],
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
        // `Less`: on a tie the triangle drawn first stays, as on the CPU.
        let depth = Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
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
        let line_attributes = [
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
                format: wgpu::VertexFormat::Float32x2,
                offset: 24,
                shader_location: 2,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 32,
                shader_location: 3,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32,
                offset: 36,
                shader_location: 4,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: 40,
                shader_location: 5,
            },
        ];
        let lines_pipeline_for = |count: u32| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("craft3d lines"),
                layout: Some(&lines_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &lines_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: LINE_VERTEX_BYTES as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &line_attributes,
                    })],
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

        Onscreen {
            device: device.clone(),
            queue: queue.clone(),
            samples,
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
            see_through: Err("it was not set up".to_string()),
            cut: None,
            cap_buffer: cut::cap_buffer(device),
            targets: None,
            staging: Mutex::new(None),
            drawn: false,
            captured: false,
            token: Arc::new(()),
        }
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
        guarded(&self.device, || {
            GpuMesh::upload(&self.device, &self.queue, mesh, &self.token)
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
        let mut count = 0_usize;
        for segment in segments {
            let Some(packed) = pack_segment(segment, origin, 1.0) else {
                continue;
            };
            if segment.axis {
                axes.push((bytes.len() as u64, *segment));
            }
            bytes.extend_from_slice(&packed);
            count += 1;
        }
        let vertices =
            u32::try_from(bytes.len() / LINE_VERTEX_BYTES).map_err(|_| GpuError::TooManyLines {
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
            vertices,
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
        let line_uniforms = lines_uniforms(lines, camera, params, overlay, self.samples)?;
        let device = self.device.clone();
        guarded(&device, || {
            self.ensure_targets(params, options.capture);
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

    fn ensure_targets(&mut self, params: &RenderParams, capture: bool) {
        let (width, height) = (params.width, params.height);
        let same_size = self
            .targets
            .as_ref()
            .is_some_and(|t| (t.width, t.height) == (width, height));
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
            let cap_bind = cut::cap_bind(device, &pipelines.cap_layout, None, &parity);
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
                Some(peel.key.as_ref().unwrap_or(&targets.depth)),
                &count.parity,
            ));
        }
        if capture
            && count.flat.is_none()
            && let Some(layout) = &pipelines.cap_flat_layout
        {
            let parity = cut::parity_view(device, targets.width, targets.height, 1);
            let bind = cut::cap_bind(device, layout, None, &parity);
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
        let peel_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("craft3d front"),
            layout: &pipelines.peel_layout,
            entries: &[bound(0, key.as_ref().unwrap_or(&targets.depth))],
        });
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
    ) -> Result<(), GpuError> {
        let targets = self.targets.as_ref().ok_or(GpuError::NothingDrawn)?;
        self.queue
            .write_buffer(&self.scene_uniforms, 0, &float_bytes(scene));
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
                    load: wgpu::LoadOp::Clear(1.0),
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
        {
            // The bodies. Seen through, this is the front layer: its alpha
            // says where it is seen through (0 where nothing is drawn).
            let (view, clear) = match peeled {
                Some((_, peel)) => (&peel.front, wgpu::Color { r, g, b, a: 0.0 }),
                None => (color_view, background),
            };
            // Where there is no front there is no fragment to compare a key
            // with: any clear value will do.
            let key = peeled.and_then(|(_, peel)| peel.key.as_ref());
            let attachments = [
                color_attachment(view, wgpu::LoadOp::Clear(clear)),
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
            pass.set_pipeline(bodies);
            pass.set_bind_group(0, &self.scene_bind, &[]);
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
                && lines.vertices > 0
            {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &targets.lines_bind, &[]);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..lines.vertices, 0..1);
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
                        load: wgpu::LoadOp::Clear(1.0),
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
    let wide = [
        f64::from(params.width),
        f64::from(params.height),
        slack,
        f64::from(samples),
        f64::from(overlay.hidden_opacity) / 255.0,
        0.0,
        0.0,
        0.0,
        eye.x,
        eye.y,
        eye.z,
        0.0,
    ];
    let mut out = [0.0_f32; LINES_FLOATS];
    for (slot, value) in out.iter_mut().zip(wide) {
        // Deliberate narrowing at the GPU boundary.
        *slot = value as f32;
    }
    // Infinite slack is fine; an eye that does not fit is not.
    if out
        .get(8..11)
        .is_some_and(|eye| eye.iter().any(|c| !c.is_finite()))
    {
        return Err(GpuError::OutOfRange("the camera"));
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
        }
    }

    #[test]
    fn a_segment_packs_to_six_vertices_with_its_colour_width_and_flags() {
        let packed = pack_segment(
            &segment(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)),
            Vec3::new(1.0, 1.0, 1.0),
            1.0,
        )
        .unwrap();
        assert_eq!(packed.len(), 6 * LINE_VERTEX_BYTES);
        let floats = |bytes: &[u8]| -> Vec<f32> {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect()
        };
        let first = &packed[..LINE_VERTEX_BYTES];
        assert_eq!(
            floats(&first[..32]),
            [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 0.0, -1.0]
        );
        assert_eq!(&first[32..36], &[1, 2, 3, 255]);
        assert_eq!(floats(&first[36..40]), [1.5]);
        assert_eq!(&first[40..44], &FLAG_TESTED.to_le_bytes());
        // The corners go round two triangles.
        let corners: Vec<[f32; 2]> = packed
            .as_chunks::<LINE_VERTEX_BYTES>()
            .0
            .iter()
            .map(|v| {
                let f = floats(&v[24..32]);
                [f[0], f[1]]
            })
            .collect();
        assert_eq!(
            corners,
            [
                [0.0, -1.0],
                [1.0, -1.0],
                [1.0, 1.0],
                [0.0, -1.0],
                [1.0, 1.0],
                [0.0, 1.0]
            ]
        );
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
